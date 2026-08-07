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
        self, AppDefinitionQuery, AppDefinitionRecord, AppDeploymentCommand, AppDeploymentQuery,
        AppDeploymentRecord,
    },
};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use tokio::net::TcpListener;

#[derive(Default)]
struct MockAppRepository {
    deployments: Mutex<Vec<AppDeploymentRecord>>,
    app_definitions: Mutex<Vec<AppDefinitionRecord>>,
}

impl MockAppRepository {
    fn with_seed() -> Self {
        let repository = Self::default();
        repository
            .deployments
            .lock()
            .unwrap()
            .push(AppDeploymentRecord {
                id: "deployment-1".to_string(),
                name: "Employee apps".to_string(),
                category: Some("HR".to_string()),
                deployed_at: 1_713_674_400_000,
                resource_names: vec!["employee-app.json".to_string()],
                tenant_id: Some("tenant-a".to_string()),
            });
        repository
            .deployments
            .lock()
            .unwrap()
            .push(AppDeploymentRecord {
                id: "deployment-2".to_string(),
                name: "Public apps".to_string(),
                category: None,
                deployed_at: 1_713_674_450_000,
                resource_names: vec!["public-app.app".to_string()],
                tenant_id: None,
            });
        repository
            .app_definitions
            .lock()
            .unwrap()
            .push(AppDefinitionRecord {
                id: "app-1".to_string(),
                key: "employee-portal".to_string(),
                name: "Employee Portal".to_string(),
                description: Some("Employee self-service workspace".to_string()),
                category: Some("HR".to_string()),
                version: 1,
                deployment_id: "deployment-1".to_string(),
                resource_name: "employee-app.json".to_string(),
                tenant_id: Some("tenant-a".to_string()),
            });
        repository
            .app_definitions
            .lock()
            .unwrap()
            .push(AppDefinitionRecord {
                id: "app-2".to_string(),
                key: "public-portal".to_string(),
                name: "Public Portal".to_string(),
                description: Some("Public workspace".to_string()),
                category: None,
                version: 1,
                deployment_id: "deployment-2".to_string(),
                resource_name: "public-app.app".to_string(),
                tenant_id: None,
            });
        repository
    }
}

impl apps::AppRepositoryApi for MockAppRepository {
    fn deploy_applications(
        &self,
        command: AppDeploymentCommand,
    ) -> Result<AppDeploymentRecord, ApiError> {
        let deployment_id = {
            let deployments = self.deployments.lock().unwrap();
            format!("deployment-{}", deployments.len() + 1)
        };
        let deployment = AppDeploymentRecord {
            id: deployment_id.clone(),
            name: command.name,
            category: None,
            deployed_at: 1_713_674_500_000,
            resource_names: command
                .resources
                .iter()
                .map(|resource| resource.resource_name.clone())
                .collect(),
            tenant_id: command.tenant_id.clone(),
        };

        if let Some(resource) = command.resources.first() {
            let app_definition_id = {
                let definitions = self.app_definitions.lock().unwrap();
                format!("app-{}", definitions.len() + 1)
            };
            let resource_json: Value = serde_json::from_slice(&resource.resource)
                .map_err(|error| ApiError::bad_request(error.to_string()))?;
            let key = resource_json["key"]
                .as_str()
                .ok_or_else(|| ApiError::bad_request("App definition key is required"))?;
            let name = resource_json["name"]
                .as_str()
                .ok_or_else(|| ApiError::bad_request("App definition name is required"))?;

            self.app_definitions
                .lock()
                .unwrap()
                .push(AppDefinitionRecord {
                    id: app_definition_id,
                    key: key.to_string(),
                    name: name.to_string(),
                    description: resource_json["description"]
                        .as_str()
                        .map(ToString::to_string),
                    category: resource_json["category"].as_str().map(ToString::to_string),
                    version: 1,
                    deployment_id,
                    resource_name: resource.resource_name.clone(),
                    tenant_id: command.tenant_id,
                });
        }

        self.deployments.lock().unwrap().push(deployment.clone());
        Ok(deployment)
    }

    fn list_app_deployments(
        &self,
        query: AppDeploymentQuery,
    ) -> Result<PagedResponse<AppDeploymentRecord>, ApiError> {
        let filtered = self
            .deployments
            .lock()
            .unwrap()
            .iter()
            .filter(|deployment| {
                query
                    .id
                    .as_ref()
                    .is_none_or(|value| deployment.id == *value)
                    && query
                        .name
                        .as_ref()
                        .is_none_or(|value| deployment.name == *value)
                    && query
                        .name_like
                        .as_ref()
                        .is_none_or(|value| matches_sql_like(&deployment.name, value))
                    && query
                        .tenant_id
                        .as_ref()
                        .is_none_or(|value| deployment.tenant_id.as_deref() == Some(value))
                    && query.tenant_id_like.as_ref().is_none_or(|value| {
                        deployment
                            .tenant_id
                            .as_deref()
                            .is_some_and(|tenant_id| matches_sql_like(tenant_id, value))
                    })
                    && (!query.without_tenant_id || deployment.tenant_id.is_none())
            })
            .cloned()
            .collect::<Vec<_>>();

        Ok(query.paging.paginate(filtered))
    }

    fn get_app_deployment(&self, deployment_id: &str) -> Result<AppDeploymentRecord, ApiError> {
        self.deployments
            .lock()
            .unwrap()
            .iter()
            .find(|deployment| deployment.id == deployment_id)
            .cloned()
            .ok_or_else(|| {
                ApiError::NotFound(format!("App deployment '{deployment_id}' was not found"))
            })
    }

    fn delete_app_deployment(&self, deployment_id: &str) -> Result<(), ApiError> {
        let mut deployments = self.deployments.lock().unwrap();
        let original_len = deployments.len();
        deployments.retain(|deployment| deployment.id != deployment_id);
        if deployments.len() == original_len {
            return Err(ApiError::NotFound(format!(
                "App deployment '{deployment_id}' was not found"
            )));
        }
        drop(deployments);

        self.app_definitions
            .lock()
            .unwrap()
            .retain(|definition| definition.deployment_id != deployment_id);
        Ok(())
    }

    fn list_app_deployment_resources(
        &self,
        deployment_id: &str,
    ) -> Result<Vec<apps::AppDeploymentResourceRecord>, ApiError> {
        let deployment = self.get_app_deployment(deployment_id)?;
        Ok(deployment
            .resource_names
            .into_iter()
            .map(|resource_name| apps::AppDeploymentResourceRecord {
                deployment_id: deployment_id.to_string(),
                resource_name,
                resource_type: "resource".to_string(),
                content_type: "application/json".to_string(),
                bytes: b"{}".to_vec(),
            })
            .collect())
    }

    fn get_app_deployment_resource(
        &self,
        deployment_id: &str,
        resource_name: &str,
    ) -> Result<apps::AppDeploymentResourceRecord, ApiError> {
        self.list_app_deployment_resources(deployment_id)?
            .into_iter()
            .find(|resource| resource.resource_name == resource_name)
            .ok_or_else(|| {
                ApiError::NotFound(format!(
                    "App deployment resource '{resource_name}' was not found"
                ))
            })
    }

    fn list_app_definitions(
        &self,
        query: AppDefinitionQuery,
    ) -> Result<PagedResponse<AppDefinitionRecord>, ApiError> {
        let filtered = self
            .app_definitions
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
                        .key_like
                        .as_ref()
                        .is_none_or(|value| matches_sql_like(&definition.key, value))
                    && query
                        .name
                        .as_ref()
                        .is_none_or(|value| definition.name == *value)
                    && query
                        .name_like
                        .as_ref()
                        .is_none_or(|value| matches_sql_like(&definition.name, value))
                    && query
                        .category
                        .as_ref()
                        .is_none_or(|value| definition.category.as_deref() == Some(value))
                    && query.category_like.as_ref().is_none_or(|value| {
                        definition
                            .category
                            .as_deref()
                            .is_some_and(|category| matches_sql_like(category, value))
                    })
                    && query
                        .category_not_equals
                        .as_ref()
                        .is_none_or(|value| definition.category.as_deref() != Some(value))
                    && query
                        .deployment_id
                        .as_ref()
                        .is_none_or(|value| definition.deployment_id == *value)
                    && query
                        .version
                        .is_none_or(|value| definition.version == value)
                    && query
                        .version_greater_than
                        .is_none_or(|value| definition.version > value)
                    && query
                        .version_greater_than_or_equals
                        .is_none_or(|value| definition.version >= value)
                    && query
                        .version_lower_than
                        .is_none_or(|value| definition.version < value)
                    && query
                        .version_lower_than_or_equals
                        .is_none_or(|value| definition.version <= value)
                    && query
                        .resource_name
                        .as_ref()
                        .is_none_or(|value| definition.resource_name == *value)
                    && query
                        .resource_name_like
                        .as_ref()
                        .is_none_or(|value| matches_sql_like(&definition.resource_name, value))
                    && query
                        .tenant_id
                        .as_ref()
                        .is_none_or(|value| definition.tenant_id.as_deref() == Some(value))
                    && query.tenant_id_like.as_ref().is_none_or(|value| {
                        definition
                            .tenant_id
                            .as_deref()
                            .is_some_and(|tenant_id| matches_sql_like(tenant_id, value))
                    })
                    && (!query.without_tenant_id || definition.tenant_id.is_none())
            })
            .cloned()
            .collect::<Vec<_>>();

        let filtered = if query.latest {
            let mut latest = Vec::<AppDefinitionRecord>::new();
            for definition in filtered {
                if let Some(existing) = latest
                    .iter_mut()
                    .find(|existing| existing.key == definition.key)
                {
                    if definition.version > existing.version {
                        *existing = definition;
                    }
                } else {
                    latest.push(definition);
                }
            }
            latest
        } else {
            filtered
        };

        Ok(query.paging.paginate(filtered))
    }

    fn get_app_definition(&self, app_definition_id: &str) -> Result<AppDefinitionRecord, ApiError> {
        self.app_definitions
            .lock()
            .unwrap()
            .iter()
            .find(|definition| definition.id == app_definition_id)
            .cloned()
            .ok_or_else(|| {
                ApiError::NotFound(format!(
                    "App definition '{app_definition_id}' was not found"
                ))
            })
    }

    fn get_app_definition_resource_data(
        &self,
        app_definition_id: &str,
    ) -> Result<apps::AppDeploymentResourceRecord, ApiError> {
        let definition = self.get_app_definition(app_definition_id)?;
        self.get_app_deployment_resource(&definition.deployment_id, &definition.resource_name)
    }
}

fn matches_sql_like(value: &str, pattern: &str) -> bool {
    if pattern == "%" {
        return true;
    }

    match (pattern.strip_prefix('%'), pattern.strip_suffix('%')) {
        (Some(_), Some(_)) if pattern.len() >= 2 => value.contains(&pattern[1..pattern.len() - 1]),
        (Some(_), Some(_)) => value == pattern,
        (Some(suffix), None) => value.ends_with(suffix),
        (None, Some(prefix)) => value.starts_with(prefix),
        (None, None) => value == pattern,
    }
}

impl apps::AppRuntimeApi for MockAppRepository {
    fn list_app_compositions(
        &self,
        _query: apps::AppCompositionQuery,
    ) -> Result<PagedResponse<apps::AppCompositionRecord>, ApiError> {
        Ok(PagedResponse {
            start: 0,
            size: 0,
            total: 0,
            data: Vec::new(),
            sort: None,
            order: None,
        })
    }

    fn get_app_composition(
        &self,
        _app_definition_id: &str,
        _filter: apps::AppCompositionFilter,
    ) -> Result<apps::AppCompositionRecord, ApiError> {
        Err(ApiError::InternalServerError(
            "runtime stub not used in repository tests".to_string(),
        ))
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

async fn spawn_server(repository: Arc<MockAppRepository>) -> (String, reqwest::Client) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());

    let app = Router::new()
        .merge(apps::router(repository.clone(), repository))
        .layer(middleware::from_fn(auth_middleware));

    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    (base_url, reqwest::Client::new())
}

#[tokio::test]
async fn app_repository_routes_follow_common_rest_contract() {
    let (base_url, client) = spawn_server(Arc::new(MockAppRepository::with_seed())).await;

    let deploy_response = client
        .post(format!("{}/app-repository/deployments", base_url))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "Operations apps",
            "tenantId": "tenant-a",
            "resourceName": "operations-app.json",
            "resource": json!({
                "key": "operations-portal",
                "name": "Operations Portal",
                "description": "Operations cockpit",
                "category": "Operations",
                "pages": [],
                "references": []
            }).to_string()
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(deploy_response.status(), StatusCode::CREATED);
    let deploy_body: Value = deploy_response.json().await.unwrap();
    assert_eq!(deploy_body["name"], "Operations apps");
    assert_eq!(deploy_body["tenantId"], "tenant-a");
    assert_eq!(deploy_body["resourceNames"][0], "operations-app.json");
    assert_eq!(
        deploy_body["url"],
        "/app-repository/deployments/deployment-3"
    );
    assert_eq!(
        deploy_body["deploymentTime"],
        "2024-04-21T04:41:40.000+00:00"
    );
    assert!(deploy_body["category"].is_null());
    assert_eq!(deploy_body["deployedAt"], 1_713_674_500_000_i64);

    let deployments_response = client
        .get(format!(
            "{}/app-repository/deployments?name=Employee%20apps&start=0&size=10",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(deployments_response.status(), StatusCode::OK);
    let deployments_body: Value = deployments_response.json().await.unwrap();
    assert_eq!(deployments_body["start"], 0);
    assert_eq!(deployments_body["size"], 1);
    assert_eq!(deployments_body["total"], 1);
    let deployment = &deployments_body["data"][0];
    assert_eq!(deployment["id"], "deployment-1");
    assert_eq!(deployment["name"], "Employee apps");
    assert_eq!(deployment["tenantId"], "tenant-a");
    assert_eq!(
        deployment["url"],
        "/app-repository/deployments/deployment-1"
    );
    assert_eq!(
        deployment["deploymentTime"],
        "2024-04-21T04:40:00.000+00:00"
    );
    assert_eq!(deployment["category"], "HR");
    assert_eq!(deployment["deployedAt"], 1_713_674_400_000_i64);

    let deployment_get = client
        .get(format!(
            "{}/app-repository/deployments/deployment-1",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(deployment_get.status(), StatusCode::OK);
    let deployment_get_body: Value = deployment_get.json().await.unwrap();
    assert_eq!(deployment_get_body["id"], "deployment-1");
    assert_eq!(deployment_get_body["resourceNames"][0], "employee-app.json");
    assert_eq!(
        deployment_get_body["url"],
        "/app-repository/deployments/deployment-1"
    );
    assert_eq!(
        deployment_get_body["deploymentTime"],
        "2024-04-21T04:40:00.000+00:00"
    );

    let definitions_response = client
        .get(format!(
            "{}/app-repository/app-definitions?key=employee-portal&start=0&size=10",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(definitions_response.status(), StatusCode::OK);
    let definitions_body: Value = definitions_response.json().await.unwrap();
    assert_eq!(definitions_body["start"], 0);
    assert_eq!(definitions_body["size"], 1);
    assert_eq!(definitions_body["total"], 1);
    let definition = &definitions_body["data"][0];
    assert_eq!(definition["id"], "app-1");
    assert_eq!(definition["key"], "employee-portal");
    assert_eq!(definition["deploymentId"], "deployment-1");
    assert_eq!(definition["category"], "HR");
    assert_eq!(definition["url"], "/app-repository/app-definitions/app-1");

    let definition_get = client
        .get(format!("{}/app-repository/app-definitions/app-1", base_url))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(definition_get.status(), StatusCode::OK);
    let definition_get_body: Value = definition_get.json().await.unwrap();
    assert_eq!(definition_get_body["id"], "app-1");
    assert_eq!(definition_get_body["key"], "employee-portal");
    assert_eq!(definition_get_body["version"], 1);
    assert_eq!(
        definition_get_body["url"],
        "/app-repository/app-definitions/app-1"
    );
}

#[tokio::test]
async fn app_repository_routes_expose_model_resource() {
    let (base_url, client) = spawn_server(Arc::new(MockAppRepository::with_seed())).await;

    let model_response = client
        .get(format!(
            "{}/app-repository/app-definitions/app-1/model",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(model_response.status(), StatusCode::OK);
    let model_body: Value = model_response.json().await.unwrap();
    assert_eq!(model_body["id"], "app-1");
    assert_eq!(model_body["key"], "employee-portal");
    assert_eq!(model_body["name"], "Employee Portal");
    assert_eq!(model_body["resourceName"], "employee-app.json");
}

#[tokio::test]
async fn app_management_route_exposes_engine_info() {
    let (base_url, client) = spawn_server(Arc::new(MockAppRepository::with_seed())).await;

    let response = client
        .get(format!("{}/app-management/engine", base_url))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["name"], "flowable-app-engine");
    assert!(body["version"].is_string());
    assert!(body["resourceUrl"].is_null());
    assert!(body["exception"].is_null());
}

#[tokio::test]
async fn app_repository_routes_accept_definition_filters() {
    let repository = MockAppRepository::with_seed();
    repository
        .deployments
        .lock()
        .unwrap()
        .push(AppDeploymentRecord {
            id: "deployment-3".to_string(),
            name: "Employee apps v2".to_string(),
            category: Some("HR".to_string()),
            deployed_at: 1_713_674_600_000,
            resource_names: vec!["employee-app-v2.app".to_string()],
            tenant_id: Some("tenant-a".to_string()),
        });
    repository
        .app_definitions
        .lock()
        .unwrap()
        .push(AppDefinitionRecord {
            id: "app-3".to_string(),
            key: "employee-portal".to_string(),
            name: "Employee Portal".to_string(),
            description: Some("Employee self-service workspace v2".to_string()),
            category: Some("HR".to_string()),
            version: 2,
            deployment_id: "deployment-3".to_string(),
            resource_name: "employee-app-v2.app".to_string(),
            tenant_id: Some("tenant-a".to_string()),
        });

    let (base_url, client) = spawn_server(Arc::new(repository)).await;

    let latest = client
        .get(format!(
            "{}/app-repository/app-definitions?keyLike=employee%25&nameLike=Employee%25&category=HR&resourceNameLike=%25-v2.app&latest=true&sort=version&order=desc",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(latest.status(), StatusCode::OK);
    let latest_body: Value = latest.json().await.unwrap();
    assert_eq!(latest_body["total"], 1);
    assert_eq!(latest_body["data"][0]["id"], "app-3");
    assert_eq!(latest_body["data"][0]["version"], 2);

    let without_tenant = client
        .get(format!(
            "{}/app-repository/app-definitions?withoutTenantId=true&categoryNotEquals=HR",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(without_tenant.status(), StatusCode::OK);
    let without_tenant_body: Value = without_tenant.json().await.unwrap();
    assert_eq!(without_tenant_body["total"], 1);
    assert_eq!(without_tenant_body["data"][0]["id"], "app-2");
}

#[tokio::test]
async fn app_repository_routes_accept_deployment_filters_and_validate_sorting() {
    let (base_url, client) = spawn_server(Arc::new(MockAppRepository::with_seed())).await;

    let filtered = client
        .get(format!(
            "{}/app-repository/deployments?nameLike=Public%25&withoutTenantId=true&sort=deployTime&order=asc",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(filtered.status(), StatusCode::OK);
    let filtered_body: Value = filtered.json().await.unwrap();
    assert_eq!(filtered_body["total"], 1);
    assert_eq!(filtered_body["data"][0]["id"], "deployment-2");

    let invalid_sort = client
        .get(format!(
            "{}/app-repository/deployments?sort=deploymentTime",
            base_url
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

    let invalid_order = client
        .get(format!(
            "{}/app-repository/app-definitions?sort=name&order=ascending",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(invalid_order.status(), StatusCode::BAD_REQUEST);
    let invalid_order_body: Value = invalid_order.json().await.unwrap();
    assert_eq!(invalid_order_body["code"], "BAD_REQUEST");
    assert!(
        invalid_order_body["details"]
            .as_str()
            .unwrap()
            .contains("order")
    );
}

#[tokio::test]
async fn app_repository_deployment_delete_returns_204_and_cascades() {
    let repository = Arc::new(MockAppRepository::with_seed());
    let (base_url, client) = spawn_server(repository).await;

    let delete_response = client
        .delete(format!(
            "{}/app-repository/deployments/deployment-1",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(delete_response.status(), StatusCode::NO_CONTENT);
    assert_eq!(delete_response.bytes().await.unwrap().len(), 0);

    let deleted_deployment = client
        .get(format!(
            "{}/app-repository/deployments/deployment-1",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(deleted_deployment.status(), StatusCode::NOT_FOUND);

    let definitions = client
        .get(format!(
            "{}/app-repository/app-definitions?deploymentId=deployment-1",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(definitions.status(), StatusCode::OK);
    let definitions_body: Value = definitions.json().await.unwrap();
    assert_eq!(definitions_body["total"], 0);

    let missing_delete = client
        .delete(format!(
            "{}/app-repository/deployments/missing-deployment",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(missing_delete.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn app_repository_routes_enforce_auth_and_structured_errors() {
    let (base_url, client) = spawn_server(Arc::new(MockAppRepository::with_seed())).await;

    let unauthorized = client
        .get(format!("{}/app-repository/app-definitions", base_url))
        .send()
        .await
        .unwrap();

    assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);
    let unauthorized_body: Value = unauthorized.json().await.unwrap();
    assert_eq!(unauthorized_body["code"], "UNAUTHORIZED");

    let bad_query = client
        .get(format!(
            "{}/app-repository/app-definitions?flowApp=true",
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
            .contains("flowApp")
    );

    let invalid_deploy = client
        .post(format!("{}/app-repository/deployments", base_url))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "Broken app deployment",
            "resources": []
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(invalid_deploy.status(), StatusCode::BAD_REQUEST);
    let invalid_deploy_body: Value = invalid_deploy.json().await.unwrap();
    assert_eq!(invalid_deploy_body["code"], "BAD_REQUEST");

    let missing_deployment = client
        .get(format!(
            "{}/app-repository/deployments/missing-deployment",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(missing_deployment.status(), StatusCode::NOT_FOUND);
    let missing_deployment_body: Value = missing_deployment.json().await.unwrap();
    assert_eq!(missing_deployment_body["code"], "NOT_FOUND");

    let missing_definition = client
        .get(format!(
            "{}/app-repository/app-definitions/missing-definition",
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
