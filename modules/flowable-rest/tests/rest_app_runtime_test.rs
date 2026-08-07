use axum::{
    Router,
    extract::Request,
    http::{StatusCode, header},
    middleware::{self, Next},
    response::Response,
};
use base64::Engine;
use flowable_rest::{
    common::PagedResponse,
    error::ApiError,
    routes::apps::{
        self, AppCompositionFilter, AppCompositionQuery, AppCompositionRecord, AppDefinitionQuery,
        AppDefinitionRecord, AppDeploymentCommand, AppDeploymentQuery, AppDeploymentRecord,
        AppResolvedReferenceRecord,
    },
};
use serde_json::Value;
use std::sync::Arc;
use tokio::net::TcpListener;

#[derive(Default)]
struct MockAppRuntime;

impl apps::AppRepositoryApi for MockAppRuntime {
    fn deploy_applications(
        &self,
        _command: AppDeploymentCommand,
    ) -> Result<AppDeploymentRecord, ApiError> {
        Err(ApiError::InternalServerError(
            "repository stub not used in runtime tests".to_string(),
        ))
    }

    fn list_app_deployments(
        &self,
        _query: AppDeploymentQuery,
    ) -> Result<PagedResponse<AppDeploymentRecord>, ApiError> {
        Ok(PagedResponse {
            start: 0,
            size: 0,
            total: 0,
            data: Vec::new(),
            sort: None,
            order: None,
        })
    }

    fn get_app_deployment(&self, _deployment_id: &str) -> Result<AppDeploymentRecord, ApiError> {
        Err(ApiError::NotFound(
            "App deployment was not found".to_string(),
        ))
    }

    fn list_app_definitions(
        &self,
        _query: AppDefinitionQuery,
    ) -> Result<PagedResponse<AppDefinitionRecord>, ApiError> {
        Ok(PagedResponse {
            start: 0,
            size: 0,
            total: 0,
            data: Vec::new(),
            sort: None,
            order: None,
        })
    }

    fn get_app_definition(
        &self,
        _app_definition_id: &str,
    ) -> Result<AppDefinitionRecord, ApiError> {
        Err(ApiError::NotFound(
            "App definition was not found".to_string(),
        ))
    }
}

impl apps::AppRuntimeApi for MockAppRuntime {
    fn list_app_compositions(
        &self,
        query: AppCompositionQuery,
    ) -> Result<PagedResponse<AppCompositionRecord>, ApiError> {
        let app_definition_key = query
            .app_definition_key
            .as_deref()
            .ok_or_else(|| ApiError::bad_request("appDefinitionKey is required"))?;

        if app_definition_key != "employee-portal" {
            return Err(ApiError::NotFound(format!(
                "Resolved app composition for key '{app_definition_key}' was not found"
            )));
        }

        let definition_type = query.definition_type.as_deref();
        if let Some(value) = definition_type
            && !matches!(
                value,
                "bpmnProcess" | "dmnDecision" | "cmmnCase" | "eventRegistry"
            )
        {
            return Err(ApiError::bad_request(format!(
                "Unsupported app composition definitionType '{value}'"
            )));
        }

        let mut references = vec![
            AppResolvedReferenceRecord {
                page_id: "page-process".to_string(),
                page_name: Some("Process Dashboard".to_string()),
                reference_id: "start-onboarding".to_string(),
                reference_name: Some("Start onboarding".to_string()),
                definition_type: "bpmnProcess".to_string(),
                resolved_definition_id: "process-definition-1".to_string(),
                resolved_definition_key: "employee-onboarding".to_string(),
                resolved_definition_name: "Employee Onboarding".to_string(),
                resolved_definition_version: 3,
                resolved_tenant_id: Some("tenant-a".to_string()),
            },
            AppResolvedReferenceRecord {
                page_id: "page-process".to_string(),
                page_name: Some("Process Dashboard".to_string()),
                reference_id: "benefits-check".to_string(),
                reference_name: Some("Benefits check".to_string()),
                definition_type: "dmnDecision".to_string(),
                resolved_definition_id: "decision-definition-1".to_string(),
                resolved_definition_key: "benefits-eligibility".to_string(),
                resolved_definition_name: "Benefits Eligibility".to_string(),
                resolved_definition_version: 2,
                resolved_tenant_id: Some("tenant-a".to_string()),
            },
        ];

        if let Some(value) = definition_type {
            references.retain(|reference| reference.definition_type == value);
        }

        let records = vec![AppCompositionRecord {
            app_definition_id: "app-1".to_string(),
            app_definition_key: "employee-portal".to_string(),
            app_definition_name: "Employee Portal".to_string(),
            app_definition_version: 1,
            deployment_id: "deployment-1".to_string(),
            tenant_id: Some("tenant-a".to_string()),
            references,
        }];

        Ok(query.paging.paginate(records))
    }

    fn get_app_composition(
        &self,
        app_definition_id: &str,
        filter: AppCompositionFilter,
    ) -> Result<AppCompositionRecord, ApiError> {
        if app_definition_id != "app-1" {
            return Err(ApiError::NotFound(format!(
                "Resolved app composition for definition '{app_definition_id}' was not found"
            )));
        }

        if let Some(value) = filter.definition_type.as_deref()
            && !matches!(
                value,
                "bpmnProcess" | "dmnDecision" | "cmmnCase" | "eventRegistry"
            )
        {
            return Err(ApiError::bad_request(format!(
                "Unsupported app composition definitionType '{value}'"
            )));
        }

        let mut references = vec![
            AppResolvedReferenceRecord {
                page_id: "page-process".to_string(),
                page_name: Some("Process Dashboard".to_string()),
                reference_id: "start-onboarding".to_string(),
                reference_name: Some("Start onboarding".to_string()),
                definition_type: "bpmnProcess".to_string(),
                resolved_definition_id: "process-definition-1".to_string(),
                resolved_definition_key: "employee-onboarding".to_string(),
                resolved_definition_name: "Employee Onboarding".to_string(),
                resolved_definition_version: 3,
                resolved_tenant_id: Some("tenant-a".to_string()),
            },
            AppResolvedReferenceRecord {
                page_id: "page-process".to_string(),
                page_name: Some("Process Dashboard".to_string()),
                reference_id: "benefits-check".to_string(),
                reference_name: Some("Benefits check".to_string()),
                definition_type: "dmnDecision".to_string(),
                resolved_definition_id: "decision-definition-1".to_string(),
                resolved_definition_key: "benefits-eligibility".to_string(),
                resolved_definition_name: "Benefits Eligibility".to_string(),
                resolved_definition_version: 2,
                resolved_tenant_id: Some("tenant-a".to_string()),
            },
        ];

        if let Some(value) = filter.definition_type {
            references.retain(|reference| reference.definition_type == value);
        }

        Ok(AppCompositionRecord {
            app_definition_id: "app-1".to_string(),
            app_definition_key: "employee-portal".to_string(),
            app_definition_name: "Employee Portal".to_string(),
            app_definition_version: 1,
            deployment_id: "deployment-1".to_string(),
            tenant_id: Some("tenant-a".to_string()),
            references,
        })
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

async fn spawn_server(runtime: Arc<MockAppRuntime>) -> (String, reqwest::Client) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());

    let app = Router::new()
        .merge(apps::router(runtime.clone(), runtime))
        .layer(middleware::from_fn(auth_middleware));

    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    (base_url, reqwest::Client::new())
}

#[tokio::test]
async fn app_runtime_routes_follow_owned_contract() {
    let (base_url, client) = spawn_server(Arc::new(MockAppRuntime)).await;

    let list_response = client
        .get(format!(
            "{}/app-runtime/compositions?appDefinitionKey=employee-portal&definitionType=dmnDecision&start=0&size=10",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(list_response.status(), StatusCode::OK);
    let list_body: Value = list_response.json().await.unwrap();
    assert_eq!(list_body["start"], 0);
    assert_eq!(list_body["size"], 1);
    assert_eq!(list_body["total"], 1);
    let composition = &list_body["data"][0];
    assert_eq!(composition["appDefinitionId"], "app-1");
    assert_eq!(composition["appDefinitionKey"], "employee-portal");
    assert_eq!(
        composition["references"][0]["definitionType"],
        "dmnDecision"
    );
    assert_eq!(
        composition["references"][0]["resolvedDefinitionKey"],
        "benefits-eligibility"
    );

    let get_response = client
        .get(format!("{}/app-runtime/compositions/app-1", base_url))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(get_response.status(), StatusCode::OK);
    let get_body: Value = get_response.json().await.unwrap();
    assert_eq!(get_body["appDefinitionId"], "app-1");
    assert_eq!(get_body["appDefinitionVersion"], 1);
    assert_eq!(get_body["references"][0]["resolvedDefinitionVersion"], 3);
}

#[tokio::test]
async fn app_runtime_routes_enforce_auth_and_structured_errors() {
    let (base_url, client) = spawn_server(Arc::new(MockAppRuntime)).await;

    let unauthorized = client
        .get(format!(
            "{}/app-runtime/compositions?appDefinitionKey=employee-portal",
            base_url
        ))
        .send()
        .await
        .unwrap();

    assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);
    let unauthorized_body: Value = unauthorized.json().await.unwrap();
    assert_eq!(unauthorized_body["code"], "UNAUTHORIZED");

    let bad_query = client
        .get(format!(
            "{}/app-runtime/compositions?deploymentId=deployment-1",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(bad_query.status(), StatusCode::BAD_REQUEST);
    let bad_query_body: Value = bad_query.json().await.unwrap();
    assert_eq!(bad_query_body["code"], "BAD_REQUEST");
    assert!(
        bad_query_body["details"]
            .as_str()
            .unwrap()
            .contains("deploymentId")
    );

    let unsupported_type = client
        .get(format!(
            "{}/app-runtime/compositions?appDefinitionKey=employee-portal&definitionType=formDefinition",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(unsupported_type.status(), StatusCode::BAD_REQUEST);
    let unsupported_type_body: Value = unsupported_type.json().await.unwrap();
    assert_eq!(unsupported_type_body["code"], "BAD_REQUEST");
    assert!(
        unsupported_type_body["details"]
            .as_str()
            .unwrap()
            .contains("definitionType")
    );

    let missing = client
        .get(format!("{}/app-runtime/compositions/missing-app", base_url))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
    let missing_body: Value = missing.json().await.unwrap();
    assert_eq!(missing_body["code"], "NOT_FOUND");
}
