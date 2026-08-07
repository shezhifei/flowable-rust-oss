//! C10: Human Task integration surface — flowable extension attributes.
//!
//! Java parity: `HumanTaskXmlConverter` reads the flowable extension attributes
//! (assignee/owner/priority/formKey/dueDate/category/candidateUsers/
//! candidateGroups/taskIdVariableName/taskCompleterVariableName) off the
//! `<humanTask>` element (HumanTaskXmlConverter.java:37-61), and
//! `HumanTaskActivityBehavior` copies them onto the created `TaskEntity`
//! (HumanTaskActivityBehavior.java:107-147). Candidate users/groups become
//! `IdentityLinkType.CANDIDATE` links on the task (:146-147).
//!
//! These tests exercise the full pipeline: CMMN XML -> converter -> engine
//! `CmmnModel` -> deploy -> start -> the created human task instance carries
//! the static attributes and candidate identity links. Expression resolution
//! against case variables is a known dialect gap and is not modelled: values
//! are applied verbatim.

use flowable_cmmn_engine::{
    CmmnCaseInstanceStartRequest, CmmnDeploymentRequest, CmmnEngine, CmmnHumanTaskInstance,
    CmmnHumanTaskState, CmmnModel,
};

/// Deploys the given CMMN XML through the converter -> engine model pipeline.
fn deploy_xml(engine: &CmmnEngine, xml: &str) {
    let definitions =
        flowable_cmmn_converter::parse_cmmn_definitions(xml).expect("parse cmmn definitions");
    let model = CmmnModel::from(definitions);
    engine
        .deploy(CmmnDeploymentRequest::new("c10-human-task-attributes").with_resource("c10.cmmn", model))
        .expect("deployment");
}

fn active_task(engine: &CmmnEngine, case_instance_id: &str) -> CmmnHumanTaskInstance {
    engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(case_instance_id)
        .state(CmmnHumanTaskState::Active)
        .list()
        .expect("active tasks")
        .into_iter()
        .next()
        .expect("one active task")
}

/// A blocking human task declaring assignee/owner/priority/dueDate/category via
/// flowable extension attributes has those values copied onto the created task
/// instance (HumanTaskActivityBehavior.java:107-147).
#[test]
fn flowable_attributes_applied_to_active_task_instance() {
    const XML: &str = r#"
<definitions xmlns="http://www.omg.org/spec/CMMN/20151109/MODEL"
             xmlns:flowable="http://flowable.org/cmmn"
             targetNamespace="http://flowable.org/cmmn">
  <case id="c10AttributesCase" name="C10 attributes case">
    <casePlanModel id="planModel" name="Plan Model">
      <planItem id="planItemA" name="Review" definitionRef="taskA" />
      <humanTask id="taskA" name="Review"
                 flowable:assignee="alice"
                 flowable:owner="bob"
                 flowable:priority="42"
                 flowable:dueDate="2026-08-01T00:00:00Z"
                 flowable:category="urgent" />
    </casePlanModel>
  </case>
</definitions>
"#;

    let engine = CmmnEngine::new_in_memory().expect("engine");
    deploy_xml(&engine, XML);

    let case_instance = engine
        .start_case_instance_by_key("c10AttributesCase", CmmnCaseInstanceStartRequest::new())
        .expect("case instance");

    let task = active_task(&engine, &case_instance.id);
    assert_eq!(task.assignee.as_deref(), Some("alice"));
    assert_eq!(task.owner.as_deref(), Some("bob"));
    assert_eq!(task.priority.as_deref(), Some("42"));
    assert_eq!(task.due_date.as_deref(), Some("2026-08-01T00:00:00Z"));
    assert_eq!(task.category.as_deref(), Some("urgent"));
}

/// Candidate users and groups are comma-delimited lists
/// (CmmnXmlUtil.parseDelimitedList) that become `candidate` identity links
/// scoped to the created human task (HumanTaskActivityBehavior.java:146-147).
#[test]
fn candidate_users_and_groups_become_humantask_identity_links() {
    const XML: &str = r#"
<definitions xmlns="http://www.omg.org/spec/CMMN/20151109/MODEL"
             xmlns:flowable="http://flowable.org/cmmn"
             targetNamespace="http://flowable.org/cmmn">
  <case id="c10CandidatesCase" name="C10 candidates case">
    <casePlanModel id="planModel" name="Plan Model">
      <planItem id="planItemA" name="Approve" definitionRef="taskA" />
      <humanTask id="taskA" name="Approve"
                 flowable:candidateUsers="alice, bob"
                 flowable:candidateGroups="managers,auditors" />
    </casePlanModel>
  </case>
</definitions>
"#;

    let engine = CmmnEngine::new_in_memory().expect("engine");
    deploy_xml(&engine, XML);

    let case_instance = engine
        .start_case_instance_by_key("c10CandidatesCase", CmmnCaseInstanceStartRequest::new())
        .expect("case instance");

    let task = active_task(&engine, &case_instance.id);

    let links = engine
        .identity_link_service()
        .list_identity_links("humanTask", &task.id)
        .expect("identity links");

    let mut users: Vec<String> = links
        .iter()
        .filter(|link| link.link_type == "candidate")
        .filter_map(|link| link.user_id.clone())
        .collect();
    users.sort();
    let mut groups: Vec<String> = links
        .iter()
        .filter(|link| link.link_type == "candidate")
        .filter_map(|link| link.group_id.clone())
        .collect();
    groups.sort();

    assert_eq!(users, vec!["alice".to_string(), "bob".to_string()]);
    assert_eq!(
        groups,
        vec!["auditors".to_string(), "managers".to_string()]
    );
}

/// A human task without flowable extension attributes still deploys and its
/// created instance carries no assignee/owner and no candidate links, so the
/// new parsing path does not regress the bare-task case.
#[test]
fn bare_human_task_has_no_attributes_or_candidate_links() {
    const XML: &str = r#"
<definitions xmlns="http://www.omg.org/spec/CMMN/20151109/MODEL"
             targetNamespace="http://flowable.org/cmmn">
  <case id="c10BareCase" name="C10 bare case">
    <casePlanModel id="planModel" name="Plan Model">
      <planItem id="planItemA" name="Plain" definitionRef="taskA" />
      <humanTask id="taskA" name="Plain" />
    </casePlanModel>
  </case>
</definitions>
"#;

    let engine = CmmnEngine::new_in_memory().expect("engine");
    deploy_xml(&engine, XML);

    let case_instance = engine
        .start_case_instance_by_key("c10BareCase", CmmnCaseInstanceStartRequest::new())
        .expect("case instance");

    let task = active_task(&engine, &case_instance.id);
    assert_eq!(task.assignee, None);
    assert_eq!(task.owner, None);
    assert_eq!(task.priority, None);
    assert_eq!(task.due_date, None);
    assert_eq!(task.category, None);

    let links = engine
        .identity_link_service()
        .list_identity_links("humanTask", &task.id)
        .expect("identity links");
    assert!(links.is_empty());
}
