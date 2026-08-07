use flowable_cmmn_engine::{
    CmmnCase, CmmnCaseFileItem, CmmnCaseFileItemOnPart, CmmnCaseFileItemState,
    CmmnCaseInstanceStartRequest, CmmnCaseInstanceState, CmmnCasePlanModel, CmmnDeploymentRequest,
    CmmnDiscretionaryItem, CmmnEngine, CmmnHumanTask, CmmnHumanTaskCompletionRequest,
    CmmnHumanTaskState, CmmnModel, CmmnPlanFragment, CmmnPlanItem, CmmnPlanItemOnPart, CmmnSentry,
};
use serde_json::json;

#[test]
fn case_file_item_on_part_create_triggers_sentry() {
    let engine = CmmnEngine::new_in_memory().expect("engine");

    let sentry = CmmnSentry::new(
        "sentry-case-file-create",
        CmmnPlanItemOnPart::new("on-task-complete", "plan-item-intake", "complete"),
    )
    .with_case_file_item_on_part(CmmnCaseFileItemOnPart::new(
        "on-document-create",
        "document",
        "create",
    ));

    let plan_model = CmmnCasePlanModel::new("case-plan-model", "Case plan model")
        .with_human_task(CmmnHumanTask::new("human-task-intake", "Intake"))
        .with_plan_item(CmmnPlanItem::new("plan-item-intake", "human-task-intake"))
        .with_human_task(CmmnHumanTask::new("human-task-review", "Review"))
        .with_plan_item(
            CmmnPlanItem::new("plan-item-review", "human-task-review")
                .with_entry_criterion("sentry-case-file-create"),
        )
        .with_sentry(sentry);

    let model = CmmnModel::new(vec![CmmnCase::new(
        "case-file-item",
        "caseFileItemCase",
        "Case file item case",
        plan_model,
    )]);

    engine
        .deploy(
            CmmnDeploymentRequest::new("case-file-item")
                .with_resource("case-file-item.cmmn", model),
        )
        .expect("deployment");

    let case_instance = engine
        .start_case_instance_by_key("caseFileItemCase", CmmnCaseInstanceStartRequest::new())
        .expect("case instance");

    assert_eq!(case_instance.state, CmmnCaseInstanceState::Active);

    let active_tasks = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_instance.id)
        .state(CmmnHumanTaskState::Active)
        .list()
        .expect("active tasks");

    assert_eq!(active_tasks.len(), 1);
    assert_eq!(active_tasks[0].name, "Intake");

    let case_file_service = engine.runtime_service().case_file_item_service();
    let document = CmmnCaseFileItem::new("document", "Document");
    case_file_service
        .create_case_file_item(&case_instance.id, document)
        .expect("create case file item");

    let active_tasks = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_instance.id)
        .state(CmmnHumanTaskState::Active)
        .list()
        .expect("active tasks");

    assert_eq!(active_tasks.len(), 2);
    assert!(active_tasks.iter().any(|t| t.name == "Review"));
}

#[test]
fn case_file_item_on_part_only_triggers_matching_case_file_item_ref() {
    let engine = CmmnEngine::new_in_memory().expect("engine");

    let document_a_sentry = CmmnSentry::new(
        "sentry-document-a-create",
        CmmnPlanItemOnPart::new("on-task-complete-a", "plan-item-intake", "complete"),
    )
    .with_case_file_item_on_part(CmmnCaseFileItemOnPart::new(
        "on-document-a-create",
        "documentA",
        "create",
    ));
    let document_b_sentry = CmmnSentry::new(
        "sentry-document-b-create",
        CmmnPlanItemOnPart::new("on-task-complete-b", "plan-item-intake", "complete"),
    )
    .with_case_file_item_on_part(CmmnCaseFileItemOnPart::new(
        "on-document-b-create",
        "documentB",
        "create",
    ));

    let plan_model = CmmnCasePlanModel::new("case-plan-model", "Case plan model")
        .with_human_task(CmmnHumanTask::new("human-task-intake", "Intake"))
        .with_plan_item(CmmnPlanItem::new("plan-item-intake", "human-task-intake"))
        .with_human_task(CmmnHumanTask::new("human-task-review-a", "Review A"))
        .with_plan_item(
            CmmnPlanItem::new("plan-item-review-a", "human-task-review-a")
                .with_entry_criterion("sentry-document-a-create"),
        )
        .with_human_task(CmmnHumanTask::new("human-task-review-b", "Review B"))
        .with_plan_item(
            CmmnPlanItem::new("plan-item-review-b", "human-task-review-b")
                .with_entry_criterion("sentry-document-b-create"),
        )
        .with_sentry(document_a_sentry)
        .with_sentry(document_b_sentry);

    let model = CmmnModel::new(vec![CmmnCase::new(
        "case-file-item-ref",
        "caseFileItemRefCase",
        "Case file item ref case",
        plan_model,
    )]);

    engine
        .deploy(
            CmmnDeploymentRequest::new("case-file-item-ref")
                .with_resource("case-file-item-ref.cmmn", model),
        )
        .expect("deployment");

    let case_instance = engine
        .start_case_instance_by_key("caseFileItemRefCase", CmmnCaseInstanceStartRequest::new())
        .expect("case instance");

    let case_file_service = engine.runtime_service().case_file_item_service();
    case_file_service
        .create_case_file_item(
            &case_instance.id,
            CmmnCaseFileItem::new("documentA", "Document A"),
        )
        .expect("create document A");

    let active_tasks = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_instance.id)
        .state(CmmnHumanTaskState::Active)
        .list()
        .expect("active tasks");

    assert_eq!(active_tasks.len(), 2);
    assert!(active_tasks.iter().any(|t| t.name == "Intake"));
    assert!(active_tasks.iter().any(|t| t.name == "Review A"));
    assert!(!active_tasks.iter().any(|t| t.name == "Review B"));
}

#[test]
fn case_file_item_on_part_update_triggers_sentry() {
    let engine = CmmnEngine::new_in_memory().expect("engine");

    let sentry = CmmnSentry::new(
        "sentry-case-file-update",
        CmmnPlanItemOnPart::new("on-task-complete", "plan-item-intake", "complete"),
    )
    .with_case_file_item_on_part(CmmnCaseFileItemOnPart::new(
        "on-document-update",
        "document",
        "update",
    ));

    let plan_model = CmmnCasePlanModel::new("case-plan-model", "Case plan model")
        .with_human_task(CmmnHumanTask::new("human-task-intake", "Intake"))
        .with_plan_item(CmmnPlanItem::new("plan-item-intake", "human-task-intake"))
        .with_human_task(CmmnHumanTask::new("human-task-review", "Review"))
        .with_plan_item(
            CmmnPlanItem::new("plan-item-review", "human-task-review")
                .with_entry_criterion("sentry-case-file-update"),
        )
        .with_sentry(sentry);

    let model = CmmnModel::new(vec![CmmnCase::new(
        "case-file-update",
        "caseFileUpdateCase",
        "Case file update case",
        plan_model,
    )]);

    engine
        .deploy(
            CmmnDeploymentRequest::new("case-file-update")
                .with_resource("case-file-update.cmmn", model),
        )
        .expect("deployment");

    let case_instance = engine
        .start_case_instance_by_key("caseFileUpdateCase", CmmnCaseInstanceStartRequest::new())
        .expect("case instance");

    let case_file_service = engine.runtime_service().case_file_item_service();
    let document = CmmnCaseFileItem::new("document", "Document");
    case_file_service
        .create_case_file_item(&case_instance.id, document)
        .expect("create case file item");

    let active_tasks = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_instance.id)
        .state(CmmnHumanTaskState::Active)
        .list()
        .expect("active tasks");

    assert_eq!(active_tasks.len(), 1);
    assert_eq!(active_tasks[0].name, "Intake");

    case_file_service
        .update_case_file_item(&case_instance.id, "document", json!({"status": "updated"}))
        .expect("update case file item");

    let active_tasks = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_instance.id)
        .state(CmmnHumanTaskState::Active)
        .list()
        .expect("active tasks");

    assert_eq!(active_tasks.len(), 2);
    assert!(active_tasks.iter().any(|t| t.name == "Review"));
}

#[test]
fn case_file_item_on_part_delete_triggers_sentry() {
    let engine = CmmnEngine::new_in_memory().expect("engine");

    let sentry = CmmnSentry::new(
        "sentry-case-file-delete",
        CmmnPlanItemOnPart::new("on-task-complete", "plan-item-intake", "complete"),
    )
    .with_case_file_item_on_part(CmmnCaseFileItemOnPart::new(
        "on-document-delete",
        "document",
        "delete",
    ));

    let plan_model = CmmnCasePlanModel::new("case-plan-model", "Case plan model")
        .with_human_task(CmmnHumanTask::new("human-task-intake", "Intake"))
        .with_plan_item(CmmnPlanItem::new("plan-item-intake", "human-task-intake"))
        .with_human_task(CmmnHumanTask::new("human-task-archive", "Archive"))
        .with_plan_item(
            CmmnPlanItem::new("plan-item-archive", "human-task-archive")
                .with_entry_criterion("sentry-case-file-delete"),
        )
        .with_sentry(sentry);

    let model = CmmnModel::new(vec![CmmnCase::new(
        "case-file-delete",
        "caseFileDeleteCase",
        "Case file delete case",
        plan_model,
    )]);

    engine
        .deploy(
            CmmnDeploymentRequest::new("case-file-delete")
                .with_resource("case-file-delete.cmmn", model),
        )
        .expect("deployment");

    let case_instance = engine
        .start_case_instance_by_key("caseFileDeleteCase", CmmnCaseInstanceStartRequest::new())
        .expect("case instance");

    let case_file_service = engine.runtime_service().case_file_item_service();
    let document = CmmnCaseFileItem::new("document", "Document");
    case_file_service
        .create_case_file_item(&case_instance.id, document)
        .expect("create case file item");

    let active_tasks = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_instance.id)
        .state(CmmnHumanTaskState::Active)
        .list()
        .expect("active tasks");

    assert_eq!(active_tasks.len(), 1);
    assert_eq!(active_tasks[0].name, "Intake");

    case_file_service
        .delete_case_file_item(&case_instance.id, "document")
        .expect("delete case file item");

    let active_tasks = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_instance.id)
        .state(CmmnHumanTaskState::Active)
        .list()
        .expect("active tasks");

    assert_eq!(active_tasks.len(), 2);
    assert!(active_tasks.iter().any(|t| t.name == "Archive"));
}

#[test]
fn case_file_item_on_part_complete_triggers_sentry() {
    let engine = CmmnEngine::new_in_memory().expect("engine");

    let sentry = CmmnSentry::new(
        "sentry-case-file-complete",
        CmmnPlanItemOnPart::new("on-task-complete", "plan-item-intake", "complete"),
    )
    .with_case_file_item_on_part(CmmnCaseFileItemOnPart::new(
        "on-document-complete",
        "document",
        "complete",
    ));

    let plan_model = CmmnCasePlanModel::new("case-plan-model", "Case plan model")
        .with_human_task(CmmnHumanTask::new("human-task-intake", "Intake"))
        .with_plan_item(CmmnPlanItem::new("plan-item-intake", "human-task-intake"))
        .with_human_task(CmmnHumanTask::new("human-task-close", "Close"))
        .with_plan_item(
            CmmnPlanItem::new("plan-item-close", "human-task-close")
                .with_entry_criterion("sentry-case-file-complete"),
        )
        .with_sentry(sentry);

    let model = CmmnModel::new(vec![CmmnCase::new(
        "case-file-complete",
        "caseFileCompleteCase",
        "Case file complete case",
        plan_model,
    )]);

    engine
        .deploy(
            CmmnDeploymentRequest::new("case-file-complete")
                .with_resource("case-file-complete.cmmn", model),
        )
        .expect("deployment");

    let case_instance = engine
        .start_case_instance_by_key("caseFileCompleteCase", CmmnCaseInstanceStartRequest::new())
        .expect("case instance");

    let case_file_service = engine.runtime_service().case_file_item_service();
    let document = CmmnCaseFileItem::new("document", "Document");
    case_file_service
        .create_case_file_item(&case_instance.id, document)
        .expect("create case file item");

    let active_tasks = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_instance.id)
        .state(CmmnHumanTaskState::Active)
        .list()
        .expect("active tasks");

    assert_eq!(active_tasks.len(), 1);
    assert_eq!(active_tasks[0].name, "Intake");

    case_file_service
        .complete_case_file_item(&case_instance.id, "document")
        .expect("complete case file item");

    let active_tasks = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_instance.id)
        .state(CmmnHumanTaskState::Active)
        .list()
        .expect("active tasks");

    assert_eq!(active_tasks.len(), 2);
    assert!(active_tasks.iter().any(|t| t.name == "Close"));
}

#[test]
fn case_file_item_not_found_returns_error() {
    let engine = CmmnEngine::new_in_memory().expect("engine");

    let plan_model = CmmnCasePlanModel::new("case-plan-model", "Case plan model")
        .with_human_task(CmmnHumanTask::new("human-task-intake", "Intake"))
        .with_plan_item(CmmnPlanItem::new("plan-item-intake", "human-task-intake"));

    let model = CmmnModel::new(vec![CmmnCase::new(
        "case-file-not-found",
        "caseFileNotFoundCase",
        "Case file not found case",
        plan_model,
    )]);

    engine
        .deploy(
            CmmnDeploymentRequest::new("case-file-not-found")
                .with_resource("case-file-not-found.cmmn", model),
        )
        .expect("deployment");

    let case_instance = engine
        .start_case_instance_by_key("caseFileNotFoundCase", CmmnCaseInstanceStartRequest::new())
        .expect("case instance");

    let case_file_service = engine.runtime_service().case_file_item_service();
    let result = case_file_service.get_case_file_item(&case_instance.id, "nonexistent");
    assert!(result.is_err());
}

#[test]
fn case_file_item_delete_removed_item_returns_error() {
    let engine = CmmnEngine::new_in_memory().expect("engine");

    let plan_model = CmmnCasePlanModel::new("case-plan-model", "Case plan model")
        .with_human_task(CmmnHumanTask::new("human-task-intake", "Intake"))
        .with_plan_item(CmmnPlanItem::new("plan-item-intake", "human-task-intake"));

    let model = CmmnModel::new(vec![CmmnCase::new(
        "case-file-removed",
        "caseFileRemovedCase",
        "Case file removed case",
        plan_model,
    )]);

    engine
        .deploy(
            CmmnDeploymentRequest::new("case-file-removed")
                .with_resource("case-file-removed.cmmn", model),
        )
        .expect("deployment");

    let case_instance = engine
        .start_case_instance_by_key("caseFileRemovedCase", CmmnCaseInstanceStartRequest::new())
        .expect("case instance");

    let case_file_service = engine.runtime_service().case_file_item_service();
    let document = CmmnCaseFileItem::new("document", "Document");
    case_file_service
        .create_case_file_item(&case_instance.id, document)
        .expect("create case file item");

    case_file_service
        .delete_case_file_item(&case_instance.id, "document")
        .expect("delete case file item");

    let result = case_file_service.complete_case_file_item(&case_instance.id, "document");
    assert!(result.is_err());
}

#[test]
fn discretionary_item_creation() {
    let item = CmmnDiscretionaryItem::new("di-1", "Review Document", "human-task-review")
        .with_required(false)
        .with_manual_activation(true)
        .with_parent_stage_id("stage-review");

    assert_eq!(item.id, "di-1");
    assert_eq!(item.name, "Review Document");
    assert_eq!(item.definition_ref, "human-task-review");
    assert!(!item.required);
    assert!(item.manual_activation);
    assert_eq!(item.parent_stage_id, Some("stage-review".to_string()));
}

#[test]
fn discretionary_item_with_planning_table() {
    let item = CmmnDiscretionaryItem::new("di-2", "Approve", "human-task-approve")
        .with_planning_table("planning-table-1")
        .with_required(true);

    assert_eq!(item.planning_table, Some("planning-table-1".to_string()));
    assert!(item.required);
}

#[test]
fn plan_fragment_creation() {
    let fragment = CmmnPlanFragment::new("pf-1", "Review Fragment")
        .with_plan_item(CmmnPlanItem::new("plan-item-review", "human-task-review"))
        .with_human_task(CmmnHumanTask::new("human-task-review", "Review"))
        .with_sentry(CmmnSentry::new(
            "sentry-review-complete",
            CmmnPlanItemOnPart::new("on-review-complete", "plan-item-review", "complete"),
        ));

    assert_eq!(fragment.id, "pf-1");
    assert_eq!(fragment.name, "Review Fragment");
    assert_eq!(fragment.plan_items.len(), 1);
    assert_eq!(fragment.human_tasks.len(), 1);
    assert_eq!(fragment.sentries.len(), 1);
}

#[test]
fn plan_fragment_find_sentry() {
    let fragment = CmmnPlanFragment::new("pf-1", "Review Fragment")
        .with_sentry(CmmnSentry::new(
            "sentry-1",
            CmmnPlanItemOnPart::new("on-1", "plan-item-1", "complete"),
        ))
        .with_sentry(CmmnSentry::new(
            "sentry-2",
            CmmnPlanItemOnPart::new("on-2", "plan-item-2", "complete"),
        ));

    assert!(fragment.find_sentry("sentry-1").is_some());
    assert!(fragment.find_sentry("sentry-2").is_some());
    assert!(fragment.find_sentry("sentry-3").is_none());
}

#[test]
fn plan_fragment_find_plan_item() {
    let fragment = CmmnPlanFragment::new("pf-1", "Review Fragment")
        .with_plan_item(CmmnPlanItem::new("plan-item-1", "human-task-1"))
        .with_plan_item(CmmnPlanItem::new("plan-item-2", "human-task-2"));

    assert!(fragment.find_plan_item("plan-item-1").is_some());
    assert!(fragment.find_plan_item("plan-item-2").is_some());
    assert!(fragment.find_plan_item("plan-item-3").is_none());
}

#[test]
fn plan_fragment_with_multiple_element_types() {
    let fragment = CmmnPlanFragment::new("pf-complex", "Complex Fragment")
        .with_plan_item(CmmnPlanItem::new("plan-item-1", "human-task-1"))
        .with_plan_item(CmmnPlanItem::new("plan-item-2", "decision-task-1"))
        .with_human_task(CmmnHumanTask::new("human-task-1", "Human Task 1"))
        .with_decision_task(flowable_cmmn_engine::CmmnDecisionTask::new(
            "decision-task-1",
            "Decision Task 1",
        ))
        .with_milestone(flowable_cmmn_engine::CmmnMilestone::new(
            "milestone-1",
            "Milestone 1",
        ))
        .with_event_listener(flowable_cmmn_engine::CmmnEventListener::new(
            "listener-1",
            "message",
        ));

    assert_eq!(fragment.plan_items.len(), 2);
    assert_eq!(fragment.human_tasks.len(), 1);
    assert_eq!(fragment.decision_tasks.len(), 1);
    assert_eq!(fragment.milestones.len(), 1);
    assert_eq!(fragment.event_listeners.len(), 1);
}

#[test]
fn feel_engine_modulo_operator() {
    let engine = CmmnEngine::new_in_memory().expect("engine");

    let sentry = CmmnSentry::new(
        "sentry-modulo",
        CmmnPlanItemOnPart::new("on-task-complete", "plan-item-intake", "complete"),
    )
    .with_if_part("amount == 10");

    let plan_model = CmmnCasePlanModel::new("case-plan-model", "Case plan model")
        .with_human_task(CmmnHumanTask::new("human-task-intake", "Intake"))
        .with_plan_item(CmmnPlanItem::new("plan-item-intake", "human-task-intake"))
        .with_human_task(CmmnHumanTask::new("human-task-even", "Even Amount"))
        .with_plan_item(
            CmmnPlanItem::new("plan-item-even", "human-task-even")
                .with_entry_criterion("sentry-modulo"),
        )
        .with_sentry(sentry);

    let model = CmmnModel::new(vec![CmmnCase::new(
        "case-modulo",
        "moduloCase",
        "Modulo case",
        plan_model,
    )]);

    engine
        .deploy(CmmnDeploymentRequest::new("modulo").with_resource("modulo-case.cmmn", model))
        .expect("deployment");

    let case_instance = engine
        .start_case_instance_by_key(
            "moduloCase",
            CmmnCaseInstanceStartRequest::new().with_variables(json!({ "amount": 10 })),
        )
        .expect("case instance");

    // 初始状态：只有 Intake 任务处于活动状态
    let initial_tasks = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_instance.id)
        .state(CmmnHumanTaskState::Active)
        .list()
        .expect("active tasks");
    assert_eq!(initial_tasks.len(), 1);
    assert_eq!(initial_tasks[0].name, "Intake");

    // 完成 Intake 任务，触发 sentry 条件评估
    let intake_task = initial_tasks[0].clone();
    engine
        .runtime_service()
        .complete_human_task(&intake_task.id, CmmnHumanTaskCompletionRequest::new())
        .expect("complete intake task");

    // sentry 条件 amount == 10 为 true，Even Amount 任务应被激活
    let active_tasks = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_instance.id)
        .state(CmmnHumanTaskState::Active)
        .list()
        .expect("active tasks");

    assert_eq!(active_tasks.len(), 1);
    assert!(active_tasks.iter().any(|t| t.name == "Even Amount"));
}

#[test]
fn feel_engine_arithmetic_expressions() {
    let engine = CmmnEngine::new_in_memory().expect("engine");

    let sentry = CmmnSentry::new(
        "sentry-arithmetic",
        CmmnPlanItemOnPart::new("on-task-complete", "plan-item-intake", "complete"),
    )
    .with_if_part("price > 100");

    let plan_model = CmmnCasePlanModel::new("case-plan-model", "Case plan model")
        .with_human_task(CmmnHumanTask::new("human-task-intake", "Intake"))
        .with_plan_item(CmmnPlanItem::new("plan-item-intake", "human-task-intake"))
        .with_human_task(CmmnHumanTask::new("human-task-discount", "Apply Discount"))
        .with_plan_item(
            CmmnPlanItem::new("plan-item-discount", "human-task-discount")
                .with_entry_criterion("sentry-arithmetic"),
        )
        .with_sentry(sentry);

    let model = CmmnModel::new(vec![CmmnCase::new(
        "case-arithmetic",
        "arithmeticCase",
        "Arithmetic case",
        plan_model,
    )]);

    engine
        .deploy(
            CmmnDeploymentRequest::new("arithmetic").with_resource("arithmetic-case.cmmn", model),
        )
        .expect("deployment");

    let case_instance = engine
        .start_case_instance_by_key(
            "arithmeticCase",
            CmmnCaseInstanceStartRequest::new()
                .with_variables(json!({ "price": 150, "quantity": 3 })),
        )
        .expect("case instance");

    // 初始状态：只有 Intake 任务处于活动状态
    let initial_tasks = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_instance.id)
        .state(CmmnHumanTaskState::Active)
        .list()
        .expect("active tasks");
    assert_eq!(initial_tasks.len(), 1);
    assert_eq!(initial_tasks[0].name, "Intake");

    // 完成 Intake 任务，触发 sentry 条件评估
    let intake_task = initial_tasks[0].clone();
    engine
        .runtime_service()
        .complete_human_task(&intake_task.id, CmmnHumanTaskCompletionRequest::new())
        .expect("complete intake task");

    // sentry 条件 price > 100 为 true (150 > 100)，Apply Discount 任务应被激活
    let active_tasks = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_instance.id)
        .state(CmmnHumanTaskState::Active)
        .list()
        .expect("active tasks");

    assert_eq!(active_tasks.len(), 1);
    assert!(active_tasks.iter().any(|t| t.name == "Apply Discount"));
}

#[test]
fn feel_engine_comparison_operators() {
    let engine = CmmnEngine::new_in_memory().expect("engine");

    let sentry = CmmnSentry::new(
        "sentry-comparison",
        CmmnPlanItemOnPart::new("on-task-complete", "plan-item-intake", "complete"),
    )
    .with_if_part("age >= 18");

    let plan_model = CmmnCasePlanModel::new("case-plan-model", "Case plan model")
        .with_human_task(CmmnHumanTask::new("human-task-intake", "Intake"))
        .with_plan_item(CmmnPlanItem::new("plan-item-intake", "human-task-intake"))
        .with_human_task(CmmnHumanTask::new("human-task-adult", "Adult Task"))
        .with_plan_item(
            CmmnPlanItem::new("plan-item-adult", "human-task-adult")
                .with_entry_criterion("sentry-comparison"),
        )
        .with_sentry(sentry);

    let model = CmmnModel::new(vec![CmmnCase::new(
        "case-comparison",
        "comparisonCase",
        "Comparison case",
        plan_model,
    )]);

    engine
        .deploy(
            CmmnDeploymentRequest::new("comparison").with_resource("comparison-case.cmmn", model),
        )
        .expect("deployment");

    let case_instance = engine
        .start_case_instance_by_key(
            "comparisonCase",
            CmmnCaseInstanceStartRequest::new().with_variables(json!({ "age": 25 })),
        )
        .expect("case instance");

    // 初始状态：只有 Intake 任务处于活动状态
    let initial_tasks = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_instance.id)
        .state(CmmnHumanTaskState::Active)
        .list()
        .expect("active tasks");
    assert_eq!(initial_tasks.len(), 1);
    assert_eq!(initial_tasks[0].name, "Intake");

    // 完成 Intake 任务，触发 sentry 条件评估
    let intake_task = initial_tasks[0].clone();
    engine
        .runtime_service()
        .complete_human_task(&intake_task.id, CmmnHumanTaskCompletionRequest::new())
        .expect("complete intake task");

    // sentry 条件 age >= 18 为 true (25 >= 18)，Adult Task 任务应被激活
    let active_tasks = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_instance.id)
        .state(CmmnHumanTaskState::Active)
        .list()
        .expect("active tasks");

    assert_eq!(active_tasks.len(), 1);
    assert!(active_tasks.iter().any(|t| t.name == "Adult Task"));
}

#[test]
fn feel_engine_string_functions() {
    let engine = CmmnEngine::new_in_memory().expect("engine");

    let sentry = CmmnSentry::new(
        "sentry-string",
        CmmnPlanItemOnPart::new("on-task-complete", "plan-item-intake", "complete"),
    )
    .with_if_part("name == 'Alice'");

    let plan_model = CmmnCasePlanModel::new("case-plan-model", "Case plan model")
        .with_human_task(CmmnHumanTask::new("human-task-intake", "Intake"))
        .with_plan_item(CmmnPlanItem::new("plan-item-intake", "human-task-intake"))
        .with_human_task(CmmnHumanTask::new("human-task-a-name", "A Name Task"))
        .with_plan_item(
            CmmnPlanItem::new("plan-item-a-name", "human-task-a-name")
                .with_entry_criterion("sentry-string"),
        )
        .with_sentry(sentry);

    let model = CmmnModel::new(vec![CmmnCase::new(
        "case-string",
        "stringCase",
        "String case",
        plan_model,
    )]);

    engine
        .deploy(CmmnDeploymentRequest::new("string").with_resource("string-case.cmmn", model))
        .expect("deployment");

    let case_instance = engine
        .start_case_instance_by_key(
            "stringCase",
            CmmnCaseInstanceStartRequest::new().with_variables(json!({ "name": "Alice" })),
        )
        .expect("case instance");

    // 初始状态：只有 Intake 任务处于活动状态
    let initial_tasks = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_instance.id)
        .state(CmmnHumanTaskState::Active)
        .list()
        .expect("active tasks");
    assert_eq!(initial_tasks.len(), 1);
    assert_eq!(initial_tasks[0].name, "Intake");

    // 完成 Intake 任务，触发 sentry 条件评估
    let intake_task = initial_tasks[0].clone();
    engine
        .runtime_service()
        .complete_human_task(&intake_task.id, CmmnHumanTaskCompletionRequest::new())
        .expect("complete intake task");

    // sentry 条件 name == 'Alice' 为 true，A Name Task 任务应被激活
    let active_tasks = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_instance.id)
        .state(CmmnHumanTaskState::Active)
        .list()
        .expect("active tasks");

    assert_eq!(active_tasks.len(), 1);
    assert!(active_tasks.iter().any(|t| t.name == "A Name Task"));
}

#[test]
fn feel_engine_logical_operators() {
    let engine = CmmnEngine::new_in_memory().expect("engine");

    let sentry = CmmnSentry::new(
        "sentry-logical",
        CmmnPlanItemOnPart::new("on-task-complete", "plan-item-intake", "complete"),
    )
    .with_if_part("approved == true && priority == 'high'");

    let plan_model = CmmnCasePlanModel::new("case-plan-model", "Case plan model")
        .with_human_task(CmmnHumanTask::new("human-task-intake", "Intake"))
        .with_plan_item(CmmnPlanItem::new("plan-item-intake", "human-task-intake"))
        .with_human_task(CmmnHumanTask::new("human-task-process", "Process"))
        .with_plan_item(
            CmmnPlanItem::new("plan-item-process", "human-task-process")
                .with_entry_criterion("sentry-logical"),
        )
        .with_sentry(sentry);

    let model = CmmnModel::new(vec![CmmnCase::new(
        "case-logical",
        "logicalCase",
        "Logical case",
        plan_model,
    )]);

    engine
        .deploy(CmmnDeploymentRequest::new("logical").with_resource("logical-case.cmmn", model))
        .expect("deployment");

    let case_instance = engine
        .start_case_instance_by_key(
            "logicalCase",
            CmmnCaseInstanceStartRequest::new()
                .with_variables(json!({ "approved": true, "priority": "high", "amount": 100 })),
        )
        .expect("case instance");

    // 初始状态：只有 Intake 任务处于活动状态
    let initial_tasks = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_instance.id)
        .state(CmmnHumanTaskState::Active)
        .list()
        .expect("active tasks");
    assert_eq!(initial_tasks.len(), 1);
    assert_eq!(initial_tasks[0].name, "Intake");

    // 完成 Intake 任务，触发 sentry 条件评估
    let intake_task = initial_tasks[0].clone();
    engine
        .runtime_service()
        .complete_human_task(&intake_task.id, CmmnHumanTaskCompletionRequest::new())
        .expect("complete intake task");

    // sentry 条件 approved == true && priority == 'high' 为 true，Process 任务应被激活
    let active_tasks = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_instance.id)
        .state(CmmnHumanTaskState::Active)
        .list()
        .expect("active tasks");

    assert_eq!(active_tasks.len(), 1);
    assert!(active_tasks.iter().any(|t| t.name == "Process"));
}

#[test]
fn case_file_item_on_part_standard_events() {
    assert!(CmmnCaseFileItemOnPart::is_supported_standard_event(
        "create"
    ));
    assert!(CmmnCaseFileItemOnPart::is_supported_standard_event(
        "update"
    ));
    assert!(CmmnCaseFileItemOnPart::is_supported_standard_event(
        "delete"
    ));
    assert!(CmmnCaseFileItemOnPart::is_supported_standard_event(
        "complete"
    ));
    assert!(!CmmnCaseFileItemOnPart::is_supported_standard_event(
        "unknown"
    ));
}

#[test]
fn case_file_item_state_serialization() {
    let available = CmmnCaseFileItemState::Available;
    let removed = CmmnCaseFileItemState::Removed;

    assert_eq!(available.as_str(), "AVAILABLE");
    assert_eq!(removed.as_str(), "REMOVED");
}

#[test]
fn case_file_item_creation_with_value() {
    let item = CmmnCaseFileItem::new("doc-1", "Document")
        .with_value(json!({"title": "Test", "status": "draft"}));

    assert_eq!(item.id, "doc-1");
    assert_eq!(item.name, "Document");
    assert!(item.value.is_some());
    assert_eq!(item.state, CmmnCaseFileItemState::Available);
}
