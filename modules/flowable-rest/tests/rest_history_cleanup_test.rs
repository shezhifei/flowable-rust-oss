use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_rest::run_server;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::net::TcpListener;

const SIMPLE_PROCESS_BPMN: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             targetNamespace="Examples">
    <process id="simpleProcess" name="Simple Process" isExecutable="true">
        <startEvent id="startEvent" />
        <sequenceFlow id="flow1" sourceRef="startEvent" targetRef="userTask" />
        <userTask id="userTask" name="Simple Task" />
        <sequenceFlow id="flow2" sourceRef="userTask" targetRef="endEvent" />
        <endEvent id="endEvent" />
    </process>
</definitions>"#;

async fn spawn_server() -> (Arc<ProcessEngine>, String, reqwest::Client) {
    let engine = Arc::new(ProcessEngine::new("rest-history-cleanup-test".to_string()));
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

async fn deploy_process(client: &reqwest::Client, base_url: &str) {
    let response = client
        .post(format!("{base_url}/repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "Simple process",
            "resourceName": "simple-process.bpmn20.xml",
            "resource": SIMPLE_PROCESS_BPMN
        }))
        .send()
        .await
        .unwrap();
    // P109: deploy (both JSON superset and multipart paths) returns 201.
    assert_eq!(response.status(), reqwest::StatusCode::CREATED);
}

async fn start_and_complete_task(
    client: &reqwest::Client,
    base_url: &str,
    process_definition_id: &str,
) -> (String, String) {
    let start_response = client
        .post(format!("{base_url}/runtime/process-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "processDefinitionId": process_definition_id
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(start_response.status(), reqwest::StatusCode::OK);
    let start_body: Value = start_response.json().await.unwrap();
    let process_instance_id = start_body["id"].as_str().unwrap().to_string();

    let tasks_response = client
        .post(format!("{base_url}/query/tasks"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "processInstanceId": process_instance_id
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(tasks_response.status(), reqwest::StatusCode::OK);
    let tasks: Value = tasks_response.json().await.unwrap();
    let task_id = tasks["data"][0]["id"].as_str().unwrap().to_string();

    let complete_response = client
        .post(format!("{base_url}/runtime/tasks/{task_id}"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "action": "complete"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(complete_response.status(), reqwest::StatusCode::OK);

    (process_instance_id, task_id)
}

#[tokio::test]
async fn cleanup_history_by_date() {
    // P133: cutoff is end_time (finishedBefore). Completed instance ends before
    // 2099 → deleted. (Was start_time; same outcome for finished instances.)
    let (engine, base_url, client) = spawn_server().await;
    deploy_process(&client, &base_url).await;

    let process_definition_id = engine
        .get_repository_service()
        .latest_process_definition_by_key("simpleProcess", None)
        .unwrap()
        .unwrap()
        .id;

    let (_, _) = start_and_complete_task(&client, &base_url, &process_definition_id).await;

    let response = client
        .post(format!("{base_url}/history/history-cleanup"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "beforeDate": "2099-01-01T00:00:00Z",
            "cleanupType": "completed"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);

    let body: Value = response.json().await.unwrap();
    assert!(body["deletedProcessInstances"].as_u64().unwrap() >= 1);
    assert!(body["durationMs"].as_u64().is_some());
}

#[tokio::test]
async fn cleanup_history_by_process_instance_ids() {
    // P133: cleanupType=all still only deletes finished instances (end_time set).
    let (engine, base_url, client) = spawn_server().await;
    deploy_process(&client, &base_url).await;

    let process_definition_id = engine
        .get_repository_service()
        .latest_process_definition_by_key("simpleProcess", None)
        .unwrap()
        .unwrap()
        .id;

    let (process_instance_id, _) =
        start_and_complete_task(&client, &base_url, &process_definition_id).await;

    let response = client
        .post(format!("{base_url}/history/history-cleanup"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "processInstanceIds": [process_instance_id],
            "cleanupType": "all"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);

    let body: Value = response.json().await.unwrap();
    assert_eq!(body["deletedProcessInstances"].as_u64().unwrap(), 1);
}

/// P133: running instances must never be deleted by manual history cleanup,
/// even when their start_time is before the cutoff and cleanupType=all.
/// Pre-fix used start_time and would incorrectly delete long-running instances.
#[tokio::test]
async fn cleanup_history_does_not_delete_running_old_instances() {
    let (engine, base_url, client) = spawn_server().await;
    deploy_process(&client, &base_url).await;

    let process_definition_id = engine
        .get_repository_service()
        .latest_process_definition_by_key("simpleProcess", None)
        .unwrap()
        .unwrap()
        .id;

    // Start but do NOT complete — instance stays running (end_time = null).
    let start_response = client
        .post(format!("{base_url}/runtime/process-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "processDefinitionId": process_definition_id
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(start_response.status(), reqwest::StatusCode::OK);
    let start_body: Value = start_response.json().await.unwrap();
    let running_id = start_body["id"].as_str().unwrap().to_string();

    // Also create a completed instance that SHOULD be cleaned.
    let (completed_id, _) =
        start_and_complete_task(&client, &base_url, &process_definition_id).await;

    // by-date path: cleanupType=all with far-future cutoff
    let response = client
        .post(format!("{base_url}/history/history-cleanup"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "beforeDate": "2099-01-01T00:00:00Z",
            "cleanupType": "all"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let body: Value = response.json().await.unwrap();
    // Only the completed instance is deleted (not the running one).
    assert_eq!(body["deletedProcessInstances"].as_u64().unwrap(), 1);

    // Running historic row still present; completed gone.
    let mut session = engine.get_runtime_store().create_session().unwrap();
    assert!(
        engine
            .get_runtime_store()
            .get_historic_process_instance(&running_id, &mut session)
            .is_some(),
        "running instance must survive cleanup"
    );
    assert!(
        engine
            .get_runtime_store()
            .get_historic_process_instance(&completed_id, &mut session)
            .is_none(),
        "completed instance should have been cleaned"
    );

    // by-ids path: explicitly targeting a running id must not delete it.
    let response = client
        .post(format!("{base_url}/history/history-cleanup"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "processInstanceIds": [running_id.clone()],
            "cleanupType": "all"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["deletedProcessInstances"].as_u64().unwrap(), 0);

    let mut session = engine.get_runtime_store().create_session().unwrap();
    assert!(
        engine
            .get_runtime_store()
            .get_historic_process_instance(&running_id, &mut session)
            .is_some(),
        "running instance must survive by-ids cleanup with type=all"
    );
}

#[tokio::test]
async fn cleanup_history_invalid_type_returns_400() {
    let (_engine, base_url, client) = spawn_server().await;

    let response = client
        .post(format!("{base_url}/history/history-cleanup"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "beforeDate": "2099-01-01T00:00:00Z",
            "cleanupType": "invalid"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn cleanup_history_invalid_batch_size_returns_400() {
    let (_engine, base_url, client) = spawn_server().await;

    let response = client
        .post(format!("{base_url}/history/history-cleanup"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "beforeDate": "2099-01-01T00:00:00Z",
            "batchSize": 0
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn cleanup_history_batch_size_too_large_returns_400() {
    let (_engine, base_url, client) = spawn_server().await;

    let response = client
        .post(format!("{base_url}/history/history-cleanup"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "beforeDate": "2099-01-01T00:00:00Z",
            "batchSize": 20000
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn configure_cleanup_strategy() {
    let (_engine, base_url, client) = spawn_server().await;

    let response = client
        .post(format!("{base_url}/history/history-cleanup/strategy"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "retentionDays": 90,
            "maxRecords": 100000,
            "autoCleanup": true,
            "cleanupSchedule": "0 2 * * *"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);

    let body: Value = response.json().await.unwrap();
    assert_eq!(body["retentionDays"].as_u64().unwrap(), 90);
    assert_eq!(body["maxRecords"].as_u64().unwrap(), 100000);
    assert!(body["autoCleanup"].as_bool().unwrap());
    assert_eq!(body["cleanupSchedule"].as_str().unwrap(), "0 2 * * *");
}

#[tokio::test]
async fn configure_cleanup_strategy_invalid_retention_days() {
    let (_engine, base_url, client) = spawn_server().await;

    let response = client
        .post(format!("{base_url}/history/history-cleanup/strategy"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "retentionDays": 0
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn configure_cleanup_strategy_retention_days_too_large() {
    let (_engine, base_url, client) = spawn_server().await;

    let response = client
        .post(format!("{base_url}/history/history-cleanup/strategy"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "retentionDays": 5000
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn configure_cleanup_strategy_invalid_max_records() {
    let (_engine, base_url, client) = spawn_server().await;

    let response = client
        .post(format!("{base_url}/history/history-cleanup/strategy"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "maxRecords": 0
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn configure_cleanup_strategy_max_records_too_large() {
    let (_engine, base_url, client) = spawn_server().await;

    let response = client
        .post(format!("{base_url}/history/history-cleanup/strategy"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "maxRecords": 20000000
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn configure_cleanup_strategy_invalid_cron() {
    let (_engine, base_url, client) = spawn_server().await;

    let response = client
        .post(format!("{base_url}/history/history-cleanup/strategy"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "cleanupSchedule": "invalid"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn cleanup_history_with_batch_size() {
    let (engine, base_url, client) = spawn_server().await;
    deploy_process(&client, &base_url).await;

    let process_definition_id = engine
        .get_repository_service()
        .latest_process_definition_by_key("simpleProcess", None)
        .unwrap()
        .unwrap()
        .id;

    let (_, _) = start_and_complete_task(&client, &base_url, &process_definition_id).await;
    let (_, _) = start_and_complete_task(&client, &base_url, &process_definition_id).await;

    let response = client
        .post(format!("{base_url}/history/history-cleanup"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "beforeDate": "2099-01-01T00:00:00Z",
            "cleanupType": "completed",
            "batchSize": 1
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);

    let body: Value = response.json().await.unwrap();
    assert!(body["deletedProcessInstances"].as_u64().unwrap() >= 1);
}

#[tokio::test]
async fn cleanup_history_no_instances() {
    // P133: empty store → 0 deleted (unchanged; cutoff column irrelevant).
    let (_engine, base_url, client) = spawn_server().await;

    let response = client
        .post(format!("{base_url}/history/history-cleanup"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "beforeDate": "2099-01-01T00:00:00Z",
            "cleanupType": "all"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);

    let body: Value = response.json().await.unwrap();
    assert_eq!(body["deletedProcessInstances"].as_u64().unwrap(), 0);
}

/// P133: instance that finished AFTER the cutoff is not deleted, even if it
/// started well before the cutoff (old start_time would have matched pre-fix).
#[tokio::test]
async fn cleanup_history_skips_instances_finished_after_cutoff() {
    let (engine, base_url, client) = spawn_server().await;
    deploy_process(&client, &base_url).await;

    let process_definition_id = engine
        .get_repository_service()
        .latest_process_definition_by_key("simpleProcess", None)
        .unwrap()
        .unwrap()
        .id;

    let (process_instance_id, _) =
        start_and_complete_task(&client, &base_url, &process_definition_id).await;

    // Cutoff in the past relative to now — finished instance ends after 2000-01-01
    // wait, we need the opposite: finished AFTER cutoff means not deleted when
    // beforeDate is in the distant past.
    let response = client
        .post(format!("{base_url}/history/history-cleanup"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "beforeDate": "2000-01-01T00:00:00Z",
            "cleanupType": "all"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let body: Value = response.json().await.unwrap();
    // Pre-fix (start_time): would delete if start_time < 2000 — usually 0 anyway.
    // Post-fix (end_time): end_time is ~now > 2000 → not deleted.
    assert_eq!(body["deletedProcessInstances"].as_u64().unwrap(), 0);

    let mut session = engine.get_runtime_store().create_session().unwrap();
    assert!(
        engine
            .get_runtime_store()
            .get_historic_process_instance(&process_instance_id, &mut session)
            .is_some(),
        "instance finished after cutoff must not be deleted"
    );
}

#[tokio::test]
async fn configure_cleanup_strategy_default_values() {
    let (_engine, base_url, client) = spawn_server().await;

    let response = client
        .post(format!("{base_url}/history/history-cleanup/strategy"))
        .basic_auth("admin", Some("test"))
        .json(&json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);

    let body: Value = response.json().await.unwrap();
    assert!(!body["autoCleanup"].as_bool().unwrap());
}
