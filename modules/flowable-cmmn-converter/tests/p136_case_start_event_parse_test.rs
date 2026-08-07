//! P136 1a: case-level event-registry start subscription extensions.
//!
//! Java:
//! - ExtensionElementsXMLConverter.java:396-411 — `eventType` text → Case.startEventType
//! - CmmnXmlConstants.java:224-230 — eventType / eventCorrelationParameter /
//!   startEventCorrelationConfiguration (`storeAsUniqueReferenceId` / `manualSubscription`)
//! - CmmnCorrelationUtil.java:29-46 — eventCorrelationParameter name/value, static (no expr)

use flowable_cmmn_converter::parse_cmmn_definitions;
use flowable_cmmn_model::EventCorrelationParameter;

fn parse_case(case_body: &str) -> flowable_cmmn_model::Case {
    let xml = format!(
        r#"<definitions xmlns="http://www.omg.org/spec/CMMN/20151109/MODEL"
             xmlns:flowable="http://flowable.org/cmmn"
             targetNamespace="http://flowable.org/cmmn">
  <case id="startCase">{case_body}</case>
</definitions>"#
    );
    let definitions = parse_cmmn_definitions(&xml).expect("parse");
    definitions.cases.into_iter().next().expect("case")
}

#[test]
fn parses_start_event_type_on_case() {
    let case = parse_case(
        r#"
      <extensionElements>
        <flowable:eventType>myStartEvent</flowable:eventType>
      </extensionElements>
      <casePlanModel id="planModelA" />
    "#,
    );
    assert_eq!(case.start_event_type.as_deref(), Some("myStartEvent"));
    assert!(case.start_correlation_configuration.is_none());
    assert!(case.start_correlation_parameters.is_empty());
}

#[test]
fn parses_store_as_unique_reference_id_configuration() {
    let case = parse_case(
        r#"
      <extensionElements>
        <flowable:eventType>orderEvent</flowable:eventType>
        <flowable:startEventCorrelationConfiguration>storeAsUniqueReferenceId</flowable:startEventCorrelationConfiguration>
        <flowable:eventCorrelationParameter name="orderId" value="static-order" />
      </extensionElements>
      <casePlanModel id="planModelA" />
    "#,
    );
    assert_eq!(case.start_event_type.as_deref(), Some("orderEvent"));
    assert_eq!(
        case.start_correlation_configuration.as_deref(),
        Some("storeAsUniqueReferenceId")
    );
    assert_eq!(
        case.start_correlation_parameters,
        vec![EventCorrelationParameter::new("orderId", "static-order")]
    );
}

#[test]
fn parses_manual_subscription_configuration() {
    let case = parse_case(
        r#"
      <extensionElements>
        <flowable:eventType>manualEvent</flowable:eventType>
        <flowable:startEventCorrelationConfiguration>manualSubscription</flowable:startEventCorrelationConfiguration>
      </extensionElements>
      <casePlanModel id="planModelA" />
    "#,
    );
    assert_eq!(case.start_event_type.as_deref(), Some("manualEvent"));
    assert_eq!(
        case.start_correlation_configuration.as_deref(),
        Some("manualSubscription")
    );
}

#[test]
fn parses_multiple_static_correlation_parameters() {
    let case = parse_case(
        r#"
      <extensionElements>
        <flowable:eventType>multiCorr</flowable:eventType>
        <flowable:eventCorrelationParameter name="customerId" value="c1" />
        <flowable:eventCorrelationParameter name="region" value="eu" />
      </extensionElements>
      <casePlanModel id="planModelA" />
    "#,
    );
    assert_eq!(case.start_correlation_parameters.len(), 2);
    assert_eq!(case.start_correlation_parameters[0].name, "customerId");
    assert_eq!(case.start_correlation_parameters[0].value, "c1");
    assert_eq!(case.start_correlation_parameters[1].name, "region");
    assert_eq!(case.start_correlation_parameters[1].value, "eu");
}

#[test]
fn coexists_with_case_lifecycle_listener() {
    let case = parse_case(
        r#"
      <extensionElements>
        <flowable:eventType>coexistEvent</flowable:eventType>
        <flowable:caseLifecycleListener class="com.example.Audit" sourceState="active" targetState="completed" />
      </extensionElements>
      <casePlanModel id="planModelA" />
    "#,
    );
    assert_eq!(case.start_event_type.as_deref(), Some("coexistEvent"));
    assert_eq!(case.lifecycle_listeners.len(), 1);
    assert_eq!(case.lifecycle_listeners[0].implementation, "com.example.Audit");
}

#[test]
fn empty_event_type_text_is_ignored() {
    let case = parse_case(
        r#"
      <extensionElements>
        <flowable:eventType>   </flowable:eventType>
      </extensionElements>
      <casePlanModel id="planModelA" />
    "#,
    );
    assert!(case.start_event_type.is_none());
}

#[test]
fn no_start_extensions_leave_fields_empty() {
    let case = parse_case(r#"<casePlanModel id="planModelA" />"#);
    assert!(case.start_event_type.is_none());
    assert!(case.start_correlation_configuration.is_none());
    assert!(case.start_correlation_parameters.is_empty());
}
