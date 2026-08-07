//! P109 — POST /repository/deployments multipart/form-data alignment with
//! Java `DeploymentCollectionResource.uploadDeployment`.
//!
//! Covers the multipart contract (single `.bpmn20.xml`/`.bpmn` resource,
//! `.zip`/`.bar` expansion, file-name validation, query-parameter passthrough)
//! plus the kept Rust JSON superset path and the 400 for non-multipart
//! non-JSON bodies.

use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_rest::run_server;
use reqwest::multipart::{Form, Part};
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::net::TcpListener;

const BPMN_PROCESS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
    <process id="multipartContractProcess" isExecutable="true">
        <startEvent id="start" />
        <sequenceFlow id="f1" sourceRef="start" targetRef="end" />
        <endEvent id="end" />
    </process>
</definitions>"#;

/// A second process with a distinct key, for zip tests.
fn second_process_xml() -> String {
    r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
    <process id="multipartZipSecondProcess" isExecutable="true">
        <startEvent id="start" />
        <sequenceFlow id="f1" sourceRef="start" targetRef="end" />
        <endEvent id="end" />
    </process>
</definitions>"#
    .to_string()
}

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
    let addr = listener.local_addr().unwrap();
    let base_url = format!("http://{}", addr);

    let engine_clone = Arc::clone(&engine);
    tokio::spawn(async move {
        run_server(engine_clone, listener).await.unwrap();
    });

    (engine, base_url, reqwest::Client::new())
}

fn error_details(body: &Value) -> String {
    body["details"].as_str().unwrap_or_default().to_string()
}

#[tokio::test]
async fn multipart_single_bpmn_deploys_with_full_response() {
    let (engine, base_url, client) = spawn_server("rest-multipart-deploy-single").await;

    let form = Form::new().part(
        "file",
        Part::bytes(BPMN_PROCESS.as_bytes().to_vec())
            .file_name("multipart_process.bpmn20.xml"),
    );
    let response = client
        .post(format!(
            "{base_url}/repository/deployments?deploymentKey=key-1&tenantId=tenant-1"
        ))
        .basic_auth("admin", Some("test"))
        .multipart(form)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::CREATED);

    let body: Value = response.json().await.unwrap();
    assert!(body["id"].is_string());
    assert_eq!(body["name"], "multipart_process"); // derived from file name
    assert!(body["deploymentTime"].is_string());
    assert_eq!(body["key"], "key-1");
    assert_eq!(body["tenantId"], "tenant-1");
    // P109: multipart response carries the full deployment field set.
    assert!(body.get("category").is_some());
    assert!(body.get("parentDeploymentId").is_some());

    let deployment_id = body["id"].as_str().unwrap();
    let resources = client
        .get(format!("{base_url}/repository/deployments/{deployment_id}/resources"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(resources.status(), reqwest::StatusCode::OK);
    let resources: Value = resources.json().await.unwrap();
    assert_eq!(resources.as_array().unwrap().len(), 1);
    assert_eq!(
        resources[0]["id"],
        "multipart_process.bpmn20.xml",
        "the uploaded file is stored under its original file name"
    );

    let definitions = engine.get_repository_service().get_process_definition_ids().unwrap();
    assert_eq!(definitions.len(), 1);
    assert!(engine.get_repository_service().get_process_definition_ids().unwrap().iter().any(
        |id| id.starts_with("multipartContractProcess:")
    ));
}

#[tokio::test]
async fn multipart_deployment_name_query_param_wins() {
    let (_, base_url, client) = spawn_server("rest-multipart-deploy-name-param").await;

    let form = Form::new().part(
        "file",
        Part::bytes(BPMN_PROCESS.as_bytes().to_vec()).file_name("ignored.bpmn20.xml"),
    );
    let response = client
        .post(format!(
            "{base_url}/repository/deployments?deploymentName=explicit-deployment-name"
        ))
        .basic_auth("admin", Some("test"))
        .multipart(form)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::CREATED);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["name"], "explicit-deployment-name");
}

#[tokio::test]
async fn multipart_tenant_id_form_field_passthrough() {
    // Java documents `tenantId` as a multipart form-field
    // (DeploymentCollectionResource.java:152); Spring's @RequestParam resolves
    // it from the form when no query parameter is present.
    let (_, base_url, client) = spawn_server("rest-multipart-deploy-tenant-form").await;

    let form = Form::new()
        .text("tenantId", "tenant-form")
        .part(
            "file",
            Part::bytes(BPMN_PROCESS.as_bytes().to_vec()).file_name("tenant_form.bpmn20.xml"),
        );
    let response = client
        .post(format!("{base_url}/repository/deployments"))
        .basic_auth("admin", Some("test"))
        .multipart(form)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::CREATED);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["tenantId"], "tenant-form");
}

#[tokio::test]
async fn multipart_invalid_file_name_returns_400() {
    let (_, base_url, client) = spawn_server("rest-multipart-deploy-invalid-name").await;

    let form = Form::new().part(
        "file",
        Part::bytes("not a bpmn file".as_bytes().to_vec()).file_name("notes.txt"),
    );
    let response = client
        .post(format!("{base_url}/repository/deployments"))
        .basic_auth("admin", Some("test"))
        .multipart(form)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
    let body: Value = response.json().await.unwrap();
    assert_eq!(
        error_details(&body),
        "File must be of type .bpmn20.xml, .bpmn, .bar or .zip"
    );
}

#[tokio::test]
async fn non_multipart_non_json_body_returns_400() {
    let (_, base_url, client) = spawn_server("rest-multipart-deploy-wrong-content-type").await;

    let response = client
        .post(format!("{base_url}/repository/deployments"))
        .basic_auth("admin", Some("test"))
        .header(reqwest::header::CONTENT_TYPE, "text/plain")
        .body("just some text")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
    let body: Value = response.json().await.unwrap();
    // Java: `FlowableIllegalArgumentException("Multipart request is required")`
    assert_eq!(error_details(&body), "Multipart request is required");
}

#[tokio::test]
async fn multipart_zip_with_two_bpmn_files_registers_two_definitions() {
    let (engine, base_url, client) = spawn_server("rest-multipart-deploy-zip").await;

    let zip_bytes = build_zip(&[
        ("processes/first.bpmn20.xml", BPMN_PROCESS.as_bytes()),
        ("processes/second.bpmn20.xml", second_process_xml().as_bytes()),
    ]);

    let form = Form::new().part(
        "file",
        Part::bytes(zip_bytes).file_name("processes.zip"),
    );
    let response = client
        .post(format!("{base_url}/repository/deployments"))
        .basic_auth("admin", Some("test"))
        .multipart(form)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::CREATED);
    let body: Value = response.json().await.unwrap();
    // No deploymentName: the zip file name without extension is the deployment name.
    assert_eq!(body["name"], "processes");

    let deployment_id = body["id"].as_str().unwrap();
    let resources = client
        .get(format!("{base_url}/repository/deployments/{deployment_id}/resources"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    let resources: Value = resources.json().await.unwrap();
    assert_eq!(resources.as_array().unwrap().len(), 2);
    assert_eq!(resources[0]["id"], "processes/first.bpmn20.xml");
    assert_eq!(resources[1]["id"], "processes/second.bpmn20.xml");

    let definitions = engine
        .get_repository_service()
        .get_process_definition_ids()
        .unwrap();
    assert_eq!(definitions.len(), 2);
    assert!(definitions.iter().any(|id| id.starts_with("multipartContractProcess:")));
    assert!(definitions.iter().any(|id| id.starts_with("multipartZipSecondProcess:")));
}

#[tokio::test]
async fn json_deploy_path_regression_returns_201_full_fields() {
    // The JSON request shape is a Rust superset kept for existing clients; it
    // returns 201 with the same full response fields as the multipart path.
    let (_, base_url, client) = spawn_server("rest-multipart-deploy-json-superset").await;

    let response = client
        .post(format!("{base_url}/repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "Json Superset Deployment",
            "resourceName": "json_superset.bpmn20.xml",
            "resource": BPMN_PROCESS
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::CREATED);
    let body: Value = response.json().await.unwrap();
    assert!(body["id"].is_string());
    assert_eq!(body["name"], "Json Superset Deployment");
    assert!(body["deploymentTime"].is_string());
    assert!(body.get("category").is_some());
    assert!(body.get("key").is_some());
    assert!(body.get("tenantId").is_some());
    assert!(body.get("parentDeploymentId").is_some());
}

/// Builds an in-memory zip with the given `(entry_name, bytes)` entries,
/// mirroring Java `DeploymentBuilderImpl.addZipInputStream` input.
fn build_zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let cursor = std::io::Cursor::new(Vec::new());
    let mut writer = zip::ZipWriter::new(cursor);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    for (name, bytes) in entries {
        writer.start_file(*name, options).unwrap();
        std::io::Write::write_all(&mut writer, bytes).unwrap();
    }
    writer.finish().unwrap().into_inner()
}
