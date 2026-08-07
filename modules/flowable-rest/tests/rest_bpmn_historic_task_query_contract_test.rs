use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_rest::run_server;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::net::TcpListener;

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

fn user_task_process_xml(process_id: &str, task_name: &str) -> String {
    user_task_process_xml_with_assignee(process_id, task_name, None)
}

fn user_task_process_xml_with_task_id(process_id: &str, task_id: &str, task_name: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" xmlns:flowable="http://flowable.org/bpmn" targetNamespace="Examples">
    <process id="{process_id}" name="{process_id}" isExecutable="true">
        <startEvent id="start" />
        <sequenceFlow id="flow1" sourceRef="start" targetRef="{task_id}" />
        <userTask id="{task_id}" name="{task_name}" />
        <sequenceFlow id="flow2" sourceRef="{task_id}" targetRef="end" />
        <endEvent id="end" />
    </process>
</definitions>"#
    )
}

fn user_task_process_xml_with_assignee(
    process_id: &str,
    task_name: &str,
    assignee: Option<&str>,
) -> String {
    user_task_process_xml_with_assignee_and_owner(process_id, task_name, assignee, None)
}

fn user_task_process_xml_with_assignee_and_owner(
    process_id: &str,
    task_name: &str,
    assignee: Option<&str>,
    owner: Option<&str>,
) -> String {
    let assignee_attribute = assignee
        .map(|value| format!(r#" flowable:assignee="{value}""#))
        .unwrap_or_default();
    let owner_attribute = owner
        .map(|value| format!(r#" flowable:owner="{value}""#))
        .unwrap_or_default();
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" xmlns:flowable="http://flowable.org/bpmn" targetNamespace="Examples">
    <process id="{process_id}" name="{process_id}" isExecutable="true">
        <startEvent id="start" />
        <sequenceFlow id="flow1" sourceRef="start" targetRef="task1" />
        <userTask id="task1" name="{task_name}"{assignee_attribute}{owner_attribute} />
        <sequenceFlow id="flow2" sourceRef="task1" targetRef="end" />
        <endEvent id="end" />
    </process>
</definitions>"#
    )
}

fn user_task_process_xml_with_candidates(
    process_id: &str,
    task_name: &str,
    candidate_users: &str,
    candidate_groups: &str,
) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" xmlns:flowable="http://flowable.org/bpmn" targetNamespace="Examples">
    <process id="{process_id}" name="{process_id}" isExecutable="true">
        <startEvent id="start" />
        <sequenceFlow id="flow1" sourceRef="start" targetRef="task1" />
        <userTask id="task1" name="{task_name}" flowable:candidateUsers="{candidate_users}" flowable:candidateGroups="{candidate_groups}" />
        <sequenceFlow id="flow2" sourceRef="task1" targetRef="end" />
        <endEvent id="end" />
    </process>
</definitions>"#
    )
}

fn user_task_process_xml_with_priority_and_due_date(
    process_id: &str,
    task_name: &str,
    priority: i32,
    due_date: &str,
) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" xmlns:flowable="http://flowable.org/bpmn" targetNamespace="Examples">
    <process id="{process_id}" name="{process_id}" isExecutable="true">
        <startEvent id="start" />
        <sequenceFlow id="flow1" sourceRef="start" targetRef="task1" />
        <userTask id="task1" name="{task_name}" flowable:priority="{priority}" flowable:dueDate="{due_date}" />
        <sequenceFlow id="flow2" sourceRef="task1" targetRef="end" />
        <endEvent id="end" />
    </process>
</definitions>"#
    )
}

async fn deploy_process(
    client: &reqwest::Client,
    base_url: &str,
    process_id: &str,
    task_name: &str,
) {
    let response = client
        .post(format!("{base_url}/repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": format!("{process_id} deployment"),
            "resourceName": format!("{process_id}.bpmn20.xml"),
            "resource": user_task_process_xml(process_id, task_name)
        }))
        .send()
        .await
        .unwrap();

    assert!(response.status().is_success(), "{response:?}");
}

async fn deploy_process_with_task_id(
    client: &reqwest::Client,
    base_url: &str,
    process_id: &str,
    task_id: &str,
    task_name: &str,
) {
    let response = client
        .post(format!("{base_url}/repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": format!("{process_id} deployment"),
            "resourceName": format!("{process_id}.bpmn20.xml"),
            "resource": user_task_process_xml_with_task_id(process_id, task_id, task_name)
        }))
        .send()
        .await
        .unwrap();

    assert!(response.status().is_success(), "{response:?}");
}

async fn deploy_process_with_priority_and_due_date(
    client: &reqwest::Client,
    base_url: &str,
    process_id: &str,
    task_name: &str,
    priority: i32,
    due_date: &str,
) {
    let response = client
        .post(format!("{base_url}/repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": format!("{process_id} deployment"),
            "resourceName": format!("{process_id}.bpmn20.xml"),
            "resource": user_task_process_xml_with_priority_and_due_date(
                process_id,
                task_name,
                priority,
                due_date
            )
        }))
        .send()
        .await
        .unwrap();

    assert!(response.status().is_success(), "{response:?}");
}

async fn deploy_process_with_assignee(
    client: &reqwest::Client,
    base_url: &str,
    process_id: &str,
    task_name: &str,
    assignee: &str,
) {
    let response = client
        .post(format!("{base_url}/repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": format!("{process_id} deployment"),
            "resourceName": format!("{process_id}.bpmn20.xml"),
            "resource": user_task_process_xml_with_assignee(process_id, task_name, Some(assignee))
        }))
        .send()
        .await
        .unwrap();

    assert!(response.status().is_success(), "{response:?}");
}

async fn deploy_process_with_owner(
    client: &reqwest::Client,
    base_url: &str,
    process_id: &str,
    task_name: &str,
    owner: &str,
) {
    let response = client
        .post(format!("{base_url}/repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": format!("{process_id} deployment"),
            "resourceName": format!("{process_id}.bpmn20.xml"),
            "resource": user_task_process_xml_with_assignee_and_owner(
                process_id,
                task_name,
                None,
                Some(owner)
            )
        }))
        .send()
        .await
        .unwrap();

    assert!(response.status().is_success(), "{response:?}");
}

async fn deploy_process_with_candidates(
    client: &reqwest::Client,
    base_url: &str,
    process_id: &str,
    task_name: &str,
    candidate_users: &str,
    candidate_groups: &str,
) {
    let response = client
        .post(format!("{base_url}/repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": format!("{process_id} deployment"),
            "resourceName": format!("{process_id}.bpmn20.xml"),
            "resource": user_task_process_xml_with_candidates(
                process_id,
                task_name,
                candidate_users,
                candidate_groups
            )
        }))
        .send()
        .await
        .unwrap();

    assert!(response.status().is_success(), "{response:?}");
}

async fn start_process(client: &reqwest::Client, base_url: &str, process_key: &str) -> Value {
    let response = client
        .post(format!("{base_url}/runtime/process-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "processDefinitionKey": process_key,
            "businessKey": format!("{process_key}-business-key")
        }))
        .send()
        .await
        .unwrap();

    assert!(response.status().is_success(), "{response:?}");
    response.json().await.unwrap()
}

async fn runtime_task_for_process(
    client: &reqwest::Client,
    base_url: &str,
    process_instance_id: &str,
) -> Value {
    let response = client
        .get(format!(
            "{base_url}/runtime/tasks?processInstanceId={process_instance_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["total"], 1);
    body["data"][0].clone()
}

async fn complete_task(client: &reqwest::Client, base_url: &str, task_id: &str) {
    let response = client
        .post(format!("{base_url}/runtime/tasks/{task_id}/complete"))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "action": "complete" }))
        .send()
        .await
        .unwrap();

    assert!(response.status().is_success(), "{response:?}");
}

async fn update_task_description(
    client: &reqwest::Client,
    base_url: &str,
    task_id: &str,
    description: &str,
) {
    let response = client
        .put(format!("{base_url}/runtime/tasks/{task_id}"))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "description": description }))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), reqwest::StatusCode::OK);
}

fn historic_task_names(page: &Value) -> Vec<&str> {
    page["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|task| task["name"].as_str().unwrap())
        .collect()
}

async fn deploy_and_start_historic_task_fixture(
    client: &reqwest::Client,
    base_url: &str,
) -> Vec<(&'static str, &'static str, Value)> {
    let mut instances = Vec::new();
    for (process_key, task_name) in [
        ("historicTaskZuluProcess", "Zulu approval"),
        ("historicTaskAlphaProcess", "Alpha approval"),
        ("historicTaskMiddleProcess", "Middle approval"),
    ] {
        deploy_process(client, base_url, process_key, task_name).await;
        let instance = start_process(client, base_url, process_key).await;
        instances.push((process_key, task_name, instance));
    }
    instances
}

#[tokio::test]
async fn historic_task_instances_accept_metadata_filters_and_sort_aliases() {
    let (_engine, base_url, client) = spawn_server("rest-bpmn-historic-task-field-sort").await;
    deploy_process_with_task_id(
        &client,
        &base_url,
        "historicSortAlphaProcess",
        "alphaTask",
        "Alpha approval",
    )
    .await;
    deploy_process_with_assignee(
        &client,
        &base_url,
        "historicSortMiddleProcess",
        "Middle approval",
        "bob",
    )
    .await;
    deploy_process_with_task_id(
        &client,
        &base_url,
        "historicSortZuluProcess",
        "zuluTask",
        "Zulu approval",
    )
    .await;

    let alpha_instance = start_process(&client, &base_url, "historicSortAlphaProcess").await;
    let middle_instance = start_process(&client, &base_url, "historicSortMiddleProcess").await;
    let zulu_instance = start_process(&client, &base_url, "historicSortZuluProcess").await;

    for instance in [&alpha_instance, &middle_instance, &zulu_instance] {
        let task =
            runtime_task_for_process(&client, &base_url, instance["id"].as_str().unwrap()).await;
        complete_task(&client, &base_url, task["id"].as_str().unwrap()).await;
    }

    let by_task_definition_key = client
        .get(format!(
            "{base_url}/history/historic-task-instances?finished=true&sort=taskDefinitionKey&order=asc"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(by_task_definition_key.status(), reqwest::StatusCode::OK);
    let by_task_definition_key_body: Value = by_task_definition_key.json().await.unwrap();
    assert_eq!(
        historic_task_names(&by_task_definition_key_body),
        vec!["Alpha approval", "Middle approval", "Zulu approval"]
    );

    let by_created_alias = client
        .post(format!("{base_url}/query/historic-task-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "finished": true,
            "sort": "created",
            "order": "desc"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(by_created_alias.status(), reqwest::StatusCode::OK);
    let by_created_alias_body: Value = by_created_alias.json().await.unwrap();
    assert_eq!(
        historic_task_names(&by_created_alias_body),
        vec!["Zulu approval", "Middle approval", "Alpha approval"]
    );

    let by_create_time_alias = client
        .get(format!(
            "{base_url}/history/historic-task-instances?finished=true&sort=createTime&order=asc"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(by_create_time_alias.status(), reqwest::StatusCode::OK);
    let by_create_time_alias_body: Value = by_create_time_alias.json().await.unwrap();
    assert_eq!(
        historic_task_names(&by_create_time_alias_body),
        vec!["Alpha approval", "Middle approval", "Zulu approval"]
    );

    for sort in [
        "assignee", "owner", "category", "tenantId", "priority", "dueDate",
    ] {
        let response = client
            .get(format!(
                "{base_url}/history/historic-task-instances?finished=true&sort={sort}&order=asc"
            ))
            .basic_auth("admin", Some("test"))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::OK, "{sort}");
    }

    let category_filter_response = client
        .post(format!("{base_url}/query/historic-task-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "category": "approval",
            "tenantId": "tenant-a"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(category_filter_response.status(), reqwest::StatusCode::OK);
    let category_filter_body: Value = category_filter_response.json().await.unwrap();
    assert_eq!(category_filter_body["total"], 0);
}

#[tokio::test]
async fn historic_task_instances_reject_unknown_sort_and_order_with_structured_400() {
    let (_engine, base_url, client) = spawn_server("rest-bpmn-historic-task-bad-sort").await;

    let bad_sort_response = client
        .get(format!(
            "{base_url}/history/historic-task-instances?sort=unsupportedField&order=asc"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(bad_sort_response.status(), reqwest::StatusCode::BAD_REQUEST);
    let bad_sort_body: Value = bad_sort_response.json().await.unwrap();
    assert_eq!(bad_sort_body["code"], "BAD_REQUEST");
    assert!(
        bad_sort_body["details"]
            .as_str()
            .unwrap()
            .contains("Unsupported historic task sort field 'unsupportedField'")
    );

    let bad_order_response = client
        .post(format!("{base_url}/query/historic-task-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "sort": "name",
            "order": "sideways"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        bad_order_response.status(),
        reqwest::StatusCode::BAD_REQUEST
    );
    let bad_order_body: Value = bad_order_response.json().await.unwrap();
    assert_eq!(bad_order_body["code"], "BAD_REQUEST");
    assert!(
        bad_order_body["details"]
            .as_str()
            .unwrap()
            .contains("Unsupported historic task sort order 'sideways'")
    );
}

#[tokio::test]
async fn historic_task_instances_filter_finished_and_page_after_sorting() {
    let (_engine, base_url, client) = spawn_server("rest-bpmn-historic-task-query-sort").await;
    let instances = deploy_and_start_historic_task_fixture(&client, &base_url).await;
    let alpha_process_instance_id = instances
        .iter()
        .find(|(process_key, _, _)| *process_key == "historicTaskAlphaProcess")
        .unwrap()
        .2["id"]
        .as_str()
        .unwrap();
    let alpha_task = runtime_task_for_process(&client, &base_url, alpha_process_instance_id).await;
    let alpha_task_id = alpha_task["id"].as_str().unwrap();
    complete_task(&client, &base_url, alpha_task_id).await;

    let finished_response = client
        .get(format!(
            "{base_url}/history/historic-task-instances?finished=true&sort=name&order=asc"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(finished_response.status(), reqwest::StatusCode::OK);
    let finished_body: Value = finished_response.json().await.unwrap();
    assert_eq!(finished_body["total"], 1);
    assert_eq!(finished_body["data"][0]["id"], alpha_task_id);
    assert_eq!(finished_body["data"][0]["name"], "Alpha approval");
    assert!(finished_body["data"][0]["endTime"].is_string());

    let unfinished_response = client
        .get(format!(
            "{base_url}/history/historic-task-instances?finished=false&sort=name&order=desc&start=1&size=1"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(unfinished_response.status(), reqwest::StatusCode::OK);
    let unfinished_body: Value = unfinished_response.json().await.unwrap();
    assert_eq!(unfinished_body["start"], 1);
    assert_eq!(unfinished_body["size"], 1);
    assert_eq!(unfinished_body["total"], 2);
    assert_eq!(unfinished_body["data"][0]["name"], "Middle approval");
    assert!(unfinished_body["data"][0]["endTime"].is_null());
}

#[tokio::test]
async fn query_historic_task_instances_accepts_body_filters_and_url_paging_overrides_body_paging() {
    let (_engine, base_url, client) = spawn_server("rest-bpmn-query-historic-task-filter").await;
    let instances = deploy_and_start_historic_task_fixture(&client, &base_url).await;
    let mut expected_unfinished = instances
        .iter()
        .map(|(_, task_name, instance)| (*task_name, instance["id"].as_str().unwrap().to_string()))
        .collect::<Vec<_>>();
    expected_unfinished.sort_by(|left, right| left.0.cmp(right.0));

    let paged_response = client
        .post(format!(
            "{base_url}/query/historic-task-instances?start=1&size=1"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "start": 0,
            "size": 3,
            "finished": false,
            "sort": "name",
            "order": "asc"
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(paged_response.status(), reqwest::StatusCode::OK);
    let paged_body: Value = paged_response.json().await.unwrap();
    assert_eq!(paged_body["start"], 1);
    assert_eq!(paged_body["size"], 1);
    assert_eq!(paged_body["total"], 3);
    assert_eq!(paged_body["data"][0]["name"], expected_unfinished[1].0);
    assert_eq!(
        paged_body["data"][0]["processInstanceId"],
        expected_unfinished[1].1
    );

    let zulu_process_instance_id = instances
        .iter()
        .find(|(process_key, _, _)| *process_key == "historicTaskZuluProcess")
        .unwrap()
        .2["id"]
        .as_str()
        .unwrap();
    let filtered_response = client
        .post(format!("{base_url}/query/historic-task-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "processInstanceId": zulu_process_instance_id,
            "taskName": "Zulu approval",
            "finished": false
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(filtered_response.status(), reqwest::StatusCode::OK);
    let filtered_body: Value = filtered_response.json().await.unwrap();
    assert_eq!(filtered_body["total"], 1);
    assert_eq!(filtered_body["data"][0]["name"], "Zulu approval");
    assert_eq!(
        filtered_body["data"][0]["processInstanceId"],
        zulu_process_instance_id
    );
}

#[tokio::test]
async fn historic_task_instances_get_filters_by_name_like_process_definition_and_time_ranges() {
    let (_engine, base_url, client) =
        spawn_server("rest-bpmn-historic-task-query-get-matrix").await;
    let instances = deploy_and_start_historic_task_fixture(&client, &base_url).await;
    let alpha_process_instance = instances
        .iter()
        .find(|(process_key, _, _)| *process_key == "historicTaskAlphaProcess")
        .unwrap()
        .2
        .clone();
    let alpha_process_instance_id = alpha_process_instance["id"].as_str().unwrap();
    let alpha_process_definition_id = alpha_process_instance["processDefinitionId"]
        .as_str()
        .unwrap();
    let alpha_task = runtime_task_for_process(&client, &base_url, alpha_process_instance_id).await;
    let alpha_task_id = alpha_task["id"].as_str().unwrap();
    complete_task(&client, &base_url, alpha_task_id).await;

    let filtered_response = client
        .get(format!(
            "{base_url}/history/historic-task-instances?taskNameLike=Alpha%25&processDefinitionId={alpha_process_definition_id}&finished=true&startedAfter=1970-01-01T00:00:00Z&startedBefore=9999-12-31T23:59:59Z&finishedAfter=1970-01-01T00:00:00Z&finishedBefore=9999-12-31T23:59:59Z"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(filtered_response.status(), reqwest::StatusCode::OK);
    let filtered_body: Value = filtered_response.json().await.unwrap();
    assert_eq!(filtered_body["total"], 1);
    assert_eq!(filtered_body["data"][0]["id"], alpha_task_id);
    assert_eq!(filtered_body["data"][0]["name"], "Alpha approval");
    assert_eq!(
        filtered_body["data"][0]["processInstanceId"],
        alpha_process_instance_id
    );
    assert_eq!(
        filtered_body["data"][0]["processDefinitionId"],
        alpha_process_definition_id
    );

    let before_epoch_response = client
        .get(format!(
            "{base_url}/history/historic-task-instances?nameLike=%25approval&startedBefore=1970-01-01T00:00:00Z"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(before_epoch_response.status(), reqwest::StatusCode::OK);
    let before_epoch_body: Value = before_epoch_response.json().await.unwrap();
    assert_eq!(before_epoch_body["total"], 0);
}

#[tokio::test]
async fn query_historic_task_instances_post_filters_by_name_like_and_finished_time_range() {
    let (_engine, base_url, client) =
        spawn_server("rest-bpmn-historic-task-query-post-matrix").await;
    let instances = deploy_and_start_historic_task_fixture(&client, &base_url).await;
    let zulu_process_instance_id = instances
        .iter()
        .find(|(process_key, _, _)| *process_key == "historicTaskZuluProcess")
        .unwrap()
        .2["id"]
        .as_str()
        .unwrap();
    let zulu_process_definition_id = instances
        .iter()
        .find(|(process_key, _, _)| *process_key == "historicTaskZuluProcess")
        .unwrap()
        .2["processDefinitionId"]
        .as_str()
        .unwrap();
    let zulu_task = runtime_task_for_process(&client, &base_url, zulu_process_instance_id).await;
    let zulu_task_id = zulu_task["id"].as_str().unwrap();
    complete_task(&client, &base_url, zulu_task_id).await;

    let filtered_response = client
        .post(format!("{base_url}/query/historic-task-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "nameLike": "%approval",
            "processInstanceId": zulu_process_instance_id,
            "finished": true,
            "finishedAfter": "1970-01-01T00:00:00Z",
            "finishedBefore": "9999-12-31T23:59:59Z"
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(filtered_response.status(), reqwest::StatusCode::OK);
    let filtered_body: Value = filtered_response.json().await.unwrap();
    assert_eq!(filtered_body["total"], 1);
    assert_eq!(filtered_body["data"][0]["id"], zulu_task_id);
    assert_eq!(filtered_body["data"][0]["name"], "Zulu approval");
    assert_eq!(
        filtered_body["data"][0]["processDefinitionId"],
        zulu_process_definition_id
    );
    assert!(filtered_body["data"][0]["endTime"].is_string());

    let due_date_filtered_response = client
        .post(format!("{base_url}/query/historic-task-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "dueDate": "2026-01-01T00:00:00Z"
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(due_date_filtered_response.status(), reqwest::StatusCode::OK);
    let due_date_filtered_body: Value = due_date_filtered_response.json().await.unwrap();
    assert_eq!(due_date_filtered_body["total"], 0);
}

#[tokio::test]
async fn query_tasks_filters_by_stable_fields_and_priority_due_date() {
    let (_engine, base_url, client) =
        spawn_server("rest-bpmn-query-task-filter-stable-fields").await;
    let instances = deploy_and_start_historic_task_fixture(&client, &base_url).await;
    let middle_process_instance_id = instances
        .iter()
        .find(|(process_key, _, _)| *process_key == "historicTaskMiddleProcess")
        .unwrap()
        .2["id"]
        .as_str()
        .unwrap();

    let filtered_response = client
        .post(format!("{base_url}/query/tasks"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "processInstanceId": middle_process_instance_id,
            "name": "Middle approval"
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(filtered_response.status(), reqwest::StatusCode::OK);
    let filtered_body: Value = filtered_response.json().await.unwrap();
    assert_eq!(filtered_body["total"], 1);
    assert_eq!(filtered_body["data"][0]["name"], "Middle approval");
    assert_eq!(
        filtered_body["data"][0]["processInstanceId"],
        middle_process_instance_id
    );

    let priority_filtered_response = client
        .post(format!("{base_url}/query/tasks"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "priority": 50
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(priority_filtered_response.status(), reqwest::StatusCode::OK);
    let priority_filtered_body: Value = priority_filtered_response.json().await.unwrap();
    assert_eq!(priority_filtered_body["total"], 0);
}

#[tokio::test]
async fn query_tasks_accepts_unassigned_and_delegation_filters() {
    let (_engine, base_url, client) = spawn_server("rest-bpmn-query-task-delegation").await;
    deploy_process(
        &client,
        &base_url,
        "runtimeLikeAlphaProcess",
        "Runtime Alpha Review",
    )
    .await;
    deploy_process_with_assignee(
        &client,
        &base_url,
        "runtimeLikeBetaProcess",
        "Runtime Beta Review",
        "kermit",
    )
    .await;
    deploy_process(
        &client,
        &base_url,
        "runtimeLikeGammaProcess",
        "Runtime Gamma Approval",
    )
    .await;

    let alpha_instance = start_process(&client, &base_url, "runtimeLikeAlphaProcess").await;
    let beta_instance = start_process(&client, &base_url, "runtimeLikeBetaProcess").await;
    let gamma_instance = start_process(&client, &base_url, "runtimeLikeGammaProcess").await;
    let alpha_task =
        runtime_task_for_process(&client, &base_url, alpha_instance["id"].as_str().unwrap()).await;
    let beta_task =
        runtime_task_for_process(&client, &base_url, beta_instance["id"].as_str().unwrap()).await;
    let gamma_task =
        runtime_task_for_process(&client, &base_url, gamma_instance["id"].as_str().unwrap()).await;
    let alpha_task_id = alpha_task["id"].as_str().unwrap();
    let beta_task_id = beta_task["id"].as_str().unwrap();
    let gamma_task_id = gamma_task["id"].as_str().unwrap();

    let name_like_response = client
        .get(format!(
            "{base_url}/runtime/tasks?nameLike=Runtime%25Review&sort=name&order=asc"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(name_like_response.status(), reqwest::StatusCode::OK);
    let name_like_body: Value = name_like_response.json().await.unwrap();
    assert_eq!(name_like_body["total"], 2);
    assert_eq!(name_like_body["data"][0]["id"], alpha_task_id);
    assert_eq!(name_like_body["data"][1]["id"], beta_task_id);

    let assignee_like_response = client
        .post(format!("{base_url}/query/tasks"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "assigneeLike": "ker%",
            "unassigned": false
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(assignee_like_response.status(), reqwest::StatusCode::OK);
    let assignee_like_body: Value = assignee_like_response.json().await.unwrap();
    assert_eq!(assignee_like_body["total"], 1);
    assert_eq!(assignee_like_body["data"][0]["id"], beta_task_id);

    let unassigned_response = client
        .get(format!(
            "{base_url}/runtime/tasks?unassigned=true&sort=name&order=asc"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(unassigned_response.status(), reqwest::StatusCode::OK);
    let unassigned_body: Value = unassigned_response.json().await.unwrap();
    assert_eq!(unassigned_body["total"], 2);
    assert_eq!(unassigned_body["data"][0]["id"], alpha_task_id);
    assert_eq!(unassigned_body["data"][1]["id"], gamma_task_id);

    let delegation_update = client
        .put(format!("{base_url}/runtime/tasks/{beta_task_id}"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "delegationState": "pending"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(delegation_update.status(), reqwest::StatusCode::OK);

    let delegation_response = client
        .post(format!("{base_url}/query/tasks"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "delegationState": "pending"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(delegation_response.status(), reqwest::StatusCode::OK);
    let delegation_body: Value = delegation_response.json().await.unwrap();
    assert_eq!(delegation_body["total"], 1);
    assert_eq!(delegation_body["data"][0]["id"], beta_task_id);

    let invalid_delegation = client
        .get(format!("{base_url}/runtime/tasks?delegationState=sideways"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(
        invalid_delegation.status(),
        reqwest::StatusCode::BAD_REQUEST
    );
    let invalid_delegation_body: Value = invalid_delegation.json().await.unwrap();
    assert_eq!(invalid_delegation_body["code"], "BAD_REQUEST");
    assert!(
        invalid_delegation_body["details"]
            .as_str()
            .unwrap()
            .contains("Unsupported task delegationState 'sideways'")
    );
}

#[tokio::test]
async fn task_and_historic_task_definition_key_filters_match_canonical_rest_fields() {
    let (_engine, base_url, client) =
        spawn_server("rest-bpmn-task-definition-key-filter-chain").await;
    deploy_process_with_task_id(
        &client,
        &base_url,
        "definitionKeyTaskProcess",
        "approveInvoiceTask",
        "Approve invoice",
    )
    .await;
    deploy_process_with_task_id(
        &client,
        &base_url,
        "otherDefinitionKeyTaskProcess",
        "reviewContractTask",
        "Review contract",
    )
    .await;

    let instance = start_process(&client, &base_url, "definitionKeyTaskProcess").await;
    let other_instance = start_process(&client, &base_url, "otherDefinitionKeyTaskProcess").await;
    let process_instance_id = instance["id"].as_str().unwrap();
    let process_definition_id = instance["processDefinitionId"].as_str().unwrap();
    let other_process_instance_id = other_instance["id"].as_str().unwrap();
    let task = runtime_task_for_process(&client, &base_url, process_instance_id).await;
    let task_id = task["id"].as_str().unwrap();
    assert_eq!(task["taskDefinitionKey"], "approveInvoiceTask");

    let runtime_get_response = client
        .get(format!(
            "{base_url}/runtime/tasks?taskDefinitionKey=approveInvoiceTask"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(runtime_get_response.status(), reqwest::StatusCode::OK);
    let runtime_get_body: Value = runtime_get_response.json().await.unwrap();
    assert_eq!(runtime_get_body["total"], 1);
    assert_eq!(runtime_get_body["data"][0]["id"], task_id);
    assert_eq!(
        runtime_get_body["data"][0]["taskDefinitionKey"],
        "approveInvoiceTask"
    );

    let runtime_like_post_response = client
        .post(format!("{base_url}/query/tasks"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "taskDefinitionKeyLike": "approve%Task",
            "processInstanceId": process_instance_id
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(runtime_like_post_response.status(), reqwest::StatusCode::OK);
    let runtime_like_post_body: Value = runtime_like_post_response.json().await.unwrap();
    assert_eq!(runtime_like_post_body["total"], 1);
    assert_eq!(runtime_like_post_body["data"][0]["id"], task_id);

    let runtime_wrong_key_response = client
        .get(format!(
            "{base_url}/runtime/tasks?processInstanceId={process_instance_id}&taskDefinitionKey=reviewContractTask"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(runtime_wrong_key_response.status(), reqwest::StatusCode::OK);
    let runtime_wrong_key_body: Value = runtime_wrong_key_response.json().await.unwrap();
    assert_eq!(runtime_wrong_key_body["total"], 0);

    complete_task(&client, &base_url, task_id).await;

    let historic_get_response = client
        .get(format!(
            "{base_url}/history/historic-task-instances?taskDefinitionKey=approveInvoiceTask&processDefinitionId={process_definition_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(historic_get_response.status(), reqwest::StatusCode::OK);
    let historic_get_body: Value = historic_get_response.json().await.unwrap();
    assert_eq!(historic_get_body["total"], 1);
    assert_eq!(historic_get_body["data"][0]["id"], task_id);
    assert_eq!(
        historic_get_body["data"][0]["taskDefinitionKey"],
        "approveInvoiceTask"
    );

    let historic_like_post_response = client
        .post(format!("{base_url}/query/historic-task-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "taskDefinitionKeyLike": "approve%Task",
            "finished": true
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        historic_like_post_response.status(),
        reqwest::StatusCode::OK
    );
    let historic_like_post_body: Value = historic_like_post_response.json().await.unwrap();
    assert_eq!(historic_like_post_body["total"], 1);
    assert_eq!(historic_like_post_body["data"][0]["id"], task_id);

    let historic_wrong_key_response = client
        .get(format!(
            "{base_url}/history/historic-task-instances?processInstanceId={other_process_instance_id}&taskDefinitionKey=approveInvoiceTask"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(
        historic_wrong_key_response.status(),
        reqwest::StatusCode::OK
    );
    let historic_wrong_key_body: Value = historic_wrong_key_response.json().await.unwrap();
    assert_eq!(historic_wrong_key_body["total"], 0);
}

#[tokio::test]
async fn task_and_historic_task_priority_due_date_round_trip_response_and_filters() {
    let (_engine, base_url, client) = spawn_server("rest-bpmn-task-priority-due-date-chain").await;
    let due_date = "2026-01-31T10:20:30Z";
    deploy_process_with_priority_and_due_date(
        &client,
        &base_url,
        "priorityDueDateTaskProcess",
        "Priority due approval",
        70,
        due_date,
    )
    .await;
    deploy_process_with_priority_and_due_date(
        &client,
        &base_url,
        "otherPriorityDueDateTaskProcess",
        "Other priority due approval",
        20,
        "2026-02-01T00:00:00Z",
    )
    .await;

    let instance = start_process(&client, &base_url, "priorityDueDateTaskProcess").await;
    let _other_instance =
        start_process(&client, &base_url, "otherPriorityDueDateTaskProcess").await;
    let process_instance_id = instance["id"].as_str().unwrap();
    let process_definition_id = instance["processDefinitionId"].as_str().unwrap();
    let task = runtime_task_for_process(&client, &base_url, process_instance_id).await;
    let task_id = task["id"].as_str().unwrap();
    assert_eq!(task["priority"], 70);
    assert_eq!(
        chrono::DateTime::parse_from_rfc3339(task["dueDate"].as_str().unwrap())
            .unwrap()
            .timestamp(),
        chrono::DateTime::parse_from_rfc3339(due_date)
            .unwrap()
            .timestamp()
    );

    let runtime_get_response = client
        .get(format!(
            "{base_url}/runtime/tasks?priority=70&dueDate=2026-01-31T10%3A20%3A30Z"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(runtime_get_response.status(), reqwest::StatusCode::OK);
    let runtime_get_body: Value = runtime_get_response.json().await.unwrap();
    assert_eq!(runtime_get_body["total"], 1);
    assert_eq!(runtime_get_body["data"][0]["id"], task_id);
    assert_eq!(runtime_get_body["data"][0]["priority"], 70);

    let runtime_post_response = client
        .post(format!("{base_url}/query/tasks"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "processInstanceId": process_instance_id,
            "priority": 70,
            "dueDate": due_date
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(runtime_post_response.status(), reqwest::StatusCode::OK);
    let runtime_post_body: Value = runtime_post_response.json().await.unwrap();
    assert_eq!(runtime_post_body["total"], 1);
    assert_eq!(runtime_post_body["data"][0]["id"], task_id);

    let runtime_wrong_priority_response = client
        .post(format!("{base_url}/query/tasks"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "processInstanceId": process_instance_id,
            "priority": 20
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        runtime_wrong_priority_response.status(),
        reqwest::StatusCode::OK
    );
    let runtime_wrong_priority_body: Value = runtime_wrong_priority_response.json().await.unwrap();
    assert_eq!(runtime_wrong_priority_body["total"], 0);

    complete_task(&client, &base_url, task_id).await;

    let historic_get_response = client
        .get(format!(
            "{base_url}/history/historic-task-instances?priority=70&dueDate=2026-01-31T10%3A20%3A30Z&processDefinitionId={process_definition_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(historic_get_response.status(), reqwest::StatusCode::OK);
    let historic_get_body: Value = historic_get_response.json().await.unwrap();
    assert_eq!(historic_get_body["total"], 1);
    assert_eq!(historic_get_body["data"][0]["id"], task_id);
    assert_eq!(historic_get_body["data"][0]["priority"], 70);
    assert_eq!(
        chrono::DateTime::parse_from_rfc3339(
            historic_get_body["data"][0]["dueDate"].as_str().unwrap()
        )
        .unwrap()
        .timestamp(),
        chrono::DateTime::parse_from_rfc3339(due_date)
            .unwrap()
            .timestamp()
    );

    let historic_post_response = client
        .post(format!("{base_url}/query/historic-task-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "processInstanceId": process_instance_id,
            "priority": 70,
            "dueDate": due_date,
            "finished": true
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(historic_post_response.status(), reqwest::StatusCode::OK);
    let historic_post_body: Value = historic_post_response.json().await.unwrap();
    assert_eq!(historic_post_body["total"], 1);
    assert_eq!(historic_post_body["data"][0]["id"], task_id);

    let historic_wrong_due_date_response = client
        .get(format!(
            "{base_url}/history/historic-task-instances?processInstanceId={process_instance_id}&dueDate=2026-02-01T00%3A00%3A00Z"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(
        historic_wrong_due_date_response.status(),
        reqwest::StatusCode::OK
    );
    let historic_wrong_due_date_body: Value =
        historic_wrong_due_date_response.json().await.unwrap();
    assert_eq!(historic_wrong_due_date_body["total"], 0);
}

#[tokio::test]
async fn task_and_historic_task_assignee_round_trip_response_and_filters() {
    let (_engine, base_url, client) = spawn_server("rest-bpmn-task-assignee-chain").await;
    deploy_process_with_assignee(
        &client,
        &base_url,
        "assigneeTaskProcess",
        "Assignee approval",
        "kermit",
    )
    .await;
    deploy_process_with_assignee(
        &client,
        &base_url,
        "otherAssigneeTaskProcess",
        "Other approval",
        "fozzie",
    )
    .await;
    let instance = start_process(&client, &base_url, "assigneeTaskProcess").await;
    let other_instance = start_process(&client, &base_url, "otherAssigneeTaskProcess").await;
    let process_instance_id = instance["id"].as_str().unwrap();
    let process_definition_id = instance["processDefinitionId"].as_str().unwrap();
    let task = runtime_task_for_process(&client, &base_url, process_instance_id).await;
    let task_id = task["id"].as_str().unwrap();
    assert_eq!(task["assignee"], "kermit");

    let runtime_get_response = client
        .get(format!("{base_url}/runtime/tasks?assignee=kermit"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(runtime_get_response.status(), reqwest::StatusCode::OK);
    let runtime_get_body: Value = runtime_get_response.json().await.unwrap();
    assert_eq!(runtime_get_body["total"], 1);
    assert_eq!(runtime_get_body["data"][0]["id"], task_id);
    assert_eq!(runtime_get_body["data"][0]["assignee"], "kermit");

    let runtime_post_response = client
        .post(format!("{base_url}/query/tasks"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "processInstanceId": process_instance_id,
            "assignee": "kermit"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(runtime_post_response.status(), reqwest::StatusCode::OK);
    let runtime_post_body: Value = runtime_post_response.json().await.unwrap();
    assert_eq!(runtime_post_body["total"], 1);
    assert_eq!(runtime_post_body["data"][0]["id"], task_id);
    assert_eq!(runtime_post_body["data"][0]["assignee"], "kermit");

    let runtime_wrong_assignee_response = client
        .post(format!("{base_url}/query/tasks"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "processInstanceId": process_instance_id,
            "assignee": "fozzie"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        runtime_wrong_assignee_response.status(),
        reqwest::StatusCode::OK
    );
    let runtime_wrong_assignee_body: Value = runtime_wrong_assignee_response.json().await.unwrap();
    assert_eq!(runtime_wrong_assignee_body["total"], 0);

    complete_task(&client, &base_url, task_id).await;

    let historic_get_response = client
        .get(format!(
            "{base_url}/history/historic-task-instances?assignee=kermit&processDefinitionId={process_definition_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(historic_get_response.status(), reqwest::StatusCode::OK);
    let historic_get_body: Value = historic_get_response.json().await.unwrap();
    assert_eq!(historic_get_body["total"], 1);
    assert_eq!(historic_get_body["data"][0]["id"], task_id);
    assert_eq!(historic_get_body["data"][0]["assignee"], "kermit");
    assert_eq!(
        historic_get_body["data"][0]["processDefinitionId"],
        process_definition_id
    );

    let historic_post_response = client
        .post(format!("{base_url}/query/historic-task-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "processInstanceId": process_instance_id,
            "assignee": "kermit",
            "finished": true
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(historic_post_response.status(), reqwest::StatusCode::OK);
    let historic_post_body: Value = historic_post_response.json().await.unwrap();
    assert_eq!(historic_post_body["total"], 1);
    assert_eq!(historic_post_body["data"][0]["id"], task_id);
    assert_eq!(historic_post_body["data"][0]["assignee"], "kermit");

    let other_process_instance_id = other_instance["id"].as_str().unwrap();
    let historic_wrong_assignee_response = client
        .get(format!(
            "{base_url}/history/historic-task-instances?processInstanceId={other_process_instance_id}&assignee=kermit"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(
        historic_wrong_assignee_response.status(),
        reqwest::StatusCode::OK
    );
    let historic_wrong_assignee_body: Value =
        historic_wrong_assignee_response.json().await.unwrap();
    assert_eq!(historic_wrong_assignee_body["total"], 0);

    let priority_filtered_response = client
        .post(format!("{base_url}/query/tasks"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "priority": 50
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(priority_filtered_response.status(), reqwest::StatusCode::OK);
    let priority_filtered_body: Value = priority_filtered_response.json().await.unwrap();
    assert_eq!(priority_filtered_body["total"], 0);
}

#[tokio::test]
async fn task_and_historic_task_owner_round_trip_response_and_filters() {
    let (_engine, base_url, client) = spawn_server("rest-bpmn-task-owner-chain").await;
    deploy_process_with_owner(
        &client,
        &base_url,
        "ownerTaskProcess",
        "Owner approval",
        "gonzo",
    )
    .await;
    deploy_process_with_owner(
        &client,
        &base_url,
        "otherOwnerTaskProcess",
        "Other owner approval",
        "rizzo",
    )
    .await;
    let instance = start_process(&client, &base_url, "ownerTaskProcess").await;
    let other_instance = start_process(&client, &base_url, "otherOwnerTaskProcess").await;
    let process_instance_id = instance["id"].as_str().unwrap();
    let process_definition_id = instance["processDefinitionId"].as_str().unwrap();
    let task = runtime_task_for_process(&client, &base_url, process_instance_id).await;
    let task_id = task["id"].as_str().unwrap();
    assert_eq!(task["owner"], "gonzo");

    let runtime_get_response = client
        .get(format!("{base_url}/runtime/tasks?owner=gonzo"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(runtime_get_response.status(), reqwest::StatusCode::OK);
    let runtime_get_body: Value = runtime_get_response.json().await.unwrap();
    assert_eq!(runtime_get_body["total"], 1);
    assert_eq!(runtime_get_body["data"][0]["id"], task_id);
    assert_eq!(runtime_get_body["data"][0]["owner"], "gonzo");

    let runtime_post_response = client
        .post(format!("{base_url}/query/tasks"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "processInstanceId": process_instance_id,
            "owner": "gonzo"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(runtime_post_response.status(), reqwest::StatusCode::OK);
    let runtime_post_body: Value = runtime_post_response.json().await.unwrap();
    assert_eq!(runtime_post_body["total"], 1);
    assert_eq!(runtime_post_body["data"][0]["id"], task_id);
    assert_eq!(runtime_post_body["data"][0]["owner"], "gonzo");

    let runtime_wrong_owner_response = client
        .post(format!("{base_url}/query/tasks"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "processInstanceId": process_instance_id,
            "owner": "rizzo"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        runtime_wrong_owner_response.status(),
        reqwest::StatusCode::OK
    );
    let runtime_wrong_owner_body: Value = runtime_wrong_owner_response.json().await.unwrap();
    assert_eq!(runtime_wrong_owner_body["total"], 0);

    complete_task(&client, &base_url, task_id).await;

    let historic_get_response = client
        .get(format!(
            "{base_url}/history/historic-task-instances?owner=gonzo&processDefinitionId={process_definition_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(historic_get_response.status(), reqwest::StatusCode::OK);
    let historic_get_body: Value = historic_get_response.json().await.unwrap();
    assert_eq!(historic_get_body["total"], 1);
    assert_eq!(historic_get_body["data"][0]["id"], task_id);
    assert_eq!(historic_get_body["data"][0]["owner"], "gonzo");
    assert_eq!(
        historic_get_body["data"][0]["processDefinitionId"],
        process_definition_id
    );

    let historic_post_response = client
        .post(format!("{base_url}/query/historic-task-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "processInstanceId": process_instance_id,
            "owner": "gonzo",
            "finished": true
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(historic_post_response.status(), reqwest::StatusCode::OK);
    let historic_post_body: Value = historic_post_response.json().await.unwrap();
    assert_eq!(historic_post_body["total"], 1);
    assert_eq!(historic_post_body["data"][0]["id"], task_id);
    assert_eq!(historic_post_body["data"][0]["owner"], "gonzo");

    let other_process_instance_id = other_instance["id"].as_str().unwrap();
    let historic_wrong_owner_response = client
        .get(format!(
            "{base_url}/history/historic-task-instances?processInstanceId={other_process_instance_id}&owner=gonzo"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(
        historic_wrong_owner_response.status(),
        reqwest::StatusCode::OK
    );
    let historic_wrong_owner_body: Value = historic_wrong_owner_response.json().await.unwrap();
    assert_eq!(historic_wrong_owner_body["total"], 0);

    let runtime_priority_filtered_response = client
        .post(format!("{base_url}/query/tasks"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "priority": 50
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        runtime_priority_filtered_response.status(),
        reqwest::StatusCode::OK
    );
    let runtime_priority_filtered_body: Value =
        runtime_priority_filtered_response.json().await.unwrap();
    assert_eq!(runtime_priority_filtered_body["total"], 0);

    let historic_due_date_filtered_response = client
        .post(format!("{base_url}/query/historic-task-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "dueDate": "2026-01-01T00:00:00Z"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        historic_due_date_filtered_response.status(),
        reqwest::StatusCode::OK
    );
    let historic_due_date_filtered_body: Value =
        historic_due_date_filtered_response.json().await.unwrap();
    assert_eq!(historic_due_date_filtered_body["total"], 0);
}

#[tokio::test]
async fn task_and_historic_task_candidate_identity_links_response_and_filters() {
    let (_engine, base_url, client) = spawn_server("rest-bpmn-task-candidate-chain").await;
    deploy_process_with_candidates(
        &client,
        &base_url,
        "candidateTaskProcess",
        "Candidate approval",
        "kermit, gonzo",
        "management, sales",
    )
    .await;
    deploy_process_with_candidates(
        &client,
        &base_url,
        "otherCandidateTaskProcess",
        "Other candidate approval",
        "fozzie",
        "engineering",
    )
    .await;

    let instance = start_process(&client, &base_url, "candidateTaskProcess").await;
    let other_instance = start_process(&client, &base_url, "otherCandidateTaskProcess").await;
    let process_instance_id = instance["id"].as_str().unwrap();
    let process_definition_id = instance["processDefinitionId"].as_str().unwrap();
    let task = runtime_task_for_process(&client, &base_url, process_instance_id).await;
    let task_id = task["id"].as_str().unwrap();
    assert_eq!(task["candidateUsers"], json!(["kermit", "gonzo"]));
    assert_eq!(task["candidateGroups"], json!(["management", "sales"]));

    let runtime_candidate_user_get_response = client
        .get(format!("{base_url}/runtime/tasks?candidateUser=gonzo"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(
        runtime_candidate_user_get_response.status(),
        reqwest::StatusCode::OK
    );
    let runtime_candidate_user_get_body: Value =
        runtime_candidate_user_get_response.json().await.unwrap();
    assert_eq!(runtime_candidate_user_get_body["total"], 1);
    assert_eq!(runtime_candidate_user_get_body["data"][0]["id"], task_id);
    assert_eq!(
        runtime_candidate_user_get_body["data"][0]["candidateUsers"],
        json!(["kermit", "gonzo"])
    );

    let runtime_candidate_group_post_response = client
        .post(format!("{base_url}/query/tasks"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "candidateGroup": "management"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        runtime_candidate_group_post_response.status(),
        reqwest::StatusCode::OK
    );
    let runtime_candidate_group_post_body: Value =
        runtime_candidate_group_post_response.json().await.unwrap();
    assert_eq!(runtime_candidate_group_post_body["total"], 1);
    assert_eq!(runtime_candidate_group_post_body["data"][0]["id"], task_id);
    assert_eq!(
        runtime_candidate_group_post_body["data"][0]["candidateGroups"],
        json!(["management", "sales"])
    );

    let runtime_wrong_candidate_response = client
        .post(format!("{base_url}/query/tasks"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "candidateUser": "fozzie",
            "processInstanceId": process_instance_id
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        runtime_wrong_candidate_response.status(),
        reqwest::StatusCode::OK
    );
    let runtime_wrong_candidate_body: Value =
        runtime_wrong_candidate_response.json().await.unwrap();
    assert_eq!(runtime_wrong_candidate_body["total"], 0);

    let task_links_response = client
        .get(format!("{base_url}/runtime/tasks/{task_id}/identitylinks"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(task_links_response.status(), reqwest::StatusCode::OK);
    let task_links: Value = task_links_response.json().await.unwrap();
    assert!(
        task_links
            .as_array()
            .unwrap()
            .iter()
            .any(|link| link["user"] == "gonzo" && link["type"] == "candidate")
    );
    assert!(
        task_links
            .as_array()
            .unwrap()
            .iter()
            .any(|link| link["group"] == "management" && link["type"] == "candidate")
    );

    complete_task(&client, &base_url, task_id).await;

    let historic_candidate_user_get_response = client
        .get(format!(
            "{base_url}/history/historic-task-instances?candidateUser=gonzo&processDefinitionId={process_definition_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(
        historic_candidate_user_get_response.status(),
        reqwest::StatusCode::OK
    );
    let historic_candidate_user_get_body: Value =
        historic_candidate_user_get_response.json().await.unwrap();
    assert_eq!(historic_candidate_user_get_body["total"], 1);
    assert_eq!(historic_candidate_user_get_body["data"][0]["id"], task_id);
    assert_eq!(
        historic_candidate_user_get_body["data"][0]["candidateUsers"],
        json!(["kermit", "gonzo"])
    );

    let historic_candidate_group_post_response = client
        .post(format!("{base_url}/query/historic-task-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "candidateGroup": "sales",
            "finished": true
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        historic_candidate_group_post_response.status(),
        reqwest::StatusCode::OK
    );
    let historic_candidate_group_post_body: Value =
        historic_candidate_group_post_response.json().await.unwrap();
    assert_eq!(historic_candidate_group_post_body["total"], 1);
    assert_eq!(historic_candidate_group_post_body["data"][0]["id"], task_id);
    assert_eq!(
        historic_candidate_group_post_body["data"][0]["candidateGroups"],
        json!(["management", "sales"])
    );

    let other_process_instance_id = other_instance["id"].as_str().unwrap();
    let historic_wrong_candidate_response = client
        .get(format!(
            "{base_url}/history/historic-task-instances?processInstanceId={other_process_instance_id}&candidateGroup=sales"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(
        historic_wrong_candidate_response.status(),
        reqwest::StatusCode::OK
    );
    let historic_wrong_candidate_body: Value =
        historic_wrong_candidate_response.json().await.unwrap();
    assert_eq!(historic_wrong_candidate_body["total"], 0);
}

#[tokio::test]
async fn task_and_historic_task_range_queries_filter_priority_and_due_date_fields() {
    let (_engine, base_url, client) = spawn_server("rest-bpmn-task-priority-due-range-chain").await;
    deploy_process_with_priority_and_due_date(
        &client,
        &base_url,
        "lowPriorityDueRangeTaskProcess",
        "Low range approval",
        10,
        "2026-01-10T00:00:00Z",
    )
    .await;
    deploy_process_with_priority_and_due_date(
        &client,
        &base_url,
        "targetPriorityDueRangeTaskProcess",
        "Target range approval",
        50,
        "2026-01-20T00:00:00Z",
    )
    .await;
    deploy_process_with_priority_and_due_date(
        &client,
        &base_url,
        "highPriorityDueRangeTaskProcess",
        "High range approval",
        90,
        "2026-02-10T00:00:00Z",
    )
    .await;
    deploy_process(
        &client,
        &base_url,
        "withoutDueRangeTaskProcess",
        "Without due range approval",
    )
    .await;

    start_process(&client, &base_url, "lowPriorityDueRangeTaskProcess").await;
    let target_instance =
        start_process(&client, &base_url, "targetPriorityDueRangeTaskProcess").await;
    start_process(&client, &base_url, "highPriorityDueRangeTaskProcess").await;
    let without_due_instance =
        start_process(&client, &base_url, "withoutDueRangeTaskProcess").await;
    let target_process_instance_id = target_instance["id"].as_str().unwrap();
    let without_due_process_instance_id = without_due_instance["id"].as_str().unwrap();
    let target_task =
        runtime_task_for_process(&client, &base_url, target_process_instance_id).await;
    let target_task_id = target_task["id"].as_str().unwrap();
    let without_due_task =
        runtime_task_for_process(&client, &base_url, without_due_process_instance_id).await;
    let without_due_task_id = without_due_task["id"].as_str().unwrap();
    assert!(without_due_task["dueDate"].is_null());

    let runtime_get_response = client
        .get(format!(
            "{base_url}/runtime/tasks?minimumPriority=40&maximumPriority=60&dueAfter=2026-01-15T00%3A00%3A00Z&dueBefore=2026-02-01T00%3A00%3A00Z"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(runtime_get_response.status(), reqwest::StatusCode::OK);
    let runtime_get_body: Value = runtime_get_response.json().await.unwrap();
    assert_eq!(runtime_get_body["total"], 1);
    assert_eq!(runtime_get_body["data"][0]["id"], target_task_id);

    let runtime_post_response = client
        .post(format!("{base_url}/query/tasks"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "minimumPriority": 40,
            "maximumPriority": 60,
            "dueAfter": "2026-01-15T00:00:00Z",
            "dueBefore": "2026-02-01T00:00:00Z"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(runtime_post_response.status(), reqwest::StatusCode::OK);
    let runtime_post_body: Value = runtime_post_response.json().await.unwrap();
    assert_eq!(runtime_post_body["total"], 1);
    assert_eq!(runtime_post_body["data"][0]["id"], target_task_id);

    let runtime_without_due_get_response = client
        .get(format!(
            "{base_url}/runtime/tasks?withoutDueDate=true&processInstanceId={without_due_process_instance_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(
        runtime_without_due_get_response.status(),
        reqwest::StatusCode::OK
    );
    let runtime_without_due_get_body: Value =
        runtime_without_due_get_response.json().await.unwrap();
    assert_eq!(runtime_without_due_get_body["total"], 1);
    assert_eq!(
        runtime_without_due_get_body["data"][0]["id"],
        without_due_task_id
    );
    assert!(runtime_without_due_get_body["data"][0]["dueDate"].is_null());

    let runtime_without_due_post_response = client
        .post(format!("{base_url}/query/tasks"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "processInstanceId": without_due_process_instance_id,
            "withoutDueDate": true
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        runtime_without_due_post_response.status(),
        reqwest::StatusCode::OK
    );
    let runtime_without_due_post_body: Value =
        runtime_without_due_post_response.json().await.unwrap();
    assert_eq!(runtime_without_due_post_body["total"], 1);
    assert_eq!(
        runtime_without_due_post_body["data"][0]["id"],
        without_due_task_id
    );

    complete_task(&client, &base_url, target_task_id).await;
    complete_task(&client, &base_url, without_due_task_id).await;

    let historic_get_response = client
        .get(format!(
            "{base_url}/history/historic-task-instances?finished=true&minimumPriority=40&maximumPriority=60&dueAfter=2026-01-15T00%3A00%3A00Z&dueBefore=2026-02-01T00%3A00%3A00Z"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(historic_get_response.status(), reqwest::StatusCode::OK);
    let historic_get_body: Value = historic_get_response.json().await.unwrap();
    assert_eq!(historic_get_body["total"], 1);
    assert_eq!(historic_get_body["data"][0]["id"], target_task_id);

    let historic_post_response = client
        .post(format!("{base_url}/query/historic-task-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "finished": true,
            "minimumPriority": 40,
            "maximumPriority": 60,
            "dueAfter": "2026-01-15T00:00:00Z",
            "dueBefore": "2026-02-01T00:00:00Z"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(historic_post_response.status(), reqwest::StatusCode::OK);
    let historic_post_body: Value = historic_post_response.json().await.unwrap();
    assert_eq!(historic_post_body["total"], 1);
    assert_eq!(historic_post_body["data"][0]["id"], target_task_id);

    let historic_without_due_get_response = client
        .get(format!(
            "{base_url}/history/historic-task-instances?finished=true&withoutDueDate=true&processInstanceId={without_due_process_instance_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(
        historic_without_due_get_response.status(),
        reqwest::StatusCode::OK
    );
    let historic_without_due_get_body: Value =
        historic_without_due_get_response.json().await.unwrap();
    assert_eq!(historic_without_due_get_body["total"], 1);
    assert_eq!(
        historic_without_due_get_body["data"][0]["id"],
        without_due_task_id
    );
    assert!(historic_without_due_get_body["data"][0]["dueDate"].is_null());

    let historic_without_due_post_response = client
        .post(format!("{base_url}/query/historic-task-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "processInstanceId": without_due_process_instance_id,
            "finished": true,
            "withoutDueDate": true
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        historic_without_due_post_response.status(),
        reqwest::StatusCode::OK
    );
    let historic_without_due_post_body: Value =
        historic_without_due_post_response.json().await.unwrap();
    assert_eq!(historic_without_due_post_body["total"], 1);
    assert_eq!(
        historic_without_due_post_body["data"][0]["id"],
        without_due_task_id
    );
}

#[tokio::test]
async fn query_tasks_accepts_description_sort_for_get_and_post() {
    let (_engine, base_url, client) = spawn_server("rest-bpmn-query-task-description-sort").await;
    let instances = deploy_and_start_historic_task_fixture(&client, &base_url).await;

    for (process_key, description) in [
        ("historicTaskAlphaProcess", "Bravo description"),
        ("historicTaskMiddleProcess", "Charlie description"),
        ("historicTaskZuluProcess", "Alpha description"),
    ] {
        let process_instance_id = instances
            .iter()
            .find(|(key, _, _)| *key == process_key)
            .unwrap()
            .2["id"]
            .as_str()
            .unwrap();
        let task = runtime_task_for_process(&client, &base_url, process_instance_id).await;
        let task_id = task["id"].as_str().unwrap();
        update_task_description(&client, &base_url, task_id, description).await;
    }

    let get_response = client
        .get(format!(
            "{base_url}/runtime/tasks?sort=description&order=asc&start=0&size=3"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(get_response.status(), reqwest::StatusCode::OK);
    let get_body: Value = get_response.json().await.unwrap();
    assert_eq!(get_body["total"], 3);
    assert_eq!(get_body["data"][0]["description"], "Alpha description");
    assert_eq!(get_body["data"][1]["description"], "Bravo description");
    assert_eq!(get_body["data"][2]["description"], "Charlie description");

    let post_response = client
        .post(format!("{base_url}/query/tasks"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "sort": "description",
            "order": "desc",
            "start": 0,
            "size": 3
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(post_response.status(), reqwest::StatusCode::OK);
    let post_body: Value = post_response.json().await.unwrap();
    assert_eq!(post_body["total"], 3);
    assert_eq!(post_body["data"][0]["description"], "Charlie description");
    assert_eq!(post_body["data"][1]["description"], "Bravo description");
    assert_eq!(post_body["data"][2]["description"], "Alpha description");

    let override_response = client
        .post(format!(
            "{base_url}/query/tasks?sort=description&order=asc&start=0&size=1"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "sort": "description",
            "order": "desc",
            "start": 99,
            "size": 99
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(override_response.status(), reqwest::StatusCode::OK);
    let override_body: Value = override_response.json().await.unwrap();
    assert_eq!(override_body["start"], 0);
    assert_eq!(override_body["size"], 1);
    assert_eq!(override_body["total"], 3);
    assert_eq!(override_body["data"][0]["description"], "Alpha description");
}

#[tokio::test]
async fn historic_task_response_preserves_task_metadata_claim_time_and_work_time() {
    // Java parity:
    // HistoricTaskInstanceResponse exposes description/claimTime/formKey/
    // workTimeInMillis/tenantId/parentTaskId/category, and ClaimTaskCmd.java:52
    // persists CLAIM_TIME_ into the historic task row.
    let (_engine, base_url, client) = spawn_server("rest-bpmn-p34-historic-task-fields").await;
    deploy_process(
        &client,
        &base_url,
        "historicTaskFieldParityProcess",
        "Review request",
    )
    .await;
    let instance = start_process(&client, &base_url, "historicTaskFieldParityProcess").await;
    let task = runtime_task_for_process(&client, &base_url, instance["id"].as_str().unwrap()).await;
    let task_id = task["id"].as_str().unwrap();

    let update = client
        .put(format!("{base_url}/runtime/tasks/{task_id}"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "description": "Review the customer request",
            "category": "customer-review",
            "formKey": "customerReviewForm",
            "tenantId": "tenant-p34",
            "parentTaskId": "parent-task-p34"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(update.status(), reqwest::StatusCode::OK);

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

    let claimed = client
        .get(format!(
            "{base_url}/history/historic-task-instances/{task_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(claimed.status(), reqwest::StatusCode::OK);
    let claimed: Value = claimed.json().await.unwrap();
    assert_eq!(claimed["description"], "Review the customer request");
    assert_eq!(claimed["category"], "customer-review");
    assert_eq!(claimed["formKey"], "customerReviewForm");
    assert_eq!(claimed["tenantId"], "tenant-p34");
    assert_eq!(claimed["parentTaskId"], "parent-task-p34");
    assert_eq!(claimed["assignee"], "kermit");
    assert!(claimed["claimTime"].is_string());
    assert!(claimed["workTimeInMillis"].is_null());

    complete_task(&client, &base_url, task_id).await;

    let completed = client
        .get(format!(
            "{base_url}/history/historic-task-instances/{task_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(completed.status(), reqwest::StatusCode::OK);
    let completed: Value = completed.json().await.unwrap();
    assert!(completed["endTime"].is_string());
    assert!(completed["durationInMillis"].is_number());
    assert!(completed["workTimeInMillis"].as_i64().unwrap() >= 0);
}
