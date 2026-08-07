use flowable_bpmn_model::model::{BpmnModel, ErrorEventDefinition};

pub(crate) fn resolve_error_event_ref(
    error_definition: &ErrorEventDefinition,
    model: Option<&BpmnModel>,
) -> String {
    error_definition
        .error_code
        .clone()
        .or_else(|| {
            error_definition
                .error_ref
                .as_deref()
                .and_then(|error_ref| model.and_then(|model| model.errors.get(error_ref).cloned()))
        })
        .or_else(|| error_definition.error_ref.clone())
        .unwrap_or_default()
}
