use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_rest::run_server;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::net::TcpListener;

async fn start_server(name: &str) -> (Arc<ProcessEngine>, String) {
    let engine = Arc::new(ProcessEngine::new(name.to_string()));
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

    (engine, base_url)
}

async fn deploy_and_start(
    client: &reqwest::Client,
    engine: &ProcessEngine,
    base_url: &str,
) -> String {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="changeStateProcess" name="Change State Process" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="f1" sourceRef="start" targetRef="reviewA" />
            <userTask id="reviewA" name="Review A" />
            <sequenceFlow id="f2" sourceRef="reviewA" targetRef="reviewB" />
            <userTask id="reviewB" name="Review B" />
            <sequenceFlow id="f3" sourceRef="reviewB" targetRef="reviewC" />
            <userTask id="reviewC" name="Review C" />
            <sequenceFlow id="f4" sourceRef="reviewC" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#;

    let deploy_response = client
        .post(format!("{base_url}/repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "Change State Deployment",
            "resourceName": "change_state_process.bpmn20.xml",
            "resource": xml
        }))
        .send()
        .await
        .unwrap();
    assert!(deploy_response.status().is_success());

    let process_definition_id = engine
        .get_repository_service()
        .get_process_definition_ids()
        .unwrap()[0]
        .clone();

    let start_response = client
        .post(format!("{base_url}/runtime/process-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "processDefinitionId": process_definition_id,
            "businessKey": "Change State Instance"
        }))
        .send()
        .await
        .unwrap();
    assert!(start_response.status().is_success());
    let start_body: Value = start_response.json().await.unwrap();
    start_body["id"].as_str().unwrap().to_string()
}

async fn deploy_and_start_parallel(
    client: &reqwest::Client,
    engine: &ProcessEngine,
    base_url: &str,
) -> String {
    // Mirrors org/flowable/rest/service/api/runtime/parallelTask.bpmn20.xml
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions id="definitions" xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="startParallelProcess" isExecutable="true">
            <startEvent id="theStart" />
            <sequenceFlow id="flow1" sourceRef="theStart" targetRef="taskBefore" />
            <userTask id="taskBefore" name="Task before sub process" />
            <sequenceFlow id="flow2" sourceRef="taskBefore" targetRef="parallelFork" />
            <parallelGateway id="parallelFork" />
            <sequenceFlow id="flow3" sourceRef="parallelFork" targetRef="task1" />
            <userTask id="task1" />
            <sequenceFlow id="flow4" sourceRef="parallelFork" targetRef="task2" />
            <userTask id="task2" />
            <sequenceFlow id="flow5" sourceRef="task1" targetRef="parallelJoin" />
            <sequenceFlow id="flow6" sourceRef="task2" targetRef="parallelJoin" />
            <parallelGateway id="parallelJoin" />
            <sequenceFlow id="flow7" sourceRef="parallelJoin" targetRef="taskAfter" />
            <userTask id="taskAfter" name="Task after sub process" />
            <sequenceFlow id="flow8" sourceRef="taskAfter" targetRef="theEnd" />
            <endEvent id="theEnd" />
        </process>
    </definitions>"#;

    let deploy_response = client
        .post(format!("{base_url}/repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "Parallel Change State Deployment",
            "resourceName": "parallelTask.bpmn20.xml",
            "resource": xml
        }))
        .send()
        .await
        .unwrap();
    assert!(deploy_response.status().is_success());

    let process_definition_id = engine
        .get_repository_service()
        .get_process_definition_ids()
        .unwrap()[0]
        .clone();

    let start_response = client
        .post(format!("{base_url}/runtime/process-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "processDefinitionId": process_definition_id }))
        .send()
        .await
        .unwrap();
    assert!(start_response.status().is_success());
    let start_body: Value = start_response.json().await.unwrap();
    start_body["id"].as_str().unwrap().to_string()
}

async fn complete_task(client: &reqwest::Client, base_url: &str, task_id: &str) {
    let response = client
        .post(format!("{base_url}/runtime/tasks/{task_id}"))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "action": "complete" }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
}

async fn task_definition_keys(
    client: &reqwest::Client,
    base_url: &str,
    process_instance_id: &str,
) -> Vec<String> {
    let tasks = task_query(client, base_url, process_instance_id).await;
    let mut keys = tasks["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|task| {
            task["taskDefinitionKey"]
                .as_str()
                .unwrap_or_default()
                .to_string()
        })
        .collect::<Vec<_>>();
    keys.sort();
    keys
}

async fn task_query(client: &reqwest::Client, base_url: &str, process_instance_id: &str) -> Value {
    let tasks_response = client
        .post(format!("{base_url}/query/tasks"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "processInstanceId": process_instance_id
        }))
        .send()
        .await
        .unwrap();
    assert!(tasks_response.status().is_success());
    tasks_response.json().await.unwrap()
}

#[tokio::test]
async fn process_instance_change_state_moves_user_task_wait_state() {
    let (engine, base_url) = start_server("rest-bpmn-change-state-process").await;
    let client = reqwest::Client::new();
    let process_instance_id = deploy_and_start(&client, &engine, &base_url).await;

    let initial_tasks = task_query(&client, &base_url, &process_instance_id).await;
    assert_eq!(initial_tasks["total"], 1);
    assert_eq!(initial_tasks["data"][0]["name"], "Review A");

    let change_response = client
        .post(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}/change-state"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "cancelActivityIds": ["reviewA"],
            "startActivityIds": ["reviewB"]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(change_response.status(), 200);

    let changed_tasks = task_query(&client, &base_url, &process_instance_id).await;
    assert_eq!(changed_tasks["total"], 1);
    assert_eq!(changed_tasks["data"][0]["name"], "Review B");

    let missing_response = client
        .post(format!(
            "{base_url}/runtime/process-instances/missing-process/change-state"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "cancelActivityIds": ["reviewA"],
            "startActivityIds": ["reviewB"]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(missing_response.status(), 404);

    let unsupported_response = client
        .post(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}/change-state"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "cancelActivityIds": ["reviewA", "reviewB"],
            "startActivityIds": ["reviewB", "reviewC"]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(unsupported_response.status(), 400);
    let unsupported_body: Value = unsupported_response.json().await.unwrap();
    assert_eq!(unsupported_body["code"], "BAD_REQUEST");
    assert!(
        unsupported_body["details"]
            .as_str()
            .unwrap()
            .contains("Only single-to-many or many-to-single")
    );
}

#[tokio::test]
async fn process_instance_change_state_accepts_move_activity_id_to_map_shape() {
    let (engine, base_url) = start_server("rest-bpmn-change-state-move-map").await;
    let client = reqwest::Client::new();
    let process_instance_id = deploy_and_start(&client, &engine, &base_url).await;

    let change_response = client
        .post(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}/change-state"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "moveActivityIdTo": {
                "reviewA": "reviewB"
            }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(change_response.status(), 200);

    let changed_tasks = task_query(&client, &base_url, &process_instance_id).await;
    assert_eq!(changed_tasks["total"], 1);
    assert_eq!(changed_tasks["data"][0]["name"], "Review B");
}

#[tokio::test]
async fn execution_change_state_accepts_same_move_activity_array_shape() {
    let (engine, base_url) = start_server("rest-bpmn-change-state-execution-move").await;
    let client = reqwest::Client::new();
    let process_instance_id = deploy_and_start(&client, &engine, &base_url).await;
    let initial_tasks = task_query(&client, &base_url, &process_instance_id).await;
    let execution_id = initial_tasks["data"][0]["executionId"]
        .as_str()
        .unwrap()
        .to_string();

    let change_response = client
        .post(format!(
            "{base_url}/runtime/executions/{execution_id}/change-state"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "moveActivityIdTo": [
                {
                    "sourceActivityId": "reviewA",
                    "targetActivityId": "reviewC"
                }
            ]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(change_response.status(), 200);

    let changed_tasks = task_query(&client, &base_url, &process_instance_id).await;
    assert_eq!(changed_tasks["total"], 1);
    assert_eq!(changed_tasks["data"][0]["name"], "Review C");
}

#[tokio::test]
async fn change_state_rejects_mixed_or_incomplete_move_shapes() {
    let (engine, base_url) = start_server("rest-bpmn-change-state-move-errors").await;
    let client = reqwest::Client::new();
    let process_instance_id = deploy_and_start(&client, &engine, &base_url).await;

    let mixed_response = client
        .post(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}/change-state"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "cancelActivityIds": ["reviewA"],
            "moveActivityIdTo": {
                "reviewA": "reviewB"
            }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(mixed_response.status(), 400);
    let mixed_body: Value = mixed_response.json().await.unwrap();
    assert_eq!(mixed_body["code"], "BAD_REQUEST");
    assert!(
        mixed_body["details"]
            .as_str()
            .unwrap()
            .contains("Use either cancelActivityIds/startActivityIds")
    );

    let incomplete_response = client
        .post(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}/change-state"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "moveActivityIdTo": {
                "sourceActivityId": "reviewA"
            }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(incomplete_response.status(), 400);
    let incomplete_body: Value = incomplete_response.json().await.unwrap();
    assert_eq!(incomplete_body["code"], "BAD_REQUEST");
    assert!(
        incomplete_body["details"]
            .as_str()
            .unwrap()
            .contains("moveActivityIdTo target is required")
    );
}

#[tokio::test]
async fn process_instance_change_state_cancel_activity_ids_can_cancel_wait_state() {
    let (engine, base_url) = start_server("rest-bpmn-change-state-cancel-only").await;
    let client = reqwest::Client::new();
    let process_instance_id = deploy_and_start(&client, &engine, &base_url).await;

    let initial_tasks = task_query(&client, &base_url, &process_instance_id).await;
    assert_eq!(initial_tasks["total"], 1);
    assert_eq!(initial_tasks["data"][0]["name"], "Review A");

    let change_response = client
        .post(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}/change-state"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "cancelActivityIds": ["reviewA"]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(change_response.status(), 200);

    let changed_tasks = task_query(&client, &base_url, &process_instance_id).await;
    assert_eq!(changed_tasks["total"], 0);

    let instance_response = client
        .get(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(instance_response.status(), 200);
    let instance_body: Value = instance_response.json().await.unwrap();
    assert_eq!(instance_body["isEnded"], true);

    let empty_response = client
        .post(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}/change-state"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(empty_response.status(), 400);
    let empty_body: Value = empty_response.json().await.unwrap();
    assert_eq!(empty_body["code"], "BAD_REQUEST");
    assert!(
        empty_body["details"]
            .as_str()
            .unwrap()
            .contains("At least one of cancelActivityIds or startActivityIds")
    );
}

#[tokio::test]
async fn execution_change_state_moves_execution_to_started_activity() {
    let (engine, base_url) = start_server("rest-bpmn-change-state-execution").await;
    let client = reqwest::Client::new();
    let process_instance_id = deploy_and_start(&client, &engine, &base_url).await;

    let initial_tasks = task_query(&client, &base_url, &process_instance_id).await;
    let execution_id = initial_tasks["data"][0]["executionId"]
        .as_str()
        .unwrap()
        .to_string();

    let change_response = client
        .post(format!(
            "{base_url}/runtime/executions/{execution_id}/change-state"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "startActivityIds": ["reviewC"]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(change_response.status(), 200);

    let changed_tasks = task_query(&client, &base_url, &process_instance_id).await;
    assert_eq!(changed_tasks["total"], 1);
    assert_eq!(changed_tasks["data"][0]["name"], "Review C");

    let missing_response = client
        .post(format!(
            "{base_url}/runtime/executions/missing-execution/change-state"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "startActivityIds": ["reviewB"]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(missing_response.status(), 404);

    let unsupported_response = client
        .post(format!(
            "{base_url}/runtime/executions/{execution_id}/change-state"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "cancelActivityIds": ["reviewA"],
            "startActivityIds": ["reviewB"]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(unsupported_response.status(), 400);
}

#[tokio::test]
async fn execution_change_state_cancel_activity_ids_can_cancel_or_move_execution() {
    let (engine, base_url) = start_server("rest-bpmn-change-state-execution-cancel").await;
    let client = reqwest::Client::new();
    let process_instance_id = deploy_and_start(&client, &engine, &base_url).await;

    let initial_tasks = task_query(&client, &base_url, &process_instance_id).await;
    let execution_id = initial_tasks["data"][0]["executionId"]
        .as_str()
        .unwrap()
        .to_string();

    let move_response = client
        .post(format!(
            "{base_url}/runtime/executions/{execution_id}/change-state"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "cancelActivityIds": ["reviewA"],
            "startActivityIds": ["reviewB"]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(move_response.status(), 200);

    let moved_tasks = task_query(&client, &base_url, &process_instance_id).await;
    assert_eq!(moved_tasks["total"], 1);
    assert_eq!(moved_tasks["data"][0]["name"], "Review B");
    let moved_execution_id = moved_tasks["data"][0]["executionId"]
        .as_str()
        .unwrap()
        .to_string();

    let unsupported_response = client
        .post(format!(
            "{base_url}/runtime/executions/{moved_execution_id}/change-state"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "cancelActivityIds": ["reviewA", "reviewB"],
            "startActivityIds": ["reviewC"]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(unsupported_response.status(), 400);
    let unsupported_body: Value = unsupported_response.json().await.unwrap();
    assert_eq!(unsupported_body["code"], "BAD_REQUEST");
    assert!(
        unsupported_body["details"]
            .as_str()
            .unwrap()
            .contains("only one cancelActivityId")
    );

    let cancel_response = client
        .post(format!(
            "{base_url}/runtime/executions/{moved_execution_id}/change-state"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "cancelActivityIds": ["reviewB"]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(cancel_response.status(), 200);

    let changed_tasks = task_query(&client, &base_url, &process_instance_id).await;
    assert_eq!(changed_tasks["total"], 0);
}

/// Java parity: ProcessInstanceChangeActivityStateResourceTest#testChangeActivityStateManyToOne.
/// Cancels both parallel branches back to a single upstream activity, then verifies the process
/// can still fan out through the fork and converge on the join afterwards.
#[tokio::test]
async fn process_instance_change_state_many_to_one_restores_parallel_round_trip() {
    let (engine, base_url) = start_server("rest-bpmn-change-state-many-to-one").await;
    let client = reqwest::Client::new();
    let process_instance_id = deploy_and_start_parallel(&client, &engine, &base_url).await;

    let initial_tasks = task_query(&client, &base_url, &process_instance_id).await;
    assert_eq!(initial_tasks["total"], 1);
    assert_eq!(initial_tasks["data"][0]["taskDefinitionKey"], "taskBefore");
    let task_before_id = initial_tasks["data"][0]["id"].as_str().unwrap().to_string();

    complete_task(&client, &base_url, &task_before_id).await;
    assert_eq!(
        task_definition_keys(&client, &base_url, &process_instance_id).await,
        vec!["task1".to_string(), "task2".to_string()]
    );

    // Many-to-one: both parallel activities are cancelled and a single execution resumes
    // at taskBefore, matching moveActivityIdsToSingleActivityId in Java.
    let change_response = client
        .post(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}/change-state"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "cancelActivityIds": ["task1", "task2"],
            "startActivityIds": ["taskBefore"]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(change_response.status(), 200);

    let changed_tasks = task_query(&client, &base_url, &process_instance_id).await;
    assert_eq!(changed_tasks["total"], 1);
    assert_eq!(changed_tasks["data"][0]["taskDefinitionKey"], "taskBefore");
    let restored_task_id = changed_tasks["data"][0]["id"].as_str().unwrap().to_string();

    // The fork must still work after the state change: completing taskBefore fans out again.
    complete_task(&client, &base_url, &restored_task_id).await;
    assert_eq!(
        task_definition_keys(&client, &base_url, &process_instance_id).await,
        vec!["task1".to_string(), "task2".to_string()]
    );

    // And the join must still converge exactly once onto taskAfter.
    let parallel_tasks = task_query(&client, &base_url, &process_instance_id).await;
    let parallel_task_ids = parallel_tasks["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|task| task["id"].as_str().unwrap().to_string())
        .collect::<Vec<_>>();
    for task_id in parallel_task_ids {
        complete_task(&client, &base_url, &task_id).await;
    }

    let after_tasks = task_query(&client, &base_url, &process_instance_id).await;
    assert_eq!(after_tasks["total"], 1);
    assert_eq!(after_tasks["data"][0]["taskDefinitionKey"], "taskAfter");
    let task_after_id = after_tasks["data"][0]["id"].as_str().unwrap().to_string();

    complete_task(&client, &base_url, &task_after_id).await;

    let instance_response = client
        .get(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(instance_response.status(), 200);
    let instance_body: Value = instance_response.json().await.unwrap();
    assert_eq!(instance_body["isEnded"], true);
    assert_eq!(
        task_query(&client, &base_url, &process_instance_id).await["total"],
        0
    );
}

#[tokio::test]
async fn process_instance_change_state_ended_instance_returns_404() {
    let (engine, base_url) = start_server("rest-bpmn-change-state-ended-instance").await;
    let client = reqwest::Client::new();
    let process_instance_id = deploy_and_start(&client, &engine, &base_url).await;

    let cancel_response = client
        .post(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}/change-state"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "cancelActivityIds": ["reviewA"]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(cancel_response.status(), 200);

    let instance_response = client
        .get(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(instance_response.status(), 200);
    let instance_body: serde_json::Value = instance_response.json().await.unwrap();
    assert_eq!(instance_body["isEnded"], true);

    let ended_response = client
        .post(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}/change-state"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "cancelActivityIds": ["reviewA"]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(ended_response.status(), 404);
    let ended_body: serde_json::Value = ended_response.json().await.unwrap();
    assert_eq!(ended_body["code"], "NOT_FOUND");
}

#[tokio::test]
async fn process_instance_change_state_suspended_instance_returns_500() {
    let (engine, base_url) = start_server("rest-bpmn-change-state-suspended-instance").await;
    let client = reqwest::Client::new();
    let process_instance_id = deploy_and_start(&client, &engine, &base_url).await;

    let suspend_response = client
        .put(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "action": "suspend"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(suspend_response.status(), 200);

    let change_response = client
        .post(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}/change-state"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "cancelActivityIds": ["reviewA"],
            "startActivityIds": ["reviewB"]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(change_response.status(), 500);
}

#[tokio::test]
async fn process_instance_change_state_missing_activity_returns_500() {
    let (engine, base_url) = start_server("rest-bpmn-change-state-missing-activity").await;
    let client = reqwest::Client::new();
    let process_instance_id = deploy_and_start(&client, &engine, &base_url).await;

    let change_response = client
        .post(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}/change-state"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "cancelActivityIds": ["nonexistentActivity"]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(change_response.status(), 500);
    let body: serde_json::Value = change_response.json().await.unwrap();
    assert_eq!(body["code"], "INTERNAL_SERVER_ERROR");
    // 5xx details are generic (no internal exception text echo).
    assert_eq!(body["details"], "Internal server error");
}

#[tokio::test]
async fn execution_change_state_ended_execution_returns_404() {
    let (engine, base_url) = start_server("rest-bpmn-change-state-ended-execution").await;
    let client = reqwest::Client::new();
    let process_instance_id = deploy_and_start(&client, &engine, &base_url).await;

    let initial_tasks = task_query(&client, &base_url, &process_instance_id).await;
    let execution_id = initial_tasks["data"][0]["executionId"]
        .as_str()
        .unwrap()
        .to_string();

    let cancel_response = client
        .post(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}/change-state"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "cancelActivityIds": ["reviewA"]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(cancel_response.status(), 200);

    let ended_response = client
        .post(format!(
            "{base_url}/runtime/executions/{execution_id}/change-state"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "startActivityIds": ["reviewB"]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(ended_response.status(), 404);
    let ended_body: serde_json::Value = ended_response.json().await.unwrap();
    assert_eq!(ended_body["code"], "NOT_FOUND");
}

#[tokio::test]
async fn execution_change_state_suspended_execution_returns_500() {
    let (engine, base_url) = start_server("rest-bpmn-change-state-suspended-execution").await;
    let client = reqwest::Client::new();
    let process_instance_id = deploy_and_start(&client, &engine, &base_url).await;

    let initial_tasks = task_query(&client, &base_url, &process_instance_id).await;
    let execution_id = initial_tasks["data"][0]["executionId"]
        .as_str()
        .unwrap()
        .to_string();

    let suspend_response = client
        .put(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "action": "suspend"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(suspend_response.status(), 200);

    let change_response = client
        .post(format!(
            "{base_url}/runtime/executions/{execution_id}/change-state"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "startActivityIds": ["reviewB"]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(change_response.status(), 500);
}

// ── P67: moveExecutionToActivityId + enableEventSubProcessStartEvent ─────────

/// P67: execution change-state with `moveExecutionToActivityId` string preserves
/// the execution id (true move, not cancel+start). Java
/// `ChangeActivityStateBuilder#moveExecutionToActivityId`.
#[tokio::test]
async fn execution_change_state_move_execution_to_activity_id_preserves_id() {
    let (engine, base_url) = start_server("rest-p67-exec-true-move").await;
    let client = reqwest::Client::new();
    let process_instance_id = deploy_and_start(&client, &engine, &base_url).await;

    let initial_tasks = task_query(&client, &base_url, &process_instance_id).await;
    let execution_id = initial_tasks["data"][0]["executionId"]
        .as_str()
        .unwrap()
        .to_string();

    // Seed a local variable on the source execution via engine API.
    engine
        .get_runtime_service()
        .set_variable_local(
            execution_id.clone(),
            "carryLocal".to_string(),
            json!("from-reviewA"),
        )
        .unwrap();

    let change_response = client
        .post(format!(
            "{base_url}/runtime/executions/{execution_id}/change-state"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "moveExecutionToActivityId": "reviewB"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        change_response.status(),
        200,
        "body: {:?}",
        change_response.text().await
    );

    let moved_tasks = task_query(&client, &base_url, &process_instance_id).await;
    assert_eq!(moved_tasks["total"], 1);
    assert_eq!(moved_tasks["data"][0]["name"], "Review B");
    let moved_execution_id = moved_tasks["data"][0]["executionId"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(
        moved_execution_id, execution_id,
        "true move must reuse the source execution id"
    );
    assert_eq!(
        engine
            .get_runtime_service()
            .get_variable_local(execution_id, "carryLocal".to_string())
            .unwrap(),
        Some(json!("from-reviewA")),
        "local variables must survive the true move"
    );
}

/// P67: process-instance change-state with object-form moveExecutionToActivityId.
#[tokio::test]
async fn process_instance_change_state_move_execution_to_activity_id_object_shape() {
    let (engine, base_url) = start_server("rest-p67-pi-true-move").await;
    let client = reqwest::Client::new();
    let process_instance_id = deploy_and_start(&client, &engine, &base_url).await;

    let initial_tasks = task_query(&client, &base_url, &process_instance_id).await;
    let execution_id = initial_tasks["data"][0]["executionId"]
        .as_str()
        .unwrap()
        .to_string();

    let change_response = client
        .post(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}/change-state"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "moveExecutionToActivityId": {
                "executionId": execution_id,
                "activityId": "reviewC"
            }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(change_response.status(), 200);

    let moved_tasks = task_query(&client, &base_url, &process_instance_id).await;
    assert_eq!(moved_tasks["total"], 1);
    assert_eq!(moved_tasks["data"][0]["name"], "Review C");
    assert_eq!(
        moved_tasks["data"][0]["executionId"].as_str().unwrap(),
        execution_id
    );
}

/// P67: enableEventSubProcessStartEvent re-arms a message ES start via REST.
#[tokio::test]
async fn process_instance_change_state_enable_event_subprocess_start_event() {
    let (engine, base_url) = start_server("rest-p67-enable-es").await;
    let client = reqwest::Client::new();

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
                 xmlns:flowable="http://flowable.org/bpmn"
                 targetNamespace="Examples">
        <signal id="mySignal" name="mySignal" flowable:scope="global"/>
        <message id="myMessage" name="myMessage"/>
        <process id="changeStateForEventSubProcess" isExecutable="true">
            <startEvent id="theStart"/>
            <sequenceFlow id="f1" sourceRef="theStart" targetRef="processTask"/>
            <userTask id="processTask"/>
            <sequenceFlow id="f2" sourceRef="processTask" targetRef="theEnd"/>
            <endEvent id="theEnd"/>
            <subProcess id="eventSubProcess" triggeredByEvent="true">
                <startEvent id="eventSubProcessStart" isInterrupting="true">
                    <signalEventDefinition signalRef="mySignal" />
                </startEvent>
                <sequenceFlow id="esf1" sourceRef="eventSubProcessStart" targetRef="eventSubProcessTask" />
                <userTask id="eventSubProcessTask"/>
                <sequenceFlow id="esf2" sourceRef="eventSubProcessTask" targetRef="eventSubProcessEnd" />
                <endEvent id="eventSubProcessEnd" />
                <startEvent id="messageEventSubProcessStart" isInterrupting="true">
                    <messageEventDefinition messageRef="myMessage"/>
                </startEvent>
                <sequenceFlow id="esf3" sourceRef="messageEventSubProcessStart" targetRef="messageEventSubProcessTask" />
                <userTask id="messageEventSubProcessTask"/>
                <sequenceFlow id="esf4" sourceRef="messageEventSubProcessTask" targetRef="messageEventSubProcessEnd" />
                <endEvent id="messageEventSubProcessEnd" />
            </subProcess>
        </process>
    </definitions>"#;

    let deploy_response = client
        .post(format!("{base_url}/repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "ES Enable Deployment",
            "resourceName": "multi_es.bpmn20.xml",
            "resource": xml
        }))
        .send()
        .await
        .unwrap();
    assert!(deploy_response.status().is_success());

    let process_definition_id = engine
        .get_repository_service()
        .get_process_definition_ids()
        .unwrap()[0]
        .clone();
    let start_response = client
        .post(format!("{base_url}/runtime/process-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "processDefinitionId": process_definition_id
        }))
        .send()
        .await
        .unwrap();
    assert!(start_response.status().is_success());
    let start_body: Value = start_response.json().await.unwrap();
    let process_instance_id = start_body["id"].as_str().unwrap().to_string();

    // Disarm subscriptions (engine may re-arm on task create).
    {
        let store = engine.get_runtime_store();
        let mut session = store.create_session().unwrap();
        store.delete_event_subprocess_event_subscriptions_by_process_instance_id(
            &process_instance_id,
            &mut session,
        );
        session.flush_and_commit().unwrap();
    }

    let enable_response = client
        .post(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}/change-state"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "enableEventSubProcessStartEvent": "messageEventSubProcessStart"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        enable_response.status(),
        200,
        "body: {:?}",
        enable_response.text().await
    );

    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let subs = store.find_event_subprocess_event_subscriptions_by_process_instance_id(
        &process_instance_id,
        &mut session,
    );
    assert_eq!(subs.len(), 1, "exactly one subscription after enable: {subs:?}");
    assert_eq!(subs[0].start_event_id, "messageEventSubProcessStart");
    assert_eq!(subs[0].event_ref, "myMessage");
}

/// P67: combining exclusive shapes with cancel/start is rejected.
#[tokio::test]
async fn change_state_rejects_move_execution_mixed_with_cancel_start() {
    let (engine, base_url) = start_server("rest-p67-mixed-shapes").await;
    let client = reqwest::Client::new();
    let process_instance_id = deploy_and_start(&client, &engine, &base_url).await;

    let response = client
        .post(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}/change-state"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "moveExecutionToActivityId": "reviewB",
            "executionId": "whatever",
            "cancelActivityIds": ["reviewA"]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 400);
    let body: Value = response.json().await.unwrap();
    assert!(
        body["details"]
            .as_str()
            .unwrap_or_default()
            .contains("cannot be combined"),
        "got {:?}",
        body["details"]
    );
}
