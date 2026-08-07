use axum::{
    Router,
    extract::Request,
    http::{StatusCode, header},
    middleware::{self, Next},
    response::Response,
};
use base64::Engine;
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_form_service::FormOutcome;
use flowable_rest::{
    error::ApiError,
    routes::forms::{
        self, DynFormRepository, FormDefinitionQuery, FormDefinitionRecord,
        FormDefinitionVersionRecord, FormDeleteQuery, FormDeploymentCommand, FormDeploymentRecord,
    },
    run_server,
};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use tokio::net::TcpListener;

#[derive(Default)]
struct MockFormRepository {
    definitions: Mutex<Vec<FormDefinitionRecord>>,
    versions: Mutex<Vec<FormDefinitionVersionRecord>>,
    layout: Mutex<Value>,
    outcomes: Mutex<Vec<FormOutcome>>,
    active_latest_version: Mutex<i32>,
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
                version: 2,
                deployment_id: "deployment-2".to_string(),
                resource_name: "expense-approval-v2.form".to_string(),
                tenant_id: None,
                active: Some(true),
            });
        *repository.versions.lock().unwrap() = vec![
            FormDefinitionVersionRecord {
                id: "form-1".to_string(),
                key: "expenseApproval".to_string(),
                name: "Expense approval".to_string(),
                version: 2,
                deployment_id: "deployment-2".to_string(),
                resource_name: "expense-approval-v2.form".to_string(),
                tenant_id: None,
                active: Some(true),
            },
            FormDefinitionVersionRecord {
                id: "form-0".to_string(),
                key: "expenseApproval".to_string(),
                name: "Expense approval".to_string(),
                version: 1,
                deployment_id: "deployment-1".to_string(),
                resource_name: "expense-approval.form".to_string(),
                tenant_id: None,
                active: Some(true),
            },
        ];
        *repository.layout.lock().unwrap() = json!({"row": 1, "col": 2, "colSpan": 6});
        *repository.outcomes.lock().unwrap() = vec![FormOutcome {
            id: Some("approve".to_string()),
            name: Some("Approve".to_string()),
        }];
        *repository.active_latest_version.lock().unwrap() = 2;
        repository
    }
}

impl forms::FormRepositoryApi for MockFormRepository {
    fn deploy_form_definitions(
        &self,
        command: FormDeploymentCommand,
    ) -> Result<FormDeploymentRecord, ApiError> {
        Ok(FormDeploymentRecord {
            id: "deployment-x".to_string(),
            name: command.name,
            deployed_at: 1,
            resource_names: command
                .resources
                .into_iter()
                .map(|r| r.resource_name)
                .collect(),
        })
    }

    fn list_form_definitions(
        &self,
        query: FormDefinitionQuery,
    ) -> Result<flowable_rest::common::PagedResponse<FormDefinitionRecord>, ApiError> {
        let requested_version = *self.active_latest_version.lock().unwrap();
        let defs: Vec<FormDefinitionRecord> = self
            .definitions
            .lock()
            .unwrap()
            .iter()
            .filter(|d| query.id.as_ref().is_none_or(|v| d.id == *v))
            .filter(|d| query.key.as_ref().is_none_or(|v| d.key == *v))
            .map(|d| {
                let mut d = d.clone();
                d.version = requested_version;
                d
            })
            .collect();
        Ok(query.paging.paginate(defs))
    }

    fn get_form_definition(
        &self,
        form_definition_id: &str,
    ) -> Result<FormDefinitionRecord, ApiError> {
        let requested_version = *self.active_latest_version.lock().unwrap();
        self.definitions
            .lock()
            .unwrap()
            .iter()
            .find(|d| d.id == form_definition_id)
            .map(|d| {
                let mut d = d.clone();
                d.version = requested_version;
                d
            })
            .ok_or_else(|| {
                ApiError::NotFound(format!(
                    "Form definition '{form_definition_id}' was not found"
                ))
            })
    }

    fn list_form_definition_versions(
        &self,
        _form_definition_id: &str,
    ) -> Result<Vec<FormDefinitionVersionRecord>, ApiError> {
        Ok(self.versions.lock().unwrap().clone())
    }

    fn get_form_definition_layout(&self, _form_definition_id: &str) -> Result<Value, ApiError> {
        Ok(self.layout.lock().unwrap().clone())
    }

    fn get_form_definition_outcomes(
        &self,
        _form_definition_id: &str,
    ) -> Result<Vec<FormOutcome>, ApiError> {
        Ok(self.outcomes.lock().unwrap().clone())
    }

    fn delete_form_definitions(&self, query: FormDeleteQuery) -> Result<usize, ApiError> {
        if query.deployment_id.as_deref() == Some("deployment-2") {
            Ok(1)
        } else if query.key.as_deref() == Some("expenseApproval") {
            Ok(2)
        } else {
            Ok(0)
        }
    }

    fn set_form_definition_activation(
        &self,
        form_definition_id: &str,
        active: bool,
    ) -> Result<FormDefinitionRecord, ApiError> {
        if form_definition_id != "form-1" {
            return Err(ApiError::NotFound(format!(
                "Form definition '{form_definition_id}' was not found"
            )));
        }
        *self.active_latest_version.lock().unwrap() = if active { 2 } else { 1 };
        // Update the active flag on the stored definition so get_form_definition reflects it
        if let Some(d) = self
            .definitions
            .lock()
            .unwrap()
            .iter_mut()
            .find(|d| d.id == form_definition_id)
        {
            d.active = Some(active);
        }
        self.get_form_definition(form_definition_id)
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

async fn spawn_real_server() -> (String, reqwest::Client) {
    let engine = Arc::new(ProcessEngine::new("rest-form-breadth".to_string()));
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
    tokio::spawn(async move {
        run_server(engine, listener).await.unwrap();
    });

    (base_url, reqwest::Client::new())
}

#[tokio::test]
async fn form_breadth_routes_cover_versions_layout_outcomes_delete_and_activation() {
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
    assert_eq!(versions_body.as_array().unwrap().len(), 2);
    assert_eq!(versions_body[0]["version"], 2);

    let layout = client
        .get(format!(
            "{}/form-repository/form-definitions/form-1/layout",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(layout.status(), StatusCode::OK);
    assert_eq!(layout.json::<Value>().await.unwrap()["colSpan"], 6);

    let outcomes = client
        .get(format!(
            "{}/form-repository/form-definitions/form-1/outcomes",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(outcomes.status(), StatusCode::OK);
    assert_eq!(outcomes.json::<Value>().await.unwrap()[0]["id"], "approve");

    let delete_by_deployment = client
        .delete(format!(
            "{}/form-repository/form-definitions?deploymentId=deployment-2",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(delete_by_deployment.status(), StatusCode::OK);
    assert_eq!(
        delete_by_deployment.json::<Value>().await.unwrap()["deleted"],
        1
    );

    let delete_by_key = client
        .delete(format!(
            "{}/form-repository/form-definitions?key=expenseApproval",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(delete_by_key.status(), StatusCode::OK);
    assert_eq!(delete_by_key.json::<Value>().await.unwrap()["deleted"], 2);

    let deactivate = client
        .put(format!(
            "{}/form-repository/form-definitions/form-1/activation",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({"active": false}))
        .send()
        .await
        .unwrap();
    assert_eq!(deactivate.status(), StatusCode::OK);
    let deactivate_body = deactivate.json::<Value>().await.unwrap();
    assert_eq!(deactivate_body["version"], 1);
    assert_eq!(deactivate_body["active"], false);

    let latest = client
        .get(format!(
            "{}/form-repository/form-definitions/form-1",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(latest.status(), StatusCode::OK);
    assert_eq!(latest.json::<Value>().await.unwrap()["version"], 1);
}

#[tokio::test]
async fn form_repository_breadth_routes_use_real_form_service_semantics() {
    let (base_url, client) = spawn_real_server().await;

    let first_deploy = client
        .post(format!("{base_url}/form-repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "Expense forms v1",
            "resources": [{
                "resourceName": "expense-approval.form",
                "resource": json!({
                    "key": "expenseApproval",
                    "name": "Expense approval",
                    "resourceName": "expense-approval.form",
                    "layout": { "row": 1, "col": 1, "colSpan": 4 },
                    "outcomes": [{ "id": "approve", "name": "Approve" }],
                    "fields": [
                        { "id": "amount", "name": "Amount", "type": "number" }
                    ]
                }).to_string()
            }]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(first_deploy.status(), StatusCode::CREATED);

    let second_deploy = client
        .post(format!("{base_url}/form-repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "Expense forms v2",
            "resources": [{
                "resourceName": "expense-approval-v2.form",
                "resource": json!({
                    "key": "expenseApproval",
                    "name": "Expense approval",
                    "resourceName": "expense-approval-v2.form",
                    "layout": { "row": 1, "col": 2, "colSpan": 6 },
                    "outcomes": [
                        { "id": "approve", "name": "Approve" },
                        { "id": "reject", "name": "Reject" }
                    ],
                    "fields": [
                        { "id": "amount", "name": "Amount", "type": "number" },
                        { "id": "comment", "name": "Comment", "type": "string" }
                    ]
                }).to_string()
            }]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(second_deploy.status(), StatusCode::CREATED);
    let second_deploy_body: Value = second_deploy.json().await.unwrap();
    let second_deployment_id = second_deploy_body["id"].as_str().unwrap().to_string();

    let definitions = client
        .get(format!(
            "{base_url}/form-repository/form-definitions?key=expenseApproval"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(definitions.status(), StatusCode::OK);
    let definitions_body: Value = definitions.json().await.unwrap();
    assert_eq!(definitions_body["total"], 2);
    assert_eq!(definitions_body["data"][0]["version"], 2);
    let latest_definition_id = definitions_body["data"][0]["id"]
        .as_str()
        .unwrap()
        .to_string();

    let versions = client
        .get(format!(
            "{base_url}/form-repository/form-definitions/{latest_definition_id}/versions"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(versions.status(), StatusCode::OK);
    let versions_body: Value = versions.json().await.unwrap();
    assert_eq!(versions_body.as_array().unwrap().len(), 2);
    assert_eq!(versions_body[0]["version"], 2);
    assert_eq!(versions_body[1]["version"], 1);

    let layout = client
        .get(format!(
            "{base_url}/form-repository/form-definitions/{latest_definition_id}/layout"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(layout.status(), StatusCode::OK);
    assert_eq!(layout.json::<Value>().await.unwrap()["colSpan"], 6);

    let outcomes = client
        .get(format!(
            "{base_url}/form-repository/form-definitions/{latest_definition_id}/outcomes"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(outcomes.status(), StatusCode::OK);
    let outcomes_body: Value = outcomes.json().await.unwrap();
    assert_eq!(outcomes_body.as_array().unwrap().len(), 2);
    assert_eq!(outcomes_body[1]["id"], "reject");

    let deactivate = client
        .put(format!(
            "{base_url}/form-repository/form-definitions/{latest_definition_id}/activation"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({"active": false}))
        .send()
        .await
        .unwrap();
    assert_eq!(deactivate.status(), StatusCode::OK);
    assert_eq!(deactivate.json::<Value>().await.unwrap()["active"], false);

    let deactivated = client
        .get(format!(
            "{base_url}/form-repository/form-definitions/{latest_definition_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(deactivated.status(), StatusCode::OK);
    assert_eq!(deactivated.json::<Value>().await.unwrap()["active"], false);

    let delete_with_ambiguous_selector = client
        .delete(format!(
            "{base_url}/form-repository/form-definitions?deploymentId={second_deployment_id}&key=expenseApproval"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(
        delete_with_ambiguous_selector.status(),
        StatusCode::BAD_REQUEST
    );

    let delete_by_deployment = client
        .delete(format!(
            "{base_url}/form-repository/form-definitions?deploymentId={second_deployment_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(delete_by_deployment.status(), StatusCode::OK);
    assert_eq!(
        delete_by_deployment.json::<Value>().await.unwrap()["deleted"],
        1
    );

    let remaining_versions = client
        .get(format!(
            "{base_url}/form-repository/form-definitions?key=expenseApproval"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(remaining_versions.status(), StatusCode::OK);
    let remaining_versions_body: Value = remaining_versions.json().await.unwrap();
    assert_eq!(remaining_versions_body["total"], 1);
    assert_eq!(remaining_versions_body["data"][0]["version"], 1);
}

#[tokio::test]
async fn form_repository_definitions_accept_version_resource_name_and_latest_filters() {
    let (base_url, client) = spawn_real_server().await;

    for (name, resource_name, payload) in [
        (
            "Expense forms v1",
            "expense-approval.form",
            json!({
                "key": "expenseApproval",
                "name": "Expense approval",
                "resourceName": "expense-approval.form",
                "fields": [
                    { "id": "amount", "name": "Amount", "type": "number" }
                ]
            }),
        ),
        (
            "Expense forms v2",
            "expense-approval-v2.form",
            json!({
                "key": "expenseApproval",
                "name": "Expense approval",
                "resourceName": "expense-approval-v2.form",
                "fields": [
                    { "id": "amount", "name": "Amount", "type": "number" },
                    { "id": "comment", "name": "Comment", "type": "string" }
                ]
            }),
        ),
        (
            "Travel forms",
            "travel-request.form",
            json!({
                "key": "travelRequest",
                "name": "Travel request",
                "resourceName": "travel-request.form",
                "fields": [
                    { "id": "destination", "name": "Destination", "type": "string" }
                ]
            }),
        ),
    ] {
        let response = client
            .post(format!("{base_url}/form-repository/deployments"))
            .basic_auth("admin", Some("test"))
            .json(&json!({
                "name": name,
                "resources": [{
                    "resourceName": resource_name,
                    "resource": payload.to_string()
                }]
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);
    }

    let version_filter = client
        .get(format!(
            "{base_url}/form-repository/form-definitions?key=expenseApproval&version=1"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(version_filter.status(), StatusCode::OK);
    let version_body: Value = version_filter.json().await.unwrap();
    assert_eq!(version_body["total"], 1);
    assert_eq!(version_body["data"][0]["version"], 1);
    assert_eq!(
        version_body["data"][0]["resourceName"],
        "expense-approval.form"
    );

    let resource_name_filter = client
        .get(format!(
            "{base_url}/form-repository/form-definitions?resourceName=expense-approval-v2.form"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(resource_name_filter.status(), StatusCode::OK);
    let resource_name_body: Value = resource_name_filter.json().await.unwrap();
    assert_eq!(resource_name_body["total"], 1);
    assert_eq!(resource_name_body["data"][0]["key"], "expenseApproval");
    assert_eq!(resource_name_body["data"][0]["version"], 2);

    let like_and_sort_filter = client
        .get(format!(
            "{base_url}/form-repository/form-definitions?keyLike=expense%&nameLike=%approval&resourceNameLike=%v2.form&sort=version&order=desc"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(like_and_sort_filter.status(), StatusCode::OK);
    let like_and_sort_body: Value = like_and_sort_filter.json().await.unwrap();
    assert_eq!(like_and_sort_body["total"], 1);
    assert_eq!(like_and_sort_body["data"][0]["key"], "expenseApproval");
    assert_eq!(like_and_sort_body["data"][0]["version"], 2);

    let invalid_sort = client
        .get(format!(
            "{base_url}/form-repository/form-definitions?sort=tenantId&order=sideways"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(invalid_sort.status(), StatusCode::BAD_REQUEST);
    let invalid_sort_body: Value = invalid_sort.json().await.unwrap();
    assert_eq!(invalid_sort_body["code"], "BAD_REQUEST");
    assert!(
        invalid_sort_body["details"]
            .as_str()
            .unwrap()
            .contains("sort")
    );

    let latest_filter = client
        .get(format!(
            "{base_url}/form-repository/form-definitions?latest=true&start=0&size=10"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(latest_filter.status(), StatusCode::OK);
    let latest_body: Value = latest_filter.json().await.unwrap();
    assert_eq!(latest_body["total"], 2);
    let latest_versions = latest_body["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|definition| {
            (
                definition["key"].as_str().unwrap().to_string(),
                definition["version"].as_i64().unwrap(),
            )
        })
        .collect::<Vec<_>>();
    assert!(latest_versions.contains(&("expenseApproval".to_string(), 2)));
    assert!(latest_versions.contains(&("travelRequest".to_string(), 1)));

    let illegal_latest_combo = client
        .get(format!(
            "{base_url}/form-repository/form-definitions?latest=true&deploymentId=deployment-1"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(illegal_latest_combo.status(), StatusCode::BAD_REQUEST);
    let illegal_latest_combo_body: Value = illegal_latest_combo.json().await.unwrap();
    assert_eq!(illegal_latest_combo_body["code"], "BAD_REQUEST");
    assert!(
        illegal_latest_combo_body["details"]
            .as_str()
            .unwrap()
            .contains("latest")
    );
}
