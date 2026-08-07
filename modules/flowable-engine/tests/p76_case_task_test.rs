//! P76 — BPMN `caseServiceTask` full chain contract tests.
//!
//! Java parity:
//! - CaseTaskActivityBehavior.java (start case, in/out params, leave on complete)
//! - ServiceTaskXMLConverter.java:123-124 (type="case" → CaseServiceTask)
//! - DefaultCaseInstanceService.java:54-95
//! - ChildBpmnCaseInstanceStateChangeCallback.java:50-88

use flowable_cmmn_engine::{
    CmmnCase, CmmnCasePlanModel, CmmnDeploymentRequest, CmmnHumanTask,
    CmmnHumanTaskCompletionRequest, CmmnModel, CmmnPlanItem,
};
use flowable_engine::engine::process_engine::ProcessEngine;
use serde_json::json;

const BPMN_CASE_SERVICE_TASK: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
  <process id="caseServiceProcess" isExecutable="true">
    <startEvent id="start"/>
    <sequenceFlow id="f1" sourceRef="start" targetRef="caseTask"/>
    <serviceTask id="caseTask" name="Case task" flowable:type="case"
                 flowable:caseDefinitionKey="childCase"
                 flowable:businessKey="${processBusinessKey}"
                 flowable:idVariableName="startedCaseId">
      <extensionElements>
        <flowable:in source="parentInput" target="childInput"/>
        <flowable:in sourceExpression="${'literal-in'}" target="childLiteral"/>
        <flowable:out source="childResult" target="parentResult"/>
      </extensionElements>
    </serviceTask>
    <sequenceFlow id="f2" sourceRef="caseTask" targetRef="afterCase"/>
    <userTask id="afterCase" name="After case"/>
    <sequenceFlow id="f3" sourceRef="afterCase" targetRef="end"/>
    <endEvent id="end"/>
  </process>
</definitions>"#;

const BPMN_CASE_EL_KEY: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
  <process id="caseElKeyProcess" isExecutable="true">
    <startEvent id="start"/>
    <sequenceFlow id="f1" sourceRef="start" targetRef="caseTask"/>
    <serviceTask id="caseTask" flowable:type="case"
                 flowable:caseDefinitionKey="${caseKey}">
      <extensionElements>
        <flowable:in source="seed" target="seed"/>
        <flowable:out source="seed" target="echo"/>
      </extensionElements>
    </serviceTask>
    <sequenceFlow id="f2" sourceRef="caseTask" targetRef="afterCase"/>
    <userTask id="afterCase" name="After"/>
    <sequenceFlow id="f3" sourceRef="afterCase" targetRef="end"/>
    <endEvent id="end"/>
  </process>
</definitions>"#;

fn child_case_model() -> CmmnModel {
    CmmnModel::new(vec![CmmnCase::new(
        "case-child",
        "childCase",
        "Child case",
        CmmnCasePlanModel::new("child-plan-model", "Child plan")
            .with_human_task(CmmnHumanTask::new("human-task-child", "Child work"))
            .with_plan_item(CmmnPlanItem::new("plan-item-child", "human-task-child")),
    )])
}

fn auto_case_model() -> CmmnModel {
    // Empty plan with autoComplete completes immediately on start.
    CmmnModel::new(vec![CmmnCase::new(
        "case-auto",
        "autoCase",
        "Auto case",
        CmmnCasePlanModel::new("auto-plan-model", "Auto plan").with_auto_complete(true),
    )])
}

fn deploy_bpmn(engine: &ProcessEngine, name: &str, xml: &str) {
    let repository_service = engine.get_repository_service();
    let deployment = repository_service
        .create_deployment()
        .name(name.to_string())
        .add_string(format!("{name}.bpmn20.xml"), xml.to_string());
    repository_service.deploy(deployment).expect("deploy bpmn");
}

fn deploy_cmmn(engine: &ProcessEngine, name: &str, model: CmmnModel) {
    let cmmn = engine
        .get_config()
        .cmmn_engine
        .as_ref()
        .expect("cmmn engine")
        .clone();
    cmmn.deploy(CmmnDeploymentRequest::new(name).with_resource(format!("{name}.cmmn"), model))
        .expect("deploy cmmn");
}

#[test]
fn p76_case_service_task_starts_case_maps_in_out_and_continues() {
    let engine = ProcessEngine::new_with_memory_backend("p76-case-task".into());
    deploy_bpmn(&engine, "p76-case-service", BPMN_CASE_SERVICE_TASK);
    deploy_cmmn(&engine, "p76-child-case", child_case_model());

    let runtime = engine.get_runtime_service();
    let task_service = engine.get_task_service();
    let variable_service = engine.get_variable_service();

    let builder = runtime
        .create_process_instance_builder()
        .process_definition_key("caseServiceProcess".to_string())
        .business_key("bk-from-process".to_string())
        .variable("parentInput".to_string(), json!("from-parent"))
        .variable("processBusinessKey".to_string(), json!("bk-expr-value"));
    let pi = runtime
        .start_process_instance(builder)
        .expect("start process");

    // Process waits on caseServiceTask (blocking).
    let tasks_before = task_service
        .get_tasks_by_process_instance_id(pi.id.clone())
        .expect("bpmn tasks");
    assert!(
        tasks_before.is_empty(),
        "process must wait on caseServiceTask; got {:?}",
        tasks_before
            .iter()
            .map(|t| t.task_definition_key.clone())
            .collect::<Vec<_>>()
    );

    let cmmn = engine.get_config().cmmn_engine.as_ref().unwrap().clone();
    let case_instances = cmmn
        .runtime_service()
        .create_case_instance_query()
        .case_definition_key("childCase")
        .list()
        .expect("list cases");
    assert_eq!(case_instances.len(), 1, "exactly one child case");
    let case_instance = &case_instances[0];
    assert_eq!(
        case_instance.variables.get("childInput"),
        Some(&json!("from-parent")),
        "in-parameter parentInput→childInput"
    );
    assert_eq!(
        case_instance.variables.get("childLiteral"),
        Some(&json!("literal-in")),
        "in-parameter sourceExpression"
    );
    assert_eq!(
        case_instance.business_key.as_deref(),
        Some("bk-expr-value"),
        "businessKey expression from process"
    );
    assert_eq!(
        case_instance.callback_type.as_deref(),
        Some(flowable_cmmn_engine::CMMN_EXECUTION_CHILD_CASE_CALLBACK_TYPE)
    );

    assert_eq!(
        variable_service
            .get_variable(pi.id.clone(), "startedCaseId".to_string())
            .expect("var"),
        Some(json!(case_instance.id)),
        "caseInstanceIdVariableName written"
    );

    // Set out-source variable then complete child human task.
    cmmn.runtime_service()
        .set_case_instance_variables(
            &case_instance.id,
            vec![("childResult".to_string(), json!("from-child"))],
        )
        .expect("set case variables");

    let human_tasks = cmmn
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_instance.id)
        .list()
        .expect("human tasks");
    assert_eq!(human_tasks.len(), 1);
    cmmn.complete_human_task(
        &human_tasks[0].id,
        CmmnHumanTaskCompletionRequest::new().with_completed_by("tester"),
    )
    .expect("complete human task");

    let tasks_after = task_service
        .get_tasks_by_process_instance_id(pi.id.clone())
        .expect("bpmn tasks after");
    assert_eq!(tasks_after.len(), 1);
    assert_eq!(tasks_after[0].task_definition_key, "afterCase");

    assert_eq!(
        variable_service
            .get_variable(pi.id.clone(), "parentResult".to_string())
            .expect("out var"),
        Some(json!("from-child")),
        "out-parameter childResult→parentResult"
    );
}

#[test]
fn p76_case_definition_key_el_and_sync_auto_complete_case() {
    let engine = ProcessEngine::new_with_memory_backend("p76-case-el".into());
    deploy_bpmn(&engine, "p76-case-el", BPMN_CASE_EL_KEY);
    deploy_cmmn(&engine, "p76-auto-case", auto_case_model());

    let runtime = engine.get_runtime_service();
    let task_service = engine.get_task_service();
    let variable_service = engine.get_variable_service();

    let builder = runtime
        .create_process_instance_builder()
        .process_definition_key("caseElKeyProcess".to_string())
        .variable("caseKey".to_string(), json!("autoCase"))
        .variable("seed".to_string(), json!(42));
    let pi = runtime.start_process_instance(builder).expect("start");

    let tasks = task_service
        .get_tasks_by_process_instance_id(pi.id.clone())
        .expect("tasks");
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].task_definition_key, "afterCase");

    assert_eq!(
        variable_service
            .get_variable(pi.id.clone(), "echo".to_string())
            .expect("echo"),
        Some(json!(42))
    );
}

#[test]
fn p76_converter_materializes_case_service_task() {
    use flowable_bpmn_converter::BpmnXMLConverter;
    use flowable_bpmn_model::model::FlowElementEnum;

    let model = BpmnXMLConverter::new()
        .try_convert_to_bpmn_model(BPMN_CASE_SERVICE_TASK)
        .expect("convert");
    let process = model.main_process.expect("main process");
    let element = process
        .flow_element_map
        .get("caseTask")
        .or_else(|| {
            process.flow_elements.iter().find(|e| {
                matches!(
                    e,
                    FlowElementEnum::CaseServiceTask(t) if t.activity_id() == Some("caseTask")
                )
            })
        })
        .expect("caseTask element");
    match element {
        FlowElementEnum::CaseServiceTask(task) => {
            assert_eq!(task.case_definition_key.as_deref(), Some("childCase"));
            assert_eq!(task.service_task.task_type.as_deref(), Some("case"));
            assert_eq!(task.in_parameters().len(), 2);
            assert_eq!(task.out_parameters().len(), 1);
            assert_eq!(
                task.case_instance_id_variable_name.as_deref(),
                Some("startedCaseId")
            );
        }
        other => panic!("expected CaseServiceTask, got {:?}", other),
    }
}
