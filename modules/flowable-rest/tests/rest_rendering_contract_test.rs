use axum::{
    Router,
    extract::Request,
    http::{StatusCode, header},
    middleware::{self, Next},
    response::Response,
};
use base64::Engine;
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_rest::{
    error::ApiError,
    routes::rendering::{self, RenderingRequest},
    run_server,
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};
use tokio::net::TcpListener;

const PROCESS_WITH_USER_TASK: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
  <process id="renderingContractProcess" name="Rendering Contract Process" isExecutable="true">
    <startEvent id="start" />
    <sequenceFlow id="flow1" sourceRef="start" targetRef="reviewTask" />
    <userTask id="reviewTask" name="Review Rendering Alias" />
    <sequenceFlow id="flow2" sourceRef="reviewTask" targetRef="end" />
    <endEvent id="end" />
  </process>
</definitions>"#;

const RENDERING_DMN: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="https://www.omg.org/spec/DMN/20191111/MODEL/"
             namespace="http://flowable.org/dmn"
             name="Rendering DMN">
  <decision id="renderingContractDecision" name="Rendering Contract Decision">
    <decisionTable id="renderingContractTable" hitPolicy="FIRST">
      <input id="input1" label="Score">
        <inputExpression id="inputExpression1" typeRef="integer">
          <text>score</text>
        </inputExpression>
      </input>
      <output id="output1" label="Approved" name="approved" typeRef="boolean" />
      <rule id="rule1">
        <inputEntry id="inputEntry1"><text>10</text></inputEntry>
        <outputEntry id="outputEntry1"><text>true</text></outputEntry>
      </rule>
    </decisionTable>
  </decision>
</definitions>"#;

const RENDERING_CMMN: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/CMMN/20151109/MODEL"
             targetNamespace="http://flowable.org/cmmn">
  <case id="renderingContractCase" name="Rendering Contract Case">
    <casePlanModel id="renderingContractPlan" name="Rendering Contract Plan" autoComplete="false">
      <planItem id="reviewStagePlanItem" name="Review Rendering Stage" definitionRef="reviewStage" />
      <humanTask id="rootReviewTask" name="Root Rendering Review" isBlocking="true" />
      <stage id="reviewStage" name="Review Rendering Stage" autoComplete="false">
        <planItem id="reviewPlanItem" name="Review Rendering Case" definitionRef="reviewTask" />
        <humanTask id="reviewTask" name="Review Rendering Case" isBlocking="true" />
      </stage>
    </casePlanModel>
  </case>
</definitions>"#;

#[derive(Default)]
struct MockRenderingApi {
    process_images: Mutex<BTreeMap<String, String>>,
    decision_images: Mutex<BTreeMap<String, String>>,
    case_images: Mutex<BTreeMap<String, String>>,
    app_images: Mutex<BTreeMap<String, String>>,
}

impl MockRenderingApi {
    fn with_seed() -> Self {
        let api = Self::default();
        api.process_images.lock().unwrap().insert(
            "process-1".to_string(),
            r#"<svg xmlns="http://www.w3.org/2000/svg"><text>process-1</text></svg>"#.to_string(),
        );
        api.decision_images.lock().unwrap().insert(
            "decision-1".to_string(),
            r#"<svg xmlns="http://www.w3.org/2000/svg"><text>decision-1</text></svg>"#.to_string(),
        );
        api.case_images.lock().unwrap().insert(
            "case-1".to_string(),
            r#"<svg xmlns="http://www.w3.org/2000/svg"><text>case-1</text></svg>"#.to_string(),
        );
        api.app_images.lock().unwrap().insert(
            "app-1".to_string(),
            r#"<svg xmlns="http://www.w3.org/2000/svg"><text>app-1</text></svg>"#.to_string(),
        );
        api
    }

    fn image_from(
        images: &Mutex<BTreeMap<String, String>>,
        definition_id: &str,
    ) -> Result<String, ApiError> {
        images
            .lock()
            .unwrap()
            .get(definition_id)
            .cloned()
            .ok_or_else(|| {
                ApiError::NotFound(format!("Definition '{definition_id}' was not found"))
            })
    }
}

impl rendering::RenderingApi for MockRenderingApi {
    fn render_process_definition_image(
        &self,
        process_definition_id: &str,
        _request: RenderingRequest,
    ) -> Result<String, ApiError> {
        Self::image_from(&self.process_images, process_definition_id)
    }

    fn render_process_instance_diagram(
        &self,
        process_instance_id: &str,
        _request: RenderingRequest,
    ) -> Result<String, ApiError> {
        Self::image_from(&self.process_images, process_instance_id)
    }

    fn render_decision_table_image(
        &self,
        decision_table_id: &str,
        _request: RenderingRequest,
    ) -> Result<String, ApiError> {
        Self::image_from(&self.decision_images, decision_table_id)
    }

    fn render_case_definition_image(
        &self,
        case_definition_id: &str,
        _request: RenderingRequest,
    ) -> Result<String, ApiError> {
        Self::image_from(&self.case_images, case_definition_id)
    }

    fn render_case_instance_diagram(
        &self,
        case_instance_id: &str,
        _request: RenderingRequest,
    ) -> Result<String, ApiError> {
        Self::image_from(&self.case_images, case_instance_id)
    }

    fn render_app_definition_image(
        &self,
        app_definition_id: &str,
        _request: RenderingRequest,
    ) -> Result<String, ApiError> {
        Self::image_from(&self.app_images, app_definition_id)
    }
}

async fn auth_middleware(req: Request, next: Next) -> Result<Response, ApiError> {
    let auth_header = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|header| header.to_str().ok())
        .unwrap_or("");

    if !auth_header.starts_with("Basic ") {
        return Err(ApiError::Unauthorized);
    }

    let encoded = auth_header.trim_start_matches("Basic ");
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|_| ApiError::Unauthorized)?;
    let decoded = String::from_utf8(decoded).map_err(|_| ApiError::Unauthorized)?;

    if decoded != "admin:test" {
        return Err(ApiError::Unauthorized);
    }

    Ok(next.run(req).await)
}

async fn spawn_server(api: Arc<MockRenderingApi>) -> (String, reqwest::Client) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());

    let rendering_api: rendering::DynRenderingApi = api;
    let app = Router::new()
        .merge(rendering::router(rendering_api))
        .layer(middleware::from_fn(auth_middleware));

    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    (base_url, reqwest::Client::new())
}

async fn spawn_real_server(test_name: &str) -> (Arc<ProcessEngine>, String, reqwest::Client) {
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

    let engine_clone = Arc::clone(&engine);
    tokio::spawn(async move {
        run_server(engine_clone, listener).await.unwrap();
    });

    (engine, base_url, reqwest::Client::new())
}

async fn deploy_process(client: &reqwest::Client, base_url: &str) {
    let response = client
        .post(format!("{base_url}/repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "Rendering Contract Process Deployment",
            "resourceName": "rendering-contract.bpmn20.xml",
            "resource": PROCESS_WITH_USER_TASK
        }))
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success());
}

async fn deploy_decision(client: &reqwest::Client, base_url: &str) -> String {
    let response = client
        .post(format!("{base_url}/dmn-repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "Rendering Contract DMN Deployment",
            "resourceName": "rendering-contract.dmn",
            "resource": RENDERING_DMN
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);

    let decisions = client
        .get(format!(
            "{base_url}/dmn-repository/decisions?key=renderingContractDecision&start=0&size=10"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(decisions.status(), StatusCode::OK);
    decisions.json::<Value>().await.unwrap()["data"][0]["id"]
        .as_str()
        .unwrap()
        .to_string()
}

async fn deploy_and_start_case(client: &reqwest::Client, base_url: &str) -> String {
    let deploy_response = client
        .post(format!("{base_url}/cmmn-repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "Rendering Contract CMMN Deployment",
            "resourceName": "rendering-contract.cmmn",
            "resource": RENDERING_CMMN
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(deploy_response.status(), StatusCode::CREATED);

    let start_response = client
        .post(format!("{base_url}/cmmn-runtime/case-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "caseDefinitionKey": "renderingContractCase",
            "businessKey": "rendering-contract-cmmn",
            "variables": {
                "customer": "acme"
            }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(start_response.status(), StatusCode::CREATED);
    start_response.json::<Value>().await.unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string()
}

async fn assert_svg_response_contains(
    client: &reqwest::Client,
    url: String,
    marker: &str,
) -> String {
    let unauthorized = client
        .get(&url)
        .header(header::ACCEPT, "image/svg+xml")
        .send()
        .await
        .unwrap();
    assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);

    let response = client
        .get(url)
        .basic_auth("admin", Some("test"))
        .header(header::ACCEPT, "image/svg+xml")
        .send()
        .await
        .unwrap();
    let status = response.status();
    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let body = response.text().await.unwrap();
    assert_eq!(status, StatusCode::OK, "unexpected response body: {body}");
    assert_eq!(
        content_type.as_deref(),
        Some("image/svg+xml"),
        "unexpected response body: {body}"
    );
    assert!(body.contains(marker), "missing marker {marker} in {body}");
    body
}

#[tokio::test]
async fn rendering_routes_return_svg_for_owned_repository_endpoints() {
    let (base_url, client) = spawn_server(Arc::new(MockRenderingApi::with_seed())).await;

    let cases = [
        (
            format!(
                "{}/repository/process-definitions/process-1/image",
                base_url
            ),
            "process-1",
        ),
        (
            format!(
                "{}/dmn-repository/decision-tables/decision-1/image?renderer=svg",
                base_url
            ),
            "decision-1",
        ),
        (
            format!(
                "{}/dmn-repository/decisions/decision-1/image?renderer=image/svg+xml",
                base_url
            ),
            "decision-1",
        ),
        (
            format!("{}/cmmn-repository/case-definitions/case-1/image", base_url),
            "case-1",
        ),
        (
            format!("{}/app-repository/app-definitions/app-1/image", base_url),
            "app-1",
        ),
    ];

    for (url, marker) in cases {
        let response = client
            .get(url)
            .basic_auth("admin", Some("test"))
            .header(header::ACCEPT, "image/svg+xml")
            .send()
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response
                .headers()
                .get(header::CONTENT_TYPE)
                .unwrap()
                .to_str()
                .unwrap(),
            "image/svg+xml"
        );

        let body = response.text().await.unwrap();
        assert!(body.contains(marker), "missing marker {marker} in {body}");
    }
}

#[tokio::test]
async fn rendering_routes_enforce_auth_and_structured_errors() {
    let (base_url, client) = spawn_server(Arc::new(MockRenderingApi::with_seed())).await;

    let unauthorized = client
        .get(format!(
            "{}/repository/process-definitions/process-1/image",
            base_url
        ))
        .send()
        .await
        .unwrap();

    assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);
    let unauthorized_body: Value = unauthorized.json().await.unwrap();
    assert_eq!(unauthorized_body["code"], "UNAUTHORIZED");

    let default_svg_extension = client
        .get(format!(
            "{}/repository/process-definitions/process-1/image",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(default_svg_extension.status(), StatusCode::OK);
    assert_eq!(
        default_svg_extension
            .headers()
            .get(header::CONTENT_TYPE)
            .unwrap()
            .to_str()
            .unwrap(),
        "image/svg+xml"
    );
    assert!(
        default_svg_extension
            .text()
            .await
            .unwrap()
            .contains("process-1")
    );

    let png_accept_response = client
        .get(format!(
            "{}/repository/process-definitions/process-1/image",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .header(header::ACCEPT, "image/png")
        .send()
        .await
        .unwrap();

    assert_eq!(png_accept_response.status(), StatusCode::OK);
    assert_eq!(
        png_accept_response
            .headers()
            .get(header::CONTENT_TYPE)
            .unwrap()
            .to_str()
            .unwrap(),
        "image/png"
    );
    let png_accept_body = png_accept_response.bytes().await.unwrap();
    assert_eq!(&png_accept_body[..8], b"\x89PNG\r\n\x1a\n");

    let png_renderer_response = client
        .get(format!(
            "{}/repository/process-definitions/process-1/image?renderer=image/png",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(png_renderer_response.status(), StatusCode::OK);
    assert_eq!(
        png_renderer_response
            .headers()
            .get(header::CONTENT_TYPE)
            .unwrap()
            .to_str()
            .unwrap(),
        "image/png"
    );
    let png_renderer_body = png_renderer_response.bytes().await.unwrap();
    assert_eq!(&png_renderer_body[..8], b"\x89PNG\r\n\x1a\n");

    let unsupported_renderer = client
        .get(format!(
            "{}/repository/process-definitions/process-1/image?renderer=pdf",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(unsupported_renderer.status(), StatusCode::BAD_REQUEST);
    let unsupported_renderer_body: Value = unsupported_renderer.json().await.unwrap();
    assert_eq!(unsupported_renderer_body["code"], "BAD_REQUEST");
    assert!(
        unsupported_renderer_body["details"]
            .as_str()
            .unwrap()
            .contains("Unsupported renderer")
    );

    let missing_definition = client
        .get(format!(
            "{}/app-repository/app-definitions/missing-app/image?renderer=svg",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(missing_definition.status(), StatusCode::NOT_FOUND);
    let missing_definition_body: Value = missing_definition.json().await.unwrap();
    assert_eq!(missing_definition_body["code"], "NOT_FOUND");
}

#[tokio::test]
async fn dmn_decision_image_alias_renders_real_decision_svg() {
    let (_engine, base_url, client) = spawn_real_server("rest-rendering-dmn-alias").await;
    let decision_id = deploy_decision(&client, &base_url).await;

    assert_svg_response_contains(
        &client,
        format!("{base_url}/dmn-repository/decisions/{decision_id}/image"),
        "Rendering Contract Decision",
    )
    .await;
}

#[tokio::test]
async fn process_instance_diagram_alias_renders_real_process_definition_svg() {
    let (engine, base_url, client) = spawn_real_server("rest-rendering-process-alias").await;
    deploy_process(&client, &base_url).await;
    let process_definition_id = engine
        .get_repository_service()
        .get_process_definition_ids()
        .unwrap()[0]
        .clone();

    let start_response = client
        .post(format!("{base_url}/runtime/process-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "processDefinitionId": process_definition_id,
            "businessKey": "rendering-contract-process"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(start_response.status(), StatusCode::OK);
    let process_instance_id = start_response.json::<Value>().await.unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();

    assert_svg_response_contains(
        &client,
        format!("{base_url}/runtime/process-instances/{process_instance_id}/diagram"),
        "Review Rendering Alias",
    )
    .await;
}

#[tokio::test]
async fn case_instance_diagram_alias_renders_real_case_definition_svg() {
    let (_engine, base_url, client) = spawn_real_server("rest-rendering-case-alias").await;
    let case_instance_id = deploy_and_start_case(&client, &base_url).await;

    assert_svg_response_contains(
        &client,
        format!("{base_url}/cmmn-runtime/case-instances/{case_instance_id}/diagram"),
        "Rendering Contract Case",
    )
    .await;
}
