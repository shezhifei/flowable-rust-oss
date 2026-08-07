//! P83 task B — DMN historic decision execution carries process correlation.
//!
//! Java reference: `DmnActivityBehavior.java:99-103` sets instanceId /
//! executionId / activityId on the `ExecuteDecisionBuilder`;
//! `PersistHistoricDecisionExecutionCmd.java:56-59` writes them (plus
//! `SCOPE_TYPE_`) onto the historic decision execution row.

use flowable_dmn_engine::{
    DmnDecision, DmnDeploymentRequest, DmnEngine, DmnExecutionRequest, DmnHitPolicy,
    DmnInputClause, DmnModel, DmnOutputClause, DmnRule, DmnRuleInputEntry, DmnRuleOutputEntry,
    DmnUnaryTest, HistoricDecisionExecution,
};
use serde_json::json;

fn dish_decision() -> DmnDecision {
    DmnDecision::new(
        "decision-1",
        "dishDecision",
        "Dish decision",
        DmnHitPolicy::First,
        vec![DmnInputClause::new("input-1", "dishType")],
        vec![DmnOutputClause::new("output-1", "dish")],
        vec![DmnRule::new(
            "rule-1",
            vec![DmnRuleInputEntry::new(DmnUnaryTest::Any)],
            vec![DmnRuleOutputEntry::new(json!("light"))],
        )],
    )
}

fn engine_with_decision() -> DmnEngine {
    let engine = DmnEngine::new_in_memory().expect("engine");
    engine
        .repository_service()
        .deploy(
            DmnDeploymentRequest::new("audit")
                .with_resource("dish.dmn", DmnModel::new(vec![dish_decision()])),
        )
        .expect("deployment");
    engine
}

fn single_history_row(engine: &DmnEngine) -> HistoricDecisionExecution {
    let rows = engine
        .history_service()
        .create_execution_history_query()
        .list()
        .expect("history query");
    assert_eq!(rows.len(), 1, "expected exactly one history row");
    rows.into_iter().next().unwrap()
}

#[test]
fn correlation_ids_are_persisted_on_the_history_row() {
    let engine = engine_with_decision();

    engine
        .decision_service()
        .execute_by_key(
            "dishDecision",
            DmnExecutionRequest::new(json!({"dishType": "salad"}))
                .with_audit_correlation(
                    Some("proc-inst-1".to_string()),
                    Some("exec-7".to_string()),
                    Some("dmnTask1".to_string()),
                )
                .with_scope_type("bpmn"),
        )
        .expect("execution");

    let row = single_history_row(&engine);
    assert_eq!(row.instance_id.as_deref(), Some("proc-inst-1"));
    assert_eq!(row.scope_execution_id.as_deref(), Some("exec-7"));
    assert_eq!(row.activity_id.as_deref(), Some("dmnTask1"));
    assert_eq!(row.scope_type.as_deref(), Some("bpmn"));
    // The DMN execution's own id (Java's `ID_`) stays distinct from the
    // process execution id.
    assert!(row.execution_id.starts_with("dmn-execution:"));
}

/// Direct API executions carry no process context — Java leaves those columns
/// null (`ExecuteDecisionContext` fields simply never set).
#[test]
fn direct_api_execution_leaves_correlation_ids_empty() {
    let engine = engine_with_decision();

    engine
        .decision_service()
        .execute_by_key(
            "dishDecision",
            DmnExecutionRequest::new(json!({"dishType": "salad"})),
        )
        .expect("execution");

    let row = single_history_row(&engine);
    assert_eq!(row.instance_id, None);
    assert_eq!(row.scope_execution_id, None);
    assert_eq!(row.activity_id, None);
    assert_eq!(row.scope_type, None);
}

/// Pre-P83 history JSON has none of the new keys; it must still load.
#[test]
fn legacy_history_json_without_correlation_deserializes() {
    let legacy = json!({
        "execution_id": "dmn-execution:legacy",
        "decision_definition_id": "def-1",
        "deployment_id": "dep-1",
        "decision_key": "dishDecision",
        "decision_name": "Dish decision",
        "decision_version": 1,
        "hit_policy": "First",
        "matched_rule_id": "rule-1",
        "matched_rule_count": 1,
        "rule_executions": [],
        "business_key": null,
        "tenant_id": null,
        "executed_at": "2026-01-01T00:00:00Z",
        "inputs": {"dishType": "salad"},
        "decision_result": [{"dish": "light"}],
        "multiple_results": false
    });

    let row: HistoricDecisionExecution =
        serde_json::from_value(legacy).expect("legacy history JSON should deserialize");

    assert_eq!(row.execution_id, "dmn-execution:legacy");
    assert_eq!(row.instance_id, None);
    assert_eq!(row.scope_execution_id, None);
    assert_eq!(row.activity_id, None);
    assert_eq!(row.scope_type, None);
}

/// A failed evaluation still writes a history row.
///
/// Java catches the evaluation error, clears the rule results and records it on
/// the audit container (`RuleEngineExecutorImpl.java:154-158`) rather than
/// aborting, so `finalizeDecisionExecutionAudit`
/// (`DmnDecisionServiceImpl.java:238-248`) persists the row with `FAILED_ = true`
/// (`PersistHistoricDecisionExecutionCmd.java:62-65`) — correlation columns
/// included. The caller then throws (`DmnActivityBehavior.java:112-115`).
#[test]
fn failed_evaluation_still_records_a_history_row_with_correlation() {
    let engine = DmnEngine::new_in_memory().expect("engine");
    // UNIQUE hit policy with two rules that both match → evaluation error.
    let conflicting = DmnDecision::new(
        "decision-1",
        "dishDecision",
        "Dish decision",
        DmnHitPolicy::Unique,
        vec![DmnInputClause::new("input-1", "dishType")],
        vec![DmnOutputClause::new("output-1", "dish")],
        vec![
            DmnRule::new(
                "rule-1",
                vec![DmnRuleInputEntry::new(DmnUnaryTest::Any)],
                vec![DmnRuleOutputEntry::new(json!("light"))],
            ),
            DmnRule::new(
                "rule-2",
                vec![DmnRuleInputEntry::new(DmnUnaryTest::Any)],
                vec![DmnRuleOutputEntry::new(json!("heavy"))],
            ),
        ],
    );
    engine
        .repository_service()
        .deploy(
            DmnDeploymentRequest::new("audit-failed")
                .with_resource("dish.dmn", DmnModel::new(vec![conflicting])),
        )
        .expect("deployment");

    let error = engine
        .decision_service()
        .execute_by_key(
            "dishDecision",
            DmnExecutionRequest::new(json!({"dishType": "salad"}))
                .with_audit_correlation(
                    Some("proc-inst-3".to_string()),
                    Some("exec-11".to_string()),
                    Some("dmnTask3".to_string()),
                )
                .with_scope_type("bpmn"),
        )
        .expect_err("UNIQUE hit policy violation");
    assert!(
        error.to_string().contains("UNIQUE hit policy violation"),
        "unexpected error: {error}"
    );

    let row = single_history_row(&engine);
    assert!(row.failed, "history row should be marked failed");
    // Java clears the rule results before persisting (:156).
    assert!(row.decision_result.is_empty());
    assert_eq!(row.matched_rule_id, None);
    assert_eq!(row.matched_rule_count, 0);
    // Correlation is written on the failed row too (:56-59).
    assert_eq!(row.instance_id.as_deref(), Some("proc-inst-3"));
    assert_eq!(row.scope_execution_id.as_deref(), Some("exec-11"));
    assert_eq!(row.activity_id.as_deref(), Some("dmnTask3"));
    assert_eq!(row.scope_type.as_deref(), Some("bpmn"));
    // Inputs are preserved so the failure can be diagnosed.
    assert_eq!(row.inputs["dishType"], json!("salad"));
}

/// A successful execution is not marked failed.
#[test]
fn successful_execution_is_not_marked_failed() {
    let engine = engine_with_decision();

    engine
        .decision_service()
        .execute_by_key(
            "dishDecision",
            DmnExecutionRequest::new(json!({"dishType": "salad"})),
        )
        .expect("execution");

    assert!(!single_history_row(&engine).failed);
}

/// Child decisions of a DRD inherit the caller's correlation, because Java
/// keeps a single `ExecuteDecisionContext` for the whole execution.
#[test]
fn required_decisions_inherit_correlation() {
    let engine = DmnEngine::new_in_memory().expect("engine");
    let child = DmnDecision::new(
        "child-1",
        "childDecision",
        "Child",
        DmnHitPolicy::First,
        vec![DmnInputClause::new("input-1", "dishType")],
        vec![DmnOutputClause::new("output-1", "portion")],
        vec![DmnRule::new(
            "rule-1",
            vec![DmnRuleInputEntry::new(DmnUnaryTest::Any)],
            vec![DmnRuleOutputEntry::new(json!("large"))],
        )],
    );
    let mut parent = dish_decision();
    parent.required_decisions = vec!["childDecision".to_string()];

    engine
        .repository_service()
        .deploy(
            DmnDeploymentRequest::new("drd")
                .with_resource("drd.dmn", DmnModel::new(vec![child, parent])),
        )
        .expect("deployment");

    engine
        .decision_service()
        .execute_by_key(
            "dishDecision",
            DmnExecutionRequest::new(json!({"dishType": "salad"})).with_audit_correlation(
                Some("proc-inst-2".to_string()),
                Some("exec-9".to_string()),
                Some("dmnTask2".to_string()),
            ),
        )
        .expect("execution");

    let rows = engine
        .history_service()
        .create_execution_history_query()
        .list()
        .expect("history query");
    assert_eq!(rows.len(), 2, "parent + child decisions each record history");
    for row in rows {
        assert_eq!(
            row.instance_id.as_deref(),
            Some("proc-inst-2"),
            "decision '{}' lost its correlation",
            row.decision_key
        );
        assert_eq!(row.scope_execution_id.as_deref(), Some("exec-9"));
        assert_eq!(row.activity_id.as_deref(), Some("dmnTask2"));
    }
}
