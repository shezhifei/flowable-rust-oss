// P115: CMMN task-local variables over REST — the /cmmn-runtime/tasks/{taskId}
// /variables local scope.
//
// Java references:
// - TaskVariableCollectionResource.java:68-96 (GET scope merge), :122-212
//   (POST create, scope defaults to LOCAL :164-166), :219-228 (DELETE all local)
// - TaskVariableResource.java:65-70 (GET single), :94-130 (PUT single,
//   default LOCAL TaskVariableBaseResource.java:210-213), :138-167 (DELETE single,
//   default LOCAL :147-150)
// - TaskVariableBaseResource.java:67-106 (getVariableFromRequestWithoutAccessCheck:
//   no scope → local first, then global), :108-122 (hasVariableOnScope),
//   :221-253 (setVariable: create when missing → 409, update when missing → 404)
// - Lifecycle: task-local variables die with the task on completion
//   (HumanTaskActivityBehavior.java:482 → CMMN TaskHelper.java:109-128)

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
      <planItem id="planItemApprove" name="Approve" definitionRef="approveTask" />
      <humanTask id="reviewTask" name="Review" isBlocking="true" />
      <humanTask id="approveTask" name="Approve" isBlocking="true" />
    </casePlanModel>
  </case>
</definitions>"#;

async fn spawn_server() -> (String, reqwest::Client) {
    let engine = Arc::new(ProcessEngine::new("rest-cmmn-task-local-vars".to_string()));
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

async fn deploy_and_start_case(base_url: &str, client: &reqwest::Client) -> (String, Vec<String>) {
    let deploy_response = client
        .post(format!("{base_url}/cmmn-repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "CMMN task local variables deployment",
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
    let mut task_ids: Vec<String> = tasks_body["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|task| task["id"].as_str().unwrap().to_string())
        .collect();
    task_ids.sort();

    (case_instance_id, task_ids)
}

async fn task_local_variable_names(
    base_url: &str,
    client: &reqwest::Client,
    task_id: &str,
    scope: &str,
) -> Vec<String> {
    let response = client
        .get(format!(
            "{base_url}/cmmn-runtime/tasks/{task_id}/variables?scope={scope}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success());
    response
        .json::<Value>()
        .await
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|variable| variable["name"].as_str().map(str::to_string))
        .collect()
}

#[tokio::test]
async fn cmmn_task_local_variables_crud() {
    let (base_url, client) = spawn_server().await;
    let (_case_id, task_ids) = deploy_and_start_case(&base_url, &client).await;
    let task_id = &task_ids[0];

    // POST local → 201 (TaskVariableCollectionResource.java:164-166 default LOCAL).
    let create = client
        .post(format!("{base_url}/cmmn-runtime/tasks/{task_id}/variables"))
        .basic_auth("admin", Some("test"))
        .json(&json!([
            { "name": "alpha", "value": 1, "scope": "local" },
            { "name": "beta", "value": "two", "scope": "local" }
        ]))
        .send()
        .await
        .unwrap();
    assert_eq!(create.status().as_u16(), 201);
    let created: Value = create.json().await.unwrap();
    assert_eq!(created.as_array().unwrap().len(), 2);
    assert_eq!(created[0]["value"], json!(1));
    assert_eq!(created[1]["value"], json!("two"));

    // POST an existing LOCAL variable → 409 (TaskVariableCollectionResource.java:174-176).
    let conflict = client
        .post(format!("{base_url}/cmmn-runtime/tasks/{task_id}/variables"))
        .basic_auth("admin", Some("test"))
        .json(&json!([{ "name": "alpha", "value": 9, "scope": "local" }]))
        .send()
        .await
        .unwrap();
    assert_eq!(conflict.status().as_u16(), 409);

    // GET scope=local → only task-local variables.
    let locals = task_local_variable_names(&base_url, &client, task_id, "local").await;
    assert_eq!(locals, vec!["alpha", "beta"]);

    // GET single local variable.
    let get_one = client
        .get(format!("{base_url}/cmmn-runtime/tasks/{task_id}/variables/alpha?scope=local"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(get_one.status().as_u16(), 200);
    assert_eq!(get_one.json::<Value>().await.unwrap()["value"], json!(1));

    // PUT local → 200 (TaskVariableBaseResource.java:241-242 setVariableLocal).
    let update = client
        .put(format!("{base_url}/cmmn-runtime/tasks/{task_id}/variables/alpha?scope=local"))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "name": "alpha", "value": 42 }))
        .send()
        .await
        .unwrap();
    assert_eq!(update.status().as_u16(), 200);
    assert_eq!(update.json::<Value>().await.unwrap()["value"], json!(42));

    // PUT a missing local variable → 404 (TaskVariableBaseResource.java:229-231).
    let update_missing = client
        .put(format!("{base_url}/cmmn-runtime/tasks/{task_id}/variables/ghost?scope=local"))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "name": "ghost", "value": 1 }))
        .send()
        .await
        .unwrap();
    assert_eq!(update_missing.status().as_u16(), 404);

    // DELETE single local → 204; then GET → 404 (TaskVariableResource.java:152-161).
    let delete_one = client
        .delete(format!("{base_url}/cmmn-runtime/tasks/{task_id}/variables/beta?scope=local"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(delete_one.status().as_u16(), 204);
    let get_deleted = client
        .get(format!("{base_url}/cmmn-runtime/tasks/{task_id}/variables/beta?scope=local"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(get_deleted.status().as_u16(), 404);

    // DELETE all local → 204 (TaskVariableCollectionResource.java:219-228).
    let delete_all = client
        .delete(format!("{base_url}/cmmn-runtime/tasks/{task_id}/variables"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(delete_all.status().as_u16(), 204);
    let locals = task_local_variable_names(&base_url, &client, task_id, "local").await;
    assert!(locals.is_empty(), "all local variables deleted");
}

#[tokio::test]
async fn cmmn_task_local_variable_shadows_case_variable_on_unspecified_scope() {
    let (base_url, client) = spawn_server().await;
    let (case_id, task_ids) = deploy_and_start_case(&base_url, &client).await;
    let task_id = &task_ids[0];

    // A case (GLOBAL) variable under the same name.
    let set_case = client
        .post(format!("{base_url}/cmmn-runtime/case-instances/{case_id}/variables"))
        .basic_auth("admin", Some("test"))
        .json(&json!([{ "name": "shared", "value": "case", "scope": "global" }]))
        .send()
        .await
        .unwrap();
    assert!(set_case.status().is_success());

    // Write the task-local shadow.
    let set_local = client
        .post(format!("{base_url}/cmmn-runtime/tasks/{task_id}/variables"))
        .basic_auth("admin", Some("test"))
        .json(&json!([{ "name": "shared", "value": "local", "scope": "local" }]))
        .send()
        .await
        .unwrap();
    assert_eq!(set_local.status().as_u16(), 201);

    // GET with no scope → local wins (TaskVariableCollectionResource.java:76-96).
    let merged: Value = client
        .get(format!("{base_url}/cmmn-runtime/tasks/{task_id}/variables"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let shared = merged
        .as_array()
        .unwrap()
        .iter()
        .find(|variable| variable["name"] == "shared")
        .expect("shared present in merged list");
    assert_eq!(shared["value"], json!("local"), "local shadows case on merged GET");

    // GET single with no scope → local first (TaskVariableBaseResource.java:73-87).
    let single: Value = client
        .get(format!("{base_url}/cmmn-runtime/tasks/{task_id}/variables/shared"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(single["value"], json!("local"));

    // scope=local and scope=global each see only their own scope.
    assert_eq!(
        task_local_variable_names(&base_url, &client, task_id, "local").await,
        vec!["shared"]
    );
    let globals: Value = client
        .get(format!("{base_url}/cmmn-runtime/tasks/{task_id}/variables?scope=global"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let global_shared = globals
        .as_array()
        .unwrap()
        .iter()
        .find(|variable| variable["name"] == "shared")
        .expect("shared in global list");
    assert_eq!(global_shared["value"], json!("case"), "case value untouched");

    // The case variable is not overwritten by the local write.
    let case_vars: Value = client
        .get(format!("{base_url}/cmmn-runtime/case-instances/{case_id}/variables"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let case_shared = case_vars
        .as_array()
        .unwrap()
        .iter()
        .find(|variable| variable["name"] == "shared")
        .expect("shared in case variables");
    assert_eq!(case_shared["value"], json!("case"));
}

#[tokio::test]
async fn cmmn_task_completion_clears_local_variables() {
    let (base_url, client) = spawn_server().await;
    let (_case_id, task_ids) = deploy_and_start_case(&base_url, &client).await;
    let task_id = &task_ids[0];

    let create = client
        .post(format!("{base_url}/cmmn-runtime/tasks/{task_id}/variables"))
        .basic_auth("admin", Some("test"))
        .json(&json!([{ "name": "scratch", "value": "value", "scope": "local" }]))
        .send()
        .await
        .unwrap();
    assert_eq!(create.status().as_u16(), 201);

    // Complete the task (TaskResource.java:109-137 POST action "complete").
    let complete = client
        .post(format!("{base_url}/cmmn-runtime/tasks/{task_id}"))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "action": "complete" }))
        .send()
        .await
        .unwrap();
    assert_eq!(complete.status().as_u16(), 200);

    // Task-local variables die with the task
    // (HumanTaskActivityBehavior.java:482 → CMMN TaskHelper.java:109-128).
    let locals = task_local_variable_names(&base_url, &client, task_id, "local").await;
    assert!(locals.is_empty(), "local variables cleared on completion");
}

#[tokio::test]
async fn cmmn_task_local_variables_are_isolated_between_tasks() {
    let (base_url, client) = spawn_server().await;
    let (_case_id, task_ids) = deploy_and_start_case(&base_url, &client).await;
    assert_eq!(task_ids.len(), 2, "two human tasks expected");
    let (task_a, task_b) = (&task_ids[0], &task_ids[1]);

    let create_a = client
        .post(format!("{base_url}/cmmn-runtime/tasks/{task_a}/variables"))
        .basic_auth("admin", Some("test"))
        .json(&json!([{ "name": "shared", "value": "from-a", "scope": "local" }]))
        .send()
        .await
        .unwrap();
    assert_eq!(create_a.status().as_u16(), 201);
    let create_b = client
        .post(format!("{base_url}/cmmn-runtime/tasks/{task_b}/variables"))
        .basic_auth("admin", Some("test"))
        .json(&json!([{ "name": "shared", "value": "from-b", "scope": "local" }]))
        .send()
        .await
        .unwrap();
    assert_eq!(create_b.status().as_u16(), 201);

    let a: Value = client
        .get(format!("{base_url}/cmmn-runtime/tasks/{task_a}/variables/shared?scope=local"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let b: Value = client
        .get(format!("{base_url}/cmmn-runtime/tasks/{task_b}/variables/shared?scope=local"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(a["value"], json!("from-a"));
    assert_eq!(b["value"], json!("from-b"));

    // Deleting from task A leaves task B untouched.
    let delete_a = client
        .delete(format!("{base_url}/cmmn-runtime/tasks/{task_a}/variables/shared?scope=local"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(delete_a.status().as_u16(), 204);
    let get_b: Value = client
        .get(format!("{base_url}/cmmn-runtime/tasks/{task_b}/variables/shared?scope=local"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(get_b["value"], json!("from-b"));
}
