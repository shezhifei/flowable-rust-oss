use crate::bpmn::listener::execution_listener_util::{
    listener_matches_event, reject_transaction_hook,
};
use crate::bpmn::listener::listener_registry::{
    LocalTaskListenerRegistry, TASK_LISTENER_REGISTRY_CACHE_KEY, TaskListenerContext,
};
use crate::el::expression::{Expression, SimpleExpression};
use crate::error::FlowableError;
use crate::interceptor::command_context::CommandContext;
use crate::runtime::execution::Execution;
use crate::task::Task;
use flowable_bpmn_model::model::{FieldExtension, FlowableListener};
use serde_json::{Map, Value};

/// Notify task listeners for the given event (`create` / `assignment` / `complete`).
pub fn notify_task_listeners(
    task: &mut Task,
    execution: &mut Execution,
    command_context: &CommandContext,
    listeners: &[FlowableListener],
    event: &str,
    evaluation_execution: &Execution,
) -> Result<(), FlowableError> {
    if listeners.is_empty() {
        return Ok(());
    }

    for listener in listeners {
        if !listener_matches_event(listener, event) {
            continue;
        }
        reject_transaction_hook(listener, "taskListener")?;
        invoke_task_listener(
            task,
            execution,
            command_context,
            listener,
            event,
            evaluation_execution,
        )?;
    }
    Ok(())
}

fn invoke_task_listener(
    task: &mut Task,
    execution: &mut Execution,
    command_context: &CommandContext,
    listener: &FlowableListener,
    event: &str,
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
                "taskListener on event '{}' has no implementation",
                event
            ))
        })?;

    match impl_type {
        "expression" => {
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
                .get(TASK_LISTENER_REGISTRY_CACHE_KEY)
                .and_then(|reg| reg.downcast_ref::<LocalTaskListenerRegistry>())
                .and_then(|reg| reg.get(&delegate_name));

            let listener_impl = registered.ok_or_else(|| {
                FlowableError::ExecutionError(format!(
                    "No local task listener '{}' is registered for event '{}'",
                    delegate_name, event
                ))
            })?;

            let mut ctx = TaskListenerContext {
                event,
                task,
                execution,
                fields: &fields,
            };
            listener_impl.notify(&mut ctx)
        }
        other => Err(FlowableError::BadRequest(format!(
            "Unsupported taskListener implementationType '{}' for event '{}'",
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
        return Ok(implementation.to_string());
    }

    if implementation.starts_with("${") && implementation.ends_with('}') {
        let value = SimpleExpression::new(implementation.to_string())
            .get_value(execution)
            .ok_or_else(|| {
                FlowableError::ExecutionError(format!(
                    "taskListener could not resolve delegateExpression '{}'",
                    implementation
                ))
            })?;
        value.as_str().map(|s| s.to_string()).ok_or_else(|| {
            FlowableError::ExecutionError(format!(
                "taskListener delegateExpression '{}' resolved to a non-string value",
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
                    "taskListener field extension is missing a name".to_string(),
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
                        "taskListener field '{}' expression '{}' could not be resolved",
                        name, expr
                    ))
                })?,
            (Some(_), Some(_)) => {
                return Err(FlowableError::ExecutionError(format!(
                    "taskListener field '{}' cannot have both stringValue and expression",
                    name
                )));
            }
            (None, None) => Value::Null,
        };
        out.insert(name.to_string(), value);
    }
    Ok(out)
}
