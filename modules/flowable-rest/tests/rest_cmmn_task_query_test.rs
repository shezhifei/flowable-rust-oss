// P100: CMMN task query surface — GET /cmmn-runtime/tasks and POST /cmmn-query/tasks.
//
// Java references:
// - TaskCollectionResource.java:125-349 (GET param parsing)
// - TaskQueryResource.java:50-52 (POST body → same query)
// - TaskBaseResource.java:74-86 (delegationState validation), :138-358 (query builders)
// - PaginateListUtil.java:117-131 (sort/order validation)
//
// P114: candidateUser/candidateGroup/candidateGroups/candidateOrAssigned/
// ignoreAssignee are now supported (see rest_p114_candidate_query_test.rs).
// Intentional cuts (P100 acceptance): involvedUser/involvedGroups, variable
// conditions and the tenantId family are not implemented (rejected via
// deny_unknown_fields); task-local variables are always empty; `active=false`
// (suspended) returns no tasks because the Rust engine never suspends cases.

use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::identity::entities::User;
use flowable_rest::run_server;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::net::TcpListener;

const TASK_CMMN: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/CMMN/20151109/MODEL"
             targetNamespace="http://flowable.org/cmmn">
  <case id="taskQueryCase" name="Task Query Case">
    <casePlanModel id="taskQueryPlan" name="Task Plan" autoComplete="false">
      <planItem id="planItemAlpha" definitionRef="reviewTaskAlpha" />
      <planItem id="planItemBeta" definitionRef="reviewTaskBeta" />
      <planItem id="planItemGamma" definitionRef="reviewTaskGamma" />
      <humanTask id="reviewTaskAlpha" name="Alpha review" assignee="alice" owner="owner-a" priority="50" dueDate="2026-12-31" category="work" isBlocking="true" />
      <humanTask id="reviewTaskBeta" name="Beta review" assignee="bob" owner="owner-b" priority="70" category="personal" isBlocking="true" />
      <humanTask id="reviewTaskGamma" name="Gamma deep dive" isBlocking="true" />
    </casePlanModel>
  </case>
</definitions>"#;

async fn spawn_server() -> (String, reqwest::Client) {
    let engine = Arc::new(ProcessEngine::new("rest-cmmn-task-query".to_string()));
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

async fn deploy_and_start_case(
    base_url: &str,
    client: &reqwest::Client,
) -> (String, String) {
    let deploy_response = client
        .post(format!("{base_url}/cmmn-repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "CMMN task query deployment",
            "resourceName": "task-query.cmmn",
            "resource": TASK_CMMN
        }))
        .send()
        .await
        .unwrap();
    assert!(deploy_response.status().is_success());

    let start_response = client
        .post(format!("{base_url}/cmmn-runtime/case-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "caseDefinitionKey": "taskQueryCase" }))
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
    let task_ids = tasks_body["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|task| task["id"].as_str().unwrap().to_string())
        .collect::<Vec<_>>();
    assert_eq!(task_ids.len(), 3);

    (case_instance_id, task_ids[0].clone())
}

async fn get_task_names(base_url: &str, client: &reqwest::Client, query: &str) -> Vec<String> {
    let response = client
        .get(format!("{base_url}/cmmn-runtime/tasks{query}"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success());
    let body: Value = response.json().await.unwrap();
    body["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|task| task["name"].as_str().unwrap().to_string())
        .collect()
}

#[tokio::test]
async fn cmmn_task_query_filters_by_name_and_assignee() {
    let (base_url, client) = spawn_server().await;
    let (case_id, _) = deploy_and_start_case(&base_url, &client).await;

    assert_eq!(
        get_task_names(&base_url, &client, "?name=Alpha review").await,
        vec!["Alpha review"]
    );
    assert_eq!(
        get_task_names(&base_url, &client, "?nameLike=Alpha%").await,
        vec!["Alpha review"]
    );
    assert_eq!(
        get_task_names(&base_url, &client, "?nameLikeIgnoreCase=alpha%").await,
        vec!["Alpha review"]
    );
    assert_eq!(
        get_task_names(&base_url, &client, "?nameLike=%review").await.len(),
        2
    );

    assert_eq!(
        get_task_names(&base_url, &client, "?assignee=alice").await,
        vec!["Alpha review"]
    );
    assert_eq!(
        get_task_names(&base_url, &client, "?assigneeLike=b%").await,
        vec!["Beta review"]
    );
    assert_eq!(
        get_task_names(&base_url, &client, "?owner=owner-b").await,
        vec!["Beta review"]
    );
    assert_eq!(
        get_task_names(&base_url, &client, "?ownerLike=%er%").await.len(),
        2
    );

    // unassigned present → only the task with no assignee (Java applies the
    // filter whenever the param is present; TaskBaseResource.java:182-184).
    assert_eq!(
        get_task_names(&base_url, &client, "?unassigned=true").await,
        vec!["Gamma deep dive"]
    );

    // scopeId maps to the case instance id.
    assert_eq!(
        get_task_names(&base_url, &client, &format!("?scopeId={case_id}")).await.len(),
        3
    );
}

#[tokio::test]
async fn cmmn_task_query_filters_by_category() {
    let (base_url, client) = spawn_server().await;
    deploy_and_start_case(&base_url, &client).await;

    assert_eq!(
        get_task_names(&base_url, &client, "?category=work").await,
        vec!["Alpha review"]
    );
    assert_eq!(
        get_task_names(&base_url, &client, "?categoryIn=work,personal").await.len(),
        2
    );
    assert_eq!(
        get_task_names(&base_url, &client, "?categoryNotIn=personal").await.len(),
        2,
        "work + no-category survive"
    );
    assert_eq!(
        get_task_names(&base_url, &client, "?withoutCategory=true").await,
        vec!["Gamma deep dive"]
    );
}

#[tokio::test]
async fn cmmn_task_query_filters_by_task_definition_key_and_case_definition() {
    let (base_url, client) = spawn_server().await;
    deploy_and_start_case(&base_url, &client).await;

    assert_eq!(
        get_task_names(&base_url, &client, "?taskDefinitionKey=reviewTaskBeta").await,
        vec!["Beta review"]
    );
    assert_eq!(
        get_task_names(&base_url, &client, "?taskDefinitionKeyLike=reviewTask%").await.len(),
        3
    );

    // caseDefinitionId requires the deployed definition id — fetch it first.
    let defs_response = client
        .get(format!("{base_url}/cmmn-repository/case-definitions?key=taskQueryCase"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    let defs: Value = defs_response.json().await.unwrap();
    let case_definition_id = defs["data"][0]["id"].as_str().unwrap().to_string();

    assert_eq!(
        get_task_names(
            &base_url,
            &client,
            &format!("?caseDefinitionId={case_definition_id}")
        )
        .await
        .len(),
        3
    );
    assert_eq!(
        get_task_names(&base_url, &client, "?caseDefinitionKey=taskQueryCase").await.len(),
        3
    );
    assert_eq!(
        get_task_names(&base_url, &client, "?caseDefinitionKeyLike=taskQuery%").await.len(),
        3
    );
    assert_eq!(
        get_task_names(
            &base_url,
            &client,
            "?caseDefinitionKeyLikeIgnoreCase=taskquery%"
        )
        .await
        .len(),
        3
    );
}

#[tokio::test]
async fn cmmn_task_query_filters_by_priority() {
    let (base_url, client) = spawn_server().await;
    deploy_and_start_case(&base_url, &client).await;

    assert_eq!(
        get_task_names(&base_url, &client, "?priority=50").await,
        vec!["Alpha review"]
    );
    assert_eq!(
        get_task_names(&base_url, &client, "?minimumPriority=60").await,
        vec!["Beta review"]
    );
    assert_eq!(
        get_task_names(&base_url, &client, "?maximumPriority=60").await,
        vec!["Alpha review"]
    );

    // Non-integer priority → 400 (Java Integer.valueOf, TaskCollectionResource.java:149-159).
    let response = client
        .get(format!("{base_url}/cmmn-runtime/tasks?priority=high"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 400);
}

#[tokio::test]
async fn cmmn_task_query_filters_by_due_date() {
    let (base_url, client) = spawn_server().await;
    deploy_and_start_case(&base_url, &client).await;

    assert_eq!(
        get_task_names(&base_url, &client, "?dueDate=2026-12-31").await,
        vec!["Alpha review"]
    );
    assert_eq!(
        get_task_names(&base_url, &client, "?dueBefore=2027-01-01").await,
        vec!["Alpha review"]
    );
    assert_eq!(
        get_task_names(&base_url, &client, "?dueAfter=2026-01-01").await,
        vec!["Alpha review"]
    );
    assert_eq!(
        get_task_names(&base_url, &client, "?withoutDueDate=true").await,
        vec!["Beta review", "Gamma deep dive"]
    );
}

#[tokio::test]
async fn cmmn_task_query_filters_by_created_time() {
    let (base_url, client) = spawn_server().await;
    let (case_id, _) = deploy_and_start_case(&base_url, &client).await;

    let created_at = get_task_names(
        &base_url,
        &client,
        &format!("?caseInstanceId={case_id}&nameLike=Alpha%"),
    )
    .await;
    assert_eq!(created_at, vec!["Alpha review"]);

    // createdBefore a far-future instant returns all three tasks.
    assert_eq!(
        get_task_names(&base_url, &client, "?createdBefore=2099-01-01T00:00:00Z").await.len(),
        3
    );
    // createdAfter a far-past instant returns all three tasks.
    assert_eq!(
        get_task_names(&base_url, &client, "?createdAfter=2000-01-01T00:00:00Z").await.len(),
        3
    );
    // createdOn a far-future instant returns none.
    assert_eq!(
        get_task_names(&base_url, &client, "?createdOn=2099-01-01T00:00:00Z").await.len(),
        0
    );
}

#[tokio::test]
async fn cmmn_task_query_filters_by_active_and_delegation_state() {
    let (base_url, client) = spawn_server().await;
    let (_, task_id) = deploy_and_start_case(&base_url, &client).await;

    // active=true → all (Rust never suspends); active=false (suspended) → none.
    assert_eq!(
        get_task_names(&base_url, &client, "?active=true").await.len(),
        3
    );
    assert_eq!(
        get_task_names(&base_url, &client, "?active=false").await.len(),
        0
    );

    // Delegate → delegationState=pending.
    client
        .post(format!("{base_url}/cmmn-runtime/tasks/{task_id}"))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "action": "claim", "assignee": "alice" }))
        .send()
        .await
        .unwrap();
    client
        .post(format!("{base_url}/cmmn-runtime/tasks/{task_id}"))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "action": "delegate", "assignee": "bob" }))
        .send()
        .await
        .unwrap();

    assert_eq!(
        get_task_names(&base_url, &client, "?delegationState=pending").await,
        vec!["Alpha review"]
    );
    assert_eq!(
        get_task_names(&base_url, &client, "?delegationState=resolved").await.len(),
        0
    );

    // Invalid delegationState → 400 (TaskBaseResource.java:82-83).
    let response = client
        .get(format!("{base_url}/cmmn-runtime/tasks?delegationState=banana"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 400);
}

#[tokio::test]
async fn cmmn_task_query_include_process_variables() {
    let (base_url, client) = spawn_server().await;
    let (case_id, _) = deploy_and_start_case(&base_url, &client).await;

    // Seed a case variable.
    client
        .put(format!("{base_url}/cmmn-runtime/case-instances/{case_id}/variables"))
        .basic_auth("admin", Some("test"))
        .json(&json!([{ "name": "requester", "value": "alice" }]))
        .send()
        .await
        .unwrap();

    let response = client
        .get(format!(
            "{base_url}/cmmn-runtime/tasks?includeProcessVariables=true&nameLike=Alpha%"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    let body: Value = response.json().await.unwrap();
    let variables = body["data"][0]["variables"].as_array().unwrap();
    let variable_names = variables
        .iter()
        .map(|variable| variable["name"].as_str().unwrap().to_string())
        .collect::<Vec<_>>();
    assert_eq!(variable_names, vec!["requester"]);
    assert_eq!(variables[0]["scope"], "global");

    // Without the include flag the variables array is absent/empty.
    let response = client
        .get(format!(
            "{base_url}/cmmn-runtime/tasks?name=Alpha%"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    let body: Value = response.json().await.unwrap();
    assert!(body["data"][0].get("variables").is_none());
}

#[tokio::test]
async fn cmmn_task_query_sort_and_order() {
    let (base_url, client) = spawn_server().await;
    deploy_and_start_case(&base_url, &client).await;

    // Sort by name ascending.
    assert_eq!(
        get_task_names(&base_url, &client, "?sort=name&order=asc").await,
        vec!["Alpha review", "Beta review", "Gamma deep dive"]
    );
    // Sort by name descending.
    assert_eq!(
        get_task_names(&base_url, &client, "?sort=name&order=desc").await,
        vec!["Gamma deep dive", "Beta review", "Alpha review"]
    );
    // Sort by priority ascending — Rust stores the task priority as an optional
    // string, so a task with no priority (None) sorts before any set priority.
    let response = client
        .get(format!("{base_url}/cmmn-runtime/tasks?sort=priority&order=asc"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    let body: Value = response.json().await.unwrap();
    let sorted = body["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|task| task["name"].as_str().unwrap().to_string())
        .collect::<Vec<_>>();
    assert_eq!(sorted, vec!["Gamma deep dive", "Alpha review", "Beta review"]);

    // Invalid sort field → 400 (PaginateListUtil.java:119-121).
    let response = client
        .get(format!("{base_url}/cmmn-runtime/tasks?sort=banana"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 400);

    // Invalid order → 400 (PaginateListUtil.java:128-129).
    let response = client
        .get(format!("{base_url}/cmmn-runtime/tasks?sort=name&order=sideways"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 400);
}

#[tokio::test]
async fn cmmn_task_query_post_body_supports_same_filters() {
    let (base_url, client) = spawn_server().await;
    deploy_and_start_case(&base_url, &client).await;

    let response = client
        .post(format!("{base_url}/cmmn-query/tasks"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "nameLike": "Beta%",
            "sort": "name",
            "order": "desc"
        }))
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success());
    let body: Value = response.json().await.unwrap();
    let names = body["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|task| task["name"].as_str().unwrap().to_string())
        .collect::<Vec<_>>();
    assert_eq!(names, vec!["Beta review"]);
}
