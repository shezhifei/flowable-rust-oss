//! P21: Call activity attribute wiring (parsed but unread / wrong-priority).
//!
//! Java evidence lives in
//! `CallActivityBehavior.java` / `IOParameterUtil.java` / `EndExecutionOperation.java`
//! and the CallActivity* tests cited per case below.

use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::engine::query::Query;
use flowable_engine::persistence::runtime_store::job_handler_types;
use flowable_engine::runtime::process_instance::ProcessInstanceUpdate;
use flowable_engine::service::config::ProcessEngineConfiguration;
use serde_json::json;

fn child_user_task_xml(process_id: &str, task_id: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
  <process id="{process_id}" isExecutable="true">
    <startEvent id="childStart" />
    <sequenceFlow id="childFlow1" sourceRef="childStart" targetRef="{task_id}" />
    <userTask id="{task_id}" name="{task_id}" />
    <sequenceFlow id="childFlow2" sourceRef="{task_id}" targetRef="childEnd" />
    <endEvent id="childEnd" />
  </process>
</definitions>"#
    )
}

fn child_auto_complete_xml(process_id: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
  <process id="{process_id}" isExecutable="true">
    <startEvent id="childStart" />
    <sequenceFlow id="childFlow1" sourceRef="childStart" targetRef="childEnd" />
    <endEvent id="childEnd" />
  </process>
</definitions>"#
    )
}

fn find_child_pi(
    engine: &ProcessEngine,
    parent_id: &str,
) -> flowable_engine::runtime::process_instance::ProcessInstance {
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    store
        .snapshot_process_instances(&mut session)
        .into_values()
        .find(|pi| {
            pi.super_execution_id
                .as_deref()
                .is_some_and(|id| id.starts_with(parent_id))
        })
        .expect("child process instance should exist")
}

// ─── 1. calledElementType="id" ───────────────────────────────────────────────
// Java CallActivityBehavior:242-249,287-290; CallActivityWithElementType.java

#[test]
fn p21_called_element_type_id_resolves_by_definition_id() {
    let engine = ProcessEngine::new("p21-element-type-id".into());
    let repo = engine.get_repository_service();
    let runtime = engine.get_runtime_service();
    let task_service = engine.get_task_service();

    repo.deploy(
        repo.create_deployment()
            .name("child".into())
            .add_string(
                "child.bpmn20.xml".into(),
                child_user_task_xml("childById", "childTask"),
            ),
    )
    .unwrap();

    let child_def = repo
        .latest_process_definition_by_key("childById", None)
        .unwrap()
        .expect("child definition");

    let parent_xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
  <process id="parentById" isExecutable="true">
    <startEvent id="start" />
    <sequenceFlow id="f1" sourceRef="start" targetRef="call" />
    <callActivity id="call" calledElement="{}" flowable:calledElementType="id" />
    <sequenceFlow id="f2" sourceRef="call" targetRef="outer" />
    <userTask id="outer" />
    <sequenceFlow id="f3" sourceRef="outer" targetRef="end" />
    <endEvent id="end" />
  </process>
</definitions>"#,
        child_def.id
    );

    repo.deploy(
        repo.create_deployment()
            .name("parent".into())
            .add_string("parent.bpmn20.xml".into(), parent_xml),
    )
    .unwrap();

    let parent = runtime
        .start_process_instance(
            runtime
                .create_process_instance_builder()
                .process_definition_key("parentById".into()),
        )
        .unwrap();

    let child = find_child_pi(&engine, &parent.id);
    assert_eq!(
        child.process_definition_id, child_def.id,
        "calledElementType=id must resolve by process definition id"
    );

    let tasks = task_service
        .get_tasks_by_process_instance_id(child.id)
        .unwrap();
    assert_eq!(tasks[0].task_definition_key, "childTask");
}

// ─── 2. fallbackToDefaultTenant ──────────────────────────────────────────────
// Java CallActivityBehavior:316-325; CallActivityAdvancedTest:1104-1346

#[test]
fn p21_fallback_to_default_tenant_resolves_global_definition() {
    let engine = ProcessEngine::new("p21-fallback-tenant".into());
    let repo = engine.get_repository_service();
    let runtime = engine.get_runtime_service();
    let task_service = engine.get_task_service();

    // Child only in no-tenant (global / default).
    repo.deploy(
        repo.create_deployment()
            .name("global child".into())
            .add_string(
                "child.bpmn20.xml".into(),
                child_user_task_xml("fallbackChild", "fallbackChildTask"),
            ),
    )
    .unwrap();

    let parent_xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
  <process id="parentFallback" isExecutable="true">
    <startEvent id="start" />
    <sequenceFlow id="f1" sourceRef="start" targetRef="call" />
    <callActivity id="call" calledElement="fallbackChild" flowable:fallbackToDefaultTenant="true" />
    <sequenceFlow id="f2" sourceRef="call" targetRef="outer" />
    <userTask id="outer" />
    <sequenceFlow id="f3" sourceRef="outer" targetRef="end" />
    <endEvent id="end" />
  </process>
</definitions>"#;

    repo.deploy(
        repo.create_deployment()
            .name("tenant parent".into())
            .tenant_id("acme".into())
            .add_string("parent.bpmn20.xml".into(), parent_xml.to_string()),
    )
    .unwrap();

    let parent = runtime
        .start_process_instance(
            runtime
                .create_process_instance_builder()
                .process_definition_key("parentFallback".into())
                .tenant_id("acme".into()),
        )
        .unwrap();

    let global_child = repo
        .latest_process_definition_by_key("fallbackChild", None)
        .unwrap()
        .expect("global child definition");

    let child = find_child_pi(&engine, &parent.id);
    assert_eq!(
        child.process_definition_id, global_child.id,
        "fallbackToDefaultTenant must resolve the no-tenant definition by key"
    );
    assert_eq!(child.process_definition_key, "fallbackChild");

    let tasks = task_service
        .get_tasks_by_process_instance_id(child.id)
        .unwrap();
    assert_eq!(tasks[0].task_definition_key, "fallbackChildTask");
}

#[test]
fn p21_without_fallback_tenant_miss_is_not_found() {
    let engine = ProcessEngine::new("p21-no-fallback-tenant".into());
    let repo = engine.get_repository_service();
    let runtime = engine.get_runtime_service();

    repo.deploy(
        repo.create_deployment()
            .name("global child".into())
            .add_string(
                "child.bpmn20.xml".into(),
                child_user_task_xml("strictChild", "strictChildTask"),
            ),
    )
    .unwrap();

    let parent_xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
  <process id="parentStrict" isExecutable="true">
    <startEvent id="start" />
    <sequenceFlow id="f1" sourceRef="start" targetRef="call" />
    <callActivity id="call" calledElement="strictChild" />
    <sequenceFlow id="f2" sourceRef="call" targetRef="end" />
    <endEvent id="end" />
  </process>
</definitions>"#;

    repo.deploy(
        repo.create_deployment()
            .name("tenant parent".into())
            .tenant_id("acme".into())
            .add_string("parent.bpmn20.xml".into(), parent_xml.to_string()),
    )
    .unwrap();

    let err = runtime
        .start_process_instance(
            runtime
                .create_process_instance_builder()
                .process_definition_key("parentStrict".into())
                .tenant_id("acme".into()),
        )
        .expect_err("strict tenant filter must not fall back");
    assert!(
        err.to_string().to_lowercase().contains("not found")
            || err.to_string().contains("strictChild"),
        "unexpected error: {err}"
    );
}

// ─── 3. useLocalScopeForOutParameters ────────────────────────────────────────
// Java CallActivityBehavior:261-269

#[test]
fn p21_use_local_scope_for_out_parameters() {
    let engine = ProcessEngine::new("p21-local-out".into());
    let repo = engine.get_repository_service();
    let runtime = engine.get_runtime_service();
    let task_service = engine.get_task_service();
    let variables = engine.get_variable_service();

    let parent_xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
  <process id="parentLocalOut" isExecutable="true">
    <startEvent id="start" />
    <sequenceFlow id="f1" sourceRef="start" targetRef="call" />
    <callActivity id="call" calledElement="childLocalOut"
                  flowable:useLocalScopeForOutParameters="true">
      <extensionElements>
        <flowable:out source="childResult" target="parentResult" />
      </extensionElements>
    </callActivity>
    <sequenceFlow id="f2" sourceRef="call" targetRef="outer" />
    <userTask id="outer" />
    <sequenceFlow id="f3" sourceRef="outer" targetRef="end" />
    <endEvent id="end" />
  </process>
</definitions>"#;

    repo.deploy(
        repo.create_deployment()
            .name("local out".into())
            .add_string("parent.bpmn20.xml".into(), parent_xml.to_string())
            .add_string(
                "child.bpmn20.xml".into(),
                child_user_task_xml("childLocalOut", "childTask"),
            ),
    )
    .unwrap();

    let parent = runtime
        .start_process_instance(
            runtime
                .create_process_instance_builder()
                .process_definition_key("parentLocalOut".into()),
        )
        .unwrap();
    let child = find_child_pi(&engine, &parent.id);

    variables
        .set_variable(child.id.clone(), "childResult".into(), json!("local-val"))
        .unwrap();
    let child_tasks = task_service
        .get_tasks_by_process_instance_id(child.id)
        .unwrap();
    task_service
        .complete_task_by_id(child_tasks[0].id.clone())
        .unwrap();

    // get_variable merges local→process, so inspect the maps directly:
    // durable process variables must not own the out target.
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let parent_execs: Vec<_> = store
        .snapshot_executions(&mut session)
        .into_values()
        .filter(|e| e.process_instance_id.as_deref() == Some(parent.id.as_str()))
        .collect();
    let in_process = parent_execs
        .iter()
        .any(|e| e.variables.get("parentResult") == Some(&json!("local-val")));
    let in_local = parent_execs
        .iter()
        .any(|e| e.local_variables.get("parentResult") == Some(&json!("local-val")));
    assert!(
        !in_process,
        "useLocalScopeForOutParameters must not write process-scoped variables"
    );
    assert!(
        in_local,
        "out parameter must land in local_variables; execs={:?}",
        parent_execs
            .iter()
            .map(|e| (
                e.id.clone(),
                e.activity_id.clone(),
                e.local_variables.clone(),
                e.variables.clone()
            ))
            .collect::<Vec<_>>()
    );
    // Variable service still resolves it (local is visible via get_variable).
    assert_eq!(
        variables
            .get_variable(parent.id.clone(), "parentResult".into())
            .unwrap(),
        Some(json!("local-val"))
    );
}

// ─── 5. businessKey priority: explicit > inherit ─────────────────────────────
// Java CallActivityBehavior:122-130

#[test]
fn p21_explicit_business_key_wins_over_inherit() {
    let engine = ProcessEngine::new("p21-bk-priority".into());
    let repo = engine.get_repository_service();
    let runtime = engine.get_runtime_service();

    let parent_xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
  <process id="parentBkPriority" isExecutable="true">
    <startEvent id="start" />
    <sequenceFlow id="f1" sourceRef="start" targetRef="call" />
    <callActivity id="call" calledElement="childBk"
                  flowable:businessKey="${explicitBk}"
                  flowable:inheritBusinessKey="true" />
    <sequenceFlow id="f2" sourceRef="call" targetRef="end" />
    <endEvent id="end" />
  </process>
</definitions>"#;

    repo.deploy(
        repo.create_deployment()
            .name("bk priority".into())
            .add_string("parent.bpmn20.xml".into(), parent_xml.to_string())
            .add_string(
                "child.bpmn20.xml".into(),
                child_user_task_xml("childBk", "childTask"),
            ),
    )
    .unwrap();

    let parent = runtime
        .start_process_instance(
            runtime
                .create_process_instance_builder()
                .process_definition_key("parentBkPriority".into())
                .business_key("parent-bk".into())
                .variable("explicitBk".into(), json!("child-explicit-bk")),
        )
        .unwrap();

    let child = find_child_pi(&engine, &parent.id);
    assert_eq!(
        child.business_key.as_deref(),
        Some("child-explicit-bk"),
        "explicit businessKey must beat inheritBusinessKey"
    );
}

// ─── 6. out targetExpression on child scope + transient ──────────────────────
// Java IOParameterUtil:84-104

#[test]
fn p21_out_target_expression_evaluated_on_child_scope() {
    let engine = ProcessEngine::new("p21-out-target-expr".into());
    let repo = engine.get_repository_service();
    let runtime = engine.get_runtime_service();
    let task_service = engine.get_task_service();
    let variables = engine.get_variable_service();

    let parent_xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
  <process id="parentOutTarget" isExecutable="true">
    <startEvent id="start" />
    <sequenceFlow id="f1" sourceRef="start" targetRef="call" />
    <callActivity id="call" calledElement="childOutTarget">
      <extensionElements>
        <flowable:out source="childResult" targetExpression="${targetName}" />
      </extensionElements>
    </callActivity>
    <sequenceFlow id="f2" sourceRef="call" targetRef="outer" />
    <userTask id="outer" />
    <sequenceFlow id="f3" sourceRef="outer" targetRef="end" />
    <endEvent id="end" />
  </process>
</definitions>"#;

    repo.deploy(
        repo.create_deployment()
            .name("out target".into())
            .add_string("parent.bpmn20.xml".into(), parent_xml.to_string())
            .add_string(
                "child.bpmn20.xml".into(),
                child_user_task_xml("childOutTarget", "childTask"),
            ),
    )
    .unwrap();

    let parent = runtime
        .start_process_instance(
            runtime
                .create_process_instance_builder()
                .process_definition_key("parentOutTarget".into())
                // Parent has a misleading targetName that must NOT be used.
                .variable("targetName".into(), json!("from-parent-wrong")),
        )
        .unwrap();
    let child = find_child_pi(&engine, &parent.id);

    variables
        .set_variable(child.id.clone(), "targetName".into(), json!("fromChild"))
        .unwrap();
    variables
        .set_variable(child.id.clone(), "childResult".into(), json!("payload"))
        .unwrap();
    let child_tasks = task_service
        .get_tasks_by_process_instance_id(child.id)
        .unwrap();
    task_service
        .complete_task_by_id(child_tasks[0].id.clone())
        .unwrap();

    assert_eq!(
        variables
            .get_variable(parent.id.clone(), "fromChild".into())
            .unwrap(),
        Some(json!("payload")),
        "targetExpression must resolve against child scope"
    );
    assert_eq!(
        variables
            .get_variable(parent.id.clone(), "from-parent-wrong".into())
            .unwrap(),
        None,
        "parent scope targetName must not be used for out targetExpression"
    );
}

#[test]
fn p21_out_parameter_transient_routes_to_transient_variables() {
    // P45: transient out is pure memory (Java VariableScopeImpl). Mid-command
    // it is visible to subsequent routing in the same complete-task command;
    // after commit it is stripped and must not appear as durable or via get_variable.
    let engine = ProcessEngine::new("p21-out-transient".into());
    let repo = engine.get_repository_service();
    let runtime = engine.get_runtime_service();
    let task_service = engine.get_task_service();
    let variables = engine.get_variable_service();

    let parent_xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
  <process id="parentOutTransient" isExecutable="true">
    <startEvent id="start" />
    <sequenceFlow id="f1" sourceRef="start" targetRef="call" />
    <callActivity id="call" calledElement="childOutTransient">
      <extensionElements>
        <flowable:out source="childResult" target="parentTransient" transient="true" />
      </extensionElements>
    </callActivity>
    <sequenceFlow id="f2" sourceRef="call" targetRef="gw" />
    <exclusiveGateway id="gw" default="toFail" />
    <sequenceFlow id="toOk" sourceRef="gw" targetRef="outerOk">
      <conditionExpression><![CDATA[${parentTransient == 't-val'}]]></conditionExpression>
    </sequenceFlow>
    <sequenceFlow id="toFail" sourceRef="gw" targetRef="outerFail" />
    <userTask id="outerOk" />
    <userTask id="outerFail" />
    <sequenceFlow id="f3" sourceRef="outerOk" targetRef="end" />
    <sequenceFlow id="f4" sourceRef="outerFail" targetRef="end" />
    <endEvent id="end" />
  </process>
</definitions>"#;

    repo.deploy(
        repo.create_deployment()
            .name("out transient".into())
            .add_string("parent.bpmn20.xml".into(), parent_xml.to_string())
            .add_string(
                "child.bpmn20.xml".into(),
                child_user_task_xml("childOutTransient", "childTask"),
            ),
    )
    .unwrap();

    let parent = runtime
        .start_process_instance(
            runtime
                .create_process_instance_builder()
                .process_definition_key("parentOutTransient".into()),
        )
        .unwrap();
    let child = find_child_pi(&engine, &parent.id);
    variables
        .set_variable(child.id.clone(), "childResult".into(), json!("t-val"))
        .unwrap();
    let child_tasks = task_service
        .get_tasks_by_process_instance_id(child.id)
        .unwrap();
    task_service
        .complete_task_by_id(child_tasks[0].id.clone())
        .unwrap();

    // Same-command visibility: gateway condition saw transient out and routed to outerOk.
    let parent_tasks = task_service
        .get_tasks_by_process_instance_id(parent.id.clone())
        .unwrap();
    assert_eq!(
        parent_tasks.len(),
        1,
        "parent should land on exactly one user task after child complete"
    );
    assert_eq!(
        parent_tasks[0].name.as_str(),
        "outerOk",
        "transient out must be visible mid-command for gateway routing"
    );

    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let parent_execs: Vec<_> = store
        .snapshot_executions(&mut session)
        .into_values()
        .filter(|e| e.process_instance_id.as_deref() == Some(parent.id.as_str()))
        .collect();
    assert!(
        parent_execs
            .iter()
            .all(|e| e.variables.get("parentTransient").is_none()),
        "transient out parameter must not write durable process variables"
    );
    assert!(
        parent_execs
            .iter()
            .all(|e| e.transient_variables.get("parentTransient").is_none()),
        "transient out must be stripped on commit (Java VariableScopeImpl parity)"
    );
    assert_eq!(
        variables
            .get_variable(parent.id.clone(), "parentTransient".into())
            .unwrap(),
        None,
        "cross-command get_variable must not see stripped transient out"
    );
}

// ─── 7. inheritVariables preserves transient ─────────────────────────────────
// Java CallActivityBehavior:154-172,185-187; CallActivityTest:213

#[test]
fn p21_inherit_variables_keeps_transient_as_transient() {
    // P45: inheritVariables must copy parent transient onto the child as
    // transient (not durable) so mid-command child routing can read it.
    // After commit, transient is stripped (Java VariableScopeImpl parity).
    let engine = ProcessEngine::new("p21-inherit-transient".into());
    let repo = engine.get_repository_service();
    let runtime = engine.get_runtime_service();
    let task_service = engine.get_task_service();

    let parent_xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
  <process id="parentInheritTransient" isExecutable="true">
    <startEvent id="start" />
    <sequenceFlow id="f1" sourceRef="start" targetRef="call" />
    <callActivity id="call" calledElement="childInheritTransient"
                  flowable:inheritVariables="true" />
    <sequenceFlow id="f2" sourceRef="call" targetRef="end" />
    <endEvent id="end" />
  </process>
</definitions>"#;

    // Child gateway reads transientVar from the child scope (not parent EL
    // flattening), proving inheritVariables kept the transient/durable split.
    let child_xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
  <process id="childInheritTransient" isExecutable="true">
    <startEvent id="childStart" />
    <sequenceFlow id="c1" sourceRef="childStart" targetRef="gw" />
    <exclusiveGateway id="gw" default="toFail" />
    <sequenceFlow id="toOk" sourceRef="gw" targetRef="okTask">
      <conditionExpression><![CDATA[${transientVar == 't'}]]></conditionExpression>
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
            .name("inherit transient".into())
            .add_string("parent.bpmn20.xml".into(), parent_xml.to_string())
            .add_string("child.bpmn20.xml".into(), child_xml.to_string()),
    )
    .unwrap();

    let parent = runtime
        .start_process_instance(
            runtime
                .create_process_instance_builder()
                .process_definition_key("parentInheritTransient".into())
                .variable("durableVar".into(), json!("d"))
                .transient_variable("transientVar".into(), json!("t")),
        )
        .unwrap();

    let child = find_child_pi(&engine, &parent.id);
    let child_tasks = task_service
        .get_tasks_by_process_instance_id(child.id.clone())
        .unwrap();
    assert_eq!(child_tasks.len(), 1);
    assert_eq!(
        child_tasks[0].name.as_str(),
        "okTask",
        "inheritVariables must place parent transient on the child so \
         mid-command child routing can resolve transientVar"
    );

    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let child_root = store
        .find_execution(&child.id, &mut session)
        .expect("child root execution");

    assert_eq!(
        child_root.variables.get("durableVar"),
        Some(&json!("d")),
        "durable vars must be inherited into process variables"
    );
    assert!(
        !child_root.variables.contains_key("transientVar"),
        "transient must not be promoted to durable process variables"
    );
    assert!(
        !child_root.transient_variables.contains_key("transientVar"),
        "inherited transient must be stripped on commit (Java VariableScopeImpl parity)"
    );
}

// ─── 8. processInstanceName ──────────────────────────────────────────────────
// Java CallActivity.java:30,93-97; CallActivityBehavior:189-195

#[test]
fn p21_process_instance_name_expression() {
    let engine = ProcessEngine::new("p21-pi-name".into());
    let repo = engine.get_repository_service();
    let runtime = engine.get_runtime_service();

    let parent_xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
  <process id="parentPiName" isExecutable="true">
    <startEvent id="start" />
    <sequenceFlow id="f1" sourceRef="start" targetRef="call" />
    <callActivity id="call" calledElement="childPiName"
                  flowable:processInstanceName="${childName}"
                  flowable:inheritVariables="true" />
    <sequenceFlow id="f2" sourceRef="call" targetRef="end" />
    <endEvent id="end" />
  </process>
</definitions>"#;

    repo.deploy(
        repo.create_deployment()
            .name("pi name".into())
            .add_string("parent.bpmn20.xml".into(), parent_xml.to_string())
            .add_string(
                "child.bpmn20.xml".into(),
                child_user_task_xml("childPiName", "childTask"),
            ),
    )
    .unwrap();

    let parent = runtime
        .start_process_instance(
            runtime
                .create_process_instance_builder()
                .process_definition_key("parentPiName".into())
                .variable("childName".into(), json!("named-child-42")),
        )
        .unwrap();

    let child = find_child_pi(&engine, &parent.id);
    assert_eq!(
        child.name.as_deref(),
        Some("named-child-42"),
        "processInstanceName expression must name the child PI"
    );
}

// ─── 9. suspended parent check on child complete ─────────────────────────────
// Java CallActivityBehavior:279-285; CallActivityAdvancedTest:1421,1454

#[test]
fn p21_child_complete_rejects_when_parent_suspended() {
    let engine = ProcessEngine::new("p21-suspended-parent".into());
    let repo = engine.get_repository_service();
    let runtime = engine.get_runtime_service();
    let task_service = engine.get_task_service();

    let parent_xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
  <process id="parentSuspended" isExecutable="true">
    <startEvent id="start" />
    <sequenceFlow id="f1" sourceRef="start" targetRef="call" />
    <callActivity id="call" calledElement="childSuspended" />
    <sequenceFlow id="f2" sourceRef="call" targetRef="outer" />
    <userTask id="outer" />
    <sequenceFlow id="f3" sourceRef="outer" targetRef="end" />
    <endEvent id="end" />
  </process>
</definitions>"#;

    repo.deploy(
        repo.create_deployment()
            .name("suspended parent".into())
            .add_string("parent.bpmn20.xml".into(), parent_xml.to_string())
            .add_string(
                "child.bpmn20.xml".into(),
                child_user_task_xml("childSuspended", "childTask"),
            ),
    )
    .unwrap();

    let parent = runtime
        .start_process_instance(
            runtime
                .create_process_instance_builder()
                .process_definition_key("parentSuspended".into()),
        )
        .unwrap();
    let child = find_child_pi(&engine, &parent.id);

    runtime
        .suspend_process_instance(parent.id.clone(), ProcessInstanceUpdate::default())
        .unwrap();

    let child_tasks = task_service
        .get_tasks_by_process_instance_id(child.id)
        .unwrap();
    let err = task_service
        .complete_task_by_id(child_tasks[0].id.clone())
        .expect_err("completing child under suspended parent must fail");
    assert!(
        err.to_string().to_lowercase().contains("suspended"),
        "error should mention suspended: {err}"
    );
}

// ─── 10. sameDeployment miss falls back to latest-by-key ─────────────────────
// Java CallActivityBehavior:299-313; CallActivityTest:359,393

#[test]
fn p21_same_deployment_miss_falls_back_to_latest_by_key() {
    let engine = ProcessEngine::new("p21-same-dep-miss".into());
    let repo = engine.get_repository_service();
    let runtime = engine.get_runtime_service();
    let task_service = engine.get_task_service();

    // Child only in a separate deployment (v1 then v2).
    repo.deploy(
        repo.create_deployment()
            .name("child v1".into())
            .add_string(
                "child_v1.bpmn20.xml".into(),
                child_user_task_xml("sameDepMissChild", "childTaskV1"),
            ),
    )
    .unwrap();
    repo.deploy(
        repo.create_deployment()
            .name("child v2".into())
            .add_string(
                "child_v2.bpmn20.xml".into(),
                child_user_task_xml("sameDepMissChild", "childTaskV2"),
            ),
    )
    .unwrap();

    let parent_xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
  <process id="parentSameDepMiss" isExecutable="true">
    <startEvent id="start" />
    <sequenceFlow id="f1" sourceRef="start" targetRef="call" />
    <callActivity id="call" calledElement="sameDepMissChild"
                  flowable:sameDeployment="true" />
    <sequenceFlow id="f2" sourceRef="call" targetRef="end" />
    <endEvent id="end" />
  </process>
</definitions>"#;

    // Parent alone in its own deployment — sameDeployment miss.
    repo.deploy(
        repo.create_deployment()
            .name("parent only".into())
            .add_string("parent.bpmn20.xml".into(), parent_xml.to_string()),
    )
    .unwrap();

    let parent = runtime
        .start_process_instance(
            runtime
                .create_process_instance_builder()
                .process_definition_key("parentSameDepMiss".into()),
        )
        .unwrap();
    let child = find_child_pi(&engine, &parent.id);
    assert_eq!(
        child.process_definition_version, 2,
        "sameDeployment miss must fall back to latest-by-key"
    );

    let tasks = task_service
        .get_tasks_by_process_instance_id(child.id)
        .unwrap();
    assert_eq!(tasks[0].task_definition_key, "childTaskV2");
}

// ─── 11. processInstanceIdVariableName as expression ─────────────────────────
// Java CallActivityBehavior:207-213; CallActivityAdvancedTest:1401

#[test]
fn p21_process_instance_id_variable_name_expression() {
    let engine = ProcessEngine::new("p21-id-var-expr".into());
    let repo = engine.get_repository_service();
    let runtime = engine.get_runtime_service();
    let variables = engine.get_variable_service();

    let parent_xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
  <process id="parentIdVarExpr" isExecutable="true">
    <startEvent id="start" />
    <sequenceFlow id="f1" sourceRef="start" targetRef="call" />
    <callActivity id="call" calledElement="childIdVar"
                  flowable:processInstanceIdVariableName="${idVarName}" />
    <sequenceFlow id="f2" sourceRef="call" targetRef="outer" />
    <userTask id="outer" />
    <sequenceFlow id="f3" sourceRef="outer" targetRef="end" />
    <endEvent id="end" />
  </process>
</definitions>"#;

    repo.deploy(
        repo.create_deployment()
            .name("id var expr".into())
            .add_string("parent.bpmn20.xml".into(), parent_xml.to_string())
            .add_string(
                "child.bpmn20.xml".into(),
                child_user_task_xml("childIdVar", "childTask"),
            ),
    )
    .unwrap();

    let parent = runtime
        .start_process_instance(
            runtime
                .create_process_instance_builder()
                .process_definition_key("parentIdVarExpr".into())
                .variable("idVarName".into(), json!("resolvedChildId")),
        )
        .unwrap();
    let child = find_child_pi(&engine, &parent.id);

    assert_eq!(
        variables
            .get_variable(parent.id.clone(), "resolvedChildId".into())
            .unwrap(),
        Some(json!(child.id)),
        "processInstanceIdVariableName expression must resolve to the variable name"
    );
    // Must not store under the literal expression string.
    assert_eq!(
        variables
            .get_variable(parent.id, "${idVarName}".into())
            .unwrap(),
        None
    );
}

// ─── 12. entity links default off ────────────────────────────────────────────
// Java CallActivityBehavior:202-205 enableEntityLinks default false

#[test]
fn p21_entity_links_default_off_no_links_created() {
    let engine = ProcessEngine::new("p21-entity-links-off".into());
    let repo = engine.get_repository_service();
    let runtime = engine.get_runtime_service();

    let parent_xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
  <process id="parentNoLinks" isExecutable="true">
    <startEvent id="start" />
    <sequenceFlow id="f1" sourceRef="start" targetRef="call" />
    <callActivity id="call" calledElement="childNoLinks" />
    <sequenceFlow id="f2" sourceRef="call" targetRef="end" />
    <endEvent id="end" />
  </process>
</definitions>"#;

    repo.deploy(
        repo.create_deployment()
            .name("no links".into())
            .add_string("parent.bpmn20.xml".into(), parent_xml.to_string())
            .add_string(
                "child.bpmn20.xml".into(),
                child_user_task_xml("childNoLinks", "childTask"),
            ),
    )
    .unwrap();

    let parent = runtime
        .start_process_instance(
            runtime
                .create_process_instance_builder()
                .process_definition_key("parentNoLinks".into()),
        )
        .unwrap();
    let _child = find_child_pi(&engine, &parent.id);

    let links = engine
        .get_entity_link_service()
        .create_entity_link_query()
        .list()
        .unwrap();
    assert!(
        links.is_empty(),
        "enableEntityLinks default false → no entity links"
    );
}

#[test]
fn p21_entity_links_created_when_enabled() {
    let config = ProcessEngineConfiguration {
        enable_entity_links: true,
        ..Default::default()
    };
    let engine = ProcessEngine::new_with_config("p21-entity-links-on".into(), config);
    let repo = engine.get_repository_service();
    let runtime = engine.get_runtime_service();

    let parent_xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
  <process id="parentWithLinks" isExecutable="true">
    <startEvent id="start" />
    <sequenceFlow id="f1" sourceRef="start" targetRef="call" />
    <callActivity id="call" calledElement="childWithLinks" />
    <sequenceFlow id="f2" sourceRef="call" targetRef="end" />
    <endEvent id="end" />
  </process>
</definitions>"#;

    repo.deploy(
        repo.create_deployment()
            .name("with links".into())
            .add_string("parent.bpmn20.xml".into(), parent_xml.to_string())
            .add_string(
                "child.bpmn20.xml".into(),
                child_user_task_xml("childWithLinks", "childTask"),
            ),
    )
    .unwrap();

    let parent = runtime
        .start_process_instance(
            runtime
                .create_process_instance_builder()
                .process_definition_key("parentWithLinks".into()),
        )
        .unwrap();
    let child = find_child_pi(&engine, &parent.id);

    let links = engine
        .get_entity_link_service()
        .create_entity_link_query()
        .list()
        .unwrap();
    assert_eq!(links.len(), 1, "enableEntityLinks true → one parent→child link");
    assert_eq!(links[0].scope_id.as_deref(), Some(parent.id.as_str()));
    assert_eq!(
        links[0].reference_scope_id.as_deref(),
        Some(child.id.as_str())
    );
}

// ─── 4. completeAsync — deferred end via async job (P47) ────────────────────
// Java EndExecutionOperation.java:94-96 (defer when !forceSynchronous and the
// super execution's call activity has completeAsync) + :159-180 (job on the
// *parent* execution, configuration = child PI id, TYPE with the original
// Java misspelling "async-complete-call-actiivty") +
// AsyncCompleteCallActivityJobHandler.java:44-47 (replay end synchronously).

#[test]
fn p21_complete_async_defers_parent_continuation_to_job() {
    let engine = ProcessEngine::new("p21-complete-async".into());
    let repo = engine.get_repository_service();
    let runtime = engine.get_runtime_service();
    let task_service = engine.get_task_service();

    let parent_xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
  <process id="parentCompleteAsync" isExecutable="true">
    <startEvent id="start" />
    <sequenceFlow id="f1" sourceRef="start" targetRef="call" />
    <callActivity id="call" calledElement="childCompleteAsync"
                  flowable:completeAsync="true" />
    <sequenceFlow id="f2" sourceRef="call" targetRef="outer" />
    <userTask id="outer" />
    <sequenceFlow id="f3" sourceRef="outer" targetRef="end" />
    <endEvent id="end" />
  </process>
</definitions>"#;

    repo.deploy(
        repo.create_deployment()
            .name("complete async".into())
            .add_string("parent.bpmn20.xml".into(), parent_xml.to_string())
            .add_string(
                "child.bpmn20.xml".into(),
                child_auto_complete_xml("childCompleteAsync"),
            ),
    )
    .unwrap();

    let parent = runtime
        .start_process_instance(
            runtime
                .create_process_instance_builder()
                .process_definition_key("parentCompleteAsync".into()),
        )
        .unwrap();

    // Java EndExecutionOperation:94-96: the child's end operation is deferred
    // into an async job — the parent must NOT continue synchronously.
    let tasks = task_service
        .get_tasks_by_process_instance_id(parent.id.clone())
        .unwrap();
    assert!(
        tasks.is_empty(),
        "completeAsync defers the end: the parent must not reach outerTask yet"
    );

    // The child PI is still alive (its end has not run yet).
    let child = find_child_pi(&engine, &parent.id);
    assert!(!child.is_ended, "child PI must still be alive before the job");

    // Java :159-180: one job on the parent (super) execution, with the child
    // PI id as configuration and the original Java handler-type misspelling.
    let jobs: Vec<_> = engine
        .get_management_service()
        .list_executable_jobs()
        .into_iter()
        .filter(|job| {
            job.handler_type.as_deref()
                == Some(job_handler_types::ASYNC_COMPLETE_CALL_ACTIVITY)
        })
        .collect();
    assert_eq!(jobs.len(), 1, "exactly one async-complete-call-activity job");
    assert_eq!(
        jobs[0].handler_type.as_deref(),
        Some("async-complete-call-actiivty"),
        "the Java misspelling is part of the wire contract"
    );
    assert_eq!(
        jobs[0].execution_id,
        child.super_execution_id.clone().unwrap(),
        "Java :164-165: the job hangs off the parent (super) execution"
    );
    assert_eq!(
        jobs[0].process_instance_id, parent.id,
        "Java :170: parent process instance id"
    );
    assert_eq!(
        jobs[0].job_handler_configuration.as_deref(),
        Some(child.id.as_str()),
        "Java :167-168: child PI execution id as configuration"
    );
    assert_eq!(
        jobs[0].process_definition_id.as_deref(),
        Some(child.process_definition_id.as_str()),
        "Java :172: process definition of the child process instance"
    );

    // Java AsyncCompleteCallActivityJobHandler:44-47: the job replays the end
    // synchronously → child ends, out params copy, parent continues.
    engine
        .get_management_service()
        .execute_job(&jobs[0].timer_job_id)
        .expect("the async-complete-call-activity job should execute");

    let tasks = task_service
        .get_tasks_by_process_instance_id(parent.id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 1, "parent continues after the job runs");
    assert_eq!(tasks[0].task_definition_key, "outer");

    // The child PI is gone and the job was consumed.
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    assert!(
        store
            .find_process_instance(&child.id, &mut session)
            .is_none_or(|pi| pi.is_ended),
        "child PI must be ended after the job"
    );
    assert!(
        engine
            .get_management_service()
            .list_executable_jobs()
            .into_iter()
            .all(|job| job.handler_type.as_deref()
                != Some(job_handler_types::ASYNC_COMPLETE_CALL_ACTIVITY)),
        "the job must be consumed"
    );
}
