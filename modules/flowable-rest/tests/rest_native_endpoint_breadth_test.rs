use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::identity::entities::{BatchEntity, BatchPartEntity};
use flowable_engine::persistence::runtime_store::RuntimeTimerJobState;
use flowable_rest::run_server;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::net::TcpListener;

const PROCESS_NO_DECISION_BPMN: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             targetNamespace="Examples">
    <process id="nativeProcessNoDecision" name="Native process without decision" isExecutable="true">
        <startEvent id="startEvent" />
        <sequenceFlow id="flow1" sourceRef="startEvent" targetRef="endEvent" />
        <endEvent id="endEvent" />
    </process>
</definitions>"#;

const PROCESS_WITH_DECISION: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
    <process id="nativeProcessWithDecision" name="Native process with decision" isExecutable="true">
        <startEvent id="start" />
        <sequenceFlow id="flow1" sourceRef="start" targetRef="eligibility" />
        <businessRuleTask id="eligibility" name="Eligibility" flowable:decisionRef="breadthLoanDecision" />
        <sequenceFlow id="flow2" sourceRef="eligibility" targetRef="end" />
        <endEvent id="end" />
    </process>
</definitions>"#;

const DECISION_DMN: &str = r#"
<definitions xmlns="https://www.omg.org/spec/DMN/20191111/MODEL/"
             id="breadth-loan-defs"
             name="Breadth Loan Decisions"
             namespace="http://flowable.org/dmn">
  <decision id="breadthLoanDecision" name="Breadth Loan Decision">
    <decisionTable id="breadthLoanDecisionTable" hitPolicy="FIRST">
      <input id="breadthInput1" label="Credit score">
        <inputExpression id="breadthInputExpression1" typeRef="number">
          <text>creditScore</text>
        </inputExpression>
      </input>
      <output id="breadthOutput1" label="Approved" name="approved" typeRef="boolean" />
      <rule id="breadthRule1">
        <inputEntry id="breadthInputEntry1"><text>730</text></inputEntry>
        <outputEntry id="breadthOutputEntry1"><text>true</text></outputEntry>
      </rule>
    </decisionTable>
  </decision>
</definitions>
"#;

async fn spawn_server() -> (Arc<ProcessEngine>, String, reqwest::Client) {
    let engine = Arc::new(ProcessEngine::new(
        "rest-native-endpoint-breadth".to_string(),
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
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let engine_for_server = Arc::clone(&engine);
    tokio::spawn(async move {
        run_server(engine_for_server, listener).await.unwrap();
    });

    (engine, base_url, reqwest::Client::new())
}

async fn deploy_no_decision_process(client: &reqwest::Client, base_url: &str) {
    let response = client
        .post(format!("{base_url}/repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&serde_json::json!({
            "name": "Native process without decision",
            "resourceName": "native-process-no-decision.bpmn20.xml",
            "resource": PROCESS_NO_DECISION_BPMN
        }))
        .send()
        .await
        .unwrap();
    // P109: deploy (both JSON superset and multipart paths) returns 201.
    assert_eq!(response.status(), reqwest::StatusCode::CREATED);
}

#[tokio::test]
async fn process_definition_decision_tables_endpoint_returns_empty_paged_response() {
    let (engine, base_url, client) = spawn_server().await;
    deploy_no_decision_process(&client, &base_url).await;

    let process_definition_id = engine
        .get_repository_service()
        .latest_process_definition_by_key("nativeProcessNoDecision", None)
        .unwrap()
        .unwrap()
        .id;

    let response = client
        .get(format!(
            "{base_url}/repository/process-definitions/{process_definition_id}/decision-tables"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["start"], 0);
    assert_eq!(body["total"], 0);
    assert_eq!(body["data"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn process_definition_decisions_endpoint_returns_empty_paged_response() {
    let (engine, base_url, client) = spawn_server().await;
    deploy_no_decision_process(&client, &base_url).await;

    let process_definition_id = engine
        .get_repository_service()
        .latest_process_definition_by_key("nativeProcessNoDecision", None)
        .unwrap()
        .unwrap()
        .id;

    let response = client
        .get(format!(
            "{base_url}/repository/process-definitions/{process_definition_id}/decisions"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["start"], 0);
    assert_eq!(body["total"], 0);
    assert_eq!(body["data"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn decision_tables_endpoint_rejects_missing_process_definition() {
    let (_engine, base_url, client) = spawn_server().await;

    let response = client
        .get(format!(
            "{base_url}/repository/process-definitions/missing-process-def-id/decision-tables"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn decisions_endpoint_rejects_missing_process_definition() {
    let (_engine, base_url, client) = spawn_server().await;

    let response = client
        .get(format!(
            "{base_url}/repository/process-definitions/missing-process-def-id/decisions"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn management_batch_get_endpoint_returns_single_batch_payload() {
    let (engine, base_url, client) = spawn_server().await;
    let batch_service = engine.get_batch_service();
    batch_service.create_batch(BatchEntity {
        id: "batch-breadth-1".to_string(),
        batch_type: "processMigration".to_string(),
        search_key: Some("breadth-search".to_string()),
        search_key2: Some("breadth-search-2".to_string()),
        status: "in-progress".to_string(),
        total_items: 1,
        items_processed: 0,
        create_time: 1_775_000_000_000,
        end_time: None,
        tenant_id: Some("tenant-breadth".to_string()),
        batch_document_json: Some(r#"{"migration":"breadth"}"#.to_string()),
    });
    batch_service.create_batch_part(BatchPartEntity {
        id: "batch-breadth-part-1".to_string(),
        batch_id: "batch-breadth-1".to_string(),
        batch_type: "processMigration".to_string(),
        search_key: Some("breadth-part-search".to_string()),
        search_key2: None,
        scope_id: Some("breadth-scope-1".to_string()),
        sub_scope_id: None,
        scope_type: Some("bpmn".to_string()),
        create_time: 1_775_000_001_000,
        complete_time: None,
        status: "waiting".to_string(),
        tenant_id: Some("tenant-breadth".to_string()),
        batch_part_document_json: Some(r#"{"part":"breadth"}"#.to_string()),
    });

    let response = client
        .get(format!("{base_url}/management/batches/batch-breadth-1"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["id"], "batch-breadth-1");
    assert_eq!(body["batchType"], "processMigration");
    assert_eq!(body["status"], "in-progress");
    assert_eq!(body["totalItems"], 1);
    assert_eq!(body["itemsProcessed"], 0);
    assert_eq!(body["tenantId"], "tenant-breadth");
    assert_eq!(body["url"], "/management/batches/batch-breadth-1");
    let create_time = body["createTime"]
        .as_str()
        .expect("createTime should be a string");
    assert!(!create_time.is_empty());
    assert!(body["completeTime"].is_null());
    assert!(body["endTime"].is_null());

    let missing = client
        .get(format!("{base_url}/management/batches/missing-batch-id"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(missing.status(), reqwest::StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn management_batch_get_endpoint_reflects_completed_status_and_complete_time() {
    let (engine, base_url, client) = spawn_server().await;
    let batch_service = engine.get_batch_service();
    batch_service.create_batch(BatchEntity {
        id: "batch-breadth-completed".to_string(),
        batch_type: "asyncHistory".to_string(),
        search_key: Some("completed-search".to_string()),
        search_key2: None,
        status: "completed".to_string(),
        total_items: 4,
        items_processed: 4,
        create_time: 1_775_000_000_000,
        end_time: Some(1_775_000_010_000),
        tenant_id: None,
        batch_document_json: None,
    });

    let response = client
        .get(format!(
            "{base_url}/management/batches/batch-breadth-completed"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["id"], "batch-breadth-completed");
    assert_eq!(body["status"], "completed");
    assert_eq!(body["totalItems"], 4);
    assert_eq!(body["itemsProcessed"], 4);
    let complete_time = body["completeTime"]
        .as_str()
        .expect("completeTime should be a string");
    assert!(!complete_time.is_empty());
    let end_time = body["endTime"]
        .as_str()
        .expect("endTime should be a string");
    assert!(!end_time.is_empty());
    assert!(body["tenantId"].is_null());
}

#[tokio::test]
async fn management_timer_job_exception_stacktrace_endpoint_returns_error_details() {
    let (engine, base_url, client) = spawn_server().await;
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    store.insert_timer_job_state(
        &RuntimeTimerJobState {
            timer_job_id: "timer-breadth-failed".to_string(),
            process_instance_id: "process-breadth-1".to_string(),
            execution_id: "execution-breadth-1".to_string(),
            activity_id: "activity-breadth-timer".to_string(),
            job_state: Some("timer".to_string()),
            is_boundary: false,
            attached_activity_id: None,
            cancel_activity: false,
            time_duration: Some("PT5M".to_string()),
            time_date: None,
            time_cycle: None,
            due_time: Some(1_775_000_000_000),
            lock_owner: None,
            lock_time: None,
            lock_expiration_time: None,
            retries: Some(1),
            error_message: Some("Timer failed for breadth test".to_string()),
            error_details: Some(
                "flowable_runtime_error: timer breadth failure\n\tat TimerBoundary.execute"
                    .to_string(),
            ),
            category: None,
            ..Default::default()
},
        &mut session,
    );
    session.flush_and_commit().unwrap();

    let mut session = store.create_session().unwrap();
    store.insert_timer_job_state(
        &RuntimeTimerJobState {
            timer_job_id: "timer-breadth-clean".to_string(),
            process_instance_id: "process-breadth-2".to_string(),
            execution_id: "execution-breadth-2".to_string(),
            activity_id: "activity-breadth-timer-clean".to_string(),
            job_state: Some("timer".to_string()),
            is_boundary: false,
            attached_activity_id: None,
            cancel_activity: false,
            time_duration: Some("PT5M".to_string()),
            time_date: None,
            time_cycle: None,
            due_time: Some(1_775_000_000_000),
            lock_owner: None,
            lock_time: None,
            lock_expiration_time: None,
            retries: Some(1),
            error_message: None,
            error_details: None,
            category: None,
            ..Default::default()
},
        &mut session,
    );
    session.flush_and_commit().unwrap();

    let response = client
        .get(format!(
            "{base_url}/management/timer-jobs/timer-breadth-failed/exception-stacktrace"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert_eq!(
        response.headers().get("content-type").unwrap(),
        "text/plain"
    );
    let body = response.text().await.unwrap();
    assert!(body.contains("timer breadth failure"));

    let no_stacktrace = client
        .get(format!(
            "{base_url}/management/timer-jobs/timer-breadth-clean/exception-stacktrace"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(no_stacktrace.status(), reqwest::StatusCode::NOT_FOUND);

    let missing = client
        .get(format!(
            "{base_url}/management/timer-jobs/missing-timer-id/exception-stacktrace"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(missing.status(), reqwest::StatusCode::NOT_FOUND);
}

async fn deploy_process_and_dmn(client: &reqwest::Client, base_url: &str) -> (String, String) {
    let deploy_process = client
        .post(format!("{base_url}/repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "Native process with decision",
            "resourceName": "native-process-with-decision.bpmn20.xml",
            "resource": PROCESS_WITH_DECISION
        }))
        .send()
        .await
        .unwrap();
    // P109: deploy (both JSON superset and multipart paths) returns 201.
    assert_eq!(deploy_process.status(), reqwest::StatusCode::CREATED);

    let deploy_dmn = client
        .post(format!("{base_url}/dmn-repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "Breadth loan decision",
            "resourceName": "breadth-loan-decision.dmn",
            "resource": DECISION_DMN
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(deploy_dmn.status(), reqwest::StatusCode::CREATED);

    let process_response = client
        .get(format!(
            "{base_url}/repository/process-definitions?key=nativeProcessWithDecision&latest=true"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(process_response.status(), reqwest::StatusCode::OK);
    let process_body: Value = process_response.json().await.unwrap();
    let process_definition_id = process_body["data"][0]["id"]
        .as_str()
        .expect("process definition id")
        .to_string();

    let decision_response = client
        .get(format!(
            "{base_url}/dmn-repository/decision-tables?key=breadthLoanDecision&latest=true"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(decision_response.status(), reqwest::StatusCode::OK);
    let decision_body: Value = decision_response.json().await.unwrap();
    let decision_id = decision_body["data"][0]["id"]
        .as_str()
        .expect("decision id")
        .to_string();

    (process_definition_id, decision_id)
}

#[tokio::test]
async fn process_definition_decision_tables_endpoint_returns_linked_decision() {
    let (_engine, base_url, client) = spawn_server().await;
    let (process_definition_id, decision_id) = deploy_process_and_dmn(&client, &base_url).await;

    let response = client
        .get(format!(
            "{base_url}/repository/process-definitions/{process_definition_id}/decision-tables"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["total"], 1);
    assert_eq!(body["data"].as_array().unwrap().len(), 1);
    assert_eq!(body["data"][0]["id"], decision_id);
    assert_eq!(body["data"][0]["key"], "breadthLoanDecision");
    assert_eq!(body["data"][0]["name"], "Breadth Loan Decision");
}

#[tokio::test]
async fn process_definition_decisions_endpoint_returns_linked_decision() {
    let (_engine, base_url, client) = spawn_server().await;
    let (process_definition_id, decision_id) = deploy_process_and_dmn(&client, &base_url).await;

    let response = client
        .get(format!(
            "{base_url}/repository/process-definitions/{process_definition_id}/decisions"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["total"], 1);
    assert_eq!(body["data"].as_array().unwrap().len(), 1);
    assert_eq!(body["data"][0]["id"], decision_id);
    assert_eq!(body["data"][0]["key"], "breadthLoanDecision");
}
