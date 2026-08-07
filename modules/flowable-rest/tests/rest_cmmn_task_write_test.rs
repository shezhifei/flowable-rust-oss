// P99: CMMN task write endpoints — update / actions / delete / task variables.
//
// Java references:
// - TaskResource.java:76-99 (PUT update), :109-137 (POST actions), :149-174 (DELETE 403)
// - TaskVariableCollectionResource.java:122-212 (POST create), :219-228 (DELETE all local)
// - TaskVariableResource.java:94-130 (PUT single), :138-167 (DELETE single)
// - RestVariable scope defaults to LOCAL (TaskVariableCollectionResource.java:164-166)
//
// P115: task-local variables are supported (no longer empty); GLOBAL-scope
// writes land on the case instance. Standalone task creation is out of scope.

use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::identity::entities::User;
use flowable_rest::run_server;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::net::TcpListener;

const TASK_CMMN: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/CMMN/20151109/MODEL"
             targetNamespace="http://flowable.org/cmmn">
  <case id="taskCase" name="Task Case">
    <casePlanModel id="taskPlan" name="Task Plan" autoComplete="false">
      <planItem id="planItemReview" name="Review" definitionRef="reviewTask" />
      <humanTask id="reviewTask" name="Review" isBlocking="true" />
    </casePlanModel>
  </case>
</definitions>"#;

async fn spawn_server() -> (String, reqwest::Client) {
    let engine = Arc::new(ProcessEngine::new("rest-cmmn-task-write".to_string()));
    engine.get_identity_service().save_user(User {
        id: "admin".to_string(),
        first_name: None,
        last_name: None,
        email: None,
        password: Some("test".to_string()),
        tenant_id: None,
    });

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        run_server(engine, listener).await.unwrap();
    });

    (base_url, reqwest::Client::new())
}

async fn deploy_and_start_case(base_url: &str, client: &reqwest::Client) -> (String, String) {
    let deploy_response = client
        .post(format!("{base_url}/cmmn-repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "CMMN task write deployment",
            "resourceName": "task.cmmn",
            "resource": TASK_CMMN
        }))
        .send()
        .await
        .unwrap();
    assert!(deploy_response.status().is_success());

    let start_response = client
        .post(format!("{base_url}/cmmn-runtime/case-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "caseDefinitionKey": "taskCase" }))
        .send()
        .await
        .unwrap();
    assert!(start_response.status().is_success());
    let case_instance_id = start_response.json::<Value>().await.unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();

    let tasks_response = client
        .get(format!(
            "{base_url}/cmmn-runtime/tasks?caseInstanceId={case_instance_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(tasks_response.status().is_success());
    let tasks_body: Value = tasks_response.json().await.unwrap();
    let task_id = tasks_body["data"][0]["id"].as_str().unwrap().to_string();

    (case_instance_id, task_id)
}

#[tokio::test]
async fn cmmn_task_update_sets_fields_and_clears_with_explicit_null() {
    // Java: TaskResource.java:76-99 — PUT update; explicit null clears.
    let (base_url, client) = spawn_server().await;
    let (_case_id, task_id) = deploy_and_start_case(&base_url, &client).await;

    let update_response = client
        .put(format!("{base_url}/cmmn-runtime/tasks/{task_id}"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "Renamed",
            "assignee": "alice",
            "owner": "owner",
            "priority": 50,
            "dueDate": "2026-12-31",
            "category": "work"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(update_response.status().as_u16(), 200);
    let body: Value = update_response.json().await.unwrap();
    assert_eq!(body["name"], "Renamed");
    assert_eq!(body["assignee"], "alice");
    assert_eq!(body["owner"], "owner");
    assert_eq!(body["priority"], "50");
    assert_eq!(body["dueDate"], "2026-12-31");
    assert_eq!(body["category"], "work");

    // Explicit null clears (TaskResource.java:70 — {"dueDate": null}).
    let clear_response = client
        .put(format!("{base_url}/cmmn-runtime/tasks/{task_id}"))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "assignee": null, "dueDate": null }))
        .send()
        .await
        .unwrap();
    assert_eq!(clear_response.status().as_u16(), 200);
    let cleared: Value = clear_response.json().await.unwrap();
    assert_eq!(cleared["assignee"], Value::Null);
    assert_eq!(cleared["dueDate"], Value::Null);
    assert_eq!(cleared["name"], "Renamed", "untouched fields stay");

    // Missing task → 404.
    let missing = client
        .put(format!("{base_url}/cmmn-runtime/tasks/missing-task"))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "name": "x" }))
        .send()
        .await
        .unwrap();
    assert_eq!(missing.status().as_u16(), 404);
}

#[tokio::test]
async fn cmmn_task_action_complete_writes_variables_and_outcome() {
    // Java: TaskResource.java:197-236 — complete with variables + outcome.
    let (base_url, client) = spawn_server().await;
    let (case_id, task_id) = deploy_and_start_case(&base_url, &client).await;

    let complete_response = client
        .post(format!("{base_url}/cmmn-runtime/tasks/{task_id}"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "action": "complete",
            "outcome": "approved",
            "variables": [
                { "name": "completedFlag", "value": true },
                { "name": "completer", "value": "alice" }
            ]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(complete_response.status().as_u16(), 200, "complete → 200 empty body");

    // GLOBAL completion variables land on the case (CompleteTaskCmd.java:100-101).
    let variables = client
        .get(format!(
            "{base_url}/cmmn-runtime/case-instances/{case_id}/variables"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    let variables: Value = variables.json().await.unwrap();
    let names: Vec<&str> = variables
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|v| v["name"].as_str())
        .collect();
    assert!(names.contains(&"completedFlag"));
    assert!(names.contains(&"completer"));
    let flag = variables
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["name"] == "completedFlag")
        .unwrap();
    assert_eq!(flag["value"], json!(true));
}

#[tokio::test]
async fn cmmn_task_action_claim_success_conflict_and_missing_assignee() {
    // Java: TaskResource.java:249-254 + ClaimTaskCmd.java:51.
    let (base_url, client) = spawn_server().await;
    let (_case_id, task_id) = deploy_and_start_case(&base_url, &client).await;

    let claim = client
        .post(format!("{base_url}/cmmn-runtime/tasks/{task_id}"))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "action": "claim", "assignee": "alice" }))
        .send()
        .await
        .unwrap();
    assert_eq!(claim.status().as_u16(), 200);

    let task = client
        .get(format!("{base_url}/cmmn-runtime/tasks/{task_id}"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    let task: Value = task.json().await.unwrap();
    assert_eq!(task["assignee"], "alice");

    // Claim by another user → 409 (FlowableTaskAlreadyClaimedException).
    let conflict = client
        .post(format!("{base_url}/cmmn-runtime/tasks/{task_id}"))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "action": "claim", "assignee": "bob" }))
        .send()
        .await
        .unwrap();
    assert_eq!(conflict.status().as_u16(), 409);

    // Missing assignee → 400.
    let no_assignee = client
        .post(format!("{base_url}/cmmn-runtime/tasks/{task_id}"))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "action": "claim" }))
        .send()
        .await
        .unwrap();
    assert_eq!(no_assignee.status().as_u16(), 400);

    // Invalid action → 400 (TaskResource.java:135).
    let invalid = client
        .post(format!("{base_url}/cmmn-runtime/tasks/{task_id}"))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "action": "teleport" }))
        .send()
        .await
        .unwrap();
    assert_eq!(invalid.status().as_u16(), 400);
}

#[tokio::test]
async fn cmmn_task_action_delegate_then_resolve() {
    // Java: DelegateTaskCmd.java:37-47 + ResolveTaskCmd.java:55-57.
    let (base_url, client) = spawn_server().await;
    let (_case_id, task_id) = deploy_and_start_case(&base_url, &client).await;

    client
        .post(format!("{base_url}/cmmn-runtime/tasks/{task_id}"))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "action": "claim", "assignee": "alice" }))
        .send()
        .await
        .unwrap();

    let delegate = client
        .post(format!("{base_url}/cmmn-runtime/tasks/{task_id}"))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "action": "delegate", "assignee": "bob" }))
        .send()
        .await
        .unwrap();
    assert_eq!(delegate.status().as_u16(), 200);

    let task: Value = client
        .get(format!("{base_url}/cmmn-runtime/tasks/{task_id}"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(task["assignee"], "bob");
    assert_eq!(task["owner"], "alice");

    let resolve = client
        .post(format!("{base_url}/cmmn-runtime/tasks/{task_id}"))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "action": "resolve" }))
        .send()
        .await
        .unwrap();
    assert_eq!(resolve.status().as_u16(), 200);

    let resolved: Value = client
        .get(format!("{base_url}/cmmn-runtime/tasks/{task_id}"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(resolved["assignee"], "alice", "assignee returns to owner");
    assert_eq!(resolved["owner"], "alice");
}

#[tokio::test]
async fn cmmn_task_delete_is_forbidden() {
    // Java: TaskResource.java:155-157 — CMMN task deletion is always 403.
    let (base_url, client) = spawn_server().await;
    let (_case_id, task_id) = deploy_and_start_case(&base_url, &client).await;

    let delete = client
        .delete(format!("{base_url}/cmmn-runtime/tasks/{task_id}"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(delete.status().as_u16(), 403);

    let missing = client
        .delete(format!("{base_url}/cmmn-runtime/tasks/missing-task"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(missing.status().as_u16(), 404);
}

#[tokio::test]
async fn cmmn_task_variables_create_conflict_scope_and_delete() {
    // Java: TaskVariableCollectionResource.java:122-228 / TaskVariableResource.java:94-167.
    let (base_url, client) = spawn_server().await;
    let (case_id, task_id) = deploy_and_start_case(&base_url, &client).await;

    // G6: create two GLOBAL variables → 201.
    let create = client
        .post(format!("{base_url}/cmmn-runtime/tasks/{task_id}/variables"))
        .basic_auth("admin", Some("test"))
        .json(&json!([
            { "name": "alpha", "value": 1, "scope": "global" },
            { "name": "beta", "value": "two", "scope": "global" }
        ]))
        .send()
        .await
        .unwrap();
    assert_eq!(create.status().as_u16(), 201);
    let created: Value = create.json().await.unwrap();
    assert_eq!(created.as_array().unwrap().len(), 2);

    // G6: already exists → 409.
    let conflict = client
        .post(format!("{base_url}/cmmn-runtime/tasks/{task_id}/variables"))
        .basic_auth("admin", Some("test"))
        .json(&json!([{ "name": "alpha", "value": 9, "scope": "global" }]))
        .send()
        .await
        .unwrap();
    assert_eq!(conflict.status().as_u16(), 409);

    // G6: mixed scopes → 400 (TaskVariableCollectionResource.java:170-172).
    let mixed = client
        .post(format!("{base_url}/cmmn-runtime/tasks/{task_id}/variables"))
        .basic_auth("admin", Some("test"))
        .json(&json!([
            { "name": "a", "value": 1, "scope": "global" },
            { "name": "b", "value": 2, "scope": "local" }
        ]))
        .send()
        .await
        .unwrap();
    assert_eq!(mixed.status().as_u16(), 400);

    // G6/P115: local scope (Java default) creates a task-local variable → 201.
    let local = client
        .post(format!("{base_url}/cmmn-runtime/tasks/{task_id}/variables"))
        .basic_auth("admin", Some("test"))
        .json(&json!([{ "name": "localVar", "value": 1, "scope": "local" }]))
        .send()
        .await
        .unwrap();
    assert_eq!(local.status().as_u16(), 201);
    let local_created: Value = local.json().await.unwrap();
    assert_eq!(local_created.as_array().unwrap().len(), 1);
    assert_eq!(local_created[0]["value"], json!(1));

    // G8: update existing variable → 200.
    let update = client
        .put(format!(
            "{base_url}/cmmn-runtime/tasks/{task_id}/variables/alpha?scope=global"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "name": "alpha", "value": 42 }))
        .send()
        .await
        .unwrap();
    assert_eq!(update.status().as_u16(), 200);
    assert_eq!(update.json::<Value>().await.unwrap()["value"], json!(42));

    // G8: body name mismatch → 400 (TaskVariableResource.java:123-125).
    let mismatch = client
        .put(format!(
            "{base_url}/cmmn-runtime/tasks/{task_id}/variables/alpha?scope=global"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "name": "beta", "value": 3 }))
        .send()
        .await
        .unwrap();
    assert_eq!(mismatch.status().as_u16(), 400);

    // G8: missing variable → 404 (TaskVariableBaseResource.java:229-231).
    let missing = client
        .put(format!(
            "{base_url}/cmmn-runtime/tasks/{task_id}/variables/ghost?scope=global"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "name": "ghost", "value": 1 }))
        .send()
        .await
        .unwrap();
    assert_eq!(missing.status().as_u16(), 404);

    // G8/G9: alpha lives in the GLOBAL scope, so a LOCAL-scope read/update of it
    // is not found (TaskVariableBaseResource.java:94-99 / :229-231).
    let local_get = client
        .get(format!(
            "{base_url}/cmmn-runtime/tasks/{task_id}/variables/alpha?scope=local"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(local_get.status().as_u16(), 404, "global variable is not in local scope");
    let local_put = client
        .put(format!(
            "{base_url}/cmmn-runtime/tasks/{task_id}/variables/alpha?scope=local"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "name": "alpha", "value": 1 }))
        .send()
        .await
        .unwrap();
    assert_eq!(local_put.status().as_u16(), 404);

    // G7: delete all (local) variables → 204, case variables untouched.
    let delete_all = client
        .delete(format!("{base_url}/cmmn-runtime/tasks/{task_id}/variables"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(delete_all.status().as_u16(), 204);

    // G9: delete single GLOBAL variable → 204; then missing → 404.
    let delete_one = client
        .delete(format!(
            "{base_url}/cmmn-runtime/tasks/{task_id}/variables/beta?scope=global"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(delete_one.status().as_u16(), 204);

    let delete_missing = client
        .delete(format!(
            "{base_url}/cmmn-runtime/tasks/{task_id}/variables/beta?scope=global"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(delete_missing.status().as_u16(), 404);

    // Case still has the non-deleted variable.
    let case_vars: Value = client
        .get(format!(
            "{base_url}/cmmn-runtime/case-instances/{case_id}/variables"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let names: Vec<&str> = case_vars
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|v| v["name"].as_str())
        .collect();
    assert!(names.contains(&"alpha"), "G7/G9 do not touch GLOBAL case variables");
    assert!(!names.contains(&"beta"), "beta deleted by G9");
}
