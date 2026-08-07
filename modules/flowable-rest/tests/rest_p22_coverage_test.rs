//! P22 contract tests: deployment collection GET, standalone task POST,
//! bulk task PUT, execution PUT actions, task event DELETE and the widened
//! task query parameter surface. Observable semantics follow the Java
//! resources (`DeploymentCollectionResource`, `TaskCollectionResource`,
//! `ExecutionResource`, `ExecutionCollectionResource`, `TaskEventResource`).

use chrono::{Duration, Utc};
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::task::Task;
use flowable_rest::run_server;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::net::TcpListener;

const PROC_A_BPMN: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
    xmlns:flowable="http://flowable.org/bpmn" targetNamespace="Examples">
    <process id="p22ProcA" name="P22 Proc A" isExecutable="true">
        <startEvent id="start" />
        <sequenceFlow id="f1" sourceRef="start" targetRef="taskA" />
        <userTask id="taskA" name="Alpha Review" flowable:candidateGroups="sales" />
        <sequenceFlow id="f2" sourceRef="taskA" targetRef="end" />
        <endEvent id="end" />
    </process>
</definitions>"#;

const PROC_B_BPMN: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
    xmlns:flowable="http://flowable.org/bpmn" targetNamespace="Examples">
    <process id="p22ProcB" name="P22 Proc B" isExecutable="true">
        <startEvent id="start" />
        <sequenceFlow id="f1" sourceRef="start" targetRef="taskB" />
        <userTask id="taskB" name="beta check" flowable:candidateGroups="hr" flowable:candidateUsers="kermit" />
        <sequenceFlow id="f2" sourceRef="taskB" targetRef="end" />
        <endEvent id="end" />
    </process>
</definitions>"#;

const RECEIVE_TASK_BPMN: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
    <process id="p22ReceiveProcess" name="P22 Receive Process" isExecutable="true">
        <startEvent id="start" />
        <sequenceFlow id="f1" sourceRef="start" targetRef="waitHere" />
        <receiveTask id="waitHere" name="Wait Here" />
        <sequenceFlow id="f2" sourceRef="waitHere" targetRef="afterWait" />
        <userTask id="afterWait" name="After Wait" />
        <sequenceFlow id="f3" sourceRef="afterWait" targetRef="end" />
        <endEvent id="end" />
    </process>
</definitions>"#;

const SIGNAL_CATCH_BPMN: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
    <signal id="p22Signal" name="P22 Alert" />
    <process id="p22SignalCatchProcess" name="P22 Signal Catch" isExecutable="true">
        <startEvent id="start" />
        <sequenceFlow id="f1" sourceRef="start" targetRef="catchAlert" />
        <intermediateCatchEvent id="catchAlert" name="Catch Alert">
            <signalEventDefinition signalRef="p22Signal" />
        </intermediateCatchEvent>
        <sequenceFlow id="f2" sourceRef="catchAlert" targetRef="afterSignal" />
        <userTask id="afterSignal" name="After Signal" />
        <sequenceFlow id="f3" sourceRef="afterSignal" targetRef="end" />
        <endEvent id="end" />
    </process>
</definitions>"#;

async fn spawn_server(test_name: &str) -> (Arc<ProcessEngine>, String, reqwest::Client) {
    let engine = Arc::new(ProcessEngine::new(test_name.to_string()));
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

async fn deploy(client: &reqwest::Client, base_url: &str, name: &str, resource: &str) {
    let response = client
        .post(format!("{base_url}/repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": name,
            "resourceName": format!("{}.bpmn20.xml", name.replace(' ', "-").to_lowercase()),
            "resource": resource
        }))
        .send()
        .await
        .unwrap();
    // P109: deploy (both JSON superset and multipart paths) returns 201.
    assert_eq!(response.status(), reqwest::StatusCode::CREATED);
}

async fn start_process_by_key(
    engine: &Arc<ProcessEngine>,
    client: &reqwest::Client,
    base_url: &str,
    key: &str,
    variables: Option<Value>,
) -> String {
    let process_definition_id = engine
        .get_repository_service()
        .latest_process_definition_by_key(key, None)
        .unwrap()
        .unwrap()
        .id;
    let mut body = json!({ "processDefinitionId": process_definition_id });
    if let Some(variables) = variables {
        body["variables"] = variables;
    }
    let response = client
        .post(format!("{base_url}/runtime/process-instances"))
        .basic_auth("admin", Some("test"))
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let body: Value = response.json().await.unwrap();
    body["id"].as_str().unwrap().to_string()
}

fn find_waiting_execution(engine: &Arc<ProcessEngine>, process_instance_id: &str) -> String {
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let execution_id = store
        .snapshot_event_wait_states(&mut session)
        .into_values()
        .find(|ws| ws.process_instance_id == process_instance_id)
        .map(|ws| ws.execution_id)
        .unwrap();
    let _ = session.rollback();
    execution_id
}

async fn get_tasks(client: &reqwest::Client, base_url: &str, query: &str) -> Value {
    let response = client
        .get(format!("{base_url}/runtime/tasks?{query}"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        reqwest::StatusCode::OK,
        "GET /runtime/tasks?{query}"
    );
    response.json().await.unwrap()
}

// ── A1: GET /repository/deployments ──

#[tokio::test]
async fn deployment_collection_filters_and_sorts() {
    let (_engine, base_url, client) = spawn_server("rest-p22-deployments").await;
    deploy(&client, &base_url, "P22 Alpha", PROC_A_BPMN).await;
    deploy(&client, &base_url, "P22 Beta", PROC_B_BPMN).await;

    let response = client
        .get(format!("{base_url}/repository/deployments"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["total"], 2);
    assert_eq!(body["start"], 0);
    assert_eq!(body["sort"], "id");
    assert_eq!(body["order"], "asc");
    assert!(body["data"][0]["deploymentTime"].is_string());

    let response = client
        .get(format!("{base_url}/repository/deployments?name=P22 Alpha"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["total"], 1);
    assert_eq!(body["data"][0]["name"], "P22 Alpha");

    let response = client
        .get(format!(
            "{base_url}/repository/deployments?nameLike=%25Beta%25"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["total"], 1);
    assert_eq!(body["data"][0]["name"], "P22 Beta");

    let response = client
        .get(format!(
            "{base_url}/repository/deployments?sort=name&order=desc"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["data"][0]["name"], "P22 Beta");
    assert_eq!(body["data"][1]["name"], "P22 Alpha");

    // Unknown sort property and unknown query parameter are both 400.
    let response = client
        .get(format!("{base_url}/repository/deployments?sort=bogus"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);

    let response = client
        .get(format!("{base_url}/repository/deployments?bogusParam=1"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
}

// ── A2: POST /runtime/tasks ──

#[tokio::test]
async fn create_standalone_task_returns_201() {
    let (_engine, base_url, client) = spawn_server("rest-p22-create-task").await;

    let response = client
        .post(format!("{base_url}/runtime/tasks"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "Standalone",
            "description": "created via REST",
            "assignee": "kermit",
            "owner": "fozzie",
            "priority": 20,
            "category": "misc"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::CREATED);
    let body: Value = response.json().await.unwrap();
    let task_id = body["id"].as_str().unwrap().to_string();
    assert!(!task_id.is_empty());
    assert_eq!(body["name"], "Standalone");
    assert_eq!(body["description"], "created via REST");
    assert_eq!(body["assignee"], "kermit");
    assert_eq!(body["owner"], "fozzie");
    assert_eq!(body["priority"], 20);
    assert_eq!(body["category"], "misc");

    let response = client
        .get(format!("{base_url}/runtime/tasks/{task_id}"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);

    // Java `TaskRequest.getDelegationState`: only pending/resolved are legal.
    let response = client
        .post(format!("{base_url}/runtime/tasks"))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "name": "Bad", "delegationState": "wrong" }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["details"], "Illegal value for delegationState: wrong");
}

// ── A3: PUT /runtime/tasks (bulk update) ──

#[tokio::test]
async fn bulk_update_tasks_contract() {
    let (_engine, base_url, client) = spawn_server("rest-p22-bulk-update").await;

    let mut ids = Vec::new();
    for name in ["Bulk One", "Bulk Two"] {
        let response = client
            .post(format!("{base_url}/runtime/tasks"))
            .basic_auth("admin", Some("test"))
            .json(&json!({ "name": name }))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::CREATED);
        let body: Value = response.json().await.unwrap();
        ids.push(body["id"].as_str().unwrap().to_string());
    }

    let response = client
        .put(format!("{base_url}/runtime/tasks"))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "taskIds": ids, "priority": 77, "assignee": "gonzo" }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let body: Value = response.json().await.unwrap();
    let data = body["data"].as_array().unwrap();
    assert_eq!(data.len(), 2);
    for task in data {
        assert_eq!(task["priority"], 77);
        assert_eq!(task["assignee"], "gonzo");
    }

    // Missing taskIds → 400 with the verbatim Java message.
    let response = client
        .put(format!("{base_url}/runtime/tasks"))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "priority": 1 }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
    let body: Value = response.json().await.unwrap();
    assert_eq!(
        body["details"],
        "taskIds can not be null for bulk update tasks requests"
    );

    // Unknown task id → 404 listing the missing id.
    let response = client
        .put(format!("{base_url}/runtime/tasks"))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "taskIds": ["missing-task"], "priority": 1 }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::NOT_FOUND);
    let body: Value = response.json().await.unwrap();
    assert_eq!(
        body["details"],
        "Could not find task instance with id:missing-task"
    );
}

// ── A6: DELETE /runtime/tasks/{taskId}/events/{eventId} ──

#[tokio::test]
async fn delete_task_event_contract() {
    let (engine, base_url, client) = spawn_server("rest-p22-delete-event").await;

    let response = client
        .post(format!("{base_url}/runtime/tasks"))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "name": "Event Host" }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::CREATED);
    let body: Value = response.json().await.unwrap();
    let task_id = body["id"].as_str().unwrap().to_string();

    let event = engine
        .get_history_service()
        .record_task_event(&task_id, "AddUserLink", vec!["kermit".to_string()], None)
        .unwrap();

    // Unknown event id on an existing task → 404 with the Java message shape.
    let response = client
        .delete(format!(
            "{base_url}/runtime/tasks/{task_id}/events/no-such-event"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::NOT_FOUND);
    let body: Value = response.json().await.unwrap();
    assert_eq!(
        body["details"],
        format!(
            "Task '{}' does not have an event with id 'no-such-event'.",
            task_id
        )
    );

    let response = client
        .delete(format!(
            "{base_url}/runtime/tasks/{task_id}/events/{}",
            event.id
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::NO_CONTENT);

    // The event is gone afterwards.
    let response = client
        .get(format!(
            "{base_url}/runtime/tasks/{task_id}/events/{}",
            event.id
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::NOT_FOUND);

    // Unknown task → 404 as well.
    let response = client
        .delete(format!(
            "{base_url}/runtime/tasks/no-such-task/events/whatever"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::NOT_FOUND);
}

// ── A4: PUT /runtime/executions/{executionId} ──

#[tokio::test]
async fn put_execution_trigger_completes_receive_task() {
    let (engine, base_url, client) = spawn_server("rest-p22-exec-trigger").await;
    deploy(&client, &base_url, "P22 Receive", RECEIVE_TASK_BPMN).await;
    let process_instance_id =
        start_process_by_key(&engine, &client, &base_url, "p22ReceiveProcess", None).await;
    let execution_id = find_waiting_execution(&engine, &process_instance_id);

    let response = client
        .put(format!("{base_url}/runtime/executions/{execution_id}"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "action": "trigger",
            "variables": [{ "name": "triggerVar", "value": "from-put" }]
        }))
        .send()
        .await
        .unwrap();
    // Java: 200 + ExecutionResponse when the execution survives, 204 when it
    // finished with the action.
    assert!(
        response.status() == reqwest::StatusCode::OK
            || response.status() == reqwest::StatusCode::NO_CONTENT,
        "unexpected status {}",
        response.status()
    );

    let tasks = engine
        .get_task_service()
        .get_tasks_by_process_instance_id(process_instance_id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].task_definition_key, "afterWait");

    let variables = engine
        .get_variable_service()
        .get_variables(process_instance_id.clone())
        .unwrap();
    assert_eq!(variables.get("triggerVar"), Some(&json!("from-put")));
}

#[tokio::test]
async fn put_execution_action_error_paths() {
    let (engine, base_url, client) = spawn_server("rest-p22-exec-errors").await;
    deploy(&client, &base_url, "P22 Receive Err", RECEIVE_TASK_BPMN).await;
    let process_instance_id =
        start_process_by_key(&engine, &client, &base_url, "p22ReceiveProcess", None).await;
    let execution_id = find_waiting_execution(&engine, &process_instance_id);

    // Unknown execution → 404.
    let response = client
        .put(format!("{base_url}/runtime/executions/no-such-execution"))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "action": "trigger" }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::NOT_FOUND);

    // Illegal action → 400 "Invalid action: 'dance'.".
    let response = client
        .put(format!("{base_url}/runtime/executions/{execution_id}"))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "action": "dance" }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["details"], "Invalid action: 'dance'.");

    // signalEventReceived without a name → 400 (no trailing period in Java).
    let response = client
        .put(format!("{base_url}/runtime/executions/{execution_id}"))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "action": "signalEventReceived" }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["details"], "Signal name is required");

    // messageEventReceived without a name → 400.
    let response = client
        .put(format!("{base_url}/runtime/executions/{execution_id}"))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "action": "messageEventReceived" }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["details"], "Message name is required");

    // Java MessageEventReceivedCmd: no subscription → FlowableException (500).
    let response = client
        .put(format!("{base_url}/runtime/executions/{execution_id}"))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "action": "messageEventReceived", "messageName": "nope" }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        reqwest::StatusCode::INTERNAL_SERVER_ERROR
    );
}

#[tokio::test]
async fn put_execution_signal_event_received() {
    let (engine, base_url, client) = spawn_server("rest-p22-exec-signal").await;
    deploy(&client, &base_url, "P22 Signal Catch", SIGNAL_CATCH_BPMN).await;
    let process_instance_id =
        start_process_by_key(&engine, &client, &base_url, "p22SignalCatchProcess", None).await;
    let execution_id = find_waiting_execution(&engine, &process_instance_id);

    let response = client
        .put(format!("{base_url}/runtime/executions/{execution_id}"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "action": "signalEventReceived",
            "signalName": "P22 Alert",
            "variables": [{ "name": "signalVar", "value": 7 }]
        }))
        .send()
        .await
        .unwrap();
    assert!(
        response.status() == reqwest::StatusCode::OK
            || response.status() == reqwest::StatusCode::NO_CONTENT,
        "unexpected status {}",
        response.status()
    );

    let tasks = engine
        .get_task_service()
        .get_tasks_by_process_instance_id(process_instance_id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].task_definition_key, "afterSignal");

    let variables = engine
        .get_variable_service()
        .get_variables(process_instance_id.clone())
        .unwrap();
    assert_eq!(variables.get("signalVar"), Some(&json!(7)));
}

// ── A5: PUT /runtime/executions (collection action) ──

#[tokio::test]
async fn put_executions_collection_broadcasts_signal() {
    let (engine, base_url, client) = spawn_server("rest-p22-exec-broadcast").await;
    deploy(&client, &base_url, "P22 Broadcast", SIGNAL_CATCH_BPMN).await;
    let pi1 =
        start_process_by_key(&engine, &client, &base_url, "p22SignalCatchProcess", None).await;
    let pi2 =
        start_process_by_key(&engine, &client, &base_url, "p22SignalCatchProcess", None).await;

    // Illegal action → 400 "Illegal action: 'trigger'.".
    let response = client
        .put(format!("{base_url}/runtime/executions"))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "action": "trigger" }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["details"], "Illegal action: 'trigger'.");

    // Missing signal name → 400 with a trailing period (collection variant).
    let response = client
        .put(format!("{base_url}/runtime/executions"))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "action": "signalEventReceived" }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["details"], "Signal name is required.");

    let response = client
        .put(format!("{base_url}/runtime/executions"))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "action": "signalEventReceived", "signalName": "P22 Alert" }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::NO_CONTENT);

    for process_instance_id in [pi1, pi2] {
        let tasks = engine
            .get_task_service()
            .get_tasks_by_process_instance_id(process_instance_id)
            .unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].task_definition_key, "afterSignal");
    }
}

// ── B: task query parameters with filter-effect assertions ──

#[tokio::test]
async fn task_query_candidate_group_in_filters() {
    let (engine, base_url, client) = spawn_server("rest-p22-cand-group-in").await;
    deploy(&client, &base_url, "P22 Proc A", PROC_A_BPMN).await;
    deploy(&client, &base_url, "P22 Proc B", PROC_B_BPMN).await;
    start_process_by_key(&engine, &client, &base_url, "p22ProcA", None).await;
    start_process_by_key(&engine, &client, &base_url, "p22ProcB", None).await;

    let body = get_tasks(&client, &base_url, "candidateGroupIn=sales,management").await;
    assert_eq!(body["total"], 1);
    assert_eq!(body["data"][0]["name"], "Alpha Review");

    // POST /query/tasks takes the same filter as a JSON array.
    let response = client
        .post(format!("{base_url}/query/tasks"))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "candidateGroupIn": ["hr"] }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["total"], 1);
    assert_eq!(body["data"][0]["name"], "beta check");
}

#[tokio::test]
async fn task_query_candidate_or_assigned() {
    let (engine, base_url, client) = spawn_server("rest-p22-cand-or-assigned").await;
    deploy(&client, &base_url, "P22 Proc A", PROC_A_BPMN).await;
    deploy(&client, &base_url, "P22 Proc B", PROC_B_BPMN).await;
    // sales task: unrelated; hr task: kermit is candidate user (unassigned).
    start_process_by_key(&engine, &client, &base_url, "p22ProcA", None).await;
    start_process_by_key(&engine, &client, &base_url, "p22ProcB", None).await;

    // Standalone task directly assigned to kermit.
    let response = client
        .post(format!("{base_url}/runtime/tasks"))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "name": "Assigned Task", "assignee": "kermit" }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::CREATED);

    let body = get_tasks(&client, &base_url, "candidateOrAssigned=kermit").await;
    assert_eq!(body["total"], 2);
    let names: Vec<&str> = body["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|task| task["name"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"Assigned Task"));
    assert!(names.contains(&"beta check"));

    // Group membership route: fozzie belongs to group hr → candidate group hit.
    engine
        .get_identity_service()
        .save_group(flowable_engine::identity::entities::Group {
            id: "hr".to_string(),
            name: "HR".to_string(),
            group_type: None,
        });
    engine
        .get_identity_service()
        .create_membership("fozzie".to_string(), "hr".to_string());
    let body = get_tasks(&client, &base_url, "candidateOrAssigned=fozzie").await;
    assert_eq!(body["total"], 1);
    assert_eq!(body["data"][0]["name"], "beta check");
}

#[tokio::test]
async fn task_query_name_like_ignore_case() {
    let (engine, base_url, client) = spawn_server("rest-p22-name-ilike").await;
    deploy(&client, &base_url, "P22 Proc A", PROC_A_BPMN).await;
    deploy(&client, &base_url, "P22 Proc B", PROC_B_BPMN).await;
    start_process_by_key(&engine, &client, &base_url, "p22ProcA", None).await;
    start_process_by_key(&engine, &client, &base_url, "p22ProcB", None).await;

    let body = get_tasks(&client, &base_url, "nameLikeIgnoreCase=%25ALPHA%25").await;
    assert_eq!(body["total"], 1);
    assert_eq!(body["data"][0]["name"], "Alpha Review");
}

#[tokio::test]
async fn task_query_created_after() {
    let (engine, base_url, client) = spawn_server("rest-p22-created-after").await;

    let mut old_task = Task::new(
        String::new(),
        String::new(),
        String::new(),
        String::new(),
        "Old Task".to_string(),
    );
    old_task.created_time = Some(Utc::now() - Duration::days(2));
    engine.get_task_service().create_task(old_task).unwrap();

    let mut new_task = Task::new(
        String::new(),
        String::new(),
        String::new(),
        String::new(),
        "New Task".to_string(),
    );
    new_task.created_time = Some(Utc::now());
    engine.get_task_service().create_task(new_task).unwrap();

    let threshold = (Utc::now() - Duration::days(1)).to_rfc3339();
    let body = get_tasks(
        &client,
        &base_url,
        &format!("createdAfter={}", urlencode(&threshold)),
    )
    .await;
    assert_eq!(body["total"], 1);
    assert_eq!(body["data"][0]["name"], "New Task");

    let body = get_tasks(
        &client,
        &base_url,
        &format!("createdBefore={}", urlencode(&threshold)),
    )
    .await;
    assert_eq!(body["total"], 1);
    assert_eq!(body["data"][0]["name"], "Old Task");

    // Malformed date → 400.
    let response = client
        .get(format!("{base_url}/runtime/tasks?createdAfter=not-a-date"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
}

fn urlencode(value: &str) -> String {
    value.replace('+', "%2B").replace(':', "%3A")
}

#[tokio::test]
async fn task_query_process_definition_key() {
    let (engine, base_url, client) = spawn_server("rest-p22-proc-def-key").await;
    deploy(&client, &base_url, "P22 Proc A", PROC_A_BPMN).await;
    deploy(&client, &base_url, "P22 Proc B", PROC_B_BPMN).await;
    start_process_by_key(&engine, &client, &base_url, "p22ProcA", None).await;
    start_process_by_key(&engine, &client, &base_url, "p22ProcB", None).await;

    let body = get_tasks(&client, &base_url, "processDefinitionKey=p22ProcA").await;
    assert_eq!(body["total"], 1);
    assert_eq!(body["data"][0]["name"], "Alpha Review");

    let body = get_tasks(&client, &base_url, "processDefinitionKeyLike=%25ProcB%25").await;
    assert_eq!(body["total"], 1);
    assert_eq!(body["data"][0]["name"], "beta check");

    let body = get_tasks(&client, &base_url, "processDefinitionName=P22 Proc A").await;
    assert_eq!(body["total"], 1);
    assert_eq!(body["data"][0]["name"], "Alpha Review");
}

#[tokio::test]
async fn task_query_include_process_variables() {
    let (engine, base_url, client) = spawn_server("rest-p22-include-vars").await;
    deploy(&client, &base_url, "P22 Proc A", PROC_A_BPMN).await;
    let process_instance_id = start_process_by_key(
        &engine,
        &client,
        &base_url,
        "p22ProcA",
        Some(json!([{ "name": "reviewTarget", "value": "unit-p22" }])),
    )
    .await;

    let body = get_tasks(
        &client,
        &base_url,
        &format!("processInstanceId={process_instance_id}&includeProcessVariables=true"),
    )
    .await;
    assert_eq!(body["total"], 1);
    let variables = body["data"][0]["variables"].as_array().unwrap();
    let review = variables
        .iter()
        .find(|variable| variable["name"] == "reviewTarget")
        .expect("process variable in task response");
    assert_eq!(review["value"], "unit-p22");
    assert_eq!(review["scope"], "global");

    // Without the flag the variables array stays empty.
    let body = get_tasks(
        &client,
        &base_url,
        &format!("processInstanceId={process_instance_id}"),
    )
    .await;
    assert_eq!(body["data"][0]["variables"].as_array().unwrap().len(), 0);
}

// ── B: smoke — remaining new parameters are accepted (no 400) ──

#[tokio::test]
async fn task_query_new_parameters_smoke() {
    let (_engine, base_url, client) = spawn_server("rest-p22-param-smoke").await;

    let get_params = [
        "description=x",
        "descriptionLike=%25x%25",
        "assigneeLike=%25k%25",
        "ownerLike=%25k%25",
        "involvedUser=kermit",
        "ignoreAssignee=true",
        "processInstanceIdWithChildren=pi-1",
        "withoutProcessInstanceId=true",
        "processInstanceBusinessKeyLike=%25key%25",
        "processDefinitionId=def-1",
        "processDefinitionNameLike=%25name%25",
        "executionId=exec-1",
        "createdOn=2020-01-01T00%3A00%3A00Z",
        "excludeSubTasks=true",
        "taskDefinitionKeys=a,b",
        "withoutCategory=true",
        "categoryIn=a,b",
        "categoryNotIn=c",
        "includeTaskLocalVariables=true",
        "scopeId=s1",
        "scopeType=cmmn",
        "scopeDefinitionId=sd1",
        "withoutScopeId=true",
        "rootScopeId=r1",
        "parentScopeId=p1",
        "propagatedStageInstanceId=st1",
        "tenantIdLike=%25t%25",
        "withoutTenantId=true",
        "candidateGroups=a,b",
        "active=true",
    ];
    for param in get_params {
        let response = client
            .get(format!("{base_url}/runtime/tasks?{param}"))
            .basic_auth("admin", Some("test"))
            .send()
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            reqwest::StatusCode::OK,
            "GET /runtime/tasks?{param}"
        );
    }

    // POST /query/tasks accepts the same surface as JSON fields.
    let response = client
        .post(format!("{base_url}/query/tasks"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "nameLikeIgnoreCase": "%a%",
            "candidateGroupIn": ["a", "b"],
            "candidateOrAssigned": "kermit",
            "involvedUser": "kermit",
            "processDefinitionKey": "someKey",
            "createdAfter": "2020-01-01T00:00:00Z",
            "excludeSubTasks": true,
            "includeProcessVariables": true,
            "taskVariables": [
                { "name": "v", "operation": "equals", "value": 1 }
            ]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);

    // Unknown parameter is still a hard 400.
    let response = client
        .get(format!("{base_url}/runtime/tasks?definitelyNotAParam=1"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
}
