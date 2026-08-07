//! P120 — CMMN historic query parameter surface.
//!
//! Covers the high-frequency `HistoricCaseInstanceCollectionResource` /
//! `HistoricTaskInstanceCollectionResource` parameter subset, plus the
//! history-side candidate/involved identity-link filters.

use std::sync::Arc;

use chrono::{Duration, Utc};
use flowable_cmmn_engine::{
    CmmnCase, CmmnCaseInstanceStartRequest, CmmnCaseInstanceState, CmmnCasePlanModel,
    CmmnDeploymentRequest, CmmnEngine, CmmnHumanTask, CmmnHumanTaskCompletionRequest,
    CmmnHumanTaskState, CmmnModel, CmmnPlanItem,
};

fn plain_model() -> CmmnModel {
    let plan_model = CmmnCasePlanModel::new("case-plan-model", "Case plan model")
        .with_human_task(CmmnHumanTask::new("human-task-review", "Review application"))
        .with_plan_item(CmmnPlanItem::new("plan-item-review", "human-task-review"));

    CmmnModel::new(vec![CmmnCase::new(
        "case-p120",
        "p120Case",
        "P120 case",
        plan_model,
    )])
}

fn candidate_model() -> CmmnModel {
    let plan_model = CmmnCasePlanModel::new("case-plan-model", "Case plan model")
        .with_human_task(
            CmmnHumanTask::new("human-task-candidate", "Candidate review")
                .with_candidate_users(vec!["kermit".to_string()])
                .with_candidate_groups(vec!["sales".to_string(), "support".to_string()]),
        )
        .with_plan_item(CmmnPlanItem::new(
            "plan-item-candidate",
            "human-task-candidate",
        ));

    CmmnModel::new(vec![CmmnCase::new(
        "case-p120-candidate",
        "p120CandidateCase",
        "P120 candidate case",
        plan_model,
    )])
}

fn assigned_model() -> CmmnModel {
    let plan_model = CmmnCasePlanModel::new("case-plan-model", "Case plan model")
        .with_human_task(
            CmmnHumanTask::new("human-task-assigned", "Assigned review")
                .with_assignee("fozzie")
                .with_owner("gonzo")
                .with_category("finance")
                .with_candidate_groups(vec!["sales".to_string()]),
        )
        .with_plan_item(CmmnPlanItem::new(
            "plan-item-assigned",
            "human-task-assigned",
        ));

    CmmnModel::new(vec![CmmnCase::new(
        "case-p120-assigned",
        "p120AssignedCase",
        "P120 assigned case",
        plan_model,
    )])
}

fn deploy_all(engine: &CmmnEngine) {
    engine
        .deploy(
            CmmnDeploymentRequest::new("p120")
                .with_category("finance-cases")
                .with_resource("p120.cmmn", plain_model())
                .with_resource("p120-candidate.cmmn", candidate_model())
                .with_resource("p120-assigned.cmmn", assigned_model()),
        )
        .expect("deployment");
}

/// `start_case_instance_by_key` resolves the definition within the request's
/// tenant, so a tenant-scoped case needs its own tenant-scoped deployment.
fn deploy_tenant(engine: &CmmnEngine, tenant_id: &str) {
    engine
        .deploy(
            CmmnDeploymentRequest::new("p120-tenant")
                .with_tenant_id(tenant_id)
                .with_resource("p120.cmmn", plain_model()),
        )
        .expect("tenant deployment");
}

fn active_task_id(engine: &CmmnEngine, case_instance_id: &str) -> String {
    engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(case_instance_id)
        .state(CmmnHumanTaskState::Active)
        .single_result()
        .expect("task query")
        .expect("task")
        .id
}

/// Java `HistoricCaseInstanceCollectionResource.java:108-300` — the
/// high-frequency filter subset over `HistoricCaseInstance.xml`.
#[test]
fn historic_case_query_supports_high_frequency_parameters() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    deploy_all(&engine);

    let finished = engine
        .start_case_instance_by_key(
            "p120Case",
            CmmnCaseInstanceStartRequest::new()
                .with_business_key("BK-100")
                .with_name("Alpha review")
                .with_started_by("kermit"),
        )
        .expect("finished case");
    let finished_task = active_task_id(&engine, &finished.id);
    engine
        .complete_human_task(&finished_task, CmmnHumanTaskCompletionRequest::new())
        .expect("complete");

    let running = engine
        .start_case_instance_by_key(
            "p120Case",
            CmmnCaseInstanceStartRequest::new()
                .with_business_key("BK-200")
                .with_name("Beta review")
                .with_started_by("fozzie"),
        )
        .expect("running case");

    let history = engine.history_service();

    // caseInstanceIds → `ID_ in (...)`.
    let by_ids = history
        .create_historic_case_instance_query()
        .case_instance_ids(vec![finished.id.clone(), "missing".to_string()])
        .list()
        .expect("by ids");
    assert_eq!(by_ids.len(), 1);
    assert_eq!(by_ids[0].case_instance_id, finished.id);

    // startedBy → `START_USER_ID_`.
    let by_started_by = history
        .create_historic_case_instance_query()
        .started_by("fozzie")
        .list()
        .expect("by startedBy");
    assert_eq!(by_started_by.len(), 1);
    assert_eq!(by_started_by[0].case_instance_id, running.id);

    // finished()/unfinished() → `END_TIME_ is (not) null`.
    let finished_only = history
        .create_historic_case_instance_query()
        .finished(true)
        .list()
        .expect("finished");
    assert_eq!(finished_only.len(), 1);
    assert_eq!(finished_only[0].case_instance_id, finished.id);

    let unfinished_only = history
        .create_historic_case_instance_query()
        .finished(false)
        .list()
        .expect("unfinished");
    assert_eq!(unfinished_only.len(), 1);
    assert_eq!(unfinished_only[0].case_instance_id, running.id);

    // startedBefore/After bracket START_TIME_.
    let future = Utc::now() + Duration::hours(1);
    let past = Utc::now() - Duration::hours(1);
    assert_eq!(
        history
            .create_historic_case_instance_query()
            .started_before(future)
            .started_after(past)
            .list()
            .expect("started window")
            .len(),
        2
    );
    assert_eq!(
        history
            .create_historic_case_instance_query()
            .started_after(future)
            .list()
            .expect("started after future")
            .len(),
        0
    );

    // finishedBefore/After compare END_TIME_, so the unfinished case never
    // satisfies either bound.
    let finished_window = history
        .create_historic_case_instance_query()
        .finished_before(future)
        .finished_after(past)
        .list()
        .expect("finished window");
    assert_eq!(finished_window.len(), 1);
    assert_eq!(finished_window[0].case_instance_id, finished.id);

    // name / nameLike / nameLikeIgnoreCase.
    assert_eq!(
        history
            .create_historic_case_instance_query()
            .name("Alpha review")
            .list()
            .expect("name")
            .len(),
        1
    );
    assert_eq!(
        history
            .create_historic_case_instance_query()
            .name_like("%review")
            .list()
            .expect("nameLike")
            .len(),
        2
    );
    assert_eq!(
        history
            .create_historic_case_instance_query()
            .name_like_ignore_case("beta%")
            .list()
            .expect("nameLikeIgnoreCase")
            .len(),
        1
    );

    // businessKeyLike / caseDefinitionKeyLike.
    assert_eq!(
        history
            .create_historic_case_instance_query()
            .business_key_like("BK-1%")
            .list()
            .expect("businessKeyLike")
            .len(),
        1
    );
    assert_eq!(
        history
            .create_historic_case_instance_query()
            .case_definition_key_like_ignore_case("P120CASE")
            .list()
            .expect("caseDefinitionKeyLikeIgnoreCase")
            .len(),
        2
    );

    // caseDefinitionNameLikeIgnoreCase — the definition name is "P120 case".
    assert_eq!(
        history
            .create_historic_case_instance_query()
            .case_definition_name_like_ignore_case("p120 CASE")
            .list()
            .expect("caseDefinitionNameLikeIgnoreCase")
            .len(),
        2
    );

    // businessStatusLike(IgnoreCase) — only a reached milestone writes a business
    // status, so it is set directly here.
    engine
        .runtime_service()
        .update_business_status(&running.id, "approved")
        .expect("business status");
    assert_eq!(
        history
            .create_historic_case_instance_query()
            .business_status_like("appr%")
            .list()
            .expect("businessStatusLike")
            .len(),
        1
    );
    assert_eq!(
        history
            .create_historic_case_instance_query()
            .business_status_like_ignore_case("APPR%")
            .list()
            .expect("businessStatusLikeIgnoreCase")
            .len(),
        1
    );

    // state filter still narrows to the terminal state.
    let completed = history
        .create_historic_case_instance_query()
        .state(CmmnCaseInstanceState::Completed)
        .list()
        .expect("state");
    assert_eq!(completed.len(), 1);
    assert_eq!(completed[0].case_instance_id, finished.id);
}

/// Java `tenantId` / `tenantIdLike` / `withoutTenantId`; the last renders
/// `TENANT_ID_ is null or = ''` (HistoricCaseInstance.xml withoutTenantId block).
#[test]
fn historic_case_query_supports_the_tenant_parameters() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    deploy_all(&engine);
    deploy_tenant(&engine, "tenant-a");

    let tenant_case = engine
        .start_case_instance_by_key(
            "p120Case",
            CmmnCaseInstanceStartRequest::new().with_tenant_id("tenant-a"),
        )
        .expect("tenant case");
    let tenantless_case = engine
        .start_case_instance_by_key("p120CandidateCase", CmmnCaseInstanceStartRequest::new())
        .expect("tenantless case");

    let history = engine.history_service();

    let by_tenant = history
        .create_historic_case_instance_query()
        .tenant_id("tenant-a")
        .list()
        .expect("tenantId");
    assert_eq!(by_tenant.len(), 1);
    assert_eq!(by_tenant[0].case_instance_id, tenant_case.id);

    let by_tenant_like = history
        .create_historic_case_instance_query()
        .tenant_id_like("tenant%")
        .list()
        .expect("tenantIdLike");
    assert_eq!(by_tenant_like.len(), 1);
    assert_eq!(by_tenant_like[0].case_instance_id, tenant_case.id);

    let by_tenant_like_ignore_case = history
        .create_historic_case_instance_query()
        .tenant_id_like_ignore_case("TENANT%")
        .list()
        .expect("tenantIdLikeIgnoreCase");
    assert_eq!(by_tenant_like_ignore_case.len(), 1);
    assert_eq!(by_tenant_like_ignore_case[0].case_instance_id, tenant_case.id);

    let without_tenant = history
        .create_historic_case_instance_query()
        .without_tenant_id()
        .list()
        .expect("withoutTenantId");
    assert_eq!(without_tenant.len(), 1);
    assert_eq!(without_tenant[0].case_instance_id, tenantless_case.id);
}

/// Java joins `ACT_CMMN_CASEDEF` for the `caseDefinitionCategory` family
/// (HistoricCaseInstance.xml:359-381); the Rust historic row has no category
/// column, so the definition is resolved through the repository service.
#[test]
fn historic_case_query_resolves_case_definition_category_through_the_definition() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    deploy_all(&engine);

    let case_instance = engine
        .start_case_instance_by_key("p120Case", CmmnCaseInstanceStartRequest::new())
        .expect("case instance");

    let history = engine.history_service();

    let matched = history
        .create_historic_case_instance_query()
        .case_definition_category("finance-cases")
        .list()
        .expect("category");
    assert!(
        matched
            .iter()
            .any(|item| item.case_instance_id == case_instance.id)
    );

    assert_eq!(
        history
            .create_historic_case_instance_query()
            .case_definition_category("other")
            .list()
            .expect("category miss")
            .len(),
        0
    );
    assert!(
        !history
            .create_historic_case_instance_query()
            .case_definition_category_like("finance%")
            .list()
            .expect("categoryLike")
            .is_empty()
    );
    assert!(
        !history
            .create_historic_case_instance_query()
            .case_definition_category_like_ignore_case("FINANCE%")
            .list()
            .expect("categoryLikeIgnoreCase")
            .is_empty()
    );
}

/// Java `HistoricTaskInstanceCollectionResource.java:97-306` — the
/// high-frequency historic task filter subset.
#[test]
fn historic_task_query_supports_high_frequency_parameters() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    deploy_all(&engine);

    let assigned_case = engine
        .start_case_instance_by_key("p120AssignedCase", CmmnCaseInstanceStartRequest::new())
        .expect("assigned case");
    let assigned_task = active_task_id(&engine, &assigned_case.id);

    let plain_case = engine
        .start_case_instance_by_key("p120Case", CmmnCaseInstanceStartRequest::new())
        .expect("plain case");
    let plain_task = active_task_id(&engine, &plain_case.id);
    engine
        .complete_human_task(&plain_task, CmmnHumanTaskCompletionRequest::new())
        .expect("complete");

    let history = engine.history_service();

    // taskId.
    let by_task_id = history
        .create_historic_human_task_query()
        .task_id(&assigned_task)
        .list()
        .expect("taskId");
    assert_eq!(by_task_id.len(), 1);

    // taskName / taskNameLike / taskNameLikeIgnoreCase.
    assert_eq!(
        history
            .create_historic_human_task_query()
            .name("Review application")
            .list()
            .expect("taskName")
            .len(),
        1
    );
    assert_eq!(
        history
            .create_historic_human_task_query()
            .name_like("%review")
            .list()
            .expect("taskNameLike")
            .len(),
        1
    );
    assert_eq!(
        history
            .create_historic_human_task_query()
            .name_like_ignore_case("ASSIGNED%")
            .list()
            .expect("taskNameLikeIgnoreCase")
            .len(),
        1
    );

    // taskDefinitionKey(Like).
    let by_definition_key = history
        .create_historic_human_task_query()
        .task_definition_key("human-task-assigned")
        .list()
        .expect("taskDefinitionKey");
    assert_eq!(by_definition_key.len(), 1);
    assert_eq!(by_definition_key[0].task_id, assigned_task);
    assert_eq!(
        history
            .create_historic_human_task_query()
            .task_definition_key_like("human-task-%")
            .list()
            .expect("taskDefinitionKeyLike")
            .len(),
        2
    );

    // taskAssignee(Like) / taskOwner(Like) / taskCategory.
    assert_eq!(
        history
            .create_historic_human_task_query()
            .assignee("fozzie")
            .list()
            .expect("taskAssignee")
            .len(),
        1
    );
    assert_eq!(
        history
            .create_historic_human_task_query()
            .assignee_like("foz%")
            .list()
            .expect("taskAssigneeLike")
            .len(),
        1
    );
    assert_eq!(
        history
            .create_historic_human_task_query()
            .owner("gonzo")
            .list()
            .expect("taskOwner")
            .len(),
        1
    );
    assert_eq!(
        history
            .create_historic_human_task_query()
            .category("finance")
            .list()
            .expect("taskCategory")
            .len(),
        1
    );

    // finished()/unfinished().
    let finished = history
        .create_historic_human_task_query()
        .finished(true)
        .list()
        .expect("finished");
    assert_eq!(finished.len(), 1);
    assert_eq!(finished[0].task_id, plain_task);

    let unfinished = history
        .create_historic_human_task_query()
        .finished(false)
        .list()
        .expect("unfinished");
    assert_eq!(unfinished.len(), 1);
    assert_eq!(unfinished[0].task_id, assigned_task);

    // taskCreatedBefore/After and taskCompletedBefore/After.
    let future = Utc::now() + Duration::hours(1);
    let past = Utc::now() - Duration::hours(1);
    assert_eq!(
        history
            .create_historic_human_task_query()
            .created_before(future)
            .created_after(past)
            .list()
            .expect("created window")
            .len(),
        2
    );
    let completed_window = history
        .create_historic_human_task_query()
        .completed_before(future)
        .completed_after(past)
        .list()
        .expect("completed window");
    assert_eq!(completed_window.len(), 1);
    assert_eq!(completed_window[0].task_id, plain_task);

    // state.
    assert_eq!(
        history
            .create_historic_human_task_query()
            .state(CmmnHumanTaskState::Completed)
            .list()
            .expect("state")
            .len(),
        1
    );

    // taskDeleteReason: Java compares DELETE_REASON_, which Rust never records,
    // so the filter is accepted and selects nothing.
    assert_eq!(
        history
            .create_historic_human_task_query()
            .delete_reason("completed")
            .list()
            .expect("taskDeleteReason")
            .len(),
        0
    );
}

/// Java reads `ACT_HI_IDENTITYLINK` with `TYPE_ = 'candidate'` plus an implicit
/// `ASSIGNEE_ is null` gate (HistoricTaskInstance.xml:1484-1512). Rust keeps a
/// single identity-link table shared by runtime and history, so the historic
/// query reads the same rows.
#[test]
fn historic_task_query_supports_candidate_and_involved_filters() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    deploy_all(&engine);

    let candidate_case = engine
        .start_case_instance_by_key("p120CandidateCase", CmmnCaseInstanceStartRequest::new())
        .expect("candidate case");
    let candidate_task = active_task_id(&engine, &candidate_case.id);

    let assigned_case = engine
        .start_case_instance_by_key("p120AssignedCase", CmmnCaseInstanceStartRequest::new())
        .expect("assigned case");
    let assigned_task = active_task_id(&engine, &assigned_case.id);

    // Completing the candidate task proves the links survive into history: Rust
    // only drops identity links when the case history itself is deleted.
    engine
        .complete_human_task(&candidate_task, CmmnHumanTaskCompletionRequest::new())
        .expect("complete candidate task");

    let history = engine.history_service();

    // taskCandidateGroup matches the candidate link on the completed task; the
    // assigned task is excluded by the implicit assignee-null gate even though it
    // also carries a `sales` candidate group.
    let by_group = history
        .create_historic_human_task_query()
        .candidate_group("sales")
        .list()
        .expect("taskCandidateGroup");
    assert_eq!(by_group.len(), 1);
    assert_eq!(by_group[0].task_id, candidate_task);

    // ignoreTaskAssignee drops that gate, so the assigned task joins the result.
    let ignoring_assignee = history
        .create_historic_human_task_query()
        .candidate_group("sales")
        .ignore_assignee_value()
        .list()
        .expect("ignoreTaskAssignee");
    assert_eq!(ignoring_assignee.len(), 2);
    assert!(
        ignoring_assignee
            .iter()
            .any(|task| task.task_id == assigned_task)
    );

    // taskCandidateGroupIn.
    assert_eq!(
        history
            .create_historic_human_task_query()
            .candidate_group_in(vec!["support".to_string(), "unknown".to_string()])
            .list()
            .expect("taskCandidateGroupIn")
            .len(),
        1
    );

    // taskCandidateUser matches the direct user link.
    let by_user = history
        .create_historic_human_task_query()
        .candidate_user("kermit")
        .list()
        .expect("taskCandidateUser");
    assert_eq!(by_user.len(), 1);
    assert_eq!(by_user[0].task_id, candidate_task);

    // A user with no direct link matches through the resolver-expanded groups,
    // mirroring Java's `getGroupsForCandidateUser`.
    let resolver = Arc::new(|user: &str| {
        if user == "piggy" {
            vec!["support".to_string()]
        } else {
            Vec::new()
        }
    });
    assert_eq!(
        history
            .create_historic_human_task_query()
            .candidate_user("piggy")
            .user_group_resolver(resolver.clone())
            .list()
            .expect("candidate user via groups")
            .len(),
        1
    );
    // Without the resolver the same user matches nothing.
    assert_eq!(
        history
            .create_historic_human_task_query()
            .candidate_user("piggy")
            .list()
            .expect("candidate user without resolver")
            .len(),
        0
    );

    // taskInvolvedUser matches the assignee, the owner, or any identity link, and
    // carries no assignee-null gate.
    assert_eq!(
        history
            .create_historic_human_task_query()
            .involved_user("fozzie")
            .list()
            .expect("involved assignee")
            .len(),
        1
    );
    assert_eq!(
        history
            .create_historic_human_task_query()
            .involved_user("gonzo")
            .list()
            .expect("involved owner")
            .len(),
        1
    );
    let involved_candidate = history
        .create_historic_human_task_query()
        .involved_user("kermit")
        .list()
        .expect("involved via link");
    assert_eq!(involved_candidate.len(), 1);
    assert_eq!(involved_candidate[0].task_id, candidate_task);

    // taskInvolvedGroups matches any link carrying one of the group ids.
    assert_eq!(
        history
            .create_historic_human_task_query()
            .involved_groups(vec!["sales".to_string()])
            .list()
            .expect("taskInvolvedGroups")
            .len(),
        2
    );
}
