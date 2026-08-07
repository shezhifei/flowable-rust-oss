use chrono::Utc;
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::history::historic_entities::HistoricTaskInstance;
use flowable_engine::task::Task;
use flowable_rest::run_server;
use serde_json::{Value, json};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
use tokio::net::TcpListener;

const TASK_ACTION_PROCESS_BPMN: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             targetNamespace="Examples">
    <process id="taskActionProcess" name="Task Action Process" isExecutable="true">
        <startEvent id="startEvent" />
        <sequenceFlow id="flow1" sourceRef="startEvent" targetRef="reviewTask" />
        <userTask id="reviewTask" name="Review action" />
        <sequenceFlow id="flow2" sourceRef="reviewTask" targetRef="endEvent" />
        <endEvent id="endEvent" />
    </process>
</definitions>"#;

const TASK_COMPLETE_VARIABLES_PROCESS_BPMN: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             targetNamespace="Examples">
    <process id="taskCompleteVariablesProcess" name="Task Complete Variables Process" isExecutable="true">
        <startEvent id="startEvent" />
        <sequenceFlow id="flow1" sourceRef="startEvent" targetRef="reviewTask" />
        <userTask id="reviewTask" name="Review action" />
        <sequenceFlow id="flow2" sourceRef="reviewTask" targetRef="decision" />
        <exclusiveGateway id="decision" default="rejectedFlow" />
        <sequenceFlow id="approvedFlow" sourceRef="decision" targetRef="approvedTask">
            <conditionExpression><![CDATA[${approved == true}]]></conditionExpression>
        </sequenceFlow>
        <sequenceFlow id="rejectedFlow" sourceRef="decision" targetRef="rejectedTask" />
        <userTask id="approvedTask" name="Approved follow-up" />
        <sequenceFlow id="flow3" sourceRef="approvedTask" targetRef="endEvent" />
        <userTask id="rejectedTask" name="Rejected follow-up" />
        <sequenceFlow id="flow4" sourceRef="rejectedTask" targetRef="endEvent" />
        <endEvent id="endEvent" />
    </process>
</definitions>"#;

const TASK_QUERY_VARIABLE_PROCESS_BPMN: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             targetNamespace="Examples">
    <process id="taskQueryVariableProcess" name="Task Query Variable Process" isExecutable="true">
        <startEvent id="startEvent" />
        <sequenceFlow id="flow1" sourceRef="startEvent" targetRef="reviewTask" />
        <userTask id="reviewTask" name="Review variable query" />
        <sequenceFlow id="flow2" sourceRef="reviewTask" targetRef="endEvent" />
        <endEvent id="endEvent" />
    </process>
</definitions>"#;

static ENGINE_COUNTER: AtomicU64 = AtomicU64::new(0);

async fn spawn_server() -> (Arc<ProcessEngine>, String, reqwest::Client) {
    let engine_id = ENGINE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let engine = Arc::new(ProcessEngine::new(format!(
        "rest-task-action-native-contract-{engine_id}"
    )));
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
    deploy_process_xml(
        client,
        base_url,
        "Task action process",
        "task-action-process.bpmn20.xml",
        TASK_ACTION_PROCESS_BPMN,
    )
    .await
}

async fn deploy_complete_variables_process(client: &reqwest::Client, base_url: &str) {
    deploy_process_xml(
        client,
        base_url,
        "Task complete variables process",
        "task-complete-variables-process.bpmn20.xml",
        TASK_COMPLETE_VARIABLES_PROCESS_BPMN,
    )
    .await
}

async fn deploy_task_query_variable_process(client: &reqwest::Client, base_url: &str) {
    deploy_process_xml(
        client,
        base_url,
        "Task query variable process",
        "task-query-variable-process.bpmn20.xml",
        TASK_QUERY_VARIABLE_PROCESS_BPMN,
    )
    .await
}

async fn deploy_process_xml(
    client: &reqwest::Client,
    base_url: &str,
    name: &str,
    resource_name: &str,
    resource: &str,
) {
    let response = client
        .post(format!("{base_url}/repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": name,
            "resourceName": resource_name,
            "resource": resource
        }))
        .send()
        .await
        .unwrap();
    // P109: deploy (both JSON superset and multipart paths) returns 201.
    assert_eq!(response.status(), reqwest::StatusCode::CREATED);
}

async fn start_process(
    client: &reqwest::Client,
    base_url: &str,
    process_definition_id: &str,
) -> Value {
    let response = client
        .post(format!("{base_url}/runtime/process-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "processDefinitionId": process_definition_id
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    response.json::<Value>().await.unwrap()
}

async fn latest_definition_id(engine: &ProcessEngine, key: &str) -> String {
    engine
        .get_repository_service()
        .latest_process_definition_by_key(key, None)
        .unwrap()
        .unwrap()
        .id
}

async fn active_task(client: &reqwest::Client, base_url: &str) -> Value {
    let response = client
        .get(format!("{base_url}/runtime/tasks"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    response.json::<Value>().await.unwrap()["data"][0].clone()
}

async fn get_task(client: &reqwest::Client, base_url: &str, task_id: &str) -> Value {
    let response = client
        .get(format!("{base_url}/runtime/tasks/{task_id}"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    response.json::<Value>().await.unwrap()
}

async fn tasks_for_process(
    client: &reqwest::Client,
    base_url: &str,
    process_instance_id: &str,
) -> Value {
    let response = client
        .get(format!(
            "{base_url}/runtime/tasks?processInstanceId={process_instance_id}&start=0&size=10"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    response.json::<Value>().await.unwrap()
}

async fn historic_process_instances(client: &reqwest::Client, base_url: &str) -> Value {
    let response = client
        .get(format!(
            "{base_url}/history/historic-process-instances?start=0&size=10"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    response.json::<Value>().await.unwrap()
}

async fn historic_variables_for_process(
    client: &reqwest::Client,
    base_url: &str,
    process_instance_id: &str,
    variable_name: &str,
) -> Value {
    let response = client
        .get(format!(
            "{base_url}/history/historic-variable-instances?processInstanceId={process_instance_id}&variableName={variable_name}&start=0&size=10"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    response.json::<Value>().await.unwrap()
}

#[tokio::test]
async fn query_tasks_filters_by_task_and_process_instance_variables() {
    let (engine, base_url, client) = spawn_server().await;
    deploy_task_query_variable_process(&client, &base_url).await;
    let process_definition_id = latest_definition_id(&engine, "taskQueryVariableProcess").await;
    let alpha = start_process(&client, &base_url, &process_definition_id).await;
    let beta = start_process(&client, &base_url, &process_definition_id).await;

    let alpha_tasks = tasks_for_process(&client, &base_url, alpha["id"].as_str().unwrap()).await;
    let beta_tasks = tasks_for_process(&client, &base_url, beta["id"].as_str().unwrap()).await;
    assert_eq!(alpha_tasks["total"], 1, "body was: {alpha_tasks}");
    assert_eq!(beta_tasks["total"], 1, "body was: {beta_tasks}");

    let alpha_task_id = alpha_tasks["data"][0]["id"].as_str().unwrap();
    let beta_task_id = beta_tasks["data"][0]["id"].as_str().unwrap();
    let alpha_execution_id = alpha_tasks["data"][0]["executionId"].as_str().unwrap();
    let beta_execution_id = beta_tasks["data"][0]["executionId"].as_str().unwrap();

    engine
        .get_task_service()
        .set_task_local_variable(
            alpha_task_id.to_string(),
            "approval".to_string(),
            json!("TaskAccepted"),
        )
        .unwrap();
    engine
        .get_task_service()
        .set_task_local_variable(alpha_task_id.to_string(), "rank".to_string(), json!(7))
        .unwrap();
    engine
        .get_task_service()
        .set_task_local_variable(
            beta_task_id.to_string(),
            "approval".to_string(),
            json!("TaskRejected"),
        )
        .unwrap();
    engine
        .get_variable_service()
        .set_variable(
            alpha_execution_id.to_string(),
            "route".to_string(),
            json!("Approved"),
        )
        .unwrap();
    engine
        .get_variable_service()
        .set_variable(
            beta_execution_id.to_string(),
            "route".to_string(),
            json!("Rejected"),
        )
        .unwrap();

    for request_body in [
        json!({"taskVariables": [{"name": "approval", "operation": "equals", "value": "TaskAccepted"}]}),
        json!({"taskVariables": [{"name": "approval", "operation": "equalsIgnoreCase", "value": "taskaccepted"}]}),
        json!({"taskVariables": [{"name": "approval", "operation": "notEquals", "value": "TaskRejected"}]}),
        json!({"taskVariables": [{"name": "approval", "operation": "notEqualsIgnoreCase", "value": "taskrejected"}]}),
        json!({"taskVariables": [{"operation": "equals", "value": 7}]}),
        json!({"processInstanceVariables": [{"name": "route", "operation": "equalsIgnoreCase", "value": "approved"}]}),
    ] {
        let response = client
            .post(format!("{base_url}/query/tasks"))
            .basic_auth("admin", Some("test"))
            .json(&request_body)
            .send()
            .await
            .unwrap();

        assert_eq!(response.status(), reqwest::StatusCode::OK);
        let body: Value = response.json().await.unwrap();
        assert_eq!(body["total"], 1, "body was: {body}");
        assert_eq!(body["data"][0]["id"], alpha_task_id);
    }
}

#[tokio::test]
async fn query_tasks_variable_filters_return_structured_bad_request_for_errors() {
    let (_engine, base_url, client) = spawn_server().await;

    for (request_body, expected_detail) in [
        (
            json!({"taskVariables": [{"name": "approval", "value": "Accepted"}]}),
            "Variable operation is missing for variable: approval",
        ),
        (
            json!({"taskVariables": [{"name": "approval", "operation": "equals"}]}),
            "Variable value is missing for variable: approval",
        ),
        (
            json!({"taskVariables": [{"name": "approval", "operation": "equals", "value": null}]}),
            "Variable value is missing for variable: approval",
        ),
        (
            json!({"taskVariables": [{"operation": "notEquals", "value": "Accepted"}]}),
            "Value-only query (without a variable-name) is only supported when using 'equals' operation.",
        ),
        (
            json!({"taskVariables": [{"name": "approval", "operation": "equalsIgnoreCase", "value": 7}]}),
            "Only string variable values are supported when ignoring casing",
        ),
        (
            json!({"processInstanceVariables": [{"name": "approval", "operation": "bogusOp", "value": "Accept"}]}),
            "Unsupported variable query operation: bogusOp",
        ),
    ] {
        let response = client
            .post(format!("{base_url}/query/tasks"))
            .basic_auth("admin", Some("test"))
            .json(&request_body)
            .send()
            .await
            .unwrap();

        assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
        let body: Value = response.json().await.unwrap();
        assert_eq!(body["code"], "BAD_REQUEST");
        assert_eq!(body["message"], "Bad Request");
        assert!(
            body["details"].as_str().unwrap().contains(expected_detail),
            "details were: {}",
            body["details"]
        );
    }
}

#[tokio::test]
async fn task_action_claim_and_unclaim_update_assignee() {
    let (engine, base_url, client) = spawn_server().await;
    deploy_process(&client, &base_url).await;
    let process_definition_id = engine
        .get_repository_service()
        .latest_process_definition_by_key("taskActionProcess", None)
        .unwrap()
        .unwrap()
        .id;
    start_process(&client, &base_url, &process_definition_id).await;
    let task = active_task(&client, &base_url).await;
    let task_id = task["id"].as_str().unwrap();
    assert!(task["assignee"].is_null());
    assert_eq!(task["state"], "created");
    assert!(task["claimTime"].is_null());

    let claim = client
        .post(format!("{base_url}/runtime/tasks/{task_id}"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "action": "claim",
            "assignee": "kermit"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(claim.status(), reqwest::StatusCode::OK);
    assert!(claim.text().await.unwrap().is_empty());

    let claimed_task = get_task(&client, &base_url, task_id).await;
    assert_eq!(claimed_task["assignee"], "kermit");
    assert_eq!(claimed_task["state"], "claimed");
    assert!(claimed_task["claimTime"].is_string());

    let unclaim = client
        .post(format!("{base_url}/runtime/tasks/{task_id}"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "action": "claim",
            "assignee": null
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(unclaim.status(), reqwest::StatusCode::OK);
    assert!(unclaim.text().await.unwrap().is_empty());

    let unclaimed_task = get_task(&client, &base_url, task_id).await;
    assert!(unclaimed_task["assignee"].is_null());
    assert_eq!(unclaimed_task["state"], "created");
    assert!(unclaimed_task["claimTime"].is_null());

    let events = client
        .get(format!("{base_url}/runtime/tasks/{task_id}/events"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(events.status(), reqwest::StatusCode::OK);
    let events: Value = events.json().await.unwrap();
    assert!(events.as_array().unwrap().iter().any(|event| {
        event["action"] == "AddUserLink" && event["message"] == json!(["kermit", "assignee"])
    }));
    assert!(events.as_array().unwrap().iter().any(|event| {
        event["action"] == "DeleteUserLink" && event["message"] == json!(["kermit", "assignee"])
    }));
}

#[tokio::test]
async fn task_action_claim_rejects_different_assignee_and_keeps_query_visibility() {
    let (engine, base_url, client) = spawn_server().await;
    deploy_process(&client, &base_url).await;
    let process_definition_id = latest_definition_id(&engine, "taskActionProcess").await;
    start_process(&client, &base_url, &process_definition_id).await;
    let task = active_task(&client, &base_url).await;
    let task_id = task["id"].as_str().unwrap();

    let claim = client
        .post(format!("{base_url}/runtime/tasks/{task_id}"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "action": "claim",
            "assignee": "kermit"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(claim.status(), reqwest::StatusCode::OK);

    let same_claim = client
        .post(format!("{base_url}/runtime/tasks/{task_id}"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "action": "claim",
            "assignee": "kermit"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(same_claim.status(), reqwest::StatusCode::OK);

    let second_claim = client
        .post(format!("{base_url}/runtime/tasks/{task_id}"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "action": "claim",
            "assignee": "fozzie"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(second_claim.status(), reqwest::StatusCode::CONFLICT);
    let second_claim_body = second_claim.json::<Value>().await.unwrap();
    assert_eq!(second_claim_body["code"], "CONFLICT");
    assert_eq!(second_claim_body["message"], "Conflict");
    assert!(
        second_claim_body["details"]
            .as_str()
            .unwrap()
            .contains("already claimed")
    );
    assert!(
        second_claim_body["details"]
            .as_str()
            .unwrap()
            .contains("kermit")
    );

    let claimed_task = get_task(&client, &base_url, task_id).await;
    assert_eq!(claimed_task["assignee"], "kermit");

    let kermit_tasks = client
        .get(format!("{base_url}/runtime/tasks?assignee=kermit"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(kermit_tasks.status(), reqwest::StatusCode::OK);
    let kermit_tasks_body: Value = kermit_tasks.json().await.unwrap();
    assert_eq!(kermit_tasks_body["total"], 1);
    assert_eq!(kermit_tasks_body["data"][0]["id"], task_id);
    assert_eq!(kermit_tasks_body["data"][0]["assignee"], "kermit");

    let fozzie_tasks = client
        .get(format!("{base_url}/runtime/tasks?assignee=fozzie"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(fozzie_tasks.status(), reqwest::StatusCode::OK);
    let fozzie_tasks_body: Value = fozzie_tasks.json().await.unwrap();
    assert_eq!(fozzie_tasks_body["total"], 0);
}

#[tokio::test]
async fn task_action_delegate_sets_owner_assignee_and_pending_state() {
    let (engine, base_url, client) = spawn_server().await;
    deploy_process(&client, &base_url).await;
    let process_definition_id = latest_definition_id(&engine, "taskActionProcess").await;
    start_process(&client, &base_url, &process_definition_id).await;
    let task = active_task(&client, &base_url).await;
    let task_id = task["id"].as_str().unwrap();

    let claim = client
        .post(format!("{base_url}/runtime/tasks/{task_id}"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "action": "claim",
            "assignee": "kermit"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(claim.status(), reqwest::StatusCode::OK);

    let delegate = client
        .post(format!("{base_url}/runtime/tasks/{task_id}"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "action": "delegate",
            "userId": "fozzie"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(delegate.status(), reqwest::StatusCode::OK);
    assert!(delegate.text().await.unwrap().is_empty());

    let delegated_task = get_task(&client, &base_url, task_id).await;
    assert_eq!(delegated_task["owner"], "kermit");
    assert_eq!(delegated_task["assignee"], "fozzie");
    assert_eq!(delegated_task["delegationState"], "pending");
}

#[tokio::test]
async fn task_action_resolve_marks_pending_delegate_resolved() {
    let (engine, base_url, client) = spawn_server().await;
    deploy_process(&client, &base_url).await;
    let process_definition_id = latest_definition_id(&engine, "taskActionProcess").await;
    start_process(&client, &base_url, &process_definition_id).await;
    let task = active_task(&client, &base_url).await;
    let task_id = task["id"].as_str().unwrap();

    let claim = client
        .post(format!("{base_url}/runtime/tasks/{task_id}"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "action": "claim",
            "assignee": "kermit"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(claim.status(), reqwest::StatusCode::OK);

    let delegate = client
        .post(format!("{base_url}/runtime/tasks/{task_id}"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "action": "delegate",
            "assignee": "fozzie"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(delegate.status(), reqwest::StatusCode::OK);

    let resolve = client
        .post(format!("{base_url}/runtime/tasks/{task_id}"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "action": "resolve"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resolve.status(), reqwest::StatusCode::OK);
    assert!(resolve.text().await.unwrap().is_empty());

    let resolved_task = get_task(&client, &base_url, task_id).await;
    assert_eq!(resolved_task["assignee"], "kermit");
    assert_eq!(resolved_task["owner"], "kermit");
    assert_eq!(resolved_task["delegationState"], "resolved");
}

#[tokio::test]
async fn task_action_complete_finishes_task_and_advances_process() {
    let (engine, base_url, client) = spawn_server().await;
    deploy_process(&client, &base_url).await;
    let process_definition_id = engine
        .get_repository_service()
        .latest_process_definition_by_key("taskActionProcess", None)
        .unwrap()
        .unwrap()
        .id;
    let process_instance = start_process(&client, &base_url, &process_definition_id).await;
    let process_instance_id = process_instance["id"].as_str().unwrap();
    let tasks = tasks_for_process(&client, &base_url, process_instance_id).await;
    assert_eq!(tasks["total"], 1);
    let task_id = tasks["data"][0]["id"].as_str().unwrap();

    let complete = client
        .post(format!("{base_url}/runtime/tasks/{task_id}"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "action": "complete"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(complete.status(), reqwest::StatusCode::OK);
    assert!(complete.text().await.unwrap().is_empty());

    let completed_task = client
        .get(format!("{base_url}/runtime/tasks/{task_id}"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(completed_task.status(), reqwest::StatusCode::NOT_FOUND);
    let completed_task_body = completed_task.json::<Value>().await.unwrap();
    assert_eq!(completed_task_body["code"], "NOT_FOUND");

    let remaining_tasks = tasks_for_process(&client, &base_url, process_instance_id).await;
    assert_eq!(remaining_tasks["total"], 0);
    assert!(remaining_tasks["data"].as_array().unwrap().is_empty());

    let history = historic_process_instances(&client, &base_url).await;
    let completed_instance = history["data"]
        .as_array()
        .unwrap()
        .iter()
        .find(|instance| instance["id"] == process_instance["id"])
        .expect("completed process instance should be present in history");
    assert!(completed_instance["endTime"].is_string());
}

#[tokio::test]
async fn task_action_complete_variables_are_visible_to_following_nodes() {
    let (engine, base_url, client) = spawn_server().await;
    deploy_complete_variables_process(&client, &base_url).await;
    let process_definition_id = latest_definition_id(&engine, "taskCompleteVariablesProcess").await;
    let process_instance = start_process(&client, &base_url, &process_definition_id).await;
    let process_instance_id = process_instance["id"].as_str().unwrap();
    let tasks = tasks_for_process(&client, &base_url, process_instance_id).await;
    let task_id = tasks["data"][0]["id"].as_str().unwrap();

    let complete = client
        .post(format!("{base_url}/runtime/tasks/{task_id}"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "action": "complete",
            "variables": [{
                "name": "approved",
                "type": "boolean",
                "value": true
            }, {
                "name": "routeNote",
                "type": "string",
                "value": "from-action"
            }]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(complete.status(), reqwest::StatusCode::OK);
    assert!(complete.text().await.unwrap().is_empty());

    let remaining_tasks = tasks_for_process(&client, &base_url, process_instance_id).await;
    assert_eq!(remaining_tasks["total"], 1);
    assert_eq!(
        remaining_tasks["data"][0]["taskDefinitionKey"],
        "approvedTask"
    );

    let follow_up_task_id = remaining_tasks["data"][0]["id"].as_str().unwrap();
    let variables_response = client
        .get(format!(
            "{base_url}/runtime/tasks/{follow_up_task_id}/variables"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(variables_response.status(), reqwest::StatusCode::OK);
    let variables_body: Value = variables_response.json().await.unwrap();
    assert!(
        variables_body
            .as_array()
            .unwrap()
            .iter()
            .any(|variable| variable["name"] == "approved" && variable["value"] == true)
    );
}

#[tokio::test]
async fn task_action_complete_with_form_definition_persists_instance_and_outcome() {
    // Java TaskResource.completeTask + formDefinitionId → CompleteTaskWithFormCmd
    let (engine, base_url, client) = spawn_server().await;
    deploy_process(&client, &base_url).await;

    // Deploy a form definition the complete action will reference.
    let form_deploy = client
        .post(format!("{base_url}/form-repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "Task action form",
            "resources": [{
                "resourceName": "review.form",
                "resource": json!({
                    "key": "reviewForm",
                    "name": "Review form",
                    "resourceName": "review.form",
                    "outcomeVariableName": "reviewOutcome",
                    "outcomes": [
                        { "id": "approved", "name": "Approved" },
                        { "id": "rejected", "name": "Rejected" }
                    ],
                    "fields": [
                        { "id": "approved", "name": "Approved", "type": "boolean", "required": true },
                        { "id": "note", "name": "Note", "type": "string" }
                    ]
                }).to_string()
            }]
        }))
        .send()
        .await
        .unwrap();
    assert!(
        form_deploy.status().is_success(),
        "form deploy status={}",
        form_deploy.status()
    );

    let form_defs = client
        .get(format!(
            "{base_url}/form-repository/form-definitions?key=reviewForm"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(form_defs.status(), reqwest::StatusCode::OK);
    let form_defs_body: Value = form_defs.json().await.unwrap();
    let form_definition_id = form_defs_body["data"][0]["id"]
        .as_str()
        .unwrap()
        .to_string();

    let process_definition_id = latest_definition_id(&engine, "taskActionProcess").await;
    let process_instance = start_process(&client, &base_url, &process_definition_id).await;
    let process_instance_id = process_instance["id"].as_str().unwrap().to_string();
    let task = active_task(&client, &base_url).await;
    let task_id = task["id"].as_str().unwrap().to_string();

    let complete = client
        .post(format!("{base_url}/runtime/tasks/{task_id}"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "action": "complete",
            "formDefinitionId": form_definition_id,
            "outcome": "approved",
            "variables": [{
                "name": "approved",
                "type": "boolean",
                "value": true
            }, {
                "name": "note",
                "type": "string",
                "value": "looks good"
            }]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(complete.status(), reqwest::StatusCode::OK);
    assert!(complete.text().await.unwrap().is_empty());

    let remaining_tasks = tasks_for_process(&client, &base_url, &process_instance_id).await;
    assert_eq!(remaining_tasks["total"], 0);

    // Form instance persisted with outcome (Java saveFormInstance(..., outcome))
    let instances = client
        .get(format!("{base_url}/form/form-instances?taskId={task_id}"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(instances.status(), reqwest::StatusCode::OK);
    let instances_body: Value = instances.json().await.unwrap();
    assert_eq!(instances_body["total"], 1);
    assert_eq!(
        instances_body["data"][0]["formDefinitionId"]
            .as_str()
            .unwrap(),
        form_definition_id
    );

    // Outcome variable written to process
    let form_service = flowable_form_service::FlowableFormService::new(Arc::clone(&engine));
    let stored = form_service
        .create_form_instance_query()
        .task_id(task_id.clone())
        .list()
        .unwrap();
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].outcome.as_deref(), Some("approved"));
    assert_eq!(stored[0].values.get("approved"), Some(&json!(true)));

    let vars = engine
        .get_variable_service()
        .get_variables(process_instance_id)
        .unwrap();
    assert_eq!(vars.get("approved"), Some(&json!(true)));
    assert_eq!(vars.get("reviewOutcome"), Some(&json!("approved")));
}

#[tokio::test]
async fn task_action_complete_with_missing_form_definition_is_not_found() {
    let (engine, base_url, client) = spawn_server().await;
    deploy_process(&client, &base_url).await;
    let process_definition_id = latest_definition_id(&engine, "taskActionProcess").await;
    let process_instance = start_process(&client, &base_url, &process_definition_id).await;
    let process_instance_id = process_instance["id"].as_str().unwrap();
    let task = active_task(&client, &base_url).await;
    let task_id = task["id"].as_str().unwrap();

    let complete = client
        .post(format!("{base_url}/runtime/tasks/{task_id}"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "action": "complete",
            "formDefinitionId": "form-definition-does-not-exist",
            "outcome": "approved",
            "variables": [{
                "name": "approved",
                "type": "boolean",
                "value": true
            }]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(complete.status(), reqwest::StatusCode::NOT_FOUND);

    // No partial side effects: task still open
    let remaining_tasks = tasks_for_process(&client, &base_url, process_instance_id).await;
    assert_eq!(remaining_tasks["total"], 1);
    assert_eq!(remaining_tasks["data"][0]["id"], task_id);
}

#[tokio::test]
async fn task_action_complete_with_form_unsupported_field_is_bad_request_and_rolls_back() {
    let (engine, base_url, client) = spawn_server().await;
    deploy_process(&client, &base_url).await;

    let form_deploy = client
        .post(format!("{base_url}/form-repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "Unsupported form",
            "resources": [{
                "resourceName": "custom.form",
                "resource": json!({
                    "key": "customWidgetForm",
                    "name": "Custom widget form",
                    "resourceName": "custom.form",
                    "fields": [
                        // P65-form: `upload` is a registered handler; unregistered
                        // custom types must fail BadRequest (no silent text fallback).
                        { "id": "attachment", "name": "Attachment", "type": "custom_widget", "required": true }
                    ]
                }).to_string()
            }]
        }))
        .send()
        .await
        .unwrap();
    assert!(
        form_deploy.status().is_success(),
        "form deploy status={}",
        form_deploy.status()
    );

    let form_defs = client
        .get(format!(
            "{base_url}/form-repository/form-definitions?key=customWidgetForm"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    let form_defs_body: Value = form_defs.json().await.unwrap();
    let form_definition_id = form_defs_body["data"][0]["id"].as_str().unwrap();

    let process_definition_id = latest_definition_id(&engine, "taskActionProcess").await;
    let process_instance = start_process(&client, &base_url, &process_definition_id).await;
    let process_instance_id = process_instance["id"].as_str().unwrap();
    let task = active_task(&client, &base_url).await;
    let task_id = task["id"].as_str().unwrap();

    let complete = client
        .post(format!("{base_url}/runtime/tasks/{task_id}"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "action": "complete",
            "formDefinitionId": form_definition_id,
            "variables": [{
                "name": "attachment",
                "type": "string",
                "value": "payload"
            }]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(complete.status(), reqwest::StatusCode::BAD_REQUEST);
    let body: Value = complete.json().await.unwrap();
    assert_eq!(body["code"], "BAD_REQUEST");
    assert!(
        body["details"]
            .as_str()
            .unwrap_or("")
            .contains("Unsupported")
    );

    let remaining_tasks = tasks_for_process(&client, &base_url, process_instance_id).await;
    assert_eq!(remaining_tasks["total"], 1);

    let form_service = flowable_form_service::FlowableFormService::new(Arc::clone(&engine));
    assert!(
        form_service
            .create_form_instance_query()
            .task_id(task_id)
            .list()
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn task_action_complete_variable_scope_local_is_not_global() {
    let (engine, base_url, client) = spawn_server().await;
    deploy_complete_variables_process(&client, &base_url).await;
    let process_definition_id = latest_definition_id(&engine, "taskCompleteVariablesProcess").await;
    let process_instance = start_process(&client, &base_url, &process_definition_id).await;
    let process_instance_id = process_instance["id"].as_str().unwrap();
    let tasks = tasks_for_process(&client, &base_url, process_instance_id).await;
    let task_id = tasks["data"][0]["id"].as_str().unwrap();

    let global_seed = client
        .post(format!(
            "{base_url}/runtime/tasks/{task_id}/variables?scope=global"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!([{
            "name": "approved",
            "type": "boolean",
            "value": false
        }]))
        .send()
        .await
        .unwrap();
    assert_eq!(global_seed.status(), reqwest::StatusCode::CREATED);

    let complete = client
        .post(format!("{base_url}/runtime/tasks/{task_id}"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "action": "complete",
            "variables": [{
                "name": "approved",
                "type": "boolean",
                "value": true,
                "scope": "local"
            }]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(complete.status(), reqwest::StatusCode::OK);
    assert!(complete.text().await.unwrap().is_empty());

    let remaining_tasks = tasks_for_process(&client, &base_url, process_instance_id).await;
    assert_eq!(remaining_tasks["total"], 1);
    assert_eq!(
        remaining_tasks["data"][0]["taskDefinitionKey"],
        "rejectedTask"
    );

    let historic_global =
        historic_variables_for_process(&client, &base_url, process_instance_id, "approved").await;
    assert!(
        historic_global["data"]
            .as_array()
            .unwrap()
            .iter()
            .any(|variable| variable["value"] == false)
    );
}

#[tokio::test]
async fn task_action_complete_transient_variables_are_visible_without_history() {
    let (engine, base_url, client) = spawn_server().await;
    deploy_complete_variables_process(&client, &base_url).await;
    let process_definition_id = latest_definition_id(&engine, "taskCompleteVariablesProcess").await;
    let process_instance = start_process(&client, &base_url, &process_definition_id).await;
    let process_instance_id = process_instance["id"].as_str().unwrap();
    let tasks = tasks_for_process(&client, &base_url, process_instance_id).await;
    let task_id = tasks["data"][0]["id"].as_str().unwrap();

    let complete = client
        .post(format!("{base_url}/runtime/tasks/{task_id}"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "action": "complete",
            "transientVariables": [{
                "name": "approved",
                "type": "boolean",
                "value": true
            }]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(complete.status(), reqwest::StatusCode::OK);
    assert!(complete.text().await.unwrap().is_empty());

    let remaining_tasks = tasks_for_process(&client, &base_url, process_instance_id).await;
    assert_eq!(remaining_tasks["total"], 1);
    assert_eq!(
        remaining_tasks["data"][0]["taskDefinitionKey"],
        "approvedTask"
    );

    let historic_transient =
        historic_variables_for_process(&client, &base_url, process_instance_id, "approved").await;
    assert_eq!(historic_transient["total"], 0);
}

#[tokio::test]
async fn task_action_complete_local_scope_keeps_variables_out_of_following_nodes() {
    let (engine, base_url, client) = spawn_server().await;
    deploy_complete_variables_process(&client, &base_url).await;
    let process_definition_id = latest_definition_id(&engine, "taskCompleteVariablesProcess").await;
    let process_instance = start_process(&client, &base_url, &process_definition_id).await;
    let process_instance_id = process_instance["id"].as_str().unwrap();
    let tasks = tasks_for_process(&client, &base_url, process_instance_id).await;
    let task_id = tasks["data"][0]["id"].as_str().unwrap();

    let global_seed = client
        .post(format!(
            "{base_url}/runtime/tasks/{task_id}/variables?scope=global"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!([{
            "name": "approved",
            "type": "boolean",
            "value": false
        }]))
        .send()
        .await
        .unwrap();
    assert_eq!(global_seed.status(), reqwest::StatusCode::CREATED);

    let complete = client
        .post(format!("{base_url}/runtime/tasks/{task_id}"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "action": "complete",
            "localScope": true,
            "variables": [{
                "name": "approved",
                "type": "boolean",
                "value": true
            }]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(complete.status(), reqwest::StatusCode::OK);
    assert!(complete.text().await.unwrap().is_empty());

    let remaining_tasks = tasks_for_process(&client, &base_url, process_instance_id).await;
    assert_eq!(remaining_tasks["total"], 1);
    assert_eq!(
        remaining_tasks["data"][0]["taskDefinitionKey"],
        "rejectedTask"
    );

    let historic_local =
        historic_variables_for_process(&client, &base_url, process_instance_id, "approved").await;
    assert_eq!(historic_local["total"], 2);
    assert!(
        historic_local["data"]
            .as_array()
            .unwrap()
            .iter()
            .any(|variable| variable["taskId"] == task_id && variable["value"] == true)
    );
}

#[tokio::test]
async fn task_complete_rejects_empty_deprecated_body_shape() {
    let (engine, base_url, client) = spawn_server().await;
    deploy_process(&client, &base_url).await;
    let process_definition_id = latest_definition_id(&engine, "taskActionProcess").await;
    start_process(&client, &base_url, &process_definition_id).await;
    let task = active_task(&client, &base_url).await;
    let task_id = task["id"].as_str().unwrap();

    let empty_body = client
        .post(format!("{base_url}/runtime/tasks/{task_id}/complete"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(empty_body.status(), reqwest::StatusCode::BAD_REQUEST);
    let empty_body = empty_body.json::<Value>().await.unwrap();
    assert_eq!(empty_body["code"], "BAD_REQUEST");
    assert!(
        empty_body["details"]
            .as_str()
            .unwrap()
            .contains("canonical"),
        "details: {}",
        empty_body["details"]
    );

    let empty_object = client
        .post(format!("{base_url}/runtime/tasks/{task_id}/complete"))
        .basic_auth("admin", Some("test"))
        .json(&json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(empty_object.status(), reqwest::StatusCode::BAD_REQUEST);
    let empty_object = empty_object.json::<Value>().await.unwrap();
    assert_eq!(empty_object["code"], "BAD_REQUEST");
    assert!(
        empty_object["details"]
            .as_str()
            .unwrap()
            .contains("canonical"),
        "details: {}",
        empty_object["details"]
    );
}

#[tokio::test]
async fn task_complete_rejects_array_variable_body_shape() {
    let (engine, base_url, client) = spawn_server().await;
    deploy_complete_variables_process(&client, &base_url).await;
    let process_definition_id = latest_definition_id(&engine, "taskCompleteVariablesProcess").await;
    let process_instance = start_process(&client, &base_url, &process_definition_id).await;
    let process_instance_id = process_instance["id"].as_str().unwrap();
    let tasks = tasks_for_process(&client, &base_url, process_instance_id).await;
    let task_id = tasks["data"][0]["id"].as_str().unwrap();

    let array_body = client
        .post(format!("{base_url}/runtime/tasks/{task_id}/complete"))
        .basic_auth("admin", Some("test"))
        .json(&json!([{
            "name": "approved",
            "type": "boolean",
            "value": true
        }]))
        .send()
        .await
        .unwrap();
    assert_eq!(array_body.status(), reqwest::StatusCode::BAD_REQUEST);
    let array_body = array_body.json::<Value>().await.unwrap();
    assert_eq!(array_body["code"], "BAD_REQUEST");
    assert!(
        array_body["details"]
            .as_str()
            .unwrap()
            .contains("canonical"),
        "details: {}",
        array_body["details"]
    );

    let remaining_tasks = tasks_for_process(&client, &base_url, process_instance_id).await;
    assert_eq!(remaining_tasks["total"], 1);
    assert_eq!(remaining_tasks["data"][0]["id"], task_id);
}

#[tokio::test]
async fn task_complete_rejects_single_variable_body_shape() {
    let (engine, base_url, client) = spawn_server().await;
    deploy_complete_variables_process(&client, &base_url).await;
    let process_definition_id = latest_definition_id(&engine, "taskCompleteVariablesProcess").await;
    let process_instance = start_process(&client, &base_url, &process_definition_id).await;
    let process_instance_id = process_instance["id"].as_str().unwrap();
    let tasks = tasks_for_process(&client, &base_url, process_instance_id).await;
    let task_id = tasks["data"][0]["id"].as_str().unwrap();

    let single_body = client
        .post(format!("{base_url}/runtime/tasks/{task_id}/complete"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "approved",
            "type": "boolean",
            "value": true
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(single_body.status(), reqwest::StatusCode::BAD_REQUEST);
    let single_body = single_body.json::<Value>().await.unwrap();
    assert_eq!(single_body["code"], "BAD_REQUEST");
    assert!(
        single_body["details"]
            .as_str()
            .unwrap()
            .contains("canonical"),
        "details: {}",
        single_body["details"]
    );

    let remaining_tasks = tasks_for_process(&client, &base_url, process_instance_id).await;
    assert_eq!(remaining_tasks["total"], 1);
    assert_eq!(remaining_tasks["data"][0]["id"], task_id);
}

#[tokio::test]
async fn task_complete_rejects_deprecated_transient_variable_shape() {
    let (engine, base_url, client) = spawn_server().await;
    deploy_complete_variables_process(&client, &base_url).await;
    let process_definition_id = latest_definition_id(&engine, "taskCompleteVariablesProcess").await;
    let process_instance = start_process(&client, &base_url, &process_definition_id).await;
    let process_instance_id = process_instance["id"].as_str().unwrap();
    let task = active_task(&client, &base_url).await;
    let task_id = task["id"].as_str().unwrap();

    let transient_body = client
        .post(format!("{base_url}/runtime/tasks/{task_id}/complete"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "transientVariables": [{
                "name": "approved",
                "type": "boolean",
                "value": true
            }]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(transient_body.status(), reqwest::StatusCode::BAD_REQUEST);
    let transient_body = transient_body.json::<Value>().await.unwrap();
    assert_eq!(transient_body["code"], "BAD_REQUEST");
    assert!(
        transient_body["details"]
            .as_str()
            .unwrap()
            .contains("canonical"),
        "details: {}",
        transient_body["details"]
    );

    let remaining_tasks = tasks_for_process(&client, &base_url, process_instance_id).await;
    assert_eq!(remaining_tasks["total"], 1);
    assert_eq!(remaining_tasks["data"][0]["id"], task_id);

    let historic_transient =
        historic_variables_for_process(&client, &base_url, process_instance_id, "approved").await;
    assert_eq!(historic_transient["total"], 0);
}

#[tokio::test]
async fn task_complete_rejects_deprecated_local_scope_shape() {
    let (engine, base_url, client) = spawn_server().await;
    deploy_complete_variables_process(&client, &base_url).await;
    let process_definition_id = latest_definition_id(&engine, "taskCompleteVariablesProcess").await;
    let process_instance = start_process(&client, &base_url, &process_definition_id).await;
    let process_instance_id = process_instance["id"].as_str().unwrap();
    let tasks = tasks_for_process(&client, &base_url, process_instance_id).await;
    let task_id = tasks["data"][0]["id"].as_str().unwrap();

    let global_seed = client
        .post(format!(
            "{base_url}/runtime/tasks/{task_id}/variables?scope=global"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!([{
            "name": "approved",
            "type": "boolean",
            "value": false
        }]))
        .send()
        .await
        .unwrap();
    assert_eq!(global_seed.status(), reqwest::StatusCode::CREATED);

    let local_scope_body = client
        .post(format!("{base_url}/runtime/tasks/{task_id}/complete"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "localScope": true,
            "variables": [{
                "name": "approved",
                "type": "boolean",
                "value": true
            }]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(local_scope_body.status(), reqwest::StatusCode::BAD_REQUEST);
    let local_scope_body = local_scope_body.json::<Value>().await.unwrap();
    assert_eq!(local_scope_body["code"], "BAD_REQUEST");
    assert!(
        local_scope_body["details"]
            .as_str()
            .unwrap()
            .contains("canonical"),
        "details: {}",
        local_scope_body["details"]
    );

    let remaining_tasks = tasks_for_process(&client, &base_url, process_instance_id).await;
    assert_eq!(remaining_tasks["total"], 1);
    assert_eq!(remaining_tasks["data"][0]["id"], task_id);
}

#[tokio::test]
async fn task_variable_endpoints_support_local_scope_separate_from_global() {
    let (engine, base_url, client) = spawn_server().await;
    deploy_complete_variables_process(&client, &base_url).await;
    let process_definition_id = latest_definition_id(&engine, "taskCompleteVariablesProcess").await;
    let process_instance = start_process(&client, &base_url, &process_definition_id).await;
    let process_instance_id = process_instance["id"].as_str().unwrap();
    let task = active_task(&client, &base_url).await;
    let task_id = task["id"].as_str().unwrap();

    let local_create = client
        .post(format!(
            "{base_url}/runtime/tasks/{task_id}/variables?scope=LoCaL"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!([{
            "name": "reviewNote",
            "type": "string",
            "value": "task-only"
        }]))
        .send()
        .await
        .unwrap();
    assert_eq!(local_create.status(), reqwest::StatusCode::CREATED);
    let local_create_body = local_create.json::<Value>().await.unwrap();
    assert_eq!(local_create_body[0]["scope"], "local");

    let local_list = client
        .get(format!(
            "{base_url}/runtime/tasks/{task_id}/variables?scope=LoCaL"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(local_list.status(), reqwest::StatusCode::OK);
    let local_list_body = local_list.json::<Value>().await.unwrap();
    assert_eq!(local_list_body.as_array().unwrap().len(), 1);
    assert_eq!(local_list_body[0]["name"], "reviewNote");
    assert_eq!(local_list_body[0]["value"], "task-only");

    let global_before = client
        .get(format!(
            "{base_url}/runtime/tasks/{task_id}/variables?scope=global"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(global_before.status(), reqwest::StatusCode::OK);
    assert!(
        global_before
            .json::<Value>()
            .await
            .unwrap()
            .as_array()
            .unwrap()
            .iter()
            .all(|variable| variable["name"] != "reviewNote")
    );

    let global_create = client
        .post(format!(
            "{base_url}/runtime/tasks/{task_id}/variables?scope=GlObAl"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!([{
            "name": "approved",
            "type": "boolean",
            "value": true
        }]))
        .send()
        .await
        .unwrap();
    assert_eq!(global_create.status(), reqwest::StatusCode::CREATED);
    let global_create_body = global_create.json::<Value>().await.unwrap();
    assert_eq!(global_create_body[0]["scope"], "global");

    let unknown_scope = client
        .get(format!(
            "{base_url}/runtime/tasks/{task_id}/variables?scope=unknown"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(unknown_scope.status(), reqwest::StatusCode::BAD_REQUEST);
    let unknown_scope_body = unknown_scope.json::<Value>().await.unwrap();
    assert_eq!(unknown_scope_body["code"], "BAD_REQUEST");
    assert!(
        unknown_scope_body["details"]
            .as_str()
            .unwrap()
            .contains("Unsupported task variable scope 'unknown'")
    );

    let complete = client
        .post(format!("{base_url}/runtime/tasks/{task_id}"))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "action": "complete" }))
        .send()
        .await
        .unwrap();
    assert_eq!(complete.status(), reqwest::StatusCode::OK);

    let remaining_tasks = tasks_for_process(&client, &base_url, process_instance_id).await;
    assert_eq!(remaining_tasks["total"], 1);
    assert_eq!(
        remaining_tasks["data"][0]["taskDefinitionKey"],
        "approvedTask"
    );

    let historic_local =
        historic_variables_for_process(&client, &base_url, process_instance_id, "reviewNote").await;
    assert_eq!(historic_local["total"], 1);
    assert_eq!(historic_local["data"][0]["taskId"], task_id);

    let historic_global =
        historic_variables_for_process(&client, &base_url, process_instance_id, "approved").await;
    assert_eq!(historic_global["total"], 1);
    assert!(historic_global["data"][0]["taskId"].is_null());
}

#[tokio::test]
async fn task_variable_data_endpoint_roundtrips_local_bytes() {
    let (engine, base_url, client) = spawn_server().await;
    deploy_complete_variables_process(&client, &base_url).await;
    let process_definition_id = latest_definition_id(&engine, "taskCompleteVariablesProcess").await;
    start_process(&client, &base_url, &process_definition_id).await;
    let task = active_task(&client, &base_url).await;
    let task_id = task["id"].as_str().unwrap();

    let create = client
        .post(format!(
            "{base_url}/runtime/tasks/{task_id}/variables?scope=local"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!([{
            "name": "payload",
            "type": "bytes",
            "value": null
        }]))
        .send()
        .await
        .unwrap();
    assert_eq!(create.status(), reqwest::StatusCode::CREATED);
    let create_body = create.json::<Value>().await.unwrap();
    assert_eq!(create_body[0]["name"], "payload");
    assert_eq!(create_body[0]["type"], "bytes");
    assert!(create_body[0]["value"].is_null());
    assert_eq!(create_body[0]["scope"], "local");

    let raw_bytes = vec![0x00, 0x01, 0x02, 0xfe, 0xff, b'r', b'u', b's', b't'];
    let update = client
        .put(format!(
            "{base_url}/runtime/tasks/{task_id}/variables/payload/data?scope=local"
        ))
        .basic_auth("admin", Some("test"))
        .header(reqwest::header::CONTENT_TYPE, "application/octet-stream")
        .body(raw_bytes.clone())
        .send()
        .await
        .unwrap();
    assert_eq!(update.status(), reqwest::StatusCode::NO_CONTENT);

    let data = client
        .get(format!(
            "{base_url}/runtime/tasks/{task_id}/variables/payload/data?scope=local"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(data.status(), reqwest::StatusCode::OK);
    assert_eq!(
        data.headers()
            .get(reqwest::header::CONTENT_TYPE)
            .unwrap()
            .to_str()
            .unwrap(),
        "application/octet-stream"
    );
    assert_eq!(data.bytes().await.unwrap().as_ref(), raw_bytes.as_slice());
}

#[tokio::test]
async fn task_variable_data_endpoint_roundtrips_global_binary_and_updates_execution_variable() {
    let (engine, base_url, client) = spawn_server().await;
    deploy_complete_variables_process(&client, &base_url).await;
    let process_definition_id = latest_definition_id(&engine, "taskCompleteVariablesProcess").await;
    start_process(&client, &base_url, &process_definition_id).await;
    let task = active_task(&client, &base_url).await;
    let task_id = task["id"].as_str().unwrap();
    let execution_id = task["executionId"].as_str().unwrap();

    let create = client
        .post(format!(
            "{base_url}/runtime/tasks/{task_id}/variables?scope=global"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!([{
            "name": "document",
            "type": "binary",
            "value": null
        }]))
        .send()
        .await
        .unwrap();
    assert_eq!(create.status(), reqwest::StatusCode::CREATED);
    let create_body = create.json::<Value>().await.unwrap();
    assert_eq!(create_body[0]["name"], "document");
    assert_eq!(create_body[0]["type"], "binary");
    assert!(create_body[0]["value"].is_null());
    assert_eq!(create_body[0]["scope"], "global");

    let raw_bytes = b"binary payload from task data endpoint".to_vec();
    let update = client
        .put(format!(
            "{base_url}/runtime/tasks/{task_id}/variables/document/data"
        ))
        .basic_auth("admin", Some("test"))
        .header(reqwest::header::CONTENT_TYPE, "application/octet-stream")
        .body(raw_bytes.clone())
        .send()
        .await
        .unwrap();
    assert_eq!(update.status(), reqwest::StatusCode::NO_CONTENT);

    let data = client
        .get(format!(
            "{base_url}/runtime/tasks/{task_id}/variables/document/data"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(data.status(), reqwest::StatusCode::OK);
    assert_eq!(
        data.headers()
            .get(reqwest::header::CONTENT_TYPE)
            .unwrap()
            .to_str()
            .unwrap(),
        "application/octet-stream"
    );
    assert_eq!(data.bytes().await.unwrap().as_ref(), raw_bytes.as_slice());

    let execution_data = client
        .get(format!(
            "{base_url}/runtime/executions/{execution_id}/variables/document/data"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(execution_data.status(), reqwest::StatusCode::OK);
    assert_eq!(
        execution_data
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .unwrap()
            .to_str()
            .unwrap(),
        "application/octet-stream"
    );
    assert_eq!(
        execution_data.bytes().await.unwrap().as_ref(),
        raw_bytes.as_slice()
    );
}

#[tokio::test]
async fn task_variable_data_endpoint_roundtrips_local_serializable() {
    let (engine, base_url, client) = spawn_server().await;
    deploy_complete_variables_process(&client, &base_url).await;
    let process_definition_id = latest_definition_id(&engine, "taskCompleteVariablesProcess").await;
    start_process(&client, &base_url, &process_definition_id).await;
    let task = active_task(&client, &base_url).await;
    let task_id = task["id"].as_str().unwrap();

    let create = client
        .post(format!(
            "{base_url}/runtime/tasks/{task_id}/variables?scope=local"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!([{
            "name": "serializableObject",
            "type": "serializable",
            "value": null
        }]))
        .send()
        .await
        .unwrap();
    assert_eq!(create.status(), reqwest::StatusCode::CREATED);
    let create_body = create.json::<Value>().await.unwrap();
    assert_eq!(create_body[0]["name"], "serializableObject");
    assert_eq!(create_body[0]["type"], "serializable");
    assert!(create_body[0]["value"].is_null());
    assert_eq!(create_body[0]["scope"], "local");

    let object_data = json!({
        "className": "com.example.TaskPayload",
        "fields": {
            "local": true,
            "score": 7
        }
    })
    .to_string()
    .into_bytes();
    let update = client
        .put(format!(
            "{base_url}/runtime/tasks/{task_id}/variables/serializableObject/data?scope=local"
        ))
        .basic_auth("admin", Some("test"))
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(object_data.clone())
        .send()
        .await
        .unwrap();
    assert_eq!(update.status(), reqwest::StatusCode::NO_CONTENT);

    let data = client
        .get(format!(
            "{base_url}/runtime/tasks/{task_id}/variables/serializableObject/data?scope=local"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(data.status(), reqwest::StatusCode::OK);
    assert_eq!(
        data.headers()
            .get(reqwest::header::CONTENT_TYPE)
            .unwrap()
            .to_str()
            .unwrap(),
        "application/x-java-serialized-object"
    );
    let returned_bytes = data.bytes().await.unwrap();
    assert_eq!(returned_bytes.as_ref(), object_data.as_slice());
    let returned_json: Value = serde_json::from_slice(&returned_bytes).unwrap();
    assert_eq!(returned_json["fields"]["score"], 7);
}

#[tokio::test]
async fn task_variable_data_endpoint_roundtrips_global_serializable_and_updates_execution_variable()
{
    let (engine, base_url, client) = spawn_server().await;
    deploy_complete_variables_process(&client, &base_url).await;
    let process_definition_id = latest_definition_id(&engine, "taskCompleteVariablesProcess").await;
    start_process(&client, &base_url, &process_definition_id).await;
    let task = active_task(&client, &base_url).await;
    let task_id = task["id"].as_str().unwrap();
    let execution_id = task["executionId"].as_str().unwrap();

    let create = client
        .post(format!(
            "{base_url}/runtime/tasks/{task_id}/variables?scope=global"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!([{
            "name": "serializableObject",
            "type": "serializable",
            "value": null
        }]))
        .send()
        .await
        .unwrap();
    assert_eq!(create.status(), reqwest::StatusCode::CREATED);
    let create_body = create.json::<Value>().await.unwrap();
    assert_eq!(create_body[0]["name"], "serializableObject");
    assert_eq!(create_body[0]["type"], "serializable");
    assert!(create_body[0]["value"].is_null());
    assert_eq!(create_body[0]["scope"], "global");

    let object_data = json!({
        "className": "com.example.GlobalTaskPayload",
        "fields": {
            "global": true
        }
    })
    .to_string()
    .into_bytes();
    let update = client
        .put(format!(
            "{base_url}/runtime/tasks/{task_id}/variables/serializableObject/data"
        ))
        .basic_auth("admin", Some("test"))
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(object_data.clone())
        .send()
        .await
        .unwrap();
    assert_eq!(update.status(), reqwest::StatusCode::NO_CONTENT);

    let task_data = client
        .get(format!(
            "{base_url}/runtime/tasks/{task_id}/variables/serializableObject/data"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(task_data.status(), reqwest::StatusCode::OK);
    assert_eq!(
        task_data.bytes().await.unwrap().as_ref(),
        object_data.as_slice()
    );

    let execution_data = client
        .get(format!(
            "{base_url}/runtime/executions/{execution_id}/variables/serializableObject/data"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(execution_data.status(), reqwest::StatusCode::OK);
    assert_eq!(
        execution_data.bytes().await.unwrap().as_ref(),
        object_data.as_slice()
    );
}

#[tokio::test]
async fn task_variable_create_rejects_non_null_serializable_metadata() {
    let (engine, base_url, client) = spawn_server().await;
    deploy_complete_variables_process(&client, &base_url).await;
    let process_definition_id = latest_definition_id(&engine, "taskCompleteVariablesProcess").await;
    start_process(&client, &base_url, &process_definition_id).await;
    let task = active_task(&client, &base_url).await;
    let task_id = task["id"].as_str().unwrap();

    let create = client
        .post(format!("{base_url}/runtime/tasks/{task_id}/variables"))
        .basic_auth("admin", Some("test"))
        .json(&json!([{
            "name": "unsupportedPayload",
            "type": "serializable",
            "value": {
                "inline": "not accepted"
            }
        }]))
        .send()
        .await
        .unwrap();
    assert_eq!(create.status(), reqwest::StatusCode::BAD_REQUEST);
    let create_body = create.json::<Value>().await.unwrap();
    assert_eq!(create_body["code"], "BAD_REQUEST");
    assert!(
        create_body["details"]
            .as_str()
            .unwrap()
            .contains("serializable")
    );
    assert!(
        create_body["details"]
            .as_str()
            .unwrap()
            .contains("metadata must use null value")
    );
}

#[tokio::test]
async fn task_complete_rejects_unknown_fields_and_malformed_variables() {
    let (engine, base_url, client) = spawn_server().await;
    deploy_process(&client, &base_url).await;
    let process_definition_id = latest_definition_id(&engine, "taskActionProcess").await;
    start_process(&client, &base_url, &process_definition_id).await;
    let task = active_task(&client, &base_url).await;
    let task_id = task["id"].as_str().unwrap();

    let unknown = client
        .post(format!("{base_url}/runtime/tasks/{task_id}/complete"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "variables": [],
            "unexpectedField": true
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(unknown.status(), reqwest::StatusCode::BAD_REQUEST);
    let unknown_body = unknown.json::<Value>().await.unwrap();
    assert_eq!(unknown_body["code"], "BAD_REQUEST");
    assert!(
        unknown_body["details"]
            .as_str()
            .unwrap()
            .contains("canonical"),
        "details: {}",
        unknown_body["details"]
    );

    let malformed = client
        .post(format!("{base_url}/runtime/tasks/{task_id}/complete"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "variables": {
                "approved": true
            }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(malformed.status(), reqwest::StatusCode::BAD_REQUEST);
    let malformed_body = malformed.json::<Value>().await.unwrap();
    assert_eq!(malformed_body["code"], "BAD_REQUEST");
    assert!(
        malformed_body["details"]
            .as_str()
            .unwrap()
            .contains("canonical"),
        "details: {}",
        malformed_body["details"]
    );
}

#[tokio::test]
async fn task_complete_variables_fail_before_task_is_completed() {
    let (engine, base_url, client) = spawn_server().await;
    deploy_complete_variables_process(&client, &base_url).await;
    let process_definition_id = latest_definition_id(&engine, "taskCompleteVariablesProcess").await;
    let process_instance = start_process(&client, &base_url, &process_definition_id).await;
    let process_instance_id = process_instance["id"].as_str().unwrap();
    let task = active_task(&client, &base_url).await;
    let task_id = task["id"].as_str().unwrap();

    let complete = client
        .post(format!("{base_url}/runtime/tasks/{task_id}"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "action": "complete",
            "variables": [{
                "name": "payload",
                "type": "binary",
                "value": "not-json-supported"
            }]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(complete.status(), reqwest::StatusCode::BAD_REQUEST);
    let complete_body = complete.json::<Value>().await.unwrap();
    assert_eq!(complete_body["code"], "BAD_REQUEST");
    assert!(
        complete_body["details"]
            .as_str()
            .unwrap()
            .contains("not supported")
    );

    let remaining_tasks = tasks_for_process(&client, &base_url, process_instance_id).await;
    assert_eq!(remaining_tasks["total"], 1);
    assert_eq!(remaining_tasks["data"][0]["id"], task_id);
}

#[tokio::test]
async fn task_action_resolve_on_non_delegated_task_succeeds_per_java() {
    // Java parity: ResolveTaskCmd.java:45-56 — no delegation-state precondition;
    // resolving a task that was never delegated silently marks it RESOLVED and
    // sets assignee=owner (owner was never set here, so assignee resets to null).
    let (engine, base_url, client) = spawn_server().await;
    deploy_process(&client, &base_url).await;
    let process_definition_id = latest_definition_id(&engine, "taskActionProcess").await;
    start_process(&client, &base_url, &process_definition_id).await;
    let task = active_task(&client, &base_url).await;
    let task_id = task["id"].as_str().unwrap();

    let resolve = client
        .post(format!("{base_url}/runtime/tasks/{task_id}"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "action": "resolve"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resolve.status(), reqwest::StatusCode::OK);

    let resolved_task = get_task(&client, &base_url, task_id).await;
    assert_eq!(resolved_task["delegationState"], "resolved");
    assert!(resolved_task["assignee"].is_null());
}

#[tokio::test]
async fn task_action_delegate_rejects_missing_assignee_structured() {
    let (engine, base_url, client) = spawn_server().await;
    deploy_process(&client, &base_url).await;
    let process_definition_id = latest_definition_id(&engine, "taskActionProcess").await;
    start_process(&client, &base_url, &process_definition_id).await;
    let task = active_task(&client, &base_url).await;
    let task_id = task["id"].as_str().unwrap();

    let missing = client
        .post(format!("{base_url}/runtime/tasks/{task_id}"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "action": "delegate"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(missing.status(), reqwest::StatusCode::BAD_REQUEST);
    let missing_body = missing.json::<Value>().await.unwrap();
    assert_eq!(missing_body["code"], "BAD_REQUEST");
    assert!(
        missing_body["details"]
            .as_str()
            .unwrap()
            .contains("Assignee is required")
    );

    let blank = client
        .post(format!("{base_url}/runtime/tasks/{task_id}"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "action": "delegate",
            "assignee": "   "
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(blank.status(), reqwest::StatusCode::BAD_REQUEST);
    let blank_body = blank.json::<Value>().await.unwrap();
    assert_eq!(blank_body["code"], "BAD_REQUEST");
    assert!(
        blank_body["details"]
            .as_str()
            .unwrap()
            .contains("Assignee is required")
    );
}

#[tokio::test]
async fn task_action_errors_are_structured() {
    let (_engine, base_url, client) = spawn_server().await;

    let missing = client
        .post(format!("{base_url}/runtime/tasks/missing-task"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "action": "claim",
            "assignee": "kermit"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(missing.status(), reqwest::StatusCode::NOT_FOUND);
    let missing_body = missing.json::<Value>().await.unwrap();
    assert_eq!(missing_body["code"], "NOT_FOUND");
    assert!(
        missing_body["details"]
            .as_str()
            .unwrap()
            .contains("missing-task")
    );

    let missing_complete = client
        .post(format!("{base_url}/runtime/tasks/missing-task"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "action": "complete"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(missing_complete.status(), reqwest::StatusCode::NOT_FOUND);
    let missing_complete_body = missing_complete.json::<Value>().await.unwrap();
    assert_eq!(missing_complete_body["code"], "NOT_FOUND");
    assert!(
        missing_complete_body["details"]
            .as_str()
            .unwrap()
            .contains("missing-task")
    );

    let unsupported = client
        .post(format!("{base_url}/runtime/tasks/missing-task"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "action": "pause",
            "assignee": "kermit"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(unsupported.status(), reqwest::StatusCode::BAD_REQUEST);
    let unsupported_body = unsupported.json::<Value>().await.unwrap();
    assert_eq!(unsupported_body["code"], "BAD_REQUEST");
    assert!(
        unsupported_body["details"]
            .as_str()
            .unwrap()
            .contains("Unsupported task action")
    );
}

#[tokio::test]
async fn task_put_updates_mutable_metadata_and_historic_task() {
    let (engine, base_url, client) = spawn_server().await;
    deploy_process(&client, &base_url).await;
    let process_definition_id = latest_definition_id(&engine, "taskActionProcess").await;
    start_process(&client, &base_url, &process_definition_id).await;
    let task = active_task(&client, &base_url).await;
    let task_id = task["id"].as_str().unwrap();

    let response = client
        .put(format!("{base_url}/runtime/tasks/{task_id}"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "Updated review",
            "description": "REST metadata update",
            "assignee": "fozzie",
            "owner": "kermit",
            "delegationState": "pending",
            "parentTaskId": "parent-1",
            "priority": 75,
            "dueDate": "2030-01-02T03:04:05Z",
            "category": "approval",
            "formKey": "approvalForm",
            "tenantId": "tenant-a"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let body = response.json::<Value>().await.unwrap();
    assert_eq!(body["id"], task_id);
    assert_eq!(body["name"], "Updated review");
    assert_eq!(body["description"], "REST metadata update");
    assert_eq!(body["assignee"], "fozzie");
    assert_eq!(body["owner"], "kermit");
    assert_eq!(body["delegationState"], "pending");
    assert_eq!(body["parentTaskId"], "parent-1");
    assert_eq!(body["priority"], 75);
    assert_eq!(body["dueDate"], "2030-01-02T03:04:05+00:00");
    assert_eq!(body["category"], "approval");
    assert_eq!(body["formKey"], "approvalForm");
    assert_eq!(body["tenantId"], "tenant-a");

    let persisted = get_task(&client, &base_url, task_id).await;
    assert_eq!(persisted["name"], "Updated review");
    assert_eq!(persisted["category"], "approval");
    assert_eq!(persisted["formKey"], "approvalForm");

    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let historic_task = store
        .get_historic_task_instance(task_id, &mut session)
        .unwrap();
    let _ = session.rollback();
    assert_eq!(historic_task.name.as_deref(), Some("Updated review"));
    assert_eq!(
        historic_task.description.as_deref(),
        Some("REST metadata update")
    );
    assert_eq!(historic_task.assignee.as_deref(), Some("fozzie"));
    assert_eq!(historic_task.owner.as_deref(), Some("kermit"));
    assert_eq!(historic_task.priority, Some(75));
    assert_eq!(
        historic_task.due_date.unwrap().to_rfc3339(),
        "2030-01-02T03:04:05+00:00"
    );
}

#[tokio::test]
async fn task_put_clears_nullable_metadata_and_validates_bad_input() {
    let (engine, base_url, client) = spawn_server().await;
    deploy_process(&client, &base_url).await;
    let process_definition_id = latest_definition_id(&engine, "taskActionProcess").await;
    start_process(&client, &base_url, &process_definition_id).await;
    let task = active_task(&client, &base_url).await;
    let task_id = task["id"].as_str().unwrap();

    let set_response = client
        .put(format!("{base_url}/runtime/tasks/{task_id}"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "description": "temporary",
            "assignee": "fozzie",
            "owner": "kermit",
            "priority": 10,
            "dueDate": 1893456000000_i64,
            "category": "temporary",
            "formKey": "temporaryForm",
            "tenantId": "tenant-a"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(set_response.status(), reqwest::StatusCode::OK);

    let clear_response = client
        .put(format!("{base_url}/runtime/tasks/{task_id}"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "description": null,
            "assignee": null,
            "owner": null,
            "delegationState": null,
            "parentTaskId": null,
            "priority": null,
            "dueDate": null,
            "category": null,
            "formKey": null,
            "tenantId": null
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(clear_response.status(), reqwest::StatusCode::OK);
    let body = clear_response.json::<Value>().await.unwrap();
    assert!(body["description"].is_null());
    assert!(body["assignee"].is_null());
    assert!(body["owner"].is_null());
    assert!(body["priority"].is_null());
    assert!(body["dueDate"].is_null());
    assert!(body["category"].is_null());
    assert!(body["formKey"].is_null());
    assert!(body["tenantId"].is_null());

    let null_name = client
        .put(format!("{base_url}/runtime/tasks/{task_id}"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": null
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(null_name.status(), reqwest::StatusCode::BAD_REQUEST);
    let null_name_body = null_name.json::<Value>().await.unwrap();
    assert_eq!(null_name_body["code"], "BAD_REQUEST");
    assert!(
        null_name_body["details"]
            .as_str()
            .unwrap()
            .contains("Task name cannot be null")
    );

    let invalid_due_date = client
        .put(format!("{base_url}/runtime/tasks/{task_id}"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "dueDate": "not-a-date"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(invalid_due_date.status(), reqwest::StatusCode::BAD_REQUEST);
    let invalid_due_date_body = invalid_due_date.json::<Value>().await.unwrap();
    assert_eq!(invalid_due_date_body["code"], "BAD_REQUEST");
    assert!(
        invalid_due_date_body["details"]
            .as_str()
            .unwrap()
            .contains("Invalid dueDate")
    );
}

fn insert_standalone_task(engine: &ProcessEngine, task_id: &str) {
    insert_standalone_task_with_state(engine, task_id, false);
}

fn insert_standalone_task_with_state(engine: &ProcessEngine, task_id: &str, suspended: bool) {
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let mut task = Task::new(
        task_id.to_string(),
        String::new(),
        String::new(),
        "standalone".to_string(),
        "Standalone task".to_string(),
    );
    if suspended {
        task.set_suspension_state(true);
    }
    store.insert_task(&task, &mut session);
    let historic = HistoricTaskInstance {
        id: task_id.to_string(),
        process_instance_id: String::new(),
        process_definition_id: None,
        execution_id: String::new(),
        task_definition_key: Some("standalone".to_string()),
        name: Some("Standalone task".to_string()),
        description: None,
        assignee: None,
        owner: None,
        claim_time: None,
        tenant_id: None,
        category: None,
        form_key: None,
        parent_task_id: None,
        priority: None,
        due_date: None,
        start_time: Utc::now(),
        end_time: None,
        duration_ms: None,
        delete_reason: None,
    };
    store.insert_historic_task_instance(historic, &mut session);
    session.flush_and_commit().unwrap();
}

#[tokio::test]
async fn task_delete_rejects_workflow_task_with_403() {
    let (engine, base_url, client) = spawn_server().await;
    deploy_process(&client, &base_url).await;
    let process_definition_id = latest_definition_id(&engine, "taskActionProcess").await;
    start_process(&client, &base_url, &process_definition_id).await;
    let task = active_task(&client, &base_url).await;
    let task_id = task["id"].as_str().unwrap();

    let response = client
        .delete(format!("{base_url}/runtime/tasks/{task_id}"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn task_delete_cascade_purges_historic_task_instance() {
    let (engine, base_url, client) = spawn_server().await;
    let task_id = "standalone-cascade-task";
    insert_standalone_task(&engine, task_id);

    let historic_before = client
        .get(format!(
            "{base_url}/history/historic-task-instances/{task_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(historic_before.status(), reqwest::StatusCode::OK);

    let delete = client
        .delete(format!(
            "{base_url}/runtime/tasks/{task_id}?cascadeHistory=true"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(delete.status(), reqwest::StatusCode::NO_CONTENT);

    let runtime_after = client
        .get(format!("{base_url}/runtime/tasks/{task_id}"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(runtime_after.status(), reqwest::StatusCode::NOT_FOUND);

    let historic_after = client
        .get(format!(
            "{base_url}/history/historic-task-instances/{task_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(historic_after.status(), reqwest::StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn task_delete_records_delete_reason_on_historic_task_instance() {
    let (engine, base_url, client) = spawn_server().await;
    let task_id = "standalone-reason-task";
    insert_standalone_task(&engine, task_id);

    let delete = client
        .delete(format!(
            "{base_url}/runtime/tasks/{task_id}?deleteReason=removed+by+admin"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(delete.status(), reqwest::StatusCode::NO_CONTENT);

    let runtime_after = client
        .get(format!("{base_url}/runtime/tasks/{task_id}"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(runtime_after.status(), reqwest::StatusCode::NOT_FOUND);

    let historic = client
        .get(format!(
            "{base_url}/history/historic-task-instances/{task_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(historic.status(), reqwest::StatusCode::OK);
    let body = historic.json::<Value>().await.unwrap();
    assert_eq!(body["deleteReason"], "removed by admin");
}

#[tokio::test]
async fn task_delete_rejects_suspended_task_with_500() {
    let (engine, base_url, client) = spawn_server().await;
    let task_id = "standalone-suspended-task";
    insert_standalone_task_with_state(&engine, task_id, true);

    let response = client
        .delete(format!("{base_url}/runtime/tasks/{task_id}"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        reqwest::StatusCode::INTERNAL_SERVER_ERROR
    );
    let body = response.json::<Value>().await.unwrap();
    assert_eq!(body["code"], "INTERNAL_SERVER_ERROR");
    // 5xx details are generic (no suspended-task message echo).
    assert_eq!(body["details"], "Internal server error");
}
