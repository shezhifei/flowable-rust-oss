//! P91③ — soft hit-policy violations persist into history.
//!
//! Java truth: `PersistHistoricDecisionExecutionCmd.java:73` serializes the
//! whole `DecisionExecutionAuditContainer` as `EXECUTION_JSON_`, so a
//! decision-level `validationMessage` (`DecisionExecutionAuditContainer.java:56,
//! 241-247`) and rule-level messages (`RuleExecutionAuditContainer.java:38,
//! 96-102`) land in the historic row together with the results. Hard failures
//! take the separate `setFailedWithException` path and carry no
//! validationMessage.

use flowable_dmn_engine::{
    DmnDecision, DmnDeploymentRequest, DmnEngine, DmnExecutionRequest, DmnHitPolicy, DmnInputClause,
    DmnModel, DmnOutputClause, DmnRule, DmnRuleInputEntry, DmnRuleOutputEntry, DmnUnaryTest,
};
use serde_json::json;

fn unique_model() -> DmnModel {
    DmnModel::new(vec![DmnDecision::new(
        "decision-1",
        "routingDecision",
        "Routing decision",
        DmnHitPolicy::Unique,
        vec![DmnInputClause::new("input-1", "channel")],
        vec![DmnOutputClause::new("output-1", "route")],
        vec![
            DmnRule::new(
                "rule-1",
                vec![DmnRuleInputEntry::new(DmnUnaryTest::Any)],
                vec![DmnRuleOutputEntry::new(json!("fallback"))],
            ),
            DmnRule::new(
                "rule-2",
                vec![DmnRuleInputEntry::new(DmnUnaryTest::Equals(json!("email")))],
                vec![DmnRuleOutputEntry::new(json!("email-queue"))],
            ),
        ],
    )])
}

fn deploy(engine: &DmnEngine, name: &str) {
    engine
        .repository_service()
        .deploy(DmnDeploymentRequest::new(name).with_resource("routing.dmn", unique_model()))
        .expect("deployment");
}

#[test]
fn soft_violation_validation_message_persists_to_history() {
    // HitPolicyUnique.java:73 + PersistHistoricDecisionExecutionCmd.java:73 —
    // non-strict UNIQUE multi-match: result carries validationMessage and the
    // same message must be readable from the historic row.
    let engine = DmnEngine::builder()
        .strict_mode(false)
        .build_in_memory()
        .expect("engine");
    deploy(&engine, "p91-soft");

    let result = engine
        .decision_service()
        .execute_by_key(
            "routingDecision",
            DmnExecutionRequest::new(json!({ "channel": "email" })),
        )
        .expect("non-strict UNIQUE tolerates multi-match");
    let expected = result
        .validation_message
        .clone()
        .expect("decision-level validationMessage on soft violation");

    let history = engine
        .history_service()
        .create_execution_history_query()
        .decision_key("routingDecision")
        .list()
        .expect("history query");
    assert_eq!(history.len(), 1);
    let row = &history[0];
    assert!(!row.failed);
    assert_eq!(
        row.validation_message.as_deref(),
        Some(expected.as_str()),
        "historic EXECUTION_JSON_ must carry the decision-level validationMessage"
    );
    // Rule-level messages ride inside rule_executions (RuleExecutionAuditContainer.java:38).
    let valid_audits: Vec<_> = row.rule_executions.iter().filter(|r| r.valid).collect();
    assert!(valid_audits.len() >= 2);
    for audit in valid_audits {
        assert!(
            audit.validation_message.is_some(),
            "rule-level validationMessage in history; rule={}",
            audit.rule_id
        );
    }
}

#[test]
fn clean_execution_history_has_no_validation_message() {
    let engine = DmnEngine::new_in_memory().expect("engine");
    deploy(&engine, "p91-clean");

    let result = engine
        .decision_service()
        .execute_by_key(
            "routingDecision",
            // rule-2 Equals(email) does not match "sms", so only rule-1 (Any)
            // matches — a clean single-match UNIQUE execution in strict mode.
            DmnExecutionRequest::new(json!({ "channel": "sms" })),
        )
        .expect("single match in strict mode");
    assert!(result.validation_message.is_none());

    let row = engine
        .history_service()
        .create_execution_history_query()
        .decision_key("routingDecision")
        .single_result()
        .expect("history query")
        .expect("one history row");
    assert!(row.validation_message.is_none());
    // Field must be skipped in the serialized JSON, not serialized as null, so
    // pre-P91 readers (and Java's container) see an identical shape.
    let serialized = serde_json::to_string(&row).expect("serialize");
    assert!(!serialized.contains("validation_message"));
}

#[test]
fn failed_history_carries_no_validation_message() {
    // Strict UNIQUE multi-match: FAILED_=true row via the setFailedWithException
    // path — Java keeps validationMessage empty on it.
    let engine = DmnEngine::new_in_memory().expect("engine");
    deploy(&engine, "p91-failed");

    let error = engine
        .decision_service()
        .execute_by_key(
            "routingDecision",
            DmnExecutionRequest::new(json!({ "channel": "email" })),
        )
        .expect_err("strict UNIQUE rejects multi-match");
    assert!(format!("{error}").contains("UNIQUE"));

    let row = engine
        .history_service()
        .create_execution_history_query()
        .decision_key("routingDecision")
        .failed(true)
        .single_result()
        .expect("history query")
        .expect("one failed history row");
    assert!(row.failed);
    assert!(row.validation_message.is_none());
}
