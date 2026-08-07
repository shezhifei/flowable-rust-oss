use std::sync::Arc;

use flowable_bpmn_model::model::{
    BpmnModel, EndEvent, FlowElementEnum, ManualTask, Process, SequenceFlow, StartEvent,
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
        FlowElementEnum::ManualTask(task) => task
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
fn manual_task_passes_through_to_end_event() {
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
        RuntimeService::new(command_executor, Arc::from("manual-task-test-owner"));

    let process_definition_id = "manualTaskProcess".to_string();

    let mut process = Process::default();
    process.base_element.id = Some(process_definition_id.clone());
    let flow1 = build_sequence_flow("flow1", "startEvent1", "manualTask1");
    let flow2 = build_sequence_flow("flow2", "manualTask1", "endEvent1");
    let mut start_event = StartEvent::default();
    start_event.event.flow_node.flow_element.base_element.id = Some("startEvent1".to_string());
    start_event.event.flow_node.outgoing_flows = vec![flow1.clone()];
    let mut manual_task = ManualTask::default();
    manual_task
        .task
        .activity
        .flow_node
        .flow_element
        .base_element
        .id = Some("manualTask1".to_string());
    manual_task.task.activity.flow_node.flow_element.name = Some("Review Manually".to_string());
    manual_task.task.activity.flow_node.outgoing_flows = vec![flow2.clone()];
    let mut end_event = EndEvent::default();
    end_event.event.flow_node.flow_element.base_element.id = Some("endEvent1".to_string());
    process.flow_elements = vec![
        FlowElementEnum::StartEvent(start_event),
        FlowElementEnum::SequenceFlow(flow1),
        FlowElementEnum::ManualTask(manual_task),
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
            name: Some("Manual Task Process".to_string()),
            key: process_definition_id.clone(),
            description: None,
            version: 1,
            resource_name: Some("manual-task.bpmn20.xml".to_string()),
            deployment_id: Some("manual-task-deployment".to_string()),
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
        .name("Manual Task Instance".to_string());

    let process_instance = runtime_service.start_process_instance(builder).unwrap();

    let mut session = runtime_store.create_session().unwrap();
    let stored_pi = runtime_store
        .find_process_instance(&process_instance.id, &mut session)
        .expect("Process instance should be in runtime store");
    assert!(
        stored_pi.is_ended,
        "Process instance should be ended after manual task pass-through"
    );
    drop(session);
}
