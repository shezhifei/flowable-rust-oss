// P100: CMMN human-task query filter surface.
//
// Java references:
// - TaskCollectionResource.java:125-349 (GET /cmmn-runtime/tasks param parsing)
// - TaskBaseResource.java:138-358 (getTasksFromQueryRequest → TaskQuery builders)
// - TaskQueryImpl.java:1942-1958 (active()/suspended())
// - HumanTaskActivityBehavior.java:264-353 (priority int / dueDate Date storage)
//
// Intentional deviation (P100 acceptance): the Rust engine never suspends cases
// or tasks, so `active()` retains everything and `suspended()` retains nothing.

use chrono::{DateTime, Utc};
use flowable_cmmn_engine::{
    CmmnCase, CmmnCaseInstanceStartRequest, CmmnCasePlanModel, CmmnDelegationState,
    CmmnDeploymentRequest, CmmnEngine, CmmnHumanTask, CmmnModel, CmmnPlanItem,
    TaskSuspensionState,
};
use serde_json::json;

fn model_with_tasks(case_key: &str) -> CmmnModel {
    let plan_model = CmmnCasePlanModel::new("case-plan-model", "Case plan model")
        .with_human_task(
            CmmnHumanTask::new("task-alpha", "Alpha review")
                .with_assignee("alice")
                .with_owner("owner-a")
                .with_priority("50")
                .with_due_date("2026-12-31")
                .with_category("work"),
        )
        .with_human_task(
            CmmnHumanTask::new("task-beta", "Beta review")
                .with_assignee("bob")
                .with_owner("owner-b")
                .with_priority("70")
                .with_category("personal"),
        )
        .with_human_task(CmmnHumanTask::new("task-gamma", "Gamma deep dive"))
        .with_plan_item(CmmnPlanItem::new("plan-item-alpha", "task-alpha"))
        .with_plan_item(CmmnPlanItem::new("plan-item-beta", "task-beta"))
        .with_plan_item(CmmnPlanItem::new("plan-item-gamma", "task-gamma"));
    CmmnModel::new(vec![CmmnCase::new(
        format!("case-{case_key}"),
        case_key,
        "P100 human task query case",
        plan_model,
    )])
}

fn deploy_and_start(engine: &CmmnEngine, case_key: &str) -> String {
    engine
        .deploy(
            CmmnDeploymentRequest::new(format!("{case_key}-deployment"))
                .with_resource(format!("{case_key}.cmmn"), model_with_tasks(case_key)),
        )
        .expect("deployment");
    engine
        .start_case_instance_by_key(case_key, CmmnCaseInstanceStartRequest::new())
        .expect("case instance")
        .id
}

fn query(engine: &CmmnEngine) -> flowable_cmmn_engine::CmmnHumanTaskQuery {
    engine.runtime_service().create_human_task_query()
}

#[test]
fn query_filters_by_name_and_like() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let case_id = deploy_and_start(&engine, "p100Name");

    let names = query(&engine)
        .name("Alpha review")
        .list()
        .expect("query")
        .into_iter()
        .map(|task| task.name)
        .collect::<Vec<_>>();
    assert_eq!(names, vec!["Alpha review".to_string()]);

    let like = query(&engine)
        .name_like("Alpha%")
        .list()
        .expect("query")
        .into_iter()
        .map(|task| task.name)
        .collect::<Vec<_>>();
    assert_eq!(like, vec!["Alpha review".to_string()]);

    let like_middle = query(&engine)
        .name_like("%view")
        .list()
        .expect("query")
        .into_iter()
        .map(|task| task.name)
        .collect::<Vec<_>>();
    assert_eq!(like_middle.len(), 2, "Alpha and Beta end with 'view'");

    let ignore_case = query(&engine)
        .name_like_ignore_case("alpha%")
        .list()
        .expect("query")
        .into_iter()
        .map(|task| task.name)
        .collect::<Vec<_>>();
    assert_eq!(ignore_case, vec!["Alpha review".to_string()]);

    // Case-scoped name filter sees only the one case's tasks.
    let case_names = query(&engine)
        .case_instance_id(&case_id)
        .name_like("%review")
        .list()
        .expect("query")
        .into_iter()
        .map(|task| task.name)
        .collect::<Vec<_>>();
    assert_eq!(case_names.len(), 2);
}

#[test]
fn query_filters_by_assignee_owner_and_unassigned() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    deploy_and_start(&engine, "p100Assignee");

    let assigned = query(&engine)
        .assignee("alice")
        .list()
        .expect("query")
        .into_iter()
        .map(|task| task.id)
        .collect::<Vec<_>>();
    assert_eq!(assigned.len(), 1);

    let assignee_like = query(&engine)
        .assignee_like("a%")
        .list()
        .expect("query")
        .into_iter()
        .map(|task| task.assignee)
        .collect::<Vec<_>>();
    assert_eq!(assignee_like, vec![Some("alice".to_string())]);

    let owner = query(&engine)
        .owner("owner-b")
        .list()
        .expect("query")
        .into_iter()
        .map(|task| task.id)
        .collect::<Vec<_>>();
    assert_eq!(owner.len(), 1);

    let owner_like = query(&engine)
        .owner_like("%-b")
        .list()
        .expect("query")
        .into_iter()
        .map(|task| task.id)
        .collect::<Vec<_>>();
    assert_eq!(owner_like.len(), 1);

    // Java `taskUnassigned()`: only the task with no assignee.
    let unassigned = query(&engine)
        .unassigned()
        .list()
        .expect("query")
        .into_iter()
        .map(|task| task.name)
        .collect::<Vec<_>>();
    assert_eq!(unassigned, vec!["Gamma deep dive".to_string()]);
}

#[test]
fn query_filters_by_delegation_state() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let case_id = deploy_and_start(&engine, "p100Delegation");

    // Java DelegateTaskCmd.java:38 sets PENDING and moves the assignee to owner.
    let task_id = query(&engine)
        .case_instance_id(&case_id)
        .assignee("alice")
        .single_result()
        .expect("query")
        .expect("task")
        .id;
    engine
        .runtime_service()
        .delegate_human_task(&task_id, "carol")
        .expect("delegate");

    let pending = query(&engine)
        .delegation_state(CmmnDelegationState::Pending)
        .list()
        .expect("query")
        .into_iter()
        .map(|task| task.id)
        .collect::<Vec<_>>();
    assert_eq!(pending, vec![task_id]);

    let resolved = query(&engine)
        .delegation_state(CmmnDelegationState::Resolved)
        .list()
        .expect("query")
        .into_iter()
        .count();
    assert_eq!(resolved, 0);
}

#[test]
fn query_filters_by_category() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    deploy_and_start(&engine, "p100Category");

    let exact = query(&engine)
        .category("work")
        .list()
        .expect("query")
        .into_iter()
        .map(|task| task.id)
        .collect::<Vec<_>>();
    assert_eq!(exact.len(), 1);

    let in_categories = query(&engine)
        .category_in(vec!["work".to_string(), "personal".to_string()])
        .list()
        .expect("query")
        .into_iter()
        .map(|task| task.id)
        .collect::<Vec<_>>();
    assert_eq!(in_categories.len(), 2);

    let not_in = query(&engine)
        .category_not_in(vec!["personal".to_string()])
        .list()
        .expect("query")
        .into_iter()
        .map(|task| task.id)
        .collect::<Vec<_>>();
    assert_eq!(not_in.len(), 2, "work + no-category survive categoryNotIn");

    let without = query(&engine)
        .without_category()
        .list()
        .expect("query")
        .into_iter()
        .map(|task| task.name)
        .collect::<Vec<_>>();
    assert_eq!(without, vec!["Gamma deep dive".to_string()]);
}

#[test]
fn query_filters_by_task_definition_key() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    deploy_and_start(&engine, "p100TaskDef");

    let exact = query(&engine)
        .task_definition_id("task-beta")
        .list()
        .expect("query")
        .into_iter()
        .map(|task| task.name)
        .collect::<Vec<_>>();
    assert_eq!(exact, vec!["Beta review".to_string()]);

    let like = query(&engine)
        .task_definition_id_like("task-%")
        .list()
        .expect("query")
        .into_iter()
        .count();
    assert_eq!(like, 3);
}

#[test]
fn query_filters_by_priority() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    deploy_and_start(&engine, "p100Priority");

    let exact = query(&engine)
        .priority(50)
        .list()
        .expect("query")
        .into_iter()
        .map(|task| task.name)
        .collect::<Vec<_>>();
    assert_eq!(exact, vec!["Alpha review".to_string()]);

    let min = query(&engine)
        .min_priority(60)
        .list()
        .expect("query")
        .into_iter()
        .map(|task| task.name)
        .collect::<Vec<_>>();
    assert_eq!(min, vec!["Beta review".to_string()]);

    let max = query(&engine)
        .max_priority(60)
        .list()
        .expect("query")
        .into_iter()
        .map(|task| task.name)
        .collect::<Vec<_>>();
    assert_eq!(max, vec!["Alpha review".to_string()]);
}

#[test]
fn query_filters_by_created_time() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let case_id = deploy_and_start(&engine, "p100Created");
    let task = query(&engine)
        .case_instance_id(&case_id)
        .single_result()
        .expect("query")
        .expect("task");
    let activated_at = task.activated_at;

    // createdAfter the previous second excludes nothing created after that instant.
    let after = query(&engine)
        .created_after(activated_at - chrono::Duration::seconds(1))
        .list()
        .expect("query")
        .into_iter()
        .count();
    assert_eq!(after, 3);

    // createdBefore the activation instant excludes the task itself.
    let before = query(&engine)
        .created_before(activated_at + chrono::Duration::seconds(1))
        .list()
        .expect("query")
        .into_iter()
        .count();
    assert_eq!(before, 3);

    // createdOn exact instant — only the task created at that instant matches
    // (each task's activated_at is captured separately).
    let on = query(&engine)
        .created_on(activated_at)
        .list()
        .expect("query")
        .into_iter()
        .count();
    assert_eq!(on, 1);
}

#[test]
fn query_filters_by_due_date() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    deploy_and_start(&engine, "p100DueDate");

    // Stored due date "2026-12-31" parses to 2026-12-31T00:00:00Z.
    let due = query(&engine)
        .due_date(
            DateTime::parse_from_rfc3339("2026-12-31T00:00:00Z")
                .expect("date")
                .with_timezone(&Utc),
        )
        .list()
        .expect("query")
        .into_iter()
        .map(|task| task.name)
        .collect::<Vec<_>>();
    assert_eq!(due, vec!["Alpha review".to_string()]);

    let before = query(&engine)
        .due_before(
            DateTime::parse_from_rfc3339("2027-01-01T00:00:00Z")
                .expect("date")
                .with_timezone(&Utc),
        )
        .list()
        .expect("query")
        .into_iter()
        .map(|task| task.name)
        .collect::<Vec<_>>();
    assert_eq!(before, vec!["Alpha review".to_string()]);

    let after = query(&engine)
        .due_after(
            DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z")
                .expect("date")
                .with_timezone(&Utc),
        )
        .list()
        .expect("query")
        .into_iter()
        .map(|task| task.name)
        .collect::<Vec<_>>();
    assert_eq!(after, vec!["Alpha review".to_string()]);

    let without = query(&engine)
        .without_due_date()
        .list()
        .expect("query")
        .into_iter()
        .map(|task| task.name)
        .collect::<Vec<_>>();
    assert_eq!(without, vec!["Beta review".to_string(), "Gamma deep dive".to_string()]);
}

#[test]
fn query_filters_by_case_definition_key_and_suspension_state() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    deploy_and_start(&engine, "p100CaseDefKey");

    let by_key = query(&engine)
        .case_definition_key("p100CaseDefKey")
        .list()
        .expect("query")
        .into_iter()
        .count();
    assert_eq!(by_key, 3);

    let key_like = query(&engine)
        .case_definition_key_like("p100CaseDef%")
        .list()
        .expect("query")
        .into_iter()
        .count();
    assert_eq!(key_like, 3);

    let key_like_ignore_case = query(&engine)
        .case_definition_key_like_ignore_case("p100casedef%")
        .list()
        .expect("query")
        .into_iter()
        .count();
    assert_eq!(key_like_ignore_case, 3);

    // Rust never suspends cases: active() retains everything, suspended() nothing.
    let active = query(&engine)
        .suspension_state(TaskSuspensionState::Active)
        .list()
        .expect("query")
        .into_iter()
        .count();
    assert_eq!(active, 3);

    let suspended = query(&engine)
        .suspension_state(TaskSuspensionState::Suspended)
        .list()
        .expect("query")
        .into_iter()
        .count();
    assert_eq!(suspended, 0);
}

#[test]
fn query_priority_ignores_non_numeric_stored_priority() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    // A literal non-numeric priority would have failed at creation in Java
    // (HumanTaskActivityBehavior.java:280-282); in Rust it is stored verbatim, so
    // numeric filters treat it as never matching.
    let plan_model = CmmnCasePlanModel::new("case-plan-model", "Case plan model")
        .with_human_task(
            CmmnHumanTask::new("task-high", "High")
                .with_priority("high")
                .with_assignee("alice"),
        )
        .with_plan_item(CmmnPlanItem::new("plan-item-high", "task-high"));
    engine
        .deploy(
            CmmnDeploymentRequest::new("p100NonNumeric-deployment").with_resource(
                "p100NonNumeric.cmmn",
                CmmnModel::new(vec![CmmnCase::new(
                    "case-p100NonNumeric",
                    "p100NonNumeric",
                    "non-numeric priority",
                    plan_model,
                )]),
            ),
        )
        .expect("deployment");
    engine
        .start_case_instance_by_key(
            "p100NonNumeric",
            CmmnCaseInstanceStartRequest::new().with_variables(json!({})),
        )
        .expect("case instance");

    assert_eq!(
        query(&engine)
            .priority(50)
            .list()
            .expect("query")
            .into_iter()
            .count(),
        0,
        "non-numeric stored priority never matches a numeric filter"
    );
    assert_eq!(
        query(&engine)
            .min_priority(1)
            .list()
            .expect("query")
            .into_iter()
            .count(),
        0,
        "non-numeric stored priority never matches min priority"
    );
}
