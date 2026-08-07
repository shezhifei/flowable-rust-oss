use flowable_cmmn_converter::parse_cmmn_definitions;
use flowable_cmmn_model::{
    PlanItemDefinitionRef, SentryIfPartCondition, SentryIfPartExpression, SentryIfPartLiteral,
    SentryIfPartLogicalOperator, SentryIfPartOperator,
};

const OWNED_SUBSET_CMMN: &str = r#"
<definitions xmlns="http://www.omg.org/spec/CMMN/20151109/MODEL"
             xmlns:cmmndi="http://www.omg.org/spec/CMMN/20151109/CMMNDI"
             xmlns:dc="http://www.omg.org/spec/CMMN/20151109/DC"
             xmlns:di="http://www.omg.org/spec/CMMN/20151109/DI"
             targetNamespace="http://flowable.org/cmmn">
  <case id="caseA" name="Case A">
    <casePlanModel id="planModelA" name="Plan Model A" autoComplete="false">
      <planItem id="planItemStage" name="Review Stage" definitionRef="reviewStage" />
      <planItem id="planItemRootTask" name="Root Task" definitionRef="rootTask" />
      <stage id="reviewStage" name="Review Stage" autoComplete="true">
        <planItem id="planItemNestedTask" name="Prepare Review" definitionRef="prepareReview" />
        <humanTask id="prepareReview" name="Prepare Review" isBlocking="false" />
      </stage>
      <humanTask id="rootTask" name="Root Task" isBlocking="true" />
    </casePlanModel>
  </case>
</definitions>
"#;

const ENTRY_CRITERION_CMMN: &str = r#"
<definitions xmlns="http://www.omg.org/spec/CMMN/20151109/MODEL"
             targetNamespace="http://flowable.org/cmmn">
  <case id="caseA">
    <casePlanModel id="planModelA">
      <planItem id="planItemA" definitionRef="taskA" />
      <planItem id="planItemB" definitionRef="taskB">
        <entryCriterion id="entryCriterionB" sentryRef="sentryB" />
      </planItem>
      <humanTask id="taskA" />
      <humanTask id="taskB" />
      <sentry id="sentryB">
        <planItemOnPart id="onTaskAComplete" sourceRef="planItemA">
          <standardEvent>complete</standardEvent>
        </planItemOnPart>
      </sentry>
    </casePlanModel>
  </case>
</definitions>
"#;

const EXIT_CRITERION_CMMN: &str = r#"
<definitions xmlns="http://www.omg.org/spec/CMMN/20151109/MODEL"
             targetNamespace="http://flowable.org/cmmn">
  <case id="caseA">
    <casePlanModel id="planModelA">
      <planItem id="planItemA" definitionRef="taskA" />
      <planItem id="planItemB" definitionRef="taskB">
        <exitCriterion id="exitCriterionB" sentryRef="sentryB" />
      </planItem>
      <humanTask id="taskA" />
      <humanTask id="taskB" />
      <sentry id="sentryB">
        <planItemOnPart id="onTaskAComplete" sourceRef="planItemA">
          <standardEvent>complete</standardEvent>
        </planItemOnPart>
      </sentry>
    </casePlanModel>
  </case>
</definitions>
"#;

const TERMINATE_ENTRY_CRITERION_CMMN: &str = r#"
<definitions xmlns="http://www.omg.org/spec/CMMN/20151109/MODEL"
             targetNamespace="http://flowable.org/cmmn">
  <case id="caseA">
    <casePlanModel id="planModelA">
      <planItem id="planItemA" definitionRef="taskA" />
      <planItem id="planItemB" definitionRef="taskB">
        <entryCriterion id="entryCriterionB" sentryRef="sentryB" />
      </planItem>
      <humanTask id="taskA" />
      <humanTask id="taskB" />
      <sentry id="sentryB">
        <planItemOnPart id="onTaskATerminate" sourceRef="planItemA">
          <standardEvent>terminate</standardEvent>
        </planItemOnPart>
      </sentry>
    </casePlanModel>
  </case>
</definitions>
"#;

const IF_PART_ENTRY_CRITERION_CMMN: &str = r#"
<definitions xmlns="http://www.omg.org/spec/CMMN/20151109/MODEL"
             targetNamespace="http://flowable.org/cmmn">
  <case id="caseA">
    <casePlanModel id="planModelA">
      <planItem id="planItemA" definitionRef="taskA" />
      <planItem id="planItemB" definitionRef="taskB">
        <entryCriterion id="entryCriterionB" sentryRef="sentryB" />
      </planItem>
      <humanTask id="taskA" />
      <humanTask id="taskB" />
      <sentry id="sentryB">
        <planItemOnPart id="onTaskAComplete" sourceRef="planItemA">
          <standardEvent>complete</standardEvent>
        </planItemOnPart>
        <ifPart>
          <condition>approved == true</condition>
        </ifPart>
      </sentry>
    </casePlanModel>
  </case>
</definitions>
"#;

const PLAN_ITEM_CONTROL_RULES_CMMN: &str = r#"
<definitions xmlns="http://www.omg.org/spec/CMMN/20151109/MODEL"
             targetNamespace="http://flowable.org/cmmn">
  <case id="caseA">
    <casePlanModel id="planModelA">
      <planItem id="planItemReview" definitionRef="reviewTask">
        <itemControl>
          <manualActivationRule>
            <condition>${manualActivation == true}</condition>
          </manualActivationRule>
          <repetitionRule>
            <condition>repeatReview == true</condition>
          </repetitionRule>
        </itemControl>
      </planItem>
      <humanTask id="reviewTask" />
    </casePlanModel>
  </case>
</definitions>
"#;

const NON_HUMAN_PLAN_ITEM_CONTROL_RULES_CMMN: &str = r#"
<definitions xmlns="http://www.omg.org/spec/CMMN/20151109/MODEL"
             targetNamespace="http://flowable.org/cmmn">
  <case id="caseA">
    <casePlanModel id="planModelA">
      <planItem id="planItemStage" definitionRef="reviewStage">
        <itemControl>
          <manualActivationRule>
            <condition>stageManual == true</condition>
          </manualActivationRule>
          <repetitionRule>
            <condition>stageRepeat == true</condition>
          </repetitionRule>
        </itemControl>
      </planItem>
      <planItem id="planItemMilestone" definitionRef="approvalMilestone">
        <itemControl>
          <manualActivationRule>
            <condition>milestoneManual == true</condition>
          </manualActivationRule>
          <repetitionRule>
            <condition>milestoneRepeat == true</condition>
          </repetitionRule>
        </itemControl>
      </planItem>
      <planItem id="planItemEvent" definitionRef="approvalEvent">
        <itemControl>
          <manualActivationRule>
            <condition>eventManual == true</condition>
          </manualActivationRule>
          <repetitionRule>
            <condition>eventRepeat == true</condition>
          </repetitionRule>
        </itemControl>
      </planItem>
      <stage id="reviewStage" name="Review Stage" />
      <milestone id="approvalMilestone" name="Approval Milestone" />
      <eventListener id="approvalEvent" eventType="message" eventName="approvalReceived" />
    </casePlanModel>
  </case>
</definitions>
"#;

const STAGE_PLANNING_TABLE_CMMN: &str = r#"
<definitions xmlns="http://www.omg.org/spec/CMMN/20151109/MODEL"
             targetNamespace="http://flowable.org/cmmn">
  <case id="casePlanning" name="Planning case">
    <casePlanModel id="planModelPlanning" name="Planning model">
      <planItem id="planItemReviewStage" definitionRef="reviewStage" />
      <stage id="reviewStage" name="Review stage">
        <planItem id="planItemAnchor" definitionRef="anchorTask" />
        <humanTask id="anchorTask" name="Anchor task" />
        <humanTask id="peerReviewTask" name="Peer review" />
        <planningTable id="reviewPlanningTable" name="Review planning">
          <discretionaryItem id="discretionaryPeerReview" name="Peer review" definitionRef="peerReviewTask" />
        </planningTable>
      </stage>
    </casePlanModel>
  </case>
</definitions>
"#;

const CASE_LEVEL_PLANNING_TABLE_CMMN: &str = r#"
<definitions xmlns="http://www.omg.org/spec/CMMN/20151109/MODEL"
             targetNamespace="http://flowable.org/cmmn">
  <case id="casePlanning" name="Planning case">
    <casePlanModel id="planModelPlanning" name="Planning model">
      <planItem id="planItemAnchor" definitionRef="anchorTask" />
      <humanTask id="anchorTask" name="Anchor task" />
      <humanTask id="caseReviewTask" name="Case review" />
      <planningTable id="casePlanningTable" name="Case planning">
        <discretionaryItem id="discretionaryCaseReview" name="Case review" definitionRef="caseReviewTask" />
      </planningTable>
    </casePlanModel>
  </case>
</definitions>
"#;

const IF_PART_EXTENDED_CONDITION_CMMN: &str = r#"
<definitions xmlns="http://www.omg.org/spec/CMMN/20151109/MODEL"
             targetNamespace="http://flowable.org/cmmn">
  <case id="caseA">
    <casePlanModel id="planModelA">
      <planItem id="planItemA" definitionRef="taskA" />
      <planItem id="planItemB" definitionRef="taskB">
        <entryCriterion id="entryCriterionStatus" sentryRef="sentryStatus" />
        <entryCriterion id="entryCriterionAmount" sentryRef="sentryAmount" />
      </planItem>
      <planItem id="planItemC" definitionRef="taskC">
        <entryCriterion id="entryCriterionDenied" sentryRef="sentryDenied" />
      </planItem>
      <humanTask id="taskA" />
      <humanTask id="taskB" />
      <humanTask id="taskC" />
      <sentry id="sentryStatus">
        <planItemOnPart id="onTaskACompleteForStatus" sourceRef="planItemA">
          <standardEvent>complete</standardEvent>
        </planItemOnPart>
        <ifPart>
          <condition>${status == "approved"}</condition>
        </ifPart>
      </sentry>
      <sentry id="sentryAmount">
        <planItemOnPart id="onTaskACompleteForAmount" sourceRef="planItemA">
          <standardEvent>complete</standardEvent>
        </planItemOnPart>
        <ifPart>
          <condition>amount == 42.5</condition>
        </ifPart>
      </sentry>
      <sentry id="sentryDenied">
        <planItemOnPart id="onTaskACompleteForDenied" sourceRef="planItemA">
          <standardEvent>complete</standardEvent>
        </planItemOnPart>
        <ifPart>
          <condition>decision != 'denied'</condition>
        </ifPart>
      </sentry>
    </casePlanModel>
  </case>
</definitions>
"#;

const IF_PART_LOGICAL_CONDITION_CMMN: &str = r#"
<definitions xmlns="http://www.omg.org/spec/CMMN/20151109/MODEL"
             targetNamespace="http://flowable.org/cmmn">
  <case id="caseA">
    <casePlanModel id="planModelA">
      <planItem id="planItemA" definitionRef="taskA" />
      <planItem id="planItemB" definitionRef="taskB">
        <entryCriterion id="entryCriterionAnd" sentryRef="sentryAnd" />
        <entryCriterion id="entryCriterionOr" sentryRef="sentryOr" />
      </planItem>
      <humanTask id="taskA" />
      <humanTask id="taskB" />
      <sentry id="sentryAnd">
        <planItemOnPart id="onTaskACompleteForAnd" sourceRef="planItemA">
          <standardEvent>complete</standardEvent>
        </planItemOnPart>
        <ifPart>
          <condition>${(approved == true) &amp;&amp; (amount != 0)}</condition>
        </ifPart>
      </sentry>
      <sentry id="sentryOr">
        <planItemOnPart id="onTaskACompleteForOr" sourceRef="planItemA">
          <standardEvent>complete</standardEvent>
        </planItemOnPart>
        <ifPart>
          <condition>(status == "approved") or (expedited == true) or (amount == 100)</condition>
        </ifPart>
      </sentry>
    </casePlanModel>
  </case>
</definitions>
"#;

const IF_PART_ADVANCED_CONDITION_CMMN: &str = r#"
<definitions xmlns="http://www.omg.org/spec/CMMN/20151109/MODEL"
             targetNamespace="http://flowable.org/cmmn">
  <case id="caseA">
    <casePlanModel id="planModelA">
      <planItem id="planItemA" definitionRef="taskA" />
      <planItem id="planItemB" definitionRef="taskB">
        <entryCriterion id="entryCriterionB" sentryRef="sentryB" />
      </planItem>
      <humanTask id="taskA" />
      <humanTask id="taskB" />
      <sentry id="sentryB">
        <planItemOnPart id="onTaskAComplete" sourceRef="planItemA">
          <standardEvent>complete</standardEvent>
        </planItemOnPart>
        <ifPart>
          <condition>${(approved == true &amp;&amp; amount &gt; 100) || reviewer == 'lead'}</condition>
        </ifPart>
      </sentry>
    </casePlanModel>
  </case>
</definitions>
"#;

const IF_PART_NULL_EMPTY_CONDITION_CMMN: &str = r#"
<definitions xmlns="http://www.omg.org/spec/CMMN/20151109/MODEL"
             targetNamespace="http://flowable.org/cmmn">
  <case id="caseA">
    <casePlanModel id="planModelA">
      <planItem id="planItemA" definitionRef="taskA" />
      <planItem id="planItemB" definitionRef="taskB">
        <entryCriterion id="entryCriterionNull" sentryRef="sentryNull" />
        <entryCriterion id="entryCriterionEmpty" sentryRef="sentryEmpty" />
      </planItem>
      <humanTask id="taskA" />
      <humanTask id="taskB" />
      <sentry id="sentryNull">
        <planItemOnPart id="onTaskACompleteForNull" sourceRef="planItemA">
          <standardEvent>complete</standardEvent>
        </planItemOnPart>
        <ifPart>
          <condition>${optionalValue == null}</condition>
        </ifPart>
      </sentry>
      <sentry id="sentryEmpty">
        <planItemOnPart id="onTaskACompleteForEmpty" sourceRef="planItemA">
          <standardEvent>complete</standardEvent>
        </planItemOnPart>
        <ifPart>
          <condition>not empty(comment)</condition>
        </ifPart>
      </sentry>
    </casePlanModel>
  </case>
</definitions>
"#;

const IF_PART_PROPERTY_METHOD_CONDITION_CMMN: &str = r#"
<definitions xmlns="http://www.omg.org/spec/CMMN/20151109/MODEL"
             targetNamespace="http://flowable.org/cmmn">
  <case id="caseA">
    <casePlanModel id="planModelA">
      <planItem id="planItemA" definitionRef="taskA" />
      <planItem id="planItemB" definitionRef="taskB">
        <entryCriterion id="entryCriterionSize" sentryRef="sentrySize" />
        <entryCriterion id="entryCriterionLength" sentryRef="sentryLength" />
        <entryCriterion id="entryCriterionContains" sentryRef="sentryContains" />
        <entryCriterion id="entryCriterionComplexLength" sentryRef="sentryComplexLength" />
      </planItem>
      <humanTask id="taskA" />
      <humanTask id="taskB" />
      <sentry id="sentrySize">
        <planItemOnPart id="onTaskACompleteForSize" sourceRef="planItemA">
          <standardEvent>complete</standardEvent>
        </planItemOnPart>
        <ifPart>
          <condition>${items.size() &gt;= minimumItemCount}</condition>
        </ifPart>
      </sentry>
      <sentry id="sentryLength">
        <planItemOnPart id="onTaskACompleteForLength" sourceRef="planItemA">
          <standardEvent>complete</standardEvent>
        </planItemOnPart>
        <ifPart>
          <condition>${customer.name.length() &gt;= minimumNameLength}</condition>
        </ifPart>
      </sentry>
      <sentry id="sentryContains">
        <planItemOnPart id="onTaskACompleteForContains" sourceRef="planItemA">
          <standardEvent>complete</standardEvent>
        </planItemOnPart>
        <ifPart>
          <condition>${contains(customer.name + suffix, expectedNeedle)}</condition>
        </ifPart>
      </sentry>
      <sentry id="sentryComplexLength">
        <planItemOnPart id="onTaskACompleteForComplexLength" sourceRef="planItemA">
          <standardEvent>complete</standardEvent>
        </planItemOnPart>
        <ifPart>
          <condition>${length(customer.name + suffix) &gt;= minimumFullNameLength}</condition>
        </ifPart>
      </sentry>
    </casePlanModel>
  </case>
</definitions>
"#;

const ENABLE_DISABLE_ENTRY_CRITERION_CMMN: &str = r#"
<definitions xmlns="http://www.omg.org/spec/CMMN/20151109/MODEL"
             targetNamespace="http://flowable.org/cmmn">
  <case id="caseA">
    <casePlanModel id="planModelA">
      <planItem id="planItemA" definitionRef="taskA" />
      <planItem id="planItemB" definitionRef="taskB">
        <entryCriterion id="entryCriterionDisable" sentryRef="sentryDisable" />
      </planItem>
      <planItem id="planItemC" definitionRef="taskC">
        <entryCriterion id="entryCriterionEnable" sentryRef="sentryEnable" />
      </planItem>
      <humanTask id="taskA" />
      <humanTask id="taskB" />
      <humanTask id="taskC" />
      <sentry id="sentryDisable">
        <planItemOnPart id="onTaskADisable" sourceRef="planItemA">
          <standardEvent>disable</standardEvent>
        </planItemOnPart>
      </sentry>
      <sentry id="sentryEnable">
        <planItemOnPart id="onTaskAEnable" sourceRef="planItemA">
          <standardEvent>enable</standardEvent>
        </planItemOnPart>
      </sentry>
    </casePlanModel>
  </case>
</definitions>
"#;

const UNSUPPORTED_IF_PART_CONDITION_CMMN: &str = r#"
<definitions xmlns="http://www.omg.org/spec/CMMN/20151109/MODEL"
             targetNamespace="http://flowable.org/cmmn">
  <case id="caseA">
    <casePlanModel id="planModelA">
      <planItem id="planItemA" definitionRef="taskA" />
      <planItem id="planItemB" definitionRef="taskB">
        <entryCriterion id="entryCriterionB" sentryRef="sentryB" />
      </planItem>
      <humanTask id="taskA" />
      <humanTask id="taskB" />
      <sentry id="sentryB">
        <planItemOnPart id="onTaskAComplete" sourceRef="planItemA">
          <standardEvent>complete</standardEvent>
        </planItemOnPart>
        <ifPart>
          <condition>isApproved() == true</condition>
        </ifPart>
      </sentry>
    </casePlanModel>
  </case>
</definitions>
"#;

const MULTI_ON_PART_ENTRY_CRITERION_CMMN: &str = r#"
<definitions xmlns="http://www.omg.org/spec/CMMN/20151109/MODEL"
             targetNamespace="http://flowable.org/cmmn">
  <case id="caseA">
    <casePlanModel id="planModelA">
      <planItem id="planItemA" definitionRef="taskA" />
      <planItem id="planItemB" definitionRef="taskB" />
      <planItem id="planItemC" definitionRef="taskC">
        <entryCriterion id="entryCriterionC" sentryRef="sentryC" />
      </planItem>
      <humanTask id="taskA" />
      <humanTask id="taskB" />
      <humanTask id="taskC" />
      <sentry id="sentryC">
        <planItemOnPart id="onTaskAComplete" sourceRef="planItemA">
          <standardEvent>complete</standardEvent>
        </planItemOnPart>
        <planItemOnPart id="onTaskBComplete" sourceRef="planItemB">
          <standardEvent>complete</standardEvent>
        </planItemOnPart>
      </sentry>
    </casePlanModel>
  </case>
</definitions>
"#;

const COMPLEX_SENTRY_CMMN: &str = r#"
<definitions xmlns="http://www.omg.org/spec/CMMN/20151109/MODEL"
             targetNamespace="http://flowable.org/cmmn">
  <case id="caseA">
    <casePlanModel id="planModelA">
      <humanTask id="taskA" />
      <sentry id="sentryA">
        <ifPart />
      </sentry>
    </casePlanModel>
  </case>
</definitions>
"#;

const NON_LOCAL_DEFINITION_REF_CMMN: &str = r#"
<definitions xmlns="http://www.omg.org/spec/CMMN/20151109/MODEL"
             targetNamespace="http://flowable.org/cmmn">
  <case id="caseA">
    <casePlanModel id="planModelA">
      <planItem id="planItemStage" definitionRef="reviewStage" />
      <humanTask id="rootTask" />
      <stage id="reviewStage">
        <planItem id="planItemNestedTask" definitionRef="rootTask" />
      </stage>
    </casePlanModel>
  </case>
</definitions>
"#;

const EVENT_LISTENER_CMMN: &str = r#"
<definitions xmlns="http://www.omg.org/spec/CMMN/20151109/MODEL"
             targetNamespace="http://flowable.org/cmmn">
  <case id="caseA">
    <casePlanModel id="planModelA">
      <planItem id="planItemEvent" definitionRef="waitForApproval" />
      <eventListener id="waitForApproval" name="Wait for approval" eventType="message" eventName="approvalReceived" />
    </casePlanModel>
  </case>
</definitions>
"#;

const EVENT_LISTENER_OCCUR_ENTRY_CRITERION_CMMN: &str = r#"
<definitions xmlns="http://www.omg.org/spec/CMMN/20151109/MODEL"
             targetNamespace="http://flowable.org/cmmn">
  <case id="caseA">
    <casePlanModel id="planModelA">
      <planItem id="planItemEvent" definitionRef="waitForApproval" />
      <planItem id="planItemTask" definitionRef="approveTask">
        <entryCriterion id="entryCriterionTask" sentryRef="sentryAfterEvent" />
      </planItem>
      <eventListener id="waitForApproval" name="Wait for approval" eventType="message" eventName="approvalReceived" />
      <humanTask id="approveTask" />
      <sentry id="sentryAfterEvent">
        <planItemOnPart id="onEventOccur" sourceRef="planItemEvent">
          <standardEvent>occur</standardEvent>
        </planItemOnPart>
      </sentry>
    </casePlanModel>
  </case>
</definitions>
"#;

#[test]
fn parses_owned_case_stage_human_task_and_plan_item_subset() {
    let definitions = parse_cmmn_definitions(OWNED_SUBSET_CMMN).expect("owned subset should parse");

    assert_eq!(
        definitions.target_namespace.as_deref(),
        Some("http://flowable.org/cmmn")
    );
    assert_eq!(definitions.cases.len(), 1);

    let case_definition = definitions
        .find_case("caseA")
        .expect("case definition should be indexed");
    assert_eq!(case_definition.name.as_deref(), Some("Case A"));
    assert_eq!(case_definition.case_plan_model.id, "planModelA");
    assert!(!case_definition.case_plan_model.auto_complete);
    assert_eq!(case_definition.case_plan_model.plan_items.len(), 2);
    assert_eq!(case_definition.case_plan_model.stages.len(), 1);
    assert_eq!(case_definition.case_plan_model.human_tasks.len(), 1);

    let nested_stage = case_definition
        .find_plan_item_definition("reviewStage")
        .expect("nested stage should be resolvable");
    assert_eq!(nested_stage.id(), "reviewStage");
    assert_eq!(nested_stage.name(), Some("Review Stage"));
    assert!(matches!(nested_stage, PlanItemDefinitionRef::Stage(_)));

    let nested_plan_item = case_definition
        .find_plan_item("planItemNestedTask")
        .expect("nested plan item should be resolvable");
    assert_eq!(nested_plan_item.definition_ref, "prepareReview");

    let nested_task = case_definition
        .find_plan_item_definition("prepareReview")
        .expect("nested task definition should be resolvable");
    assert_eq!(nested_task.id(), "prepareReview");
    assert_eq!(nested_task.name(), Some("Prepare Review"));
    assert!(matches!(nested_task, PlanItemDefinitionRef::HumanTask(_)));
}

#[test]
fn parses_process_and_case_tasks_as_plan_item_definitions() {
    let xml = r#"
<definitions xmlns="http://www.omg.org/spec/CMMN/20151109/MODEL" targetNamespace="Examples">
  <case id="parentCase" name="Parent case">
    <casePlanModel id="casePlanModel" name="Case plan">
      <planItem id="planItemProcess" name="Launch process" definitionRef="processTaskA" />
      <planItem id="planItemCase" name="Launch case" definitionRef="caseTaskA" />
      <processTask id="processTaskA" name="Launch BPMN" processRef="approvalProcess" isBlocking="false" />
      <caseTask id="caseTaskA" name="Launch CMMN" caseRef="childCase" isBlocking="true" />
    </casePlanModel>
  </case>
</definitions>"#;

    let definitions = parse_cmmn_definitions(xml).expect("CMMN should parse");
    let case_definition = definitions.find_case("parentCase").expect("case");

    let process_task = case_definition
        .find_plan_item_definition("processTaskA")
        .expect("process task definition");
    let case_task = case_definition
        .find_plan_item_definition("caseTaskA")
        .expect("case task definition");

    match process_task {
        PlanItemDefinitionRef::ProcessTask(task) => {
            assert_eq!(task.name.as_deref(), Some("Launch BPMN"));
            assert_eq!(task.process_ref.as_deref(), Some("approvalProcess"));
            assert!(!task.is_blocking);
        }
        other => panic!("expected processTask, got {other:?}"),
    }

    match case_task {
        PlanItemDefinitionRef::CaseTask(task) => {
            assert_eq!(task.name.as_deref(), Some("Launch CMMN"));
            assert_eq!(task.case_ref.as_deref(), Some("childCase"));
            assert!(task.is_blocking);
        }
        other => panic!("expected caseTask, got {other:?}"),
    }
}

#[test]
fn parses_event_listener_plan_item_definition() {
    let definitions =
        parse_cmmn_definitions(EVENT_LISTENER_CMMN).expect("event listener should parse");
    let case_definition = definitions.find_case("caseA").expect("case");

    assert_eq!(case_definition.case_plan_model.event_listeners.len(), 1);
    let listener = &case_definition.case_plan_model.event_listeners[0];
    assert_eq!(listener.id, "waitForApproval");
    assert_eq!(listener.name.as_deref(), Some("Wait for approval"));
    assert_eq!(listener.event_type, "message");
    assert_eq!(listener.event_name.as_deref(), Some("approvalReceived"));

    let definition = case_definition
        .find_plan_item_definition("waitForApproval")
        .expect("event listener definition should be resolvable");
    assert!(matches!(
        definition,
        PlanItemDefinitionRef::EventListener(_)
    ));
}

#[test]
fn parses_stage_planning_table_discretionary_human_task() {
    let definitions =
        parse_cmmn_definitions(STAGE_PLANNING_TABLE_CMMN).expect("planning table should parse");
    let case_definition = definitions.find_case("casePlanning").expect("case");
    let stage = match case_definition
        .find_plan_item_definition("reviewStage")
        .expect("review stage")
    {
        PlanItemDefinitionRef::Stage(stage) => stage,
        other => panic!("expected stage definition, got {other:?}"),
    };

    assert_eq!(stage.planning_tables.len(), 1);
    let planning_table = &stage.planning_tables[0];
    assert_eq!(planning_table.id, "reviewPlanningTable");
    assert_eq!(planning_table.name.as_deref(), Some("Review planning"));
    assert_eq!(planning_table.discretionary_items.len(), 1);

    let discretionary_item = &planning_table.discretionary_items[0];
    assert_eq!(discretionary_item.id, "discretionaryPeerReview");
    assert_eq!(discretionary_item.name.as_deref(), Some("Peer review"));
    assert_eq!(discretionary_item.definition_ref, "peerReviewTask");
}

#[test]
fn parses_case_level_planning_table_discretionary_human_task() {
    let definitions = parse_cmmn_definitions(CASE_LEVEL_PLANNING_TABLE_CMMN)
        .expect("case-level planning table should parse");
    let case_definition = definitions.find_case("casePlanning").expect("case");

    assert_eq!(case_definition.case_plan_model.planning_tables.len(), 1);
    let planning_table = &case_definition.case_plan_model.planning_tables[0];
    assert_eq!(planning_table.id, "casePlanningTable");
    assert_eq!(planning_table.name.as_deref(), Some("Case planning"));
    assert_eq!(planning_table.discretionary_items.len(), 1);

    let discretionary_item = &planning_table.discretionary_items[0];
    assert_eq!(discretionary_item.id, "discretionaryCaseReview");
    assert_eq!(discretionary_item.name.as_deref(), Some("Case review"));
    assert_eq!(discretionary_item.definition_ref, "caseReviewTask");
}

#[test]
fn parses_basic_entry_criterion_sentry_on_source_completion() {
    let definitions =
        parse_cmmn_definitions(ENTRY_CRITERION_CMMN).expect("basic sentry should parse");
    let case_definition = definitions.find_case("caseA").expect("case");

    assert_eq!(case_definition.case_plan_model.sentries.len(), 1);
    let sentry = &case_definition.case_plan_model.sentries[0];
    assert_eq!(sentry.id, "sentryB");
    assert_eq!(sentry.plan_item_on_parts.len(), 1);
    assert_eq!(sentry.plan_item_on_parts[0].id, "onTaskAComplete");
    assert_eq!(sentry.plan_item_on_parts[0].source_ref, "planItemA");
    assert_eq!(sentry.plan_item_on_parts[0].standard_event, "complete");

    let gated_plan_item = case_definition
        .find_plan_item("planItemB")
        .expect("gated plan item");
    assert_eq!(gated_plan_item.entry_criteria.len(), 1);
    assert_eq!(gated_plan_item.entry_criteria[0].id, "entryCriterionB");
    assert_eq!(gated_plan_item.entry_criteria[0].sentry_ref, "sentryB");
}

#[test]
fn parses_exit_criterion_sentry_on_source_completion() {
    let definitions =
        parse_cmmn_definitions(EXIT_CRITERION_CMMN).expect("exit criterion should parse");
    let case_definition = definitions.find_case("caseA").expect("case");

    assert_eq!(case_definition.case_plan_model.sentries.len(), 1);
    let sentry = &case_definition.case_plan_model.sentries[0];
    assert_eq!(sentry.id, "sentryB");
    assert_eq!(sentry.plan_item_on_parts.len(), 1);
    assert_eq!(sentry.plan_item_on_parts[0].source_ref, "planItemA");
    assert_eq!(sentry.plan_item_on_parts[0].standard_event, "complete");

    let plan_item = case_definition
        .find_plan_item("planItemB")
        .expect("plan item with exit criterion");
    assert!(plan_item.entry_criteria.is_empty());
    assert_eq!(plan_item.exit_criteria.len(), 1);
    assert_eq!(plan_item.exit_criteria[0].id, "exitCriterionB");
    assert_eq!(plan_item.exit_criteria[0].sentry_ref, "sentryB");
}

#[test]
fn parses_entry_criterion_sentry_on_source_termination() {
    let definitions = parse_cmmn_definitions(TERMINATE_ENTRY_CRITERION_CMMN)
        .expect("terminate sentry should parse");
    let case_definition = definitions.find_case("caseA").expect("case");

    let sentry = &case_definition.case_plan_model.sentries[0];
    assert_eq!(sentry.plan_item_on_parts.len(), 1);
    assert_eq!(sentry.plan_item_on_parts[0].id, "onTaskATerminate");
    assert_eq!(sentry.plan_item_on_parts[0].source_ref, "planItemA");
    assert_eq!(sentry.plan_item_on_parts[0].standard_event, "terminate");
}

#[test]
fn parses_entry_criterion_sentry_on_event_listener_occurrence() {
    let definitions = parse_cmmn_definitions(EVENT_LISTENER_OCCUR_ENTRY_CRITERION_CMMN)
        .expect("event listener occur sentry should parse");
    let case_definition = definitions.find_case("caseA").expect("case");

    let sentry = &case_definition.case_plan_model.sentries[0];
    assert_eq!(sentry.plan_item_on_parts.len(), 1);
    assert_eq!(sentry.plan_item_on_parts[0].id, "onEventOccur");
    assert_eq!(sentry.plan_item_on_parts[0].source_ref, "planItemEvent");
    assert_eq!(sentry.plan_item_on_parts[0].standard_event, "occur");
}

#[test]
fn parses_entry_criterion_sentry_with_boolean_if_part() {
    let definitions =
        parse_cmmn_definitions(IF_PART_ENTRY_CRITERION_CMMN).expect("ifPart sentry should parse");
    let case_definition = definitions.find_case("caseA").expect("case");

    let sentry = &case_definition.case_plan_model.sentries[0];
    assert_eq!(
        sentry.if_part.as_ref(),
        Some(&SentryIfPartExpression::Comparison(SentryIfPartCondition {
            variable_name: "approved".to_string(),
            operator: SentryIfPartOperator::Equal,
            literal: SentryIfPartLiteral::Boolean(true),
        }))
    );
}

#[test]
fn parses_plan_item_manual_activation_and_repetition_rules() {
    let definitions = parse_cmmn_definitions(PLAN_ITEM_CONTROL_RULES_CMMN)
        .expect("plan item control rules should parse");
    let case_definition = definitions.find_case("caseA").expect("case");
    let plan_item = case_definition
        .find_plan_item("planItemReview")
        .expect("review plan item");

    assert_eq!(
        plan_item.manual_activation_rule.as_ref(),
        Some(&SentryIfPartExpression::Comparison(SentryIfPartCondition {
            variable_name: "manualActivation".to_string(),
            operator: SentryIfPartOperator::Equal,
            literal: SentryIfPartLiteral::Boolean(true),
        }))
    );
    assert_eq!(
        plan_item.repetition_rule.as_ref(),
        Some(&SentryIfPartExpression::Comparison(SentryIfPartCondition {
            variable_name: "repeatReview".to_string(),
            operator: SentryIfPartOperator::Equal,
            literal: SentryIfPartLiteral::Boolean(true),
        }))
    );
}

#[test]
fn parses_non_human_plan_item_manual_activation_and_repetition_rules() {
    let definitions = parse_cmmn_definitions(NON_HUMAN_PLAN_ITEM_CONTROL_RULES_CMMN)
        .expect("non-human plan item control rules should parse");
    let case_definition = definitions.find_case("caseA").expect("case");

    for (plan_item_id, manual_variable, repeat_variable) in [
        ("planItemStage", "stageManual", "stageRepeat"),
        ("planItemMilestone", "milestoneManual", "milestoneRepeat"),
        ("planItemEvent", "eventManual", "eventRepeat"),
    ] {
        let plan_item = case_definition
            .find_plan_item(plan_item_id)
            .expect("plan item");

        assert_eq!(
            plan_item.manual_activation_rule.as_ref(),
            Some(&SentryIfPartExpression::Comparison(SentryIfPartCondition {
                variable_name: manual_variable.to_string(),
                operator: SentryIfPartOperator::Equal,
                literal: SentryIfPartLiteral::Boolean(true),
            }))
        );
        assert_eq!(
            plan_item.repetition_rule.as_ref(),
            Some(&SentryIfPartExpression::Comparison(SentryIfPartCondition {
                variable_name: repeat_variable.to_string(),
                operator: SentryIfPartOperator::Equal,
                literal: SentryIfPartLiteral::Boolean(true),
            }))
        );
    }
}

#[test]
fn parses_entry_criterion_sentry_with_logical_if_part_conditions() {
    let definitions = parse_cmmn_definitions(IF_PART_LOGICAL_CONDITION_CMMN)
        .expect("logical ifPart sentries should parse");
    let case_definition = definitions.find_case("caseA").expect("case");
    let sentries = &case_definition.case_plan_model.sentries;

    assert_eq!(
        sentries[0].if_part.as_ref(),
        Some(&SentryIfPartExpression::Logical {
            operator: SentryIfPartLogicalOperator::And,
            operands: vec![
                SentryIfPartExpression::Comparison(SentryIfPartCondition {
                    variable_name: "approved".to_string(),
                    operator: SentryIfPartOperator::Equal,
                    literal: SentryIfPartLiteral::Boolean(true),
                }),
                SentryIfPartExpression::Comparison(SentryIfPartCondition {
                    variable_name: "amount".to_string(),
                    operator: SentryIfPartOperator::NotEqual,
                    literal: SentryIfPartLiteral::Number("0".to_string()),
                }),
            ],
        })
    );
    assert_eq!(
        sentries[1].if_part.as_ref(),
        Some(&SentryIfPartExpression::Logical {
            operator: SentryIfPartLogicalOperator::Or,
            operands: vec![
                SentryIfPartExpression::Comparison(SentryIfPartCondition {
                    variable_name: "status".to_string(),
                    operator: SentryIfPartOperator::Equal,
                    literal: SentryIfPartLiteral::String("approved".to_string()),
                }),
                SentryIfPartExpression::Comparison(SentryIfPartCondition {
                    variable_name: "expedited".to_string(),
                    operator: SentryIfPartOperator::Equal,
                    literal: SentryIfPartLiteral::Boolean(true),
                }),
                SentryIfPartExpression::Comparison(SentryIfPartCondition {
                    variable_name: "amount".to_string(),
                    operator: SentryIfPartOperator::Equal,
                    literal: SentryIfPartLiteral::Number("100".to_string()),
                }),
            ],
        })
    );
}

#[test]
fn parses_entry_criterion_sentry_with_advanced_if_part_condition() {
    let definitions = parse_cmmn_definitions(IF_PART_ADVANCED_CONDITION_CMMN)
        .expect("advanced ifPart sentry should parse");
    let case_definition = definitions.find_case("caseA").expect("case");
    let sentry = &case_definition.case_plan_model.sentries[0];

    assert_eq!(
        sentry.if_part.as_ref(),
        Some(&SentryIfPartExpression::Logical {
            operator: SentryIfPartLogicalOperator::Or,
            operands: vec![
                SentryIfPartExpression::Logical {
                    operator: SentryIfPartLogicalOperator::And,
                    operands: vec![
                        SentryIfPartExpression::Comparison(SentryIfPartCondition {
                            variable_name: "approved".to_string(),
                            operator: SentryIfPartOperator::Equal,
                            literal: SentryIfPartLiteral::Boolean(true),
                        }),
                        SentryIfPartExpression::Comparison(SentryIfPartCondition {
                            variable_name: "amount".to_string(),
                            operator: SentryIfPartOperator::GreaterThan,
                            literal: SentryIfPartLiteral::Number("100".to_string()),
                        }),
                    ],
                },
                SentryIfPartExpression::Comparison(SentryIfPartCondition {
                    variable_name: "reviewer".to_string(),
                    operator: SentryIfPartOperator::Equal,
                    literal: SentryIfPartLiteral::String("lead".to_string()),
                }),
            ],
        })
    );
}

#[test]
fn parses_entry_criterion_sentry_with_null_and_empty_if_part_conditions() {
    let definitions = parse_cmmn_definitions(IF_PART_NULL_EMPTY_CONDITION_CMMN)
        .expect("null and empty ifPart sentries should parse");
    let case_definition = definitions.find_case("caseA").expect("case");
    let sentries = &case_definition.case_plan_model.sentries;

    assert_eq!(
        sentries[0].if_part.as_ref(),
        Some(&SentryIfPartExpression::Comparison(SentryIfPartCondition {
            variable_name: "optionalValue".to_string(),
            operator: SentryIfPartOperator::Equal,
            literal: SentryIfPartLiteral::Null,
        }))
    );
    assert_eq!(
        sentries[1].if_part.as_ref(),
        Some(&SentryIfPartExpression::Not {
            operand: Box::new(SentryIfPartExpression::Empty {
                variable_name: "comment".to_string(),
            }),
        })
    );
}

#[test]
fn parses_entry_criterion_sentry_with_extended_if_part_conditions() {
    let definitions = parse_cmmn_definitions(IF_PART_EXTENDED_CONDITION_CMMN)
        .expect("extended ifPart sentries should parse");
    let case_definition = definitions.find_case("caseA").expect("case");
    let sentries = &case_definition.case_plan_model.sentries;

    assert_eq!(
        sentries[0].if_part.as_ref(),
        Some(&SentryIfPartExpression::Comparison(SentryIfPartCondition {
            variable_name: "status".to_string(),
            operator: SentryIfPartOperator::Equal,
            literal: SentryIfPartLiteral::String("approved".to_string()),
        }))
    );
    assert_eq!(
        sentries[1].if_part.as_ref(),
        Some(&SentryIfPartExpression::Comparison(SentryIfPartCondition {
            variable_name: "amount".to_string(),
            operator: SentryIfPartOperator::Equal,
            literal: SentryIfPartLiteral::Number("42.5".to_string()),
        }))
    );
    assert_eq!(
        sentries[2].if_part.as_ref(),
        Some(&SentryIfPartExpression::Comparison(SentryIfPartCondition {
            variable_name: "decision".to_string(),
            operator: SentryIfPartOperator::NotEqual,
            literal: SentryIfPartLiteral::String("denied".to_string()),
        }))
    );
}

#[test]
fn parses_entry_criterion_sentry_with_property_method_if_part_conditions() {
    let definitions = parse_cmmn_definitions(IF_PART_PROPERTY_METHOD_CONDITION_CMMN)
        .expect("property method ifPart sentries should parse");
    let case_definition = definitions.find_case("caseA").expect("case");
    let sentries = &case_definition.case_plan_model.sentries;

    assert_eq!(
        sentries[0].if_part.as_ref(),
        Some(&SentryIfPartExpression::Comparison(SentryIfPartCondition {
            variable_name: "size(items)".to_string(),
            operator: SentryIfPartOperator::GreaterThanOrEqual,
            literal: SentryIfPartLiteral::Variable("minimumItemCount".to_string()),
        }))
    );
    assert_eq!(
        sentries[1].if_part.as_ref(),
        Some(&SentryIfPartExpression::Comparison(SentryIfPartCondition {
            variable_name: "length(customer.name)".to_string(),
            operator: SentryIfPartOperator::GreaterThanOrEqual,
            literal: SentryIfPartLiteral::Variable("minimumNameLength".to_string()),
        }))
    );
    assert_eq!(
        sentries[2].if_part.as_ref(),
        Some(&SentryIfPartExpression::Contains {
            collection_variable_name: "customer.name + suffix".to_string(),
            value: SentryIfPartLiteral::Variable("expectedNeedle".to_string()),
            expected: true,
        })
    );
    assert_eq!(
        sentries[3].if_part.as_ref(),
        Some(&SentryIfPartExpression::Comparison(SentryIfPartCondition {
            variable_name: "length(customer.name + suffix)".to_string(),
            operator: SentryIfPartOperator::GreaterThanOrEqual,
            literal: SentryIfPartLiteral::Variable("minimumFullNameLength".to_string()),
        }))
    );
}

#[test]
fn parses_entry_criterion_sentries_on_human_task_enable_and_disable() {
    let definitions = parse_cmmn_definitions(ENABLE_DISABLE_ENTRY_CRITERION_CMMN)
        .expect("enable/disable sentries should parse");
    let case_definition = definitions.find_case("caseA").expect("case");
    let sentries = &case_definition.case_plan_model.sentries;

    assert_eq!(sentries.len(), 2);
    assert_eq!(sentries[0].plan_item_on_parts[0].source_ref, "planItemA");
    assert_eq!(sentries[0].plan_item_on_parts[0].standard_event, "disable");
    assert_eq!(sentries[1].plan_item_on_parts[0].source_ref, "planItemA");
    assert_eq!(sentries[1].plan_item_on_parts[0].standard_event, "enable");
}

#[test]
fn parses_entry_criterion_sentry_with_two_plan_item_on_parts() {
    let definitions = parse_cmmn_definitions(MULTI_ON_PART_ENTRY_CRITERION_CMMN)
        .expect("multi onPart sentry should parse");
    let case_definition = definitions.find_case("caseA").expect("case");

    let sentry = &case_definition.case_plan_model.sentries[0];
    assert_eq!(sentry.plan_item_on_parts.len(), 2);
    assert_eq!(sentry.plan_item_on_parts[0].source_ref, "planItemA");
    assert_eq!(sentry.plan_item_on_parts[1].source_ref, "planItemB");
}

#[test]
fn rejects_complex_sentry_structurally() {
    let error = parse_cmmn_definitions(COMPLEX_SENTRY_CMMN).expect_err("complex sentry must fail");

    assert!(
        error.to_string().contains("ifPart") && error.to_string().contains("owned M16 subset"),
        "unexpected error: {error}"
    );
}

#[test]
fn parses_if_part_condition_with_method_call() {
    let definitions = parse_cmmn_definitions(UNSUPPORTED_IF_PART_CONDITION_CMMN)
        .expect("method call ifPart condition must succeed");

    let case_definition = &definitions.cases[0];
    let sentry = &case_definition.case_plan_model.sentries[0];
    assert!(sentry.if_part.is_some(), "ifPart must be present");
}

const START_ENTRY_CRITERION_CMMN: &str = r#"
<definitions xmlns="http://www.omg.org/spec/CMMN/20151109/MODEL"
             targetNamespace="http://flowable.org/cmmn">
  <case id="caseA">
    <casePlanModel id="planModelA">
      <planItem id="planItemA" definitionRef="taskA" />
      <planItem id="planItemB" definitionRef="taskB">
        <entryCriterion id="entryCriterionB" sentryRef="sentryB" />
      </planItem>
      <humanTask id="taskA" />
      <humanTask id="taskB" />
      <sentry id="sentryB">
        <planItemOnPart id="onTaskAStart" sourceRef="planItemA">
          <standardEvent>start</standardEvent>
        </planItemOnPart>
      </sentry>
    </casePlanModel>
  </case>
</definitions>
"#;

const EXIT_STANDARD_EVENT_CMMN: &str = r#"
<definitions xmlns="http://www.omg.org/spec/CMMN/20151109/MODEL"
             targetNamespace="http://flowable.org/cmmn">
  <case id="caseA">
    <casePlanModel id="planModelA">
      <planItem id="planItemA" definitionRef="taskA" />
      <planItem id="planItemB" definitionRef="taskB">
        <entryCriterion id="entryCriterionB" sentryRef="sentryB" />
      </planItem>
      <humanTask id="taskA" />
      <humanTask id="taskB" />
      <sentry id="sentryB">
        <planItemOnPart id="onTaskAExit" sourceRef="planItemA">
          <standardEvent>exit</standardEvent>
        </planItemOnPart>
      </sentry>
    </casePlanModel>
  </case>
</definitions>
"#;

#[test]
fn rejects_non_local_definition_refs_structurally() {
    let error = parse_cmmn_definitions(NON_LOCAL_DEFINITION_REF_CMMN)
        .expect_err("non-local definitionRef must fail structurally");

    assert!(
        error.to_string().contains("definitionRef") && error.to_string().contains("rootTask"),
        "unexpected error: {error}"
    );
}

#[test]
fn parses_entry_criterion_sentry_on_human_task_start() {
    let definitions = parse_cmmn_definitions(START_ENTRY_CRITERION_CMMN)
        .expect("start standardEvent should parse");
    let case_definition = definitions.find_case("caseA").expect("case");

    let sentry = &case_definition.case_plan_model.sentries[0];
    assert_eq!(sentry.id, "sentryB");
    assert_eq!(sentry.plan_item_on_parts.len(), 1);
    assert_eq!(sentry.plan_item_on_parts[0].source_ref, "planItemA");
    assert_eq!(sentry.plan_item_on_parts[0].standard_event, "start");
}

#[test]
fn parses_exit_standard_event_on_human_task() {
    let definitions = parse_cmmn_definitions(EXIT_STANDARD_EVENT_CMMN)
        .expect("exit standardEvent should parse for human task sources");
    let case_definition = definitions.find_case("caseA").expect("case");

    let sentry = &case_definition.case_plan_model.sentries[0];
    assert_eq!(sentry.id, "sentryB");
    assert_eq!(sentry.plan_item_on_parts.len(), 1);
    assert_eq!(sentry.plan_item_on_parts[0].source_ref, "planItemA");
    assert_eq!(sentry.plan_item_on_parts[0].standard_event, "exit");
    assert!(
        flowable_cmmn_model::PlanItemOnPart::is_supported_standard_event("exit"),
        "model must advertise exit as supported"
    );
}
