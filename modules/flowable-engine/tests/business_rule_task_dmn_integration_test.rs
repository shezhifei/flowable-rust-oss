use std::sync::Arc;

use flowable_bpmn_model::model::{
    BpmnModel, BusinessRuleTask, EndEvent, FlowElementEnum, Process, SequenceFlow, StartEvent,
};
use flowable_dmn_engine::{
    DmnDecision, DmnDeploymentRequest, DmnEngine, DmnHitPolicy, DmnInputClause, DmnModel,
    DmnOutputClause, DmnRule, DmnRuleInputEntry, DmnRuleOutputEntry, DmnUnaryTest,
};
use flowable_engine::engine::deployment_manager::DeploymentManager;
use flowable_engine::engine::runtime_service::RuntimeService;
use flowable_engine::interceptor::command_executor::DefaultCommandExecutor;
use flowable_engine::persistence::db_store::DbStore;
use flowable_engine::persistence::runtime_store::RuntimeStore;
use flowable_engine::repository::process_definition::ProcessDefinition;
use flowable_engine::service::config::ProcessEngineConfiguration;
use flowable_http_service::DeterministicHttpRuntime;
use serde_json::json;

fn element_id(element: &FlowElementEnum) -> Option<String> {
    match element {
        FlowElementEnum::SequenceFlow(flow) => flow.flow_element.base_element.id.clone(),
        FlowElementEnum::BusinessRuleTask(task) => task
            .task
            .activity
            .flow_node
            .flow_element
            .base_element
            .id
            .clone(),
        FlowElementEnum::StartEvent(event) => {
            event.event.flow_node.flow_element.base_element.id.clone()
        }
        FlowElementEnum::EndEvent(event) => {
            event.event.flow_node.flow_element.base_element.id.clone()
        }
        _ => None,
    }
}

fn build_sequence_flow(id: &str, source_ref: &str, target_ref: &str) -> SequenceFlow {
    let mut flow = SequenceFlow::default();
    flow.flow_element.base_element.id = Some(id.to_string());
    flow.source_ref = Some(source_ref.to_string());
    flow.target_ref = Some(target_ref.to_string());
    flow
}

fn sample_dmn_engine() -> Arc<DmnEngine> {
    let engine = Arc::new(DmnEngine::new_in_memory().expect("dmn engine"));
    engine
        .deploy(DmnDeploymentRequest::new("loan decisions").with_resource(
            "loan-eligibility.dmn",
            DmnModel::new(vec![DmnDecision::new(
                "loanEligibility",
                "loanEligibility",
                "Loan Eligibility",
                DmnHitPolicy::First,
                vec![DmnInputClause::new("input-1", "creditScore")],
                vec![
                    DmnOutputClause::new("output-1", "approved"),
                    DmnOutputClause::new("output-2", "riskBand"),
                ],
                vec![
                    DmnRule::new(
                        "rule-1",
                        vec![DmnRuleInputEntry::new(DmnUnaryTest::Equals(json!(730)))],
                        vec![
                            DmnRuleOutputEntry::new(json!(true)),
                            DmnRuleOutputEntry::new(json!("LOW")),
                        ],
                    ),
                    DmnRule::new(
                        "rule-2",
                        vec![DmnRuleInputEntry::new(DmnUnaryTest::Any)],
                        vec![
                            DmnRuleOutputEntry::new(json!(false)),
                            DmnRuleOutputEntry::new(json!("HIGH")),
                        ],
                    ),
                ],
            )]),
        ))
        .expect("dmn deployment");
    engine
}

#[test]
fn business_rule_task_executes_dmn_and_writes_result_variable() {
    let db_store = Arc::new(DbStore::new_in_memory().unwrap());
    let deployment_manager = DeploymentManager::new_with_memory_backend_for_test(db_store.clone());
    let runtime_store = RuntimeStore::new_with_memory_backend_for_test(db_store);
    let runtime_store_for_executor = runtime_store.clone();

    let config = ProcessEngineConfiguration {
        dmn_engine: Some(sample_dmn_engine()),
        ..ProcessEngineConfiguration::default()
    };
    let command_executor = Arc::new(DefaultCommandExecutor::new(
        deployment_manager.clone(),
        runtime_store_for_executor,
        Arc::new(config),
        Arc::new(DeterministicHttpRuntime::default()),
    ));
    let runtime_service =
        RuntimeService::new(command_executor, Arc::from("business-rule-test-owner"));

    let process_definition_id = "businessRuleTaskProcess".to_string();

    let mut process = Process::default();
    process.base_element.id = Some(process_definition_id.clone());
    let flow1 = build_sequence_flow("flow1", "startEvent1", "businessRuleTask1");
    let flow2 = build_sequence_flow("flow2", "businessRuleTask1", "endEvent1");

    let mut start_event = StartEvent::default();
    start_event.event.flow_node.flow_element.base_element.id = Some("startEvent1".to_string());
    start_event.event.flow_node.outgoing_flows = vec![flow1.clone()];

    let mut business_rule_task = BusinessRuleTask::default();
    business_rule_task
        .task
        .activity
        .flow_node
        .flow_element
        .base_element
        .id = Some("businessRuleTask1".to_string());
    business_rule_task.task.activity.flow_node.flow_element.name =
        Some("Evaluate Rule".to_string());
    business_rule_task.task.activity.flow_node.outgoing_flows = vec![flow2.clone()];
    business_rule_task.decision_ref = Some("loanEligibility".to_string());
    business_rule_task.result_variable_name = Some("decisionResult".to_string());
    business_rule_task.input_variables = vec!["creditScore".to_string()];

    let mut end_event = EndEvent::default();
    end_event.event.flow_node.flow_element.base_element.id = Some("endEvent1".to_string());
    process.flow_elements = vec![
        FlowElementEnum::StartEvent(start_event),
        FlowElementEnum::SequenceFlow(flow1),
        FlowElementEnum::BusinessRuleTask(business_rule_task),
        FlowElementEnum::SequenceFlow(flow2),
        FlowElementEnum::EndEvent(end_event),
    ];
    for element in &process.flow_elements {
        if let Some(id) = element_id(element) {
            process.flow_element_map.insert(id, element.clone());
        }
    }

    let bpmn_model = BpmnModel {
        main_process: Some(process),
        ..Default::default()
    };

    deployment_manager.insert_bpmn_model(&process_definition_id, bpmn_model);
    let mut session = deployment_manager.create_session().unwrap();
    deployment_manager.insert_process_definition(
        ProcessDefinition {
            id: process_definition_id.clone(),
            category: None,
            name: Some("Business Rule Task Process".to_string()),
            key: process_definition_id.clone(),
            description: None,
            version: 1,
            resource_name: Some("business-rule-task.bpmn20.xml".to_string()),
            deployment_id: Some("business-rule-task-deployment".to_string()),
            diagram_resource_name: None,
            has_start_form_key: false,
            has_graphical_notation: false,
            is_suspended: false,
            tenant_id: None,
            engine_version: None,
            app_version: None,
        history_level: None,
        },
        &mut session,
    );
    session.flush_and_commit().unwrap();

    let builder = runtime_service
        .create_process_instance_builder()
        .process_definition_id(process_definition_id)
        .name("Business Rule Task Instance".to_string())
        .variable("creditScore".to_string(), json!(730));

    let process_instance = runtime_service.start_process_instance(builder).unwrap();

    let mut session = runtime_store.create_session().unwrap();
    let stored_pi = runtime_store
        .find_process_instance(&process_instance.id, &mut session)
        .expect("Process instance should be in runtime store");
    assert!(stored_pi.is_ended);
    drop(session);
    let result_variable = runtime_service
        .get_variable(process_instance.id.clone(), "decisionResult".to_string())
        .expect("decision result variable query should succeed")
        .expect("decision result variable should exist");
    assert_eq!(result_variable["approved"], json!(true));
    assert_eq!(result_variable["riskBand"], json!("LOW"));
}

/// P79: multi-hit RULE_ORDER writes a JSON array under decisionKey
/// (Java `DmnActivityBehavior.setVariablesOnExecution` :244-257).
#[test]
fn business_rule_task_multi_hit_writes_row_array_under_decision_key() {
    let dmn = Arc::new(DmnEngine::new_in_memory().expect("dmn engine"));
    dmn.deploy(DmnDeploymentRequest::new("multi-hit").with_resource(
        "routing.dmn",
        DmnModel::new(vec![DmnDecision::new(
            "decision-1",
            "routingDecision",
            "Routing",
            DmnHitPolicy::RuleOrder,
            vec![DmnInputClause::new("input-1", "channel")],
            vec![
                DmnOutputClause::new("output-1", "route"),
                DmnOutputClause::new("output-2", "priority"),
            ],
            vec![
                DmnRule::new(
                    "rule-1",
                    vec![DmnRuleInputEntry::new(DmnUnaryTest::Equals(json!("email")))],
                    vec![
                        DmnRuleOutputEntry::new(json!("manual")),
                        DmnRuleOutputEntry::new(json!(10)),
                    ],
                ),
                DmnRule::new(
                    "rule-2",
                    vec![DmnRuleInputEntry::new(DmnUnaryTest::Equals(json!("email")))],
                    vec![
                        DmnRuleOutputEntry::new(json!("email-queue")),
                        DmnRuleOutputEntry::new(json!(20)),
                    ],
                ),
            ],
        )]),
    ))
    .expect("dmn deployment");

    let (runtime_service, runtime_store, process_definition_id) =
        start_business_rule_process(dmn, "routingDecision", None, vec!["channel".to_string()]);

    let process_instance = runtime_service
        .start_process_instance(
            runtime_service
                .create_process_instance_builder()
                .process_definition_id(process_definition_id)
                .variable("channel".to_string(), json!("email")),
        )
        .unwrap();

    let mut session = runtime_store.create_session().unwrap();
    let stored_pi = runtime_store
        .find_process_instance(&process_instance.id, &mut session)
        .expect("pi");
    assert!(stored_pi.is_ended);
    drop(session);

    let array = runtime_service
        .get_variable(process_instance.id.clone(), "routingDecision".to_string())
        .unwrap()
        .expect("multi-hit array under decisionKey");
    assert_eq!(
        array,
        json!([
            {"route": "manual", "priority": 10},
            {"route": "email-queue", "priority": 20}
        ])
    );
}

/// P79: single-hit without resultVariableName writes each output as its own variable
/// (Java `DmnActivityBehavior.setVariablesOnExecution` :258-266).
#[test]
fn business_rule_task_single_hit_writes_each_output_variable() {
    let (runtime_service, runtime_store, process_definition_id) = start_business_rule_process(
        sample_dmn_engine(),
        "loanEligibility",
        None,
        vec!["creditScore".to_string()],
    );

    let process_instance = runtime_service
        .start_process_instance(
            runtime_service
                .create_process_instance_builder()
                .process_definition_id(process_definition_id)
                .variable("creditScore".to_string(), json!(730)),
        )
        .unwrap();

    let mut session = runtime_store.create_session().unwrap();
    let stored_pi = runtime_store
        .find_process_instance(&process_instance.id, &mut session)
        .expect("pi");
    assert!(stored_pi.is_ended);
    drop(session);

    assert_eq!(
        runtime_service
            .get_variable(process_instance.id.clone(), "approved".to_string())
            .unwrap(),
        Some(json!(true))
    );
    assert_eq!(
        runtime_service
            .get_variable(process_instance.id.clone(), "riskBand".to_string())
            .unwrap(),
        Some(json!("LOW"))
    );
}

fn start_business_rule_process(
    dmn_engine: Arc<DmnEngine>,
    decision_ref: &str,
    result_variable_name: Option<&str>,
    input_variables: Vec<String>,
) -> (RuntimeService, RuntimeStore, String) {
    let db_store = Arc::new(DbStore::new_in_memory().unwrap());
    let deployment_manager = DeploymentManager::new_with_memory_backend_for_test(db_store.clone());
    let runtime_store = RuntimeStore::new_with_memory_backend_for_test(db_store);
    let runtime_store_for_executor = runtime_store.clone();

    let config = ProcessEngineConfiguration {
        dmn_engine: Some(dmn_engine),
        ..ProcessEngineConfiguration::default()
    };
    let command_executor = Arc::new(DefaultCommandExecutor::new(
        deployment_manager.clone(),
        runtime_store_for_executor,
        Arc::new(config),
        Arc::new(DeterministicHttpRuntime::default()),
    ));
    let runtime_service = RuntimeService::new(
        command_executor,
        Arc::from(format!("business-rule-p79-{decision_ref}")),
    );

    let process_definition_id = format!("businessRuleTaskProcess-{decision_ref}");
    let mut process = Process::default();
    process.base_element.id = Some(process_definition_id.clone());
    let flow1 = build_sequence_flow("flow1", "startEvent1", "businessRuleTask1");
    let flow2 = build_sequence_flow("flow2", "businessRuleTask1", "endEvent1");

    let mut start_event = StartEvent::default();
    start_event.event.flow_node.flow_element.base_element.id = Some("startEvent1".to_string());
    start_event.event.flow_node.outgoing_flows = vec![flow1.clone()];

    let mut business_rule_task = BusinessRuleTask::default();
    business_rule_task
        .task
        .activity
        .flow_node
        .flow_element
        .base_element
        .id = Some("businessRuleTask1".to_string());
    business_rule_task.task.activity.flow_node.outgoing_flows = vec![flow2.clone()];
    business_rule_task.decision_ref = Some(decision_ref.to_string());
    business_rule_task.result_variable_name = result_variable_name.map(str::to_string);
    business_rule_task.input_variables = input_variables;

    let mut end_event = EndEvent::default();
    end_event.event.flow_node.flow_element.base_element.id = Some("endEvent1".to_string());
    process.flow_elements = vec![
        FlowElementEnum::StartEvent(start_event),
        FlowElementEnum::SequenceFlow(flow1),
        FlowElementEnum::BusinessRuleTask(business_rule_task),
        FlowElementEnum::SequenceFlow(flow2),
        FlowElementEnum::EndEvent(end_event),
    ];
    for element in &process.flow_elements {
        if let Some(id) = element_id(element) {
            process.flow_element_map.insert(id, element.clone());
        }
    }

    let bpmn_model = BpmnModel {
        main_process: Some(process),
        ..Default::default()
    };

    deployment_manager.insert_bpmn_model(&process_definition_id, bpmn_model);
    let mut session = deployment_manager.create_session().unwrap();
    deployment_manager.insert_process_definition(
        ProcessDefinition {
            id: process_definition_id.clone(),
            category: None,
            name: Some("Business Rule Task Process".to_string()),
            key: process_definition_id.clone(),
            description: None,
            version: 1,
            resource_name: Some("business-rule-task.bpmn20.xml".to_string()),
            deployment_id: Some("business-rule-task-deployment".to_string()),
            diagram_resource_name: None,
            has_start_form_key: false,
            has_graphical_notation: false,
            is_suspended: false,
            tenant_id: None,
            engine_version: None,
            app_version: None,
        history_level: None,
        },
        &mut session,
    );
    session.flush_and_commit().unwrap();

    (runtime_service, runtime_store, process_definition_id)
}
