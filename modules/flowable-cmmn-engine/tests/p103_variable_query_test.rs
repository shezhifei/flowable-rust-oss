// P103: CMMN case-instance variable-condition query on the engine API.
//
// Java: BaseCaseInstanceResource.java:204-206 + addVariables (:292-376);
// QueryVariable.java:74-96 operation enum.

use flowable_cmmn_engine::{
    CmmnCase, CmmnCaseInstanceStartRequest, CmmnCasePlanModel, CmmnDeploymentRequest, CmmnEngine,
    CmmnHumanTask, CmmnModel, CmmnPlanItem, QueryVariableCondition, QueryVariableOperation,
};
use serde_json::{Map, Value, json};

fn model() -> CmmnModel {
    let plan_model = CmmnCasePlanModel::new("case-plan-model", "Case plan model")
        .with_human_task(CmmnHumanTask::new("task-a", "Task A"))
        .with_plan_item(CmmnPlanItem::new("plan-item-a", "task-a"));
    CmmnModel::new(vec![CmmnCase::new(
        "case-p103",
        "p103VarCase",
        "P103 variable case",
        plan_model,
    )])
}

fn deploy_and_start(engine: &CmmnEngine, variables: Value) -> String {
    engine
        .deploy(
            CmmnDeploymentRequest::new("p103-deployment").with_resource("p103.cmmn", model()),
        )
        .expect("deploy");
    let mut request = CmmnCaseInstanceStartRequest::new();
    request.variables = variables;
    engine
        .start_case_instance_by_key("p103VarCase", request)
        .expect("start")
        .id
}

fn cond(name: &str, op: QueryVariableOperation, value: Value) -> QueryVariableCondition {
    QueryVariableCondition {
        name: Some(name.to_string()),
        operation: op,
        value,
    }
}

#[test]
fn case_query_filters_by_variable_conditions() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let id_hit = deploy_and_start(
        &engine,
        json!({ "amount": 100, "label": "Alpha", "flag": true }),
    );
    // Second case that should not match amount equals 100.
    let mut request = CmmnCaseInstanceStartRequest::new();
    request.variables = json!({ "amount": 50, "label": "Beta" });
    let id_miss = engine
        .start_case_instance_by_key("p103VarCase", request)
        .expect("start miss")
        .id;

    let hits = engine
        .runtime_service()
        .create_case_instance_query()
        .variable_conditions(vec![cond(
            "amount",
            QueryVariableOperation::Equals,
            json!(100),
        )])
        .list()
        .expect("query");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].id, id_hit);

    let none = engine
        .runtime_service()
        .create_case_instance_query()
        .variable_conditions(vec![cond(
            "amount",
            QueryVariableOperation::Equals,
            json!(999),
        )])
        .list()
        .expect("query miss");
    assert!(none.is_empty());

    // greaterThan + likeIgnoreCase AND.
    let and_hits = engine
        .runtime_service()
        .create_case_instance_query()
        .variable_conditions(vec![
            cond("amount", QueryVariableOperation::GreaterThan, json!(90)),
            cond(
                "label",
                QueryVariableOperation::LikeIgnoreCase,
                json!("alp%"),
            ),
        ])
        .list()
        .expect("and query");
    assert_eq!(and_hits.len(), 1);
    assert_eq!(and_hits[0].id, id_hit);

    // Ensure the miss case is still queryable without variable filters.
    let all = engine
        .runtime_service()
        .create_case_instance_query()
        .list()
        .expect("all");
    let ids: Vec<_> = all.iter().map(|c| c.id.as_str()).collect();
    assert!(ids.contains(&id_hit.as_str()));
    assert!(ids.contains(&id_miss.as_str()));
}

#[test]
fn variables_match_conditions_matrix_via_public_api() {
    // Re-export surface used by the REST adapter for plan-item join filtering.
    let mut map = Map::new();
    map.insert("n".into(), json!(42));
    map.insert("s".into(), json!("Flowable"));
    map.insert("b".into(), json!(false));

    assert!(flowable_cmmn_engine::variables_match_conditions(
        &map,
        &[cond("n", QueryVariableOperation::LessThanOrEquals, json!(42))]
    ));
    assert!(flowable_cmmn_engine::variables_match_conditions(
        &map,
        &[cond(
            "s",
            QueryVariableOperation::EqualsIgnoreCase,
            json!("flowable")
        )]
    ));
    assert!(flowable_cmmn_engine::variables_match_conditions(
        &map,
        &[cond("b", QueryVariableOperation::NotEquals, json!(true))]
    ));
    // Incomparable type pair → no match.
    assert!(!flowable_cmmn_engine::variables_match_conditions(
        &map,
        &[cond("n", QueryVariableOperation::GreaterThan, json!("10"))]
    ));
}
