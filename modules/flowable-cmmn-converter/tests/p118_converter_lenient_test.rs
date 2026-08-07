//! P118 — CMMN converter leniency + caseFileItemOnPart parse.
//!
//! Java reference:
//! - Unknown elements: `CmmnXmlConverter.java:222-226` (skip unregistered local names)
//! - Unknown attributes: individual converters only read known attrs (never reject)
//! - caseFileItemOnPart XSD: `CMMN11CaseModel.xsd:1027-1042`
//!
//! Note: Java open-source has no `CaseFileItemOnPart` converter registered
//! (`CmmnXmlConverter.java:96-141`), so Java silently drops it. Rust parses it
//! because the engine already evaluates `case_file_item_on_parts`.

use flowable_cmmn_converter::parse_cmmn_definitions;
use flowable_cmmn_model::CaseFileItemOnPart;

fn parse(xml: &str) -> flowable_cmmn_model::CmmnDefinitions {
    parse_cmmn_definitions(xml).unwrap_or_else(|e| panic!("expected parse success, got: {e}"))
}

// ─── Lenient: previously rejected, now deployable ───────────────────────────

#[test]
fn documentation_and_extension_elements_are_skipped() {
    // Java has DocumentationXmlConverter + ExtensionElementsXMLConverter; even without
    // full parity we must not 400 the model (CmmnXmlConverter.java:222-226).
    let definitions = parse(
        r#"<definitions xmlns="http://www.omg.org/spec/CMMN/20151109/MODEL"
             xmlns:flowable="http://flowable.org/cmmn"
             targetNamespace="http://flowable.org/cmmn">
  <documentation>Top-level docs</documentation>
  <case id="caseA" name="Case A">
    <documentation textFormat="text/plain">Case docs</documentation>
    <extensionElements>
      <flowable:caseLifecycleListener event="complete" class="com.example.Listener" />
    </extensionElements>
    <casePlanModel id="planModelA">
      <humanTask id="taskA" name="Task A" />
    </casePlanModel>
  </case>
</definitions>"#,
    );
    assert_eq!(definitions.cases.len(), 1);
    assert_eq!(definitions.cases[0].id, "caseA");
    assert_eq!(definitions.cases[0].case_plan_model.human_tasks.len(), 1);
}

#[test]
fn unknown_attribute_on_case_and_plan_item_is_ignored() {
    let definitions = parse(
        r#"<definitions xmlns="http://www.omg.org/spec/CMMN/20151109/MODEL"
             xmlns:flowable="http://flowable.org/cmmn"
             targetNamespace="http://flowable.org/cmmn"
             exporter="flowable" exporterVersion="7">
  <case id="caseA" flowable:candidateStarterUsers="kermit">
    <casePlanModel id="planModelA" flowable:autoCompleteCondition="${true}">
      <planItem id="piA" definitionRef="taskA" flowable:displayOrder="1" />
      <humanTask id="taskA" flowable:formFieldValidation="true" />
    </casePlanModel>
  </case>
</definitions>"#,
    );
    let case_def = &definitions.cases[0];
    assert_eq!(case_def.id, "caseA");
    assert_eq!(case_def.case_plan_model.plan_items[0].id, "piA");
    assert_eq!(case_def.case_plan_model.human_tasks[0].id, "taskA");
}

#[test]
fn process_and_decision_references_at_definitions_level_are_skipped() {
    // Java ProcessXmlConverter / DecisionXmlConverter exist; Rust has no model surface
    // for them yet — skip rather than reject so cases still deploy.
    let definitions = parse(
        r#"<definitions xmlns="http://www.omg.org/spec/CMMN/20151109/MODEL"
             targetNamespace="http://flowable.org/cmmn">
  <process id="proc1" name="P" implementationType="http://www.omg.org/spec/CMMN/ProcessType/Unspecified" externalRef="x" />
  <decision id="dec1" name="D" implementationType="http://www.omg.org/spec/CMMN/DecisionType/Unspecified" externalRef="y" />
  <case id="caseA">
    <casePlanModel id="planModelA">
      <humanTask id="taskA" />
    </casePlanModel>
  </case>
</definitions>"#,
    );
    assert_eq!(definitions.cases.len(), 1);
}

#[test]
fn plan_fragment_and_default_control_are_skipped() {
    let definitions = parse(
        r#"<definitions xmlns="http://www.omg.org/spec/CMMN/20151109/MODEL"
             targetNamespace="http://flowable.org/cmmn">
  <case id="caseA">
    <casePlanModel id="planModelA">
      <planFragment id="frag1" name="Fragment">
        <planItem id="piHidden" definitionRef="taskHidden" />
      </planFragment>
      <defaultControl>
        <requiredRule />
      </defaultControl>
      <planItem id="piA" definitionRef="taskA" />
      <humanTask id="taskA" />
    </casePlanModel>
  </case>
</definitions>"#,
    );
    let plan = &definitions.cases[0].case_plan_model;
    assert_eq!(plan.plan_items.len(), 1);
    assert_eq!(plan.plan_items[0].id, "piA");
    assert_eq!(plan.human_tasks.len(), 1);
}

#[test]
fn human_task_with_extension_elements_child_still_parses() {
    let definitions = parse(
        r#"<definitions xmlns="http://www.omg.org/spec/CMMN/20151109/MODEL"
             xmlns:flowable="http://flowable.org/cmmn"
             targetNamespace="http://flowable.org/cmmn">
  <case id="caseA">
    <casePlanModel id="planModelA">
      <planItem id="piA" definitionRef="taskA" />
      <humanTask id="taskA" name="Review" assignee="kermit">
        <extensionElements>
          <flowable:taskListener event="create" class="com.example.Listener" />
        </extensionElements>
      </humanTask>
    </casePlanModel>
  </case>
</definitions>"#,
    );
    let task = &definitions.cases[0].case_plan_model.human_tasks[0];
    assert_eq!(task.id, "taskA");
    assert_eq!(task.assignee.as_deref(), Some("kermit"));
}

#[test]
fn text_annotation_and_association_are_skipped() {
    let definitions = parse(
        r#"<definitions xmlns="http://www.omg.org/spec/CMMN/20151109/MODEL"
             targetNamespace="http://flowable.org/cmmn">
  <case id="caseA">
    <casePlanModel id="planModelA">
      <humanTask id="taskA" />
    </casePlanModel>
  </case>
  <textAnnotation id="note1">
    <text>A note</text>
  </textAnnotation>
  <association id="assoc1" sourceRef="taskA" targetRef="note1" />
</definitions>"#,
    );
    assert_eq!(definitions.cases.len(), 1);
}

// ─── Still structural errors (Java also fails / required structure) ─────────

#[test]
fn still_rejects_missing_required_id() {
    let err = parse_cmmn_definitions(
        r#"<definitions xmlns="http://www.omg.org/spec/CMMN/20151109/MODEL" targetNamespace="x">
  <case>
    <casePlanModel id="p"><humanTask id="t" /></casePlanModel>
  </case>
</definitions>"#,
    )
    .expect_err("case without id must fail");
    assert!(
        err.to_string().contains("id"),
        "unexpected error: {err}"
    );
}

#[test]
fn still_rejects_empty_if_part() {
    // Empty <ifPart/> has no condition — structural, not "unknown element".
    let err = parse_cmmn_definitions(
        r#"<definitions xmlns="http://www.omg.org/spec/CMMN/20151109/MODEL" targetNamespace="x">
  <case id="caseA">
    <casePlanModel id="planModelA">
      <humanTask id="taskA" />
      <sentry id="sentryA"><ifPart /></sentry>
    </casePlanModel>
  </case>
</definitions>"#,
    )
    .expect_err("empty ifPart must fail");
    assert!(
        err.to_string().contains("ifPart") && err.to_string().contains("condition"),
        "unexpected error: {err}"
    );
}

// ─── caseFileItemOnPart parse ───────────────────────────────────────────────

const CASE_FILE_ITEM_ON_PART_CMMN: &str = r#"
<definitions xmlns="http://www.omg.org/spec/CMMN/20151109/MODEL"
             targetNamespace="http://flowable.org/cmmn">
  <case id="caseA">
    <caseFileModel>
      <caseFileItemDefinition id="documentDef" name="Document"
        definitionType="http://www.omg.org/spec/CMMN/DefinitionType/CMISDocument" />
      <caseFileItem id="document" name="Document" definitionRef="documentDef" />
    </caseFileModel>
    <casePlanModel id="planModelA">
      <planItem id="planItemIntake" definitionRef="taskIntake" />
      <planItem id="planItemReview" definitionRef="taskReview">
        <entryCriterion id="entryReview" sentryRef="sentryDocCreate" />
      </planItem>
      <humanTask id="taskIntake" name="Intake" />
      <humanTask id="taskReview" name="Review" />
      <sentry id="sentryDocCreate">
        <caseFileItemOnPart id="onDocCreate" sourceRef="document">
          <standardEvent>create</standardEvent>
        </caseFileItemOnPart>
      </sentry>
    </casePlanModel>
  </case>
</definitions>
"#;

#[test]
fn parses_case_file_item_on_part_into_sentry() {
    let definitions = parse(CASE_FILE_ITEM_ON_PART_CMMN);
    let sentry = &definitions.cases[0].case_plan_model.sentries[0];
    assert_eq!(sentry.id, "sentryDocCreate");
    assert!(sentry.plan_item_on_parts.is_empty());
    assert_eq!(sentry.case_file_item_on_parts.len(), 1);
    let on_part = &sentry.case_file_item_on_parts[0];
    assert_eq!(on_part.id, "onDocCreate");
    // XSD sourceRef → model case_file_item_ref (CMMN11CaseModel.xsd:1034-1039).
    assert_eq!(on_part.case_file_item_ref, "document");
    assert_eq!(on_part.standard_event, CaseFileItemOnPart::STANDARD_EVENT_CREATE);
}

#[test]
fn parses_case_file_item_on_part_update_delete_complete() {
    for event in ["update", "delete", "complete"] {
        let xml = format!(
            r#"<definitions xmlns="http://www.omg.org/spec/CMMN/20151109/MODEL" targetNamespace="x">
  <case id="caseA">
    <casePlanModel id="planModelA">
      <planItem id="piA" definitionRef="taskA">
        <entryCriterion id="ec" sentryRef="s1" />
      </planItem>
      <humanTask id="taskA" />
      <sentry id="s1">
        <caseFileItemOnPart id="on" sourceRef="doc">
          <standardEvent>{event}</standardEvent>
        </caseFileItemOnPart>
      </sentry>
    </casePlanModel>
  </case>
</definitions>"#
        );
        let definitions = parse(&xml);
        let on_part = &definitions.cases[0].case_plan_model.sentries[0].case_file_item_on_parts[0];
        assert_eq!(on_part.standard_event, event);
        assert_eq!(on_part.case_file_item_ref, "doc");
    }
}

#[test]
fn rejects_case_file_item_on_part_missing_source_ref() {
    let err = parse_cmmn_definitions(
        r#"<definitions xmlns="http://www.omg.org/spec/CMMN/20151109/MODEL" targetNamespace="x">
  <case id="caseA">
    <casePlanModel id="planModelA">
      <humanTask id="taskA" />
      <sentry id="s1">
        <caseFileItemOnPart id="on">
          <standardEvent>create</standardEvent>
        </caseFileItemOnPart>
      </sentry>
    </casePlanModel>
  </case>
</definitions>"#,
    )
    .expect_err("missing sourceRef must fail");
    assert!(
        err.to_string().contains("sourceRef"),
        "unexpected error: {err}"
    );
}

#[test]
fn rejects_case_file_item_on_part_unsupported_standard_event() {
    let err = parse_cmmn_definitions(
        r#"<definitions xmlns="http://www.omg.org/spec/CMMN/20151109/MODEL" targetNamespace="x">
  <case id="caseA">
    <casePlanModel id="planModelA">
      <humanTask id="taskA" />
      <sentry id="s1">
        <caseFileItemOnPart id="on" sourceRef="doc">
          <standardEvent>addChild</standardEvent>
        </caseFileItemOnPart>
      </sentry>
    </casePlanModel>
  </case>
</definitions>"#,
    )
    .expect_err("addChild not in engine-supported set must fail");
    assert!(
        err.to_string().contains("addChild") || err.to_string().contains("unsupported"),
        "unexpected error: {err}"
    );
}

#[test]
fn sentry_with_only_case_file_item_on_part_is_valid() {
    let definitions = parse(
        r#"<definitions xmlns="http://www.omg.org/spec/CMMN/20151109/MODEL" targetNamespace="x">
  <case id="caseA">
    <casePlanModel id="planModelA">
      <planItem id="piA" definitionRef="taskA">
        <entryCriterion id="ec" sentryRef="s1" />
      </planItem>
      <humanTask id="taskA" />
      <sentry id="s1">
        <caseFileItemOnPart id="on" sourceRef="doc">
          <standardEvent>create</standardEvent>
        </caseFileItemOnPart>
      </sentry>
    </casePlanModel>
  </case>
</definitions>"#,
    );
    let sentry = &definitions.cases[0].case_plan_model.sentries[0];
    assert!(sentry.plan_item_on_parts.is_empty());
    assert_eq!(sentry.case_file_item_on_parts.len(), 1);
}
