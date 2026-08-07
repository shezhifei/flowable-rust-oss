use flowable_cmmn_engine::{
    CmmnCase, CmmnCaseInstanceStartRequest, CmmnCaseInstanceState, CmmnCasePlanModel, CmmnCaseTask,
    CmmnChangePlanItemStateRequest, CmmnDecisionTask, CmmnDeploymentRequest, CmmnEngine, CmmnError,
    CmmnEventListener, CmmnHumanTask, CmmnHumanTaskCompletionRequest, CmmnHumanTaskState,
    CmmnMilestone, CmmnModel, CmmnPlanItem, CmmnPlanItemDefinitionWithTargetIds,
    CmmnPlanItemOnPart, CmmnPlanningTable, CmmnProcessTask, CmmnProcessTaskRunner,
    CmmnProcessTaskStartRequest, CmmnProcessTaskStartResult, CmmnSentry, CmmnStage,
    CmmnTaskAssociationState,
};
use serde_json::json;
use std::sync::{Arc, Mutex};

fn runtime_model() -> CmmnModel {
    let review_stage = CmmnStage::new("stage-review", "Review stage")
        .with_human_task(CmmnHumanTask::new("human-task-approve", "Approve"))
        .with_plan_item(CmmnPlanItem::new("plan-item-approve", "human-task-approve"));

    let plan_model = CmmnCasePlanModel::new("case-plan-model", "Case plan model")
        .with_human_task(CmmnHumanTask::new("human-task-intake", "Intake"))
        .with_plan_item(CmmnPlanItem::new("plan-item-intake", "human-task-intake"))
        .with_stage(review_stage)
        .with_plan_item(CmmnPlanItem::new("plan-item-review-stage", "stage-review"));

    CmmnModel::new(vec![CmmnCase::new(
        "case-review",
        "reviewCase",
        "Review case",
        plan_model,
    )])
}

#[test]
fn starts_case_instance_activates_top_level_and_stage_tasks_and_completes_case() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    engine
        .deploy(
            CmmnDeploymentRequest::new("runtime")
                .with_resource("review-case.cmmn", runtime_model()),
        )
        .expect("deployment");

    let case_instance = engine
        .start_case_instance_by_key(
            "reviewCase",
            CmmnCaseInstanceStartRequest::new()
                .with_business_key("BK-1")
                .with_started_by("starter")
                .with_variables(json!({ "priority": "high" })),
        )
        .expect("case instance");

    assert_eq!(case_instance.case_definition_key, "reviewCase");
    assert_eq!(case_instance.state, CmmnCaseInstanceState::Active);
    assert_eq!(case_instance.variables["priority"], json!("high"));

    let active_tasks = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_instance.id)
        .state(CmmnHumanTaskState::Active)
        .list()
        .expect("active tasks");

    assert_eq!(active_tasks.len(), 2);
    assert!(active_tasks.iter().any(|task| task.name == "Intake"));
    assert!(active_tasks.iter().any(|task| task.name == "Approve"));

    let intake_task = active_tasks
        .iter()
        .find(|task| task.name == "Intake")
        .expect("intake task");
    let review_task = active_tasks
        .iter()
        .find(|task| task.name == "Approve")
        .expect("review task");

    let intake_completion = engine
        .complete_human_task(&intake_task.id, CmmnHumanTaskCompletionRequest::new())
        .expect("intake completion");
    assert_eq!(
        intake_completion.case_instance.state,
        CmmnCaseInstanceState::Active
    );

    let review_completion = engine
        .complete_human_task(
            &review_task.id,
            CmmnHumanTaskCompletionRequest::new().with_completed_by("reviewer"),
        )
        .expect("review completion");
    assert_eq!(
        review_completion.case_instance.state,
        CmmnCaseInstanceState::Completed
    );
    assert!(review_completion.case_instance.ended_at.is_some());
}

#[derive(Default)]
struct RecordingProcessTaskRunner {
    requests: Mutex<Vec<CmmnProcessTaskStartRequest>>,
}

impl CmmnProcessTaskRunner for RecordingProcessTaskRunner {
    fn start_process(
        &self,
        request: CmmnProcessTaskStartRequest,
    ) -> Result<CmmnProcessTaskStartResult, CmmnError> {
        self.requests.lock().expect("requests").push(request);
        Ok(CmmnProcessTaskStartResult {
            process_instance_id: "process-instance-1".to_string(),
            completed: false,
        })
    }
}

#[test]
fn blocking_process_task_records_association_and_completes_parent_plan_item_on_callback() {
    let runner = Arc::new(RecordingProcessTaskRunner::default());
    let engine =
        CmmnEngine::new_in_memory_with_process_task_runner(runner.clone()).expect("engine");

    let sentry = CmmnSentry::new(
        "sentry-after-process",
        CmmnPlanItemOnPart::new("on-process-complete", "plan-item-process", "complete"),
    );
    let model = CmmnModel::new(vec![CmmnCase::new(
        "case-process-parent",
        "processParentCase",
        "Process parent case",
        CmmnCasePlanModel::new("case-plan-model", "Case plan model")
            .with_process_task(
                CmmnProcessTask::new("process-task-approval", "Approval process")
                    .with_process_ref("approvalProcess"),
            )
            .with_plan_item(CmmnPlanItem::new(
                "plan-item-process",
                "process-task-approval",
            ))
            .with_human_task(CmmnHumanTask::new("human-task-archive", "Archive"))
            .with_plan_item(
                CmmnPlanItem::new("plan-item-archive", "human-task-archive")
                    .with_entry_criterion("sentry-after-process"),
            )
            .with_sentry(sentry),
    )]);

    engine
        .deploy(CmmnDeploymentRequest::new("process-task").with_resource("case.cmmn", model))
        .expect("deployment");
    let case_instance = engine
        .start_case_instance_by_key(
            "processParentCase",
            CmmnCaseInstanceStartRequest::new()
                .with_business_key("BK-P1")
                .with_variables(json!({ "amount": 42 })),
        )
        .expect("case instance");

    let requests = runner.requests.lock().expect("requests");
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].process_definition_key, "approvalProcess");
    assert_eq!(requests[0].parent_case_instance_id, case_instance.id);
    assert_eq!(requests[0].parent_plan_item_id, "plan-item-process");
    assert_eq!(requests[0].business_key.as_deref(), Some("BK-P1"));
    assert_eq!(requests[0].variables["amount"], json!(42));
    drop(requests);

    let associations = engine
        .runtime_service()
        .create_task_association_query()
        .case_instance_id(&case_instance.id)
        .list()
        .expect("associations");
    assert_eq!(associations.len(), 1);
    assert_eq!(associations[0].state, CmmnTaskAssociationState::Active);
    assert_eq!(
        associations[0].child_instance_id.as_deref(),
        Some("process-instance-1")
    );

    let active_tasks = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_instance.id)
        .state(CmmnHumanTaskState::Active)
        .list()
        .expect("active tasks");
    assert!(active_tasks.is_empty());

    engine
        .runtime_service()
        .complete_process_task_child_instance("process-instance-1")
        .expect("process task completion");

    let archive_tasks = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_instance.id)
        .state(CmmnHumanTaskState::Active)
        .list()
        .expect("archive task");
    assert_eq!(archive_tasks.len(), 1);
    assert_eq!(archive_tasks[0].name, "Archive");
}

#[test]
fn failing_process_task_records_association_failure_triggers_terminate_sentry_and_preserves_case() {
    let runner = Arc::new(RecordingProcessTaskRunner::default());
    let engine =
        CmmnEngine::new_in_memory_with_process_task_runner(runner.clone()).expect("engine");

    let sentry = CmmnSentry::new(
        "sentry-after-process-failure",
        CmmnPlanItemOnPart::new("on-process-failure", "plan-item-process", "terminate"),
    );
    let model = CmmnModel::new(vec![CmmnCase::new(
        "case-process-failure-parent",
        "processFailureParentCase",
        "Process failure parent case",
        CmmnCasePlanModel::new("case-plan-model", "Case plan model")
            .with_process_task(
                CmmnProcessTask::new("process-task-approval", "Approval process")
                    .with_process_ref("approvalProcess"),
            )
            .with_plan_item(CmmnPlanItem::new(
                "plan-item-process",
                "process-task-approval",
            ))
            .with_human_task(CmmnHumanTask::new("human-task-recovery", "Recovery"))
            .with_plan_item(
                CmmnPlanItem::new("plan-item-recovery", "human-task-recovery")
                    .with_entry_criterion("sentry-after-process-failure"),
            )
            .with_sentry(sentry),
    )]);

    engine
        .deploy(
            CmmnDeploymentRequest::new("process-task-failure").with_resource("case.cmmn", model),
        )
        .expect("deployment");
    let case_instance = engine
        .start_case_instance_by_key(
            "processFailureParentCase",
            CmmnCaseInstanceStartRequest::new().with_business_key("BK-PF-1"),
        )
        .expect("case instance");

    let association = engine
        .runtime_service()
        .create_task_association_query()
        .case_instance_id(&case_instance.id)
        .single_result()
        .expect("query")
        .expect("association");
    assert_eq!(association.state, CmmnTaskAssociationState::Active);
    assert_eq!(
        association.child_instance_id.as_deref(),
        Some("process-instance-1")
    );

    // Simulate the BPMN child process reporting a failure (delete/terminate/uncaught).
    engine
        .runtime_service()
        .fail_process_task_child_instance("process-instance-1", "BPMN child failed")
        .expect("process task failure");

    let updated_association = engine
        .runtime_service()
        .create_task_association_query()
        .id(&association.id)
        .single_result()
        .expect("query")
        .expect("association");
    assert_eq!(updated_association.state, CmmnTaskAssociationState::Failed);
    assert!(updated_association.completed_at.is_some());
    assert_eq!(
        updated_association.failure_message.as_deref(),
        Some("BPMN child failed")
    );

    // The recovery task should now be active; the case should stay active.
    let recovery_task = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_instance.id)
        .state(CmmnHumanTaskState::Active)
        .single_result()
        .expect("recovery query")
        .expect("recovery task");
    assert_eq!(recovery_task.name, "Recovery");

    let case = engine
        .runtime_service()
        .get_case_instance(&case_instance.id)
        .expect("case instance");
    assert_eq!(case.state, CmmnCaseInstanceState::Active);

    let historic_case = engine
        .history_service()
        .get_historic_case_instance(&case_instance.id)
        .expect("historic case instance");
    assert_eq!(historic_case.state, CmmnCaseInstanceState::Active);
    assert!(historic_case.completed_at.is_none());
}

#[test]
fn failing_process_task_for_unknown_child_instance_returns_not_found() {
    let runner = Arc::new(RecordingProcessTaskRunner::default());
    let engine =
        CmmnEngine::new_in_memory_with_process_task_runner(runner.clone()).expect("engine");

    let model = CmmnModel::new(vec![CmmnCase::new(
        "case-no-association",
        "noAssociationCase",
        "No association case",
        CmmnCasePlanModel::new("case-plan-model", "Case plan model")
            .with_human_task(CmmnHumanTask::new("human-task-anchor", "Anchor"))
            .with_plan_item(CmmnPlanItem::new("plan-item-anchor", "human-task-anchor")),
    )]);

    engine
        .deploy(CmmnDeploymentRequest::new("noop").with_resource("case.cmmn", model))
        .expect("deployment");
    let case_instance = engine
        .start_case_instance_by_key("noAssociationCase", CmmnCaseInstanceStartRequest::new())
        .expect("case instance");

    let result = engine
        .runtime_service()
        .fail_process_task_child_instance("unknown-process-instance", "missing");
    assert!(
        result.is_err(),
        "missing association must surface a not-found error"
    );

    let case = engine
        .runtime_service()
        .get_case_instance(&case_instance.id)
        .expect("case instance");
    assert_eq!(case.state, CmmnCaseInstanceState::Active);
}

#[test]
fn blocking_case_task_starts_child_case_and_completes_parent_plan_item_when_child_completes() {
    let engine = CmmnEngine::new_in_memory().expect("engine");

    let child_case = CmmnCase::new(
        "case-child",
        "childCase",
        "Child case",
        CmmnCasePlanModel::new("child-plan-model", "Child plan")
            .with_human_task(CmmnHumanTask::new("human-task-child", "Child work"))
            .with_plan_item(CmmnPlanItem::new("plan-item-child", "human-task-child")),
    );
    let sentry = CmmnSentry::new(
        "sentry-after-case",
        CmmnPlanItemOnPart::new("on-case-complete", "plan-item-case", "complete"),
    );
    let parent_case = CmmnCase::new(
        "case-parent",
        "caseTaskParentCase",
        "Case task parent",
        CmmnCasePlanModel::new("parent-plan-model", "Parent plan")
            .with_case_task(
                flowable_cmmn_engine::CmmnCaseTask::new("case-task-child", "Child case task")
                    .with_case_ref("childCase"),
            )
            .with_plan_item(CmmnPlanItem::new("plan-item-case", "case-task-child"))
            .with_human_task(CmmnHumanTask::new("human-task-archive", "Archive"))
            .with_plan_item(
                CmmnPlanItem::new("plan-item-archive", "human-task-archive")
                    .with_entry_criterion("sentry-after-case"),
            )
            .with_sentry(sentry),
    );

    engine
        .deploy(
            CmmnDeploymentRequest::new("case-task")
                .with_resource("cases.cmmn", CmmnModel::new(vec![child_case, parent_case])),
        )
        .expect("deployment");
    let parent_instance = engine
        .start_case_instance_by_key("caseTaskParentCase", CmmnCaseInstanceStartRequest::new())
        .expect("parent case");

    let association = engine
        .runtime_service()
        .create_task_association_query()
        .case_instance_id(&parent_instance.id)
        .single_result()
        .expect("query")
        .expect("association");
    assert_eq!(association.state, CmmnTaskAssociationState::Active);
    let child_instance_id = association
        .child_instance_id
        .clone()
        .expect("child case instance id");

    let child_task = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&child_instance_id)
        .state(CmmnHumanTaskState::Active)
        .single_result()
        .expect("child task query")
        .expect("child task");
    engine
        .complete_human_task(&child_task.id, CmmnHumanTaskCompletionRequest::new())
        .expect("child task completion");

    let updated_association = engine
        .runtime_service()
        .create_task_association_query()
        .id(&association.id)
        .single_result()
        .expect("query")
        .expect("association");
    assert_eq!(
        updated_association.state,
        CmmnTaskAssociationState::Completed
    );

    let archive_task = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&parent_instance.id)
        .state(CmmnHumanTaskState::Active)
        .single_result()
        .expect("archive query")
        .expect("archive task");
    assert_eq!(archive_task.name, "Archive");
}

fn deploy_case_task_parent_with_terminate_sentry(
    engine: &CmmnEngine,
    parent_case_key: &str,
) -> String {
    let child_case = CmmnCase::new(
        "case-child",
        "childCase",
        "Child case",
        CmmnCasePlanModel::new("child-plan-model", "Child plan")
            .with_human_task(CmmnHumanTask::new("human-task-child", "Child work"))
            .with_plan_item(CmmnPlanItem::new("plan-item-child", "human-task-child")),
    );
    let sentry = CmmnSentry::new(
        "sentry-after-case-terminate",
        CmmnPlanItemOnPart::new("on-case-terminate", "plan-item-case", "terminate"),
    );
    let parent_case = CmmnCase::new(
        "case-parent",
        parent_case_key,
        "Case task parent",
        CmmnCasePlanModel::new("parent-plan-model", "Parent plan")
            .with_case_task(
                CmmnCaseTask::new("case-task-child", "Child case task").with_case_ref("childCase"),
            )
            .with_plan_item(CmmnPlanItem::new("plan-item-case", "case-task-child"))
            .with_human_task(CmmnHumanTask::new("human-task-recovery", "Recovery"))
            .with_plan_item(
                CmmnPlanItem::new("plan-item-recovery", "human-task-recovery")
                    .with_entry_criterion("sentry-after-case-terminate"),
            )
            .with_sentry(sentry),
    );

    engine
        .deploy(
            CmmnDeploymentRequest::new("case-task-terminate")
                .with_resource("cases.cmmn", CmmnModel::new(vec![child_case, parent_case])),
        )
        .expect("deployment");
    engine
        .start_case_instance_by_key(parent_case_key, CmmnCaseInstanceStartRequest::new())
        .expect("parent case")
        .id
}

fn assert_case_task_child_end_failed_parent_association_and_activated_recovery(
    engine: &CmmnEngine,
    parent_instance_id: &str,
    association_id: &str,
) {
    let updated_association = engine
        .runtime_service()
        .create_task_association_query()
        .id(association_id)
        .single_result()
        .expect("query")
        .expect("association");
    assert_eq!(updated_association.state, CmmnTaskAssociationState::Failed);
    assert!(updated_association.completed_at.is_some());
    assert!(updated_association.failure_message.is_some());

    let recovery_task = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(parent_instance_id)
        .state(CmmnHumanTaskState::Active)
        .single_result()
        .expect("recovery query")
        .expect("recovery task");
    assert_eq!(recovery_task.name, "Recovery");
}

#[test]
fn terminating_case_task_child_marks_parent_association_failed_and_triggers_terminate_sentry() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let parent_instance_id =
        deploy_case_task_parent_with_terminate_sentry(&engine, "caseTaskTerminateParentCase");
    let association = engine
        .runtime_service()
        .create_task_association_query()
        .case_instance_id(&parent_instance_id)
        .single_result()
        .expect("query")
        .expect("association");
    let child_instance_id = association
        .child_instance_id
        .clone()
        .expect("child case instance id");

    engine
        .runtime_service()
        .terminate_case_instance(&child_instance_id)
        .expect("terminate child case");

    assert_case_task_child_end_failed_parent_association_and_activated_recovery(
        &engine,
        &parent_instance_id,
        &association.id,
    );
}

#[test]
fn deleting_case_task_child_marks_parent_association_failed_and_triggers_terminate_sentry() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let parent_instance_id =
        deploy_case_task_parent_with_terminate_sentry(&engine, "caseTaskDeleteParentCase");
    let association = engine
        .runtime_service()
        .create_task_association_query()
        .case_instance_id(&parent_instance_id)
        .single_result()
        .expect("query")
        .expect("association");
    let child_instance_id = association
        .child_instance_id
        .clone()
        .expect("child case instance id");

    engine
        .runtime_service()
        .delete_case_instance(&child_instance_id)
        .expect("delete child case");

    assert_case_task_child_end_failed_parent_association_and_activated_recovery(
        &engine,
        &parent_instance_id,
        &association.id,
    );
}

#[test]
fn completes_empty_case_immediately_on_start() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let empty_case = CmmnModel::new(vec![CmmnCase::new(
        "case-empty",
        "emptyCase",
        "Empty case",
        CmmnCasePlanModel::new("case-plan-model", "Case plan model"),
    )]);

    engine
        .deploy(CmmnDeploymentRequest::new("empty").with_resource("empty-case.cmmn", empty_case))
        .expect("deployment");

    let case_instance = engine
        .start_case_instance_by_key("emptyCase", CmmnCaseInstanceStartRequest::new())
        .expect("case instance");

    assert_eq!(case_instance.state, CmmnCaseInstanceState::Completed);
    assert!(case_instance.ended_at.is_some());
}

fn event_listener_model() -> CmmnModel {
    let plan_model = CmmnCasePlanModel::new("case-plan-model", "Case plan model")
        .with_event_listener(
            CmmnEventListener::new("approval-event-listener", "message")
                .with_name("Wait for approval")
                .with_event_name("approvalReceived"),
        )
        .with_plan_item(CmmnPlanItem::new(
            "plan-item-approval-event",
            "approval-event-listener",
        ));

    CmmnModel::new(vec![CmmnCase::new(
        "case-event-listener",
        "eventListenerCase",
        "Event listener case",
        plan_model,
    )])
}

fn manual_stage_model() -> CmmnModel {
    let review_stage = CmmnStage::new("stage-review", "Review stage")
        .with_human_task(CmmnHumanTask::new("human-task-review", "Review"))
        .with_plan_item(CmmnPlanItem::new("plan-item-review", "human-task-review"));

    let plan_model = CmmnCasePlanModel::new("case-plan-model", "Case plan model")
        .with_stage(review_stage)
        .with_plan_item(
            CmmnPlanItem::new("plan-item-review-stage", "stage-review")
                .with_manual_activation_rule("manualStage == true"),
        );

    CmmnModel::new(vec![CmmnCase::new(
        "case-manual-stage",
        "manualStageCase",
        "Manual stage case",
        plan_model,
    )])
}

fn repeatable_stage_model() -> CmmnModel {
    let review_stage = CmmnStage::new("stage-repeat", "Repeat stage")
        .with_human_task(CmmnHumanTask::new("human-task-review", "Review"))
        .with_plan_item(CmmnPlanItem::new("plan-item-review", "human-task-review"));

    let plan_model = CmmnCasePlanModel::new("case-plan-model", "Case plan model")
        .with_stage(review_stage)
        .with_plan_item(
            CmmnPlanItem::new("plan-item-repeat-stage", "stage-repeat")
                .with_repetition_rule("repeatStage == true"),
        );

    CmmnModel::new(vec![CmmnCase::new(
        "case-repeat-stage",
        "repeatStageCase",
        "Repeat stage case",
        plan_model,
    )])
}

fn stage_planning_table_model() -> CmmnModel {
    let review_stage = CmmnStage::new("stage-planning-review", "Planning review")
        .with_human_task(CmmnHumanTask::new("human-task-anchor", "Anchor"))
        .with_human_task(CmmnHumanTask::new("human-task-peer-review", "Peer review"))
        .with_plan_item(CmmnPlanItem::new("plan-item-anchor", "human-task-anchor"))
        .with_planning_table(
            CmmnPlanningTable::new("planning-table-review", "Review planning")
                .with_discretionary_item(flowable_cmmn_engine::CmmnDiscretionaryItem::new(
                    "discretionary-peer-review",
                    "Peer review",
                    "human-task-peer-review",
                )),
        );

    let plan_model = CmmnCasePlanModel::new("case-plan-model", "Case plan model")
        .with_stage(review_stage)
        .with_plan_item(CmmnPlanItem::new(
            "plan-item-planning-review-stage",
            "stage-planning-review",
        ));

    CmmnModel::new(vec![CmmnCase::new(
        "case-stage-planning-table",
        "stagePlanningTableCase",
        "Stage planning table case",
        plan_model,
    )])
}

fn case_level_planning_table_model() -> CmmnModel {
    let plan_model = CmmnCasePlanModel::new("case-plan-model", "Case plan model")
        .with_human_task(CmmnHumanTask::new("human-task-anchor", "Anchor"))
        .with_human_task(CmmnHumanTask::new("human-task-case-review", "Case review"))
        .with_plan_item(CmmnPlanItem::new("plan-item-anchor", "human-task-anchor"))
        .with_planning_table(
            CmmnPlanningTable::new("planning-table-case", "Case planning").with_discretionary_item(
                flowable_cmmn_engine::CmmnDiscretionaryItem::new(
                    "discretionary-case-review",
                    "Case review",
                    "human-task-case-review",
                ),
            ),
        );

    CmmnModel::new(vec![CmmnCase::new(
        "case-level-planning-table",
        "caseLevelPlanningTableCase",
        "Case level planning table case",
        plan_model,
    )])
}

fn discretionary_only_case_level_planning_table_model() -> CmmnModel {
    let plan_model = CmmnCasePlanModel::new("case-plan-model", "Case plan model")
        .with_human_task(CmmnHumanTask::new("human-task-case-review", "Case review"))
        .with_planning_table(
            CmmnPlanningTable::new("planning-table-case", "Case planning").with_discretionary_item(
                flowable_cmmn_engine::CmmnDiscretionaryItem::new(
                    "discretionary-case-review",
                    "Case review",
                    "human-task-case-review",
                ),
            ),
        );

    CmmnModel::new(vec![CmmnCase::new(
        "case-level-discretionary-only",
        "caseLevelDiscretionaryOnlyCase",
        "Case level discretionary only case",
        plan_model,
    )])
}

fn manual_decision_task_model() -> CmmnModel {
    let plan_model = CmmnCasePlanModel::new("case-plan-model", "Case plan model")
        .with_decision_task(
            CmmnDecisionTask::new("approval-decision", "Approval decision")
                .with_decision_ref("approvalDecision"),
        )
        .with_human_task(CmmnHumanTask::new("human-task-archive", "Archive"))
        .with_plan_item(
            CmmnPlanItem::new("plan-item-approval-decision", "approval-decision")
                .with_manual_activation_rule("manualDecision == true"),
        )
        .with_plan_item(
            CmmnPlanItem::new("plan-item-archive", "human-task-archive")
                .with_entry_criterion("sentry-after-decision"),
        )
        .with_sentry(CmmnSentry::new(
            "sentry-after-decision",
            CmmnPlanItemOnPart::new(
                "on-decision-complete",
                "plan-item-approval-decision",
                "complete",
            ),
        ));

    CmmnModel::new(vec![CmmnCase::new(
        "case-manual-decision-task",
        "manualDecisionTaskCase",
        "Manual decision task case",
        plan_model,
    )])
}

fn decision_task_repetition_model() -> CmmnModel {
    let plan_model = CmmnCasePlanModel::new("case-plan-model", "Case plan model")
        .with_decision_task(
            CmmnDecisionTask::new("approval-decision", "Approval decision")
                .with_decision_ref("approvalDecision"),
        )
        .with_human_task(CmmnHumanTask::new("human-task-archive", "Archive"))
        .with_plan_item(
            CmmnPlanItem::new("plan-item-approval-decision", "approval-decision")
                .with_manual_activation_rule("manualDecision == true")
                .with_repetition_rule("repeat == true"),
        )
        .with_plan_item(
            CmmnPlanItem::new("plan-item-archive", "human-task-archive")
                .with_entry_criterion("sentry-after-decision"),
        )
        .with_sentry(CmmnSentry::new(
            "sentry-after-decision",
            CmmnPlanItemOnPart::new(
                "on-decision-complete",
                "plan-item-approval-decision",
                "complete",
            ),
        ));

    CmmnModel::new(vec![CmmnCase::new(
        "case-decision-task-repetition",
        "decisionTaskRepetitionCase",
        "Decision task repetition case",
        plan_model,
    )])
}

fn manual_milestone_and_event_listener_model() -> CmmnModel {
    let plan_model = CmmnCasePlanModel::new("case-plan-model", "Case plan model")
        .with_milestone(CmmnMilestone::new("milestone-approved", "Approved"))
        .with_event_listener(
            CmmnEventListener::new("approval-event-listener", "message")
                .with_name("Approval event")
                .with_event_name("approvalReceived"),
        )
        .with_plan_item(
            CmmnPlanItem::new("plan-item-approved", "milestone-approved")
                .with_manual_activation_rule("manualMilestone == true"),
        )
        .with_plan_item(
            CmmnPlanItem::new("plan-item-approval-event", "approval-event-listener")
                .with_manual_activation_rule("manualEvent == true"),
        );

    CmmnModel::new(vec![CmmnCase::new(
        "case-manual-milestone-event",
        "manualMilestoneEventCase",
        "Manual milestone and event case",
        plan_model,
    )])
}

fn milestone_entry_criterion_model() -> CmmnModel {
    let plan_model = CmmnCasePlanModel::new("case-plan-model", "Case plan model")
        .with_human_task(CmmnHumanTask::new("human-task-review", "Review"))
        .with_human_task(CmmnHumanTask::new("human-task-archive", "Archive"))
        .with_milestone(CmmnMilestone::new("milestone-reviewed", "Reviewed"))
        .with_plan_item(CmmnPlanItem::new("plan-item-review", "human-task-review"))
        .with_plan_item(
            CmmnPlanItem::new("plan-item-reviewed", "milestone-reviewed")
                .with_entry_criterion("sentry-review-completed"),
        )
        .with_plan_item(
            CmmnPlanItem::new("plan-item-archive", "human-task-archive")
                .with_entry_criterion("sentry-after-reviewed"),
        )
        .with_sentry(
            CmmnSentry::new(
                "sentry-review-completed",
                CmmnPlanItemOnPart::new("on-review-complete", "plan-item-review", "complete"),
            )
            .with_if_part("approved == true"),
        )
        .with_sentry(CmmnSentry::new(
            "sentry-after-reviewed",
            CmmnPlanItemOnPart::new("on-reviewed-occur", "plan-item-reviewed", "occur"),
        ));

    CmmnModel::new(vec![CmmnCase::new(
        "case-milestone-entry-criterion",
        "milestoneEntryCriterionCase",
        "Milestone entry criterion case",
        plan_model,
    )])
}

fn change_state_id_model() -> CmmnModel {
    let source_stage = CmmnStage::new("stage-source", "Source stage")
        .with_human_task(CmmnHumanTask::new(
            "human-task-source-child",
            "Source child",
        ))
        .with_plan_item(CmmnPlanItem::new(
            "plan-item-source-child",
            "human-task-source-child",
        ));
    let target_stage = CmmnStage::new("stage-target", "Target stage")
        .with_human_task(CmmnHumanTask::new(
            "human-task-target-child",
            "Target child",
        ))
        .with_plan_item(CmmnPlanItem::new(
            "plan-item-target-child",
            "human-task-target-child",
        ));

    let plan_model = CmmnCasePlanModel::new("case-plan-model", "Case plan model")
        .with_human_task(CmmnHumanTask::new("human-task-source", "Source task"))
        .with_human_task(CmmnHumanTask::new("human-task-target", "Target task"))
        .with_plan_item(CmmnPlanItem::new("plan-item-source", "human-task-source"))
        .with_plan_item(
            CmmnPlanItem::new("plan-item-target", "human-task-target")
                .with_entry_criterion("sentry-never"),
        )
        .with_stage(source_stage)
        .with_stage(target_stage)
        .with_plan_item(CmmnPlanItem::new("plan-item-source-stage", "stage-source"))
        .with_plan_item(
            CmmnPlanItem::new("plan-item-target-stage", "stage-target")
                .with_entry_criterion("sentry-never"),
        )
        .with_sentry(
            CmmnSentry::new(
                "sentry-never",
                CmmnPlanItemOnPart::new("on-never", "plan-item-source", "complete"),
            )
            .with_if_part("enabled == true"),
        );

    CmmnModel::new(vec![CmmnCase::new(
        "case-change-state-id",
        "changeStateIdCase",
        "Change state id case",
        plan_model,
    )])
}

fn deploy_change_state_id_case(engine: &CmmnEngine) -> String {
    engine
        .deploy(
            CmmnDeploymentRequest::new("change-state-id")
                .with_resource("change-state-id-case.cmmn", change_state_id_model()),
        )
        .expect("deployment");

    engine
        .start_case_instance_by_key(
            "changeStateIdCase",
            CmmnCaseInstanceStartRequest::new().with_variables(json!({ "enabled": false })),
        )
        .expect("case instance")
        .id
}

#[test]
fn change_state_by_plan_item_instance_id_moves_human_task_to_target_plan_item() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let case_instance_id = deploy_change_state_id_case(&engine);
    let source_task = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_instance_id)
        .state(CmmnHumanTaskState::Active)
        .list()
        .expect("active tasks")
        .into_iter()
        .find(|task| task.task_definition_id == "human-task-source")
        .expect("source task");

    engine
        .runtime_service()
        .change_plan_item_state(
            &case_instance_id,
            CmmnChangePlanItemStateRequest {
                change_plan_item_ids: vec![(
                    source_task.id.clone(),
                    "plan-item-target".to_string(),
                )],
                ..Default::default()
            },
        )
        .expect("change by target plan item id");

    let tasks = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_instance_id)
        .state(CmmnHumanTaskState::Active)
        .list()
        .expect("active tasks after change");
    assert!(
        tasks
            .iter()
            .any(|task| task.task_definition_id == "human-task-target"
                && task.plan_item_id == "plan-item-target")
    );
    assert!(tasks
        .iter()
        .all(|task| task.id != source_task.id && task.task_definition_id != "human-task-source"));
}

#[test]
fn change_state_by_plan_item_instance_id_and_definition_id_moves_human_task() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let case_instance_id = deploy_change_state_id_case(&engine);
    let source_task = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_instance_id)
        .state(CmmnHumanTaskState::Active)
        .list()
        .expect("active tasks")
        .into_iter()
        .find(|task| task.task_definition_id == "human-task-source")
        .expect("source task");

    engine
        .runtime_service()
        .change_plan_item_state(
            &case_instance_id,
            CmmnChangePlanItemStateRequest {
                change_plan_item_ids_with_definition_id: vec![(
                    source_task.id,
                    "human-task-target".to_string(),
                )],
                ..Default::default()
            },
        )
        .expect("change by target definition id");

    let target_task = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_instance_id)
        .state(CmmnHumanTaskState::Active)
        .list()
        .expect("active tasks after change")
        .into_iter()
        .find(|task| task.task_definition_id == "human-task-target")
        .expect("target task");
    assert_eq!(target_task.plan_item_id, "plan-item-target");
}

#[test]
fn change_state_by_definition_mapping_moves_human_task_and_stage_targets() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let case_instance_id = deploy_change_state_id_case(&engine);

    engine
        .runtime_service()
        .change_plan_item_state(
            &case_instance_id,
            CmmnChangePlanItemStateRequest {
                change_plan_item_definitions_with_new_target_ids: vec![
                    CmmnPlanItemDefinitionWithTargetIds {
                        existing_plan_item_definition_id: "human-task-source".to_string(),
                        new_plan_item_id: "plan-item-target".to_string(),
                        new_plan_item_definition_id: "human-task-target".to_string(),
                    },
                    CmmnPlanItemDefinitionWithTargetIds {
                        existing_plan_item_definition_id: "stage-source".to_string(),
                        new_plan_item_id: "plan-item-target-stage".to_string(),
                        new_plan_item_definition_id: "stage-target".to_string(),
                    },
                ],
                ..Default::default()
            },
        )
        .expect("change by definition mappings");

    let active_tasks = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_instance_id)
        .state(CmmnHumanTaskState::Active)
        .list()
        .expect("active tasks after mapping");
    assert!(
        active_tasks
            .iter()
            .any(|task| task.task_definition_id == "human-task-target")
    );
    assert!(
        active_tasks
            .iter()
            .any(|task| task.task_definition_id == "human-task-target-child")
    );
    assert!(active_tasks.iter().all(|task| {
        task.task_definition_id != "human-task-source"
            && task.task_definition_id != "human-task-source-child"
    }));
}

#[test]
fn change_state_id_based_operation_rejects_unsupported_target_shape() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let case_instance_id = deploy_change_state_id_case(&engine);
    let source_task = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_instance_id)
        .state(CmmnHumanTaskState::Active)
        .list()
        .expect("active tasks")
        .into_iter()
        .find(|task| task.task_definition_id == "human-task-source")
        .expect("source task");

    let error = engine
        .runtime_service()
        .change_plan_item_state(
            &case_instance_id,
            CmmnChangePlanItemStateRequest {
                change_plan_item_ids_with_definition_id: vec![(
                    source_task.id,
                    "stage-target".to_string(),
                )],
                ..Default::default()
            },
        )
        .expect_err("human task to stage must be unsupported");

    match error {
        CmmnError::UnsupportedModel { feature, message } => {
            assert_eq!(feature, "change-state");
            assert!(message.contains("outside the supported runtime subset"));
        }
        other => panic!("unexpected error: {other}"),
    }
}

#[test]
fn milestone_entry_criterion_occurs_immediately_and_triggers_on_part_dependents() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    engine
        .deploy(CmmnDeploymentRequest::new("milestone-entry").with_resource(
            "milestone-entry-case.cmmn",
            milestone_entry_criterion_model(),
        ))
        .expect("deployment");

    let case_instance = engine
        .start_case_instance_by_key(
            "milestoneEntryCriterionCase",
            CmmnCaseInstanceStartRequest::new().with_variables(json!({ "approved": true })),
        )
        .expect("case instance");

    let review_task = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_instance.id)
        .state(CmmnHumanTaskState::Active)
        .single_result()
        .expect("review task query")
        .expect("review task");
    assert_eq!(review_task.name, "Review");

    engine
        .complete_human_task(&review_task.id, CmmnHumanTaskCompletionRequest::new())
        .expect("review completion");

    let milestones = engine
        .history_service()
        .create_historic_milestone_query()
        .case_instance_id(&case_instance.id)
        .milestone_id("milestone-reviewed")
        .list()
        .expect("historic milestones");
    assert_eq!(milestones.len(), 1);
    assert_eq!(milestones[0].name, "Reviewed");

    let active_tasks = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_instance.id)
        .state(CmmnHumanTaskState::Active)
        .list()
        .expect("active tasks");
    assert_eq!(active_tasks.len(), 1);
    assert_eq!(active_tasks[0].name, "Archive");

    let all_tasks = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_instance.id)
        .list()
        .expect("all tasks");
    assert!(
        all_tasks
            .iter()
            .all(|task| task.plan_item_id != "plan-item-reviewed")
    );
}

#[test]
fn starts_case_instance_creates_event_subscription_and_completion_cleans_it() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    engine
        .deploy(
            CmmnDeploymentRequest::new("event-listener")
                .with_resource("event-listener-case.cmmn", event_listener_model()),
        )
        .expect("deployment");

    let case_instance = engine
        .start_case_instance_by_key("eventListenerCase", CmmnCaseInstanceStartRequest::new())
        .expect("case instance");

    let subscriptions = engine
        .runtime_service()
        .create_event_subscription_query()
        .case_instance_id(&case_instance.id)
        .list()
        .expect("event subscriptions");
    assert_eq!(subscriptions.len(), 1);
    let subscription = &subscriptions[0];
    assert_eq!(subscription.event_type, "message");
    assert_eq!(subscription.event_name.as_deref(), Some("approvalReceived"));
    assert_eq!(
        subscription.activity_id.as_deref(),
        Some("approval-event-listener")
    );
    assert_eq!(
        subscription.case_definition_id.as_deref(),
        Some(case_instance.case_definition_id.as_str())
    );
    assert_eq!(
        subscription.plan_item_instance_id.as_deref(),
        Some("plan-item-approval-event")
    );

    let task = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_instance.id)
        .list()
        .expect("tasks");
    assert!(task.is_empty());

    engine
        .runtime_service()
        .complete_event_subscription(&subscription.id)
        .expect("complete event subscription");
    let completed_case = engine
        .runtime_service()
        .get_case_instance(&case_instance.id)
        .expect("completed case");
    assert_eq!(completed_case.state, CmmnCaseInstanceState::Completed);

    let remaining = engine
        .runtime_service()
        .create_event_subscription_query()
        .case_instance_id(&case_instance.id)
        .list()
        .expect("remaining subscriptions");
    assert!(remaining.is_empty());
}

#[test]
fn manual_activation_stage_waits_until_change_state_activation() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    engine
        .deploy(
            CmmnDeploymentRequest::new("manual-stage")
                .with_resource("manual-stage-case.cmmn", manual_stage_model()),
        )
        .expect("deployment");

    let case_instance = engine
        .start_case_instance_by_key(
            "manualStageCase",
            CmmnCaseInstanceStartRequest::new().with_variables(json!({ "manualStage": true })),
        )
        .expect("case instance");

    let active_tasks = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_instance.id)
        .state(CmmnHumanTaskState::Active)
        .list()
        .expect("active tasks");
    assert!(active_tasks.is_empty());

    let stage_overview = engine
        .runtime_service()
        .get_stage_overview(&case_instance.id)
        .expect("stage overview");
    assert_eq!(stage_overview.len(), 1);
    assert_eq!(stage_overview[0].id, "stage-review");
    assert!(!stage_overview[0].current);
    assert!(!stage_overview[0].ended);

    engine
        .runtime_service()
        .change_plan_item_state(
            &case_instance.id,
            CmmnChangePlanItemStateRequest {
                activate_plan_item_definition_ids: vec!["stage-review".to_string()],
                ..Default::default()
            },
        )
        .expect("activate manual stage");

    let active_after_activation = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_instance.id)
        .state(CmmnHumanTaskState::Active)
        .single_result()
        .expect("active task query")
        .expect("stage child task");
    assert_eq!(
        active_after_activation.task_definition_id,
        "human-task-review"
    );

    let active_stage = engine
        .runtime_service()
        .get_stage_overview(&case_instance.id)
        .expect("stage overview after activation")
        .into_iter()
        .find(|stage| stage.id == "stage-review")
        .expect("review stage");
    assert!(active_stage.current);
    assert!(!active_stage.ended);
}

#[test]
fn completing_stage_with_matching_repetition_rule_creates_available_repeat_instance() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    engine
        .deploy(
            CmmnDeploymentRequest::new("repeat-stage")
                .with_resource("repeat-stage-case.cmmn", repeatable_stage_model()),
        )
        .expect("deployment");

    let case_instance = engine
        .start_case_instance_by_key(
            "repeatStageCase",
            CmmnCaseInstanceStartRequest::new().with_variables(json!({ "repeatStage": true })),
        )
        .expect("case instance");

    let first_task = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_instance.id)
        .state(CmmnHumanTaskState::Active)
        .single_result()
        .expect("first task query")
        .expect("first stage child task");
    engine
        .complete_human_task(&first_task.id, CmmnHumanTaskCompletionRequest::new())
        .expect("complete first child task");

    let stage_overview = engine
        .runtime_service()
        .get_stage_overview(&case_instance.id)
        .expect("stage overview after repetition");
    assert_eq!(stage_overview.len(), 2);
    assert_eq!(stage_overview.iter().filter(|stage| stage.ended).count(), 1);
    assert_eq!(
        stage_overview
            .iter()
            .filter(|stage| !stage.current && !stage.ended)
            .count(),
        1
    );

    engine
        .runtime_service()
        .change_plan_item_state(
            &case_instance.id,
            CmmnChangePlanItemStateRequest {
                activate_plan_item_definition_ids: vec!["stage-repeat".to_string()],
                ..Default::default()
            },
        )
        .expect("activate repeat stage");

    let repeated_task = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_instance.id)
        .state(CmmnHumanTaskState::Active)
        .single_result()
        .expect("repeated task query")
        .expect("repeated stage child task");
    assert_ne!(repeated_task.id, first_task.id);
    assert_eq!(repeated_task.task_definition_id, "human-task-review");
}

#[test]
fn activates_discretionary_human_task_from_stage_planning_table() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    engine
        .deploy(
            CmmnDeploymentRequest::new("stage-planning-table").with_resource(
                "stage-planning-table-case.cmmn",
                stage_planning_table_model(),
            ),
        )
        .expect("deployment");

    let deployed_definition = engine
        .repository_service()
        .create_case_definition_query()
        .key("stagePlanningTableCase")
        .single_result()
        .expect("definition query")
        .expect("deployed definition");
    let deployed_stage = deployed_definition
        .model
        .case_plan_model
        .stages
        .iter()
        .find(|stage| stage.id == "stage-planning-review")
        .expect("deployed review stage");
    assert_eq!(deployed_stage.planning_tables.len(), 1);
    assert_eq!(
        deployed_stage.planning_tables[0].discretionary_items[0].definition_ref,
        "human-task-peer-review"
    );

    let case_instance = engine
        .start_case_instance_by_key(
            "stagePlanningTableCase",
            CmmnCaseInstanceStartRequest::new(),
        )
        .expect("case instance");

    let active_before = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_instance.id)
        .state(CmmnHumanTaskState::Active)
        .list()
        .expect("active tasks before discretionary activation");
    assert_eq!(active_before.len(), 1);
    let anchor_task = active_before
        .iter()
        .find(|task| task.task_definition_id == "human-task-anchor")
        .expect("anchor task");
    let stage_instance_id = anchor_task
        .stage_instance_id
        .clone()
        .expect("anchor task should belong to review stage");

    engine
        .runtime_service()
        .change_plan_item_state(
            &case_instance.id,
            CmmnChangePlanItemStateRequest {
                activate_plan_item_definition_ids: vec!["human-task-peer-review".to_string()],
                ..Default::default()
            },
        )
        .expect("activate discretionary peer review");

    let active_after = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_instance.id)
        .state(CmmnHumanTaskState::Active)
        .list()
        .expect("active tasks after discretionary activation");
    assert_eq!(active_after.len(), 2);
    let peer_review = active_after
        .iter()
        .find(|task| task.task_definition_id == "human-task-peer-review")
        .expect("peer review discretionary task");
    assert_eq!(peer_review.plan_item_id, "discretionary-peer-review");
    assert_eq!(
        peer_review.stage_instance_id.as_deref(),
        Some(stage_instance_id.as_str())
    );
}

#[test]
fn activates_discretionary_human_task_from_case_level_planning_table() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    engine
        .deploy(
            CmmnDeploymentRequest::new("case-level-planning-table").with_resource(
                "case-level-planning-table-case.cmmn",
                case_level_planning_table_model(),
            ),
        )
        .expect("deployment");

    let deployed_definition = engine
        .repository_service()
        .create_case_definition_query()
        .key("caseLevelPlanningTableCase")
        .single_result()
        .expect("definition query")
        .expect("deployed definition");
    assert_eq!(
        deployed_definition.model.case_plan_model.planning_tables[0].discretionary_items[0]
            .definition_ref,
        "human-task-case-review"
    );

    let case_instance = engine
        .start_case_instance_by_key(
            "caseLevelPlanningTableCase",
            CmmnCaseInstanceStartRequest::new(),
        )
        .expect("case instance");

    let active_before = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_instance.id)
        .state(CmmnHumanTaskState::Active)
        .list()
        .expect("active tasks before discretionary activation");
    assert_eq!(active_before.len(), 1);
    assert_eq!(active_before[0].task_definition_id, "human-task-anchor");
    assert_eq!(active_before[0].stage_instance_id, None);

    engine
        .runtime_service()
        .change_plan_item_state(
            &case_instance.id,
            CmmnChangePlanItemStateRequest {
                activate_plan_item_definition_ids: vec!["human-task-case-review".to_string()],
                ..Default::default()
            },
        )
        .expect("activate case-level discretionary review");

    let active_after = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_instance.id)
        .state(CmmnHumanTaskState::Active)
        .list()
        .expect("active tasks after discretionary activation");
    assert_eq!(active_after.len(), 2);
    let case_review = active_after
        .iter()
        .find(|task| task.task_definition_id == "human-task-case-review")
        .expect("case review discretionary task");
    assert_eq!(case_review.plan_item_id, "discretionary-case-review");
    assert_eq!(case_review.stage_instance_id, None);
}

#[test]
fn case_level_discretionary_only_case_stays_active_until_planned_task_completes() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    engine
        .deploy(
            CmmnDeploymentRequest::new("case-level-discretionary-only").with_resource(
                "case-level-discretionary-only.cmmn",
                discretionary_only_case_level_planning_table_model(),
            ),
        )
        .expect("deployment");

    let case_instance = engine
        .start_case_instance_by_key(
            "caseLevelDiscretionaryOnlyCase",
            CmmnCaseInstanceStartRequest::new(),
        )
        .expect("case instance");
    assert_eq!(case_instance.state, CmmnCaseInstanceState::Active);
    assert!(
        engine
            .runtime_service()
            .create_human_task_query()
            .case_instance_id(&case_instance.id)
            .list()
            .expect("tasks before discretionary activation")
            .is_empty()
    );

    engine
        .runtime_service()
        .change_plan_item_state(
            &case_instance.id,
            CmmnChangePlanItemStateRequest {
                activate_plan_item_definition_ids: vec!["human-task-case-review".to_string()],
                ..Default::default()
            },
        )
        .expect("activate case-level discretionary review");
    let active_task = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_instance.id)
        .state(CmmnHumanTaskState::Active)
        .single_result()
        .expect("active task query")
        .expect("active discretionary task");
    assert_eq!(active_task.plan_item_id, "discretionary-case-review");

    engine
        .runtime_service()
        .complete_human_task(&active_task.id, CmmnHumanTaskCompletionRequest::new())
        .expect("complete discretionary task");
    let completed_case = engine
        .runtime_service()
        .get_case_instance(&case_instance.id)
        .expect("completed case");
    assert_eq!(completed_case.state, CmmnCaseInstanceState::Completed);

    let err = engine
        .runtime_service()
        .change_plan_item_state(
            &case_instance.id,
            CmmnChangePlanItemStateRequest {
                activate_plan_item_definition_ids: vec!["human-task-case-review".to_string()],
                ..Default::default()
            },
        )
        .expect_err("completed case must reject change-state operations");
    assert!(matches!(err, CmmnError::Execution { .. }));
}

#[test]
fn manual_activation_decision_task_waits_available_until_change_state_activation() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    engine
        .deploy(
            CmmnDeploymentRequest::new("manual-decision-task").with_resource(
                "manual-decision-task-case.cmmn",
                manual_decision_task_model(),
            ),
        )
        .expect("deployment");

    let case_instance = engine
        .start_case_instance_by_key(
            "manualDecisionTaskCase",
            CmmnCaseInstanceStartRequest::new().with_variables(json!({
                "manualDecision": true
            })),
        )
        .expect("case instance");

    assert_eq!(
        engine
            .runtime_service()
            .get_case_instance(&case_instance.id)
            .expect("case before decision activation")
            .state,
        CmmnCaseInstanceState::Active
    );
    assert!(
        engine
            .runtime_service()
            .create_human_task_query()
            .case_instance_id(&case_instance.id)
            .list()
            .expect("tasks before activation")
            .is_empty()
    );

    engine
        .runtime_service()
        .change_plan_item_state(
            &case_instance.id,
            CmmnChangePlanItemStateRequest {
                activate_plan_item_definition_ids: vec!["approval-decision".to_string()],
                ..Default::default()
            },
        )
        .expect("activate decision task");

    let archive_task = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_instance.id)
        .state(CmmnHumanTaskState::Active)
        .single_result()
        .expect("archive task query")
        .expect("archive task after decision completion");
    assert_eq!(archive_task.task_definition_id, "human-task-archive");

    engine
        .complete_human_task(&archive_task.id, CmmnHumanTaskCompletionRequest::new())
        .expect("archive completion");
    assert_eq!(
        engine
            .runtime_service()
            .get_case_instance(&case_instance.id)
            .expect("case after archive")
            .state,
        CmmnCaseInstanceState::Completed
    );
}

#[test]
fn completing_decision_task_with_matching_repetition_rule_requeues_available_instance() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    engine
        .deploy(
            CmmnDeploymentRequest::new("decision-task-repetition").with_resource(
                "decision-task-repetition-case.cmmn",
                decision_task_repetition_model(),
            ),
        )
        .expect("deployment");

    let case_instance = engine
        .start_case_instance_by_key(
            "decisionTaskRepetitionCase",
            CmmnCaseInstanceStartRequest::new().with_variables(json!({
                "manualDecision": true,
                "repeat": true
            })),
        )
        .expect("case instance");

    assert_eq!(
        engine
            .runtime_service()
            .get_case_instance(&case_instance.id)
            .expect("case before first decision activation")
            .state,
        CmmnCaseInstanceState::Active
    );

    engine
        .runtime_service()
        .change_plan_item_state(
            &case_instance.id,
            CmmnChangePlanItemStateRequest {
                activate_plan_item_definition_ids: vec!["approval-decision".to_string()],
                ..Default::default()
            },
        )
        .expect("first decision activation");

    let archive_task = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_instance.id)
        .state(CmmnHumanTaskState::Active)
        .single_result()
        .expect("archive task query")
        .expect("archive task after first decision completion");
    assert_eq!(archive_task.task_definition_id, "human-task-archive");

    // Completing the dependent human task must not complete the case while the
    // decision plan item remains available for the next repetition.
    engine
        .complete_human_task(&archive_task.id, CmmnHumanTaskCompletionRequest::new())
        .expect("archive completion");
    assert_eq!(
        engine
            .runtime_service()
            .get_case_instance(&case_instance.id)
            .expect("case after archive with decision waiting for repetition")
            .state,
        CmmnCaseInstanceState::Active
    );

    // Second activation proves the repetition re-queued the decision task.
    engine
        .runtime_service()
        .change_plan_item_state(
            &case_instance.id,
            CmmnChangePlanItemStateRequest {
                activate_plan_item_definition_ids: vec!["approval-decision".to_string()],
                ..Default::default()
            },
        )
        .expect("second decision activation after repetition");

    assert_eq!(
        engine
            .runtime_service()
            .get_case_instance(&case_instance.id)
            .expect("case after second decision activation")
            .state,
        CmmnCaseInstanceState::Active
    );
}

#[test]
fn decision_task_without_matching_repetition_rule_completes_case_after_dependents() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    engine
        .deploy(
            CmmnDeploymentRequest::new("decision-task-no-repetition").with_resource(
                "decision-task-repetition-case.cmmn",
                decision_task_repetition_model(),
            ),
        )
        .expect("deployment");

    let case_instance = engine
        .start_case_instance_by_key(
            "decisionTaskRepetitionCase",
            CmmnCaseInstanceStartRequest::new().with_variables(json!({
                "manualDecision": true,
                "repeat": false
            })),
        )
        .expect("case instance");

    engine
        .runtime_service()
        .change_plan_item_state(
            &case_instance.id,
            CmmnChangePlanItemStateRequest {
                activate_plan_item_definition_ids: vec!["approval-decision".to_string()],
                ..Default::default()
            },
        )
        .expect("decision activation");

    let archive_task = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_instance.id)
        .state(CmmnHumanTaskState::Active)
        .single_result()
        .expect("archive task query")
        .expect("archive task");
    engine
        .complete_human_task(&archive_task.id, CmmnHumanTaskCompletionRequest::new())
        .expect("archive completion");

    assert_eq!(
        engine
            .runtime_service()
            .get_case_instance(&case_instance.id)
            .expect("case after archive without repetition")
            .state,
        CmmnCaseInstanceState::Completed
    );
}

#[test]
fn manual_milestone_and_event_listener_wait_until_change_state_activation() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    engine
        .deploy(
            CmmnDeploymentRequest::new("manual-milestone-event").with_resource(
                "manual-milestone-event-case.cmmn",
                manual_milestone_and_event_listener_model(),
            ),
        )
        .expect("deployment");

    let case_instance = engine
        .start_case_instance_by_key(
            "manualMilestoneEventCase",
            CmmnCaseInstanceStartRequest::new().with_variables(json!({
                "manualMilestone": true,
                "manualEvent": true
            })),
        )
        .expect("case instance");

    assert!(
        engine
            .history_service()
            .create_historic_milestone_query()
            .case_instance_id(&case_instance.id)
            .list()
            .expect("milestones before activation")
            .is_empty()
    );
    assert!(
        engine
            .runtime_service()
            .create_event_subscription_query()
            .case_instance_id(&case_instance.id)
            .list()
            .expect("subscriptions before activation")
            .is_empty()
    );
    assert_eq!(
        engine
            .runtime_service()
            .get_case_instance(&case_instance.id)
            .expect("case before manual activation")
            .state,
        CmmnCaseInstanceState::Active
    );

    engine
        .runtime_service()
        .change_plan_item_state(
            &case_instance.id,
            CmmnChangePlanItemStateRequest {
                activate_plan_item_definition_ids: vec!["milestone-approved".to_string()],
                ..Default::default()
            },
        )
        .expect("activate milestone");
    engine
        .runtime_service()
        .change_plan_item_state(
            &case_instance.id,
            CmmnChangePlanItemStateRequest {
                activate_plan_item_definition_ids: vec!["approval-event-listener".to_string()],
                ..Default::default()
            },
        )
        .expect("activate event listener");

    let milestones = engine
        .history_service()
        .create_historic_milestone_query()
        .case_instance_id(&case_instance.id)
        .milestone_id("milestone-approved")
        .list()
        .expect("milestones after activation");
    assert_eq!(milestones.len(), 1);

    let subscription = engine
        .runtime_service()
        .create_event_subscription_query()
        .case_instance_id(&case_instance.id)
        .single_result()
        .expect("subscription after activation")
        .expect("event subscription");
    assert_eq!(
        subscription.activity_id.as_deref(),
        Some("approval-event-listener")
    );
}

#[test]
fn terminating_or_deleting_case_instance_cleans_event_subscriptions() {
    for delete in [false, true] {
        let engine = CmmnEngine::new_in_memory().expect("engine");
        engine
            .deploy(
                CmmnDeploymentRequest::new("event-listener")
                    .with_resource("event-listener-case.cmmn", event_listener_model()),
            )
            .expect("deployment");

        let case_instance = engine
            .start_case_instance_by_key("eventListenerCase", CmmnCaseInstanceStartRequest::new())
            .expect("case instance");
        assert_eq!(
            engine
                .runtime_service()
                .create_event_subscription_query()
                .case_instance_id(&case_instance.id)
                .list()
                .expect("subscriptions")
                .len(),
            1
        );

        if delete {
            engine
                .runtime_service()
                .delete_case_instance(&case_instance.id)
                .expect("delete case");
        } else {
            engine
                .runtime_service()
                .terminate_case_instance(&case_instance.id)
                .expect("terminate case");
        }

        assert!(
            engine
                .runtime_service()
                .create_event_subscription_query()
                .case_instance_id(&case_instance.id)
                .list()
                .expect("subscriptions after end")
                .is_empty()
        );
    }
}

#[test]
fn required_rule_prevents_stage_completion_when_required_task_incomplete() {
    let engine = CmmnEngine::new_in_memory().expect("engine");

    let review_stage = CmmnStage::new("stage-review", "Review stage")
        .with_human_task(CmmnHumanTask::new("human-task-approve", "Approve"))
        .with_plan_item(
            CmmnPlanItem::new("plan-item-approve", "human-task-approve")
                .with_required_rule("${approveRequired == true}"),
        )
        .with_human_task(CmmnHumanTask::new("human-task-optional", "Optional"))
        .with_plan_item(CmmnPlanItem::new(
            "plan-item-optional",
            "human-task-optional",
        ));

    let plan_model = CmmnCasePlanModel::new("case-plan-model", "Case plan model")
        .with_human_task(CmmnHumanTask::new("human-task-intake", "Intake"))
        .with_plan_item(CmmnPlanItem::new("plan-item-intake", "human-task-intake"))
        .with_stage(review_stage)
        .with_plan_item(CmmnPlanItem::new("plan-item-review-stage", "stage-review"));

    let model = CmmnModel::new(vec![CmmnCase::new(
        "case-required",
        "requiredCase",
        "Required rule case",
        plan_model,
    )]);

    engine
        .deploy(
            CmmnDeploymentRequest::new("required-rule-test")
                .with_resource("required-case.cmmn", model),
        )
        .expect("deployment");

    let case_instance = engine
        .start_case_instance_by_key(
            "requiredCase",
            CmmnCaseInstanceStartRequest::new().with_variables(json!({ "approveRequired": true })),
        )
        .expect("case instance");

    let runtime = engine.runtime_service();

    // Complete the intake task first
    let intake_tasks = runtime
        .create_human_task_query()
        .case_instance_id(&case_instance.id)
        .list()
        .expect("intake tasks");
    let intake_task = intake_tasks
        .iter()
        .find(|t| t.name == "Intake")
        .expect("intake task");
    runtime
        .complete_human_task(&intake_task.id, CmmnHumanTaskCompletionRequest::new())
        .expect("complete intake");

    // Complete the optional task in the stage
    let all_tasks = runtime
        .create_human_task_query()
        .case_instance_id(&case_instance.id)
        .list()
        .expect("all tasks");
    let optional_task = all_tasks
        .iter()
        .find(|t| t.name == "Optional")
        .expect("optional task");
    runtime
        .complete_human_task(&optional_task.id, CmmnHumanTaskCompletionRequest::new())
        .expect("complete optional");

    // Stage should NOT be completed because the required approve task is still open
    // Case should NOT be completed
    let case_check = runtime.get_case_instance(&case_instance.id).expect("case");
    assert_ne!(
        case_check.state,
        CmmnCaseInstanceState::Completed,
        "case should not complete while required task is incomplete"
    );

    // Now complete the required approve task
    let all_tasks = runtime
        .create_human_task_query()
        .case_instance_id(&case_instance.id)
        .list()
        .expect("all tasks");
    let approve_task = all_tasks
        .iter()
        .find(|t| t.name == "Approve")
        .expect("approve task");
    runtime
        .complete_human_task(&approve_task.id, CmmnHumanTaskCompletionRequest::new())
        .expect("complete approve");

    // Now case should be completed
    let case_check = runtime.get_case_instance(&case_instance.id).expect("case");
    assert_eq!(
        case_check.state,
        CmmnCaseInstanceState::Completed,
        "case should complete after all required tasks complete"
    );
}

#[test]
fn required_rule_allows_stage_completion_when_rule_evaluates_false() {
    let engine = CmmnEngine::new_in_memory().expect("engine");

    let review_stage = CmmnStage::new("stage-review", "Review stage")
        .with_human_task(CmmnHumanTask::new("human-task-approve", "Approve"))
        .with_plan_item(
            CmmnPlanItem::new("plan-item-approve", "human-task-approve")
                .with_required_rule("${approveRequired == true}"),
        )
        .with_human_task(CmmnHumanTask::new("human-task-optional", "Optional"))
        .with_plan_item(CmmnPlanItem::new(
            "plan-item-optional",
            "human-task-optional",
        ));

    let plan_model = CmmnCasePlanModel::new("case-plan-model", "Case plan model")
        .with_human_task(CmmnHumanTask::new("human-task-intake", "Intake"))
        .with_plan_item(CmmnPlanItem::new("plan-item-intake", "human-task-intake"))
        .with_stage(review_stage)
        .with_plan_item(CmmnPlanItem::new("plan-item-review-stage", "stage-review"));

    let model = CmmnModel::new(vec![CmmnCase::new(
        "case-required",
        "requiredCase",
        "Required rule case",
        plan_model,
    )]);

    engine
        .deploy(
            CmmnDeploymentRequest::new("required-rule-false-test")
                .with_resource("required-case.cmmn", model),
        )
        .expect("deployment");

    let case_instance = engine
        .start_case_instance_by_key(
            "requiredCase",
            CmmnCaseInstanceStartRequest::new().with_variables(json!({ "approveRequired": false })),
        )
        .expect("case instance");

    let runtime = engine.runtime_service();

    // Complete all tasks - when approveRequired is false, the approve task is not required
    // but it's still an open task that prevents stage completion
    let all_tasks = runtime
        .create_human_task_query()
        .case_instance_id(&case_instance.id)
        .list()
        .expect("all tasks");

    for task in &all_tasks {
        runtime
            .complete_human_task(&task.id, CmmnHumanTaskCompletionRequest::new())
            .expect("complete task");
    }

    // Case should be completed
    let case_check = runtime.get_case_instance(&case_instance.id).expect("case");
    assert_eq!(
        case_check.state,
        CmmnCaseInstanceState::Completed,
        "case should complete when all tasks done and requiredRule is false"
    );
}
