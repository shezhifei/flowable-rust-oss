//! P89 — DMN hit policy `strictMode` tolerance.
//!
//! Java truth:
//! - `DmnEngineConfiguration.strictMode` default true (`:202`)
//! - UNIQUE non-strict: key-merge + two-level validationMessage
//!   (`HitPolicyUnique.java:38-55,62-77`; test `HitPolicyUniqueTest.java:67-101`)
//! - ANY non-strict: take last matched row + two-level validationMessage
//!   (`HitPolicyAny.java:52-64,71-84`; test `HitPolicyAnyTest.java:96-129`)
//! - PRIORITY no outputValues: only when ≥2 matches; non-strict keeps rule-order first
//!   (`HitPolicyPriority.java:60-78`; test `HitPolicyPriorityTest.java:125-150`)
//! - OUTPUT_ORDER no outputValues: non-strict sort no-op (`HitPolicyOutputOrder.java:53-80`)
//! - Value not in outputValues: never an error; ranks first (`OutputOrderComparator.java:31-33`)
//! - COLLECT non-numeric: hard-fail in any mode (`HitPolicyCollect.java:100,103`)

use flowable_dmn_engine::{
    CollectOperator, DmnDecision, DmnDeploymentRequest, DmnEngine, DmnError, DmnExecutionRequest,
    DmnHitPolicy, DmnInputClause, DmnModel, DmnOutputClause, DmnRule, DmnRuleInputEntry,
    DmnRuleOutputEntry, DmnUnaryTest,
};
use serde_json::json;

fn overlapping_model(hit_policy: DmnHitPolicy, outputs: (&str, &str)) -> DmnModel {
    DmnModel::new(vec![DmnDecision::new(
        "decision-1",
        "routingDecision",
        "Routing decision",
        hit_policy,
        vec![DmnInputClause::new("input-1", "channel")],
        vec![DmnOutputClause::new("output-1", "route")],
        vec![
            DmnRule::new(
                "rule-1",
                vec![DmnRuleInputEntry::new(DmnUnaryTest::Any)],
                vec![DmnRuleOutputEntry::new(json!(outputs.0))],
            ),
            DmnRule::new(
                "rule-2",
                vec![DmnRuleInputEntry::new(DmnUnaryTest::Equals(json!("email")))],
                vec![DmnRuleOutputEntry::new(json!(outputs.1))],
            ),
        ],
    )])
}

/// UNIQUE multi-output model so non-strict key-merge can be observed.
fn unique_multi_output_model() -> DmnModel {
    DmnModel::new(vec![DmnDecision::new(
        "decision-1",
        "routingDecision",
        "Routing decision",
        DmnHitPolicy::Unique,
        vec![DmnInputClause::new("input-1", "channel")],
        vec![
            DmnOutputClause::new("output-1", "route"),
            DmnOutputClause::new("output-2", "priority"),
        ],
        vec![
            DmnRule::new(
                "rule-1",
                vec![DmnRuleInputEntry::new(DmnUnaryTest::Any)],
                vec![
                    DmnRuleOutputEntry::new(json!("manual")),
                    DmnRuleOutputEntry::new(json!(1)),
                ],
            ),
            DmnRule::new(
                "rule-2",
                vec![DmnRuleInputEntry::new(DmnUnaryTest::Equals(json!("email")))],
                vec![
                    DmnRuleOutputEntry::new(json!("email-queue")),
                    // null must not overwrite earlier priority (HitPolicyUnique.java:67-68)
                    DmnRuleOutputEntry::new(json!(null)),
                ],
            ),
        ],
    )])
}

fn priority_without_output_values(hit_policy: DmnHitPolicy) -> DmnModel {
    DmnModel::new(vec![DmnDecision::new(
        "decision-1",
        "riskDecision",
        "Risk decision",
        hit_policy,
        vec![DmnInputClause::new("input-1", "score")],
        vec![DmnOutputClause::new("output-1", "riskBand")],
        vec![
            DmnRule::new(
                "rule-1",
                vec![DmnRuleInputEntry::new(DmnUnaryTest::GreaterThanOrEqual(
                    json!(0),
                ))],
                vec![DmnRuleOutputEntry::new(json!("LOW"))],
            ),
            DmnRule::new(
                "rule-2",
                vec![DmnRuleInputEntry::new(DmnUnaryTest::GreaterThanOrEqual(
                    json!(50),
                ))],
                vec![DmnRuleOutputEntry::new(json!("MEDIUM"))],
            ),
            DmnRule::new(
                "rule-3",
                vec![DmnRuleInputEntry::new(DmnUnaryTest::GreaterThanOrEqual(
                    json!(80),
                ))],
                vec![DmnRuleOutputEntry::new(json!("HIGH"))],
            ),
        ],
    )])
}

fn risk_with_output_values_and_unknown(hit_policy: DmnHitPolicy) -> DmnModel {
    DmnModel::new(vec![DmnDecision::new(
        "decision-1",
        "riskDecision",
        "Risk decision",
        hit_policy,
        vec![DmnInputClause::new("input-1", "score")],
        vec![
            DmnOutputClause::new("output-1", "riskBand").with_output_values(vec![
                json!("HIGH"),
                json!("MEDIUM"),
                json!("LOW"),
            ]),
        ],
        vec![
            DmnRule::new(
                "rule-1",
                vec![DmnRuleInputEntry::new(DmnUnaryTest::GreaterThanOrEqual(
                    json!(0),
                ))],
                vec![DmnRuleOutputEntry::new(json!("LOW"))],
            ),
            DmnRule::new(
                "rule-2",
                vec![DmnRuleInputEntry::new(DmnUnaryTest::GreaterThanOrEqual(
                    json!(50),
                ))],
                // Not in outputValues — Java ranks first (OutputOrderComparator.java:31-33)
                vec![DmnRuleOutputEntry::new(json!("UNKNOWN"))],
            ),
            DmnRule::new(
                "rule-3",
                vec![DmnRuleInputEntry::new(DmnUnaryTest::GreaterThanOrEqual(
                    json!(80),
                ))],
                vec![DmnRuleOutputEntry::new(json!("HIGH"))],
            ),
        ],
    )])
}

fn non_strict_engine() -> DmnEngine {
    DmnEngine::builder()
        .strict_mode(false)
        .build_in_memory()
        .expect("engine")
}

fn strict_engine() -> DmnEngine {
    DmnEngine::new_in_memory().expect("engine")
}

// ---------------------------------------------------------------------------
// UNIQUE
// ---------------------------------------------------------------------------

#[test]
fn unique_non_strict_merges_by_key_with_two_level_validation_messages() {
    // HitPolicyUniqueTest.java:67-101 — non-strict multi-match returns a result
    // with decision- and rule-level validationMessage; failed=false.
    let engine = non_strict_engine();
    engine
        .repository_service()
        .deploy(
            DmnDeploymentRequest::new("unique-nonstrict")
                .with_resource("routing.dmn", unique_multi_output_model()),
        )
        .expect("deployment");

    let result = engine
        .decision_service()
        .execute_by_key(
            "routingDecision",
            DmnExecutionRequest::new(json!({ "channel": "email" })),
        )
        .expect("UNIQUE non-strict must tolerate multi-match");

    assert_eq!(result.hit_policy, DmnHitPolicy::Unique);
    // later non-null overwrites earlier (route from rule-2); null does not overwrite priority
    assert_eq!(result.get_output("route"), Some(&json!("email-queue")));
    assert_eq!(result.get_output("priority"), Some(&json!(1)));
    assert!(
        result.validation_message.is_some(),
        "decision-level validationMessage required (HitPolicyUnique.java:73)"
    );
    let valid_audits: Vec<_> = result
        .rule_executions
        .iter()
        .filter(|r| r.valid)
        .collect();
    assert!(valid_audits.len() >= 2);
    for audit in valid_audits {
        assert!(
            audit.validation_message.is_some(),
            "rule-level validationMessage on valid rules (HitPolicyUnique.java:49-50); rule={}",
            audit.rule_id
        );
        assert!(audit.exception_message.is_none());
    }
}

#[test]
fn unique_strict_rejects_overlapping_matches() {
    let engine = strict_engine();
    engine
        .repository_service()
        .deploy(DmnDeploymentRequest::new("unique-strict").with_resource(
            "routing.dmn",
            overlapping_model(DmnHitPolicy::Unique, ("manual", "email-queue")),
        ))
        .expect("deployment");

    let error = engine
        .decision_service()
        .execute_by_key(
            "routingDecision",
            DmnExecutionRequest::new(json!({ "channel": "email" })),
        )
        .expect_err("UNIQUE strict must fail on multi-match");

    assert!(matches!(error, DmnError::Execution { .. }));
    assert!(
        error.to_string().contains("UNIQUE") && error.to_string().contains("rule-1"),
        "unexpected error: {error}"
    );
}

// ---------------------------------------------------------------------------
// ANY
// ---------------------------------------------------------------------------

#[test]
fn any_non_strict_takes_last_matched_row_with_two_level_validation_messages() {
    // HitPolicyAnyTest.java:96-129 — last matching rule's outputs win.
    let engine = non_strict_engine();
    engine
        .repository_service()
        .deploy(DmnDeploymentRequest::new("any-nonstrict").with_resource(
            "routing.dmn",
            overlapping_model(DmnHitPolicy::Any, ("manual", "email-queue")),
        ))
        .expect("deployment");

    let result = engine
        .decision_service()
        .execute_by_key(
            "routingDecision",
            DmnExecutionRequest::new(json!({ "channel": "email" })),
        )
        .expect("ANY non-strict must tolerate conflicting outputs");

    assert_eq!(result.hit_policy, DmnHitPolicy::Any);
    // last matched rule (rule-2) — NOT the first (rule-1 "manual")
    assert_eq!(result.get_output("route"), Some(&json!("email-queue")));
    assert_eq!(result.matched_rule_id.as_deref(), Some("rule-2"));
    assert!(
        result.validation_message.is_some(),
        "decision-level validationMessage (HitPolicyAny.java:74)"
    );
    let valid_audits: Vec<_> = result
        .rule_executions
        .iter()
        .filter(|r| r.valid)
        .collect();
    assert!(valid_audits.len() >= 2);
    for audit in valid_audits {
        assert!(
            audit.validation_message.is_some(),
            "rule-level validationMessage (HitPolicyAny.java:60-61); rule={}",
            audit.rule_id
        );
    }
}

#[test]
fn any_strict_rejects_conflicting_outputs() {
    let engine = strict_engine();
    engine
        .repository_service()
        .deploy(DmnDeploymentRequest::new("any-strict").with_resource(
            "routing.dmn",
            overlapping_model(DmnHitPolicy::Any, ("manual", "email-queue")),
        ))
        .expect("deployment");

    let error = engine
        .decision_service()
        .execute_by_key(
            "routingDecision",
            DmnExecutionRequest::new(json!({ "channel": "email" })),
        )
        .expect_err("ANY strict must fail on different outputs");

    assert!(matches!(error, DmnError::Execution { .. }));
    assert!(
        error.to_string().contains("ANY") && error.to_string().contains("rule-2"),
        "unexpected error: {error}"
    );
}

// ---------------------------------------------------------------------------
// PRIORITY
// ---------------------------------------------------------------------------

#[test]
fn priority_non_strict_no_output_values_keeps_rule_order_first() {
    // HitPolicyPriorityTest.java:125-150 — non-strict, no outputValues → first
    // valid result in rule order + decision-level validationMessage.
    let engine = non_strict_engine();
    engine
        .repository_service()
        .deploy(
            DmnDeploymentRequest::new("priority-nonstrict").with_resource(
                "risk.dmn",
                priority_without_output_values(DmnHitPolicy::Priority),
            ),
        )
        .expect("deployment");

    let result = engine
        .decision_service()
        .execute_by_key(
            "riskDecision",
            DmnExecutionRequest::new(json!({ "score": 90 })),
        )
        .expect("PRIORITY non-strict must tolerate missing outputValues");

    assert_eq!(result.hit_policy, DmnHitPolicy::Priority);
    // rule order first among matches (rule-1 LOW), not priority-sorted HIGH
    assert_eq!(result.get_output("riskBand"), Some(&json!("LOW")));
    assert_eq!(result.matched_rule_id.as_deref(), Some("rule-1"));
    assert!(
        result.validation_message.is_some(),
        "decision-level validationMessage (HitPolicyPriority.java:66-69)"
    );
    // PRIORITY only writes decision-level messages, not rule-level
    for audit in &result.rule_executions {
        assert!(audit.validation_message.is_none());
    }
}

#[test]
fn priority_strict_no_output_values_rejects_when_multiple_matches() {
    let engine = strict_engine();
    engine
        .repository_service()
        .deploy(
            DmnDeploymentRequest::new("priority-strict").with_resource(
                "risk.dmn",
                priority_without_output_values(DmnHitPolicy::Priority),
            ),
        )
        .expect("deployment");

    let error = engine
        .decision_service()
        .execute_by_key(
            "riskDecision",
            DmnExecutionRequest::new(json!({ "score": 90 })),
        )
        .expect_err("PRIORITY strict multi-match without outputValues must fail");

    assert!(matches!(error, DmnError::Execution { .. }));
    assert!(
        error.to_string().contains("no output values")
            || error.to_string().contains("Priority")
            || error.to_string().contains("PRIORITY"),
        "unexpected error: {error}"
    );
}

#[test]
fn priority_single_match_without_output_values_does_not_violate() {
    // HitPolicyPriority comparator only runs for ≥2 rows — single match is fine
    // even without outputValues (strict or not).
    let engine = strict_engine();
    engine
        .repository_service()
        .deploy(
            DmnDeploymentRequest::new("priority-single").with_resource(
                "risk.dmn",
                priority_without_output_values(DmnHitPolicy::Priority),
            ),
        )
        .expect("deployment");

    let result = engine
        .decision_service()
        .execute_by_key(
            "riskDecision",
            // score 10 → only rule-1 matches
            DmnExecutionRequest::new(json!({ "score": 10 })),
        )
        .expect("single-match PRIORITY without outputValues must succeed");

    assert_eq!(result.get_output("riskBand"), Some(&json!("LOW")));
    assert!(
        result.validation_message.is_none(),
        "single match must not set validationMessage"
    );
}

// ---------------------------------------------------------------------------
// OUTPUT_ORDER
// ---------------------------------------------------------------------------

#[test]
fn output_order_non_strict_no_output_values_keeps_rule_order() {
    let engine = non_strict_engine();
    engine
        .repository_service()
        .deploy(
            DmnDeploymentRequest::new("output-order-nonstrict").with_resource(
                "risk.dmn",
                priority_without_output_values(DmnHitPolicy::OutputOrder),
            ),
        )
        .expect("deployment");

    let result = engine
        .decision_service()
        .execute_by_key(
            "riskDecision",
            DmnExecutionRequest::new(json!({ "score": 90 })),
        )
        .expect("OUTPUT_ORDER non-strict must tolerate missing outputValues");

    assert_eq!(result.hit_policy, DmnHitPolicy::OutputOrder);
    assert!(result.multiple_results);
    // sort no-op → rule definition order
    assert_eq!(
        result.decision_result,
        vec![
            serde_json::Map::from_iter([("riskBand".to_string(), json!("LOW"))]),
            serde_json::Map::from_iter([("riskBand".to_string(), json!("MEDIUM"))]),
            serde_json::Map::from_iter([("riskBand".to_string(), json!("HIGH"))]),
        ]
    );
    assert!(
        result.validation_message.is_some(),
        "decision-level validationMessage (HitPolicyOutputOrder.java:58)"
    );
}

#[test]
fn output_order_strict_no_output_values_rejects() {
    let engine = strict_engine();
    engine
        .repository_service()
        .deploy(
            DmnDeploymentRequest::new("output-order-strict").with_resource(
                "risk.dmn",
                priority_without_output_values(DmnHitPolicy::OutputOrder),
            ),
        )
        .expect("deployment");

    let error = engine
        .decision_service()
        .execute_by_key(
            "riskDecision",
            DmnExecutionRequest::new(json!({ "score": 90 })),
        )
        .expect_err("OUTPUT_ORDER strict without outputValues must fail");

    assert!(matches!(error, DmnError::Execution { .. }));
    assert!(
        error.to_string().contains("no output values")
            || error.to_string().contains("OutputOrder")
            || error.to_string().contains("OUTPUT_ORDER"),
        "unexpected error: {error}"
    );
}

// ---------------------------------------------------------------------------
// Value not listed in outputValues → ranks first, never errors
// ---------------------------------------------------------------------------

#[test]
fn unknown_output_value_ranks_first_in_strict_output_order() {
    let engine = strict_engine();
    engine
        .repository_service()
        .deploy(
            DmnDeploymentRequest::new("unknown-strict").with_resource(
                "risk.dmn",
                risk_with_output_values_and_unknown(DmnHitPolicy::OutputOrder),
            ),
        )
        .expect("deployment");

    let result = engine
        .decision_service()
        .execute_by_key(
            "riskDecision",
            DmnExecutionRequest::new(json!({ "score": 90 })),
        )
        .expect("value not in outputValues must not error (OutputOrderComparator.java:31-33)");

    // UNKNOWN (not listed) ranks first; then HIGH, then LOW (MEDIUM not matched at 90? wait)
    // score 90 matches all three rules: LOW, UNKNOWN, HIGH
    // ranks: UNKNOWN=0, HIGH=1, LOW=3 → order UNKNOWN, HIGH, LOW
    assert_eq!(
        result.decision_result,
        vec![
            serde_json::Map::from_iter([("riskBand".to_string(), json!("UNKNOWN"))]),
            serde_json::Map::from_iter([("riskBand".to_string(), json!("HIGH"))]),
            serde_json::Map::from_iter([("riskBand".to_string(), json!("LOW"))]),
        ]
    );
    assert!(result.validation_message.is_none());
}

#[test]
fn unknown_output_value_ranks_first_in_non_strict_priority() {
    let engine = non_strict_engine();
    engine
        .repository_service()
        .deploy(
            DmnDeploymentRequest::new("unknown-nonstrict").with_resource(
                "risk.dmn",
                risk_with_output_values_and_unknown(DmnHitPolicy::Priority),
            ),
        )
        .expect("deployment");

    let result = engine
        .decision_service()
        .execute_by_key(
            "riskDecision",
            DmnExecutionRequest::new(json!({ "score": 90 })),
        )
        .expect("value not in outputValues must not error");

    // PRIORITY takes highest-priority (lowest rank) → UNKNOWN
    assert_eq!(result.get_output("riskBand"), Some(&json!("UNKNOWN")));
    assert_eq!(result.matched_rule_id.as_deref(), Some("rule-2"));
    assert!(result.validation_message.is_none());
}

// ---------------------------------------------------------------------------
// COLLECT non-numeric — hard fail both modes
// ---------------------------------------------------------------------------

fn collect_sum_non_numeric_model() -> DmnModel {
    DmnModel::new(vec![DmnDecision::new(
        "decision-1",
        "sumDecision",
        "Sum decision",
        DmnHitPolicy::Collect,
        vec![DmnInputClause::new("input-1", "flag")],
        vec![DmnOutputClause::new("output-1", "amount")],
        vec![
            DmnRule::new(
                "rule-1",
                vec![DmnRuleInputEntry::new(DmnUnaryTest::Any)],
                vec![DmnRuleOutputEntry::new(json!("not-a-number"))],
            ),
            DmnRule::new(
                "rule-2",
                vec![DmnRuleInputEntry::new(DmnUnaryTest::Any)],
                vec![DmnRuleOutputEntry::new(json!("also-text"))],
            ),
        ],
    )
    .with_collect_operator(CollectOperator::Sum)])
}

#[test]
fn collect_non_numeric_hard_fails_in_strict_mode() {
    let engine = strict_engine();
    // Deploy may validate numeric COLLECT outputs at deploy time — if so, that's
    // also a hard failure path. Prefer runtime if deploy succeeds.
    let deploy = engine.repository_service().deploy(
        DmnDeploymentRequest::new("collect-strict")
            .with_resource("sum.dmn", collect_sum_non_numeric_model()),
    );
    match deploy {
        Ok(_) => {
            let error = engine
                .decision_service()
                .execute_by_key(
                    "sumDecision",
                    DmnExecutionRequest::new(json!({ "flag": true })),
                )
                .expect_err("COLLECT non-numeric must hard-fail");
            assert!(matches!(
                error,
                DmnError::Execution { .. } | DmnError::Validation { .. }
            ));
        }
        Err(error) => {
            // Deploy-time numeric guard is also a hard failure (no strict branch).
            assert!(
                matches!(
                    error,
                    DmnError::Execution { .. }
                        | DmnError::Validation { .. }
                        | DmnError::UnsupportedModel { .. }
                ),
                "unexpected deploy error: {error}"
            );
        }
    }
}

#[test]
fn collect_non_numeric_hard_fails_in_non_strict_mode() {
    let engine = non_strict_engine();
    let deploy = engine.repository_service().deploy(
        DmnDeploymentRequest::new("collect-nonstrict")
            .with_resource("sum.dmn", collect_sum_non_numeric_model()),
    );
    match deploy {
        Ok(_) => {
            let error = engine
                .decision_service()
                .execute_by_key(
                    "sumDecision",
                    DmnExecutionRequest::new(json!({ "flag": true })),
                )
                .expect_err("COLLECT non-numeric must hard-fail even when non-strict");
            assert!(matches!(
                error,
                DmnError::Execution { .. } | DmnError::Validation { .. }
            ));
        }
        Err(error) => {
            assert!(
                matches!(
                    error,
                    DmnError::Execution { .. }
                        | DmnError::Validation { .. }
                        | DmnError::UnsupportedModel { .. }
                ),
                "unexpected deploy error: {error}"
            );
        }
    }
}

#[test]
fn default_engine_is_strict_mode() {
    let engine = DmnEngine::new_in_memory().expect("engine");
    assert!(
        engine.decision_service().strict_mode(),
        "default strictMode must be true (DmnEngineConfiguration.java:202)"
    );
}
