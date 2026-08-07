// P117: timerEventListener parser contract tests.
//
// Java references:
// - TimerEventListenerXmlConverter.java:36-44 (name + flowable:availableCondition)
// - TimerExpressionXmlConverter.java:39-49 (timerExpression child text)
// - TimerEventListener.java:18-30 (timerExpression on the model)
use flowable_cmmn_converter::parse_cmmn_definitions;
use flowable_cmmn_model::{EventListener, PlanItemDefinitionRef};

fn parse_case(xml_body: &str) -> flowable_cmmn_model::Case {
    let xml = format!(
        r#"<definitions xmlns="http://www.omg.org/spec/CMMN/20151109/MODEL"
             xmlns:flowable="http://flowable.org/cmmn"
             targetNamespace="http://flowable.org/cmmn">
  <case id="caseA">
    <casePlanModel id="planModelA">{xml_body}</casePlanModel>
  </case>
</definitions>"#
    );
    let definitions = parse_cmmn_definitions(&xml).expect("parse");
    assert_eq!(definitions.cases.len(), 1);
    definitions.cases.into_iter().next().expect("case")
}

fn timer_listeners(case: &flowable_cmmn_model::Case) -> Vec<EventListener> {
    case.case_plan_model
        .event_listeners
        .iter()
        .filter(|listener| listener.is_timer())
        .cloned()
        .collect()
}

#[test]
fn parses_duration_timer_expression() {
    let case = parse_case(
        r#"
      <planItem id="planItemTimer" definitionRef="timerListener" />
      <timerEventListener id="timerListener" name="Timer listener">
        <timerExpression><![CDATA[PT1H]]></timerExpression>
      </timerEventListener>
    "#,
    );
    let listeners = timer_listeners(&case);
    assert_eq!(listeners.len(), 1);
    let listener = &listeners[0];
    assert_eq!(listener.event_type, EventListener::EVENT_TYPE_TIMER);
    assert_eq!(listener.timer_expression.as_deref(), Some("PT1H"));
    assert_eq!(listener.name.as_deref(), Some("Timer listener"));
}

#[test]
fn parses_date_and_repeat_timer_expressions() {
    let case = parse_case(
        r#"
      <planItem id="planItemDate" definitionRef="dateListener" />
      <planItem id="planItemRepeat" definitionRef="repeatListener" />
      <timerEventListener id="dateListener">
        <timerExpression>2026-08-05T10:00:00Z</timerExpression>
      </timerEventListener>
      <timerEventListener id="repeatListener">
        <timerExpression><![CDATA[R3/PT20S]]></timerExpression>
      </timerEventListener>
    "#,
    );
    let listeners = timer_listeners(&case);
    assert_eq!(listeners.len(), 2);
    let date_listener = listeners
        .iter()
        .find(|listener| listener.id == "dateListener")
        .expect("date listener");
    assert_eq!(
        date_listener.timer_expression.as_deref(),
        Some("2026-08-05T10:00:00Z")
    );
    let repeat_listener = listeners
        .iter()
        .find(|listener| listener.id == "repeatListener")
        .expect("repeat listener");
    assert_eq!(repeat_listener.timer_expression.as_deref(), Some("R3/PT20S"));
}

#[test]
fn parses_available_condition_attribute() {
    // TimerEventListenerXmlConverter.java:36-44 reads flowable:availableCondition.
    let case = parse_case(
        r#"
      <planItem id="planItemTimer" definitionRef="timerListener" />
      <timerEventListener id="timerListener" flowable:availableCondition="${var:get(timerVar)}">
        <timerExpression>PT1H</timerExpression>
      </timerEventListener>
    "#,
    );
    let listeners = timer_listeners(&case);
    assert_eq!(listeners.len(), 1);
    assert_eq!(
        listeners[0].available_condition.as_deref(),
        Some("${var:get(timerVar)}")
    );
}

#[test]
fn generic_event_listener_still_parses_available_condition() {
    // GenericEventListenerXmlConverter.java:68-73 reads flowable:availableCondition too.
    let case = parse_case(
        r#"
      <planItem id="planItemListener" definitionRef="messageListener" />
      <eventListener id="messageListener" eventType="message" eventName="myMessage"
                     flowable:availableCondition="${go}"/>
    "#,
    );
    let listener = case
        .case_plan_model
        .event_listeners
        .iter()
        .find(|listener| listener.id == "messageListener")
        .expect("listener");
    assert!(!listener.is_timer());
    assert_eq!(listener.event_type, "message");
    assert_eq!(listener.available_condition.as_deref(), Some("${go}"));
}

#[test]
fn timer_listener_resolves_plan_item_and_occur_sentry() {
    // validate_sentries requires an `occur` onPart source to reference an eventListener or
    // milestone; a timerEventListener counts as an event listener (Java TimerEventListener
    // extends EventListener).
    let case = parse_case(
        r#"
      <planItem id="planItemTimer" definitionRef="timerListener" />
      <planItem id="planItemTask" definitionRef="taskA">
        <entryCriterion id="criterion1" sentryRef="sentry1" />
      </planItem>
      <sentry id="sentry1">
        <planItemOnPart id="onPart1" sourceRef="planItemTimer">
          <standardEvent>occur</standardEvent>
        </planItemOnPart>
      </sentry>
      <timerEventListener id="timerListener">
        <timerExpression>PT1H</timerExpression>
      </timerEventListener>
      <humanTask id="taskA" />
    "#,
    );

    let plan_model = &case.case_plan_model;
    let timer_plan_item = plan_model
        .plan_items
        .iter()
        .find(|plan_item| plan_item.id == "planItemTimer")
        .expect("plan item");
    let definition_ref = plan_model
        .find_plan_item_definition(&timer_plan_item.definition_ref)
        .expect("definition ref");
    assert!(matches!(
        definition_ref,
        PlanItemDefinitionRef::EventListener(listener) if listener.is_timer()
    ));
}

#[test]
fn rejects_missing_id() {
    let result = parse_cmmn_definitions(
        r#"<definitions xmlns="http://www.omg.org/spec/CMMN/20151109/MODEL" targetNamespace="x">
  <case id="caseA">
    <casePlanModel id="planModelA">
      <timerEventListener>
        <timerExpression>PT1H</timerExpression>
      </timerEventListener>
    </casePlanModel>
  </case>
</definitions>"#,
    );
    assert!(result.is_err(), "timerEventListener without id must be rejected");
}

#[test]
fn skips_unknown_child_element() {
    // P118: Java CmmnXmlConverter.java:222-226 silently skips unregistered elements;
    // Rust converter now aligns (skip + warn) instead of rejecting.
    let definitions = parse_cmmn_definitions(
        r#"<definitions xmlns="http://www.omg.org/spec/CMMN/20151109/MODEL" targetNamespace="x">
  <case id="caseA">
    <casePlanModel id="planModelA">
      <timerEventListener id="t1">
        <timerExpression>PT1H</timerExpression>
        <unexpected/>
      </timerEventListener>
    </casePlanModel>
  </case>
</definitions>"#,
    )
    .expect("unknown child inside timerEventListener must be skipped, not rejected");
    let listener = &definitions.cases[0].case_plan_model.event_listeners[0];
    assert_eq!(listener.id, "t1");
    assert_eq!(listener.timer_expression.as_deref(), Some("PT1H"));
}

#[test]
fn ignores_unknown_attribute() {
    // P118: Java converters only read known attributes (BaseCmmnXmlConverter /
    // TimerEventListenerXmlConverter); eventType on timerEventListener is ignored.
    let definitions = parse_cmmn_definitions(
        r#"<definitions xmlns="http://www.omg.org/spec/CMMN/20151109/MODEL" targetNamespace="x">
  <case id="caseA">
    <casePlanModel id="planModelA">
      <timerEventListener id="t1" eventType="message">
        <timerExpression>PT1H</timerExpression>
      </timerEventListener>
    </casePlanModel>
  </case>
</definitions>"#,
    )
    .expect("unknown attribute on timerEventListener must be ignored, not rejected");
    let listener = &definitions.cases[0].case_plan_model.event_listeners[0];
    assert_eq!(listener.id, "t1");
    // eventType is ignored; timer listeners keep the internal timer type.
    assert_eq!(listener.event_type, "timer");
    assert_eq!(listener.timer_expression.as_deref(), Some("PT1H"));
}

#[test]
fn empty_timer_expression_is_treated_as_absent() {
    // Java TimerExpressionXmlConverter.java:42-44 only sets the expression when non-empty.
    let case = parse_case(
        r#"
      <timerEventListener id="t1">
        <timerExpression></timerExpression>
      </timerEventListener>
    "#,
    );
    let listeners = timer_listeners(&case);
    assert!(listeners.is_empty(), "empty timerExpression must not mark a timer listener");
    let listener = case
        .case_plan_model
        .event_listeners
        .iter()
        .find(|listener| listener.id == "t1")
        .expect("listener");
    assert!(!listener.is_timer());
    assert_eq!(listener.event_type, EventListener::EVENT_TYPE_TIMER);
}
