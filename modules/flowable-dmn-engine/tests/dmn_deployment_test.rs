use flowable_dmn_engine::{
    DmnDecision, DmnDeploymentRequest, DmnEngine, DmnHitPolicy, DmnInputClause, DmnModel,
    DmnOutputClause, DmnRule, DmnRuleInputEntry, DmnRuleOutputEntry, DmnUnaryTest,
};
use serde_json::json;

fn sample_model(decision_key: &str) -> DmnModel {
    DmnModel::new(vec![DmnDecision::new(
        "decision-1",
        decision_key,
        "Dish decision",
        DmnHitPolicy::First,
        vec![DmnInputClause::new("input-1", "dishType")],
        vec![DmnOutputClause::new("output-1", "dish")],
        vec![DmnRule::new(
            "rule-1",
            vec![DmnRuleInputEntry::new(DmnUnaryTest::Equals(json!("salad")))],
            vec![DmnRuleOutputEntry::new(json!("light"))],
        )],
    )])
}

#[test]
fn deploys_models_and_queries_latest_decision_metadata() {
    let engine = DmnEngine::new_in_memory().expect("engine");
    let repository = engine.repository_service();

    let deployment = repository
        .deploy(
            DmnDeploymentRequest::new("dmn-deployment")
                .with_resource("dish-decision.dmn", sample_model("dishDecision")),
        )
        .expect("deployment");

    assert_eq!(deployment.name, "dmn-deployment");
    assert_eq!(deployment.resource_names, vec!["dish-decision.dmn"]);

    let definitions = repository
        .create_decision_query()
        .key("dishDecision")
        .list()
        .expect("definitions");

    assert_eq!(definitions.len(), 1);
    assert_eq!(definitions[0].deployment_id, deployment.id);
    assert_eq!(definitions[0].version, 1);

    let by_id = repository
        .get_decision(&definitions[0].id)
        .expect("decision by id");
    assert_eq!(by_id.key, "dishDecision");
    assert_eq!(by_id.resource_name, "dish-decision.dmn");
}

#[test]
fn increments_versions_across_redeployments_of_same_decision_key() {
    let engine = DmnEngine::new_in_memory().expect("engine");
    let repository = engine.repository_service();

    repository
        .deploy(
            DmnDeploymentRequest::new("v1")
                .with_resource("dish-decision-v1.dmn", sample_model("dishDecision")),
        )
        .expect("deployment v1");
    repository
        .deploy(
            DmnDeploymentRequest::new("v2")
                .with_resource("dish-decision-v2.dmn", sample_model("dishDecision")),
        )
        .expect("deployment v2");

    let definitions = repository
        .create_decision_query()
        .key("dishDecision")
        .list()
        .expect("definitions");

    assert_eq!(definitions.len(), 2);
    assert_eq!(definitions[0].version, 2);
    assert_eq!(definitions[1].version, 1);
}

#[test]
fn deploys_and_queries_deployment_category_and_parent_deployment_id() {
    let engine = DmnEngine::new_in_memory().expect("engine");
    let repository = engine.repository_service();

    let retail_deployment = repository
        .deploy(
            DmnDeploymentRequest::new("retail-deployment")
                .with_category("retail")
                .with_parent_deployment_id("case-parent-1")
                .with_tenant_id("tenant-alpha")
                .with_resource("retail-decision.dmn", sample_model("retailDecision")),
        )
        .expect("retail deployment");
    let finance_deployment = repository
        .deploy(
            DmnDeploymentRequest::new("finance-deployment")
                .with_category("finance")
                .with_parent_deployment_id("finance-parent-1")
                .with_tenant_id("tenant-beta")
                .with_resource("finance-decision.dmn", sample_model("financeDecision")),
        )
        .expect("finance deployment");

    assert_eq!(retail_deployment.category.as_deref(), Some("retail"));
    assert_eq!(
        retail_deployment.parent_deployment_id.as_deref(),
        Some("case-parent-1")
    );

    let by_category = repository
        .create_deployment_query()
        .category("retail")
        .list()
        .expect("category query");
    assert_eq!(by_category, vec![retail_deployment.clone()]);

    let by_name_like = repository
        .create_deployment_query()
        .name_like("%deployment")
        .list()
        .expect("name like query");
    assert_eq!(
        by_name_like,
        vec![retail_deployment.clone(), finance_deployment.clone()]
    );

    let not_finance = repository
        .create_deployment_query()
        .category_not_equals("finance")
        .list()
        .expect("category not equals query");
    assert_eq!(not_finance, vec![retail_deployment.clone()]);

    let by_parent = repository
        .create_deployment_query()
        .parent_deployment_id("finance-parent-1")
        .single_result()
        .expect("parent deployment query")
        .expect("parent deployment result");
    assert_eq!(by_parent, finance_deployment);

    let by_parent_like = repository
        .create_deployment_query()
        .parent_deployment_id_like("%case-parent-1")
        .list()
        .expect("parent deployment like query");
    assert_eq!(by_parent_like, vec![retail_deployment.clone()]);

    let by_tenant_like = repository
        .create_deployment_query()
        .tenant_id_like("tenant-%")
        .list()
        .expect("tenant like query");
    assert_eq!(
        by_tenant_like,
        vec![retail_deployment.clone(), finance_deployment.clone()]
    );
}
