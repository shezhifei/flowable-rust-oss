// P101: CMMN case-instance and plan-item query surface — GET/POST.
//
// Java references:
// - CaseInstanceCollectionResource.java:114-297 (GET case param parsing)
// - BaseCaseInstanceResource.java:68-263 (CaseInstanceQuery builders)
// - PlanItemInstanceCollectionResource.java:71-159 (GET plan-item param parsing)
// - PlanItemInstanceBaseResource.java:59-139 (PlanItemInstanceQuery builders)
//
// Intentional cuts (P101 acceptance): caseDefinitionCategory /
// activePlanItemDefinitionId(s) / involvedUser / tenantId-on-plan-item are not
// implemented (rejected via deny_unknown_fields); plan-item queries only cover
// human-task plan items.

use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::identity::entities::User;
use flowable_rest::run_server;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::net::TcpListener;

const CASE_CMMN: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/CMMN/20151109/MODEL"
             targetNamespace="http://flowable.org/cmmn">
  <case id="queryCase" name="Query Case">
    <casePlanModel id="queryPlan" name="Query Plan" autoComplete="false">
      <planItem id="planItemAlpha" definitionRef="queryTaskAlpha" />
      <planItem id="planItemBeta" definitionRef="queryTaskBeta" />
      <humanTask id="queryTaskAlpha" name="Alpha review" isBlocking="true" />
      <humanTask id="queryTaskBeta" name="Beta review" isBlocking="true" />
    </casePlanModel>
  </case>
</definitions>"#;

async fn spawn_server() -> (String, reqwest::Client) {
    let engine = Arc::new(ProcessEngine::new("rest-cmmn-case-query".to_string()));
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

async fn deploy(base_url: &str, client: &reqwest::Client, deployment_name: &str) {
    let response = client
        .post(format!("{base_url}/cmmn-repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": deployment_name,
            "resourceName": "query.cmmn",
            "resource": CASE_CMMN
        }))
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success());
}

async fn start_case(
    base_url: &str,
    client: &reqwest::Client,
    body: Value,
) -> String {
    let response = client
        .post(format!("{base_url}/cmmn-runtime/case-instances"))
        .basic_auth("admin", Some("test"))
        .json(&body)
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success());
    response.json::<Value>().await.unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string()
}

async fn case_ids(base_url: &str, client: &reqwest::Client, query: &str) -> Vec<String> {
    let response = client
        .get(format!("{base_url}/cmmn-runtime/case-instances{query}"))
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
        .map(|case| case["id"].as_str().unwrap().to_string())
        .collect()
}

#[tokio::test]
async fn cmmn_case_query_filters_by_definition_key_and_name() {
    let (base_url, client) = spawn_server().await;
    deploy(&base_url, &client, "P101 deployment").await;
    let id_a = start_case(
        &base_url,
        &client,
        json!({ "caseDefinitionKey": "queryCase", "name": "Order A", "businessKey": "bk-1001" }),
    )
    .await;
    let id_b = start_case(
        &base_url,
        &client,
        json!({ "caseDefinitionKey": "queryCase", "name": "Order B" }),
    )
    .await;

    assert_eq!(
        case_ids(&base_url, &client, "?ids=bogus").await,
        Vec::<String>::new()
    );
    assert_eq!(
        case_ids(&base_url, &client, &format!("?ids={id_a},{id_b}")).await.len(),
        2
    );

    assert_eq!(
        case_ids(&base_url, &client, "?caseDefinitionKey=queryCase").await.len(),
        2
    );
    assert_eq!(
        case_ids(&base_url, &client, "?caseDefinitionKeyLike=query%").await.len(),
        2
    );
    assert_eq!(
        case_ids(&base_url, &client, "?caseDefinitionKeyLikeIgnoreCase=QUERY%").await.len(),
        2
    );
    assert_eq!(
        case_ids(&base_url, &client, "?caseDefinitionName=Query Case").await.len(),
        2
    );
    assert_eq!(
        case_ids(&base_url, &client, "?caseDefinitionNameLike=Query%").await.len(),
        2
    );

    assert_eq!(
        case_ids(&base_url, &client, "?name=Order A").await,
        vec![id_a.clone()]
    );
    assert_eq!(
        case_ids(&base_url, &client, "?nameLike=Order%").await.len(),
        2
    );
    assert_eq!(
        case_ids(&base_url, &client, "?nameLikeIgnoreCase=order%").await.len(),
        2
    );
    assert_eq!(
        case_ids(&base_url, &client, "?businessKeyLike=bk-1%").await.len(),
        1
    );
    assert_eq!(
        case_ids(&base_url, &client, "?businessKeyLikeIgnoreCase=BK-1001").await,
        vec![id_a]
    );
}

#[tokio::test]
async fn cmmn_case_query_filters_by_tenant() {
    let (base_url, client) = spawn_server().await;
    deploy(&base_url, &client, "P101 tenant deployment").await;

    let id_a = start_case(
        &base_url,
        &client,
        json!({ "caseDefinitionKey": "queryCase", "name": "Default" }),
    )
    .await;

    // Deploy a tenant-scoped definition and start with a tenant id.
    let response = client
        .post(format!("{base_url}/cmmn-repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "P101 tenant deployment 2",
            "resourceName": "query-tenant.cmmn",
            "resource": CASE_CMMN,
            "tenantId": "tenant-x"
        }))
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success());
    let id_b = start_case(
        &base_url,
        &client,
        json!({ "caseDefinitionKey": "queryCase", "tenantId": "tenant-x" }),
    )
    .await;

    assert_eq!(
        case_ids(&base_url, &client, "?tenantId=tenant-x").await,
        vec![id_b.clone()]
    );
    assert_eq!(
        case_ids(&base_url, &client, "?tenantIdLike=tenant-%").await.len(),
        1
    );
    assert_eq!(
        case_ids(&base_url, &client, "?tenantIdLikeIgnoreCase=TENANT-X").await.len(),
        1
    );
    assert_eq!(
        case_ids(&base_url, &client, "?withoutTenantId=true").await,
        vec![id_a]
    );
}

#[tokio::test]
async fn cmmn_case_query_sort_and_include_variables() {
    let (base_url, client) = spawn_server().await;
    deploy(&base_url, &client, "P101 sort deployment").await;
    let id = start_case(
        &base_url,
        &client,
        json!({ "caseDefinitionKey": "queryCase", "name": "First" }),
    )
    .await;
    start_case(
        &base_url,
        &client,
        json!({ "caseDefinitionKey": "queryCase", "name": "Second" }),
    )
    .await;

    // Sort by id ascending then descending — the allowed case-instance sort
    // properties are id/caseDefinitionId/caseDefinitionKey/startTime/tenantId/
    // businessKey (BaseCaseInstanceResource.java:45-54).
    let sorted_asc = case_ids(&base_url, &client, "?sort=id&order=asc").await;
    let sorted_desc = case_ids(&base_url, &client, "?sort=id&order=desc").await;
    assert_eq!(sorted_asc.len(), 2);
    assert_eq!(sorted_desc.len(), 2);
    assert_eq!(
        sorted_desc,
        sorted_asc.iter().rev().cloned().collect::<Vec<_>>(),
        "desc reverses the asc order"
    );

    // Invalid sort → 400 (PaginateListUtil.java:119-121).
    let response = client
        .get(format!("{base_url}/cmmn-runtime/case-instances?sort=banana"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 400);

    // Seed a case variable, then verify includeCaseVariables and the names variant.
    client
        .put(format!("{base_url}/cmmn-runtime/case-instances/{id}/variables"))
        .basic_auth("admin", Some("test"))
        .json(&json!([{ "name": "priority", "value": "high" }, { "name": "extra", "value": 1 }]))
        .send()
        .await
        .unwrap();

    let response = client
        .get(format!("{base_url}/cmmn-runtime/case-instances?includeCaseVariables=true&name=First"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    let body: Value = response.json().await.unwrap();
    let variables = body["data"][0]["variables"].as_array().unwrap();
    let mut names = variables
        .iter()
        .map(|variable| variable["name"].as_str().unwrap().to_string())
        .collect::<Vec<_>>();
    names.sort();
    assert_eq!(names, vec!["extra", "priority"]);

    let response = client
        .get(format!(
            "{base_url}/cmmn-runtime/case-instances?includeCaseVariablesNames=extra&name=First"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    let body: Value = response.json().await.unwrap();
    let variables = body["data"][0]["variables"].as_array().unwrap();
    let names = variables
        .iter()
        .map(|variable| variable["name"].as_str().unwrap().to_string())
        .collect::<Vec<_>>();
    assert_eq!(names, vec!["extra"]);
}

#[tokio::test]
async fn cmmn_plan_item_query_filters_by_case_definition_id_and_element() {
    let (base_url, client) = spawn_server().await;
    deploy(&base_url, &client, "P101 plan-item deployment").await;
    let case_id = start_case(
        &base_url,
        &client,
        json!({ "caseDefinitionKey": "queryCase" }),
    )
    .await;

    let defs = client
        .get(format!("{base_url}/cmmn-repository/case-definitions?key=queryCase"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    let case_definition_id = defs["data"][0]["id"].as_str().unwrap().to_string();

    // caseDefinitionId and caseInstanceIds filters.
    let response = client
        .get(format!(
            "{base_url}/cmmn-runtime/plan-item-instances?caseDefinitionId={case_definition_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["data"].as_array().unwrap().len(), 2);

    let response = client
        .get(format!(
            "{base_url}/cmmn-runtime/plan-item-instances?caseInstanceIds={case_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["data"].as_array().unwrap().len(), 2);

    // elementId → the plan item id.
    let response = client
        .get(format!(
            "{base_url}/cmmn-runtime/plan-item-instances?elementId=planItemAlpha"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    let body: Value = response.json().await.unwrap();
    let names = body["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["name"].as_str().unwrap().to_string())
        .collect::<Vec<_>>();
    assert_eq!(names, vec!["Alpha review"]);

    // planItemDefinitionType — only humanTask type plan items exist.
    let response = client
        .get(format!(
            "{base_url}/cmmn-runtime/plan-item-instances?planItemDefinitionType=humantask"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["data"].as_array().unwrap().len(), 2);

    let response = client
        .get(format!(
            "{base_url}/cmmn-runtime/plan-item-instances?planItemDefinitionType=stage"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["data"].as_array().unwrap().len(), 0);

    // includeEnded is effectively always true (Rust task query keeps ended items).
    let response = client
        .get(format!(
            "{base_url}/cmmn-runtime/plan-item-instances?includeEnded=true"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success());
}

#[tokio::test]
async fn cmmn_case_query_post_body_supports_same_filters() {
    let (base_url, client) = spawn_server().await;
    deploy(&base_url, &client, "P101 post deployment").await;
    start_case(
        &base_url,
        &client,
        json!({ "caseDefinitionKey": "queryCase", "name": "Alpha" }),
    )
    .await;

    let response = client
        .post(format!("{base_url}/cmmn-query/case-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "nameLike": "Alp%", "sort": "id", "order": "desc" }))
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success());
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["data"].as_array().unwrap().len(), 1);
}
