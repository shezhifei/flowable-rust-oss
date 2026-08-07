use flowable_engine_common::el::VariableContainer;

pub trait Condition {
    fn evaluate(
        &self,
        element_id: Option<&str>,
        scope: &dyn VariableContainer,
    ) -> Result<bool, crate::error::FlowableError>;
}
