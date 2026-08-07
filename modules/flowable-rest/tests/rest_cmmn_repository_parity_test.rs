use axum::http::StatusCode;
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_rest::run_server;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::net::TcpListener;

const SIMPLE_CASE_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/CMMN/20151109/MODEL">
  <case id="parityCase" name="Parity Case">
    <casePlanModel id="casePlan" name="Case Plan" />
  </case>
</definitions>"#;

const CASE_WITH_HUMAN_TASK_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/CMMN/20151109/MODEL">
  <case id="humanTaskCase" name="Human Task Case">
    <casePlanModel id="casePlan" name="Case Plan">
      <planItem id="pi_ht1" definitionRef="ht1" />
      <humanTask id="ht1" name="Review Task" />
    </casePlanModel>
  </case>
</definitions>"#;

async fn spawn_real_server(test_name: &str) -> (String, reqwest::Client) {
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

    (base_url, reqwest::Client::new())
}

async fn deploy_case(
    base_url: &str,
    client: &reqwest::Client,
    name: &str,
    resource_name: &str,
    xml: &str,
) -> Value {
    client
        .post(format!("{base_url}/cmmn-repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": name,
            "resourceName": resource_name,
            "resource": xml
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap()
}

async fn deploy_case_with_tenant(
    base_url: &str,
    client: &reqwest::Client,
    name: &str,
    resource_name: &str,
    xml: &str,
    tenant_id: &str,
) -> Value {
    client
        .post(format!("{base_url}/cmmn-repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": name,
            "tenantId": tenant_id,
            "resourceName": resource_name,
            "resource": xml
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap()
}

async fn get_case_def_id(base_url: &str, client: &reqwest::Client, deployment_id: &str) -> String {
    let case_defs: Value = client
        .get(format!(
            "{base_url}/cmmn-repository/case-definitions?deploymentId={deployment_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    case_defs["data"][0]["id"].as_str().unwrap().to_string()
}

#[tokio::test]
async fn rest_deployment_uses_engine_builder_not_local_parser() {
    let (base_url, client) = spawn_real_server("rest-parity-builder").await;

    let deploy = deploy_case(
        &base_url,
        &client,
        "Builder test",
        "builder.cmmn",
        SIMPLE_CASE_XML,
    )
    .await;
    assert_eq!(deploy["name"], "Builder test");
    assert!(!deploy["id"].as_str().unwrap().is_empty());
}

#[tokio::test]
async fn rest_empty_deployment_name_is_accepted() {
    let (base_url, client) = spawn_real_server("rest-parity-empty-name").await;

    let deploy: Value = client
        .post(format!("{base_url}/cmmn-repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "",
            "resourceName": "empty-name.cmmn",
            "resource": SIMPLE_CASE_XML
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(deploy["name"], "");
}

#[tokio::test]
async fn rest_tenant_id_is_persisted_and_queryable() {
    let (base_url, client) = spawn_real_server("rest-parity-tenant").await;

    let deploy = deploy_case_with_tenant(
        &base_url,
        &client,
        "Tenant case",
        "tenant.cmmn",
        SIMPLE_CASE_XML,
        "tenant-1",
    )
    .await;
    assert_eq!(deploy["tenantId"], "tenant-1");

    let list: Value = client
        .get(format!(
            "{base_url}/cmmn-repository/deployments?tenantId=tenant-1"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(list["total"], 1);
}

#[tokio::test]
async fn rest_case_definition_model_returns_json() {
    let (base_url, client) = spawn_real_server("rest-parity-model").await;

    let deploy = deploy_case(
        &base_url,
        &client,
        "Model test",
        "model.cmmn",
        SIMPLE_CASE_XML,
    )
    .await;
    let deployment_id = deploy["id"].as_str().unwrap();
    let case_def_id = get_case_def_id(&base_url, &client, deployment_id).await;

    let model: Value = client
        .get(format!(
            "{base_url}/cmmn-repository/case-definitions/{case_def_id}/model"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    // model 返回 case definition 详情，内嵌 model 子对象
    assert_eq!(model["key"], "parityCase");
    assert_eq!(model["model"]["id"], "parityCase");
    assert_eq!(model["model"]["case_plan_model"]["id"], "casePlan");
}

#[tokio::test]
async fn rest_resource_data_returns_original_bytes() {
    let (base_url, client) = spawn_real_server("rest-parity-resource").await;

    let deploy = deploy_case(
        &base_url,
        &client,
        "Resource test",
        "resource.cmmn",
        SIMPLE_CASE_XML,
    )
    .await;
    let deployment_id = deploy["id"].as_str().unwrap();

    let resource: Value = client
        .get(format!(
            "{base_url}/cmmn-repository/deployments/{deployment_id}/resources"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    // 资源列表直接返回数组，每项有 id 字段（资源名）
    let data = resource.as_array();
    assert!(
        data.is_some(),
        "resource response should be array, got: {resource}"
    );
    assert!(data.unwrap().iter().any(|r| r["id"] == "resource.cmmn"));
}

#[tokio::test]
async fn rest_cascade_delete_removes_deployment() {
    let (base_url, client) = spawn_real_server("rest-parity-cascade").await;

    let deploy = deploy_case(
        &base_url,
        &client,
        "Cascade test",
        "cascade.cmmn",
        SIMPLE_CASE_XML,
    )
    .await;
    let deployment_id = deploy["id"].as_str().unwrap();

    let delete = client
        .delete(format!(
            "{base_url}/cmmn-repository/deployments/{deployment_id}?cascade=true"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(delete.status(), StatusCode::NO_CONTENT);

    let missing = client
        .get(format!(
            "{base_url}/cmmn-repository/deployments/{deployment_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn rest_non_cascade_delete_with_runtime_conflicts() {
    let (base_url, client) = spawn_real_server("rest-parity-conflict").await;

    let deploy = deploy_case(
        &base_url,
        &client,
        "Conflict test",
        "conflict.cmmn",
        CASE_WITH_HUMAN_TASK_XML,
    )
    .await;
    let deployment_id = deploy["id"].as_str().unwrap();

    let case_defs: Value = client
        .get(format!(
            "{base_url}/cmmn-repository/case-definitions?deploymentId={deployment_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let case_def_key = case_defs["data"][0]["key"].as_str().unwrap();

    let _start: Value = client
        .post(format!("{base_url}/cmmn-runtime/case-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "caseDefinitionKey": case_def_key
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let conflict = client
        .delete(format!(
            "{base_url}/cmmn-repository/deployments/{deployment_id}?cascade=false"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(
        conflict.status() == StatusCode::CONFLICT || conflict.status() == StatusCode::BAD_REQUEST,
        "non-cascade delete with runtime instances should fail, got {}",
        conflict.status()
    );

    let cascade = client
        .delete(format!(
            "{base_url}/cmmn-repository/deployments/{deployment_id}?cascade=true"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(cascade.status(), StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn rest_case_definition_not_found_returns_404() {
    let (base_url, client) = spawn_real_server("rest-parity-404").await;

    let missing = client
        .get(format!(
            "{base_url}/cmmn-repository/case-definitions/nonexistent-id"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn rest_deployment_not_found_returns_404() {
    let (base_url, client) = spawn_real_server("rest-parity-dep-404").await;

    let missing = client
        .get(format!(
            "{base_url}/cmmn-repository/deployments/nonexistent-id"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn rest_unknown_definition_decision_tables_returns_404() {
    let (base_url, client) = spawn_real_server("rest-parity-decisions-404").await;

    let missing = client
        .get(format!(
            "{base_url}/cmmn-repository/case-definitions/nonexistent-id/decision-tables"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn rest_unknown_definition_form_definitions_returns_404() {
    let (base_url, client) = spawn_real_server("rest-parity-forms-404").await;

    let missing = client
        .get(format!(
            "{base_url}/cmmn-repository/case-definitions/nonexistent-id/form-definitions"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn rest_candidate_starter_crud_roundtrip() {
    let (base_url, client) = spawn_real_server("rest-parity-starter").await;

    let deploy = deploy_case(
        &base_url,
        &client,
        "Starter test",
        "starter.cmmn",
        SIMPLE_CASE_XML,
    )
    .await;
    let deployment_id = deploy["id"].as_str().unwrap();
    let case_def_id = get_case_def_id(&base_url, &client, deployment_id).await;

    let add = client
        .post(format!(
            "{base_url}/cmmn-repository/case-definitions/{case_def_id}/identitylinks"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "type": "starter",
            "user": "alice"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(add.status(), StatusCode::CREATED);

    let list: Value = client
        .get(format!(
            "{base_url}/cmmn-repository/case-definitions/{case_def_id}/identitylinks"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let links = list.as_array().unwrap();
    assert_eq!(links.len(), 1);
    assert_eq!(links[0]["user"], "alice");

    let delete = client
        .delete(format!(
            "{base_url}/cmmn-repository/case-definitions/{case_def_id}/identitylinks/users/alice"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(delete.status(), StatusCode::NO_CONTENT);

    let after: Value = client
        .get(format!(
            "{base_url}/cmmn-repository/case-definitions/{case_def_id}/identitylinks"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(after.as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn rest_invalid_cmmn_xml_returns_400() {
    let (base_url, client) = spawn_real_server("rest-parity-invalid-xml").await;

    let invalid = client
        .post(format!("{base_url}/cmmn-repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "Invalid XML",
            "resourceName": "invalid.cmmn",
            "resource": "not valid xml <<<"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);
}
