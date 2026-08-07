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
    common::PagedResponse,
    error::ApiError,
    routes::dmn::{
        self, DecisionExecutionCommand, DecisionExecutionRecord, DecisionTableQuery,
        DecisionTableRecord, DmnDeploymentCommand, DmnDeploymentQuery, DmnDeploymentRecord,
        DmnResourceDataRecord, HistoricDecisionExecutionQuery, HistoricDecisionExecutionRecord,
    },
    run_server,
};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use tokio::net::TcpListener;

#[derive(Default)]
struct MockDmnApi {
    decision_tables: Mutex<Vec<DecisionTableRecord>>,
    deployments: Mutex<Vec<DmnDeploymentRecord>>,
    resources: Mutex<Vec<(String, String, Vec<u8>)>>,
}

impl MockDmnApi {
    fn with_seed() -> Self {
        let repository = Self::default();
        repository
            .decision_tables
            .lock()
            .unwrap()
            .push(DecisionTableRecord {
                id: "decision-1".to_string(),
                key: "loanEligibility".to_string(),
                name: "Loan Eligibility".to_string(),
                version: 1,
                deployment_id: "deployment-1".to_string(),
                resource_name: "loan-eligibility.dmn".to_string(),
                category: None,
                description: Some("Loan approval rules".to_string()),
                tenant_id: None,
                parent_deployment_id: None,
            });
        repository
            .deployments
            .lock()
            .unwrap()
            .push(DmnDeploymentRecord {
                id: "deployment-1".to_string(),
                name: "Loan decisions".to_string(),
                category: None,
                parent_deployment_id: None,
                deployed_at: 1_713_674_400_000,
                resource_names: vec!["loan-eligibility.dmn".to_string()],
                tenant_id: None,
            });
        repository.resources.lock().unwrap().push((
            "deployment-1".to_string(),
            "loan-eligibility.dmn".to_string(),
            b"<definitions />".to_vec(),
        ));
        repository
    }

    fn with_dmn_query_seed() -> Self {
        let repository = Self::with_seed();
        repository
            .decision_tables
            .lock()
            .unwrap()
            .push(DecisionTableRecord {
                id: "decision-2".to_string(),
                key: "loanEligibility".to_string(),
                name: "Loan Eligibility".to_string(),
                version: 2,
                deployment_id: "deployment-2".to_string(),
                resource_name: "loan-eligibility-v2.dmn".to_string(),
                category: Some("finance".to_string()),
                description: Some("Latest loan approval rules".to_string()),
                tenant_id: Some("tenant-a".to_string()),
                parent_deployment_id: Some("parent-a".to_string()),
            });
        repository
            .decision_tables
            .lock()
            .unwrap()
            .push(DecisionTableRecord {
                id: "decision-3".to_string(),
                key: "pricingDecision".to_string(),
                name: "Pricing Decision".to_string(),
                version: 1,
                deployment_id: "deployment-3".to_string(),
                resource_name: "pricing-decision.dmn".to_string(),
                category: Some("pricing".to_string()),
                description: Some("Pricing rules".to_string()),
                tenant_id: Some("tenant-b".to_string()),
                parent_deployment_id: Some("parent-b".to_string()),
            });
        repository
            .deployments
            .lock()
            .unwrap()
            .push(DmnDeploymentRecord {
                id: "deployment-2".to_string(),
                name: "Loan decisions v2".to_string(),
                category: Some("finance".to_string()),
                parent_deployment_id: Some("parent-a".to_string()),
                deployed_at: 1_713_674_500_000,
                resource_names: vec!["loan-eligibility-v2.dmn".to_string()],
                tenant_id: Some("tenant-a".to_string()),
            });
        repository
            .deployments
            .lock()
            .unwrap()
            .push(DmnDeploymentRecord {
                id: "deployment-3".to_string(),
                name: "Pricing decisions".to_string(),
                category: Some("pricing".to_string()),
                parent_deployment_id: Some("parent-b".to_string()),
                deployed_at: 1_713_674_600_000,
                resource_names: vec!["pricing-decision.dmn".to_string()],
                tenant_id: Some("tenant-b".to_string()),
            });
        repository
    }
}

impl dmn::DmnRepositoryApi for MockDmnApi {
    fn deploy_decision_tables(
        &self,
        command: DmnDeploymentCommand,
    ) -> Result<DmnDeploymentRecord, ApiError> {
        let deployment_id = {
            let deployments = self.deployments.lock().unwrap();
            format!("deployment-{}", deployments.len() + 1)
        };

        let deployment = DmnDeploymentRecord {
            id: deployment_id.clone(),
            name: command.name,
            category: command.category,
            parent_deployment_id: command.parent_deployment_id,
            deployed_at: 1_713_674_500_000,
            resource_names: command
                .resources
                .iter()
                .map(|resource| resource.resource_name.clone())
                .collect(),
            tenant_id: command.tenant_id,
        };

        for resource in &command.resources {
            self.resources.lock().unwrap().push((
                deployment_id.clone(),
                resource.resource_name.clone(),
                resource.resource.clone().into_bytes(),
            ));
        }

        if let Some(resource) = command.resources.first() {
            let decision_id = {
                let decision_tables = self.decision_tables.lock().unwrap();
                format!("decision-{}", decision_tables.len() + 1)
            };
            self.decision_tables
                .lock()
                .unwrap()
                .push(DecisionTableRecord {
                    id: decision_id,
                    key: resource.resource_name.trim_end_matches(".dmn").to_string(),
                    name: resource.resource_name.trim_end_matches(".dmn").to_string(),
                    version: 1,
                    deployment_id,
                    resource_name: resource.resource_name.clone(),
                    category: None,
                    description: None,
                    tenant_id: None,
                    parent_deployment_id: None,
                });
        }

        self.deployments.lock().unwrap().push(deployment.clone());
        Ok(deployment)
    }

    fn list_deployments(
        &self,
        query: DmnDeploymentQuery,
    ) -> Result<PagedResponse<DmnDeploymentRecord>, ApiError> {
        let mut filtered: Vec<DmnDeploymentRecord> =
            self.deployments
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
                            .is_none_or(|value| deployment.name.contains(value))
                        && query.category.as_ref().is_none_or(|value| {
                            deployment.category.as_deref() == Some(value.as_str())
                        })
                        && query.category_not_equals.as_ref().is_none_or(|value| {
                            deployment.category.as_deref() != Some(value.as_str())
                        })
                        && query.parent_deployment_id.as_ref().is_none_or(|value| {
                            deployment.parent_deployment_id.as_deref() == Some(value.as_str())
                        })
                        && query
                            .parent_deployment_id_like
                            .as_ref()
                            .is_none_or(|value| {
                                deployment
                                    .parent_deployment_id
                                    .as_deref()
                                    .is_some_and(|parent| wildcard_like(parent, value))
                            })
                        && query.tenant_id.as_ref().is_none_or(|value| {
                            deployment.tenant_id.as_deref() == Some(value.as_str())
                        })
                        && query.tenant_id_like.as_ref().is_none_or(|value| {
                            deployment
                                .tenant_id
                                .as_deref()
                                .is_some_and(|tenant_id| tenant_id.contains(value))
                        })
                        && (!query.without_tenant_id || deployment.tenant_id.is_none())
                        && query.resource_name.as_ref().is_none_or(|value| {
                            deployment
                                .resource_names
                                .iter()
                                .any(|resource_name| resource_name == value)
                        })
                })
                .cloned()
                .collect();

        filtered.sort_by(|left, right| {
            let ordering = match query.sort.as_deref().unwrap_or("id") {
                "name" => left.name.cmp(&right.name),
                "category" => left.category.cmp(&right.category),
                "deployTime" => left.deployed_at.cmp(&right.deployed_at),
                "parentDeploymentId" => left.parent_deployment_id.cmp(&right.parent_deployment_id),
                "tenantId" => left.tenant_id.cmp(&right.tenant_id),
                _ => left.id.cmp(&right.id),
            };
            if query.order.as_deref() == Some("desc") {
                ordering.reverse()
            } else {
                ordering
            }
        });
        Ok(query.paging.paginate(filtered))
    }

    fn get_deployment(&self, deployment_id: &str) -> Result<DmnDeploymentRecord, ApiError> {
        self.deployments
            .lock()
            .unwrap()
            .iter()
            .find(|deployment| deployment.id == deployment_id)
            .cloned()
            .ok_or_else(|| {
                ApiError::NotFound(format!("DMN deployment '{deployment_id}' was not found"))
            })
    }

    fn delete_deployment(&self, deployment_id: &str, _cascade: bool) -> Result<(), ApiError> {
        self.get_deployment(deployment_id)?;
        self.deployments
            .lock()
            .unwrap()
            .retain(|deployment| deployment.id != deployment_id);
        self.decision_tables
            .lock()
            .unwrap()
            .retain(|decision| decision.deployment_id != deployment_id);
        self.resources
            .lock()
            .unwrap()
            .retain(|(candidate, _, _)| candidate != deployment_id);
        Ok(())
    }

    fn get_deployment_resource_data(
        &self,
        deployment_id: &str,
        resource_name: &str,
    ) -> Result<DmnResourceDataRecord, ApiError> {
        self.resources
            .lock()
            .unwrap()
            .iter()
            .find(|(candidate_deployment_id, candidate_resource_name, _)| {
                candidate_deployment_id == deployment_id && candidate_resource_name == resource_name
            })
            .map(|(_, _, bytes)| DmnResourceDataRecord {
                mime_type: "application/xml".to_string(),
                bytes: bytes.clone(),
            })
            .ok_or_else(|| {
                ApiError::NotFound(format!(
                    "DMN deployment resource '{resource_name}' was not found in deployment '{deployment_id}'"
                ))
            })
    }

    fn list_decision_tables(
        &self,
        query: DecisionTableQuery,
    ) -> Result<PagedResponse<DecisionTableRecord>, ApiError> {
        let mut filtered: Vec<DecisionTableRecord> =
            self.decision_tables
                .lock()
                .unwrap()
                .iter()
                .filter(|decision| {
                    query.id.as_ref().is_none_or(|value| decision.id == *value)
                        && query
                            .key
                            .as_ref()
                            .is_none_or(|value| decision.key == *value)
                        && query
                            .key_like
                            .as_ref()
                            .is_none_or(|value| wildcard_like(&decision.key, value))
                        && query
                            .name
                            .as_ref()
                            .is_none_or(|value| decision.name == *value)
                        && query
                            .name_like
                            .as_ref()
                            .is_none_or(|value| wildcard_like(&decision.name, value))
                        && query.category.as_ref().is_none_or(|value| {
                            decision.category.as_deref() == Some(value.as_str())
                        })
                        && query.category_not_equals.as_ref().is_none_or(|value| {
                            decision.category.as_deref() != Some(value.as_str())
                        })
                        && query
                            .deployment_id
                            .as_ref()
                            .is_none_or(|value| decision.deployment_id == *value)
                        && query.parent_deployment_id.as_ref().is_none_or(|value| {
                            decision.parent_deployment_id.as_deref() == Some(value.as_str())
                        })
                        && query
                            .resource_name
                            .as_ref()
                            .is_none_or(|value| decision.resource_name == *value)
                        && query
                            .resource_name_like
                            .as_ref()
                            .is_none_or(|value| wildcard_like(&decision.resource_name, value))
                        && query.tenant_id.as_ref().is_none_or(|value| {
                            decision.tenant_id.as_deref() == Some(value.as_str())
                        })
                        && query.tenant_id_like.as_ref().is_none_or(|value| {
                            decision
                                .tenant_id
                                .as_deref()
                                .is_some_and(|tenant_id| wildcard_like(tenant_id, value))
                        })
                        && query.version.is_none_or(|value| decision.version == value)
                })
                .cloned()
                .collect();

        if query.latest {
            filtered.sort_by(|left, right| {
                left.key
                    .cmp(&right.key)
                    .then(right.version.cmp(&left.version))
                    .then(left.id.cmp(&right.id))
            });
            let mut seen_keys = std::collections::BTreeSet::new();
            filtered.retain(|record| seen_keys.insert(record.key.clone()));
        }

        sort_decisions(&mut filtered, query.sort.as_deref(), query.order.as_deref());
        Ok(query.paging.paginate(filtered))
    }

    fn get_decision_table(&self, decision_table_id: &str) -> Result<DecisionTableRecord, ApiError> {
        self.decision_tables
            .lock()
            .unwrap()
            .iter()
            .find(|decision| decision.id == decision_table_id)
            .cloned()
            .ok_or_else(|| {
                ApiError::NotFound(format!(
                    "Decision table '{decision_table_id}' was not found"
                ))
            })
    }

    fn get_drd(&self, _drd_id: &str) -> Result<Value, ApiError> {
        Err(ApiError::NotFound("DRD not found".to_string()))
    }

    fn list_drds(&self) -> Result<PagedResponse<Value>, ApiError> {
        Ok(PagedResponse {
            start: 0,
            size: 0,
            total: 0,
            data: Vec::new(),
            sort: None,
            order: None,
        })
    }

    fn get_drd_resource_data(&self, _drd_id: &str) -> Result<dmn::DmnResourceDataRecord, ApiError> {
        Err(ApiError::NotFound(
            "DRD resource data not found".to_string(),
        ))
    }
}

fn wildcard_like(candidate: &str, pattern: &str) -> bool {
    match (pattern.strip_prefix('%'), pattern.strip_suffix('%')) {
        (Some(_), Some(_)) if pattern.len() >= 2 => {
            candidate.contains(&pattern[1..pattern.len() - 1])
        }
        (Some(suffix), _) => candidate.ends_with(suffix),
        (_, Some(prefix)) => candidate.starts_with(prefix),
        _ => candidate.contains(pattern),
    }
}

fn sort_decisions(decisions: &mut [DecisionTableRecord], sort: Option<&str>, order: Option<&str>) {
    let descending = matches!(order, Some("desc"));
    decisions.sort_by(|left, right| {
        let ordering = match sort.unwrap_or("name") {
            "id" => left.id.cmp(&right.id),
            "key" => left.key.cmp(&right.key),
            "category" => left.category.cmp(&right.category),
            "deploymentId" => left.deployment_id.cmp(&right.deployment_id),
            "tenantId" => left.tenant_id.cmp(&right.tenant_id),
            "version" => left.version.cmp(&right.version),
            _ => left.name.cmp(&right.name),
        };
        if descending {
            ordering.reverse()
        } else {
            ordering
        }
        .then_with(|| left.id.cmp(&right.id))
    });
}

impl dmn::DmnRuntimeApi for MockDmnApi {
    fn execute_decision(
        &self,
        _command: DecisionExecutionCommand,
    ) -> Result<DecisionExecutionRecord, ApiError> {
        Err(ApiError::InternalServerError(
            "runtime stub not used in repository tests".to_string(),
        ))
    }
}

impl dmn::DmnHistoryApi for MockDmnApi {
    fn list_historic_decision_executions(
        &self,
        _query: HistoricDecisionExecutionQuery,
    ) -> Result<PagedResponse<HistoricDecisionExecutionRecord>, ApiError> {
        Ok(PagedResponse {
            start: 0,
            size: 0,
            total: 0,
            data: Vec::new(),
            sort: None,
            order: None,
        })
    }

    fn delete_historic_decision_execution(
        &self,
        _historic_decision_execution_id: &str,
    ) -> Result<(), ApiError> {
        Ok(())
    }

    fn bulk_delete_historic_decision_executions(
        &self,
        _historic_decision_execution_ids: Vec<String>,
    ) -> Result<(), ApiError> {
        Ok(())
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

async fn spawn_server(api: Arc<MockDmnApi>) -> (String, reqwest::Client) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());

    let repository: dmn::DynDmnRepository = api.clone();
    let runtime: dmn::DynDmnRuntime = api.clone();
    let history: dmn::DynDmnHistory = api;
    let app = Router::new()
        .merge(dmn::router(repository, runtime, history))
        .layer(middleware::from_fn(auth_middleware));

    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    (base_url, reqwest::Client::new())
}

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

#[tokio::test]
async fn dmn_repository_routes_follow_common_rest_contract() {
    let (base_url, client) = spawn_server(Arc::new(MockDmnApi::with_seed())).await;

    let deploy_response = client
        .post(format!("{}/dmn-repository/deployments", base_url))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "Pricing decisions",
            "resourceName": "pricing-decision.dmn",
            "resource": "<definitions />"
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(deploy_response.status(), StatusCode::CREATED);
    let deploy_body: Value = deploy_response.json().await.unwrap();
    assert_eq!(deploy_body["name"], "Pricing decisions");
    assert_eq!(deploy_body["resourceNames"][0], "pricing-decision.dmn");

    let list_response = client
        .get(format!(
            "{}/dmn-repository/decision-tables?key=loanEligibility&start=0&size=10",
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
    let decision = &list_body["data"][0];
    assert_eq!(decision["id"], "decision-1");
    assert_eq!(decision["key"], "loanEligibility");
    assert_eq!(decision["deploymentId"], "deployment-1");
    assert_eq!(decision["resourceName"], "loan-eligibility.dmn");

    let get_response = client
        .get(format!(
            "{}/dmn-repository/decision-tables/decision-1",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(get_response.status(), StatusCode::OK);
    let get_body: Value = get_response.json().await.unwrap();
    assert_eq!(get_body["id"], "decision-1");
    assert_eq!(get_body["key"], "loanEligibility");
    assert_eq!(get_body["version"], 1);
}

#[tokio::test]
async fn dmn_deployment_lifecycle_routes_match_repository_contract() {
    let (base_url, client) = spawn_server(Arc::new(MockDmnApi::with_dmn_query_seed())).await;

    let collection = client
        .get(format!(
            "{}/dmn-repository/deployments?nameLike=Loan&resourceName=loan-eligibility.dmn&start=0&size=10",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(collection.status(), StatusCode::OK);
    let collection_body: Value = collection.json().await.unwrap();
    assert_eq!(collection_body["start"], 0);
    assert_eq!(collection_body["size"], 1);
    assert_eq!(collection_body["total"], 1);
    assert_eq!(collection_body["data"][0]["id"], "deployment-1");

    let canonical_filtered_collection = client
        .get(format!(
            "{}/dmn-repository/deployments?tenantIdLike=tenant-&withoutTenantId=false&sort=tenantId&order=desc&start=0&size=2",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(canonical_filtered_collection.status(), StatusCode::OK);
    let canonical_filtered_body: Value = canonical_filtered_collection.json().await.unwrap();
    assert_eq!(canonical_filtered_body["total"], 2);
    assert_eq!(canonical_filtered_body["data"][0]["id"], "deployment-3");
    assert_eq!(canonical_filtered_body["data"][1]["id"], "deployment-2");

    let unsupported_sort = client
        .get(format!(
            "{}/dmn-repository/deployments?sort=unsupportedDeploymentSort",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(unsupported_sort.status(), StatusCode::BAD_REQUEST);
    let unsupported_sort_body: Value = unsupported_sort.json().await.unwrap();
    assert_eq!(unsupported_sort_body["code"], "BAD_REQUEST");
    assert!(
        unsupported_sort_body["details"]
            .as_str()
            .unwrap()
            .contains("Unsupported sort property 'unsupportedDeploymentSort'")
    );

    let resources = client
        .get(format!(
            "{}/dmn-repository/deployments/deployment-1/resources",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(resources.status(), StatusCode::OK);
    let resources_body: Value = resources.json().await.unwrap();
    assert_eq!(resources_body[0]["id"], "loan-eligibility.dmn");
    assert_eq!(resources_body[0]["type"], "dmn");

    let resource = client
        .get(format!(
            "{}/dmn-repository/deployments/deployment-1/resources/loan-eligibility.dmn",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(resource.status(), StatusCode::OK);

    let resource_data = client
        .get(format!(
            "{}/dmn-repository/deployments/deployment-1/resourcedata/loan-eligibility.dmn",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(resource_data.status(), StatusCode::OK);
    assert_eq!(resource_data.text().await.unwrap(), "<definitions />");

    let delete = client
        .delete(format!(
            "{}/dmn-repository/deployments/deployment-1?cascade=true",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(delete.status(), StatusCode::NO_CONTENT);

    let missing = client
        .get(format!(
            "{}/dmn-repository/deployments/deployment-1",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);

    let remaining_collection: Value = client
        .get(format!("{}/dmn-repository/deployments", base_url))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(remaining_collection["total"], 2);
}

#[tokio::test]
async fn dmn_repository_routes_expose_decision_aliases_and_models() {
    let (base_url, client) = spawn_server(Arc::new(MockDmnApi::with_seed())).await;

    let decisions_response = client
        .get(format!(
            "{}/dmn-repository/decisions?key=loanEligibility&start=0&size=10",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(decisions_response.status(), StatusCode::OK);
    let decisions_body: Value = decisions_response.json().await.unwrap();
    assert_eq!(decisions_body["total"], 1);
    assert_eq!(decisions_body["data"][0]["id"], "decision-1");
    assert_eq!(decisions_body["data"][0]["key"], "loanEligibility");

    let decision_response = client
        .get(format!("{}/dmn-repository/decisions/decision-1", base_url))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(decision_response.status(), StatusCode::OK);
    let decision_body: Value = decision_response.json().await.unwrap();
    assert_eq!(decision_body["id"], "decision-1");
    assert_eq!(decision_body["key"], "loanEligibility");

    let model_response = client
        .get(format!(
            "{}/dmn-repository/decision-tables/decision-1/model",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(model_response.status(), StatusCode::OK);
    let model_body: Value = model_response.json().await.unwrap();
    assert_eq!(model_body["id"], "decision-1");
    assert_eq!(model_body["key"], "loanEligibility");

    let decision_model_response = client
        .get(format!(
            "{}/dmn-repository/decisions/decision-1/model",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(decision_model_response.status(), StatusCode::OK);
    let decision_model_body: Value = decision_model_response.json().await.unwrap();
    assert_eq!(decision_model_body["id"], "decision-1");
    assert_eq!(decision_model_body["key"], "loanEligibility");
}

#[tokio::test]
async fn dmn_decision_collection_accepts_canonical_filters_latest_sort_and_paging() {
    let (base_url, client) = spawn_server(Arc::new(MockDmnApi::with_dmn_query_seed())).await;

    let key_like_response = client
        .get(format!(
            "{}/dmn-repository/decisions?keyLike=%25Eligibility&latest=true&sort=version&order=desc&start=0&size=10",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(key_like_response.status(), StatusCode::OK);
    let key_like_body: Value = key_like_response.json().await.unwrap();
    assert_eq!(key_like_body["total"], 1);
    assert_eq!(key_like_body["data"][0]["id"], "decision-2");
    assert_eq!(key_like_body["data"][0]["version"], 2);

    let resource_like_response = client
        .get(format!(
            "{}/dmn-repository/decision-tables?resourceNameLike=%25pricing%25&categoryNotEquals=finance&tenantIdLike=%25-b&sort=key&order=asc",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(resource_like_response.status(), StatusCode::OK);
    let resource_like_body: Value = resource_like_response.json().await.unwrap();
    assert_eq!(resource_like_body["total"], 1);
    assert_eq!(resource_like_body["data"][0]["id"], "decision-3");

    let parent_deployment_response = client
        .get(format!(
            "{}/dmn-repository/decisions?parentDeploymentId=parent-a",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(parent_deployment_response.status(), StatusCode::OK);
    let parent_deployment_body: Value = parent_deployment_response.json().await.unwrap();
    assert_eq!(parent_deployment_body["total"], 1);
    assert_eq!(parent_deployment_body["data"][0]["id"], "decision-2");
}

#[tokio::test]
async fn dmn_decision_collection_rejects_unsupported_sort_order_values() {
    let (base_url, client) = spawn_server(Arc::new(MockDmnApi::with_dmn_query_seed())).await;

    let bad_sort = client
        .get(format!(
            "{}/dmn-repository/decisions?sort=unsupported",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(bad_sort.status(), StatusCode::BAD_REQUEST);
    let bad_sort_body: Value = bad_sort.json().await.unwrap();
    assert_eq!(bad_sort_body["code"], "BAD_REQUEST");
    assert!(bad_sort_body["details"].as_str().unwrap().contains("sort"));

    let bad_order = client
        .get(format!(
            "{}/dmn-repository/decisions?order=sideways",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(bad_order.status(), StatusCode::BAD_REQUEST);
    let bad_order_body: Value = bad_order.json().await.unwrap();
    assert_eq!(bad_order_body["code"], "BAD_REQUEST");
    assert!(
        bad_order_body["details"]
            .as_str()
            .unwrap()
            .contains("order")
    );
}

#[tokio::test]
async fn dmn_repository_routes_enforce_auth_and_structured_errors() {
    let (base_url, client) = spawn_server(Arc::new(MockDmnApi::with_seed())).await;

    let unauthorized = client
        .get(format!("{}/dmn-repository/decision-tables", base_url))
        .send()
        .await
        .unwrap();

    assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);
    let unauthorized_body: Value = unauthorized.json().await.unwrap();
    assert_eq!(unauthorized_body["code"], "UNAUTHORIZED");

    let bad_query = client
        .get(format!(
            "{}/dmn-repository/decision-tables?unsupportedDecisionFilter=tenant-a",
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
            .contains("unsupportedDecisionFilter")
    );

    let bad_deployment_query = client
        .get(format!(
            "{}/dmn-repository/deployments?unknown=value",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(bad_deployment_query.status(), StatusCode::BAD_REQUEST);
    let bad_deployment_query_body: Value = bad_deployment_query.json().await.unwrap();
    assert_eq!(bad_deployment_query_body["code"], "BAD_REQUEST");
    assert!(
        bad_deployment_query_body["details"]
            .as_str()
            .unwrap()
            .contains("unknown")
    );

    let invalid_delete_query = client
        .delete(format!(
            "{}/dmn-repository/deployments/deployment-1?cascade=maybe",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(invalid_delete_query.status(), StatusCode::BAD_REQUEST);
    let invalid_delete_query_body: Value = invalid_delete_query.json().await.unwrap();
    assert_eq!(invalid_delete_query_body["code"], "BAD_REQUEST");

    let missing = client
        .get(format!(
            "{}/dmn-repository/decision-tables/missing-decision",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
    let missing_body: Value = missing.json().await.unwrap();
    assert_eq!(missing_body["code"], "NOT_FOUND");

    let invalid_deploy = client
        .post(format!("{}/dmn-repository/deployments", base_url))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "Broken deployment",
            "resources": []
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(invalid_deploy.status(), StatusCode::BAD_REQUEST);
    let invalid_deploy_body: Value = invalid_deploy.json().await.unwrap();
    assert_eq!(invalid_deploy_body["code"], "BAD_REQUEST");
}

#[tokio::test]
async fn dmn_real_repository_deployment_delete_removes_engine_deployment() {
    let (base_url, client) = spawn_real_server("rest-dmn-deployment-delete").await;

    let dmn_xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="https://www.omg.org/spec/DMN/20191111/MODEL/" id="deleteDecisionDefinitions" name="Delete Decisions">
  <decision id="deleteDecision" name="Delete Decision">
    <decisionTable id="deleteTable" hitPolicy="FIRST">
      <input id="input1" label="Amount">
        <inputExpression id="inputExpression1" typeRef="number">
          <text>amount</text>
        </inputExpression>
      </input>
      <output id="output1" label="Approved" name="approved" typeRef="boolean" />
      <rule id="rule1">
        <inputEntry id="inputEntry1"><text>-</text></inputEntry>
        <outputEntry id="outputEntry1"><text>true</text></outputEntry>
      </rule>
    </decisionTable>
  </decision>
</definitions>"#;

    let deploy: Value = client
        .post(format!("{}/dmn-repository/deployments", base_url))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "Delete decision deployment",
            "resourceName": "delete-decision.dmn",
            "resource": dmn_xml
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let deployment_id = deploy["id"].as_str().unwrap();

    let listed: Value = client
        .get(format!(
            "{}/dmn-repository/deployments?id={deployment_id}",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(listed["total"], 1);

    let delete = client
        .delete(format!(
            "{}/dmn-repository/deployments/{deployment_id}?cascade=false",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(delete.status(), StatusCode::NO_CONTENT);

    let missing = client
        .get(format!(
            "{}/dmn-repository/deployments/{deployment_id}",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);

    let empty: Value = client
        .get(format!(
            "{}/dmn-repository/deployments?id={deployment_id}",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(empty["total"], 0);
}

#[tokio::test]
async fn dmn_real_repository_deployment_metadata_uses_canonical_fields_and_filters() {
    let (base_url, client) = spawn_real_server("rest-dmn-deployment-metadata").await;

    let dmn_xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="https://www.omg.org/spec/DMN/20191111/MODEL/" id="metadataDecisionDefinitions" name="Metadata Decisions">
  <decision id="metadataDecision" name="Metadata Decision">
    <decisionTable id="metadataTable" hitPolicy="FIRST">
      <input id="input1" label="Amount"><inputExpression id="inputExpression1" typeRef="number"><text>amount</text></inputExpression></input>
      <output id="output1" label="Approved" name="approved" typeRef="boolean" />
      <rule id="rule1"><inputEntry id="inputEntry1"><text>-</text></inputEntry><outputEntry id="outputEntry1"><text>true</text></outputEntry></rule>
    </decisionTable>
  </decision>
</definitions>"#;

    let deploy_response = client
        .post(format!("{}/dmn-repository/deployments", base_url))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "Metadata deployment",
            "category": "retail",
            "parentDeploymentId": "case-parent-1",
            "resourceName": "metadata-decision.dmn",
            "resource": dmn_xml
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(deploy_response.status(), StatusCode::CREATED);
    let deploy: Value = deploy_response.json().await.unwrap();
    let deployment_id = deploy["id"].as_str().unwrap();
    assert_eq!(deploy["category"], "retail");
    assert_eq!(deploy["parentDeploymentId"], "case-parent-1");

    let by_category: Value = client
        .get(format!(
            "{}/dmn-repository/deployments?category=retail",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(by_category["total"], 1);
    assert_eq!(by_category["data"][0]["id"], deployment_id);

    let by_parent_like: Value = client
        .get(format!(
            "{}/dmn-repository/deployments?categoryNotEquals=finance&parentDeploymentIdLike=%25parent-1",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(by_parent_like["total"], 1);
    assert_eq!(
        by_parent_like["data"][0]["parentDeploymentId"],
        "case-parent-1"
    );

    let missing_parent: Value = client
        .get(format!(
            "{}/dmn-repository/deployments?parentDeploymentId=missing-parent",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(missing_parent["total"], 0);
}

#[tokio::test]
async fn dmn_real_repository_decision_query_honors_latest_and_sort() {
    let (base_url, client) = spawn_real_server("rest-dmn-decision-query").await;

    let first_dmn = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="https://www.omg.org/spec/DMN/20191111/MODEL/" id="queryDecisionDefinitions1" name="Query Decisions">
  <decision id="queryDecision" name="Query Decision">
    <decisionTable id="queryTable1" hitPolicy="FIRST">
      <input id="input1" label="Amount"><inputExpression id="inputExpression1" typeRef="number"><text>amount</text></inputExpression></input>
      <output id="output1" label="Approved" name="approved" typeRef="boolean" />
      <rule id="rule1"><inputEntry id="inputEntry1"><text>-</text></inputEntry><outputEntry id="outputEntry1"><text>false</text></outputEntry></rule>
    </decisionTable>
  </decision>
</definitions>"#;

    let second_dmn = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="https://www.omg.org/spec/DMN/20191111/MODEL/" id="queryDecisionDefinitions2" name="Query Decisions">
  <decision id="queryDecision" name="Query Decision">
    <decisionTable id="queryTable2" hitPolicy="FIRST">
      <input id="input1" label="Amount"><inputExpression id="inputExpression1" typeRef="number"><text>amount</text></inputExpression></input>
      <output id="output1" label="Approved" name="approved" typeRef="boolean" />
      <rule id="rule1"><inputEntry id="inputEntry1"><text>-</text></inputEntry><outputEntry id="outputEntry1"><text>true</text></outputEntry></rule>
    </decisionTable>
  </decision>
</definitions>"#;

    for (name, resource_name, resource, tenant_id) in [
        (
            "Query deployment one",
            "query-decision-one.dmn",
            first_dmn,
            "tenant-b",
        ),
        (
            "Query deployment two",
            "query-decision-two.dmn",
            second_dmn,
            "tenant-b",
        ),
    ] {
        let response = client
            .post(format!("{}/dmn-repository/deployments", base_url))
            .basic_auth("admin", Some("test"))
            .json(&json!({
                "name": name,
                "resourceName": resource_name,
                "resource": resource,
                "tenantId": tenant_id
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);
    }

    let latest: Value = client
        .get(format!(
            "{}/dmn-repository/decisions?keyLike=%25Decision&latest=true&sort=version&order=desc",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(latest["total"], 1);
    assert_eq!(latest["data"][0]["key"], "queryDecision");
    assert_eq!(latest["data"][0]["version"], 2);

    let resource_like: Value = client
        .get(format!(
            "{}/dmn-repository/decision-tables?resourceNameLike=%25-two.dmn&tenantIdLike=%25-b",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(resource_like["total"], 1);
    assert_eq!(
        resource_like["data"][0]["resourceName"],
        "query-decision-two.dmn"
    );
    assert_eq!(resource_like["data"][0]["tenantId"], "tenant-b");
}
