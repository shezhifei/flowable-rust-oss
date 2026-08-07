//! P114: CMMN human task candidate query — candidateUser / candidateGroup /
//! candidateGroupIn / candidateOrAssigned / ignoreAssignee, with identity group
//! expansion.
//!
//! Java parity: TaskQueryImpl candidate setters (TaskQueryImpl.java:576-687) and
//! the Task.xml candidate SQL blocks (Task.xml:867-896, 1090-1131). The Rust CMMN
//! engine has no identity store, so the user→groups expansion for
//! candidateUser / candidateOrAssigned is supplied per-query via
//! `user_group_resolver` (Java TaskQueryImpl.getGroupsForCandidateUser,
//! TaskQueryImpl.java:2021-2032); without one, only direct user links match.

use flowable_cmmn_engine::{
    CmmnCaseInstanceStartRequest, CmmnDeploymentRequest, CmmnEngine, CmmnHumanTaskState,
    CmmnModel, CmmnUserGroupResolver,
};
use std::sync::Arc;

/// A blocking human task with candidate users/groups, plus a group-only task and
/// a bare task. Candidate identity links are created for active tasks (C10,
/// HumanTaskActivityBehavior.java:146-147).
const XML: &str = r#"
<definitions xmlns="http://www.omg.org/spec/CMMN/20151109/MODEL"
             xmlns:flowable="http://flowable.org/cmmn"
             targetNamespace="http://flowable.org/cmmn">
  <case id="p114CandidateCase" name="P114 candidate case">
    <casePlanModel id="planModel" name="Plan Model">
      <planItem id="planItemReview" name="Review" definitionRef="reviewTask" />
      <planItem id="planItemApprove" name="Approve" definitionRef="approveTask" />
      <planItem id="planItemAudit" name="Audit" definitionRef="auditTask" />
      <humanTask id="reviewTask" name="Review"
                 flowable:candidateUsers="alice, bob"
                 flowable:candidateGroups="managers,auditors" />
      <humanTask id="approveTask" name="Approve"
                 flowable:candidateGroups="sales" />
      <humanTask id="auditTask" name="Audit" />
    </casePlanModel>
  </case>
</definitions>
"#;

/// Identity fixture backing the group expansion: charlie/carol belong to
/// `managers` (a candidate group on the Review task); nobody else has groups.
fn identity_resolver() -> CmmnUserGroupResolver {
    Arc::new(|user_id: &str| match user_id {
        "charlie" | "carol" => vec!["managers".to_string()],
        _ => Vec::new(),
    })
}

fn deploy_and_start(engine: &CmmnEngine) -> String {
    let definitions =
        flowable_cmmn_converter::parse_cmmn_definitions(XML).expect("parse cmmn definitions");
    let model = CmmnModel::from(definitions);
    engine
        .deploy(
            CmmnDeploymentRequest::new("p114-candidate-query").with_resource("p114.cmmn", model),
        )
        .expect("deployment");
    engine
        .start_case_instance_by_key("p114CandidateCase", CmmnCaseInstanceStartRequest::new())
        .expect("case instance")
        .id
}

fn task_names(engine: &CmmnEngine, case_instance_id: &str) -> Vec<String> {
    engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(case_instance_id)
        .list()
        .expect("task list")
        .into_iter()
        .map(|task| task.name)
        .collect()
}

/// candidateUser matches a task with a direct candidate link for that user
/// (TaskQueryImpl.java:576-588 / Task.xml:867-896).
#[test]
fn candidate_user_direct_link_hits() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let case_instance_id = deploy_and_start(&engine);
    assert_eq!(
        task_names(&engine, &case_instance_id),
        vec!["Review", "Approve", "Audit"]
    );

    let tasks = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_instance_id)
        .candidate_user("alice")
        .user_group_resolver(identity_resolver())
        .list()
        .expect("candidateUser query");
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].name, "Review");

    // Unknown user → empty result.
    let tasks = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_instance_id)
        .candidate_user("nobody")
        .user_group_resolver(identity_resolver())
        .list()
        .expect("candidateUser miss");
    assert!(tasks.is_empty());
}

/// candidateUser expands the user's group memberships: a user with no direct
/// link hits a task whose candidate group they belong to
/// (TaskQueryImpl.getGroupsForCandidateUser, TaskQueryImpl.java:2021-2032).
#[test]
fn candidate_user_group_expansion_hits() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let case_instance_id = deploy_and_start(&engine);

    let tasks = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_instance_id)
        .candidate_user("charlie")
        .user_group_resolver(identity_resolver())
        .list()
        .expect("group-expanded candidateUser query");
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].name, "Review");

    // Without a resolver the same user only matches direct links → empty.
    let tasks = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_instance_id)
        .candidate_user("charlie")
        .list()
        .expect("candidateUser without resolver");
    assert!(tasks.is_empty());
}

/// candidateGroup matches tasks with a candidate link for the given group
/// (TaskQueryImpl.java:620-635).
#[test]
fn candidate_group_hits() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let case_instance_id = deploy_and_start(&engine);

    let tasks = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_instance_id)
        .candidate_group("sales")
        .list()
        .expect("candidateGroup query");
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].name, "Approve");
}

/// candidateGroupIn matches any task with a candidate link for one of the groups
/// (TaskQueryImpl.java:658-677).
#[test]
fn candidate_group_in_multi_group_hits() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let case_instance_id = deploy_and_start(&engine);

    let tasks = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_instance_id)
        .candidate_group_in(vec!["sales".to_string(), "auditors".to_string()])
        .list()
        .expect("candidateGroupIn query");
    let mut names: Vec<String> = tasks.into_iter().map(|task| task.name).collect();
    names.sort();
    assert_eq!(names, vec!["Approve", "Review"]);
}

/// candidateOrAssigned matches a task assigned to the user OR a task for which
/// the user is a candidate (directly or via groups)
/// (TaskQueryImpl.java:638-655 / Task.xml:1090-1131).
#[test]
fn candidate_or_assigned_matches_assignee_and_candidate() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let case_instance_id = deploy_and_start(&engine);

    // Claim the bare Audit task for carol.
    let audit = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_instance_id)
        .name("Audit")
        .single_result()
        .expect("audit query")
        .expect("audit task");
    engine
        .runtime_service()
        .claim_human_task(&audit.id, "carol")
        .expect("claim");

    // carol: assigned to Audit, candidate via the managers group on Review.
    let tasks = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_instance_id)
        .candidate_or_assigned("carol")
        .user_group_resolver(identity_resolver())
        .list()
        .expect("candidateOrAssigned query");
    let mut names: Vec<String> = tasks.into_iter().map(|task| task.name).collect();
    names.sort();
    assert_eq!(names, vec!["Audit", "Review"]);
}

/// Default candidate queries exclude already-assigned tasks; ignoreAssigneeValue
/// keeps them (Task.xml:868-870).
#[test]
fn ignore_assignee_excludes_assigned_unless_ignored() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let case_instance_id = deploy_and_start(&engine);

    // Claim the Review task (it keeps its candidate links).
    let review = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_instance_id)
        .name("Review")
        .single_result()
        .expect("review query")
        .expect("review task");
    engine
        .runtime_service()
        .claim_human_task(&review.id, "zed")
        .expect("claim");

    // Default: assigned candidate task is excluded.
    let tasks = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_instance_id)
        .candidate_user("alice")
        .user_group_resolver(identity_resolver())
        .list()
        .expect("default candidateUser query");
    assert!(tasks.is_empty());

    // ignoreAssigneeValue: the assigned candidate task is kept.
    let tasks = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_instance_id)
        .candidate_user("alice")
        .user_group_resolver(identity_resolver())
        .ignore_assignee_value()
        .list()
        .expect("ignoreAssignee candidateUser query");
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].name, "Review");
}

/// candidateOrAssigned with a claimed (non-matching) assignee respects the same
/// assignee gate on the candidate arm.
#[test]
fn candidate_or_assigned_gate_applies_unless_ignored() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let case_instance_id = deploy_and_start(&engine);

    // Claim Review for someone other than carol.
    let review = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_instance_id)
        .name("Review")
        .single_result()
        .expect("review query")
        .expect("review task");
    engine
        .runtime_service()
        .claim_human_task(&review.id, "zed")
        .expect("claim");

    // carol is a candidate on Review via managers, but it is assigned to zed →
    // default candidateOrAssigned excludes it.
    let tasks = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_instance_id)
        .candidate_or_assigned("carol")
        .user_group_resolver(identity_resolver())
        .list()
        .expect("default candidateOrAssigned query");
    assert!(tasks.is_empty());

    // ignoreAssigneeValue keeps it.
    let tasks = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_instance_id)
        .candidate_or_assigned("carol")
        .user_group_resolver(identity_resolver())
        .ignore_assignee_value()
        .list()
        .expect("ignoreAssignee candidateOrAssigned query");
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].name, "Review");
}

/// State filtering composes with candidate filtering (all three tasks active
/// here, so state=Active is a no-op guard against regression).
#[test]
fn candidate_query_composes_with_state() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let case_instance_id = deploy_and_start(&engine);

    let tasks = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_instance_id)
        .state(CmmnHumanTaskState::Active)
        .candidate_group_in(vec!["sales".to_string()])
        .list()
        .expect("candidate + state query");
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].name, "Approve");
}
