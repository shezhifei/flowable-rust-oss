//! P78 — ad-hoc subprocess auto-complete + cancelRemainingInstances.
//!
//! Java parity:
//! - `TakeOutgoingSequenceFlowsOperation.handleAdhocSubProcess` (:293-326)
//! - `AdhocSubProcessTest.testSimpleCompletionCondition`
//! - `AdhocSubProcessTest.testParallelAdhocSubProcess` (cancelRemaining default true)
//! - `AdhocSubProcessTest.testKeepRemainingInstancesAdhocSubProcess` (false)
//! - Explicit API `CompleteAdhocSubProcessCmd` still errors with running children

use flowable_engine::engine::process_engine::ProcessEngine;
use serde_json::json;
use std::collections::HashMap;

/// Sequential ad-hoc with completion condition (default cancelRemaining=true).
const COMPLETION_CONDITION_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
  <process id="simpleSubProcess" isExecutable="true">
    <startEvent id="theStart" />
    <sequenceFlow id="flow1" sourceRef="theStart" targetRef="adhocSubProcess" />
    <adHocSubProcess id="adhocSubProcess" ordering="Sequential">
      <userTask id="subProcessTask" name="Task in subprocess" />
      <userTask id="subProcessTask2" name="Task2 in subprocess" />
      <completionCondition>${completed}</completionCondition>
    </adHocSubProcess>
    <sequenceFlow id="flow2" sourceRef="adhocSubProcess" targetRef="afterTask" />
    <userTask id="afterTask" name="After task" />
    <sequenceFlow id="flow3" sourceRef="afterTask" targetRef="theEnd" />
    <endEvent id="theEnd" />
  </process>
</definitions>"#;

/// Parallel ad-hoc, cancelRemainingInstances default true + completion condition.
const PARALLEL_CANCEL_TRUE_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
  <process id="simpleSubProcess" isExecutable="true">
    <startEvent id="theStart" />
    <sequenceFlow id="flow1" sourceRef="theStart" targetRef="adhocSubProcess" />
    <adHocSubProcess id="adhocSubProcess" ordering="Parallel">
      <userTask id="subProcessTask" name="Task in subprocess" />
      <userTask id="subProcessTask2" name="Task2 in subprocess" />
      <completionCondition>${completed}</completionCondition>
    </adHocSubProcess>
    <sequenceFlow id="flow2" sourceRef="adhocSubProcess" targetRef="afterTask" />
    <userTask id="afterTask" name="After task" />
    <sequenceFlow id="flow3" sourceRef="afterTask" targetRef="theEnd" />
    <endEvent id="theEnd" />
  </process>
</definitions>"#;

/// Parallel ad-hoc with cancelRemainingInstances=false.
const KEEP_REMAINING_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
  <process id="simpleSubProcess" isExecutable="true">
    <startEvent id="theStart" />
    <sequenceFlow id="flow1" sourceRef="theStart" targetRef="adhocSubProcess" />
    <adHocSubProcess id="adhocSubProcess" cancelRemainingInstances="false" ordering="Parallel">
      <userTask id="subProcessTask" name="Task in subprocess" />
      <userTask id="subProcessTask2" name="Task2 in subprocess" />
      <completionCondition>${completed}</completionCondition>
    </adHocSubProcess>
    <sequenceFlow id="flow2" sourceRef="adhocSubProcess" targetRef="afterTask" />
    <userTask id="afterTask" name="After task" />
    <sequenceFlow id="flow3" sourceRef="afterTask" targetRef="theEnd" />
    <endEvent id="theEnd" />
  </process>
</definitions>"#;

/// Ad-hoc with an inner sequence flow (take-outgoing path, not leaf-only).
const FLOWS_INSIDE_ADHOC_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
  <process id="flowsAdhoc" isExecutable="true">
    <startEvent id="theStart" />
    <sequenceFlow id="flow1" sourceRef="theStart" targetRef="adhocSubProcess" />
    <adHocSubProcess id="adhocSubProcess" ordering="Parallel">
      <userTask id="subProcessTask" name="Task in subprocess" />
      <sequenceFlow id="innerFlow" sourceRef="subProcessTask" targetRef="nextTask" />
      <userTask id="nextTask" name="The next task" />
      <userTask id="subProcessTask2" name="Task2 in subprocess" />
      <completionCondition>${completed}</completionCondition>
    </adHocSubProcess>
    <sequenceFlow id="flow2" sourceRef="adhocSubProcess" targetRef="afterTask" />
    <userTask id="afterTask" name="After task" />
    <sequenceFlow id="flow3" sourceRef="afterTask" targetRef="theEnd" />
    <endEvent id="theEnd" />
  </process>
</definitions>"#;

fn start_with_completed_false(engine: &ProcessEngine, xml: &str, resource: &str) -> (String, String) {
    let repo = engine.get_repository_service();
    let runtime = engine.get_runtime_service();
    repo.deploy(
        repo.create_deployment()
            .add_string(resource.to_string(), xml.to_string()),
    )
    .unwrap();
    let def_id = repo.get_process_definition_ids().unwrap()[0].clone();
    let builder = runtime
        .create_process_instance_builder()
        .process_definition_id(def_id)
        .variable("completed".to_string(), json!(false));
    let pi = runtime.start_process_instance(builder).unwrap();
    let adhoc_id = runtime
        .get_adhoc_subprocess_executions(&pi.id)
        .unwrap()[0]
        .id
        .clone();
    (pi.id, adhoc_id)
}

/// completion condition true after normal task complete → adhoc ends, afterTask.
#[test]
fn p78_completion_condition_true_auto_ends_adhoc() {
    let engine = ProcessEngine::new("default".to_string());
    let runtime = engine.get_runtime_service();
    let task_svc = engine.get_task_service();
    let (pi_id, adhoc_id) =
        start_with_completed_false(&engine, COMPLETION_CONDITION_XML, "p78-cc.bpmn20.xml");

    runtime
        .execute_activity_in_adhoc_subprocess(&adhoc_id, "subProcessTask")
        .unwrap();
    let tasks = task_svc
        .get_tasks_by_process_instance_id(pi_id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].name, "Task in subprocess");

    // Condition still false → adhoc stays open.
    task_svc.complete_task_by_id(tasks[0].id.clone()).unwrap();
    assert_eq!(
        runtime.get_adhoc_subprocess_executions(&pi_id).unwrap().len(),
        1
    );

    runtime
        .execute_activity_in_adhoc_subprocess(&adhoc_id, "subProcessTask2")
        .unwrap();
    let tasks = task_svc
        .get_tasks_by_process_instance_id(pi_id.clone())
        .unwrap();
    assert_eq!(tasks[0].name, "Task2 in subprocess");

    let mut vars = HashMap::new();
    vars.insert("completed".to_string(), json!(true));
    task_svc
        .complete_task_by_id_with_variables(tasks[0].id.clone(), vars)
        .unwrap();

    let after = task_svc
        .get_tasks_by_process_instance_id(pi_id.clone())
        .unwrap();
    assert_eq!(after.len(), 1, "expected afterTask after auto-complete");
    assert_eq!(after[0].name, "After task");
    assert!(
        runtime
            .get_adhoc_subprocess_executions(&pi_id)
            .unwrap()
            .is_empty()
    );

    task_svc.complete_task_by_id(after[0].id.clone()).unwrap();
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let pi = store.find_process_instance(&pi_id, &mut session).unwrap();
    assert!(pi.is_ended);
}

/// cancelRemainingInstances=true (default): sibling tasks deleted when condition fires.
#[test]
fn p78_cancel_remaining_instances_true_deletes_siblings() {
    let engine = ProcessEngine::new("default".to_string());
    let runtime = engine.get_runtime_service();
    let task_svc = engine.get_task_service();
    let (pi_id, adhoc_id) =
        start_with_completed_false(&engine, PARALLEL_CANCEL_TRUE_XML, "p78-cancel-true.bpmn20.xml");

    runtime
        .execute_activity_in_adhoc_subprocess(&adhoc_id, "subProcessTask")
        .unwrap();
    runtime
        .execute_activity_in_adhoc_subprocess(&adhoc_id, "subProcessTask2")
        .unwrap();
    let tasks = task_svc
        .get_tasks_by_process_instance_id(pi_id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 2);

    let first = tasks
        .iter()
        .find(|t| t.task_definition_key == "subProcessTask")
        .expect("subProcessTask");
    let mut vars = HashMap::new();
    vars.insert("completed".to_string(), json!(true));
    task_svc
        .complete_task_by_id_with_variables(first.id.clone(), vars)
        .unwrap();

    let after = task_svc
        .get_tasks_by_process_instance_id(pi_id.clone())
        .unwrap();
    assert_eq!(
        after.len(),
        1,
        "sibling should be cancelled; only afterTask remains"
    );
    assert_eq!(after[0].name, "After task");
    assert!(
        !after
            .iter()
            .any(|t| t.task_definition_key == "subProcessTask2"),
        "sibling task2 must be deleted when cancelRemainingInstances=true"
    );
}

/// cancelRemainingInstances=false: siblings survive until all finished; then auto-end.
#[test]
fn p78_cancel_remaining_instances_false_waits_for_siblings() {
    let engine = ProcessEngine::new("default".to_string());
    let runtime = engine.get_runtime_service();
    let task_svc = engine.get_task_service();
    let (pi_id, adhoc_id) =
        start_with_completed_false(&engine, KEEP_REMAINING_XML, "p78-keep.bpmn20.xml");

    runtime
        .execute_activity_in_adhoc_subprocess(&adhoc_id, "subProcessTask")
        .unwrap();
    runtime
        .execute_activity_in_adhoc_subprocess(&adhoc_id, "subProcessTask2")
        .unwrap();
    let tasks = task_svc
        .get_tasks_by_process_instance_id(pi_id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 2);

    let first = tasks
        .iter()
        .find(|t| t.task_definition_key == "subProcessTask")
        .expect("subProcessTask");
    let mut vars = HashMap::new();
    vars.insert("completed".to_string(), json!(true));
    task_svc
        .complete_task_by_id_with_variables(first.id.clone(), vars)
        .unwrap();

    // Condition true but cancelRemaining=false → sibling lives; adhoc not ended.
    let remaining = task_svc
        .get_tasks_by_process_instance_id(pi_id.clone())
        .unwrap();
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].name, "Task2 in subprocess");
    assert_eq!(
        runtime.get_adhoc_subprocess_executions(&pi_id).unwrap().len(),
        1
    );

    task_svc
        .complete_task_by_id(remaining[0].id.clone())
        .unwrap();

    let after = task_svc
        .get_tasks_by_process_instance_id(pi_id.clone())
        .unwrap();
    assert_eq!(after.len(), 1);
    assert_eq!(after[0].name, "After task");
}

/// completion condition false: adhoc does not auto-end; siblings can still run.
#[test]
fn p78_completion_condition_false_keeps_adhoc_open() {
    let engine = ProcessEngine::new("default".to_string());
    let runtime = engine.get_runtime_service();
    let task_svc = engine.get_task_service();
    let (pi_id, adhoc_id) =
        start_with_completed_false(&engine, PARALLEL_CANCEL_TRUE_XML, "p78-cc-false.bpmn20.xml");

    runtime
        .execute_activity_in_adhoc_subprocess(&adhoc_id, "subProcessTask")
        .unwrap();
    runtime
        .execute_activity_in_adhoc_subprocess(&adhoc_id, "subProcessTask2")
        .unwrap();

    let tasks = task_svc
        .get_tasks_by_process_instance_id(pi_id.clone())
        .unwrap();
    let first = tasks
        .iter()
        .find(|t| t.task_definition_key == "subProcessTask")
        .expect("subProcessTask");
    // completed remains false
    task_svc.complete_task_by_id(first.id.clone()).unwrap();

    let remaining = task_svc
        .get_tasks_by_process_instance_id(pi_id.clone())
        .unwrap();
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].task_definition_key, "subProcessTask2");
    assert_eq!(
        runtime.get_adhoc_subprocess_executions(&pi_id).unwrap().len(),
        1,
        "adhoc must stay open when completion condition is false"
    );

    // Sibling can still be completed without auto-ending (condition still false).
    task_svc
        .complete_task_by_id(remaining[0].id.clone())
        .unwrap();
    assert_eq!(
        runtime.get_adhoc_subprocess_executions(&pi_id).unwrap().len(),
        1
    );
    // Explicit complete API still required when condition never fires.
    runtime.complete_adhoc_subprocess(&adhoc_id).unwrap();
    let after = task_svc
        .get_tasks_by_process_instance_id(pi_id.clone())
        .unwrap();
    assert_eq!(after[0].name, "After task");
}

/// Take-outgoing path: completing a task with an outgoing flow evaluates the
/// adhoc completion condition (not only the leaf/no-outgoing shortcut).
#[test]
fn p78_take_outgoing_path_evaluates_completion_condition() {
    let engine = ProcessEngine::new("default".to_string());
    let runtime = engine.get_runtime_service();
    let task_svc = engine.get_task_service();
    let (pi_id, adhoc_id) =
        start_with_completed_false(&engine, FLOWS_INSIDE_ADHOC_XML, "p78-flows.bpmn20.xml");

    runtime
        .execute_activity_in_adhoc_subprocess(&adhoc_id, "subProcessTask")
        .unwrap();
    runtime
        .execute_activity_in_adhoc_subprocess(&adhoc_id, "subProcessTask2")
        .unwrap();

    let tasks = task_svc
        .get_tasks_by_process_instance_id(pi_id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 2);

    let first = tasks
        .iter()
        .find(|t| t.task_definition_key == "subProcessTask")
        .expect("subProcessTask");
    let mut vars = HashMap::new();
    vars.insert("completed".to_string(), json!(true));
    // Completing subProcessTask takes inner outgoing → nextTask; condition true
    // + cancelRemaining default true → cancel siblings and end adhoc.
    task_svc
        .complete_task_by_id_with_variables(first.id.clone(), vars)
        .unwrap();

    let after = task_svc
        .get_tasks_by_process_instance_id(pi_id.clone())
        .unwrap();
    assert_eq!(
        after.len(),
        1,
        "expected afterTask; got {:?}",
        after
            .iter()
            .map(|t| t.task_definition_key.as_str())
            .collect::<Vec<_>>()
    );
    assert_eq!(after[0].name, "After task");
}

/// Explicit completeAdhocSubProcess still errors when children are running
/// (Java CompleteAdhocSubProcessCmd.java:53-56 — not cancelRemaining).
#[test]
fn p78_explicit_complete_api_errors_with_running_children() {
    let engine = ProcessEngine::new("default".to_string());
    let runtime = engine.get_runtime_service();
    let (_pi_id, adhoc_id) =
        start_with_completed_false(&engine, PARALLEL_CANCEL_TRUE_XML, "p78-api.bpmn20.xml");

    runtime
        .execute_activity_in_adhoc_subprocess(&adhoc_id, "subProcessTask")
        .unwrap();

    let err = runtime.complete_adhoc_subprocess(&adhoc_id).unwrap_err();
    let msg = format!("{:?}", err);
    assert!(
        msg.contains("running child") || msg.contains("completed first"),
        "unexpected error: {}",
        msg
    );
}
