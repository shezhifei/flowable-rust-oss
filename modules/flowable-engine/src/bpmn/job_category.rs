// Pre-existing `unwrap()` call(s) in the inline `#[cfg(test)]` module,
// grandfathered by the workspace clippy ratchet
// (`[workspace.lints.clippy] unwrap_used = "warn"` in the root Cargo.toml).
// `unwrap()` is the intended assertion tool there (JUnit-equivalent); the
// exemption is scoped with `cfg_attr(test, ...)` so it covers only this file's
// test code — a NEW unwrap() in production code is still surfaced.
// Do not add more without an audit note.
#![cfg_attr(test, allow(clippy::unwrap_used))]

use crate::el::expression::SimpleExpression;
use crate::error::FlowableError;
use crate::runtime::execution::Execution;
use flowable_bpmn_model::model::{BaseElement, FlowElementEnum};
use serde_json::Value;

/// Resolve the job category for a BPMN element, matching Java `JobUtil` /
/// `TimerUtil` semantics:
/// - first non-empty `jobCategory` extension wins
/// - literal text is returned unchanged
/// - `${...}` expressions are evaluated strictly against the execution
///   (A.2 #5, group S — evaluation errors propagate)
/// - string / number / bool results become unquoted strings
/// - null / missing / unsupported values map to `None`
pub(crate) fn resolve_job_category(
    base_element: &BaseElement,
    execution: &Execution,
) -> Result<Option<String>, FlowableError> {
    let text = first_job_category_text(base_element);
    match text {
        Some(text) => resolve_job_category_text(text, execution),
        None => Ok(None),
    }
}

/// Exhaustive helper so async-before/async-after paths do not duplicate nested
/// field access when obtaining the owning element's `BaseElement`.
pub(crate) fn flow_element_base_element(flow_element: &FlowElementEnum) -> &BaseElement {
    match flow_element {
        FlowElementEnum::SequenceFlow(flow) => &flow.flow_element.base_element,
        FlowElementEnum::Task(task) => &task.activity.flow_node.flow_element.base_element,
        FlowElementEnum::UserTask(task) => &task.task.activity.flow_node.flow_element.base_element,
        FlowElementEnum::ServiceTask(task) => {
            &task.task.activity.flow_node.flow_element.base_element
        }
        FlowElementEnum::CaseServiceTask(task) => {
            &task
                .service_task
                .task
                .activity
                .flow_node
                .flow_element
                .base_element
        }
        FlowElementEnum::SendTask(task) => {
            &task
                .service_task
                .task
                .activity
                .flow_node
                .flow_element
                .base_element
        }
        FlowElementEnum::ScriptTask(task) => {
            &task.task.activity.flow_node.flow_element.base_element
        }
        FlowElementEnum::ManualTask(task) => {
            &task.task.activity.flow_node.flow_element.base_element
        }
        FlowElementEnum::ReceiveTask(task) => {
            &task.task.activity.flow_node.flow_element.base_element
        }
        FlowElementEnum::BusinessRuleTask(task) => {
            &task.task.activity.flow_node.flow_element.base_element
        }
        FlowElementEnum::StartEvent(event) => &event.event.flow_node.flow_element.base_element,
        FlowElementEnum::EndEvent(event) => &event.event.flow_node.flow_element.base_element,
        FlowElementEnum::ExclusiveGateway(gateway) => {
            &gateway.gateway.flow_node.flow_element.base_element
        }
        FlowElementEnum::ParallelGateway(gateway) => {
            &gateway.gateway.flow_node.flow_element.base_element
        }
        FlowElementEnum::InclusiveGateway(gateway) => {
            &gateway.gateway.flow_node.flow_element.base_element
        }
        FlowElementEnum::EventBasedGateway(gateway) => {
            &gateway.gateway.flow_node.flow_element.base_element
        }
        FlowElementEnum::ComplexGateway(gateway) => {
            &gateway.gateway.flow_node.flow_element.base_element
        }
        FlowElementEnum::IntermediateCatchEvent(event) => {
            &event.event.flow_node.flow_element.base_element
        }
        FlowElementEnum::IntermediateThrowEvent(event) => {
            &event.event.flow_node.flow_element.base_element
        }
        FlowElementEnum::SubProcess(sub_process) => {
            &sub_process.activity.flow_node.flow_element.base_element
        }
        FlowElementEnum::Transaction(transaction) => {
            &transaction
                .sub_process
                .activity
                .flow_node
                .flow_element
                .base_element
        }
        FlowElementEnum::EventSubProcess(sub_process) => {
            &sub_process
                .sub_process
                .activity
                .flow_node
                .flow_element
                .base_element
        }
        FlowElementEnum::AdhocSubProcess(sub_process) => {
            &sub_process
                .sub_process
                .activity
                .flow_node
                .flow_element
                .base_element
        }
        FlowElementEnum::CallActivity(activity) => {
            &activity.activity.flow_node.flow_element.base_element
        }
        FlowElementEnum::ValuedDataObject(data_object) => &data_object.base_element,
        FlowElementEnum::BoundaryEvent(event) => &event.event.flow_node.flow_element.base_element,
    }
}

fn first_job_category_text(base_element: &BaseElement) -> Option<&str> {
    let elements = base_element.extension_elements.get("jobCategory")?;
    let first = elements.first()?;
    let text = first.element_text.as_deref()?.trim();
    if text.is_empty() { None } else { Some(text) }
}

fn resolve_job_category_text(
    text: &str,
    execution: &Execution,
) -> Result<Option<String>, FlowableError> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }

    if trimmed.starts_with("${") && trimmed.ends_with('}') {
        // A.2 #5 group S: jobCategory expression evaluation errors propagate.
        let value = SimpleExpression::new(trimmed.to_string())
            .get_value_strict(execution)
            .map_err(|error| {
                FlowableError::ExecutionError(format!(
                    "jobCategory expression '{trimmed}' failed: {error}"
                ))
            })?;
        return Ok(category_value_to_string(&value.unwrap_or(Value::Null)));
    }

    // Literal category text is returned unchanged (Java ExpressionManager
    // treats non-expression text as a fixed value).
    Ok(Some(trimmed.to_string()))
}

fn category_value_to_string(value: &Value) -> Option<String> {
    match value {
        Value::Null => None,
        Value::String(s) => {
            let trimmed = s.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        }
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        // Arrays/objects are unsupported for job category resolution.
        Value::Array(_) | Value::Object(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flowable_bpmn_model::model::ExtensionElement;
    use serde_json::json;
    use std::collections::HashMap;

    fn base_with_categories(texts: &[&str]) -> BaseElement {
        let mut base = BaseElement::default();
        let elements: Vec<ExtensionElement> = texts
            .iter()
            .map(|text| ExtensionElement {
                name: Some("jobCategory".to_string()),
                element_text: Some((*text).to_string()),
                ..Default::default()
            })
            .collect();
        base.extension_elements
            .insert("jobCategory".to_string(), elements);
        base
    }

    fn execution_with(vars: HashMap<String, Value>) -> Execution {
        Execution {
            variables: vars,
            ..Default::default()
        }
    }

    #[test]
    fn literal_category_is_returned_unchanged() {
        let base = base_with_categories(&["orders"]);
        let execution = Execution::default();
        assert_eq!(
            resolve_job_category(&base, &execution).unwrap().as_deref(),
            Some("orders")
        );
    }

    #[test]
    fn expression_string_category_is_evaluated() {
        let base = base_with_categories(&["${categoryValue}"]);
        let mut vars = HashMap::new();
        vars.insert("categoryValue".to_string(), json!("orders"));
        let execution = execution_with(vars);
        assert_eq!(
            resolve_job_category(&base, &execution).unwrap().as_deref(),
            Some("orders")
        );
    }

    #[test]
    fn expression_number_and_bool_use_java_style_string_value() {
        let base_number = base_with_categories(&["${categoryValue}"]);
        let mut number_vars = HashMap::new();
        number_vars.insert("categoryValue".to_string(), json!(42));
        assert_eq!(
            resolve_job_category(&base_number, &execution_with(number_vars))
                .unwrap()
                .as_deref(),
            Some("42")
        );

        let base_bool = base_with_categories(&["${categoryValue}"]);
        let mut bool_vars = HashMap::new();
        bool_vars.insert("categoryValue".to_string(), json!(true));
        assert_eq!(
            resolve_job_category(&base_bool, &execution_with(bool_vars))
                .unwrap()
                .as_deref(),
            Some("true")
        );
    }

    #[test]
    fn defined_null_expression_resolves_to_none() {
        let base = base_with_categories(&["${categoryValue}"]);
        let mut vars = HashMap::new();
        vars.insert("categoryValue".to_string(), Value::Null);
        assert_eq!(
            resolve_job_category(&base, &execution_with(vars)).unwrap(),
            None
        );
    }

    #[test]
    fn undefined_expression_is_error() {
        // A.2 #5 group S: undefined variable in a jobCategory expression must
        // propagate (Java getValue → PropertyNotFoundException).
        let base = base_with_categories(&["${categoryValue}"]);
        let execution = Execution::default();
        assert!(resolve_job_category(&base, &execution).is_err());
    }

    #[test]
    fn empty_text_resolves_to_none() {
        let base = base_with_categories(&["   "]);
        let execution = Execution::default();
        assert_eq!(resolve_job_category(&base, &execution).unwrap(), None);

        let base_empty = base_with_categories(&[""]);
        assert_eq!(resolve_job_category(&base_empty, &execution).unwrap(), None);
    }

    #[test]
    fn first_job_category_entry_wins() {
        let base = base_with_categories(&["first", "second"]);
        let execution = Execution::default();
        assert_eq!(
            resolve_job_category(&base, &execution).unwrap().as_deref(),
            Some("first")
        );
    }

    #[test]
    fn missing_extension_resolves_to_none() {
        let base = BaseElement::default();
        let execution = Execution::default();
        assert_eq!(resolve_job_category(&base, &execution).unwrap(), None);
    }
}
