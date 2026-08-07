//! P105 — XML-deployed `<sendTask>` and `<manualTask>` runtime semantics.
//!
//! Java reference: `SendTaskParseHandler.java:37-56` (mail/dmn/none dispatch),
//! `ManualTaskActivityBehavior` (pass-through), `ContinueProcessOperation.java:172-181`
//! (null behavior → take outgoing sequence flows).

use flowable_dmn_engine::{
    DmnDecision, DmnDeploymentRequest, DmnEngine, DmnHitPolicy, DmnInputClause, DmnModel,
    DmnOutputClause, DmnRule, DmnRuleInputEntry, DmnRuleOutputEntry, DmnUnaryTest,
};
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::service::config::ProcessEngineConfiguration;
use serde_json::json;
use std::sync::Arc;

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
        vec![DmnRule::new(
            "rule-1",
            vec![DmnRuleInputEntry::new(DmnUnaryTest::Equals(json!(730)))],
            vec![
                DmnRuleOutputEntry::new(json!(true)),
                DmnRuleOutputEntry::new(json!("LOW")),
            ],
        )],
    )
}

fn deploy_decision(engine: &DmnEngine, name: &str, decision: DmnDecision) {
    engine
        .deploy(DmnDeploymentRequest::new(name).with_resource(
            format!("{name}.dmn"),
            DmnModel::new(vec![decision]),
        ))
        .expect("dmn deploy");
}

/// P105 — sendTask `flowable:type="mail"` executes the mail behavior (same
/// helper as serviceTask mail) and continues the process.
#[test]
fn send_task_mail_sends_mail_and_continues() {
    let process_engine = ProcessEngine::new("default".to_string());
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();
    let task_service = process_engine.get_task_service();

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
                 xmlns:flowable="http://flowable.org/bpmn"
                 targetNamespace="Examples">
        <process id="sendMailProcess" name="Send Mail Process">
            <startEvent id="startEvent1" />
            <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="sendTask1" />
            <sendTask id="sendTask1" name="Notify Ops" flowable:type="mail">
                <extensionElements>
                    <flowable:to>ops@example.flowable.local</flowable:to>
                    <flowable:from>noreply@example.flowable.local</flowable:from>
                    <flowable:subject>Deployment finished</flowable:subject>
                    <flowable:text>Process deployment completed.</flowable:text>
                </extensionElements>
            </sendTask>
            <sequenceFlow id="flow2" sourceRef="sendTask1" targetRef="userTask1" />
            <userTask id="userTask1" name="Review Mail Result" />
        </process>
    </definitions>"#;

    let builder = repository_service
        .create_deployment()
        .name("Send Mail Deployment".to_string())
        .add_string("sendMailProcess.bpmn20.xml".to_string(), xml.to_string());
    repository_service.deploy(builder).unwrap();
    let process_definition_id = repository_service.get_process_definition_ids().unwrap()[0].clone();

    let process_instance = runtime_service
        .start_process_instance(
            runtime_service
                .create_process_instance_builder()
                .process_definition_id(process_definition_id)
                .name("Send Mail Instance".to_string()),
        )
        .unwrap();

    let tasks = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 1, "sendTask mail should continue into the user task");
    assert_eq!(tasks[0].name, "Review Mail Result");

    let runtime_store = process_engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let outbox = runtime_store.list_mail_outbox_records(&mut session);
    assert_eq!(outbox.len(), 1, "sendTask mail should create an outbox record");
    assert_eq!(outbox[0].subject, "Deployment finished");
}

/// P105 — a sendTask without `type` is only warned by Java
/// (SendTaskParseHandler.java:55) and then passes through: it must deploy and
/// run to completion without sending mail.
#[test]
fn send_task_without_type_passes_through() {
    let process_engine = ProcessEngine::new("default".to_string());
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
                 xmlns:flowable="http://flowable.org/bpmn"
                 targetNamespace="Examples">
        <process id="sendNoTypeProcess" name="No Type Send">
            <startEvent id="startEvent1" />
            <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="sendTask1" />
            <sendTask id="sendTask1" name="Plain Send" />
            <sequenceFlow id="flow2" sourceRef="sendTask1" targetRef="endEvent1" />
            <endEvent id="endEvent1" />
        </process>
    </definitions>"#;

    let builder = repository_service
        .create_deployment()
        .name("Send No Type Deployment".to_string())
        .add_string("sendNoTypeProcess.bpmn20.xml".to_string(), xml.to_string());
    repository_service.deploy(builder).unwrap();
    let process_definition_id = repository_service.get_process_definition_ids().unwrap()[0].clone();

    let process_instance = runtime_service
        .start_process_instance(
            runtime_service
                .create_process_instance_builder()
                .process_definition_id(process_definition_id)
                .name("Send No Type Instance".to_string()),
        )
        .unwrap();

    let runtime_store = process_engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let stored = runtime_store
        .find_process_instance(&process_instance.id, &mut session)
        .expect("process instance should be in runtime store");
    assert!(
        stored.is_ended,
        "no-type sendTask should pass through to the end event"
    );
    assert!(
        runtime_store.list_mail_outbox_records(&mut session).is_empty(),
        "no-type sendTask must not send mail"
    );
}

/// P105 — sendTask `flowable:type="dmn"` executes the same DMN behavior as
/// serviceTask type=dmn (DmnActivityBehavior) and writes decision outputs.
#[test]
fn send_task_dmn_executes_decision_and_writes_outputs() {
    let dmn = Arc::new(DmnEngine::new_in_memory().expect("dmn"));
    deploy_decision(&dmn, "loan", loan_eligibility_decision());
    let config = ProcessEngineConfiguration {
        dmn_engine: Some(dmn),
        ..ProcessEngineConfiguration::default()
    };
    let process_engine = ProcessEngine::new_with_config("default".to_string(), config);
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
                 xmlns:flowable="http://flowable.org/bpmn"
                 targetNamespace="Examples">
        <process id="sendDmnProcess" name="Send DMN Process">
            <startEvent id="startEvent1" />
            <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="sendTask1" />
            <sendTask id="sendTask1" name="Evaluate Loan" flowable:type="dmn">
                <extensionElements>
                    <flowable:field name="decisionTableReferenceKey">
                        <flowable:string>loanEligibility</flowable:string>
                    </flowable:field>
                </extensionElements>
            </sendTask>
            <sequenceFlow id="flow2" sourceRef="sendTask1" targetRef="endEvent1" />
            <endEvent id="endEvent1" />
        </process>
    </definitions>"#;

    let builder = repository_service
        .create_deployment()
        .name("Send DMN Deployment".to_string())
        .add_string("sendDmnProcess.bpmn20.xml".to_string(), xml.to_string());
    repository_service.deploy(builder).unwrap();
    let process_definition_id = repository_service.get_process_definition_ids().unwrap()[0].clone();

    let process_instance = runtime_service
        .start_process_instance(
            runtime_service
                .create_process_instance_builder()
                .process_definition_id(process_definition_id)
                .name("Send DMN Instance".to_string())
                .variable("creditScore".to_string(), json!(730)),
        )
        .unwrap();

    let runtime_store = process_engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let stored = runtime_store
        .find_process_instance(&process_instance.id, &mut session)
        .expect("process instance should be in runtime store");
    assert!(stored.is_ended, "sendTask dmn should complete the process");

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

/// P105 — XML-deployed manualTask uses the existing pass-through behavior
/// (Java ManualTaskActivityBehavior extends TaskActivityBehavior).
#[test]
fn manual_task_xml_deploy_passes_through() {
    let process_engine = ProcessEngine::new("default".to_string());
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
                 targetNamespace="Examples">
        <process id="manualTaskXmlProcess" name="Manual Task XML">
            <startEvent id="startEvent1" />
            <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="manualTask1" />
            <manualTask id="manualTask1" name="Review Manually" />
            <sequenceFlow id="flow2" sourceRef="manualTask1" targetRef="endEvent1" />
            <endEvent id="endEvent1" />
        </process>
    </definitions>"#;

    let builder = repository_service
        .create_deployment()
        .name("Manual Task XML Deployment".to_string())
        .add_string("manualTaskXmlProcess.bpmn20.xml".to_string(), xml.to_string());
    repository_service.deploy(builder).unwrap();
    let process_definition_id = repository_service.get_process_definition_ids().unwrap()[0].clone();

    let process_instance = runtime_service
        .start_process_instance(
            runtime_service
                .create_process_instance_builder()
                .process_definition_id(process_definition_id)
                .name("Manual Task XML Instance".to_string()),
        )
        .unwrap();

    let runtime_store = process_engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let stored = runtime_store
        .find_process_instance(&process_instance.id, &mut session)
        .expect("process instance should be in runtime store");
    assert!(
        stored.is_ended,
        "manualTask should pass through to the end event (matching the programmatic semantics)"
    );
}

/// P105 — core regression: a deployed model containing both sendTask and
/// manualTask must keep every node and run the process end-to-end. Before the
/// converter arms existed, both node types were silently dropped, leaving
/// sequence flows dangling.
#[test]
fn mixed_send_and_manual_tasks_deploy_and_run_to_completion() {
    let process_engine = ProcessEngine::new("default".to_string());
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
                 xmlns:flowable="http://flowable.org/bpmn"
                 targetNamespace="Examples">
        <process id="mixedSendManualProcess" name="Mixed Process">
            <startEvent id="startEvent1" />
            <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="sendTask1" />
            <sendTask id="sendTask1" name="Notify" flowable:type="mail">
                <extensionElements>
                    <flowable:to>ops@example.flowable.local</flowable:to>
                    <flowable:subject>Mixed</flowable:subject>
                    <flowable:text>body</flowable:text>
                </extensionElements>
            </sendTask>
            <sequenceFlow id="flow2" sourceRef="sendTask1" targetRef="manualTask1" />
            <manualTask id="manualTask1" name="Review" />
            <sequenceFlow id="flow3" sourceRef="manualTask1" targetRef="endEvent1" />
            <endEvent id="endEvent1" />
        </process>
    </definitions>"#;

    let builder = repository_service
        .create_deployment()
        .name("Mixed Deployment".to_string())
        .add_string("mixedSendManualProcess.bpmn20.xml".to_string(), xml.to_string());
    repository_service.deploy(builder).unwrap();
    let process_definition_id = repository_service.get_process_definition_ids().unwrap()[0].clone();

    let process_instance = runtime_service
        .start_process_instance(
            runtime_service
                .create_process_instance_builder()
                .process_definition_id(process_definition_id)
                .name("Mixed Instance".to_string()),
        )
        .unwrap();

    let runtime_store = process_engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let stored = runtime_store
        .find_process_instance(&process_instance.id, &mut session)
        .expect("process instance should be in runtime store");
    assert!(
        stored.is_ended,
        "sendTask (mail) + manualTask must both execute and reach the end event"
    );
    assert_eq!(
        runtime_store.list_mail_outbox_records(&mut session).len(),
        1,
        "the sendTask mail should have been sent exactly once"
    );
}

/// P105 — deliberate deviation: webservice sendTask is not ported (Java
/// WebServiceActivityBehavior is a legacy module), so deployment must fail with
/// a clear error instead of silently dropping the node.
#[test]
fn send_task_webservice_deployment_is_rejected() {
    let process_engine = ProcessEngine::new("default".to_string());
    let repository_service = process_engine.get_repository_service();

    let xml = r###"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
                 xmlns:flowable="http://flowable.org/bpmn"
                 targetNamespace="http://example.flowable.local/services">
        <process id="sendWsProcess" name="WebService Send">
            <startEvent id="startEvent1" />
            <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="sendTask1" />
            <sendTask id="sendTask1" name="Invoke WS"
                      implementation="##WebService"
                      operationRef="tns:myOperation" />
            <sequenceFlow id="flow2" sourceRef="sendTask1" targetRef="endEvent1" />
            <endEvent id="endEvent1" />
        </process>
    </definitions>"###;

    let builder = repository_service
        .create_deployment()
        .name("Send WS Deployment".to_string())
        .add_string("sendWsProcess.bpmn20.xml".to_string(), xml.to_string());
    let err = repository_service.deploy(builder).unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("webservice") && msg.contains("not supported"),
        "expected a clear webservice rejection, got: {msg}"
    );
}
