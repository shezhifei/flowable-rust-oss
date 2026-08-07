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
    routes::cmmn::{
        self, CaseDefinitionQuery, CaseDefinitionRecord, CaseInstanceQuery, CaseInstanceRecord,
        CmmnDeploymentCommand, CmmnDeploymentQuery, CmmnDeploymentRecord, CmmnResourceDataRecord,
        HistoricCaseInstanceQuery, HistoricCaseInstanceRecord, HistoricPlanItemInstanceQuery,
        HistoricPlanItemInstanceRecord, PlanItemInstanceQuery, PlanItemInstanceRecord,
        StartCaseInstanceCommand,
    },
    run_server,
};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use tokio::net::TcpListener;

#[derive(Default)]
struct MockCmmnApi {
    case_definitions: Mutex<Vec<CaseDefinitionRecord>>,
    deployments: Mutex<Vec<CmmnDeploymentRecord>>,
    resources: Mutex<Vec<(String, String, Vec<u8>)>>,
}

impl MockCmmnApi {
    fn with_seed() -> Self {
        let repository = Self::default();
        repository
            .case_definitions
            .lock()
            .unwrap()
            .push(CaseDefinitionRecord {
                id: "case-definition-1".to_string(),
                key: "loanApprovalCase".to_string(),
                name: "Loan Approval Case".to_string(),
                version: 1,
                deployment_id: "deployment-1".to_string(),
                resource_name: "loan-approval-case.cmmn".to_string(),
                category: None,
                description: Some("Loan approval case model".to_string()),
                tenant_id: None,
                parent_deployment_id: None,
            });
        repository
            .deployments
            .lock()
            .unwrap()
            .push(CmmnDeploymentRecord {
                id: "deployment-1".to_string(),
                name: "Loan cases".to_string(),
                deployed_at: 1_713_674_400_000,
                resource_names: vec!["loan-approval-case.cmmn".to_string()],
                tenant_id: None,
            });
        repository.resources.lock().unwrap().push((
            "deployment-1".to_string(),
            "loan-approval-case.cmmn".to_string(),
            b"<definitions />".to_vec(),
        ));
        repository
    }
}

impl cmmn::CmmnRepositoryApi for MockCmmnApi {
    fn deploy_case_definitions(
        &self,
        command: CmmnDeploymentCommand,
    ) -> Result<CmmnDeploymentRecord, ApiError> {
        let deployment_id = {
            let deployments = self.deployments.lock().unwrap();
            format!("deployment-{}", deployments.len() + 1)
        };

        let deployment = CmmnDeploymentRecord {
            id: deployment_id.clone(),
            name: command.name,
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
            let case_definition_id = {
                let case_definitions = self.case_definitions.lock().unwrap();
                format!("case-definition-{}", case_definitions.len() + 1)
            };
            self.case_definitions
                .lock()
                .unwrap()
                .push(CaseDefinitionRecord {
                    id: case_definition_id,
                    key: resource.resource_name.trim_end_matches(".cmmn").to_string(),
                    name: resource.resource_name.trim_end_matches(".cmmn").to_string(),
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
        query: CmmnDeploymentQuery,
    ) -> Result<PagedResponse<CmmnDeploymentRecord>, ApiError> {
        let filtered =
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
                        && query.tenant_id.as_ref().is_none_or(|value| {
                            deployment.tenant_id.as_deref() == Some(value.as_str())
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

        Ok(query.paging.paginate(filtered))
    }

    fn get_deployment(&self, deployment_id: &str) -> Result<CmmnDeploymentRecord, ApiError> {
        self.deployments
            .lock()
            .unwrap()
            .iter()
            .find(|deployment| deployment.id == deployment_id)
            .cloned()
            .ok_or_else(|| {
                ApiError::NotFound(format!("CMMN deployment '{deployment_id}' was not found"))
            })
    }

    fn delete_deployment(&self, deployment_id: &str, _cascade: bool) -> Result<(), ApiError> {
        self.get_deployment(deployment_id)?;
        self.deployments
            .lock()
            .unwrap()
            .retain(|deployment| deployment.id != deployment_id);
        self.case_definitions
            .lock()
            .unwrap()
            .retain(|definition| definition.deployment_id != deployment_id);
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
    ) -> Result<CmmnResourceDataRecord, ApiError> {
        self.resources
            .lock()
            .unwrap()
            .iter()
            .find(|(candidate_deployment_id, candidate_resource_name, _)| {
                candidate_deployment_id == deployment_id && candidate_resource_name == resource_name
            })
            .map(|(_, _, bytes)| CmmnResourceDataRecord {
                mime_type: "application/xml".to_string(),
                bytes: bytes.clone(),
            })
            .ok_or_else(|| {
                ApiError::NotFound(format!(
                    "CMMN deployment resource '{resource_name}' was not found in deployment '{deployment_id}'"
                ))
            })
    }

    fn list_case_definitions(
        &self,
        query: CaseDefinitionQuery,
    ) -> Result<PagedResponse<CaseDefinitionRecord>, ApiError> {
        let filtered = self
            .case_definitions
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
                    && query
                        .version
                        .is_none_or(|value| definition.version == value)
            })
            .cloned()
            .collect();

        Ok(query.paging.paginate(filtered))
    }

    fn get_case_definition(
        &self,
        case_definition_id: &str,
    ) -> Result<CaseDefinitionRecord, ApiError> {
        self.case_definitions
            .lock()
            .unwrap()
            .iter()
            .find(|definition| definition.id == case_definition_id)
            .cloned()
            .ok_or_else(|| {
                ApiError::NotFound(format!(
                    "Case definition '{case_definition_id}' was not found"
                ))
            })
    }
}

impl cmmn::CmmnRuntimeApi for MockCmmnApi {
    fn start_case_instance(
        &self,
        _command: StartCaseInstanceCommand,
    ) -> Result<CaseInstanceRecord, ApiError> {
        Err(ApiError::InternalServerError(
            "runtime stub not used in repository tests".to_string(),
        ))
    }

    fn list_case_instances(
        &self,
        _query: CaseInstanceQuery,
    ) -> Result<PagedResponse<CaseInstanceRecord>, ApiError> {
        Ok(PagedResponse {
            start: 0,
            size: 0,
            total: 0,
            data: Vec::new(),
            sort: None,
            order: None,
        })
    }

    fn list_plan_item_instances(
        &self,
        _query: PlanItemInstanceQuery,
    ) -> Result<PagedResponse<PlanItemInstanceRecord>, ApiError> {
        Ok(PagedResponse {
            start: 0,
            size: 0,
            total: 0,
            data: Vec::new(),
            sort: None,
            order: None,
        })
    }

    fn complete_plan_item_instance(&self, _plan_item_instance_id: &str) -> Result<(), ApiError> {
        Err(ApiError::InternalServerError(
            "runtime stub not used in repository tests".to_string(),
        ))
    }
}

impl cmmn::CmmnHistoryApi for MockCmmnApi {
    fn list_historic_case_instances(
        &self,
        _query: HistoricCaseInstanceQuery,
    ) -> Result<PagedResponse<HistoricCaseInstanceRecord>, ApiError> {
        Ok(PagedResponse {
            start: 0,
            size: 0,
            total: 0,
            data: Vec::new(),
            sort: None,
            order: None,
        })
    }

    fn list_historic_plan_item_instances(
        &self,
        _query: HistoricPlanItemInstanceQuery,
    ) -> Result<PagedResponse<HistoricPlanItemInstanceRecord>, ApiError> {
        Ok(PagedResponse {
            start: 0,
            size: 0,
            total: 0,
            data: Vec::new(),
            sort: None,
            order: None,
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

async fn spawn_server(api: Arc<MockCmmnApi>) -> (String, reqwest::Client) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());

    let repository: cmmn::DynCmmnRepository = api.clone();
    let runtime: cmmn::DynCmmnRuntime = api.clone();
    let history: cmmn::DynCmmnHistory = api;
    let app = Router::new()
        .merge(cmmn::router(repository, runtime, history))
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
async fn cmmn_repository_routes_follow_common_rest_contract() {
    let (base_url, client) = spawn_server(Arc::new(MockCmmnApi::with_seed())).await;

    let deploy_response = client
        .post(format!("{}/cmmn-repository/deployments", base_url))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "Customer onboarding cases",
            "resourceName": "customer-onboarding.cmmn",
            "resource": "<definitions />"
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(deploy_response.status(), StatusCode::CREATED);
    let deploy_body: Value = deploy_response.json().await.unwrap();
    assert_eq!(deploy_body["name"], "Customer onboarding cases");
    assert_eq!(deploy_body["resourceNames"][0], "customer-onboarding.cmmn");

    let list_response = client
        .get(format!(
            "{}/cmmn-repository/case-definitions?key=loanApprovalCase&start=0&size=10",
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
    let definition = &list_body["data"][0];
    assert_eq!(definition["id"], "case-definition-1");
    assert_eq!(definition["key"], "loanApprovalCase");
    assert_eq!(definition["deploymentId"], "deployment-1");
    assert_eq!(definition["resourceName"], "loan-approval-case.cmmn");

    let get_response = client
        .get(format!(
            "{}/cmmn-repository/case-definitions/case-definition-1",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(get_response.status(), StatusCode::OK);
    let get_body: Value = get_response.json().await.unwrap();
    assert_eq!(get_body["id"], "case-definition-1");
    assert_eq!(get_body["key"], "loanApprovalCase");
    assert_eq!(get_body["version"], 1);
}

#[tokio::test]
async fn cmmn_deployment_lifecycle_routes_match_repository_contract() {
    let (base_url, client) = spawn_server(Arc::new(MockCmmnApi::with_seed())).await;

    let collection = client
        .get(format!(
            "{}/cmmn-repository/deployments?nameLike=Loan&resourceName=loan-approval-case.cmmn&start=0&size=10",
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

    let resources = client
        .get(format!(
            "{}/cmmn-repository/deployments/deployment-1/resources",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(resources.status(), StatusCode::OK);
    let resources_body: Value = resources.json().await.unwrap();
    assert_eq!(resources_body[0]["id"], "loan-approval-case.cmmn");
    assert_eq!(resources_body[0]["type"], "cmmn");

    let resource = client
        .get(format!(
            "{}/cmmn-repository/deployments/deployment-1/resources/loan-approval-case.cmmn",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(resource.status(), StatusCode::OK);

    let resource_data = client
        .get(format!(
            "{}/cmmn-repository/deployments/deployment-1/resourcedata/loan-approval-case.cmmn",
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
            "{}/cmmn-repository/deployments/deployment-1?cascade=true",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(delete.status(), StatusCode::NO_CONTENT);

    let missing = client
        .get(format!(
            "{}/cmmn-repository/deployments/deployment-1",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);

    let empty_collection: Value = client
        .get(format!("{}/cmmn-repository/deployments", base_url))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(empty_collection["total"], 0);
}

#[tokio::test]
async fn cmmn_repository_routes_enforce_auth_and_structured_errors() {
    let (base_url, client) = spawn_server(Arc::new(MockCmmnApi::with_seed())).await;

    let unauthorized = client
        .get(format!("{}/cmmn-repository/case-definitions", base_url))
        .send()
        .await
        .unwrap();

    assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);
    let unauthorized_body: Value = unauthorized.json().await.unwrap();
    assert_eq!(unauthorized_body["code"], "UNAUTHORIZED");

    // P133: tenantId is now supported (CaseDefinitionCollectionResource.java:150).
    // Reject still-unsupported startableByUser (no reliable engine identity source wired).
    let bad_query = client
        .get(format!(
            "{}/cmmn-repository/case-definitions?startableByUser=kermit",
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
            .contains("startableByUser")
    );

    let bad_deployment_query = client
        .get(format!(
            "{}/cmmn-repository/deployments?unknown=value",
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
            "{}/cmmn-repository/deployments/deployment-1?cascade=maybe",
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
            "{}/cmmn-repository/case-definitions/missing-definition",
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
        .post(format!("{}/cmmn-repository/deployments", base_url))
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
async fn cmmn_real_repository_deployment_delete_removes_engine_deployment() {
    let (base_url, client) = spawn_real_server("rest-cmmn-deployment-delete").await;

    let cmmn_xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/CMMN/20151109/MODEL">
  <case id="deleteCase" name="Delete Case">
    <casePlanModel id="casePlan" name="Case Plan" />
  </case>
</definitions>"#;

    let deploy: Value = client
        .post(format!("{}/cmmn-repository/deployments", base_url))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "Delete case deployment",
            "resourceName": "delete-case.cmmn",
            "resource": cmmn_xml
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
            "{}/cmmn-repository/deployments?id={deployment_id}",
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
            "{}/cmmn-repository/deployments/{deployment_id}?cascade=false",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(delete.status(), StatusCode::NO_CONTENT);

    let missing = client
        .get(format!(
            "{}/cmmn-repository/deployments/{deployment_id}",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);

    let empty: Value = client
        .get(format!(
            "{}/cmmn-repository/deployments?id={deployment_id}",
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
async fn cmmn_case_definition_linked_repository_routes_are_available() {
    let (base_url, client) = spawn_server(Arc::new(MockCmmnApi::with_seed())).await;

    for path in ["decision-tables", "decisions", "form-definitions"] {
        let response = client
            .get(format!(
                "{}/cmmn-repository/case-definitions/case-definition-1/{path}?start=0&size=10",
                base_url
            ))
            .basic_auth("admin", Some("test"))
            .send()
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK, "path {path}");
        let body: Value = response.json().await.unwrap();
        assert_eq!(body["start"], 0, "path {path}");
        assert_eq!(body["size"], 0, "path {path}");
        assert_eq!(body["total"], 0, "path {path}");
        assert_eq!(body["data"].as_array().unwrap().len(), 0, "path {path}");
    }

    let start_form = client
        .get(format!(
            "{}/cmmn-repository/case-definitions/case-definition-1/start-form",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(start_form.status(), StatusCode::NOT_FOUND);
    let body: Value = start_form.json().await.unwrap();
    assert_eq!(body["code"], "NOT_FOUND");
}

#[tokio::test]
async fn cmmn_case_definition_linked_repository_routes_validate_missing_definition() {
    let (base_url, client) = spawn_server(Arc::new(MockCmmnApi::with_seed())).await;

    for path in [
        "decision-tables",
        "decisions",
        "form-definitions",
        "start-form",
    ] {
        let response = client
            .get(format!(
                "{}/cmmn-repository/case-definitions/missing-definition/{path}",
                base_url
            ))
            .basic_auth("admin", Some("test"))
            .send()
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND, "path {path}");
        let body: Value = response.json().await.unwrap();
        assert_eq!(body["code"], "NOT_FOUND", "path {path}");
    }
}

#[tokio::test]
async fn cmmn_case_definition_image_returns_png_and_structured_errors() {
    let (base_url, client) = spawn_real_server("rest-cmmn-case-definition-image").await;

    let cmmn_xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/CMMN/20151109/MODEL"
             targetNamespace="http://flowable.org/cmmn">
  <case id="imageCase" name="Image Case">
    <casePlanModel id="planModelA" name="Plan Model A" autoComplete="false">
      <planItem id="planItemStage" name="Review Stage" definitionRef="reviewStage" />
      <planItem id="planItemRootTask" name="Root Task" definitionRef="rootTask" />
      <stage id="reviewStage" name="Review Stage" autoComplete="true">
        <planItem id="planItemNestedTask" name="Prepare Review" definitionRef="prepareReview" />
        <humanTask id="prepareReview" name="Prepare Review" isBlocking="false" />
      </stage>
      <humanTask id="rootTask" name="Root Task" isBlocking="true" />
    </casePlanModel>
  </case>
</definitions>"#;

    let deploy_response = client
        .post(format!("{}/cmmn-repository/deployments", base_url))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "Image case deployment",
            "resourceName": "image-case.cmmn",
            "resource": cmmn_xml
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(deploy_response.status(), StatusCode::CREATED);

    let definitions = client
        .get(format!(
            "{}/cmmn-repository/case-definitions?key=imageCase",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(definitions.status(), StatusCode::OK);
    let definitions_body: Value = definitions.json().await.unwrap();
    let case_definition_id = definitions_body["data"][0]["id"].as_str().unwrap();

    let image_response = client
        .get(format!(
            "{}/cmmn-repository/case-definitions/{}/image",
            base_url, case_definition_id
        ))
        .basic_auth("admin", Some("test"))
        .header(header::ACCEPT, "image/png")
        .send()
        .await
        .unwrap();

    assert_eq!(image_response.status(), StatusCode::OK);
    assert_eq!(
        image_response.headers().get(header::CONTENT_TYPE).unwrap(),
        "image/png"
    );
    let image_bytes = image_response.bytes().await.unwrap();
    assert!(image_bytes.starts_with(b"\x89PNG\r\n\x1a\n"));

    let missing = client
        .get(format!(
            "{}/cmmn-repository/case-definitions/missing-definition/image",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .header(header::ACCEPT, "image/png")
        .send()
        .await
        .unwrap();
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
    let missing_body: Value = missing.json().await.unwrap();
    assert_eq!(missing_body["code"], "NOT_FOUND");
}

#[tokio::test]
async fn cmmn_case_definition_linked_repository_routes_return_empty_for_deployed_case_without_links()
 {
    let (base_url, client) = spawn_server(Arc::new(MockCmmnApi::default())).await;

    let deploy_response = client
        .post(format!("{}/cmmn-repository/deployments", base_url))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "Empty linked resource case",
            "resourceName": "empty-linked-resource.cmmn",
            "resource": "<definitions />"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(deploy_response.status(), StatusCode::CREATED);

    for path in ["decision-tables", "decisions", "form-definitions"] {
        let response = client
            .get(format!(
                "{}/cmmn-repository/case-definitions/case-definition-1/{path}",
                base_url
            ))
            .basic_auth("admin", Some("test"))
            .send()
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK, "path {path}");
        let body: Value = response.json().await.unwrap();
        assert_eq!(body["total"], 0, "path {path}");
        assert_eq!(body["data"].as_array().unwrap().len(), 0, "path {path}");
    }

    let start_form = client
        .get(format!(
            "{}/cmmn-repository/case-definitions/case-definition-1/start-form",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(start_form.status(), StatusCode::NOT_FOUND);
    let body: Value = start_form.json().await.unwrap();
    assert_eq!(body["code"], "NOT_FOUND");
}

#[tokio::test]
async fn cmmn_case_definition_linked_repository_routes_return_real_deployed_resources() {
    let (base_url, client) = spawn_real_server("rest-cmmn-linked-resources").await;

    let dmn_xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="https://www.omg.org/spec/DMN/20191111/MODEL/" id="linkedDecisionDefinitions" name="Linked Decisions">
  <decision id="eligibilityDecision" name="Eligibility Decision">
    <decisionTable id="eligibilityTable" hitPolicy="FIRST">
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

    let form_payload = json!({
        "key": "caseStartForm",
        "name": "Case Start Form",
        "fields": []
    });
    let task_form_payload = json!({
        "key": "caseTaskForm",
        "name": "Case Task Form",
        "fields": []
    });
    let cmmn_xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/CMMN/20151109/MODEL" xmlns:flowable="http://flowable.org/cmmn">
  <case id="linkedCase" name="Linked Case">
    <casePlanModel id="casePlan" name="Case Plan" flowable:formKey="caseStartForm">
      <planItem id="taskPlanItem" definitionRef="taskWithForm" />
      <planItem id="decisionPlanItem" definitionRef="eligibilityTask" />
      <humanTask id="taskWithForm" name="Task With Form" flowable:formKey="caseTaskForm" />
      <decisionTask id="eligibilityTask" name="Eligibility Task" decisionRef="eligibilityDecision" />
    </casePlanModel>
  </case>
</definitions>"#;

    let dmn_deploy = client
        .post(format!("{}/dmn-repository/deployments", base_url))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "Linked decisions",
            "resourceName": "eligibility.dmn",
            "resource": dmn_xml
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(dmn_deploy.status(), StatusCode::CREATED);

    for (resource_name, resource) in [
        ("case-start.form", form_payload.to_string()),
        ("case-task.form", task_form_payload.to_string()),
    ] {
        let form_deploy = client
            .post(format!("{}/form-repository/deployments", base_url))
            .basic_auth("admin", Some("test"))
            .json(&json!({
                "name": resource_name,
                "resourceName": resource_name,
                "resource": resource
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(form_deploy.status(), StatusCode::CREATED);
    }

    let cmmn_deploy = client
        .post(format!("{}/cmmn-repository/deployments", base_url))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "Linked case",
            "resourceName": "linked-case.cmmn",
            "resource": cmmn_xml
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(cmmn_deploy.status(), StatusCode::CREATED);

    let definitions: Value = client
        .get(format!(
            "{}/cmmn-repository/case-definitions?key=linkedCase",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let case_definition_id = definitions["data"][0]["id"].as_str().unwrap();

    for path in ["decision-tables", "decisions"] {
        let response: Value = client
            .get(format!(
                "{}/cmmn-repository/case-definitions/{case_definition_id}/{path}",
                base_url
            ))
            .basic_auth("admin", Some("test"))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(response["total"], 1, "path {path}");
        assert_eq!(
            response["data"][0]["key"], "eligibilityDecision",
            "path {path}"
        );
    }

    let forms: Value = client
        .get(format!(
            "{}/cmmn-repository/case-definitions/{case_definition_id}/form-definitions",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(forms["total"], 2);
    let form_keys = forms["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["key"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert!(form_keys.contains(&"caseStartForm"));
    assert!(form_keys.contains(&"caseTaskForm"));

    let start_form: Value = client
        .get(format!(
            "{}/cmmn-repository/case-definitions/{case_definition_id}/start-form",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(start_form["key"], "caseStartForm");
}
