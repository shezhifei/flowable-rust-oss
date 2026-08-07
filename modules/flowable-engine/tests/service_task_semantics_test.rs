use flowable_bpmn_converter::BpmnXMLConverter;
use flowable_bpmn_model::model::{BpmnModel, FlowElementEnum, Process};
use flowable_engine::engine::outbound_event_dispatch::{
    OutboundEventDispatchHandle, OutboundEventDispatchHook, OutboundEventDispatchRequest,
};
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::engine::time_source::TestTimeSource;
use flowable_engine::error::FlowableError;
use flowable_engine::interceptor::command_context::CommandContext;
use flowable_engine::persistence::db_store::DbStore;
use flowable_engine::cmd::trigger_send_event_service_task_cmd::TriggerSendEventServiceTaskCmd;
use flowable_engine::interceptor::command::Command;
use flowable_engine::persistence::runtime_store::{
    EventRegistryChannelDefinition, EventRegistryEventDefinition, EventRegistryEventDirection,
    EventRegistryEventInstanceStatus, EventSubscriptionKind, RuntimeEventWaitKind, RuntimeStore,
};
use flowable_engine::runtime::execution::Execution;
use flowable_engine::service::config::{
    HttpServiceRuntimeMode, HttpServiceTaskConfiguration, ProcessEngineConfiguration,
    RealHttpClientConfiguration,
};
use flowable_engine::validation::unsupported_model_validator::UnsupportedModelValidator;
use flowable_engine::{
    agenda::FlowableEngineAgenda,
    bpmn::behavior::service_task_activity_behavior::{
        LocalServiceTaskDelegate, LocalServiceTaskDelegateContext,
        LocalServiceTaskDelegateRegistry, SERVICE_TASK_DELEGATE_REGISTRY_CACHE_KEY,
        ServiceTaskActivityBehavior,
    },
    delegate::activity_behavior::{ActivityBehavior, TriggerableActivityBehavior},
    engine::deployment_manager::DeploymentManager,
};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};

const SKIP_SERVICE_TASK_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
    <process id="skipServiceTaskProcess" name="Skip Service Task Process">
        <startEvent id="startEvent1" />
        <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="httpTask1" />
        <serviceTask id="httpTask1"
                     name="Maybe Invoke HTTP"
                     flowable:type="http"
                     flowable:skipExpression="${skipService}"
                     flowable:resultVariableName="httpResult">
            <extensionElements>
                <flowable:requestMethod>GET</flowable:requestMethod>
                <flowable:requestUrl>https://example.flowable.local/skip-test</flowable:requestUrl>
            </extensionElements>
        </serviceTask>
        <sequenceFlow id="flow2" sourceRef="httpTask1" targetRef="userTask1" />
        <userTask id="userTask1" name="After Skip" />
    </process>
</definitions>"#;

const SEND_EVENT_TASK_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
    <process id="sendEventSkipProcess" name="Send Event Skip Process">
        <startEvent id="startEvent1" />
        <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="sendEventTask1" />
        <serviceTask id="sendEventTask1"
                     name="Maybe Publish Event"
                     flowable:type="send-event"
                     flowable:skipExpression="${skipSendEvent}">
            <extensionElements>
                <flowable:eventType>orderPublished</flowable:eventType>
                <flowable:eventInParameter sourceExpression="${orderId}" target="orderId" />
            </extensionElements>
        </serviceTask>
        <sequenceFlow id="flow2" sourceRef="sendEventTask1" targetRef="userTask1" />
        <userTask id="userTask1" name="After Send Event" />
    </process>
</definitions>"#;

const SEND_AND_RECEIVE_EVENT_TASK_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
    <process id="sendAndReceiveEventProcess" name="Send And Receive Event Process">
        <startEvent id="startEvent1" />
        <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="sendEventTask1" />
        <serviceTask id="sendEventTask1"
                     name="Publish And Await Acceptance"
                     flowable:type="send-event"
                     flowable:triggerable="true">
            <extensionElements>
                <flowable:eventType>orderPublished</flowable:eventType>
                <flowable:triggerEventType>orderAccepted</flowable:triggerEventType>
                <flowable:eventInParameter sourceExpression="${orderId}" target="orderId" />
                <flowable:eventOutParameter source="acceptedBy" target="acceptedBy" />
                <flowable:out source="payload.acceptedBy" target="acceptedByGenericOut" />
            </extensionElements>
        </serviceTask>
        <sequenceFlow id="flow2" sourceRef="sendEventTask1" targetRef="userTask1" />
        <userTask id="userTask1" name="After Send And Receive" />
    </process>
</definitions>"#;

const HTTP_SERVICE_TASK_IO_TRANSIENT_RESULT_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
    <process id="httpServiceTaskIoProcess" name="HTTP Service Task IO Process">
        <startEvent id="startEvent1" />
        <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="httpTask1" />
        <serviceTask id="httpTask1"
                     name="Invoke HTTP"
                     flowable:type="http"
                     flowable:resultVariableName="httpResult"
                     flowable:storeResultVariableAsTransient="true">
            <extensionElements>
                <flowable:requestMethod>POST</flowable:requestMethod>
                <flowable:requestUrl>https://example.flowable.local/io-test</flowable:requestUrl>
                <flowable:in source="customerId" target="requestCustomer" />
                <flowable:in sourceExpression="${'gold'}" target="requestTier" />
                <flowable:out sourceExpression="${requestCustomer}" target="copiedCustomer" />
                <flowable:out sourceExpression="${requestTier}" target="copiedTier" />
                <flowable:out source="response.statusCode" target="httpStatusCode" />
                <flowable:out sourceExpression="${httpResult}" target="copiedHttpResult" />
            </extensionElements>
        </serviceTask>
        <sequenceFlow id="flow2" sourceRef="httpTask1" targetRef="userTask1" />
        <userTask id="userTask1" name="After HTTP" />
    </process>
</definitions>"#;

const HTTP_SERVICE_TASK_LOCAL_RESULT_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
    <process id="httpServiceTaskLocalResultProcess" name="HTTP Service Task Local Result Process">
        <startEvent id="startEvent1" />
        <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="httpTask1" />
        <serviceTask id="httpTask1"
                     name="Invoke HTTP"
                     flowable:type="http"
                     flowable:resultVariableName="httpResult"
                     flowable:useLocalScopeForResultVariable="true">
            <extensionElements>
                <flowable:requestMethod>GET</flowable:requestMethod>
                <flowable:requestUrl>https://example.flowable.local/local-result</flowable:requestUrl>
                <flowable:out sourceExpression="${httpResult}" target="copiedHttpResult" />
            </extensionElements>
        </serviceTask>
        <sequenceFlow id="flow2" sourceRef="httpTask1" targetRef="userTask1" />
        <userTask id="userTask1" name="After HTTP" />
    </process>
</definitions>"#;

const DELEGATE_EXPRESSION_SERVICE_TASK_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
    <process id="delegateExpressionProcess" name="Delegate Expression Process">
        <startEvent id="startEvent1" />
        <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="delegateTask1" />
        <serviceTask id="delegateTask1"
                     name="Invoke Local Delegate"
                     flowable:delegateExpression="${delegateName}"
                     flowable:resultVariableName="delegateResult">
            <extensionElements>
                <flowable:field name="literalGreeting" stringValue="hello" />
                <flowable:field name="customerFromExpression" expression="${customerId}" />
                <flowable:out source="writtenByDelegate" target="delegateWriteBack" />
                <flowable:out source="fields.customerFromExpression" target="delegateFieldCopy" />
            </extensionElements>
        </serviceTask>
        <sequenceFlow id="flow2" sourceRef="delegateTask1" targetRef="userTask1" />
        <userTask id="userTask1" name="After Delegate" />
    </process>
</definitions>"#;

// P51 S4 — Java ServiceTaskDelegateExpressionActivityBehavior:79-126,179-181
// Any class/delegateExpression service task may set triggerable; execute does not leave.
const TRIGGERABLE_DELEGATE_EXPRESSION_SERVICE_TASK_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
    <process id="triggerableDelegateProcess" name="Triggerable Delegate Process">
        <startEvent id="startEvent1" />
        <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="delegateTask1" />
        <serviceTask id="delegateTask1"
                     name="Invoke Triggerable Delegate"
                     flowable:delegateExpression="${delegateName}"
                     flowable:triggerable="true"
                     flowable:resultVariableName="delegateResult">
            <extensionElements>
                <flowable:field name="literalGreeting" stringValue="hello" />
                <flowable:field name="customerFromExpression" expression="${customerId}" />
            </extensionElements>
        </serviceTask>
        <sequenceFlow id="flow2" sourceRef="delegateTask1" targetRef="userTask1" />
        <userTask id="userTask1" name="After Trigger" />
    </process>
</definitions>"#;

const TRIGGERABLE_CLASS_SERVICE_TASK_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
    <process id="triggerableClassProcess" name="Triggerable Class Process">
        <startEvent id="startEvent1" />
        <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="delegateTask1" />
        <serviceTask id="delegateTask1"
                     name="Invoke Triggerable Class Delegate"
                     flowable:class="com.example.TriggerableDelegate"
                     flowable:triggerable="true"
                     flowable:resultVariableName="delegateResult">
            <extensionElements>
                <flowable:field name="literalGreeting" stringValue="class-hello" />
                <flowable:field name="customerFromExpression" expression="${customerId}" />
            </extensionElements>
        </serviceTask>
        <sequenceFlow id="flow2" sourceRef="delegateTask1" targetRef="userTask1" />
        <userTask id="userTask1" name="After Class Trigger" />
    </process>
</definitions>"#;

fn process_service_task_mut<'a>(
    process: &'a mut Process,
    activity_id: &str,
) -> Option<&'a mut flowable_bpmn_model::model::ServiceTask> {
    process
        .flow_elements
        .iter_mut()
        .find_map(|element| match element {
            FlowElementEnum::ServiceTask(service_task)
                if service_task
                    .task
                    .activity
                    .flow_node
                    .flow_element
                    .base_element
                    .id
                    .as_deref()
                    == Some(activity_id) =>
            {
                Some(service_task)
            }
            _ => None,
        })
}

fn process_service_task<'a>(
    process: &'a Process,
    activity_id: &str,
) -> Option<&'a flowable_bpmn_model::model::ServiceTask> {
    process
        .flow_elements
        .iter()
        .find_map(|element| match element {
            FlowElementEnum::ServiceTask(service_task)
                if service_task
                    .task
                    .activity
                    .flow_node
                    .flow_element
                    .base_element
                    .id
                    .as_deref()
                    == Some(activity_id) =>
            {
                Some(service_task)
            }
            _ => None,
        })
}

fn service_task<'a>(
    model: &'a BpmnModel,
    activity_id: &str,
) -> &'a flowable_bpmn_model::model::ServiceTask {
    model
        .main_process
        .as_ref()
        .and_then(|process| process_service_task(process, activity_id))
        .or_else(|| {
            model
                .processes
                .iter()
                .find_map(|process| process_service_task(process, activity_id))
        })
        .expect("service task should be present in parsed model")
}

fn skip_service_task_model() -> BpmnModel {
    let converter = BpmnXMLConverter::new();
    converter.convert_to_bpmn_model(SKIP_SERVICE_TASK_XML)
}

fn send_event_task_model() -> BpmnModel {
    let converter = BpmnXMLConverter::new();
    converter.convert_to_bpmn_model(SEND_EVENT_TASK_XML)
}

fn send_and_receive_event_task_model() -> BpmnModel {
    let converter = BpmnXMLConverter::new();
    converter.convert_to_bpmn_model(SEND_AND_RECEIVE_EVENT_TASK_XML)
}

fn http_service_task_io_transient_result_model() -> BpmnModel {
    let converter = BpmnXMLConverter::new();
    converter.convert_to_bpmn_model(HTTP_SERVICE_TASK_IO_TRANSIENT_RESULT_XML)
}

fn http_service_task_local_result_model() -> BpmnModel {
    let converter = BpmnXMLConverter::new();
    converter.convert_to_bpmn_model(HTTP_SERVICE_TASK_LOCAL_RESULT_XML)
}

fn delegate_expression_service_task_model() -> BpmnModel {
    let converter = BpmnXMLConverter::new();
    converter.convert_to_bpmn_model(DELEGATE_EXPRESSION_SERVICE_TASK_XML)
}

struct RecordingDelegate;

impl LocalServiceTaskDelegate for RecordingDelegate {
    fn execute(
        &self,
        context: &mut LocalServiceTaskDelegateContext<'_>,
    ) -> Result<Value, FlowableError> {
        let customer = context
            .fields
            .get("customerFromExpression")
            .cloned()
            .unwrap_or(Value::Null);
        context
            .execution
            .set_process_variable("writtenByDelegate".to_string(), customer.clone());
        Ok(json!({
            "delegate": "recording",
            "activityId": context.service_task_id,
            "fields": context.fields,
            "writtenByDelegate": customer,
        }))
    }
}

fn run_owned_http_service_task_with_skip_value_and_enabled_flag(
    skip_service: serde_json::Value,
    skip_enabled: bool,
) -> Result<CommandContext, FlowableError> {
    let db_store = Arc::new(DbStore::new_in_memory().unwrap());
    let deployment_manager = DeploymentManager::new_with_memory_backend_for_test(db_store.clone());
    deployment_manager.insert_bpmn_model("skip-service-task-process:1", skip_service_task_model());

    let runtime_store = RuntimeStore::new_with_memory_backend_for_test(db_store);
    let session = runtime_store.create_session().unwrap();
    let config = Arc::new(ProcessEngineConfiguration {
        http_service: HttpServiceTaskConfiguration {
            enabled: false,
            ..Default::default()
        },
        ..Default::default()
    });
    let mut command_context = CommandContext::new(
        deployment_manager,
        runtime_store,
        session,
        config,
        Arc::new(flowable_http_service::DeterministicHttpRuntime::default()),
    );

    let mut execution = Execution {
        id: "skip-service-task-process-instance".to_string(),
        root_process_instance_id: Some("skip-service-task-process-instance".to_string()),
        process_instance_id: Some("skip-service-task-process-instance".to_string()),
        process_definition_id: Some("skip-service-task-process:1".to_string()),
        process_definition_key: Some("skipServiceTaskProcess".to_string()),
        activity_id: Some("httpTask1".to_string()),
        is_active: true,
        is_scope: true,
        ..Default::default()
    };
    execution.set_process_variable("skipService".to_string(), skip_service);
    execution.set_process_variable(
        "_FLOWABLE_SKIP_EXPRESSION_ENABLED".to_string(),
        serde_json::Value::Bool(skip_enabled),
    );
    {
        let (store, sess) = command_context.store_and_session();
        store.insert_execution(&execution, sess);
    }

    ServiceTaskActivityBehavior::new().execute(&mut execution, &mut command_context)?;
    while let Some(operation) = command_context.agenda().pop_operation() {
        operation.run(&mut command_context)?;
    }
    command_context.session().flush_and_commit()?;

    Ok(command_context)
}

fn run_send_event_task_with_skip_and_enabled_variable(
    skip_send_event: Value,
    skip_enabled: Option<Value>,
) -> Result<CommandContext, FlowableError> {
    run_send_event_task_with_tenants(skip_send_event, skip_enabled, None, None)
}

fn run_send_event_task_with_outbound_hook(
    outbound_hook: Option<OutboundEventDispatchHandle>,
) -> Result<CommandContext, FlowableError> {
    run_send_event_task_with_tenants_and_hook(
        json!(false),
        Some(json!(true)),
        None,
        None,
        outbound_hook,
    )
}

fn run_send_event_task_with_tenants(
    skip_send_event: Value,
    skip_enabled: Option<Value>,
    definition_tenant_id: Option<&str>,
    execution_tenant_id: Option<&str>,
) -> Result<CommandContext, FlowableError> {
    run_send_event_task_with_tenants_and_hook(
        skip_send_event,
        skip_enabled,
        definition_tenant_id,
        execution_tenant_id,
        None,
    )
}

fn run_send_event_task_with_tenants_and_hook(
    skip_send_event: Value,
    skip_enabled: Option<Value>,
    definition_tenant_id: Option<&str>,
    execution_tenant_id: Option<&str>,
    outbound_hook: Option<OutboundEventDispatchHandle>,
) -> Result<CommandContext, FlowableError> {
    let db_store = Arc::new(DbStore::new_in_memory().unwrap());
    let deployment_manager = DeploymentManager::new_with_memory_backend_for_test(db_store.clone());
    deployment_manager.insert_bpmn_model("send-event-skip-process:1", send_event_task_model());

    let runtime_store = RuntimeStore::new_with_memory_backend_for_test(db_store);
    {
        let mut session = runtime_store.create_session().unwrap();
        runtime_store.insert_event_registry_channel_definition(
            EventRegistryChannelDefinition {
                id: "event-registry-deployment:test:orders-outbound.channel".to_string(),
                deployment_id: "event-registry-deployment:test".to_string(),
                key: "ordersOutbound".to_string(),
                name: "Orders outbound".to_string(),
                description: None,
                category: None,
                channel_type: "outbound".to_string(),
                resource_name: "orders-outbound.channel".to_string(),
                version: 1,
                create_time: 0,
                tenant_id: definition_tenant_id.map(str::to_string),
                parent_deployment_id: None,
                configuration: json!({ "destination": "orders-outbound" }),
            },
            &mut session,
        );
        runtime_store.insert_event_registry_event_definition(
            EventRegistryEventDefinition {
                id: "event-registry-deployment:test:order-published.event".to_string(),
                deployment_id: "event-registry-deployment:test".to_string(),
                key: "orderPublished".to_string(),
                name: "Order published".to_string(),
                description: None,
                category: None,
                event_type: "order.published".to_string(),
                channel_key: "ordersOutbound".to_string(),
                resource_name: "order-published.event".to_string(),
                version: 1,
                tenant_id: definition_tenant_id.map(str::to_string),
                parent_deployment_id: None,
                payload: json!([]),
            },
            &mut session,
        );
        session.flush_and_commit().unwrap();
    }

    let config = Arc::new(ProcessEngineConfiguration::default());
    if let Some(hook) = outbound_hook {
        config.outbound_event_dispatch.install(hook);
    }
    let session = runtime_store.create_session().unwrap();
    let mut command_context = CommandContext::new(
        deployment_manager,
        runtime_store,
        session,
        config,
        Arc::new(flowable_http_service::DeterministicHttpRuntime::default()),
    );

    let mut execution = Execution {
        id: "send-event-process-instance".to_string(),
        root_process_instance_id: Some("send-event-process-instance".to_string()),
        process_instance_id: Some("send-event-process-instance".to_string()),
        process_definition_id: Some("send-event-skip-process:1".to_string()),
        process_definition_key: Some("sendEventSkipProcess".to_string()),
        activity_id: Some("sendEventTask1".to_string()),
        tenant_id: execution_tenant_id.map(str::to_string),
        is_active: true,
        is_scope: true,
        ..Default::default()
    };
    execution.set_process_variable("skipSendEvent".to_string(), skip_send_event);
    execution.set_process_variable("orderId".to_string(), json!("A-100"));
    if let Some(skip_enabled) = skip_enabled {
        execution.set_process_variable(
            "_FLOWABLE_SKIP_EXPRESSION_ENABLED".to_string(),
            skip_enabled,
        );
    }
    {
        let (store, sess) = command_context.store_and_session();
        store.insert_execution(&execution, sess);
    }

    ServiceTaskActivityBehavior::new().execute(&mut execution, &mut command_context)?;
    while let Some(operation) = command_context.agenda().pop_operation() {
        operation.run(&mut command_context)?;
    }
    command_context.session().flush_and_commit()?;

    Ok(command_context)
}

fn run_send_and_receive_event_task_until_waiting()
-> Result<(CommandContext, Execution), FlowableError> {
    let db_store = Arc::new(DbStore::new_in_memory().unwrap());
    let deployment_manager = DeploymentManager::new_with_memory_backend_for_test(db_store.clone());
    deployment_manager.insert_bpmn_model(
        "send-and-receive-event-process:1",
        send_and_receive_event_task_model(),
    );

    let runtime_store = RuntimeStore::new_with_memory_backend_for_test(db_store);
    {
        let mut session = runtime_store.create_session().unwrap();
        runtime_store.insert_event_registry_channel_definition(
            EventRegistryChannelDefinition {
                id: "event-registry-deployment:test:orders-outbound.channel".to_string(),
                deployment_id: "event-registry-deployment:test".to_string(),
                key: "ordersOutbound".to_string(),
                name: "Orders outbound".to_string(),
                description: None,
                category: None,
                channel_type: "outbound".to_string(),
                resource_name: "orders-outbound.channel".to_string(),
                version: 1,
                create_time: 0,
                tenant_id: None,
                parent_deployment_id: None,
                configuration: json!({ "destination": "orders-outbound" }),
            },
            &mut session,
        );
        runtime_store.insert_event_registry_channel_definition(
            EventRegistryChannelDefinition {
                id: "event-registry-deployment:test:orders-inbound.channel".to_string(),
                deployment_id: "event-registry-deployment:test".to_string(),
                key: "ordersInbound".to_string(),
                name: "Orders inbound".to_string(),
                description: None,
                category: None,
                channel_type: "inbound".to_string(),
                resource_name: "orders-inbound.channel".to_string(),
                version: 1,
                create_time: 0,
                tenant_id: None,
                parent_deployment_id: None,
                configuration: json!({ "destination": "orders-inbound" }),
            },
            &mut session,
        );
        runtime_store.insert_event_registry_event_definition(
            EventRegistryEventDefinition {
                id: "event-registry-deployment:test:order-published.event".to_string(),
                deployment_id: "event-registry-deployment:test".to_string(),
                key: "orderPublished".to_string(),
                name: "Order published".to_string(),
                description: None,
                category: None,
                event_type: "order.published".to_string(),
                channel_key: "ordersOutbound".to_string(),
                resource_name: "order-published.event".to_string(),
                version: 1,
                tenant_id: None,
                parent_deployment_id: None,
                payload: json!([]),
            },
            &mut session,
        );
        runtime_store.insert_event_registry_event_definition(
            EventRegistryEventDefinition {
                id: "event-registry-deployment:test:order-accepted.event".to_string(),
                deployment_id: "event-registry-deployment:test".to_string(),
                key: "orderAccepted".to_string(),
                name: "Order accepted".to_string(),
                description: None,
                category: None,
                event_type: "order.accepted".to_string(),
                channel_key: "ordersInbound".to_string(),
                resource_name: "order-accepted.event".to_string(),
                version: 1,
                tenant_id: None,
                parent_deployment_id: None,
                payload: json!([]),
            },
            &mut session,
        );
        session.flush_and_commit().unwrap();
    }

    let config = Arc::new(ProcessEngineConfiguration::default());
    let session = runtime_store.create_session().unwrap();
    let mut command_context = CommandContext::new(
        deployment_manager,
        runtime_store,
        session,
        config,
        Arc::new(flowable_http_service::DeterministicHttpRuntime::default()),
    );

    let mut execution = Execution {
        id: "send-and-receive-execution".to_string(),
        root_process_instance_id: Some("send-and-receive-process-instance".to_string()),
        process_instance_id: Some("send-and-receive-process-instance".to_string()),
        process_definition_id: Some("send-and-receive-event-process:1".to_string()),
        process_definition_key: Some("sendAndReceiveEventProcess".to_string()),
        activity_id: Some("sendEventTask1".to_string()),
        is_active: true,
        is_scope: true,
        ..Default::default()
    };
    execution.set_process_variable("orderId".to_string(), json!("A-200"));
    {
        let (store, sess) = command_context.store_and_session();
        store.insert_execution(&execution, sess);
    }

    ServiceTaskActivityBehavior::new().execute(&mut execution, &mut command_context)?;
    while let Some(operation) = command_context.agenda().pop_operation() {
        operation.run(&mut command_context)?;
    }

    let stored_execution = {
        let (store, sess) = command_context.store_and_session();
        store
            .find_execution("send-and-receive-execution", sess)
            .expect("send-and-receive execution should remain in runtime store")
            .clone()
    };

    Ok((command_context, stored_execution))
}

fn run_http_service_task_with_io_transient_result()
-> Result<(CommandContext, Execution), FlowableError> {
    let db_store = Arc::new(DbStore::new_in_memory().unwrap());
    let deployment_manager = DeploymentManager::new_with_memory_backend_for_test(db_store.clone());
    deployment_manager.insert_bpmn_model(
        "http-service-task-io-process:1",
        http_service_task_io_transient_result_model(),
    );

    let runtime_store = RuntimeStore::new_with_memory_backend_for_test(db_store);
    let session = runtime_store.create_session().unwrap();
    let config = Arc::new(ProcessEngineConfiguration::default());
    let mut command_context = CommandContext::new(
        deployment_manager,
        runtime_store,
        session,
        config,
        Arc::new(flowable_http_service::DeterministicHttpRuntime::default()),
    );

    let mut execution = Execution {
        id: "http-service-task-io-execution".to_string(),
        root_process_instance_id: Some("http-service-task-io-process-instance".to_string()),
        process_instance_id: Some("http-service-task-io-process-instance".to_string()),
        process_definition_id: Some("http-service-task-io-process:1".to_string()),
        process_definition_key: Some("httpServiceTaskIoProcess".to_string()),
        activity_id: Some("httpTask1".to_string()),
        is_active: true,
        is_scope: true,
        ..Default::default()
    };
    execution.set_process_variable("customerId".to_string(), json!("C-123"));
    {
        let (store, sess) = command_context.store_and_session();
        store.insert_execution(&execution, sess);
    }

    ServiceTaskActivityBehavior::new().execute(&mut execution, &mut command_context)?;
    {
        let (store, sess) = command_context.store_and_session();
        store.update_execution(&execution, sess);
    }
    command_context.session().flush_and_commit()?;

    Ok((command_context, execution))
}

fn run_http_service_task_with_local_result() -> Result<(CommandContext, Execution), FlowableError> {
    let db_store = Arc::new(DbStore::new_in_memory().unwrap());
    let deployment_manager = DeploymentManager::new_with_memory_backend_for_test(db_store.clone());
    deployment_manager.insert_bpmn_model(
        "http-service-task-local-result-process:1",
        http_service_task_local_result_model(),
    );

    let runtime_store = RuntimeStore::new_with_memory_backend_for_test(db_store);
    let session = runtime_store.create_session().unwrap();
    let config = Arc::new(ProcessEngineConfiguration::default());
    let mut command_context = CommandContext::new(
        deployment_manager,
        runtime_store,
        session,
        config,
        Arc::new(flowable_http_service::DeterministicHttpRuntime::default()),
    );

    let mut execution = Execution {
        id: "http-service-task-local-result-execution".to_string(),
        root_process_instance_id: Some(
            "http-service-task-local-result-process-instance".to_string(),
        ),
        process_instance_id: Some("http-service-task-local-result-process-instance".to_string()),
        process_definition_id: Some("http-service-task-local-result-process:1".to_string()),
        process_definition_key: Some("httpServiceTaskLocalResultProcess".to_string()),
        activity_id: Some("httpTask1".to_string()),
        is_active: true,
        is_scope: true,
        ..Default::default()
    };
    {
        let (store, sess) = command_context.store_and_session();
        store.insert_execution(&execution, sess);
    }

    ServiceTaskActivityBehavior::new().execute(&mut execution, &mut command_context)?;
    {
        let (store, sess) = command_context.store_and_session();
        store.update_execution(&execution, sess);
    }
    command_context.session().flush_and_commit()?;

    Ok((command_context, execution))
}

fn run_delegate_expression_service_task(
    register_delegate: bool,
) -> Result<(CommandContext, Execution), FlowableError> {
    let db_store = Arc::new(DbStore::new_in_memory().unwrap());
    let deployment_manager = DeploymentManager::new_with_memory_backend_for_test(db_store.clone());
    deployment_manager.insert_bpmn_model(
        "delegate-expression-process:1",
        delegate_expression_service_task_model(),
    );

    let runtime_store = RuntimeStore::new_with_memory_backend_for_test(db_store);
    let session = runtime_store.create_session().unwrap();
    let config = Arc::new(ProcessEngineConfiguration::default());
    let mut command_context = CommandContext::new(
        deployment_manager,
        runtime_store,
        session,
        config,
        Arc::new(flowable_http_service::DeterministicHttpRuntime::default()),
    );
    if register_delegate {
        let mut registry = LocalServiceTaskDelegateRegistry::new();
        registry.register("recordingDelegate", Arc::new(RecordingDelegate));
        command_context.session_caches().insert(
            SERVICE_TASK_DELEGATE_REGISTRY_CACHE_KEY.to_string(),
            Box::new(registry),
        );
    }

    let mut execution = Execution {
        id: "delegate-expression-execution".to_string(),
        root_process_instance_id: Some("delegate-expression-process-instance".to_string()),
        process_instance_id: Some("delegate-expression-process-instance".to_string()),
        process_definition_id: Some("delegate-expression-process:1".to_string()),
        process_definition_key: Some("delegateExpressionProcess".to_string()),
        activity_id: Some("delegateTask1".to_string()),
        is_active: true,
        is_scope: true,
        ..Default::default()
    };
    execution.set_process_variable("delegateName".to_string(), json!("recordingDelegate"));
    execution.set_process_variable("customerId".to_string(), json!("C-987"));
    {
        let (store, sess) = command_context.store_and_session();
        store.insert_execution(&execution, sess);
    }

    ServiceTaskActivityBehavior::new().execute(&mut execution, &mut command_context)?;
    while let Some(operation) = command_context.agenda().pop_operation() {
        operation.run(&mut command_context)?;
    }
    command_context.session().flush_and_commit()?;

    Ok((command_context, execution))
}

fn triggerable_delegate_expression_model() -> BpmnModel {
    let converter = BpmnXMLConverter::new();
    converter.convert_to_bpmn_model(TRIGGERABLE_DELEGATE_EXPRESSION_SERVICE_TASK_XML)
}

fn triggerable_class_service_task_model() -> BpmnModel {
    let converter = BpmnXMLConverter::new();
    converter.convert_to_bpmn_model(TRIGGERABLE_CLASS_SERVICE_TASK_XML)
}

fn run_triggerable_local_delegate_until_waiting(
    use_class: bool,
) -> Result<(CommandContext, Execution), FlowableError> {
    let db_store = Arc::new(DbStore::new_in_memory().unwrap());
    let deployment_manager = DeploymentManager::new_with_memory_backend_for_test(db_store.clone());
    let (process_def_id, model, process_key, process_instance_id, execution_id, registry_key) =
        if use_class {
            (
                "triggerable-class-process:1",
                triggerable_class_service_task_model(),
                "triggerableClassProcess",
                "triggerable-class-process-instance",
                "triggerable-class-execution",
                "com.example.TriggerableDelegate",
            )
        } else {
            (
                "triggerable-delegate-process:1",
                triggerable_delegate_expression_model(),
                "triggerableDelegateProcess",
                "triggerable-delegate-process-instance",
                "triggerable-delegate-execution",
                "recordingDelegate",
            )
        };
    deployment_manager.insert_bpmn_model(process_def_id, model);

    let runtime_store = RuntimeStore::new_with_memory_backend_for_test(db_store);
    let session = runtime_store.create_session().unwrap();
    let config = Arc::new(ProcessEngineConfiguration::default());
    let mut command_context = CommandContext::new(
        deployment_manager,
        runtime_store,
        session,
        config,
        Arc::new(flowable_http_service::DeterministicHttpRuntime::default()),
    );
    let mut registry = LocalServiceTaskDelegateRegistry::new();
    registry.register(registry_key, Arc::new(RecordingDelegate));
    command_context.session_caches().insert(
        SERVICE_TASK_DELEGATE_REGISTRY_CACHE_KEY.to_string(),
        Box::new(registry),
    );

    let mut execution = Execution {
        id: execution_id.to_string(),
        root_process_instance_id: Some(process_instance_id.to_string()),
        process_instance_id: Some(process_instance_id.to_string()),
        process_definition_id: Some(process_def_id.to_string()),
        process_definition_key: Some(process_key.to_string()),
        activity_id: Some("delegateTask1".to_string()),
        is_active: true,
        is_scope: true,
        ..Default::default()
    };
    if !use_class {
        execution.set_process_variable("delegateName".to_string(), json!("recordingDelegate"));
    }
    execution.set_process_variable("customerId".to_string(), json!("C-555"));
    {
        let (store, sess) = command_context.store_and_session();
        store.insert_execution(&execution, sess);
    }

    ServiceTaskActivityBehavior::new().execute(&mut execution, &mut command_context)?;
    while let Some(operation) = command_context.agenda().pop_operation() {
        operation.run(&mut command_context)?;
    }

    let stored_execution = {
        let (store, sess) = command_context.store_and_session();
        store
            .find_execution(execution_id, sess)
            .expect("triggerable execution should remain in runtime store")
            .clone()
    };

    Ok((command_context, stored_execution))
}

fn run_owned_http_service_task_with_skip_variable_and_enabled_flag(
    skip_service: bool,
    skip_enabled: bool,
) -> Result<CommandContext, FlowableError> {
    run_owned_http_service_task_with_skip_value_and_enabled_flag(
        serde_json::Value::Bool(skip_service),
        skip_enabled,
    )
}

fn run_owned_http_service_task_with_skip_variable(
    skip_service: bool,
) -> Result<CommandContext, FlowableError> {
    run_owned_http_service_task_with_skip_variable_and_enabled_flag(skip_service, true)
}

#[test]
fn service_task_passes_through_to_end_event() {
    let process_engine = ProcessEngine::new("default".to_string());

    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="serviceTaskProcess" name="Service Task Process">
            <startEvent id="startEvent1" />
            <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="serviceTask1" />
            <serviceTask id="serviceTask1" name="Do Work" />
            <sequenceFlow id="flow2" sourceRef="serviceTask1" targetRef="endEvent1" />
            <endEvent id="endEvent1" />
        </process>
    </definitions>"#;

    let builder = repository_service
        .create_deployment()
        .name("Service Task Deployment".to_string())
        .add_string("serviceTaskProcess.bpmn20.xml".to_string(), xml.to_string());

    repository_service.deploy(builder).unwrap();
    let process_definition_id = repository_service.get_process_definition_ids().unwrap()[0].clone();

    let builder = runtime_service
        .create_process_instance_builder()
        .process_definition_id(process_definition_id)
        .name("Service Task Instance".to_string());

    let process_instance = runtime_service.start_process_instance(builder).unwrap();

    let runtime_store = process_engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let stored_pi = runtime_store
        .find_process_instance(&process_instance.id, &mut session)
        .expect("Process instance should be in runtime store");
    assert!(
        stored_pi.is_ended,
        "Process instance should be ended after service task pass-through"
    );
}

#[test]
fn service_task_skip_expression_is_parsed_from_bpmn_xml() {
    let model = skip_service_task_model();
    assert_eq!(
        service_task(&model, "httpTask1").skip_expression.as_deref(),
        Some("${skipService}")
    );
}

#[test]
fn send_event_service_task_skip_expression_and_event_metadata_are_parsed_from_bpmn_xml() {
    let model = send_event_task_model();
    let send_event_task = service_task(&model, "sendEventTask1");

    assert_eq!(send_event_task.task_type.as_deref(), Some("send-event"));
    assert_eq!(
        send_event_task.skip_expression.as_deref(),
        Some("${skipSendEvent}")
    );
    assert_eq!(
        send_event_task.event_type.as_deref(),
        Some("orderPublished")
    );
    assert_eq!(send_event_task.event_in_parameters.len(), 1);
    assert_eq!(
        send_event_task.event_in_parameters[0]
            .source_expression
            .as_deref(),
        Some("${orderId}")
    );
    assert_eq!(
        send_event_task.event_in_parameters[0].target.as_deref(),
        Some("orderId")
    );
}

#[test]
fn owned_service_task_skip_expression_true_skips_behavior_and_takes_outgoing_flow() {
    let command_context = run_owned_http_service_task_with_skip_variable(true)
        .expect("skipExpression=true should skip the disabled HTTP behavior");

    let runtime_store = command_context.runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    assert!(
        runtime_store
            .find_http_task_records_by_process_instance_id(
                "skip-service-task-process-instance",
                &mut session
            )
            .is_empty(),
        "skipped service task must not execute the HTTP runtime"
    );
    let tasks = runtime_store
        .find_tasks_by_process_instance_id("skip-service-task-process-instance", &mut session);
    assert_eq!(tasks.len(), 1, "skip should still take the outgoing flow");
    assert_eq!(tasks[0].name, "After Skip");
}

#[test]
fn send_event_task_skip_expression_true_skips_publish_and_takes_outgoing_flow() {
    let command_context =
        run_send_event_task_with_skip_and_enabled_variable(json!(true), Some(json!(true)))
            .expect("skipExpression=true should skip the send-event runtime");

    let runtime_store = command_context.runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    assert!(
        runtime_store
            .list_event_registry_event_instance_deliveries(&mut session)
            .unwrap()
            .is_empty(),
        "skipped send-event task must not publish an event registry delivery"
    );
    let tasks = runtime_store
        .find_tasks_by_process_instance_id("send-event-process-instance", &mut session);
    assert_eq!(tasks.len(), 1, "skip should still take the outgoing flow");
    assert_eq!(tasks[0].name, "After Send Event");
}

#[test]
fn send_event_task_skip_expression_false_publishes_outbound_event() {
    let command_context =
        run_send_event_task_with_skip_and_enabled_variable(json!(false), Some(json!(true)))
            .expect("skipExpression=false should execute the send-event runtime");

    let runtime_store = command_context.runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let deliveries = runtime_store
        .list_event_registry_event_instance_deliveries(&mut session)
        .unwrap();
    assert_eq!(deliveries.len(), 1);
    assert_eq!(
        deliveries[0].direction,
        EventRegistryEventDirection::Outbound
    );
    assert_eq!(
        deliveries[0].status,
        EventRegistryEventInstanceStatus::Published
    );
    assert_eq!(
        deliveries[0].status_history,
        vec![
            EventRegistryEventInstanceStatus::Created,
            EventRegistryEventInstanceStatus::Published
        ]
    );
    assert_eq!(deliveries[0].event_definition_key, "orderPublished");
    assert_eq!(deliveries[0].payload, json!({ "orderId": "A-100" }));
    assert_eq!(
        deliveries[0].channel_definition_id.as_deref(),
        Some("event-registry-deployment:test:orders-outbound.channel")
    );
}

#[test]
fn send_event_task_resolves_registry_definitions_in_execution_tenant() {
    let exact = run_send_event_task_with_tenants(
        json!(false),
        Some(json!(true)),
        Some("tenant-a"),
        Some("tenant-a"),
    )
    .expect("same-tenant event and channel should resolve");
    let exact_deliveries = {
        let store = exact.runtime_store();
        let mut session = store.create_session().unwrap();
        store
            .list_event_registry_event_instance_deliveries(&mut session)
            .unwrap()
    };
    assert_eq!(exact_deliveries.len(), 1);
    assert_eq!(exact_deliveries[0].tenant_id.as_deref(), Some("tenant-a"));

    let foreign_error = match run_send_event_task_with_tenants(
        json!(false),
        Some(json!(true)),
        Some("tenant-a"),
        Some("tenant-b"),
    ) {
        Ok(_) => panic!("tenant-b must not use tenant-a event/channel definitions"),
        Err(error) => error,
    };
    assert!(
        matches!(foreign_error, FlowableError::NotFound(_)),
        "foreign-tenant resolution should fail with NotFound, got {foreign_error:?}"
    );

    let fallback =
        run_send_event_task_with_tenants(json!(false), Some(json!(true)), None, Some("tenant-b"))
            .expect("tenant-scoped execution should fall back to tenantless definitions");
    let fallback_deliveries = {
        let store = fallback.runtime_store();
        let mut session = store.create_session().unwrap();
        store
            .list_event_registry_event_instance_deliveries(&mut session)
            .unwrap()
    };
    assert_eq!(fallback_deliveries.len(), 1);
    assert_eq!(fallback_deliveries[0].tenant_id, None);
}

#[test]
fn send_event_task_skip_expression_is_ignored_until_enabled_variable_is_true() {
    for skip_enabled in [None, Some(json!(false))] {
        let command_context =
            run_send_event_task_with_skip_and_enabled_variable(json!(true), skip_enabled)
                .expect("disabled skipExpression should execute the send-event runtime");

        let runtime_store = command_context.runtime_store();
        let mut session = runtime_store.create_session().unwrap();
        let deliveries = runtime_store
            .list_event_registry_event_instance_deliveries(&mut session)
            .unwrap();
        assert_eq!(deliveries.len(), 1);
        assert_eq!(
            deliveries[0].status,
            EventRegistryEventInstanceStatus::Published
        );
    }
}

#[test]
fn send_event_task_skip_enabled_variable_requires_boolean_value() {
    let err =
        match run_send_event_task_with_skip_and_enabled_variable(json!(true), Some(json!("yes"))) {
            Ok(_) => panic!("_FLOWABLE_SKIP_EXPRESSION_ENABLED must reject non-boolean values"),
            Err(err) => err,
        };

    assert!(
        err.to_string().contains("Skip expression variable") && err.to_string().contains("boolean"),
        "expected skipExpression enable variable boolean error, got {err}"
    );
}

/// Recording hook used to prove BPMN send-event invokes the outbound path (P94).
struct RecordingOutboundDispatch {
    requests: Mutex<Vec<OutboundEventDispatchRequest>>,
}

impl OutboundEventDispatchHook for RecordingOutboundDispatch {
    fn dispatch_outbound(
        &self,
        request: &OutboundEventDispatchRequest,
    ) -> Result<(), FlowableError> {
        self.requests.lock().unwrap().push(request.clone());
        Ok(())
    }
}

struct FailingOutboundDispatch {
    message: String,
}

impl OutboundEventDispatchHook for FailingOutboundDispatch {
    fn dispatch_outbound(
        &self,
        _request: &OutboundEventDispatchRequest,
    ) -> Result<(), FlowableError> {
        Err(FlowableError::ExecutionError(self.message.clone()))
    }
}

#[test]
fn send_event_task_outbound_dispatch_success_sets_dispatch_token_and_published() {
    let recorder = Arc::new(RecordingOutboundDispatch {
        requests: Mutex::new(Vec::new()),
    });
    let command_context = run_send_event_task_with_outbound_hook(Some(
        Arc::clone(&recorder) as OutboundEventDispatchHandle
    ))
    .expect("send-event with successful outbound hook should publish");

    let requests = recorder.requests.lock().unwrap();
    assert_eq!(requests.len(), 1, "outbound hook must be invoked once");
    assert_eq!(requests[0].channel_key, "ordersOutbound");
    assert_eq!(requests[0].event_type, "order.published");
    assert_eq!(requests[0].payload, json!({ "orderId": "A-100" }));
    assert!(
        requests[0]
            .dispatch_token
            .as_deref()
            .is_some_and(|token| token.starts_with("dispatch:")),
        "dispatch token must be assigned before adapter I/O"
    );

    let runtime_store = command_context.runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let deliveries = runtime_store
        .list_event_registry_event_instance_deliveries(&mut session)
        .unwrap();
    assert_eq!(deliveries.len(), 1);
    assert_eq!(
        deliveries[0].status,
        EventRegistryEventInstanceStatus::Published
    );
    assert_eq!(
        deliveries[0].status_history,
        vec![
            EventRegistryEventInstanceStatus::Created,
            EventRegistryEventInstanceStatus::Published
        ]
    );
    assert!(
        deliveries[0]
            .dispatch_token
            .as_deref()
            .is_some_and(|token| token.starts_with("dispatch:")),
        "delivery must retain dispatch token after success"
    );
    assert!(deliveries[0].last_error.is_none());
}

#[test]
fn send_event_task_outbound_dispatch_failure_marks_failed_and_errors() {
    let hook: OutboundEventDispatchHandle = Arc::new(FailingOutboundDispatch {
        message: "adapter refused delivery".to_string(),
    });
    let (mut command_context, err) = run_send_event_task_capturing_context_on_error(hook);
    assert!(
        err.to_string().contains("adapter refused delivery"),
        "expected adapter error to surface, got {err}"
    );

    // Reuse the command session (open write tx) so SQLite does not report a locked table.
    let deliveries = {
        let (store, sess) = command_context.store_and_session();
        store
            .list_event_registry_event_instance_deliveries(sess)
            .expect("list deliveries on active command session")
    };
    assert_eq!(deliveries.len(), 1);
    assert_eq!(
        deliveries[0].status,
        EventRegistryEventInstanceStatus::Failed
    );
    assert_eq!(
        deliveries[0].status_history,
        vec![
            EventRegistryEventInstanceStatus::Created,
            EventRegistryEventInstanceStatus::Failed
        ]
    );
    assert!(
        deliveries[0]
            .last_error
            .as_deref()
            .is_some_and(|msg| msg.contains("adapter refused delivery")),
        "last_error should capture adapter failure, got {:?}",
        deliveries[0].last_error
    );
    assert!(deliveries[0].last_failure_at.is_some());
    assert!(deliveries[0].next_retry_at.is_some());
    assert_eq!(deliveries[0].retry_count, 0);
    assert!(
        deliveries[0].dispatch_token.is_some(),
        "Failed delivery keeps dispatch token for at-least-once retry"
    );
}

/// Like the happy-path helper, but returns the CommandContext even when execute fails
/// so delivery status (Failed) can be inspected (session not rolled back by the helper).
fn run_send_event_task_capturing_context_on_error(
    outbound_hook: OutboundEventDispatchHandle,
) -> (CommandContext, FlowableError) {
    let db_store = Arc::new(DbStore::new_in_memory().unwrap());
    let deployment_manager = DeploymentManager::new_with_memory_backend_for_test(db_store.clone());
    deployment_manager.insert_bpmn_model("send-event-skip-process:1", send_event_task_model());

    let runtime_store = RuntimeStore::new_with_memory_backend_for_test(db_store);
    {
        let mut session = runtime_store.create_session().unwrap();
        runtime_store.insert_event_registry_channel_definition(
            EventRegistryChannelDefinition {
                id: "event-registry-deployment:test:orders-outbound.channel".to_string(),
                deployment_id: "event-registry-deployment:test".to_string(),
                key: "ordersOutbound".to_string(),
                name: "Orders outbound".to_string(),
                description: None,
                category: None,
                channel_type: "outbound".to_string(),
                resource_name: "orders-outbound.channel".to_string(),
                version: 1,
                create_time: 0,
                tenant_id: None,
                parent_deployment_id: None,
                configuration: json!({ "destination": "orders-outbound" }),
            },
            &mut session,
        );
        runtime_store.insert_event_registry_event_definition(
            EventRegistryEventDefinition {
                id: "event-registry-deployment:test:order-published.event".to_string(),
                deployment_id: "event-registry-deployment:test".to_string(),
                key: "orderPublished".to_string(),
                name: "Order published".to_string(),
                description: None,
                category: None,
                event_type: "order.published".to_string(),
                channel_key: "ordersOutbound".to_string(),
                resource_name: "order-published.event".to_string(),
                version: 1,
                tenant_id: None,
                parent_deployment_id: None,
                payload: json!([]),
            },
            &mut session,
        );
        session.flush_and_commit().unwrap();
    }

    let config = Arc::new(ProcessEngineConfiguration::default());
    config.outbound_event_dispatch.install(outbound_hook);
    let session = runtime_store.create_session().unwrap();
    let mut command_context = CommandContext::new(
        deployment_manager,
        runtime_store,
        session,
        config,
        Arc::new(flowable_http_service::DeterministicHttpRuntime::default()),
    );

    let mut execution = Execution {
        id: "send-event-process-instance".to_string(),
        root_process_instance_id: Some("send-event-process-instance".to_string()),
        process_instance_id: Some("send-event-process-instance".to_string()),
        process_definition_id: Some("send-event-skip-process:1".to_string()),
        process_definition_key: Some("sendEventSkipProcess".to_string()),
        activity_id: Some("sendEventTask1".to_string()),
        is_active: true,
        is_scope: true,
        ..Default::default()
    };
    execution.set_process_variable("skipSendEvent".to_string(), json!(false));
    execution.set_process_variable("orderId".to_string(), json!("A-100"));
    execution.set_process_variable(
        "_FLOWABLE_SKIP_EXPRESSION_ENABLED".to_string(),
        json!(true),
    );
    {
        let (store, sess) = command_context.store_and_session();
        store.insert_execution(&execution, sess);
    }

    let err = ServiceTaskActivityBehavior::new()
        .execute(&mut execution, &mut command_context)
        .expect_err("failing outbound hook must error");
    (command_context, err)
}

#[test]
fn owned_service_task_skip_expression_is_ignored_until_enabled() {
    let err = match run_owned_http_service_task_with_skip_variable_and_enabled_flag(true, false) {
        Ok(_) => panic!("skipExpression should not apply until the enable variable is true"),
        Err(err) => err,
    };

    assert!(
        err.to_string().contains("HTTP service tasks are disabled"),
        "expected disabled HTTP behavior when skipExpression is not enabled, got {err}"
    );
}

#[test]
fn owned_service_task_skip_expression_false_keeps_existing_behavior() {
    let err = match run_owned_http_service_task_with_skip_variable(false) {
        Ok(_) => panic!("skipExpression=false should execute the HTTP behavior"),
        Err(err) => err,
    };

    assert!(
        err.to_string().contains("HTTP service tasks are disabled"),
        "expected disabled HTTP behavior error, got {err}"
    );
}

#[test]
fn owned_service_task_skip_expression_requires_boolean_result() {
    let err = match run_owned_http_service_task_with_skip_value_and_enabled_flag(
        serde_json::Value::String("yes".to_string()),
        true,
    ) {
        Ok(_) => panic!("skipExpression must reject non-boolean results"),
        Err(err) => err,
    };

    assert!(
        err.to_string().contains("skipExpression") && err.to_string().contains("boolean"),
        "expected skipExpression boolean error, got {err}"
    );
}

#[test]
fn validator_allows_skip_expression_for_owned_http_service_task_subset() {
    let model = skip_service_task_model();

    UnsupportedModelValidator::validate(&model, &ProcessEngineConfiguration::default())
        .expect("owned HTTP service task skipExpression should be deployable");
}

#[test]
fn validator_allows_minimal_send_event_service_task_subset_with_skip_expression() {
    let model = send_event_task_model();

    UnsupportedModelValidator::validate(&model, &ProcessEngineConfiguration::default())
        .expect("minimal send-event service task skipExpression should be deployable");
}

#[test]
fn validator_allows_minimal_send_and_receive_event_task_shape() {
    let converter = BpmnXMLConverter::new();
    let model = converter.convert_to_bpmn_model(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
    <process id="sendAndReceiveEventProcess">
        <startEvent id="startEvent1" />
        <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="sendEventTask1" />
        <serviceTask id="sendEventTask1" flowable:type="send-event" flowable:triggerable="true">
            <extensionElements>
                <flowable:eventType>orderPublished</flowable:eventType>
                <flowable:triggerEventType>orderAccepted</flowable:triggerEventType>
            </extensionElements>
        </serviceTask>
        <sequenceFlow id="flow2" sourceRef="sendEventTask1" targetRef="endEvent1" />
        <endEvent id="endEvent1" />
    </process>
</definitions>"#,
    );

    UnsupportedModelValidator::validate(&model, &ProcessEngineConfiguration::default())
        .expect("minimal send-and-receive event task shape should be deployable");
}

#[test]
fn validator_allows_bounded_service_task_io_and_result_scope_flags() {
    let model = http_service_task_io_transient_result_model();

    UnsupportedModelValidator::validate(&model, &ProcessEngineConfiguration::default())
        .expect("bounded service task in/out parameters and result scope flags should deploy");

    let model = http_service_task_local_result_model();
    UnsupportedModelValidator::validate(&model, &ProcessEngineConfiguration::default())
        .expect("bounded service task local result flag should deploy");
}

#[test]
fn validator_allows_send_and_receive_event_task_shape() {
    let model = send_and_receive_event_task_model();

    UnsupportedModelValidator::validate(&model, &ProcessEngineConfiguration::default())
        .expect("send-and-receive send-event task shape should be deployable");
}

#[test]
fn http_service_task_maps_in_out_parameters_and_keeps_transient_result_unpersisted() {
    let (command_context, execution) = run_http_service_task_with_io_transient_result()
        .expect("bounded service task in/out parameters should execute");

    assert_eq!(
        execution.persistent_process_variable("copiedCustomer"),
        Some(json!("C-123"))
    );
    assert_eq!(
        execution.persistent_process_variable("copiedTier"),
        Some(json!("gold"))
    );
    assert_eq!(
        execution.persistent_process_variable("httpStatusCode"),
        Some(json!(200))
    );
    assert_eq!(
        execution
            .persistent_process_variable("copiedHttpResult")
            .and_then(|value| value["response"]["statusCode"].as_i64().map(Value::from)),
        Some(json!(200))
    );
    assert_eq!(
        execution.persistent_process_variable("httpResult"),
        None,
        "storeResultVariableAsTransient must not promote the result to process variables"
    );
    assert!(
        execution.process_variable("httpResult").is_some(),
        "transient result should remain available in the current execution chain"
    );

    let runtime_store = command_context.runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let persisted_variables = runtime_store
        .find_variables_by_execution_id("http-service-task-io-execution", &mut session);
    assert!(
        !persisted_variables.contains_key("httpResult"),
        "transient result must not be written to the runtime variable store"
    );
    assert_eq!(
        persisted_variables.get("copiedCustomer"),
        Some(&json!("C-123"))
    );
    assert_eq!(persisted_variables.get("copiedTier"), Some(&json!("gold")));
    assert_eq!(persisted_variables.get("httpStatusCode"), Some(&json!(200)));
}

#[test]
fn http_service_task_local_result_is_available_to_out_parameters_without_process_pollution() {
    let (command_context, execution) = run_http_service_task_with_local_result()
        .expect("local result should be available to bounded out parameters");

    assert_eq!(
        execution
            .persistent_process_variable("copiedHttpResult")
            .and_then(|value| value["response"]["statusCode"].as_i64().map(Value::from)),
        Some(json!(200))
    );
    assert_eq!(
        execution.persistent_process_variable("httpResult"),
        None,
        "useLocalScopeForResultVariable must not promote the result to process variables"
    );
    assert!(
        execution.process_variable("httpResult").is_some(),
        "local result should remain available in the current execution chain"
    );

    let runtime_store = command_context.runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let persisted_variables = runtime_store
        .find_variables_by_execution_id("http-service-task-local-result-execution", &mut session);
    // Java stores local variables in the runtime variable table. The row-level
    // projection dual-writes `local_variables` as well as `variables`, so a
    // useLocalScopeForResultVariable result is queryable as a variable instance
    // on this execution — without promoting it into `Execution::variables`
    // (asserted above via persistent_process_variable).
    assert!(
        persisted_variables.contains_key("httpResult"),
        "local result must project into the runtime variables table (local scope)"
    );
    assert!(
        persisted_variables.contains_key("copiedHttpResult"),
        "out parameters should write selected local result data back to process variables"
    );
}

#[test]
fn validator_allows_bounded_delegate_expression_with_field_extensions() {
    let model = delegate_expression_service_task_model();

    UnsupportedModelValidator::validate(&model, &ProcessEngineConfiguration::default())
        .expect("bounded delegateExpression service task should deploy");
}

#[test]
fn delegate_expression_service_task_executes_registered_local_delegate_with_fields() {
    let (command_context, execution) = run_delegate_expression_service_task(true)
        .expect("registered delegateExpression service task should execute");

    assert_eq!(
        execution.persistent_process_variable("writtenByDelegate"),
        Some(json!("C-987"))
    );
    assert_eq!(
        execution.persistent_process_variable("delegateWriteBack"),
        Some(json!("C-987"))
    );
    assert_eq!(
        execution.persistent_process_variable("delegateFieldCopy"),
        Some(json!("C-987"))
    );
    assert_eq!(
        execution
            .persistent_process_variable("delegateResult")
            .and_then(|value| value["fields"]["literalGreeting"].as_str().map(Value::from)),
        Some(json!("hello"))
    );

    let runtime_store = command_context.runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let tasks = runtime_store
        .find_tasks_by_process_instance_id("delegate-expression-process-instance", &mut session);
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].name, "After Delegate");
}

#[test]
fn delegate_expression_service_task_requires_registered_local_delegate() {
    let err = match run_delegate_expression_service_task(false) {
        Ok(_) => panic!("delegateExpression without a registered local delegate should fail"),
        Err(err) => err,
    };

    assert!(
        err.to_string()
            .contains("No local service task delegate 'recordingDelegate'"),
        "expected missing local delegate error, got {err}"
    );
}

#[test]
fn validator_allows_class_implementation_as_registry_key() {
    let converter = BpmnXMLConverter::new();
    let model = converter.convert_to_bpmn_model(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
    <process id="classDelegateProcess">
        <startEvent id="startEvent1" />
        <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="delegateTask1" />
        <serviceTask id="delegateTask1" flowable:class="com.example.MyDelegate">
            <extensionElements>
                <flowable:field name="literalGreeting" stringValue="hello" />
            </extensionElements>
        </serviceTask>
        <sequenceFlow id="flow2" sourceRef="delegateTask1" targetRef="endEvent1" />
        <endEvent id="endEvent1" />
    </process>
</definitions>"#,
    );

    UnsupportedModelValidator::validate(&model, &ProcessEngineConfiguration::default())
        .expect("class implementation is a registry key in the owned subset");
}

#[test]
fn validator_rejects_unsupported_delegate_implementation_type() {
    let converter = BpmnXMLConverter::new();
    let model = converter.convert_to_bpmn_model(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
    <process id="expressionDelegateProcess">
        <startEvent id="startEvent1" />
        <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="delegateTask1" />
        <serviceTask id="delegateTask1" flowable:expression="${doWork()}">
            <extensionElements>
                <flowable:field name="literalGreeting" stringValue="hello" />
            </extensionElements>
        </serviceTask>
        <sequenceFlow id="flow2" sourceRef="delegateTask1" targetRef="endEvent1" />
        <endEvent id="endEvent1" />
    </process>
</definitions>"#,
    );

    let err = UnsupportedModelValidator::validate(&model, &ProcessEngineConfiguration::default())
        .expect_err("expression implementation type remains outside the owned subset");

    assert!(
        err.to_string()
            .contains("only supports class or delegateExpression")
            && err.to_string().contains("expression"),
        "expected structured delegate implementation rejection, got {err}"
    );
}

#[test]
fn send_and_receive_event_task_waits_for_matching_trigger_and_maps_event_out_parameters() {
    let (mut command_context, mut execution) = run_send_and_receive_event_task_until_waiting()
        .expect("send-and-receive send-event task should publish outbound and wait");

    let deliveries = {
        let (store, sess) = command_context.store_and_session();
        store
            .list_event_registry_event_instance_deliveries(sess)
            .unwrap()
    };
    assert_eq!(deliveries.len(), 1);
    assert_eq!(
        deliveries[0].direction,
        EventRegistryEventDirection::Outbound
    );
    assert_eq!(
        deliveries[0].status,
        EventRegistryEventInstanceStatus::Published
    );
    assert_eq!(deliveries[0].payload, json!({ "orderId": "A-200" }));
    assert_eq!(
        deliveries[0].channel_definition_id.as_deref(),
        Some("event-registry-deployment:test:orders-outbound.channel")
    );

    let wait_states = {
        let (store, sess) = command_context.store_and_session();
        store.find_event_wait_states_by_process_instance_id(
            "send-and-receive-process-instance",
            sess,
        )
    };
    assert_eq!(wait_states.len(), 1);
    assert_eq!(wait_states[0].execution_id, "send-and-receive-execution");
    assert_eq!(
        wait_states[0].activity_id.as_deref(),
        Some("sendEventTask1")
    );
    // P130: wait kind + EventRegistry subscription (Java SendEventTaskActivityBehavior.java:140-151)
    assert_eq!(wait_states[0].wait_kind, RuntimeEventWaitKind::SendEventTask);
    assert_eq!(
        wait_states[0]
            .event_subscription
            .as_ref()
            .map(|subscription| subscription.kind.clone()),
        Some(EventSubscriptionKind::EventRegistry)
    );
    assert_eq!(
        wait_states[0]
            .event_subscription
            .as_ref()
            .map(|subscription| subscription.event_ref.as_str()),
        Some("orderAccepted")
    );
    assert!(!execution.is_active);
    let tasks_empty = {
        let (store, sess) = command_context.store_and_session();
        store
            .find_tasks_by_process_instance_id("send-and-receive-process-instance", sess)
            .is_empty()
    };
    assert!(
        tasks_empty,
        "send-and-receive task must not continue before the inbound trigger"
    );

    ServiceTaskActivityBehavior::new()
        .trigger(
            &mut execution,
            &mut command_context,
            Some("wrongEvent".to_string()),
            Some(json!({ "acceptedBy": "Nina" })),
        )
        .expect("non-matching inbound event should be ignored");
    let wait_state_present = {
        let (store, sess) = command_context.store_and_session();
        store
            .find_event_wait_state_by_execution_id("send-and-receive-execution", sess)
            .is_some()
    };
    assert!(
        wait_state_present,
        "non-matching inbound event must leave the wait state in place"
    );

    ServiceTaskActivityBehavior::new()
        .trigger(
            &mut execution,
            &mut command_context,
            Some("orderAccepted".to_string()),
            Some(json!({ "acceptedBy": "Nina" })),
        )
        .expect("matching inbound event should trigger the waiting send-event task");
    while let Some(operation) = command_context.agenda().pop_operation() {
        operation
            .run(&mut command_context)
            .expect("triggered outgoing flow should run");
    }

    let wait_state_absent = {
        let (store, sess) = command_context.store_and_session();
        store
            .find_event_wait_state_by_execution_id("send-and-receive-execution", sess)
            .is_none()
    };
    assert!(
        wait_state_absent,
        "matching inbound event must consume the wait state"
    );

    let inbound = {
        let (store, sess) = command_context.store_and_session();
        store
            .list_event_registry_event_instance_deliveries(sess)
            .unwrap()
            .into_iter()
            .find(|delivery| delivery.direction == EventRegistryEventDirection::Inbound)
            .expect("matching inbound trigger should be recorded")
    };
    assert_eq!(inbound.event_definition_key, "orderAccepted");
    assert_eq!(inbound.status, EventRegistryEventInstanceStatus::Processed);
    assert_eq!(
        inbound.channel_definition_id.as_deref(),
        Some("event-registry-deployment:test:orders-inbound.channel")
    );
    assert_eq!(
        inbound.status_history,
        vec![
            EventRegistryEventInstanceStatus::Received,
            EventRegistryEventInstanceStatus::Processed
        ]
    );

    let stored_execution = {
        let (store, sess) = command_context.store_and_session();
        store
            .find_execution("send-and-receive-execution", sess)
            .expect("triggered execution should remain until outgoing flow completes")
    };
    assert_eq!(
        stored_execution.process_variable("acceptedBy"),
        Some(json!("Nina"))
    );
    assert_eq!(
        stored_execution.process_variable("acceptedByGenericOut"),
        Some(json!("Nina"))
    );

    let tasks = {
        let (store, sess) = command_context.store_and_session();
        store.find_tasks_by_process_instance_id("send-and-receive-process-instance", sess)
    };
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].name, "After Send And Receive");
}

/// Java EventInstanceBpmnUtil.java:127 — missing payload field writes null, not the source name.
#[test]
fn send_and_receive_event_out_parameter_missing_payload_field_writes_null() {
    let (mut command_context, mut execution) = run_send_and_receive_event_task_until_waiting()
        .expect("send-and-receive should reach wait state");

    ServiceTaskActivityBehavior::new()
        .trigger(
            &mut execution,
            &mut command_context,
            Some("orderAccepted".to_string()),
            // payload omits acceptedBy
            Some(json!({ "otherField": "x" })),
        )
        .expect("matching event key should trigger even when out fields are absent");
    while let Some(operation) = command_context.agenda().pop_operation() {
        operation
            .run(&mut command_context)
            .expect("outgoing after missing-field out map");
    }

    let stored = {
        let (store, sess) = command_context.store_and_session();
        store
            .find_execution("send-and-receive-execution", sess)
            .expect("execution present")
    };
    assert_eq!(
        stored.process_variable("acceptedBy"),
        Some(Value::Null),
        "missing eventOutParameter source must set target to null (Java :127)"
    );
}

/// Production cmd path (TriggerCmd analog) maps resultVariable + outParameters.
#[test]
fn trigger_send_event_service_task_cmd_applies_result_variable_and_out_parameters() {
    let converter = BpmnXMLConverter::new();
    let model = converter.convert_to_bpmn_model(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
    <process id="sendAndReceiveResultProcess" name="Send And Receive With Result">
        <startEvent id="startEvent1" />
        <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="sendEventTask1" />
        <serviceTask id="sendEventTask1"
                     name="Publish And Await"
                     flowable:type="send-event"
                     flowable:triggerable="true"
                     flowable:resultVariableName="sendEventResult">
            <extensionElements>
                <flowable:eventType>orderPublished</flowable:eventType>
                <flowable:triggerEventType>orderAccepted</flowable:triggerEventType>
                <flowable:eventInParameter sourceExpression="${orderId}" target="orderId" />
                <flowable:eventOutParameter source="acceptedBy" target="acceptedBy" />
                <flowable:out source="payload.acceptedBy" target="acceptedByFromResult" />
            </extensionElements>
        </serviceTask>
        <sequenceFlow id="flow2" sourceRef="sendEventTask1" targetRef="userTask1" />
        <userTask id="userTask1" name="After" />
    </process>
</definitions>"#,
    );

    let db_store = Arc::new(DbStore::new_in_memory().unwrap());
    let deployment_manager = DeploymentManager::new_with_memory_backend_for_test(db_store.clone());
    deployment_manager.insert_bpmn_model("send-and-receive-result-process:1", model);

    let runtime_store = RuntimeStore::new_with_memory_backend_for_test(db_store);
    {
        let mut session = runtime_store.create_session().unwrap();
        runtime_store.insert_event_registry_channel_definition(
            EventRegistryChannelDefinition {
                id: "event-registry-deployment:test:orders-outbound.channel".to_string(),
                deployment_id: "event-registry-deployment:test".to_string(),
                key: "ordersOutbound".to_string(),
                name: "Orders outbound".to_string(),
                description: None,
                category: None,
                channel_type: "outbound".to_string(),
                resource_name: "orders-outbound.channel".to_string(),
                version: 1,
                create_time: 0,
                tenant_id: None,
                parent_deployment_id: None,
                configuration: json!({ "destination": "orders-outbound" }),
            },
            &mut session,
        );
        runtime_store.insert_event_registry_channel_definition(
            EventRegistryChannelDefinition {
                id: "event-registry-deployment:test:orders-inbound.channel".to_string(),
                deployment_id: "event-registry-deployment:test".to_string(),
                key: "ordersInbound".to_string(),
                name: "Orders inbound".to_string(),
                description: None,
                category: None,
                channel_type: "inbound".to_string(),
                resource_name: "orders-inbound.channel".to_string(),
                version: 1,
                create_time: 0,
                tenant_id: None,
                parent_deployment_id: None,
                configuration: json!({ "destination": "orders-inbound" }),
            },
            &mut session,
        );
        runtime_store.insert_event_registry_event_definition(
            EventRegistryEventDefinition {
                id: "event-registry-deployment:test:order-published.event".to_string(),
                deployment_id: "event-registry-deployment:test".to_string(),
                key: "orderPublished".to_string(),
                name: "Order published".to_string(),
                description: None,
                category: None,
                event_type: "order.published".to_string(),
                channel_key: "ordersOutbound".to_string(),
                resource_name: "order-published.event".to_string(),
                version: 1,
                tenant_id: None,
                parent_deployment_id: None,
                payload: json!([]),
            },
            &mut session,
        );
        runtime_store.insert_event_registry_event_definition(
            EventRegistryEventDefinition {
                id: "event-registry-deployment:test:order-accepted.event".to_string(),
                deployment_id: "event-registry-deployment:test".to_string(),
                key: "orderAccepted".to_string(),
                name: "Order accepted".to_string(),
                description: None,
                category: None,
                event_type: "order.accepted".to_string(),
                channel_key: "ordersInbound".to_string(),
                resource_name: "order-accepted.event".to_string(),
                version: 1,
                tenant_id: None,
                parent_deployment_id: None,
                payload: json!([]),
            },
            &mut session,
        );
        session.flush_and_commit().unwrap();
    }

    let config = Arc::new(ProcessEngineConfiguration::default());
    let session = runtime_store.create_session().unwrap();
    let mut command_context = CommandContext::new(
        deployment_manager,
        runtime_store,
        session,
        config,
        Arc::new(flowable_http_service::DeterministicHttpRuntime::default()),
    );

    let mut execution = Execution {
        id: "send-and-receive-result-execution".to_string(),
        root_process_instance_id: Some("send-and-receive-result-pi".to_string()),
        process_instance_id: Some("send-and-receive-result-pi".to_string()),
        process_definition_id: Some("send-and-receive-result-process:1".to_string()),
        process_definition_key: Some("sendAndReceiveResultProcess".to_string()),
        activity_id: Some("sendEventTask1".to_string()),
        is_active: true,
        is_scope: true,
        ..Default::default()
    };
    execution.set_process_variable("orderId".to_string(), json!("R-1"));
    {
        let (store, sess) = command_context.store_and_session();
        store.insert_execution(&execution, sess);
    }

    ServiceTaskActivityBehavior::new()
        .execute(&mut execution, &mut command_context)
        .expect("outbound send-event should publish and wait");
    while let Some(operation) = command_context.agenda().pop_operation() {
        operation.run(&mut command_context).unwrap();
    }

    // Production path: TriggerSendEventServiceTaskCmd (Java TriggerCmd + TriggerExecutionOperation)
    TriggerSendEventServiceTaskCmd::new(
        "send-and-receive-result-execution",
        "orderAccepted",
        json!({ "acceptedBy": "Kai" }),
    )
    .execute(&mut command_context)
    .expect("production trigger cmd should leave send-event wait");
    while let Some(operation) = command_context.agenda().pop_operation() {
        operation.run(&mut command_context).unwrap();
    }

    let stored = {
        let (store, sess) = command_context.store_and_session();
        store
            .find_execution("send-and-receive-result-execution", sess)
            .expect("execution after trigger")
    };
    assert_eq!(stored.process_variable("acceptedBy"), Some(json!("Kai")));
    assert_eq!(
        stored.process_variable("acceptedByFromResult"),
        Some(json!("Kai")),
        "generic outParameters must resolve against result payload"
    );
    let result = stored
        .process_variable("sendEventResult")
        .expect("resultVariableName must be written on trigger");
    assert_eq!(result["service"], json!("send-event"));
    assert_eq!(result["triggerEventType"], json!("orderAccepted"));
    assert_eq!(result["payload"]["acceptedBy"], json!("Kai"));

    let wait_gone = {
        let (store, sess) = command_context.store_and_session();
        store
            .find_event_wait_state_by_execution_id("send-and-receive-result-execution", sess)
            .is_none()
    };
    assert!(wait_gone, "cmd path must consume the send-event wait state");

    let tasks = {
        let (store, sess) = command_context.store_and_session();
        store.find_tasks_by_process_instance_id("send-and-receive-result-pi", sess)
    };
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].name, "After");
}

#[test]
fn validator_still_rejects_delegate_implementation_with_skip_expression() {
    let converter = BpmnXMLConverter::new();
    let mut model = converter.convert_to_bpmn_model(SKIP_SERVICE_TASK_XML);
    for process in &mut model.processes {
        let updated_service_task =
            if let Some(service_task) = process_service_task_mut(process, "httpTask1") {
                service_task.implementation_type = Some("delegateExpression".to_string());
                service_task.implementation = Some("${delegate}".to_string());
                Some(service_task.clone())
            } else {
                None
            };
        if let Some(service_task) = updated_service_task {
            process.flow_element_map.insert(
                "httpTask1".to_string(),
                FlowElementEnum::ServiceTask(service_task),
            );
        }
    }
    if let Some(main_process) = &mut model.main_process {
        let updated_service_task =
            if let Some(service_task) = process_service_task_mut(main_process, "httpTask1") {
                service_task.implementation_type = Some("delegateExpression".to_string());
                service_task.implementation = Some("${delegate}".to_string());
                Some(service_task.clone())
            } else {
                None
            };
        if let Some(service_task) = updated_service_task {
            main_process.flow_element_map.insert(
                "httpTask1".to_string(),
                FlowElementEnum::ServiceTask(service_task),
            );
        }
    }

    let err = UnsupportedModelValidator::validate(&model, &ProcessEngineConfiguration::default())
        .expect_err("skipExpression must not allow unsupported delegate service task shapes");

    assert!(
        err.to_string().contains("implementation delegates"),
        "expected delegate rejection, got {err}"
    );
}

#[test]
fn failed_timer_job_with_no_retries_is_visible_as_deadletter() {
    let time_source = Arc::new(TestTimeSource::new(chrono::Utc::now()));
    let process_engine = ProcessEngine::build_with_config(
        "timer-job-deadletter-test".to_string(),
        time_source.clone(),
        ProcessEngineConfiguration {
            http_service: flowable_engine::service::config::HttpServiceTaskConfiguration {
                runtime_mode: HttpServiceRuntimeMode::Real,
                real_client: RealHttpClientConfiguration {
                    allow_private_networks: true,
                    ..Default::default()
                },
                ..Default::default()
            },
            ..Default::default()
        },
    )
    .expect("engine should build");

    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();
    let management_service = process_engine.get_management_service();

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
                 xmlns:flowable="http://flowable.org/bpmn"
                 targetNamespace="Examples">
        <process id="timerHttpFailureProcess" name="Timer HTTP Failure Process">
            <startEvent id="startEvent1" />
            <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="timerCatch1" />
            <intermediateCatchEvent id="timerCatch1" name="Timer Catch">
                <timerEventDefinition>
                    <timeDuration>PT5M</timeDuration>
                </timerEventDefinition>
            </intermediateCatchEvent>
            <sequenceFlow id="flow2" sourceRef="timerCatch1" targetRef="httpTask1" />
            <serviceTask id="httpTask1" name="Invoke Failing HTTP" flowable:type="http">
                <extensionElements>
                    <flowable:requestMethod>GET</flowable:requestMethod>
                    <flowable:requestUrl>http://127.0.0.1:9/unavailable</flowable:requestUrl>
                </extensionElements>
            </serviceTask>
            <sequenceFlow id="flow3" sourceRef="httpTask1" targetRef="endEvent1" />
            <endEvent id="endEvent1" />
        </process>
    </definitions>"#;

    repository_service
        .deploy(
            repository_service
                .create_deployment()
                .name("Timer HTTP Failure Deployment".to_string())
                .add_string(
                    "timerHttpFailureProcess.bpmn20.xml".to_string(),
                    xml.to_string(),
                ),
        )
        .unwrap();

    let process_definition_id = repository_service.get_process_definition_ids().unwrap()[0].clone();
    let process_instance = runtime_service
        .start_process_instance(
            runtime_service
                .create_process_instance_builder()
                .process_definition_id(process_definition_id),
        )
        .unwrap();

    let runtime_store = process_engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let timer_jobs = runtime_store
        .find_timer_job_states_by_process_instance_id(&process_instance.id, &mut session);
    assert_eq!(timer_jobs.len(), 1, "process should wait on one timer job");
    let mut timer_job = timer_jobs
        .into_iter()
        .next()
        .expect("process should wait on one timer job");
    timer_job.retries = Some(1);
    runtime_store.insert_timer_job_state(&timer_job, &mut session);
    session.flush_and_commit().unwrap();

    time_source.advance_time(5 * 60 * 1000);

    let executed = runtime_service.run_due_timers().unwrap();
    assert!(
        executed.is_empty(),
        "failing timer work should not be reported as executed"
    );

    assert!(
        management_service
            .find_timer_job_by_id(&timer_job.timer_job_id)
            .is_none(),
        "exhausted failed job should no longer be an executable timer job"
    );

    let deadletter = management_service
        .find_deadletter_job_by_id(&timer_job.timer_job_id)
        .expect("exhausted failed job should be available as deadletter");

    assert_eq!(deadletter.retries, Some(0));
    assert_eq!(deadletter.job_state.as_deref(), Some("deadletter"));
    assert!(
        deadletter
            .error_message
            .as_deref()
            .is_some_and(|message| message.contains("127.0.0.1:9")),
        "deadletter should retain the runtime exception message, got {:?}",
        deadletter.error_message
    );
    assert!(
        management_service
            .list_deadletter_jobs()
            .iter()
            .any(|job| job.timer_job_id == timer_job.timer_job_id)
    );
}

// ─── P51 S4: class/delegateExpression triggerable ───────────────────────────

#[test]
fn triggerable_attribute_is_parsed_from_bpmn_xml_for_delegate_expression() {
    let model = triggerable_delegate_expression_model();
    assert!(
        service_task(&model, "delegateTask1").triggerable,
        "flowable:triggerable=true must be parsed onto ServiceTask"
    );
}

#[test]
fn validator_allows_triggerable_delegate_expression_and_class() {
    UnsupportedModelValidator::validate(
        &triggerable_delegate_expression_model(),
        &ProcessEngineConfiguration::default(),
    )
    .expect("triggerable delegateExpression is owned after P51 S4");

    UnsupportedModelValidator::validate(
        &triggerable_class_service_task_model(),
        &ProcessEngineConfiguration::default(),
    )
    .expect("triggerable class is owned after P51 S4");
}

#[test]
fn triggerable_delegate_expression_executes_but_does_not_leave_until_trigger() {
    // Java ServiceTaskDelegateExpressionActivityBehavior.java:179-181 — if (!triggerable) leave
    let (mut command_context, mut execution) = run_triggerable_local_delegate_until_waiting(false)
        .expect("triggerable delegateExpression should execute the local delegate");

    assert_eq!(
        execution
            .persistent_process_variable("delegateResult")
            .and_then(|value| value["delegate"].as_str().map(Value::from)),
        Some(json!("recording")),
        "delegate must run during execute even when triggerable"
    );
    assert_eq!(execution.activity_id.as_deref(), Some("delegateTask1"));

    let tasks_empty = {
        let (store, sess) = command_context.store_and_session();
        store
            .find_tasks_by_process_instance_id("triggerable-delegate-process-instance", sess)
            .is_empty()
    };
    assert!(
        tasks_empty,
        "triggerable delegateExpression must not leave until external trigger"
    );

    ServiceTaskActivityBehavior::new()
        .trigger(&mut execution, &mut command_context, None, None)
        .expect("trigger should leave the waiting triggerable service task");
    while let Some(operation) = command_context.agenda().pop_operation() {
        operation
            .run(&mut command_context)
            .expect("outgoing after trigger should run");
    }

    let tasks = {
        let (store, sess) = command_context.store_and_session();
        store.find_tasks_by_process_instance_id("triggerable-delegate-process-instance", sess)
    };
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].name, "After Trigger");
}

#[test]
fn triggerable_class_service_task_executes_but_does_not_leave_until_trigger() {
    // Java ServiceTaskJavaDelegateActivityBehavior.java:139-141 — if (!triggerable) leave
    let (mut command_context, mut execution) = run_triggerable_local_delegate_until_waiting(true)
        .expect("triggerable class should execute the registered local delegate");

    assert_eq!(
        execution
            .persistent_process_variable("delegateResult")
            .and_then(|value| value["fields"]["literalGreeting"].as_str().map(Value::from)),
        Some(json!("class-hello"))
    );

    let tasks_empty = {
        let (store, sess) = command_context.store_and_session();
        store
            .find_tasks_by_process_instance_id("triggerable-class-process-instance", sess)
            .is_empty()
    };
    assert!(
        tasks_empty,
        "triggerable class service task must not leave until external trigger"
    );

    ServiceTaskActivityBehavior::new()
        .trigger(&mut execution, &mut command_context, None, None)
        .expect("trigger should leave the waiting class service task");
    while let Some(operation) = command_context.agenda().pop_operation() {
        operation
            .run(&mut command_context)
            .expect("outgoing after class trigger should run");
    }

    let tasks = {
        let (store, sess) = command_context.store_and_session();
        store.find_tasks_by_process_instance_id("triggerable-class-process-instance", sess)
    };
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].name, "After Class Trigger");
}

#[test]
fn non_triggerable_delegate_expression_rejects_trigger() {
    let (mut command_context, mut execution) = run_delegate_expression_service_task(true)
        .expect("non-triggerable delegate should execute and leave");

    let err = ServiceTaskActivityBehavior::new()
        .trigger(&mut execution, &mut command_context, None, None)
        .expect_err("non-triggerable class/delegateExpression must reject trigger");
    assert!(
        err.to_string().contains("not triggerable"),
        "expected not-triggerable error, got {err}"
    );
}
