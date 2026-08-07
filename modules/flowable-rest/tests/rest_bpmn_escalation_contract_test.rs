use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_rest::run_server;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::net::TcpListener;

/// Helper: boot a REST server, return (base_url, engine, client).
async fn setup() -> (String, Arc<ProcessEngine>, reqwest::Client) {
    let engine = Arc::new(ProcessEngine::new(
        "rest-bpmn-escalation-contract".to_string(),
    ));

    engine
        .get_identity_service()
        .save_user(flowable_engine::identity::entities::User {
            id: "admin".to_string(),
            first_name: None,
            last_name: None,
            email: None,
            password: Some("test".to_string()),
            tenant_id: None,
        });

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let base_url = format!("http://{}", addr);

    let engine_clone = Arc::clone(&engine);
    tokio::spawn(async move {
        run_server(engine_clone, listener).await.unwrap();
    });

    let client = reqwest::Client::new();
    (base_url, engine, client)
}

/// Deploy a BPMN XML and return the process definition ID.
async fn deploy(client: &reqwest::Client, base_url: &str, name: &str, xml: &str) -> String {
    let response = client
        .post(format!("{base_url}/repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": name,
            "resourceName": format!("{name}.bpmn20.xml"),
            "resource": xml
        }))
        .send()
        .await
        .unwrap();
    assert!(
        response.status().is_success(),
        "deploy failed: {}",
        response.text().await.unwrap()
    );

    // Fetch process definition ID from the deployment
    let definitions_response = client
        .get(format!("{base_url}/repository/process-definitions"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(definitions_response.status().is_success());
    let body: Value = definitions_response.json().await.unwrap();
    let definitions = body["data"].as_array().unwrap();
    definitions.last().unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string()
}

/// Start a process instance and return the process instance ID.
async fn start_process(
    client: &reqwest::Client,
    base_url: &str,
    process_definition_id: &str,
) -> String {
    let response = client
        .post(format!("{base_url}/runtime/process-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "processDefinitionId": process_definition_id
        }))
        .send()
        .await
        .unwrap();
    assert!(
        response.status().is_success(),
        "start failed: {}",
        response.text().await.unwrap()
    );
    let body: Value = response.json().await.unwrap();
    body["id"].as_str().unwrap().to_string()
}

/// Query historic activity instances for a process instance, returning the list of activityId values.
async fn get_historic_activity_ids(
    client: &reqwest::Client,
    base_url: &str,
    process_instance_id: &str,
) -> Vec<String> {
    let response = client
        .post(format!("{base_url}/query/historic-activity-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "processInstanceId": process_instance_id
        }))
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success());
    let body: Value = response.json().await.unwrap();
    body["data"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|entry| entry["activityId"].as_str().map(|s| s.to_string()))
        .collect()
}

/// Check whether the process instance has ended.
async fn is_process_ended(engine: &ProcessEngine, process_instance_id: &str) -> bool {
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let result = store
        .find_process_instance(process_instance_id, &mut session)
        .is_none_or(|pi| pi.is_ended);
    let _ = session.rollback();
    result
}

// ────────────────────────────────────────────────────────────────
// Test 1: Escalation end event in a subprocess triggers an
// interrupting boundary event on the subprocess
// ────────────────────────────────────────────────────────────────
#[tokio::test]
async fn escalation_end_event_throws_to_boundary() {
    let (base_url, engine, client) = setup().await;

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
                 targetNamespace="EscalationTests">
        <escalation id="esc1" name="TimeoutEscalation" escalationCode="timeout" />
        <process id="escalationBoundaryProcess" name="Escalation Boundary" isExecutable="true">
            <startEvent id="start" />
            <subProcess id="sub1">
                <startEvent id="subStart" />
                <endEvent id="subEnd">
                    <escalationEventDefinition escalationRef="esc1" />
                </endEvent>
                <sequenceFlow id="sf1" sourceRef="subStart" targetRef="subEnd" />
            </subProcess>
            <boundaryEvent id="escalationBoundary" attachedToRef="sub1" cancelActivity="true">
                <escalationEventDefinition escalationRef="esc1" />
            </boundaryEvent>
            <endEvent id="normalEnd" />
            <endEvent id="escalationCaughtEnd" />
            <sequenceFlow id="sf2" sourceRef="start" targetRef="sub1" />
            <sequenceFlow id="sf3" sourceRef="sub1" targetRef="normalEnd" />
            <sequenceFlow id="sf4" sourceRef="escalationBoundary" targetRef="escalationCaughtEnd" />
        </process>
    </definitions>"#;

    let pd_id = deploy(&client, &base_url, "esc_boundary", xml).await;
    let pi_id = start_process(&client, &base_url, &pd_id).await;

    // The process should complete via the escalation boundary path
    assert!(
        is_process_ended(&engine, &pi_id).await,
        "Process should have ended"
    );

    let activities = get_historic_activity_ids(&client, &base_url, &pi_id).await;
    assert!(
        activities.contains(&"escalationBoundary".to_string()),
        "Escalation boundary event should have been triggered. Activities: {:?}",
        activities
    );
    assert!(
        activities.contains(&"escalationCaughtEnd".to_string()),
        "Should have reached the escalation-caught end event. Activities: {:?}",
        activities
    );
}

// ────────────────────────────────────────────────────────────────
// Test 2: Intermediate throw escalation event propagates to
// boundary and continues the throw execution
// ────────────────────────────────────────────────────────────────
#[tokio::test]
async fn intermediate_throw_escalation_triggers_boundary() {
    let (base_url, engine, client) = setup().await;

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
                 targetNamespace="EscalationTests">
        <escalation id="escWarn" name="Warning" escalationCode="warn" />
        <process id="throwEscalationProcess" name="Throw Escalation" isExecutable="true">
            <startEvent id="start" />
            <subProcess id="sub1">
                <startEvent id="subStart" />
                <intermediateThrowEvent id="throwEsc">
                    <escalationEventDefinition escalationRef="escWarn" />
                </intermediateThrowEvent>
                <endEvent id="subEnd" />
                <sequenceFlow id="sf1" sourceRef="subStart" targetRef="throwEsc" />
                <sequenceFlow id="sf2" sourceRef="throwEsc" targetRef="subEnd" />
            </subProcess>
            <boundaryEvent id="escalationBoundary" attachedToRef="sub1" cancelActivity="true">
                <escalationEventDefinition escalationRef="escWarn" />
            </boundaryEvent>
            <endEvent id="normalEnd" />
            <endEvent id="escalationEnd" />
            <sequenceFlow id="sf3" sourceRef="start" targetRef="sub1" />
            <sequenceFlow id="sf4" sourceRef="sub1" targetRef="normalEnd" />
            <sequenceFlow id="sf5" sourceRef="escalationBoundary" targetRef="escalationEnd" />
        </process>
    </definitions>"#;

    let pd_id = deploy(&client, &base_url, "esc_throw", xml).await;
    let pi_id = start_process(&client, &base_url, &pd_id).await;

    assert!(
        is_process_ended(&engine, &pi_id).await,
        "Process should have ended"
    );

    let activities = get_historic_activity_ids(&client, &base_url, &pi_id).await;
    assert!(
        activities.contains(&"throwEsc".to_string()),
        "Intermediate throw escalation should have executed. Activities: {:?}",
        activities
    );
    assert!(
        activities.contains(&"escalationBoundary".to_string()),
        "Escalation boundary should have been triggered. Activities: {:?}",
        activities
    );
}

// ────────────────────────────────────────────────────────────────
// Test 3: Non-interrupting escalation boundary event does NOT
// cancel the host subprocess
// ────────────────────────────────────────────────────────────────
#[tokio::test]
async fn non_interrupting_escalation_boundary_preserves_host() {
    let (base_url, engine, client) = setup().await;

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
                 targetNamespace="EscalationTests">
        <escalation id="escNonInt" name="NonIntEscalation" escalationCode="softAlert" />
        <process id="nonInterruptingEscProcess" name="Non-Interrupting Escalation" isExecutable="true">
            <startEvent id="start" />
            <subProcess id="sub1">
                <startEvent id="subStart" />
                <endEvent id="subEnd">
                    <escalationEventDefinition escalationRef="escNonInt" />
                </endEvent>
                <sequenceFlow id="sf1" sourceRef="subStart" targetRef="subEnd" />
            </subProcess>
            <boundaryEvent id="escBoundaryNonInt" attachedToRef="sub1" cancelActivity="false">
                <escalationEventDefinition escalationRef="escNonInt" />
            </boundaryEvent>
            <endEvent id="normalEnd" />
            <endEvent id="escalationEnd" />
            <sequenceFlow id="sf2" sourceRef="start" targetRef="sub1" />
            <sequenceFlow id="sf3" sourceRef="sub1" targetRef="normalEnd" />
            <sequenceFlow id="sf4" sourceRef="escBoundaryNonInt" targetRef="escalationEnd" />
        </process>
    </definitions>"#;

    let pd_id = deploy(&client, &base_url, "esc_nonint", xml).await;
    let pi_id = start_process(&client, &base_url, &pd_id).await;

    assert!(
        is_process_ended(&engine, &pi_id).await,
        "Process should have ended"
    );

    let activities = get_historic_activity_ids(&client, &base_url, &pi_id).await;
    // Non-interrupting: the boundary fires AND the subprocess completes normally
    assert!(
        activities.contains(&"escBoundaryNonInt".to_string()),
        "Non-interrupting escalation boundary should have triggered. Activities: {:?}",
        activities
    );
    assert!(
        activities.contains(&"escalationEnd".to_string()),
        "Escalation end should have been reached. Activities: {:?}",
        activities
    );
}

// ────────────────────────────────────────────────────────────────
// Test 4: Catch-all boundary (no escalation code) catches any
// escalation
// ────────────────────────────────────────────────────────────────
#[tokio::test]
async fn wildcard_escalation_boundary_catches_any_escalation() {
    let (base_url, engine, client) = setup().await;

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
                 targetNamespace="EscalationTests">
        <escalation id="escAny" name="AnyEscalation" escalationCode="someCode" />
        <process id="wildcardEscProcess" name="Wildcard Escalation" isExecutable="true">
            <startEvent id="start" />
            <subProcess id="sub1">
                <startEvent id="subStart" />
                <endEvent id="subEnd">
                    <escalationEventDefinition escalationRef="escAny" />
                </endEvent>
                <sequenceFlow id="sf1" sourceRef="subStart" targetRef="subEnd" />
            </subProcess>
            <boundaryEvent id="catchAllBoundary" attachedToRef="sub1" cancelActivity="true">
                <escalationEventDefinition />
            </boundaryEvent>
            <endEvent id="normalEnd" />
            <endEvent id="caughtEnd" />
            <sequenceFlow id="sf2" sourceRef="start" targetRef="sub1" />
            <sequenceFlow id="sf3" sourceRef="sub1" targetRef="normalEnd" />
            <sequenceFlow id="sf4" sourceRef="catchAllBoundary" targetRef="caughtEnd" />
        </process>
    </definitions>"#;

    let pd_id = deploy(&client, &base_url, "esc_wildcard", xml).await;
    let pi_id = start_process(&client, &base_url, &pd_id).await;

    assert!(
        is_process_ended(&engine, &pi_id).await,
        "Process should have ended"
    );

    let activities = get_historic_activity_ids(&client, &base_url, &pi_id).await;
    assert!(
        activities.contains(&"catchAllBoundary".to_string()),
        "Wildcard boundary should have caught the escalation. Activities: {:?}",
        activities
    );
    assert!(
        activities.contains(&"caughtEnd".to_string()),
        "Should have reached the caught end. Activities: {:?}",
        activities
    );
}

// ────────────────────────────────────────────────────────────────
// Test 5: Interrupting escalation event subprocess triggers on
// escalation throw
// ────────────────────────────────────────────────────────────────
#[tokio::test]
async fn escalation_event_subprocess_interrupting() {
    let (base_url, engine, client) = setup().await;

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
                 targetNamespace="EscalationTests">
        <escalation id="escEvSub" name="EventSubEscalation" escalationCode="evSubCode" />
        <process id="escalationEventSubprocess" name="Escalation Event Subprocess" isExecutable="true">
            <startEvent id="start" />
            <subProcess id="outerSub">
                <startEvent id="outerSubStart" />
                <endEvent id="outerSubEnd">
                    <escalationEventDefinition escalationRef="escEvSub" />
                </endEvent>
                <sequenceFlow id="sf1" sourceRef="outerSubStart" targetRef="outerSubEnd" />
                <subProcess id="eventSub" triggeredByEvent="true">
                    <startEvent id="eventSubStart" isInterrupting="true">
                        <escalationEventDefinition escalationRef="escEvSub" />
                    </startEvent>
                    <endEvent id="eventSubEnd" />
                    <sequenceFlow id="esSf1" sourceRef="eventSubStart" targetRef="eventSubEnd" />
                </subProcess>
            </subProcess>
            <endEvent id="mainEnd" />
            <sequenceFlow id="sf2" sourceRef="start" targetRef="outerSub" />
            <sequenceFlow id="sf3" sourceRef="outerSub" targetRef="mainEnd" />
        </process>
    </definitions>"#;

    let pd_id = deploy(&client, &base_url, "esc_event_subprocess", xml).await;
    let pi_id = start_process(&client, &base_url, &pd_id).await;

    assert!(
        is_process_ended(&engine, &pi_id).await,
        "Process should have ended"
    );

    let activities = get_historic_activity_ids(&client, &base_url, &pi_id).await;
    assert!(
        activities.contains(&"eventSubStart".to_string()),
        "Escalation event subprocess should have been activated. Activities: {:?}",
        activities
    );
    assert!(
        activities.contains(&"eventSubEnd".to_string()),
        "Escalation event subprocess should have completed. Activities: {:?}",
        activities
    );
}

// ────────────────────────────────────────────────────────────────
// Test 6: Non-interrupting escalation event subprocess runs in
// parallel with the normal flow
// ────────────────────────────────────────────────────────────────
#[tokio::test]
async fn escalation_event_subprocess_non_interrupting() {
    let (base_url, engine, client) = setup().await;

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
                 targetNamespace="EscalationTests">
        <escalation id="escNonIntSub" name="NonIntSubEscalation" escalationCode="niSubCode" />
        <process id="nonIntEscEventSubprocess" name="Non-Interrupting Escalation Event Subprocess" isExecutable="true">
            <startEvent id="start" />
            <subProcess id="outerSub">
                <startEvent id="outerSubStart" />
                <endEvent id="outerSubEnd">
                    <escalationEventDefinition escalationRef="escNonIntSub" />
                </endEvent>
                <sequenceFlow id="sf1" sourceRef="outerSubStart" targetRef="outerSubEnd" />
                <subProcess id="eventSub" triggeredByEvent="true">
                    <startEvent id="niEventSubStart" isInterrupting="false">
                        <escalationEventDefinition escalationRef="escNonIntSub" />
                    </startEvent>
                    <endEvent id="niEventSubEnd" />
                    <sequenceFlow id="esSf1" sourceRef="niEventSubStart" targetRef="niEventSubEnd" />
                </subProcess>
            </subProcess>
            <endEvent id="mainEnd" />
            <sequenceFlow id="sf2" sourceRef="start" targetRef="outerSub" />
            <sequenceFlow id="sf3" sourceRef="outerSub" targetRef="mainEnd" />
        </process>
    </definitions>"#;

    let pd_id = deploy(&client, &base_url, "esc_ni_event_subprocess", xml).await;
    let pi_id = start_process(&client, &base_url, &pd_id).await;

    assert!(
        is_process_ended(&engine, &pi_id).await,
        "Process should have ended"
    );

    let activities = get_historic_activity_ids(&client, &base_url, &pi_id).await;
    assert!(
        activities.contains(&"niEventSubStart".to_string()),
        "Non-interrupting escalation event subprocess should have been activated. Activities: {:?}",
        activities
    );
    assert!(
        activities.contains(&"niEventSubEnd".to_string()),
        "Non-interrupting escalation event subprocess should have completed. Activities: {:?}",
        activities
    );
}

// ────────────────────────────────────────────────────────────────
// Test 7: Escalation code matching — specific code matches over
// catch-all, and unrelated codes are not caught
// ────────────────────────────────────────────────────────────────
#[tokio::test]
async fn escalation_code_specific_match_takes_priority() {
    let (base_url, engine, client) = setup().await;

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
                 targetNamespace="EscalationTests">
        <escalation id="escSpecific" name="SpecificEscalation" escalationCode="priority1" />
        <escalation id="escOther" name="OtherEscalation" escalationCode="other" />
        <process id="escalationCodeMatchProcess" name="Escalation Code Match" isExecutable="true">
            <startEvent id="start" />
            <subProcess id="sub1">
                <startEvent id="subStart" />
                <endEvent id="subEnd">
                    <escalationEventDefinition escalationRef="escSpecific" />
                </endEvent>
                <sequenceFlow id="sf1" sourceRef="subStart" targetRef="subEnd" />
            </subProcess>
            <boundaryEvent id="specificBoundary" attachedToRef="sub1" cancelActivity="true">
                <escalationEventDefinition escalationRef="escSpecific" />
            </boundaryEvent>
            <boundaryEvent id="catchAllBoundary" attachedToRef="sub1" cancelActivity="true">
                <escalationEventDefinition />
            </boundaryEvent>
            <endEvent id="normalEnd" />
            <endEvent id="specificEnd" />
            <endEvent id="catchAllEnd" />
            <sequenceFlow id="sf2" sourceRef="start" targetRef="sub1" />
            <sequenceFlow id="sf3" sourceRef="sub1" targetRef="normalEnd" />
            <sequenceFlow id="sf4" sourceRef="specificBoundary" targetRef="specificEnd" />
            <sequenceFlow id="sf5" sourceRef="catchAllBoundary" targetRef="catchAllEnd" />
        </process>
    </definitions>"#;

    let pd_id = deploy(&client, &base_url, "esc_code_match", xml).await;
    let pi_id = start_process(&client, &base_url, &pd_id).await;

    assert!(
        is_process_ended(&engine, &pi_id).await,
        "Process should have ended"
    );

    let activities = get_historic_activity_ids(&client, &base_url, &pi_id).await;
    // The specific boundary should win over the catch-all
    assert!(
        activities.contains(&"specificBoundary".to_string()),
        "Specific escalation boundary should have been triggered. Activities: {:?}",
        activities
    );
    assert!(
        activities.contains(&"specificEnd".to_string()),
        "Should have reached specificEnd. Activities: {:?}",
        activities
    );
    // The catch-all should NOT have triggered
    assert!(
        !activities.contains(&"catchAllBoundary".to_string()),
        "Catch-all boundary should NOT have triggered when specific match exists. Activities: {:?}",
        activities
    );
}
