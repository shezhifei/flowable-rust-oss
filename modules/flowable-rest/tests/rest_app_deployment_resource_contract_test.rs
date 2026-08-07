use axum::Router;
use flowable_app_converter::parse_app_definition;
use flowable_app_engine::{
    AppDeploymentRequest as EngineAppDeploymentRequest, AppEngine, AppModel as EngineAppModel,
    canonical_definition_to_engine,
};
use flowable_rest::{
    common::PagedResponse,
    error::ApiError,
    routes::apps::{
        self, AppDefinitionQuery, AppDefinitionRecord, AppDeploymentCommand, AppDeploymentQuery,
        AppDeploymentRecord,
    },
};
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::net::TcpListener;

#[derive(Clone)]
struct RealAppRepository {
    engine: Arc<AppEngine>,
}

impl RealAppRepository {
    fn new() -> Self {
        Self {
            engine: Arc::new(AppEngine::new_in_memory().unwrap()),
        }
    }

    fn with_engine(engine: Arc<AppEngine>) -> Self {
        Self { engine }
    }
}

impl apps::AppRepositoryApi for RealAppRepository {
    fn deploy_applications(
        &self,
        command: AppDeploymentCommand,
    ) -> Result<AppDeploymentRecord, ApiError> {
        let mut request = EngineAppDeploymentRequest::new(command.name);
        if let Some(category) = command.category {
            request = request.with_category(category);
        }
        if let Some(tenant_id) = command.tenant_id {
            request = request.with_tenant_id(tenant_id);
        }
        for resource in command.resources {
            let definition = parse_app_definition(
                std::str::from_utf8(&resource.resource)
                    .map_err(|error| ApiError::bad_request(error.to_string()))?,
            )
            .map_err(|error| ApiError::bad_request(error.to_string()))?;
            let model =
                EngineAppModel::new().with_app_definition(canonical_definition_to_engine(definition));
            request = request.with_resource_bytes(
                resource.resource_name,
                model,
                resource.resource.clone(),
            );
        }
        let deployment = self.engine.deploy(request)?;
        Ok(AppDeploymentRecord {
            id: deployment.id,
            name: deployment.name,
            category: deployment.category,
            deployed_at: deployment.deployed_at.timestamp_millis(),
            resource_names: deployment.resource_names,
            tenant_id: deployment.tenant_id,
        })
    }

    fn list_app_deployments(
        &self,
        query: AppDeploymentQuery,
    ) -> Result<PagedResponse<AppDeploymentRecord>, ApiError> {
        let mut service_query = self.engine.repository_service().create_deployment_query();
        if let Some(id) = query.id {
            service_query = service_query.id(id);
        }
        if let Some(name) = query.name {
            service_query = service_query.name(name);
        }
        if let Some(tenant_id) = query.tenant_id {
            service_query = service_query.tenant_id(tenant_id);
        }
        let page = if let Some(size) = query.paging.size {
            service_query.page(query.paging.start, size).list_page()?
        } else {
            service_query.list_page()?
        };
        Ok(PagedResponse {
            start: page.start,
            size: page.size,
            total: page.total,
            data: page
                .data
                .into_iter()
                .map(|deployment| AppDeploymentRecord {
                    id: deployment.id,
                    name: deployment.name,
                    category: deployment.category,
                    deployed_at: deployment.deployed_at.timestamp_millis(),
                    resource_names: deployment.resource_names,
                    tenant_id: deployment.tenant_id,
                })
                .collect(),
            sort: None,
            order: None,
        })
    }

    fn get_app_deployment(&self, deployment_id: &str) -> Result<AppDeploymentRecord, ApiError> {
        let deployment = self
            .engine
            .repository_service()
            .get_deployment(deployment_id)?;
        Ok(AppDeploymentRecord {
            id: deployment.id,
            name: deployment.name,
            category: deployment.category,
            deployed_at: deployment.deployed_at.timestamp_millis(),
            resource_names: deployment.resource_names,
            tenant_id: deployment.tenant_id,
        })
    }

    fn list_app_deployment_resources(
        &self,
        deployment_id: &str,
    ) -> Result<Vec<apps::AppDeploymentResourceRecord>, ApiError> {
        Ok(self
            .engine
            .repository_service()
            .get_deployment_resources(deployment_id)?
            .into_iter()
            .map(to_resource_record)
            .collect())
    }

    fn get_app_deployment_resource(
        &self,
        deployment_id: &str,
        resource_name: &str,
    ) -> Result<apps::AppDeploymentResourceRecord, ApiError> {
        Ok(to_resource_record(
            self.engine
                .repository_service()
                .get_deployment_resource(deployment_id, resource_name)?,
        ))
    }

    fn list_app_definitions(
        &self,
        query: AppDefinitionQuery,
    ) -> Result<PagedResponse<AppDefinitionRecord>, ApiError> {
        let mut service_query = self
            .engine
            .repository_service()
            .create_app_definition_query();
        if let Some(deployment_id) = query.deployment_id {
            service_query = service_query.deployment_id(deployment_id);
        }
        let page = service_query.list_page()?;
        Ok(PagedResponse {
            start: page.start,
            size: page.size,
            total: page.total,
            data: page.data.into_iter().map(to_definition_record).collect(),
            sort: None,
            order: None,
        })
    }

    fn get_app_definition(&self, app_definition_id: &str) -> Result<AppDefinitionRecord, ApiError> {
        Ok(to_definition_record(
            self.engine
                .deployment_manager()
                .get_app_definition(app_definition_id)?,
        ))
    }

    fn get_app_definition_resource_data(
        &self,
        app_definition_id: &str,
    ) -> Result<apps::AppDeploymentResourceRecord, ApiError> {
        let definition = self
            .engine
            .deployment_manager()
            .get_app_definition(app_definition_id)?;
        self.get_app_deployment_resource(&definition.deployment_id, &definition.resource_name)
    }

    fn get_app_definition_model(&self, app_definition_id: &str) -> Result<Value, ApiError> {
        let entry = self
            .engine
            .deployment_manager()
            .resolve_app_definition(app_definition_id)?;
        serde_json::to_value(&entry.definition.model)
            .map_err(|error| ApiError::InternalServerError(error.to_string()))
    }
}

impl apps::AppRuntimeApi for RealAppRepository {
    fn list_app_compositions(
        &self,
        query: apps::AppCompositionQuery,
    ) -> Result<PagedResponse<apps::AppCompositionRecord>, ApiError> {
        let mut composition_query = self
            .engine
            .runtime_service()
            .create_resolved_composition_query();
        if let Some(app_definition_id) = query.app_definition_id {
            composition_query = composition_query.app_definition_id(app_definition_id);
        }
        if let Some(app_definition_key) = query.app_definition_key {
            composition_query = composition_query.app_definition_key(app_definition_key);
        }
        if let Some(tenant_id) = query.tenant_id {
            composition_query = composition_query.tenant_id(tenant_id);
        }
        let page = if let Some(size) = query.paging.size {
            composition_query
                .page(query.paging.start, size)
                .list_page()?
        } else {
            composition_query.list_page()?
        };
        Ok(PagedResponse {
            start: page.start,
            size: page.size,
            total: page.total,
            data: page
                .data
                .into_iter()
                .map(|composition| apps::AppCompositionRecord {
                    app_definition_id: composition.app_definition_id,
                    app_definition_key: composition.app_definition_key,
                    app_definition_name: composition.app_definition_name,
                    app_definition_version: composition.version,
                    deployment_id: composition.deployment_id,
                    tenant_id: composition.tenant_id,
                    references: composition
                        .references
                        .into_iter()
                        .map(|reference| apps::AppResolvedReferenceRecord {
                            page_id: reference.page_id,
                            page_name: Some(reference.page_name),
                            reference_id: reference.reference_id,
                            reference_name: reference.reference_name,
                            definition_type: match reference.definition_type {
                                flowable_app_engine::DefinitionType::BpmnProcess => {
                                    "bpmnProcess".to_string()
                                }
                                flowable_app_engine::DefinitionType::DmnDecision => {
                                    "dmnDecision".to_string()
                                }
                                flowable_app_engine::DefinitionType::CmmnCase => {
                                    "cmmnCase".to_string()
                                }
                                flowable_app_engine::DefinitionType::EventRegistry => {
                                    "eventRegistry".to_string()
                                }
                            },
                            resolved_definition_id: reference.resolved_definition_id,
                            resolved_definition_key: reference.resolved_definition_key,
                            resolved_definition_name: reference.resolved_definition_name,
                            resolved_definition_version: reference.resolved_definition_version,
                            resolved_tenant_id: reference.tenant_id,
                        })
                        .collect(),
                })
                .collect(),
            sort: None,
            order: None,
        })
    }

    fn get_app_composition(
        &self,
        app_definition_id: &str,
        filter: apps::AppCompositionFilter,
    ) -> Result<apps::AppCompositionRecord, ApiError> {
        let mut composition = self
            .engine
            .deployment_manager()
            .get_resolved_composition(app_definition_id)?;
        if let Some(definition_type) = filter.definition_type.as_deref() {
            composition.references.retain(|reference| {
                let label = match reference.definition_type {
                    flowable_app_engine::DefinitionType::BpmnProcess => "bpmnProcess",
                    flowable_app_engine::DefinitionType::DmnDecision => "dmnDecision",
                    flowable_app_engine::DefinitionType::CmmnCase => "cmmnCase",
                    flowable_app_engine::DefinitionType::EventRegistry => "eventRegistry",
                };
                label == definition_type
            });
        }
        Ok(apps::AppCompositionRecord {
            app_definition_id: composition.app_definition_id,
            app_definition_key: composition.app_definition_key,
            app_definition_name: composition.app_definition_name,
            app_definition_version: composition.version,
            deployment_id: composition.deployment_id,
            tenant_id: composition.tenant_id,
            references: composition
                .references
                .into_iter()
                .map(|reference| apps::AppResolvedReferenceRecord {
                    page_id: reference.page_id,
                    page_name: Some(reference.page_name),
                    reference_id: reference.reference_id,
                    reference_name: reference.reference_name,
                    definition_type: match reference.definition_type {
                        flowable_app_engine::DefinitionType::BpmnProcess => {
                            "bpmnProcess".to_string()
                        }
                        flowable_app_engine::DefinitionType::DmnDecision => {
                            "dmnDecision".to_string()
                        }
                        flowable_app_engine::DefinitionType::CmmnCase => "cmmnCase".to_string(),
                        flowable_app_engine::DefinitionType::EventRegistry => {
                            "eventRegistry".to_string()
                        }
                    },
                    resolved_definition_id: reference.resolved_definition_id,
                    resolved_definition_key: reference.resolved_definition_key,
                    resolved_definition_name: reference.resolved_definition_name,
                    resolved_definition_version: reference.resolved_definition_version,
                    resolved_tenant_id: reference.tenant_id,
                })
                .collect(),
        })
    }
}

fn to_resource_record(
    resource: flowable_app_engine::AppDeploymentResourceData,
) -> apps::AppDeploymentResourceRecord {
    apps::AppDeploymentResourceRecord {
        deployment_id: resource.deployment_id,
        resource_name: resource.resource_name,
        resource_type: resource.resource_type,
        content_type: resource.content_type,
        bytes: resource.bytes,
    }
}

fn to_definition_record(
    definition: flowable_app_engine::AppDefinitionRecord,
) -> AppDefinitionRecord {
    AppDefinitionRecord {
        id: definition.id,
        key: definition.key,
        name: definition.name,
        description: definition.model.description,
        category: None,
        version: definition.version,
        deployment_id: definition.deployment_id,
        resource_name: definition.resource_name,
        tenant_id: definition.tenant_id,
    }
}

async fn spawn_server() -> (String, reqwest::Client) {
    let repository = Arc::new(RealAppRepository::new());

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());

    let app = Router::new().merge(apps::router(repository.clone(), repository));

    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    (base_url, reqwest::Client::new())
}

async fn deploy_app(client: &reqwest::Client, base_url: &str) -> (String, String, String) {
    let resource_name = "orders-app.app".to_string();
    let resource = json!({
        "key": "orders-app",
        "name": "Orders App",
        "description": "Tracks order operations",
        "pages": [],
        "references": []
    })
    .to_string();

    let response = client
        .post(format!("{base_url}/app-repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "Orders app deployment",
            "resourceName": resource_name,
            "resource": resource
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), reqwest::StatusCode::CREATED);
    let body: Value = response.json().await.unwrap();
    let deployment_id = body["id"].as_str().unwrap().to_string();

    let definitions = client
        .get(format!(
            "{base_url}/app-repository/app-definitions?deploymentId={deployment_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(definitions.status(), reqwest::StatusCode::OK);
    let definitions_body: Value = definitions.json().await.unwrap();
    let app_definition_id = definitions_body["data"][0]["id"]
        .as_str()
        .unwrap()
        .to_string();

    (deployment_id, app_definition_id, resource)
}

#[tokio::test]
async fn app_deployment_resource_endpoints_return_stored_bytes() {
    let (base_url, client) = spawn_server().await;
    let (deployment_id, app_definition_id, resource) = deploy_app(&client, &base_url).await;

    let resources = client
        .get(format!(
            "{base_url}/app-repository/deployments/{deployment_id}/resources"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(resources.status(), reqwest::StatusCode::OK);
    let resources_body: Value = resources.json().await.unwrap();
    assert_eq!(resources_body.as_array().unwrap().len(), 1);
    assert_eq!(resources_body[0]["id"], "orders-app.app");
    assert_eq!(resources_body[0]["mediaType"], "application/json");
    assert_eq!(
        resources_body[0]["url"],
        format!("/app-repository/deployments/{deployment_id}/resources/orders-app.app")
    );
    assert_eq!(
        resources_body[0]["contentUrl"],
        format!("/app-repository/deployments/{deployment_id}/resourcedata/orders-app.app")
    );
    assert_eq!(resources_body[0]["type"], "appDefinition");

    let resource_data = client
        .get(format!(
            "{base_url}/app-repository/deployments/{deployment_id}/resourcedata/orders-app.app"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(resource_data.status(), reqwest::StatusCode::OK);
    assert_eq!(
        resource_data
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .unwrap()
            .to_str()
            .unwrap(),
        "application/json"
    );
    assert_eq!(resource_data.text().await.unwrap(), resource);

    let resource_metadata = client
        .get(format!(
            "{base_url}/app-repository/deployments/{deployment_id}/resources/orders-app.app"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(resource_metadata.status(), reqwest::StatusCode::OK);
    let metadata_body: Value = resource_metadata.json().await.unwrap();
    assert_eq!(metadata_body["id"], "orders-app.app");
    assert_eq!(
        metadata_body["url"],
        format!("/app-repository/deployments/{deployment_id}/resources/orders-app.app")
    );
    assert_eq!(
        metadata_body["contentUrl"],
        format!("/app-repository/deployments/{deployment_id}/resourcedata/orders-app.app")
    );
    assert_eq!(metadata_body["type"], "appDefinition");

    let definition_data = client
        .get(format!(
            "{base_url}/app-repository/app-definitions/{app_definition_id}/resourcedata"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(definition_data.status(), reqwest::StatusCode::OK);
    assert_eq!(
        definition_data
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .unwrap()
            .to_str()
            .unwrap(),
        "application/json"
    );
    assert_eq!(definition_data.text().await.unwrap(), resource);
}

#[tokio::test]
async fn cold_cache_rehydrates_model_and_composition_from_real_app_engine() {
    let catalog = Arc::new(
        flowable_app_engine::InMemoryDefinitionCatalog::builder()
            .with_process_definition("orderProcess", "Order Process", 1, None)
            .build(),
    ) as Arc<dyn flowable_app_engine::DefinitionCatalog>;
    let engine = Arc::new(AppEngine::new_in_memory_with_catalog(catalog).unwrap());
    let repository = Arc::new(RealAppRepository::with_engine(Arc::clone(&engine)));

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let app = Router::new().merge(apps::router(repository.clone(), repository));
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let client = reqwest::Client::new();

    let resource = json!({
        "key": "orders-app",
        "name": "Orders App",
        "description": "Tracks order operations",
        "pages": [{
            "id": "page-process",
            "name": "Orders",
            "pageType": "process",
            "definitionKey": "orderProcess"
        }],
        "references": []
    })
    .to_string();

    let deploy = client
        .post(format!("{base_url}/app-repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "Orders app deployment",
            "resourceName": "orders-app.app",
            "resource": resource
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(deploy.status(), reqwest::StatusCode::CREATED);

    let definitions = client
        .get(format!("{base_url}/app-repository/app-definitions?key=orders-app"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    let definitions_body: Value = definitions.json().await.unwrap();
    let app_definition_id = definitions_body["data"][0]["id"].as_str().unwrap().to_string();

    // Warm then explicitly cold-cache the engine.
    engine
        .deployment_manager()
        .resolve_app_definition(&app_definition_id)
        .unwrap();
    engine
        .deployment_manager()
        .evict_app_definition(&app_definition_id);
    assert!(
        !engine
            .deployment_manager()
            .is_cached(&app_definition_id)
            .unwrap()
    );

    let model = client
        .get(format!(
            "{base_url}/app-repository/app-definitions/{app_definition_id}/model"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(model.status(), reqwest::StatusCode::OK);
    let model_body: Value = model.json().await.unwrap();
    assert_eq!(model_body["key"], "orders-app");
    assert_eq!(model_body["name"], "Orders App");
    assert_eq!(
        model_body["pages"][0]["references"][0]["definition_key"],
        "orderProcess"
    );

    let composition = client
        .get(format!(
            "{base_url}/app-runtime/compositions/{app_definition_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(composition.status(), reqwest::StatusCode::OK);
    let composition_body: Value = composition.json().await.unwrap();
    assert_eq!(composition_body["appDefinitionKey"], "orders-app");
    assert_eq!(composition_body["appDefinitionVersion"], 1);
    assert_eq!(
        composition_body["references"][0]["resolvedDefinitionKey"],
        "orderProcess"
    );
    assert_eq!(
        composition_body["references"][0]["resolvedDefinitionVersion"],
        1
    );
    assert!(
        engine
            .deployment_manager()
            .is_cached(&app_definition_id)
            .unwrap()
    );
}
