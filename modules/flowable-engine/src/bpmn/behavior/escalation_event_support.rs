use flowable_bpmn_model::model::{BpmnModel, EscalationEventDefinition};

/// Resolves the effective escalation reference for an `EscalationEventDefinition`.
///
/// Resolution priority:
/// 1. `escalation_code` directly on the event definition
/// 2. Look up the `escalation_ref` in the model's `escalations` list and use its `escalation_code`
/// 3. Fall back to the raw `escalation_ref` string
/// 4. Empty string if nothing is set
pub(crate) fn resolve_escalation_event_ref(
    escalation_definition: &EscalationEventDefinition,
    model: Option<&BpmnModel>,
) -> String {
    // Prefer the inline escalation_code if present
    if let Some(code) = &escalation_definition.escalation_code {
        return code.clone();
    }

    // Try to resolve via escalation_ref -> model.escalations
    if let Some(escalation_ref) = &escalation_definition.escalation_ref {
        if let Some(model) = model
            && let Some(escalation) = model
                .escalations
                .iter()
                .find(|e| e.base_element.id.as_deref() == Some(escalation_ref))
        {
            return escalation
                .escalation_code
                .clone()
                .unwrap_or_else(|| escalation_ref.clone());
        }
        return escalation_ref.clone();
    }

    String::new()
}
