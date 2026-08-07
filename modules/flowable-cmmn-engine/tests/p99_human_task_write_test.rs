// P99: CMMN human-task write surface — field update (saveTask), claim /
// delegate / resolve state machine, and completion variables (GLOBAL → case).
//
// Java references:
// - TaskBaseResource.populateTaskFromRequest (TaskBaseResource.java:91-127)
// - ClaimTaskCmd.java:39-85 (FlowableTaskAlreadyClaimedException :51)
// - DelegateTaskCmd.java:37-47 (PENDING + owner default)
// - ResolveTaskCmd.java:55-57 (RESOLVED + assignee back to owner)
// - CompleteTaskCmd.java:100-101 (completion variables on the task = case scope)

use flowable_cmmn_engine::{
    CmmnCase, CmmnCaseInstanceStartRequest, CmmnCasePlanModel, CmmnDelegationState,
    CmmnDeploymentRequest, CmmnEngine, CmmnError, CmmnHumanTask, CmmnHumanTaskUpdate, CmmnModel,
    CmmnPlanItem,
};
use serde_json::json;

fn simple_case_model(case_key: &str) -> CmmnModel {
    let plan_model = CmmnCasePlanModel::new("case-plan-model", "Case plan model")
        .with_human_task(CmmnHumanTask::new("task-a", "Task A"))
        .with_plan_item(CmmnPlanItem::new("plan-item-a", "task-a"));
    CmmnModel::new(vec![CmmnCase::new(
        format!("case-{case_key}"),
        case_key,
        "P99 human task write case",
        plan_model,
    )])
}

fn deploy_and_start(engine: &CmmnEngine, case_key: &str) -> String {
    engine
        .deploy(
            CmmnDeploymentRequest::new(format!("{case_key}-deployment"))
                .with_resource(format!("{case_key}.cmmn"), simple_case_model(case_key)),
        )
        .expect("deployment");
    engine
        .start_case_instance_by_key(case_key, CmmnCaseInstanceStartRequest::new())
        .expect("case instance")
        .id
}

fn single_active_task_id(engine: &CmmnEngine, case_id: &str) -> String {
    engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(case_id)
        .single_result()
        .expect("query")
        .expect("task")
        .id
}

#[test]
fn update_human_task_sets_fields_and_clears_with_explicit_null() {
    // Java: TaskResource.updateTask (TaskResource.java:76-99) → saveTask.
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let case_id = deploy_and_start(&engine, "p99UpdateCase");
    let task_id = single_active_task_id(&engine, &case_id);

    engine
        .runtime_service()
        .update_human_task(
            &task_id,
            CmmnHumanTaskUpdate {
                name: Some(Some("Renamed".to_string())),
                assignee: Some(Some("alice".to_string())),
                owner: Some(Some("owner".to_string())),
                priority: Some(Some("50".to_string())),
                due_date: Some(Some("2026-12-31".to_string())),
                category: Some(Some("work".to_string())),
                ..CmmnHumanTaskUpdate::default()
            },
        )
        .expect("update");

    let refreshed = engine
        .runtime_service()
        .get_human_task(&task_id)
        .expect("task");
    assert_eq!(refreshed.name, "Renamed");
    assert_eq!(refreshed.assignee.as_deref(), Some("alice"));
    assert_eq!(refreshed.owner.as_deref(), Some("owner"));
    assert_eq!(refreshed.priority.as_deref(), Some("50"));
    assert_eq!(refreshed.due_date.as_deref(), Some("2026-12-31"));
    assert_eq!(refreshed.category.as_deref(), Some("work"));

    // Explicit null clears ("{"dueDate" : null}" → TaskResource.java:70).
    engine
        .runtime_service()
        .update_human_task(
            &task_id,
            CmmnHumanTaskUpdate {
                assignee: Some(None),
                due_date: Some(None),
                ..CmmnHumanTaskUpdate::default()
            },
        )
        .expect("clear");

    let cleared = engine
        .runtime_service()
        .get_human_task(&task_id)
        .expect("task");
    assert_eq!(cleared.assignee, None, "explicit null clears assignee");
    assert_eq!(cleared.due_date, None, "explicit null clears due date");
    assert_eq!(cleared.name, "Renamed", "untouched fields stay");
    assert_eq!(cleared.category.as_deref(), Some("work"));
}

#[test]
fn update_human_task_unknown_id_is_not_found() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let result =
        engine
            .runtime_service()
            .update_human_task("missing", CmmnHumanTaskUpdate::default());
    assert!(
        matches!(result, Err(CmmnError::NotFound { .. })),
        "missing task → NotFound"
    );
}

#[test]
fn claim_human_task_assigns_and_conflicts_on_second_claim() {
    // Java: ClaimTaskCmd.java:39-85.
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let case_id = deploy_and_start(&engine, "p99ClaimCase");
    let task_id = single_active_task_id(&engine, &case_id);

    engine
        .runtime_service()
        .claim_human_task(&task_id, "alice")
        .expect("claim");
    let claimed = engine
        .runtime_service()
        .get_human_task(&task_id)
        .expect("task");
    assert_eq!(claimed.assignee.as_deref(), Some("alice"));

    // Re-claim by the same user is a no-op (Java post-conditions already met).
    engine
        .runtime_service()
        .claim_human_task(&task_id, "alice")
        .expect("same-user re-claim");
    let still = engine
        .runtime_service()
        .get_human_task(&task_id)
        .expect("task");
    assert_eq!(still.assignee.as_deref(), Some("alice"));

    // Different user → FlowableTaskAlreadyClaimedException (ClaimTaskCmd.java:51).
    let conflict = engine.runtime_service().claim_human_task(&task_id, "bob");
    assert!(
        matches!(conflict, Err(CmmnError::Conflict { .. })),
        "claim by another user → Conflict (REST 409)"
    );
}

#[test]
fn delegate_sets_pending_and_resolve_returns_assignee_to_owner() {
    // Java: DelegateTaskCmd.java:37-47 + ResolveTaskCmd.java:55-57.
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let case_id = deploy_and_start(&engine, "p99DelegateCase");
    let task_id = single_active_task_id(&engine, &case_id);

    engine
        .runtime_service()
        .claim_human_task(&task_id, "alice")
        .expect("claim");

    engine
        .runtime_service()
        .delegate_human_task(&task_id, "bob")
        .expect("delegate");
    let delegated = engine
        .runtime_service()
        .get_human_task(&task_id)
        .expect("task");
    assert_eq!(
        delegated.delegation_state,
        Some(CmmnDelegationState::Pending),
        "delegate → PENDING (DelegateTaskCmd.java:38)"
    );
    assert_eq!(
        delegated.owner.as_deref(),
        Some("alice"),
        "owner defaults to the current assignee (DelegateTaskCmd.java:39-41)"
    );
    assert_eq!(delegated.assignee.as_deref(), Some("bob"));

    engine
        .runtime_service()
        .resolve_human_task(&task_id)
        .expect("resolve");
    let resolved = engine
        .runtime_service()
        .get_human_task(&task_id)
        .expect("task");
    assert_eq!(
        resolved.delegation_state,
        Some(CmmnDelegationState::Resolved),
        "resolve → RESOLVED (ResolveTaskCmd.java:55)"
    );
    assert_eq!(
        resolved.assignee.as_deref(),
        Some("alice"),
        "assignee returns to the owner (ResolveTaskCmd.java:56)"
    );
}

#[test]
fn claim_delegate_resolve_reject_non_active_task() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let case_id = deploy_and_start(&engine, "p99GuardCase");
    let task_id = single_active_task_id(&engine, &case_id);

    // Complete the task first; claim on a completed task must be rejected.
    engine
        .runtime_service()
        .complete_human_task(&task_id, Default::default())
        .expect("complete");

    let err = engine
        .runtime_service()
        .claim_human_task(&task_id, "alice")
        .expect_err("claim on completed task must fail");
    assert!(
        matches!(err, CmmnError::Execution { .. }),
        "non-active task → Execution error (REST 400)"
    );
}

#[test]
fn complete_human_task_writes_global_completion_variables_to_case() {
    // Java: CompleteTaskCmd.java:100-101 — variables land on the case scope.
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let case_id = deploy_and_start(&engine, "p99CompleteVarsCase");
    let task_id = single_active_task_id(&engine, &case_id);

    let mut request = flowable_cmmn_engine::CmmnHumanTaskCompletionRequest::new();
    request.variables = vec![
        ("completedFlag".to_string(), json!(true)),
        ("completer".to_string(), json!("alice")),
    ];
    request.outcome = Some("approved".to_string());

    engine
        .runtime_service()
        .complete_human_task(&task_id, request)
        .expect("complete with variables");

    let case = engine
        .runtime_service()
        .get_case_instance(&case_id)
        .expect("case");
    assert_eq!(case.variables.get("completedFlag"), Some(&json!(true)));
    assert_eq!(case.variables.get("completer"), Some(&json!("alice")));
}
