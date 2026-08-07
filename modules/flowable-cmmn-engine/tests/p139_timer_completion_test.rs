// P139: AVAILABLE timerEventListener must block non-autocomplete case/stage completion.
//
// Java references:
// - PlanItemInstanceContainerUtil.java:73-146 (shouldPlanItemContainerComplete):
//   END_STATES skip; ACTIVE_STATES block; AVAILABLE/ENABLED only block when
//   container is not autocomplete (:143-146).
// - ExpressionUtil.java:260-264 (evaluateAutoComplete): stage.isAutoComplete(),
//   Java Stage.autoComplete is a native boolean defaulting to false.
// - TimerEventListenerTest.java:56-62 (testTimerExpressionDuration): after start,
//   case count stays 1 and the timer listener is AVAILABLE. Model
//   TimerEventListenerTest.testTimerExpressionDuration.cmmn leaves casePlanModel
//   autoComplete unset (default false).
// - TimerEventListenerActivityBehaviour.java:66-78 / :172-212: timer jobs are
//   scheduled on activate; no ACT_CMMN_EVENT_SUBSCRIPTION row is written.
use flowable_cmmn_engine::{
    CmmnCase, CmmnCaseInstanceStartRequest, CmmnCaseInstanceState, CmmnCasePlanModel,
    CmmnDeploymentRequest, CmmnEngine, CmmnEventListener, CmmnJob, CmmnJobFamily, CmmnModel,
    CmmnPlanItem, CmmnStage, TYPE_TRIGGER_TIMER,
};

/// Lone timerEventListener on the case plan model (no human tasks / sentries).
fn lone_timer_case_model(case_key: &str, auto_complete: Option<bool>) -> CmmnModel {
    let mut plan_model = CmmnCasePlanModel::new("case-plan-model", "Case plan model")
        .with_event_listener(
            CmmnEventListener::new("timer-listener", CmmnEventListener::EVENT_TYPE_TIMER)
                .with_name("Timer listener")
                .with_timer_expression("PT1H"),
        )
        .with_plan_item(CmmnPlanItem::new("plan-item-timer", "timer-listener"));
    if let Some(flag) = auto_complete {
        plan_model = plan_model.with_auto_complete(flag);
    }
    CmmnModel::new(vec![CmmnCase::new(
        format!("case-{case_key}"),
        case_key,
        "P139 lone timer completion case",
        plan_model,
    )])
}

/// TimerEventListener nested under a single stage; stage has no other open work.
fn stage_nested_timer_case_model(case_key: &str) -> CmmnModel {
    let stage = CmmnStage::new("stage-timer", "Timer stage")
        .with_event_listener(
            CmmnEventListener::new("timer-listener", CmmnEventListener::EVENT_TYPE_TIMER)
                .with_name("Stage timer listener")
                .with_timer_expression("PT1H"),
        )
        .with_plan_item(CmmnPlanItem::new("plan-item-timer", "timer-listener"));
    let plan_model = CmmnCasePlanModel::new("case-plan-model", "Case plan model")
        .with_stage(stage)
        .with_plan_item(CmmnPlanItem::new("plan-item-stage", "stage-timer"));
    CmmnModel::new(vec![CmmnCase::new(
        format!("case-{case_key}"),
        case_key,
        "P139 stage-nested timer completion case",
        plan_model,
    )])
}

fn deploy_and_start(engine: &CmmnEngine, case_key: &str, model: CmmnModel) -> String {
    engine
        .deploy(
            CmmnDeploymentRequest::new(format!("{case_key}-deployment"))
                .with_resource(format!("{case_key}.cmmn"), model),
        )
        .expect("deployment");
    engine
        .start_case_instance_by_key(case_key, CmmnCaseInstanceStartRequest::new())
        .expect("case instance")
        .id
}

fn case_state(engine: &CmmnEngine, case_instance_id: &str) -> CmmnCaseInstanceState {
    engine
        .runtime_service()
        .get_case_instance(case_instance_id)
        .expect("case instance")
        .state
}

fn timer_jobs_for_case(engine: &CmmnEngine, case_id: &str) -> Vec<CmmnJob> {
    engine
        .management_service()
        .create_job_query()
        .family(CmmnJobFamily::Timer)
        .list()
        .expect("timer jobs")
        .into_iter()
        .filter(|job| job.scope_id.as_deref() == Some(case_id))
        .collect()
}

// Java TimerEventListenerTest.testTimerExpressionDuration + PlanItemInstanceContainerUtil
// :143-146: autoComplete defaults false → AVAILABLE timer keeps the case Active.
#[test]
fn lone_timer_without_auto_complete_keeps_case_active_with_timer_job() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let case_id = deploy_and_start(
        &engine,
        "p139LoneTimerDefault",
        lone_timer_case_model("p139LoneTimerDefault", None),
    );

    assert_eq!(
        case_state(&engine, &case_id),
        CmmnCaseInstanceState::Active,
        "AVAILABLE timerEventListener must block non-autocomplete case completion"
    );

    let jobs = timer_jobs_for_case(&engine, &case_id);
    assert_eq!(jobs.len(), 1, "timer job must remain scheduled");
    assert_eq!(jobs[0].handler_type.as_deref(), Some(TYPE_TRIGGER_TIMER));
    assert_eq!(jobs[0].element_id.as_deref(), Some("timer-listener"));
}

// Counterpart: autoComplete=true ignores AVAILABLE/ENABLED children
// (PlanItemInstanceContainerUtil.java:143-146) → case completes immediately.
#[test]
fn lone_timer_with_auto_complete_completes_case() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let case_id = deploy_and_start(
        &engine,
        "p139LoneTimerAutoComplete",
        lone_timer_case_model("p139LoneTimerAutoComplete", Some(true)),
    );

    assert_eq!(
        case_state(&engine, &case_id),
        CmmnCaseInstanceState::Completed,
        "autoComplete=true must still complete with only an AVAILABLE timer listener"
    );
}

// Stage branch: AVAILABLE timer nested under a stage (stage autoComplete default false)
// must keep both the stage and the case Active.
#[test]
fn stage_nested_timer_without_auto_complete_keeps_case_and_stage_active() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let case_id = deploy_and_start(
        &engine,
        "p139StageNestedTimer",
        stage_nested_timer_case_model("p139StageNestedTimer"),
    );

    assert_eq!(
        case_state(&engine, &case_id),
        CmmnCaseInstanceState::Active,
        "stage-nested AVAILABLE timer must keep the case Active"
    );

    let overview = engine
        .runtime_service()
        .get_stage_overview(&case_id)
        .expect("stage overview");
    assert_eq!(overview.len(), 1, "one stage instance expected");
    assert_eq!(overview[0].id, "stage-timer");
    assert!(
        overview[0].current && !overview[0].ended,
        "stage must stay Active while its timer listener is AVAILABLE, got current={} ended={}",
        overview[0].current,
        overview[0].ended
    );

    let jobs = timer_jobs_for_case(&engine, &case_id);
    assert_eq!(jobs.len(), 1, "stage-nested timer job must remain scheduled");
}
