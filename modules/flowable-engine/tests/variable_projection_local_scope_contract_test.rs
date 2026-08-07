//! Contract tests for the runtime `variables` projection table vs the
//! dual-map execution row (`Execution::variables` ∪ `Execution::local_variables`).
//!
//! Java stores one variable table per execution scope. Rust dual-writes that
//! projection from `RuntimeStore::insert_execution` so
//! `VariableService::create_variable_instance_query` / REST
//! `/runtime/variable-instances` see the same names. The projection is filled
//! by the store root on every execution insert/update — the engine API path
//! (`SetVariablesLocalCmd` / `RuntimeService#setVariableLocal`) and the
//! REST-scope cmd path (`MutateExecutionVariablesCmd`) alike.

use flowable_engine::cmd::execution_variable_cmd::{
    ExecutionVariableMutation, ExecutionVariableScope,
};
use flowable_engine::cmd::task_variable_cmd::VariableMutationMode;
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::engine::query::Query;
use serde_json::json;
use std::collections::HashMap;

const SIMPLE_TASK_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
    <process id="projSimpleProcess" name="Simple Process" isExecutable="true">
        <startEvent id="start" />
        <sequenceFlow id="f1" sourceRef="start" targetRef="task1" />
        <userTask id="task1" name="Task 1" />
        <sequenceFlow id="f2" sourceRef="task1" targetRef="end" />
        <endEvent id="end" />
    </process>
</definitions>"#;

fn deploy_and_start(engine: &ProcessEngine) -> String {
    let repo = engine.get_repository_service();
    repo.deploy(repo.create_deployment().add_string(
        "proj_simple.bpmn20.xml".to_string(),
        SIMPLE_TASK_XML.to_string(),
    ))
    .unwrap();
    let definition_id = repo.get_process_definition_ids().unwrap()[0].clone();
    engine
        .get_runtime_service()
        .start_process_instance_by_id(definition_id, None)
        .unwrap()
        .id
}

/// Process-level (`Execution::variables`) writes already project. This must stay
/// green before the local-scope root fix so the change is not a free pass that
/// merely rewrites projection behaviour wholesale.
#[test]
fn process_variable_remains_visible_in_the_runtime_projection() {
    let engine = ProcessEngine::new("proj-process-var-regression".to_string());
    let process_instance_id = deploy_and_start(&engine);
    let runtime = engine.get_runtime_service();

    runtime
        .set_variable(
            process_instance_id.clone(),
            "processNote".to_string(),
            json!("from-process"),
        )
        .expect("setting a process variable should succeed");

    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let projected = store.find_variables_by_execution_id(&process_instance_id, &mut session);
    assert_eq!(
        projected.get("processNote"),
        Some(&json!("from-process")),
        "process-variable writes must keep projecting into the runtime variables table"
    );

    let instances = engine
        .get_variable_service()
        .create_variable_instance_query()
        .list()
        .unwrap();
    assert!(
        instances.iter().any(|instance| {
            instance.process_instance_id == process_instance_id
                && instance.name == "processNote"
                && instance.value == json!("from-process")
        }),
        "variable-instance query must still surface process-level variables"
    );
}

/// The gap this work item closes: `SetVariablesLocalCmd` writes
/// `local_variables` and updates the execution row, but historically
/// `insert_execution` only projected `Execution::variables`. After the root
/// fix the local write must appear in the projection table / query.
#[test]
fn set_variable_local_is_visible_in_the_runtime_projection() {
    let engine = ProcessEngine::new("proj-local-var-gap".to_string());
    let process_instance_id = deploy_and_start(&engine);
    let runtime = engine.get_runtime_service();

    // On a simple sequential process the waiting execution is the process
    // instance / scope row itself, so the local write lands on that row.
    runtime
        .set_variable_local(
            process_instance_id.clone(),
            "localNote".to_string(),
            json!("from-local"),
        )
        .expect("setting a local variable should succeed");

    // Direct store projection (what insert_execution dual-writes).
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let projected = store.find_variables_by_execution_id(&process_instance_id, &mut session);
    assert_eq!(
        projected.get("localNote"),
        Some(&json!("from-local")),
        "SetVariablesLocalCmd must dual-write local_variables into the runtime projection"
    );

    // Public query surface used by REST /runtime/variable-instances.
    let instances = engine
        .get_variable_service()
        .create_variable_instance_query()
        .list()
        .unwrap();
    assert!(
        instances.iter().any(|instance| {
            instance.execution_id == process_instance_id
                && instance.name == "localNote"
                && instance.value == json!("from-local")
        }),
        "variable-instance query must include SetVariablesLocalCmd writes"
    );
}

/// Row-level LOCAL = variables ∪ local_variables with local winning a name
/// clash (handoff §6). The projection must not emit two rows for one name, and
/// the surviving value must be the local one.
#[test]
fn local_variable_shadows_process_variable_in_the_projection() {
    let engine = ProcessEngine::new("proj-local-shadow".to_string());
    let process_instance_id = deploy_and_start(&engine);
    let runtime = engine.get_runtime_service();

    runtime
        .set_variable(
            process_instance_id.clone(),
            "shared".to_string(),
            json!("process-value"),
        )
        .unwrap();

    // Force the dual-map clash the public write paths normally avoid: put the
    // same name into local_variables without clearing variables.
    {
        let store = engine.get_runtime_store();
        let mut session = store.create_session().unwrap();
        let mut execution = store
            .find_execution(&process_instance_id, &mut session)
            .expect("scope execution");
        execution
            .local_variables
            .insert("shared".to_string(), json!("local-value"));
        store.update_execution(&execution, &mut session);
        session.flush_and_commit().unwrap();
    }

    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let projected = store.find_variables_by_execution_id(&process_instance_id, &mut session);
    assert_eq!(
        projected.get("shared"),
        Some(&json!("local-value")),
        "local_variables must win the projection name clash"
    );
    // find_variables_by_execution_id is a HashMap keyed by name — one row per
    // name. The variable-instance query must also not list the name twice.
    let instances = engine
        .get_variable_service()
        .create_variable_instance_query()
        .list()
        .unwrap();
    let matches: Vec<_> = instances
        .iter()
        .filter(|instance| {
            instance.execution_id == process_instance_id && instance.name == "shared"
        })
        .collect();
    assert_eq!(
        matches.len(),
        1,
        "projection must not emit duplicate rows for a shadowed name"
    );
    assert_eq!(matches[0].value, json!("local-value"));
}

/// Batch local write path (`set_variables_local`) must project every name.
#[test]
fn set_variables_local_batch_is_visible_in_the_runtime_projection() {
    let engine = ProcessEngine::new("proj-local-batch".to_string());
    let process_instance_id = deploy_and_start(&engine);
    let runtime = engine.get_runtime_service();

    let mut batch = HashMap::new();
    batch.insert("a".to_string(), json!(1));
    batch.insert("b".to_string(), json!(2));
    runtime
        .set_variables_local(process_instance_id.clone(), batch)
        .unwrap();

    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let projected = store.find_variables_by_execution_id(&process_instance_id, &mut session);
    assert_eq!(projected.get("a"), Some(&json!(1)));
    assert_eq!(projected.get("b"), Some(&json!(2)));
}

/// The REST-scope mutation command (`MutateExecutionVariablesCmd`, behind the
/// execution/process-instance variable endpoints) writes through
/// `execution_entity_manager.update`, whose store root already dual-writes the
/// projection. This pins that contract: no cmd-layer `insert_variable` patch
/// is needed for a scope-routed write to be visible in the projection table.
#[test]
fn scope_cmd_mutation_is_visible_in_the_runtime_projection() {
    let engine = ProcessEngine::new("proj-scope-cmd".to_string());
    let process_instance_id = deploy_and_start(&engine);

    engine
        .get_variable_service()
        .mutate_variables_on_scope(
            process_instance_id.clone(),
            ExecutionVariableScope::Local,
            VariableMutationMode::Upsert,
            vec![ExecutionVariableMutation {
                name: "scopedNote".to_string(),
                value: json!("from-scope-cmd"),
            }],
        )
        .expect("scope-routed mutation should succeed");

    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let projected = store.find_variables_by_execution_id(&process_instance_id, &mut session);
    assert_eq!(
        projected.get("scopedNote"),
        Some(&json!("from-scope-cmd")),
        "MutateExecutionVariablesCmd writes must project via the store root"
    );

    let instances = engine
        .get_variable_service()
        .create_variable_instance_query()
        .list()
        .unwrap();
    assert!(
        instances.iter().any(|instance| {
            instance.execution_id == process_instance_id
                && instance.name == "scopedNote"
                && instance.value == json!("from-scope-cmd")
        }),
        "variable-instance query must include MutateExecutionVariablesCmd writes"
    );
}

/// Removing a variable must sweep its projection row: the projection is
/// synced by delete-then-reinsert on every execution write, so it mirrors the
/// two maps exactly instead of accumulating orphan rows.
#[test]
fn removed_variable_disappears_from_the_runtime_projection() {
    let engine = ProcessEngine::new("proj-remove-sweep".to_string());
    let process_instance_id = deploy_and_start(&engine);
    let runtime = engine.get_runtime_service();

    runtime
        .set_variable(process_instance_id.clone(), "doomed".to_string(), json!(1))
        .unwrap();
    runtime
        .set_variable(process_instance_id.clone(), "kept".to_string(), json!(2))
        .unwrap();

    engine
        .get_variable_service()
        .remove_variables_on_scope(
            process_instance_id.clone(),
            ExecutionVariableScope::Local,
            Some(vec!["doomed".to_string()]),
            false,
        )
        .expect("removing a variable should succeed");

    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let projected = store.find_variables_by_execution_id(&process_instance_id, &mut session);
    assert_eq!(
        projected.get("doomed"),
        None,
        "a removed variable must not linger in the projection"
    );
    assert_eq!(
        projected.get("kept"),
        Some(&json!(2)),
        "unrelated projection rows must survive the sweep"
    );

    let instances = engine
        .get_variable_service()
        .create_variable_instance_query()
        .list()
        .unwrap();
    assert!(
        !instances.iter().any(|instance| instance.name == "doomed"),
        "variable-instance query must not surface removed variables"
    );
}

const DATA_INPUT_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
    <process id="projDataInputProcess" name="Data Input Process" isExecutable="true">
        <startEvent id="start" />
        <sequenceFlow id="f1" sourceRef="start" targetRef="task1" />
        <userTask id="task1" name="Task 1">
            <dataInputAssociation>
                <sourceRef>globalVar</sourceRef>
                <targetRef>taskVar</targetRef>
            </dataInputAssociation>
        </userTask>
        <sequenceFlow id="f2" sourceRef="task1" targetRef="end" />
        <endEvent id="end" />
    </process>
</definitions>"#;

/// Orphan sweep: the userTask data-input-association path REPLACES the
/// execution's `variables` map wholesale (`set_process_variables`), dropping
/// every name the association does not route. The projection must mirror the
/// shrunken map exactly — rows for dropped names are orphans and must go.
#[test]
fn projection_sweeps_rows_for_names_dropped_from_the_execution_maps() {
    let engine = ProcessEngine::new("proj-orphan-sweep".to_string());
    let repo = engine.get_repository_service();
    repo.deploy(repo.create_deployment().add_string(
        "proj_data_input.bpmn20.xml".to_string(),
        DATA_INPUT_XML.to_string(),
    ))
    .unwrap();
    let definition_id = repo.get_process_definition_ids().unwrap()[0].clone();
    let process_instance_id = engine
        .get_runtime_service()
        .start_process_instance(
            engine
                .get_runtime_service()
                .create_process_instance_builder()
                .process_definition_id(definition_id)
                .variable("globalVar".to_string(), json!("value1"))
                .variable("unrelated".to_string(), json!("x")),
        )
        .unwrap()
        .id;

    // The execution row's maps were replaced by the routed variables only.
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let execution = store
        .find_execution(&process_instance_id, &mut session)
        .expect("scope execution");
    assert!(
        !execution.variables.contains_key("unrelated"),
        "test premise: the data input association shrinks the variables map"
    );

    let projected = store.find_variables_by_execution_id(&process_instance_id, &mut session);
    assert_eq!(
        projected.get("taskVar"),
        Some(&json!("value1")),
        "the routed variable must be projected"
    );
    assert_eq!(
        projected.get("globalVar"),
        None,
        "names dropped from the map must not linger in the projection"
    );
    assert_eq!(
        projected.get("unrelated"),
        None,
        "names dropped from the map must not linger in the projection"
    );
}
