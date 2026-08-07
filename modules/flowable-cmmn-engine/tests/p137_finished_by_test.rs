//! P137 — explicit CMMN finishing actor persistence and historic filtering.

use flowable_cmmn_engine::{
    CmmnCase, CmmnCaseInstanceStartRequest, CmmnCasePlanModel, CmmnDeploymentRequest,
    CmmnEngine, CmmnHumanTask, CmmnHumanTaskCompletionRequest, CmmnHumanTaskState, CmmnModel,
    CmmnPlanItem,
};

fn setup() -> CmmnEngine {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let plan_model = CmmnCasePlanModel::new("case-plan", "Case plan")
        .with_human_task(CmmnHumanTask::new("review-task", "Review"))
        .with_plan_item(CmmnPlanItem::new("review-plan-item", "review-task"));
    let model = CmmnModel::new(vec![CmmnCase::new(
        "case-p137-finished-by",
        "p137FinishedByCase",
        "P137 finished-by case",
        plan_model,
    )]);
    engine
        .deploy(
            CmmnDeploymentRequest::new("p137-finished-by")
                .with_resource("p137-finished-by.cmmn", model),
        )
        .expect("deployment");
    engine
}

fn start_case(engine: &CmmnEngine) -> String {
    engine
        .start_case_instance_by_key(
            "p137FinishedByCase",
            CmmnCaseInstanceStartRequest::new(),
        )
        .expect("start case")
        .id
}

#[test]
fn terminate_with_actor_persists_finished_by_and_filters_exactly() {
    let engine = setup();
    let matching = start_case(&engine);
    let other = start_case(&engine);

    engine
        .runtime_service()
        .terminate_case_instance_with_actor(&matching, Some("kermit"))
        .expect("terminate with actor");
    engine
        .runtime_service()
        .terminate_case_instance_with_actor(&other, Some("fozzie"))
        .expect("terminate other actor");

    let hit = engine
        .history_service()
        .create_historic_case_instance_query()
        .finished_by("kermit")
        .list()
        .expect("finishedBy hit");
    assert_eq!(hit.len(), 1);
    assert_eq!(hit[0].case_instance_id, matching);
    assert_eq!(hit[0].finished_by.as_deref(), Some("kermit"));

    assert!(
        engine
            .history_service()
            .create_historic_case_instance_query()
            .finished_by("missing")
            .list()
            .expect("finishedBy miss")
            .is_empty()
    );
}

#[test]
fn implicit_completion_and_rest_style_termination_do_not_invent_an_actor() {
    let engine = setup();
    let completed = start_case(&engine);
    let completed_task = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&completed)
        .state(CmmnHumanTaskState::Active)
        .single_result()
        .expect("task query")
        .expect("task")
        .id;
    engine
        .runtime_service()
        .complete_human_task(
            &completed_task,
            CmmnHumanTaskCompletionRequest::new(),
        )
        .expect("complete case");

    let terminated = start_case(&engine);
    engine
        .runtime_service()
        .terminate_case_instance(&terminated)
        .expect("terminate without actor");

    for case_instance_id in [completed, terminated] {
        let historic = engine
            .history_service()
            .create_historic_case_instance_query()
            .case_instance_id(case_instance_id)
            .single_result()
            .expect("historic query")
            .expect("historic case");
        assert!(historic.finished_by.is_none());
    }
}

#[test]
fn terminate_with_actor_keeps_not_found_as_an_error() {
    let engine = setup();
    let error = engine
        .runtime_service()
        .terminate_case_instance_with_actor("missing-case", Some("kermit"))
        .expect_err("missing case must fail");
    assert!(error.to_string().contains("missing-case"));
    assert!(
        engine
            .history_service()
            .create_historic_case_instance_query()
            .finished_by("kermit")
            .list()
            .expect("history")
            .is_empty()
    );
}
