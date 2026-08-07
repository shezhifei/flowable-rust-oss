use crate::agenda::FlowableEngineAgenda;
use crate::agenda::future_operations::{
    PENDING_FUTURE_ID_VARIABLE, PendingOperationResult, WaitForFutureContinuation,
    plan_wait_for_future, resolve_pending_future_registry,
};
use crate::bpmn::fault::{
    EngineFault, clear_boundaries_for_execution, propagate_bpmn_error,
    register_error_boundaries_for_execution, uncaught_bpmn_error,
};
use crate::bpmn::http_handler::{
    HTTP_HANDLER_REGISTRY_CACHE_KEY, HttpHandlerRegistry, HttpRequestHandler,
    HttpRequestHandlerContext, HttpResponseHandlerPlan, SecureScriptHttpHandler,
};
use crate::bpmn::http_task::{
    HttpExecutionMode, HttpTaskOutcome, HttpTaskSpec, JavaHttpContract, PendingHttpCompletion,
    RustHttpProjection, parse_status_codes, project_java_request_variables,
};
use crate::delegate::activity_behavior::{ActivityBehavior, TriggerableActivityBehavior};
use crate::el::expression::{Expression, SimpleExpression};
use crate::error::FlowableError;
use crate::interceptor::command_context::CommandContext;
use crate::persistence::runtime_store::{
    EventRegistryEventDirection, EventRegistryEventInstanceDelivery,
    EventRegistryEventInstanceStatus, EventSubscription, EventSubscriptionKind, HttpTaskRecord,
    HttpTaskRecordStatus, MailOutboxRecord, MailOutboxStatus, RuntimeEventWaitKind,
    RuntimeEventWaitState,
};
use crate::runtime::execution::Execution;
use flowable_bpmn_model::model::{
    ExtensionElement, FieldExtension, FlowElementEnum, IOParameter, ServiceTask,
};
use flowable_http_service::{BasicAuth, HttpRequest, HttpRuntimeMode};
use flowable_mail_service::{MailAttachment, MailMessage};
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::OnceLock;
use uuid::Uuid;

pub const SERVICE_TASK_DELEGATE_REGISTRY_CACHE_KEY: &str = "flowable.serviceTaskDelegateRegistry";

pub struct LocalServiceTaskDelegateContext<'a> {
    pub service_task_id: &'a str,
    pub execution: &'a mut Execution,
    pub fields: &'a Map<String, Value>,
}

pub trait LocalServiceTaskDelegate: Send + Sync {
    fn execute(
        &self,
        context: &mut LocalServiceTaskDelegateContext<'_>,
    ) -> Result<Value, FlowableError>;
}

/// In-process registry for service-task delegates.
///
/// Keys are either resolved delegateExpression names or class FQCN-like
/// strings (M76). This is not JVM classloading — callers register Rust
/// implementations under those names before execution.
#[derive(Clone, Default)]
pub struct LocalServiceTaskDelegateRegistry {
    delegates: BTreeMap<String, Arc<dyn LocalServiceTaskDelegate>>,
}

impl std::fmt::Debug for LocalServiceTaskDelegateRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LocalServiceTaskDelegateRegistry")
            .field("delegates", &self.delegates.keys().collect::<Vec<_>>())
            .finish()
    }
}

impl LocalServiceTaskDelegateRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(
        &mut self,
        name: impl Into<String>,
        delegate: Arc<dyn LocalServiceTaskDelegate>,
    ) {
        self.delegates.insert(name.into(), delegate);
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn LocalServiceTaskDelegate>> {
        self.delegates.get(name).cloned()
    }
}

pub struct ServiceTaskActivityBehavior;

enum HttpServiceTaskExecution {
    Completed(Value),
    BpmnFaultHandled,
}

struct ResolvedRequestHandler {
    handler: Arc<dyn HttpRequestHandler>,
    fields: BTreeMap<String, Value>,
}

impl Default for ServiceTaskActivityBehavior {
    fn default() -> Self {
        Self::new()
    }
}

impl ServiceTaskActivityBehavior {
    pub fn new() -> Self {
        Self
    }

    fn resolve_service_task(
        &self,
        execution: &Execution,
        command_context: &mut CommandContext,
    ) -> Result<ServiceTask, FlowableError> {
        let activity_id = execution.activity_id.as_ref().ok_or_else(|| {
            FlowableError::ExecutionError("Service task execution has no activity_id".to_string())
        })?;
        let process_def_id = execution.process_definition_id.as_ref().ok_or_else(|| {
            FlowableError::ExecutionError(
                "Service task execution has no process_definition_id".to_string(),
            )
        })?;

        let model = command_context
            .deployment_manager
            .get_bpmn_model(process_def_id)
            .ok_or_else(|| {
                FlowableError::ExecutionError(format!(
                    "No BPMN model found for process definition: {}",
                    process_def_id
                ))
            })?;
        let process = model.main_process.as_ref().ok_or_else(|| {
            FlowableError::ExecutionError("No main process in BPMN model".to_string())
        })?;

        let flow_element =
            crate::agenda::continue_process_operation::find_flow_element(process, activity_id)
                .ok_or_else(|| {
                    FlowableError::ExecutionError(format!(
                        "ServiceTask element '{}' not found in process model",
                        activity_id
                    ))
                })?;

        match flow_element {
            FlowElementEnum::ServiceTask(service_task) => Ok(service_task.clone()),
            _ => Err(FlowableError::ExecutionError(
                "Activity is not a ServiceTask element".to_string(),
            )),
        }
    }
}

impl ActivityBehavior for ServiceTaskActivityBehavior {
    fn execute(
        &self,
        execution: &mut Execution,
        command_context: &mut CommandContext,
    ) -> Result<(), FlowableError> {
        let service_task = self.resolve_service_task(execution, command_context)?;
        let evaluation_execution =
            crate::engine::variable_service::evaluation_execution(command_context, execution);
        if crate::bpmn::skip_expression::should_skip_flow_element(
            service_task.skip_expression.as_deref(),
            "ServiceTask",
            evaluation_execution.activity_id.as_deref(),
            &evaluation_execution,
        )? {
            if !should_defer_outgoing_to_multi_instance_parent(execution, command_context) {
                command_context
                    .agenda
                    .plan_take_outgoing_sequence_flows_operation(execution.clone());
            }
            return Ok(());
        }

        let task_type = service_task
            .task_type
            .as_deref()
            .map(str::to_lowercase)
            .unwrap_or_default();

        apply_service_task_in_parameters(&service_task, execution)?;

        match task_type.as_str() {
            "http" => {
                register_error_boundaries_for_execution(execution, command_context)?;
                let spec = build_http_task_spec(&service_task, execution, command_context)?;
                if command_context.is_automatic_job_execution()
                    || spec.execution_mode(command_context.http_runtime.mode())
                        == HttpExecutionMode::ParallelInSameTransaction
                {
                    execute_async_http_service_task(
                        &service_task,
                        spec,
                        execution,
                        command_context,
                    )?;
                    return Ok(());
                }
                match execute_http_service_task(&service_task, spec, execution, command_context)? {
                    HttpServiceTaskExecution::Completed(result) => {
                        apply_service_task_result_and_out_parameters(
                            &service_task,
                            execution,
                            Some(result),
                        )?;
                    }
                    HttpServiceTaskExecution::BpmnFaultHandled => return Ok(()),
                }
            }
            "shell" => {
                // Security deviation from Java: Java ShellActivityBehavior is enabled by
                // default (known dangerous default). Require explicit shell_tasks_enabled.
                if !command_context.config.shell_tasks_enabled {
                    return Err(FlowableError::ExecutionError(format!(
                        "Shell service task '{}' is disabled. Set ProcessEngineConfiguration.\
                         shell_tasks_enabled = true to enable shell tasks \
                         (security deviation from Java; Java ShellActivityBehavior is \
                         enabled by default).",
                        activity_id(&service_task)
                    )));
                }
                let result = execute_shell_service_task(&service_task, execution, command_context)?;
                apply_service_task_result_and_out_parameters(
                    &service_task,
                    execution,
                    Some(result),
                )?;
            }
            // P138: Java mail has no resultVariableName consumer.
            // DefaultActivityBehaviorFactory.java:234-239 (serviceTask type=mail) and
            // :242-244 (sendTask type=mail) only wrap BpmnMailActivityDelegate — no
            // resultVariable wiring. getResultVariableName() is consumed only by
            // ServiceTaskExpressionActivityBehavior and businessRuleTask
            // (DefaultActivityBehaviorFactory.java:385-386). BpmnMailActivityDelegate
            // and flowable-mail-engine have zero resultVariable hits. Drop the send
            // payload (and outParameters) rather than writing a Rust-only super-set
            // variable; outbox (MailOutboxRecord) already holds the sent content.
            "mail" => {
                let _ = execute_mail_service_task(&service_task, execution, command_context)?;
            }
            // Java DmnActivityBehavior.java:58-195 — serviceTask flowable:type="dmn"
            "dmn" => {
                execute_dmn_service_task(&service_task, execution, command_context)?;
            }
            "send-event" => {
                if let Some(result) =
                    execute_send_event_service_task(&service_task, execution, command_context)?
                {
                    apply_service_task_result_and_out_parameters(
                        &service_task,
                        execution,
                        Some(result),
                    )?;
                    command_context
                        .agenda
                        .plan_take_outgoing_sequence_flows_operation(execution.clone());
                }
                return Ok(());
            }
            // Java ExternalWorkerTaskActivityBehavior: create external-worker job and wait.
            // skipExpression is handled above (common ServiceTask path) — when skip is true
            // we never reach here and already left. jobCategory + interceptor live on the
            // create path in external_worker_service::create_external_worker_service_task_job.
            "external-worker" => {
                crate::engine::external_worker_service::create_external_worker_service_task_job(
                    &service_task,
                    execution,
                    command_context,
                )?;
                return Ok(());
            }
            _ => {
                if is_local_delegate_service_task(&service_task) {
                    if try_execute_async_local_delegate_service_task(
                        &service_task,
                        execution,
                        command_context,
                        &evaluation_execution,
                    )? {
                        // Async path plans WaitForFutureOperation; do not take outgoing yet.
                        return Ok(());
                    }
                    let result = execute_local_delegate_service_task(
                        &service_task,
                        execution,
                        command_context,
                        &evaluation_execution,
                    )?;
                    apply_service_task_result_and_out_parameters(
                        &service_task,
                        execution,
                        Some(result),
                    )?;
                    // Java: if (triggerable) do not leave — wait for external trigger.
                    if service_task.triggerable {
                        command_context
                            .execution_entity_manager
                            .update(execution, &mut command_context.session);
                        return Ok(());
                    }
                } else {
                    apply_service_task_result_and_out_parameters(&service_task, execution, None)?;
                }
            }
        }

        if !should_defer_outgoing_to_multi_instance_parent(execution, command_context) {
            command_context
                .agenda
                .plan_take_outgoing_sequence_flows_operation(execution.clone());
        }
        Ok(())
    }
}

fn execute_async_http_service_task(
    service_task: &ServiceTask,
    mut spec: HttpTaskSpec,
    execution: &mut Execution,
    command_context: &mut CommandContext,
) -> Result<(), FlowableError> {
    if !command_context.config.http_service.enabled {
        return Err(FlowableError::ExecutionError(
            "HTTP service tasks are disabled in the current engine configuration".to_string(),
        ));
    }
    if let Some(handler) = resolve_http_request_handler(service_task, execution, command_context)? {
        let mut context = HttpRequestHandlerContext {
            execution,
            request: &mut spec.request,
            fields: &handler.fields,
        };
        handler.handler.handle_request(&mut context)?;
    }
    project_java_request_variables(execution, &spec.java, &spec.request);
    let response_handler = resolve_http_response_handler(service_task, execution, command_context)?;
    let registry = resolve_pending_future_registry(command_context).ok_or_else(|| {
        FlowableError::ExecutionError(
            "PendingFutureRegistry is not available for async HTTP service task".to_string(),
        )
    })?;
    let pending = registry.create();
    let future_id = pending.id.clone();
    let runtime = Arc::clone(&command_context.http_runtime);
    let spec_for_future = spec.clone();
    let response_handler_for_future = response_handler;
    async_http_runtime()?.spawn(async move {
        match runtime
            .execute_async_with_status(&spec_for_future.request)
            .await
        {
            Ok(exchange) => {
                pending.complete_operation(PendingOperationResult::Http(PendingHttpCompletion {
                    spec: spec_for_future,
                    transport_result: Ok(exchange),
                    response_handler: response_handler_for_future,
                }));
            }
            Err(error) if spec_for_future.java.ignore_exception => {
                pending.complete_operation(PendingOperationResult::Http(PendingHttpCompletion {
                    spec: spec_for_future,
                    transport_result: Err(error.to_string()),
                    response_handler: response_handler_for_future,
                }));
            }
            Err(error) => pending.complete(Err(error.to_string())),
        }
    });
    execution.set_transient_variable(
        PENDING_FUTURE_ID_VARIABLE.to_string(),
        Value::String(future_id.clone()),
    );
    command_context
        .execution_entity_manager
        .update(execution, &mut command_context.session);
    plan_wait_for_future(
        command_context,
        future_id,
        execution.clone(),
        WaitForFutureContinuation {
            result_variable_name: spec.rust.result_variable_name.clone(),
            store_result_as_transient: spec.rust.store_result_as_transient,
            use_local_scope: spec.rust.use_local_scope,
        },
    )?;
    Ok(())
}

fn async_http_runtime() -> Result<&'static tokio::runtime::Runtime, FlowableError> {
    static RUNTIME: OnceLock<Result<tokio::runtime::Runtime, String>> = OnceLock::new();
    RUNTIME
        .get_or_init(|| {
            tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .thread_name("flowable-http-io")
                .build()
                .map_err(|error| error.to_string())
        })
        .as_ref()
        .map_err(|error| {
            FlowableError::ExecutionError(format!(
                "failed to initialize async HTTP runtime: {error}"
            ))
        })
}

impl TriggerableActivityBehavior for ServiceTaskActivityBehavior {
    fn trigger(
        &self,
        execution: &mut Execution,
        command_context: &mut CommandContext,
        signal_name: Option<String>,
        signal_data: Option<Value>,
    ) -> Result<(), FlowableError> {
        let service_task = self.resolve_service_task(execution, command_context)?;
        let task_type = service_task
            .task_type
            .as_deref()
            .map(str::to_lowercase)
            .unwrap_or_default();
        if task_type == "send-event" {
            return trigger_send_event_service_task(
                &service_task,
                execution,
                command_context,
                signal_name,
                signal_data.unwrap_or(Value::Null),
            );
        }

        // External-worker wait-state leave (Java ExternalWorkerTaskActivityBehavior#trigger).
        if task_type == "external-worker" {
            if !should_defer_outgoing_to_multi_instance_parent(execution, command_context) {
                command_context
                    .agenda
                    .plan_take_outgoing_sequence_flows_operation(execution.clone());
            }
            return Ok(());
        }

        // Java ServiceTaskDelegateExpressionActivityBehavior / ServiceTaskJavaDelegateActivityBehavior:
        // any class/delegateExpression task marked triggerable waits after execute; trigger leaves.
        // LocalServiceTaskDelegate has no trigger hook (unlike TriggerableJavaDelegate), so leave only.
        if is_local_delegate_service_task(&service_task) {
            if !service_task.triggerable {
                return Err(FlowableError::ExecutionError(format!(
                    "Service task '{}' is not triggerable",
                    activity_id(&service_task)
                )));
            }
            if !should_defer_outgoing_to_multi_instance_parent(execution, command_context) {
                command_context
                    .agenda
                    .plan_take_outgoing_sequence_flows_operation(execution.clone());
            }
            return Ok(());
        }

        Err(FlowableError::ExecutionError(format!(
            "Service task '{}' does not support trigger",
            activity_id(&service_task)
        )))
    }
}

/// `class` and `delegateExpression` both resolve through
/// [`LocalServiceTaskDelegateRegistry`]. `class` uses the implementation string
/// as a registry key (FQCN-like); `delegateExpression` evaluates `${...}` first.
fn is_local_delegate_service_task(service_task: &ServiceTask) -> bool {
    service_task
        .implementation_type
        .as_deref()
        .is_some_and(|kind| kind == "delegateExpression" || kind == "class")
}

fn execute_local_delegate_service_task(
    service_task: &ServiceTask,
    execution: &mut Execution,
    command_context: &CommandContext,
    evaluation_execution: &Execution,
) -> Result<Value, FlowableError> {
    let activity_id = activity_id(service_task);
    let implementation_type = service_task
        .implementation_type
        .as_deref()
        .unwrap_or("delegateExpression");
    let implementation = service_task
        .implementation
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            FlowableError::ExecutionError(format!(
                "Delegate service task '{}' requires an implementation",
                activity_id
            ))
        })?;

    let delegate_name = resolve_local_delegate_name(
        implementation_type,
        implementation,
        &activity_id,
        evaluation_execution,
    )?;

    let fields = resolve_field_extensions(service_task, evaluation_execution)?;
    let delegate = command_context
        .session_caches
        .get(SERVICE_TASK_DELEGATE_REGISTRY_CACHE_KEY)
        .and_then(|registry| registry.downcast_ref::<LocalServiceTaskDelegateRegistry>())
        .and_then(|registry| registry.get(&delegate_name))
        .ok_or_else(|| {
            FlowableError::ExecutionError(format!(
                "No local service task delegate '{}' is registered for service task '{}'",
                delegate_name, activity_id
            ))
        })?;

    let mut context = LocalServiceTaskDelegateContext {
        service_task_id: &activity_id,
        execution,
        fields: &fields,
    };
    delegate.execute(&mut context)
}

/// When the resolved delegate is registered as async-capable, submit background work
/// and plan WaitForFutureOperation. Returns true if the async path was taken.
fn try_execute_async_local_delegate_service_task(
    service_task: &ServiceTask,
    execution: &mut Execution,
    command_context: &mut CommandContext,
    evaluation_execution: &Execution,
) -> Result<bool, FlowableError> {
    let activity_id = activity_id(service_task);
    let implementation_type = service_task
        .implementation_type
        .as_deref()
        .unwrap_or("delegateExpression");
    let implementation = service_task
        .implementation
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            FlowableError::ExecutionError(format!(
                "Delegate service task '{}' requires an implementation",
                activity_id
            ))
        })?;

    let delegate_name = resolve_local_delegate_name(
        implementation_type,
        implementation,
        &activity_id,
        evaluation_execution,
    )?;

    if !crate::bpmn::behavior::async_delegate_activity_behavior::is_async_delegate_registered(
        command_context,
        &delegate_name,
    ) {
        return Ok(false);
    }

    let fields = resolve_field_extensions(service_task, evaluation_execution)?;
    crate::bpmn::behavior::async_delegate_activity_behavior::execute_async_local_delegate_service_task(
        service_task,
        execution,
        command_context,
        &delegate_name,
        fields,
    )?;
    Ok(true)
}

fn resolve_local_delegate_name(
    implementation_type: &str,
    implementation: &str,
    activity_id: &str,
    execution: &Execution,
) -> Result<String, FlowableError> {
    if implementation_type == "class" {
        // `class` is a registry key (Rust mapping of FQCN), not JVM classloading.
        return Ok(implementation.to_string());
    }

    // delegateExpression: evaluate `${name}` to a string registry key.
    let resolved = SimpleExpression::new(implementation.to_string())
        .get_value(execution)
        .ok_or_else(|| {
            FlowableError::ExecutionError(format!(
                "Delegate expression service task '{}' could not resolve delegateExpression '{}'",
                activity_id, implementation
            ))
        })?;
    resolved.as_str().map(|s| s.to_string()).ok_or_else(|| {
        FlowableError::ExecutionError(format!(
            "Delegate expression service task '{}' resolved delegateExpression '{}' to a non-string value",
            activity_id, implementation
        ))
    })
}

fn resolve_field_extensions(
    service_task: &ServiceTask,
    execution: &Execution,
) -> Result<Map<String, Value>, FlowableError> {
    let mut fields = Map::new();
    for field in &service_task.task.activity.field_extensions {
        let name = field
            .field_name
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                FlowableError::ExecutionError(format!(
                    "Delegate expression service task '{}' has a field extension without name",
                    activity_id(service_task)
                ))
            })?;
        let value = field_extension_value(service_task, field, execution)?;
        fields.insert(name.to_string(), value);
    }
    Ok(fields)
}

fn field_extension_value(
    service_task: &ServiceTask,
    field: &FieldExtension,
    execution: &Execution,
) -> Result<Value, FlowableError> {
    let string_value = field
        .string_value
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let expression = field
        .expression
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    match (string_value, expression) {
        (Some(value), None) => Ok(Value::String(value.to_string())),
        (None, Some(expression)) => SimpleExpression::new(expression.to_string())
            .get_value(execution)
            .ok_or_else(|| {
                FlowableError::ExecutionError(format!(
                    "Delegate expression service task '{}' could not resolve field '{}' expression '{}'",
                    activity_id(service_task),
                    field.field_name.as_deref().unwrap_or_default(),
                    expression
                ))
            }),
        _ => Err(FlowableError::ExecutionError(format!(
            "Delegate expression service task '{}' field '{}' must define exactly one of stringValue or expression",
            activity_id(service_task),
            field.field_name.as_deref().unwrap_or_default()
        ))),
    }
}

fn execute_send_event_service_task(
    service_task: &ServiceTask,
    execution: &mut Execution,
    command_context: &mut CommandContext,
) -> Result<Option<Value>, FlowableError> {
    let event_definition_key = service_task
        .event_type
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            FlowableError::ExecutionError(format!(
                "Send event service task '{}' requires eventType",
                activity_id(service_task)
            ))
        })?;

    let store = &command_context.runtime_store;
    let definition = store
        .find_event_registry_event_definition_by_key_and_tenant(
            event_definition_key,
            execution.tenant_id.as_deref(),
            &mut command_context.session,
        )
        .ok_or_else(|| {
            FlowableError::NotFound(format!(
                "Event Registry event definition '{}' was not found",
                event_definition_key
            ))
        })?;
    let channel = store
        .find_event_registry_channel_definition_by_key_and_tenant(
            &definition.channel_key,
            definition.tenant_id.as_deref(),
            &mut command_context.session,
        )
        .ok_or_else(|| {
            FlowableError::NotFound(format!(
                "Event Registry channel '{}' was not found",
                definition.channel_key
            ))
        })?;

    if channel.channel_type != "outbound" {
        return Err(FlowableError::BadRequest(format!(
            "Event definition '{}' is not bound to an outbound channel",
            definition.key
        )));
    }

    let mut payload = Map::new();
    for parameter in &service_task.event_in_parameters {
        let target = parameter
            .target
            .as_deref()
            .or(parameter.target_expression.as_deref())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                FlowableError::ExecutionError(format!(
                    "Send event service task '{}' has an eventInParameter without target",
                    activity_id(service_task)
                ))
            })?;

        let value = if let Some(source_expression) = parameter.source_expression.as_deref() {
            SimpleExpression::new(source_expression.to_string())
                .get_value(execution)
                .ok_or_else(|| {
                    FlowableError::ExecutionError(format!(
                        "Send event service task '{}' could not resolve sourceExpression '{}'",
                        activity_id(service_task),
                        source_expression
                    ))
                })?
        } else if let Some(source) = parameter.source.as_deref() {
            execution
                .process_variable(source)
                .unwrap_or_else(|| Value::String(source.to_string()))
        } else {
            Value::Null
        };

        payload.insert(target.to_string(), value);
    }

    // Java SendEventTaskActivityBehavior.java:103-110 → EventRegistry.sendEventOutbound
    // → DefaultOutboundEventProcessor.java:32-66 (transform + channel adapter).
    // Rust: event-registry-service depends on engine (cycle if engine called service
    // directly). Dispatch goes through ProcessEngineConfiguration.outbound_event_dispatch
    // (P94 option b); service installs a configuration-backed hook at construction.
    // Failure semantics mirror runtime/mod.rs:572-698 (Created → Failed, retryable).
    let now = store.time_source().now().timestamp_millis();
    let dispatch_token = format!("dispatch:{}", Uuid::new_v4());
    let mut delivery = EventRegistryEventInstanceDelivery {
        id: format!("event-instance:{}", Uuid::new_v4()),
        event_definition_id: definition.id.clone(),
        event_definition_key: definition.key.clone(),
        event_type: definition.event_type.clone(),
        channel_key: definition.channel_key.clone(),
        direction: EventRegistryEventDirection::Outbound,
        status: EventRegistryEventInstanceStatus::Created,
        status_history: vec![EventRegistryEventInstanceStatus::Created],
        last_error: None,
        retry_count: 0,
        last_retry_at: None,
        last_failure_at: None,
        next_retry_at: None,
        dispatch_token: Some(dispatch_token.clone()),
        channel_definition_id: Some(channel.id.clone()),
        tenant_id: definition.tenant_id.clone(),
        payload: Value::Object(payload),
        created_at: now,
        updated_at: now,
    };
    store.insert_event_registry_event_instance_delivery(
        delivery.clone(),
        &mut command_context.session,
    )?;

    let dispatch_result = command_context.config.outbound_event_dispatch.dispatch(
        &crate::engine::outbound_event_dispatch::OutboundEventDispatchRequest {
            channel_key: channel.key.clone(),
            channel_configuration: channel.configuration.clone(),
            event_type: definition.event_type.clone(),
            payload: delivery.payload.clone(),
            dispatch_token: Some(dispatch_token),
        },
    );

    if let Err(error) = dispatch_result {
        // mark_delivery_failed(..., is_retry=false): Failed + last_error + retry window.
        // runtime/delivery.rs:20-43
        let message = error.to_string();
        delivery.status = EventRegistryEventInstanceStatus::Failed;
        delivery.updated_at = now;
        delivery.last_error = Some(message);
        delivery.last_failure_at = Some(now);
        delivery.next_retry_at = Some(now);
        if delivery.status_history.last() != Some(&EventRegistryEventInstanceStatus::Failed) {
            delivery
                .status_history
                .push(EventRegistryEventInstanceStatus::Failed);
        }
        store.update_event_registry_event_instance_delivery(
            delivery,
            &mut command_context.session,
        )?;
        return Err(error);
    }

    delivery.status = EventRegistryEventInstanceStatus::Published;
    delivery
        .status_history
        .push(EventRegistryEventInstanceStatus::Published);
    delivery.updated_at = now;
    store.update_event_registry_event_instance_delivery(
        delivery.clone(),
        &mut command_context.session,
    )?;

    let result = json!({
        "service": "send-event",
        "eventDefinitionKey": delivery.event_definition_key,
        "eventType": delivery.event_type,
        "channelKey": delivery.channel_key,
        "status": delivery.status,
        "payload": delivery.payload,
    });

    let trigger_event_type = service_task
        .trigger_event_type
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if service_task.triggerable || trigger_event_type.is_some() {
        let trigger_event_type = trigger_event_type.ok_or_else(|| {
            FlowableError::ExecutionError(format!(
                "Send event service task '{}' requires triggerEventType for triggerable send-and-receive semantics",
                activity_id(service_task)
            ))
        })?;
        let process_instance_id = execution.process_instance_id.clone().ok_or_else(|| {
            FlowableError::ExecutionError(format!(
                "Send event service task '{}' execution has no process_instance_id",
                activity_id(service_task)
            ))
        })?;

        execution.is_active = false;
        command_context
            .execution_entity_manager
            .update(execution, &mut command_context.session);
        // Java SendEventTaskActivityBehavior.java:140-151 — create EventSubscription
        // (event-type / EventRegistry) and do not leave. P130: kind must be
        // EventRegistry so BpmnEventRegistryConsumer can route inbound events
        // (previously Conditional, unreachable from the consumer filter).
        // P134: configuration = CorrelationUtil.getCorrelationKey(
        // ELEMENT_TRIGGER_EVENT_CORRELATION_PARAMETER, …) — Java :140.
        let configuration = crate::bpmn::event_registry_correlation::trigger_event_correlation_key_from_base_element(
            &service_task
                .task
                .activity
                .flow_node
                .flow_element
                .base_element,
            Some(execution),
        );
        store.insert_event_wait_state(
            &RuntimeEventWaitState {
                wait_kind: RuntimeEventWaitKind::SendEventTask,
                process_instance_id,
                execution_id: execution.id.clone(),
                task_id: None,
                activity_id: execution.activity_id.clone(),
                display_name: service_task
                    .task
                    .activity
                    .flow_node
                    .flow_element
                    .name
                    .clone(),
                event_subscription: Some(EventSubscription {
                    kind: EventSubscriptionKind::EventRegistry,
                    event_ref: trigger_event_type.to_string(),
                }),
                configuration,
            },
            &mut command_context.session,
        );
        return Ok(None);
    }

    Ok(Some(result))
}

fn trigger_send_event_service_task(
    service_task: &ServiceTask,
    execution: &mut Execution,
    command_context: &mut CommandContext,
    signal_name: Option<String>,
    signal_data: Value,
) -> Result<(), FlowableError> {
    let expected_event_type = service_task
        .trigger_event_type
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            FlowableError::ExecutionError(format!(
                "Send event service task '{}' requires triggerEventType for trigger",
                activity_id(service_task)
            ))
        })?;

    if signal_name.as_deref() != Some(expected_event_type) {
        return Ok(());
    }

    let Some(wait_state) = command_context
        .runtime_store
        .find_event_wait_state_by_execution_id(&execution.id, &mut command_context.session)
    else {
        return Ok(());
    };
    if wait_state
        .event_subscription
        .as_ref()
        .is_none_or(|subscription| subscription.event_ref != expected_event_type)
    {
        return Ok(());
    }

    map_event_out_parameters(service_task, execution, &signal_data)?;
    let result = json!({
        "service": "send-event",
        "triggerEventType": expected_event_type,
        "payload": signal_data.clone(),
    });
    apply_service_task_result_and_out_parameters(service_task, execution, Some(result))?;
    record_inbound_event_registry_delivery(
        expected_event_type,
        &signal_data,
        execution.tenant_id.as_deref(),
        command_context,
    )?;

    command_context
        .runtime_store
        .delete_event_wait_state_by_execution_id(&execution.id, &mut command_context.session);
    execution.is_active = true;
    command_context
        .execution_entity_manager
        .update(execution, &mut command_context.session);
    command_context
        .agenda
        .plan_take_outgoing_sequence_flows_operation(execution.clone());

    Ok(())
}

fn map_event_out_parameters(
    service_task: &ServiceTask,
    execution: &mut Execution,
    payload: &Value,
) -> Result<(), FlowableError> {
    for parameter in &service_task.event_out_parameters {
        let target = parameter
            .target
            .as_deref()
            .or(parameter.target_expression.as_deref())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                FlowableError::ExecutionError(format!(
                    "Send event service task '{}' has an eventOutParameter without target",
                    activity_id(service_task)
                ))
            })?;

        // Java EventInstanceBpmnUtil.java:122-134 (SendEventServiceTask branch):
        // payload field absent → setVariable(target, null). Do not fall back to
        // the source string (P130 alignment with :127).
        let value = if let Some(source) = parameter.source.as_deref() {
            payload.get(source).cloned().unwrap_or(Value::Null)
        } else if let Some(source_expression) = parameter.source_expression.as_deref() {
            if let Some(property_name) = expression_property_name(source_expression) {
                payload.get(property_name).cloned().unwrap_or(Value::Null)
            } else {
                Value::Null
            }
        } else {
            Value::Null
        };

        execution.set_process_variable(target.to_string(), value);
    }
    Ok(())
}

fn record_inbound_event_registry_delivery(
    event_definition_key: &str,
    payload: &Value,
    execution_tenant_id: Option<&str>,
    command_context: &mut CommandContext,
) -> Result<(), FlowableError> {
    use crate::cmd::trigger_send_event_service_task_cmd::INBOUND_EVENT_DELIVERY_ID_CACHE_KEY;

    // P134: when the inbound pipeline already created a delivery and threaded
    // its id through TriggerSendEventServiceTaskCmd, associate-update that row
    // instead of inserting a second event-instance (Java has a single delivery).
    // Leave status at Received — the pipeline transitions to Processed after
    // consumer return (runtime/mod.rs). Replay is idempotent: same id, update.
    if let Some(delivery_id) = command_context
        .session_caches
        .get(INBOUND_EVENT_DELIVERY_ID_CACHE_KEY)
        .and_then(|value| value.downcast_ref::<String>())
        .cloned()
    {
        let store = &command_context.runtime_store;
        if let Some(mut existing) = store
            .find_event_registry_event_instance_delivery(
                &delivery_id,
                &mut command_context.session,
            )
            .map_err(|e| FlowableError::Internal(e.to_string()))?
        {
            let now = store.time_source().now().timestamp_millis();
            // Keep definition/channel fields from the pipeline row; refresh payload
            // and touch updated_at so the trigger association is visible.
            existing.payload = payload.clone();
            existing.updated_at = now;
            if existing.event_definition_key.is_empty() {
                existing.event_definition_key = event_definition_key.to_string();
            }
            store
                .update_event_registry_event_instance_delivery(
                    existing,
                    &mut command_context.session,
                )
                .map_err(|e| FlowableError::Internal(e.to_string()))?;
            return Ok(());
        }
        // Delivery id was stashed but the row is missing (rare race / purged).
        // Fall through to the insert path so direct-trigger semantics still work.
        tracing::warn!(
            delivery_id = %delivery_id,
            "inbound delivery id stashed for send-event trigger but row not found; inserting fallback delivery"
        );
    }

    // Fallback: direct behavior.trigger / unit tests with no pipeline delivery.
    let store = &command_context.runtime_store;
    let definition = store.find_event_registry_event_definition_by_key_and_tenant(
        event_definition_key,
        execution_tenant_id,
        &mut command_context.session,
    );
    let channel = definition.as_ref().and_then(|definition| {
        store.find_event_registry_channel_definition_by_key_and_tenant(
            &definition.channel_key,
            definition.tenant_id.as_deref(),
            &mut command_context.session,
        )
    });
    let now = store.time_source().now().timestamp_millis();
    let mut delivery = EventRegistryEventInstanceDelivery {
        id: format!("event-instance:{}", Uuid::new_v4()),
        event_definition_id: definition
            .as_ref()
            .map(|definition| definition.id.clone())
            .unwrap_or_else(|| event_definition_key.to_string()),
        event_definition_key: event_definition_key.to_string(),
        event_type: definition
            .as_ref()
            .map(|definition| definition.event_type.clone())
            .unwrap_or_else(|| event_definition_key.to_string()),
        channel_key: definition
            .as_ref()
            .map(|definition| definition.channel_key.clone())
            .unwrap_or_default(),
        direction: EventRegistryEventDirection::Inbound,
        status: EventRegistryEventInstanceStatus::Received,
        status_history: vec![EventRegistryEventInstanceStatus::Received],
        last_error: None,
        retry_count: 0,
        last_retry_at: None,
        last_failure_at: None,
        next_retry_at: None,
        dispatch_token: None,
        channel_definition_id: channel.as_ref().map(|channel| channel.id.clone()),
        tenant_id: definition
            .as_ref()
            .and_then(|definition| definition.tenant_id.clone())
            .or_else(|| execution_tenant_id.map(str::to_string)),
        payload: payload.clone(),
        created_at: now,
        updated_at: now,
    };
    store.insert_event_registry_event_instance_delivery(
        delivery.clone(),
        &mut command_context.session,
    )?;

    delivery.status = EventRegistryEventInstanceStatus::Processed;
    delivery
        .status_history
        .push(EventRegistryEventInstanceStatus::Processed);
    delivery.updated_at = now;
    store.update_event_registry_event_instance_delivery(delivery, &mut command_context.session)?;
    Ok(())
}

fn expression_property_name(expression: &str) -> Option<&str> {
    expression
        .strip_prefix("${")
        .and_then(|value| value.strip_suffix('}'))
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn java_http_contract(
    service_task: &ServiceTask,
    execution: &Execution,
) -> Result<JavaHttpContract, FlowableError> {
    let fallback_prefix = execution
        .activity_id
        .clone()
        .unwrap_or_else(|| "httpTask".to_string());
    Ok(JavaHttpContract {
        ignore_exception: parse_optional_bool_extension(service_task, "ignoreException", "HTTP")?
            .unwrap_or(false),
        save_request_variables: parse_optional_bool_extension(
            service_task,
            "saveRequestVariables",
            "HTTP",
        )?
        .unwrap_or(false),
        save_response_parameters: parse_optional_bool_extension(
            service_task,
            "saveResponseParameters",
            "HTTP",
        )?
        .unwrap_or(false),
        save_response_parameters_transient: parse_optional_bool_extension(
            service_task,
            "saveResponseParametersTransient",
            "HTTP",
        )?
        .unwrap_or(false),
        save_response_variable_as_json: parse_optional_bool_extension(
            service_task,
            "saveResponseVariableAsJson",
            "HTTP",
        )?
        .unwrap_or(false),
        response_variable_name: optional_extension_text(service_task, "responseVariableName"),
        result_variable_prefix: optional_extension_text(service_task, "resultVariablePrefix")
            .unwrap_or(fallback_prefix),
        fail_status_codes: parse_status_codes(optional_extension_text(
            service_task,
            "failStatusCodes",
        )),
        handle_status_codes: parse_status_codes(optional_extension_text(
            service_task,
            "handleStatusCodes",
        )),
        parallel_in_same_transaction: service_task.parallel_in_same_transaction.or(
            parse_optional_bool_extension(service_task, "parallelInSameTransaction", "HTTP")?,
        ),
    })
}

fn resolve_http_request_handler(
    service_task: &ServiceTask,
    execution: &Execution,
    command_context: &mut CommandContext,
) -> Result<Option<ResolvedRequestHandler>, FlowableError> {
    let Some((implementation_type, implementation, fields)) =
        http_handler_definition(service_task, "httpRequestHandler", execution)?
    else {
        return Ok(None);
    };
    if implementation_type.eq_ignore_ascii_case("script") {
        let handler =
            resolve_http_script_handler(service_task, "httpRequestHandler", command_context)?;
        return Ok(Some(ResolvedRequestHandler {
            handler: Arc::new(handler),
            fields,
        }));
    }
    let Some(registry) = command_context
        .session_caches()
        .get(HTTP_HANDLER_REGISTRY_CACHE_KEY)
        .and_then(|entry| entry.downcast_ref::<HttpHandlerRegistry>())
    else {
        return Err(FlowableError::ExecutionError(format!(
            "HTTP request handler '{}' is configured but no handler registry is available",
            implementation
        )));
    };
    let handler = registry.request_handler(&implementation).ok_or_else(|| {
        FlowableError::ExecutionError(format!(
            "No HTTP request handler '{}' is registered",
            implementation
        ))
    })?;
    Ok(Some(ResolvedRequestHandler { handler, fields }))
}

fn resolve_http_response_handler(
    service_task: &ServiceTask,
    execution: &Execution,
    command_context: &mut CommandContext,
) -> Result<Option<HttpResponseHandlerPlan>, FlowableError> {
    let Some((implementation_type, implementation, fields)) =
        http_handler_definition(service_task, "httpResponseHandler", execution)?
    else {
        return Ok(None);
    };
    if implementation_type.eq_ignore_ascii_case("script") {
        let handler =
            resolve_http_script_handler(service_task, "httpResponseHandler", command_context)?;
        return Ok(Some(HttpResponseHandlerPlan {
            handler: Arc::new(handler),
            fields,
        }));
    }
    let Some(registry) = command_context
        .session_caches()
        .get(HTTP_HANDLER_REGISTRY_CACHE_KEY)
        .and_then(|entry| entry.downcast_ref::<HttpHandlerRegistry>())
    else {
        return Err(FlowableError::ExecutionError(format!(
            "HTTP response handler '{}' is configured but no handler registry is available",
            implementation
        )));
    };
    let handler = registry.response_handler(&implementation).ok_or_else(|| {
        FlowableError::ExecutionError(format!(
            "No HTTP response handler '{}' is registered",
            implementation
        ))
    })?;
    Ok(Some(HttpResponseHandlerPlan { handler, fields }))
}

fn http_handler_definition(
    service_task: &ServiceTask,
    element_name: &str,
    execution: &Execution,
) -> Result<Option<(String, String, BTreeMap<String, Value>)>, FlowableError> {
    let typed = match element_name {
        "httpRequestHandler" => service_task.http_request_handler.as_ref(),
        "httpResponseHandler" => service_task.http_response_handler.as_ref(),
        _ => None,
    };
    if let Some(handler) = typed {
        let implementation_type = handler.implementation_type.clone().ok_or_else(|| {
            FlowableError::ExecutionError(format!(
                "HTTP {} is missing implementation type",
                element_name
            ))
        })?;
        let implementation = if implementation_type.eq_ignore_ascii_case("delegateExpression") {
            let expression = handler.implementation.as_deref().unwrap_or_default();
            SimpleExpression::new(expression.to_string())
                .get_value(execution)
                .and_then(|value| value.as_str().map(str::to_string))
                .ok_or_else(|| {
                    FlowableError::ExecutionError(format!(
                        "HTTP {} handler delegateExpression '{}' did not resolve to a string",
                        element_name, expression
                    ))
                })?
        } else {
            handler.implementation.clone().unwrap_or_default()
        };
        let fields = resolve_typed_http_handler_fields(&handler.field_extensions, execution)?;
        return Ok(Some((implementation_type, implementation, fields)));
    }
    let Some(element) = extension_elements(service_task, element_name).first() else {
        return Ok(None);
    };
    let attribute = |name: &str| {
        element
            .base_element
            .attributes
            .get(name)
            .and_then(|values| values.first())
            .and_then(|attribute| attribute.value.clone())
    };
    let (implementation_type, implementation) = if let Some(value) = attribute("class") {
        ("class".to_string(), value)
    } else if let Some(value) = attribute("delegateExpression") {
        let resolved = SimpleExpression::new(value.clone())
            .get_value(execution)
            .and_then(|value| value.as_str().map(str::to_string))
            .ok_or_else(|| {
                FlowableError::ExecutionError(format!(
                    "HTTP {} handler delegateExpression '{}' did not resolve to a string",
                    element_name, value
                ))
            })?;
        ("delegateExpression".to_string(), resolved)
    } else if let Some(value) = attribute("type") {
        (value, String::new())
    } else {
        return Err(FlowableError::ExecutionError(format!(
            "HTTP {} must define class, delegateExpression, or type",
            element_name
        )));
    };
    let fields = resolve_http_handler_fields(element, execution)?;
    Ok(Some((implementation_type, implementation, fields)))
}

fn resolve_typed_http_handler_fields(
    fields: &[FieldExtension],
    execution: &Execution,
) -> Result<BTreeMap<String, Value>, FlowableError> {
    let mut resolved = BTreeMap::new();
    for field in fields {
        let name = field
            .field_name
            .as_deref()
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .ok_or_else(|| {
                FlowableError::ExecutionError(
                    "HTTP handler field extension requires name".to_string(),
                )
            })?;
        let value = match (field.string_value.as_deref(), field.expression.as_deref()) {
            (Some(value), None) => Value::String(value.trim().to_string()),
            (None, Some(expression)) => SimpleExpression::new(expression.trim().to_string())
                .get_value(execution)
                .ok_or_else(|| {
                    FlowableError::ExecutionError(format!(
                        "HTTP handler field '{}' expression '{}' could not be resolved",
                        name, expression
                    ))
                })?,
            _ => {
                return Err(FlowableError::ExecutionError(format!(
                    "HTTP handler field '{}' must define exactly one of stringValue or expression",
                    name
                )));
            }
        };
        resolved.insert(name.to_string(), value);
    }
    Ok(resolved)
}

fn resolve_http_script_handler(
    service_task: &ServiceTask,
    element_name: &str,
    command_context: &CommandContext,
) -> Result<SecureScriptHttpHandler, FlowableError> {
    if !command_context.config.enable_secure_scripting {
        return Err(FlowableError::ExecutionError(format!(
            "HTTP {} script handler requires secure scripting to be enabled",
            element_name
        )));
    }
    let typed = match element_name {
        "httpRequestHandler" => service_task.http_request_handler.as_ref(),
        "httpResponseHandler" => service_task.http_response_handler.as_ref(),
        _ => None,
    };
    if let Some(script_info) = typed.and_then(|handler| handler.script_info.as_ref()) {
        let language = script_info
            .language
            .clone()
            .unwrap_or_else(|| "javascript".to_string());
        crate::scripting::secure_engine::validate_script_task(
            Some(&language),
            command_context.config.enable_secure_scripting,
            &command_context.config.supported_script_languages,
        )?;
        let script = script_info
            .script
            .clone()
            .filter(|script| !script.trim().is_empty())
            .ok_or_else(|| {
                FlowableError::ExecutionError(format!(
                    "HTTP {} script handler requires a non-empty script body",
                    element_name
                ))
            })?;
        return Ok(SecureScriptHttpHandler::new(
            language,
            script,
            script_info.result_variable.clone(),
            command_context.config.supported_script_languages.clone(),
        ));
    }
    let handler = extension_elements(service_task, element_name)
        .first()
        .ok_or_else(|| {
            FlowableError::ExecutionError(format!("HTTP {} is missing", element_name))
        })?;
    let script = handler
        .child_elements
        .get("script")
        .and_then(|scripts| scripts.first())
        .ok_or_else(|| {
            FlowableError::ExecutionError(format!(
                "HTTP {} type='script' requires a flowable:script child",
                element_name
            ))
        })?;
    let script_attribute = |name: &str| {
        script
            .base_element
            .attributes
            .get(name)
            .and_then(|values| values.first())
            .and_then(|attribute| attribute.value.clone())
    };
    let language = script_attribute("language").unwrap_or_else(|| "javascript".to_string());
    crate::scripting::secure_engine::validate_script_task(
        Some(&language),
        command_context.config.enable_secure_scripting,
        &command_context.config.supported_script_languages,
    )?;
    let script_body = script
        .element_text
        .clone()
        .filter(|body| !body.trim().is_empty())
        .ok_or_else(|| {
            FlowableError::ExecutionError(format!(
                "HTTP {} script handler requires a non-empty script body",
                element_name
            ))
        })?;
    Ok(SecureScriptHttpHandler::new(
        language,
        script_body,
        script_attribute("resultVariable"),
        command_context.config.supported_script_languages.clone(),
    ))
}

fn resolve_http_handler_fields(
    element: &flowable_bpmn_model::model::ExtensionElement,
    execution: &Execution,
) -> Result<BTreeMap<String, Value>, FlowableError> {
    let mut fields = BTreeMap::new();
    for field in element.child_elements.get("field").into_iter().flatten() {
        let name = field
            .base_element
            .attributes
            .get("name")
            .and_then(|values| values.first())
            .and_then(|attribute| attribute.value.clone())
            .filter(|name| !name.trim().is_empty())
            .ok_or_else(|| {
                FlowableError::ExecutionError(
                    "HTTP handler field extension requires name".to_string(),
                )
            })?;
        let raw = field
            .child_elements
            .get("expression")
            .and_then(|values| values.first())
            .and_then(|value| value.element_text.clone())
            .or_else(|| {
                field
                    .child_elements
                    .get("string")
                    .and_then(|values| values.first())
                    .and_then(|value| value.element_text.clone())
            })
            .or_else(|| field.element_text.clone())
            .unwrap_or_default();
        let value = if raw.trim().starts_with("${") && raw.trim().ends_with('}') {
            SimpleExpression::new(raw.trim().to_string())
                .get_value(execution)
                .ok_or_else(|| {
                    FlowableError::ExecutionError(format!(
                        "HTTP handler field '{}' expression '{}' could not be resolved",
                        name, raw
                    ))
                })?
        } else {
            Value::String(raw.trim().to_string())
        };
        fields.insert(name, value);
    }
    Ok(fields)
}

fn build_http_task_spec(
    service_task: &ServiceTask,
    execution: &Execution,
    command_context: &CommandContext,
) -> Result<HttpTaskSpec, FlowableError> {
    let method = required_http_extension_text(service_task, "requestMethod", "HTTP", execution)?
        .to_uppercase();
    if !command_context
        .config
        .http_service
        .supported_methods
        .iter()
        .any(|supported| supported.eq_ignore_ascii_case(&method))
    {
        return Err(FlowableError::ExecutionError(format!(
            "HTTP method '{method}' is not configured as supported"
        )));
    }
    let url = required_http_extension_text(service_task, "requestUrl", "HTTP", execution)?;
    let headers =
        parse_http_string_map_extension(service_task, "requestHeaders", "HTTP", execution)?;
    let request_body = resolve_http_extension_value(service_task, "requestBody", execution)
        .map(|value| match value {
            Value::String(raw) => parse_json_or_string(&raw),
            value => value,
        })
        .unwrap_or(Value::Null);
    let request = HttpRequest {
        method,
        url,
        headers: headers
            .into_iter()
            .map(|(key, value)| {
                let value = value
                    .as_str()
                    .map(str::to_string)
                    .unwrap_or_else(|| value.to_string());
                (key, value)
            })
            .collect(),
        body: (!request_body.is_null()).then_some(request_body),
        timeout_ms: parse_optional_u64_extension(service_task, "requestTimeout", "HTTP")?,
        connect_timeout_ms: parse_optional_u64_extension(service_task, "connectTimeout", "HTTP")?,
        follow_redirects: java_compatible_follow_redirects(service_task)?,
        basic_auth: parse_basic_auth_extension(service_task, "HTTP")?,
        body_encoding: parse_body_encoding_extension(service_task, "HTTP")?,
    };
    Ok(HttpTaskSpec {
        request,
        java: java_http_contract(service_task, execution)?,
        rust: RustHttpProjection {
            result_variable_name: service_task.result_variable_name.clone(),
            store_result_as_transient: service_task.store_result_variable_as_transient,
            use_local_scope: service_task.use_local_scope_for_result_variable,
        },
    })
}

fn java_compatible_follow_redirects(
    service_task: &ServiceTask,
) -> Result<Option<bool>, FlowableError> {
    if parse_optional_bool_extension(service_task, "disallowRedirects", "HTTP")? == Some(true) {
        return Ok(Some(false));
    }
    parse_optional_bool_extension(service_task, "followRedirects", "HTTP")
}

fn execute_http_service_task(
    service_task: &ServiceTask,
    mut spec: HttpTaskSpec,
    execution: &mut Execution,
    command_context: &mut CommandContext,
) -> Result<HttpServiceTaskExecution, FlowableError> {
    if !command_context.config.http_service.enabled {
        return Err(FlowableError::ExecutionError(
            "HTTP service tasks are disabled in the current engine configuration".to_string(),
        ));
    }

    if let Some(handler) = resolve_http_request_handler(service_task, execution, command_context)? {
        let mut context = HttpRequestHandlerContext {
            execution,
            request: &mut spec.request,
            fields: &handler.fields,
        };
        handler.handler.handle_request(&mut context)?;
    }
    project_java_request_variables(execution, &spec.java, &spec.request);
    let method = spec.request.method.clone();
    let url = spec.request.url.clone();
    let request_body = spec.request.body.clone().unwrap_or(Value::Null);
    let runtime_mode = command_context.http_runtime.mode();
    let mut exchange = match command_context
        .http_runtime
        .execute_with_status(&spec.request)
    {
        Ok(exchange) => exchange,
        Err(error) if spec.java.ignore_exception => {
            let outcome = HttpTaskOutcome::ignored_transport_error(&spec, &error.to_string());
            let result = outcome
                .apply_to(execution)
                .map_err(EngineFault::into_flowable_error)?;
            clear_boundaries_for_execution(&execution.id, command_context);
            return Ok(HttpServiceTaskExecution::Completed(result));
        }
        Err(error) => return Err(FlowableError::ExecutionError(error.to_string())),
    };
    if let Some(handler) = resolve_http_response_handler(service_task, execution, command_context)?
    {
        handler.invoke(execution, &mut exchange)?;
    }
    let outcome = HttpTaskOutcome::success(&spec, &exchange);
    let result = match outcome.apply_to(execution) {
        Ok(result) => result,
        Err(EngineFault::BpmnError { code, .. }) => {
            if propagate_bpmn_error(execution, &code, command_context)? {
                return Ok(HttpServiceTaskExecution::BpmnFaultHandled);
            }
            return Err(uncaught_bpmn_error(&code));
        }
        Err(fault) => return Err(fault.into_flowable_error()),
    };
    clear_boundaries_for_execution(&execution.id, command_context);

    let now = command_context
        .runtime_store
        .time_source()
        .now()
        .timestamp_millis();
    command_context.runtime_store.insert_http_task_record(
        HttpTaskRecord {
            id: format!("http-task-record:{}", Uuid::new_v4()),
            process_instance_id: execution.process_instance_id.clone().unwrap_or_default(),
            execution_id: execution.id.clone(),
            activity_id: execution.activity_id.clone().unwrap_or_default(),
            method,
            url,
            request_body: (!request_body.is_null()).then(|| request_body.to_string()),
            response_status_code: result["response"]["statusCode"]
                .as_u64()
                .map(|code| code as u16),
            response_body: Some(result["response"]["body"].to_string()),
            status: HttpTaskRecordStatus::Completed,
            created_at: now,
        },
        &mut command_context.session,
    );

    let activity_id = execution.activity_id.as_deref().unwrap_or("<unknown>");
    let runtime_label = match runtime_mode {
        HttpRuntimeMode::Deterministic => "deterministic runtime baseline",
        HttpRuntimeMode::Real => "real HTTP runtime",
        HttpRuntimeMode::Async => "async pooled HTTP runtime",
    };
    command_context.history_manager.record_audit_event(
        "http-service-task-executed",
        execution.process_instance_id.as_deref(),
        execution.process_definition_id.as_deref(),
        Some(&format!(
            "HTTP service task '{}' executed via {}",
            activity_id, runtime_label
        )),
        &mut command_context.session,
    );

    Ok(HttpServiceTaskExecution::Completed(result))
}

fn execute_shell_service_task(
    service_task: &ServiceTask,
    execution: &mut Execution,
    command_context: &mut CommandContext,
) -> Result<Value, FlowableError> {
    let command_str = required_extension_text(service_task, "command", "Shell")?;

    let mut cmd = std::process::Command::new(&command_str);

    let args = extension_elements(service_task, "arg");
    for arg in args {
        if let Some(text) = arg
            .element_text
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
        {
            cmd.arg(text);
        }
    }

    if let Some(dir) = optional_extension_text(service_task, "workingDirectory") {
        cmd.current_dir(&dir);
    }

    // Java ShellActivityBehavior.java:108-118 — redirectErrorStream + env.clear
    let redirect_error = optional_extension_text(service_task, "redirectError")
        .map(|v| v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    let clean_env = optional_extension_text(service_task, "cleanEnv")
        .map(|v| v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    if clean_env {
        cmd.env_clear();
    }

    let timeout_ms = parse_optional_u64_extension(service_task, "timeout", "Shell")?;

    let wait = optional_extension_text(service_task, "wait")
        .map(|v| v.eq_ignore_ascii_case("true"))
        .unwrap_or(true);

    // Java ShellActivityBehavior field names: outputVariable / errorCodeVariable.
    // (Previous Rust-only names errorVariable / exitCodeVariable removed for BPMN portability.)
    let output_variable = optional_extension_text(service_task, "outputVariable");
    let error_code_variable = optional_extension_text(service_task, "errorCodeVariable");

    let output = if wait {
        if let Some(timeout_ms) = timeout_ms {
            let (tx, rx) = std::sync::mpsc::channel();
            let activity_id_str = activity_id(service_task);
            let command_str_clone = command_str.clone();
            std::thread::spawn(move || {
                let res = cmd.output();
                let _ = tx.send(res);
            });
            match rx.recv_timeout(std::time::Duration::from_millis(timeout_ms)) {
                Ok(Ok(output)) => output,
                Ok(Err(e)) => {
                    return Err(FlowableError::ExecutionError(format!(
                        "Shell service task '{}' failed to execute command '{}': {}",
                        activity_id_str, command_str_clone, e
                    )));
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                    return Err(FlowableError::ExecutionError(format!(
                        "Shell service task '{}' command '{}' timed out after {} ms",
                        activity_id_str, command_str_clone, timeout_ms
                    )));
                }
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    return Err(FlowableError::ExecutionError(
                        "Shell command thread disconnected".into(),
                    ));
                }
            }
        } else {
            cmd.output().map_err(|e| {
                FlowableError::ExecutionError(format!(
                    "Shell service task '{}' failed to execute command '{}': {}",
                    activity_id(service_task),
                    command_str,
                    e
                ))
            })?
        }
    } else {
        let mut child = cmd.spawn().map_err(|e| {
            FlowableError::ExecutionError(format!(
                "Shell service task '{}' failed to spawn command '{}': {}",
                activity_id(service_task),
                command_str,
                e
            ))
        })?;
        let _ = child.wait().map_err(|e| {
            FlowableError::ExecutionError(format!(
                "Shell service task '{}' failed to wait for command '{}': {}",
                activity_id(service_task),
                command_str,
                e
            ))
        })?;
        std::process::Output {
            status: std::process::ExitStatus::default(),
            stdout: Vec::new(),
            stderr: Vec::new(),
        }
    };

    let mut stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    // ProcessBuilder.redirectErrorStream merges stderr into the process input stream
    // that becomes outputVariable; approximate by concatenating after capture.
    if redirect_error && !stderr.is_empty() {
        stdout.push_str(&stderr);
    }
    let exit_code = output.status.code().unwrap_or(-1);

    if let Some(var_name) = output_variable {
        execution.set_process_variable(var_name, Value::String(stdout.clone()));
    }
    if let Some(var_name) = error_code_variable {
        // Java stores Integer.toString(errorCode); keep JSON number for engine variables.
        execution.set_process_variable(var_name, Value::Number(exit_code.into()));
    }

    let result_stderr = if redirect_error {
        String::new()
    } else {
        stderr
    };
    let result = json!({
        "service": "shell",
        "command": command_str,
        "stdout": stdout,
        "stderr": result_stderr,
        "exitCode": exit_code,
        "redirectError": redirect_error,
        "cleanEnv": clean_env,
    });

    let activity_id = execution.activity_id.as_deref().unwrap_or("<unknown>");
    command_context.history_manager.record_audit_event(
        "shell-service-task-executed",
        execution.process_instance_id.as_deref(),
        execution.process_definition_id.as_deref(),
        Some(&format!(
            "Shell service task '{}' executed command '{}' with exit code {}",
            activity_id, command_str, exit_code
        )),
        &mut command_context.session,
    );

    Ok(result)
}

/// Execute `serviceTask` with `flowable:type="dmn"`.
///
/// Java reference: `DmnActivityBehavior.java:58-195` (field resolution, execute,
/// noHits throw, sameDeployment / parentDeployment). Writeback via shared
/// `dmn_result_writeback` with `result_variable_name: None`
/// (`DmnActivityBehavior.java:236-267` / `:197-234`).
pub(crate) fn execute_dmn_service_task(
    service_task: &ServiceTask,
    execution: &mut Execution,
    command_context: &mut CommandContext,
) -> Result<(), FlowableError> {
    let task_id = activity_id(service_task);
    let execution_label = format_execution_for_error(execution);

    // Java DmnActivityBehavior.java:60-65 — decisionTableReferenceKey required
    // (stringValue or expression non-empty).
    let field = find_dmn_field(service_task, "decisionTableReferenceKey");
    let active_decision_key = match field {
        Some(field) => {
            let expression = field
                .expression
                .as_deref()
                .map(str::trim)
                .filter(|v| !v.is_empty());
            let string_value = field
                .string_value
                .as_deref()
                .map(str::trim)
                .filter(|v| !v.is_empty());
            // Java :67-73 — expression preferred over stringValue when both present.
            match (expression, string_value) {
                (Some(expr), _) => expr.to_string(),
                (None, Some(sv)) => sv.to_string(),
                (None, None) => {
                    return Err(FlowableError::ExecutionError(format!(
                        "decisionTableReferenceKey is a required field extension for the dmn task {task_id} in {execution_label}"
                    )));
                }
            }
        }
        None => {
            return Err(FlowableError::ExecutionError(format!(
                "decisionTableReferenceKey is a required field extension for the dmn task {task_id} in {execution_label}"
            )));
        }
    };

    // Java :84-95 — always evaluate through expression manager; non-String /
    // empty → FlowableIllegalArgumentException messages (mapped to ExecutionError).
    let evaluation_execution =
        crate::engine::variable_service::evaluation_execution(command_context, execution);
    let decision_key_value =
        evaluate_dmn_expression_or_literal(&active_decision_key, &evaluation_execution);
    let final_decision_key = match decision_key_value {
        Some(Value::String(s)) if !s.is_empty() => s,
        Some(Value::String(_)) | None => {
            return Err(FlowableError::ExecutionError(format!(
                "decisionTableReferenceKey expression resolves to an empty value: {}",
                value_debug_label(decision_key_value.as_ref())
            )));
        }
        Some(other) => {
            return Err(FlowableError::ExecutionError(format!(
                "decisionTableReferenceKey expression does not resolve to a string: {}",
                value_debug_label(Some(&other))
            )));
        }
    };

    // Variables: Java :104 execution.getVariables()
    let mut inputs = Map::new();
    for (key, value) in evaluation_execution.process_variables() {
        inputs.insert(key, value);
    }

    let mut request = flowable_dmn_engine::DmnExecutionRequest::new(Value::Object(inputs));
    if let Some(tenant_id) = evaluation_execution.tenant_id.clone() {
        request.tenant_id = Some(tenant_id);
    }

    // Java :99-103 — audit correlation on the ExecuteDecisionBuilder:
    // instanceId = processInstanceId, executionId = execution id, activityId = task id.
    request.instance_id = execution.process_instance_id.clone();
    request.execution_id = Some(execution.id.clone());
    request.activity_id = Some(task_id.to_string());

    // Java applyFallbackToDefaultTenant :167-175 — stringValue only
    // (Boolean.parseBoolean); the expression attribute is deliberately ignored.
    request.fallback_to_default_tenant = dmn_field_string_value_is_true(
        service_task,
        "fallbackToDefaultTenant",
    );

    // Java applyParentDeployment :177-195
    // - sameDeployment field absent → always pass parentDeploymentId (back-compat)
    // - sameDeployment "true" → pass process definition deploymentId
    // - sameDeployment "false" → pass null
    request.parent_deployment_id =
        resolve_dmn_parent_deployment_id(service_task, execution, command_context);

    let dmn_engine = command_context.config.dmn_engine.clone().ok_or_else(|| {
        FlowableError::ExecutionError(
            "DMN engine is not configured for serviceTask type=dmn execution".to_string(),
        )
    })?;

    let execution_result = dmn_engine
        .decision_service()
        .execute_by_key(&final_decision_key, request)
        .map_err(|error| {
            FlowableError::ExecutionError(format!(
                "DMN decision with key {final_decision_key} execution failed in {execution_label}: {error}"
            ))
        })?;

    // Java :117-141 — throw on no hits when flag set.
    // noHits = decisionResult empty (Java getDecisionResult().isEmpty() :128).
    if execution_result.decision_result.is_empty() {
        maybe_throw_on_no_hits(
            service_task,
            &final_decision_key,
            &execution_label,
            &evaluation_execution,
        )?;
    }

    // Java :153 — multipleResults = audit.isMultipleResults() && alwaysUseArrays...
    let always_use_arrays = command_context
        .config
        .always_use_arrays_for_dmn_multi_hit_policies;
    let multiple_results = execution_result.multiple_results && always_use_arrays;

    // serviceTask type=dmn has no resultVariableName branch (Java writeback only);
    // businessRuleTask's Rust extension stays in shared helper with Some(...).
    crate::bpmn::behavior::dmn_result_writeback::write_dmn_result_to_execution(
        execution,
        &final_decision_key,
        &execution_result,
        None,
        multiple_results,
    );

    let details = format!(
        "DMN service task '{}' executed decision '{}'",
        task_id, final_decision_key
    );
    command_context.history_manager.record_audit_event(
        "dmn-service-task-executed",
        execution.process_instance_id.as_deref(),
        execution.process_definition_id.as_deref(),
        Some(&details),
        &mut command_context.session,
    );

    Ok(())
}

/// Java `DmnActivityBehavior.applyParentDeployment` :177-195.
fn resolve_dmn_parent_deployment_id(
    service_task: &ServiceTask,
    execution: &Execution,
    command_context: &mut CommandContext,
) -> Option<String> {
    let process_definition_id = execution.process_definition_id.as_deref()?;
    let definition_deployment_id = command_context
        .deployment_manager
        .get_process_definitions(&mut command_context.session)
        .get(process_definition_id)
        .and_then(|def| def.deployment_id.clone());

    match find_dmn_field(service_task, "sameDeployment") {
        Some(field) => {
            // Java :183 — only stringValue, Boolean.parseBoolean
            let raw = field
                .string_value
                .as_deref()
                .map(str::trim)
                .unwrap_or("");
            if raw.eq_ignore_ascii_case("true") {
                definition_deployment_id
            } else {
                // false or any non-true → do not pass parentDeploymentId
                None
            }
        }
        // Field absent → backwards compatibility: always apply parent deployment id
        None => definition_deployment_id,
    }
}

/// Java `DmnActivityBehavior.execute` :117-141 — throwErrorOnNoHits.
fn maybe_throw_on_no_hits(
    service_task: &ServiceTask,
    decision_key: &str,
    execution_label: &str,
    evaluation_execution: &Execution,
) -> Result<(), FlowableError> {
    let Some(field) = find_dmn_field(service_task, "decisionTaskThrowErrorOnNoHits") else {
        return Ok(());
    };

    // Java :120-126 — stringValue preferred, else expression text.
    let throw_error_string = field
        .string_value
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .or_else(|| {
            field
                .expression
                .as_deref()
                .map(str::trim)
                .filter(|v| !v.is_empty())
        });

    let Some(throw_error_string) = throw_error_string else {
        return Ok(());
    };

    let should_throw = if throw_error_string.eq_ignore_ascii_case("true") {
        true
    } else if throw_error_string.eq_ignore_ascii_case("false") {
        false
    } else {
        // Java :132-138 — evaluate as expression; Boolean true → throw
        match evaluate_dmn_expression_or_literal(throw_error_string, evaluation_execution) {
            Some(Value::Bool(true)) => true,
            _ => false,
        }
    };

    if should_throw {
        return Err(FlowableError::ExecutionError(format!(
            "DMN decision with key {decision_key} did not hit any rules for the provided input. In {execution_label}"
        )));
    }
    Ok(())
}

/// Locate a field extension by name on the service task activity.
/// Java `DelegateHelper.getFlowElementField` (:162-173) — name match on field extensions.
/// Reads a DMN field extension as a boolean the way Java does for
/// `sameDeployment` (:183) and `fallbackToDefaultTenant` (:169-172): only
/// `stringValue` is consulted, parsed with `Boolean.parseBoolean` semantics
/// (case-insensitive `"true"`, everything else false). The `expression`
/// attribute is deliberately ignored — Java does not evaluate it for these
/// two fields.
fn dmn_field_string_value_is_true(service_task: &ServiceTask, name: &str) -> bool {
    find_dmn_field(service_task, name)
        .and_then(|field| field.string_value.as_deref())
        .map(str::trim)
        .is_some_and(|raw| raw.eq_ignore_ascii_case("true"))
}

fn find_dmn_field<'a>(service_task: &'a ServiceTask, name: &str) -> Option<&'a FieldExtension> {
    service_task
        .task
        .activity
        .field_extensions
        .iter()
        .find(|field| field.field_name.as_deref() == Some(name))
}

/// Evaluate EL when text is `${...}` / `#{...}`; otherwise treat as literal string.
/// Mirrors Java ExpressionManager treating plain text as a string literal expression.
fn evaluate_dmn_expression_or_literal(text: &str, execution: &Execution) -> Option<Value> {
    let trimmed = text.trim();
    if (trimmed.starts_with("${") && trimmed.ends_with('}'))
        || (trimmed.starts_with("#{") && trimmed.ends_with('}'))
    {
        SimpleExpression::new(trimmed.to_string()).get_value(execution)
    } else {
        Some(Value::String(trimmed.to_string()))
    }
}

fn format_execution_for_error(execution: &Execution) -> String {
    format!(
        "Execution[id={}, activityId={}, processInstanceId={}]",
        execution.id,
        execution.activity_id.as_deref().unwrap_or(""),
        execution.process_instance_id.as_deref().unwrap_or("")
    )
}

fn value_debug_label(value: Option<&Value>) -> String {
    match value {
        None => "null".to_string(),
        Some(Value::Null) => "null".to_string(),
        Some(Value::String(s)) => s.clone(),
        Some(other) => other.to_string(),
    }
}

pub(crate) fn execute_mail_service_task(
    service_task: &ServiceTask,
    execution: &mut Execution,
    command_context: &mut CommandContext,
) -> Result<Value, FlowableError> {
    if !command_context.config.mail_service.enabled {
        return Err(FlowableError::ExecutionError(
            "Mail service tasks are disabled in the current engine configuration".to_string(),
        ));
    }

    let ignore_exception = optional_mail_extension_text(service_task, "ignoreException", execution)
        .map(|v| v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    let exception_variable_name =
        optional_mail_extension_text(service_task, "exceptionVariableName", execution);

    match build_and_send_mail(service_task, execution, command_context) {
        Ok(result) => Ok(result),
        Err(error) => handle_mail_exception(
            error,
            ignore_exception,
            exception_variable_name.as_deref(),
            execution,
        ),
    }
}

/// Build the mail message (Java `BaseMailActivityDelegate.createMessage`) and send it.
fn build_and_send_mail(
    service_task: &ServiceTask,
    execution: &mut Execution,
    command_context: &mut CommandContext,
) -> Result<Value, FlowableError> {
    // Java BaseMailActivityDelegate: all fields are Expressions; evaluate ${...} against execution.
    // `to` may be empty when cc/bcc supply recipients (Java parseRecipients).
    let to = optional_mail_extension_text(service_task, "to", execution).unwrap_or_default();
    let to_recipients = split_recipients(&to);
    let cc = optional_mail_extension_text(service_task, "cc", execution).unwrap_or_default();
    let cc_recipients = split_recipients(&cc);
    let bcc = optional_mail_extension_text(service_task, "bcc", execution).unwrap_or_default();
    let bcc_recipients = split_recipients(&bcc);
    let from = optional_mail_extension_text(service_task, "from", execution)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| command_context.config.mail_service.default_from.clone());
    let subject = required_mail_extension_text(service_task, "subject", execution)?;

    // Java BaseMailActivityDelegate.createMessage:100-105 —
    // textVar / htmlVar take priority over text / html when the field is present.
    // textVar holds a *variable name* (expression text, not evaluated as EL value);
    // that variable's content is then evaluated as the body template.
    let text = resolve_mail_body_field(service_task, execution, "textVar", "text")?;
    let html = resolve_mail_body_field(service_task, execution, "htmlVar", "html")?;

    // Java createMessage:112-114 — at least one of html/text after resolution.
    if text.as_deref().map(str::trim).is_none_or(|t| t.is_empty())
        && html.as_deref().map(str::trim).is_none_or(|h| h.is_empty())
    {
        return Err(FlowableError::ExecutionError(
            "'html' or 'text' is required to be defined when using the mail activity".to_string(),
        ));
    }
    let text = text.unwrap_or_default();

    let charset = optional_mail_extension_text(service_task, "charset", execution);

    // Java BaseMailActivityDelegate.addHeader:134-147 — newline-separated "Name: value".
    let headers = parse_mail_headers(
        optional_mail_extension_text(service_task, "headers", execution).as_deref(),
    )?;

    // Java BaseMailActivityDelegate.addAttachments:149-167 — expression → collection/value.
    let attachments = resolve_mail_attachments(service_task, execution)?;

    // Java: at least one of to/cc/bcc must resolve to a recipient.
    if to_recipients.is_empty() && cc_recipients.is_empty() && bcc_recipients.is_empty() {
        return Err(FlowableError::ExecutionError(format!(
            "Mail service task '{}' has no recipient (to/cc/bcc)",
            activity_id(service_task)
        )));
    }

    let runtime = command_context
        .config
        .mail_service
        .build_runtime()
        .map_err(FlowableError::ExecutionError)?;

    // Owned mail runtime accepts a single to/recipients list; fold cc/bcc into recipients
    // for transport while preserving separate fields on the result payload.
    let mut all_recipients = to_recipients.clone();
    all_recipients.extend(cc_recipients.iter().cloned());
    all_recipients.extend(bcc_recipients.iter().cloned());
    let transport_to = if !to_recipients.is_empty() {
        to_recipients.join(",")
    } else {
        all_recipients.join(",")
    };

    let send_record = runtime
        .send(MailMessage {
            to: transport_to,
            recipients: if !to_recipients.is_empty() {
                to_recipients.clone()
            } else {
                all_recipients.clone()
            },
            from: Some(from.clone()),
            subject: subject.clone(),
            text: text.clone(),
            html: html.clone(),
            headers: headers.clone(),
            attachments: attachments.clone(),
        })
        .map_err(|error| FlowableError::ExecutionError(error.to_string()))?;

    let headers_json: BTreeMap<String, Value> = send_record
        .message
        .headers
        .iter()
        .map(|(k, v)| (k.clone(), Value::String(v.clone())))
        .collect();
    let attachments_json: Vec<Value> = send_record
        .message
        .attachments
        .iter()
        .map(|a| {
            json!({
                "name": a.name,
                "contentType": a.content_type,
            })
        })
        .collect();

    let result = json!({
        "service": "mail",
        "status": send_record.status,
        "transport": send_record.transport,
        "message": {
            "to": send_record.message.to,
            "cc": cc,
            "bcc": bcc,
            "recipients": send_record.message.recipients,
            "ccRecipients": cc_recipients,
            "bccRecipients": bcc_recipients,
            "from": send_record.message.from,
            "subject": send_record.message.subject,
            "text": send_record.message.text,
            "html": send_record.message.html,
            "charset": charset,
            "headers": headers_json,
            "attachments": attachments_json,
        }
    });

    let now = command_context
        .runtime_store
        .time_source()
        .now()
        .timestamp_millis();
    command_context.runtime_store.insert_mail_outbox_record(
        MailOutboxRecord {
            id: format!("mail-outbox-record:{}", Uuid::new_v4()),
            process_instance_id: execution.process_instance_id.clone().unwrap_or_default(),
            execution_id: execution.id.clone(),
            activity_id: execution.activity_id.clone().unwrap_or_default(),
            recipient: send_record.message.to.clone(),
            recipients: send_record.message.recipients.clone(),
            subject,
            body: text,
            html_body: html,
            status: MailOutboxStatus::Sent,
            created_at: now,
        },
        &mut command_context.session,
    );

    let activity_id = execution.activity_id.as_deref().unwrap_or("<unknown>");
    command_context.history_manager.record_audit_event(
        "mail-service-task-executed",
        execution.process_instance_id.as_deref(),
        execution.process_definition_id.as_deref(),
        Some(&format!(
            "Mail service task '{}' executed via deterministic outbox baseline",
            activity_id
        )),
        &mut command_context.session,
    );

    Ok(result)
}

/// Resolve body via `*Var` (variable-name field) or literal `text`/`html` field.
///
/// Java `BaseMailActivityDelegate.createMessage:100-105` + `getExpression:236-239`:
/// when `textVar`/`htmlVar` is set, its *expression text* is the process-variable
/// name; the variable value is then treated as a body template expression.
fn resolve_mail_body_field(
    service_task: &ServiceTask,
    execution: &Execution,
    var_field: &str,
    literal_field: &str,
) -> Result<Option<String>, FlowableError> {
    if let Some(var_name) = raw_mail_extension_text(service_task, var_field) {
        // Java getExpression uses Expression.getExpressionText() as variable name
        // (not getValue) — so bare "html" / "bodyTemplate" are variable names.
        let var_name = var_name.trim();
        if var_name.is_empty() {
            return Ok(None);
        }
        let Some(template_value) = execution.process_variable(var_name) else {
            // Java createExpression(null) NPEs on null.trim() — treat as hard error.
            return Err(FlowableError::ExecutionError(format!(
                "Mail service task '{}' {var_field} variable '{var_name}' is not set",
                activity_id(service_task)
            )));
        };
        let template = match template_value {
            Value::String(s) => s,
            Value::Null => {
                return Err(FlowableError::ExecutionError(format!(
                    "Mail service task '{}' {var_field} variable '{var_name}' is null",
                    activity_id(service_task)
                )));
            }
            other => value_to_plain_string(&other),
        };
        return Ok(Some(evaluate_mail_body_template(&template, execution)));
    }
    // Literal text/html fields are also JUEL expressions in Java
    // (BaseMailActivityDelegate.java:100-105 getStringFromField(text/html)).
    // Use the same composite evaluator so mixed templates expand.
    if let Some(raw) = raw_mail_extension_text(service_task, literal_field) {
        let expanded = evaluate_mail_body_template(&raw, execution);
        let trimmed = expanded.trim();
        if trimmed.is_empty() {
            return Ok(None);
        }
        return Ok(Some(expanded));
    }
    Ok(None)
}

/// Evaluate a mail body template against the execution.
///
/// P134: JUEL composite semantics (`ExpressionManager.createExpression`) —
/// mixed text like `"Hello ${gender}!"` is expanded segment-by-segment.
/// Applies to textVar/htmlVar templates and literal text/html fields
/// (Java `BaseMailActivityDelegate.java:94-105`). Pure `${…}` and pure
/// literals behave as before (empty on failed pure expression).
fn evaluate_mail_body_template(template: &str, execution: &Execution) -> String {
    use flowable_engine_common::el::evaluate_composite_expression;
    evaluate_composite_expression(template, execution)
}

fn value_to_plain_string(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// Java `BaseMailActivityDelegate.addHeader:134-147` — split on newlines, each
/// entry must be `Name: value` (exactly one colon-separated pair after split on first `:`).
fn parse_mail_headers(
    headers_str: Option<&str>,
) -> Result<BTreeMap<String, String>, FlowableError> {
    let Some(headers_str) = headers_str.filter(|s| !s.trim().is_empty()) else {
        return Ok(BTreeMap::new());
    };
    let mut headers = BTreeMap::new();
    for header_entry in headers_str.split(['\n', '\r']) {
        let entry = header_entry.trim();
        if entry.is_empty() {
            continue;
        }
        // Java: headerEntry.split(":") with length != 2 throws. Use splitn(2) so
        // values containing colons (e.g. URLs) remain valid as a single value half.
        // Strict length==2 after full split is harsher on "X-Url: http://x"; Java's
        // split without limit also yields >2 for those — document as owned subset:
        // first colon separates name/value (common MIME practice).
        let Some((name, value)) = entry.split_once(':') else {
            return Err(FlowableError::ExecutionError(
                "When using email headers name and value must be defined colon separated. (e.g. X-Attribute: value"
                    .to_string(),
            ));
        };
        let name = name.trim();
        let value = value.trim();
        if name.is_empty() {
            return Err(FlowableError::ExecutionError(
                "When using email headers name and value must be defined colon separated. (e.g. X-Attribute: value"
                    .to_string(),
            ));
        }
        headers.insert(name.to_string(), value.to_string());
    }
    Ok(headers)
}

/// Java `BaseMailActivityDelegate.addAttachments:149-211`.
/// Owned subset: strings / string arrays / collections of strings recorded as
/// attachment name refs (deterministic outbox; no real file IO).
fn resolve_mail_attachments(
    service_task: &ServiceTask,
    execution: &Execution,
) -> Result<Vec<MailAttachment>, FlowableError> {
    let Some(value) = resolve_http_extension_value(service_task, "attachments", execution) else {
        return Ok(Vec::new());
    };
    if value.is_null() {
        return Ok(Vec::new());
    }
    let mut attachments = Vec::new();
    collect_mail_attachment_value(&value, &mut attachments, service_task)?;
    Ok(attachments)
}

fn collect_mail_attachment_value(
    value: &Value,
    out: &mut Vec<MailAttachment>,
    service_task: &ServiceTask,
) -> Result<(), FlowableError> {
    match value {
        Value::Null => Ok(()),
        Value::String(name) => {
            let name = name.trim();
            if !name.is_empty() {
                // Java: String filename → File; fileExists check skips missing files.
                // Deterministic outbox records the path/name for passthrough; skip only
                // when it looks like a filesystem path and the file is absent.
                if looks_like_filesystem_path(name) && !std::path::Path::new(name).is_file() {
                    return Ok(());
                }
                out.push(MailAttachment {
                    name: name.to_string(),
                    content_type: None,
                });
            }
            Ok(())
        }
        Value::Array(items) => {
            for item in items {
                collect_mail_attachment_value(item, out, service_task)?;
            }
            Ok(())
        }
        Value::Object(obj) => {
            // JSON object form: { "name": "...", "contentType": "..." } for tests/API.
            let name = obj
                .get("name")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| {
                    FlowableError::ExecutionError(format!(
                        "Invalid attachment type: object without name for mail task '{}'",
                        activity_id(service_task)
                    ))
                })?;
            let content_type = obj
                .get("contentType")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            out.push(MailAttachment {
                name: name.to_string(),
                content_type,
            });
            Ok(())
        }
        other => Err(FlowableError::ExecutionError(format!(
            "Invalid attachment type: {} for mail task '{}'",
            value_type_name(other),
            activity_id(service_task)
        ))),
    }
}

fn looks_like_filesystem_path(name: &str) -> bool {
    name.contains('/')
        || name.contains('\\')
        || (name.len() >= 3 && name.as_bytes()[1] == b':' && name.as_bytes()[0].is_ascii_alphabetic())
}

fn value_type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

/// Raw field/extension text without EL evaluation — used for textVar/htmlVar
/// variable *names* (Java `Expression.getExpressionText()`).
fn raw_mail_extension_text(service_task: &ServiceTask, name: &str) -> Option<String> {
    if let Some(field) = service_task
        .task
        .activity
        .field_extensions
        .iter()
        .find(|field| field.field_name.as_deref() == Some(name))
    {
        return field
            .string_value
            .clone()
            .or_else(|| field.expression.clone())
            .map(|text| text.trim().to_string())
            .filter(|text| !text.is_empty());
    }
    extension_elements(service_task, name)
        .first()
        .and_then(|element| element.element_text.clone())
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
}

/// Java BaseMailActivityDelegate.handleException: swallow when ignoreException=true
/// and optionally store the message under exceptionVariableName.
fn handle_mail_exception(
    error: FlowableError,
    ignore_exception: bool,
    exception_variable_name: Option<&str>,
    execution: &mut Execution,
) -> Result<Value, FlowableError> {
    if !ignore_exception {
        return Err(error);
    }
    if let Some(var_name) = exception_variable_name.filter(|name| !name.is_empty()) {
        execution.set_process_variable(var_name.to_string(), Value::String(error.to_string()));
    }
    Ok(json!({
        "service": "mail",
        "status": "IGNORED_ERROR",
        "error": error.to_string(),
    }))
}

fn required_mail_extension_text(
    service_task: &ServiceTask,
    name: &str,
    execution: &Execution,
) -> Result<String, FlowableError> {
    optional_mail_extension_text(service_task, name, execution).ok_or_else(|| {
        FlowableError::ExecutionError(format!(
            "Mail service task is missing required extension '{}'",
            name
        ))
    })
}

fn optional_mail_extension_text(
    service_task: &ServiceTask,
    name: &str,
    execution: &Execution,
) -> Option<String> {
    resolve_http_extension_value(service_task, name, execution).and_then(|value| match value {
        Value::String(value) => {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        }
        Value::Number(value) => Some(value.to_string()),
        Value::Bool(value) => Some(value.to_string()),
        _ => None,
    })
}

fn apply_service_task_in_parameters(
    service_task: &ServiceTask,
    execution: &mut Execution,
) -> Result<(), FlowableError> {
    for parameter in &service_task.in_parameters {
        let target = parameter_target(service_task, parameter, "inParameter")?;
        let value = parameter_value(parameter, execution, None);
        execution.set_local_variable(target.to_string(), value);
    }
    Ok(())
}

fn apply_service_task_result_and_out_parameters(
    service_task: &ServiceTask,
    execution: &mut Execution,
    result: Option<Value>,
) -> Result<(), FlowableError> {
    if let (Some(result_variable_name), Some(result)) =
        (service_task.result_variable_name.as_ref(), result.as_ref())
    {
        if service_task.store_result_variable_as_transient {
            execution.set_transient_variable(result_variable_name.clone(), result.clone());
        } else if service_task.use_local_scope_for_result_variable {
            execution.set_local_variable(result_variable_name.clone(), result.clone());
        } else {
            execution.set_process_variable(result_variable_name.clone(), result.clone());
        }
    }

    for parameter in &service_task.out_parameters {
        let target = parameter_target(service_task, parameter, "outParameter")?;
        let value = parameter_value(parameter, execution, result.as_ref());
        execution.set_process_variable(target.to_string(), value);
    }

    Ok(())
}

fn parameter_target<'a>(
    service_task: &ServiceTask,
    parameter: &'a IOParameter,
    parameter_kind: &str,
) -> Result<&'a str, FlowableError> {
    parameter
        .target
        .as_deref()
        .or(parameter.target_expression.as_deref())
        .map(str::trim)
        .filter(|target| !target.is_empty())
        .ok_or_else(|| {
            FlowableError::ExecutionError(format!(
                "Service task '{}' has an {} without target",
                activity_id(service_task),
                parameter_kind
            ))
        })
}

fn parameter_value(
    parameter: &IOParameter,
    execution: &Execution,
    result: Option<&Value>,
) -> Value {
    if let Some(source_expression) = parameter.source_expression.as_deref() {
        return expression_value_from_context(source_expression, execution, result);
    }

    if let Some(source) = parameter.source.as_deref() {
        return source_value_from_context(source, execution, result);
    }

    Value::Null
}

fn expression_value_from_context(
    source_expression: &str,
    execution: &Execution,
    result: Option<&Value>,
) -> Value {
    if let Some(value) = SimpleExpression::new(source_expression.to_string()).get_value(execution) {
        return value;
    }

    if let Some(property_name) = expression_property_name(source_expression) {
        return source_value_from_context(property_name, execution, result);
    }

    Value::Null
}

fn source_value_from_context(source: &str, execution: &Execution, result: Option<&Value>) -> Value {
    let source = source.trim();
    if source.is_empty() {
        return Value::Null;
    }

    if let Some(value) = execution.process_variable(source) {
        return value;
    }

    if let Some(result) = result
        && let Some(value) = value_at_path(result, source)
    {
        return value;
    }

    if let Some(value) = parse_literal_value(source) {
        return value;
    }

    Value::String(source.to_string())
}

fn value_at_path(value: &Value, path: &str) -> Option<Value> {
    let mut current = value;
    for segment in path.split('.') {
        let segment = segment.trim();
        if segment.is_empty() {
            return None;
        }
        current = match current {
            Value::Object(object) => object.get(segment)?,
            Value::Array(array) => {
                let index = segment.parse::<usize>().ok()?;
                array.get(index)?
            }
            _ => return None,
        };
    }
    Some(current.clone())
}

fn parse_literal_value(source: &str) -> Option<Value> {
    if source.eq_ignore_ascii_case("true") {
        return Some(Value::Bool(true));
    }
    if source.eq_ignore_ascii_case("false") {
        return Some(Value::Bool(false));
    }
    if let Some(stripped) = source
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
    {
        return Some(Value::String(stripped.to_string()));
    }
    if let Some(stripped) = source
        .strip_prefix('\'')
        .and_then(|value| value.strip_suffix('\''))
    {
        return Some(Value::String(stripped.to_string()));
    }
    if let Ok(integer) = source.parse::<i64>() {
        return Some(Value::Number(integer.into()));
    }
    if let Ok(float) = source.parse::<f64>() {
        return serde_json::Number::from_f64(float).map(Value::Number);
    }
    None
}

fn activity_id(service_task: &ServiceTask) -> String {
    service_task
        .task
        .activity
        .flow_node
        .flow_element
        .base_element
        .id
        .clone()
        .unwrap_or_default()
}

fn required_extension_text(
    service_task: &ServiceTask,
    name: &str,
    label: &str,
) -> Result<String, FlowableError> {
    optional_extension_text(service_task, name).ok_or_else(|| {
        FlowableError::ExecutionError(format!(
            "{} service task is missing required extension '{}'",
            label, name
        ))
    })
}

fn required_http_extension_text(
    service_task: &ServiceTask,
    name: &str,
    label: &str,
    execution: &Execution,
) -> Result<String, FlowableError> {
    resolve_http_extension_value(service_task, name, execution)
        .and_then(|value| match value {
            Value::String(value) if !value.trim().is_empty() => Some(value.trim().to_string()),
            Value::Number(value) => Some(value.to_string()),
            Value::Bool(value) => Some(value.to_string()),
            _ => None,
        })
        .ok_or_else(|| {
            FlowableError::ExecutionError(format!(
                "{} service task is missing required extension '{}'",
                label, name
            ))
        })
}

fn resolve_http_extension_value(
    service_task: &ServiceTask,
    name: &str,
    execution: &Execution,
) -> Option<Value> {
    if let Some(field) = service_task
        .task
        .activity
        .field_extensions
        .iter()
        .find(|field| field.field_name.as_deref() == Some(name))
    {
        if let Some(expression) = field.expression.as_deref() {
            return SimpleExpression::new(expression.trim().to_string()).get_value(execution);
        }
        if let Some(raw) = field.string_value.as_deref().map(str::trim) {
            if raw.starts_with("${") && raw.ends_with('}') {
                return SimpleExpression::new(raw.to_string()).get_value(execution);
            }
            return Some(Value::String(raw.to_string()));
        }
        return None;
    }

    if let Some(raw) = extension_elements(service_task, name)
        .first()
        .and_then(|element| element.element_text.clone())
    {
        let raw = raw.trim();
        if raw.starts_with("${") && raw.ends_with('}') {
            return SimpleExpression::new(raw.to_string()).get_value(execution);
        }
        return Some(Value::String(raw.to_string()));
    }
    None
}

fn parse_http_string_map_extension(
    service_task: &ServiceTask,
    name: &str,
    label: &str,
    execution: &Execution,
) -> Result<Map<String, Value>, FlowableError> {
    let Some(value) = resolve_http_extension_value(service_task, name, execution) else {
        return Ok(Map::new());
    };
    let parsed = match value {
        Value::String(raw) => serde_json::from_str::<Value>(&raw).map_err(|error| {
            FlowableError::ExecutionError(format!(
                "{} service task extension '{}' must be valid JSON: {}",
                label, name, error
            ))
        })?,
        value => value,
    };
    let Value::Object(object) = parsed else {
        return Err(FlowableError::ExecutionError(format!(
            "{} service task extension '{}' must be a JSON object",
            label, name
        )));
    };
    Ok(object)
}

fn optional_extension_text(service_task: &ServiceTask, name: &str) -> Option<String> {
    if let Some(value) = extension_elements(service_task, name)
        .first()
        .and_then(|element| element.element_text.clone())
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
    {
        return Some(value);
    }
    service_task
        .task
        .activity
        .field_extensions
        .iter()
        .find(|field| field.field_name.as_deref() == Some(name))
        .and_then(|field| {
            field
                .string_value
                .clone()
                .or_else(|| field.expression.clone())
        })
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
}

fn parse_optional_u64_extension(
    service_task: &ServiceTask,
    name: &str,
    label: &str,
) -> Result<Option<u64>, FlowableError> {
    optional_extension_text(service_task, name)
        .map(|raw| {
            raw.parse::<u64>().map_err(|error| {
                FlowableError::ExecutionError(format!(
                    "{} service task extension '{}' must be an integer number of milliseconds: {}",
                    label, name, error
                ))
            })
        })
        .transpose()
}

fn parse_optional_bool_extension(
    service_task: &ServiceTask,
    name: &str,
    label: &str,
) -> Result<Option<bool>, FlowableError> {
    optional_extension_text(service_task, name)
        .map(|raw| match raw.as_str() {
            "true" => Ok(true),
            "false" => Ok(false),
            _ => Err(FlowableError::ExecutionError(format!(
                "{} service task extension '{}' must be true or false",
                label, name
            ))),
        })
        .transpose()
}

fn parse_basic_auth_extension(
    service_task: &ServiceTask,
    label: &str,
) -> Result<Option<BasicAuth>, FlowableError> {
    let username = optional_extension_text(service_task, "basicAuthenticationUsername");
    let password = optional_extension_text(service_task, "basicAuthenticationPassword");
    match (username, password) {
        (Some(username), Some(password)) => Ok(Some(BasicAuth { username, password })),
        (None, None) => Ok(None),
        _ => Err(FlowableError::ExecutionError(format!(
            "{} service task extensions 'basicAuthenticationUsername' and 'basicAuthenticationPassword' must be configured together",
            label
        ))),
    }
}

fn parse_body_encoding_extension(
    service_task: &ServiceTask,
    label: &str,
) -> Result<Option<String>, FlowableError> {
    let Some(body_encoding) = optional_extension_text(service_task, "bodyEncoding")
        .or_else(|| optional_extension_text(service_task, "requestBodyEncoding"))
    else {
        return Ok(None);
    };
    let normalized = body_encoding.to_ascii_lowercase();
    if matches!(normalized.as_str(), "json" | "form" | "text") {
        Ok(Some(normalized))
    } else {
        Err(FlowableError::ExecutionError(format!(
            "{} service task extension 'bodyEncoding' must be one of json, form, or text; got '{}'",
            label, body_encoding
        )))
    }
}

fn split_recipients(value: &str) -> Vec<String> {
    value
        .split([',', ';'])
        .map(str::trim)
        .filter(|recipient| !recipient.is_empty())
        .map(str::to_string)
        .collect()
}

fn extension_elements<'a>(service_task: &'a ServiceTask, name: &str) -> &'a [ExtensionElement] {
    service_task
        .task
        .activity
        .flow_node
        .flow_element
        .base_element
        .extension_elements
        .get(name)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

fn parse_json_or_string(raw: &str) -> Value {
    serde_json::from_str(raw).unwrap_or_else(|_| Value::String(raw.to_string()))
}

fn should_defer_outgoing_to_multi_instance_parent(
    execution: &Execution,
    command_context: &mut CommandContext,
) -> bool {
    let Some(parent_id) = execution.parent_id.as_deref() else {
        return false;
    };
    let Some(activity_id) = execution.activity_id.as_deref() else {
        return false;
    };
    let Some(parent) = command_context
        .runtime_store
        .find_execution(parent_id, &mut command_context.session)
    else {
        return false;
    };
    if parent.activity_id.as_deref() != Some(activity_id) {
        return false;
    }
    let Some(process_definition_id) = parent.process_definition_id.as_deref() else {
        return false;
    };

    command_context
        .deployment_manager
        .get_bpmn_model(process_definition_id)
        .as_ref()
        .and_then(|model| model.main_process.as_ref())
        .and_then(|process| {
            crate::agenda::continue_process_operation::find_flow_element(process, activity_id)
        })
        .is_some_and(|flow_element| {
            matches!(
                flow_element,
                FlowElementEnum::ServiceTask(service_task)
                    if service_task.task.activity.loop_characteristics.is_some()
            )
        })
}
