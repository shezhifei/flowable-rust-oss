//! P80 — serviceTask `flowable:type="dmn"` full chain.
//!
//! Java reference: `DmnActivityBehavior.java:58-195` (execute), `:197-267` (writeback),
//! `ExternalInvocationTaskValidator.java:88-108` (deployment validation).

use flowable_bpmn_model::model::{
    BpmnModel, EndEvent, FieldExtension, FlowElementEnum, Process, SequenceFlow, ServiceTask,
    StartEvent,
};
use flowable_dmn_engine::{
    DecisionService, DmnDecision, DmnDeploymentRequest, DmnEngine, DmnHitPolicy, DmnInputClause,
    DmnModel, DmnOutputClause, DmnRule, DmnRuleInputEntry, DmnRuleOutputEntry, DmnUnaryTest,
};
use flowable_engine::engine::deployment_manager::DeploymentManager;
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::engine::runtime_service::RuntimeService;
use flowable_engine::error::FlowableError;
use flowable_engine::interceptor::command_executor::DefaultCommandExecutor;
use flowable_engine::persistence::db_store::DbStore;
use flowable_engine::persistence::runtime_store::RuntimeStore;
use flowable_engine::repository::process_definition::ProcessDefinition;
use flowable_engine::service::config::ProcessEngineConfiguration;
use flowable_engine::validation::unsupported_model_validator::UnsupportedModelValidator;
use flowable_http_service::DeterministicHttpRuntime;
use serde_json::{Value, json};
use std::sync::Arc;

// ── helpers ──────────────────────────────────────────────────────────────

fn field(name: &str, string_value: Option<&str>, expression: Option<&str>) -> FieldExtension {
    FieldExtension {
        field_name: Some(name.to_string()),
        string_value: string_value.map(str::to_string),
        expression: expression.map(str::to_string),
        ..Default::default()
    }
}

fn build_sequence_flow(id: &str, source_ref: &str, target_ref: &str) -> SequenceFlow {
    let mut flow = SequenceFlow::default();
    flow.flow_element.base_element.id = Some(id.to_string());
    flow.source_ref = Some(source_ref.to_string());
    flow.target_ref = Some(target_ref.to_string());
    flow
}

fn element_id(element: &FlowElementEnum) -> Option<String> {
    match element {
        FlowElementEnum::SequenceFlow(flow) => flow.flow_element.base_element.id.clone(),
        FlowElementEnum::ServiceTask(task) => task
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

fn loan_eligibility_decision() -> DmnDecision {
    DmnDecision::new(
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
    )
}

fn never_hit_decision() -> DmnDecision {
    DmnDecision::new(
        "neverHit",
        "neverHit",
        "Never Hit",
        DmnHitPolicy::First,
        vec![DmnInputClause::new("input-1", "creditScore")],
        vec![DmnOutputClause::new("output-1", "approved")],
        vec![DmnRule::new(
            "rule-1",
            // Only matches an impossible score — ensures empty result for normal inputs.
            vec![DmnRuleInputEntry::new(DmnUnaryTest::Equals(json!(-99999)))],
            vec![DmnRuleOutputEntry::new(json!(true))],
        )],
    )
}

fn multi_hit_routing_decision() -> DmnDecision {
    DmnDecision::new(
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
    )
}

/// Process model: start → dmn serviceTask → end.
fn dmn_service_task_process(
    process_id: &str,
    fields: Vec<FieldExtension>,
    skip_expression: Option<&str>,
) -> BpmnModel {
    let mut process = Process::default();
    process.base_element.id = Some(process_id.to_string());
    let flow1 = build_sequence_flow("flow1", "startEvent1", "dmnTask1");
    let flow2 = build_sequence_flow("flow2", "dmnTask1", "endEvent1");

    let mut start_event = StartEvent::default();
    start_event.event.flow_node.flow_element.base_element.id = Some("startEvent1".to_string());
    start_event.event.flow_node.outgoing_flows = vec![flow1.clone()];

    let mut dmn_task = ServiceTask::default();
    dmn_task
        .task
        .activity
        .flow_node
        .flow_element
        .base_element
        .id = Some("dmnTask1".to_string());
    dmn_task.task.activity.flow_node.outgoing_flows = vec![flow2.clone()];
    dmn_task.task_type = Some("dmn".to_string());
    dmn_task.task.activity.field_extensions = fields;
    dmn_task.skip_expression = skip_expression.map(str::to_string);

    let mut end_event = EndEvent::default();
    end_event.event.flow_node.flow_element.base_element.id = Some("endEvent1".to_string());

    process.flow_elements = vec![
        FlowElementEnum::StartEvent(start_event),
        FlowElementEnum::SequenceFlow(flow1),
        FlowElementEnum::ServiceTask(dmn_task),
        FlowElementEnum::SequenceFlow(flow2),
        FlowElementEnum::EndEvent(end_event),
    ];
    for element in &process.flow_elements {
        if let Some(id) = element_id(element) {
            process.flow_element_map.insert(id, element.clone());
        }
    }

    BpmnModel {
        main_process: Some(process.clone()),
        processes: vec![process],
        ..Default::default()
    }
}

fn start_with_programmatic_process(
    dmn_engine: Arc<DmnEngine>,
    process_id: &str,
    fields: Vec<FieldExtension>,
    skip_expression: Option<&str>,
    deployment_id: &str,
    variables: Vec<(String, Value)>,
) -> Result<(RuntimeService, RuntimeStore, String), FlowableError> {
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
    let runtime_service =
        RuntimeService::new(command_executor, Arc::from(format!("p80-dmn-{process_id}")));

    let bpmn_model = dmn_service_task_process(process_id, fields, skip_expression);
    deployment_manager.insert_bpmn_model(process_id, bpmn_model);
    let mut session = deployment_manager.create_session().unwrap();
    deployment_manager.insert_process_definition(
        ProcessDefinition {
            id: process_id.to_string(),
            category: None,
            name: Some(process_id.to_string()),
            key: process_id.to_string(),
            description: None,
            version: 1,
            resource_name: Some(format!("{process_id}.bpmn20.xml")),
            deployment_id: Some(deployment_id.to_string()),
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

    let mut builder = runtime_service
        .create_process_instance_builder()
        .process_definition_id(process_id.to_string())
        .name(format!("{process_id}-instance"));
    for (k, v) in variables {
        builder = builder.variable(k, v);
    }
    let process_instance = runtime_service.start_process_instance(builder)?;
    Ok((runtime_service, runtime_store, process_instance.id))
}

fn deploy_decision(engine: &DmnEngine, name: &str, decision: DmnDecision) {
    engine
        .deploy(DmnDeploymentRequest::new(name).with_resource(
            format!("{name}.dmn"),
            DmnModel::new(vec![decision]),
        ))
        .expect("dmn deploy");
}

fn deploy_decision_with_parent(
    engine: &DmnEngine,
    name: &str,
    parent_deployment_id: &str,
    decision: DmnDecision,
) {
    engine
        .deploy(
            DmnDeploymentRequest::new(name)
                .with_parent_deployment_id(parent_deployment_id)
                .with_resource(format!("{name}.dmn"), DmnModel::new(vec![decision])),
        )
        .expect("dmn deploy with parent");
}

fn assert_ended(runtime_store: &RuntimeStore, process_instance_id: &str) {
    let mut session = runtime_store.create_session().unwrap();
    let stored = runtime_store
        .find_process_instance(process_instance_id, &mut session)
        .expect("pi");
    assert!(stored.is_ended, "process should have completed");
}

// ── 1. literal decisionTableReferenceKey single-hit ─────────────────────

#[test]
fn service_task_dmn_literal_key_single_hit_writes_outputs_and_leaves() {
    let dmn = Arc::new(DmnEngine::new_in_memory().expect("dmn"));
    deploy_decision(&dmn, "loan", loan_eligibility_decision());

    let (runtime, store, pi_id) = start_with_programmatic_process(
        dmn,
        "dmnLiteralSingleHit",
        vec![field(
            "decisionTableReferenceKey",
            Some("loanEligibility"),
            None,
        )],
        None,
        "dep-literal",
        vec![("creditScore".to_string(), json!(730))],
    )
    .expect("start");

    assert_ended(&store, &pi_id);
    assert_eq!(
        runtime
            .get_variable(pi_id.clone(), "approved".to_string())
            .unwrap(),
        Some(json!(true))
    );
    assert_eq!(
        runtime
            .get_variable(pi_id, "riskBand".to_string())
            .unwrap(),
        Some(json!("LOW"))
    );
}

// ── 2. EL decision key + negative cases ──────────────────────────────────

#[test]
fn service_task_dmn_el_key_resolves_at_runtime() {
    let dmn = Arc::new(DmnEngine::new_in_memory().expect("dmn"));
    deploy_decision(&dmn, "loan", loan_eligibility_decision());

    let (runtime, store, pi_id) = start_with_programmatic_process(
        dmn,
        "dmnElKey",
        vec![field(
            "decisionTableReferenceKey",
            None,
            Some("${decisionKey}"),
        )],
        None,
        "dep-el",
        vec![
            ("creditScore".to_string(), json!(730)),
            ("decisionKey".to_string(), json!("loanEligibility")),
        ],
    )
    .expect("start");

    assert_ended(&store, &pi_id);
    assert_eq!(
        runtime
            .get_variable(pi_id, "approved".to_string())
            .unwrap(),
        Some(json!(true))
    );
}

#[test]
fn service_task_dmn_el_key_non_string_errors() {
    let dmn = Arc::new(DmnEngine::new_in_memory().expect("dmn"));
    deploy_decision(&dmn, "loan", loan_eligibility_decision());

    let err = match start_with_programmatic_process(
        dmn,
        "dmnElNonString",
        vec![field(
            "decisionTableReferenceKey",
            None,
            Some("${decisionKey}"),
        )],
        None,
        "dep-el-ns",
        vec![
            ("creditScore".to_string(), json!(730)),
            ("decisionKey".to_string(), json!(42)),
        ],
    ) {
        Ok(_) => panic!("non-string EL must fail"),
        Err(e) => e,
    };

    let msg = err.to_string();
    assert!(
        msg.contains("decisionTableReferenceKey expression does not resolve to a string"),
        "unexpected message: {msg}"
    );
}

#[test]
fn service_task_dmn_el_key_empty_errors() {
    let dmn = Arc::new(DmnEngine::new_in_memory().expect("dmn"));
    deploy_decision(&dmn, "loan", loan_eligibility_decision());

    let err = match start_with_programmatic_process(
        dmn,
        "dmnElEmpty",
        vec![field(
            "decisionTableReferenceKey",
            None,
            Some("${decisionKey}"),
        )],
        None,
        "dep-el-empty",
        vec![
            ("creditScore".to_string(), json!(730)),
            ("decisionKey".to_string(), json!("")),
        ],
    ) {
        Ok(_) => panic!("empty EL must fail"),
        Err(e) => e,
    };

    let msg = err.to_string();
    assert!(
        msg.contains("decisionTableReferenceKey expression resolves to an empty value"),
        "unexpected message: {msg}"
    );
}

// ── 3. missing key — runtime + validator ─────────────────────────────────

#[test]
fn service_task_dmn_missing_key_runtime_required_message() {
    let dmn = Arc::new(DmnEngine::new_in_memory().expect("dmn"));
    deploy_decision(&dmn, "loan", loan_eligibility_decision());

    let err = match start_with_programmatic_process(
        dmn,
        "dmnMissingKey",
        vec![],
        None,
        "dep-missing",
        vec![("creditScore".to_string(), json!(730))],
    ) {
        Ok(_) => panic!("missing key must fail at runtime"),
        Err(e) => e,
    };

    let msg = err.to_string();
    assert!(
        msg.contains(
            "decisionTableReferenceKey is a required field extension for the dmn task dmnTask1"
        ),
        "unexpected message: {msg}"
    );
}

#[test]
fn service_task_dmn_missing_key_validator_rejects() {
    let model = dmn_service_task_process("dmnValidatorMissing", vec![], None);
    let err = UnsupportedModelValidator::validate(&model, &ProcessEngineConfiguration::default())
        .expect_err("validator must reject missing key");
    let msg = err.to_string();
    assert!(
        msg.contains("No decision table or decision service reference key"),
        "unexpected message: {msg}"
    );
}

#[test]
fn service_task_dmn_validator_accepts_decision_table_key() {
    let model = dmn_service_task_process(
        "dmnValidatorOk",
        vec![field(
            "decisionTableReferenceKey",
            Some("loanEligibility"),
            None,
        )],
        None,
    );
    UnsupportedModelValidator::validate(&model, &ProcessEngineConfiguration::default())
        .expect("valid dmn service task should pass validator");
}

// ── 4. throwErrorOnNoHits ────────────────────────────────────────────────

#[test]
fn service_task_dmn_throw_on_no_hits_true() {
    let dmn = Arc::new(DmnEngine::new_in_memory().expect("dmn"));
    deploy_decision(&dmn, "never", never_hit_decision());

    let err = match start_with_programmatic_process(
        dmn,
        "dmnThrowTrue",
        vec![
            field("decisionTableReferenceKey", Some("neverHit"), None),
            field("decisionTaskThrowErrorOnNoHits", Some("true"), None),
        ],
        None,
        "dep-throw-true",
        vec![("creditScore".to_string(), json!(730))],
    ) {
        Ok(_) => panic!("true must throw on no hits"),
        Err(e) => e,
    };

    let msg = err.to_string();
    assert!(
        msg.contains("did not hit any rules for the provided input"),
        "unexpected message: {msg}"
    );
}

#[test]
fn service_task_dmn_throw_on_no_hits_false_continues() {
    let dmn = Arc::new(DmnEngine::new_in_memory().expect("dmn"));
    deploy_decision(&dmn, "never", never_hit_decision());

    let (runtime, store, pi_id) = start_with_programmatic_process(
        dmn,
        "dmnThrowFalse",
        vec![
            field("decisionTableReferenceKey", Some("neverHit"), None),
            field("decisionTaskThrowErrorOnNoHits", Some("false"), None),
        ],
        None,
        "dep-throw-false",
        vec![("creditScore".to_string(), json!(730))],
    )
    .expect("false must not throw");

    assert_ended(&store, &pi_id);
    assert_eq!(
        runtime
            .get_variable(pi_id, "approved".to_string())
            .unwrap(),
        None
    );
}

#[test]
fn service_task_dmn_throw_on_no_hits_el_true() {
    let dmn = Arc::new(DmnEngine::new_in_memory().expect("dmn"));
    deploy_decision(&dmn, "never", never_hit_decision());

    let err = match start_with_programmatic_process(
        dmn,
        "dmnThrowEl",
        vec![
            field("decisionTableReferenceKey", Some("neverHit"), None),
            field("decisionTaskThrowErrorOnNoHits", None, Some("${flag}")),
        ],
        None,
        "dep-throw-el",
        vec![
            ("creditScore".to_string(), json!(730)),
            ("flag".to_string(), json!(true)),
        ],
    ) {
        Ok(_) => panic!("EL true must throw"),
        Err(e) => e,
    };

    let msg = err.to_string();
    assert!(
        msg.contains("did not hit any rules for the provided input"),
        "unexpected message: {msg}"
    );
}

// ── 5. multi-hit / multipleResults writeback ─────────────────────────────

#[test]
fn service_task_dmn_multi_hit_writes_row_array_under_decision_key() {
    let dmn = Arc::new(DmnEngine::new_in_memory().expect("dmn"));
    deploy_decision(&dmn, "routing", multi_hit_routing_decision());

    let (runtime, store, pi_id) = start_with_programmatic_process(
        dmn,
        "dmnMultiHit",
        vec![field(
            "decisionTableReferenceKey",
            Some("routingDecision"),
            None,
        )],
        None,
        "dep-multi",
        vec![("channel".to_string(), json!("email"))],
    )
    .expect("start");

    assert_ended(&store, &pi_id);
    let array = runtime
        .get_variable(pi_id, "routingDecision".to_string())
        .unwrap()
        .expect("multi-hit array");
    assert_eq!(
        array,
        json!([
            {"route": "manual", "priority": 10},
            {"route": "email-queue", "priority": 20}
        ])
    );
}

#[test]
fn service_task_dmn_single_hit_writes_each_output_variable() {
    // Covered also by literal single-hit; explicit name mirrors business_rule P79 tests.
    let dmn = Arc::new(DmnEngine::new_in_memory().expect("dmn"));
    deploy_decision(&dmn, "loan", loan_eligibility_decision());

    let (runtime, store, pi_id) = start_with_programmatic_process(
        dmn,
        "dmnSingleHitFlat",
        vec![field(
            "decisionTableReferenceKey",
            Some("loanEligibility"),
            None,
        )],
        None,
        "dep-flat",
        vec![("creditScore".to_string(), json!(100))], // hits rule-2 (any)
    )
    .expect("start");

    assert_ended(&store, &pi_id);
    assert_eq!(
        runtime
            .get_variable(pi_id.clone(), "approved".to_string())
            .unwrap(),
        Some(json!(false))
    );
    assert_eq!(
        runtime
            .get_variable(pi_id, "riskBand".to_string())
            .unwrap(),
        Some(json!("HIGH"))
    );
}

// ── 6. decision service path ─────────────────────────────────────────────

#[test]
fn service_task_dmn_decision_service_all_single_hit_writes_flat_outputs() {
    let dmn = Arc::new(DmnEngine::new_in_memory().expect("dmn"));

    let d1 = DmnDecision::new(
        "childDecisionA",
        "childDecisionA",
        "Child A",
        DmnHitPolicy::First,
        vec![DmnInputClause::new("input-1", "score")],
        vec![DmnOutputClause::new("output-1", "band")],
        vec![DmnRule::new(
            "rule-1",
            vec![DmnRuleInputEntry::new(DmnUnaryTest::Any)],
            vec![DmnRuleOutputEntry::new(json!("A"))],
        )],
    );
    let d2 = DmnDecision::new(
        "childDecisionB",
        "childDecisionB",
        "Child B",
        DmnHitPolicy::First,
        vec![DmnInputClause::new("input-1", "score")],
        vec![DmnOutputClause::new("output-1", "tier")],
        vec![DmnRule::new(
            "rule-1",
            vec![DmnRuleInputEntry::new(DmnUnaryTest::Any)],
            vec![DmnRuleOutputEntry::new(json!(1))],
        )],
    );
    let mut model = DmnModel::new(vec![d1, d2]);
    // Two output decisions → multiple_results true (service path ObjectNode write).
    // For all-single-hit flat write we need a single-output service.
    model.decision_services.push(DecisionService {
        id: "loanServiceSingle".to_string(),
        name: "Loan Service Single".to_string(),
        required_decisions: vec![],
        output_decisions: vec!["childDecisionA".to_string()],
    });
    dmn.deploy(DmnDeploymentRequest::new("svc-single").with_resource("svc.dmn", model))
        .expect("deploy service");

    // Also deploy child B alone is unused here — single-output service only uses A.
    let (runtime, store, pi_id) = start_with_programmatic_process(
        dmn,
        "dmnServiceSingle",
        vec![field(
            "decisionTableReferenceKey",
            Some("loanServiceSingle"),
            None,
        )],
        None,
        "dep-svc-single",
        vec![("score".to_string(), json!(10))],
    )
    .expect("start");

    assert_ended(&store, &pi_id);
    // Single-output decision service with single hit → flat outputs
    // (Java setDecisionServiceVariablesOnExecution :222-231)
    assert_eq!(
        runtime.get_variable(pi_id, "band".to_string()).unwrap(),
        Some(json!("A"))
    );
}

#[test]
fn service_task_dmn_decision_service_multi_output_writes_object_node() {
    let dmn = Arc::new(DmnEngine::new_in_memory().expect("dmn"));

    let d1 = DmnDecision::new(
        "childDecisionA",
        "childDecisionA",
        "Child A",
        DmnHitPolicy::First,
        vec![DmnInputClause::new("input-1", "score")],
        vec![DmnOutputClause::new("output-1", "band")],
        vec![DmnRule::new(
            "rule-1",
            vec![DmnRuleInputEntry::new(DmnUnaryTest::Any)],
            vec![DmnRuleOutputEntry::new(json!("A"))],
        )],
    );
    let d2 = DmnDecision::new(
        "childDecisionB",
        "childDecisionB",
        "Child B",
        DmnHitPolicy::First,
        vec![DmnInputClause::new("input-1", "score")],
        vec![DmnOutputClause::new("output-1", "tier")],
        vec![DmnRule::new(
            "rule-1",
            vec![DmnRuleInputEntry::new(DmnUnaryTest::Any)],
            vec![DmnRuleOutputEntry::new(json!(1))],
        )],
    );
    let mut model = DmnModel::new(vec![d1, d2]);
    model.decision_services.push(DecisionService {
        id: "loanServiceMulti".to_string(),
        name: "Loan Service Multi".to_string(),
        required_decisions: vec![],
        output_decisions: vec![
            "childDecisionA".to_string(),
            "childDecisionB".to_string(),
        ],
    });
    dmn.deploy(DmnDeploymentRequest::new("svc-multi").with_resource("svc.dmn", model))
        .expect("deploy service");

    let (runtime, store, pi_id) = start_with_programmatic_process(
        dmn,
        "dmnServiceMulti",
        vec![field(
            "decisionTableReferenceKey",
            Some("loanServiceMulti"),
            None,
        )],
        None,
        "dep-svc-multi",
        vec![("score".to_string(), json!(10))],
    )
    .expect("start");

    assert_ended(&store, &pi_id);
    // >1 output decision → ObjectNode under decisionServiceKey
    // (Java :205-221; multiple_results true because output_decisions.len() > 1)
    let node = runtime
        .get_variable(pi_id, "loanServiceMulti".to_string())
        .unwrap()
        .expect("service result ObjectNode");
    assert_eq!(
        node,
        json!({
            "childDecisionA": [{"band": "A"}],
            "childDecisionB": [{"tier": 1}]
        })
    );
}

// ── 7. sameDeployment parent deployment scoping ──────────────────────────

#[test]
fn service_task_dmn_same_deployment_default_uses_parent_deployment_id() {
    let dmn = Arc::new(DmnEngine::new_in_memory().expect("dmn"));
    let parent = "process-deployment-same";

    // Parent-scoped decision returns "scoped"
    let scoped = DmnDecision::new(
        "scopedDecision",
        "scopedDecision",
        "Scoped",
        DmnHitPolicy::First,
        vec![DmnInputClause::new("input-1", "x")],
        vec![DmnOutputClause::new("output-1", "label")],
        vec![DmnRule::new(
            "rule-1",
            vec![DmnRuleInputEntry::new(DmnUnaryTest::Any)],
            vec![DmnRuleOutputEntry::new(json!("scoped"))],
        )],
    );
    deploy_decision_with_parent(&dmn, "scoped-v1", parent, scoped);

    // Later global deployment returns "global" — default sameDeployment must still hit scoped
    let global = DmnDecision::new(
        "scopedDecision",
        "scopedDecision",
        "Global",
        DmnHitPolicy::First,
        vec![DmnInputClause::new("input-1", "x")],
        vec![DmnOutputClause::new("output-1", "label")],
        vec![DmnRule::new(
            "rule-1",
            vec![DmnRuleInputEntry::new(DmnUnaryTest::Any)],
            vec![DmnRuleOutputEntry::new(json!("global"))],
        )],
    );
    deploy_decision(&dmn, "global-v2", global);

    let (runtime, store, pi_id) = start_with_programmatic_process(
        dmn,
        "dmnSameDepDefault",
        vec![field(
            "decisionTableReferenceKey",
            Some("scopedDecision"),
            None,
        )],
        None,
        parent,
        vec![("x".to_string(), json!(1))],
    )
    .expect("start");

    assert_ended(&store, &pi_id);
    assert_eq!(
        runtime.get_variable(pi_id, "label".to_string()).unwrap(),
        Some(json!("scoped")),
        "default sameDeployment must pass parentDeploymentId"
    );
}

#[test]
fn service_task_dmn_same_deployment_false_skips_parent_filter() {
    let dmn = Arc::new(DmnEngine::new_in_memory().expect("dmn"));
    let parent = "process-deployment-false";

    let scoped = DmnDecision::new(
        "scopedDecision2",
        "scopedDecision2",
        "Scoped",
        DmnHitPolicy::First,
        vec![DmnInputClause::new("input-1", "x")],
        vec![DmnOutputClause::new("output-1", "label")],
        vec![DmnRule::new(
            "rule-1",
            vec![DmnRuleInputEntry::new(DmnUnaryTest::Any)],
            vec![DmnRuleOutputEntry::new(json!("scoped"))],
        )],
    );
    deploy_decision_with_parent(&dmn, "scoped2-v1", parent, scoped);

    let global = DmnDecision::new(
        "scopedDecision2",
        "scopedDecision2",
        "Global",
        DmnHitPolicy::First,
        vec![DmnInputClause::new("input-1", "x")],
        vec![DmnOutputClause::new("output-1", "label")],
        vec![DmnRule::new(
            "rule-1",
            vec![DmnRuleInputEntry::new(DmnUnaryTest::Any)],
            vec![DmnRuleOutputEntry::new(json!("global"))],
        )],
    );
    deploy_decision(&dmn, "global2-v2", global);

    let (runtime, store, pi_id) = start_with_programmatic_process(
        dmn,
        "dmnSameDepFalse",
        vec![
            field("decisionTableReferenceKey", Some("scopedDecision2"), None),
            field("sameDeployment", Some("false"), None),
        ],
        None,
        parent,
        vec![("x".to_string(), json!(1))],
    )
    .expect("start");

    assert_ended(&store, &pi_id);
    assert_eq!(
        runtime.get_variable(pi_id, "label".to_string()).unwrap(),
        Some(json!("global")),
        "sameDeployment=false must not pass parentDeploymentId (latest global wins)"
    );
}

// ── 8. skipExpression regression ─────────────────────────────────────────

#[test]
fn service_task_dmn_skip_expression_leaves_without_executing() {
    let dmn = Arc::new(DmnEngine::new_in_memory().expect("dmn"));
    // Deliberately do NOT deploy the decision — skip must leave without calling DMN.
    let (runtime, store, pi_id) = start_with_programmatic_process(
        dmn,
        "dmnSkip",
        vec![field(
            "decisionTableReferenceKey",
            Some("loanEligibility"),
            None,
        )],
        Some("${shouldSkip}"),
        "dep-skip",
        vec![
            (
                "_FLOWABLE_SKIP_EXPRESSION_ENABLED".to_string(),
                json!(true),
            ),
            ("shouldSkip".to_string(), json!(true)),
        ],
    )
    .expect("skip must leave successfully");

    assert_ended(&store, &pi_id);
    assert_eq!(
        runtime
            .get_variable(pi_id, "approved".to_string())
            .unwrap(),
        None,
        "skip must not write DMN outputs"
    );
}

// ── XML deploy path (converter + validator + default engine DMN) ─────────

#[test]
fn service_task_dmn_xml_deploy_and_execute() {
    let process_engine = ProcessEngine::new("p80-dmn-xml".to_string());
    let dmn = process_engine
        .get_config()
        .dmn_engine
        .clone()
        .expect("default config includes dmn_engine");
    deploy_decision(&dmn, "loan-xml", loan_eligibility_decision());

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
                 xmlns:flowable="http://flowable.org/bpmn"
                 targetNamespace="Examples">
        <process id="dmnXmlProcess" name="DMN XML Process">
            <startEvent id="startEvent1" />
            <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="dmnTask1" />
            <serviceTask id="dmnTask1" name="Evaluate" flowable:type="dmn">
                <extensionElements>
                    <flowable:field name="decisionTableReferenceKey">
                        <flowable:string>loanEligibility</flowable:string>
                    </flowable:field>
                </extensionElements>
            </serviceTask>
            <sequenceFlow id="flow2" sourceRef="dmnTask1" targetRef="endEvent1" />
            <endEvent id="endEvent1" />
        </process>
    </definitions>"#;

    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();
    repository_service
        .deploy(
            repository_service
                .create_deployment()
                .name("dmn-xml".to_string())
                .add_string("dmnXmlProcess.bpmn20.xml".to_string(), xml.to_string()),
        )
        .expect("deploy");

    let process_definition_id = repository_service.get_process_definition_ids().unwrap()[0].clone();
    let process_instance = runtime_service
        .start_process_instance(
            runtime_service
                .create_process_instance_builder()
                .process_definition_id(process_definition_id)
                .variable("creditScore".to_string(), json!(730)),
        )
        .expect("start");

    assert_eq!(
        runtime_service
            .get_variable(process_instance.id.clone(), "approved".to_string())
            .unwrap(),
        Some(json!(true))
    );
    assert_eq!(
        runtime_service
            .get_variable(process_instance.id, "riskBand".to_string())
            .unwrap(),
        Some(json!("LOW"))
    );
}
