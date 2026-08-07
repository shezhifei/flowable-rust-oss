//! P49 REST contract tests for candidate group expansion, involvedGroups,
//! and ignoreAssignee (Java TaskCollectionResourceTest candidateUser group
//! membership + ignoreAssignee paths).

use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::identity::entities::Group;
use flowable_rest::run_server;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::net::TcpListener;

const CAND_GROUP_BPMN: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
    xmlns:flowable="http://flowable.org/bpmn" targetNamespace="Examples">
    <process id="p49CandGroup" name="P49 Cand Group" isExecutable="true">
        <startEvent id="start" />
        <sequenceFlow id="f1" sourceRef="start" targetRef="taskA" />
        <userTask id="taskA" name="Sales Review" flowable:candidateGroups="sales" />
        <sequenceFlow id="f2" sourceRef="taskA" targetRef="end" />
        <endEvent id="end" />
    </process>
</definitions>"#;

const INVOLVED_BPMN: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
    xmlns:flowable="http://flowable.org/bpmn" targetNamespace="Examples">
    <process id="p49Involved" name="P49 Involved" isExecutable="true">
        <startEvent id="start" />
        <sequenceFlow id="f1" sourceRef="start" targetRef="taskB" />
        <userTask id="taskB" name="HR Check" flowable:candidateGroups="hr" />
        <sequenceFlow id="f2" sourceRef="taskB" targetRef="end" />
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

async fn start_by_key(engine: &Arc<ProcessEngine>, client: &reqwest::Client, base_url: &str, key: &str) {
    let process_definition_id = engine
        .get_repository_service()
        .latest_process_definition_by_key(key, None)
        .unwrap()
        .unwrap()
        .id;
    let response = client
        .post(format!("{base_url}/runtime/process-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "processDefinitionId": process_definition_id }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
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

/// T1 REST: candidateUser expands memberships (Java TaskCollectionResourceTest
/// `?candidateUser=aSalesUser` after group membership).
#[tokio::test]
async fn t1_candidate_user_group_expansion_via_rest() {
    let (engine, base_url, client) = spawn_server("rest-p49-t1").await;
    deploy(&client, &base_url, "P49 Cand Group", CAND_GROUP_BPMN).await;
    start_by_key(&engine, &client, &base_url, "p49CandGroup").await;

    engine.get_identity_service().save_group(Group {
        id: "sales".to_string(),
        name: "Sales".to_string(),
        group_type: None,
    });
    engine
        .get_identity_service()
        .create_membership("aSalesUser".to_string(), "sales".to_string());

    let body = get_tasks(&client, &base_url, "candidateUser=aSalesUser").await;
    assert_eq!(body["total"], 1);
    assert_eq!(body["data"][0]["name"], "Sales Review");

    let body = get_tasks(&client, &base_url, "candidateUser=nobody").await;
    assert_eq!(body["total"], 0);
}

/// T3 REST: involvedGroups filters by identity-link group ids.
#[tokio::test]
async fn t3_involved_groups_via_rest() {
    let (engine, base_url, client) = spawn_server("rest-p49-t3").await;
    deploy(&client, &base_url, "P49 Involved", INVOLVED_BPMN).await;
    start_by_key(&engine, &client, &base_url, "p49Involved").await;

    let body = get_tasks(&client, &base_url, "involvedGroups=hr").await;
    assert_eq!(body["total"], 1);
    assert_eq!(body["data"][0]["name"], "HR Check");

    let response = client
        .post(format!("{base_url}/query/tasks"))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "involvedGroups": ["sales"] }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["total"], 0);
}

/// T4 REST: claimed candidate task excluded unless ignoreAssignee=true
/// (Java TaskCollectionResourceTest claim + ignoreAssignee).
#[tokio::test]
async fn t4_ignore_assignee_via_rest() {
    let (engine, base_url, client) = spawn_server("rest-p49-t4").await;
    deploy(&client, &base_url, "P49 Cand Group", CAND_GROUP_BPMN).await;
    start_by_key(&engine, &client, &base_url, "p49CandGroup").await;

    let body = get_tasks(&client, &base_url, "candidateGroup=sales").await;
    assert_eq!(body["total"], 1);
    let task_id = body["data"][0]["id"].as_str().unwrap().to_string();

    // Claim → default candidate query must drop it.
    let response = client
        .post(format!("{base_url}/runtime/tasks/{task_id}"))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "action": "claim", "assignee": "johnDoe" }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);

    let body = get_tasks(&client, &base_url, "candidateGroup=sales").await;
    assert_eq!(
        body["total"], 0,
        "default candidateGroup must exclude assigned tasks"
    );

    let body = get_tasks(
        &client,
        &base_url,
        "candidateGroup=sales&ignoreAssignee=true",
    )
    .await;
    assert_eq!(body["total"], 1);
    assert_eq!(body["data"][0]["id"], task_id);

    // candidateGroupIn REST-side path must honour the same default.
    let body = get_tasks(&client, &base_url, "candidateGroupIn=sales").await;
    assert_eq!(body["total"], 0);
    let body = get_tasks(
        &client,
        &base_url,
        "candidateGroupIn=sales&ignoreAssignee=true",
    )
    .await;
    assert_eq!(body["total"], 1);
}
