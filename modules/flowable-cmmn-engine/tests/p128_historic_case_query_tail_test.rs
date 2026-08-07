//! P128 — the CMMN historic-case query tail left after P120.
//!
//! Java references:
//! - `HistoricCaseInstanceQueryImpl.java:699-755,1011-1045`
//! - `HistoricCaseInstance.xml:767-818`

use flowable_cmmn_engine::{
    CmmnCase, CmmnCaseInstanceStartRequest, CmmnCasePlanModel, CmmnDeploymentRequest, CmmnEngine,
    CmmnHumanTask, CmmnHumanTaskCompletionRequest, CmmnHumanTaskState, CmmnIdentityLink, CmmnModel,
    CmmnPlanItem,
};

fn deploy(engine: &CmmnEngine) {
    let plan_model = CmmnCasePlanModel::new("p128-plan", "P128 plan")
        .with_human_task(CmmnHumanTask::new("review-task", "Review"))
        .with_plan_item(CmmnPlanItem::new("review-plan-item", "review-task"));
    let model = CmmnModel::new(vec![CmmnCase::new(
        "p128-case",
        "p128Case",
        "P128 case",
        plan_model,
    )]);
    engine
        .deploy(CmmnDeploymentRequest::new("p128").with_resource("p128.cmmn", model))
        .expect("deployment");
}

fn active_task_id(engine: &CmmnEngine, case_instance_id: &str) -> String {
    engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(case_instance_id)
        .state(CmmnHumanTaskState::Active)
        .single_result()
        .expect("task query")
        .expect("active task")
        .id
}

#[test]
fn historic_case_query_filters_callback_metadata_and_missing_callback() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    deploy(&engine);

    let callback_case = engine
        .start_case_instance_by_key(
            "p128Case",
            CmmnCaseInstanceStartRequest::new()
                .with_name("callback")
                .with_callback("execution-128", "bpmn-2.0-to-cmmn-1.1-child-case"),
        )
        .expect("callback case");
    let plain_case = engine
        .start_case_instance_by_key(
            "p128Case",
            CmmnCaseInstanceStartRequest::new().with_name("plain"),
        )
        .expect("plain case");

    let history = engine.history_service();
    let by_id = history
        .create_historic_case_instance_query()
        .callback_id("execution-128")
        .list()
        .expect("callbackId");
    assert_eq!(by_id.len(), 1);
    assert_eq!(by_id[0].case_instance_id, callback_case.id);
    assert_eq!(by_id[0].callback_id.as_deref(), Some("execution-128"));

    assert_eq!(
        history
            .create_historic_case_instance_query()
            .callback_ids(vec!["missing".to_string(), "execution-128".to_string()])
            .list()
            .expect("callbackIds")
            .len(),
        1
    );
    assert_eq!(
        history
            .create_historic_case_instance_query()
            .callback_type("bpmn-2.0-to-cmmn-1.1-child-case")
            .list()
            .expect("callbackType")
            .len(),
        1
    );

    let without_callback = history
        .create_historic_case_instance_query()
        .without_callback_id()
        .list()
        .expect("without callback");
    assert_eq!(without_callback.len(), 1);
    assert_eq!(without_callback[0].case_instance_id, plain_case.id);
}

#[test]
fn historic_case_query_filters_involved_user_and_active_plan_item_definition() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    deploy(&engine);

    let active_case = engine
        .start_case_instance_by_key("p128Case", CmmnCaseInstanceStartRequest::new())
        .expect("active case");
    let completed_case = engine
        .start_case_instance_by_key("p128Case", CmmnCaseInstanceStartRequest::new())
        .expect("completed case");
    let completed_task = active_task_id(&engine, &completed_case.id);
    engine
        .complete_human_task(&completed_task, CmmnHumanTaskCompletionRequest::new())
        .expect("complete task");

    engine
        .identity_link_service()
        .add_identity_link(CmmnIdentityLink {
            id: "p128-participant".to_string(),
            scope_type: "caseInstance".to_string(),
            scope_id: completed_case.id.clone(),
            link_type: "participant".to_string(),
            user_id: Some("kermit".to_string()),
            group_id: None,
        })
        .expect("case participant link");

    let history = engine.history_service();
    let involved = history
        .create_historic_case_instance_query()
        .involved_user("kermit")
        .list()
        .expect("involvedUser");
    assert_eq!(involved.len(), 1);
    assert_eq!(involved[0].case_instance_id, completed_case.id);

    let active_plan_item = history
        .create_historic_case_instance_query()
        .active_plan_item_definition_id("review-task")
        .list()
        .expect("activePlanItemDefinitionId");
    assert_eq!(active_plan_item.len(), 1);
    assert_eq!(active_plan_item[0].case_instance_id, active_case.id);

    assert!(
        history
            .create_historic_case_instance_query()
            .active_plan_item_definition_id("missing-definition")
            .list()
            .expect("missing active definition")
            .is_empty()
    );
}
