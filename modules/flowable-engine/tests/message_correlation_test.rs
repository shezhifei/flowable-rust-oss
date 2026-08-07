use flowable_engine::cmd::correlate_message_cmd::CorrelateMessageOptions;
use flowable_engine::cmd::correlate_message_cmd::CorrelateMessageResult;
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::engine::query::Query;
use serde_json::json;
use std::collections::HashMap;

fn deploy_message_catch_process(engine: &ProcessEngine, deployment_name: &str) -> String {
    let repository_service = engine.get_repository_service();
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <message id="approvalMsg" name="approvalMessage" />
        <process id="messageCorrelateProcess" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="flow1" sourceRef="start" targetRef="waitForMessage" />
            <intermediateCatchEvent id="waitForMessage">
                <messageEventDefinition messageRef="approvalMsg" />
            </intermediateCatchEvent>
            <sequenceFlow id="flow2" sourceRef="waitForMessage" targetRef="reviewTask" />
            <userTask id="reviewTask" name="Review" />
            <sequenceFlow id="flow3" sourceRef="reviewTask" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#;

    repository_service
        .deploy(
            repository_service
                .create_deployment()
                .name(deployment_name.to_string())
                .add_string("message_correlate.bpmn20.xml".to_string(), xml.to_string()),
        )
        .unwrap();
    repository_service.get_process_definition_ids().unwrap()[0].clone()
}

fn deploy_message_start_process(engine: &ProcessEngine, deployment_name: &str) {
    deploy_message_start_process_with_tenant(engine, deployment_name, None);
}

fn deploy_message_start_process_with_tenant(
    engine: &ProcessEngine,
    deployment_name: &str,
    tenant_id: Option<&str>,
) {
    let repository_service = engine.get_repository_service();
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <message id="startMsg" name="startOrder" />
        <process id="messageStartProcess" isExecutable="true">
            <startEvent id="messageStart">
                <messageEventDefinition messageRef="startMsg" />
            </startEvent>
            <sequenceFlow id="flow1" sourceRef="messageStart" targetRef="orderTask" />
            <userTask id="orderTask" name="Process Order" />
            <sequenceFlow id="flow2" sourceRef="orderTask" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#;

    let mut deployment = repository_service
        .create_deployment()
        .name(deployment_name.to_string())
        .add_string(
            "message_start_correlate.bpmn20.xml".to_string(),
            xml.to_string(),
        );
    if let Some(tenant_id) = tenant_id {
        deployment = deployment.tenant_id(tenant_id.to_string());
    }

    repository_service.deploy(deployment).unwrap();
}

fn deploy_receive_task_process(engine: &ProcessEngine, deployment_name: &str) -> String {
    let repository_service = engine.get_repository_service();
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <message id="receiveMsg" name="receiveNotification" />
        <process id="receiveTaskCorrelateProcess" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="flow1" sourceRef="start" targetRef="receiveTask" />
            <receiveTask id="receiveTask" messageRef="receiveMsg" />
            <sequenceFlow id="flow2" sourceRef="receiveTask" targetRef="doneTask" />
            <userTask id="doneTask" name="Done" />
            <sequenceFlow id="flow3" sourceRef="doneTask" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#;

    repository_service
        .deploy(
            repository_service
                .create_deployment()
                .name(deployment_name.to_string())
                .add_string(
                    "receive_task_correlate.bpmn20.xml".to_string(),
                    xml.to_string(),
                ),
        )
        .unwrap();
    repository_service.get_process_definition_ids().unwrap()[0].clone()
}

// ── Test 1: correlate_message matches intermediate catch event ──

#[test]
fn test_correlate_message_matches_intermediate_catch_event() {
    let engine = ProcessEngine::new("correlate-catch-test".to_string());
    let runtime_service = engine.get_runtime_service();
    let task_service = engine.get_task_service();
    let history_service = engine.get_history_service();

    let _pd_id = deploy_message_catch_process(&engine, "Correlate Catch");

    let process_instance = runtime_service
        .start_process_instance(
            runtime_service
                .create_process_instance_builder()
                .process_definition_id(
                    engine
                        .get_repository_service()
                        .get_process_definition_ids()
                        .unwrap()[0]
                        .clone(),
                ),
        )
        .unwrap();

    // Verify waiting
    let wait_states =
        runtime_service.get_event_wait_states_by_process_instance_id(process_instance.id.clone());
    assert_eq!(wait_states.len(), 1);

    // Correlate
    let result = runtime_service
        .correlate_message("approvalMessage".to_string())
        .unwrap();
    match &result {
        CorrelateMessageResult::MatchedExecution {
            process_instance_id,
            ..
        } => assert_eq!(process_instance_id, &process_instance.id),
        _ => panic!("Expected MatchedExecution"),
    }

    // Should have moved to reviewTask
    let tasks = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].task_definition_key, "reviewTask");

    // History should record the correlation variable if any were passed
    let audits = history_service
        .create_historic_audit_log_query()
        .process_instance_id(process_instance.id.clone())
        .list()
        .unwrap();
    assert!(
        !audits.is_empty(),
        "Should have audit entries after correlation"
    );
}

// ── Test 2: correlate_message matches receive task ──

#[test]
fn test_correlate_message_matches_receive_task() {
    let engine = ProcessEngine::new("correlate-receive-test".to_string());
    let runtime_service = engine.get_runtime_service();
    let task_service = engine.get_task_service();

    let _pd_id = deploy_receive_task_process(&engine, "Correlate Receive");

    let process_instance = runtime_service
        .start_process_instance(
            runtime_service
                .create_process_instance_builder()
                .process_definition_id(
                    engine
                        .get_repository_service()
                        .get_process_definition_ids()
                        .unwrap()[0]
                        .clone(),
                ),
        )
        .unwrap();

    // Verify waiting
    let wait_states =
        runtime_service.get_event_wait_states_by_process_instance_id(process_instance.id.clone());
    assert_eq!(wait_states.len(), 1);

    // Correlate
    let result = runtime_service
        .correlate_message("receiveNotification".to_string())
        .unwrap();
    match &result {
        CorrelateMessageResult::MatchedExecution {
            process_instance_id,
            ..
        } => assert_eq!(process_instance_id, &process_instance.id),
        _ => panic!("Expected MatchedExecution"),
    }

    // Should have moved to doneTask
    let tasks = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].task_definition_key, "doneTask");
}

// ── Test 3: correlate_message with wrong name returns NoMatch ──

#[test]
fn test_correlate_message_wrong_name_returns_no_match() {
    let engine = ProcessEngine::new("correlate-no-match-test".to_string());
    let runtime_service = engine.get_runtime_service();

    let _pd_id = deploy_message_catch_process(&engine, "Correlate No Match");

    let _process_instance = runtime_service
        .start_process_instance(
            runtime_service
                .create_process_instance_builder()
                .process_definition_id(
                    engine
                        .get_repository_service()
                        .get_process_definition_ids()
                        .unwrap()[0]
                        .clone(),
                ),
        )
        .unwrap();

    let result = runtime_service
        .correlate_message("wrongMessage".to_string())
        .unwrap();
    match result {
        CorrelateMessageResult::NoMatch => {}
        _ => panic!("Expected NoMatch for wrong message name"),
    }
}

// ── Test 4: correlate_message targets specific process instance ──

#[test]
fn test_correlate_message_targets_specific_process_instance() {
    let engine = ProcessEngine::new("correlate-target-pi-test".to_string());
    let runtime_service = engine.get_runtime_service();
    let task_service = engine.get_task_service();

    let _pd_id = deploy_message_catch_process(&engine, "Correlate Target PI");

    let pd_id = engine
        .get_repository_service()
        .get_process_definition_ids()
        .unwrap()[0]
        .clone();

    let pi1 = runtime_service
        .start_process_instance(
            runtime_service
                .create_process_instance_builder()
                .process_definition_id(pd_id.clone()),
        )
        .unwrap();
    let pi2 = runtime_service
        .start_process_instance(
            runtime_service
                .create_process_instance_builder()
                .process_definition_id(pd_id.clone()),
        )
        .unwrap();

    // Correlate to pi2 specifically
    let result = runtime_service
        .correlate_message_to_process_instance("approvalMessage".to_string(), pi2.id.clone())
        .unwrap();
    match &result {
        CorrelateMessageResult::MatchedExecution {
            process_instance_id,
            ..
        } => assert_eq!(process_instance_id, &pi2.id),
        _ => panic!("Expected MatchedExecution for pi2"),
    }

    // pi1 should still be waiting
    let pi1_wait = runtime_service.get_event_wait_states_by_process_instance_id(pi1.id.clone());
    assert_eq!(pi1_wait.len(), 1, "pi1 should still be waiting");

    // pi2 should have moved on
    let pi2_tasks = task_service
        .get_tasks_by_process_instance_id(pi2.id.clone())
        .unwrap();
    assert_eq!(pi2_tasks.len(), 1);
    assert_eq!(pi2_tasks[0].task_definition_key, "reviewTask");
}

// ── Test 5: correlate_message with business_key filter ──

#[test]
fn test_correlate_message_by_business_key() {
    let engine = ProcessEngine::new("correlate-bk-test".to_string());
    let runtime_service = engine.get_runtime_service();
    let task_service = engine.get_task_service();

    let _pd_id = deploy_message_catch_process(&engine, "Correlate BK");

    let pd_id = engine
        .get_repository_service()
        .get_process_definition_ids()
        .unwrap()[0]
        .clone();

    let pi1 = runtime_service
        .start_process_instance(
            runtime_service
                .create_process_instance_builder()
                .process_definition_id(pd_id.clone())
                .business_key("ORDER-001".to_string()),
        )
        .unwrap();
    let pi2 = runtime_service
        .start_process_instance(
            runtime_service
                .create_process_instance_builder()
                .process_definition_id(pd_id.clone())
                .business_key("ORDER-002".to_string()),
        )
        .unwrap();

    // Correlate by business key ORDER-002
    let result = runtime_service
        .correlate_message_by_business_key("approvalMessage".to_string(), "ORDER-002".to_string())
        .unwrap();
    match &result {
        CorrelateMessageResult::MatchedExecution {
            process_instance_id,
            ..
        } => assert_eq!(process_instance_id, &pi2.id),
        _ => panic!("Expected MatchedExecution for ORDER-002"),
    }

    // pi1 should still be waiting
    let pi1_wait = runtime_service.get_event_wait_states_by_process_instance_id(pi1.id.clone());
    assert_eq!(pi1_wait.len(), 1, "pi1 should still be waiting");

    // pi2 should have moved on
    let pi2_tasks = task_service
        .get_tasks_by_process_instance_id(pi2.id.clone())
        .unwrap();
    assert_eq!(pi2_tasks.len(), 1);
}

// ── Test 6: correlate_message with wrong business_key returns NoMatch ──

#[test]
fn test_correlate_message_wrong_business_key_returns_no_match() {
    let engine = ProcessEngine::new("correlate-wrong-bk-test".to_string());
    let runtime_service = engine.get_runtime_service();

    let _pd_id = deploy_message_catch_process(&engine, "Correlate Wrong BK");

    let pd_id = engine
        .get_repository_service()
        .get_process_definition_ids()
        .unwrap()[0]
        .clone();

    let _pi = runtime_service
        .start_process_instance(
            runtime_service
                .create_process_instance_builder()
                .process_definition_id(pd_id)
                .business_key("ORDER-001".to_string()),
        )
        .unwrap();

    let result = runtime_service
        .correlate_message_by_business_key("approvalMessage".to_string(), "NONEXISTENT".to_string())
        .unwrap();
    match result {
        CorrelateMessageResult::NoMatch => {}
        _ => panic!("Expected NoMatch for wrong business key"),
    }
}

// ── Test 7: correlate_message with variables ──

#[test]
fn test_correlate_message_with_variables() {
    let engine = ProcessEngine::new("correlate-vars-test".to_string());
    let runtime_service = engine.get_runtime_service();
    let task_service = engine.get_task_service();
    let history_service = engine.get_history_service();

    let _pd_id = deploy_message_catch_process(&engine, "Correlate Vars");

    let pd_id = engine
        .get_repository_service()
        .get_process_definition_ids()
        .unwrap()[0]
        .clone();

    let process_instance = runtime_service
        .start_process_instance(
            runtime_service
                .create_process_instance_builder()
                .process_definition_id(pd_id),
        )
        .unwrap();

    let mut variables = HashMap::new();
    variables.insert("approvedBy".to_string(), json!("correlator"));
    variables.insert("approvalNote".to_string(), json!("auto-approved"));

    let result = runtime_service
        .correlate_message_with_variables("approvalMessage".to_string(), variables)
        .unwrap();
    match &result {
        CorrelateMessageResult::MatchedExecution { .. } => {}
        _ => panic!("Expected MatchedExecution"),
    }

    // Variables should be on the execution
    let tasks = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 1);

    let approved_by = runtime_service
        .get_variable(tasks[0].execution_id.clone(), "approvedBy".to_string())
        .unwrap();
    assert_eq!(approved_by, Some(json!("correlator")));

    // History should have the variable
    let historic_vars = history_service
        .create_historic_variable_instance_query()
        .process_instance_id(process_instance.id.clone())
        .variable_name("approvedBy".to_string())
        .list()
        .unwrap();
    assert_eq!(historic_vars.len(), 1);
    assert_eq!(historic_vars[0].value, json!("correlator"));
}

// ── Test 8: correlate_message multi-match only triggers first ──

#[test]
fn test_correlate_message_multi_match_triggers_first_only() {
    let engine = ProcessEngine::new("correlate-multi-test".to_string());
    let runtime_service = engine.get_runtime_service();
    let task_service = engine.get_task_service();

    let _pd_id = deploy_message_catch_process(&engine, "Correlate Multi");

    let pd_id = engine
        .get_repository_service()
        .get_process_definition_ids()
        .unwrap()[0]
        .clone();

    // Start 3 instances
    let pi1 = runtime_service
        .start_process_instance(
            runtime_service
                .create_process_instance_builder()
                .process_definition_id(pd_id.clone()),
        )
        .unwrap();
    let pi2 = runtime_service
        .start_process_instance(
            runtime_service
                .create_process_instance_builder()
                .process_definition_id(pd_id.clone()),
        )
        .unwrap();
    let pi3 = runtime_service
        .start_process_instance(
            runtime_service
                .create_process_instance_builder()
                .process_definition_id(pd_id.clone()),
        )
        .unwrap();

    // First correlate should match exactly one
    let result = runtime_service
        .correlate_message("approvalMessage".to_string())
        .unwrap();
    let matched_pi = match &result {
        CorrelateMessageResult::MatchedExecution {
            process_instance_id,
            ..
        } => process_instance_id.clone(),
        _ => panic!("Expected MatchedExecution"),
    };

    // Second correlate should match a different one
    let result2 = runtime_service
        .correlate_message("approvalMessage".to_string())
        .unwrap();
    let matched_pi2 = match &result2 {
        CorrelateMessageResult::MatchedExecution {
            process_instance_id,
            ..
        } => process_instance_id.clone(),
        _ => panic!("Expected MatchedExecution"),
    };

    assert_ne!(
        matched_pi, matched_pi2,
        "Each correlate should match a different instance"
    );

    // Third correlate should match the remaining one
    let result3 = runtime_service
        .correlate_message("approvalMessage".to_string())
        .unwrap();
    let matched_pi3 = match &result3 {
        CorrelateMessageResult::MatchedExecution {
            process_instance_id,
            ..
        } => process_instance_id.clone(),
        _ => panic!("Expected MatchedExecution"),
    };

    assert_ne!(matched_pi, matched_pi3);
    assert_ne!(matched_pi2, matched_pi3);

    // Fourth correlate should find no match
    let result4 = runtime_service
        .correlate_message("approvalMessage".to_string())
        .unwrap();
    match result4 {
        CorrelateMessageResult::NoMatch => {}
        _ => panic!("Expected NoMatch after all instances triggered"),
    }

    // All instances should have moved on
    for pi_id in [&pi1.id, &pi2.id, &pi3.id] {
        let tasks = task_service
            .get_tasks_by_process_instance_id(pi_id.clone())
            .unwrap();
        assert_eq!(tasks.len(), 1, "Instance {} should have reviewTask", pi_id);
        assert_eq!(tasks[0].task_definition_key, "reviewTask");
    }
}

// ── Test 9: correlate_message_or_start starts new when no match ──

#[test]
fn test_correlate_message_or_start_starts_new_process() {
    let engine = ProcessEngine::new("correlate-or-start-test".to_string());
    let runtime_service = engine.get_runtime_service();
    let task_service = engine.get_task_service();

    deploy_message_start_process(&engine, "Correlate Or Start");

    // No running instances — should start new
    let result = runtime_service
        .correlate_message_or_start("startOrder".to_string())
        .unwrap();
    match &result {
        CorrelateMessageResult::StartedProcess(pi) => {
            assert_eq!(pi.process_definition_key, "messageStartProcess");
        }
        _ => panic!("Expected StartedProcess"),
    }

    // Should have created the orderTask
    let all_tasks = task_service.create_task_query().list().unwrap();
    let order_tasks: Vec<_> = all_tasks
        .iter()
        .filter(|t| t.task_definition_key == "orderTask")
        .collect();
    assert_eq!(order_tasks.len(), 1);
}

#[test]
fn test_correlate_message_or_start_applies_tenant_business_key_and_variables() {
    let engine = ProcessEngine::new("correlate-or-start-options-test".to_string());
    let runtime_service = engine.get_runtime_service();
    let task_service = engine.get_task_service();

    deploy_message_start_process_with_tenant(&engine, "Correlate Or Start Global", None);
    deploy_message_start_process_with_tenant(
        &engine,
        "Correlate Or Start Tenant A",
        Some("tenant-A"),
    );

    let mut variables = HashMap::new();
    variables.insert("orderTotal".to_string(), json!(42));

    let options = CorrelateMessageOptions {
        tenant_id: Some("tenant-A".to_string()),
        business_key: Some("ORDER-START-001".to_string()),
        variables,
        start_new_if_no_match: true,
        ..Default::default()
    };
    let result = runtime_service
        .correlate_message_with_options("startOrder".to_string(), options)
        .unwrap();

    let process_instance = match result {
        CorrelateMessageResult::StartedProcess(pi) => pi,
        _ => panic!("Expected StartedProcess"),
    };
    assert_eq!(process_instance.tenant_id.as_deref(), Some("tenant-A"));
    assert_eq!(
        process_instance.business_key.as_deref(),
        Some("ORDER-START-001")
    );

    let order_total = runtime_service
        .get_variable(process_instance.id.clone(), "orderTotal".to_string())
        .unwrap();
    assert_eq!(order_total, Some(json!(42)));

    let tasks = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].tenant_id.as_deref(), Some("tenant-A"));
}

// ── Test 10: correlate_message_or_start matches existing first ──

#[test]
fn test_correlate_message_or_start_matches_existing_before_starting() {
    let engine = ProcessEngine::new("correlate-or-start-existing-test".to_string());
    let runtime_service = engine.get_runtime_service();
    let task_service = engine.get_task_service();

    let _pd_id = deploy_message_catch_process(&engine, "Correlate Or Start Existing");

    let pd_id = engine
        .get_repository_service()
        .get_process_definition_ids()
        .unwrap()[0]
        .clone();

    let pi = runtime_service
        .start_process_instance(
            runtime_service
                .create_process_instance_builder()
                .process_definition_id(pd_id),
        )
        .unwrap();

    // Should match existing, not start new
    let result = runtime_service
        .correlate_message_or_start("approvalMessage".to_string())
        .unwrap();
    match &result {
        CorrelateMessageResult::MatchedExecution {
            process_instance_id,
            ..
        } => assert_eq!(process_instance_id, &pi.id),
        _ => panic!("Expected MatchedExecution, not StartedProcess"),
    }

    let tasks = task_service
        .get_tasks_by_process_instance_id(pi.id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].task_definition_key, "reviewTask");
}

// ── Test 11: correlate_message with combined filters ──

#[test]
fn test_correlate_message_with_combined_filters() {
    let engine = ProcessEngine::new("correlate-combined-test".to_string());
    let runtime_service = engine.get_runtime_service();

    let _pd_id = deploy_message_catch_process(&engine, "Correlate Combined");

    let pd_id = engine
        .get_repository_service()
        .get_process_definition_ids()
        .unwrap()[0]
        .clone();

    let pi = runtime_service
        .start_process_instance(
            runtime_service
                .create_process_instance_builder()
                .process_definition_id(pd_id)
                .business_key("ORDER-100".to_string())
                .tenant_id("tenant-A".to_string()),
        )
        .unwrap();

    // Wrong tenant
    let options = CorrelateMessageOptions {
        tenant_id: Some("tenant-B".to_string()),
        ..Default::default()
    };
    let result = runtime_service
        .correlate_message_with_options("approvalMessage".to_string(), options)
        .unwrap();
    match result {
        CorrelateMessageResult::NoMatch => {}
        _ => panic!("Expected NoMatch for wrong tenant"),
    }

    // Correct tenant + business key
    let options = CorrelateMessageOptions {
        tenant_id: Some("tenant-A".to_string()),
        business_key: Some("ORDER-100".to_string()),
        ..Default::default()
    };
    let result = runtime_service
        .correlate_message_with_options("approvalMessage".to_string(), options)
        .unwrap();
    match &result {
        CorrelateMessageResult::MatchedExecution {
            process_instance_id,
            ..
        } => assert_eq!(process_instance_id, &pi.id),
        _ => panic!("Expected MatchedExecution for correct tenant+bk"),
    }
}

// ── Test 12: correlate_message no instances at all returns NoMatch ──

#[test]
fn test_correlate_message_no_instances_returns_no_match() {
    let engine = ProcessEngine::new("correlate-empty-test".to_string());
    let runtime_service = engine.get_runtime_service();

    deploy_message_catch_process(&engine, "Correlate Empty");

    let result = runtime_service
        .correlate_message("approvalMessage".to_string())
        .unwrap();
    match result {
        CorrelateMessageResult::NoMatch => {}
        _ => panic!("Expected NoMatch when no instances exist"),
    }
}
