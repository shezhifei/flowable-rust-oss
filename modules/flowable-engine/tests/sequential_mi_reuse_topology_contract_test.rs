//! Contract tests for Java sequential multi-instance **child execution reuse**.
//!
//! Java (`SequentialMultiInstanceBehavior#continueSequentialMultiInstance`, lines
//! 106–124) reuses the same instance child for non-SubProcess sequential MI:
//! after each round it clears the child's local variables (except `nrOf*`),
//! increments `loopCounter`, and re-executes on the **same execution id**.
//! Ended child rows do not accumulate across rounds.
//!
//! Observable differences locked here (were red under the pre-P6-A "spawn a new
//! child every round" topology):
//!   1. Child execution id is stable across all sequential rounds.
//!   2. No ended sibling rows pile up under the MI root while instances remain.
//!   3. `loopCounter` / element / element-index variables advance correctly on
//!      that same child after each completion.
//!
//! Regression guards (must stay green; P5-B wait-state + completion-condition
//! semantics must not regress):
//!   - Sequential MI with a completionCondition exits early.
//!   - Sequential MI wait-state (user task) suspends and resumes for all rounds.

use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::runtime::process_instance::ProcessInstance;
use serde_json::json;
use std::collections::HashMap;

const SEQUENTIAL_MI_THREE_ROUNDS_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
    <process id="seqMiReuseTopology" isExecutable="true">
        <startEvent id="start" />
        <sequenceFlow id="f1" sourceRef="start" targetRef="miTask" />
        <userTask id="miTask" name="MI Task">
            <multiInstanceLoopCharacteristics isSequential="true"
                                              flowable:collection="approvers"
                                              flowable:elementVariable="approver"
                                              flowable:elementIndexVariable="approverIndex" />
        </userTask>
        <sequenceFlow id="f2" sourceRef="miTask" targetRef="end" />
        <endEvent id="end" />
    </process>
</definitions>"#;

const SEQUENTIAL_MI_COMPLETION_CONDITION_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
    <process id="seqMiCompletionEarlyExit" isExecutable="true">
        <startEvent id="start" />
        <sequenceFlow id="f1" sourceRef="start" targetRef="miTask" />
        <userTask id="miTask" name="MI Task">
            <multiInstanceLoopCharacteristics isSequential="true">
                <loopCardinality>5</loopCardinality>
                <completionCondition>${nrOfCompletedInstances >= 2}</completionCondition>
            </multiInstanceLoopCharacteristics>
        </userTask>
        <sequenceFlow id="f2" sourceRef="miTask" targetRef="afterMi" />
        <userTask id="afterMi" name="After MI" />
        <sequenceFlow id="f3" sourceRef="afterMi" targetRef="end" />
        <endEvent id="end" />
    </process>
</definitions>"#;

fn deploy(engine: &ProcessEngine, resource: &str, xml: &str) -> String {
    let repo = engine.get_repository_service();
    repo.deploy(
        repo.create_deployment()
            .add_string(resource.to_string(), xml.to_string()),
    )
    .unwrap();
    engine
        .get_repository_service()
        .get_process_definition_ids()
        .unwrap()[0]
        .clone()
}

fn start(
    engine: &ProcessEngine,
    definition_id: String,
    variables: HashMap<String, serde_json::Value>,
) -> ProcessInstance {
    let runtime = engine.get_runtime_service();
    let mut builder = runtime
        .create_process_instance_builder()
        .process_definition_id(definition_id);
    for (name, value) in variables {
        builder = builder.variable(name, value);
    }
    runtime.start_process_instance(builder).unwrap()
}

fn children_of(
    engine: &ProcessEngine,
    parent_id: &str,
) -> Vec<flowable_engine::runtime::execution::Execution> {
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    store
        .snapshot_executions(&mut session)
        .into_values()
        .filter(|e| e.parent_id.as_deref() == Some(parent_id))
        .collect()
}

fn mi_root_id(engine: &ProcessEngine, child_execution_id: &str) -> String {
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    store
        .find_execution(child_execution_id, &mut session)
        .expect("child execution")
        .parent_id
        .expect("MI child must have a parent (the MI root)")
}

/// Difference: sequential MI reuses one child execution id for every round.
/// Java `continueSequentialMultiInstance` keeps the same instance execution.
#[test]
fn sequential_mi_reuses_same_child_execution_id_across_rounds() {
    let engine = ProcessEngine::new("seq-mi-reuse-child-id".to_string());
    let definition_id = deploy(
        &engine,
        "seq_mi_reuse.bpmn20.xml",
        SEQUENTIAL_MI_THREE_ROUNDS_XML,
    );
    let mut variables = HashMap::new();
    variables.insert("approvers".to_string(), json!(["amy", "ben", "cy"]));
    let process_instance = start(&engine, definition_id, variables);
    let task_service = engine.get_task_service();
    let runtime = engine.get_runtime_service();

    let mut stable_child_id: Option<String> = None;

    for (expected_index, expected_approver) in ["amy", "ben", "cy"].into_iter().enumerate() {
        let tasks = task_service
            .get_tasks_by_process_instance_id(process_instance.id.clone())
            .unwrap();
        assert_eq!(tasks.len(), 1, "sequential MI exposes one active task");

        let child_id = tasks[0].execution_id.clone();
        match &stable_child_id {
            None => stable_child_id = Some(child_id.clone()),
            Some(first) => assert_eq!(
                &child_id, first,
                "Java reuses the same sequential MI child execution id across rounds \
                 (round index {expected_index})"
            ),
        }

        assert_eq!(
            runtime
                .get_variable_local(child_id.clone(), "loopCounter".to_string())
                .unwrap(),
            Some(json!(expected_index as i64)),
            "loopCounter on the reused child"
        );
        assert_eq!(
            runtime
                .get_variable_local(child_id.clone(), "approverIndex".to_string())
                .unwrap(),
            Some(json!(expected_index as i64)),
            "element index on the reused child"
        );
        assert_eq!(
            runtime
                .get_variable_local(child_id.clone(), "approver".to_string())
                .unwrap(),
            Some(json!(expected_approver)),
            "element variable on the reused child"
        );

        task_service
            .complete_task_by_id(tasks[0].id.clone())
            .unwrap();
    }

    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let pi = store
        .find_process_instance(&process_instance.id, &mut session)
        .unwrap();
    assert!(pi.is_ended, "process ends after three sequential rounds");
}

/// Difference: while sequential rounds remain, the MI root has no pile of ended
/// sibling instance rows (Java never ends the reused child between rounds).
#[test]
fn sequential_mi_does_not_accumulate_ended_child_rows_between_rounds() {
    let engine = ProcessEngine::new("seq-mi-reuse-no-ended-pile".to_string());
    let definition_id = deploy(
        &engine,
        "seq_mi_reuse.bpmn20.xml",
        SEQUENTIAL_MI_THREE_ROUNDS_XML,
    );
    let mut variables = HashMap::new();
    variables.insert("approvers".to_string(), json!(["amy", "ben", "cy"]));
    let process_instance = start(&engine, definition_id, variables);
    let task_service = engine.get_task_service();

    for round in 0..3 {
        let tasks = task_service
            .get_tasks_by_process_instance_id(process_instance.id.clone())
            .unwrap();
        assert_eq!(tasks.len(), 1);

        let root = mi_root_id(&engine, &tasks[0].execution_id);
        let children = children_of(&engine, &root);
        let ended = children.iter().filter(|e| e.is_ended).count();
        let active = children.iter().filter(|e| !e.is_ended).count();

        assert_eq!(
            active, 1,
            "round {round}: exactly one non-ended sequential instance child"
        );
        assert_eq!(
            ended,
            0,
            "round {round}: no ended sibling rows accumulate under the MI root \
             while sequential instances remain (found {ended} ended of {} children)",
            children.len()
        );

        task_service
            .complete_task_by_id(tasks[0].id.clone())
            .unwrap();
    }
}

/// Regression guard (green before and after): sequential MI completionCondition
/// exits early and leaves the process on the post-MI activity.
#[test]
fn sequential_mi_completion_condition_exits_early() {
    let engine = ProcessEngine::new("seq-mi-completion-early".to_string());
    let definition_id = deploy(
        &engine,
        "seq_mi_completion.bpmn20.xml",
        SEQUENTIAL_MI_COMPLETION_CONDITION_XML,
    );
    let process_instance = start(&engine, definition_id, HashMap::new());
    let task_service = engine.get_task_service();

    // Round 1 of 5
    let tasks = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].task_definition_key, "miTask");
    task_service
        .complete_task_by_id(tasks[0].id.clone())
        .unwrap();

    // Round 2 of 5 → completionCondition (nrOfCompletedInstances >= 2) fires
    let tasks = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].task_definition_key, "miTask");
    task_service
        .complete_task_by_id(tasks[0].id.clone())
        .unwrap();

    let tasks = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(
        tasks.len(),
        1,
        "completion condition must leave the MI after 2 of 5 instances"
    );
    assert_eq!(
        tasks[0].task_definition_key, "afterMi",
        "process must continue to the post-MI activity"
    );
}

/// Regression guard (green before and after): sequential wait-state suspends
/// after each round and resumes on the next user-task complete until finished.
#[test]
fn sequential_mi_wait_state_suspends_and_resumes_all_rounds() {
    let engine = ProcessEngine::new("seq-mi-wait-state-resume".to_string());
    let definition_id = deploy(
        &engine,
        "seq_mi_reuse.bpmn20.xml",
        SEQUENTIAL_MI_THREE_ROUNDS_XML,
    );
    let mut variables = HashMap::new();
    variables.insert("approvers".to_string(), json!(["amy", "ben", "cy"]));
    let process_instance = start(&engine, definition_id, variables);
    let task_service = engine.get_task_service();

    for expected_round in 1..=3 {
        let tasks = task_service
            .get_tasks_by_process_instance_id(process_instance.id.clone())
            .unwrap();
        assert_eq!(
            tasks.len(),
            1,
            "wait-state round {expected_round}: exactly one active task"
        );
        assert_eq!(tasks[0].task_definition_key, "miTask");
        task_service
            .complete_task_by_id(tasks[0].id.clone())
            .unwrap();
    }

    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let pi = store
        .find_process_instance(&process_instance.id, &mut session)
        .unwrap();
    assert!(
        pi.is_ended,
        "after three wait-state resumes the process must end"
    );
}
