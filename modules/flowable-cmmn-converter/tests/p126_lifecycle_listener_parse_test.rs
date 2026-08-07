// P126: CMMN lifecycle listener parser contract tests.
//
// Java references:
// - CmmnXmlConstants.java:64-65 — the only two lifecycle listener element names:
//   `planItemLifecycleListener` (on any plan item definition) and `caseLifecycleListener`
//   (on the case).
// - HasLifecycleListeners.java:21-25 — implemented by exactly two model classes,
//   Case.java:20 and PlanItemDefinition.java:21.
// - ExtensionElementsXMLConverter.java:121-124, :369-383 — dispatch + readLifecycleListener.
// - ListenerXmlConverterUtil.java:28-53 — attribute precedence class → expression →
//   delegateExpression, plus event / sourceState / targetState.
//
// 勘误 (correction to the P126 brief): Java has TWO lifecycle listener elements, not three.
// There is no `stageLifecycleListener` and no `TaskLifecycleListener` anywhere in the Java
// source tree. A stage receives listeners through `planItemLifecycleListener` because
// `Stage extends PlanItemDefinition` (AbstractPlanItemDefinitionExport.java:113 writes that
// element for every plan item definition, stages included). The `taskListener` element is a
// separate BPMN-style create/assignment/complete/delete mechanism, not a lifecycle listener.
use flowable_cmmn_converter::parse_cmmn_definitions;
use flowable_cmmn_model::{FlowableListener, ListenerImplementationType};

fn parse_case(case_body: &str) -> flowable_cmmn_model::Case {
    let xml = format!(
        r#"<definitions xmlns="http://www.omg.org/spec/CMMN/20151109/MODEL"
             xmlns:flowable="http://flowable.org/cmmn"
             targetNamespace="http://flowable.org/cmmn">
  <case id="caseA">{case_body}</case>
</definitions>"#
    );
    let definitions = parse_cmmn_definitions(&xml).expect("parse");
    definitions.cases.into_iter().next().expect("case")
}

fn parse_plan_model(plan_model_body: &str) -> flowable_cmmn_model::Case {
    parse_case(&format!(
        r#"<casePlanModel id="planModelA">{plan_model_body}</casePlanModel>"#
    ))
}

#[test]
fn parses_case_lifecycle_listener_with_all_attributes() {
    let case = parse_case(
        r#"
      <extensionElements>
        <flowable:caseLifecycleListener
            class="com.example.CaseAudit"
            sourceState="active"
            targetState="completed" />
      </extensionElements>
      <casePlanModel id="planModelA" />
    "#,
    );

    assert_eq!(
        case.lifecycle_listeners,
        vec![FlowableListener {
            implementation_type: ListenerImplementationType::Class,
            implementation: "com.example.CaseAudit".to_string(),
            source_state: Some("active".to_string()),
            target_state: Some("completed".to_string()),
            event: None,
        }]
    );
}

#[test]
fn parses_all_three_implementation_types() {
    let case = parse_case(
        r#"
      <extensionElements>
        <flowable:caseLifecycleListener class="com.example.Audit" />
        <flowable:caseLifecycleListener expression="${execution.setVariable('x', 1)}" />
        <flowable:caseLifecycleListener delegateExpression="${auditBean}" />
      </extensionElements>
      <casePlanModel id="planModelA" />
    "#,
    );

    let types: Vec<_> = case
        .lifecycle_listeners
        .iter()
        .map(|listener| (listener.implementation_type, listener.implementation.as_str()))
        .collect();
    assert_eq!(
        types,
        vec![
            (ListenerImplementationType::Class, "com.example.Audit"),
            (
                ListenerImplementationType::Expression,
                "${execution.setVariable('x', 1)}"
            ),
            (ListenerImplementationType::DelegateExpression, "${auditBean}"),
        ]
    );
}

// Java ListenerXmlConverterUtil.java:31-42 resolves the mutually exclusive attributes in the
// order class → expression → delegateExpression, so `class` wins when several are present.
#[test]
fn class_attribute_wins_over_expression_and_delegate_expression() {
    let case = parse_case(
        r#"
      <extensionElements>
        <flowable:caseLifecycleListener class="com.example.Audit"
            expression="${ignored}" delegateExpression="${alsoIgnored}" />
      </extensionElements>
      <casePlanModel id="planModelA" />
    "#,
    );

    let listener = &case.lifecycle_listeners[0];
    assert_eq!(listener.implementation_type, ListenerImplementationType::Class);
    assert_eq!(listener.implementation, "com.example.Audit");
}

// Java leaves the implementation type null when none of the attributes is present
// (ListenerXmlConverterUtil.java:31-42) and CmmnListenerNotificationHelper.java:88-100 then
// creates no listener at all — observably equivalent to dropping it here.
#[test]
fn listener_without_implementation_attribute_is_dropped() {
    let case = parse_case(
        r#"
      <extensionElements>
        <flowable:caseLifecycleListener sourceState="active" targetState="completed" />
      </extensionElements>
      <casePlanModel id="planModelA" />
    "#,
    );

    assert!(case.lifecycle_listeners.is_empty());
}

// Absent sourceState/targetState means "match any state"
// (CaseInstanceLifeCycleListenerUtil.java:76-78 `StringUtils.isEmpty(expected) || equals`).
#[test]
fn absent_state_filters_stay_none() {
    let case = parse_case(
        r#"
      <extensionElements>
        <flowable:caseLifecycleListener class="com.example.Audit" />
      </extensionElements>
      <casePlanModel id="planModelA" />
    "#,
    );

    let listener = &case.lifecycle_listeners[0];
    assert_eq!(listener.source_state, None);
    assert_eq!(listener.target_state, None);
}

#[test]
fn parses_plan_item_lifecycle_listener_on_human_task() {
    let case = parse_plan_model(
        r#"
      <planItem id="planItemTask" definitionRef="taskA" />
      <humanTask id="taskA" name="Task A">
        <extensionElements>
          <flowable:planItemLifecycleListener
              expression="${setVar('touched', true)}"
              sourceState="available"
              targetState="active" />
        </extensionElements>
      </humanTask>
    "#,
    );

    let task = &case.case_plan_model.human_tasks[0];
    assert_eq!(
        task.lifecycle_listeners,
        vec![FlowableListener {
            implementation_type: ListenerImplementationType::Expression,
            implementation: "${setVar('touched', true)}".to_string(),
            source_state: Some("available".to_string()),
            target_state: Some("active".to_string()),
            event: None,
        }]
    );
}

// A stage is a PlanItemDefinition, so it takes `planItemLifecycleListener` — there is no
// `stageLifecycleListener` element in Java.
#[test]
fn parses_plan_item_lifecycle_listener_on_stage_and_case_plan_model() {
    let case = parse_plan_model(
        r#"
      <extensionElements>
        <flowable:planItemLifecycleListener class="com.example.PlanModelAudit" />
      </extensionElements>
      <planItem id="planItemStage" definitionRef="stageA" />
      <stage id="stageA" name="Stage A">
        <extensionElements>
          <flowable:planItemLifecycleListener class="com.example.StageAudit"
              targetState="completed" />
        </extensionElements>
      </stage>
    "#,
    );

    assert_eq!(
        case.case_plan_model.lifecycle_listeners[0].implementation,
        "com.example.PlanModelAudit"
    );
    let stage = &case.case_plan_model.stages[0];
    assert_eq!(stage.lifecycle_listeners[0].implementation, "com.example.StageAudit");
    assert_eq!(
        stage.lifecycle_listeners[0].target_state,
        Some("completed".to_string())
    );
}

#[test]
fn parses_plan_item_lifecycle_listener_on_milestone_and_tasks() {
    let case = parse_plan_model(
        r#"
      <planItem id="planItemMilestone" definitionRef="milestoneA" />
      <milestone id="milestoneA" name="Milestone A">
        <extensionElements>
          <flowable:planItemLifecycleListener class="com.example.MilestoneAudit" />
        </extensionElements>
      </milestone>
      <planItem id="planItemProcess" definitionRef="processA" />
      <processTask id="processA" processRef="someProcess">
        <extensionElements>
          <flowable:planItemLifecycleListener class="com.example.ProcessAudit" />
        </extensionElements>
      </processTask>
      <planItem id="planItemCase" definitionRef="caseTaskA" />
      <caseTask id="caseTaskA" caseRef="someCase">
        <extensionElements>
          <flowable:planItemLifecycleListener class="com.example.CaseTaskAudit" />
        </extensionElements>
      </caseTask>
      <planItem id="planItemDecision" definitionRef="decisionA" />
      <decisionTask id="decisionA" decisionRef="someDecision">
        <extensionElements>
          <flowable:planItemLifecycleListener class="com.example.DecisionAudit" />
        </extensionElements>
      </decisionTask>
    "#,
    );

    let plan_model = &case.case_plan_model;
    assert_eq!(
        plan_model.milestones[0].lifecycle_listeners[0].implementation,
        "com.example.MilestoneAudit"
    );
    assert_eq!(
        plan_model.process_tasks[0].lifecycle_listeners[0].implementation,
        "com.example.ProcessAudit"
    );
    assert_eq!(
        plan_model.case_tasks[0].lifecycle_listeners[0].implementation,
        "com.example.CaseTaskAudit"
    );
    assert_eq!(
        plan_model.decision_tasks[0].lifecycle_listeners[0].implementation,
        "com.example.DecisionAudit"
    );
}

// Event listeners are plan item definitions too (PlanItemDefinition.java:21), including the
// timerEventListener, whose converter owns its own child dispatch loop.
#[test]
fn parses_plan_item_lifecycle_listener_on_event_listeners() {
    let case = parse_plan_model(
        r#"
      <planItem id="planItemUser" definitionRef="userListener" />
      <eventListener id="userListener" eventType="user">
        <extensionElements>
          <flowable:planItemLifecycleListener class="com.example.UserEventAudit" />
        </extensionElements>
      </eventListener>
      <planItem id="planItemTimer" definitionRef="timerListener" />
      <timerEventListener id="timerListener">
        <timerExpression>PT1H</timerExpression>
        <extensionElements>
          <flowable:planItemLifecycleListener class="com.example.TimerAudit" />
        </extensionElements>
      </timerEventListener>
    "#,
    );

    let listeners = &case.case_plan_model.event_listeners;
    assert_eq!(listeners[0].lifecycle_listeners[0].implementation, "com.example.UserEventAudit");
    assert_eq!(listeners[1].lifecycle_listeners[0].implementation, "com.example.TimerAudit");
    // the timerExpression sibling is still parsed
    assert_eq!(listeners[1].timer_expression, Some("PT1H".to_string()));
}

// P118 leniency is preserved: an unrelated extension element is still skipped, not rejected.
#[test]
fn unrelated_extension_elements_are_still_skipped() {
    let case = parse_plan_model(
        r#"
      <planItem id="planItemTask" definitionRef="taskA" />
      <humanTask id="taskA">
        <extensionElements>
          <flowable:somethingElse foo="bar" />
          <flowable:planItemLifecycleListener class="com.example.Audit" />
        </extensionElements>
      </humanTask>
    "#,
    );

    assert_eq!(
        case.case_plan_model.human_tasks[0].lifecycle_listeners[0].implementation,
        "com.example.Audit"
    );
}

// A caseLifecycleListener is only read on the `case` element and a planItemLifecycleListener
// only on plan item definitions — Java's readLifecycleListener throws when the owner does not
// implement HasLifecycleListeners (ExtensionElementsXMLConverter.java:369-383); the lenient
// Rust converter skips instead (P118).
#[test]
fn wrong_owner_element_name_is_skipped_not_parsed() {
    let case = parse_plan_model(
        r#"
      <planItem id="planItemTask" definitionRef="taskA" />
      <humanTask id="taskA">
        <extensionElements>
          <flowable:caseLifecycleListener class="com.example.Audit" />
        </extensionElements>
      </humanTask>
    "#,
    );

    assert!(case.case_plan_model.human_tasks[0].lifecycle_listeners.is_empty());
}
