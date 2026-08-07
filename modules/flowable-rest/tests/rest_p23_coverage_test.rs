//! P23 contract tests: historic-process-instances / historic-task-instances /
//! historic-activity-instances query parameter widening plus the historic
//! process instance comment POST/DELETE endpoints. Observable semantics follow
//! the Java resources (`HistoricProcessInstanceQueryRequest`,
//! `HistoricTaskInstanceQueryRequest`, `HistoricActivityInstanceQueryRequest`,
//! `HistoricProcessInstanceCommentCollectionResource`,
//! `HistoricProcessInstanceCommentResource`, `AddCommentCmd`).

use chrono::{DateTime, Duration, Utc};
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::history::historic_entities::{
    HistoricActivityInstance, HistoricProcessInstance, HistoricTaskInstance,
    HistoricVariableInstance,
};
use flowable_rest::run_server;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::net::TcpListener;

const USER_TASK_BPMN: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
    xmlns:flowable="http://flowable.org/bpmn" targetNamespace="P23Category">
    <process id="p23UserTaskProcess" name="P23 User Task" isExecutable="true">
        <startEvent id="start" />
        <sequenceFlow id="f1" sourceRef="start" targetRef="review" />
        <userTask id="review" name="P23 Review" />
        <sequenceFlow id="f2" sourceRef="review" targetRef="end" />
        <endEvent id="end" />
    </process>
</definitions>"#;

const CHILD_BPMN: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
    targetNamespace="P23Category">
    <process id="p23Child" name="P23 Child" isExecutable="true">
        <startEvent id="start" />
        <sequenceFlow id="f1" sourceRef="start" targetRef="childTask" />
        <userTask id="childTask" name="Child Task" />
        <sequenceFlow id="f2" sourceRef="childTask" targetRef="end" />
        <endEvent id="end" />
    </process>
</definitions>"#;

const PARENT_BPMN: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
    targetNamespace="P23Category">
    <process id="p23Parent" name="P23 Parent" isExecutable="true">
        <startEvent id="start" />
        <sequenceFlow id="f1" sourceRef="start" targetRef="callChild" />
        <callActivity id="callChild" calledElement="p23Child" />
        <sequenceFlow id="f2" sourceRef="callChild" targetRef="end" />
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
) -> String {
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
    let body: Value = response.json().await.unwrap();
    body["id"].as_str().unwrap().to_string()
}

async fn post_query(
    client: &reqwest::Client,
    base_url: &str,
    path: &str,
    body: Value,
) -> (reqwest::StatusCode, Value) {
    let response = client
        .post(format!("{base_url}{path}"))
        .basic_auth("admin", Some("test"))
        .json(&body)
        .send()
        .await
        .unwrap();
    let status = response.status();
    let body: Value = response.json().await.unwrap();
    (status, body)
}

fn returned_ids(body: &Value) -> Vec<String> {
    body["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["id"].as_str().unwrap().to_string())
        .collect()
}

fn millis(time: DateTime<Utc>) -> String {
    time.timestamp_millis().to_string()
}

fn base_time() -> DateTime<Utc> {
    DateTime::from_timestamp_millis(1_750_000_000_000).unwrap()
}

#[allow(clippy::too_many_arguments)]
fn seed_historic_process_instance(
    engine: &Arc<ProcessEngine>,
    id: &str,
    process_definition_id: &str,
    business_key: Option<&str>,
    start_time: DateTime<Utc>,
    end_time: Option<DateTime<Utc>>,
    delete_reason: Option<&str>,
) {
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    store.insert_historic_process_instance(
        &HistoricProcessInstance {
            id: id.to_string(),
            process_definition_id: process_definition_id.to_string(),
            business_key: business_key.map(str::to_string),
            start_time,
            end_time,
            duration_ms: end_time.map(|end| (end - start_time).num_milliseconds()),
            start_user_id: None,
            delete_reason: delete_reason.map(str::to_string),
        },
        &mut session,
    );
    session.flush_and_commit().unwrap();
}

#[allow(clippy::too_many_arguments)]
fn seed_historic_task(
    engine: &Arc<ProcessEngine>,
    id: &str,
    process_instance_id: &str,
    process_definition_id: Option<&str>,
    task_definition_key: &str,
    name: &str,
    description: Option<&str>,
    assignee: Option<&str>,
    owner: Option<&str>,
    priority: Option<i32>,
    due_date: Option<DateTime<Utc>>,
    start_time: DateTime<Utc>,
    end_time: Option<DateTime<Utc>>,
    delete_reason: Option<&str>,
) {
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    store.insert_historic_task_instance(
        HistoricTaskInstance {
            id: id.to_string(),
            process_instance_id: process_instance_id.to_string(),
            process_definition_id: process_definition_id.map(str::to_string),
            execution_id: format!("{id}-execution"),
            task_definition_key: Some(task_definition_key.to_string()),
            name: Some(name.to_string()),
            description: description.map(str::to_string),
            assignee: assignee.map(str::to_string),
            owner: owner.map(str::to_string),
            claim_time: None,
            tenant_id: None,
            category: None,
            form_key: None,
            parent_task_id: None,
            priority,
            due_date,
            start_time,
            end_time,
            duration_ms: end_time.map(|end| (end - start_time).num_milliseconds()),
            delete_reason: delete_reason.map(str::to_string),
        },
        &mut session,
    );
    session.flush_and_commit().unwrap();
}

fn seed_historic_activity(
    engine: &Arc<ProcessEngine>,
    id: &str,
    process_instance_id: &str,
    activity_id: &str,
    activity_type: &str,
    assignee: Option<&str>,
    end_time: Option<DateTime<Utc>>,
) {
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    store.insert_historic_activity_instance(
        HistoricActivityInstance {
            id: id.to_string(),
            activity_id: activity_id.to_string(),
            activity_name: Some(activity_id.to_string()),
            activity_type: activity_type.to_string(),
            process_instance_id: process_instance_id.to_string(),
            execution_id: format!("{process_instance_id}-execution"),
            start_time: base_time(),
            end_time,
            duration_ms: None,
            assignee: assignee.map(str::to_string),
            delete_reason: None,
        },
        &mut session,
    );
    session.flush_and_commit().unwrap();
}

fn seed_historic_variable(
    engine: &Arc<ProcessEngine>,
    process_instance_id: &str,
    task_id: Option<&str>,
    name: &str,
    value: Value,
) {
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    store.insert_historic_variable_instance(
        &HistoricVariableInstance {
            id: format!(
                "{process_instance_id}-{}-{name}",
                task_id.unwrap_or("global")
            ),
            process_instance_id: process_instance_id.to_string(),
            execution_id: None,
            task_id: task_id.map(str::to_string),
            name: name.to_string(),
            variable_type: "string".to_string(),
            value,
            create_time: base_time(),
            last_updated_time: base_time(),
        },
        &mut session,
    );
    session.flush_and_commit().unwrap();
}

// ── D: historic process instance comments POST / DELETE ──

#[tokio::test]
async fn historic_process_instance_comment_post_get_delete_lifecycle() {
    let (engine, base_url, client) = spawn_server("rest-p23-comments-lifecycle").await;
    deploy(&client, &base_url, "P23 User Task", USER_TASK_BPMN).await;
    let process_instance_id =
        start_process_by_key(&engine, &client, &base_url, "p23UserTaskProcess").await;

    // POST → 201 with the Java CommentResponse shape (taskId null, author =
    // authenticated user, processInstanceId set).
    let response = client
        .post(format!(
            "{base_url}/history/historic-process-instances/{process_instance_id}/comments"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({"message": "This is a comment."}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::CREATED);
    let comment: Value = response.json().await.unwrap();
    assert_eq!(comment["message"], "This is a comment.");
    assert_eq!(comment["author"], "admin");
    assert_eq!(comment["processInstanceId"], process_instance_id.as_str());
    assert!(comment["taskId"].is_null());
    assert!(comment["taskUrl"].is_null());
    let comment_id = comment["id"].as_str().unwrap().to_string();
    assert!(
        comment["processInstanceUrl"]
            .as_str()
            .unwrap()
            .contains(&format!(
                "/history/historic-process-instances/{process_instance_id}/comments/{comment_id}"
            ))
    );

    // GET collection and single comment see the new comment.
    let response = client
        .get(format!(
            "{base_url}/history/historic-process-instances/{process_instance_id}/comments"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let comments: Value = response.json().await.unwrap();
    assert_eq!(comments.as_array().unwrap().len(), 1);

    // DELETE → 204, afterwards GET and DELETE both 404.
    let comment_url = format!(
        "{base_url}/history/historic-process-instances/{process_instance_id}/comments/{comment_id}"
    );
    let response = client
        .delete(&comment_url)
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::NO_CONTENT);

    let response = client
        .get(&comment_url)
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::NOT_FOUND);

    let response = client
        .delete(&comment_url)
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn historic_process_instance_comment_error_contract() {
    let (engine, base_url, client) = spawn_server("rest-p23-comments-errors").await;
    deploy(&client, &base_url, "P23 User Task", USER_TASK_BPMN).await;
    let running_pi = start_process_by_key(&engine, &client, &base_url, "p23UserTaskProcess").await;

    // Unknown historic process instance → 404.
    let response = client
        .post(format!(
            "{base_url}/history/historic-process-instances/does-not-exist/comments"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({"message": "hello"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::NOT_FOUND);

    // Missing message → 400 "Comment text is required." (Java
    // HistoricProcessInstanceCommentCollectionResource).
    for body in [json!({}), json!({"message": null})] {
        let response = client
            .post(format!(
                "{base_url}/history/historic-process-instances/{running_pi}/comments"
            ))
            .basic_auth("admin", Some("test"))
            .json(&body)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
        let error: Value = response.json().await.unwrap();
        assert!(
            error["details"]
                .as_str()
                .unwrap()
                .contains("Comment text is required."),
            "details were: {}",
            error["details"]
        );
    }

    // Finished instance: historic row exists but the runtime execution is
    // gone → Java AddCommentCmd throws "execution {id} doesn't exist" (404).
    let definition_id = engine
        .get_repository_service()
        .latest_process_definition_by_key("p23UserTaskProcess", None)
        .unwrap()
        .unwrap()
        .id;
    seed_historic_process_instance(
        &engine,
        "p23-finished-pi",
        &definition_id,
        None,
        base_time(),
        Some(base_time() + Duration::minutes(1)),
        None,
    );
    let response = client
        .post(format!(
            "{base_url}/history/historic-process-instances/p23-finished-pi/comments"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({"message": "too late"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::NOT_FOUND);
    let error: Value = response.json().await.unwrap();
    assert!(
        error["details"]
            .as_str()
            .unwrap()
            .contains("execution p23-finished-pi doesn't exist"),
        "details were: {}",
        error["details"]
    );

    // DELETE of a comment that belongs to another process instance → 404.
    let response = client
        .post(format!(
            "{base_url}/history/historic-process-instances/{running_pi}/comments"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({"message": "owned by running pi"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::CREATED);
    let comment: Value = response.json().await.unwrap();
    let comment_id = comment["id"].as_str().unwrap();
    let response = client
        .delete(format!(
            "{base_url}/history/historic-process-instances/p23-finished-pi/comments/{comment_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::NOT_FOUND);
}

// ── A: historic-process-instances query parameters ──

#[tokio::test]
async fn historic_process_instance_query_definition_and_seeded_parameters() {
    let (engine, base_url, client) = spawn_server("rest-p23-pi-query-seeded").await;
    deploy(&client, &base_url, "P23 User Task", USER_TASK_BPMN).await;
    deploy(&client, &base_url, "P23 Child", CHILD_BPMN).await;

    let user_task_definition = engine
        .get_repository_service()
        .latest_process_definition_by_key("p23UserTaskProcess", None)
        .unwrap()
        .unwrap();
    let child_definition = engine
        .get_repository_service()
        .latest_process_definition_by_key("p23Child", None)
        .unwrap()
        .unwrap();

    let start = base_time();
    seed_historic_process_instance(
        &engine,
        "p23-pi-alpha",
        &user_task_definition.id,
        Some("ORDER-Alpha-1"),
        start,
        Some(start + Duration::minutes(5)),
        None,
    );
    seed_historic_process_instance(
        &engine,
        "p23-pi-beta",
        &child_definition.id,
        Some("invoice-beta"),
        start + Duration::minutes(1),
        None,
        None,
    );
    seed_historic_process_instance(
        &engine,
        "p23-pi-gamma",
        &user_task_definition.id,
        None,
        start + Duration::minutes(2),
        Some(start + Duration::minutes(10)),
        Some("terminated by test"),
    );
    seed_historic_variable(&engine, "p23-pi-alpha", None, "route", json!("approved"));
    seed_historic_variable(&engine, "p23-pi-alpha", None, "amount", json!(42));
    seed_historic_variable(&engine, "p23-pi-beta", None, "route", json!("rejected"));

    let path = "/query/historic-process-instances";

    // processInstanceIds (non-empty list applied, Java base resource).
    let (status, body) = post_query(
        &client,
        &base_url,
        path,
        json!({"processInstanceIds": ["p23-pi-alpha", "p23-pi-gamma"]}),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert_eq!(returned_ids(&body), vec!["p23-pi-alpha", "p23-pi-gamma"]);

    // businessKeyLikeIgnoreCase (Java processBusinessKeyLikeIgnoreCase).
    let (status, body) = post_query(
        &client,
        &base_url,
        path,
        json!({"businessKeyLikeIgnoreCase": "order-%"}),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert_eq!(returned_ids(&body), vec!["p23-pi-alpha"]);

    // processDefinitionKeys / processDefinitionKeyIn include, the exclude
    // variants remove.
    for field in ["processDefinitionKeys", "processDefinitionKeyIn"] {
        let (status, body) =
            post_query(&client, &base_url, path, json!({field: ["p23Child"]})).await;
        assert_eq!(status, reqwest::StatusCode::OK);
        assert_eq!(returned_ids(&body), vec!["p23-pi-beta"], "field {field}");
    }
    for field in ["excludeProcessDefinitionKeys", "processDefinitionKeyNotIn"] {
        let (status, body) =
            post_query(&client, &base_url, path, json!({field: ["p23Child"]})).await;
        assert_eq!(status, reqwest::StatusCode::OK);
        assert_eq!(
            returned_ids(&body),
            vec!["p23-pi-alpha", "p23-pi-gamma"],
            "field {field}"
        );
    }

    // processDefinitionName / NameLike / NameLikeIgnoreCase via repository
    // metadata join.
    let (status, body) = post_query(
        &client,
        &base_url,
        path,
        json!({"processDefinitionName": "P23 Child"}),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert_eq!(returned_ids(&body), vec!["p23-pi-beta"]);
    let (_, body) = post_query(
        &client,
        &base_url,
        path,
        json!({"processDefinitionNameLike": "%User Task"}),
    )
    .await;
    assert_eq!(returned_ids(&body), vec!["p23-pi-alpha", "p23-pi-gamma"]);
    let (_, body) = post_query(
        &client,
        &base_url,
        path,
        json!({"processDefinitionNameLikeIgnoreCase": "p23 child"}),
    )
    .await;
    assert_eq!(returned_ids(&body), vec!["p23-pi-beta"]);

    // processDefinitionVersion and processDefinitionCategory (targetNamespace).
    let (_, body) = post_query(
        &client,
        &base_url,
        path,
        json!({"processDefinitionVersion": 1}),
    )
    .await;
    assert_eq!(body["total"], 3);
    let (_, body) = post_query(
        &client,
        &base_url,
        path,
        json!({"processDefinitionVersion": 2}),
    )
    .await;
    assert_eq!(body["total"], 0);
    let (_, body) = post_query(
        &client,
        &base_url,
        path,
        json!({"processDefinitionCategory": "P23Category"}),
    )
    .await;
    assert_eq!(body["total"], 3);
    let (_, body) = post_query(
        &client,
        &base_url,
        path,
        json!({"processDefinitionCategoryLike": "P23%"}),
    )
    .await;
    assert_eq!(body["total"], 3);
    let (_, body) = post_query(
        &client,
        &base_url,
        path,
        json!({"processDefinitionCategoryLikeIgnoreCase": "p23category"}),
    )
    .await;
    assert_eq!(body["total"], 3);

    // deploymentId / deploymentIdIn.
    let child_deployment = child_definition.deployment_id.clone().unwrap();
    let (_, body) = post_query(
        &client,
        &base_url,
        path,
        json!({"deploymentId": child_deployment}),
    )
    .await;
    assert_eq!(returned_ids(&body), vec!["p23-pi-beta"]);
    let (_, body) = post_query(
        &client,
        &base_url,
        path,
        json!({"deploymentIdIn": [child_deployment, "missing-deployment"]}),
    )
    .await;
    assert_eq!(returned_ids(&body), vec!["p23-pi-beta"]);

    // startedBefore / startedAfter / finishedBefore / finishedAfter already
    // existed; state derives from end time + delete reason.
    let (_, body) = post_query(&client, &base_url, path, json!({"state": "running"})).await;
    assert_eq!(returned_ids(&body), vec!["p23-pi-beta"]);
    let (_, body) = post_query(&client, &base_url, path, json!({"state": "completed"})).await;
    assert_eq!(returned_ids(&body), vec!["p23-pi-alpha"]);
    let (_, body) = post_query(&client, &base_url, path, json!({"state": "cancelled"})).await;
    assert_eq!(returned_ids(&body), vec!["p23-pi-gamma"]);

    // variables: named equals plus the Java validation contract (value-only
    // is allowed for equals only on the process instance query).
    let (_, body) = post_query(
        &client,
        &base_url,
        path,
        json!({"variables": [{"name": "route", "operation": "equals", "value": "approved"}]}),
    )
    .await;
    assert_eq!(returned_ids(&body), vec!["p23-pi-alpha"]);
    let (_, body) = post_query(
        &client,
        &base_url,
        path,
        json!({"variables": [{"operation": "equals", "value": "rejected"}]}),
    )
    .await;
    assert_eq!(returned_ids(&body), vec!["p23-pi-beta"]);
    let (status, body) = post_query(
        &client,
        &base_url,
        path,
        json!({"variables": [{"operation": "notEquals", "value": "x"}]}),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::BAD_REQUEST);
    assert!(
        body["details"].as_str().unwrap().contains(
            "Value-only query (without a variable-name) is only supported when using 'equals' operation."
        ),
        "details were: {}",
        body["details"]
    );

    // includeProcessVariablesNames only attaches the requested variables.
    let (_, body) = post_query(
        &client,
        &base_url,
        path,
        json!({"processInstanceId": "p23-pi-alpha", "includeProcessVariablesNames": ["route"]}),
    )
    .await;
    let variables = body["data"][0]["variables"].as_array().unwrap();
    assert_eq!(variables.len(), 1);
    assert_eq!(variables[0]["name"], "route");

    // startedBy: the engine never records a start user id, so a filter can
    // never match (documented data limitation, Java would match START_USER_ID_).
    let (_, body) = post_query(&client, &base_url, path, json!({"startedBy": "admin"})).await;
    assert_eq!(body["total"], 0);
    // finishedBy / parentCaseInstanceId: no end-user / CMMN data exists in the
    // BPMN engine, both filters are honest empty matches.
    let (_, body) = post_query(&client, &base_url, path, json!({"finishedBy": "admin"})).await;
    assert_eq!(body["total"], 0);
    let (_, body) = post_query(
        &client,
        &base_url,
        path,
        json!({"parentCaseInstanceId": "case-1"}),
    )
    .await;
    assert_eq!(body["total"], 0);

    // Unknown fields stay hard 400 (deny_unknown_fields).
    let (status, _) = post_query(&client, &base_url, path, json!({"bogusParam": 1})).await;
    assert_eq!(status, reqwest::StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn historic_process_instance_query_runtime_join_parameters() {
    let (engine, base_url, client) = spawn_server("rest-p23-pi-query-runtime").await;
    deploy(&client, &base_url, "P23 Child", CHILD_BPMN).await;
    deploy(&client, &base_url, "P23 Parent", PARENT_BPMN).await;

    let parent_pi = start_process_by_key(&engine, &client, &base_url, "p23Parent").await;
    let path = "/query/historic-process-instances";

    let (_, body) = post_query(&client, &base_url, path, json!({})).await;
    assert_eq!(body["total"], 2);
    let child_pi = returned_ids(&body)
        .into_iter()
        .find(|id| id != &parent_pi)
        .unwrap();

    // superProcessInstanceId resolves through the call activity's super
    // execution (runtime join, only available while the child is running).
    let (status, body) = post_query(
        &client,
        &base_url,
        path,
        json!({"superProcessInstanceId": parent_pi}),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert_eq!(returned_ids(&body), vec![child_pi.clone()]);

    // excludeSubprocesses removes call activity children.
    let (_, body) = post_query(
        &client,
        &base_url,
        path,
        json!({"excludeSubprocesses": true}),
    )
    .await;
    assert_eq!(returned_ids(&body), vec![parent_pi.clone()]);

    // activeActivityId / activeActivityIds match unfinished historic
    // activities.
    let (_, body) = post_query(
        &client,
        &base_url,
        path,
        json!({"activeActivityId": "childTask"}),
    )
    .await;
    assert_eq!(returned_ids(&body), vec![child_pi.clone()]);
    let (_, body) = post_query(
        &client,
        &base_url,
        path,
        json!({"activeActivityIds": ["childTask", "unknown"]}),
    )
    .await;
    assert_eq!(returned_ids(&body), vec![child_pi.clone()]);

    // involvedUser matches historic process-instance identity links
    // (Java HistoricProcessInstance.xml → ACT_HI_IDENTITYLINK; P77).
    // Use IdentityLinkService so the AUDIT historic mirror is written.
    engine.get_identity_link_service().add_identity_link(
        flowable_engine::identity::entities::IdentityLink {
            id: "p23-involved-link".to_string(),
            link_type: "participant".to_string(),
            user_id: Some("kermit".to_string()),
            group_id: None,
            task_id: None,
            process_instance_id: Some(parent_pi.clone()),
            process_definition_id: None,
        },
    );
    let (_, body) = post_query(&client, &base_url, path, json!({"involvedUser": "kermit"})).await;
    assert_eq!(returned_ids(&body), vec![parent_pi.clone()]);

    // callbackId / callbackType / withoutCallbackId run against the runtime
    // join; no callbacks are set on these instances.
    let (_, body) = post_query(&client, &base_url, path, json!({"callbackId": "cb-1"})).await;
    assert_eq!(body["total"], 0);
    let (_, body) = post_query(
        &client,
        &base_url,
        path,
        json!({"callbackIds": ["cb-1", "cb-2"]}),
    )
    .await;
    assert_eq!(body["total"], 0);
    let (_, body) = post_query(&client, &base_url, path, json!({"callbackType": "cmmn"})).await;
    assert_eq!(body["total"], 0);
    let (_, body) = post_query(&client, &base_url, path, json!({"withoutCallbackId": true})).await;
    assert_eq!(body["total"], 2);

    // rootScopeId: the top-level instance is its own root. (Call activity
    // children do not inherit the parent root id in this engine —
    // documented data limitation.)
    let (_, body) = post_query(&client, &base_url, path, json!({"rootScopeId": parent_pi})).await;
    assert_eq!(returned_ids(&body), vec![parent_pi.clone()]);
    // parentScopeId resolves like superProcessInstanceId.
    let (_, body) = post_query(
        &client,
        &base_url,
        path,
        json!({"parentScopeId": parent_pi}),
    )
    .await;
    assert_eq!(returned_ids(&body), vec![child_pi.clone()]);

    // state=running covers both live instances; suspended matches none.
    let (_, body) = post_query(&client, &base_url, path, json!({"state": "running"})).await;
    assert_eq!(body["total"], 2);
    let (_, body) = post_query(&client, &base_url, path, json!({"state": "suspended"})).await;
    assert_eq!(body["total"], 0);
}

// ── B: historic-task-instances query parameters ──

#[tokio::test]
async fn historic_task_instance_query_java_parameter_names() {
    let (engine, base_url, client) = spawn_server("rest-p23-task-query").await;
    deploy(&client, &base_url, "P23 User Task", USER_TASK_BPMN).await;
    let definition = engine
        .get_repository_service()
        .latest_process_definition_by_key("p23UserTaskProcess", None)
        .unwrap()
        .unwrap();

    let start = base_time();
    seed_historic_process_instance(
        &engine,
        "p23-task-pi-finished",
        &definition.id,
        Some("BK-Alpha"),
        start,
        Some(start + Duration::minutes(30)),
        None,
    );
    seed_historic_process_instance(
        &engine,
        "p23-task-pi-open",
        &definition.id,
        Some("bk-beta"),
        start,
        None,
        None,
    );

    let alpha_created = start + Duration::minutes(1);
    let alpha_completed = start + Duration::minutes(9);
    seed_historic_task(
        &engine,
        "p23-task-alpha",
        "p23-task-pi-finished",
        Some(&definition.id),
        "review",
        "Alpha Review",
        Some("first pass review"),
        Some("kermit"),
        Some("fozzie"),
        Some(50),
        Some(start + Duration::days(1)),
        alpha_created,
        Some(alpha_completed),
        Some("completed"),
    );
    let beta_created = start + Duration::minutes(5);
    seed_historic_task(
        &engine,
        "p23-task-beta",
        "p23-task-pi-open",
        Some(&definition.id),
        "approve",
        "beta approval",
        None,
        Some("gonzo"),
        None,
        Some(80),
        None,
        beta_created,
        None,
        None,
    );
    seed_historic_variable(
        &engine,
        "p23-task-pi-finished",
        Some("p23-task-alpha"),
        "approval",
        json!("granted"),
    );
    seed_historic_variable(&engine, "p23-task-pi-open", None, "route", json!("beta"));

    let path = "/query/historic-task-instances";

    // Java task* prefixed names plus the legacy aliases stay accepted.
    for body in [
        json!({"taskAssignee": "kermit"}),
        json!({"assignee": "kermit"}),
        json!({"taskAssigneeLike": "ker%"}),
    ] {
        let (status, result) = post_query(&client, &base_url, path, body.clone()).await;
        assert_eq!(status, reqwest::StatusCode::OK, "body {body}");
        assert_eq!(returned_ids(&result), vec!["p23-task-alpha"], "body {body}");
    }
    for body in [
        json!({"taskOwner": "fozzie"}),
        json!({"owner": "fozzie"}),
        json!({"taskOwnerLike": "foz%"}),
    ] {
        let (_, result) = post_query(&client, &base_url, path, body.clone()).await;
        assert_eq!(returned_ids(&result), vec!["p23-task-alpha"], "body {body}");
    }

    // taskPriority / taskMinPriority / taskMaxPriority (aliases minimum/
    // maximumPriority preserved).
    let (_, body) = post_query(&client, &base_url, path, json!({"taskPriority": 80})).await;
    assert_eq!(returned_ids(&body), vec!["p23-task-beta"]);
    let (_, body) = post_query(&client, &base_url, path, json!({"taskMinPriority": 60})).await;
    assert_eq!(returned_ids(&body), vec!["p23-task-beta"]);
    let (_, body) = post_query(&client, &base_url, path, json!({"taskMaxPriority": 60})).await;
    assert_eq!(returned_ids(&body), vec!["p23-task-alpha"]);
    let (_, body) = post_query(&client, &base_url, path, json!({"minimumPriority": 60})).await;
    assert_eq!(returned_ids(&body), vec!["p23-task-beta"]);

    // dueDateBefore / dueDateAfter (aliases dueBefore / dueAfter preserved) and
    // withoutDueDate.
    let due_probe = millis(start + Duration::days(2));
    let (_, body) = post_query(
        &client,
        &base_url,
        path,
        json!({"dueDateBefore": due_probe}),
    )
    .await;
    assert_eq!(returned_ids(&body), vec!["p23-task-alpha"]);
    let (_, body) = post_query(
        &client,
        &base_url,
        path,
        json!({"dueDateAfter": millis(start)}),
    )
    .await;
    assert_eq!(returned_ids(&body), vec!["p23-task-alpha"]);
    let (_, body) = post_query(&client, &base_url, path, json!({"withoutDueDate": true})).await;
    assert_eq!(returned_ids(&body), vec!["p23-task-beta"]);

    // taskCreatedOn / Before / After on the create (start) time.
    let (_, body) = post_query(
        &client,
        &base_url,
        path,
        json!({"taskCreatedOn": millis(alpha_created)}),
    )
    .await;
    assert_eq!(returned_ids(&body), vec!["p23-task-alpha"]);
    let (_, body) = post_query(
        &client,
        &base_url,
        path,
        json!({"taskCreatedBefore": millis(start + Duration::minutes(2))}),
    )
    .await;
    assert_eq!(returned_ids(&body), vec!["p23-task-alpha"]);
    let (_, body) = post_query(
        &client,
        &base_url,
        path,
        json!({"taskCreatedAfter": millis(start + Duration::minutes(2))}),
    )
    .await;
    assert_eq!(returned_ids(&body), vec!["p23-task-beta"]);

    // taskCompletedOn / Before / After on the end time.
    let (_, body) = post_query(
        &client,
        &base_url,
        path,
        json!({"taskCompletedOn": millis(alpha_completed)}),
    )
    .await;
    assert_eq!(returned_ids(&body), vec!["p23-task-alpha"]);
    let (_, body) = post_query(
        &client,
        &base_url,
        path,
        json!({"taskCompletedBefore": millis(start + Duration::minutes(10))}),
    )
    .await;
    assert_eq!(returned_ids(&body), vec!["p23-task-alpha"]);
    let (_, body) = post_query(
        &client,
        &base_url,
        path,
        json!({"taskCompletedAfter": millis(start)}),
    )
    .await;
    assert_eq!(returned_ids(&body), vec!["p23-task-alpha"]);

    // taskDeleteReason / Like / withoutDeleteReason.
    let (_, body) = post_query(
        &client,
        &base_url,
        path,
        json!({"taskDeleteReason": "completed"}),
    )
    .await;
    assert_eq!(returned_ids(&body), vec!["p23-task-alpha"]);
    let (_, body) = post_query(
        &client,
        &base_url,
        path,
        json!({"taskDeleteReasonLike": "comp%"}),
    )
    .await;
    assert_eq!(returned_ids(&body), vec!["p23-task-alpha"]);
    let (_, body) = post_query(
        &client,
        &base_url,
        path,
        json!({"withoutDeleteReason": true}),
    )
    .await;
    assert_eq!(returned_ids(&body), vec!["p23-task-beta"]);

    // taskDescription / Like and taskNameLikeIgnoreCase.
    let (_, body) = post_query(
        &client,
        &base_url,
        path,
        json!({"taskDescription": "first pass review"}),
    )
    .await;
    assert_eq!(returned_ids(&body), vec!["p23-task-alpha"]);
    let (_, body) = post_query(
        &client,
        &base_url,
        path,
        json!({"taskDescriptionLike": "first%"}),
    )
    .await;
    assert_eq!(returned_ids(&body), vec!["p23-task-alpha"]);
    let (_, body) = post_query(
        &client,
        &base_url,
        path,
        json!({"taskNameLikeIgnoreCase": "alpha%"}),
    )
    .await;
    assert_eq!(returned_ids(&body), vec!["p23-task-alpha"]);

    // taskDefinitionKeys and executionId.
    let (_, body) = post_query(
        &client,
        &base_url,
        path,
        json!({"taskDefinitionKeys": ["approve", "missing"]}),
    )
    .await;
    assert_eq!(returned_ids(&body), vec!["p23-task-beta"]);
    let (_, body) = post_query(
        &client,
        &base_url,
        path,
        json!({"executionId": "p23-task-alpha-execution"}),
    )
    .await;
    assert_eq!(returned_ids(&body), vec!["p23-task-alpha"]);

    // processFinished joins the historic process instance end time.
    let (_, body) = post_query(&client, &base_url, path, json!({"processFinished": true})).await;
    assert_eq!(returned_ids(&body), vec!["p23-task-alpha"]);
    let (_, body) = post_query(&client, &base_url, path, json!({"processFinished": false})).await;
    assert_eq!(returned_ids(&body), vec!["p23-task-beta"]);

    // processBusinessKey / Like and process definition metadata joins.
    let (_, body) = post_query(
        &client,
        &base_url,
        path,
        json!({"processBusinessKey": "BK-Alpha"}),
    )
    .await;
    assert_eq!(returned_ids(&body), vec!["p23-task-alpha"]);
    let (_, body) = post_query(
        &client,
        &base_url,
        path,
        json!({"processBusinessKeyLike": "bk-%"}),
    )
    .await;
    assert_eq!(returned_ids(&body), vec!["p23-task-beta"]);
    let (_, body) = post_query(
        &client,
        &base_url,
        path,
        json!({"processDefinitionKey": "p23UserTaskProcess"}),
    )
    .await;
    assert_eq!(body["total"], 2);
    let (_, body) = post_query(
        &client,
        &base_url,
        path,
        json!({"processDefinitionKeyLike": "p23User%"}),
    )
    .await;
    assert_eq!(body["total"], 2);
    let (_, body) = post_query(
        &client,
        &base_url,
        path,
        json!({"processDefinitionName": "P23 User Task"}),
    )
    .await;
    assert_eq!(body["total"], 2);
    let (_, body) = post_query(
        &client,
        &base_url,
        path,
        json!({"processDefinitionNameLike": "%User%"}),
    )
    .await;
    assert_eq!(body["total"], 2);

    // withoutProcessInstanceId only keeps standalone tasks (none seeded here).
    let (_, body) = post_query(
        &client,
        &base_url,
        path,
        json!({"withoutProcessInstanceId": true}),
    )
    .await;
    assert_eq!(body["total"], 0);

    // taskCategory: this engine does not persist a historic task category, so
    // equality can never match while taskWithoutCategory keeps everything
    // (Java: CATEGORY_ IS NULL matches).
    let (_, body) = post_query(&client, &base_url, path, json!({"taskCategory": "x"})).await;
    assert_eq!(body["total"], 0);
    let (_, body) = post_query(
        &client,
        &base_url,
        path,
        json!({"taskCategoryIn": ["x", "y"]}),
    )
    .await;
    assert_eq!(body["total"], 0);
    // NOT IN with a NULL category excludes the row in SQL semantics.
    let (_, body) = post_query(
        &client,
        &base_url,
        path,
        json!({"taskCategoryNotIn": ["x"]}),
    )
    .await;
    assert_eq!(body["total"], 0);
    let (_, body) = post_query(
        &client,
        &base_url,
        path,
        json!({"taskWithoutCategory": true}),
    )
    .await;
    assert_eq!(body["total"], 2);

    // BPMN tasks carry no scope columns (Java stores NULL): equality filters
    // never match, withoutScopeId keeps all rows.
    let (_, body) = post_query(&client, &base_url, path, json!({"scopeId": "scope-1"})).await;
    assert_eq!(body["total"], 0);
    let (_, body) = post_query(
        &client,
        &base_url,
        path,
        json!({"scopeIds": ["scope-1", "scope-2"]}),
    )
    .await;
    assert_eq!(body["total"], 0);
    let (_, body) = post_query(&client, &base_url, path, json!({"scopeType": "cmmn"})).await;
    assert_eq!(body["total"], 0);
    let (_, body) = post_query(
        &client,
        &base_url,
        path,
        json!({"scopeDefinitionId": "sd-1"}),
    )
    .await;
    assert_eq!(body["total"], 0);
    let (_, body) = post_query(&client, &base_url, path, json!({"withoutScopeId": true})).await;
    assert_eq!(body["total"], 2);
    let (_, body) = post_query(
        &client,
        &base_url,
        path,
        json!({"propagatedStageInstanceId": "stage-1"}),
    )
    .await;
    assert_eq!(body["total"], 0);

    // parentTaskId / rootScopeId / parentScopeId: not recorded for historic
    // BPMN tasks — honest empty matches (documented data limitation).
    let (_, body) = post_query(&client, &base_url, path, json!({"parentTaskId": "p"})).await;
    assert_eq!(body["total"], 0);
    let (_, body) = post_query(&client, &base_url, path, json!({"rootScopeId": "r"})).await;
    assert_eq!(body["total"], 0);
    let (_, body) = post_query(&client, &base_url, path, json!({"parentScopeId": "p"})).await;
    assert_eq!(body["total"], 0);

    // ignoreTaskAssignee is accepted and wired to ignoreAssigneeValue
    // (HistoricTaskInstanceBaseResource.java:294-296 / P75a).
    let (status, _) = post_query(
        &client,
        &base_url,
        path,
        json!({"ignoreTaskAssignee": true}),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::OK);

    // taskVariables / processVariables: the historic task query rejects
    // value-only filters outright (HistoricTaskInstanceBaseResource), unlike
    // the process instance query.
    for field in ["taskVariables", "processVariables"] {
        let (status, body) = post_query(
            &client,
            &base_url,
            path,
            json!({field: [{"operation": "equals", "value": "granted"}]}),
        )
        .await;
        assert_eq!(status, reqwest::StatusCode::BAD_REQUEST, "field {field}");
        assert!(
            body["details"]
                .as_str()
                .unwrap()
                .contains("Value-only query (without a variable-name) is not supported."),
            "details were: {}",
            body["details"]
        );
    }
    let (_, body) = post_query(
        &client,
        &base_url,
        path,
        json!({"taskVariables": [{"name": "approval", "operation": "equals", "value": "granted"}]}),
    )
    .await;
    assert_eq!(returned_ids(&body), vec!["p23-task-alpha"]);
    // Java name processVariables (alias processInstanceVariables kept).
    for field in ["processVariables", "processInstanceVariables"] {
        let (_, body) = post_query(
            &client,
            &base_url,
            path,
            json!({field: [{"name": "route", "operation": "equals", "value": "beta"}]}),
        )
        .await;
        assert_eq!(returned_ids(&body), vec!["p23-task-beta"], "field {field}");
    }

    // GET surface accepts the Java parameter names too.
    let response = client
        .get(format!(
            "{base_url}/history/historic-task-instances?taskAssignee=kermit&taskMaxPriority=60"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let body: Value = response.json().await.unwrap();
    assert_eq!(returned_ids(&body), vec!["p23-task-alpha"]);

    // Unknown parameters remain a hard 400.
    let (status, _) = post_query(&client, &base_url, path, json!({"bogus": true})).await;
    assert_eq!(status, reqwest::StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn historic_task_instance_query_process_instance_id_with_children() {
    let (engine, base_url, client) = spawn_server("rest-p23-task-children").await;
    deploy(&client, &base_url, "P23 Child", CHILD_BPMN).await;
    deploy(&client, &base_url, "P23 Parent", PARENT_BPMN).await;
    let parent_pi = start_process_by_key(&engine, &client, &base_url, "p23Parent").await;

    // The child user task belongs to the call activity child instance; the
    // withChildren variant finds it starting from the parent id (runtime
    // join, running instances only).
    let (status, body) = post_query(
        &client,
        &base_url,
        "/query/historic-task-instances",
        json!({"processInstanceIdWithChildren": parent_pi}),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert_eq!(body["total"], 1);
    assert_eq!(body["data"][0]["name"], "Child Task");

    let (_, body) = post_query(
        &client,
        &base_url,
        "/query/historic-task-instances",
        json!({"processInstanceId": parent_pi}),
    )
    .await;
    assert_eq!(body["total"], 0);

    // taskInvolvedUser matches task identity links.
    let task_id = {
        let (_, body) = post_query(
            &client,
            &base_url,
            "/query/historic-task-instances",
            json!({}),
        )
        .await;
        body["data"][0]["id"].as_str().unwrap().to_string()
    };
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    store.insert_identity_link(
        flowable_engine::identity::entities::IdentityLink {
            id: "p23-task-involved-link".to_string(),
            link_type: "participant".to_string(),
            user_id: Some("scooter".to_string()),
            group_id: None,
            task_id: Some(task_id.clone()),
            process_instance_id: None,
            process_definition_id: None,
        },
        &mut session,
    );
    session.flush_and_commit().unwrap();
    let (_, body) = post_query(
        &client,
        &base_url,
        "/query/historic-task-instances",
        json!({"taskInvolvedUser": "scooter"}),
    )
    .await;
    assert_eq!(returned_ids(&body), vec![task_id]);
    let (_, body) = post_query(
        &client,
        &base_url,
        "/query/historic-task-instances",
        json!({"taskInvolvedUser": "nobody"}),
    )
    .await;
    assert_eq!(body["total"], 0);
}

// ── C: historic-activity-instances query parameters ──

#[tokio::test]
async fn historic_activity_instance_query_added_parameters() {
    let (engine, base_url, client) = spawn_server("rest-p23-activity-query").await;
    deploy(&client, &base_url, "P23 User Task", USER_TASK_BPMN).await;
    let definition = engine
        .get_repository_service()
        .latest_process_definition_by_key("p23UserTaskProcess", None)
        .unwrap()
        .unwrap();

    seed_historic_process_instance(
        &engine,
        "p23-act-pi-1",
        &definition.id,
        None,
        base_time(),
        None,
        None,
    );
    seed_historic_process_instance(
        &engine,
        "p23-act-pi-2",
        "missing-definition:1:x",
        None,
        base_time(),
        None,
        None,
    );
    seed_historic_activity(
        &engine,
        "p23-act-a",
        "p23-act-pi-1",
        "review",
        "userTask",
        Some("kermit"),
        None,
    );
    seed_historic_activity(
        &engine,
        "p23-act-b",
        "p23-act-pi-2",
        "approve",
        "userTask",
        None,
        Some(base_time() + Duration::minutes(1)),
    );

    let path = "/query/historic-activity-instances";

    // processInstanceIds (applied when non-empty).
    let (status, body) = post_query(
        &client,
        &base_url,
        path,
        json!({"processInstanceIds": ["p23-act-pi-1"]}),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert_eq!(returned_ids(&body), vec!["p23-act-a"]);

    // processDefinitionId joins the historic process instance.
    let (_, body) = post_query(
        &client,
        &base_url,
        path,
        json!({"processDefinitionId": definition.id}),
    )
    .await;
    assert_eq!(returned_ids(&body), vec!["p23-act-a"]);

    // taskAssignee.
    let (_, body) = post_query(&client, &base_url, path, json!({"taskAssignee": "kermit"})).await;
    assert_eq!(returned_ids(&body), vec!["p23-act-a"]);

    // tenantId / tenantIdLike never match without tenants; withoutTenantId
    // (only Boolean.TRUE applies, Java base resource) keeps everything.
    let (_, body) = post_query(&client, &base_url, path, json!({"tenantId": "acme"})).await;
    assert_eq!(body["total"], 0);
    let (_, body) = post_query(&client, &base_url, path, json!({"tenantIdLike": "ac%"})).await;
    assert_eq!(body["total"], 0);
    let (_, body) = post_query(&client, &base_url, path, json!({"withoutTenantId": true})).await;
    assert_eq!(body["total"], 2);
    let (_, body) = post_query(&client, &base_url, path, json!({"withoutTenantId": false})).await;
    assert_eq!(body["total"], 2);

    // calledProcessInstanceIds: the engine does not record the called process
    // instance id on historic activities — honest empty match (documented
    // data limitation).
    let (_, body) = post_query(
        &client,
        &base_url,
        path,
        json!({"calledProcessInstanceIds": ["p23-act-pi-1"]}),
    )
    .await;
    assert_eq!(body["total"], 0);

    // Unknown parameters remain a hard 400.
    let (status, _) = post_query(&client, &base_url, path, json!({"nope": 1})).await;
    assert_eq!(status, reqwest::StatusCode::BAD_REQUEST);
}
