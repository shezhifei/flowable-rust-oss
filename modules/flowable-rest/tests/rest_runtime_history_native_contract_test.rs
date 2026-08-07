use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_rest::run_server;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::net::TcpListener;

async fn spawn_server(test_name: &str) -> (Arc<ProcessEngine>, String, reqwest::Client) {
    let engine = Arc::new(ProcessEngine::new(test_name.to_string()));

    let user = flowable_engine::identity::entities::User {
        id: "admin".to_string(),
        first_name: None,
        last_name: None,
        email: None,
        password: Some("test".to_string()),
        tenant_id: None,
    };
    engine.get_identity_service().save_user(user);

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let base_url = format!("http://{}", addr);

    let engine_clone = Arc::clone(&engine);
    tokio::spawn(async move {
        run_server(engine_clone, listener).await.unwrap();
    });

    (engine, base_url, reqwest::Client::new())
}

async fn deploy_process(client: &reqwest::Client, base_url: &str) {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="runtimeHistoryContractProcess" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="f1" sourceRef="start" targetRef="task1" />
            <userTask id="task1" name="Contract Task" />
            <sequenceFlow id="f2" sourceRef="task1" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#;

    let response = client
        .post(format!("{}/repository/deployments", base_url))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "Runtime History Contract Deployment",
            "resourceName": "runtime_history_contract.bpmn20.xml",
            "resource": xml
        }))
        .send()
        .await
        .unwrap();

    assert!(response.status().is_success());

    let _: Value = response.json().await.unwrap();
}

async fn start_process(
    client: &reqwest::Client,
    base_url: &str,
    process_definition_id: &str,
    business_key: &str,
) -> Value {
    let response = client
        .post(format!("{}/runtime/process-instances", base_url))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "processDefinitionId": process_definition_id,
            "businessKey": business_key
        }))
        .send()
        .await
        .unwrap();

    assert!(response.status().is_success());
    response.json().await.unwrap()
}

async fn complete_first_task(client: &reqwest::Client, base_url: &str, process_instance_id: &str) {
    let tasks = client
        .get(format!(
            "{}/runtime/tasks?processInstanceId={}&start=0&size=10",
            base_url, process_instance_id
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert!(tasks.status().is_success());
    let tasks_body: Value = tasks.json().await.unwrap();
    let task_id = tasks_body["data"][0]["id"].as_str().unwrap();

    let complete = client
        .post(format!("{}/runtime/tasks/{}/complete", base_url, task_id))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "action": "complete" }))
        .send()
        .await
        .unwrap();

    assert!(complete.status().is_success());
}

async fn first_task_id(
    client: &reqwest::Client,
    base_url: &str,
    process_instance_id: &str,
) -> String {
    let tasks = client
        .get(format!(
            "{}/runtime/tasks?processInstanceId={}&start=0&size=10",
            base_url, process_instance_id
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert!(tasks.status().is_success());
    let tasks_body: Value = tasks.json().await.unwrap();
    tasks_body["data"][0]["id"].as_str().unwrap().to_string()
}

async fn first_task_execution_id(
    client: &reqwest::Client,
    base_url: &str,
    process_instance_id: &str,
) -> String {
    let tasks = client
        .get(format!(
            "{}/runtime/tasks?processInstanceId={}&start=0&size=10",
            base_url, process_instance_id
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert!(tasks.status().is_success());
    let tasks_body: Value = tasks.json().await.unwrap();
    tasks_body["data"][0]["executionId"]
        .as_str()
        .unwrap()
        .to_string()
}

#[tokio::test]
async fn runtime_and_history_owned_routes_follow_m11_contract() {
    let (engine, base_url, client) = spawn_server("rest-runtime-history-contract").await;
    deploy_process(&client, &base_url).await;
    let process_definition_id = engine
        .get_repository_service()
        .get_process_definition_ids()
        .unwrap()[0]
        .clone();

    let started_one =
        start_process(&client, &base_url, &process_definition_id, "contract-one").await;
    let started_two =
        start_process(&client, &base_url, &process_definition_id, "contract-two").await;

    let process_instances = client
        .get(format!(
            "{}/runtime/process-instances?start=1&size=1",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert!(process_instances.status().is_success());
    let process_instances_body: Value = process_instances.json().await.unwrap();
    assert_eq!(process_instances_body["start"], 1);
    assert_eq!(process_instances_body["size"], 1);
    assert_eq!(process_instances_body["total"], 2);
    assert_eq!(process_instances_body["data"].as_array().unwrap().len(), 1);

    let tasks = client
        .get(format!(
            "{}/runtime/tasks?processInstanceId={}&start=0&size=10",
            base_url,
            started_one["id"].as_str().unwrap()
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert!(tasks.status().is_success());
    let tasks_body: Value = tasks.json().await.unwrap();
    assert_eq!(tasks_body["start"], 0);
    assert_eq!(tasks_body["size"], 1);
    assert_eq!(tasks_body["total"], 1);
    let task = &tasks_body["data"][0];
    assert_eq!(task["processInstanceId"], started_one["id"]);
    let task_id = task["id"].as_str().unwrap();

    let complete = client
        .post(format!("{}/runtime/tasks/{}/complete", base_url, task_id))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "action": "complete" }))
        .send()
        .await
        .unwrap();

    assert!(complete.status().is_success());

    let history = client
        .get(format!(
            "{}/history/historic-process-instances?start=0&size=10",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert!(history.status().is_success());
    let history_body: Value = history.json().await.unwrap();
    assert_eq!(history_body["start"], 0);
    assert_eq!(history_body["total"], 2);
    assert_eq!(history_body["size"], 2);
    let historic_instances = history_body["data"].as_array().unwrap();
    assert_eq!(historic_instances.len(), 2);

    let completed = historic_instances
        .iter()
        .find(|instance| instance["id"] == started_one["id"])
        .expect("completed instance should be present in historic collection");
    assert!(completed["endTime"].is_string());

    let running = historic_instances
        .iter()
        .find(|instance| instance["id"] == started_two["id"])
        .expect("active instance should still be present in historic collection");
    assert!(running["endTime"].is_null());

    let second_tasks = client
        .get(format!(
            "{}/runtime/tasks?processInstanceId={}&start=0&size=10",
            base_url,
            started_two["id"].as_str().unwrap()
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert!(second_tasks.status().is_success());
    let second_tasks_body: Value = second_tasks.json().await.unwrap();
    assert_eq!(second_tasks_body["total"], 1);
    assert_eq!(
        second_tasks_body["data"][0]["processInstanceId"],
        started_two["id"]
    );
}

#[tokio::test]
async fn historic_variable_instances_match_filter_aliases_sorting_and_errors() {
    let (engine, base_url, client) =
        spawn_server("rest-bpmn-historic-variable-query-contract").await;
    deploy_process(&client, &base_url).await;
    let process_definition_id = engine
        .get_repository_service()
        .get_process_definition_ids()
        .unwrap()[0]
        .clone();

    let alpha = start_process(
        &client,
        &base_url,
        &process_definition_id,
        "historic-variable-alpha",
    )
    .await;
    let beta = start_process(
        &client,
        &base_url,
        &process_definition_id,
        "historic-variable-beta",
    )
    .await;
    let alpha_id = alpha["id"].as_str().unwrap();
    let beta_id = beta["id"].as_str().unwrap();
    let alpha_execution_id = first_task_execution_id(&client, &base_url, alpha_id).await;
    let beta_execution_id = first_task_execution_id(&client, &base_url, beta_id).await;

    engine
        .get_variable_service()
        .set_variable(
            alpha_execution_id.clone(),
            "approvalAlpha".to_string(),
            json!("Accepted"),
        )
        .unwrap();
    engine
        .get_variable_service()
        .set_variable(alpha_execution_id.clone(), "count".to_string(), json!(7))
        .unwrap();
    engine
        .get_variable_service()
        .set_variable(
            beta_execution_id.clone(),
            "approvalBeta".to_string(),
            json!("Rejected"),
        )
        .unwrap();

    let get_filtered = client
        .get(format!(
            "{base_url}/history/historic-variable-instances?variableNameLike=approval%25&variableType=string&sort=variableName&order=asc"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(get_filtered.status(), reqwest::StatusCode::OK);
    let get_body: Value = get_filtered.json().await.unwrap();
    assert_eq!(get_body["total"], 2, "body was: {get_body}");
    assert_eq!(get_body["data"][0]["name"], "approvalAlpha");
    assert_eq!(get_body["data"][0]["variableType"], "string");
    assert_eq!(get_body["data"][0]["processInstanceId"], alpha_id);
    assert_eq!(get_body["data"][0]["executionId"], alpha_execution_id);
    assert_eq!(get_body["data"][1]["name"], "approvalBeta");
    assert_eq!(get_body["data"][1]["processInstanceId"], beta_id);
    assert_eq!(get_body["data"][1]["executionId"], beta_execution_id);

    let post_filtered = client
        .post(format!(
            "{base_url}/query/historic-variable-instances?start=0&size=1"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "approvalBeta",
            "type": "string",
            "executionId": beta_execution_id,
            "sort": "name",
            "order": "desc",
            "start": 99,
            "size": 99
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(post_filtered.status(), reqwest::StatusCode::OK);
    let post_body: Value = post_filtered.json().await.unwrap();
    assert_eq!(post_body["start"], 0);
    assert_eq!(post_body["size"], 1);
    assert_eq!(post_body["total"], 1);
    assert_eq!(post_body["data"][0]["name"], "approvalBeta");
    assert_eq!(post_body["data"][0]["variableType"], "string");

    let bad_sort = client
        .get(format!(
            "{base_url}/history/historic-variable-instances?sort=unsupportedHistoricVariableSort"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(bad_sort.status(), reqwest::StatusCode::BAD_REQUEST);
    let bad_sort_body: Value = bad_sort.json().await.unwrap();
    assert_eq!(bad_sort_body["code"], "BAD_REQUEST");
    assert_eq!(bad_sort_body["message"], "Bad Request");
    assert!(
        bad_sort_body["details"]
            .as_str()
            .unwrap()
            .contains("unsupportedHistoricVariableSort"),
        "details were: {}",
        bad_sort_body["details"]
    );

    let bad_order = client
        .post(format!("{base_url}/query/historic-variable-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "order": "sideways"
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(bad_order.status(), reqwest::StatusCode::BAD_REQUEST);
    let bad_order_body: Value = bad_order.json().await.unwrap();
    assert_eq!(bad_order_body["code"], "BAD_REQUEST");
    assert_eq!(bad_order_body["message"], "Bad Request");
    assert!(
        bad_order_body["details"]
            .as_str()
            .unwrap()
            .contains("sideways"),
        "details were: {}",
        bad_order_body["details"]
    );
}

#[tokio::test]
async fn historic_task_query_url_sort_and_order_override_body() {
    let (engine, base_url, client) =
        spawn_server("rest-historic-task-url-sort-order-override").await;
    deploy_process(&client, &base_url).await;
    let process_definition_id = engine
        .get_repository_service()
        .get_process_definition_ids()
        .unwrap()[0]
        .clone();

    start_process(&client, &base_url, &process_definition_id, "task-sort-one").await;
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    start_process(&client, &base_url, &process_definition_id, "task-sort-two").await;

    let expected_desc = client
        .get(format!(
            "{base_url}/history/historic-task-instances?sort=startTime&order=desc&start=0&size=1"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(expected_desc.status(), reqwest::StatusCode::OK);
    let expected_desc_body: Value = expected_desc.json().await.unwrap();
    assert_eq!(expected_desc_body["total"], 2);
    let expected_first_id = expected_desc_body["data"][0]["id"].clone();

    let response = client
        .post(format!(
            "{base_url}/query/historic-task-instances?sort=startTime&order=desc&start=0&size=1"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "sort": "startTime",
            "order": "asc",
            "start": 99,
            "size": 99
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["start"], 0);
    assert_eq!(body["size"], 1);
    assert_eq!(body["total"], 2);
    assert_eq!(body["data"][0]["id"], expected_first_id);
}

#[tokio::test]
async fn historic_process_and_task_queries_include_variables_after_sorting_and_paging() {
    let (engine, base_url, client) = spawn_server("rest-historic-query-include-variables").await;
    deploy_process(&client, &base_url).await;
    let process_definition_id = engine
        .get_repository_service()
        .get_process_definition_ids()
        .unwrap()[0]
        .clone();

    let alpha = start_process(
        &client,
        &base_url,
        &process_definition_id,
        "include-vars-alpha",
    )
    .await;
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    let beta = start_process(
        &client,
        &base_url,
        &process_definition_id,
        "include-vars-beta",
    )
    .await;
    let alpha_id = alpha["id"].as_str().unwrap();
    let beta_id = beta["id"].as_str().unwrap();
    let beta_task_id = first_task_id(&client, &base_url, beta_id).await;

    engine
        .get_variable_service()
        .set_variable(alpha_id.to_string(), "stage".to_string(), json!("alpha"))
        .unwrap();
    engine
        .get_variable_service()
        .set_variable(beta_id.to_string(), "stage".to_string(), json!("beta"))
        .unwrap();
    engine
        .get_task_service()
        .set_task_local_variable(
            beta_task_id.clone(),
            "decision".to_string(),
            json!("approved"),
        )
        .unwrap();

    let process_get = client
        .get(format!(
            "{base_url}/history/historic-process-instances?includeProcessVariables=true&sort=businessKey&order=desc&start=0&size=1"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(process_get.status(), reqwest::StatusCode::OK);
    let process_get_body: Value = process_get.json().await.unwrap();
    assert_eq!(process_get_body["total"], 2);
    assert_eq!(process_get_body["size"], 1);
    assert_eq!(process_get_body["data"][0]["id"], beta_id);
    assert_eq!(
        process_get_body["data"][0]["variables"],
        json!([{
            "name": "stage",
            "type": "string",
            "value": "beta",
            "scope": "global"
        }])
    );

    let process_post = client
        .post(format!(
            "{base_url}/query/historic-process-instances?includeProcessVariables=true&sort=businessKey&order=asc&start=0&size=1"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "includeProcessVariables": false,
            "sort": "businessKey",
            "order": "desc",
            "start": 99,
            "size": 99
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(process_post.status(), reqwest::StatusCode::OK);
    let process_post_body: Value = process_post.json().await.unwrap();
    assert_eq!(process_post_body["start"], 0);
    assert_eq!(process_post_body["size"], 1);
    assert_eq!(process_post_body["data"][0]["id"], alpha_id);
    assert_eq!(
        process_post_body["data"][0]["variables"],
        json!([{
            "name": "stage",
            "type": "string",
            "value": "alpha",
            "scope": "global"
        }])
    );

    let task_get = client
        .get(format!(
            "{base_url}/history/historic-task-instances?taskId={beta_task_id}&includeTaskLocalVariables=true"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(task_get.status(), reqwest::StatusCode::OK);
    let task_get_body: Value = task_get.json().await.unwrap();
    assert_eq!(task_get_body["total"], 1);
    assert_eq!(
        task_get_body["data"][0]["variables"],
        json!([{
            "name": "decision",
            "type": "string",
            "value": "approved",
            "scope": "local"
        }])
    );

    let task_post = client
        .post(format!(
            "{base_url}/query/historic-task-instances?includeProcessVariables=true&includeTaskLocalVariables=true&sort=startTime&order=desc&start=0&size=1"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "includeProcessVariables": false,
            "includeTaskLocalVariables": false,
            "sort": "startTime",
            "order": "asc",
            "start": 99,
            "size": 99
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(task_post.status(), reqwest::StatusCode::OK);
    let task_post_body: Value = task_post.json().await.unwrap();
    assert_eq!(task_post_body["start"], 0);
    assert_eq!(task_post_body["size"], 1);
    assert_eq!(task_post_body["data"][0]["id"], beta_task_id);
    assert_eq!(
        task_post_body["data"][0]["variables"],
        json!([
            {
                "name": "decision",
                "type": "string",
                "value": "approved",
                "scope": "local"
            },
            {
                "name": "stage",
                "type": "string",
                "value": "beta",
                "scope": "global"
            }
        ])
    );
}

#[tokio::test]
async fn historic_task_query_filters_by_task_local_and_process_variables() {
    let (engine, base_url, client) = spawn_server("rest-historic-task-variable-filters").await;
    deploy_process(&client, &base_url).await;
    let process_definition_id = engine
        .get_repository_service()
        .get_process_definition_ids()
        .unwrap()[0]
        .clone();

    let alpha = start_process(
        &client,
        &base_url,
        &process_definition_id,
        "historic-task-var-alpha",
    )
    .await;
    let beta = start_process(
        &client,
        &base_url,
        &process_definition_id,
        "historic-task-var-beta",
    )
    .await;
    let alpha_id = alpha["id"].as_str().unwrap();
    let beta_id = beta["id"].as_str().unwrap();
    let alpha_task_id = first_task_id(&client, &base_url, alpha_id).await;
    let beta_task_id = first_task_id(&client, &base_url, beta_id).await;
    let alpha_execution_id = first_task_execution_id(&client, &base_url, alpha_id).await;
    let beta_execution_id = first_task_execution_id(&client, &base_url, beta_id).await;

    engine
        .get_task_service()
        .set_task_local_variable(
            alpha_task_id.clone(),
            "approval".to_string(),
            json!("Accepted"),
        )
        .unwrap();
    engine
        .get_task_service()
        .set_task_local_variable(beta_task_id, "approval".to_string(), json!("Rejected"))
        .unwrap();
    engine
        .get_variable_service()
        .set_variable(alpha_execution_id, "route".to_string(), json!("AlphaRoute"))
        .unwrap();
    engine
        .get_variable_service()
        .set_variable(beta_execution_id, "route".to_string(), json!("BetaRoute"))
        .unwrap();

    let response = client
        .post(format!("{base_url}/query/historic-task-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "taskVariables": [{
                "name": "approval",
                "operation": "equalsIgnoreCase",
                "value": "accepted"
            }],
            "processInstanceVariables": [{
                "name": "route",
                "operation": "notEquals",
                "value": "BetaRoute"
            }],
            "includeTaskLocalVariables": true,
            "includeProcessVariables": true
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["total"], 1, "body was: {body}");
    assert_eq!(body["data"][0]["id"], alpha_task_id);
    assert_eq!(
        body["data"][0]["variables"],
        json!([
            {
                "name": "approval",
                "type": "string",
                "value": "Accepted",
                "scope": "local"
            },
            {
                "name": "route",
                "type": "string",
                "value": "AlphaRoute",
                "scope": "global"
            }
        ])
    );
}

#[tokio::test]
async fn historic_activity_instances_match_filters_sorts_and_errors() {
    let (engine, base_url, client) =
        spawn_server("rest-bpmn-historic-activity-query-contract").await;
    deploy_process(&client, &base_url).await;
    let process_definition_id = engine
        .get_repository_service()
        .get_process_definition_ids()
        .unwrap()[0]
        .clone();

    let alpha = start_process(
        &client,
        &base_url,
        &process_definition_id,
        "historic-activity-alpha",
    )
    .await;
    let beta = start_process(
        &client,
        &base_url,
        &process_definition_id,
        "historic-activity-beta",
    )
    .await;
    let alpha_id = alpha["id"].as_str().unwrap();
    let beta_id = beta["id"].as_str().unwrap();
    complete_first_task(&client, &base_url, alpha_id).await;

    let get_filtered = client
        .get(format!(
            "{base_url}/history/historic-activity-instances?processInstanceId={alpha_id}&activityId=task1&activityType=userTask&finished=true&sort=activityType&order=asc"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(get_filtered.status(), reqwest::StatusCode::OK);
    let get_body: Value = get_filtered.json().await.unwrap();
    assert_eq!(get_body["total"], 1, "body was: {get_body}");
    assert_eq!(get_body["data"][0]["activityId"], "task1");
    assert_eq!(get_body["data"][0]["activityType"], "UserTask");
    assert_eq!(get_body["data"][0]["processInstanceId"], alpha_id);
    assert!(get_body["data"][0]["endTime"].is_string());
    let activity_instance_id = get_body["data"][0]["id"].as_str().unwrap();
    let execution_id = get_body["data"][0]["executionId"].as_str().unwrap();

    let post_filtered = client
        .post(format!(
            "{base_url}/query/historic-activity-instances?start=0&size=1"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "activityInstanceId": activity_instance_id,
            "executionId": execution_id,
            "sort": "durationInMillis",
            "order": "desc",
            "start": 99,
            "size": 99
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(post_filtered.status(), reqwest::StatusCode::OK);
    let post_body: Value = post_filtered.json().await.unwrap();
    assert_eq!(post_body["start"], 0);
    assert_eq!(post_body["size"], 1);
    assert_eq!(post_body["total"], 1);
    assert_eq!(post_body["data"][0]["id"], activity_instance_id);
    assert_eq!(post_body["data"][0]["executionId"], execution_id);

    let unfinished = client
        .get(format!(
            "{base_url}/history/historic-activity-instances?processInstanceId={beta_id}&activityId=task1&unfinished=true"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(unfinished.status(), reqwest::StatusCode::OK);
    let unfinished_body: Value = unfinished.json().await.unwrap();
    assert_eq!(unfinished_body["total"], 1);
    assert!(unfinished_body["data"][0]["endTime"].is_null());

    let bad_sort = client
        .get(format!(
            "{base_url}/history/historic-activity-instances?sort=unsupportedHistoricActivitySort"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(bad_sort.status(), reqwest::StatusCode::BAD_REQUEST);
    let bad_sort_body: Value = bad_sort.json().await.unwrap();
    assert_eq!(bad_sort_body["code"], "BAD_REQUEST");
    assert!(
        bad_sort_body["details"]
            .as_str()
            .unwrap()
            .contains("Unsupported historic activity sort field 'unsupportedHistoricActivitySort'"),
        "details were: {}",
        bad_sort_body["details"]
    );

    let bad_order = client
        .post(format!("{base_url}/query/historic-activity-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "sort": "activityId",
            "order": "sideways"
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(bad_order.status(), reqwest::StatusCode::BAD_REQUEST);
    let bad_order_body: Value = bad_order.json().await.unwrap();
    assert_eq!(bad_order_body["code"], "BAD_REQUEST");
    assert!(
        bad_order_body["details"]
            .as_str()
            .unwrap()
            .contains("Unsupported historic activity sort order 'sideways'"),
        "details were: {}",
        bad_order_body["details"]
    );
}

#[tokio::test]
async fn historic_process_query_matches_field_aliases_filters_and_sort_errors() {
    let (engine, base_url, client) =
        spawn_server("rest-bpmn-historic-process-query-contract").await;
    deploy_process(&client, &base_url).await;
    let process_definition_id = engine
        .get_repository_service()
        .get_process_definition_ids()
        .unwrap()[0]
        .clone();

    let finished_instance =
        start_process(&client, &base_url, &process_definition_id, "contract-one").await;
    tokio::time::sleep(std::time::Duration::from_millis(2)).await;
    let unfinished_instance =
        start_process(&client, &base_url, &process_definition_id, "contract-two").await;
    complete_first_task(
        &client,
        &base_url,
        finished_instance["id"].as_str().unwrap(),
    )
    .await;

    let get_filtered = client
        .get(format!(
            "{base_url}/history/historic-process-instances?processDefinitionKey=runtimeHistoryContractProcess&businessKey=contract-one&finished=true&startedAfter=0&finishedAfter=0&withoutTenantId=true&sort=startTime&order=desc"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(get_filtered.status(), reqwest::StatusCode::OK);
    let get_body: Value = get_filtered.json().await.unwrap();
    assert_eq!(get_body["total"], 1);
    assert_eq!(get_body["data"][0]["id"], finished_instance["id"]);
    assert_eq!(
        get_body["data"][0]["processDefinitionId"],
        process_definition_id
    );
    assert_eq!(
        get_body["data"][0]["processDefinitionKey"],
        "runtimeHistoryContractProcess"
    );
    assert_eq!(get_body["data"][0]["businessKey"], "contract-one");
    assert!(get_body["data"][0]["endTime"].is_string());
    assert!(get_body["data"][0]["tenantId"].is_null());

    let get_like_unfinished = client
        .get(format!(
            "{base_url}/history/historic-process-instances?processDefinitionKeyLike=runtimeHistory%25&unfinished=true&businessKeyLike=contract-%25"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(get_like_unfinished.status(), reqwest::StatusCode::OK);
    let get_like_body: Value = get_like_unfinished.json().await.unwrap();
    assert_eq!(get_like_body["total"], 1);
    assert_eq!(get_like_body["data"][0]["id"], unfinished_instance["id"]);
    assert!(get_like_body["data"][0]["endTime"].is_null());

    let post_filtered = client
        .post(format!("{base_url}/query/historic-process-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "processDefinitionId": process_definition_id,
            "processBusinessKey": "contract-two",
            "finished": false,
            "startedBefore": "32503680000000",
            "sort": "businessKey",
            "order": "asc"
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(post_filtered.status(), reqwest::StatusCode::OK);
    let post_body: Value = post_filtered.json().await.unwrap();
    assert_eq!(post_body["total"], 1);
    assert_eq!(post_body["data"][0]["id"], unfinished_instance["id"]);
    assert_eq!(post_body["data"][0]["businessKey"], "contract-two");

    let post_alias_filtered = client
        .post(format!("{base_url}/query/historic-process-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "processInstanceBusinessKey": "contract-one",
            "finished": true
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(post_alias_filtered.status(), reqwest::StatusCode::OK);
    let post_alias_body: Value = post_alias_filtered.json().await.unwrap();
    assert_eq!(post_alias_body["total"], 1);
    assert_eq!(post_alias_body["data"][0]["id"], finished_instance["id"]);

    let bad_sort = client
        .post(format!("{base_url}/query/historic-process-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "sort": "unsupportedHistoricSort",
            "order": "asc"
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(bad_sort.status(), reqwest::StatusCode::BAD_REQUEST);
    let bad_sort_body: Value = bad_sort.json().await.unwrap();
    assert_eq!(bad_sort_body["code"], "BAD_REQUEST");
    assert_eq!(bad_sort_body["message"], "Bad Request");
    assert!(
        bad_sort_body["details"]
            .as_str()
            .unwrap()
            .contains("unsupportedHistoricSort"),
        "details were: {}",
        bad_sort_body["details"]
    );

    let bad_order = client
        .get(format!(
            "{base_url}/history/historic-process-instances?sort=processInstanceId&order=sideways"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(bad_order.status(), reqwest::StatusCode::BAD_REQUEST);
    let bad_order_body: Value = bad_order.json().await.unwrap();
    assert_eq!(bad_order_body["code"], "BAD_REQUEST");
    assert_eq!(bad_order_body["message"], "Bad Request");
    assert!(
        bad_order_body["details"]
            .as_str()
            .unwrap()
            .contains("sideways"),
        "details were: {}",
        bad_order_body["details"]
    );
}

#[tokio::test]
async fn historic_single_delete_endpoints_match_status_and_cascade() {
    let (engine, base_url, client) =
        spawn_server("rest-bpmn-historic-single-delete-contract").await;
    deploy_process(&client, &base_url).await;
    let process_definition_id = engine
        .get_repository_service()
        .get_process_definition_ids()
        .unwrap()[0]
        .clone();

    let process_to_delete =
        start_process(&client, &base_url, &process_definition_id, "delete-process").await;
    let process_to_delete_id = process_to_delete["id"].as_str().unwrap();
    let task_to_delete_id = first_task_id(&client, &base_url, process_to_delete_id).await;
    complete_first_task(&client, &base_url, process_to_delete_id).await;

    let delete_process = client
        .delete(format!(
            "{base_url}/history/historic-process-instances/{process_to_delete_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(delete_process.status(), reqwest::StatusCode::NO_CONTENT);

    let get_deleted_process = client
        .get(format!(
            "{base_url}/history/historic-process-instances/{process_to_delete_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(get_deleted_process.status(), reqwest::StatusCode::NOT_FOUND);

    let get_cascaded_task = client
        .get(format!(
            "{base_url}/history/historic-task-instances/{task_to_delete_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(get_cascaded_task.status(), reqwest::StatusCode::NOT_FOUND);

    let process_for_task_delete =
        start_process(&client, &base_url, &process_definition_id, "delete-task").await;
    let process_for_task_delete_id = process_for_task_delete["id"].as_str().unwrap();
    let task_delete_id = first_task_id(&client, &base_url, process_for_task_delete_id).await;
    complete_first_task(&client, &base_url, process_for_task_delete_id).await;

    let delete_task = client
        .delete(format!(
            "{base_url}/history/historic-task-instances/{task_delete_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(delete_task.status(), reqwest::StatusCode::NO_CONTENT);

    let get_deleted_task = client
        .get(format!(
            "{base_url}/history/historic-task-instances/{task_delete_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(get_deleted_task.status(), reqwest::StatusCode::NOT_FOUND);

    let get_remaining_process = client
        .get(format!(
            "{base_url}/history/historic-process-instances/{process_for_task_delete_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(get_remaining_process.status(), reqwest::StatusCode::OK);

    let missing_task_delete = client
        .delete(format!(
            "{base_url}/history/historic-task-instances/{task_delete_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(missing_task_delete.status(), reqwest::StatusCode::NOT_FOUND);
}
