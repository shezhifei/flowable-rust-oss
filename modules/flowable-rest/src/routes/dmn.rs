use crate::common::{PagedResponse, PagingQuery, parse_query};
use crate::error::ApiError;
use axum::{
    Extension, Json, Router,
    extract::Path,
    http::{StatusCode, Uri, header},
    response::IntoResponse,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::BTreeMap, sync::Arc};

pub type DynDmnRepository = Arc<dyn DmnRepositoryApi>;
pub type DynDmnRuntime = Arc<dyn DmnRuntimeApi>;
pub type DynDmnHistory = Arc<dyn DmnHistoryApi>;

pub trait DmnRepositoryApi: Send + Sync {
    fn deploy_decision_tables(
        &self,
        command: DmnDeploymentCommand,
    ) -> Result<DmnDeploymentRecord, ApiError>;
    fn get_deployment(&self, deployment_id: &str) -> Result<DmnDeploymentRecord, ApiError> {
        Err(ApiError::NotFound(format!(
            "DMN deployment '{deployment_id}' was not found"
        )))
    }
    fn list_deployments(
        &self,
        query: DmnDeploymentQuery,
    ) -> Result<PagedResponse<DmnDeploymentRecord>, ApiError> {
        let deployment_id = query.id.clone();
        let mut deployments = match deployment_id {
            Some(deployment_id) => vec![self.get_deployment(&deployment_id)?],
            None => Vec::new(),
        };
        deployments.retain(|deployment| deployment_matches_query(deployment, &query));
        sort_deployments(
            &mut deployments,
            query.sort.as_deref(),
            query.order.as_deref(),
        );
        Ok(query.paging.paginate(deployments))
    }
    fn delete_deployment(&self, deployment_id: &str, cascade: bool) -> Result<(), ApiError> {
        let _ = cascade;
        Err(ApiError::NotFound(format!(
            "DMN deployment '{deployment_id}' was not found"
        )))
    }
    fn get_deployment_resource_data(
        &self,
        deployment_id: &str,
        resource_name: &str,
    ) -> Result<DmnResourceDataRecord, ApiError> {
        Err(ApiError::NotFound(format!(
            "DMN deployment resource '{resource_name}' was not found in deployment '{deployment_id}'"
        )))
    }
    fn list_decision_tables(
        &self,
        query: DecisionTableQuery,
    ) -> Result<PagedResponse<DecisionTableRecord>, ApiError>;
    fn get_decision_table(&self, decision_table_id: &str) -> Result<DecisionTableRecord, ApiError>;
    fn get_decision_table_resource_data(
        &self,
        decision_table_id: &str,
    ) -> Result<DmnResourceDataRecord, ApiError> {
        Err(ApiError::NotFound(format!(
            "DMN decision table resource for '{decision_table_id}' was not found"
        )))
    }
    fn get_decision_table_model(&self, decision_table_id: &str) -> Result<Value, ApiError> {
        serde_json::to_value(self.get_decision_table(decision_table_id)?)
            .map_err(|err| ApiError::InternalServerError(err.to_string()))
    }
    fn get_drd(&self, drd_id: &str) -> Result<Value, ApiError> {
        let _ = drd_id;
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
    fn get_drd_resource_data(&self, drd_id: &str) -> Result<DmnResourceDataRecord, ApiError> {
        let _ = drd_id;
        Err(ApiError::NotFound(
            "DRD resource data not found".to_string(),
        ))
    }
    fn get_decision_image(&self, decision_id: &str) -> Result<Vec<u8>, ApiError> {
        let _ = decision_id;
        Err(ApiError::NotFound(format!(
            "DMN decision image for '{decision_id}' was not found"
        )))
    }
    fn list_decision_services(
        &self,
        query: DecisionServiceQuery,
    ) -> Result<PagedResponse<DecisionServiceRecord>, ApiError> {
        let _ = query;
        Ok(PagedResponse {
            start: 0,
            size: 0,
            total: 0,
            data: Vec::new(),
            sort: None,
            order: None,
        })
    }
    fn get_decision_service(
        &self,
        decision_service_id: &str,
    ) -> Result<DecisionServiceRecord, ApiError> {
        let page = self.list_decision_services(DecisionServiceQuery {
            paging: PagingQuery {
                start: 0,
                size: None,
            },
            id: Some(decision_service_id.to_string()),
            ..DecisionServiceQuery::default()
        })?;

        page.data.into_iter().next().ok_or_else(|| {
            ApiError::NotFound(format!(
                "DMN decision service '{decision_service_id}' was not found"
            ))
        })
    }
}

pub trait DmnRuntimeApi: Send + Sync {
    fn execute_decision(
        &self,
        command: DecisionExecutionCommand,
    ) -> Result<DecisionExecutionRecord, ApiError>;
}

pub trait DmnHistoryApi: Send + Sync {
    fn list_historic_decision_executions(
        &self,
        query: HistoricDecisionExecutionQuery,
    ) -> Result<PagedResponse<HistoricDecisionExecutionRecord>, ApiError>;
    fn get_historic_decision_execution(
        &self,
        historic_decision_execution_id: &str,
    ) -> Result<HistoricDecisionExecutionRecord, ApiError> {
        let page = self.list_historic_decision_executions(HistoricDecisionExecutionQuery {
            paging: PagingQuery {
                start: 0,
                size: Some(1),
            },
            id: Some(historic_decision_execution_id.to_string()),
            decision_key: None,
            decision_table_id: None,
            deployment_id: None,
            business_key: None,
            activity_id: None,
            instance_id: None,
            scope_type: None,
            without_scope_type: false,
            failed: None,
            tenant_id: None,
            tenant_id_like: None,
            sort: None,
            order: None,
        })?;

        page.data.into_iter().next().ok_or_else(|| {
            ApiError::NotFound(format!(
                "Historic decision execution '{historic_decision_execution_id}' was not found"
            ))
        })
    }
    fn delete_historic_decision_execution(
        &self,
        historic_decision_execution_id: &str,
    ) -> Result<(), ApiError>;
    fn bulk_delete_historic_decision_executions(
        &self,
        historic_decision_execution_ids: Vec<String>,
    ) -> Result<(), ApiError>;
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineInfoRecord {
    pub name: String,
    pub version: String,
    pub resource_url: Option<String>,
    pub exception: Option<String>,
}

#[derive(Debug, Clone)]
pub struct DmnDeploymentCommand {
    pub name: String,
    pub category: Option<String>,
    pub parent_deployment_id: Option<String>,
    pub tenant_id: Option<String>,
    pub resources: Vec<DmnDeploymentResourcePayload>,
}

#[derive(Debug, Clone)]
pub struct DmnDeploymentResourcePayload {
    pub resource_name: String,
    pub resource: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DmnDeploymentRecord {
    pub id: String,
    pub name: String,
    pub category: Option<String>,
    pub parent_deployment_id: Option<String>,
    pub deployed_at: i64,
    pub resource_names: Vec<String>,
    pub tenant_id: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct DmnDeploymentQuery {
    pub paging: PagingQuery,
    pub id: Option<String>,
    pub name: Option<String>,
    pub name_like: Option<String>,
    pub category: Option<String>,
    pub category_not_equals: Option<String>,
    pub parent_deployment_id: Option<String>,
    pub parent_deployment_id_like: Option<String>,
    pub tenant_id: Option<String>,
    pub tenant_id_like: Option<String>,
    pub without_tenant_id: bool,
    pub resource_name: Option<String>,
    pub sort: Option<String>,
    pub order: Option<String>,
}

#[derive(Debug, Clone)]
pub struct DmnResourceDataRecord {
    pub mime_type: String,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, Default)]
pub struct DecisionTableQuery {
    pub paging: PagingQuery,
    pub id: Option<String>,
    pub key: Option<String>,
    pub key_like: Option<String>,
    pub name: Option<String>,
    pub name_like: Option<String>,
    pub category: Option<String>,
    pub category_like: Option<String>,
    pub category_not_equals: Option<String>,
    pub deployment_id: Option<String>,
    pub parent_deployment_id: Option<String>,
    pub resource_name: Option<String>,
    pub resource_name_like: Option<String>,
    pub tenant_id: Option<String>,
    pub tenant_id_like: Option<String>,
    pub decision_type: Option<String>,
    pub decision_type_like: Option<String>,
    pub version: Option<i32>,
    pub latest: bool,
    pub sort: Option<String>,
    pub order: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DecisionTableRecord {
    pub id: String,
    pub key: String,
    pub name: String,
    pub version: i32,
    pub deployment_id: String,
    pub resource_name: String,
    pub category: Option<String>,
    pub description: Option<String>,
    pub tenant_id: Option<String>,
    pub parent_deployment_id: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct DecisionServiceQuery {
    pub paging: PagingQuery,
    pub id: Option<String>,
    pub key: Option<String>,
    pub key_like: Option<String>,
    pub name: Option<String>,
    pub name_like: Option<String>,
    pub deployment_id: Option<String>,
    pub parent_deployment_id: Option<String>,
    pub resource_name: Option<String>,
    /// P133: resourceNameLike (resource_name is on the record)
    pub resource_name_like: Option<String>,
    pub tenant_id: Option<String>,
    pub tenant_id_like: Option<String>,
    pub sort: Option<String>,
    pub order: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DecisionServiceRecord {
    pub id: String,
    pub key: String,
    pub name: String,
    pub deployment_id: String,
    pub resource_name: String,
    pub tenant_id: Option<String>,
    pub parent_deployment_id: Option<String>,
    pub required_decision_keys: Vec<String>,
    pub output_decision_keys: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct DecisionExecutionCommand {
    pub decision_key: String,
    pub tenant_id: Option<String>,
    pub parent_deployment_id: Option<String>,
    pub business_key: Option<String>,
    pub variables: BTreeMap<String, Value>,
    pub disable_history: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DecisionExpressionExecutionRecord {
    pub id: String,
    pub result: Value,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DecisionRuleExecutionRecord {
    pub rule_number: usize,
    pub rule_id: String,
    pub valid: bool,
    pub condition_results: Vec<DecisionExpressionExecutionRecord>,
    pub conclusion_results: Vec<DecisionExpressionExecutionRecord>,
}

/// Java `EngineRestVariable`
/// (`flowable-common-rest/src/main/java/org/flowable/common/rest/variable/EngineRestVariable.java:24-68`)
/// — `{name, type, value}` plus a `valueUrl` that DMN results never populate
/// (`DmnRestResponseFactory.java:288` leaves it as a TODO).
///
/// `type` carries `@JsonInclude(NON_NULL)` (EngineRestVariable.java:41) so it
/// disappears for a null value; `getValue` (line 50-51) has no such annotation,
/// so `value` is always emitted.
#[derive(Debug, Clone, Serialize)]
pub struct EngineRestVariable {
    pub name: String,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub variable_type: Option<String>,
    pub value: Value,
}

/// Java `DmnRestResponseFactory#createRestVariable`
/// (`flowable-dmn-rest/.../service/api/DmnRestResponseFactory.java:257-292`):
/// the first registered converter whose Java type accepts the value supplies
/// `type` (converter list at lines 297-311); a null value skips the loop
/// entirely (line 263) and yields a name-only variable with no `type`.
///
/// Java reads the type off the value's Java *class*, which the DMN engine pins
/// per output `typeRef` in `ExecutionVariableFactory.java:48-81`
/// (boolean→`Boolean`, string→`String`, number→`Double`, date→`Date`).
/// Rust decision results are plain `serde_json::Value`, so the class is
/// recovered from the JSON kind instead:
///
/// - `Bool` → `boolean` (`BooleanRestVariableConverter`, registered at :305)
/// - `String` → `string` (`StringRestVariableConverter`, :298)
/// - float → `double` (`DoubleRestVariableConverter.java:24-31`, registered at
///   :302). After P88, DMN output `typeRef` number/double always normalizes to
///   JSON f64 in the engine (`ExecutionVariableFactory.java:60-69`), so real
///   engine numeric outputs almost always report `"double"` — matching Java
///   DMN REST, where `"long"` only appears for the rare BigInteger corner
///   (`ExecutionVariableFactory.java:65-66`).
/// - integer → `long` (`LongRestVariableConverter`, :300). Kept as the JSON
///   integer fallback for untyped outputs / MockDmnApi synthetic rows that
///   still emit integer JSON; not the normal DMN number/double path.
/// - `Null` → no type, per the line 263 guard
///
/// Two deliberate departures, both outside what Java can express:
///
/// 1. `Object`/`Array` → `json`. Java's DMN factory registers no JSON
///    converter (:297-311), so such a value would fall through to
///    `SERIALIZABLE_VARIABLE_TYPE` (:281) *and be dropped* — `createRestVariable`
///    only sets the value for the serializable branch when `includeBinaryValue`
///    is true (:284-286), and every DMN call site passes `false` (:135, :151,
///    :178, :198). The Java DMN engine cannot produce map/list outputs at all
///    (`ExecutionVariableFactory.java:82-85` throws on any other typeRef), so
///    there is no Java behaviour to match here; the Rust engine does support
///    `context`/`list` typeRefs, and their values are ordinary JSON. Emitting
///    `json` (a documented engine type — EngineRestVariable.java:40) keeps the
///    data instead of silently discarding it.
/// 2. Temporal outputs surface as `string`, not Java's `date`. The Rust engine
///    normalizes them to ISO-8601 strings before they reach the REST layer, and
///    the `typeRef` is not carried on the result, so the distinction is not
///    recoverable here. Sniffing the string content would misclassify genuine
///    string outputs, which Java never does — it dispatches on the class.
fn engine_rest_variable(name: &str, value: &Value) -> EngineRestVariable {
    let variable_type = match value {
        // Java DmnRestResponseFactory.java:263 — null values get no type.
        Value::Null => None,
        Value::Bool(_) => Some("boolean"),
        Value::String(_) => Some("string"),
        Value::Number(number) => {
            if number.is_f64() {
                Some("double")
            } else {
                Some("long")
            }
        }
        Value::Object(_) | Value::Array(_) => Some("json"),
    };

    EngineRestVariable {
        name: name.to_string(),
        variable_type: variable_type.map(str::to_string),
        value: value.clone(),
    }
}

/// Wrap one output row as Java does in
/// `DmnRestResponseFactory.java:133-136` — one `EngineRestVariable` per entry,
/// in the result map's iteration order.
pub fn engine_rest_variable_row(row: &BTreeMap<String, Value>) -> Vec<EngineRestVariable> {
    row.iter()
        .map(|(name, value)| engine_rest_variable(name, value))
        .collect()
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DecisionExecutionRecord {
    pub id: String,
    pub decision_table_id: String,
    pub deployment_id: String,
    pub decision_key: String,
    pub tenant_id: Option<String>,
    pub business_key: Option<String>,
    pub hit_policy: String,
    pub executed_at: i64,
    pub rule_hit_count: usize,
    pub input_variables: BTreeMap<String, Value>,
    /// Row-shaped result (Java `DmnRuleServiceResponse.resultVariables`:
    /// `List<List<EngineRestVariable>>` —
    /// `flowable-dmn-rest/.../decision/DmnRuleServiceResponse.java:25`), one
    /// inner list per matched rule (`DmnRestResponseFactory.java:131-138`).
    pub result_variables: Vec<Vec<EngineRestVariable>>,
    /// Java `DecisionExecutionAuditContainer.multipleResults`.
    #[serde(default)]
    pub multiple_results: bool,
    pub rule_executions: Vec<DecisionRuleExecutionRecord>,
}

/// Java `DmnRuleServiceSingleResponse` — single-result endpoints flatten the
/// unique output row to one level.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DecisionExecutionSingleRecord {
    pub id: String,
    pub decision_table_id: String,
    pub deployment_id: String,
    pub decision_key: String,
    pub tenant_id: Option<String>,
    pub business_key: Option<String>,
    pub hit_policy: String,
    pub executed_at: i64,
    pub rule_hit_count: usize,
    pub input_variables: BTreeMap<String, Value>,
    /// Single-level list (Java `DmnRuleServiceSingleResponse.resultVariables`:
    /// `List<EngineRestVariable>` —
    /// `flowable-dmn-rest/.../decision/DmnRuleServiceSingleResponse.java:25`,
    /// filled one variable at a time by `DmnRestResponseFactory.java:149-153`).
    pub result_variables: Vec<EngineRestVariable>,
    #[serde(default)]
    pub multiple_results: bool,
    pub rule_executions: Vec<DecisionRuleExecutionRecord>,
}

#[derive(Debug, Clone, Default)]
pub struct HistoricDecisionExecutionQuery {
    pub paging: PagingQuery,
    pub id: Option<String>,
    pub decision_key: Option<String>,
    pub decision_table_id: Option<String>,
    pub deployment_id: Option<String>,
    pub business_key: Option<String>,
    pub activity_id: Option<String>,
    pub instance_id: Option<String>,
    pub scope_type: Option<String>,
    pub without_scope_type: bool,
    pub failed: Option<bool>,
    pub tenant_id: Option<String>,
    pub tenant_id_like: Option<String>,
    pub sort: Option<String>,
    pub order: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoricDecisionExecutionRecord {
    pub id: String,
    pub decision_table_id: String,
    pub deployment_id: String,
    pub decision_key: String,
    pub tenant_id: Option<String>,
    pub business_key: Option<String>,
    pub executed_at: i64,
    pub rule_hit_count: usize,
    pub input_variables: BTreeMap<String, Value>,
    /// Row-shaped historic result, deliberately **not** `EngineRestVariable`-
    /// wrapped: Java's `HistoricDecisionExecutionResponse.java:24-38` has no
    /// `resultVariables` field at all, and the audit-data endpoint hands back
    /// the stored execution JSON verbatim
    /// (`BaseHistoricDecisionExecutionResource.java:65-75` →
    /// `decisionExecution.getExecutionJson()`), never routing it through
    /// `DmnRestResponseFactory`. Raw values are the Java-faithful shape here.
    pub result_variables: Vec<BTreeMap<String, Value>>,
    #[serde(default)]
    pub multiple_results: bool,
    pub rule_executions: Vec<DecisionRuleExecutionRecord>,
    /// P83 — process correlation, mirroring Java
    /// `HistoricDecisionExecutionResponse.java:28-31`
    /// (`activityId` / `executionId` / `instanceId` / `scopeType`).
    ///
    /// Deviation: Java always emits these keys (null when unset); here they are
    /// omitted when absent so the P79/P82 response shape is unchanged for
    /// executions that carry no process context.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instance_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub execution_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub activity_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope_type: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DmnDeploymentRequest {
    name: String,
    category: Option<String>,
    parent_deployment_id: Option<String>,
    #[serde(default)]
    resources: Vec<DmnDeploymentResourceRequest>,
    tenant_id: Option<String>,
    resource_name: Option<String>,
    resource: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DmnDeploymentResourceRequest {
    resource_name: String,
    resource: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
struct DmnDeploymentQueryParams {
    start: usize,
    size: Option<usize>,
    id: Option<String>,
    name: Option<String>,
    name_like: Option<String>,
    category: Option<String>,
    category_not_equals: Option<String>,
    parent_deployment_id: Option<String>,
    parent_deployment_id_like: Option<String>,
    tenant_id: Option<String>,
    tenant_id_like: Option<String>,
    without_tenant_id: bool,
    resource_name: Option<String>,
    sort: Option<String>,
    order: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
struct DeleteDeploymentQueryParams {
    cascade: bool,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
struct DecisionTableQueryParams {
    start: usize,
    size: Option<usize>,
    id: Option<String>,
    key: Option<String>,
    key_like: Option<String>,
    name: Option<String>,
    name_like: Option<String>,
    category: Option<String>,
    category_like: Option<String>,
    category_not_equals: Option<String>,
    deployment_id: Option<String>,
    parent_deployment_id: Option<String>,
    resource_name: Option<String>,
    resource_name_like: Option<String>,
    tenant_id: Option<String>,
    tenant_id_like: Option<String>,
    decision_type: Option<String>,
    decision_type_like: Option<String>,
    version: Option<i32>,
    latest: bool,
    sort: Option<String>,
    order: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
struct DecisionServiceQueryParams {
    start: usize,
    size: Option<usize>,
    id: Option<String>,
    key: Option<String>,
    key_like: Option<String>,
    name: Option<String>,
    name_like: Option<String>,
    deployment_id: Option<String>,
    parent_deployment_id: Option<String>,
    resource_name: Option<String>,
    /// P133: resourceNameLike
    resource_name_like: Option<String>,
    tenant_id: Option<String>,
    tenant_id_like: Option<String>,
    sort: Option<String>,
    order: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DecisionExecutionRequest {
    decision_key: String,
    tenant_id: Option<String>,
    parent_deployment_id: Option<String>,
    business_key: Option<String>,
    #[serde(default)]
    variables: BTreeMap<String, Value>,
    #[serde(default)]
    input_variables: Vec<DecisionInputVariableRequest>,
    #[serde(default)]
    disable_history: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DecisionInputVariableRequest {
    name: String,
    value: Value,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct HistoricDecisionExecutionQueryParams {
    start: usize,
    size: Option<usize>,
    #[serde(rename = "id", alias = "decisionExecutionId")]
    id: Option<String>,
    #[serde(rename = "decisionKey")]
    decision_key: Option<String>,
    #[serde(rename = "decisionTableId", alias = "decisionDefinitionId")]
    decision_table_id: Option<String>,
    #[serde(rename = "deploymentId")]
    deployment_id: Option<String>,
    #[serde(rename = "businessKey")]
    business_key: Option<String>,
    #[serde(rename = "activityId")]
    activity_id: Option<String>,
    #[serde(rename = "executionId")]
    execution_id: Option<String>,
    #[serde(rename = "instanceId")]
    instance_id: Option<String>,
    #[serde(rename = "scopeType")]
    scope_type: Option<String>,
    #[serde(rename = "withoutScopeType")]
    without_scope_type: bool,
    #[serde(rename = "processInstanceIdWithChildren")]
    process_instance_id_with_children: Option<String>,
    #[serde(rename = "caseInstanceIdWithChildren")]
    case_instance_id_with_children: Option<String>,
    failed: Option<bool>,
    #[serde(rename = "tenantId")]
    tenant_id: Option<String>,
    #[serde(rename = "tenantIdLike")]
    tenant_id_like: Option<String>,
    sort: Option<String>,
    order: Option<String>,
}

impl DmnDeploymentRequest {
    fn into_command(self) -> Result<DmnDeploymentCommand, ApiError> {
        let resources = if self.resources.is_empty() {
            match (self.resource_name, self.resource) {
                (Some(resource_name), Some(resource)) => {
                    vec![DmnDeploymentResourcePayload {
                        resource_name,
                        resource,
                    }]
                }
                _ => {
                    return Err(ApiError::bad_request(
                        "DMN deployment requires at least one resource",
                    ));
                }
            }
        } else {
            self.resources
                .into_iter()
                .map(|resource| DmnDeploymentResourcePayload {
                    resource_name: resource.resource_name,
                    resource: resource.resource,
                })
                .collect()
        };

        Ok(DmnDeploymentCommand {
            name: self.name,
            category: self.category,
            parent_deployment_id: self.parent_deployment_id,
            tenant_id: self.tenant_id,
            resources,
        })
    }
}

impl From<DmnDeploymentQueryParams> for DmnDeploymentQuery {
    fn from(value: DmnDeploymentQueryParams) -> Self {
        Self {
            paging: PagingQuery {
                start: value.start,
                size: value.size,
            },
            id: value.id,
            name: value.name,
            name_like: value.name_like,
            category: value.category,
            category_not_equals: value.category_not_equals,
            parent_deployment_id: value.parent_deployment_id,
            parent_deployment_id_like: value.parent_deployment_id_like,
            tenant_id: value.tenant_id,
            tenant_id_like: value.tenant_id_like,
            without_tenant_id: value.without_tenant_id,
            resource_name: value.resource_name,
            sort: value.sort,
            order: value.order,
        }
    }
}

impl From<DecisionTableQueryParams> for DecisionTableQuery {
    fn from(value: DecisionTableQueryParams) -> Self {
        Self {
            paging: PagingQuery {
                start: value.start,
                size: value.size,
            },
            id: value.id,
            key: value.key,
            key_like: value.key_like,
            name: value.name,
            name_like: value.name_like,
            category: value.category,
            category_like: value.category_like,
            category_not_equals: value.category_not_equals,
            deployment_id: value.deployment_id,
            parent_deployment_id: value.parent_deployment_id,
            resource_name: value.resource_name,
            resource_name_like: value.resource_name_like,
            tenant_id: value.tenant_id,
            tenant_id_like: value.tenant_id_like,
            decision_type: value.decision_type,
            decision_type_like: value.decision_type_like,
            version: value.version,
            latest: value.latest,
            sort: value.sort,
            order: value.order,
        }
    }
}

impl From<DecisionServiceQueryParams> for DecisionServiceQuery {
    fn from(value: DecisionServiceQueryParams) -> Self {
        Self {
            paging: PagingQuery {
                start: value.start,
                size: value.size,
            },
            id: value.id,
            key: value.key,
            key_like: value.key_like,
            name: value.name,
            name_like: value.name_like,
            deployment_id: value.deployment_id,
            parent_deployment_id: value.parent_deployment_id,
            resource_name: value.resource_name,
            resource_name_like: value.resource_name_like,
            tenant_id: value.tenant_id,
            tenant_id_like: value.tenant_id_like,
            sort: value.sort,
            order: value.order,
        }
    }
}

impl From<DecisionExecutionRequest> for DecisionExecutionCommand {
    fn from(value: DecisionExecutionRequest) -> Self {
        let mut variables = value.variables;
        for input_variable in value.input_variables {
            variables.insert(input_variable.name, input_variable.value);
        }
        let _ = value.disable_history;
        Self {
            decision_key: value.decision_key,
            tenant_id: value.tenant_id,
            parent_deployment_id: value.parent_deployment_id,
            business_key: value.business_key,
            variables,
            disable_history: value.disable_history,
        }
    }
}

impl From<HistoricDecisionExecutionQueryParams> for HistoricDecisionExecutionQuery {
    fn from(value: HistoricDecisionExecutionQueryParams) -> Self {
        Self {
            paging: PagingQuery {
                start: value.start,
                size: value.size,
            },
            id: value.id,
            decision_key: value.decision_key,
            decision_table_id: value.decision_table_id,
            deployment_id: value.deployment_id,
            business_key: value.business_key,
            activity_id: value.activity_id,
            instance_id: value.instance_id,
            scope_type: value.scope_type,
            without_scope_type: value.without_scope_type,
            failed: value.failed,
            tenant_id: value.tenant_id,
            tenant_id_like: value.tenant_id_like,
            sort: value.sort,
            order: value.order,
        }
    }
}

pub fn router(
    repository: DynDmnRepository,
    runtime: DynDmnRuntime,
    history: DynDmnHistory,
) -> Router {
    Router::new()
        .route(
            "/dmn-repository/deployments",
            get(list_deployments).post(deploy),
        )
        .route(
            "/dmn-repository/deployments/:deployment_id",
            get(get_deployment).delete(delete_deployment),
        )
        .route(
            "/dmn-repository/deployments/:deployment_id/resources",
            get(list_deployment_resources),
        )
        .route(
            "/dmn-repository/deployments/:deployment_id/resourcedata/*resource_name",
            get(get_deployment_resource_data),
        )
        .route(
            "/dmn-repository/deployments/:deployment_id/resources/*resource_name",
            get(get_deployment_resource),
        )
        .route("/dmn-repository/decision-tables", get(list_decision_tables))
        .route("/dmn-repository/decisions", get(list_decision_tables))
        .route(
            "/dmn-repository/decision-services",
            get(list_decision_services),
        )
        .route(
            "/dmn-repository/decision-services/:decision_service_id",
            get(get_decision_service),
        )
        .route(
            "/dmn-repository/decision-tables/:decision_table_id/resourcedata",
            get(get_decision_table_resource_data),
        )
        .route(
            "/dmn-repository/decisions/:decision_table_id/resourcedata",
            get(get_decision_table_resource_data),
        )
        .route(
            "/dmn-repository/decision-tables/:decision_table_id/model",
            get(get_decision_table_model),
        )
        .route(
            "/dmn-repository/decisions/:decision_table_id/model",
            get(get_decision_table_model),
        )
        .route(
            "/dmn-repository/decision-tables/:decision_table_id",
            get(get_decision_table),
        )
        .route(
            "/dmn-repository/decisions/:decision_table_id",
            get(get_decision_table),
        )
        .route("/dmn-runtime/decision-executions", post(execute_decision))
        .route("/dmn-rule/execute", post(execute_decision))
        // P82d: single-result endpoints validate ≤1 row and flatten (Java
        // DmnRuleServiceResource single-result + DmnDecisionServiceImpl:118-146)
        .route(
            "/dmn-rule/execute/single-result",
            post(execute_decision_single_result),
        )
        .route("/dmn-rule/execute-decision", post(execute_decision))
        .route("/dmn-rule/execute-decision-service", post(execute_decision))
        .route(
            "/dmn-rule/execute-decision/single-result",
            post(execute_decision_single_result),
        )
        .route(
            "/dmn-rule/execute-decision-service/single-result",
            post(execute_decision_service_single_result),
        )
        .route("/dmn-management/engine", get(get_engine_info))
        .route(
            "/dmn-history/historic-decision-executions",
            get(list_historic_decision_executions),
        )
        .route(
            "/dmn-query/historic-decision-executions",
            post(query_historic_decision_executions),
        )
        .route(
            "/dmn-history/historic-decision-executions/:historic_decision_execution_id/auditdata",
            get(get_historic_decision_execution_audit_data),
        )
        .route(
            "/dmn-repository/decision-requirements-diagrams",
            get(list_drds),
        )
        .route(
            "/dmn-repository/decision-requirements-diagrams/:drd_id",
            get(get_drd),
        )
        .route(
            "/dmn-repository/decision-requirements-diagrams/:drd_id/resourcedata",
            get(get_drd_resource_data),
        )
        .route(
            "/dmn-history/historic-decision-executions/:historic_decision_execution_id",
            get(get_historic_decision_execution).delete(delete_historic_decision_execution),
        )
        .route(
            "/dmn-history/historic-decision-executions/delete",
            post(bulk_delete_historic_decision_executions),
        )
        .layer(Extension(repository))
        .layer(Extension(runtime))
        .layer(Extension(history))
}

pub async fn deploy(
    Extension(repository): Extension<DynDmnRepository>,
    body: String,
) -> Result<impl IntoResponse, ApiError> {
    let payload: DmnDeploymentRequest =
        serde_json::from_str(&body).map_err(|err| ApiError::bad_request(err.to_string()))?;
    let deployment = repository.deploy_decision_tables(payload.into_command()?)?;
    Ok((StatusCode::CREATED, Json(deployment)))
}

pub async fn list_deployments(
    Extension(repository): Extension<DynDmnRepository>,
    uri: Uri,
) -> Result<Json<PagedResponse<DmnDeploymentRecord>>, ApiError> {
    let query: DmnDeploymentQueryParams = parse_query(&uri)?;
    validate_deployment_query(&query)?;
    Ok(Json(repository.list_deployments(query.into())?))
}

pub async fn get_deployment(
    Extension(repository): Extension<DynDmnRepository>,
    Path(deployment_id): Path<String>,
) -> Result<Json<DmnDeploymentRecord>, ApiError> {
    Ok(Json(repository.get_deployment(&deployment_id)?))
}

pub async fn delete_deployment(
    Extension(repository): Extension<DynDmnRepository>,
    Path(deployment_id): Path<String>,
    uri: Uri,
) -> Result<StatusCode, ApiError> {
    let query: DeleteDeploymentQueryParams = parse_query(&uri)?;
    repository.delete_deployment(&deployment_id, query.cascade)?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn list_deployment_resources(
    Extension(repository): Extension<DynDmnRepository>,
    Path(deployment_id): Path<String>,
) -> Result<Json<Vec<Value>>, ApiError> {
    let deployment = repository.get_deployment(&deployment_id)?;
    let resources = deployment
        .resource_names
        .iter()
        .map(|resource_name| resource_response(&deployment_id, resource_name))
        .collect();
    Ok(Json(resources))
}

pub async fn get_deployment_resource(
    Extension(repository): Extension<DynDmnRepository>,
    Path((deployment_id, resource_name)): Path<(String, String)>,
) -> Result<Json<Value>, ApiError> {
    repository.get_deployment_resource_data(&deployment_id, &resource_name)?;
    Ok(Json(resource_response(&deployment_id, &resource_name)))
}

pub async fn get_deployment_resource_data(
    Extension(repository): Extension<DynDmnRepository>,
    Path((deployment_id, resource_name)): Path<(String, String)>,
) -> Result<impl IntoResponse, ApiError> {
    Ok(resource_data_response(
        repository.get_deployment_resource_data(&deployment_id, &resource_name)?,
    ))
}

pub async fn list_decision_tables(
    Extension(repository): Extension<DynDmnRepository>,
    uri: Uri,
) -> Result<Json<PagedResponse<DecisionTableRecord>>, ApiError> {
    let query: DecisionTableQueryParams = parse_query(&uri)?;
    validate_decision_table_query(&query)?;
    Ok(Json(repository.list_decision_tables(query.into())?))
}

pub async fn get_decision_table(
    Extension(repository): Extension<DynDmnRepository>,
    Path(decision_table_id): Path<String>,
) -> Result<Json<DecisionTableRecord>, ApiError> {
    Ok(Json(repository.get_decision_table(&decision_table_id)?))
}

pub async fn get_decision_table_resource_data(
    Extension(repository): Extension<DynDmnRepository>,
    Path(decision_table_id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    Ok(resource_data_response(
        repository.get_decision_table_resource_data(&decision_table_id)?,
    ))
}

pub async fn get_decision_table_model(
    Extension(repository): Extension<DynDmnRepository>,
    Path(decision_table_id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    Ok(Json(
        repository.get_decision_table_model(&decision_table_id)?,
    ))
}

pub async fn list_decision_services(
    Extension(repository): Extension<DynDmnRepository>,
    uri: Uri,
) -> Result<Json<PagedResponse<DecisionServiceRecord>>, ApiError> {
    let query: DecisionServiceQueryParams = parse_query(&uri)?;
    validate_decision_service_query(&query)?;
    Ok(Json(repository.list_decision_services(query.into())?))
}

pub async fn get_decision_service(
    Extension(repository): Extension<DynDmnRepository>,
    Path(decision_service_id): Path<String>,
) -> Result<Json<DecisionServiceRecord>, ApiError> {
    Ok(Json(repository.get_decision_service(&decision_service_id)?))
}

fn resource_data_response(resource: DmnResourceDataRecord) -> impl IntoResponse {
    ([(header::CONTENT_TYPE, resource.mime_type)], resource.bytes)
}

fn resource_response(deployment_id: &str, resource_name: &str) -> Value {
    serde_json::json!({
        "id": resource_name,
        "url": format!(
            "/dmn-repository/deployments/{deployment_id}/resourcedata/{resource_name}"
        ),
        "contentUrl": format!(
            "/dmn-repository/deployments/{deployment_id}/resourcedata/{resource_name}"
        ),
        "mediaType": "application/xml",
        "type": "dmn",
    })
}

fn deployment_matches_query(deployment: &DmnDeploymentRecord, query: &DmnDeploymentQuery) -> bool {
    query.id.as_ref().is_none_or(|id| deployment.id == *id)
        && query
            .name
            .as_ref()
            .is_none_or(|name| deployment.name == *name)
        && query
            .name_like
            .as_ref()
            .is_none_or(|name_like| deployment.name.contains(name_like))
        && query
            .category
            .as_ref()
            .is_none_or(|category| deployment.category.as_deref() == Some(category.as_str()))
        && query
            .category_not_equals
            .as_ref()
            .is_none_or(|category| deployment.category.as_deref() != Some(category.as_str()))
        && query.parent_deployment_id.as_ref().is_none_or(|parent| {
            deployment.parent_deployment_id.as_deref() == Some(parent.as_str())
        })
        && query
            .parent_deployment_id_like
            .as_ref()
            .is_none_or(|parent| {
                deployment
                    .parent_deployment_id
                    .as_deref()
                    .is_some_and(|candidate| wildcard_like(candidate, parent))
            })
        && query
            .tenant_id
            .as_ref()
            .is_none_or(|tenant_id| deployment.tenant_id.as_deref() == Some(tenant_id.as_str()))
        && query.tenant_id_like.as_ref().is_none_or(|tenant_id_like| {
            deployment
                .tenant_id
                .as_deref()
                .is_some_and(|tenant_id| tenant_id.contains(tenant_id_like))
        })
        && (!query.without_tenant_id || deployment.tenant_id.is_none())
        && query.resource_name.as_ref().is_none_or(|resource_name| {
            deployment
                .resource_names
                .iter()
                .any(|candidate| candidate == resource_name)
        })
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

fn validate_deployment_query(query: &DmnDeploymentQueryParams) -> Result<(), ApiError> {
    match query.sort.as_deref() {
        None
        | Some("id")
        | Some("name")
        | Some("category")
        | Some("deployTime")
        | Some("parentDeploymentId")
        | Some("tenantId") => {}
        Some(sort) => {
            return Err(ApiError::bad_request(format!(
                "Unsupported sort property '{sort}' for DMN deployments"
            )));
        }
    }

    match query.order.as_deref() {
        None | Some("asc") | Some("desc") => Ok(()),
        Some(order) => Err(ApiError::bad_request(format!(
            "Unsupported order '{order}' for DMN deployments"
        ))),
    }
}

pub(crate) fn sort_deployments(
    deployments: &mut [DmnDeploymentRecord],
    sort: Option<&str>,
    order: Option<&str>,
) {
    deployments.sort_by(|left, right| {
        let ordering = match sort.unwrap_or("id") {
            "name" => left.name.cmp(&right.name),
            "category" => left.category.cmp(&right.category),
            "deployTime" => left.deployed_at.cmp(&right.deployed_at),
            "parentDeploymentId" => left.parent_deployment_id.cmp(&right.parent_deployment_id),
            "tenantId" => left.tenant_id.cmp(&right.tenant_id),
            _ => left.id.cmp(&right.id),
        };
        if order == Some("desc") {
            ordering.reverse()
        } else {
            ordering
        }
    });
}

fn validate_decision_table_query(query: &DecisionTableQueryParams) -> Result<(), ApiError> {
    match query.sort.as_deref() {
        None | Some("id") | Some("key") | Some("category") | Some("name") | Some("version")
        | Some("deploymentId") | Some("tenantId") | Some("decisionType") => {}
        Some(sort) => {
            return Err(ApiError::bad_request(format!(
                "Unsupported sort property '{sort}' for DMN decisions"
            )));
        }
    }

    match query.order.as_deref() {
        None | Some("asc") | Some("desc") => Ok(()),
        Some(order) => Err(ApiError::bad_request(format!(
            "Unsupported order '{order}' for DMN decisions"
        ))),
    }
}

fn validate_decision_service_query(query: &DecisionServiceQueryParams) -> Result<(), ApiError> {
    match query.sort.as_deref() {
        None | Some("id") | Some("key") | Some("name") | Some("deploymentId")
        | Some("tenantId") | Some("resourceName") => {}
        Some(sort) => {
            return Err(ApiError::bad_request(format!(
                "Unsupported sort property '{sort}' for DMN decision services"
            )));
        }
    }

    match query.order.as_deref() {
        None | Some("asc") | Some("desc") => Ok(()),
        Some(order) => Err(ApiError::bad_request(format!(
            "Unsupported order '{order}' for DMN decision services"
        ))),
    }
}

pub async fn execute_decision(
    Extension(runtime): Extension<DynDmnRuntime>,
    body: String,
) -> Result<impl IntoResponse, ApiError> {
    let payload: DecisionExecutionRequest =
        serde_json::from_str(&body).map_err(|err| ApiError::bad_request(err.to_string()))?;
    let execution = runtime.execute_decision(payload.into())?;
    Ok((StatusCode::CREATED, Json(execution)))
}

/// Java `DmnRuleServiceResource#executeWithSingleResult` /
/// `#executeDecisionWithSingleResult` — multi-row → HTTP 500.
pub async fn execute_decision_single_result(
    Extension(runtime): Extension<DynDmnRuntime>,
    body: String,
) -> Result<impl IntoResponse, ApiError> {
    let payload: DecisionExecutionRequest =
        serde_json::from_str(&body).map_err(|err| ApiError::bad_request(err.to_string()))?;
    let execution = runtime.execute_decision(payload.into())?;
    let single = enforce_single_result(execution, false)?;
    Ok((StatusCode::CREATED, Json(single)))
}

/// Java `DmnRuleServiceResource#executeDecisionServiceWithSingleResult` —
/// multi-row for a child decision → "more than one result in decision: <key>".
pub async fn execute_decision_service_single_result(
    Extension(runtime): Extension<DynDmnRuntime>,
    body: String,
) -> Result<impl IntoResponse, ApiError> {
    let payload: DecisionExecutionRequest =
        serde_json::from_str(&body).map_err(|err| ApiError::bad_request(err.to_string()))?;
    let execution = runtime.execute_decision(payload.into())?;
    let single = enforce_single_result(execution, true)?;
    Ok((StatusCode::CREATED, Json(single)))
}

/// Java `DmnDecisionServiceImpl.java:118-121,139-146`.
///
/// Note: `DmnError::Execution` maps to 400 in `error.rs`; multi-result is a
/// Java `FlowableException` (HTTP 500), so we construct `InternalServerError`
/// directly at the REST layer rather than via `DmnError`.
fn enforce_single_result(
    execution: DecisionExecutionRecord,
    decision_service: bool,
) -> Result<DecisionExecutionSingleRecord, ApiError> {
    if execution.result_variables.len() > 1 {
        // Java DmnDecisionServiceImpl.java:119-120 / 142-143
        let message = if decision_service {
            format!(
                "more than one result in decision: {}",
                execution.decision_key
            )
        } else {
            "more than one result".to_string()
        };
        return Err(ApiError::InternalServerError(message));
    }

    // 0 results → empty list; 1 result → that row's variables, one level up.
    // Java DmnRestResponseFactory.java:146-158 builds the single response by
    // adding each variable of the one result map directly to the response.
    let result_variables = execution
        .result_variables
        .into_iter()
        .next()
        .unwrap_or_default();

    Ok(DecisionExecutionSingleRecord {
        id: execution.id,
        decision_table_id: execution.decision_table_id,
        deployment_id: execution.deployment_id,
        decision_key: execution.decision_key,
        tenant_id: execution.tenant_id,
        business_key: execution.business_key,
        hit_policy: execution.hit_policy,
        executed_at: execution.executed_at,
        rule_hit_count: execution.rule_hit_count,
        input_variables: execution.input_variables,
        result_variables,
        multiple_results: false,
        rule_executions: execution.rule_executions,
    })
}

pub async fn list_historic_decision_executions(
    Extension(history): Extension<DynDmnHistory>,
    uri: Uri,
) -> Result<Json<PagedResponse<HistoricDecisionExecutionRecord>>, ApiError> {
    let query: HistoricDecisionExecutionQueryParams = parse_query(&uri)?;
    historic_decision_execution_response(history, query.into())
}

pub async fn query_historic_decision_executions(
    Extension(history): Extension<DynDmnHistory>,
    uri: Uri,
    body: String,
) -> Result<Json<PagedResponse<HistoricDecisionExecutionRecord>>, ApiError> {
    let mut query: HistoricDecisionExecutionQueryParams =
        serde_json::from_str(&body).map_err(|err| ApiError::bad_request(err.to_string()))?;
    let url_query: HistoricDecisionExecutionQueryParams = parse_query(&uri)?;
    query.start = url_query.start;
    query.size = url_query.size.or(query.size);
    query.sort = url_query.sort.or(query.sort);
    query.order = url_query.order.or(query.order);
    historic_decision_execution_response(history, query.into())
}

fn historic_decision_execution_response(
    history: DynDmnHistory,
    query: HistoricDecisionExecutionQuery,
) -> Result<Json<PagedResponse<HistoricDecisionExecutionRecord>>, ApiError> {
    validate_historic_decision_execution_query(&query)?;
    Ok(Json(history.list_historic_decision_executions(query)?))
}

fn validate_historic_decision_execution_query(
    query: &HistoricDecisionExecutionQuery,
) -> Result<(), ApiError> {
    match query.sort.as_deref() {
        None
        | Some("id")
        | Some("decisionExecutionId")
        | Some("decisionKey")
        | Some("decisionTableId")
        | Some("decisionDefinitionId")
        | Some("deploymentId")
        | Some("businessKey")
        | Some("tenantId")
        | Some("startTime")
        | Some("executionTime")
        | Some("executedAt") => {}
        Some(sort) => {
            return Err(ApiError::bad_request(format!(
                "Unsupported historic decision execution sort field '{sort}'"
            )));
        }
    }

    match query.order.as_deref() {
        None | Some("asc") | Some("desc") => Ok(()),
        Some(order) => Err(ApiError::bad_request(format!(
            "Unsupported historic decision execution sort order '{order}'"
        ))),
    }
}

pub async fn get_historic_decision_execution(
    Extension(history): Extension<DynDmnHistory>,
    Path(historic_decision_execution_id): Path<String>,
) -> Result<Json<HistoricDecisionExecutionRecord>, ApiError> {
    Ok(Json(history.get_historic_decision_execution(
        &historic_decision_execution_id,
    )?))
}

pub async fn get_historic_decision_execution_audit_data(
    Extension(history): Extension<DynDmnHistory>,
    Path(historic_decision_execution_id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let execution = history.get_historic_decision_execution(&historic_decision_execution_id)?;
    Ok(Json(serde_json::json!({
        "id": execution.id,
        "decisionTableId": execution.decision_table_id,
        "deploymentId": execution.deployment_id,
        "decisionKey": execution.decision_key,
        "tenantId": execution.tenant_id,
        "businessKey": execution.business_key,
        "executedAt": execution.executed_at,
        "ruleHitCount": execution.rule_hit_count,
        "inputVariables": execution.input_variables,
        "resultVariables": execution.result_variables,
        "ruleExecutions": execution.rule_executions,
    })))
}

pub async fn get_engine_info() -> Json<EngineInfoRecord> {
    Json(EngineInfoRecord {
        name: "flowable-dmn-engine".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        resource_url: None,
        exception: None,
    })
}

pub async fn list_drds(
    Extension(repository): Extension<DynDmnRepository>,
) -> Result<Json<PagedResponse<Value>>, ApiError> {
    Ok(Json(repository.list_drds()?))
}

pub async fn get_drd(
    Extension(repository): Extension<DynDmnRepository>,
    Path(drd_id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    Ok(Json(repository.get_drd(&drd_id)?))
}

pub async fn get_drd_resource_data(
    Extension(repository): Extension<DynDmnRepository>,
    Path(drd_id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    Ok(resource_data_response(
        repository.get_drd_resource_data(&drd_id)?,
    ))
}

pub async fn delete_historic_decision_execution(
    Extension(history): Extension<DynDmnHistory>,
    Path(historic_decision_execution_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    history.delete_historic_decision_execution(&historic_decision_execution_id)?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BulkDeleteHistoricDecisionExecutionsRequest {
    decision_execution_ids: Vec<String>,
}

pub async fn bulk_delete_historic_decision_executions(
    Extension(history): Extension<DynDmnHistory>,
    body: String,
) -> Result<StatusCode, ApiError> {
    let payload: BulkDeleteHistoricDecisionExecutionsRequest =
        serde_json::from_str(&body).map_err(|err| ApiError::bad_request(err.to_string()))?;

    if payload.decision_execution_ids.is_empty() {
        return Err(ApiError::bad_request(
            "decisionExecutionIds cannot be empty",
        ));
    }

    history.bulk_delete_historic_decision_executions(payload.decision_execution_ids)?;
    Ok(StatusCode::NO_CONTENT)
}
