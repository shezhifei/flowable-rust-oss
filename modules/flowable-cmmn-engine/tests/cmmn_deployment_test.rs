use flowable_cmmn_engine::{
    CmmnCase, CmmnCasePlanModel, CmmnCaseTask, CmmnDecisionTask, CmmnDeploymentRequest,
    CmmnDiscretionaryItem, CmmnEngine, CmmnEventListener, CmmnHumanTask,
    CmmnHumanTaskCompletionRequest, CmmnHumanTaskState, CmmnMilestone, CmmnModel, CmmnPlanItem,
    CmmnPlanItemOnPart, CmmnPlanningTable, CmmnProcessTask, CmmnSentry, CmmnSentryIfPartCondition,
    CmmnSentryIfPartExpression, CmmnSentryIfPartLiteral, CmmnSentryIfPartOperator, CmmnStage,
};
use uuid::Uuid;

fn simple_case_model(case_key: &str) -> CmmnModel {
    let plan_model = CmmnCasePlanModel::new("case-plan-model", "Case plan model")
        .with_human_task(CmmnHumanTask::new("human-task-review", "Review"))
        .with_plan_item(CmmnPlanItem::new("plan-item-review", "human-task-review"));

    CmmnModel::new(vec![CmmnCase::new(
        format!("case-{case_key}"),
        case_key,
        "Review case",
        plan_model,
    )])
}

fn event_listener_occur_entry_criterion_model() -> CmmnModel {
    let plan_model = CmmnCasePlanModel::new("case-plan-model", "Case plan model")
        .with_event_listener(
            CmmnEventListener::new("approval-event-listener", "message")
                .with_name("Wait for approval")
                .with_event_name("approvalReceived"),
        )
        .with_human_task(CmmnHumanTask::new("human-task-approval", "Approve"))
        .with_plan_item(CmmnPlanItem::new(
            "plan-item-approval-event",
            "approval-event-listener",
        ))
        .with_plan_item(
            CmmnPlanItem::new("plan-item-approval-task", "human-task-approval")
                .with_entry_criterion("sentry-after-approval-event"),
        )
        .with_sentry(CmmnSentry::new(
            "sentry-after-approval-event",
            CmmnPlanItemOnPart::new(
                "on-approval-event-occur",
                "plan-item-approval-event",
                "occur",
            ),
        ));

    CmmnModel::new(vec![CmmnCase::new(
        "case-event-listener-occur-entry-criterion",
        "eventListenerOccurEntryCriterionCase",
        "Event listener occur entry criterion case",
        plan_model,
    )])
}

fn shared_model_with_plan_item_control_rules() -> flowable_cmmn_model::CmmnDefinitions {
    flowable_cmmn_model::CmmnDefinitions {
        id: None,
        name: None,
        target_namespace: Some("http://flowable.org/cmmn".to_string()),
        expression_language: None,
        type_language: None,
        exporter: None,
        exporter_version: None,
        namespaces: Default::default(),
        cases: vec![flowable_cmmn_model::Case {
            lifecycle_listeners: Vec::new(),
            start_event_type: None,
            start_correlation_configuration: None,
            start_correlation_parameters: Vec::new(),
            id: "caseA".to_string(),
            name: Some("Case A".to_string()),
            case_plan_model: flowable_cmmn_model::CasePlanModel {
                lifecycle_listeners: Vec::new(),
                id: "planModelA".to_string(),
                name: Some("Plan Model A".to_string()),
                auto_complete: false,
                form_key: None,
                plan_items: vec![flowable_cmmn_model::PlanItem {
                    id: "planItemReview".to_string(),
                    name: Some("Review".to_string()),
                    definition_ref: "reviewTask".to_string(),
                    entry_criteria: Vec::new(),
                    exit_criteria: Vec::new(),
                    manual_activation_rule: Some(
                        flowable_cmmn_model::parse_sentry_if_part_expression(
                            "manualActivation == true",
                        )
                        .expect("manual activation rule expression"),
                    ),
                    repetition_rule: Some(
                        flowable_cmmn_model::parse_sentry_if_part_expression(
                            "repeatReview == true",
                        )
                        .expect("repetition rule expression"),
                    ),
                    required_rule: None,
                    parent_completion_rule: None,
                    completion_neutral_rule: None,
                }],
                human_tasks: vec![flowable_cmmn_model::HumanTask {
                    id: "reviewTask".to_string(),
                    name: Some("Review Task".to_string()),
                    is_blocking: true,
                    form_key: None,
                    ..Default::default()
                }],
                decision_tasks: Vec::new(),
                process_tasks: Vec::new(),
                case_tasks: Vec::new(),
                milestones: Vec::new(),
                event_listeners: Vec::new(),
                sentries: Vec::new(),
                planning_tables: Vec::new(),
                stages: Vec::new(),
            },
        }],
    }
}

fn shared_model_with_exit_criterion() -> flowable_cmmn_model::CmmnDefinitions {
    flowable_cmmn_model::CmmnDefinitions {
        id: None,
        name: None,
        target_namespace: Some("http://flowable.org/cmmn".to_string()),
        expression_language: None,
        type_language: None,
        exporter: None,
        exporter_version: None,
        namespaces: Default::default(),
        cases: vec![flowable_cmmn_model::Case {
            lifecycle_listeners: Vec::new(),
            start_event_type: None,
            start_correlation_configuration: None,
            start_correlation_parameters: Vec::new(),
            id: "xmlExitCriterionCase".to_string(),
            name: Some("XML exit criterion case".to_string()),
            case_plan_model: flowable_cmmn_model::CasePlanModel {
                lifecycle_listeners: Vec::new(),
                id: "planModelA".to_string(),
                name: Some("Plan Model A".to_string()),
                auto_complete: false,
                form_key: None,
                plan_items: vec![
                    flowable_cmmn_model::PlanItem {
                        id: "planItemA".to_string(),
                        name: Some("Task A".to_string()),
                        definition_ref: "taskA".to_string(),
                        entry_criteria: Vec::new(),
                        exit_criteria: Vec::new(),
                        manual_activation_rule: None,
                        repetition_rule: None,
                        required_rule: None,
                        parent_completion_rule: None,
                        completion_neutral_rule: None,
                    },
                    flowable_cmmn_model::PlanItem {
                        id: "planItemB".to_string(),
                        name: Some("Task B".to_string()),
                        definition_ref: "taskB".to_string(),
                        entry_criteria: Vec::new(),
                        exit_criteria: vec![flowable_cmmn_model::EntryCriterion {
                            id: "exitCriterionB".to_string(),
                            sentry_ref: "sentryExitB".to_string(),
                        }],
                        manual_activation_rule: None,
                        repetition_rule: None,
                        required_rule: None,
                        parent_completion_rule: None,
                        completion_neutral_rule: None,
                    },
                    flowable_cmmn_model::PlanItem {
                        id: "planItemC".to_string(),
                        name: Some("Task C".to_string()),
                        definition_ref: "taskC".to_string(),
                        entry_criteria: vec![flowable_cmmn_model::EntryCriterion {
                            id: "entryCriterionC".to_string(),
                            sentry_ref: "sentryEntryC".to_string(),
                        }],
                        exit_criteria: Vec::new(),
                        manual_activation_rule: None,
                        repetition_rule: None,
                        required_rule: None,
                        parent_completion_rule: None,
                        completion_neutral_rule: None,
                    },
                ],
                human_tasks: vec![
                    flowable_cmmn_model::HumanTask {
                        id: "taskA".to_string(),
                        name: Some("Task A".to_string()),
                        is_blocking: true,
                        form_key: None,
                        ..Default::default()
                    },
                    flowable_cmmn_model::HumanTask {
                        id: "taskB".to_string(),
                        name: Some("Task B".to_string()),
                        is_blocking: true,
                        form_key: None,
                        ..Default::default()
                    },
                    flowable_cmmn_model::HumanTask {
                        id: "taskC".to_string(),
                        name: Some("Task C".to_string()),
                        is_blocking: true,
                        form_key: None,
                        ..Default::default()
                    },
                ],
                decision_tasks: Vec::new(),
                process_tasks: Vec::new(),
                case_tasks: Vec::new(),
                milestones: Vec::new(),
                event_listeners: Vec::new(),
                sentries: vec![
                    flowable_cmmn_model::Sentry {
                        id: "sentryExitB".to_string(),
                        plan_item_on_parts: vec![flowable_cmmn_model::PlanItemOnPart {
                            id: "onTaskACompleteExitB".to_string(),
                            source_ref: "planItemA".to_string(),
                            standard_event: "complete".to_string(),
                        }],
                        case_file_item_on_parts: Vec::new(),
                        if_part: None,
                    },
                    flowable_cmmn_model::Sentry {
                        id: "sentryEntryC".to_string(),
                        plan_item_on_parts: vec![flowable_cmmn_model::PlanItemOnPart {
                            id: "onTaskACompleteEntryC".to_string(),
                            source_ref: "planItemA".to_string(),
                            standard_event: "complete".to_string(),
                        }],
                        case_file_item_on_parts: Vec::new(),
                        if_part: None,
                    },
                ],
                planning_tables: Vec::new(),
                stages: Vec::new(),
            },
        }],
    }
}

fn plan_item_with_control_rules(id: &str, definition_ref: &str) -> CmmnPlanItem {
    CmmnPlanItem::new(id, definition_ref)
        .with_manual_activation_rule("manualActivation == true")
        .with_repetition_rule("repeatItem == true")
}

fn plan_item_with_manual_activation_rule(id: &str, definition_ref: &str) -> CmmnPlanItem {
    CmmnPlanItem::new(id, definition_ref).with_manual_activation_rule("manualActivation == true")
}

fn decision_task_manual_activation_model() -> CmmnModel {
    let plan_model = CmmnCasePlanModel::new("case-plan-model", "Case plan model")
        .with_decision_task(
            CmmnDecisionTask::new("approval-decision", "Approval decision")
                .with_decision_ref("approvalDecision"),
        )
        .with_plan_item(plan_item_with_manual_activation_rule(
            "plan-item-approval-decision",
            "approval-decision",
        ));

    CmmnModel::new(vec![CmmnCase::new(
        "case-decision-task-manual-activation",
        "decisionTaskManualActivationCase",
        "Decision task manual activation case",
        plan_model,
    )])
}

fn non_human_plan_item_control_model(definition_type: &str) -> CmmnModel {
    let plan_model = match definition_type {
        "stage" => CmmnCasePlanModel::new("case-plan-model", "Case plan model")
            .with_stage(
                CmmnStage::new("review-stage", "Review stage")
                    .with_human_task(CmmnHumanTask::new("nested-task", "Nested task"))
                    .with_plan_item(CmmnPlanItem::new("plan-item-nested-task", "nested-task")),
            )
            .with_plan_item(plan_item_with_control_rules(
                "plan-item-review-stage",
                "review-stage",
            )),
        "milestone" => CmmnCasePlanModel::new("case-plan-model", "Case plan model")
            .with_milestone(CmmnMilestone::new(
                "approval-milestone",
                "Approval milestone",
            ))
            .with_plan_item(plan_item_with_manual_activation_rule(
                "plan-item-approval-milestone",
                "approval-milestone",
            )),
        "eventListener" => CmmnCasePlanModel::new("case-plan-model", "Case plan model")
            .with_event_listener(
                CmmnEventListener::new("approval-event-listener", "message")
                    .with_name("Wait for approval")
                    .with_event_name("approvalReceived"),
            )
            .with_plan_item(plan_item_with_manual_activation_rule(
                "plan-item-approval-event",
                "approval-event-listener",
            )),
        "decisionTask" => CmmnCasePlanModel::new("case-plan-model", "Case plan model")
            .with_decision_task(
                CmmnDecisionTask::new("approval-decision", "Approval decision")
                    .with_decision_ref("approvalDecision"),
            )
            .with_plan_item(plan_item_with_control_rules(
                "plan-item-approval-decision",
                "approval-decision",
            )),
        other => panic!("unsupported test definition type: {other}"),
    };

    CmmnModel::new(vec![CmmnCase::new(
        format!("case-{definition_type}-plan-item-control"),
        format!("{definition_type}PlanItemControlCase"),
        "Plan item control case",
        plan_model,
    )])
}

fn unsupported_non_human_repetition_rule_model(definition_type: &str) -> CmmnModel {
    let plan_item = |id: &str, definition_ref: &str| {
        CmmnPlanItem::new(id, definition_ref).with_repetition_rule("repeatItem == true")
    };
    let plan_model = match definition_type {
        "milestone" => CmmnCasePlanModel::new("case-plan-model", "Case plan model")
            .with_milestone(CmmnMilestone::new(
                "approval-milestone",
                "Approval milestone",
            ))
            .with_plan_item(plan_item(
                "plan-item-approval-milestone",
                "approval-milestone",
            )),
        "eventListener" => CmmnCasePlanModel::new("case-plan-model", "Case plan model")
            .with_event_listener(
                CmmnEventListener::new("approval-event-listener", "message")
                    .with_name("Wait for approval")
                    .with_event_name("approvalReceived"),
            )
            .with_plan_item(plan_item(
                "plan-item-approval-event",
                "approval-event-listener",
            )),
        "decisionTask" => CmmnCasePlanModel::new("case-plan-model", "Case plan model")
            .with_decision_task(
                CmmnDecisionTask::new("approval-decision", "Approval decision")
                    .with_decision_ref("approvalDecision"),
            )
            .with_plan_item(plan_item(
                "plan-item-approval-decision",
                "approval-decision",
            )),
        other => panic!("unsupported test definition type: {other}"),
    };

    CmmnModel::new(vec![CmmnCase::new(
        format!("case-{definition_type}-unsupported-repetition"),
        format!("{definition_type}UnsupportedRepetitionCase"),
        "Unsupported repetition case",
        plan_model,
    )])
}

#[test]
fn deploys_models_queries_latest_definition_and_survives_sqlite_reopen() {
    let db_path = std::env::temp_dir().join(format!("flowable-cmmn-{}.sqlite", Uuid::new_v4()));

    let deployment_id = {
        let engine = CmmnEngine::new_sqlite(&db_path).expect("engine");
        let deployment = engine
            .deploy(
                CmmnDeploymentRequest::new("cmmn-deployment")
                    .with_resource("review-case.cmmn", simple_case_model("reviewCase")),
            )
            .expect("deployment");

        let definition = engine
            .repository_service()
            .create_case_definition_query()
            .key("reviewCase")
            .single_result()
            .expect("definition query")
            .expect("definition");

        assert_eq!(deployment.name.as_deref(), Some("cmmn-deployment"));
        assert_eq!(definition.version, 1);
        assert_eq!(definition.resource_name, "review-case.cmmn");

        deployment.id
    };

    let reopened_engine = CmmnEngine::new_sqlite(&db_path).expect("reopened engine");
    let reopened_deployment = reopened_engine
        .repository_service()
        .get_deployment(&deployment_id)
        .expect("deployment after reopen");
    let reopened_definition = reopened_engine
        .repository_service()
        .create_case_definition_query()
        .key("reviewCase")
        .single_result()
        .expect("definition query after reopen")
        .expect("definition after reopen");

    assert_eq!(reopened_deployment.id, deployment_id);
    assert_eq!(reopened_definition.key, "reviewCase");
    assert_eq!(reopened_definition.version, 1);

    let _ = std::fs::remove_file(db_path);
}

#[test]
fn deploy_preserves_plan_item_control_rules_from_shared_model() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let model = CmmnModel::from(shared_model_with_plan_item_control_rules());

    engine
        .deploy(
            CmmnDeploymentRequest::new("shared-model-rules")
                .with_resource("review-case.cmmn", model),
        )
        .expect("deployment");

    let definition = engine
        .repository_service()
        .create_case_definition_query()
        .key("caseA")
        .single_result()
        .expect("definition query")
        .expect("definition");
    let plan_item = definition
        .model
        .case_plan_model
        .plan_items
        .iter()
        .find(|candidate| candidate.id == "planItemReview")
        .expect("plan item");

    assert_eq!(
        plan_item.manual_activation_rule.as_ref(),
        Some(&CmmnSentryIfPartExpression::Comparison(
            CmmnSentryIfPartCondition {
                variable_name: "manualActivation".to_string(),
                operator: CmmnSentryIfPartOperator::Equal,
                literal: CmmnSentryIfPartLiteral::Boolean(true),
            }
        ))
    );
    assert_eq!(
        plan_item.repetition_rule.as_ref(),
        Some(&CmmnSentryIfPartExpression::Comparison(
            CmmnSentryIfPartCondition {
                variable_name: "repeatReview".to_string(),
                operator: CmmnSentryIfPartOperator::Equal,
                literal: CmmnSentryIfPartLiteral::Boolean(true),
            }
        ))
    );
}

#[test]
fn deploy_preserves_xml_exit_criterion_mapping_and_runtime_exit() {
    const EXIT_CRITERION_XML: &str = r#"
<definitions xmlns="http://www.omg.org/spec/CMMN/20151109/MODEL"
             targetNamespace="http://flowable.org/cmmn">
  <case id="xmlExitCriterionCase" name="XML exit criterion case">
    <casePlanModel id="planModelA" name="Plan Model A">
      <planItem id="planItemA" name="Task A" definitionRef="taskA" />
      <planItem id="planItemB" name="Task B" definitionRef="taskB">
        <exitCriterion id="exitCriterionB" sentryRef="sentryExitB" />
      </planItem>
      <planItem id="planItemC" name="Task C" definitionRef="taskC">
        <entryCriterion id="entryCriterionC" sentryRef="sentryEntryC" />
      </planItem>
      <humanTask id="taskA" name="Task A" />
      <humanTask id="taskB" name="Task B" />
      <humanTask id="taskC" name="Task C" />
      <sentry id="sentryExitB">
        <planItemOnPart id="onTaskACompleteExitB" sourceRef="planItemA">
          <standardEvent>complete</standardEvent>
        </planItemOnPart>
      </sentry>
      <sentry id="sentryEntryC">
        <planItemOnPart id="onTaskACompleteEntryC" sourceRef="planItemA">
          <standardEvent>complete</standardEvent>
        </planItemOnPart>
      </sentry>
    </casePlanModel>
  </case>
</definitions>
"#;

    let engine = CmmnEngine::new_in_memory().expect("engine");
    let model = CmmnModel::from(shared_model_with_exit_criterion());

    engine
        .deploy(
            CmmnDeploymentRequest::new("xml-exit-criterion").with_resource_bytes(
                "xml-exit-criterion.cmmn",
                model,
                EXIT_CRITERION_XML.as_bytes(),
            ),
        )
        .expect("deployment");

    let definition = engine
        .repository_service()
        .create_case_definition_query()
        .key("xmlExitCriterionCase")
        .single_result()
        .expect("definition query")
        .expect("definition");
    let plan_item_b = definition
        .model
        .case_plan_model
        .plan_items
        .iter()
        .find(|candidate| candidate.id == "planItemB")
        .expect("plan item B");
    assert_eq!(
        plan_item_b.exit_criterion_ids,
        vec!["sentryExitB".to_string()]
    );
    assert!(plan_item_b.entry_criterion_ids.is_empty());

    let case_instance = engine
        .start_case_instance_by_key("xmlExitCriterionCase", Default::default())
        .expect("case instance");
    let active_tasks = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_instance.id)
        .state(CmmnHumanTaskState::Active)
        .list()
        .expect("active tasks");
    assert_eq!(active_tasks.len(), 2);

    let task_a = active_tasks
        .iter()
        .find(|task| task.plan_item_id == "planItemA")
        .expect("task A");
    let task_b_id = active_tasks
        .iter()
        .find(|task| task.plan_item_id == "planItemB")
        .expect("task B")
        .id
        .clone();

    engine
        .complete_human_task(&task_a.id, CmmnHumanTaskCompletionRequest::new())
        .expect("complete task A");

    let active_after_completion = engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(&case_instance.id)
        .state(CmmnHumanTaskState::Active)
        .list()
        .expect("active tasks after task A completion");
    assert_eq!(active_after_completion.len(), 1);
    assert_eq!(active_after_completion[0].plan_item_id, "planItemC");
    assert!(
        active_after_completion
            .iter()
            .all(|task| task.plan_item_id != "planItemB")
    );
    assert!(
        engine.runtime_service().get_human_task(&task_b_id).is_err(),
        "task B should be removed from runtime after its exitCriterion is satisfied"
    );
}

#[test]
fn deploys_bounded_plan_item_control_rules_on_supported_non_human_plan_item_definitions() {
    // stage + decisionTask may include repetitionRule; milestone/eventListener use manual only.
    for definition_type in ["stage", "milestone", "eventListener", "decisionTask"] {
        let engine = CmmnEngine::new_in_memory().expect("engine");
        engine
            .deploy(
                CmmnDeploymentRequest::new(format!("{definition_type}-plan-item-control"))
                    .with_resource(
                        format!("{definition_type}-plan-item-control.cmmn"),
                        non_human_plan_item_control_model(definition_type),
                    ),
            )
            .expect("bounded non-human plan item control rules should deploy");
    }
}

#[test]
fn deploys_manual_activation_rule_on_decision_task_plan_item_definition() {
    let engine = CmmnEngine::new_in_memory().expect("engine");

    engine
        .deploy(
            CmmnDeploymentRequest::new("decision-task-manual-activation").with_resource(
                "decision-task-manual-activation.cmmn",
                decision_task_manual_activation_model(),
            ),
        )
        .expect("decision task manual activation rule should deploy");
}

#[test]
fn rejects_repetition_rule_on_milestone_and_event_listener_plan_items() {
    for definition_type in ["milestone", "eventListener"] {
        let engine = CmmnEngine::new_in_memory().expect("engine");
        let error = engine
            .deploy(
                CmmnDeploymentRequest::new(format!("{definition_type}-unsupported-repetition"))
                    .with_resource(
                        format!("{definition_type}-unsupported-repetition.cmmn"),
                        unsupported_non_human_repetition_rule_model(definition_type),
                    ),
            )
            .expect_err("unsupported non-human repetition rule must fail deployment");
        let message = error.to_string();

        assert!(
            message.contains("Unsupported CMMN plan item control")
                && message.contains(definition_type)
                && message.contains("repetitionRule")
                && message.contains("outside the supported bounded subset"),
            "unexpected error for {definition_type}: {message}"
        );
    }
}

#[test]
fn deploys_repetition_rule_on_decision_task_plan_item() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    engine
        .deploy(
            CmmnDeploymentRequest::new("decision-task-repetition").with_resource(
                "decision-task-repetition.cmmn",
                unsupported_non_human_repetition_rule_model("decisionTask"),
            ),
        )
        .expect("decisionTask repetitionRule should deploy in the bounded subset");
}

#[test]
fn increments_versions_across_redeployments_of_same_case_key() {
    let engine = CmmnEngine::new_in_memory().expect("engine");

    engine
        .deploy(
            CmmnDeploymentRequest::new("v1")
                .with_resource("review-case-v1.cmmn", simple_case_model("reviewCase")),
        )
        .expect("deployment v1");
    engine
        .deploy(
            CmmnDeploymentRequest::new("v2")
                .with_resource("review-case-v2.cmmn", simple_case_model("reviewCase")),
        )
        .expect("deployment v2");

    let definitions = engine
        .repository_service()
        .create_case_definition_query()
        .key("reviewCase")
        .list()
        .expect("definitions");

    assert_eq!(definitions.len(), 2);
    assert_eq!(definitions[0].version, 2);
    assert_eq!(definitions[1].version, 1);
}

#[test]
fn rejects_unsupported_entry_criteria_during_deployment() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let invalid_model = CmmnModel::new(vec![CmmnCase::new(
        "case-review",
        "reviewCase",
        "Review case",
        CmmnCasePlanModel::new("case-plan-model", "Case plan model")
            .with_human_task(CmmnHumanTask::new("human-task-review", "Review"))
            .with_plan_item(
                CmmnPlanItem::new("plan-item-review", "human-task-review")
                    .with_entry_criterion("criterion-1"),
            ),
    )]);

    let error = engine
        .deploy(
            CmmnDeploymentRequest::new("invalid").with_resource("review-case.cmmn", invalid_model),
        )
        .expect_err("deployment should fail");

    assert!(error.to_string().contains("entry criterion"));
}

#[test]
fn deploys_case_level_planning_table_with_discretionary_human_task() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let model = CmmnModel::new(vec![CmmnCase::new(
        "case-planning",
        "caseLevelPlanningCase",
        "Case level planning",
        CmmnCasePlanModel::new("case-plan-model", "Case plan model")
            .with_human_task(CmmnHumanTask::new("human-task-review", "Review"))
            .with_planning_table(
                CmmnPlanningTable::new("planning-table-root", "Root planning")
                    .with_discretionary_item(CmmnDiscretionaryItem::new(
                        "discretionary-review",
                        "Review",
                        "human-task-review",
                    )),
            ),
    )]);

    engine
        .deploy(
            CmmnDeploymentRequest::new("case-level-planning")
                .with_resource("case-level-planning.cmmn", model),
        )
        .expect("case-level planning table should deploy");

    let case_definition = engine
        .repository_service()
        .create_case_definition_query()
        .key("caseLevelPlanningCase")
        .single_result()
        .expect("case definition query")
        .expect("case definition");
    assert_eq!(
        case_definition.model.case_plan_model.planning_tables.len(),
        1
    );
    assert_eq!(
        case_definition.model.case_plan_model.planning_tables[0].discretionary_items[0]
            .definition_ref,
        "human-task-review"
    );
}

#[test]
fn rejects_stage_discretionary_item_with_unknown_human_task_definition() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let invalid_model = CmmnModel::new(vec![CmmnCase::new(
        "case-planning",
        "invalidStagePlanningCase",
        "Invalid stage planning",
        CmmnCasePlanModel::new("case-plan-model", "Case plan model")
            .with_stage(
                CmmnStage::new("stage-review", "Review stage").with_planning_table(
                    CmmnPlanningTable::new("planning-table-review", "Review planning")
                        .with_discretionary_item(CmmnDiscretionaryItem::new(
                            "discretionary-missing-task",
                            "Missing task",
                            "human-task-missing",
                        )),
                ),
            )
            .with_plan_item(CmmnPlanItem::new("plan-item-stage-review", "stage-review")),
    )]);

    let error = engine
        .deploy(
            CmmnDeploymentRequest::new("invalid-stage-planning")
                .with_resource("invalid-stage-planning.cmmn", invalid_model),
        )
        .expect_err("unknown discretionary human task should fail deployment");
    let message = error.to_string();
    assert!(
        message.contains("discretionary item")
            && message.contains("references unknown human task definition"),
        "unexpected error: {message}"
    );
}

#[test]
fn rejects_case_level_discretionary_item_with_non_human_task_definition() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let invalid_model = CmmnModel::new(vec![CmmnCase::new(
        "case-planning",
        "invalidCasePlanningCase",
        "Invalid case planning",
        CmmnCasePlanModel::new("case-plan-model", "Case plan model")
            .with_decision_task(CmmnDecisionTask::new("decision-review", "Decision review"))
            .with_planning_table(
                CmmnPlanningTable::new("planning-table-case", "Case planning")
                    .with_discretionary_item(CmmnDiscretionaryItem::new(
                        "discretionary-decision",
                        "Decision review",
                        "decision-review",
                    )),
            ),
    )]);

    let error = engine
        .deploy(
            CmmnDeploymentRequest::new("invalid-case-planning")
                .with_resource("invalid-case-planning.cmmn", invalid_model),
        )
        .expect_err("non-human discretionary definition should fail deployment");
    let message = error.to_string();
    assert!(
        message.contains("discretionary item")
            && message.contains("references unknown human task definition")
            && message.contains("decision-review"),
        "unexpected error: {message}"
    );
}

#[test]
fn deploys_process_task_and_case_task_definitions() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let model = CmmnModel::new(vec![CmmnCase::new(
        "case-process-case-task",
        "processCaseTaskCase",
        "Process and case task case",
        CmmnCasePlanModel::new("case-plan-model", "Case plan model")
            .with_process_task(
                CmmnProcessTask::new("process-task-approval", "Approval process")
                    .with_process_ref("approvalProcess"),
            )
            .with_plan_item(CmmnPlanItem::new(
                "plan-item-process",
                "process-task-approval",
            ))
            .with_case_task(
                CmmnCaseTask::new("case-task-child", "Child case").with_case_ref("childCase"),
            )
            .with_plan_item(CmmnPlanItem::new("plan-item-case", "case-task-child")),
    )]);

    engine
        .deploy(
            CmmnDeploymentRequest::new("process-case-task")
                .with_resource("process-case-task.cmmn", model),
        )
        .expect("processTask/caseTask deployment should pass");
}

#[test]
fn rejects_process_task_with_empty_process_ref() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let model = CmmnModel::new(vec![CmmnCase::new(
        "case-process-task",
        "invalidProcessTaskCase",
        "Invalid process task case",
        CmmnCasePlanModel::new("case-plan-model", "Case plan model")
            .with_process_task(
                CmmnProcessTask::new("process-task-approval", "Approval process")
                    .with_process_ref(" "),
            )
            .with_plan_item(CmmnPlanItem::new(
                "plan-item-process",
                "process-task-approval",
            )),
    )]);

    let error = engine
        .deploy(
            CmmnDeploymentRequest::new("invalid-process-task")
                .with_resource("invalid-process-task.cmmn", model),
        )
        .expect_err("empty processRef should fail");

    assert!(
        error.to_string().contains("process task")
            && error.to_string().contains("empty process reference"),
        "unexpected error: {error}"
    );
}

#[test]
fn rejects_case_task_with_empty_case_ref() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let model = CmmnModel::new(vec![CmmnCase::new(
        "case-case-task",
        "invalidCaseTaskCase",
        "Invalid case task case",
        CmmnCasePlanModel::new("case-plan-model", "Case plan model")
            .with_case_task(CmmnCaseTask::new("case-task-child", "Child case").with_case_ref(" "))
            .with_plan_item(CmmnPlanItem::new("plan-item-case", "case-task-child")),
    )]);

    let error = engine
        .deploy(
            CmmnDeploymentRequest::new("invalid-case-task")
                .with_resource("invalid-case-task.cmmn", model),
        )
        .expect_err("empty caseRef should fail");

    assert!(
        error.to_string().contains("case task")
            && error.to_string().contains("empty case reference"),
        "unexpected error: {error}"
    );
}

#[test]
fn validates_conservative_if_part_subset_during_deployment() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let mut valid_sentry = CmmnSentry::new(
        "sentry-complex-if-part",
        CmmnPlanItemOnPart::new("on-review-complete", "plan-item-review", "complete"),
    );
    valid_sentry.if_part = Some(CmmnSentryIfPartExpression::Comparison(
        CmmnSentryIfPartCondition {
            variable_name: "customer.age + 1".to_string(),
            operator: CmmnSentryIfPartOperator::GreaterThanOrEqual,
            literal: CmmnSentryIfPartLiteral::Variable("minAge".to_string()),
        },
    ));

    let valid_model = CmmnModel::new(vec![CmmnCase::new(
        "case-review",
        "reviewCase",
        "Review case",
        CmmnCasePlanModel::new("case-plan-model", "Case plan model")
            .with_human_task(CmmnHumanTask::new("human-task-review", "Review"))
            .with_human_task(CmmnHumanTask::new("human-task-follow-up", "Follow-up"))
            .with_plan_item(CmmnPlanItem::new("plan-item-review", "human-task-review"))
            .with_plan_item(
                CmmnPlanItem::new("plan-item-follow-up", "human-task-follow-up")
                    .with_entry_criterion("sentry-complex-if-part"),
            )
            .with_sentry(valid_sentry),
    )]);

    let result = engine.deploy(
        CmmnDeploymentRequest::new("valid-if-part").with_resource("review-case.cmmn", valid_model),
    );
    assert!(result.is_ok(), "arithmetic ifPart must succeed deployment");
}

#[test]
fn deploys_length_if_part_comparison_subset() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let sentry = CmmnSentry {
        id: "sentry-name-length".to_string(),
        plan_item_on_parts: vec![CmmnPlanItemOnPart::new(
            "on-review-complete",
            "plan-item-review",
            "complete",
        )],
        case_file_item_on_parts: Vec::new(),
        trigger_mode: None,
        if_part: Some(CmmnSentryIfPartExpression::Comparison(
            CmmnSentryIfPartCondition {
                variable_name: "length(customer.name)".to_string(),
                operator: CmmnSentryIfPartOperator::GreaterThanOrEqual,
                literal: CmmnSentryIfPartLiteral::Variable("minimumNameLength".to_string()),
            },
        )),
    };
    let model = CmmnModel::new(vec![CmmnCase::new(
        "case-review",
        "reviewCase",
        "Review case",
        CmmnCasePlanModel::new("case-plan-model", "Case plan model")
            .with_human_task(CmmnHumanTask::new("human-task-review", "Review"))
            .with_human_task(CmmnHumanTask::new("human-task-follow-up", "Follow-up"))
            .with_plan_item(CmmnPlanItem::new("plan-item-review", "human-task-review"))
            .with_plan_item(
                CmmnPlanItem::new("plan-item-follow-up", "human-task-follow-up")
                    .with_entry_criterion("sentry-name-length"),
            )
            .with_sentry(sentry),
    )]);

    engine
        .deploy(
            CmmnDeploymentRequest::new("length-if-part").with_resource("review-case.cmmn", model),
        )
        .expect("length(path) ifPart should deploy");
}

#[test]
fn deploys_contains_if_part_comparison_subset() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let sentry = CmmnSentry::new(
        "sentry-name-contains",
        CmmnPlanItemOnPart::new("on-review-complete", "plan-item-review", "complete"),
    )
    .with_if_part("contains(customer.name, 'Ann') != false");
    let not_contains_sentry = CmmnSentry::new(
        "sentry-name-not-contains",
        CmmnPlanItemOnPart::new(
            "on-review-complete-not-contains",
            "plan-item-review",
            "complete",
        ),
    )
    .with_if_part("not(contains(customer.name, 'Bob'))");
    let not_boolean_path_sentry = CmmnSentry::new(
        "sentry-not-active-customer",
        CmmnPlanItemOnPart::new(
            "on-review-complete-not-active-customer",
            "plan-item-review",
            "complete",
        ),
    )
    .with_if_part("not(customer.active)");
    let bare_boolean_path_sentry = CmmnSentry::new(
        "sentry-active-customer",
        CmmnPlanItemOnPart::new(
            "on-review-complete-active-customer",
            "plan-item-review",
            "complete",
        ),
    )
    .with_if_part("customer.active");
    let model = CmmnModel::new(vec![CmmnCase::new(
        "case-review",
        "reviewCase",
        "Review case",
        CmmnCasePlanModel::new("case-plan-model", "Case plan model")
            .with_human_task(CmmnHumanTask::new("human-task-review", "Review"))
            .with_human_task(CmmnHumanTask::new("human-task-follow-up", "Follow-up"))
            .with_human_task(CmmnHumanTask::new(
                "human-task-not-contains",
                "Not Contains",
            ))
            .with_human_task(CmmnHumanTask::new(
                "human-task-not-active-customer",
                "Not Active Customer",
            ))
            .with_human_task(CmmnHumanTask::new(
                "human-task-active-customer",
                "Active Customer",
            ))
            .with_plan_item(CmmnPlanItem::new("plan-item-review", "human-task-review"))
            .with_plan_item(
                CmmnPlanItem::new("plan-item-follow-up", "human-task-follow-up")
                    .with_entry_criterion("sentry-name-contains"),
            )
            .with_plan_item(
                CmmnPlanItem::new("plan-item-not-contains", "human-task-not-contains")
                    .with_entry_criterion("sentry-name-not-contains"),
            )
            .with_plan_item(
                CmmnPlanItem::new(
                    "plan-item-not-active-customer",
                    "human-task-not-active-customer",
                )
                .with_entry_criterion("sentry-not-active-customer"),
            )
            .with_plan_item(
                CmmnPlanItem::new("plan-item-active-customer", "human-task-active-customer")
                    .with_entry_criterion("sentry-active-customer"),
            )
            .with_sentry(sentry)
            .with_sentry(not_contains_sentry)
            .with_sentry(not_boolean_path_sentry)
            .with_sentry(bare_boolean_path_sentry),
    )]);

    engine
        .deploy(
            CmmnDeploymentRequest::new("contains-if-part").with_resource("review-case.cmmn", model),
        )
        .expect("contains(path, value) ifPart should deploy");
}

#[test]
fn rejects_unsupported_contains_if_part_paths_during_deployment() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let sentry = CmmnSentry {
        id: "sentry-malformed-contains".to_string(),
        plan_item_on_parts: vec![CmmnPlanItemOnPart::new(
            "on-review-complete",
            "plan-item-review",
            "complete",
        )],
        case_file_item_on_parts: Vec::new(),
        trigger_mode: None,
        if_part: Some(CmmnSentryIfPartExpression::Contains {
            collection_variable_name: "customer..name".to_string(),
            value: CmmnSentryIfPartLiteral::String("x".to_string()),
            expected: true,
        }),
    };
    let model = CmmnModel::new(vec![CmmnCase::new(
        "case-review",
        "reviewCase",
        "Review case",
        CmmnCasePlanModel::new("case-plan-model", "Case plan model")
            .with_human_task(CmmnHumanTask::new("human-task-review", "Review"))
            .with_human_task(CmmnHumanTask::new("human-task-follow-up", "Follow-up"))
            .with_plan_item(CmmnPlanItem::new("plan-item-review", "human-task-review"))
            .with_plan_item(
                CmmnPlanItem::new("plan-item-follow-up", "human-task-follow-up")
                    .with_entry_criterion("sentry-malformed-contains"),
            )
            .with_sentry(sentry),
    )]);

    let error = engine
        .deploy(
            CmmnDeploymentRequest::new("invalid-contains-if-part")
                .with_resource("review-case.cmmn", model),
        )
        .expect_err("malformed contains collection path must fail deployment");

    assert!(error.to_string().contains("sentry ifPart"));
}

#[test]
fn deploys_event_listener_occur_entry_criterion_model() {
    let engine = CmmnEngine::new_in_memory().expect("engine");

    engine
        .deploy(
            CmmnDeploymentRequest::new("event-listener-occur-entry-criterion").with_resource(
                "event-listener-occur-entry-criterion-case.cmmn",
                event_listener_occur_entry_criterion_model(),
            ),
        )
        .expect("deployment");
}

#[test]
fn rejects_unsupported_standard_event_during_deployment() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let sentry = CmmnSentry {
        id: "sentry-resume-event".to_string(),
        plan_item_on_parts: vec![CmmnPlanItemOnPart::new(
            "on-review-resume",
            "plan-item-review",
            "resume",
        )],
        case_file_item_on_parts: Vec::new(),
        trigger_mode: None,
        if_part: None,
    };
    let model = CmmnModel::new(vec![CmmnCase::new(
        "case-unsupported-standard-event",
        "unsupportedStandardEventCase",
        "Unsupported standard event case",
        CmmnCasePlanModel::new("case-plan-model", "Case plan model")
            .with_human_task(CmmnHumanTask::new("human-task-review", "Review"))
            .with_plan_item(CmmnPlanItem::new("plan-item-review", "human-task-review"))
            .with_plan_item(
                CmmnPlanItem::new("plan-item-follow-up", "human-task-review")
                    .with_entry_criterion("sentry-resume-event"),
            )
            .with_sentry(sentry),
    )]);

    let error = engine
        .deploy(
            CmmnDeploymentRequest::new("unsupported-standard-event")
                .with_resource("review-case.cmmn", model),
        )
        .expect_err("unsupported standard event must fail deployment");

    let message = error.to_string();
    assert!(
        message.contains("Unsupported CMMN sentry standard event")
            && message.contains("resume")
            && message.contains(
                "only complete, occur, terminate, start, enable, disable, and exit are supported"
            ),
        "unexpected error message: {message}"
    );
}

#[test]
fn rejects_occur_standard_event_on_non_event_listener_plan_item() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let sentry = CmmnSentry {
        id: "sentry-occur-invalid".to_string(),
        plan_item_on_parts: vec![CmmnPlanItemOnPart::new(
            "on-review-occur",
            "plan-item-review",
            "occur",
        )],
        case_file_item_on_parts: Vec::new(),
        trigger_mode: None,
        if_part: None,
    };
    let model = CmmnModel::new(vec![CmmnCase::new(
        "case-occur-invalid",
        "occurInvalidCase",
        "Occur on non-event-listener case",
        CmmnCasePlanModel::new("case-plan-model", "Case plan model")
            .with_human_task(CmmnHumanTask::new("human-task-review", "Review"))
            .with_plan_item(CmmnPlanItem::new("plan-item-review", "human-task-review"))
            .with_plan_item(
                CmmnPlanItem::new("plan-item-follow-up", "human-task-review")
                    .with_entry_criterion("sentry-occur-invalid"),
            )
            .with_sentry(sentry),
    )]);

    let error = engine
        .deploy(
            CmmnDeploymentRequest::new("occur-invalid").with_resource("review-case.cmmn", model),
        )
        .expect_err("occur on non-event-listener must fail deployment");

    let message = error.to_string();
    assert!(
        message.contains("occur")
            && message.contains("event listener")
            && message.contains("milestone"),
        "unexpected error message: {message}"
    );
}

#[test]
fn rejects_enable_disable_standard_event_on_non_human_task_plan_item() {
    for event in ["enable", "disable"] {
        let engine = CmmnEngine::new_in_memory().expect("engine");
        let sentry = CmmnSentry {
            id: format!("sentry-{event}-invalid"),
            plan_item_on_parts: vec![CmmnPlanItemOnPart::new(
                format!("on-process-{event}"),
                "plan-item-process",
                event,
            )],
            case_file_item_on_parts: Vec::new(),
            trigger_mode: None,
            if_part: None,
        };
        let model = CmmnModel::new(vec![CmmnCase::new(
            format!("case-{event}-invalid"),
            format!("{event}InvalidCase"),
            format!("{event} on non-human-task case"),
            CmmnCasePlanModel::new("case-plan-model", "Case plan model")
                .with_process_task(CmmnProcessTask::new("process-task-work", "work"))
                .with_plan_item(CmmnPlanItem::new("plan-item-process", "process-task-work"))
                .with_plan_item(
                    CmmnPlanItem::new("plan-item-follow-up", "process-task-work")
                        .with_entry_criterion(format!("sentry-{event}-invalid")),
                )
                .with_sentry(sentry),
        )]);

        let error = engine
            .deploy(
                CmmnDeploymentRequest::new(format!("{event}-invalid"))
                    .with_resource("review-case.cmmn", model),
            )
            .expect_err("{event} on non-human-task must fail deployment");

        let message = error.to_string();
        assert!(
            message.contains(event) && message.contains("human task"),
            "unexpected error for {event}: {message}"
        );
    }
}

#[test]
fn rejects_unsupported_empty_if_part_variable_during_deployment() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let sentry = CmmnSentry {
        id: "sentry-empty-invalid".to_string(),
        plan_item_on_parts: vec![CmmnPlanItemOnPart::new(
            "on-review-complete",
            "plan-item-review",
            "complete",
        )],
        case_file_item_on_parts: Vec::new(),
        trigger_mode: None,
        if_part: Some(CmmnSentryIfPartExpression::Empty {
            variable_name: "customer..email".to_string(),
        }),
    };
    let model = CmmnModel::new(vec![CmmnCase::new(
        "case-empty-invalid",
        "emptyInvalidCase",
        "Empty invalid case",
        CmmnCasePlanModel::new("case-plan-model", "Case plan model")
            .with_human_task(CmmnHumanTask::new("human-task-review", "Review"))
            .with_plan_item(CmmnPlanItem::new("plan-item-review", "human-task-review"))
            .with_plan_item(
                CmmnPlanItem::new("plan-item-follow-up", "human-task-review")
                    .with_entry_criterion("sentry-empty-invalid"),
            )
            .with_sentry(sentry),
    )]);

    let error = engine
        .deploy(
            CmmnDeploymentRequest::new("empty-invalid").with_resource("review-case.cmmn", model),
        )
        .expect_err("empty with invalid variable must fail deployment");

    assert!(error.to_string().contains("sentry ifPart"));
}

#[test]
fn rejects_unsupported_starts_with_if_part_variable_during_deployment() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let sentry = CmmnSentry {
        id: "sentry-starts-with-invalid".to_string(),
        plan_item_on_parts: vec![CmmnPlanItemOnPart::new(
            "on-review-complete",
            "plan-item-review",
            "complete",
        )],
        case_file_item_on_parts: Vec::new(),
        trigger_mode: None,
        if_part: Some(CmmnSentryIfPartExpression::StartsWith {
            variable_name: "customer..name".to_string(),
            prefix: "test".to_string(),
        }),
    };
    let model = CmmnModel::new(vec![CmmnCase::new(
        "case-starts-with-invalid",
        "startsWithInvalidCase",
        "StartsWith invalid case",
        CmmnCasePlanModel::new("case-plan-model", "Case plan model")
            .with_human_task(CmmnHumanTask::new("human-task-review", "Review"))
            .with_plan_item(CmmnPlanItem::new("plan-item-review", "human-task-review"))
            .with_plan_item(
                CmmnPlanItem::new("plan-item-follow-up", "human-task-review")
                    .with_entry_criterion("sentry-starts-with-invalid"),
            )
            .with_sentry(sentry),
    )]);

    let error = engine
        .deploy(
            CmmnDeploymentRequest::new("starts-with-invalid")
                .with_resource("review-case.cmmn", model),
        )
        .expect_err("startsWith with invalid variable must fail deployment");

    assert!(error.to_string().contains("sentry ifPart"));
}

#[test]
fn rejects_unsupported_ends_with_if_part_variable_during_deployment() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let sentry = CmmnSentry {
        id: "sentry-ends-with-invalid".to_string(),
        plan_item_on_parts: vec![CmmnPlanItemOnPart::new(
            "on-review-complete",
            "plan-item-review",
            "complete",
        )],
        case_file_item_on_parts: Vec::new(),
        trigger_mode: None,
        if_part: Some(CmmnSentryIfPartExpression::EndsWith {
            variable_name: "customer..name".to_string(),
            suffix: "test".to_string(),
        }),
    };
    let model = CmmnModel::new(vec![CmmnCase::new(
        "case-ends-with-invalid",
        "endsWithInvalidCase",
        "EndsWith invalid case",
        CmmnCasePlanModel::new("case-plan-model", "Case plan model")
            .with_human_task(CmmnHumanTask::new("human-task-review", "Review"))
            .with_plan_item(CmmnPlanItem::new("plan-item-review", "human-task-review"))
            .with_plan_item(
                CmmnPlanItem::new("plan-item-follow-up", "human-task-review")
                    .with_entry_criterion("sentry-ends-with-invalid"),
            )
            .with_sentry(sentry),
    )]);

    let error = engine
        .deploy(
            CmmnDeploymentRequest::new("ends-with-invalid")
                .with_resource("review-case.cmmn", model),
        )
        .expect_err("endsWith with invalid variable must fail deployment");

    assert!(error.to_string().contains("sentry ifPart"));
}

#[test]
fn rejects_unsupported_matches_if_part_variable_during_deployment() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let sentry = CmmnSentry {
        id: "sentry-matches-invalid".to_string(),
        plan_item_on_parts: vec![CmmnPlanItemOnPart::new(
            "on-review-complete",
            "plan-item-review",
            "complete",
        )],
        case_file_item_on_parts: Vec::new(),
        trigger_mode: None,
        if_part: Some(CmmnSentryIfPartExpression::Matches {
            variable_name: "customer..name".to_string(),
            regex: ".*test.*".to_string(),
        }),
    };
    let model = CmmnModel::new(vec![CmmnCase::new(
        "case-matches-invalid",
        "matchesInvalidCase",
        "Matches invalid case",
        CmmnCasePlanModel::new("case-plan-model", "Case plan model")
            .with_human_task(CmmnHumanTask::new("human-task-review", "Review"))
            .with_plan_item(CmmnPlanItem::new("plan-item-review", "human-task-review"))
            .with_plan_item(
                CmmnPlanItem::new("plan-item-follow-up", "human-task-review")
                    .with_entry_criterion("sentry-matches-invalid"),
            )
            .with_sentry(sentry),
    )]);

    let error = engine
        .deploy(
            CmmnDeploymentRequest::new("matches-invalid").with_resource("review-case.cmmn", model),
        )
        .expect_err("matches with invalid variable must fail deployment");

    assert!(error.to_string().contains("sentry ifPart"));
}
