use flowable_dmn_engine::{
    DmnDecision, DmnDeploymentRequest, DmnEngine, DmnExecutionRequest, DmnHitPolicy,
    DmnInputClause, DmnModel, DmnOutputClause, DmnRule, DmnRuleInputEntry, DmnRuleOutputEntry,
    DmnUnaryTest,
};
use serde_json::json;

fn sample_model() -> DmnModel {
    DmnModel::new(vec![DmnDecision::new(
        "decision-1",
        "dishDecision",
        "Dish decision",
        DmnHitPolicy::First,
        vec![DmnInputClause::new("input-1", "dishType")],
        vec![DmnOutputClause::new("output-1", "dish")],
        vec![
            DmnRule::new(
                "rule-1",
                vec![DmnRuleInputEntry::new(DmnUnaryTest::Equals(json!("salad")))],
                vec![DmnRuleOutputEntry::new(json!("light"))],
            ),
            DmnRule::new(
                "rule-2",
                vec![DmnRuleInputEntry::new(DmnUnaryTest::Any)],
                vec![DmnRuleOutputEntry::new(json!("default"))],
            ),
        ],
    )])
}

#[test]
fn records_execution_history_and_filters_by_decision_key() {
    let engine = DmnEngine::new_in_memory().expect("engine");
    engine
        .repository_service()
        .deploy(
            DmnDeploymentRequest::new("history").with_resource("dish-decision.dmn", sample_model()),
        )
        .expect("deployment");

    engine
        .decision_service()
        .execute_by_key(
            "dishDecision",
            DmnExecutionRequest::new(json!({
                "dishType": "salad"
            }))
            .with_business_key("order-1"),
        )
        .expect("execution");
    engine
        .decision_service()
        .execute_by_key(
            "dishDecision",
            DmnExecutionRequest::new(json!({
                "dishType": "soup"
            }))
            .with_business_key("order-2"),
        )
        .expect("execution");

    let history = engine
        .history_service()
        .create_execution_history_query()
        .decision_key("dishDecision")
        .list()
        .expect("history");

    assert_eq!(history.len(), 2);
    assert_eq!(history[0].business_key.as_deref(), Some("order-1"));
    assert_eq!(history[0].get_output("dish"), Some(&json!("light")));
    assert_eq!(history[1].business_key.as_deref(), Some("order-2"));
    assert_eq!(history[1].get_output("dish"), Some(&json!("default")));
}

#[test]
fn returns_single_execution_record_by_execution_id() {
    let engine = DmnEngine::new_in_memory().expect("engine");
    engine
        .repository_service()
        .deploy(
            DmnDeploymentRequest::new("history").with_resource("dish-decision.dmn", sample_model()),
        )
        .expect("deployment");

    let execution = engine
        .decision_service()
        .execute_by_key(
            "dishDecision",
            DmnExecutionRequest::new(json!({
                "dishType": "salad"
            })),
        )
        .expect("execution");

    let historic = engine
        .history_service()
        .create_execution_history_query()
        .execution_id(&execution.execution_id)
        .single_result()
        .expect("history query")
        .expect("history record");

    assert_eq!(historic.execution_id, execution.execution_id);
    assert_eq!(historic.matched_rule_id.as_deref(), Some("rule-1"));
}

#[test]
fn historic_execution_query_filters_sorts_and_pages() {
    let engine = DmnEngine::new_in_memory().expect("engine");
    let deployment = engine
        .repository_service()
        .deploy(
            DmnDeploymentRequest::new("history").with_resource("dish-decision.dmn", sample_model()),
        )
        .expect("deployment");

    let first = engine
        .decision_service()
        .execute_by_key(
            "dishDecision",
            DmnExecutionRequest::new(json!({
                "dishType": "salad"
            }))
            .with_business_key("order-1"),
        )
        .expect("execution");
    std::thread::sleep(std::time::Duration::from_millis(5));
    let second = engine
        .decision_service()
        .execute_by_key(
            "dishDecision",
            DmnExecutionRequest::new(json!({
                "dishType": "soup"
            }))
            .with_business_key("order-2"),
        )
        .expect("execution");

    let history = engine
        .history_service()
        .create_execution_history_query()
        .deployment_id(&deployment.id)
        .business_key("order-2")
        .order_by_execution_time()
        .desc()
        .page(0, 1)
        .list_page()
        .expect("history");

    assert_eq!(history.start, 0);
    assert_eq!(history.size, 1);
    assert_eq!(history.total, 1);
    assert_eq!(history.data[0].execution_id, second.execution_id);

    let all_desc = engine
        .history_service()
        .create_execution_history_query()
        .decision_key("dishDecision")
        .order_by_execution_time()
        .desc()
        .list()
        .expect("history");

    assert_eq!(all_desc[0].execution_id, second.execution_id);
    assert_eq!(all_desc[1].execution_id, first.execution_id);
}

#[test]
fn historic_execution_delete_removes_records_and_reports_missing_single_delete() {
    let engine = DmnEngine::new_in_memory().expect("engine");
    engine
        .repository_service()
        .deploy(
            DmnDeploymentRequest::new("history").with_resource("dish-decision.dmn", sample_model()),
        )
        .expect("deployment");

    let first = engine
        .decision_service()
        .execute_by_key(
            "dishDecision",
            DmnExecutionRequest::new(json!({
                "dishType": "salad"
            })),
        )
        .expect("execution");
    let second = engine
        .decision_service()
        .execute_by_key(
            "dishDecision",
            DmnExecutionRequest::new(json!({
                "dishType": "soup"
            })),
        )
        .expect("execution");

    engine
        .history_service()
        .delete_historic_decision_execution(&first.execution_id)
        .expect("delete");
    assert!(
        engine
            .history_service()
            .create_execution_history_query()
            .execution_id(&first.execution_id)
            .single_result()
            .expect("history")
            .is_none()
    );

    let missing = engine
        .history_service()
        .delete_historic_decision_execution(&first.execution_id)
        .expect_err("second delete should report not found");
    assert!(missing.to_string().contains("was not found"));

    engine
        .history_service()
        .bulk_delete_historic_decision_executions(std::slice::from_ref(&second.execution_id))
        .expect("bulk delete");
    assert_eq!(
        engine
            .history_service()
            .create_execution_history_query()
            .list()
            .expect("history")
            .len(),
        0
    );
}

#[test]
fn execution_request_can_disable_history_persistence() {
    let engine = DmnEngine::new_in_memory().expect("engine");
    engine
        .repository_service()
        .deploy(
            DmnDeploymentRequest::new("history").with_resource("dish-decision.dmn", sample_model()),
        )
        .expect("deployment");

    let request = DmnExecutionRequest::new(json!({
        "dishType": "salad"
    }))
    .with_business_key("order-no-history")
    .disable_history();

    let execution = engine
        .decision_service()
        .execute_by_key("dishDecision", request)
        .expect("execution");

    assert_eq!(execution.get_output("dish"), Some(&json!("light")));
    assert_eq!(
        engine
            .history_service()
            .create_execution_history_query()
            .business_key("order-no-history")
            .list()
            .expect("history")
            .len(),
        0
    );
}
