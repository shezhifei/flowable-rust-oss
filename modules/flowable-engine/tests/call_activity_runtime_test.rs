use flowable_engine::engine::process_engine::ProcessEngine;
use serde_json::json;

const PARENT_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
    <process id="parentProcess" isExecutable="true">
        <startEvent id="startEvent" />
        <sequenceFlow id="flow1" sourceRef="startEvent" targetRef="callActivity" />
        <callActivity id="callActivity" calledElement="childProcess" />
        <sequenceFlow id="flow2" sourceRef="callActivity" targetRef="outerTask" />
        <userTask id="outerTask" name="Outer Task" />
        <sequenceFlow id="flow3" sourceRef="outerTask" targetRef="outerEnd" />
        <endEvent id="outerEnd" />
    </process>
</definitions>"#;

const CHILD_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
    <process id="childProcess" isExecutable="true">
        <startEvent id="childStart" />
        <sequenceFlow id="childFlow1" sourceRef="childStart" targetRef="childTask" />
        <userTask id="childTask" name="Child Task" />
        <sequenceFlow id="childFlow2" sourceRef="childTask" targetRef="childEnd" />
        <endEvent id="childEnd" />
    </process>
</definitions>"#;

const PARENT_WITH_IN_OUT_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
    <process id="parentWithInOut" isExecutable="true">
        <startEvent id="startEvent" />
        <sequenceFlow id="flow1" sourceRef="startEvent" targetRef="callActivity" />
        <callActivity id="callActivity" calledElement="childWithResult">
            <extensionElements>
                <flowable:in source="parentInput" target="childInput" />
                <flowable:out source="childResult" target="parentResult" />
            </extensionElements>
        </callActivity>
        <sequenceFlow id="flow2" sourceRef="callActivity" targetRef="outerTask" />
        <userTask id="outerTask" name="Outer Task" />
        <sequenceFlow id="flow3" sourceRef="outerTask" targetRef="outerEnd" />
        <endEvent id="outerEnd" />
    </process>
</definitions>"#;

const CHILD_WITH_RESULT_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
    <process id="childWithResult" isExecutable="true">
        <startEvent id="childStart" />
        <sequenceFlow id="childFlow1" sourceRef="childStart" targetRef="childTask" />
        <userTask id="childTask" name="Child Task" />
        <sequenceFlow id="childFlow2" sourceRef="childTask" targetRef="childEnd" />
        <endEvent id="childEnd" />
    </process>
</definitions>"#;

const PARENT_INHERIT_BUSINESS_KEY_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
    <process id="parentInheritBusinessKey" isExecutable="true">
        <startEvent id="startEvent" />
        <sequenceFlow id="flow1" sourceRef="startEvent" targetRef="callActivity" />
        <callActivity id="callActivity" calledElement="childBusinessKey" flowable:inheritBusinessKey="true" />
        <sequenceFlow id="flow2" sourceRef="callActivity" targetRef="outerTask" />
        <userTask id="outerTask" name="Outer Task" />
        <sequenceFlow id="flow3" sourceRef="outerTask" targetRef="outerEnd" />
        <endEvent id="outerEnd" />
    </process>
</definitions>"#;

const CHILD_BUSINESS_KEY_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
    <process id="childBusinessKey" isExecutable="true">
        <startEvent id="childStart" />
        <sequenceFlow id="childFlow1" sourceRef="childStart" targetRef="childTask" />
        <userTask id="childTask" name="Child Task" />
        <sequenceFlow id="childFlow2" sourceRef="childTask" targetRef="childEnd" />
        <endEvent id="childEnd" />
    </process>
</definitions>"#;

const PARENT_TENANT_LATEST_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
    <process id="parentTenantLatest" isExecutable="true">
        <startEvent id="startEvent" />
        <sequenceFlow id="flow1" sourceRef="startEvent" targetRef="callActivity" />
        <callActivity id="callActivity" calledElement="childProcess" />
        <sequenceFlow id="flow2" sourceRef="callActivity" targetRef="outerTask" />
        <userTask id="outerTask" name="Outer Task" />
        <sequenceFlow id="flow3" sourceRef="outerTask" targetRef="outerEnd" />
        <endEvent id="outerEnd" />
    </process>
</definitions>"#;

fn child_process_xml(task_id: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
    <process id="childProcess" isExecutable="true">
        <startEvent id="childStart" />
        <sequenceFlow id="childFlow1" sourceRef="childStart" targetRef="{task_id}" />
        <userTask id="{task_id}" name="{task_id}" />
        <sequenceFlow id="childFlow2" sourceRef="{task_id}" targetRef="childEnd" />
        <endEvent id="childEnd" />
    </process>
</definitions>"#
    )
}

const PARENT_EXPRESSION_CALL_ACTIVITY_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
    <process id="parentExpressionCallActivity" isExecutable="true">
        <startEvent id="startEvent" />
        <sequenceFlow id="flow1" sourceRef="startEvent" targetRef="callActivity" />
        <callActivity id="callActivity"
                      calledElement="${childProcessKey}"
                      flowable:processInstanceIdVariableName="childInstanceId">
            <extensionElements>
                <flowable:in businessKey="${childBusinessKey}" />
                <flowable:in sourceExpression="${parentInput}" target="childInput" />
                <flowable:in sourceExpression="${'gold'}" targetExpression="childTier" />
                <flowable:out sourceExpression="${childResult}" target="parentResult" />
            </extensionElements>
        </callActivity>
        <sequenceFlow id="flow2" sourceRef="callActivity" targetRef="outerTask" />
        <userTask id="outerTask" name="Outer Task" />
        <sequenceFlow id="flow3" sourceRef="outerTask" targetRef="outerEnd" />
        <endEvent id="outerEnd" />
    </process>
</definitions>"#;

const CHILD_EXPRESSION_CALLED_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
    <process id="childExpressionTarget" isExecutable="true">
        <startEvent id="childStart" />
        <sequenceFlow id="childFlow1" sourceRef="childStart" targetRef="childTask" />
        <userTask id="childTask" name="Child Task" />
        <sequenceFlow id="childFlow2" sourceRef="childTask" targetRef="childEnd" />
        <endEvent id="childEnd" />
    </process>
</definitions>"#;

const PARENT_SAME_DEPLOYMENT_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
    <process id="parentSameDeployment" isExecutable="true">
        <startEvent id="startEvent" />
        <sequenceFlow id="flow1" sourceRef="startEvent" targetRef="callActivity" />
        <callActivity id="callActivity" calledElement="sameDeploymentChild" flowable:calledElementBinding="deployment" />
        <sequenceFlow id="flow2" sourceRef="callActivity" targetRef="outerTask" />
        <userTask id="outerTask" name="Outer Task" />
        <sequenceFlow id="flow3" sourceRef="outerTask" targetRef="outerEnd" />
        <endEvent id="outerEnd" />
    </process>
</definitions>"#;

fn same_deployment_child_xml(task_id: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
    <process id="sameDeploymentChild" isExecutable="true">
        <startEvent id="childStart" />
        <sequenceFlow id="childFlow1" sourceRef="childStart" targetRef="{task_id}" />
        <userTask id="{task_id}" name="{task_id}" />
        <sequenceFlow id="childFlow2" sourceRef="{task_id}" targetRef="childEnd" />
        <endEvent id="childEnd" />
    </process>
</definitions>"#
    )
}

#[test]
fn test_call_activity_runtime_semantics() {
    let process_engine = ProcessEngine::new("default".to_string());
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();
    let task_service = process_engine.get_task_service();

    let deployment_builder = repository_service
        .create_deployment()
        .name("Call Activity Deployment".to_string())
        .add_string(
            "parent_process.bpmn20.xml".to_string(),
            PARENT_XML.to_string(),
        )
        .add_string(
            "child_process.bpmn20.xml".to_string(),
            CHILD_XML.to_string(),
        );
    repository_service.deploy(deployment_builder).unwrap();

    let process_def_id = repository_service
        .get_process_definition_ids()
        .unwrap()
        .into_iter()
        .find(|id| id.starts_with("parentProcess"))
        .unwrap();

    let builder = runtime_service
        .create_process_instance_builder()
        .process_definition_id(process_def_id);
    let parent_pi = runtime_service.start_process_instance(builder).unwrap();

    // 1. Parent process starts, enters CallActivity, creates Child Process and Child Task
    // Find the child process instance by looking for one with super_execution_id set
    let store = process_engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let child_pi = store
        .snapshot_process_instances(&mut session)
        .into_values()
        .find(|pi| {
            pi.super_execution_id.is_some()
                && pi
                    .super_execution_id
                    .as_ref()
                    .unwrap()
                    .starts_with(&parent_pi.id)
        })
        .expect("Child process instance should be created");
    // Release the session's transaction before the next command acquires its own.
    drop(session);

    let tasks = task_service
        .get_tasks_by_process_instance_id(child_pi.id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 1, "Should be at child task");

    let child_pi_id = tasks[0].process_instance_id.clone();
    assert_ne!(
        child_pi_id, parent_pi.id,
        "Child task should be in a separate process instance"
    );

    // 2. Complete Child Task, child process should complete and resume outer flow
    task_service
        .complete_task_by_id(tasks[0].id.clone())
        .unwrap();

    // 3. Parent flow resumes, creating Outer Task
    let tasks = task_service
        .get_tasks_by_process_instance_id(parent_pi.id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 1, "Should be at outer task");
    assert_eq!(tasks[0].task_definition_key, "outerTask");

    // 4. Complete Outer Task, parent process instance should end
    task_service
        .complete_task_by_id(tasks[0].id.clone())
        .unwrap();

    let runtime_store = process_engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let pi = runtime_store
        .find_process_instance(&parent_pi.id, &mut session)
        .unwrap();
    assert!(pi.is_ended, "Parent process instance should be ended");
}

#[test]
fn test_call_activity_maps_basic_in_and_out_variables() {
    let process_engine = ProcessEngine::new("default".to_string());
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();
    let task_service = process_engine.get_task_service();
    let variable_service = process_engine.get_variable_service();

    let deployment_builder = repository_service
        .create_deployment()
        .name("Call Activity In Out Deployment".to_string())
        .add_string(
            "parent_with_in_out.bpmn20.xml".to_string(),
            PARENT_WITH_IN_OUT_XML.to_string(),
        )
        .add_string(
            "child_with_result.bpmn20.xml".to_string(),
            CHILD_WITH_RESULT_XML.to_string(),
        );
    repository_service.deploy(deployment_builder).unwrap();

    let process_def_id = repository_service
        .get_process_definition_ids()
        .unwrap()
        .into_iter()
        .find(|id| id.starts_with("parentWithInOut"))
        .unwrap();

    let builder = runtime_service
        .create_process_instance_builder()
        .process_definition_id(process_def_id)
        .process_definition_key("parentWithInOut".to_string())
        .variable("parentInput".to_string(), json!("from-parent"));
    let parent_pi = runtime_service.start_process_instance(builder).unwrap();

    let store = process_engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let child_pi = store
        .snapshot_process_instances(&mut session)
        .into_values()
        .find(|pi| {
            pi.super_execution_id.is_some()
                && pi
                    .super_execution_id
                    .as_ref()
                    .unwrap()
                    .starts_with(&parent_pi.id)
        })
        .expect("Child process instance should be created");
    drop(session);

    assert_eq!(
        variable_service
            .get_variable(child_pi.id.clone(), "childInput".to_string())
            .unwrap(),
        Some(json!("from-parent")),
        "flowable:in source/target should copy parent variable to child"
    );

    variable_service
        .set_variable(
            child_pi.id.clone(),
            "childResult".to_string(),
            json!("from-child"),
        )
        .unwrap();

    let tasks = task_service
        .get_tasks_by_process_instance_id(child_pi.id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 1, "Should be at child task");
    task_service
        .complete_task_by_id(tasks[0].id.clone())
        .unwrap();

    assert_eq!(
        variable_service
            .get_variable(parent_pi.id.clone(), "parentResult".to_string())
            .unwrap(),
        Some(json!("from-child")),
        "flowable:out source/target should copy completed child variable back to parent"
    );

    let tasks = task_service
        .get_tasks_by_process_instance_id(parent_pi.id.clone())
        .unwrap();
    assert_eq!(
        tasks.len(),
        1,
        "Parent should continue after child completion"
    );
    assert_eq!(tasks[0].task_definition_key, "outerTask");
}

#[test]
fn test_call_activity_inherits_parent_business_key() {
    let process_engine = ProcessEngine::new("default".to_string());
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();

    let deployment_builder = repository_service
        .create_deployment()
        .name("Call Activity Business Key Deployment".to_string())
        .add_string(
            "parent_inherit_business_key.bpmn20.xml".to_string(),
            PARENT_INHERIT_BUSINESS_KEY_XML.to_string(),
        )
        .add_string(
            "child_business_key.bpmn20.xml".to_string(),
            CHILD_BUSINESS_KEY_XML.to_string(),
        );
    repository_service.deploy(deployment_builder).unwrap();

    let process_def_id = repository_service
        .get_process_definition_ids()
        .unwrap()
        .into_iter()
        .find(|id| id.starts_with("parentInheritBusinessKey"))
        .unwrap();

    let builder = runtime_service
        .create_process_instance_builder()
        .process_definition_id(process_def_id)
        .process_definition_key("parentInheritBusinessKey".to_string())
        .business_key("order-4242".to_string());
    let parent_pi = runtime_service.start_process_instance(builder).unwrap();

    let store = process_engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let child_pi = store
        .snapshot_process_instances(&mut session)
        .into_values()
        .find(|pi| {
            pi.super_execution_id.is_some()
                && pi
                    .super_execution_id
                    .as_ref()
                    .unwrap()
                    .starts_with(&parent_pi.id)
        })
        .expect("Child process instance should be created");

    assert_eq!(
        child_pi.business_key.as_deref(),
        Some("order-4242"),
        "flowable:inheritBusinessKey should copy parent business key to child process instance"
    );
}

#[test]
fn test_call_activity_resolves_latest_called_element_in_parent_tenant() {
    let process_engine = ProcessEngine::new("default".to_string());
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();
    let task_service = process_engine.get_task_service();

    repository_service
        .deploy(
            repository_service
                .create_deployment()
                .name("acme child v1".to_string())
                .tenant_id("acme".to_string())
                .add_string(
                    "child_v1.bpmn20.xml".to_string(),
                    child_process_xml("childTaskV1"),
                ),
        )
        .unwrap();
    repository_service
        .deploy(
            repository_service
                .create_deployment()
                .name("acme child v2".to_string())
                .tenant_id("acme".to_string())
                .add_string(
                    "child_v2.bpmn20.xml".to_string(),
                    child_process_xml("childTaskV2"),
                ),
        )
        .unwrap();

    for version in 1..=3 {
        repository_service
            .deploy(
                repository_service
                    .create_deployment()
                    .name(format!("other child v{version}"))
                    .tenant_id("other".to_string())
                    .add_string(
                        format!("other_child_v{version}.bpmn20.xml"),
                        child_process_xml(&format!("otherChildTaskV{version}")),
                    ),
            )
            .unwrap();
    }

    repository_service
        .deploy(
            repository_service
                .create_deployment()
                .name("acme parent".to_string())
                .tenant_id("acme".to_string())
                .add_string(
                    "parent_tenant_latest.bpmn20.xml".to_string(),
                    PARENT_TENANT_LATEST_XML.to_string(),
                ),
        )
        .unwrap();

    let expected_child_definition = repository_service
        .latest_process_definition_by_key("childProcess", Some("acme"))
        .unwrap()
        .expect("acme child process definition should exist");
    assert_eq!(expected_child_definition.version, 2);

    let builder = runtime_service
        .create_process_instance_builder()
        .process_definition_key("parentTenantLatest".to_string())
        .tenant_id("acme".to_string());
    let parent_pi = runtime_service.start_process_instance(builder).unwrap();

    let store = process_engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let child_pi = store
        .snapshot_process_instances(&mut session)
        .into_values()
        .find(|pi| {
            pi.super_execution_id.is_some()
                && pi
                    .super_execution_id
                    .as_ref()
                    .unwrap()
                    .starts_with(&parent_pi.id)
        })
        .expect("Child process instance should be created");
    drop(session);

    assert_eq!(
        child_pi.process_definition_id, expected_child_definition.id,
        "callActivity calledElement should resolve to the latest definition within the parent tenant"
    );
    assert_eq!(child_pi.tenant_id.as_deref(), Some("acme"));
    assert_eq!(child_pi.process_definition_version, 2);

    let tasks = task_service
        .get_tasks_by_process_instance_id(child_pi.id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 1, "Should be at child task");
    assert_eq!(tasks[0].task_definition_key, "childTaskV2");
}

#[test]
fn test_call_activity_resolves_called_element_business_key_and_io_expressions() {
    let process_engine = ProcessEngine::new("default".to_string());
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();
    let task_service = process_engine.get_task_service();
    let variable_service = process_engine.get_variable_service();

    repository_service
        .deploy(
            repository_service
                .create_deployment()
                .name("Call Activity Expression Deployment".to_string())
                .add_string(
                    "parent_expression_call_activity.bpmn20.xml".to_string(),
                    PARENT_EXPRESSION_CALL_ACTIVITY_XML.to_string(),
                )
                .add_string(
                    "child_expression_called.bpmn20.xml".to_string(),
                    CHILD_EXPRESSION_CALLED_XML.to_string(),
                ),
        )
        .unwrap();

    let builder = runtime_service
        .create_process_instance_builder()
        .process_definition_key("parentExpressionCallActivity".to_string())
        .variable(
            "childProcessKey".to_string(),
            json!("childExpressionTarget"),
        )
        .variable("childBusinessKey".to_string(), json!("child-bk-9000"))
        .variable("parentInput".to_string(), json!("from-parent"));
    let parent_pi = runtime_service.start_process_instance(builder).unwrap();

    let store = process_engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let child_pi = store
        .snapshot_process_instances(&mut session)
        .into_values()
        .find(|pi| {
            pi.super_execution_id
                .as_deref()
                .is_some_and(|id| id.starts_with(&parent_pi.id))
        })
        .expect("Child process instance should be created");
    drop(session);

    assert_eq!(child_pi.process_definition_key, "childExpressionTarget");
    assert_eq!(child_pi.business_key.as_deref(), Some("child-bk-9000"));
    assert_eq!(
        variable_service
            .get_variable(parent_pi.id.clone(), "childInstanceId".to_string())
            .unwrap(),
        Some(json!(child_pi.id.clone())),
        "flowable:processInstanceIdVariableName should store child process instance id on parent"
    );
    assert_eq!(
        variable_service
            .get_variable(child_pi.id.clone(), "childInput".to_string())
            .unwrap(),
        Some(json!("from-parent")),
        "flowable:in sourceExpression should copy parent variable to child"
    );
    assert_eq!(
        variable_service
            .get_variable(child_pi.id.clone(), "childTier".to_string())
            .unwrap(),
        Some(json!("gold")),
        "flowable:in targetExpression should name the child variable"
    );

    variable_service
        .set_variable(
            child_pi.id.clone(),
            "childResult".to_string(),
            json!("from-child-expression"),
        )
        .unwrap();

    let tasks = task_service
        .get_tasks_by_process_instance_id(child_pi.id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 1, "Should be at child task");
    task_service
        .complete_task_by_id(tasks[0].id.clone())
        .unwrap();

    assert_eq!(
        variable_service
            .get_variable(parent_pi.id.clone(), "parentResult".to_string())
            .unwrap(),
        Some(json!("from-child-expression")),
        "flowable:out sourceExpression should copy child variable back to parent"
    );
}

#[test]
fn test_call_activity_called_element_binding_deployment_resolves_same_deployment_definition() {
    let process_engine = ProcessEngine::new("default".to_string());
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();
    let task_service = process_engine.get_task_service();

    let parent_deployment = repository_service
        .deploy(
            repository_service
                .create_deployment()
                .name("same deployment parent and child".to_string())
                .tenant_id("acme".to_string())
                .add_string(
                    "parent_same_deployment.bpmn20.xml".to_string(),
                    PARENT_SAME_DEPLOYMENT_XML.to_string(),
                )
                .add_string(
                    "same_deployment_child_v1.bpmn20.xml".to_string(),
                    same_deployment_child_xml("sameDeploymentTaskV1"),
                ),
        )
        .unwrap();

    repository_service
        .deploy(
            repository_service
                .create_deployment()
                .name("later same deployment child".to_string())
                .tenant_id("acme".to_string())
                .add_string(
                    "same_deployment_child_v2.bpmn20.xml".to_string(),
                    same_deployment_child_xml("sameDeploymentTaskV2"),
                ),
        )
        .unwrap();

    let parent_definition = repository_service
        .get_process_definitions()
        .unwrap()
        .into_iter()
        .find(|definition| {
            definition.key == "parentSameDeployment"
                && definition.deployment_id.as_deref() == Some(parent_deployment.id.as_str())
        })
        .expect("Parent definition should be deployed");
    let same_deployment_child_definition = repository_service
        .get_process_definitions()
        .unwrap()
        .into_iter()
        .find(|definition| {
            definition.key == "sameDeploymentChild"
                && definition.deployment_id.as_deref() == Some(parent_deployment.id.as_str())
        })
        .expect("Same deployment child definition should be deployed");

    let builder = runtime_service
        .create_process_instance_builder()
        .process_definition_id(parent_definition.id);
    let parent_pi = runtime_service.start_process_instance(builder).unwrap();

    let store = process_engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let child_pi = store
        .snapshot_process_instances(&mut session)
        .into_values()
        .find(|pi| {
            pi.super_execution_id
                .as_deref()
                .is_some_and(|id| id.starts_with(&parent_pi.id))
        })
        .expect("Child process instance should be created");
    drop(session);

    assert_eq!(
        child_pi.process_definition_id,
        same_deployment_child_definition.id
    );
    assert_eq!(
        child_pi.process_definition_version,
        same_deployment_child_definition.version
    );

    let tasks = task_service
        .get_tasks_by_process_instance_id(child_pi.id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 1, "Should be at child task");
    assert_eq!(tasks[0].task_definition_key, "sameDeploymentTaskV1");
}
