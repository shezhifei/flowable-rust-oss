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
    routes::dmn::{
        self, DecisionExecutionCommand, DecisionExecutionRecord, DecisionTableQuery,
        DecisionTableRecord, DmnDeploymentCommand, DmnDeploymentRecord,
        HistoricDecisionExecutionQuery, HistoricDecisionExecutionRecord, engine_rest_variable_row,
    },
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};
use tokio::net::TcpListener;

#[derive(Default)]
struct MockDmnApi {
    history: Mutex<Vec<HistoricDecisionExecutionRecord>>,
}

impl dmn::DmnRepositoryApi for MockDmnApi {
    fn deploy_decision_tables(
        &self,
        _command: DmnDeploymentCommand,
    ) -> Result<DmnDeploymentRecord, ApiError> {
        Err(ApiError::InternalServerError(
            "repository stub not used in runtime tests".to_string(),
        ))
    }

    fn list_decision_tables(
        &self,
        _query: DecisionTableQuery,
    ) -> Result<PagedResponse<DecisionTableRecord>, ApiError> {
        Ok(PagedResponse {
            start: 0,
            size: 0,
            total: 0,
            data: Vec::new(),
            sort: None,
            order: None,
        })
    }

    fn get_decision_table(
        &self,
        _decision_table_id: &str,
    ) -> Result<DecisionTableRecord, ApiError> {
        Err(ApiError::NotFound(
            "Decision table was not found".to_string(),
        ))
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

impl dmn::DmnRuntimeApi for MockDmnApi {
    fn execute_decision(
        &self,
        command: DecisionExecutionCommand,
    ) -> Result<DecisionExecutionRecord, ApiError> {
        // P85: one row covering every JSON value kind, for type inference
        if command.decision_key == "allValueKinds" {
            let mut row = BTreeMap::new();
            row.insert("aString".to_string(), json!("text"));
            row.insert("bInteger".to_string(), json!(42));
            row.insert("cNegative".to_string(), json!(-7));
            row.insert("dDouble".to_string(), json!(1.5));
            row.insert("eBoolean".to_string(), json!(false));
            row.insert("fNull".to_string(), Value::Null);
            row.insert("gObject".to_string(), json!({"nested": 1}));
            row.insert("hArray".to_string(), json!([1, "two"]));
            return Ok(DecisionExecutionRecord {
                id: format!("execution-{}", self.history.lock().unwrap().len() + 1),
                decision_table_id: "types-1".to_string(),
                deployment_id: "deployment-1".to_string(),
                decision_key: command.decision_key.clone(),
                tenant_id: command.tenant_id.clone(),
                business_key: command.business_key.clone(),
                hit_policy: "FIRST".to_string(),
                executed_at: 1_713_674_600_000,
                rule_hit_count: 1,
                input_variables: command.variables.clone(),
                result_variables: vec![engine_rest_variable_row(&row)],
                multiple_results: false,
                rule_executions: Vec::new(),
            });
        }

        // P82d: multi-row COLLECT-style decision for single-result 500 tests
        if command.decision_key == "collectRouting" {
            let mut row1 = BTreeMap::new();
            row1.insert("route".to_string(), Value::String("manual".to_string()));
            row1.insert("priority".to_string(), json!(10));
            let mut row2 = BTreeMap::new();
            row2.insert(
                "route".to_string(),
                Value::String("email-queue".to_string()),
            );
            row2.insert("priority".to_string(), json!(20));
            return Ok(DecisionExecutionRecord {
                id: format!("execution-{}", self.history.lock().unwrap().len() + 1),
                decision_table_id: "collect-1".to_string(),
                deployment_id: "deployment-1".to_string(),
                decision_key: command.decision_key.clone(),
                tenant_id: command.tenant_id.clone(),
                business_key: command.business_key.clone(),
                hit_policy: "COLLECT".to_string(),
                executed_at: 1_713_674_600_000,
                rule_hit_count: 2,
                input_variables: command.variables.clone(),
                result_variables: vec![
                    engine_rest_variable_row(&row1),
                    engine_rest_variable_row(&row2),
                ],
                multiple_results: true,
                rule_executions: Vec::new(),
            });
        }

        // P82d: zero-hit decision for single-result empty 201
        if command.decision_key == "emptyHits" {
            return Ok(DecisionExecutionRecord {
                id: format!("execution-{}", self.history.lock().unwrap().len() + 1),
                decision_table_id: "empty-1".to_string(),
                deployment_id: "deployment-1".to_string(),
                decision_key: command.decision_key.clone(),
                tenant_id: command.tenant_id.clone(),
                business_key: command.business_key.clone(),
                hit_policy: "FIRST".to_string(),
                executed_at: 1_713_674_600_000,
                rule_hit_count: 0,
                input_variables: command.variables.clone(),
                result_variables: Vec::new(),
                multiple_results: false,
                rule_executions: Vec::new(),
            });
        }

        if command.decision_key != "loanEligibility" {
            return Err(ApiError::NotFound(format!(
                "Decision key '{}' was not found",
                command.decision_key
            )));
        }

        let approved = command
            .variables
            .get("creditScore")
            .and_then(Value::as_i64)
            .is_some_and(|score| score >= 700);

        let mut row = BTreeMap::new();
        row.insert("approved".to_string(), Value::Bool(approved));
        row.insert(
            "riskBand".to_string(),
            Value::String(if approved { "LOW" } else { "HIGH" }.to_string()),
        );
        let result_variables = vec![row];

        let execution = DecisionExecutionRecord {
            id: format!("execution-{}", self.history.lock().unwrap().len() + 1),
            decision_table_id: "decision-1".to_string(),
            deployment_id: "deployment-1".to_string(),
            decision_key: command.decision_key.clone(),
            tenant_id: command.tenant_id.clone(),
            business_key: command.business_key.clone(),
            hit_policy: "FIRST".to_string(),
            executed_at: 1_713_674_600_000,
            rule_hit_count: 1,
            input_variables: command.variables.clone(),
            // P85: execution response wraps each output as EngineRestVariable;
            // the historic record below keeps the raw map shape.
            result_variables: result_variables
                .iter()
                .map(engine_rest_variable_row)
                .collect(),
            multiple_results: false,
            rule_executions: Vec::new(),
        };

        self.history
            .lock()
            .unwrap()
            .push(HistoricDecisionExecutionRecord {
                id: execution.id.clone(),
                decision_table_id: execution.decision_table_id.clone(),
                deployment_id: "deployment-1".to_string(),
                decision_key: execution.decision_key.clone(),
                tenant_id: execution.tenant_id.clone(),
                business_key: command.business_key.clone(),
                executed_at: execution.executed_at,
                rule_hit_count: execution.rule_hit_count,
                input_variables: command.variables,
                result_variables,
                multiple_results: false,
                rule_executions: Vec::new(),
                // P83 correlation columns: this mock executes decisions via the
                // REST API with no BPMN/CMMN context, so they stay unset.
                instance_id: None,
                execution_id: None,
                activity_id: None,
                scope_type: None,
            });

        Ok(execution)
    }
}

impl dmn::DmnHistoryApi for MockDmnApi {
    fn list_historic_decision_executions(
        &self,
        query: HistoricDecisionExecutionQuery,
    ) -> Result<PagedResponse<HistoricDecisionExecutionRecord>, ApiError> {
        let filtered: Vec<HistoricDecisionExecutionRecord> = self
            .history
            .lock()
            .unwrap()
            .iter()
            .filter(|entry| {
                query.id.as_ref().is_none_or(|value| entry.id == *value)
                    && query
                        .decision_key
                        .as_ref()
                        .is_none_or(|value| entry.decision_key == *value)
                    && query
                        .decision_table_id
                        .as_ref()
                        .is_none_or(|value| entry.decision_table_id == *value)
            })
            .cloned()
            .collect();

        Ok(query.paging.paginate(filtered))
    }

    fn delete_historic_decision_execution(
        &self,
        historic_decision_execution_id: &str,
    ) -> Result<(), ApiError> {
        let mut history = self.history.lock().unwrap();
        if let Some(pos) = history
            .iter()
            .position(|entry| entry.id == historic_decision_execution_id)
        {
            history.remove(pos);
            Ok(())
        } else {
            Err(ApiError::NotFound(format!(
                "Historic decision execution '{historic_decision_execution_id}' was not found"
            )))
        }
    }

    fn bulk_delete_historic_decision_executions(
        &self,
        historic_decision_execution_ids: Vec<String>,
    ) -> Result<(), ApiError> {
        let mut history = self.history.lock().unwrap();
        history.retain(|entry| !historic_decision_execution_ids.contains(&entry.id));
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

#[tokio::test]
async fn dmn_runtime_and_history_routes_follow_owned_contract() {
    let (base_url, client) = spawn_server(Arc::new(MockDmnApi::default())).await;

    let execute = client
        .post(format!("{}/dmn-runtime/decision-executions", base_url))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "decisionKey": "loanEligibility",
            "variables": {
                "creditScore": 730,
                "country": "CN"
            }
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(execute.status(), StatusCode::CREATED);
    let execute_body: Value = execute.json().await.unwrap();
    assert_eq!(execute_body["decisionKey"], "loanEligibility");
    assert_eq!(execute_body["decisionTableId"], "decision-1");
    assert_eq!(execute_body["ruleHitCount"], 1);
    // P85: rows are EngineRestVariable-wrapped, name-ordered (approved, riskBand)
    assert_eq!(
        execute_body["resultVariables"][0][0],
        json!({"name": "approved", "type": "boolean", "value": true})
    );
    assert_eq!(
        execute_body["resultVariables"][0][1],
        json!({"name": "riskBand", "type": "string", "value": "LOW"})
    );

    let history = client
        .get(format!(
            "{}/dmn-history/historic-decision-executions?decisionKey=loanEligibility&start=0&size=10",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(history.status(), StatusCode::OK);
    let history_body: Value = history.json().await.unwrap();
    assert_eq!(history_body["start"], 0);
    assert_eq!(history_body["size"], 1);
    assert_eq!(history_body["total"], 1);
    let entry = &history_body["data"][0];
    assert_eq!(entry["decisionKey"], "loanEligibility");
    assert_eq!(entry["decisionTableId"], "decision-1");
    assert_eq!(entry["inputVariables"]["country"], "CN");
    assert_eq!(entry["resultVariables"][0]["approved"], true);

    let decision_execute = client
        .post(format!("{}/dmn-rule/execute", base_url))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "decisionKey": "loanEligibility",
            "inputVariables": [
                {
                    "name": "creditScore",
                    "value": 680
                },
                {
                    "name": "country",
                    "value": "CN"
                }
            ]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(decision_execute.status(), StatusCode::CREATED);
    let decision_execute_body: Value = decision_execute.json().await.unwrap();
    assert_eq!(decision_execute_body["decisionKey"], "loanEligibility");
    assert_eq!(
        decision_execute_body["resultVariables"][0][0],
        json!({"name": "approved", "type": "boolean", "value": false})
    );

    let decision_single_result = client
        .post(format!("{}/dmn-rule/execute/single-result", base_url))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "decisionKey": "loanEligibility",
            "inputVariables": [
                {
                    "name": "creditScore",
                    "value": 710
                }
            ]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(decision_single_result.status(), StatusCode::CREATED);
    let decision_single_result_body: Value = decision_single_result.json().await.unwrap();
    assert_eq!(
        decision_single_result_body["decisionKey"],
        "loanEligibility"
    );
    // P82d: single-result stays one level deep; P85 wraps each variable
    assert_eq!(
        decision_single_result_body["resultVariables"][0],
        json!({"name": "approved", "type": "boolean", "value": true})
    );

    let decision_endpoint_single_result = client
        .post(format!(
            "{}/dmn-rule/execute-decision/single-result",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "decisionKey": "loanEligibility",
            "variables": {
                "creditScore": 650
            }
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(
        decision_endpoint_single_result.status(),
        StatusCode::CREATED
    );
    let decision_endpoint_single_result_body: Value =
        decision_endpoint_single_result.json().await.unwrap();
    assert_eq!(
        decision_endpoint_single_result_body["decisionKey"],
        "loanEligibility"
    );
    assert_eq!(
        decision_endpoint_single_result_body["resultVariables"][0],
        json!({"name": "approved", "type": "boolean", "value": false})
    );

    let decision_service = client
        .post(format!("{}/dmn-rule/execute-decision-service", base_url))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "decisionKey": "loanEligibility",
            "inputVariables": [
                {
                    "name": "creditScore",
                    "value": 705
                }
            ]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(decision_service.status(), StatusCode::CREATED);
    let decision_service_body: Value = decision_service.json().await.unwrap();
    assert_eq!(decision_service_body["decisionKey"], "loanEligibility");
    assert_eq!(
        decision_service_body["resultVariables"][0][0],
        json!({"name": "approved", "type": "boolean", "value": true})
    );

    let decision_service_single_result = client
        .post(format!(
            "{}/dmn-rule/execute-decision-service/single-result",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "decisionKey": "loanEligibility",
            "variables": {
                "creditScore": 640
            }
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(decision_service_single_result.status(), StatusCode::CREATED);
    let decision_service_single_result_body: Value =
        decision_service_single_result.json().await.unwrap();
    assert_eq!(
        decision_service_single_result_body["decisionKey"],
        "loanEligibility"
    );
    assert_eq!(
        decision_service_single_result_body["resultVariables"][0],
        json!({"name": "approved", "type": "boolean", "value": false})
    );

    let historic_id = execute_body["id"].as_str().unwrap();
    let historic_get = client
        .get(format!(
            "{}/dmn-history/historic-decision-executions/{}",
            base_url, historic_id
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(historic_get.status(), StatusCode::OK);
    let historic_get_body: Value = historic_get.json().await.unwrap();
    assert_eq!(historic_get_body["id"], historic_id);
    assert_eq!(historic_get_body["decisionKey"], "loanEligibility");
    assert_eq!(historic_get_body["inputVariables"]["creditScore"], 730);
    // Historic responses stay raw: Java serves audit data from the stored
    // execution JSON, not through DmnRestResponseFactory (P85).
    assert_eq!(historic_get_body["resultVariables"][0]["approved"], true);

    let auditdata = client
        .get(format!(
            "{}/dmn-history/historic-decision-executions/{}/auditdata",
            base_url, historic_id
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(auditdata.status(), StatusCode::OK);
    let auditdata_body: Value = auditdata.json().await.unwrap();
    assert_eq!(auditdata_body["id"], historic_id);
    assert_eq!(auditdata_body["decisionKey"], "loanEligibility");
    assert_eq!(auditdata_body["inputVariables"]["country"], "CN");
    assert_eq!(auditdata_body["resultVariables"][0]["riskBand"], "LOW");

    let dmn_engine = client
        .get(format!("{}/dmn-management/engine", base_url))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(dmn_engine.status(), StatusCode::OK);
    let dmn_engine_body: Value = dmn_engine.json().await.unwrap();
    assert_eq!(dmn_engine_body["name"], "flowable-dmn-engine");
    assert!(dmn_engine_body["version"].is_string());
    assert!(dmn_engine_body["resourceUrl"].is_null());
    assert!(dmn_engine_body["exception"].is_null());
}

#[tokio::test]
async fn dmn_runtime_and_history_routes_enforce_auth_and_structured_errors() {
    let (base_url, client) = spawn_server(Arc::new(MockDmnApi::default())).await;

    let unauthorized = client
        .post(format!("{}/dmn-runtime/decision-executions", base_url))
        .json(&json!({
            "decisionKey": "loanEligibility"
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);
    let unauthorized_body: Value = unauthorized.json().await.unwrap();
    assert_eq!(unauthorized_body["code"], "UNAUTHORIZED");

    let bad_query = client
        .get(format!(
            "{}/dmn-history/historic-decision-executions?unknownField=value",
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
            .contains("unknownField")
    );

    let missing = client
        .post(format!("{}/dmn-runtime/decision-executions", base_url))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "decisionKey": "missingDecision"
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
    let missing_body: Value = missing.json().await.unwrap();
    assert_eq!(missing_body["code"], "NOT_FOUND");

    let invalid_body = client
        .post(format!("{}/dmn-runtime/decision-executions", base_url))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "decisionKey": "loanEligibility",
            "unsupportedField": true
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(invalid_body.status(), StatusCode::BAD_REQUEST);
    let invalid_body_json: Value = invalid_body.json().await.unwrap();
    assert_eq!(invalid_body_json["code"], "BAD_REQUEST");

    let missing_historic = client
        .get(format!(
            "{}/dmn-history/historic-decision-executions/missing-execution",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(missing_historic.status(), StatusCode::NOT_FOUND);
    let missing_historic_body: Value = missing_historic.json().await.unwrap();
    assert_eq!(missing_historic_body["code"], "NOT_FOUND");
}

// ---------------------------------------------------------------------------
// P82d — single-result multi-row validation (HTTP 500) + flatten + empty
// ---------------------------------------------------------------------------

/// COLLECT multi-row via `/dmn-rule/execute/single-result` → 500 "more than one result".
#[tokio::test]
async fn p82d_single_result_collect_multi_row_returns_500() {
    let (base_url, client) = spawn_server(Arc::new(MockDmnApi::default())).await;

    for path in [
        "/dmn-rule/execute/single-result",
        "/dmn-rule/execute-decision/single-result",
    ] {
        let response = client
            .post(format!("{base_url}{path}"))
            .basic_auth("admin", Some("test"))
            .json(&json!({
                "decisionKey": "collectRouting",
                "variables": { "channel": "email" }
            }))
            .send()
            .await
            .unwrap();

        assert_eq!(
            response.status(),
            StatusCode::INTERNAL_SERVER_ERROR,
            "path {path} should be 500"
        );
        let body: Value = response.json().await.unwrap();
        assert_eq!(body["code"], "INTERNAL_SERVER_ERROR");
        // 5xx details are generic (no multi-row / decision-key text echo).
        assert_eq!(
            body["details"],
            "Internal server error",
            "path {path} unexpected details"
        );
    }
}

/// Decision-service single-result multi-row → 500 with decision key.
#[tokio::test]
async fn p82d_decision_service_single_result_multi_row_includes_key() {
    let (base_url, client) = spawn_server(Arc::new(MockDmnApi::default())).await;

    let response = client
        .post(format!(
            "{base_url}/dmn-rule/execute-decision-service/single-result"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "decisionKey": "collectRouting",
            "variables": { "channel": "email" }
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let body: Value = response.json().await.unwrap();
    // 5xx details are generic (decision key not echoed to clients).
    assert_eq!(body["details"], "Internal server error");
}

/// Zero hits on single-result → 201 with an empty resultVariables list
/// (Java `DmnRuleServiceSingleResponse.java:25` initialises it to an empty list).
#[tokio::test]
async fn p82d_single_result_zero_hits_returns_201_empty() {
    let (base_url, client) = spawn_server(Arc::new(MockDmnApi::default())).await;

    for path in [
        "/dmn-rule/execute/single-result",
        "/dmn-rule/execute-decision/single-result",
        "/dmn-rule/execute-decision-service/single-result",
    ] {
        let response = client
            .post(format!("{base_url}{path}"))
            .basic_auth("admin", Some("test"))
            .json(&json!({
                "decisionKey": "emptyHits",
                "variables": {}
            }))
            .send()
            .await
            .unwrap();

        assert_eq!(
            response.status(),
            StatusCode::CREATED,
            "path {path} should be 201"
        );
        let body: Value = response.json().await.unwrap();
        assert_eq!(body["decisionKey"], "emptyHits");
        assert_eq!(
            body["resultVariables"],
            json!([]),
            "path {path}: empty single-result must be an empty variable list"
        );
    }
}

/// P85: `type` inference over every JSON value kind, per Java
/// `DmnRestResponseFactory#createRestVariable`
/// (`DmnRestResponseFactory.java:257-292`) dispatching on the value's class.
#[tokio::test]
async fn p85_result_variable_type_inference_covers_all_value_kinds() {
    let (base_url, client) = spawn_server(Arc::new(MockDmnApi::default())).await;

    let response = client
        .post(format!("{base_url}/dmn-rule/execute"))
        .basic_auth("admin", Some("test"))
        .json(&json!({"decisionKey": "allValueKinds", "variables": {}}))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CREATED);
    let body: Value = response.json().await.unwrap();

    assert_eq!(
        body["resultVariables"][0],
        json!([
            {"name": "aString", "type": "string", "value": "text"},
            {"name": "bInteger", "type": "long", "value": 42},
            {"name": "cNegative", "type": "long", "value": -7},
            {"name": "dDouble", "type": "double", "value": 1.5},
            {"name": "eBoolean", "type": "boolean", "value": false},
            // Java DmnRestResponseFactory.java:263 skips the converter loop for
            // a null value, leaving `type` null → omitted by @JsonInclude.
            {"name": "fNull", "value": null},
            {"name": "gObject", "type": "json", "value": {"nested": 1}},
            {"name": "hArray", "type": "json", "value": [1, "two"]},
        ])
    );

    // `type` must be absent, not present-and-null, for the null value.
    let null_var = &body["resultVariables"][0][5];
    assert!(
        null_var.get("type").is_none(),
        "null value must omit type: {null_var}"
    );
}

/// P85: every variable in every row of a multi-hit result is wrapped, and the
/// outer list stays one entry per matched rule (Java
/// `DmnRuleServiceResponse.resultVariables`: `List<List<EngineRestVariable>>`).
#[tokio::test]
async fn p85_multi_hit_rows_wrap_every_variable() {
    let (base_url, client) = spawn_server(Arc::new(MockDmnApi::default())).await;

    let response = client
        .post(format!("{base_url}/dmn-rule/execute"))
        .basic_auth("admin", Some("test"))
        .json(&json!({"decisionKey": "collectRouting", "variables": {}}))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CREATED);
    let body: Value = response.json().await.unwrap();

    assert_eq!(body["multipleResults"], true);
    assert_eq!(
        body["resultVariables"],
        json!([
            [
                {"name": "priority", "type": "long", "value": 10},
                {"name": "route", "type": "string", "value": "manual"},
            ],
            [
                {"name": "priority", "type": "long", "value": 20},
                {"name": "route", "type": "string", "value": "email-queue"},
            ],
        ])
    );
}

/// P85: single-result endpoints keep P82d's one-level list but wrap each
/// variable (Java `DmnRuleServiceSingleResponse.resultVariables`:
/// `List<EngineRestVariable>` — no nesting).
#[tokio::test]
async fn p85_single_result_wraps_variables_without_nesting() {
    let (base_url, client) = spawn_server(Arc::new(MockDmnApi::default())).await;

    for path in [
        "/dmn-rule/execute/single-result",
        "/dmn-rule/execute-decision/single-result",
        "/dmn-rule/execute-decision-service/single-result",
    ] {
        let response = client
            .post(format!("{base_url}{path}"))
            .basic_auth("admin", Some("test"))
            .json(&json!({"decisionKey": "allValueKinds", "variables": {}}))
            .send()
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::CREATED, "path {path}");
        let body: Value = response.json().await.unwrap();

        // One level deep: element 0 is a variable object, not a nested list.
        assert_eq!(
            body["resultVariables"][0],
            json!({"name": "aString", "type": "string", "value": "text"}),
            "path {path}"
        );
        assert_eq!(
            body["resultVariables"].as_array().unwrap().len(),
            8,
            "path {path}: all variables of the single row are present"
        );
    }
}

/// P85: historic + audit-data responses stay raw. Java's audit endpoint returns
/// the stored execution JSON verbatim
/// (`BaseHistoricDecisionExecutionResource.java:65-75`) and
/// `HistoricDecisionExecutionResponse.java:24-38` has no `resultVariables`
/// field at all, so neither passes through `DmnRestResponseFactory`.
#[tokio::test]
async fn p85_historic_and_audit_data_keep_raw_result_variables() {
    let (base_url, client) = spawn_server(Arc::new(MockDmnApi::default())).await;

    let execute = client
        .post(format!("{base_url}/dmn-rule/execute"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "decisionKey": "loanEligibility",
            "variables": {"creditScore": 780}
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(execute.status(), StatusCode::CREATED);
    let execute_body: Value = execute.json().await.unwrap();
    let execution_id = execute_body["id"].as_str().unwrap().to_string();

    // Execution response: wrapped.
    assert_eq!(
        execute_body["resultVariables"][0][0],
        json!({"name": "approved", "type": "boolean", "value": true})
    );

    for suffix in ["", "/auditdata"] {
        let response = client
            .get(format!(
                "{base_url}/dmn-history/historic-decision-executions/{execution_id}{suffix}"
            ))
            .basic_auth("admin", Some("test"))
            .send()
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK, "suffix '{suffix}'");
        let body: Value = response.json().await.unwrap();
        assert_eq!(
            body["resultVariables"][0],
            json!({"approved": true, "riskBand": "LOW"}),
            "suffix '{suffix}': historic results stay unwrapped"
        );
    }
}
