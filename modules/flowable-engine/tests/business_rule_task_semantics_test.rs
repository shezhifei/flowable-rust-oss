use std::sync::Arc;

use flowable_bpmn_model::model::{
    BpmnModel, BusinessRuleTask, EndEvent, FlowElementEnum, Process, SequenceFlow, StartEvent,
};
use flowable_engine::engine::deployment_manager::DeploymentManager;
use flowable_engine::engine::runtime_service::RuntimeService;
use flowable_engine::interceptor::command_executor::DefaultCommandExecutor;
use flowable_engine::persistence::db_store::DbStore;
use flowable_engine::persistence::runtime_store::RuntimeStore;
use flowable_engine::repository::process_definition::ProcessDefinition;
use flowable_engine::service::config::ProcessEngineConfiguration;
use flowable_http_service::DeterministicHttpRuntime;

fn element_id(element: &FlowElementEnum) -> Option<String> {
    match element {
        FlowElementEnum::SequenceFlow(flow) => flow.flow_element.base_element.id.clone(),
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

fn build_sequence_flow(id: &str, source_ref: &str, target_ref: &str) -> SequenceFlow {
    let mut flow = SequenceFlow::default();
    flow.flow_element.base_element.id = Some(id.to_string());
    flow.source_ref = Some(source_ref.to_string());
    flow.target_ref = Some(target_ref.to_string());
    flow
}

#[test]
fn business_rule_task_requires_decision_ref_in_owned_m15_path() {
    let db_store = Arc::new(DbStore::new_in_memory().unwrap());
    let deployment_manager = DeploymentManager::new_with_memory_backend_for_test(db_store.clone());
    let runtime_store = RuntimeStore::new_with_memory_backend_for_test(db_store);
    let runtime_store_for_executor = runtime_store.clone();
    let command_executor = Arc::new(DefaultCommandExecutor::new(
        deployment_manager.clone(),
        runtime_store_for_executor,
        Arc::new(ProcessEngineConfiguration::default()),
        Arc::new(DeterministicHttpRuntime::default()),
    ));
    let runtime_service =
        RuntimeService::new(command_executor, Arc::from("business-rule-test-owner"));

    let process_definition_id = "businessRuleTaskProcess".to_string();

    let mut process = Process::default();
    process.base_element.id = Some(process_definition_id.clone());
    let flow1 = build_sequence_flow("flow1", "startEvent1", "businessRuleTask1");
    let flow2 = build_sequence_flow("flow2", "businessRuleTask1", "endEvent1");
    let mut start_event = StartEvent::default();
    start_event.event.flow_node.flow_element.base_element.id = Some("startEvent1".to_string());
    start_event.event.flow_node.outgoing_flows = vec![flow1.clone()];
    let mut business_rule_task = BusinessRuleTask::default();
    business_rule_task
        .task
        .activity
        .flow_node
        .flow_element
        .base_element
        .id = Some("businessRuleTask1".to_string());
    business_rule_task.task.activity.flow_node.flow_element.name =
        Some("Evaluate Rule".to_string());
    business_rule_task.task.activity.flow_node.outgoing_flows = vec![flow2.clone()];
    let mut end_event = EndEvent::default();
    end_event.event.flow_node.flow_element.base_element.id = Some("endEvent1".to_string());
    process.flow_elements = vec![
        FlowElementEnum::StartEvent(start_event),
        FlowElementEnum::SequenceFlow(flow1),
        FlowElementEnum::BusinessRuleTask(business_rule_task),
        FlowElementEnum::SequenceFlow(flow2),
        FlowElementEnum::EndEvent(end_event),
    ];
    for element in &process.flow_elements {
        if let Some(id) = element_id(element) {
            process.flow_element_map.insert(id, element.clone());
        }
    }

    let bpmn_model = BpmnModel {
        main_process: Some(process),
        ..Default::default()
    };

    deployment_manager.insert_bpmn_model(&process_definition_id, bpmn_model);
    let mut session = deployment_manager.create_session().unwrap();
    deployment_manager.insert_process_definition(
        ProcessDefinition {
            id: process_definition_id.clone(),
            category: None,
            name: Some("Business Rule Task Process".to_string()),
            key: process_definition_id.clone(),
            description: None,
            version: 1,
            resource_name: Some("business-rule-task.bpmn20.xml".to_string()),
            deployment_id: Some("business-rule-task-deployment".to_string()),
            diagram_resource_name: None,
            has_start_form_key: false,
            has_graphical_notation: false,
            is_suspended: false,
            tenant_id: None,
            engine_version: None,
            app_version: None,
        history_level: None,
        },
        &mut session,
    );
    session.flush_and_commit().unwrap();

    let builder = runtime_service
        .create_process_instance_builder()
        .process_definition_id(process_definition_id)
        .name("Business Rule Task Instance".to_string());

    let error = runtime_service
        .start_process_instance(builder)
        .expect_err("businessRuleTask without decisionRef must fail in M15");

    assert!(
        error.to_string().contains("missing decisionRef"),
        "unexpected error: {error}"
    );
}
