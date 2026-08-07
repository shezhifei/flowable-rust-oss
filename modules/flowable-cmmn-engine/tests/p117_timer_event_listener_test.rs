// P117: CMMN timerEventListener full chain e2e — parse → schedule → trigger → cleanup.
//
// Java references:
// - TimerEventListenerActivityBehaviour.java:66-78 (CREATE/INITIATE schedules the timer
//   job; DISMISS/TERMINATE/EXIT removes it)
// - TimerEventListenerActivityBehaviour.java:96-152 (timer expression → due date;
//   duration / date / R-cycle)
// - TimerEventListenerActivityBehaviour.java:172-212 (timer job: jobType=timer,
//   handlerType=cmmn-trigger-timer, exclusive, retries, duedate, scope/subScope,
//   elementId/Name, repeat)
// - TriggerTimerEventJobHandler.java:27-38 (fires the plan item occur)
// - OccurPlanItemInstanceOperation.java:34-61 (event listener occur → COMPLETED)
// - DefaultJobManager.java:506-536 + TimerJobSchedulerImpl.java:40-52 (repeat reschedule)
// - PlanItemInstanceEntityManagerImpl.java:531-538 (timer jobs removed when the plan item
//   instance is deleted / the case ends)
use flowable_cmmn_engine::{
    CmmnCase, CmmnCaseInstanceStartRequest, CmmnCasePlanModel, CmmnDeploymentRequest, CmmnEngine,
    CmmnEventListener, CmmnHumanTask, CmmnHumanTaskState, CmmnJob, CmmnJobFamily, CmmnModel,
    CmmnPlanItem, CmmnPlanItemOnPart, CmmnSentry, TYPE_TRIGGER_TIMER,
};
use serde_json::json;

/// Case plan model: a timer event listener occurs → sentry satisfies → human task "A"
/// activates. A keepalive task keeps the case open (and blocks completion) until the
/// timer fires.
fn timer_case_model(case_key: &str, timer_expression: &str) -> CmmnModel {
    let plan_model = CmmnCasePlanModel::new("case-plan-model", "Case plan model")
        .with_event_listener(
            CmmnEventListener::new("timer-listener", CmmnEventListener::EVENT_TYPE_TIMER)
                .with_name("Timer listener")
                .with_timer_expression(timer_expression),
        )
        .with_plan_item(CmmnPlanItem::new("plan-item-timer", "timer-listener"))
        .with_human_task(CmmnHumanTask::new("task-a", "A"))
        .with_plan_item(
            CmmnPlanItem::new("plan-item-a", "task-a").with_entry_criterion("sentry-after-timer"),
        )
        .with_human_task(CmmnHumanTask::new("task-keepalive", "Keep alive"))
        .with_plan_item(CmmnPlanItem::new("plan-item-keepalive", "task-keepalive"))
        .with_sentry(CmmnSentry::new(
            "sentry-after-timer",
            CmmnPlanItemOnPart::new(
                "on-timer-occur",
                "plan-item-timer",
                CmmnPlanItemOnPart::STANDARD_EVENT_OCCUR,
            ),
        ));
    CmmnModel::new(vec![CmmnCase::new(
        "case-p117",
        case_key,
        "P117 timer event listener case",
        plan_model,
    )])
}

fn deploy_and_start(engine: &CmmnEngine, deployment_key: &str, case_key: &str, expression: &str) -> String {
    engine
        .deploy(
            CmmnDeploymentRequest::new(deployment_key)
                .with_resource(format!("{deployment_key}.cmmn"), timer_case_model(case_key, expression)),
        )
        .expect("deployment");
    engine
        .start_case_instance_by_key(case_key, CmmnCaseInstanceStartRequest::new())
        .expect("case instance")
        .id
}

fn timer_jobs(engine: &CmmnEngine, case_id: Option<&str>) -> Vec<CmmnJob> {
    let query = engine.management_service().create_job_query().family(CmmnJobFamily::Timer);
    if let Some(case_id) = case_id {
        // CmmnManagementJobQuery has no scope filter; filter in memory.
        let jobs = query.list().expect("timer jobs");
        return jobs
            .into_iter()
            .filter(|job| job.scope_id.as_deref() == Some(case_id))
            .collect();
    }
    query.list().expect("timer jobs")
}

fn active_tasks(engine: &CmmnEngine, case_id: &str) -> Vec<String> {
    engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(case_id)
        .state(CmmnHumanTaskState::Active)
        .list()
        .expect("task query")
        .into_iter()
        .map(|task| task.name.clone())
        .collect()
}

#[test]
fn duration_timer_creates_job_and_fires_task() {
    // Java TimerEventListenerTest.testTimerExpressionDuration: PT1H duration → one timer
    // job (handler cmmn-trigger-timer, elementId = listener id), firing activates task A.
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let case_id = deploy_and_start(&engine, "p117-duration", "p117Duration", "PT1H");

    let jobs = timer_jobs(&engine, Some(&case_id));
    assert_eq!(jobs.len(), 1, "one timer job expected");
    let job = &jobs[0];
    assert_eq!(job.handler_type.as_deref(), Some(TYPE_TRIGGER_TIMER));
    assert_eq!(job.element_id.as_deref(), Some("timer-listener"));
    assert_eq!(job.scope_id.as_deref(), Some(case_id.as_str()));
    assert_eq!(job.sub_scope_id.as_deref(), Some("plan-item-timer"));

    // Due date ≈ now + PT1H (Java DueDateBusinessCalendar.java:31-52).
    let due = job.due_date.expect("due date");
    let expected = chrono::Utc::now() + chrono::Duration::hours(1);
    assert!(
        (due - expected).num_seconds().abs() < 10,
        "due {due} should be ~1h from now"
    );

    // Task A not yet active before the timer fires.
    assert_eq!(active_tasks(&engine, &case_id), vec!["Keep alive"]);

    // Fire the timer job (Java moveTimerToExecutableJob + executeJob).
    engine.execute_job(&job.id).expect("execute timer job");

    let tasks = active_tasks(&engine, &case_id);
    assert!(tasks.contains(&"A".to_string()), "task A should be active, got {tasks:?}");

    // The fired non-repeating job is deleted.
    assert!(timer_jobs(&engine, Some(&case_id)).is_empty());
}

#[test]
fn date_timer_creates_job_at_absolute_date() {
    // Java TimerEventListenerTest.testDateExpression: an absolute ISO date expression
    // schedules the timer at that instant (DateUtil.parseDate fallback).
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let target = chrono::Utc::now() + chrono::Duration::hours(5);
    let expression = target.to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let case_id = deploy_and_start(&engine, "p117-date", "p117Date", &expression);

    let jobs = timer_jobs(&engine, Some(&case_id));
    assert_eq!(jobs.len(), 1);
    let due = jobs[0].due_date.expect("due date");
    assert!((due - target).num_seconds().abs() < 10, "due {due} should be ~{target}");

    engine.execute_job(&jobs[0].id).expect("execute timer job");
    let tasks = active_tasks(&engine, &case_id);
    assert!(tasks.contains(&"A".to_string()), "task A should be active, got {tasks:?}");
}

#[test]
fn repeating_timer_reschedules_after_fire() {
    // Java TimerEventListenerTest.testRepeatingTimer: R/PT20S repeats forever; firing the
    // job reschedules the next cycle (TimerJobSchedulerImpl.java:40-52).
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let case_id = deploy_and_start(&engine, "p117-repeat", "p117Repeat", "R/PT20S");

    let jobs = timer_jobs(&engine, Some(&case_id));
    assert_eq!(jobs.len(), 1);
    let job = &jobs[0];
    // The prepared repeat expression carries an injected start anchor
    // (R/<start>/PT20S, TimerEventListenerActivityBehaviour.java:237-242).
    let config: serde_json::Value = serde_json::from_str(job.configuration.as_deref().unwrap_or("{}")).expect("config");
    let repeat = config.get("repeat").and_then(|v| v.as_str()).expect("repeat config");
    assert!(repeat.starts_with("R/"), "prepared repeat {repeat}");

    // Advance the clock is not supported by the CMMN engine; firing manually still
    // reschedules the next cycle job.
    engine.execute_job(&job.id).expect("execute timer job");
    let tasks = active_tasks(&engine, &case_id);
    assert!(tasks.contains(&"A".to_string()), "task A should be active, got {tasks:?}");

    let next_jobs = timer_jobs(&engine, Some(&case_id));
    assert_eq!(next_jobs.len(), 1, "repeat reschedules the next cycle");
    assert_ne!(next_jobs[0].id, job.id);
    // The CMMN engine has no settable clock, so firing immediately after scheduling
    // lands on the same 20s boundary (Java tests advance the clock between fires).
    assert!(
        (next_jobs[0].due_date.unwrap() - job.due_date.unwrap()).num_seconds().abs() < 5,
        "next due should be ~the 20s boundary"
    );
}

#[test]
fn limited_repeating_timer_exhausts() {
    // Java TimerEventListenerTest.testLimitedRepeatingTimerWithAvailableCondition: R4/… is
    // exhausted after 4 fires; the last fire produces no next timer job.
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let case_id = deploy_and_start(&engine, "p117-limited", "p117Limited", "R4/PT20S");

    for fire in 0..4 {
        let jobs = timer_jobs(&engine, Some(&case_id));
        assert_eq!(jobs.len(), 1, "a job should be pending before fire {fire}");
        engine.execute_job(&jobs[0].id).expect("execute timer job");
    }
    assert!(
        timer_jobs(&engine, Some(&case_id)).is_empty(),
        "R4/PT20S is exhausted after 4 fires"
    );
    assert!(active_tasks(&engine, &case_id).contains(&"A".to_string()));
}

#[test]
fn available_condition_gates_timer_job_creation() {
    // Java TimerEventListenerTest.testTimerWithAvailableCondition: the timer job is only
    // created once flowable:availableCondition evaluates true; false dismisses it again
    // (TimerEventListenerActivityBehaviour.java:66-78).
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let plan_model = CmmnCasePlanModel::new("case-plan-model", "Case plan model")
        .with_event_listener(
            CmmnEventListener::new("timer-listener", CmmnEventListener::EVENT_TYPE_TIMER)
                .with_available_condition("timerVar == true")
                .with_timer_expression("PT1H"),
        )
        .with_plan_item(CmmnPlanItem::new("plan-item-timer", "timer-listener"))
        .with_human_task(CmmnHumanTask::new("task-keepalive", "Keep alive"))
        .with_plan_item(CmmnPlanItem::new("plan-item-keepalive", "task-keepalive"));
    let model = CmmnModel::new(vec![CmmnCase::new(
        "case-p117",
        "p117Available",
        "gated timer listener",
        plan_model,
    )]);
    engine
        .deploy(
            CmmnDeploymentRequest::new("p117-available-deploy")
                .with_resource("case.cmmn", model),
        )
        .expect("deployment");

    let case_id = engine
        .start_case_instance_by_key(
            "p117Available",
            CmmnCaseInstanceStartRequest::new().with_variables(json!({ "timerVar": false })),
        )
        .expect("case instance")
        .id;
    assert!(timer_jobs(&engine, Some(&case_id)).is_empty(), "gated listener stays unavailable");

    engine
        .runtime_service()
        .set_case_instance_variables(&case_id, vec![("timerVar".to_string(), json!(true))])
        .expect("set variable");
    assert_eq!(timer_jobs(&engine, Some(&case_id)).len(), 1, "timer job appears when available");

    engine
        .runtime_service()
        .set_case_instance_variables(&case_id, vec![("timerVar".to_string(), json!(false))])
        .expect("set variable");
    assert!(timer_jobs(&engine, Some(&case_id)).is_empty(), "dismissed listener drops its job");
}

#[test]
fn terminating_case_deletes_timer_job() {
    // Java TimerEventListenerActivityBehaviour.java:72-77 (DISMISS/TERMINATE removes the
    // timer job) + PlanItemInstanceEntityManagerImpl.java:531-538 (job cleanup on delete).
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let case_id = deploy_and_start(&engine, "p117-cleanup", "p117Cleanup", "PT1H");
    assert_eq!(timer_jobs(&engine, Some(&case_id)).len(), 1);

    engine.runtime_service().terminate_case_instance(&case_id).expect("terminate case");
    assert!(
        timer_jobs(&engine, Some(&case_id)).is_empty(),
        "case termination removes the timer job"
    );
}

#[test]
fn run_due_timer_jobs_fires_due_jobs() {
    // Java job executor acquisition: a timer whose due date has passed is picked up and
    // fired by the due-timer scan (DefaultJobManager.executeTimerJob).
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let case_id = deploy_and_start(&engine, "p117-due", "p117Due", "PT1H");
    let jobs = timer_jobs(&engine, Some(&case_id));
    assert_eq!(jobs.len(), 1);

    // Pull the due date into the past, then let the scan fire it.
    let mut job = jobs[0].clone();
    job.due_date = Some(chrono::Utc::now() - chrono::Duration::seconds(1));
    engine.management_service().update_job(&job).expect("backdate job");

    let triggered = engine.run_due_timer_jobs().expect("run due timer jobs");
    assert_eq!(triggered, vec![job.id]);
    assert!(active_tasks(&engine, &case_id).contains(&"A".to_string()));
}

#[test]
fn xml_deployed_timer_event_listener_works() {
    // Converter contract: the engine's XML deployment path parses timerEventListener +
    // timerExpression (TimerEventListenerXmlConverter.java:36-44,
    // TimerExpressionXmlConverter.java:39-49) and the full chain works.
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/CMMN/20151109/MODEL"
             xmlns:flowable="http://flowable.org/cmmn" targetNamespace="http://flowable.org/casedef">
  <case id="p117XmlCase" name="p117XmlCase">
    <casePlanModel id="casePlanModel">
      <planItem id="planItem1" definitionRef="timerListener"/>
      <planItem id="planItem2" name="A" definitionRef="taskA">
        <entryCriterion id="criterion1" sentryRef="sentry1"/>
      </planItem>
      <sentry id="sentry1">
        <planItemOnPart id="onPart1" sourceRef="planItem1">
          <standardEvent>occur</standardEvent>
        </planItemOnPart>
      </sentry>
      <timerEventListener id="timerListener" name="Timer listener">
        <timerExpression><![CDATA[PT1H]]></timerExpression>
      </timerEventListener>
      <humanTask id="taskA" name="A"/>
    </casePlanModel>
  </case>
</definitions>"#;
    engine
        .repository_service()
        .new_deployment()
        .name("p117-xml-deploy")
        .add_string("case.cmmn", xml)
        .expect("add string")
        .deploy()
        .expect("deployment");

    let case_id = engine
        .start_case_instance_by_key("p117XmlCase", CmmnCaseInstanceStartRequest::new())
        .expect("case instance")
        .id;

    let jobs = timer_jobs(&engine, Some(&case_id));
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].handler_type.as_deref(), Some(TYPE_TRIGGER_TIMER));
    assert_eq!(jobs[0].element_id.as_deref(), Some("timerListener"));
    assert_eq!(jobs[0].element_id.as_deref(), Some("timerListener"));

    // P139: AVAILABLE timerEventListener blocks non-autocomplete case completion
    // (PlanItemInstanceContainerUtil.java:143-146; Java TimerEventListenerTest
    // .testTimerExpressionDuration). The mirror stays AVAILABLE until the timer fires
    // (or the case is terminated). Pre-P139 the case completed immediately and P132
    // left a TERMINATED historic mirror row.
    let mirrors = engine
        .runtime_service()
        .create_plan_item_instance_query()
        .case_instance_id(&case_id)
        .plan_item_definition_type("timereventlistener")
        .list()
        .expect("plan item mirrors");
    assert_eq!(mirrors.len(), 1);
    assert_eq!(mirrors[0].state, "AVAILABLE");
    assert!(mirrors[0].ended_at.is_none());
    assert_eq!(mirrors[0].plan_item_id, "planItem1");
    assert_eq!(
        engine
            .runtime_service()
            .get_case_instance(&case_id)
            .expect("case")
            .state,
        flowable_cmmn_engine::CmmnCaseInstanceState::Active
    );

    engine.execute_job(&jobs[0].id).expect("execute timer job");
    assert!(active_tasks(&engine, &case_id).contains(&"A".to_string()));
}
