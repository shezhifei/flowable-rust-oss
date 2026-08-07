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
    user_task_process_xml_with_task_id(process_id, "task1", task_name)
}

fn user_task_process_xml_with_task_id(process_id: &str, task_id: &str, task_name: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
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

async fn deploy_process_via_rest(
    client: &reqwest::Client,
    base_url: &str,
    process_id: &str,
    task_name: &str,
) {
    deploy_process_xml_via_rest(
        client,
        base_url,
        process_id,
        user_task_process_xml(process_id, task_name),
    )
    .await
}

async fn deploy_process_xml_via_rest(
    client: &reqwest::Client,
    base_url: &str,
    process_id: &str,
    resource: String,
) {
    let response = client
        .post(format!("{base_url}/repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": format!("{process_id} deployment"),
            "resourceName": format!("{process_id}.bpmn20.xml"),
            "resource": resource
        }))
        .send()
        .await
        .unwrap();

    assert!(response.status().is_success(), "{response:?}");
}

async fn start_by_key(
    client: &reqwest::Client,
    base_url: &str,
    process_definition_key: &str,
    business_key: &str,
) -> Value {
    let response = client
        .post(format!("{base_url}/runtime/process-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "processDefinitionKey": process_definition_key,
            "businessKey": business_key
        }))
        .send()
        .await
        .unwrap();

    assert!(response.status().is_success(), "{response:?}");
    response.json().await.unwrap()
}

async fn deploy_sort_fixture(client: &reqwest::Client, base_url: &str) {
    for (process_id, task_id, task_name, business_key) in [
        (
            "taskSortAlphaProcess",
            "alphaReviewTask",
            "Alpha approval",
            "task-sort-alpha",
        ),
        (
            "taskSortMiddleProcess",
            "middleReviewTask",
            "Middle approval",
            "task-sort-middle",
        ),
        (
            "taskSortZuluProcess",
            "zuluReviewTask",
            "Zulu approval",
            "task-sort-zulu",
        ),
    ] {
        deploy_process_xml_via_rest(
            client,
            base_url,
            process_id,
            user_task_process_xml_with_task_id(process_id, task_id, task_name),
        )
        .await;
        start_by_key(client, base_url, process_id, business_key).await;
        tokio::time::sleep(std::time::Duration::from_millis(2)).await;
    }
}

async fn task_page(client: &reqwest::Client, base_url: &str, query: &str) -> Value {
    let response = client
        .get(format!("{base_url}/runtime/tasks?{query}"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), reqwest::StatusCode::OK);
    response.json().await.unwrap()
}

async fn update_task_metadata(
    client: &reqwest::Client,
    base_url: &str,
    task_id: &str,
    assignee: &str,
    owner: &str,
    category: &str,
    tenant_id: &str,
) {
    let response = client
        .put(format!("{base_url}/runtime/tasks/{task_id}"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "assignee": assignee,
            "owner": owner,
            "category": category,
            "tenantId": tenant_id
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), reqwest::StatusCode::OK);
}

fn task_names(page: &Value) -> Vec<&str> {
    page["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|task| task["name"].as_str().unwrap())
        .collect()
}

#[tokio::test]
async fn runtime_tasks_accept_sort_order_and_apply_paging_after_sorting() {
    let (_engine, base_url, client) = spawn_server("rest-bpmn-task-sort-contract").await;
    deploy_sort_fixture(&client, &base_url).await;

    let response = client
        .get(format!(
            "{base_url}/runtime/tasks?sort=name&order=desc&start=0&size=2"
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
    assert_eq!(body["data"][0]["name"], "Zulu approval");
    assert_eq!(body["data"][1]["name"], "Middle approval");
}

#[tokio::test]
async fn query_tasks_accepts_body_sort_and_url_paging_overrides_body_paging() {
    let (_engine, base_url, client) = spawn_server("rest-bpmn-query-task-sort-contract").await;
    deploy_sort_fixture(&client, &base_url).await;

    let response = client
        .post(format!("{base_url}/query/tasks?start=1&size=1"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "start": 0,
            "size": 3,
            "sort": "name",
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
    assert_eq!(body["data"][0]["name"], "Middle approval");
}

#[tokio::test]
async fn runtime_and_query_tasks_filter_metadata_and_sort_canonical_fields() {
    let (_engine, base_url, client) = spawn_server("rest-bpmn-task-metadata-query-contract").await;
    deploy_sort_fixture(&client, &base_url).await;

    let all_tasks = task_page(&client, &base_url, "sort=name&order=asc").await;
    assert_eq!(
        task_names(&all_tasks),
        vec!["Alpha approval", "Middle approval", "Zulu approval"]
    );
    let alpha_id = all_tasks["data"][0]["id"].as_str().unwrap();
    let middle_id = all_tasks["data"][1]["id"].as_str().unwrap();
    let zulu_id = all_tasks["data"][2]["id"].as_str().unwrap();

    update_task_metadata(
        &client, &base_url, alpha_id, "alice", "owner-c", "approval", "tenant-a",
    )
    .await;
    update_task_metadata(
        &client, &base_url, middle_id, "bob", "owner-b", "review", "tenant-b",
    )
    .await;
    update_task_metadata(
        &client, &base_url, zulu_id, "carol", "owner-a", "approval", "tenant-a",
    )
    .await;

    let runtime_filtered = task_page(
        &client,
        &base_url,
        "category=approval&tenantId=tenant-a&sort=assignee&order=desc",
    )
    .await;
    assert_eq!(runtime_filtered["total"], 2);
    assert_eq!(
        task_names(&runtime_filtered),
        vec!["Zulu approval", "Alpha approval"]
    );

    let query_filtered = client
        .post(format!("{base_url}/query/tasks"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "category": "approval",
            "tenantId": "tenant-a",
            "sort": "owner",
            "order": "asc"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(query_filtered.status(), reqwest::StatusCode::OK);
    let query_filtered_body: Value = query_filtered.json().await.unwrap();
    assert_eq!(query_filtered_body["total"], 2);
    assert_eq!(
        task_names(&query_filtered_body),
        vec!["Zulu approval", "Alpha approval"]
    );

    let by_definition_key = task_page(
        &client,
        &base_url,
        "sort=taskDefinitionKey&order=desc&start=0&size=3",
    )
    .await;
    assert_eq!(
        task_names(&by_definition_key),
        vec!["Zulu approval", "Middle approval", "Alpha approval"]
    );

    let by_create_time = task_page(
        &client,
        &base_url,
        "sort=createTime&order=desc&start=0&size=1",
    )
    .await;
    assert_eq!(by_create_time["total"], 3);
    assert_eq!(task_names(&by_create_time), vec!["Zulu approval"]);

    let bad_sort = client
        .post(format!("{base_url}/query/tasks"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "sort": "unsupportedTaskSort",
            "order": "asc"
        }))
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
            .contains("unsupportedTaskSort"),
        "details were: {}",
        bad_sort_body["details"]
    );
}

#[tokio::test]
async fn process_definitions_contract_covers_tenant_latest_sort_and_structured_errors() {
    let (engine, base_url, client) =
        spawn_server("rest-bpmn-process-definition-query-contract").await;

    for version in [1, 2] {
        engine
            .get_repository_service()
            .deploy(
                engine
                    .get_repository_service()
                    .create_deployment()
                    .name(format!("Definition latest v{version}"))
                    .add_string(
                        format!("definition-latest-v{version}.bpmn20.xml"),
                        user_task_process_xml("definitionLatestProcess", "Definition task"),
                    ),
            )
            .unwrap();
    }
    engine
        .get_repository_service()
        .deploy(
            engine
                .get_repository_service()
                .create_deployment()
                .name("Definition latest tenant".to_string())
                .tenant_id("tenant-a".to_string())
                .add_string(
                    "definition-latest-tenant.bpmn20.xml".to_string(),
                    user_task_process_xml("definitionLatestProcess", "Tenant definition task"),
                ),
        )
        .unwrap();

    for definition in engine
        .get_repository_service()
        .get_process_definitions()
        .unwrap()
    {
        let category = match definition.resource_name.as_deref() {
            Some("definition-latest-v1.bpmn20.xml") => Some("alpha-category".to_string()),
            Some("definition-latest-v2.bpmn20.xml") => Some("beta-category".to_string()),
            Some("definition-latest-tenant.bpmn20.xml") => Some("tenant-category".to_string()),
            _ => None,
        };
        if let Some(category) = category {
            engine
                .get_repository_service()
                .update_process_definition_category(&definition.id, Some(category))
                .unwrap();
        }
    }

    let without_tenant = client
        .get(format!(
            "{base_url}/repository/process-definitions?withoutTenantId=true&latest=true&sort=version&order=desc&start=0&size=1"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(without_tenant.status(), reqwest::StatusCode::OK);
    let without_tenant_body: Value = without_tenant.json().await.unwrap();
    assert_eq!(without_tenant_body["start"], 0);
    assert_eq!(without_tenant_body["size"], 1);
    assert_eq!(without_tenant_body["total"], 1);
    assert_eq!(
        without_tenant_body["data"][0]["key"],
        "definitionLatestProcess"
    );
    assert_eq!(without_tenant_body["data"][0]["version"], 2);
    assert!(without_tenant_body["data"][0]["tenantId"].is_null());

    let tenant = client
        .get(format!(
            "{base_url}/repository/process-definitions?tenantId=tenant-a&latest=true&sort=tenantId&order=asc"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(tenant.status(), reqwest::StatusCode::OK);
    let tenant_body: Value = tenant.json().await.unwrap();
    assert_eq!(tenant_body["total"], 1);
    assert_eq!(tenant_body["data"][0]["tenantId"], "tenant-a");
    assert_eq!(tenant_body["data"][0]["key"], "definitionLatestProcess");

    let tenant_like = client
        .get(format!(
            "{base_url}/repository/process-definitions?tenantIdLike=tenant%&sort=tenantId&order=asc"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(tenant_like.status(), reqwest::StatusCode::OK);
    let tenant_like_body: Value = tenant_like.json().await.unwrap();
    assert_eq!(tenant_like_body["total"], 1);
    assert_eq!(tenant_like_body["data"][0]["tenantId"], "tenant-a");

    let category_resource = client
        .get(format!(
            "{base_url}/repository/process-definitions?categoryLike=beta%&nameLikeIgnoreCase=definitionlatest%&resourceNameLike=%v2.bpmn20%&sort=category&order=asc"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(category_resource.status(), reqwest::StatusCode::OK);
    let category_resource_body: Value = category_resource.json().await.unwrap();
    assert_eq!(category_resource_body["total"], 1);
    assert_eq!(
        category_resource_body["data"][0]["category"],
        "beta-category"
    );
    assert_eq!(
        category_resource_body["data"][0]["resourceName"],
        "definition-latest-v2.bpmn20.xml"
    );

    let category_not_equals = client
        .get(format!(
            "{base_url}/repository/process-definitions?categoryNotEquals=tenant-category&resourceName=definition-latest-v1.bpmn20.xml"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(category_not_equals.status(), reqwest::StatusCode::OK);
    let category_not_equals_body: Value = category_not_equals.json().await.unwrap();
    assert_eq!(category_not_equals_body["total"], 1);
    assert_eq!(
        category_not_equals_body["data"][0]["resourceName"],
        "definition-latest-v1.bpmn20.xml"
    );

    let parent_deployment_id = "definition-query-parent-deployment";
    let child_deployment = engine
        .get_repository_service()
        .deploy(
            engine
                .get_repository_service()
                .create_deployment()
                .name("Definition query child".to_string())
                .parent_deployment_id(parent_deployment_id.to_string())
                .add_string(
                    "definition-query-child.bpmn20.xml".to_string(),
                    user_task_process_xml("definitionQueryChildProcess", "Child task"),
                ),
        )
        .unwrap();
    engine
        .get_repository_service()
        .deploy(
            engine
                .get_repository_service()
                .create_deployment()
                .name("Definition query peer".to_string())
                .parent_deployment_id("other-parent-deployment".to_string())
                .add_string(
                    "definition-query-peer.bpmn20.xml".to_string(),
                    user_task_process_xml("definitionQueryPeerProcess", "Peer task"),
                ),
        )
        .unwrap();

    let by_parent_deployment = client
        .get(format!(
            "{base_url}/repository/process-definitions?parentDeploymentId={parent_deployment_id}&sort=deploymentId&order=asc"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(by_parent_deployment.status(), reqwest::StatusCode::OK);
    let by_parent_deployment_body: Value = by_parent_deployment.json().await.unwrap();
    assert_eq!(by_parent_deployment_body["total"], 1);
    assert_eq!(
        by_parent_deployment_body["data"][0]["deploymentId"],
        child_deployment.id
    );
    assert_eq!(
        by_parent_deployment_body["data"][0]["key"],
        "definitionQueryChildProcess"
    );

    let definitions = engine
        .get_repository_service()
        .get_process_definitions()
        .unwrap();
    let child_definition_id = definitions
        .iter()
        .find(|definition| {
            definition.resource_name.as_deref() == Some("definition-query-child.bpmn20.xml")
        })
        .unwrap()
        .id
        .clone();
    let peer_definition_id = definitions
        .iter()
        .find(|definition| {
            definition.resource_name.as_deref() == Some("definition-query-peer.bpmn20.xml")
        })
        .unwrap()
        .id
        .clone();

    engine
        .get_identity_service()
        .save_group(flowable_engine::identity::entities::Group {
            id: "definition-query-starters".to_string(),
            name: "Definition query starters".to_string(),
            group_type: None,
        });
    engine
        .get_identity_service()
        .create_membership("erica".to_string(), "definition-query-starters".to_string());
    engine.get_identity_link_service().add_identity_link(
        flowable_engine::identity::entities::IdentityLink {
            id: "process-definition:child:users:erica:type:starter".to_string(),
            link_type: "starter".to_string(),
            user_id: Some("erica".to_string()),
            group_id: None,
            task_id: None,
            process_instance_id: None,
            process_definition_id: Some(child_definition_id),
        },
    );
    engine.get_identity_link_service().add_identity_link(
        flowable_engine::identity::entities::IdentityLink {
            id: "process-definition:peer:groups:definition-query-starters:type:starter".to_string(),
            link_type: "starter".to_string(),
            user_id: None,
            group_id: Some("definition-query-starters".to_string()),
            task_id: None,
            process_instance_id: None,
            process_definition_id: Some(peer_definition_id),
        },
    );

    let startable_by_user = client
        .get(format!(
            "{base_url}/repository/process-definitions?startableByUser=erica&keyLike=definitionQuery%&sort=key&order=asc"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(startable_by_user.status(), reqwest::StatusCode::OK);
    let startable_by_user_body: Value = startable_by_user.json().await.unwrap();
    assert_eq!(startable_by_user_body["total"], 2);
    assert_eq!(
        startable_by_user_body["data"][0]["key"],
        "definitionQueryChildProcess"
    );
    assert_eq!(
        startable_by_user_body["data"][1]["key"],
        "definitionQueryPeerProcess"
    );

    let name_desc = client
        .get(format!(
            "{base_url}/repository/process-definitions?keyLike=definitionQuery%&sort=name&order=desc"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(name_desc.status(), reqwest::StatusCode::OK);
    let name_desc_body: Value = name_desc.json().await.unwrap();
    assert_eq!(name_desc_body["total"], 2);
    assert_eq!(
        name_desc_body["data"][0]["name"],
        "definitionQueryPeerProcess"
    );
    assert_eq!(
        name_desc_body["data"][1]["name"],
        "definitionQueryChildProcess"
    );

    let invalid_sort = client
        .get(format!(
            "{base_url}/repository/process-definitions?sort=unsupportedSort"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(invalid_sort.status(), reqwest::StatusCode::BAD_REQUEST);
    let invalid_sort_body: Value = invalid_sort.json().await.unwrap();
    assert_eq!(invalid_sort_body["code"], "BAD_REQUEST");
    assert_eq!(invalid_sort_body["message"], "Bad Request");
    assert!(
        invalid_sort_body["details"]
            .as_str()
            .unwrap()
            .contains("unsupportedSort"),
        "details were: {}",
        invalid_sort_body["details"]
    );

    let sql_like_requires_wildcard = client
        .get(format!(
            "{base_url}/repository/process-definitions?keyLike=definitionQuery&sort=key&order=asc"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(sql_like_requires_wildcard.status(), reqwest::StatusCode::OK);
    let sql_like_requires_wildcard_body: Value = sql_like_requires_wildcard.json().await.unwrap();
    assert_eq!(sql_like_requires_wildcard_body["total"], 0);
}

#[tokio::test]
async fn query_historic_process_instances_filters_business_key_and_returns_structured_bad_request()
{
    let (_engine, base_url, client) = spawn_server("rest-bpmn-historic-query-contract").await;
    deploy_process_via_rest(
        &client,
        &base_url,
        "historicQueryProcess",
        "Historic query task",
    )
    .await;
    start_by_key(&client, &base_url, "historicQueryProcess", "history-bk-1").await;
    let second_instance =
        start_by_key(&client, &base_url, "historicQueryProcess", "history-bk-2").await;
    let second_instance_id = second_instance["id"].as_str().unwrap();

    let filtered = client
        .post(format!(
            "{base_url}/query/historic-process-instances?start=0&size=1"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "businessKey": "history-bk-2",
            "start": 99,
            "size": 99
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(filtered.status(), reqwest::StatusCode::OK);
    let filtered_body: Value = filtered.json().await.unwrap();
    assert_eq!(filtered_body["start"], 0);
    assert_eq!(filtered_body["size"], 1);
    assert_eq!(filtered_body["total"], 1);
    assert_eq!(filtered_body["data"][0]["id"], second_instance_id);

    let bad_query = client
        .post(format!(
            "{base_url}/query/historic-process-instances?unexpectedField=value"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({}))
        .send()
        .await
        .unwrap();

    assert_eq!(bad_query.status(), reqwest::StatusCode::BAD_REQUEST);
    let bad_query_body: Value = bad_query.json().await.unwrap();
    assert_eq!(bad_query_body["code"], "BAD_REQUEST");
    assert_eq!(bad_query_body["message"], "Bad Request");
    assert!(
        bad_query_body["details"]
            .as_str()
            .unwrap()
            .contains("unexpectedField"),
        "details were: {}",
        bad_query_body["details"]
    );
}
