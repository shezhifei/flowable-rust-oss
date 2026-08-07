//! P137 — CMMN case reference metadata persistence and query parity.

use flowable_cmmn_engine::{
    CmmnCase, CmmnCaseInstance, CmmnCaseInstanceStartRequest, CmmnCasePlanModel,
    CmmnDeploymentRequest, CmmnEngine, CmmnHumanTask, CmmnModel, CmmnPlanItem,
};

fn deploy_case(engine: &CmmnEngine) {
    let plan_model = CmmnCasePlanModel::new("case-plan", "Case plan")
        .with_human_task(CmmnHumanTask::new("review-task", "Review"))
        .with_plan_item(CmmnPlanItem::new("review-plan-item", "review-task"));
    let model = CmmnModel::new(vec![CmmnCase::new(
        "case-p137",
        "p137ReferenceCase",
        "P137 reference case",
        plan_model,
    )]);

    engine
        .deploy(
            CmmnDeploymentRequest::new("p137-reference")
                .with_resource("p137-reference.cmmn", model),
        )
        .expect("deployment");
}

#[test]
fn reference_metadata_round_trips_and_filters_runtime_and_history() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    deploy_case(&engine);

    let matching = engine
        .start_case_instance_by_key(
            "p137ReferenceCase",
            CmmnCaseInstanceStartRequest::new()
                .with_name("matching")
                .with_reference_id("order-137")
                .with_reference_type("event-to-cmmn-1.1-case"),
        )
        .expect("matching case");
    engine
        .start_case_instance_by_key(
            "p137ReferenceCase",
            CmmnCaseInstanceStartRequest::new()
                .with_name("other")
                .with_reference_id("order-other")
                .with_reference_type("external"),
        )
        .expect("other case");

    assert_eq!(matching.reference_id.as_deref(), Some("order-137"));
    assert_eq!(
        matching.reference_type.as_deref(),
        Some("event-to-cmmn-1.1-case")
    );

    let runtime_hit = engine
        .runtime_service()
        .create_case_instance_query()
        .reference_id("order-137")
        .reference_type("event-to-cmmn-1.1-case")
        .list()
        .expect("runtime hit");
    assert_eq!(runtime_hit.len(), 1);
    assert_eq!(runtime_hit[0].id, matching.id);

    assert!(
        engine
            .runtime_service()
            .create_case_instance_query()
            .reference_id("missing")
            .list()
            .expect("runtime miss")
            .is_empty()
    );

    let historic_hit = engine
        .history_service()
        .create_historic_case_instance_query()
        .reference_id("order-137")
        .reference_type("event-to-cmmn-1.1-case")
        .list()
        .expect("historic hit");
    assert_eq!(historic_hit.len(), 1);
    assert_eq!(historic_hit[0].case_instance_id, matching.id);
    assert_eq!(historic_hit[0].reference_id.as_deref(), Some("order-137"));

    assert!(
        engine
            .history_service()
            .create_historic_case_instance_query()
            .reference_type("missing")
            .list()
            .expect("historic miss")
            .is_empty()
    );
}

#[test]
fn stored_case_json_defaults_missing_reference_fields_and_rejects_wrong_types() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    deploy_case(&engine);
    let case = engine
        .start_case_instance_by_key("p137ReferenceCase", CmmnCaseInstanceStartRequest::new())
        .expect("case");

    let mut legacy_json = serde_json::to_value(case).expect("serialize case");
    let object = legacy_json.as_object_mut().expect("case object");
    object.remove("reference_id");
    object.remove("reference_type");
    let legacy: CmmnCaseInstance =
        serde_json::from_value(legacy_json.clone()).expect("legacy JSON");
    assert!(legacy.reference_id.is_none());
    assert!(legacy.reference_type.is_none());

    legacy_json["reference_id"] = serde_json::json!(137);
    assert!(serde_json::from_value::<CmmnCaseInstance>(legacy_json).is_err());
}
