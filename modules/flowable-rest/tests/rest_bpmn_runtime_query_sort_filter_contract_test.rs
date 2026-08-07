use chrono::{SecondsFormat, Utc};
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_rest::run_server;
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::time::sleep;

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
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
    <process id="{process_id}" name="{process_id}" isExecutable="true">
        <startEvent id="start" />
        <sequenceFlow id="flow1" sourceRef="start" targetRef="task1" />
        <userTask id="task1" name="{task_name}" />
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

async fn task_execution_id(
    client: &reqwest::Client,
    base_url: &str,
    process_instance_id: &str,
) -> String {
    let response = client
        .get(format!(
            "{base_url}/runtime/executions?processInstanceId={process_instance_id}&activityId=task1"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["total"], 1, "body was: {body}");
    body["data"][0]["id"].as_str().unwrap().to_string()
}

async fn deploy_and_start_sort_fixture(
    client: &reqwest::Client,
    base_url: &str,
) -> Vec<(&'static str, &'static str, Value)> {
    let mut instances = Vec::new();
    for (process_key, task_name) in [
        ("piSortZuluProcess", "Zulu approval"),
        ("piSortAlphaProcess", "Alpha approval"),
        ("piSortMiddleProcess", "Middle approval"),
    ] {
        deploy_process(client, base_url, process_key, task_name).await;
        let instance = start_process(client, base_url, process_key).await;
        instances.push((process_key, task_name, instance));
    }
    instances
}

#[tokio::test]
async fn query_executions_filters_by_variables_and_process_instance_variables() {
    let (engine, base_url, client) =
        spawn_server("rest-bpmn-runtime-execution-variable-query").await;

    deploy_process(
        &client,
        &base_url,
        "executionVariableAlphaProcess",
        "Alpha variable approval",
    )
    .await;
    let alpha = start_process(&client, &base_url, "executionVariableAlphaProcess").await;
    deploy_process(
        &client,
        &base_url,
        "executionVariableBetaProcess",
        "Beta variable approval",
    )
    .await;
    let beta = start_process(&client, &base_url, "executionVariableBetaProcess").await;

    let alpha_id = alpha["id"].as_str().unwrap();
    let beta_id = beta["id"].as_str().unwrap();
    let alpha_execution_id = task_execution_id(&client, &base_url, alpha_id).await;
    let beta_execution_id = task_execution_id(&client, &base_url, beta_id).await;

    engine
        .get_variable_service()
        .set_variable(
            alpha_execution_id.clone(),
            "approval".to_string(),
            json!("Accepted"),
        )
        .unwrap();
    engine
        .get_variable_service()
        .set_variable(alpha_execution_id.clone(), "priority".to_string(), json!(7))
        .unwrap();
    engine
        .get_variable_service()
        .set_variable(
            beta_execution_id.clone(),
            "approval".to_string(),
            json!("Rejected"),
        )
        .unwrap();

    let equals = client
        .post(format!("{base_url}/query/executions"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "variables": [
                {"name": "approval", "operation": "equals", "value": "Accepted"}
            ]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(equals.status(), reqwest::StatusCode::OK);
    let equals_body: Value = equals.json().await.unwrap();
    assert_eq!(equals_body["total"], 1, "body was: {equals_body}");
    assert_eq!(equals_body["data"][0]["id"], alpha_execution_id);

    let equals_ignore_case = client
        .post(format!("{base_url}/query/executions"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "variables": [
                {"name": "approval", "operation": "equalsIgnoreCase", "value": "accepted"}
            ]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(equals_ignore_case.status(), reqwest::StatusCode::OK);
    let equals_ignore_case_body: Value = equals_ignore_case.json().await.unwrap();
    assert_eq!(equals_ignore_case_body["total"], 1);
    assert_eq!(equals_ignore_case_body["data"][0]["id"], alpha_execution_id);

    let not_equals = client
        .post(format!("{base_url}/query/executions"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "variables": [
                {"name": "approval", "operation": "notEquals", "value": "Rejected"}
            ]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(not_equals.status(), reqwest::StatusCode::OK);
    let not_equals_body: Value = not_equals.json().await.unwrap();
    assert_eq!(not_equals_body["total"], 1);
    assert_eq!(not_equals_body["data"][0]["id"], alpha_execution_id);

    let not_equals_ignore_case = client
        .post(format!("{base_url}/query/executions"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "variables": [
                {"name": "approval", "operation": "notEqualsIgnoreCase", "value": "rejected"}
            ]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(not_equals_ignore_case.status(), reqwest::StatusCode::OK);
    let not_equals_ignore_case_body: Value = not_equals_ignore_case.json().await.unwrap();
    assert_eq!(not_equals_ignore_case_body["total"], 1);
    assert_eq!(
        not_equals_ignore_case_body["data"][0]["id"],
        alpha_execution_id
    );

    let value_only_equals = client
        .post(format!("{base_url}/query/executions"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "variables": [
                {"operation": "equals", "value": 7}
            ]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(value_only_equals.status(), reqwest::StatusCode::OK);
    let value_only_body: Value = value_only_equals.json().await.unwrap();
    assert_eq!(value_only_body["total"], 1);
    assert_eq!(value_only_body["data"][0]["id"], alpha_execution_id);

    let process_instance_variables = client
        .post(format!("{base_url}/query/executions"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "processInstanceVariables": [
                {"name": "approval", "operation": "equalsIgnoreCase", "value": "accepted"}
            ]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(process_instance_variables.status(), reqwest::StatusCode::OK);
    let process_instance_variables_body: Value = process_instance_variables.json().await.unwrap();
    assert!(
        process_instance_variables_body["total"].as_u64().unwrap() >= 1,
        "body was: {process_instance_variables_body}"
    );
    assert!(
        process_instance_variables_body["data"]
            .as_array()
            .unwrap()
            .iter()
            .all(|execution| execution["processInstanceId"] == alpha_id),
        "body was: {process_instance_variables_body}"
    );

    // P108: full 10-op surface — like / likeIgnoreCase / greaterThan /
    // greaterThanOrEquals / lessThanOrEquals on the same fixture.
    let like = client
        .post(format!("{base_url}/query/executions"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "variables": [
                {"name": "approval", "operation": "like", "value": "Acc%"}
            ]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(like.status(), reqwest::StatusCode::OK);
    let like_body: Value = like.json().await.unwrap();
    assert_eq!(like_body["total"], 1, "body was: {like_body}");
    assert_eq!(like_body["data"][0]["id"], alpha_execution_id);

    let like_miss = client
        .post(format!("{base_url}/query/executions"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "variables": [
                {"name": "approval", "operation": "like", "value": "Nope%"}
            ]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(like_miss.status(), reqwest::StatusCode::OK);
    assert_eq!(like_miss.json::<Value>().await.unwrap()["total"], 0);

    let like_ignore_case = client
        .post(format!("{base_url}/query/executions"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "variables": [
                {"name": "approval", "operation": "likeIgnoreCase", "value": "acc%"}
            ]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(like_ignore_case.status(), reqwest::StatusCode::OK);
    let like_ignore_case_body: Value = like_ignore_case.json().await.unwrap();
    assert_eq!(
        like_ignore_case_body["total"],
        1,
        "body was: {like_ignore_case_body}"
    );
    assert_eq!(like_ignore_case_body["data"][0]["id"], alpha_execution_id);

    let greater_than = client
        .post(format!("{base_url}/query/executions"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "variables": [
                {"name": "priority", "operation": "greaterThan", "value": 5}
            ]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(greater_than.status(), reqwest::StatusCode::OK);
    let greater_than_body: Value = greater_than.json().await.unwrap();
    assert_eq!(greater_than_body["total"], 1, "body was: {greater_than_body}");
    assert_eq!(greater_than_body["data"][0]["id"], alpha_execution_id);

    let greater_than_miss = client
        .post(format!("{base_url}/query/executions"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "variables": [
                {"name": "priority", "operation": "greaterThan", "value": 8}
            ]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(greater_than_miss.status(), reqwest::StatusCode::OK);
    assert_eq!(greater_than_miss.json::<Value>().await.unwrap()["total"], 0);

    let less_than_or_equals = client
        .post(format!("{base_url}/query/executions"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "variables": [
                {"name": "priority", "operation": "lessThanOrEquals", "value": 7}
            ]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(less_than_or_equals.status(), reqwest::StatusCode::OK);
    let less_than_or_equals_body: Value = less_than_or_equals.json().await.unwrap();
    assert_eq!(
        less_than_or_equals_body["total"],
        1,
        "body was: {less_than_or_equals_body}"
    );
    assert_eq!(less_than_or_equals_body["data"][0]["id"], alpha_execution_id);
}

#[tokio::test]
async fn query_executions_variable_filters_return_structured_bad_request_for_errors() {
    let (_engine, base_url, client) =
        spawn_server("rest-bpmn-runtime-execution-variable-query-errors").await;

    for (request_body, expected_detail) in [
        (
            json!({"variables": [{"name": "approval", "value": "Accepted"}]}),
            "Variable operation is missing for variable: approval",
        ),
        (
            json!({"variables": [{"name": "approval", "operation": "equals"}]}),
            "Variable value is missing for variable: approval",
        ),
        (
            json!({"variables": [{"operation": "notEquals", "value": "Accepted"}]}),
            "Value-only query (without a variable-name) is only supported when using 'equals' operation.",
        ),
        (
            json!({"variables": [{"name": "approval", "operation": "equalsIgnoreCase", "value": 7}]}),
            "Only string variable values are supported when ignoring casing",
        ),
        (
            json!({"variables": [{"name": "approval", "operation": "bogusOp", "value": "Accept"}]}),
            "Unsupported variable query operation: bogusOp",
        ),
        (
            json!({"variables": [{"name": "priority", "operation": "greaterThan", "value": true}]}),
            "Booleans and null cannot be used in 'greater than' condition",
        ),
        (
            json!({"variables": [{"name": "approval", "operation": "like", "value": 7}]}),
            "Only string variable values are supported using like",
        ),
    ] {
        let response = client
            .post(format!("{base_url}/query/executions"))
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
async fn runtime_process_instances_accept_sort_order_and_page_after_sorting() {
    let (_engine, base_url, client) = spawn_server("rest-bpmn-runtime-process-instance-sort").await;
    let instances = deploy_and_start_sort_fixture(&client, &base_url).await;

    let mut expected_ids = instances
        .iter()
        .map(|(_, _, instance)| instance["id"].as_str().unwrap().to_string())
        .collect::<Vec<_>>();
    expected_ids.sort();
    expected_ids.reverse();

    let response = client
        .get(format!(
            "{base_url}/runtime/process-instances?sort=id&order=desc&start=0&size=2"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["start"], 0);
    assert_eq!(body["size"], 2);
    assert_eq!(body["total"], 3);
    assert_eq!(body["data"][0]["id"], expected_ids[0]);
    assert_eq!(body["data"][1]["id"], expected_ids[1]);
}

#[tokio::test]
async fn query_process_instances_accepts_body_sort_and_url_paging_overrides_body_paging() {
    let (_engine, base_url, client) = spawn_server("rest-bpmn-query-process-instance-sort").await;
    let instances = deploy_and_start_sort_fixture(&client, &base_url).await;

    let mut expected_ids_desc = instances
        .iter()
        .map(|(_, _, instance)| instance["id"].as_str().unwrap().to_string())
        .collect::<Vec<_>>();
    expected_ids_desc.sort();
    expected_ids_desc.reverse();

    let mut expected_by_key = instances
        .iter()
        .map(|(process_key, _, instance)| {
            (
                *process_key,
                instance["processDefinitionId"]
                    .as_str()
                    .unwrap()
                    .to_string(),
            )
        })
        .collect::<Vec<_>>();
    expected_by_key.sort_by(|left, right| left.0.cmp(right.0));

    let response = client
        .post(format!("{base_url}/query/process-instances?start=1&size=1"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "start": 0,
            "size": 3,
            "sort": "processDefinitionKey",
            "order": "asc"
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["start"], 1);
    assert_eq!(body["size"], 1);
    assert_eq!(body["total"], 3);
    assert_eq!(body["data"][0]["processDefinitionId"], expected_by_key[1].1);

    let url_sort_override = client
        .post(format!(
            "{base_url}/query/process-instances?start=0&size=1&sort=id&order=desc"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "start": 0,
            "size": 3,
            "sort": "processDefinitionKey",
            "order": "asc"
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(url_sort_override.status(), reqwest::StatusCode::OK);
    let url_sort_override_body: Value = url_sort_override.json().await.unwrap();
    assert_eq!(url_sort_override_body["start"], 0);
    assert_eq!(url_sort_override_body["size"], 1);
    assert_eq!(url_sort_override_body["total"], 3);
    assert_eq!(
        url_sort_override_body["data"][0]["id"],
        expected_ids_desc[0]
    );
}

#[tokio::test]
async fn runtime_process_instances_accept_metadata_filters_and_sort_aliases() {
    let (engine, base_url, client) =
        spawn_server("rest-bpmn-runtime-process-instance-metadata-filters").await;

    let started_after =
        (Utc::now() - chrono::Duration::days(1)).to_rfc3339_opts(SecondsFormat::Secs, true);
    deploy_process(
        &client,
        &base_url,
        "piRuntimeQueryAlphaProcess",
        "Alpha runtime approval",
    )
    .await;
    let alpha = start_process(&client, &base_url, "piRuntimeQueryAlphaProcess").await;
    sleep(Duration::from_millis(5)).await;
    deploy_process(
        &client,
        &base_url,
        "piRuntimeQueryBetaProcess",
        "Beta runtime approval",
    )
    .await;
    let beta = start_process(&client, &base_url, "piRuntimeQueryBetaProcess").await;
    let started_before =
        (Utc::now() + chrono::Duration::days(1)).to_rfc3339_opts(SecondsFormat::Secs, true);

    let alpha_id = alpha["id"].as_str().unwrap();
    let alpha_definition_id = alpha["processDefinitionId"].as_str().unwrap();
    let beta_id = beta["id"].as_str().unwrap();

    let update_alpha = client
        .put(format!("{base_url}/runtime/process-instances/{alpha_id}"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "Alpha runtime instance",
            "businessStatus": "Approved",
            "callbackId": "cb-alpha",
            "callbackType": "rest"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(update_alpha.status(), reqwest::StatusCode::OK);

    let by_id = client
        .get(format!(
            "{base_url}/runtime/process-instances?id={alpha_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(by_id.status(), reqwest::StatusCode::OK);
    let by_id_body: Value = by_id.json().await.unwrap();
    assert_eq!(by_id_body["total"], 1);
    assert_eq!(by_id_body["data"][0]["id"], alpha_id);

    let metadata_filtered = client
        .get(format!(
            "{base_url}/runtime/process-instances?nameLikeIgnoreCase=%25alpha%25&processDefinitionNameLikeIgnoreCase=%25runtimequeryalpha%25&processDefinitionKeyLikeIgnoreCase=%25RUNTIMEQUERYALPHA%25&processDefinitionVersion=1&businessKeyLikeIgnoreCase=%25ALPHA%25&businessStatusLikeIgnoreCase=%25approv%25&callbackId=cb-alpha&callbackType=rest&startedAfter={started_after}&startedBefore={started_before}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(metadata_filtered.status(), reqwest::StatusCode::OK);
    let metadata_body: Value = metadata_filtered.json().await.unwrap();
    assert_eq!(metadata_body["total"], 1);
    assert_eq!(metadata_body["data"][0]["id"], alpha_id);
    assert_eq!(
        metadata_body["data"][0]["processDefinitionId"],
        alpha_definition_id
    );
    assert_eq!(metadata_body["data"][0]["businessStatus"], "Approved");
    assert_eq!(metadata_body["data"][0]["callbackId"], "cb-alpha");
    assert_eq!(metadata_body["data"][0]["callbackType"], "rest");

    let start_time_sorted = client
        .get(format!(
            "{base_url}/runtime/process-instances?processDefinitionKeyLike=piRuntimeQuery%25&sort=startTime&order=desc&start=0&size=1"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(start_time_sorted.status(), reqwest::StatusCode::OK);
    let sorted_body: Value = start_time_sorted.json().await.unwrap();
    assert_eq!(sorted_body["total"], 2);
    assert_eq!(sorted_body["data"][0]["id"], beta_id);

    engine
        .get_repository_service()
        .deploy(
            engine
                .get_repository_service()
                .create_deployment()
                .name("Tenant query deployment".to_string())
                .tenant_id("tenant-a".to_string())
                .add_string(
                    "tenant_query_process.bpmn20.xml".to_string(),
                    user_task_process_xml("piRuntimeTenantQueryProcess", "Tenant approval"),
                ),
        )
        .unwrap();
    let tenant_start = client
        .post(format!("{base_url}/runtime/process-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "processDefinitionKey": "piRuntimeTenantQueryProcess",
            "tenantId": "tenant-a"
        }))
        .send()
        .await
        .unwrap();
    assert!(tenant_start.status().is_success(), "{tenant_start:?}");
    let tenant_instance: Value = tenant_start.json().await.unwrap();

    let by_tenant = client
        .get(format!(
            "{base_url}/runtime/process-instances?tenantIdLikeIgnoreCase=TENANT-A&sort=tenantId&order=asc"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(by_tenant.status(), reqwest::StatusCode::OK);
    let by_tenant_body: Value = by_tenant.json().await.unwrap();
    assert_eq!(by_tenant_body["total"], 1);
    assert_eq!(by_tenant_body["data"][0]["id"], tenant_instance["id"]);

    let query_body = client
        .post(format!("{base_url}/query/process-instances?start=0&size=1"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "processDefinitionKeyLikeIgnoreCase": "%runtimequery%",
            "sort": "startTime",
            "order": "desc",
            "start": 99,
            "size": 99
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(query_body.status(), reqwest::StatusCode::OK);
    let query_body_json: Value = query_body.json().await.unwrap();
    assert_eq!(query_body_json["start"], 0);
    assert_eq!(query_body_json["size"], 1);
    assert_eq!(query_body_json["total"], 2);
    assert_eq!(query_body_json["data"][0]["id"], beta_id);

    let tenant_conflict = client
        .get(format!(
            "{base_url}/runtime/process-instances?tenantId=tenant-a&withoutTenantId=true"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(tenant_conflict.status(), reqwest::StatusCode::BAD_REQUEST);
    let tenant_conflict_body: Value = tenant_conflict.json().await.unwrap();
    assert_eq!(tenant_conflict_body["code"], "BAD_REQUEST");
    assert!(
        tenant_conflict_body["details"]
            .as_str()
            .unwrap()
            .contains("tenantId and withoutTenantId"),
        "details were: {}",
        tenant_conflict_body["details"]
    );
}

#[tokio::test]
async fn runtime_executions_accept_canonical_filters_sorting_and_query_body() {
    let (engine, base_url, client) = spawn_server("rest-bpmn-runtime-execution-filters").await;

    deploy_process(
        &client,
        &base_url,
        "executionQueryAlphaProcess",
        "Alpha execution approval",
    )
    .await;
    let alpha = start_process(&client, &base_url, "executionQueryAlphaProcess").await;
    deploy_process(
        &client,
        &base_url,
        "executionQueryBetaProcess",
        "Beta execution approval",
    )
    .await;
    let beta = start_process(&client, &base_url, "executionQueryBetaProcess").await;

    let alpha_id = alpha["id"].as_str().unwrap();
    let alpha_definition_id = alpha["processDefinitionId"].as_str().unwrap();
    let beta_id = beta["id"].as_str().unwrap();

    let alpha_task_executions = client
        .get(format!(
            "{base_url}/runtime/executions?processInstanceBusinessKey=executionQueryAlphaProcess-business-key&processDefinitionKey=executionQueryAlphaProcess&processDefinitionId={alpha_definition_id}&activityId=task1"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(alpha_task_executions.status(), reqwest::StatusCode::OK);
    let alpha_task_body: Value = alpha_task_executions.json().await.unwrap();
    assert_eq!(alpha_task_body["total"], 1);
    let alpha_execution_id = alpha_task_body["data"][0]["id"].as_str().unwrap();
    assert_eq!(alpha_task_body["data"][0]["processInstanceId"], alpha_id);
    assert_eq!(
        alpha_task_body["data"][0]["processDefinitionId"],
        alpha_definition_id
    );
    assert_eq!(alpha_task_body["data"][0]["activityId"], "task1");

    let by_id = client
        .get(format!(
            "{base_url}/runtime/executions?id={alpha_execution_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(by_id.status(), reqwest::StatusCode::OK);
    let by_id_body: Value = by_id.json().await.unwrap();
    assert_eq!(by_id_body["total"], 1);
    assert_eq!(by_id_body["data"][0]["id"], alpha_execution_id);

    let by_instance_ids = client
        .get(format!(
            "{base_url}/runtime/executions?processInstanceIds={alpha_id},{beta_id}&sort=processDefinitionKey&order=asc"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(by_instance_ids.status(), reqwest::StatusCode::OK);
    let by_instance_ids_body: Value = by_instance_ids.json().await.unwrap();
    assert!(by_instance_ids_body["total"].as_u64().unwrap() >= 2);
    assert!(
        by_instance_ids_body["data"]
            .as_array()
            .unwrap()
            .iter()
            .all(|execution| {
                execution["processInstanceId"] == alpha_id
                    || execution["processInstanceId"] == beta_id
            }),
        "body was: {by_instance_ids_body}"
    );

    engine
        .get_repository_service()
        .deploy(
            engine
                .get_repository_service()
                .create_deployment()
                .name("Tenant execution deployment".to_string())
                .tenant_id("tenant-a".to_string())
                .add_string(
                    "tenant_execution_process.bpmn20.xml".to_string(),
                    user_task_process_xml("executionTenantQueryProcess", "Tenant execution"),
                ),
        )
        .unwrap();
    let tenant_start = client
        .post(format!("{base_url}/runtime/process-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "processDefinitionKey": "executionTenantQueryProcess",
            "tenantId": "tenant-a"
        }))
        .send()
        .await
        .unwrap();
    assert!(tenant_start.status().is_success(), "{tenant_start:?}");
    let tenant_instance: Value = tenant_start.json().await.unwrap();

    let by_tenant = client
        .get(format!(
            "{base_url}/runtime/executions?tenantIdLike=tenant%25&sort=tenantId&order=asc"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(by_tenant.status(), reqwest::StatusCode::OK);
    let by_tenant_body: Value = by_tenant.json().await.unwrap();
    assert!(by_tenant_body["total"].as_u64().unwrap() >= 1);
    assert!(
        by_tenant_body["data"]
            .as_array()
            .unwrap()
            .iter()
            .all(|execution| execution["processInstanceId"] == tenant_instance["id"]),
        "body was: {by_tenant_body}"
    );

    let mut expected_execution_ids_desc = engine
        .get_runtime_store()
        .db_store()
        .find_all::<flowable_engine::runtime::execution::Execution>("executions")
        .unwrap()
        .into_iter()
        .filter(|execution| {
            execution.process_instance_id.as_deref() == Some(alpha_id)
                || execution.process_instance_id.as_deref() == Some(beta_id)
        })
        .map(|execution| execution.id)
        .collect::<Vec<_>>();
    expected_execution_ids_desc.sort();
    expected_execution_ids_desc.reverse();

    let query_body = client
        .post(format!(
            "{base_url}/query/executions?start=0&size=1&sort=id&order=desc"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "processInstanceIds": [alpha_id, beta_id],
            "sort": "processInstanceId",
            "order": "asc",
            "start": 99,
            "size": 99
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(query_body.status(), reqwest::StatusCode::OK);
    let query_body_json: Value = query_body.json().await.unwrap();
    assert_eq!(query_body_json["start"], 0);
    assert_eq!(query_body_json["size"], 1);
    assert!(query_body_json["total"].as_u64().unwrap() >= 2);
    assert_eq!(
        query_body_json["data"][0]["id"],
        expected_execution_ids_desc[0]
    );

    let tenant_conflict = client
        .get(format!(
            "{base_url}/runtime/executions?tenantId=tenant-a&withoutTenantId=true"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(tenant_conflict.status(), reqwest::StatusCode::BAD_REQUEST);
    let tenant_conflict_body: Value = tenant_conflict.json().await.unwrap();
    assert_eq!(tenant_conflict_body["code"], "BAD_REQUEST");
    assert!(
        tenant_conflict_body["details"]
            .as_str()
            .unwrap()
            .contains("tenantId and withoutTenantId"),
        "details were: {}",
        tenant_conflict_body["details"]
    );
}

#[tokio::test]
async fn runtime_variable_instances_accept_supported_filters_sorting_and_query_body() {
    let (engine, base_url, client) =
        spawn_server("rest-bpmn-runtime-variable-instance-query").await;

    deploy_process(
        &client,
        &base_url,
        "variableQueryAlphaProcess",
        "Alpha variable task",
    )
    .await;
    let alpha = start_process(&client, &base_url, "variableQueryAlphaProcess").await;
    deploy_process(
        &client,
        &base_url,
        "variableQueryBetaProcess",
        "Beta variable task",
    )
    .await;
    let beta = start_process(&client, &base_url, "variableQueryBetaProcess").await;

    let alpha_id = alpha["id"].as_str().unwrap();
    let beta_id = beta["id"].as_str().unwrap();
    let alpha_execution_id = task_execution_id(&client, &base_url, alpha_id).await;
    let beta_execution_id = task_execution_id(&client, &base_url, beta_id).await;

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
            "{base_url}/runtime/variable-instances?variableNameLike=approval&variableType=string&sort=variableName&order=asc"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(get_filtered.status(), reqwest::StatusCode::OK);
    let get_filtered_body: Value = get_filtered.json().await.unwrap();
    assert_eq!(
        get_filtered_body["total"], 2,
        "body was: {get_filtered_body}"
    );
    assert_eq!(get_filtered_body["data"][0]["name"], "approvalAlpha");
    assert_eq!(get_filtered_body["data"][0]["type"], "string");
    assert_eq!(get_filtered_body["data"][0]["processInstanceId"], alpha_id);
    assert_eq!(get_filtered_body["data"][1]["name"], "approvalBeta");
    assert_eq!(get_filtered_body["data"][1]["processInstanceId"], beta_id);

    let post_filtered = client
        .post(format!(
            "{base_url}/query/variable-instances?start=0&size=1"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "variableNameLike": "approval",
            "type": "string",
            "sort": "name",
            "order": "desc",
            "start": 99,
            "size": 99
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(post_filtered.status(), reqwest::StatusCode::OK);
    let post_filtered_body: Value = post_filtered.json().await.unwrap();
    assert_eq!(post_filtered_body["start"], 0);
    assert_eq!(post_filtered_body["size"], 1);
    assert_eq!(post_filtered_body["total"], 2);
    assert_eq!(post_filtered_body["data"][0]["name"], "approvalBeta");
    assert_eq!(post_filtered_body["data"][0]["type"], "string");
}

#[tokio::test]
async fn runtime_variable_instances_include_task_local_variables_and_task_filters() {
    let (_engine, base_url, client) =
        spawn_server("rest-bpmn-runtime-variable-instance-task-query").await;

    deploy_process(
        &client,
        &base_url,
        "variableTaskLocalQueryProcess",
        "Task local variable task",
    )
    .await;
    let instance = start_process(&client, &base_url, "variableTaskLocalQueryProcess").await;
    let process_instance_id = instance["id"].as_str().unwrap();

    let task_response = client
        .get(format!(
            "{base_url}/runtime/tasks?processInstanceId={process_instance_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(task_response.status(), reqwest::StatusCode::OK);
    let task_body: Value = task_response.json().await.unwrap();
    assert_eq!(task_body["total"], 1, "body was: {task_body}");
    let task_id = task_body["data"][0]["id"].as_str().unwrap();

    let create_task_variable = client
        .post(format!(
            "{base_url}/runtime/tasks/{task_id}/variables?scope=local"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "taskLocalApproval",
            "type": "string",
            "value": "task-local"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(create_task_variable.status(), reqwest::StatusCode::CREATED);

    let by_task_id = client
        .get(format!(
            "{base_url}/runtime/variable-instances?taskId={task_id}&sort=taskId&order=asc"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(by_task_id.status(), reqwest::StatusCode::OK);
    let by_task_id_body: Value = by_task_id.json().await.unwrap();
    assert_eq!(by_task_id_body["total"], 1, "body was: {by_task_id_body}");
    assert_eq!(by_task_id_body["data"][0]["name"], "taskLocalApproval");
    assert_eq!(by_task_id_body["data"][0]["value"], "task-local");
    assert_eq!(by_task_id_body["data"][0]["taskId"], task_id);
    assert_eq!(
        by_task_id_body["data"][0]["processInstanceId"],
        process_instance_id
    );

    let excluded = client
        .post(format!("{base_url}/query/variable-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "variableName": "taskLocalApproval",
            "excludeTaskVariables": true
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(excluded.status(), reqwest::StatusCode::OK);
    let excluded_body: Value = excluded.json().await.unwrap();
    assert_eq!(excluded_body["total"], 0, "body was: {excluded_body}");

    let excluded_local = client
        .get(format!(
            "{base_url}/runtime/variable-instances?variableName=taskLocalApproval&excludeLocalVariables=true"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(excluded_local.status(), reqwest::StatusCode::OK);
    let excluded_local_body: Value = excluded_local.json().await.unwrap();
    assert_eq!(
        excluded_local_body["total"], 0,
        "body was: {excluded_local_body}"
    );

    let post_url_sort_override = client
        .post(format!(
            "{base_url}/query/variable-instances?sort=taskId&order=desc"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "taskId": task_id,
            "sort": "variableName",
            "order": "asc"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(post_url_sort_override.status(), reqwest::StatusCode::OK);
    let post_url_sort_override_body: Value = post_url_sort_override.json().await.unwrap();
    assert_eq!(
        post_url_sort_override_body["total"], 1,
        "body was: {post_url_sort_override_body}"
    );
    assert_eq!(post_url_sort_override_body["data"][0]["taskId"], task_id);
}

#[tokio::test]
async fn runtime_variable_instances_return_structured_bad_request_for_invalid_sort_order() {
    let (_engine, base_url, client) =
        spawn_server("rest-bpmn-runtime-variable-instance-sort-errors").await;

    let bad_sort = client
        .get(format!(
            "{base_url}/runtime/variable-instances?sort=unsupported"
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
            .contains("unsupported"),
        "details were: {}",
        bad_sort_body["details"]
    );

    let bad_order = client
        .post(format!("{base_url}/query/variable-instances"))
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
async fn runtime_tasks_filter_by_task_id_and_name_and_return_structured_bad_order() {
    let (_engine, base_url, client) = spawn_server("rest-bpmn-runtime-task-filter").await;
    let instances = deploy_and_start_sort_fixture(&client, &base_url).await;
    let alpha_process_instance_id = instances
        .iter()
        .find(|(process_key, _, _)| *process_key == "piSortAlphaProcess")
        .unwrap()
        .2["id"]
        .as_str()
        .unwrap()
        .to_string();

    let alpha_tasks = client
        .get(format!(
            "{base_url}/runtime/tasks?processInstanceId={alpha_process_instance_id}&name=Alpha approval"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(alpha_tasks.status(), reqwest::StatusCode::OK);
    let alpha_tasks_body: Value = alpha_tasks.json().await.unwrap();
    assert_eq!(alpha_tasks_body["total"], 1);
    assert_eq!(alpha_tasks_body["data"][0]["name"], "Alpha approval");
    assert_eq!(
        alpha_tasks_body["data"][0]["processInstanceId"],
        alpha_process_instance_id
    );
    let alpha_task_id = alpha_tasks_body["data"][0]["id"].as_str().unwrap();

    let by_task_id = client
        .get(format!("{base_url}/runtime/tasks?taskId={alpha_task_id}"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(by_task_id.status(), reqwest::StatusCode::OK);
    let by_task_id_body: Value = by_task_id.json().await.unwrap();
    assert_eq!(by_task_id_body["total"], 1);
    assert_eq!(by_task_id_body["data"][0]["id"], alpha_task_id);

    let bad_order = client
        .get(format!("{base_url}/runtime/tasks?order=sideways"))
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
