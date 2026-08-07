//! P83 — BPMN → DMN tenant fallback and audit correlation.
//!
//! Java reference: `DmnActivityBehavior.java:99-103` (instanceId / executionId
//! / activityId on the ExecuteDecisionBuilder), `:167-175`
//! (`applyFallbackToDefaultTenant`, stringValue only) and
//! `PersistHistoricDecisionExecutionCmd.java:56-59` (history columns).

use flowable_bpmn_model::model::{
    BpmnModel, BusinessRuleTask, EndEvent, FieldExtension, FlowElementEnum, Process, SequenceFlow,
    ServiceTask, StartEvent,
};
use flowable_dmn_engine::{
    DmnDecision, DmnDeploymentRequest, DmnEngine, DmnHitPolicy, DmnInputClause, DmnModel,
    DmnOutputClause, DmnRule, DmnRuleInputEntry, DmnRuleOutputEntry, DmnUnaryTest,
    HistoricDecisionExecution,
};
use flowable_engine::engine::deployment_manager::DeploymentManager;
use flowable_engine::engine::runtime_service::RuntimeService;
use flowable_engine::error::FlowableError;
use flowable_engine::interceptor::command_executor::DefaultCommandExecutor;
use flowable_engine::persistence::db_store::DbStore;
use flowable_engine::persistence::runtime_store::RuntimeStore;
use flowable_engine::repository::process_definition::ProcessDefinition;
use flowable_engine::service::config::ProcessEngineConfiguration;
use flowable_http_service::DeterministicHttpRuntime;
use serde_json::json;
use std::sync::Arc;

// ── helpers ──────────────────────────────────────────────────────────────

fn field(name: &str, string_value: Option<&str>, expression: Option<&str>) -> FieldExtension {
    FieldExtension {
        field_name: Some(name.to_string()),
        string_value: string_value.map(str::to_string),
        expression: expression.map(str::to_string),
        ..Default::default()
    }
}

fn build_sequence_flow(id: &str, source_ref: &str, target_ref: &str) -> SequenceFlow {
    let mut flow = SequenceFlow::default();
    flow.flow_element.base_element.id = Some(id.to_string());
    flow.source_ref = Some(source_ref.to_string());
    flow.target_ref = Some(target_ref.to_string());
    flow
}

fn element_id(element: &FlowElementEnum) -> Option<String> {
    match element {
        FlowElementEnum::SequenceFlow(flow) => flow.flow_element.base_element.id.clone(),
        FlowElementEnum::ServiceTask(task) => task
            .task
            .activity
            .flow_node
            .flow_element
            .base_element
            .id
            .clone(),
        FlowElementEnum::BusinessRuleTask(task) => task
            .task
            .activity
            .flow_node
            .flow_element
            .base_element
            .id
            .clone(),
        FlowElementEnum::StartEvent(event) => {
            event.event.flow_node.flow_element.base_element.id.clone()
        }
        FlowElementEnum::EndEvent(event) => {
            event.event.flow_node.flow_element.base_element.id.clone()
        }
        _ => None,
    }
}

fn dish_decision(dish: &str) -> DmnDecision {
    DmnDecision::new(
        "decision-1",
        "dishDecision",
        "Dish decision",
        DmnHitPolicy::First,
        vec![DmnInputClause::new("input-1", "dishType")],
        vec![DmnOutputClause::new("output-1", "dish")],
        vec![DmnRule::new(
            "rule-1",
            vec![DmnRuleInputEntry::new(DmnUnaryTest::Any)],
            vec![DmnRuleOutputEntry::new(json!(dish))],
        )],
    )
}

/// start → task → end, where `task` is a dmn serviceTask or a businessRuleTask.
fn single_task_process(process_id: &str, task: FlowElementEnum) -> BpmnModel {
    let mut process = Process::default();
    process.base_element.id = Some(process_id.to_string());
    let flow1 = build_sequence_flow("flow1", "startEvent1", "decisionTask1");
    let flow2 = build_sequence_flow("flow2", "decisionTask1", "endEvent1");

    let mut start_event = StartEvent::default();
    start_event.event.flow_node.flow_element.base_element.id = Some("startEvent1".to_string());
    start_event.event.flow_node.outgoing_flows = vec![flow1.clone()];

    let mut end_event = EndEvent::default();
    end_event.event.flow_node.flow_element.base_element.id = Some("endEvent1".to_string());

    process.flow_elements = vec![
        FlowElementEnum::StartEvent(start_event),
        FlowElementEnum::SequenceFlow(flow1),
        task,
        FlowElementEnum::SequenceFlow(flow2),
        FlowElementEnum::EndEvent(end_event),
    ];
    for element in &process.flow_elements {
        if let Some(id) = element_id(element) {
            process.flow_element_map.insert(id, element.clone());
        }
    }

    BpmnModel {
        main_process: Some(process.clone()),
        processes: vec![process],
        ..Default::default()
    }
}

fn dmn_service_task(fields: Vec<FieldExtension>) -> FlowElementEnum {
    let mut task = ServiceTask::default();
    task.task.activity.flow_node.flow_element.base_element.id = Some("decisionTask1".to_string());
    task.task.activity.flow_node.outgoing_flows =
        vec![build_sequence_flow("flow2", "decisionTask1", "endEvent1")];
    task.task_type = Some("dmn".to_string());
    task.task.activity.field_extensions = fields;
    FlowElementEnum::ServiceTask(task)
}

fn business_rule_task() -> FlowElementEnum {
    let mut task = BusinessRuleTask::default();
    task.task.activity.flow_node.flow_element.base_element.id = Some("decisionTask1".to_string());
    task.task.activity.flow_node.outgoing_flows =
        vec![build_sequence_flow("flow2", "decisionTask1", "endEvent1")];
    task.decision_ref = Some("dishDecision".to_string());
    task.result_variable_name = Some("decisionResult".to_string());
    task.input_variables = vec!["dishType".to_string()];
    FlowElementEnum::BusinessRuleTask(task)
}

/// Starts a one-task process. `tenant_id` lands on the process definition, and
/// from there on the process instance and its executions.
fn run_process(
    dmn_engine: Arc<DmnEngine>,
    process_id: &str,
    task: FlowElementEnum,
    tenant_id: Option<&str>,
) -> Result<(RuntimeService, RuntimeStore, String), FlowableError> {
    let db_store = Arc::new(DbStore::new_in_memory().unwrap());
    let deployment_manager = DeploymentManager::new_with_memory_backend_for_test(db_store.clone());
    let runtime_store = RuntimeStore::new_with_memory_backend_for_test(db_store);

    let config = ProcessEngineConfiguration {
        dmn_engine: Some(dmn_engine),
        ..ProcessEngineConfiguration::default()
    };
    let command_executor = Arc::new(DefaultCommandExecutor::new(
        deployment_manager.clone(),
        runtime_store.clone(),
        Arc::new(config),
        Arc::new(DeterministicHttpRuntime::default()),
    ));
    let runtime_service =
        RuntimeService::new(command_executor, Arc::from(format!("p83-{process_id}")));

    deployment_manager.insert_bpmn_model(process_id, single_task_process(process_id, task));
    let mut session = deployment_manager.create_session().unwrap();
    deployment_manager.insert_process_definition(
        ProcessDefinition {
            id: process_id.to_string(),
            category: None,
            name: Some(process_id.to_string()),
            key: process_id.to_string(),
            description: None,
            version: 1,
            resource_name: Some(format!("{process_id}.bpmn20.xml")),
            deployment_id: Some(format!("{process_id}-deployment")),
            diagram_resource_name: None,
            has_start_form_key: false,
            has_graphical_notation: false,
            is_suspended: false,
            tenant_id: tenant_id.map(str::to_string),
            engine_version: None,
            app_version: None,
        history_level: None,
        },
        &mut session,
    );
    session.flush_and_commit().unwrap();

    let builder = runtime_service
        .create_process_instance_builder()
        .process_definition_id(process_id.to_string())
        .name(format!("{process_id}-instance"))
        .variable("dishType".to_string(), json!("salad"));
    let process_instance = runtime_service.start_process_instance(builder)?;
    Ok((runtime_service, runtime_store, process_instance.id))
}

fn history_rows(engine: &DmnEngine) -> Vec<HistoricDecisionExecution> {
    engine
        .history_service()
        .create_execution_history_query()
        .list()
        .expect("history query")
}

// ── task B: audit correlation from BPMN ──────────────────────────────────

#[test]
fn service_task_dmn_records_instance_execution_and_activity_ids() {
    let dmn = Arc::new(DmnEngine::new_in_memory().expect("dmn"));
    dmn.deploy(
        DmnDeploymentRequest::new("dish")
            .with_resource("dish.dmn", DmnModel::new(vec![dish_decision("light")])),
    )
    .expect("dmn deploy");

    let (_runtime, _store, process_instance_id) = run_process(
        dmn.clone(),
        "p83ServiceTaskAudit",
        dmn_service_task(vec![field(
            "decisionTableReferenceKey",
            Some("dishDecision"),
            None,
        )]),
        None,
    )
    .expect("process should complete");

    let rows = history_rows(&dmn);
    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    assert_eq!(row.instance_id.as_deref(), Some(process_instance_id.as_str()));
    assert_eq!(row.activity_id.as_deref(), Some("decisionTask1"));
    assert!(
        row.scope_execution_id.is_some(),
        "executionId should be recorded (Java DmnActivityBehavior.java:101)"
    );
}

#[test]
fn business_rule_task_records_instance_execution_and_activity_ids() {
    let dmn = Arc::new(DmnEngine::new_in_memory().expect("dmn"));
    dmn.deploy(
        DmnDeploymentRequest::new("dish")
            .with_resource("dish.dmn", DmnModel::new(vec![dish_decision("light")])),
    )
    .expect("dmn deploy");

    let (_runtime, _store, process_instance_id) = run_process(
        dmn.clone(),
        "p83BusinessRuleAudit",
        business_rule_task(),
        None,
    )
    .expect("process should complete");

    let rows = history_rows(&dmn);
    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    assert_eq!(row.instance_id.as_deref(), Some(process_instance_id.as_str()));
    assert_eq!(row.activity_id.as_deref(), Some("decisionTask1"));
    assert!(row.scope_execution_id.is_some());
}

// ── task A: fallbackToDefaultTenant on the dmn serviceTask ───────────────

/// Decision exists only in the default (untenanted) deployment; the process
/// runs under `tenant-a`. Without the field the lookup must fail.
#[test]
fn service_task_dmn_without_fallback_field_fails_for_other_tenant() {
    let dmn = Arc::new(DmnEngine::new_in_memory().expect("dmn"));
    dmn.deploy(
        DmnDeploymentRequest::new("dish")
            .with_resource("dish.dmn", DmnModel::new(vec![dish_decision("default")])),
    )
    .expect("dmn deploy");

    let error = run_process(
        dmn.clone(),
        "p83NoFallback",
        dmn_service_task(vec![field(
            "decisionTableReferenceKey",
            Some("dishDecision"),
            None,
        )]),
        Some("tenant-a"),
    )
    .map(|_| ())
    .expect_err("tenant-a has no dishDecision");

    assert!(
        error.to_string().contains("was not found"),
        "unexpected error: {error}"
    );
}

#[test]
fn service_task_dmn_with_fallback_field_resolves_default_tenant_decision() {
    let dmn = Arc::new(DmnEngine::new_in_memory().expect("dmn"));
    dmn.deploy(
        DmnDeploymentRequest::new("dish")
            .with_resource("dish.dmn", DmnModel::new(vec![dish_decision("default")])),
    )
    .expect("dmn deploy");

    let (runtime, _store, process_instance_id) = run_process(
        dmn.clone(),
        "p83Fallback",
        dmn_service_task(vec![
            field("decisionTableReferenceKey", Some("dishDecision"), None),
            field("fallbackToDefaultTenant", Some("true"), None),
        ]),
        Some("tenant-a"),
    )
    .expect("fallback should resolve the untenanted decision");

    let dish = runtime
        .get_variable(process_instance_id, "dish".to_string())
        .expect("variable query")
        .expect("dish variable");
    assert_eq!(dish, json!("default"));
}

/// Java reads `fallbackToDefaultTenant` from `stringValue` only
/// (`DmnActivityBehavior.java:169-172`); an `expression` is never evaluated for
/// this field, so it must not switch the fallback on.
#[test]
fn service_task_dmn_fallback_field_ignores_expression_attribute() {
    let dmn = Arc::new(DmnEngine::new_in_memory().expect("dmn"));
    dmn.deploy(
        DmnDeploymentRequest::new("dish")
            .with_resource("dish.dmn", DmnModel::new(vec![dish_decision("default")])),
    )
    .expect("dmn deploy");

    let error = run_process(
        dmn.clone(),
        "p83FallbackExpression",
        dmn_service_task(vec![
            field("decisionTableReferenceKey", Some("dishDecision"), None),
            field("fallbackToDefaultTenant", None, Some("${true}")),
        ]),
        Some("tenant-a"),
    )
    .map(|_| ())
    .expect_err("expression form must not enable the fallback");

    assert!(
        error.to_string().contains("was not found"),
        "unexpected error: {error}"
    );
}

/// A tenant-owned decision still wins over the default-tenant one.
#[test]
fn service_task_dmn_fallback_prefers_tenant_owned_decision() {
    let dmn = Arc::new(DmnEngine::new_in_memory().expect("dmn"));
    dmn.deploy(
        DmnDeploymentRequest::new("dish-default")
            .with_resource("dish.dmn", DmnModel::new(vec![dish_decision("default")])),
    )
    .expect("default deploy");
    dmn.deploy(
        DmnDeploymentRequest::new("dish-tenant-a")
            .with_tenant_id("tenant-a")
            .with_resource("dish.dmn", DmnModel::new(vec![dish_decision("tenant-a")])),
    )
    .expect("tenant deploy");

    let (runtime, _store, process_instance_id) = run_process(
        dmn.clone(),
        "p83FallbackTenantWins",
        dmn_service_task(vec![
            field("decisionTableReferenceKey", Some("dishDecision"), None),
            field("fallbackToDefaultTenant", Some("true"), None),
        ]),
        Some("tenant-a"),
    )
    .expect("tenant decision resolves without the fallback");

    let dish = runtime
        .get_variable(process_instance_id, "dish".to_string())
        .expect("variable query")
        .expect("dish variable");
    assert_eq!(dish, json!("tenant-a"));
}
