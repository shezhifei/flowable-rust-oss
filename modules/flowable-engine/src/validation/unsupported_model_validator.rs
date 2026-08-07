use crate::error::FlowableError;
use crate::service::config::ProcessEngineConfiguration;
use flowable_bpmn_model::model::{
    BaseElement, BoundaryEvent, BpmnModel, EventDefinitionEnum, ExtensionElement, FlowElementEnum,
    Process, ServiceTask,
};
use serde_json::Value;
use std::collections::{BTreeSet, HashMap, HashSet};

/// Validates a BPMN model to ensure it does not contain composite elements
/// or other structures that are explicitly deferred to Milestone 3 (M3).
pub struct UnsupportedModelValidator;

impl UnsupportedModelValidator {
    pub fn validate(
        model: &BpmnModel,
        config: &ProcessEngineConfiguration,
    ) -> Result<(), FlowableError> {
        for process in &model.processes {
            for element in &process.flow_elements {
                match element {
                    // Composite elements implemented in M3/M4
                    FlowElementEnum::SubProcess(_)
                    | FlowElementEnum::EventSubProcess(_)
                    | FlowElementEnum::Transaction(_)
                    | FlowElementEnum::AdhocSubProcess(_)
                    | FlowElementEnum::CallActivity(_) => {}
                    FlowElementEnum::ScriptTask(st) => {
                        if !config.enable_secure_scripting {
                            return Err(FlowableError::DeploymentValidationError(format!(
                                "Secure scripting is not enabled, cannot deploy script task '{}'",
                                st.task
                                    .activity
                                    .flow_node
                                    .flow_element
                                    .base_element
                                    .id
                                    .clone()
                                    .unwrap_or_default()
                            )));
                        }
                        if let Some(format) = &st.script_format
                            && !config
                                .supported_script_languages
                                .contains(&format.to_lowercase())
                        {
                            return Err(FlowableError::DeploymentValidationError(format!(
                                "Script format '{}' is not supported",
                                format
                            )));
                        }
                    }
                    FlowElementEnum::ServiceTask(st) => {
                        if let Some(type_) = &st.task_type {
                            let type_lower = type_.to_lowercase();
                            if type_lower == "http" {
                                validate_http_service_task(st, config)?;
                            } else if type_lower == "shell" {
                                validate_shell_service_task(st)?;
                            } else if type_lower == "mail" {
                                validate_mail_service_task(st, config)?;
                            } else if type_lower == "dmn" {
                                // Java ExternalInvocationTaskValidator.java:88-108
                                validate_dmn_service_task(st)?;
                            } else if type_lower == "send-event" {
                                validate_send_event_service_task(st)?;
                            }
                        } else if st.implementation.is_some()
                            || st.implementation_type.is_some()
                            || !st.task.activity.field_extensions.is_empty()
                        {
                            validate_delegate_expression_service_task(st)?;
                        }
                    }
                    // Java SendTaskParseHandler.java:37-56 — mail/dmn reuse the
                    // service-task validators. The webservice form is a deliberate
                    // deviation (Java WebServiceActivityBehavior is a legacy module
                    // not ported): reject it with a clear error instead of silently
                    // dropping the node. No-type sendTask is only warned by Java
                    // (SendTaskParseHandler.java:55) and passes through at runtime.
                    FlowElementEnum::SendTask(st) => {
                        let type_lower = st
                            .service_task
                            .task_type
                            .as_deref()
                            .map(str::to_lowercase)
                            .unwrap_or_default();
                        if type_lower == "webservice"
                            || st.service_task.implementation_type.as_deref()
                                == Some("webservice")
                        {
                            return Err(FlowableError::DeploymentValidationError(format!(
                                "sendTask '{}' uses the legacy webservice implementation which is not supported in this port",
                                activity_id(&st.service_task)
                            )));
                        }
                        match type_lower.as_str() {
                            "mail" => validate_mail_service_task(&st.service_task, config)?,
                            "dmn" => validate_dmn_service_task(&st.service_task)?,
                            _ => {}
                        }
                    }
                    // Other supported nodes are also allowed.
                    _ => {}
                }
            }
            validate_cancel_and_compensate_events(process)?;
        }
        Ok(())
    }
}

/// P20: deployment-time validation of cancel / compensate event constraints,
/// aligned with the Java process validators (`BoundaryEventValidator`,
/// `EndEventValidator`, `EventValidator`).
fn validate_cancel_and_compensate_events(process: &Process) -> Result<(), FlowableError> {
    let mut referencable_ids = HashSet::new();
    collect_referencable_ids(&process.flow_elements, &mut referencable_ids);
    let transaction_ids = {
        let mut ids = HashSet::new();
        collect_transaction_ids(&process.flow_elements, &mut ids);
        ids
    };

    let mut cancel_boundary_counts: HashMap<String, usize> = HashMap::new();
    let mut compensate_boundary_counts: HashMap<String, usize> = HashMap::new();
    validate_event_scope(
        &process.flow_elements,
        false,
        &referencable_ids,
        &transaction_ids,
        &mut cancel_boundary_counts,
        &mut compensate_boundary_counts,
    )?;

    for (attached_to_id, count) in &cancel_boundary_counts {
        if *count > 1 {
            // Java `BoundaryEventValidator` MULTIPLE_CANCEL_BOUNDARY_MISSING
            // (message text kept verbatim, including trailing period).
            return Err(FlowableError::DeploymentValidationError(format!(
                "multiple boundary events with cancelEventDefinition not supported on same transaction subprocess. (element '{attached_to_id}')"
            )));
        }
    }
    for (attached_to_id, count) in &compensate_boundary_counts {
        if *count > 1 {
            // Java `BoundaryEventValidator` COMPENSATE_EVENT_MULTIPLE_ON_BOUNDARY
            return Err(FlowableError::DeploymentValidationError(format!(
                "Multiple boundary events of type 'compensate' is invalid (element '{attached_to_id}')"
            )));
        }
    }
    Ok(())
}

fn validate_event_scope(
    flow_elements: &[FlowElementEnum],
    container_is_transaction: bool,
    referencable_ids: &HashSet<String>,
    transaction_ids: &HashSet<String>,
    cancel_boundary_counts: &mut HashMap<String, usize>,
    compensate_boundary_counts: &mut HashMap<String, usize>,
) -> Result<(), FlowableError> {
    for element in flow_elements {
        match element {
            FlowElementEnum::BoundaryEvent(boundary) => {
                validate_boundary_event(
                    boundary,
                    referencable_ids,
                    transaction_ids,
                    cancel_boundary_counts,
                    compensate_boundary_counts,
                )?;
            }
            FlowElementEnum::EndEvent(end_event) => {
                let element_id = flow_element_label(element);
                if matches!(
                    end_event.event.event_definitions.first(),
                    Some(EventDefinitionEnum::CancelEventDefinition(_))
                ) && !container_is_transaction
                {
                    // Java `EndEventValidator` END_EVENT_CANCEL_ONLY_INSIDE_TRANSACTION
                    return Err(FlowableError::DeploymentValidationError(format!(
                        "end event with cancelEventDefinition only supported inside transaction subprocess (element '{element_id}')"
                    )));
                }
                validate_compensate_activity_ref(
                    &end_event.event.event_definitions,
                    &element_id,
                    referencable_ids,
                )?;
                validate_timer_event_definitions(&end_event.event.event_definitions, &element_id)?;
            }
            FlowElementEnum::StartEvent(start_event) => {
                validate_compensate_activity_ref(
                    &start_event.event.event_definitions,
                    &flow_element_label(element),
                    referencable_ids,
                )?;
                validate_timer_event_definitions(
                    &start_event.event.event_definitions,
                    &flow_element_label(element),
                )?;
            }
            FlowElementEnum::IntermediateCatchEvent(catch_event) => {
                validate_compensate_activity_ref(
                    &catch_event.event.event_definitions,
                    &flow_element_label(element),
                    referencable_ids,
                )?;
                validate_timer_event_definitions(
                    &catch_event.event.event_definitions,
                    &flow_element_label(element),
                )?;
            }
            // NOTE: intermediateThrowEvent + cancelEventDefinition stays
            // deployable (runtime fails gracefully, see
            // `test_unsupported_intermediate_event_fails_gracefully`); only
            // its compensate activityRef is validated here.
            FlowElementEnum::IntermediateThrowEvent(throw_event) => {
                validate_compensate_activity_ref(
                    &throw_event.event.event_definitions,
                    &flow_element_label(element),
                    referencable_ids,
                )?;
                validate_timer_event_definitions(
                    &throw_event.event.event_definitions,
                    &flow_element_label(element),
                )?;
            }
            _ => {}
        }

        if let Some(nested) = container_flow_elements(element) {
            validate_event_scope(
                nested,
                matches!(element, FlowElementEnum::Transaction(_)),
                referencable_ids,
                transaction_ids,
                cancel_boundary_counts,
                compensate_boundary_counts,
            )?;
        }
    }
    Ok(())
}

fn validate_boundary_event(
    boundary: &BoundaryEvent,
    referencable_ids: &HashSet<String>,
    transaction_ids: &HashSet<String>,
    cancel_boundary_counts: &mut HashMap<String, usize>,
    compensate_boundary_counts: &mut HashMap<String, usize>,
) -> Result<(), FlowableError> {
    let boundary_id = boundary
        .event
        .flow_node
        .flow_element
        .base_element
        .id
        .clone()
        .unwrap_or_default();
    let attached_to_id = boundary.attached_to_ref_id.clone().unwrap_or_default();

    match boundary.event.event_definitions.first() {
        Some(EventDefinitionEnum::CancelEventDefinition(_)) => {
            if !transaction_ids.contains(&attached_to_id) {
                // Java `BoundaryEventValidator` CANCEL_BOUNDARY_ONLY_ON_TRANSACTION
                return Err(FlowableError::DeploymentValidationError(format!(
                    "boundary event with cancelEventDefinition only supported on transaction subprocesses (element '{boundary_id}')"
                )));
            }
            *cancel_boundary_counts.entry(attached_to_id).or_insert(0) += 1;
        }
        Some(EventDefinitionEnum::CompensateEventDefinition(_)) => {
            *compensate_boundary_counts
                .entry(attached_to_id)
                .or_insert(0) += 1;
            validate_compensate_activity_ref(
                &boundary.event.event_definitions,
                &boundary_id,
                referencable_ids,
            )?;
        }
        _ => {}
    }
    validate_timer_event_definitions(&boundary.event.event_definitions, &boundary_id)?;
    Ok(())
}

/// Java `EventValidator#handleTimerEventDefinition` (EventValidator.java:89-93):
/// a timer event definition without any of timeDate / timeCycle / timeDuration
/// fails deployment with EVENT_TIMER_MISSING_CONFIGURATION.
fn validate_timer_event_definitions(
    event_definitions: &[EventDefinitionEnum],
    element_id: &str,
) -> Result<(), FlowableError> {
    for definition in event_definitions {
        if let EventDefinitionEnum::TimerEventDefinition(timer) = definition
            && timer
                .time_date
                .as_deref()
                .is_none_or(|value| value.trim().is_empty())
            && timer
                .time_cycle
                .as_deref()
                .is_none_or(|value| value.trim().is_empty())
            && timer
                .time_duration
                .as_deref()
                .is_none_or(|value| value.trim().is_empty())
        {
            return Err(FlowableError::DeploymentValidationError(format!(
                "Timer needs configuration (either timeDate, timeCycle or timeDuration is needed) (element '{element_id}')"
            )));
        }
    }
    Ok(())
}

/// Java `EventValidator#handleCompensationEventDefinition`: a non-empty
/// `activityRef` must reference an existing activity of the process.
fn validate_compensate_activity_ref(
    event_definitions: &[EventDefinitionEnum],
    element_id: &str,
    referencable_ids: &HashSet<String>,
) -> Result<(), FlowableError> {
    for definition in event_definitions {
        if let EventDefinitionEnum::CompensateEventDefinition(compensate) = definition
            && let Some(activity_ref) = compensate
                .activity_ref
                .as_deref()
                .filter(|activity_ref| !activity_ref.is_empty())
            && !referencable_ids.contains(activity_ref)
        {
            return Err(FlowableError::DeploymentValidationError(format!(
                "Invalid attribute value for 'activityRef': no activity with the given id (element '{element_id}', activityRef '{activity_ref}')"
            )));
        }
    }
    Ok(())
}

fn flow_element_label(element: &FlowElementEnum) -> String {
    crate::agenda::continue_process_operation::flow_element_id(element)
        .unwrap_or_default()
        .to_string()
}

fn container_flow_elements(element: &FlowElementEnum) -> Option<&[FlowElementEnum]> {
    crate::bpmn::behavior::intermediate_throw_event_activity_behavior::container_flow_elements(
        element,
    )
}

/// Ids an `activityRef` may point at: every flow element except sequence
/// flows, collected transitively.
fn collect_referencable_ids(flow_elements: &[FlowElementEnum], collected: &mut HashSet<String>) {
    for element in flow_elements {
        if !matches!(element, FlowElementEnum::SequenceFlow(_))
            && let Some(id) = crate::agenda::continue_process_operation::flow_element_id(element)
        {
            collected.insert(id.to_string());
        }
        if let Some(nested) = container_flow_elements(element) {
            collect_referencable_ids(nested, collected);
        }
    }
}

fn collect_transaction_ids(flow_elements: &[FlowElementEnum], collected: &mut HashSet<String>) {
    for element in flow_elements {
        if let FlowElementEnum::Transaction(_) = element
            && let Some(id) = crate::agenda::continue_process_operation::flow_element_id(element)
        {
            collected.insert(id.to_string());
        }
        if let Some(nested) = container_flow_elements(element) {
            collect_transaction_ids(nested, collected);
        }
    }
}

fn validate_delegate_expression_service_task(
    service_task: &ServiceTask,
) -> Result<(), FlowableError> {
    // M76: `class` is a registry key (FQCN-like string), not JVM classloading.
    // Same LocalServiceTaskDelegateRegistry as `delegateExpression`.
    let implementation_type = match service_task.implementation_type.as_deref() {
        Some("delegateExpression") | Some("class") => {
            service_task.implementation_type.as_deref().unwrap()
        }
        Some(implementation_type) => {
            return Err(FlowableError::DeploymentValidationError(format!(
                "Delegate service task '{}' only supports class or delegateExpression implementation in the owned M14 subset; got '{}'",
                activity_id(service_task),
                implementation_type
            )));
        }
        None => {
            return Err(FlowableError::DeploymentValidationError(format!(
                "Delegate service task '{}' uses field extensions without a class or delegateExpression implementation in the owned M14 subset",
                activity_id(service_task)
            )));
        }
    };

    let implementation = service_task
        .implementation
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            FlowableError::DeploymentValidationError(format!(
                "Delegate service task '{}' requires a {} implementation in the owned M14 subset",
                activity_id(service_task),
                implementation_type
            ))
        })?;
    if implementation_type == "delegateExpression" {
        // delegateExpression keeps the simple ${...} form rule.
        if !(implementation.starts_with("${") && implementation.ends_with('}')) {
            return Err(FlowableError::DeploymentValidationError(format!(
                "Delegate service task '{}' only supports simple ${{...}} delegateExpression values in the owned M14 subset",
                activity_id(service_task)
            )));
        }
    } else {
        // `class`: FQCN-like registry key — reject expression forms.
        if implementation.starts_with("${") {
            return Err(FlowableError::DeploymentValidationError(format!(
                "Delegate service task '{}' with class implementation must be a registry key (FQCN-like string), not an expression",
                activity_id(service_task)
            )));
        }
    }

    validate_field_extensions(service_task)?;
    validate_io_parameters(
        service_task,
        "Delegate",
        "inParameter",
        &service_task.in_parameters,
    )?;
    validate_io_parameters(
        service_task,
        "Delegate",
        "outParameter",
        &service_task.out_parameters,
    )?;
    if service_task.parallel_in_same_transaction.is_some() {
        return Err(FlowableError::DeploymentValidationError(format!(
            "Delegate service task '{}' uses unsupported advanced runtime flags for the owned M14 subset",
            activity_id(service_task)
        )));
    }
    // P51 S4: class/delegateExpression may set flowable:triggerable (execute without leave).

    let allowed_extensions = BTreeSet::from(["field", "in", "out"]);
    validate_allowed_extensions(service_task, "Delegate", &allowed_extensions)?;
    Ok(())
}

fn validate_field_extensions(service_task: &ServiceTask) -> Result<(), FlowableError> {
    for field in &service_task.task.activity.field_extensions {
        let field_name = field
            .field_name
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                FlowableError::DeploymentValidationError(format!(
                    "Delegate service task '{}' requires each field extension to have a name in the owned M14 subset",
                    activity_id(service_task)
                ))
            })?;

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
        if string_value.is_some() == expression.is_some() {
            return Err(FlowableError::DeploymentValidationError(format!(
                "Delegate service task '{}' field '{}' must define exactly one of stringValue or expression in the owned M14 subset",
                activity_id(service_task),
                field_name
            )));
        }
        if let Some(expression) = expression
            && !(expression.starts_with("${") && expression.ends_with('}'))
        {
            return Err(FlowableError::DeploymentValidationError(format!(
                "Delegate service task '{}' field '{}' only supports simple ${{...}} expression values in the owned M14 subset",
                activity_id(service_task),
                field_name
            )));
        }
    }
    Ok(())
}

fn validate_send_event_service_task(service_task: &ServiceTask) -> Result<(), FlowableError> {
    validate_owned_service_task_shape(service_task, "Send event")?;

    if service_task
        .event_type
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .is_none()
    {
        return Err(FlowableError::DeploymentValidationError(format!(
            "Task type 'send-event' is not supported unless it matches the owned event registry send task subset; Send event service task '{}' requires eventType",
            activity_id(service_task)
        )));
    }

    let has_trigger_event_type = service_task
        .trigger_event_type
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .is_some();
    if (service_task.triggerable || !service_task.event_out_parameters.is_empty())
        && !has_trigger_event_type
    {
        return Err(FlowableError::DeploymentValidationError(format!(
            "Task type 'send-event' is not supported unless it matches the owned event registry send task subset; Send event service task '{}' requires triggerEventType for send-and-receive trigger semantics",
            activity_id(service_task)
        )));
    }

    let allowed_extensions = BTreeSet::from([
        "eventType",
        "eventInParameter",
        "eventOutParameter",
        "in",
        "out",
        "sendSynchronously",
        "triggerEventType",
        // P134: Java SendEventTaskActivityBehavior.java:140 —
        // CorrelationUtil.getCorrelationKey(ELEMENT_TRIGGER_EVENT_CORRELATION_PARAMETER).
        "triggerEventCorrelationParameter",
    ]);
    validate_allowed_extensions(service_task, "Send event", &allowed_extensions)?;
    Ok(())
}

fn validate_shell_service_task(service_task: &ServiceTask) -> Result<(), FlowableError> {
    validate_owned_service_task_shape(service_task, "Shell")?;

    // Java ShellActivityBehavior: outputVariable / errorCodeVariable / redirectError / cleanEnv
    let allowed_extensions = BTreeSet::from([
        "arg",
        "cleanEnv",
        "command",
        "errorCodeVariable",
        "in",
        "out",
        "outputVariable",
        "redirectError",
        "timeout",
        "wait",
        "workingDirectory",
    ]);
    validate_allowed_extensions(service_task, "Shell", &allowed_extensions)?;

    let _ = require_literal_extension(service_task, "Shell", "command")?;
    let _ = optional_literal_extension(service_task, "Shell", "workingDirectory");
    validate_optional_u64_extension(service_task, "Shell", "timeout")?;
    validate_optional_bool_extension(service_task, "Shell", "wait")?;
    validate_optional_bool_extension(service_task, "Shell", "redirectError")?;
    validate_optional_bool_extension(service_task, "Shell", "cleanEnv")?;

    Ok(())
}

fn validate_http_service_task(
    service_task: &ServiceTask,
    config: &ProcessEngineConfiguration,
) -> Result<(), FlowableError> {
    if !config.http_service.enabled {
        return Err(FlowableError::DeploymentValidationError(
            "Task type 'http' is not supported in M9 unless it matches the owned M14 HTTP service task subset; HTTP service tasks are disabled in configuration".to_string(),
        ));
    }

    validate_http_service_task_shape(service_task)?;

    let allowed_extensions = BTreeSet::from([
        "basicAuthenticationPassword",
        "basicAuthenticationUsername",
        "bodyEncoding",
        "connectTimeout",
        "followRedirects",
        "in",
        "out",
        "requestBody",
        "requestHeaders",
        "requestMethod",
        "requestTimeout",
        "requestUrl",
        "requestBodyEncoding",
        "disallowRedirects",
        "failStatusCodes",
        "handleStatusCodes",
        "httpRequestHandler",
        "httpResponseHandler",
        "ignoreException",
        "saveRequestVariables",
        "saveResponseParameters",
        "saveResponseParametersTransient",
        "saveResponseVariableAsJson",
        "responseVariableName",
        "resultVariablePrefix",
        "parallelInSameTransaction",
    ]);
    validate_allowed_extensions(service_task, "HTTP", &allowed_extensions)?;
    validate_allowed_http_field_extensions(service_task, &allowed_extensions)?;
    validate_http_handler(service_task, config, "httpRequestHandler")?;
    validate_http_handler(service_task, config, "httpResponseHandler")?;

    let method = require_literal_extension(service_task, "HTTP", "requestMethod")?;
    if !is_expression_text(&method) {
        let method = method.to_uppercase();
        if !config
            .http_service
            .supported_methods
            .iter()
            .any(|supported| supported.eq_ignore_ascii_case(&method))
        {
            return Err(FlowableError::DeploymentValidationError(format!(
                "Task type 'http' is not supported in M9 unless it matches the owned M14 HTTP service task subset; HTTP service task '{}' uses unsupported method '{}'",
                activity_id(service_task),
                method
            )));
        }
    }

    let _ = require_literal_extension(service_task, "HTTP", "requestUrl")?;

    if let Some(raw_headers) = optional_literal_extension(service_task, "HTTP", "requestHeaders") {
        if !is_expression_text(&raw_headers) {
            let parsed = serde_json::from_str::<Value>(&raw_headers).map_err(|error| {
                FlowableError::DeploymentValidationError(format!(
                    "Task type 'http' is not supported in M9 unless it matches the owned M14 HTTP service task subset; HTTP service task '{}' has invalid requestHeaders JSON: {}",
                    activity_id(service_task),
                    error
                ))
            })?;
            if !parsed.is_object() {
                return Err(FlowableError::DeploymentValidationError(format!(
                    "Task type 'http' is not supported in M9 unless it matches the owned M14 HTTP service task subset; HTTP service task '{}' requires requestHeaders to be a JSON object",
                    activity_id(service_task)
                )));
            }
        }
    }

    let _ = optional_literal_extension(service_task, "HTTP", "requestBody");
    validate_http_basic_auth_extensions(service_task)?;
    validate_http_body_encoding_extension(service_task)?;
    validate_optional_u64_extension(service_task, "HTTP", "requestTimeout")?;
    validate_optional_u64_extension(service_task, "HTTP", "connectTimeout")?;
    validate_optional_bool_extension(service_task, "HTTP", "followRedirects")?;
    validate_optional_bool_extension(service_task, "HTTP", "disallowRedirects")?;
    validate_optional_bool_extension(service_task, "HTTP", "ignoreException")?;
    validate_optional_bool_extension(service_task, "HTTP", "saveRequestVariables")?;
    validate_optional_bool_extension(service_task, "HTTP", "saveResponseParameters")?;
    validate_optional_bool_extension(service_task, "HTTP", "saveResponseParametersTransient")?;
    validate_optional_bool_extension(service_task, "HTTP", "saveResponseVariableAsJson")?;
    validate_optional_bool_extension(service_task, "HTTP", "parallelInSameTransaction")?;
    Ok(())
}

fn validate_http_basic_auth_extensions(service_task: &ServiceTask) -> Result<(), FlowableError> {
    let username = optional_literal_extension(service_task, "HTTP", "basicAuthenticationUsername");
    let password = optional_literal_extension(service_task, "HTTP", "basicAuthenticationPassword");
    if username.is_some() != password.is_some() {
        return Err(FlowableError::DeploymentValidationError(format!(
            "Task type 'http' is not supported in M9 unless it matches the owned M14 HTTP service task subset; HTTP service task '{}' requires basicAuthenticationUsername and basicAuthenticationPassword to be configured together",
            activity_id(service_task)
        )));
    }
    Ok(())
}

fn validate_http_handler(
    service_task: &ServiceTask,
    config: &ProcessEngineConfiguration,
    handler_name: &str,
) -> Result<(), FlowableError> {
    let handlers = service_task_base_element(service_task)
        .extension_elements
        .get(handler_name)
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    if handlers.is_empty() {
        return Ok(());
    }
    if handlers.len() != 1 {
        return Err(FlowableError::DeploymentValidationError(format!(
            "HTTP service task '{}' must define at most one {}",
            activity_id(service_task),
            handler_name
        )));
    }
    let handler = &handlers[0];
    let class = extension_attribute_value(handler, "class");
    let delegate_expression = extension_attribute_value(handler, "delegateExpression");
    let handler_type = extension_attribute_value(handler, "type");
    let configured = [
        class.is_some(),
        delegate_expression.is_some(),
        handler_type.is_some(),
    ]
    .into_iter()
    .filter(|configured| *configured)
    .count();
    if configured != 1 {
        return Err(FlowableError::DeploymentValidationError(format!(
            "HTTP service task '{}' {} must define exactly one of class, delegateExpression, or type",
            activity_id(service_task),
            handler_name
        )));
    }
    if let Some(expression) = delegate_expression
        && !(expression.starts_with("${") && expression.ends_with('}'))
    {
        return Err(FlowableError::DeploymentValidationError(format!(
            "HTTP service task '{}' {} delegateExpression must use ${{...}} syntax",
            activity_id(service_task),
            handler_name
        )));
    }
    if let Some(handler_type) = handler_type {
        if !handler_type.eq_ignore_ascii_case("script") {
            return Err(FlowableError::DeploymentValidationError(format!(
                "HTTP service task '{}' {} uses unsupported type '{}'",
                activity_id(service_task),
                handler_name,
                handler_type
            )));
        }
        if !config.enable_secure_scripting {
            return Err(FlowableError::DeploymentValidationError(format!(
                "HTTP service task '{}' {} script requires secure scripting to be enabled",
                activity_id(service_task),
                handler_name
            )));
        }
        let scripts = handler
            .child_elements
            .get("script")
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        if scripts.len() != 1
            || scripts[0]
                .element_text
                .as_deref()
                .is_none_or(|body| body.trim().is_empty())
        {
            return Err(FlowableError::DeploymentValidationError(format!(
                "HTTP service task '{}' {} type='script' requires exactly one non-empty flowable:script child",
                activity_id(service_task),
                handler_name
            )));
        }
        let language = extension_attribute_value(&scripts[0], "language")
            .unwrap_or_else(|| "javascript".to_string());
        if !config
            .supported_script_languages
            .iter()
            .any(|supported| supported.eq_ignore_ascii_case(&language))
        {
            return Err(FlowableError::DeploymentValidationError(format!(
                "HTTP service task '{}' {} script language '{}' is not enabled",
                activity_id(service_task),
                handler_name,
                language
            )));
        }
        if handler.child_elements.keys().any(|name| name != "script") {
            return Err(FlowableError::DeploymentValidationError(format!(
                "HTTP service task '{}' {} script handler only supports a flowable:script child",
                activity_id(service_task),
                handler_name
            )));
        }
    } else if handler.child_elements.keys().any(|name| name != "field") {
        return Err(FlowableError::DeploymentValidationError(format!(
            "HTTP service task '{}' {} class/delegateExpression handler only supports flowable:field children",
            activity_id(service_task),
            handler_name
        )));
    }
    Ok(())
}

fn extension_attribute_value(
    element: &flowable_bpmn_model::model::ExtensionElement,
    name: &str,
) -> Option<String> {
    element
        .base_element
        .attributes
        .get(name)
        .and_then(|values| values.first())
        .and_then(|attribute| attribute.value.clone())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn validate_http_body_encoding_extension(service_task: &ServiceTask) -> Result<(), FlowableError> {
    let Some(body_encoding) = optional_literal_extension(service_task, "HTTP", "bodyEncoding")
        .or_else(|| optional_literal_extension(service_task, "HTTP", "requestBodyEncoding"))
    else {
        return Ok(());
    };
    if !matches!(
        body_encoding.to_ascii_lowercase().as_str(),
        "json" | "form" | "text"
    ) {
        return Err(FlowableError::DeploymentValidationError(format!(
            "Task type 'http' is not supported in M9 unless it matches the owned M14 HTTP service task subset; HTTP service task '{}' uses unsupported bodyEncoding '{}' (expected json, form, or text)",
            activity_id(service_task),
            body_encoding
        )));
    }
    Ok(())
}

/// Java `ExternalInvocationTaskValidator.validateFieldDeclarationsForDmn`
/// (`ExternalInvocationTaskValidator.java:88-108`).
///
/// Requires at least one of `decisionTableReferenceKey` / `decisionServiceReferenceKey`
/// with a non-empty stringValue (or expression). Does **not** call
/// `validate_owned_service_task_shape` (that rejects field extensions).
fn validate_dmn_service_task(service_task: &ServiceTask) -> Result<(), FlowableError> {
    // Do not reject implementation/field shape the way mail/shell do —
    // DMN is field-extension driven (Java DmnActivityBehavior holds Task fields).
    if service_task.implementation.is_some() || service_task.implementation_type.is_some() {
        return Err(FlowableError::DeploymentValidationError(format!(
            "DMN service task '{}' does not support implementation delegates",
            activity_id(service_task)
        )));
    }

    validate_io_parameters(
        service_task,
        "DMN",
        "inParameter",
        &service_task.in_parameters,
    )?;
    validate_io_parameters(
        service_task,
        "DMN",
        "outParameter",
        &service_task.out_parameters,
    )?;

    // Allowed field / extension names — Java ExternalInvocationTaskValidator +
    // DmnActivityBehavior field set (decisionTableReferenceKey, decisionServiceReferenceKey,
    // decisionTaskThrowErrorOnNoHits, fallbackToDefaultTenant, sameDeployment) plus in/out.
    let allowed_extensions = BTreeSet::from([
        "decisionTableReferenceKey",
        "decisionServiceReferenceKey",
        "decisionTaskThrowErrorOnNoHits",
        "fallbackToDefaultTenant",
        "sameDeployment",
        "field",
        "in",
        "out",
    ]);
    validate_allowed_extensions(service_task, "DMN", &allowed_extensions)?;
    validate_allowed_dmn_field_extensions(service_task, &allowed_extensions)?;

    // Java :88-107 — at least one of decisionTable / decisionService key non-empty.
    // Java only checks stringValue; we also accept expression so EL-only keys deploy.
    let key_defined = service_task.task.activity.field_extensions.iter().any(|field| {
        let name = field.field_name.as_deref().unwrap_or("");
        if name != "decisionTableReferenceKey" && name != "decisionServiceReferenceKey" {
            return false;
        }
        let string_ok = field
            .string_value
            .as_deref()
            .map(str::trim)
            .is_some_and(|v| !v.is_empty());
        let expr_ok = field
            .expression
            .as_deref()
            .map(str::trim)
            .is_some_and(|v| !v.is_empty());
        string_ok || expr_ok
    });

    if !key_defined {
        return Err(FlowableError::DeploymentValidationError(format!(
            "No decision table or decision service reference key is defined on the dmn activity (element '{}')",
            activity_id(service_task)
        )));
    }

    Ok(())
}

fn validate_allowed_dmn_field_extensions(
    service_task: &ServiceTask,
    allowed_extensions: &BTreeSet<&str>,
) -> Result<(), FlowableError> {
    for field in &service_task.task.activity.field_extensions {
        if let Some(field_name) = field.field_name.as_deref()
            && !allowed_extensions.contains(field_name)
        {
            return Err(FlowableError::DeploymentValidationError(format!(
                "DMN service task '{}' uses unsupported field extension '{}'",
                activity_id(service_task),
                field_name
            )));
        }
    }
    Ok(())
}

fn validate_mail_service_task(
    service_task: &ServiceTask,
    config: &ProcessEngineConfiguration,
) -> Result<(), FlowableError> {
    if !config.mail_service.enabled {
        return Err(FlowableError::DeploymentValidationError(
            "Task type 'mail' is not supported in M9 unless it matches the owned M14 Mail service task subset; Mail service tasks are disabled in configuration".to_string(),
        ));
    }

    // P124: allow field extensions for mail (Java BaseMailActivityDelegate fields are
    // injected via flowable:field). Shape rejects implementation delegates / triggerable
    // but no longer blanket-rejects field_extensions.
    validate_mail_service_task_shape(service_task)?;

    // Java BaseMailActivityDelegate fields: to/from/cc/bcc/headers/subject/text/textVar/
    // html/htmlVar/charset/ignoreException/exceptionVariableName/attachments
    // (BaseMailActivityDelegate.java:51-64).
    let allowed_extensions = BTreeSet::from([
        "attachments",
        "bcc",
        "cc",
        "charset",
        "exceptionVariableName",
        "from",
        "headers",
        "html",
        "htmlVar",
        "ignoreException",
        "in",
        "out",
        "subject",
        "text",
        "textVar",
        "to",
        // field element itself appears under extension_elements when parsed as child
        "field",
    ]);
    validate_allowed_extensions(service_task, "Mail", &allowed_extensions)?;
    validate_allowed_mail_field_extensions(service_task, &allowed_extensions)?;

    // P51 S2: mail fields are Expressions in Java — allow ${...} EL text as well as literals.
    // `to` is optional when cc/bcc supply recipients (runtime check); when present as
    // a non-EL literal it must contain at least one address.
    if let Some(to) = optional_expression_or_literal_extension(service_task, "Mail", "to") {
        if !is_expression_text(&to) && split_recipients(&to).is_empty() {
            return Err(FlowableError::DeploymentValidationError(format!(
                "Task type 'mail' is not supported in M9 unless it matches the owned M14 Mail service task subset; Mail service task '{}' requires at least one literal recipient",
                activity_id(service_task)
            )));
        }
    }

    let _ = optional_expression_or_literal_extension(service_task, "Mail", "from");
    let _ = optional_expression_or_literal_extension(service_task, "Mail", "cc");
    let _ = optional_expression_or_literal_extension(service_task, "Mail", "bcc");
    let _ = optional_expression_or_literal_extension(service_task, "Mail", "charset");
    let _ = optional_expression_or_literal_extension(service_task, "Mail", "headers");
    let _ = optional_expression_or_literal_extension(service_task, "Mail", "textVar");
    let _ = optional_expression_or_literal_extension(service_task, "Mail", "htmlVar");
    let _ = optional_expression_or_literal_extension(service_task, "Mail", "attachments");
    let _ = require_expression_or_literal_extension(service_task, "Mail", "subject")?;
    // Java BaseMailActivityDelegate.createMessage:112-114 — at least one of text/textVar/html/htmlVar.
    let has_body = ["text", "textVar", "html", "htmlVar"].iter().any(|name| {
        optional_expression_or_literal_extension(service_task, "Mail", name).is_some()
    });
    if !has_body {
        return Err(FlowableError::DeploymentValidationError(format!(
            "Task type 'mail' is not supported in M9 unless it matches the owned M14 Mail service task subset; Mail service task '{}' requires at least one of text/textVar/html/htmlVar",
            activity_id(service_task)
        )));
    }
    let _ = optional_expression_or_literal_extension(service_task, "Mail", "text");
    let _ = optional_expression_or_literal_extension(service_task, "Mail", "html");
    validate_optional_bool_extension(service_task, "Mail", "ignoreException")?;
    let _ = optional_expression_or_literal_extension(service_task, "Mail", "exceptionVariableName");

    Ok(())
}

/// Mail shape: reject implementation delegates / triggerable, but allow field extensions
/// (Java injects BaseMailActivityDelegate fields via flowable:field).
fn validate_mail_service_task_shape(service_task: &ServiceTask) -> Result<(), FlowableError> {
    if service_task.implementation.is_some() || service_task.implementation_type.is_some() {
        return Err(FlowableError::DeploymentValidationError(format!(
            "Mail service task '{}' does not support implementation delegates in the owned M14 subset",
            activity_id(service_task)
        )));
    }
    validate_io_parameters(
        service_task,
        "Mail",
        "inParameter",
        &service_task.in_parameters,
    )?;
    validate_io_parameters(
        service_task,
        "Mail",
        "outParameter",
        &service_task.out_parameters,
    )?;
    if service_task.parallel_in_same_transaction.is_some() {
        return Err(FlowableError::DeploymentValidationError(format!(
            "Mail service task '{}' uses unsupported advanced runtime flags for the owned M14 subset",
            activity_id(service_task)
        )));
    }
    if service_task.triggerable {
        return Err(FlowableError::DeploymentValidationError(format!(
            "Mail service task '{}' uses unsupported advanced runtime flags for the owned M14 subset",
            activity_id(service_task)
        )));
    }
    Ok(())
}

fn validate_allowed_mail_field_extensions(
    service_task: &ServiceTask,
    allowed_extensions: &BTreeSet<&str>,
) -> Result<(), FlowableError> {
    for field in &service_task.task.activity.field_extensions {
        if let Some(field_name) = field.field_name.as_deref()
            && !allowed_extensions.contains(field_name)
        {
            return Err(FlowableError::DeploymentValidationError(format!(
                "Task type 'mail' is not supported in M9 unless it matches the owned M14 Mail service task subset; Mail service task '{}' uses unsupported field extension '{}'",
                activity_id(service_task),
                field_name
            )));
        }
    }
    Ok(())
}

fn validate_optional_u64_extension(
    service_task: &ServiceTask,
    label: &str,
    extension_name: &str,
) -> Result<(), FlowableError> {
    if let Some(raw) = optional_literal_extension(service_task, label, extension_name)
        && !(label == "HTTP" && is_expression_text(&raw))
    {
        raw.parse::<u64>().map_err(|error| {
            FlowableError::DeploymentValidationError(format!(
                "{} service task '{}' requires extension '{}' to be an integer number of milliseconds: {}",
                label,
                activity_id(service_task),
                extension_name,
                error
            ))
        })?;
    }
    Ok(())
}

fn validate_optional_bool_extension(
    service_task: &ServiceTask,
    label: &str,
    extension_name: &str,
) -> Result<(), FlowableError> {
    if let Some(raw) = optional_literal_extension(service_task, label, extension_name)
        && !(label == "HTTP" && is_expression_text(&raw))
    {
        match raw.as_str() {
            "true" | "false" => {}
            _ => {
                return Err(FlowableError::DeploymentValidationError(format!(
                    "{} service task '{}' requires extension '{}' to be true or false",
                    label,
                    activity_id(service_task),
                    extension_name
                )));
            }
        }
    }
    Ok(())
}

fn split_recipients(value: &str) -> Vec<String> {
    value
        .split([',', ';'])
        .map(str::trim)
        .filter(|recipient| !recipient.is_empty())
        .map(str::to_string)
        .collect()
}

fn validate_owned_service_task_shape(
    service_task: &ServiceTask,
    label: &str,
) -> Result<(), FlowableError> {
    if service_task.implementation.is_some() || service_task.implementation_type.is_some() {
        return Err(FlowableError::DeploymentValidationError(format!(
            "{} service task '{}' does not support implementation delegates in the owned M14 subset",
            label,
            activity_id(service_task)
        )));
    }
    if !service_task.task.activity.field_extensions.is_empty() {
        return Err(FlowableError::DeploymentValidationError(format!(
            "{} service task '{}' does not support field extensions in the owned M14 subset",
            label,
            activity_id(service_task)
        )));
    }
    validate_io_parameters(
        service_task,
        label,
        "inParameter",
        &service_task.in_parameters,
    )?;
    validate_io_parameters(
        service_task,
        label,
        "outParameter",
        &service_task.out_parameters,
    )?;
    if service_task.parallel_in_same_transaction.is_some() {
        return Err(FlowableError::DeploymentValidationError(format!(
            "{} service task '{}' uses unsupported advanced runtime flags for the owned M14 subset",
            label,
            activity_id(service_task)
        )));
    }
    if service_task.triggerable && label != "Send event" {
        return Err(FlowableError::DeploymentValidationError(format!(
            "{} service task '{}' uses unsupported advanced runtime flags for the owned M14 subset",
            label,
            activity_id(service_task)
        )));
    }

    Ok(())
}

fn validate_io_parameters(
    service_task: &ServiceTask,
    label: &str,
    parameter_kind: &str,
    parameters: &[flowable_bpmn_model::model::IOParameter],
) -> Result<(), FlowableError> {
    for parameter in parameters {
        if parameter.target_expression.is_some() {
            return Err(FlowableError::DeploymentValidationError(format!(
                "{} service task '{}' only supports {} with literal target in the owned M14 subset",
                label,
                activity_id(service_task),
                parameter_kind
            )));
        }

        let target = parameter
            .target
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        if target.is_none() {
            return Err(FlowableError::DeploymentValidationError(format!(
                "{} service task '{}' requires {} target in the owned M14 subset",
                label,
                activity_id(service_task),
                parameter_kind
            )));
        }

        let source = parameter
            .source
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let source_expression = parameter
            .source_expression
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        if source.is_some() == source_expression.is_some() {
            return Err(FlowableError::DeploymentValidationError(format!(
                "{} service task '{}' requires {} to define exactly one of source or sourceExpression in the owned M14 subset",
                label,
                activity_id(service_task),
                parameter_kind
            )));
        }

        if let Some(source_expression) = source_expression
            && !(source_expression.starts_with("${") && source_expression.ends_with('}'))
        {
            return Err(FlowableError::DeploymentValidationError(format!(
                "{} service task '{}' only supports simple ${{...}} sourceExpression values for {} in the owned M14 subset",
                label,
                activity_id(service_task),
                parameter_kind
            )));
        }
    }

    Ok(())
}

fn validate_allowed_extensions(
    service_task: &ServiceTask,
    label: &str,
    allowed_extensions: &BTreeSet<&str>,
) -> Result<(), FlowableError> {
    let base = service_task_base_element(service_task);
    for extension_name in base.extension_elements.keys() {
        if !allowed_extensions.contains(extension_name.as_str()) {
            return Err(FlowableError::DeploymentValidationError(format!(
                "Task type '{}' is not supported in M9 unless it matches the owned M14 {} service task subset; {} service task '{}' uses unsupported extension '{}'",
                service_task
                    .task_type
                    .as_deref()
                    .unwrap_or(label)
                    .to_lowercase(),
                label,
                label,
                activity_id(service_task),
                extension_name
            )));
        }
    }
    Ok(())
}

fn validate_allowed_http_field_extensions(
    service_task: &ServiceTask,
    allowed_extensions: &BTreeSet<&str>,
) -> Result<(), FlowableError> {
    for field in &service_task.task.activity.field_extensions {
        if let Some(field_name) = field.field_name.as_deref()
            && !allowed_extensions.contains(field_name)
        {
            return Err(FlowableError::DeploymentValidationError(format!(
                "Task type 'http' is not supported in M9 unless it matches the owned M14 HTTP service task subset; HTTP service task '{}' uses unsupported field extension '{}'",
                activity_id(service_task),
                field_name
            )));
        }
    }
    Ok(())
}

fn require_literal_extension(
    service_task: &ServiceTask,
    label: &str,
    extension_name: &str,
) -> Result<String, FlowableError> {
    optional_literal_extension(service_task, label, extension_name).ok_or_else(|| {
        FlowableError::DeploymentValidationError(format!(
            "Task type '{}' is not supported in M9 unless it matches the owned M14 {} service task subset; {} service task '{}' requires extension '{}'",
            service_task.task_type.as_deref().unwrap_or(label).to_lowercase(),
            label,
            label,
            activity_id(service_task),
            extension_name
        ))
    })
}

/// Like [`require_literal_extension`] but permits `${...}` / `#{...}` EL (Mail Java parity).
fn require_expression_or_literal_extension(
    service_task: &ServiceTask,
    label: &str,
    extension_name: &str,
) -> Result<String, FlowableError> {
    optional_expression_or_literal_extension(service_task, label, extension_name).ok_or_else(|| {
        FlowableError::DeploymentValidationError(format!(
            "Task type '{}' is not supported in M9 unless it matches the owned M14 {} service task subset; {} service task '{}' requires extension '{}'",
            service_task.task_type.as_deref().unwrap_or(label).to_lowercase(),
            label,
            label,
            activity_id(service_task),
            extension_name
        ))
    })
}

fn optional_expression_or_literal_extension(
    service_task: &ServiceTask,
    _label: &str,
    extension_name: &str,
) -> Option<String> {
    let elements = &service_task_base_element(service_task).extension_elements;
    if let Some(elements) = elements.get(extension_name) {
        if elements.len() != 1 {
            return None;
        }
        let element = &elements[0];
        if !element.child_elements.is_empty() || !element.base_element.attributes.is_empty() {
            return None;
        }
        return element
            .element_text
            .as_ref()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
    }
    service_task
        .task
        .activity
        .field_extensions
        .iter()
        .find(|field| field.field_name.as_deref() == Some(extension_name))
        .and_then(|field| {
            field
                .string_value
                .clone()
                .or_else(|| field.expression.clone())
        })
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
}

fn optional_literal_extension(
    service_task: &ServiceTask,
    label: &str,
    extension_name: &str,
) -> Option<String> {
    let elements = &service_task_base_element(service_task).extension_elements;
    if let Some(elements) = elements.get(extension_name) {
        if elements.len() != 1 {
            return None;
        }
        let text =
            validate_literal_extension_element(service_task, label, extension_name, &elements[0])
                .ok()?;
        return Some(text);
    }
    service_task
        .task
        .activity
        .field_extensions
        .iter()
        .find(|field| field.field_name.as_deref() == Some(extension_name))
        .and_then(|field| {
            field
                .string_value
                .clone()
                .or_else(|| field.expression.clone())
        })
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
}

fn is_expression_text(value: &str) -> bool {
    let value = value.trim();
    value.starts_with("${") && value.ends_with('}')
}

fn validate_http_service_task_shape(service_task: &ServiceTask) -> Result<(), FlowableError> {
    if service_task.implementation.is_some() || service_task.implementation_type.is_some() {
        return Err(FlowableError::DeploymentValidationError(format!(
            "HTTP service task '{}' does not support service-task implementation delegates; use Java-compatible HTTP request/response handlers instead",
            activity_id(service_task)
        )));
    }
    validate_io_parameters(
        service_task,
        "HTTP",
        "inParameter",
        &service_task.in_parameters,
    )?;
    validate_io_parameters(
        service_task,
        "HTTP",
        "outParameter",
        &service_task.out_parameters,
    )?;
    if service_task.triggerable {
        return Err(FlowableError::DeploymentValidationError(format!(
            "HTTP service task '{}' does not support triggerable=true",
            activity_id(service_task)
        )));
    }
    Ok(())
}

fn validate_literal_extension_element(
    service_task: &ServiceTask,
    label: &str,
    extension_name: &str,
    element: &ExtensionElement,
) -> Result<String, FlowableError> {
    if !element.child_elements.is_empty() || !element.base_element.attributes.is_empty() {
        return Err(FlowableError::DeploymentValidationError(format!(
            "{} service task '{}' requires flat literal extension '{}' in the owned M14 subset",
            label,
            activity_id(service_task),
            extension_name
        )));
    }

    let text = element
        .element_text
        .as_ref()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            FlowableError::DeploymentValidationError(format!(
                "{} service task '{}' requires non-empty extension '{}'",
                label,
                activity_id(service_task),
                extension_name
            ))
        })?;

    if text.contains("${") || text.contains("#{") {
        return Err(FlowableError::DeploymentValidationError(format!(
            "{} service task '{}' does not support expression-based extension '{}' in the owned M14 subset",
            label,
            activity_id(service_task),
            extension_name
        )));
    }

    Ok(text)
}

fn service_task_base_element(service_task: &ServiceTask) -> &BaseElement {
    &service_task
        .task
        .activity
        .flow_node
        .flow_element
        .base_element
}

fn activity_id(service_task: &ServiceTask) -> String {
    service_task_base_element(service_task)
        .id
        .clone()
        .unwrap_or_default()
}
