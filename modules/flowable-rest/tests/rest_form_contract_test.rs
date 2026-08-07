use axum::{
    Router,
    extract::Request,
    http::{StatusCode, header},
    middleware::{self, Next},
    response::Response,
};
use base64::Engine;
use flowable_rest::{
    error::ApiError,
    routes::forms::{
        self, DynFormRepository, FormDefinitionQuery, FormDefinitionRecord, FormDeploymentCommand,
        FormDeploymentRecord,
    },
};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use tokio::net::TcpListener;

#[derive(Default)]
struct MockFormRepository {
    definitions: Mutex<Vec<FormDefinitionRecord>>,
    deployments: Mutex<Vec<FormDeploymentRecord>>,
}

impl MockFormRepository {
    fn with_seed() -> Self {
        let repository = Self::default();
        repository
            .definitions
            .lock()
            .unwrap()
            .push(FormDefinitionRecord {
                id: "form-1".to_string(),
                key: "expenseApproval".to_string(),
                name: "Expense approval".to_string(),
                version: 1,
                deployment_id: "deployment-1".to_string(),
                resource_name: "expense-approval.form".to_string(),
                tenant_id: None,
                active: Some(true),
            });
        repository
            .deployments
            .lock()
            .unwrap()
            .push(FormDeploymentRecord {
                id: "deployment-1".to_string(),
                name: "Expense forms".to_string(),
                deployed_at: 1_713_674_400_000,
                resource_names: vec!["expense-approval.form".to_string()],
            });
        repository
    }
}

impl forms::FormRepositoryApi for MockFormRepository {
    fn deploy_form_definitions(
        &self,
        command: FormDeploymentCommand,
    ) -> Result<FormDeploymentRecord, ApiError> {
        let deployment_id = {
            let deployments = self.deployments.lock().unwrap();
            format!("deployment-{}", deployments.len() + 1)
        };
        let deployment = FormDeploymentRecord {
            id: deployment_id,
            name: command.name,
            deployed_at: 1_713_674_500_000,
            resource_names: command
                .resources
                .iter()
                .map(|resource| resource.resource_name.clone())
                .collect(),
        };

        if let Some(resource) = command.resources.first() {
            let form_id = {
                let definitions = self.definitions.lock().unwrap();
                format!("form-{}", definitions.len() + 1)
            };
            self.definitions.lock().unwrap().push(FormDefinitionRecord {
                id: form_id,
                key: resource.resource_name.trim_end_matches(".form").to_string(),
                name: resource.resource_name.trim_end_matches(".form").to_string(),
                version: 1,
                deployment_id: deployment.id.clone(),
                resource_name: resource.resource_name.clone(),
                tenant_id: None,
                active: Some(true),
            });
        }

        self.deployments.lock().unwrap().push(deployment.clone());
        Ok(deployment)
    }

    fn list_form_definitions(
        &self,
        query: FormDefinitionQuery,
    ) -> Result<flowable_rest::common::PagedResponse<FormDefinitionRecord>, ApiError> {
        let filtered: Vec<FormDefinitionRecord> = self
            .definitions
            .lock()
            .unwrap()
            .iter()
            .filter(|definition| {
                query
                    .id
                    .as_ref()
                    .is_none_or(|value| definition.id == *value)
                    && query
                        .key
                        .as_ref()
                        .is_none_or(|value| definition.key == *value)
                    && query
                        .name
                        .as_ref()
                        .is_none_or(|value| definition.name == *value)
                    && query
                        .deployment_id
                        .as_ref()
                        .is_none_or(|value| definition.deployment_id == *value)
            })
            .cloned()
            .collect();

        Ok(query.paging.paginate(filtered))
    }

    fn get_form_definition(
        &self,
        form_definition_id: &str,
    ) -> Result<FormDefinitionRecord, ApiError> {
        self.definitions
            .lock()
            .unwrap()
            .iter()
            .find(|definition| definition.id == form_definition_id)
            .cloned()
            .ok_or_else(|| {
                ApiError::NotFound(format!(
                    "Form definition '{form_definition_id}' was not found"
                ))
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

async fn spawn_server(repository: DynFormRepository) -> (String, reqwest::Client) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());

    let app = Router::new()
        .merge(forms::router(repository))
        .layer(middleware::from_fn(auth_middleware));

    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    (base_url, reqwest::Client::new())
}

#[tokio::test]
async fn form_repository_routes_follow_common_rest_contract() {
    let (base_url, client) = spawn_server(Arc::new(MockFormRepository::with_seed())).await;

    let deploy_response = client
        .post(format!("{}/form-repository/deployments", base_url))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "Travel request forms",
            "resourceName": "travel-request.form",
            "resource": "{\"key\":\"travel-request\"}"
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(deploy_response.status(), StatusCode::CREATED);
    let deploy_body: Value = deploy_response.json().await.unwrap();
    assert_eq!(deploy_body["name"], "Travel request forms");
    assert_eq!(deploy_body["resourceNames"][0], "travel-request.form");

    let list_response = client
        .get(format!(
            "{}/form-repository/form-definitions?key=expenseApproval&start=0&size=10",
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
    let form = &list_body["data"][0];
    assert_eq!(form["id"], "form-1");
    assert_eq!(form["key"], "expenseApproval");
    assert_eq!(form["deploymentId"], "deployment-1");
    assert_eq!(form["resourceName"], "expense-approval.form");

    let get_response = client
        .get(format!(
            "{}/form-repository/form-definitions/form-1",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(get_response.status(), StatusCode::OK);
    let get_body: Value = get_response.json().await.unwrap();
    assert_eq!(get_body["id"], "form-1");
    assert_eq!(get_body["key"], "expenseApproval");
    assert_eq!(get_body["version"], 1);
}

#[tokio::test]
async fn form_repository_routes_enforce_auth_and_structured_errors() {
    let (base_url, client) = spawn_server(Arc::new(MockFormRepository::with_seed())).await;

    let unauthorized = client
        .get(format!("{}/form-repository/form-definitions", base_url))
        .send()
        .await
        .unwrap();

    assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);
    let unauthorized_body: Value = unauthorized.json().await.unwrap();
    assert_eq!(unauthorized_body["code"], "UNAUTHORIZED");

    let bad_query = client
        .get(format!(
            "{}/form-repository/form-definitions?tenantId=tenant-a",
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
            .contains("tenantId")
    );

    let missing = client
        .get(format!(
            "{}/form-repository/form-definitions/missing-form",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
    let missing_body: Value = missing.json().await.unwrap();
    assert_eq!(missing_body["code"], "NOT_FOUND");
}

#[tokio::test]
async fn form_repository_routes_return_structured_limits_when_repository_lacks_optional_features() {
    let (base_url, client) = spawn_server(Arc::new(MockFormRepository::with_seed())).await;

    let versions = client
        .get(format!(
            "{}/form-repository/form-definitions/form-1/versions",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(versions.status(), StatusCode::OK);
    let versions_body: Value = versions.json().await.unwrap();
    assert_eq!(versions_body.as_array().unwrap().len(), 1);
    assert_eq!(versions_body[0]["id"], "form-1");

    let layout = client
        .get(format!(
            "{}/form-repository/form-definitions/form-1/layout",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(layout.status(), StatusCode::BAD_REQUEST);
    let layout_body: Value = layout.json().await.unwrap();
    assert_eq!(layout_body["code"], "BAD_REQUEST");
    assert!(
        layout_body["details"]
            .as_str()
            .unwrap()
            .contains("not supported")
    );

    let outcomes = client
        .get(format!(
            "{}/form-repository/form-definitions/form-1/outcomes",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(outcomes.status(), StatusCode::BAD_REQUEST);
    let outcomes_body: Value = outcomes.json().await.unwrap();
    assert_eq!(outcomes_body["code"], "BAD_REQUEST");
    assert!(
        outcomes_body["details"]
            .as_str()
            .unwrap()
            .contains("not supported")
    );

    let delete_without_selector = client
        .delete(format!("{}/form-repository/form-definitions", base_url))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(delete_without_selector.status(), StatusCode::BAD_REQUEST);
    let delete_body: Value = delete_without_selector.json().await.unwrap();
    assert_eq!(delete_body["code"], "BAD_REQUEST");
    assert!(
        delete_body["details"]
            .as_str()
            .unwrap()
            .contains("not supported")
    );

    let activation = client
        .put(format!(
            "{}/form-repository/form-definitions/form-1/activation",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({"active": false}))
        .send()
        .await
        .unwrap();
    assert_eq!(activation.status(), StatusCode::BAD_REQUEST);
    let activation_body: Value = activation.json().await.unwrap();
    assert_eq!(activation_body["code"], "BAD_REQUEST");
    assert!(
        activation_body["details"]
            .as_str()
            .unwrap()
            .contains("not supported")
    );
}
