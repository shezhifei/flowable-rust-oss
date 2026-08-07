//! P45: transient variable lifecycle — Java VariableScopeImpl parity.
//!
//! Java: `VariableScopeImpl.transientVariables` is pure memory (line 58); not
//! written to ACT_RU_VARIABLE and discarded when the command/transaction ends.
//!
//! Rust historically serialized `Execution.transient_variables` into the
//! execution JSON (P21 removed `skip_serializing` so call-activity same-command
//! reloads could still see them). That leaked across commands: stale reads,
//! REST phantom variables, durable writes shadowed by leftover transient.
//!
//! Fix: keep mid-command serialization for same-command reloads; strip on
//! commit via `RuntimeStore::strip_transient_variables_before_commit`.

use flowable_engine::engine::process_engine::ProcessEngine;
use serde_json::json;

const ONE_TASK_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
  <process id="p45OneTask" isExecutable="true">
    <startEvent id="start" />
    <sequenceFlow id="f1" sourceRef="start" targetRef="task1" />
    <userTask id="task1" name="Task 1" />
    <sequenceFlow id="f2" sourceRef="task1" targetRef="end" />
    <endEvent id="end" />
  </process>
</definitions>"#;

fn deploy_one_task(engine: &ProcessEngine) {
    let repo = engine.get_repository_service();
    repo.deploy(
        repo.create_deployment()
            .name("p45-one-task".into())
            .add_string("p45.bpmn20.xml".into(), ONE_TASK_XML.to_string()),
    )
    .unwrap();
}

/// Cross-command: start-time transient must vanish after the start command commits.
#[test]
fn transient_start_variable_invisible_after_command_commits() {
    let engine = ProcessEngine::new("p45-cross-cmd-start".into());
    deploy_one_task(&engine);
    let runtime = engine.get_runtime_service();

    let pi = runtime
        .start_process_instance(
            runtime
                .create_process_instance_builder()
                .process_definition_key("p45OneTask".into())
                .variable("durable".into(), json!("keep"))
                .transient_variable("ghost".into(), json!("should-vanish")),
        )
        .unwrap();

    assert_eq!(
        runtime
            .get_variable(pi.id.clone(), "durable".into())
            .unwrap(),
        Some(json!("keep")),
        "durable variables survive the start command"
    );
    assert_eq!(
        runtime
            .get_variable(pi.id.clone(), "ghost".into())
            .unwrap(),
        None,
        "start-time transient must not be readable after the start command commits"
    );

    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let root = store
        .find_execution(&pi.id, &mut session)
        .expect("root execution");
    assert!(
        root.transient_variables.is_empty(),
        "execution JSON must not retain transient_variables after commit"
    );
    assert!(
        !root.variables.contains_key("ghost"),
        "transient must not have been promoted to durable variables"
    );
}

/// Cross-command: durable write after a same-named start-time transient must not
/// be permanently shadowed by leftover transient (pre-P45 bug).
#[test]
fn durable_write_not_shadowed_by_stale_transient() {
    let engine = ProcessEngine::new("p45-shadow-guard".into());
    deploy_one_task(&engine);
    let runtime = engine.get_runtime_service();
    let variables = engine.get_variable_service();

    let pi = runtime
        .start_process_instance(
            runtime
                .create_process_instance_builder()
                .process_definition_key("p45OneTask".into())
                .transient_variable("shared".into(), json!("from-transient")),
        )
        .unwrap();

    // After start, transient is gone — a durable write must stick.
    assert_eq!(
        variables
            .get_variable(pi.id.clone(), "shared".into())
            .unwrap(),
        None,
        "precondition: stale transient must already be gone before durable write"
    );

    variables
        .set_variable(pi.id.clone(), "shared".into(), json!("from-durable"))
        .unwrap();

    assert_eq!(
        variables
            .get_variable(pi.id.clone(), "shared".into())
            .unwrap(),
        Some(json!("from-durable")),
        "durable write must win; leftover transient must not shadow it"
    );
    let all = variables.get_variables(pi.id.clone()).unwrap();
    assert_eq!(
        all.get("shared"),
        Some(&json!("from-durable")),
        "get_variables must agree with get_variable for the durable value"
    );
}

/// Call-activity inheritVariables: durable inherited; transient visible mid-command
/// on the child (gateway), not promoted to durable, gone after commit.
#[test]
fn call_activity_inherit_variables_transient_split_and_strip() {
    let engine = ProcessEngine::new("p45-ca-inherit".into());
    let repo = engine.get_repository_service();
    let runtime = engine.get_runtime_service();
    let task_service = engine.get_task_service();

    let parent_xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
  <process id="p45ParentInherit" isExecutable="true">
    <startEvent id="start" />
    <sequenceFlow id="f1" sourceRef="start" targetRef="call" />
    <callActivity id="call" calledElement="p45ChildInherit"
                  flowable:inheritVariables="true" />
    <sequenceFlow id="f2" sourceRef="call" targetRef="end" />
    <endEvent id="end" />
  </process>
</definitions>"#;

    let child_xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
  <process id="p45ChildInherit" isExecutable="true">
    <startEvent id="childStart" />
    <sequenceFlow id="c1" sourceRef="childStart" targetRef="gw" />
    <exclusiveGateway id="gw" default="toFail" />
    <sequenceFlow id="toOk" sourceRef="gw" targetRef="okTask">
      <conditionExpression><![CDATA[${tVar == 'tv'}]]></conditionExpression>
    </sequenceFlow>
    <sequenceFlow id="toFail" sourceRef="gw" targetRef="failTask" />
    <userTask id="okTask" name="okTask" />
    <userTask id="failTask" name="failTask" />
    <sequenceFlow id="c2" sourceRef="okTask" targetRef="childEnd" />
    <sequenceFlow id="c3" sourceRef="failTask" targetRef="childEnd" />
    <endEvent id="childEnd" />
  </process>
</definitions>"#;

    repo.deploy(
        repo.create_deployment()
            .name("p45-inherit".into())
            .add_string("parent.bpmn20.xml".into(), parent_xml.to_string())
            .add_string("child.bpmn20.xml".into(), child_xml.to_string()),
    )
    .unwrap();

    let parent = runtime
        .start_process_instance(
            runtime
                .create_process_instance_builder()
                .process_definition_key("p45ParentInherit".into())
                .variable("dVar".into(), json!("dv"))
                .transient_variable("tVar".into(), json!("tv")),
        )
        .unwrap();

    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let child = store
        .snapshot_process_instances(&mut session)
        .into_values()
        .find(|pi| {
            pi.super_execution_id
                .as_deref()
                .is_some_and(|id| id.starts_with(parent.id.as_str()))
        })
        .expect("child PI");

    let tasks = task_service
        .get_tasks_by_process_instance_id(child.id.clone())
        .unwrap();
    assert_eq!(
        tasks[0].name.as_str(),
        "okTask",
        "child must see inherited transient mid-command"
    );

    let child_root = store
        .find_execution(&child.id, &mut session)
        .expect("child root");
    assert_eq!(child_root.variables.get("dVar"), Some(&json!("dv")));
    assert!(
        !child_root.variables.contains_key("tVar"),
        "inherited transient must not become durable"
    );
    assert!(
        !child_root.transient_variables.contains_key("tVar"),
        "inherited transient must be stripped after commit"
    );
}

/// Call-activity out parameter with transient="true": mid-command routing sees
/// it; after commit it is gone and never durable.
#[test]
fn call_activity_out_parameter_transient_mid_command_only() {
    let engine = ProcessEngine::new("p45-ca-out-transient".into());
    let repo = engine.get_repository_service();
    let runtime = engine.get_runtime_service();
    let task_service = engine.get_task_service();
    let variables = engine.get_variable_service();

    let parent_xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
  <process id="p45ParentOut" isExecutable="true">
    <startEvent id="start" />
    <sequenceFlow id="f1" sourceRef="start" targetRef="call" />
    <callActivity id="call" calledElement="p45ChildOut">
      <extensionElements>
        <flowable:out source="childResult" target="outT" transient="true" />
      </extensionElements>
    </callActivity>
    <sequenceFlow id="f2" sourceRef="call" targetRef="gw" />
    <exclusiveGateway id="gw" default="toFail" />
    <sequenceFlow id="toOk" sourceRef="gw" targetRef="outerOk">
      <conditionExpression><![CDATA[${outT == 'done'}]]></conditionExpression>
    </sequenceFlow>
    <sequenceFlow id="toFail" sourceRef="gw" targetRef="outerFail" />
    <userTask id="outerOk" name="outerOk" />
    <userTask id="outerFail" name="outerFail" />
    <sequenceFlow id="f3" sourceRef="outerOk" targetRef="end" />
    <sequenceFlow id="f4" sourceRef="outerFail" targetRef="end" />
    <endEvent id="end" />
  </process>
</definitions>"#;

    let child_xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
  <process id="p45ChildOut" isExecutable="true">
    <startEvent id="childStart" />
    <sequenceFlow id="c1" sourceRef="childStart" targetRef="childTask" />
    <userTask id="childTask" name="childTask" />
    <sequenceFlow id="c2" sourceRef="childTask" targetRef="childEnd" />
    <endEvent id="childEnd" />
  </process>
</definitions>"#;

    repo.deploy(
        repo.create_deployment()
            .name("p45-out".into())
            .add_string("parent.bpmn20.xml".into(), parent_xml.to_string())
            .add_string("child.bpmn20.xml".into(), child_xml.to_string()),
    )
    .unwrap();

    let parent = runtime
        .start_process_instance(
            runtime
                .create_process_instance_builder()
                .process_definition_key("p45ParentOut".into()),
        )
        .unwrap();

    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let child = store
        .snapshot_process_instances(&mut session)
        .into_values()
        .find(|pi| {
            pi.super_execution_id
                .as_deref()
                .is_some_and(|id| id.starts_with(parent.id.as_str()))
        })
        .expect("child PI");

    variables
        .set_variable(child.id.clone(), "childResult".into(), json!("done"))
        .unwrap();
    let child_tasks = task_service
        .get_tasks_by_process_instance_id(child.id)
        .unwrap();
    task_service
        .complete_task_by_id(child_tasks[0].id.clone())
        .unwrap();

    let parent_tasks = task_service
        .get_tasks_by_process_instance_id(parent.id.clone())
        .unwrap();
    assert_eq!(
        parent_tasks[0].name.as_str(),
        "outerOk",
        "transient out must drive mid-command gateway routing"
    );
    assert_eq!(
        variables
            .get_variable(parent.id.clone(), "outT".into())
            .unwrap(),
        None,
        "transient out must not survive the complete-task command"
    );
}
