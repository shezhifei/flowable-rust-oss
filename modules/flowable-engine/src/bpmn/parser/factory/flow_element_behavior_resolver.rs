use crate::delegate::activity_behavior::ActivityBehavior;
use flowable_bpmn_model::model::FlowElementEnum;

pub trait FlowElementBehaviorResolver {
    fn resolve_behavior(&self, flow_element: &FlowElementEnum)
    -> Option<Box<dyn ActivityBehavior>>;
}
