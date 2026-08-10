use crate::bpmn::listener::listener_registry::{
    EXECUTION_LISTENER_REGISTRY_CACHE_KEY, ExecutionListenerContext, LocalExecutionListenerRegistry,
};
use crate::el::expression::{Expression, SimpleExpression};
use crate::error::FlowableError;
use crate::interceptor::command_context::CommandContext;
use crate::runtime::execution::Execution;
use flowable_bpmn_model::model::{FieldExtension, FlowElementEnum, FlowableListener};
use serde_json::{Map, Value};

/// Notify execution listeners declared on a flow element for the given event.
pub fn notify_execution_listeners(
    execution: &mut Execution,
    command_context: &CommandContext,
    flow_element: &FlowElementEnum,
    event: &str,
    evaluation_execution: &Execution,
) -> Result<(), FlowableError> {
    let listeners = flow_element_execution_listeners(flow_element);
    execute_execution_listeners(
        execution,
        command_context,
        listeners,
        event,
        evaluation_execution,
    )
}

/// Execute a list of parsed execution listeners for `event` (`start` / `end` / …).
pub fn execute_execution_listeners(
    execution: &mut Execution,
    command_context: &CommandContext,
    listeners: &[FlowableListener],
    event: &str,
    evaluation_execution: &Execution,
) -> Result<(), FlowableError> {
    if listeners.is_empty() {
        return Ok(());
    }

    let activity_id = execution.activity_id.clone();
    for listener in listeners {
        if !listener_matches_event(listener, event) {
            continue;
        }
        reject_transaction_hook(listener, "executionListener")?;
        invoke_execution_listener(
            execution,
            command_context,
            listener,
            event,
            activity_id.as_deref(),
            evaluation_execution,
        )?;
    }
    Ok(())
}

fn invoke_execution_listener(
    execution: &mut Execution,
    command_context: &CommandContext,
    listener: &FlowableListener,
    event: &str,
    activity_id: Option<&str>,
    evaluation_execution: &Execution,
) -> Result<(), FlowableError> {
    let impl_type = listener
        .implementation_type
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .unwrap_or("");
    let implementation = listener
        .implementation
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .ok_or_else(|| {
            FlowableError::ExecutionError(format!(
                "executionListener on event '{}' has no implementation",
                event
            ))
        })?;

    match impl_type {
        "expression" => {
            // Evaluate for side effects when the expression language supports them.
            // Currently SimpleExpression is read-only; evaluation still validates syntax
            // and variable resolution.
            let _ =
                SimpleExpression::new(implementation.to_string()).get_value(evaluation_execution);
            Ok(())
        }
        "delegateExpression" | "class" | "" => {
            let delegate_name =
                resolve_listener_name(impl_type, implementation, evaluation_execution)?;
            let fields =
                resolve_field_extensions(&listener.field_extensions, evaluation_execution)?;
            let registered = command_context
                .session_caches
                .get(EXECUTION_LISTENER_REGISTRY_CACHE_KEY)
                .and_then(|reg| reg.downcast_ref::<LocalExecutionListenerRegistry>())
                .and_then(|reg| reg.get(&delegate_name));

            let listener_impl = registered.ok_or_else(|| {
                FlowableError::ExecutionError(format!(
                    "No local execution listener '{}' is registered for event '{}'",
                    delegate_name, event
                ))
            })?;

            let mut ctx = ExecutionListenerContext {
                event,
                activity_id,
                execution,
                fields: &fields,
            };
            listener_impl.notify(&mut ctx)
        }
        other => Err(FlowableError::BadRequest(format!(
            "Unsupported executionListener implementationType '{}' for event '{}'",
            other, event
        ))),
    }
}

fn resolve_listener_name(
    impl_type: &str,
    implementation: &str,
    execution: &Execution,
) -> Result<String, FlowableError> {
    if impl_type == "class" || impl_type.is_empty() {
        // `class` is treated as a registry key (Rust mapping of FQCN), not JVM load.
        return Ok(implementation.to_string());
    }

    // delegateExpression: resolve `${name}` or bare name to a string key.
    if implementation.starts_with("${") && implementation.ends_with('}') {
        let value = SimpleExpression::new(implementation.to_string())
            .get_value(execution)
            .ok_or_else(|| {
                FlowableError::ExecutionError(format!(
                    "executionListener could not resolve delegateExpression '{}'",
                    implementation
                ))
            })?;
        value.as_str().map(|s| s.to_string()).ok_or_else(|| {
            FlowableError::ExecutionError(format!(
                "executionListener delegateExpression '{}' resolved to a non-string value",
                implementation
            ))
        })
    } else {
        Ok(implementation.to_string())
    }
}

fn resolve_field_extensions(
    fields: &[FieldExtension],
    execution: &Execution,
) -> Result<Map<String, Value>, FlowableError> {
    let mut out = Map::new();
    for field in fields {
        let name = field
            .field_name
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .ok_or_else(|| {
                FlowableError::ExecutionError(
                    "executionListener field extension is missing a name".to_string(),
                )
            })?;
        let string_value = field
            .string_value
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty());
        let expression = field
            .expression
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty());
        let value = match (string_value, expression) {
            (Some(v), None) => Value::String(v.to_string()),
            (None, Some(expr)) => SimpleExpression::new(expr.to_string())
                .get_value(execution)
                .ok_or_else(|| {
                    FlowableError::ExecutionError(format!(
                        "executionListener field '{}' expression '{}' could not be resolved",
                        name, expr
                    ))
                })?,
            (Some(_), Some(_)) => {
                return Err(FlowableError::ExecutionError(format!(
                    "executionListener field '{}' cannot have both stringValue and expression",
                    name
                )));
            }
            (None, None) => Value::Null,
        };
        out.insert(name.to_string(), value);
    }
    Ok(out)
}

pub(crate) fn listener_matches_event(listener: &FlowableListener, event: &str) -> bool {
    match listener
        .event
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        None => true,
        // Java `ListenerNotificationHelper.java:122`: a listener configured for
        // `allEvents` matches every fired event (`event.equals(eventType) ||
        // event.equals(TaskListener.EVENTNAME_ALL_EVENTS)`).
        Some(configured) if configured.eq_ignore_ascii_case("allEvents") => true,
        Some(configured) => configured.eq_ignore_ascii_case(event),
    }
}

pub(crate) fn reject_transaction_hook(
    listener: &FlowableListener,
    kind: &str,
) -> Result<(), FlowableError> {
    if let Some(hook) = listener
        .on_transaction
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        return Err(FlowableError::ExecutionError(format!(
            "{} onTransaction='{}' is not supported; transaction-phase listeners are deferred",
            kind, hook
        )));
    }
    Ok(())
}

/// Extract execution listeners from any flow element variant.
pub fn flow_element_execution_listeners(flow_element: &FlowElementEnum) -> &[FlowableListener] {
    match flow_element {
        FlowElementEnum::SequenceFlow(f) => &f.flow_element.execution_listeners,
        FlowElementEnum::Task(t) => &t.activity.flow_node.flow_element.execution_listeners,
        FlowElementEnum::UserTask(t) => &t.task.activity.flow_node.flow_element.execution_listeners,
        FlowElementEnum::ServiceTask(t) => {
            &t.task.activity.flow_node.flow_element.execution_listeners
        }
        FlowElementEnum::CaseServiceTask(t) => {
            &t.service_task.task.activity.flow_node.flow_element.execution_listeners
        }
        FlowElementEnum::SendTask(t) => {
            &t.service_task.task.activity.flow_node.flow_element.execution_listeners
        }
        FlowElementEnum::ScriptTask(t) => {
            &t.task.activity.flow_node.flow_element.execution_listeners
        }
        FlowElementEnum::ManualTask(t) => {
            &t.task.activity.flow_node.flow_element.execution_listeners
        }
        FlowElementEnum::ReceiveTask(t) => {
            &t.task.activity.flow_node.flow_element.execution_listeners
        }
        FlowElementEnum::BusinessRuleTask(t) => {
            &t.task.activity.flow_node.flow_element.execution_listeners
        }
        FlowElementEnum::StartEvent(e) => &e.event.flow_node.flow_element.execution_listeners,
        FlowElementEnum::EndEvent(e) => &e.event.flow_node.flow_element.execution_listeners,
        FlowElementEnum::ExclusiveGateway(g) => {
            &g.gateway.flow_node.flow_element.execution_listeners
        }
        FlowElementEnum::ParallelGateway(g) => {
            &g.gateway.flow_node.flow_element.execution_listeners
        }
        FlowElementEnum::InclusiveGateway(g) => {
            &g.gateway.flow_node.flow_element.execution_listeners
        }
        FlowElementEnum::EventBasedGateway(g) => {
            &g.gateway.flow_node.flow_element.execution_listeners
        }
        FlowElementEnum::ComplexGateway(g) => {
            &g.gateway.flow_node.flow_element.execution_listeners
        }
        FlowElementEnum::IntermediateCatchEvent(e) => {
            &e.event.flow_node.flow_element.execution_listeners
        }
        FlowElementEnum::IntermediateThrowEvent(e) => {
            &e.event.flow_node.flow_element.execution_listeners
        }
        FlowElementEnum::SubProcess(s) => &s.activity.flow_node.flow_element.execution_listeners,
        FlowElementEnum::Transaction(t) => {
            &t.sub_process
                .activity
                .flow_node
                .flow_element
                .execution_listeners
        }
        FlowElementEnum::EventSubProcess(s) => {
            &s.sub_process
                .activity
                .flow_node
                .flow_element
                .execution_listeners
        }
        FlowElementEnum::AdhocSubProcess(s) => {
            &s.sub_process
                .activity
                .flow_node
                .flow_element
                .execution_listeners
        }
        FlowElementEnum::CallActivity(a) => &a.activity.flow_node.flow_element.execution_listeners,
        FlowElementEnum::BoundaryEvent(e) => &e.event.flow_node.flow_element.execution_listeners,
        FlowElementEnum::ValuedDataObject(d) => &d.execution_listeners,
    }
}
