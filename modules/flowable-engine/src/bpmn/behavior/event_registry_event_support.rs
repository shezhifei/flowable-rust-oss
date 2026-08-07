//! Event Registry wait-state helpers.
//!
//! Java sources:
//! - `BpmnXMLConstants.ELEMENT_EVENT_TYPE` = `"eventType"`
//! - `BpmnXMLConstants.START_EVENT_CORRELATION_CONFIGURATION` /
//!   `START_EVENT_CORRELATION_MANUAL` (`EventSubscriptionManager.java:226-231`)
//! - Extension element read sites:
//!   `StartEventParseHandler.java:77,120`,
//!   `BoundaryEventParseHandler.java:76`,
//!   `IntermediateCatchEventParseHandler.java:57`,
//!   `ReceiveTaskParseHandler.java:41`,
//!   `ProcessInstanceHelper.java:371-398`

use flowable_bpmn_model::model::BaseElement;

/// Flowable extension element name for event-registry event type
/// (`BpmnXMLConstants.ELEMENT_EVENT_TYPE`).
pub(crate) const ELEMENT_EVENT_TYPE: &str = "eventType";

/// Start-event correlation configuration extension
/// (`BpmnXMLConstants.START_EVENT_CORRELATION_CONFIGURATION`).
pub(crate) const START_EVENT_CORRELATION_CONFIGURATION: &str = "startEventCorrelationConfiguration";

/// Manual subscription value
/// (`BpmnXMLConstants.START_EVENT_CORRELATION_MANUAL`).
pub(crate) const START_EVENT_CORRELATION_MANUAL: &str = "manualSubscription";

/// Reads `flowable:eventType` extension element text from a base element.
///
/// Java: `element.getExtensionElements().get("eventType").get(0).getElementText()`.
pub(crate) fn resolve_event_type_extension(base_element: &BaseElement) -> Option<String> {
    let elements = base_element.extension_elements.get(ELEMENT_EVENT_TYPE)?;
    let text = elements.first()?.element_text.as_deref()?.trim();
    if text.is_empty() {
        None
    } else {
        Some(text.to_string())
    }
}

/// Returns true when start-event correlation is `manualSubscription`, which
/// skips deploy-time event-registry start subscription registration
/// (Java `EventSubscriptionManager.insertEventRegistryEvent:226-231`).
pub(crate) fn is_manual_event_registry_start_correlation(base_element: &BaseElement) -> bool {
    base_element
        .extension_elements
        .get(START_EVENT_CORRELATION_CONFIGURATION)
        .and_then(|elements| elements.first())
        .and_then(|element| element.element_text.as_deref())
        .map(|text| text.trim() == START_EVENT_CORRELATION_MANUAL)
        .unwrap_or(false)
}
