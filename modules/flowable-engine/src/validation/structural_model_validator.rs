use crate::error::FlowableError;
use flowable_bpmn_model::model::{
    BaseElement, BpmnModel, EventDefinitionEnum, FlowElementEnum, Process, SequenceFlow,
};
use std::collections::HashSet;

const SEQ_FLOW_INVALID_SRC: &str = "flowable-seq-flow-invalid-src";
const SEQ_FLOW_INVALID_TARGET: &str = "flowable-seq-flow-invalid-target";
const EXCLUSIVE_GATEWAY_NO_OUTGOING: &str =
    "flowable-exclusive-gateway-no-outgoing-seq-flow";
const EXCLUSIVE_GATEWAY_SINGLE_CONDITION: &str =
    "flowable-exclusive-gateway-condition-not-allowed-on-single-seq-flow";
const EXCLUSIVE_GATEWAY_DEFAULT_CONDITION: &str =
    "flowable-exclusive-gateway-condition-on-seq-flow";
const COMPLEX_GATEWAY_UNSUPPORTED: &str = "flowable-complex-gateway-not-supported";
const START_EVENT_MULTIPLE_FOUND: &str = "flowable-start-event-multiple-found";
const START_EVENT_INVALID_DEFINITION: &str = "flowable-start-event-invalid-event-definition";
const SUBPROCESS_MULTIPLE_START_EVENTS: &str = "flowable-subprocess-multiple-start-event";
const SUBPROCESS_START_EVENT_DEFINITION: &str =
    "flowable-subprocess-start-event-event-definition-not-allowed";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StructuralValidationError {
    pub problem_code: &'static str,
    pub process_id: Option<String>,
    pub element_id: Option<String>,
    pub message: String,
}

impl StructuralValidationError {
    fn new(
        problem_code: &'static str,
        process_id: Option<&str>,
        element_id: Option<&str>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            problem_code,
            process_id: process_id.map(str::to_string),
            element_id: element_id.map(str::to_string),
            message: message.into(),
        }
    }

    fn format(&self) -> String {
        let mut location = Vec::new();
        if let Some(process_id) = &self.process_id {
            location.push(format!("process '{process_id}'"));
        }
        if let Some(element_id) = &self.element_id {
            location.push(format!("element '{element_id}'"));
        }
        if location.is_empty() {
            format!("[{}] {}", self.problem_code, self.message)
        } else {
            format!(
                "[{}] {}: {}",
                self.problem_code,
                location.join(", "),
                self.message
            )
        }
    }
}

pub struct StructuralModelValidator;

impl StructuralModelValidator {
    pub fn validate(model: &BpmnModel) -> Result<(), FlowableError> {
        let errors = Self::validate_all(model);
        if errors.is_empty() {
            return Ok(());
        }
        Err(FlowableError::DeploymentValidationError(
            errors
                .iter()
                .map(StructuralValidationError::format)
                .collect::<Vec<_>>()
                .join("; "),
        ))
    }

    pub fn validate_all(model: &BpmnModel) -> Vec<StructuralValidationError> {
        let mut errors = Vec::new();
        for process in &model.processes {
            validate_process(process, &mut errors);
        }
        errors
    }
}

fn validate_process(process: &Process, errors: &mut Vec<StructuralValidationError>) {
    let process_id = process.base_element.id.as_deref();
    validate_container(&process.flow_elements, process_id, errors);
    validate_process_start_events(&process.flow_elements, process_id, errors);
}

fn validate_container(
    elements: &[FlowElementEnum],
    process_id: Option<&str>,
    errors: &mut Vec<StructuralValidationError>,
) {
    let element_ids = elements
        .iter()
        .filter_map(flow_element_base)
        .filter_map(|base| base.id.as_deref())
        .collect::<HashSet<_>>();

    for element in elements {
        match element {
            FlowElementEnum::SequenceFlow(sequence_flow) => {
                validate_sequence_flow(sequence_flow, &element_ids, process_id, errors);
            }
            FlowElementEnum::ComplexGateway(gateway) => {
                errors.push(StructuralValidationError::new(
                    COMPLEX_GATEWAY_UNSUPPORTED,
                    process_id,
                    gateway
                        .gateway
                        .flow_node
                        .flow_element
                        .base_element
                        .id
                        .as_deref(),
                    "Complex gateway execution is not supported by this engine",
                ));
            }
            FlowElementEnum::ExclusiveGateway(gateway) => {
                let gateway_id = gateway
                    .gateway
                    .flow_node
                    .flow_element
                    .base_element
                    .id
                    .as_deref();
                let outgoing = elements
                    .iter()
                    .filter_map(|element| match element {
                        FlowElementEnum::SequenceFlow(flow)
                            if flow.source_ref.as_deref() == gateway_id =>
                        {
                            Some(flow)
                        }
                        _ => None,
                    })
                    .collect::<Vec<_>>();
                validate_exclusive_gateway(
                    gateway_id,
                    gateway.gateway.default_flow.as_deref(),
                    &outgoing,
                    process_id,
                    errors,
                );
            }
            FlowElementEnum::SubProcess(sub_process) => {
                validate_standard_subprocess(
                    &sub_process.flow_elements,
                    process_id,
                    sub_process
                        .activity
                        .flow_node
                        .flow_element
                        .base_element
                        .id
                        .as_deref(),
                    errors,
                );
            }
            FlowElementEnum::Transaction(transaction) => {
                let sub_process = &transaction.sub_process;
                validate_standard_subprocess(
                    &sub_process.flow_elements,
                    process_id,
                    sub_process
                        .activity
                        .flow_node
                        .flow_element
                        .base_element
                        .id
                        .as_deref(),
                    errors,
                );
            }
            FlowElementEnum::AdhocSubProcess(adhoc) => {
                let sub_process = &adhoc.sub_process;
                validate_standard_subprocess(
                    &sub_process.flow_elements,
                    process_id,
                    sub_process
                        .activity
                        .flow_node
                        .flow_element
                        .base_element
                        .id
                        .as_deref(),
                    errors,
                );
            }
            FlowElementEnum::EventSubProcess(event_subprocess) => {
                validate_container(
                    &event_subprocess.sub_process.flow_elements,
                    process_id,
                    errors,
                );
            }
            _ => {}
        }
    }
}

fn validate_sequence_flow(
    flow: &SequenceFlow,
    element_ids: &HashSet<&str>,
    process_id: Option<&str>,
    errors: &mut Vec<StructuralValidationError>,
) {
    let element_id = flow.flow_element.base_element.id.as_deref();
    let source_valid = flow
        .source_ref
        .as_deref()
        .filter(|reference| !reference.is_empty())
        .is_some_and(|reference| element_ids.contains(reference));
    if !source_valid {
        errors.push(StructuralValidationError::new(
            SEQ_FLOW_INVALID_SRC,
            process_id,
            element_id,
            "Invalid source for sequenceflow",
        ));
    }

    let target_valid = flow
        .target_ref
        .as_deref()
        .filter(|reference| !reference.is_empty())
        .is_some_and(|reference| element_ids.contains(reference));
    if !target_valid {
        errors.push(StructuralValidationError::new(
            SEQ_FLOW_INVALID_TARGET,
            process_id,
            element_id,
            "Invalid target for sequenceflow or target is not defined in the same scope",
        ));
    }
}

fn validate_exclusive_gateway(
    gateway_id: Option<&str>,
    default_flow: Option<&str>,
    outgoing: &[&SequenceFlow],
    process_id: Option<&str>,
    errors: &mut Vec<StructuralValidationError>,
) {
    if outgoing.is_empty() {
        errors.push(StructuralValidationError::new(
            EXCLUSIVE_GATEWAY_NO_OUTGOING,
            process_id,
            gateway_id,
            "Exclusive gateway has no outgoing sequence flow",
        ));
        return;
    }
    if outgoing.len() == 1 {
        if has_text(outgoing[0].condition_expression.as_deref()) {
            errors.push(StructuralValidationError::new(
                EXCLUSIVE_GATEWAY_SINGLE_CONDITION,
                process_id,
                gateway_id,
                "Exclusive gateway has only one outgoing sequence flow, which cannot have a condition",
            ));
        }
        return;
    }

    for flow in outgoing {
        let is_default = flow.flow_element.base_element.id.as_deref() == default_flow;
        if is_default && has_text(flow.condition_expression.as_deref()) {
            errors.push(StructuralValidationError::new(
                EXCLUSIVE_GATEWAY_DEFAULT_CONDITION,
                process_id,
                gateway_id,
                "Default sequenceflow has a condition, which is not allowed",
            ));
        }
    }
}

fn validate_process_start_events(
    elements: &[FlowElementEnum],
    process_id: Option<&str>,
    errors: &mut Vec<StructuralValidationError>,
) {
    let starts = elements
        .iter()
        .filter_map(|element| match element {
            FlowElementEnum::StartEvent(start) => Some(start),
            _ => None,
        })
        .collect::<Vec<_>>();

    let none_starts = starts
        .iter()
        .filter(|start| {
            start.event.event_definitions.is_empty()
                && !start
                    .event
                    .flow_node
                    .flow_element
                    .base_element
                    .extension_elements
                    .contains_key("eventType")
        })
        .collect::<Vec<_>>();
    if none_starts.len() > 1 {
        for start in none_starts {
            errors.push(StructuralValidationError::new(
                START_EVENT_MULTIPLE_FOUND,
                process_id,
                start
                    .event
                    .flow_node
                    .flow_element
                    .base_element
                    .id
                    .as_deref(),
                "Multiple none start events are not supported",
            ));
        }
    }

    for start in starts {
        if start.event.event_definitions.first().is_some_and(|definition| {
            !matches!(
                definition,
                EventDefinitionEnum::MessageEventDefinition(_)
                    | EventDefinitionEnum::TimerEventDefinition(_)
                    | EventDefinitionEnum::SignalEventDefinition(_)
            )
        }) {
            errors.push(StructuralValidationError::new(
                START_EVENT_INVALID_DEFINITION,
                process_id,
                start
                    .event
                    .flow_node
                    .flow_element
                    .base_element
                    .id
                    .as_deref(),
                "Unsupported event definition on start event",
            ));
        }
    }
}

fn validate_standard_subprocess(
    elements: &[FlowElementEnum],
    process_id: Option<&str>,
    subprocess_id: Option<&str>,
    errors: &mut Vec<StructuralValidationError>,
) {
    validate_container(elements, process_id, errors);
    let starts = elements
        .iter()
        .filter_map(|element| match element {
            FlowElementEnum::StartEvent(start) => Some(start),
            _ => None,
        })
        .collect::<Vec<_>>();
    if starts.len() > 1 {
        errors.push(StructuralValidationError::new(
            SUBPROCESS_MULTIPLE_START_EVENTS,
            process_id,
            subprocess_id,
            "Multiple start events are not supported for subprocess",
        ));
    }
    for start in starts {
        if !start.event.event_definitions.is_empty() {
            errors.push(StructuralValidationError::new(
                SUBPROCESS_START_EVENT_DEFINITION,
                process_id,
                start
                    .event
                    .flow_node
                    .flow_element
                    .base_element
                    .id
                    .as_deref(),
                "Event definitions are only allowed on an event subprocess start event",
            ));
        }
    }
}

fn has_text(value: Option<&str>) -> bool {
    value.is_some_and(|value| !value.trim().is_empty())
}

fn flow_element_base(element: &FlowElementEnum) -> Option<&BaseElement> {
    Some(match element {
        FlowElementEnum::SequenceFlow(value) => &value.flow_element.base_element,
        FlowElementEnum::Task(value) => &value.activity.flow_node.flow_element.base_element,
        FlowElementEnum::UserTask(value) => {
            &value.task.activity.flow_node.flow_element.base_element
        }
        FlowElementEnum::ServiceTask(value) => {
            &value.task.activity.flow_node.flow_element.base_element
        }
        FlowElementEnum::CaseServiceTask(value) => {
            &value.service_task.task.activity.flow_node.flow_element.base_element
        }
        FlowElementEnum::SendTask(value) => {
            &value.service_task.task.activity.flow_node.flow_element.base_element
        }
        FlowElementEnum::ScriptTask(value) => {
            &value.task.activity.flow_node.flow_element.base_element
        }
        FlowElementEnum::ManualTask(value) => {
            &value.task.activity.flow_node.flow_element.base_element
        }
        FlowElementEnum::ReceiveTask(value) => {
            &value.task.activity.flow_node.flow_element.base_element
        }
        FlowElementEnum::BusinessRuleTask(value) => {
            &value.task.activity.flow_node.flow_element.base_element
        }
        FlowElementEnum::StartEvent(value) => &value.event.flow_node.flow_element.base_element,
        FlowElementEnum::EndEvent(value) => &value.event.flow_node.flow_element.base_element,
        FlowElementEnum::ExclusiveGateway(value) => {
            &value.gateway.flow_node.flow_element.base_element
        }
        FlowElementEnum::ParallelGateway(value) => {
            &value.gateway.flow_node.flow_element.base_element
        }
        FlowElementEnum::InclusiveGateway(value) => {
            &value.gateway.flow_node.flow_element.base_element
        }
        FlowElementEnum::EventBasedGateway(value) => {
            &value.gateway.flow_node.flow_element.base_element
        }
        FlowElementEnum::ComplexGateway(value) => {
            &value.gateway.flow_node.flow_element.base_element
        }
        FlowElementEnum::IntermediateCatchEvent(value) => {
            &value.event.flow_node.flow_element.base_element
        }
        FlowElementEnum::IntermediateThrowEvent(value) => {
            &value.event.flow_node.flow_element.base_element
        }
        FlowElementEnum::SubProcess(value) => {
            &value.activity.flow_node.flow_element.base_element
        }
        FlowElementEnum::Transaction(value) => {
            &value.sub_process.activity.flow_node.flow_element.base_element
        }
        FlowElementEnum::EventSubProcess(value) => {
            &value.sub_process.activity.flow_node.flow_element.base_element
        }
        FlowElementEnum::AdhocSubProcess(value) => {
            &value.sub_process.activity.flow_node.flow_element.base_element
        }
        FlowElementEnum::CallActivity(value) => &value.activity.flow_node.flow_element.base_element,
        FlowElementEnum::ValuedDataObject(value) => &value.base_element,
        FlowElementEnum::BoundaryEvent(value) => &value.event.flow_node.flow_element.base_element,
    })
}
