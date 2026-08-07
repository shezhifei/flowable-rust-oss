//! Contract tests for Java **SubProcess sequential multi-instance** topology.
//!
//! Java (`SequentialMultiInstanceBehavior#continueSequentialMultiInstance`,
//! lines 106–124) treats SubProcess differently from userTask:
//!
//! | Aspect | userTask sequential MI | SubProcess sequential MI |
//! |---|---|---|
//! | Round N child | **reuse** same instance child | **new** child under MI root |
//! | `is_scope` on instance child | false (typically) | **true** (scope row) |
//! | Between rounds | clear locals on same id | DestroyScope old + createChild new |
//! | Nested wait state | task on instance child | task under scope child |
//! | Leave (final) | cleanupMiRoot | cleanupMiRoot (same) |
//!
//! Observable gaps locked here (P6-A only covered userTask reuse):
//!   1. Each sequential SubProcess round uses a distinct scope child id.
//!   2. Instance children under the MI root are `is_scope == true`.
//!   3. Nested userTask hangs under the scope child (not directly under MI root).
//!   4. Element / loopCounter locals live on the scope child and advance per round.
//!   5. After final leave, MI root is gone and process continues on afterMi.
//!
//! Regression guards: userTask sequential MI reuse (P6-A) and MI-root materialize
//! (P7) stay green via their own test binaries — this file only adds SubProcess
//! probes plus a light userTask reuse guard.

use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::runtime::execution::Execution;
use flowable_engine::runtime::process_instance::ProcessInstance;
use serde_json::json;
use std::collections::HashMap;

const SUBPROCESS_SEQUENTIAL_MI_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
    <process id="subSeqMiTopology" isExecutable="true">
        <startEvent id="start" />
        <sequenceFlow id="f1" sourceRef="start" targetRef="miSub" />
        <subProcess id="miSub" name="MI SubProcess">
            <multiInstanceLoopCharacteristics isSequential="true"
                                              flowable:collection="approvers"
                                              flowable:elementVariable="approver"
                                              flowable:elementIndexVariable="approverIndex" />
            <startEvent id="subStart" />
            <sequenceFlow id="sf1" sourceRef="subStart" targetRef="innerTask" />
            <userTask id="innerTask" name="Inner Task" />
            <sequenceFlow id="sf2" sourceRef="innerTask" targetRef="subEnd" />
            <endEvent id="subEnd" />
        </subProcess>
        <sequenceFlow id="f2" sourceRef="miSub" targetRef="afterMi" />
        <userTask id="afterMi" name="After MI" />
        <sequenceFlow id="f3" sourceRef="afterMi" targetRef="end" />
        <endEvent id="end" />
    </process>
</definitions>"#;

/// Regression guard XML: plain userTask sequential MI (P6-A reuse contract).
const USERTASK_SEQUENTIAL_MI_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
    <process id="userTaskSeqMiGuard" isExecutable="true">
        <startEvent id="start" />
        <sequenceFlow id="f1" sourceRef="start" targetRef="miTask" />
        <userTask id="miTask" name="MI Task">
            <multiInstanceLoopCharacteristics isSequential="true"
                                              flowable:collection="approvers"
                                              flowable:elementVariable="approver" />
        </userTask>
        <sequenceFlow id="f2" sourceRef="miTask" targetRef="end" />
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

fn all_executions(engine: &ProcessEngine, process_instance_id: &str) -> Vec<Execution> {
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    store
        .snapshot_executions(&mut session)
        .into_values()
        .filter(|e| e.process_instance_id.as_deref() == Some(process_instance_id))
        .collect()
}

fn children_of<'a>(executions: &'a [Execution], parent_id: &str) -> Vec<&'a Execution> {
    executions
        .iter()
        .filter(|e| e.parent_id.as_deref() == Some(parent_id))
        .collect()
}

fn find_mi_roots(executions: &[Execution]) -> Vec<&Execution> {
    executions
        .iter()
        .filter(|e| e.is_multi_instance_root)
        .collect()
}

/// Difference: each sequential SubProcess round uses a **new** scope child under
/// the MI root (Java `continueSequentialMultiInstance` SubProcess branch).
#[test]
fn subprocess_sequential_mi_creates_new_scope_child_each_round() {
    let engine = ProcessEngine::new("sub-seq-mi-new-scope-each-round".to_string());
    let definition_id = deploy(
        &engine,
        "sub_seq_mi.bpmn20.xml",
        SUBPROCESS_SEQUENTIAL_MI_XML,
    );
    let mut variables = HashMap::new();
    variables.insert("approvers".to_string(), json!(["amy", "ben"]));
    let process_instance = start(&engine, definition_id, variables);
    let task_service = engine.get_task_service();
    let runtime = engine.get_runtime_service();

    let mut previous_scope_id: Option<String> = None;

    for (expected_index, expected_approver) in ["amy", "ben"].into_iter().enumerate() {
        let tasks = task_service
            .get_tasks_by_process_instance_id(process_instance.id.clone())
            .unwrap();
        assert_eq!(
            tasks.len(),
            1,
            "round {expected_index}: sequential SubProcess MI exposes one active task"
        );
        assert_eq!(tasks[0].task_definition_key, "innerTask");

        let executions = all_executions(&engine, &process_instance.id);
        let mi_roots = find_mi_roots(&executions);
        assert_eq!(
            mi_roots.len(),
            1,
            "round {expected_index}: dedicated MI root must exist"
        );
        let mi_root = mi_roots[0];
        assert!(!mi_root.is_active);
        assert_eq!(mi_root.activity_id.as_deref(), Some("miSub"));

        // Task hangs under the SubProcess scope child, not directly under MI root.
        let task_exec = executions
            .iter()
            .find(|e| e.id == tasks[0].execution_id)
            .expect("task execution row");
        let scope_id = task_exec
            .parent_id
            .as_deref()
            .expect("inner task must hang under SubProcess scope child");
        assert_ne!(
            scope_id, mi_root.id,
            "round {expected_index}: inner task must not hang directly under MI root"
        );
        assert_eq!(
            task_exec
                .parent_id
                .as_ref()
                .and_then(|pid| executions.iter().find(|e| e.id == *pid))
                .and_then(|scope| scope.parent_id.as_deref()),
            Some(mi_root.id.as_str()),
            "round {expected_index}: scope child's parent must be the MI root"
        );

        let scope = executions
            .iter()
            .find(|e| e.id == scope_id)
            .expect("scope child row");
        assert!(
            scope.is_scope,
            "round {expected_index}: SubProcess MI instance child must be is_scope=true \
             (Java setScope(true) / SubProcessActivityBehavior.execute)"
        );
        assert!(
            !scope.is_multi_instance_root,
            "round {expected_index}: instance child is not the MI root"
        );
        assert!(
            !scope.is_ended,
            "round {expected_index}: active scope child must not be ended"
        );

        // Distinct id each round (no reuse).
        match &previous_scope_id {
            None => previous_scope_id = Some(scope.id.clone()),
            Some(prev) => assert_ne!(
                &scope.id, prev,
                "Java creates a new SubProcess scope child each sequential round \
                 (round index {expected_index}); id must change"
            ),
        }

        // P5-B local semantics: loopCounter / element live on the scope child.
        assert_eq!(
            runtime
                .get_variable_local(scope.id.clone(), "loopCounter".to_string())
                .unwrap(),
            Some(json!(expected_index as i64)),
            "loopCounter local on SubProcess scope child"
        );
        assert_eq!(
            runtime
                .get_variable_local(scope.id.clone(), "approverIndex".to_string())
                .unwrap(),
            Some(json!(expected_index as i64)),
            "element index local on SubProcess scope child"
        );
        assert_eq!(
            runtime
                .get_variable_local(scope.id.clone(), "approver".to_string())
                .unwrap(),
            Some(json!(expected_approver)),
            "element variable local on SubProcess scope child"
        );

        // No pile of live scope siblings under the MI root while one round is active.
        let mi_children = children_of(&executions, &mi_root.id);
        let live_scopes = mi_children
            .iter()
            .filter(|e| e.is_scope && !e.is_ended)
            .count();
        assert_eq!(
            live_scopes, 1,
            "round {expected_index}: exactly one live SubProcess scope under MI root"
        );

        task_service
            .complete_task_by_id(tasks[0].id.clone())
            .unwrap();
    }

    // After both rounds: cleanupMiRoot → afterMi, no MI root.
    let after_tasks = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(
        after_tasks.len(),
        1,
        "after two sequential SubProcess rounds the process continues to afterMi"
    );
    assert_eq!(after_tasks[0].task_definition_key, "afterMi");

    let executions = all_executions(&engine, &process_instance.id);
    assert!(
        find_mi_roots(&executions).is_empty(),
        "cleanupMiRoot must delete the MI root on final SubProcess sequential leave"
    );
    let leave_exec = executions
        .iter()
        .find(|e| e.id == after_tasks[0].execution_id)
        .expect("afterMi execution");
    assert!(
        !leave_exec.is_multi_instance_root,
        "leave execution must not carry the MI root flag"
    );
}

/// Difference: after round-1 complete and before round-2 wait, the previous
/// SubProcess scope child must not remain as a live scope sibling under the MI root
/// (Java DestroyScope + createChild).
#[test]
fn subprocess_sequential_mi_destroys_prior_scope_before_next_round() {
    let engine = ProcessEngine::new("sub-seq-mi-destroy-prior-scope".to_string());
    let definition_id = deploy(
        &engine,
        "sub_seq_mi.bpmn20.xml",
        SUBPROCESS_SEQUENTIAL_MI_XML,
    );
    let mut variables = HashMap::new();
    variables.insert("approvers".to_string(), json!(["amy", "ben"]));
    let process_instance = start(&engine, definition_id, variables);
    let task_service = engine.get_task_service();

    let tasks = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 1);
    let round1_task_exec = tasks[0].execution_id.clone();
    let executions = all_executions(&engine, &process_instance.id);
    let round1_scope_id = executions
        .iter()
        .find(|e| e.id == round1_task_exec)
        .and_then(|e| e.parent_id.clone())
        .expect("round-1 scope id");

    task_service
        .complete_task_by_id(tasks[0].id.clone())
        .unwrap();

    // Round 2 active.
    let tasks = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].task_definition_key, "innerTask");

    let executions = all_executions(&engine, &process_instance.id);
    // Prior scope row is gone (or ended and not a live sibling).
    let prior_still_live = executions
        .iter()
        .any(|e| e.id == round1_scope_id && e.is_scope && !e.is_ended);
    assert!(
        !prior_still_live,
        "Java DestroyScope removes the completed SubProcess scope before the next round; \
         prior scope {round1_scope_id} must not remain live"
    );

    let mi_roots = find_mi_roots(&executions);
    assert_eq!(mi_roots.len(), 1);
    let live_scopes = children_of(&executions, &mi_roots[0].id)
        .into_iter()
        .filter(|e| e.is_scope && !e.is_ended)
        .count();
    assert_eq!(
        live_scopes, 1,
        "only the current SubProcess scope should be live under the MI root"
    );
}

/// Regression guard (green before and after): userTask sequential MI still reuses
/// the same child id across rounds (P6-A contract must not regress).
#[test]
fn regression_usertask_sequential_mi_still_reuses_child_id() {
    let engine = ProcessEngine::new("sub-seq-mi-usertask-reuse-guard".to_string());
    let definition_id = deploy(
        &engine,
        "user_task_seq_mi.bpmn20.xml",
        USERTASK_SEQUENTIAL_MI_XML,
    );
    let mut variables = HashMap::new();
    variables.insert("approvers".to_string(), json!(["amy", "ben"]));
    let process_instance = start(&engine, definition_id, variables);
    let task_service = engine.get_task_service();

    let mut stable_child_id: Option<String> = None;
    for expected_index in 0..2 {
        let tasks = task_service
            .get_tasks_by_process_instance_id(process_instance.id.clone())
            .unwrap();
        assert_eq!(tasks.len(), 1);
        let child_id = tasks[0].execution_id.clone();
        match &stable_child_id {
            None => stable_child_id = Some(child_id),
            Some(first) => assert_eq!(
                &child_id, first,
                "P6-A: userTask sequential MI reuses the same child id \
                 (round {expected_index})"
            ),
        }
        task_service
            .complete_task_by_id(tasks[0].id.clone())
            .unwrap();
    }
}
