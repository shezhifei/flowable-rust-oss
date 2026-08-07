//! P69: availableCondition and human-task attributes via SimpleExpression.
//!
//! Java:
//! - AbstractEvaluationCriteriaOperation.java:584-604 — availableCondition is
//!   fully EL-evaluated; only Boolean true makes the listener available.
//! - HumanTaskActivityBehavior.java:107-147 — assignee/owner/priority/dueDate/
//!   candidateUsers/Groups resolve `${…}` against case variables; candidates
//!   are comma-split after expression evaluation.
//!
//! Non-`${…}` literals keep C7/C10 behavior (if-part dialect / verbatim copy).

use flowable_cmmn_engine::{
    CmmnCase, CmmnCaseInstanceStartRequest, CmmnCasePlanModel, CmmnDeploymentRequest, CmmnEngine,
    CmmnEventListener, CmmnHumanTask, CmmnHumanTaskState, CmmnModel, CmmnPlanItem,
};
use serde_json::json;

fn deploy(engine: &CmmnEngine, key: &str, model: CmmnModel) {
    engine
        .deploy(CmmnDeploymentRequest::new(key).with_resource("case.cmmn", model))
        .expect("deployment");
}

fn single_case(case_key: &str, plan_model: CmmnCasePlanModel) -> CmmnModel {
    CmmnModel::new(vec![CmmnCase::new(
        "p69-case",
        case_key,
        "P69 EL case",
        plan_model,
    )])
}

fn subscription_count(engine: &CmmnEngine, case_id: &str) -> usize {
    engine
        .runtime_service()
        .create_event_subscription_query()
        .case_instance_id(case_id)
        .list()
        .expect("subscriptions")
        .len()
}

/// availableCondition `${go}` against a case variable creates the event
/// subscription only when the expression evaluates to boolean true.
#[test]
fn available_condition_uel_uses_case_variable() {
    let plan = CmmnCasePlanModel::new("case-plan-model", "Case plan model")
        .with_event_listener(
            CmmnEventListener::new("gated-el-listener", "message")
                .with_event_name("gatedEvent")
                .with_available_condition("${go}"),
        )
        .with_plan_item(CmmnPlanItem::new(
            "plan-item-gated-listener",
            "gated-el-listener",
        ))
        .with_human_task(CmmnHumanTask::new("human-task-keepalive", "Keep alive"))
        .with_plan_item(CmmnPlanItem::new(
            "plan-item-keepalive",
            "human-task-keepalive",
        ));

    let engine = CmmnEngine::new_in_memory().expect("engine");
    deploy(
        &engine,
        "p69-available-uel",
        single_case("p69AvailableUelCase", plan),
    );

    // Missing / non-true → no subscription
    let case_id = engine
        .start_case_instance_by_key(
            "p69AvailableUelCase",
            CmmnCaseInstanceStartRequest::new().with_variables(json!({ "go": false })),
        )
        .expect("start")
        .id;
    assert_eq!(subscription_count(&engine, &case_id), 0);

    // Flip to true → subscription appears (re-evaluation cycle)
    engine
        .runtime_service()
        .set_case_instance_variables(&case_id, vec![("go".to_string(), json!(true))])
        .expect("set var");
    assert_eq!(subscription_count(&engine, &case_id), 1);
}

/// Human task assignee `${manager}` resolves from case variables at task create.
#[test]
fn human_task_assignee_uel_resolves_from_case_variables() {
    let plan = CmmnCasePlanModel::new("case-plan-model", "Case plan model")
        .with_human_task(
            CmmnHumanTask::new("review-task", "Review")
                .with_assignee("${manager}")
                .with_owner("${ownerUser}")
                .with_priority("${prio}")
                .with_candidate_users(vec!["${reviewers}".to_string()]),
        )
        .with_plan_item(CmmnPlanItem::new("plan-item-review", "review-task"));

    let engine = CmmnEngine::new_in_memory().expect("engine");
    deploy(
        &engine,
        "p69-human-task-uel",
        single_case("p69HumanTaskUelCase", plan),
    );

    let case_instance = engine
        .start_case_instance_by_key(
            "p69HumanTaskUelCase",
            CmmnCaseInstanceStartRequest::new().with_variables(json!({
                "manager": "alice",
                "ownerUser": "bob",
                "prio": "42",
                "reviewers": "carol, dave",
            })),
        )
        .expect("start");

    let task = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_instance.id)
        .state(CmmnHumanTaskState::Active)
        .list()
        .expect("tasks")
        .into_iter()
        .next()
        .expect("one active task");

    assert_eq!(task.assignee.as_deref(), Some("alice"));
    assert_eq!(task.owner.as_deref(), Some("bob"));
    assert_eq!(task.priority.as_deref(), Some("42"));

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
    assert_eq!(users, vec!["carol".to_string(), "dave".to_string()]);
}

/// Literal (non-UEL) human-task attributes still copy verbatim (C10 regression).
#[test]
fn human_task_literal_attributes_unchanged() {
    let plan = CmmnCasePlanModel::new("case-plan-model", "Case plan model")
        .with_human_task(
            CmmnHumanTask::new("review-task", "Review")
                .with_assignee("literal-alice")
                .with_candidate_users(vec!["u1".to_string(), "u2".to_string()]),
        )
        .with_plan_item(CmmnPlanItem::new("plan-item-review", "review-task"));

    let engine = CmmnEngine::new_in_memory().expect("engine");
    deploy(
        &engine,
        "p69-human-task-literal",
        single_case("p69HumanTaskLiteralCase", plan),
    );

    let case_instance = engine
        .start_case_instance_by_key(
            "p69HumanTaskLiteralCase",
            CmmnCaseInstanceStartRequest::new(),
        )
        .expect("start");

    let task = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_instance.id)
        .state(CmmnHumanTaskState::Active)
        .list()
        .expect("tasks")
        .into_iter()
        .next()
        .expect("one active task");

    assert_eq!(task.assignee.as_deref(), Some("literal-alice"));
    let links = engine
        .identity_link_service()
        .list_identity_links("humanTask", &task.id)
        .expect("identity links");
    let mut users: Vec<String> = links
        .iter()
        .filter_map(|link| link.user_id.clone())
        .collect();
    users.sort();
    assert_eq!(users, vec!["u1".to_string(), "u2".to_string()]);
}
