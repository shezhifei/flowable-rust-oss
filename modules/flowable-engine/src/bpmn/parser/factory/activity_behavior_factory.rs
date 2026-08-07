use crate::bpmn::behavior::adhoc_subprocess_activity_behavior::AdhocSubProcessActivityBehavior;
use crate::bpmn::behavior::boundary_event_activity_behavior::BoundaryEventActivityBehavior;
use crate::bpmn::behavior::business_rule_task_activity_behavior::BusinessRuleTaskActivityBehavior;
use crate::bpmn::behavior::call_activity_behavior::CallActivityBehavior;
use crate::bpmn::behavior::cancel_end_event_activity_behavior::CancelEndEventActivityBehavior;
use crate::bpmn::behavior::end_event_activity_behavior::EndEventActivityBehavior;
use crate::bpmn::behavior::event_based_gateway_activity_behavior::EventBasedGatewayActivityBehavior;
use crate::bpmn::behavior::event_subprocess_activity_behavior::EventSubprocessActivityBehavior;
use crate::bpmn::behavior::exclusive_gateway_activity_behavior::ExclusiveGatewayActivityBehavior;
use crate::bpmn::behavior::inclusive_gateway_activity_behavior::InclusiveGatewayActivityBehavior;
use crate::bpmn::behavior::intermediate_catch_event_activity_behavior::IntermediateCatchEventActivityBehavior;
use crate::bpmn::behavior::intermediate_throw_event_activity_behavior::IntermediateThrowEventActivityBehavior;
use crate::bpmn::behavior::manual_task_activity_behavior::ManualTaskActivityBehavior;
use crate::bpmn::behavior::multi_instance_support::MultiInstanceActivityBehavior;
use crate::bpmn::behavior::parallel_gateway_activity_behavior::ParallelGatewayActivityBehavior;
use crate::bpmn::behavior::receive_task_activity_behavior::ReceiveTaskActivityBehavior;
use crate::bpmn::behavior::script_task_activity_behavior::ScriptTaskActivityBehavior;
use crate::bpmn::behavior::send_task_activity_behavior::SendTaskActivityBehavior;
use crate::bpmn::behavior::service_task_activity_behavior::ServiceTaskActivityBehavior;
use crate::bpmn::behavior::start_event_activity_behavior::StartEventActivityBehavior;
use crate::bpmn::behavior::sub_process_activity_behavior::SubProcessActivityBehavior;
use crate::bpmn::behavior::task_activity_behavior::TaskActivityBehavior;
use crate::bpmn::behavior::transaction_activity_behavior::TransactionActivityBehavior;
use crate::bpmn::behavior::unsupported_activity_behavior::UnsupportedActivityBehavior;
use crate::bpmn::behavior::user_task_activity_behavior::UserTaskActivityBehavior;
use crate::bpmn::behavior::variable_listener_event_behavior::VariableListenerEventBehavior;
use crate::bpmn::parser::factory::flow_element_behavior_resolver::FlowElementBehaviorResolver;
use crate::delegate::activity_behavior::ActivityBehavior;
use flowable_bpmn_model::model::{BoundaryEvent, EventDefinitionEnum, FlowElementEnum};

fn is_wait_state_intermediate_catch_event(flow_element: &FlowElementEnum) -> bool {
    match flow_element {
        FlowElementEnum::IntermediateCatchEvent(event) => {
            if event.event.event_definitions.is_empty() {
                return true;
            }

            matches!(
                event.event.event_definitions.as_slice(),
                [EventDefinitionEnum::MessageEventDefinition(_)]
            ) || matches!(
                event.event.event_definitions.as_slice(),
                [EventDefinitionEnum::SignalEventDefinition(_)]
            ) || matches!(
                event.event.event_definitions.as_slice(),
                [EventDefinitionEnum::TimerEventDefinition(_)]
            ) || matches!(
                event.event.event_definitions.as_slice(),
                [EventDefinitionEnum::ConditionalEventDefinition(_)]
            ) || matches!(
                event.event.event_definitions.as_slice(),
                [EventDefinitionEnum::LinkEventDefinition(_)]
            )
        }
        _ => false,
    }
}

fn is_supported_intermediate_throw_event(flow_element: &FlowElementEnum) -> bool {
    match flow_element {
        FlowElementEnum::IntermediateThrowEvent(event) => {
            if event.event.event_definitions.is_empty() {
                return true;
            }
            matches!(
                event.event.event_definitions.as_slice(),
                [EventDefinitionEnum::LinkEventDefinition(_)]
            ) || matches!(
                event.event.event_definitions.as_slice(),
                [EventDefinitionEnum::MessageEventDefinition(_)]
            ) || matches!(
                event.event.event_definitions.as_slice(),
                [EventDefinitionEnum::SignalEventDefinition(_)]
            ) || matches!(
                event.event.event_definitions.as_slice(),
                [EventDefinitionEnum::CompensateEventDefinition(_)]
            ) || matches!(
                event.event.event_definitions.as_slice(),
                [EventDefinitionEnum::EscalationEventDefinition(_)]
            )
        }
        _ => false,
    }
}

fn is_supported_boundary_event(boundary_event: &BoundaryEvent) -> bool {
    // Java BoundaryEventParseHandler.java:76 — empty defs + eventType is supported.
    if boundary_event.event.event_definitions.is_empty() {
        return crate::bpmn::behavior::event_registry_event_support::resolve_event_type_extension(
            &boundary_event.event.flow_node.flow_element.base_element,
        )
        .is_some();
    }
    matches!(
        boundary_event.event.event_definitions.as_slice(),
        [EventDefinitionEnum::MessageEventDefinition(_)]
    ) || matches!(
        boundary_event.event.event_definitions.as_slice(),
        [EventDefinitionEnum::SignalEventDefinition(_)]
    ) || matches!(
        boundary_event.event.event_definitions.as_slice(),
        [EventDefinitionEnum::TimerEventDefinition(_)]
    ) || matches!(
        boundary_event.event.event_definitions.as_slice(),
        [EventDefinitionEnum::ConditionalEventDefinition(_)]
    ) || matches!(
        boundary_event.event.event_definitions.as_slice(),
        [EventDefinitionEnum::ErrorEventDefinition(_)]
    ) || matches!(
        boundary_event.event.event_definitions.as_slice(),
        [EventDefinitionEnum::CancelEventDefinition(_)]
    ) || matches!(
        boundary_event.event.event_definitions.as_slice(),
        [EventDefinitionEnum::CompensateEventDefinition(_)]
    ) || matches!(
        boundary_event.event.event_definitions.as_slice(),
        [EventDefinitionEnum::EscalationEventDefinition(_)]
    )
}

pub trait ActivityBehaviorFactory {
    fn create_behavior(&self, flow_element: &FlowElementEnum) -> Option<Box<dyn ActivityBehavior>>;
}

pub struct DefaultActivityBehaviorFactory;

impl Default for DefaultActivityBehaviorFactory {
    fn default() -> Self {
        Self::new()
    }
}

impl DefaultActivityBehaviorFactory {
    pub fn new() -> Self {
        Self
    }
}

impl FlowElementBehaviorResolver for DefaultActivityBehaviorFactory {
    fn resolve_behavior(
        &self,
        flow_element: &FlowElementEnum,
    ) -> Option<Box<dyn ActivityBehavior>> {
        let behavior: Option<Box<dyn ActivityBehavior>> = match flow_element {
            FlowElementEnum::StartEvent(start_event) => {
                // Prefer VariableListener-specific behavior when the start event
                // carries that definition (event-subprocess start).
                let vl = start_event.event.event_definitions.iter().find_map(|d| {
                    if let EventDefinitionEnum::VariableListenerEventDefinition(def) = d {
                        Some(VariableListenerEventBehavior::from_definition(def))
                    } else {
                        None
                    }
                });
                if let Some(vl) = vl {
                    Some(Box::new(vl))
                } else {
                    Some(Box::new(StartEventActivityBehavior::new()))
                }
            }
            FlowElementEnum::EndEvent(end_event) => {
                if !end_event.event.event_definitions.is_empty()
                    && matches!(
                        end_event.event.event_definitions[0],
                        EventDefinitionEnum::CancelEventDefinition(_)
                    )
                {
                    Some(Box::new(CancelEndEventActivityBehavior::new()))
                } else {
                    Some(Box::new(EndEventActivityBehavior::new()))
                }
            }
            FlowElementEnum::ExclusiveGateway(_) => {
                Some(Box::new(ExclusiveGatewayActivityBehavior::new()))
            }
            FlowElementEnum::InclusiveGateway(_) => {
                Some(Box::new(InclusiveGatewayActivityBehavior::new()))
            }
            FlowElementEnum::ParallelGateway(_) => {
                Some(Box::new(ParallelGatewayActivityBehavior::new()))
            }
            FlowElementEnum::EventBasedGateway(_) => {
                Some(Box::new(EventBasedGatewayActivityBehavior::new()))
            }
            FlowElementEnum::UserTask(_) => Some(Box::new(UserTaskActivityBehavior::new())),
            FlowElementEnum::ReceiveTask(_) => Some(Box::new(ReceiveTaskActivityBehavior::new())),
            FlowElementEnum::BusinessRuleTask(_) => {
                Some(Box::new(BusinessRuleTaskActivityBehavior::new()))
            }
            FlowElementEnum::IntermediateCatchEvent(_)
                if is_wait_state_intermediate_catch_event(flow_element) =>
            {
                Some(Box::new(IntermediateCatchEventActivityBehavior::new()))
            }
            FlowElementEnum::IntermediateThrowEvent(_)
                if is_supported_intermediate_throw_event(flow_element) =>
            {
                Some(Box::new(IntermediateThrowEventActivityBehavior::new()))
            }
            FlowElementEnum::ServiceTask(_) => Some(Box::new(ServiceTaskActivityBehavior::new())),
            // Java CaseServiceTaskParseHandler.java:30-31 → CaseTaskActivityBehavior
            FlowElementEnum::CaseServiceTask(_) => Some(Box::new(
                crate::bpmn::behavior::case_task_activity_behavior::CaseTaskActivityBehavior::new(),
            )),
            // Java SendTaskParseHandler.java:37-56 — the behavior dispatches on
            // `type` (mail / dmn) and otherwise passes through.
            FlowElementEnum::SendTask(_) => Some(Box::new(SendTaskActivityBehavior::new())),
            FlowElementEnum::ScriptTask(_) => Some(Box::new(ScriptTaskActivityBehavior::new())),
            FlowElementEnum::ManualTask(_) => Some(Box::new(ManualTaskActivityBehavior::new())),
            FlowElementEnum::Task(_) => Some(Box::new(TaskActivityBehavior::new())),
            FlowElementEnum::SubProcess(_) => Some(Box::new(SubProcessActivityBehavior::new())),
            FlowElementEnum::EventSubProcess(_) => {
                Some(Box::new(EventSubprocessActivityBehavior::new()))
            }
            FlowElementEnum::Transaction(_) => Some(Box::new(TransactionActivityBehavior::new())),
            FlowElementEnum::AdhocSubProcess(_) => {
                Some(Box::new(AdhocSubProcessActivityBehavior::new()))
            }
            FlowElementEnum::BoundaryEvent(boundary_event)
                if is_supported_boundary_event(boundary_event) =>
            {
                Some(Box::new(BoundaryEventActivityBehavior::new()))
            }
            FlowElementEnum::CallActivity(_) => Some(Box::new(CallActivityBehavior::new())),
            _ => {
                let debug_str = format!("{:?}", flow_element);
                let element_type = debug_str.split('(').next().unwrap_or("Unknown").to_string();
                Some(Box::new(UnsupportedActivityBehavior::new(element_type)))
            }
        };

        if let Some(inner_behavior) = behavior {
            let mi_characteristics = match flow_element {
                FlowElementEnum::Task(t) => t.activity.loop_characteristics.clone(),
                FlowElementEnum::UserTask(t) => t.task.activity.loop_characteristics.clone(),
                FlowElementEnum::ServiceTask(t) => t.task.activity.loop_characteristics.clone(),
                FlowElementEnum::CaseServiceTask(t) => t.service_task.task.activity.loop_characteristics.clone(),
                FlowElementEnum::SendTask(t) => t.service_task.task.activity.loop_characteristics.clone(),
                FlowElementEnum::ScriptTask(t) => t.task.activity.loop_characteristics.clone(),
                FlowElementEnum::ManualTask(t) => t.task.activity.loop_characteristics.clone(),
                FlowElementEnum::ReceiveTask(t) => t.task.activity.loop_characteristics.clone(),
                FlowElementEnum::BusinessRuleTask(t) => {
                    t.task.activity.loop_characteristics.clone()
                }
                FlowElementEnum::SubProcess(t) => t.activity.loop_characteristics.clone(),
                FlowElementEnum::Transaction(t) => {
                    t.sub_process.activity.loop_characteristics.clone()
                }
                FlowElementEnum::AdhocSubProcess(t) => {
                    t.sub_process.activity.loop_characteristics.clone()
                }
                FlowElementEnum::CallActivity(t) => t.activity.loop_characteristics.clone(),
                _ => None,
            };

            if let Some(mi) = mi_characteristics {
                // Java `SequentialMultiInstanceBehavior#continueSequentialMultiInstance`
                // treats SubProcess (and transaction / ad-hoc wrappers) as scope
                // rows that are recreated each sequential round.
                let inner_is_subprocess = matches!(
                    flow_element,
                    FlowElementEnum::SubProcess(_)
                        | FlowElementEnum::Transaction(_)
                        | FlowElementEnum::AdhocSubProcess(_)
                );
                return Some(Box::new(MultiInstanceActivityBehavior::new(
                    inner_behavior,
                    mi,
                    inner_is_subprocess,
                )));
            }
            return Some(inner_behavior);
        }

        None
    }
}

impl<T> ActivityBehaviorFactory for T
where
    T: FlowElementBehaviorResolver,
{
    fn create_behavior(&self, flow_element: &FlowElementEnum) -> Option<Box<dyn ActivityBehavior>> {
        self.resolve_behavior(flow_element)
    }
}
