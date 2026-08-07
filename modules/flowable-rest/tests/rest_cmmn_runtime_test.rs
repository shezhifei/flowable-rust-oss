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
        CmmnDeploymentCommand, CmmnDeploymentRecord, CmmnManagementJobFamily,
        CmmnManagementJobQuery, CmmnManagementJobRecord, HistoricCaseInstanceQuery,
        HistoricCaseInstanceRecord, HistoricPlanItemInstanceQuery, HistoricPlanItemInstanceRecord,
        PlanItemInstanceQuery, PlanItemInstanceRecord, StartCaseInstanceCommand,
    },
    run_server,
};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use tokio::net::TcpListener;

#[derive(Default)]
struct MockCmmnApi {
    case_instances: Mutex<Vec<CaseInstanceRecord>>,
    plan_item_instances: Mutex<Vec<PlanItemInstanceRecord>>,
    historic_case_instances: Mutex<Vec<HistoricCaseInstanceRecord>>,
    historic_plan_item_instances: Mutex<Vec<HistoricPlanItemInstanceRecord>>,
}

impl cmmn::CmmnRepositoryApi for MockCmmnApi {
    fn deploy_case_definitions(
        &self,
        _command: CmmnDeploymentCommand,
    ) -> Result<CmmnDeploymentRecord, ApiError> {
        Err(ApiError::InternalServerError(
            "repository stub not used in runtime tests".to_string(),
        ))
    }

    fn list_case_definitions(
        &self,
        _query: CaseDefinitionQuery,
    ) -> Result<PagedResponse<CaseDefinitionRecord>, ApiError> {
        Ok(PagedResponse {
            start: 0,
            size: 0,
            total: 0,
            data: Vec::new(),
            sort: None,
            order: None,
        })
    }

    fn get_case_definition(
        &self,
        case_definition_id: &str,
    ) -> Result<CaseDefinitionRecord, ApiError> {
        if case_definition_id != "case-definition-1" {
            return Err(ApiError::NotFound(
                "Case definition was not found".to_string(),
            ));
        }
        Ok(CaseDefinitionRecord {
            id: "case-definition-1".to_string(),
            key: "loanApprovalCase".to_string(),
            name: "Loan Approval Case".to_string(),
            version: 1,
            deployment_id: "deployment-1".to_string(),
            resource_name: "loan-approval-case.cmmn".to_string(),
            category: None,
            description: None,
            tenant_id: None,
            parent_deployment_id: None,
        })
    }
}

impl cmmn::CmmnRuntimeApi for MockCmmnApi {
    fn start_case_instance(
        &self,
        command: StartCaseInstanceCommand,
    ) -> Result<CaseInstanceRecord, ApiError> {
        let case_definition_key = if let Some(case_definition_key) = command.case_definition_key {
            case_definition_key
        } else if command.case_definition_id.as_deref() == Some("case-definition-1") {
            "loanApprovalCase".to_string()
        } else {
            return Err(ApiError::NotFound(
                "Case definition was not found".to_string(),
            ));
        };

        if case_definition_key != "loanApprovalCase" {
            return Err(ApiError::NotFound(format!(
                "Case definition key '{}' was not found",
                case_definition_key
            )));
        }

        let sequence = self.case_instances.lock().unwrap().len() + 1;
        let now = "2026-04-21T09:30:00Z".to_string();
        let case_instance = CaseInstanceRecord {
            id: format!("case-instance-{sequence}"),
            case_definition_id: "case-definition-1".to_string(),
            case_definition_key: "loanApprovalCase".to_string(),
            business_key: command.business_key,
            name: command.name,
            state: "ACTIVE".to_string(),
            business_status: None,
            started_by: None,
            callback_id: None,
            callback_type: None,
            reference_id: None,
            reference_type: None,
            case_definition_name: None,
            variables: Vec::new(),
            tenant_id: command.tenant_id,
            started_at: now.clone(),
        };

        self.case_instances
            .lock()
            .unwrap()
            .push(case_instance.clone());
        self.plan_item_instances
            .lock()
            .unwrap()
            .push(PlanItemInstanceRecord {
                id: format!("plan-item-{sequence}"),
                case_instance_id: case_instance.id.clone(),
                case_definition_id: case_instance.case_definition_id.clone(),
                plan_item_definition_id: "humanTask1".to_string(),
                plan_item_definition_type: "humantask".to_string(),
                element_id: "plan-item-1".to_string(),
                stage_instance_id: None,
                stage: false,
                name: "Review application".to_string(),
                state: "AVAILABLE".to_string(),
                occurred_time: None,
                assignee: None,
                owner: None,
                priority: None,
                due_date: None,
                category: None,
                delegation_state: None,
                variables: Vec::new(),
                tenant_id: case_instance.tenant_id.clone(),
                created_at: now.clone(),
                ended_at: None,
            });

        Ok(case_instance)
    }

    fn list_case_instances(
        &self,
        query: CaseInstanceQuery,
    ) -> Result<PagedResponse<CaseInstanceRecord>, ApiError> {
        let filtered = self
            .case_instances
            .lock()
            .unwrap()
            .iter()
            .filter(|instance| {
                query.id.as_ref().is_none_or(|value| instance.id == *value)
                    && query
                        .case_definition_id
                        .as_ref()
                        .is_none_or(|value| instance.case_definition_id == *value)
                    && query
                        .case_definition_key
                        .as_ref()
                        .is_none_or(|value| instance.case_definition_key == *value)
                    && query
                        .business_key
                        .as_ref()
                        .is_none_or(|value| instance.business_key.as_ref() == Some(value))
                    && query
                        .state
                        .as_ref()
                        .is_none_or(|value| instance.state == *value)
            })
            .cloned()
            .collect();

        Ok(query.paging.paginate(filtered))
    }

    fn list_plan_item_instances(
        &self,
        query: PlanItemInstanceQuery,
    ) -> Result<PagedResponse<PlanItemInstanceRecord>, ApiError> {
        let filtered = self
            .plan_item_instances
            .lock()
            .unwrap()
            .iter()
            .filter(|plan_item| {
                query.id.as_ref().is_none_or(|value| plan_item.id == *value)
                    && query
                        .case_instance_id
                        .as_ref()
                        .is_none_or(|value| plan_item.case_instance_id == *value)
                    && query
                        .plan_item_definition_id
                        .as_ref()
                        .is_none_or(|value| plan_item.plan_item_definition_id == *value)
                    && query
                        .state
                        .as_ref()
                        .is_none_or(|value| plan_item.state == *value)
            })
            .cloned()
            .collect();

        Ok(query.paging.paginate(filtered))
    }

    fn complete_plan_item_instance(&self, plan_item_instance_id: &str) -> Result<(), ApiError> {
        let ended_at = "2026-04-21T09:45:00Z".to_string();
        let mut plan_items = self.plan_item_instances.lock().unwrap();
        let plan_item = plan_items
            .iter_mut()
            .find(|plan_item| plan_item.id == plan_item_instance_id)
            .ok_or_else(|| {
                ApiError::NotFound(format!(
                    "Plan item instance '{plan_item_instance_id}' was not found"
                ))
            })?;

        plan_item.state = "COMPLETED".to_string();
        plan_item.ended_at = Some(ended_at.clone());

        self.historic_plan_item_instances
            .lock()
            .unwrap()
            .push(HistoricPlanItemInstanceRecord {
                id: plan_item.id.clone(),
                case_instance_id: plan_item.case_instance_id.clone(),
                case_definition_id: plan_item.case_definition_id.clone(),
                plan_item_definition_id: plan_item.plan_item_definition_id.clone(),
                plan_item_definition_type: plan_item.plan_item_definition_type.clone(),
                element_id: plan_item.element_id.clone(),
                stage_instance_id: plan_item.stage_instance_id.clone(),
                name: plan_item.name.clone(),
                state: plan_item.state.clone(),
                assignee: plan_item.assignee.clone(),
                tenant_id: plan_item.tenant_id.clone(),
                created_at: plan_item.created_at.clone(),
                ended_at: plan_item.ended_at.clone(),
            });

        let mut case_instances = self.case_instances.lock().unwrap();
        let case_instance = case_instances
            .iter_mut()
            .find(|instance| instance.id == plan_item.case_instance_id)
            .expect("case instance should exist for plan item");
        case_instance.state = "COMPLETED".to_string();

        self.historic_case_instances
            .lock()
            .unwrap()
            .push(HistoricCaseInstanceRecord {
                id: case_instance.id.clone(),
                case_definition_id: case_instance.case_definition_id.clone(),
                case_definition_key: case_instance.case_definition_key.clone(),
                business_key: case_instance.business_key.clone(),
                name: case_instance.name.clone(),
                state: case_instance.state.clone(),
                tenant_id: case_instance.tenant_id.clone(),
                started_at: case_instance.started_at.clone(),
                ended_at: Some(ended_at),
                reference_id: case_instance.reference_id.clone(),
                reference_type: case_instance.reference_type.clone(),
                end_user_id: None,
                variables: Vec::new(),
            });

        Ok(())
    }
}

impl cmmn::CmmnHistoryApi for MockCmmnApi {
    fn list_historic_case_instances(
        &self,
        query: HistoricCaseInstanceQuery,
    ) -> Result<PagedResponse<HistoricCaseInstanceRecord>, ApiError> {
        let filtered = self
            .historic_case_instances
            .lock()
            .unwrap()
            .iter()
            .filter(|instance| {
                query.id.as_ref().is_none_or(|value| instance.id == *value)
                    && query
                        .case_definition_id
                        .as_ref()
                        .is_none_or(|value| instance.case_definition_id == *value)
                    && query
                        .case_definition_key
                        .as_ref()
                        .is_none_or(|value| instance.case_definition_key == *value)
                    && query
                        .business_key
                        .as_ref()
                        .is_none_or(|value| instance.business_key.as_ref() == Some(value))
                    && query
                        .state
                        .as_ref()
                        .is_none_or(|value| instance.state == *value)
            })
            .cloned()
            .collect();

        Ok(query.paging.paginate(filtered))
    }

    fn list_historic_plan_item_instances(
        &self,
        query: HistoricPlanItemInstanceQuery,
    ) -> Result<PagedResponse<HistoricPlanItemInstanceRecord>, ApiError> {
        let filtered = self
            .historic_plan_item_instances
            .lock()
            .unwrap()
            .iter()
            .filter(|plan_item| {
                query.id.as_ref().is_none_or(|value| plan_item.id == *value)
                    && query
                        .case_instance_id
                        .as_ref()
                        .is_none_or(|value| plan_item.case_instance_id == *value)
                    && query
                        .plan_item_definition_id
                        .as_ref()
                        .is_none_or(|value| plan_item.plan_item_definition_id == *value)
                    && query
                        .state
                        .as_ref()
                        .is_none_or(|value| plan_item.state == *value)
            })
            .cloned()
            .collect();

        Ok(query.paging.paginate(filtered))
    }
}

impl cmmn::CmmnManagementApi for MockCmmnApi {
    fn list_jobs(
        &self,
        query: CmmnManagementJobQuery,
    ) -> Result<PagedResponse<CmmnManagementJobRecord>, ApiError> {
        Ok(query.paging.paginate(Vec::new()))
    }

    fn get_job(
        &self,
        family: CmmnManagementJobFamily,
        job_id: &str,
    ) -> Result<CmmnManagementJobRecord, ApiError> {
        let _ = family;
        Err(ApiError::NotFound(format!(
            "CMMN job '{job_id}' was not found"
        )))
    }

    fn get_job_exception_stacktrace(
        &self,
        family: CmmnManagementJobFamily,
        job_id: &str,
    ) -> Result<String, ApiError> {
        let _ = family;
        Err(ApiError::NotFound(format!(
            "CMMN job '{job_id}' was not found"
        )))
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
    let history: cmmn::DynCmmnHistory = api.clone();
    let management: cmmn::DynCmmnManagement = api;
    let app = Router::new()
        .merge(cmmn::router_with_management(
            repository, runtime, history, management,
        ))
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
    tokio::spawn(async move {
        run_server(engine, listener).await.unwrap();
    });

    (base_url, reqwest::Client::new())
}

#[tokio::test]
async fn cmmn_case_instance_diagram_returns_png_and_structured_not_found() {
    let (base_url, client) = spawn_real_server("rest-cmmn-case-instance-diagram").await;

    let cmmn_xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/CMMN/20151109/MODEL"
             targetNamespace="http://flowable.org/cmmn">
  <case id="diagramCase" name="Diagram Case">
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
            "name": "Diagram case deployment",
            "resourceName": "diagram-case.cmmn",
            "resource": cmmn_xml
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(deploy_response.status(), StatusCode::CREATED);

    let start_response = client
        .post(format!("{}/cmmn-runtime/case-instances", base_url))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "caseDefinitionKey": "diagramCase",
            "businessKey": "diagram-1"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(start_response.status(), StatusCode::CREATED);
    let started_case: Value = start_response.json().await.unwrap();
    let case_instance_id = started_case["id"].as_str().unwrap();

    let diagram = client
        .get(format!(
            "{}/cmmn-runtime/case-instances/{}/diagram",
            base_url, case_instance_id
        ))
        .basic_auth("admin", Some("test"))
        .header(header::ACCEPT, "image/png")
        .send()
        .await
        .unwrap();
    assert_eq!(diagram.status(), StatusCode::OK);
    assert_eq!(
        diagram.headers().get(header::CONTENT_TYPE).unwrap(),
        "image/png"
    );
    let diagram_bytes = diagram.bytes().await.unwrap();
    assert!(diagram_bytes.starts_with(b"\x89PNG\r\n\x1a\n"));

    let missing_diagram = client
        .get(format!(
            "{}/cmmn-runtime/case-instances/missing-case/diagram",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .header(header::ACCEPT, "image/png")
        .send()
        .await
        .unwrap();
    assert_eq!(missing_diagram.status(), StatusCode::NOT_FOUND);
    let missing_diagram_body: Value = missing_diagram.json().await.unwrap();
    assert_eq!(missing_diagram_body["code"], "NOT_FOUND");
}

#[tokio::test]
async fn cmmn_runtime_and_history_routes_follow_owned_contract() {
    let (base_url, client) = spawn_server(Arc::new(MockCmmnApi::default())).await;

    let start_response = client
        .post(format!("{}/cmmn-runtime/case-instances", base_url))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "caseDefinitionKey": "loanApprovalCase",
            "businessKey": "customer-42",
            "name": "Loan approval for customer 42",
            "variables": {
                "amount": 25000,
                "region": "CN"
            }
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(start_response.status(), StatusCode::CREATED);
    let started_case: Value = start_response.json().await.unwrap();
    assert_eq!(started_case["caseDefinitionId"], "case-definition-1");
    assert_eq!(started_case["caseDefinitionKey"], "loanApprovalCase");
    assert_eq!(started_case["businessKey"], "customer-42");
    assert_eq!(started_case["state"], "ACTIVE");

    let case_instance_id = started_case["id"].as_str().unwrap();

    let runtime_cases = client
        .get(format!(
            "{}/cmmn-runtime/case-instances?caseDefinitionKey=loanApprovalCase&start=0&size=10",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(runtime_cases.status(), StatusCode::OK);
    let runtime_cases_body: Value = runtime_cases.json().await.unwrap();
    assert_eq!(runtime_cases_body["start"], 0);
    assert_eq!(runtime_cases_body["size"], 1);
    assert_eq!(runtime_cases_body["total"], 1);
    assert_eq!(runtime_cases_body["data"][0]["id"], case_instance_id);

    let queried_cases = client
        .post(format!("{}/cmmn-query/case-instances", base_url))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "caseDefinitionKey": "loanApprovalCase",
            "start": 0,
            "size": 10
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(queried_cases.status(), StatusCode::OK);
    let queried_cases_body: Value = queried_cases.json().await.unwrap();
    assert_eq!(queried_cases_body["total"], 1);
    assert_eq!(queried_cases_body["data"][0]["id"], case_instance_id);

    let runtime_case = client
        .get(format!(
            "{}/cmmn-runtime/case-instances/{}",
            base_url, case_instance_id
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(runtime_case.status(), StatusCode::OK);
    let runtime_case_body: Value = runtime_case.json().await.unwrap();
    assert_eq!(runtime_case_body["id"], case_instance_id);
    assert_eq!(runtime_case_body["caseDefinitionKey"], "loanApprovalCase");

    let runtime_plan_items = client
        .get(format!(
            "{}/cmmn-runtime/plan-item-instances?caseInstanceId={}&start=0&size=10",
            base_url, case_instance_id
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(runtime_plan_items.status(), StatusCode::OK);
    let runtime_plan_items_body: Value = runtime_plan_items.json().await.unwrap();
    assert_eq!(runtime_plan_items_body["start"], 0);
    assert_eq!(runtime_plan_items_body["size"], 1);
    assert_eq!(runtime_plan_items_body["total"], 1);
    assert_eq!(
        runtime_plan_items_body["data"][0]["planItemDefinitionId"],
        "humanTask1"
    );
    let plan_item_instance_id = runtime_plan_items_body["data"][0]["id"].as_str().unwrap();

    let runtime_tasks = client
        .get(format!(
            "{}/cmmn-runtime/tasks?caseInstanceId={}&start=0&size=10",
            base_url, case_instance_id
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(runtime_tasks.status(), StatusCode::OK);
    let runtime_tasks_body: Value = runtime_tasks.json().await.unwrap();
    assert_eq!(runtime_tasks_body["total"], 1);
    assert_eq!(runtime_tasks_body["data"][0]["id"], plan_item_instance_id);
    assert_eq!(
        runtime_tasks_body["data"][0]["planItemDefinitionId"],
        "humanTask1"
    );

    let queried_tasks = client
        .post(format!("{}/cmmn-query/tasks", base_url))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "caseInstanceId": case_instance_id,
            "start": 0,
            "size": 10
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(queried_tasks.status(), StatusCode::OK);
    let queried_tasks_body: Value = queried_tasks.json().await.unwrap();
    assert_eq!(queried_tasks_body["total"], 1);
    assert_eq!(queried_tasks_body["data"][0]["id"], plan_item_instance_id);

    let runtime_task = client
        .get(format!(
            "{}/cmmn-runtime/tasks/{}",
            base_url, plan_item_instance_id
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(runtime_task.status(), StatusCode::OK);
    let runtime_task_body: Value = runtime_task.json().await.unwrap();
    assert_eq!(runtime_task_body["id"], plan_item_instance_id);
    assert_eq!(runtime_task_body["caseInstanceId"], case_instance_id);

    let queried_plan_items = client
        .post(format!("{}/cmmn-query/plan-item-instances", base_url))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "caseInstanceId": case_instance_id,
            "start": 0,
            "size": 10
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(queried_plan_items.status(), StatusCode::OK);
    let queried_plan_items_body: Value = queried_plan_items.json().await.unwrap();
    assert_eq!(queried_plan_items_body["total"], 1);
    assert_eq!(
        queried_plan_items_body["data"][0]["id"],
        plan_item_instance_id
    );

    let runtime_plan_item = client
        .get(format!(
            "{}/cmmn-runtime/plan-item-instances/{}",
            base_url, plan_item_instance_id
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(runtime_plan_item.status(), StatusCode::OK);
    let runtime_plan_item_body: Value = runtime_plan_item.json().await.unwrap();
    assert_eq!(runtime_plan_item_body["id"], plan_item_instance_id);
    assert_eq!(runtime_plan_item_body["planItemDefinitionId"], "humanTask1");

    let complete_response = client
        .post(format!(
            "{}/cmmn-runtime/plan-item-instances/{}/complete",
            base_url, plan_item_instance_id
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(complete_response.status(), StatusCode::OK);
    let complete_body: Value = complete_response.json().await.unwrap();
    assert_eq!(complete_body, json!({"status": "completed"}));

    let history_cases = client
        .get(format!(
            "{}/cmmn-history/historic-case-instances?caseDefinitionKey=loanApprovalCase&state=COMPLETED&start=0&size=10",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(history_cases.status(), StatusCode::OK);
    let history_cases_body: Value = history_cases.json().await.unwrap();
    assert_eq!(history_cases_body["start"], 0);
    assert_eq!(history_cases_body["size"], 1);
    assert_eq!(history_cases_body["total"], 1);
    let historic_case = &history_cases_body["data"][0];
    assert_eq!(historic_case["id"], case_instance_id);
    assert_eq!(historic_case["state"], "COMPLETED");
    assert_eq!(historic_case["endedAt"], "2026-04-21T09:45:00Z");

    let queried_history_cases = client
        .post(format!("{}/cmmn-query/historic-case-instances", base_url))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "caseDefinitionKey": "loanApprovalCase",
            "state": "COMPLETED",
            "start": 0,
            "size": 10
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(queried_history_cases.status(), StatusCode::OK);
    let queried_history_cases_body: Value = queried_history_cases.json().await.unwrap();
    assert_eq!(queried_history_cases_body["total"], 1);
    assert_eq!(
        queried_history_cases_body["data"][0]["id"],
        case_instance_id
    );

    let history_case = client
        .get(format!(
            "{}/cmmn-history/historic-case-instances/{}",
            base_url, case_instance_id
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(history_case.status(), StatusCode::OK);
    let history_case_body: Value = history_case.json().await.unwrap();
    assert_eq!(history_case_body["id"], case_instance_id);
    assert_eq!(history_case_body["state"], "COMPLETED");

    let history_plan_items = client
        .get(format!(
            "{}/cmmn-history/historic-plan-item-instances?caseInstanceId={}&state=COMPLETED&start=0&size=10",
            base_url, case_instance_id
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(history_plan_items.status(), StatusCode::OK);
    let history_plan_items_body: Value = history_plan_items.json().await.unwrap();
    assert_eq!(history_plan_items_body["start"], 0);
    assert_eq!(history_plan_items_body["size"], 1);
    assert_eq!(history_plan_items_body["total"], 1);
    let historic_plan_item = &history_plan_items_body["data"][0];
    assert_eq!(historic_plan_item["id"], plan_item_instance_id);
    assert_eq!(historic_plan_item["state"], "COMPLETED");
    assert_eq!(historic_plan_item["endedAt"], "2026-04-21T09:45:00Z");

    let history_tasks = client
        .get(format!(
            "{}/cmmn-history/historic-task-instances?caseInstanceId={}&state=COMPLETED&start=0&size=10",
            base_url, case_instance_id
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(history_tasks.status(), StatusCode::OK);
    let history_tasks_body: Value = history_tasks.json().await.unwrap();
    assert_eq!(history_tasks_body["total"], 1);
    assert_eq!(history_tasks_body["data"][0]["id"], plan_item_instance_id);
    assert_eq!(history_tasks_body["data"][0]["state"], "COMPLETED");

    let queried_history_tasks = client
        .post(format!("{}/cmmn-query/historic-task-instances", base_url))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "caseInstanceId": case_instance_id,
            "state": "COMPLETED",
            "start": 0,
            "size": 10
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(queried_history_tasks.status(), StatusCode::OK);
    let queried_history_tasks_body: Value = queried_history_tasks.json().await.unwrap();
    assert_eq!(queried_history_tasks_body["total"], 1);
    assert_eq!(
        queried_history_tasks_body["data"][0]["id"],
        plan_item_instance_id
    );

    let history_task = client
        .get(format!(
            "{}/cmmn-history/historic-task-instances/{}",
            base_url, plan_item_instance_id
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(history_task.status(), StatusCode::OK);
    let history_task_body: Value = history_task.json().await.unwrap();
    assert_eq!(history_task_body["id"], plan_item_instance_id);
    assert_eq!(history_task_body["state"], "COMPLETED");

    let queried_history_plan_items = client
        .post(format!(
            "{}/cmmn-query/historic-planitem-instances",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "caseInstanceId": case_instance_id,
            "state": "COMPLETED",
            "start": 0,
            "size": 10
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(queried_history_plan_items.status(), StatusCode::OK);
    let queried_history_plan_items_body: Value = queried_history_plan_items.json().await.unwrap();
    assert_eq!(queried_history_plan_items_body["total"], 1);
    assert_eq!(
        queried_history_plan_items_body["data"][0]["id"],
        plan_item_instance_id
    );

    let history_plan_item = client
        .get(format!(
            "{}/cmmn-history/historic-planitem-instances/{}",
            base_url, plan_item_instance_id
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(history_plan_item.status(), StatusCode::OK);
    let history_plan_item_body: Value = history_plan_item.json().await.unwrap();
    assert_eq!(history_plan_item_body["id"], plan_item_instance_id);
    assert_eq!(history_plan_item_body["state"], "COMPLETED");

    let spelling_history_plan_items = client
        .get(format!(
            "{}/cmmn-history/historic-planitem-instances?caseInstanceId={}&state=COMPLETED&start=0&size=10",
            base_url, case_instance_id
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(spelling_history_plan_items.status(), StatusCode::OK);
    let spelling_body: Value = spelling_history_plan_items.json().await.unwrap();
    assert_eq!(spelling_body["total"], 1);
    assert_eq!(spelling_body["data"][0]["id"], plan_item_instance_id);
}

#[tokio::test]
async fn cmmn_runtime_and_history_routes_enforce_auth_and_structured_errors() {
    let (base_url, client) = spawn_server(Arc::new(MockCmmnApi::default())).await;

    let unauthorized = client
        .post(format!("{}/cmmn-runtime/case-instances", base_url))
        .json(&json!({
            "caseDefinitionKey": "loanApprovalCase"
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);
    let unauthorized_body: Value = unauthorized.json().await.unwrap();
    assert_eq!(unauthorized_body["code"], "UNAUTHORIZED");

    // P120: `tenantId` used to land here because the historic-case param surface
    // rejected it, but Java accepts it
    // (HistoricCaseInstanceCollectionResource.java:306-308) and Rust now honours
    // it. P128 also implements `callbackId`; `rootScopeId` keeps this case
    // pointed at a param that remains an intentional cut, so the structured
    // `deny_unknown_fields` error shape stays covered.
    let bad_query = client
        .get(format!(
            "{}/cmmn-history/historic-case-instances?rootScopeId=root-a",
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
            .contains("rootScopeId")
    );

    let missing_case_definition = client
        .post(format!("{}/cmmn-runtime/case-instances", base_url))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "caseDefinitionKey": "missingCase"
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(missing_case_definition.status(), StatusCode::NOT_FOUND);
    let missing_case_definition_body: Value = missing_case_definition.json().await.unwrap();
    assert_eq!(missing_case_definition_body["code"], "NOT_FOUND");

    let missing_plan_item = client
        .post(format!(
            "{}/cmmn-runtime/plan-item-instances/missing-plan-item/complete",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(missing_plan_item.status(), StatusCode::NOT_FOUND);
    let missing_plan_item_body: Value = missing_plan_item.json().await.unwrap();
    assert_eq!(missing_plan_item_body["code"], "NOT_FOUND");

    let missing_case_instance = client
        .get(format!(
            "{}/cmmn-runtime/case-instances/missing-case-instance",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(missing_case_instance.status(), StatusCode::NOT_FOUND);
    let missing_case_instance_body: Value = missing_case_instance.json().await.unwrap();
    assert_eq!(missing_case_instance_body["code"], "NOT_FOUND");

    let invalid_body = client
        .post(format!("{}/cmmn-runtime/case-instances", base_url))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "caseDefinitionKey": "loanApprovalCase",
            "unexpectedField": true
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(invalid_body.status(), StatusCode::BAD_REQUEST);
    let invalid_body_json: Value = invalid_body.json().await.unwrap();
    assert_eq!(invalid_body_json["code"], "BAD_REQUEST");
}
