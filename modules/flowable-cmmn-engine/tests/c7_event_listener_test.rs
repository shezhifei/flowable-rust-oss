// C7 parity tests: event listener availableCondition, variable event listeners and
// case completion interplay.
// Java references:
// - EventListener.java:20 + AbstractEvaluationCriteriaOperation.java:584-604 — a non-empty
//   availableCondition gates the listener; only a Boolean true makes it available and the
//   condition is re-evaluated on every evaluation cycle (both directions).
// - VariableEventListener.java:23-24 + EvaluateVariableEventListenersOperation.java:58-104 —
//   "variable" event subscriptions trigger the listener plan item when a matching variable
//   is written, honoring the configured changeType (:80-95).
// - PlanItemInstanceContainerUtil.java:143-146 — only AVAILABLE/ENABLED plan items block a
//   non-autocomplete container; an unavailable listener does not.
use flowable_cmmn_engine::{
    CmmnCase, CmmnCaseInstanceStartRequest, CmmnCaseInstanceState, CmmnCasePlanModel,
    CmmnDeploymentRequest, CmmnEngine, CmmnEventListener, CmmnHumanTask,
    CmmnHumanTaskCompletionRequest, CmmnHumanTaskState, CmmnModel, CmmnPlanItem,
    CmmnPlanItemOnPart, CmmnSentry,
};
use serde_json::json;

fn deploy_case(engine: &CmmnEngine, deployment_key: &str, model: CmmnModel) {
    engine
        .deploy(CmmnDeploymentRequest::new(deployment_key).with_resource("case.cmmn", model))
        .expect("deployment");
}

fn start_case(engine: &CmmnEngine, case_key: &str) -> String {
    engine
        .start_case_instance_by_key(case_key, CmmnCaseInstanceStartRequest::new())
        .expect("case instance")
        .id
}

fn single_case_model(case_key: &str, plan_model: CmmnCasePlanModel) -> CmmnModel {
    CmmnModel::new(vec![CmmnCase::new(
        "case-event-listener-parity",
        case_key,
        "Event listener parity case",
        plan_model,
    )])
}

fn subscription_count(engine: &CmmnEngine, case_id: &str) -> usize {
    engine
        .runtime_service()
        .create_event_subscription_query()
        .case_instance_id(case_id)
        .list()
        .expect("event subscriptions")
        .len()
}

fn set_variable(engine: &CmmnEngine, case_id: &str, name: &str, value: serde_json::Value) {
    engine
        .runtime_service()
        .set_case_instance_variables(case_id, vec![(name.to_string(), value)])
        .expect("variable update");
}

fn conditional_listener_plan_model() -> CmmnCasePlanModel {
    CmmnCasePlanModel::new("case-plan-model", "Case plan model")
        .with_event_listener(
            CmmnEventListener::new("gated-event-listener", "message")
                .with_event_name("gatedEvent")
                .with_available_condition("go == true"),
        )
        .with_plan_item(CmmnPlanItem::new(
            "plan-item-gated-listener",
            "gated-event-listener",
        ))
        .with_human_task(CmmnHumanTask::new("human-task-keepalive", "Keep alive"))
        .with_plan_item(CmmnPlanItem::new(
            "plan-item-keepalive",
            "human-task-keepalive",
        ))
}

#[test]
fn available_condition_false_gates_subscription_creation() {
    // AbstractEvaluationCriteriaOperation.java:584-604 — a false availableCondition keeps
    // the listener unavailable: no event subscription is created at activation time.
    let engine = CmmnEngine::new_in_memory().expect("engine");
    deploy_case(
        &engine,
        "gated-listener-false",
        single_case_model("gatedListenerFalseCase", conditional_listener_plan_model()),
    );

    let case_id = start_case(&engine, "gatedListenerFalseCase");

    assert_eq!(subscription_count(&engine, &case_id), 0);
}

#[test]
fn available_condition_true_creates_subscription_on_activation() {
    // AbstractEvaluationCriteriaOperation.java:596-597 — a Boolean true result makes the
    // listener available, so its subscription is created like an unconditional listener.
    let engine = CmmnEngine::new_in_memory().expect("engine");
    deploy_case(
        &engine,
        "gated-listener-true",
        single_case_model("gatedListenerTrueCase", conditional_listener_plan_model()),
    );

    let case_id = engine
        .start_case_instance_by_key(
            "gatedListenerTrueCase",
            CmmnCaseInstanceStartRequest::new().with_variables(json!({ "go": true })),
        )
        .expect("case instance")
        .id;

    assert_eq!(subscription_count(&engine, &case_id), 1);
}

#[test]
fn variable_update_moves_listener_between_unavailable_and_available() {
    // The availableCondition is re-evaluated on every evaluation cycle
    // (AbstractEvaluationCriteriaOperation.java:584-604): true -> dispatch to available
    // (subscription created), false again -> back to unavailable (subscription removed).
    let engine = CmmnEngine::new_in_memory().expect("engine");
    deploy_case(
        &engine,
        "gated-listener-toggle",
        single_case_model("gatedListenerToggleCase", conditional_listener_plan_model()),
    );

    let case_id = start_case(&engine, "gatedListenerToggleCase");
    assert_eq!(subscription_count(&engine, &case_id), 0);

    set_variable(&engine, &case_id, "go", json!(true));
    assert_eq!(subscription_count(&engine, &case_id), 1);

    set_variable(&engine, &case_id, "go", json!(false));
    assert_eq!(subscription_count(&engine, &case_id), 0);
}

#[test]
fn unavailable_event_listener_does_not_block_case_completion() {
    // PlanItemInstanceContainerUtil.java:143-146 — only AVAILABLE/ENABLED plan items block
    // a non-autocomplete container; the gated (unavailable) listener owns no subscription
    // and must not keep the case open once the last task completes.
    let engine = CmmnEngine::new_in_memory().expect("engine");
    deploy_case(
        &engine,
        "gated-listener-completion",
        single_case_model(
            "gatedListenerCompletionCase",
            conditional_listener_plan_model(),
        ),
    );

    let case_id = start_case(&engine, "gatedListenerCompletionCase");

    let task = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_id)
        .state(CmmnHumanTaskState::Active)
        .single_result()
        .expect("task query")
        .expect("keepalive task");
    engine
        .complete_human_task(&task.id, CmmnHumanTaskCompletionRequest::new())
        .expect("completion");

    let case_instance = engine
        .runtime_service()
        .get_case_instance(&case_id)
        .expect("case instance");
    assert_eq!(case_instance.state, CmmnCaseInstanceState::Completed);
}

fn variable_listener_plan_model(change_type: Option<&str>) -> CmmnCasePlanModel {
    let mut listener = CmmnEventListener::new(
        "variable-event-listener",
        CmmnEventListener::EVENT_TYPE_VARIABLE,
    )
    .with_event_name("watchedVar");
    if let Some(change_type) = change_type {
        listener = listener.with_variable_change_type(change_type);
    }
    CmmnCasePlanModel::new("case-plan-model", "Case plan model")
        .with_event_listener(listener)
        .with_plan_item(CmmnPlanItem::new(
            "plan-item-variable-listener",
            "variable-event-listener",
        ))
        .with_human_task(CmmnHumanTask::new("human-task-follow-up", "Follow up"))
        .with_plan_item(
            CmmnPlanItem::new("plan-item-follow-up", "human-task-follow-up")
                .with_entry_criterion("sentry-after-variable-listener"),
        )
        .with_sentry(CmmnSentry::new(
            "sentry-after-variable-listener",
            CmmnPlanItemOnPart::new(
                "on-variable-listener-occur",
                "plan-item-variable-listener",
                CmmnPlanItemOnPart::STANDARD_EVENT_OCCUR,
            ),
        ))
        .with_human_task(CmmnHumanTask::new("human-task-keepalive", "Keep alive"))
        .with_plan_item(CmmnPlanItem::new(
            "plan-item-keepalive",
            "human-task-keepalive",
        ))
}

#[test]
fn variable_event_listener_triggers_on_matching_variable_write() {
    // EvaluateVariableEventListenersOperation.java:58-104 — a "variable" subscription with
    // no configured changeType matches every write ("all") and triggers the listener plan
    // item (occur), which fires the downstream sentry.
    let engine = CmmnEngine::new_in_memory().expect("engine");
    deploy_case(
        &engine,
        "variable-listener-all",
        single_case_model(
            "variableListenerAllCase",
            variable_listener_plan_model(None),
        ),
    );

    let case_id = start_case(&engine, "variableListenerAllCase");
    assert_eq!(subscription_count(&engine, &case_id), 1);

    // Writing an unrelated variable must not trigger the listener (:64 eventName match).
    set_variable(&engine, &case_id, "otherVar", json!("noise"));
    assert_eq!(subscription_count(&engine, &case_id), 1);

    set_variable(&engine, &case_id, "watchedVar", json!("payload"));

    assert_eq!(subscription_count(&engine, &case_id), 0);
    let follow_up = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_id)
        .state(CmmnHumanTaskState::Active)
        .list()
        .expect("task list")
        .into_iter()
        .find(|task| task.name == "Follow up");
    assert!(follow_up.is_some());
}

#[test]
fn variable_event_listener_respects_update_change_type() {
    // EvaluateVariableEventListenersOperation.java:93-95 — a subscription configured with
    // changeType "update" ignores the initial create write and only triggers on an update.
    let engine = CmmnEngine::new_in_memory().expect("engine");
    deploy_case(
        &engine,
        "variable-listener-update",
        single_case_model(
            "variableListenerUpdateCase",
            variable_listener_plan_model(Some(CmmnEventListener::CHANGE_TYPE_UPDATE)),
        ),
    );

    let case_id = start_case(&engine, "variableListenerUpdateCase");
    assert_eq!(subscription_count(&engine, &case_id), 1);

    // First write creates the variable -> changeType "create" does not match "update".
    set_variable(&engine, &case_id, "watchedVar", json!(1));
    assert_eq!(subscription_count(&engine, &case_id), 1);

    // Second write updates the variable -> the listener triggers.
    set_variable(&engine, &case_id, "watchedVar", json!(2));
    assert_eq!(subscription_count(&engine, &case_id), 0);
}
