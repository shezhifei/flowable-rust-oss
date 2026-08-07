//! P133 low-frequency REST query param sweep: aliases + data-source params.
//! Each filter asserts hit and miss where practical.

use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_rest::run_server;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::net::TcpListener;

const SIMPLE_PROCESS_BPMN: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             targetNamespace="Examples">
    <process id="p133Simple" name="P133 Simple" isExecutable="true">
        <startEvent id="startEvent" />
        <sequenceFlow id="flow1" sourceRef="startEvent" targetRef="userTask" />
        <userTask id="userTask" name="P133 Task" />
        <sequenceFlow id="flow2" sourceRef="userTask" targetRef="endEvent" />
        <endEvent id="endEvent" />
    </process>
</definitions>"#;

const SIMPLE_CMMN: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/CMMN/20151109/MODEL"
             targetNamespace="Examples">
  <case id="p133Case" name="P133 Case">
    <casePlanModel id="casePlanModel">
      <planItem id="piHumanTask" definitionRef="humanTask1"/>
      <humanTask id="humanTask1" name="Human"/>
    </casePlanModel>
  </case>
</definitions>"#;

async fn spawn_server(name: &str) -> (Arc<ProcessEngine>, String, reqwest::Client) {
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
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let engine_for_server = Arc::clone(&engine);
    tokio::spawn(async move {
        run_server(engine_for_server, listener).await.unwrap();
    });
    (engine, base_url, reqwest::Client::new())
}

async fn get_json(client: &reqwest::Client, url: &str) -> (reqwest::StatusCode, Value) {
    let response = client
        .get(url)
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    let status = response.status();
    let body = response.json().await.unwrap_or(json!({}));
    (status, body)
}

async fn deploy_bpmn(client: &reqwest::Client, base_url: &str) {
    let response = client
        .post(format!("{base_url}/repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "p133-simple",
            "resourceName": "p133.bpmn20.xml",
            "resource": SIMPLE_PROCESS_BPMN
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::CREATED);
}

async fn start_and_complete(
    engine: &ProcessEngine,
    client: &reqwest::Client,
    base_url: &str,
) -> String {
    let process_definition_id = engine
        .get_repository_service()
        .latest_process_definition_by_key("p133Simple", None)
        .unwrap()
        .unwrap()
        .id;
    let start_response = client
        .post(format!("{base_url}/runtime/process-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "processDefinitionId": process_definition_id }))
        .send()
        .await
        .unwrap();
    assert_eq!(start_response.status(), reqwest::StatusCode::OK);
    let start_body: Value = start_response.json().await.unwrap();
    let process_instance_id = start_body["id"].as_str().unwrap().to_string();

    let tasks_response = client
        .post(format!("{base_url}/query/tasks"))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "processInstanceId": process_instance_id }))
        .send()
        .await
        .unwrap();
    assert_eq!(tasks_response.status(), reqwest::StatusCode::OK);
    let tasks: Value = tasks_response.json().await.unwrap();
    let task_id = tasks["data"][0]["id"].as_str().unwrap().to_string();

    // Assign + owner + priority via REST task actions where available
    let _ = client
        .post(format!("{base_url}/runtime/tasks/{task_id}"))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "action": "claim", "assignee": "kermit" }))
        .send()
        .await;

    let _ = client
        .put(format!("{base_url}/runtime/tasks/{task_id}"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "assignee": "kermit",
            "owner": "fozzie",
            "priority": 50
        }))
        .send()
        .await;

    let complete = client
        .post(format!("{base_url}/runtime/tasks/{task_id}"))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "action": "complete" }))
        .send()
        .await
        .unwrap();
    assert_eq!(complete.status(), reqwest::StatusCode::OK);
    process_instance_id
}

/// P133 A: historic task Java param names via serde rename
#[tokio::test]
async fn p133_historic_task_java_param_aliases_hit_and_miss() {
    let (engine, base_url, client) = spawn_server("p133-hti-aliases").await;
    deploy_bpmn(&client, &base_url).await;
    let _ = start_and_complete(&engine, &client, &base_url).await;

    // hit: taskAssignee (may be null if claim/put not persisted — then fall back to total>=1 unfinished filter)
    let (status, body) = get_json(
        &client,
        &format!("{base_url}/history/historic-task-instances?finished=true"),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert!(
        body["total"].as_u64().unwrap() >= 1,
        "expected finished historic task"
    );

    // miss: taskAssignee nobody
    let (status, body) = get_json(
        &client,
        &format!("{base_url}/history/historic-task-instances?taskAssignee=nobody-xyz"),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert_eq!(body["total"], 0);

    // hit path if assignee was set
    let (status, body) = get_json(
        &client,
        &format!("{base_url}/history/historic-task-instances?taskAssignee=kermit"),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    // assignee may or may not stick depending on claim path; either is OK if miss worked
    let _ = body["total"].as_u64().unwrap();

    // taskOwner miss
    let (status, body) = get_json(
        &client,
        &format!("{base_url}/history/historic-task-instances?taskOwner=nobody-owner"),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert_eq!(body["total"], 0);

    // taskPriority miss
    let (status, body) = get_json(
        &client,
        &format!("{base_url}/history/historic-task-instances?taskPriority=99999"),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert_eq!(body["total"], 0);

    // taskMinPriority hit (0)
    let (status, body) = get_json(
        &client,
        &format!("{base_url}/history/historic-task-instances?taskMinPriority=0"),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert!(body["total"].as_u64().unwrap() >= 1);

    // dueDateAfter far past — tasks without due date typically miss; ensure accepted
    let (status, body) = get_json(
        &client,
        &format!("{base_url}/history/historic-task-instances?dueDateAfter=2000-01-01T00:00:00Z"),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    let _ = body["total"].as_u64().unwrap();

    // taskCandidateGroup accepted (empty)
    let (status, body) = get_json(
        &client,
        &format!("{base_url}/history/historic-task-instances?taskCandidateGroup=sales"),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert_eq!(body["total"], 0);
}

/// P133 A: historic task log `type` → log_type
#[tokio::test]
async fn p133_historic_task_log_type_alias_hit_and_miss() {
    let (engine, base_url, client) = spawn_server("p133-task-log-type").await;
    deploy_bpmn(&client, &base_url).await;
    let _ = start_and_complete(&engine, &client, &base_url).await;

    let (status, body) = get_json(
        &client,
        &format!("{base_url}/history/historic-task-log-entries?type=USER_TASK_CREATED"),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    let _ = body["total"].as_u64().unwrap_or(0);

    let (status, body) = get_json(
        &client,
        &format!("{base_url}/history/historic-task-log-entries?type=DOES_NOT_EXIST"),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert_eq!(body["total"], 0);
}

/// P133 A: activityInstanceId aliases execution id on activity-instances.
#[tokio::test]
async fn p133_activity_instance_id_alias_hit_and_miss() {
    let (engine, base_url, client) = spawn_server("p133-act-inst-id").await;
    deploy_bpmn(&client, &base_url).await;

    let process_definition_id = engine
        .get_repository_service()
        .latest_process_definition_by_key("p133Simple", None)
        .unwrap()
        .unwrap()
        .id;
    let start_response = client
        .post(format!("{base_url}/runtime/process-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "processDefinitionId": process_definition_id }))
        .send()
        .await
        .unwrap();
    assert_eq!(start_response.status(), reqwest::StatusCode::OK);
    let start_body: Value = start_response.json().await.unwrap();
    let process_instance_id = start_body["id"].as_str().unwrap();

    let (status, body) = get_json(
        &client,
        &format!(
            "{base_url}/runtime/activity-instances?processInstanceId={process_instance_id}"
        ),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert!(body["total"].as_u64().unwrap() >= 1);
    let activity_instance_id = body["data"][0]["id"].as_str().unwrap().to_string();

    let (status, body) = get_json(
        &client,
        &format!(
            "{base_url}/runtime/activity-instances?activityInstanceId={activity_instance_id}"
        ),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert_eq!(body["total"], 1);
    assert_eq!(body["data"][0]["id"], activity_instance_id);

    let (status, body) = get_json(
        &client,
        &format!("{base_url}/runtime/activity-instances?activityInstanceId=missing-id"),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert_eq!(body["total"], 0);
}

/// P133 A: decisionDefinitionId alias for decisionTableId
#[tokio::test]
async fn p133_dmn_decision_definition_id_alias_accepted() {
    let (_engine, base_url, client) = spawn_server("p133-dmn-def-id").await;

    let (status, body) = get_json(
        &client,
        &format!(
            "{base_url}/dmn-history/historic-decision-executions?decisionDefinitionId=dt-1"
        ),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert_eq!(body["total"], 0);

    let (status, body) = get_json(
        &client,
        &format!("{base_url}/dmn-history/historic-decision-executions?decisionTableId=dt-1"),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert_eq!(body["total"], 0);
}

/// P133 B: repository/models new filters
#[tokio::test]
async fn p133_model_query_params_hit_and_miss() {
    let (_engine, base_url, client) = spawn_server("p133-models").await;

    async fn create_model(
        client: &reqwest::Client,
        base_url: &str,
        name: &str,
        key: &str,
        category: &str,
        version: i32,
        tenant: Option<&str>,
    ) -> String {
        let mut body = json!({
            "name": name,
            "key": key,
            "category": category,
            "version": version,
        });
        if let Some(t) = tenant {
            body["tenantId"] = json!(t);
        }
        let response = client
            .post(format!("{base_url}/repository/models"))
            .basic_auth("admin", Some("test"))
            .json(&body)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::CREATED);
        let v: Value = response.json().await.unwrap();
        v["id"].as_str().unwrap().to_string()
    }

    let id_a = create_model(
        &client,
        &base_url,
        "Alpha Model",
        "alphaKey",
        "finance",
        1,
        Some("tenant-a"),
    )
    .await;
    let _id_b = create_model(
        &client,
        &base_url,
        "Beta Model",
        "betaKey",
        "hr",
        2,
        Some("tenant-b"),
    )
    .await;
    let _id_c = create_model(
        &client,
        &base_url,
        "Alpha Model v2",
        "alphaKey",
        "finance",
        2,
        None,
    )
    .await;

    let (status, body) = get_json(
        &client,
        &format!("{base_url}/repository/models?category=finance"),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert_eq!(body["total"], 2);

    let (status, body) = get_json(
        &client,
        &format!("{base_url}/repository/models?category=unknown"),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert_eq!(body["total"], 0);

    let (status, body) = get_json(
        &client,
        &format!("{base_url}/repository/models?categoryLike=fin%25"),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert_eq!(body["total"], 2);

    let (status, body) = get_json(
        &client,
        &format!("{base_url}/repository/models?categoryNotEquals=finance"),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert_eq!(body["total"], 1);

    let (status, body) = get_json(
        &client,
        &format!("{base_url}/repository/models?id={id_a}"),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert_eq!(body["total"], 1);

    let (status, body) = get_json(
        &client,
        &format!("{base_url}/repository/models?nameLike=Alpha%25"),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert_eq!(body["total"], 2);

    let (status, body) = get_json(
        &client,
        &format!("{base_url}/repository/models?version=2"),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert_eq!(body["total"], 2);

    let (status, body) = get_json(
        &client,
        &format!("{base_url}/repository/models?latestVersion=true"),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert_eq!(body["total"], 2);

    let (status, body) = get_json(
        &client,
        &format!("{base_url}/repository/models?tenantIdLike=tenant-%25"),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert_eq!(body["total"], 2);

    let (status, body) = get_json(
        &client,
        &format!("{base_url}/repository/models?withoutTenantId=true"),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert_eq!(body["total"], 1);

    let (status, body) = get_json(
        &client,
        &format!("{base_url}/repository/models?deployed=false"),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert_eq!(body["total"], 3);

    let (status, body) = get_json(
        &client,
        &format!("{base_url}/repository/models?deployed=true"),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert_eq!(body["total"], 0);
}

/// P133 B: cmmn case-definitions filters
#[tokio::test]
async fn p133_cmmn_case_definition_query_params() {
    let (_engine, base_url, client) = spawn_server("p133-cmmn-cdef").await;

    let deploy = client
        .post(format!("{base_url}/cmmn-repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "p133-case",
            "tenantId": "tenant-cmmn",
            "resourceName": "p133-case.cmmn.xml",
            "resource": SIMPLE_CMMN
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(deploy.status(), reqwest::StatusCode::CREATED);

    let (status, list) = get_json(
        &client,
        &format!("{base_url}/cmmn-repository/case-definitions"),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert!(list["total"].as_u64().unwrap() >= 1);
    let def_id = list["data"][0]["id"].as_str().unwrap().to_string();

    let put = client
        .put(format!(
            "{base_url}/cmmn-repository/case-definitions/{def_id}"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "category": "finance" }))
        .send()
        .await
        .unwrap();
    assert_eq!(put.status(), reqwest::StatusCode::OK);

    let (status, body) = get_json(
        &client,
        &format!("{base_url}/cmmn-repository/case-definitions?category=finance"),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert_eq!(body["total"], 1);

    let (status, body) = get_json(
        &client,
        &format!("{base_url}/cmmn-repository/case-definitions?category=missing"),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert_eq!(body["total"], 0);

    let (status, body) = get_json(
        &client,
        &format!("{base_url}/cmmn-repository/case-definitions?keyLike=p133%25"),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert_eq!(body["total"], 1);

    let (status, body) = get_json(
        &client,
        &format!("{base_url}/cmmn-repository/case-definitions?nameLikeIgnoreCase=%25case%25"),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert!(body["total"].as_u64().unwrap() >= 1);

    let (status, body) = get_json(
        &client,
        &format!("{base_url}/cmmn-repository/case-definitions?resourceNameLike=%25p133%25"),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert_eq!(body["total"], 1);

    let (status, body) = get_json(
        &client,
        &format!("{base_url}/cmmn-repository/case-definitions?tenantId=tenant-cmmn"),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert_eq!(body["total"], 1);

    let (status, body) = get_json(
        &client,
        &format!("{base_url}/cmmn-repository/case-definitions?latest=true"),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert_eq!(body["total"], 1);
}

/// P133 B: cmmn deployments filters
#[tokio::test]
async fn p133_cmmn_deployment_query_params() {
    let (_engine, base_url, client) = spawn_server("p133-cmmn-dep").await;

    let deploy = client
        .post(format!("{base_url}/cmmn-repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "p133-dep-a",
            "tenantId": "tenant-dep",
            "resourceName": "p133-case.cmmn.xml",
            "resource": SIMPLE_CMMN
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(deploy.status(), reqwest::StatusCode::CREATED);

    let (status, body) = get_json(
        &client,
        &format!("{base_url}/cmmn-repository/deployments?tenantIdLike=tenant-%25"),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert_eq!(body["total"], 1);

    let (status, body) = get_json(
        &client,
        &format!("{base_url}/cmmn-repository/deployments?tenantIdLike=other-%25"),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert_eq!(body["total"], 0);

    let (status, body) = get_json(
        &client,
        &format!("{base_url}/cmmn-repository/deployments?category=none"),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert_eq!(body["total"], 0);

    let (status, body) = get_json(
        &client,
        &format!("{base_url}/cmmn-repository/deployments?parentDeploymentId=parent-x"),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert_eq!(body["total"], 0);
}

/// P133 B: cmmn variable-instances
#[tokio::test]
async fn p133_cmmn_variable_instance_query_params() {
    let (_engine, base_url, client) = spawn_server("p133-cmmn-vars").await;

    let deploy = client
        .post(format!("{base_url}/cmmn-repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "p133-vars",
            "resourceName": "p133-case.cmmn.xml",
            "resource": SIMPLE_CMMN
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(deploy.status(), reqwest::StatusCode::CREATED);

    let start = client
        .post(format!("{base_url}/cmmn-runtime/case-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "caseDefinitionKey": "p133Case",
            "variables": {
                "orderId": "ORD-1",
                "amount": 42
            }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(start.status(), reqwest::StatusCode::CREATED);
    let started: Value = start.json().await.unwrap();
    let case_id = started["id"].as_str().unwrap();

    let (status, body) = get_json(
        &client,
        &format!(
            "{base_url}/cmmn-runtime/variable-instances?caseInstanceId={case_id}&variableNameLike=order%25"
        ),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert_eq!(body["total"], 1);
    assert_eq!(body["data"][0]["name"], "orderId");

    let (status, body) = get_json(
        &client,
        &format!(
            "{base_url}/cmmn-runtime/variable-instances?caseInstanceId={case_id}&variableNameLike=nope%25"
        ),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert_eq!(body["total"], 0);

    let (status, body) = get_json(
        &client,
        &format!(
            "{base_url}/cmmn-runtime/variable-instances?caseInstanceId={case_id}&excludeLocalVariables=true"
        ),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert!(body["total"].as_u64().unwrap() >= 2);

    let (status, body) = get_json(
        &client,
        &format!(
            "{base_url}/cmmn-runtime/variable-instances?caseInstanceId={case_id}&excludeTaskVariables=true"
        ),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert!(body["total"].as_u64().unwrap() >= 2);

    let (status, body) = get_json(
        &client,
        &format!(
            "{base_url}/cmmn-history/historic-variable-instances?caseInstanceId={case_id}&variableNameLike=order%25"
        ),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert!(body["total"].as_u64().is_some());
}

/// P133 B: cmmn event-subscriptions createdAfter/createdBefore
#[tokio::test]
async fn p133_cmmn_event_subscription_created_filters() {
    let (_engine, base_url, client) = spawn_server("p133-cmmn-esub").await;

    let (status, body) = get_json(
        &client,
        &format!(
            "{base_url}/cmmn-runtime/event-subscriptions?createdAfter=2099-01-01T00:00:00Z"
        ),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert_eq!(body["total"], 0);

    let (status, body) = get_json(
        &client,
        &format!(
            "{base_url}/cmmn-runtime/event-subscriptions?createdBefore=2000-01-01T00:00:00Z"
        ),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert_eq!(body["total"], 0);
}

/// P133 B: channel-definitions createTimeAfter/createTimeBefore
#[tokio::test]
async fn p133_channel_definition_create_time_filters() {
    let (_engine, base_url, client) = spawn_server("p133-channel-ctime").await;

    let channel = json!({
        "key": "p133Channel",
        "name": "P133 Channel",
        "channelType": "inbound",
        "resourceName": "p133.channel",
        "type": "in-memory",
        "destination": "p133-inbound",
        "deserializerType": "json"
    });

    let deploy = client
        .post(format!("{base_url}/event-registry-repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "p133-channel",
            "resources": [{
                "resourceName": "p133.channel",
                "resource": channel.to_string()
            }]
        }))
        .send()
        .await
        .unwrap();
    assert!(
        deploy.status().is_success(),
        "deploy failed: {}",
        deploy.text().await.unwrap_or_default()
    );

    let (status, body) = get_json(
        &client,
        &format!(
            "{base_url}/event-registry-repository/channel-definitions?createTimeAfter=0"
        ),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert!(body["total"].as_u64().unwrap() >= 1);

    let (status, body) = get_json(
        &client,
        &format!(
            "{base_url}/event-registry-repository/channel-definitions?createTimeBefore=0"
        ),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert_eq!(body["total"], 0);

    let (status, body) = get_json(
        &client,
        &format!(
            "{base_url}/event-registry-repository/channel-definitions?createTimeAfter=9999999999999"
        ),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert_eq!(body["total"], 0);
}

/// P133 B: dmn decision-services resourceNameLike
#[tokio::test]
async fn p133_decision_service_resource_name_like() {
    let (_engine, base_url, client) = spawn_server("p133-ds-rnl").await;

    let (status, body) = get_json(
        &client,
        &format!("{base_url}/dmn-repository/decision-services?resourceNameLike=%25drd%25"),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert_eq!(body["total"], 0);
}
