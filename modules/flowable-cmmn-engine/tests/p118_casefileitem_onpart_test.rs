//! P118 — end-to-end: CMMN XML with caseFileItemOnPart deploys and fires sentry
//! when a matching case file item event occurs.
//!
//! Converter parses `caseFileItemOnPart` (sourceRef → case_file_item_ref,
//! standardEvent child). Engine `From<Sentry>` maps the field through.
//! Runtime: `create_case_file_item` → `handle_case_file_item_on_part` with
//! `STANDARD_EVENT_CREATE` (runtime.rs case-file service path).

use flowable_cmmn_engine::{
    CmmnCaseFileItem, CmmnCaseInstanceStartRequest, CmmnEngine, CmmnHumanTaskState,
};

const CASE_FILE_ITEM_ON_PART_XML: &str = r#"
<definitions xmlns="http://www.omg.org/spec/CMMN/20151109/MODEL"
             targetNamespace="http://flowable.org/cmmn">
  <case id="caseFileOnPartCase" name="Case file onPart case">
    <caseFileModel>
      <caseFileItemDefinition id="documentDef" name="Document"
        definitionType="http://www.omg.org/spec/CMMN/DefinitionType/CMISDocument" />
      <caseFileItem id="document" name="Document" definitionRef="documentDef" />
    </caseFileModel>
    <casePlanModel id="planModel">
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
fn xml_case_file_item_on_part_create_triggers_downstream_task() {
    let engine = CmmnEngine::new_in_memory().expect("engine");

    engine
        .repository_service()
        .new_deployment()
        .name("p118-casefile-onpart")
        .add_string("case-file-onpart.cmmn", CASE_FILE_ITEM_ON_PART_XML)
        .expect("add cmmn")
        .deploy()
        .expect("deploy XML with caseFileItemOnPart");

    let case_instance = engine
        .start_case_instance_by_key("caseFileOnPartCase", CmmnCaseInstanceStartRequest::new())
        .expect("start case");

    let active_before = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_instance.id)
        .state(CmmnHumanTaskState::Active)
        .list()
        .expect("active tasks before");
    assert_eq!(active_before.len(), 1);
    assert_eq!(active_before[0].name, "Intake");

    // Runtime matches case_file_item_ref against definition_ref ancestry
    // (CaseFileGraph::ancestry_definition_refs). Use id == sourceRef so the
    // default definition_ref equals the XML sourceRef ("document").
    let document = CmmnCaseFileItem::new("document", "Document");
    engine
        .runtime_service()
        .case_file_item_service()
        .create_case_file_item(&case_instance.id, document)
        .expect("create case file item");

    let active_after = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_instance.id)
        .state(CmmnHumanTaskState::Active)
        .list()
        .expect("active tasks after");
    assert_eq!(
        active_after.len(),
        2,
        "create on case file item should fire caseFileItemOnPart sentry and activate Review"
    );
    assert!(active_after.iter().any(|t| t.name == "Review"));
}

#[test]
fn xml_case_file_item_on_part_update_triggers_downstream_task() {
    let xml = r#"
<definitions xmlns="http://www.omg.org/spec/CMMN/20151109/MODEL"
             targetNamespace="http://flowable.org/cmmn">
  <case id="caseFileUpdateOnPart" name="Update onPart">
    <casePlanModel id="planModel">
      <planItem id="planItemIntake" definitionRef="taskIntake" />
      <planItem id="planItemReview" definitionRef="taskReview">
        <entryCriterion id="entryReview" sentryRef="sentryDocUpdate" />
      </planItem>
      <humanTask id="taskIntake" name="Intake" />
      <humanTask id="taskReview" name="Review" />
      <sentry id="sentryDocUpdate">
        <caseFileItemOnPart id="onDocUpdate" sourceRef="document">
          <standardEvent>update</standardEvent>
        </caseFileItemOnPart>
      </sentry>
    </casePlanModel>
  </case>
</definitions>
"#;

    let engine = CmmnEngine::new_in_memory().expect("engine");
    engine
        .repository_service()
        .new_deployment()
        .name("p118-casefile-update")
        .add_string("update.cmmn", xml)
        .expect("add")
        .deploy()
        .expect("deploy");

    let case_instance = engine
        .start_case_instance_by_key("caseFileUpdateOnPart", CmmnCaseInstanceStartRequest::new())
        .expect("start");

    let case_file = engine.runtime_service().case_file_item_service();
    case_file
        .create_case_file_item(
            &case_instance.id,
            CmmnCaseFileItem::new("document", "Document"),
        )
        .expect("create");

    let active = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_instance.id)
        .state(CmmnHumanTaskState::Active)
        .list()
        .expect("list");
    assert_eq!(active.len(), 1, "create must not fire update sentry");

    case_file
        .update_case_file_item(
            &case_instance.id,
            "document",
            serde_json::json!({"status": "ready"}),
        )
        .expect("update");

    let active = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_instance.id)
        .state(CmmnHumanTaskState::Active)
        .list()
        .expect("list after update");
    assert_eq!(active.len(), 2);
    assert!(active.iter().any(|t| t.name == "Review"));
}

#[test]
fn xml_case_file_item_on_part_mismatched_ref_does_not_trigger() {
    let xml = r#"
<definitions xmlns="http://www.omg.org/spec/CMMN/20151109/MODEL"
             targetNamespace="http://flowable.org/cmmn">
  <case id="caseFileMismatch" name="Mismatch">
    <casePlanModel id="planModel">
      <planItem id="planItemIntake" definitionRef="taskIntake" />
      <planItem id="planItemReview" definitionRef="taskReview">
        <entryCriterion id="entryReview" sentryRef="sentryOnlyDocA" />
      </planItem>
      <humanTask id="taskIntake" name="Intake" />
      <humanTask id="taskReview" name="Review" />
      <sentry id="sentryOnlyDocA">
        <caseFileItemOnPart id="onDocA" sourceRef="documentA">
          <standardEvent>create</standardEvent>
        </caseFileItemOnPart>
      </sentry>
    </casePlanModel>
  </case>
</definitions>
"#;

    let engine = CmmnEngine::new_in_memory().expect("engine");
    engine
        .repository_service()
        .new_deployment()
        .name("p118-mismatch")
        .add_string("mismatch.cmmn", xml)
        .expect("add")
        .deploy()
        .expect("deploy");

    let case_instance = engine
        .start_case_instance_by_key("caseFileMismatch", CmmnCaseInstanceStartRequest::new())
        .expect("start");

    engine
        .runtime_service()
        .case_file_item_service()
        .create_case_file_item(
            &case_instance.id,
            CmmnCaseFileItem::new("documentB", "Document B"),
        )
        .expect("create B");

    let active = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_instance.id)
        .state(CmmnHumanTaskState::Active)
        .list()
        .expect("list");
    assert_eq!(active.len(), 1);
    assert_eq!(active[0].name, "Intake");
}
