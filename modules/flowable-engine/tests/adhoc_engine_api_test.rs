//! P24 sub-item 2: ad-hoc subprocess engine APIs.
//! Java parity: AdhocSubProcessTest.testSimpleAdhocSubProcess /
//! testSimpleAdhocSubProcessViaExecution.

use flowable_engine::engine::process_engine::ProcessEngine;

const SIMPLE_ADHOC_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
  <process id="simpleSubProcess" isExecutable="true">
    <startEvent id="theStart" />
    <sequenceFlow id="flow1" sourceRef="theStart" targetRef="adhocSubProcess" />
    <adHocSubProcess id="adhocSubProcess" ordering="Parallel">
      <userTask id="subProcessTask" name="Task in subprocess" />
      <userTask id="subProcessTask2" name="Task2 in subprocess" />
    </adHocSubProcess>
    <sequenceFlow id="flow2" sourceRef="adhocSubProcess" targetRef="afterTask" />
    <userTask id="afterTask" name="After task" />
    <sequenceFlow id="flow3" sourceRef="afterTask" targetRef="theEnd" />
    <endEvent id="theEnd" />
  </process>
</definitions>"#;

const SEQUENTIAL_ADHOC_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
  <process id="seqAdhoc" isExecutable="true">
    <startEvent id="theStart" />
    <sequenceFlow id="flow1" sourceRef="theStart" targetRef="adhocSubProcess" />
    <adHocSubProcess id="adhocSubProcess" ordering="Sequential">
      <userTask id="subProcessTask" name="Task in subprocess" />
      <userTask id="subProcessTask2" name="Task2 in subprocess" />
    </adHocSubProcess>
    <sequenceFlow id="flow2" sourceRef="adhocSubProcess" targetRef="theEnd" />
    <endEvent id="theEnd" />
  </process>
</definitions>"#;

#[test]
fn test_simple_adhoc_subprocess_engine_api() {
    let engine = ProcessEngine::new("default".to_string());
    let repo = engine.get_repository_service();
    let runtime = engine.get_runtime_service();
    let task_svc = engine.get_task_service();

    repo.deploy(
        repo.create_deployment()
            .add_string("simple-adhoc.bpmn20.xml".to_string(), SIMPLE_ADHOC_XML.to_string()),
    )
    .unwrap();

    let def_id = repo.get_process_definition_ids().unwrap()[0].clone();
    let pi = runtime.start_process_instance_by_id(def_id, None).unwrap();

    let adhoc_execs = runtime
        .get_adhoc_subprocess_executions(&pi.id)
        .unwrap();
    assert_eq!(adhoc_execs.len(), 1);
    let adhoc_id = adhoc_execs[0].id.clone();
    assert_eq!(
        adhoc_execs[0].activity_id.as_deref(),
        Some("adhocSubProcess")
    );

    let enabled = runtime
        .get_enabled_activities_from_adhoc_subprocess(&adhoc_id)
        .unwrap();
    assert_eq!(enabled.len(), 2);
    let ids: Vec<_> = enabled.iter().map(|a| a.id.as_str()).collect();
    assert!(ids.contains(&"subProcessTask"));
    assert!(ids.contains(&"subProcessTask2"));

    let child = runtime
        .execute_activity_in_adhoc_subprocess(&adhoc_id, "subProcessTask")
        .unwrap();
    assert!(child.id.len() > 0);
    assert_eq!(child.activity_id.as_deref(), Some("subProcessTask"));

    let tasks = task_svc
        .get_tasks_by_process_instance_id(pi.id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].name, "Task in subprocess");
    task_svc.complete_task_by_id(tasks[0].id.clone()).unwrap();

    // Still enabled after complete (parallel).
    let enabled = runtime
        .get_enabled_activities_from_adhoc_subprocess(&adhoc_id)
        .unwrap();
    assert_eq!(enabled.len(), 2);

    runtime.complete_adhoc_subprocess(&adhoc_id).unwrap();

    let after = task_svc
        .get_tasks_by_process_instance_id(pi.id.clone())
        .unwrap();
    assert_eq!(after.len(), 1);
    assert_eq!(after[0].name, "After task");
    task_svc.complete_task_by_id(after[0].id.clone()).unwrap();

    assert!(
        runtime
            .get_adhoc_subprocess_executions(&pi.id)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn test_sequential_adhoc_blocks_second_execute() {
    let engine = ProcessEngine::new("default".to_string());
    let repo = engine.get_repository_service();
    let runtime = engine.get_runtime_service();

    repo.deploy(
        repo.create_deployment()
            .add_string(
                "seq-adhoc.bpmn20.xml".to_string(),
                SEQUENTIAL_ADHOC_XML.to_string(),
            ),
    )
    .unwrap();

    let def_id = repo.get_process_definition_ids().unwrap()[0].clone();
    let pi = runtime.start_process_instance_by_id(def_id, None).unwrap();
    let adhoc_id = runtime
        .get_adhoc_subprocess_executions(&pi.id)
        .unwrap()[0]
        .id
        .clone();

    let enabled = runtime
        .get_enabled_activities_from_adhoc_subprocess(&adhoc_id)
        .unwrap();
    assert_eq!(enabled.len(), 2);

    runtime
        .execute_activity_in_adhoc_subprocess(&adhoc_id, "subProcessTask")
        .unwrap();

    // Sequential: no enabled while a child is active.
    let enabled = runtime
        .get_enabled_activities_from_adhoc_subprocess(&adhoc_id)
        .unwrap();
    assert!(enabled.is_empty());

    let err = runtime
        .execute_activity_in_adhoc_subprocess(&adhoc_id, "subProcessTask2")
        .unwrap_err();
    let msg = format!("{:?}", err);
    assert!(
        msg.contains("Sequential") || msg.contains("active"),
        "unexpected error: {}",
        msg
    );
}
