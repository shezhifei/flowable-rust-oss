//! P132 termination-path lifecycle listener coverage.
//!
//! Java routes every cascaded terminal transition through
//! `AbstractMovePlanItemInstanceToTerminalStateOperation.java:74-143` and
//! `CmmnListenerNotificationHelper.java:103-159`.

use flowable_cmmn_engine::{
    CmmnCase, CmmnCaseInstanceStartRequest, CmmnCasePlanModel, CmmnDeploymentRequest,
    CmmnEngine, CmmnError, CmmnEventListener, CmmnHumanTask,
    CmmnHumanTaskCompletionRequest, CmmnHumanTaskState, CmmnLifecycleListenerContext,
    CmmnLifecycleListenerHandler, CmmnListenerImplementationType, CmmnLifecycleListener,
    CmmnMilestone, CmmnModel, CmmnPlanItem, CmmnPlanItemOnPart, CmmnSentry, CmmnStage,
};
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug, PartialEq, Eq)]
struct Transition {
    definition_id: String,
    definition_type: String,
    old_state: String,
    new_state: String,
}

#[derive(Clone, Default)]
struct RecordingListener {
    transitions: Arc<Mutex<Vec<Transition>>>,
}

impl RecordingListener {
    fn transitions(&self) -> Vec<Transition> {
        self.transitions.lock().expect("transitions").clone()
    }
}

impl CmmnLifecycleListenerHandler for RecordingListener {
    fn state_changed(&self, context: &CmmnLifecycleListenerContext) -> Result<(), CmmnError> {
        self.transitions
            .lock()
            .expect("transitions")
            .push(Transition {
                definition_id: context
                    .plan_item_definition_id
                    .clone()
                    .expect("plan item definition id"),
                definition_type: context
                    .plan_item_definition_type
                    .clone()
                    .expect("plan item definition type"),
                old_state: context.old_state.clone(),
                new_state: context.new_state.clone(),
            });
        Ok(())
    }
}

fn listener(handler: &str, source_state: &str) -> CmmnLifecycleListener {
    CmmnLifecycleListener {
        implementation_type: CmmnListenerImplementationType::Class,
        implementation: handler.to_string(),
        source_state: Some(source_state.to_string()),
        target_state: Some("terminated".to_string()),
        event: None,
    }
}

fn deploy_and_start(engine: &CmmnEngine, case: CmmnCase) -> String {
    let key = case.key.clone();
    engine
        .deploy(
            CmmnDeploymentRequest::new(format!("{key}-deployment"))
                .with_resource(format!("{key}.cmmn"), CmmnModel::new(vec![case])),
        )
        .expect("deployment");
    engine
        .start_case_instance_by_key(&key, CmmnCaseInstanceStartRequest::new())
        .expect("case instance")
        .id
}

fn complete_task(engine: &CmmnEngine, case_id: &str, definition_id: &str) {
    let task = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(case_id)
        .state(CmmnHumanTaskState::Active)
        .list()
        .expect("active tasks")
        .into_iter()
        .find(|task| task.task_definition_id == definition_id)
        .unwrap_or_else(|| panic!("active task '{definition_id}'"));
    engine
        .complete_human_task(&task.id, CmmnHumanTaskCompletionRequest::new())
        .expect("task completion");
}

fn completion_sentry(id: &str, source_plan_item_id: &str) -> CmmnSentry {
    CmmnSentry::new(
        id,
        CmmnPlanItemOnPart::new(
            format!("on-{source_plan_item_id}-complete"),
            source_plan_item_id,
            "complete",
        ),
    )
}

fn expected(definition_id: &str, definition_type: &str, old_state: &str) -> Vec<Transition> {
    vec![Transition {
        definition_id: definition_id.to_string(),
        definition_type: definition_type.to_string(),
        old_state: old_state.to_string(),
        new_state: "terminated".to_string(),
    }]
}

#[test]
fn direct_human_task_termination_notifies_humantask_listener() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let recorder = RecordingListener::default();
    engine.register_lifecycle_listener("DirectTaskAudit", Arc::new(recorder.clone()));
    let plan_model = CmmnCasePlanModel::new("plan-model", "Plan model")
        .with_human_task(CmmnHumanTask::new("task-source", "Source"))
        .with_human_task(CmmnHumanTask::new("task-target", "Target"))
        .with_human_task(CmmnHumanTask::new("task-keepalive", "Keep alive"))
        .with_plan_item(CmmnPlanItem::new("plan-source", "task-source"))
        .with_plan_item(
            CmmnPlanItem::new("plan-target", "task-target").with_exit_criterion("exit-target"),
        )
        .with_plan_item(CmmnPlanItem::new("plan-keepalive", "task-keepalive"))
        .with_sentry(completion_sentry("exit-target", "plan-source"));
    let case = CmmnCase::new("case-direct", "p132DirectTerminate", "Direct", plan_model)
        .with_plan_item_lifecycle_listener("task-target", listener("DirectTaskAudit", "active"));
    let case_id = deploy_and_start(&engine, case);

    complete_task(&engine, &case_id, "task-source");

    assert_eq!(
        recorder.transitions(),
        expected("task-target", "humantask", "active")
    );
}

#[test]
fn cascaded_stage_child_termination_notifies_humantask_listener() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let recorder = RecordingListener::default();
    engine.register_lifecycle_listener("ChildTaskAudit", Arc::new(recorder.clone()));
    let stage = CmmnStage::new("stage-target", "Target stage")
        .with_human_task(CmmnHumanTask::new("task-child", "Child"))
        .with_plan_item(CmmnPlanItem::new("plan-child", "task-child"));
    let plan_model = CmmnCasePlanModel::new("plan-model", "Plan model")
        .with_stage(stage)
        .with_human_task(CmmnHumanTask::new("task-source", "Source"))
        .with_human_task(CmmnHumanTask::new("task-keepalive", "Keep alive"))
        .with_plan_item(
            CmmnPlanItem::new("plan-stage", "stage-target").with_exit_criterion("exit-stage"),
        )
        .with_plan_item(CmmnPlanItem::new("plan-source", "task-source"))
        .with_plan_item(CmmnPlanItem::new("plan-keepalive", "task-keepalive"))
        .with_sentry(completion_sentry("exit-stage", "plan-source"));
    let case = CmmnCase::new("case-cascade", "p132CascadeTerminate", "Cascade", plan_model)
        .with_plan_item_lifecycle_listener("task-child", listener("ChildTaskAudit", "active"));
    let case_id = deploy_and_start(&engine, case);

    complete_task(&engine, &case_id, "task-source");

    assert_eq!(
        recorder.transitions(),
        expected("task-child", "humantask", "active")
    );
}

#[test]
fn occurred_milestone_termination_notifies_milestone_listener() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let recorder = RecordingListener::default();
    engine.register_lifecycle_listener("MilestoneAudit", Arc::new(recorder.clone()));
    let plan_model = CmmnCasePlanModel::new("plan-model", "Plan model")
        .with_human_task(CmmnHumanTask::new("task-reach", "Reach"))
        .with_human_task(CmmnHumanTask::new("task-terminate", "Terminate"))
        .with_human_task(CmmnHumanTask::new("task-keepalive", "Keep alive"))
        .with_milestone(CmmnMilestone::new("milestone-target", "Target milestone"))
        .with_plan_item(CmmnPlanItem::new("plan-reach", "task-reach"))
        .with_plan_item(CmmnPlanItem::new("plan-terminate", "task-terminate"))
        .with_plan_item(CmmnPlanItem::new("plan-keepalive", "task-keepalive"))
        .with_plan_item(
            CmmnPlanItem::new("plan-milestone", "milestone-target")
                .with_entry_criterion("entry-milestone")
                .with_exit_criterion("exit-milestone"),
        )
        .with_sentry(completion_sentry("entry-milestone", "plan-reach"))
        .with_sentry(completion_sentry("exit-milestone", "plan-terminate"));
    let case = CmmnCase::new(
        "case-milestone",
        "p132MilestoneTerminate",
        "Milestone",
        plan_model,
    )
    .with_plan_item_lifecycle_listener(
        "milestone-target",
        listener("MilestoneAudit", "completed"),
    );
    let case_id = deploy_and_start(&engine, case);

    complete_task(&engine, &case_id, "task-reach");
    complete_task(&engine, &case_id, "task-terminate");

    assert_eq!(
        recorder.transitions(),
        expected("milestone-target", "milestone", "completed")
    );
}

#[test]
fn timer_event_listener_termination_notifies_actual_definition_type() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let recorder = RecordingListener::default();
    engine.register_lifecycle_listener("EventListenerAudit", Arc::new(recorder.clone()));
    let plan_model = CmmnCasePlanModel::new("plan-model", "Plan model")
        .with_human_task(CmmnHumanTask::new("task-source", "Source"))
        .with_human_task(CmmnHumanTask::new("task-keepalive", "Keep alive"))
        .with_event_listener(
            CmmnEventListener::new("listener-target", "timer").with_timer_expression("PT1H"),
        )
        .with_plan_item(CmmnPlanItem::new("plan-source", "task-source"))
        .with_plan_item(CmmnPlanItem::new("plan-keepalive", "task-keepalive"))
        .with_plan_item(
            CmmnPlanItem::new("plan-listener", "listener-target")
                .with_exit_criterion("exit-listener"),
        )
        .with_sentry(completion_sentry("exit-listener", "plan-source"));
    let case = CmmnCase::new("case-listener", "p132ListenerTerminate", "Listener", plan_model)
        .with_plan_item_lifecycle_listener(
            "listener-target",
            listener("EventListenerAudit", "available"),
        );
    let case_id = deploy_and_start(&engine, case);

    complete_task(&engine, &case_id, "task-source");

    assert_eq!(
        recorder.transitions(),
        expected("listener-target", "timereventlistener", "available")
    );
}
