use crate::common::{PagedResponse, PagingQuery, parse_query, parse_rfc3339_datetime};
use crate::error::ApiError;
use axum::{
    Extension, Json, Router,
    extract::Path,
    http::{HeaderMap, StatusCode, Uri, header},
    response::IntoResponse,
    routing::{get, post, put},
};
use base64::Engine;
use chrono::{SecondsFormat, TimeZone, Utc};
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_form_service::{
    FlowableFormService, FormData as RuntimeFormData, FormInstance, FormOutcome,
    FormSubmissionProperty, FormSubmissionRequest, FormSubmissionResult,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::sync::Arc;

pub type DynFormRepository = Arc<dyn FormRepositoryApi>;

const FORM_DATA_PATH: &str = "/form/form-data";
const FORM_INSTANCES_PATH: &str = "/form/form-instances";
const FORM_INSTANCE_PATH: &str = "/form/form-instances/:form_instance_id";
/// Rust-owned extension (not Java Form REST parity): form instance values bytes.
const FORM_INSTANCE_VALUES_PATH: &str = "/form/form-instances/:form_instance_id/values";
const FORM_DEPLOYMENTS_PATH: &str = "/form-repository/deployments";
const FORM_DEFINITIONS_PATH: &str = "/form-repository/form-definitions";
const FORM_DEFINITION_PATH: &str = "/form-repository/form-definitions/:form_definition_id";
const FORM_DEFINITION_VERSIONS_PATH: &str =
    "/form-repository/form-definitions/:form_definition_id/versions";
const FORM_DEFINITION_LAYOUT_PATH: &str =
    "/form-repository/form-definitions/:form_definition_id/layout";
const FORM_DEFINITION_OUTCOMES_PATH: &str =
    "/form-repository/form-definitions/:form_definition_id/outcomes";
const FORM_DEFINITION_ACTIVATION_PATH: &str =
    "/form-repository/form-definitions/:form_definition_id/activation";

pub trait FormRepositoryApi: Send + Sync {
    fn deploy_form_definitions(
        &self,
        command: FormDeploymentCommand,
    ) -> Result<FormDeploymentRecord, ApiError>;
    fn list_form_definitions(
        &self,
        query: FormDefinitionQuery,
    ) -> Result<PagedResponse<FormDefinitionRecord>, ApiError>;
    fn get_form_definition(
        &self,
        form_definition_id: &str,
    ) -> Result<FormDefinitionRecord, ApiError>;
    /// M41: List all versions of a form definition by key.
    fn list_form_definition_versions(
        &self,
        form_definition_id: &str,
    ) -> Result<Vec<FormDefinitionVersionRecord>, ApiError> {
        let definition = self.get_form_definition(form_definition_id)?;
        let definitions = self.list_form_definitions(FormDefinitionQuery {
            key: Some(definition.key),
            ..FormDefinitionQuery::default()
        })?;
        let mut versions = definitions
            .data
            .into_iter()
            .map(|definition| FormDefinitionVersionRecord {
                id: definition.id,
                key: definition.key,
                name: definition.name,
                version: definition.version,
                deployment_id: definition.deployment_id,
                resource_name: definition.resource_name,
                tenant_id: definition.tenant_id,
                active: definition.active,
            })
            .collect::<Vec<_>>();
        versions.sort_by(|left, right| {
            right
                .version
                .cmp(&left.version)
                .then(left.id.cmp(&right.id))
        });
        Ok(versions)
    }
    /// M41: Get the layout of a form definition.
    fn get_form_definition_layout(&self, form_definition_id: &str) -> Result<Value, ApiError> {
        let _ = form_definition_id;
        Err(ApiError::bad_request(
            "Form definition layout is not supported by the configured form repository",
        ))
    }
    /// M41: Get the outcomes of a form definition.
    fn get_form_definition_outcomes(
        &self,
        form_definition_id: &str,
    ) -> Result<Vec<FormOutcome>, ApiError> {
        let _ = form_definition_id;
        Err(ApiError::bad_request(
            "Form definition outcomes are not supported by the configured form repository",
        ))
    }
    /// M41: Batch delete form definitions by deploymentId or key.
    fn delete_form_definitions(&self, query: FormDeleteQuery) -> Result<usize, ApiError> {
        let _ = query;
        Err(ApiError::bad_request(
            "Batch deletion is not supported by the configured form repository",
        ))
    }
    /// M41: Activate or deactivate a form definition.
    fn set_form_definition_activation(
        &self,
        form_definition_id: &str,
        active: bool,
    ) -> Result<FormDefinitionRecord, ApiError> {
        let _ = (form_definition_id, active);
        Err(ApiError::bad_request(
            "Form definition activation is not supported by the configured form repository",
        ))
    }
}

#[derive(Debug, Clone)]
pub struct FormDeploymentCommand {
    pub name: String,
    pub resources: Vec<FormDeploymentResourcePayload>,
}

#[derive(Debug, Clone)]
pub struct FormDeploymentResourcePayload {
    pub resource_name: String,
    pub resource: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FormDeploymentRecord {
    pub id: String,
    pub name: String,
    pub deployed_at: i64,
    pub resource_names: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct FormDefinitionQuery {
    pub paging: PagingQuery,
    pub id: Option<String>,
    pub key: Option<String>,
    pub name: Option<String>,
    pub deployment_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FormDefinitionRecord {
    pub id: String,
    pub key: String,
    pub name: String,
    pub version: i32,
    pub deployment_id: String,
    pub resource_name: String,
    pub tenant_id: Option<String>,
    pub active: Option<bool>,
}

/// M41: Version record for a form definition.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FormDefinitionVersionRecord {
    pub id: String,
    pub key: String,
    pub name: String,
    pub version: i32,
    pub deployment_id: String,
    pub resource_name: String,
    pub tenant_id: Option<String>,
    pub active: Option<bool>,
}

/// M41: Query for batch delete operations.
#[derive(Debug, Clone, Default)]
pub struct FormDeleteQuery {
    pub deployment_id: Option<String>,
    pub key: Option<String>,
}

/// M41: Activation request body.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ActivationRequest {
    active: bool,
}

/// M41: Query params for batch delete.
#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct FormDeleteQueryParams {
    deployment_id: Option<String>,
    key: Option<String>,
}

impl From<FormDeleteQueryParams> for FormDeleteQuery {
    fn from(value: FormDeleteQueryParams) -> Self {
        Self {
            deployment_id: value.deployment_id,
            key: value.key,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FormDeploymentRequest {
    name: String,
    #[serde(default)]
    resources: Vec<FormDeploymentResourceRequest>,
    resource_name: Option<String>,
    resource: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FormDeploymentResourceRequest {
    resource_name: String,
    resource: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
struct FormDefinitionQueryParams {
    start: usize,
    size: Option<usize>,
    id: Option<String>,
    key: Option<String>,
    key_like: Option<String>,
    name: Option<String>,
    name_like: Option<String>,
    deployment_id: Option<String>,
    version: Option<i32>,
    resource_name: Option<String>,
    resource_name_like: Option<String>,
    latest: Option<bool>,
    sort: Option<String>,
    order: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
struct RuntimeFormQueryParams {
    task_id: Option<String>,
    process_definition_id: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
struct FormInstanceQueryParams {
    start: usize,
    size: Option<usize>,
    form_definition_id: Option<String>,
    form_definition_key: Option<String>,
    process_definition_id: Option<String>,
    process_instance_id: Option<String>,
    task_id: Option<String>,
    submitted_date: Option<String>,
    submitted_date_before: Option<String>,
    submitted_date_after: Option<String>,
    submitted_by: Option<String>,
    submitted_by_like: Option<String>,
    tenant_id: Option<String>,
    tenant_id_like: Option<String>,
    without_tenant_id: Option<bool>,
    sort: Option<String>,
    order: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SubmitFormRequestPayload {
    action: Option<String>,
    process_definition_id: Option<String>,
    task_id: Option<String>,
    business_key: Option<String>,
    outcome: Option<String>,
    #[serde(default)]
    properties: Vec<SubmitFormPropertyPayload>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SubmitFormPropertyPayload {
    id: String,
    value: Value,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FormInstanceResponse {
    pub id: String,
    pub url: String,
    pub form_definition_id: String,
    pub form_definition_key: String,
    pub form_definition_name: String,
    pub deployment_id: String,
    pub task_id: Option<String>,
    pub process_instance_id: Option<String>,
    pub process_definition_id: Option<String>,
    pub submitted_date: String,
    pub submitted_by: Option<String>,
    pub form_values_id: Option<String>,
    pub tenant_id: Option<String>,
}

impl FormDeploymentRequest {
    fn into_command(self) -> Result<FormDeploymentCommand, ApiError> {
        let resources = if self.resources.is_empty() {
            match (self.resource_name, self.resource) {
                (Some(resource_name), Some(resource)) => {
                    vec![FormDeploymentResourcePayload {
                        resource_name,
                        resource,
                    }]
                }
                _ => {
                    return Err(ApiError::bad_request(
                        "Form deployment requires at least one resource",
                    ));
                }
            }
        } else {
            self.resources
                .into_iter()
                .map(|resource| FormDeploymentResourcePayload {
                    resource_name: resource.resource_name,
                    resource: resource.resource,
                })
                .collect()
        };

        Ok(FormDeploymentCommand {
            name: self.name,
            resources,
        })
    }
}

impl From<FormDefinitionQueryParams> for FormDefinitionQuery {
    fn from(value: FormDefinitionQueryParams) -> Self {
        Self {
            paging: PagingQuery {
                start: value.start,
                size: value.size,
            },
            id: value.id,
            key: value.key,
            name: value.name,
            deployment_id: value.deployment_id,
        }
    }
}

impl FormDefinitionQueryParams {
    fn requested_paging(&self) -> PagingQuery {
        PagingQuery {
            start: self.start,
            size: self.size,
        }
    }

    fn repository_query(&self) -> FormDefinitionQuery {
        FormDefinitionQuery {
            paging: if self.requires_route_filtering() {
                PagingQuery::default()
            } else {
                self.requested_paging()
            },
            id: self.id.clone(),
            key: self.key.clone(),
            name: self.name.clone(),
            deployment_id: self.deployment_id.clone(),
        }
    }

    fn requires_route_filtering(&self) -> bool {
        self.version.is_some()
            || self.key_like.is_some()
            || self.name_like.is_some()
            || self.resource_name.is_some()
            || self.resource_name_like.is_some()
            || self.latest.unwrap_or(false)
            || self.sort.is_some()
            || self.order.is_some()
    }

    fn validate(&self) -> Result<(), ApiError> {
        validate_form_definition_sort(self.sort.as_deref())?;
        let _ = descending_order(self.order.as_deref())?;
        if self.latest.unwrap_or(false)
            && (self.id.is_some()
                || self.name.is_some()
                || self.name_like.is_some()
                || self.deployment_id.is_some()
                || self.version.is_some()
                || self.resource_name.is_some()
                || self.resource_name_like.is_some())
        {
            return Err(ApiError::bad_request(
                "The latest parameter can only be used without other filters or together with key/keyLike",
            ));
        }
        Ok(())
    }

    fn apply_route_filters(
        &self,
        mut definitions: Vec<FormDefinitionRecord>,
    ) -> Vec<FormDefinitionRecord> {
        if let Some(key_like) = self.key_like.as_deref() {
            definitions.retain(|definition| sql_like_matches(&definition.key, key_like));
        }
        if let Some(name_like) = self.name_like.as_deref() {
            definitions.retain(|definition| sql_like_matches(&definition.name, name_like));
        }
        if let Some(version) = self.version {
            definitions.retain(|definition| definition.version == version);
        }
        if let Some(resource_name) = self.resource_name.as_deref() {
            definitions.retain(|definition| definition.resource_name == resource_name);
        }
        if let Some(resource_name_like) = self.resource_name_like.as_deref() {
            definitions.retain(|definition| {
                sql_like_matches(&definition.resource_name, resource_name_like)
            });
        }
        if self.latest.unwrap_or(false) {
            let mut latest_by_key = BTreeMap::<String, FormDefinitionRecord>::new();
            for definition in definitions {
                latest_by_key
                    .entry(definition.key.clone())
                    .and_modify(|current| {
                        if definition.version > current.version
                            || (definition.version == current.version && definition.id < current.id)
                        {
                            *current = definition.clone();
                        }
                    })
                    .or_insert(definition);
            }
            definitions = latest_by_key.into_values().collect();
        }
        definitions.sort_by(|left, right| {
            let ordering = match self.sort.as_deref().unwrap_or("key") {
                "id" => left.id.cmp(&right.id),
                "name" => left.name.cmp(&right.name),
                "deploymentId" => left.deployment_id.cmp(&right.deployment_id),
                "version" => left.version.cmp(&right.version),
                "resourceName" => left.resource_name.cmp(&right.resource_name),
                _ => left.key.cmp(&right.key),
            }
            .then(right.version.cmp(&left.version))
            .then(left.id.cmp(&right.id));
            if descending_order(self.order.as_deref()).unwrap_or(false) {
                ordering.reverse()
            } else {
                ordering
            }
        });
        definitions
    }
}

impl FormInstanceQueryParams {
    fn requested_paging(&self) -> PagingQuery {
        PagingQuery {
            start: self.start,
            size: self.size,
        }
    }

    fn validate(&self) -> Result<(), ApiError> {
        match self.sort.as_deref().unwrap_or("submittedDate") {
            "submittedDate"
            | "submittedBy"
            | "formDefinitionId"
            | "formDefinitionKey"
            | "processDefinitionId"
            | "processInstanceId"
            | "taskId"
            | "tenantId" => {}
            sort => {
                return Err(ApiError::bad_request(format!(
                    "Unsupported form instance sort '{sort}'"
                )));
            }
        }
        let _ = descending_order(self.order.as_deref())?;
        if (self.tenant_id.is_some() || self.tenant_id_like.is_some())
            && self.without_tenant_id.unwrap_or(false)
        {
            return Err(ApiError::bad_request(
                "Only one of tenantId/tenantIdLike or withoutTenantId can be provided",
            ));
        }
        Ok(())
    }

    fn submitted_date_millis(&self) -> Result<Option<i64>, ApiError> {
        self.submitted_date
            .as_deref()
            .map(|value| {
                parse_rfc3339_datetime(value, "submittedDate").map(|date| date.timestamp_millis())
            })
            .transpose()
    }

    fn submitted_date_before_millis(&self) -> Result<Option<i64>, ApiError> {
        self.submitted_date_before
            .as_deref()
            .map(|value| {
                parse_rfc3339_datetime(value, "submittedDateBefore")
                    .map(|date| date.timestamp_millis())
            })
            .transpose()
    }

    fn submitted_date_after_millis(&self) -> Result<Option<i64>, ApiError> {
        self.submitted_date_after
            .as_deref()
            .map(|value| {
                parse_rfc3339_datetime(value, "submittedDateAfter")
                    .map(|date| date.timestamp_millis())
            })
            .transpose()
    }
}

fn validate_form_definition_sort(sort: Option<&str>) -> Result<(), ApiError> {
    match sort {
        None | Some("id" | "key" | "name" | "deploymentId" | "version" | "resourceName") => Ok(()),
        Some(sort) => Err(ApiError::bad_request(format!(
            "Unsupported form definition sort '{sort}'"
        ))),
    }
}

fn descending_order(order: Option<&str>) -> Result<bool, ApiError> {
    match order.unwrap_or("asc") {
        order if order.eq_ignore_ascii_case("asc") => Ok(false),
        order if order.eq_ignore_ascii_case("desc") => Ok(true),
        order => Err(ApiError::bad_request(format!(
            "Unsupported form definition order '{order}'"
        ))),
    }
}

/// Max Unicode scalar count for in-memory SQL-LIKE filter operands (tests pin
/// the shared 512 bound from `flowable_engine_common::like`).
#[cfg(test)]
const MAX_SQL_LIKE_LEN: usize = flowable_engine_common::like::MAX_SQL_LIKE_LEN;

/// SQL-LIKE style match for in-memory filters (`%` any sequence, `_` one char,
/// other chars literal). Case-sensitive; callers lower-case both sides for
/// ignore-case variants.
///
/// Local signature is `(value, pattern)`; shared impl is `(pattern, value)`.
/// Space is O(value length) via two rolling rows (not O(n×m) full DP matrix).
fn sql_like_matches(value: &str, pattern: &str) -> bool {
    // Delegates to flowable_engine_common::like::sql_like_matches (P143 unified LIKE, O(m)+512 cap).
    flowable_engine_common::like::sql_like_matches(pattern, value)
}

pub fn router(repository: DynFormRepository) -> Router {
    router_with_prefix("", repository)
}

fn router_with_prefix(prefix: &str, repository: DynFormRepository) -> Router {
    Router::new()
        .route(
            &format!("{prefix}{FORM_DATA_PATH}"),
            get(get_form_data).post(submit_form),
        )
        .route(
            &format!("{prefix}{FORM_INSTANCES_PATH}"),
            get(list_form_instances),
        )
        .route(
            &format!("{prefix}{FORM_INSTANCE_PATH}"),
            get(get_form_instance).delete(delete_form_instance),
        )
        .route(
            &format!("{prefix}{FORM_INSTANCE_VALUES_PATH}"),
            get(get_form_instance_values),
        )
        .route(&format!("{prefix}{FORM_DEPLOYMENTS_PATH}"), post(deploy))
        .route(
            &format!("{prefix}{FORM_DEFINITIONS_PATH}"),
            get(list_form_definitions).delete(delete_form_definitions),
        )
        .route(
            &format!("{prefix}{FORM_DEFINITION_PATH}"),
            get(get_form_definition),
        )
        .route(
            &format!("{prefix}{FORM_DEFINITION_VERSIONS_PATH}"),
            get(list_form_definition_versions),
        )
        .route(
            &format!("{prefix}{FORM_DEFINITION_LAYOUT_PATH}"),
            get(get_form_definition_layout),
        )
        .route(
            &format!("{prefix}{FORM_DEFINITION_OUTCOMES_PATH}"),
            get(get_form_definition_outcomes),
        )
        .route(
            &format!("{prefix}{FORM_DEFINITION_ACTIVATION_PATH}"),
            put(set_form_definition_activation),
        )
        .layer(Extension(repository))
}

pub async fn deploy(
    Extension(repository): Extension<DynFormRepository>,
    body: String,
) -> Result<impl IntoResponse, ApiError> {
    let payload: FormDeploymentRequest =
        serde_json::from_str(&body).map_err(|err| ApiError::bad_request(err.to_string()))?;
    let deployment = repository.deploy_form_definitions(payload.into_command()?)?;
    Ok((StatusCode::CREATED, Json(deployment)))
}

pub async fn list_form_definitions(
    Extension(repository): Extension<DynFormRepository>,
    uri: Uri,
) -> Result<Json<PagedResponse<FormDefinitionRecord>>, ApiError> {
    let query: FormDefinitionQueryParams = parse_query(&uri)?;
    query.validate()?;
    let requested_paging = query.requested_paging();
    let page = repository.list_form_definitions(query.repository_query())?;
    if !query.requires_route_filtering() {
        return Ok(Json(page));
    }
    Ok(Json(
        requested_paging.paginate(query.apply_route_filters(page.data)),
    ))
}

pub async fn get_form_definition(
    Extension(repository): Extension<DynFormRepository>,
    Path(form_definition_id): Path<String>,
) -> Result<Json<FormDefinitionRecord>, ApiError> {
    Ok(Json(repository.get_form_definition(&form_definition_id)?))
}

pub async fn get_form_data(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    uri: Uri,
) -> Result<Json<RuntimeFormData>, ApiError> {
    let query: RuntimeFormQueryParams = parse_query(&uri)?;
    let service = FlowableFormService::new(engine);

    match (
        query.process_definition_id.as_deref(),
        query.task_id.as_deref(),
    ) {
        (Some(process_definition_id), None) => {
            Ok(Json(service.get_start_form_data(process_definition_id)?))
        }
        (None, Some(task_id)) => Ok(Json(service.get_task_form_data(task_id)?)),
        (Some(_), Some(_)) => Err(ApiError::bad_request(
            "Only one of processDefinitionId or taskId can be provided",
        )),
        (None, None) => Err(ApiError::bad_request(
            "processDefinitionId or taskId is required",
        )),
    }
}

pub async fn submit_form(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    headers: HeaderMap,
    body: String,
) -> Result<impl IntoResponse, ApiError> {
    let payload: SubmitFormRequestPayload =
        serde_json::from_str(&body).map_err(|error| ApiError::bad_request(error.to_string()))?;
    if let Some(action) = payload.action.as_deref()
        && !action.eq_ignore_ascii_case("submit")
    {
        return Err(ApiError::bad_request(format!(
            "Unsupported form action '{}'",
            action
        )));
    }

    let service = FlowableFormService::new(engine);
    let request = FormSubmissionRequest {
        process_definition_id: payload.process_definition_id,
        task_id: payload.task_id,
        business_key: payload.business_key,
        outcome: payload.outcome,
        properties: payload
            .properties
            .into_iter()
            .map(|property| FormSubmissionProperty {
                id: property.id,
                value: property.value,
            })
            .collect(),
    };
    let submitted_by = submitted_by_from_basic_auth(&headers);
    let result = match submitted_by {
        Some(user_id) => service.submit_form_as(request, user_id),
        None => service.submit_form(request),
    }?;

    match result {
        FormSubmissionResult::ProcessInstance(process_instance) => Ok((
            StatusCode::OK,
            Json(super::process_instances::to_process_instance_response(
                process_instance,
            )),
        )
            .into_response()),
        FormSubmissionResult::TaskCompleted(_) => Ok(StatusCode::NO_CONTENT.into_response()),
    }
}

pub async fn list_form_instances(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    uri: Uri,
) -> Result<Json<PagedResponse<FormInstanceResponse>>, ApiError> {
    let query: FormInstanceQueryParams = parse_query(&uri)?;
    query.validate()?;

    let mut instance_query =
        FlowableFormService::new(Arc::clone(&engine)).create_form_instance_query();
    if let Some(form_definition_id) = query.form_definition_id.clone() {
        instance_query = instance_query.form_definition_id(form_definition_id);
    }
    if let Some(form_definition_key) = query.form_definition_key.clone() {
        instance_query = instance_query.form_definition_key(form_definition_key);
    }
    if let Some(process_definition_id) = query.process_definition_id.clone() {
        instance_query = instance_query.process_definition_id(process_definition_id);
    }
    if let Some(process_instance_id) = query.process_instance_id.clone() {
        instance_query = instance_query.process_instance_id(process_instance_id);
    }
    if let Some(task_id) = query.task_id.clone() {
        instance_query = instance_query.task_id(task_id);
    }
    if let Some(submitted_date) = query.submitted_date_millis()? {
        instance_query = instance_query.submitted_date(submitted_date);
    }
    if let Some(submitted_date_before) = query.submitted_date_before_millis()? {
        instance_query = instance_query.submitted_date_before(submitted_date_before);
    }
    if let Some(submitted_date_after) = query.submitted_date_after_millis()? {
        instance_query = instance_query.submitted_date_after(submitted_date_after);
    }
    if let Some(submitted_by) = query.submitted_by.clone() {
        instance_query = instance_query.submitted_by(submitted_by);
    }
    if let Some(submitted_by_like) = query.submitted_by_like.clone() {
        instance_query = instance_query.submitted_by_like(submitted_by_like);
    }

    // Push tenant filters into the engine query so total/page boundaries stay correct
    // (no post-page filtering).
    if query.without_tenant_id.unwrap_or(false) {
        instance_query = instance_query.without_tenant_id();
    } else {
        if let Some(tenant_id) = query.tenant_id.clone() {
            instance_query = instance_query.tenant_id(tenant_id);
        }
        if let Some(tenant_id_like) = query.tenant_id_like.clone() {
            instance_query = instance_query.tenant_id_like(tenant_id_like);
        }
    }

    match query.sort.as_deref().unwrap_or("submittedDate") {
        "tenantId" => {
            instance_query = instance_query.order_by_tenant_id();
        }
        _ => {
            instance_query = instance_query.order_by_submitted_date();
        }
    }
    if descending_order(query.order.as_deref())? {
        instance_query = instance_query.desc();
    } else {
        instance_query = instance_query.asc();
    }

    // Engine-level paging when sort is fully handled by FormInstanceQuery.
    // REST still re-sorts for secondary keys (submittedBy, formDefinitionId, …).
    let mut instances = instance_query.list()?;
    sort_form_instances(&query, &mut instances)?;

    Ok(Json(query.requested_paging().paginate(
        instances.into_iter().map(form_instance_response).collect(),
    )))
}

pub async fn get_form_instance(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(form_instance_id): Path<String>,
) -> Result<Json<FormInstanceResponse>, ApiError> {
    let instance =
        FlowableFormService::new(Arc::clone(&engine)).get_form_instance(&form_instance_id)?;
    Ok(Json(form_instance_response(instance)))
}

/// Rust-owned Form REST extension: return form instance value bytes.
/// Java FormService.getFormInstanceValues is an engine API; no stable Java Form
/// REST path exists in this workspace's Java truth sources.
pub async fn get_form_instance_values(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(form_instance_id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let bytes =
        FlowableFormService::new(Arc::clone(&engine)).get_form_instance_values(&form_instance_id)?;
    Ok((
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json")],
        bytes,
    ))
}

/// Rust-owned Form REST extension: delete a form instance by id.
/// Java FormService.deleteFormInstance is an engine API; labeled as owned extension.
pub async fn delete_form_instance(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(form_instance_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    FlowableFormService::new(Arc::clone(&engine)).delete_form_instance(&form_instance_id)?;
    Ok(StatusCode::NO_CONTENT)
}

fn sort_form_instances(
    query: &FormInstanceQueryParams,
    instances: &mut [FormInstance],
) -> Result<(), ApiError> {
    let descending = descending_order(query.order.as_deref())?;
    instances.sort_by(|left, right| {
        let ordering = match query.sort.as_deref().unwrap_or("submittedDate") {
            "submittedBy" => left.submitted_by.cmp(&right.submitted_by),
            "formDefinitionId" => left.form_definition_id.cmp(&right.form_definition_id),
            "formDefinitionKey" => left.form_definition_key.cmp(&right.form_definition_key),
            "processDefinitionId" => left.process_definition_id.cmp(&right.process_definition_id),
            "processInstanceId" => left.process_instance_id.cmp(&right.process_instance_id),
            "taskId" => left.task_id.cmp(&right.task_id),
            "tenantId" => left.tenant_id.cmp(&right.tenant_id),
            _ => left.submitted_at.cmp(&right.submitted_at),
        }
        .then(left.id.cmp(&right.id));

        if descending {
            ordering.reverse()
        } else {
            ordering
        }
    });
    Ok(())
}

fn form_instance_response(instance: FormInstance) -> FormInstanceResponse {
    let url = format!("/form/form-instances/{}", instance.id);
    FormInstanceResponse {
        id: instance.id,
        url,
        form_definition_id: instance.form_definition_id,
        form_definition_key: instance.form_definition_key,
        form_definition_name: instance.form_definition_name,
        deployment_id: instance.deployment_id,
        task_id: instance.task_id,
        process_instance_id: instance.process_instance_id,
        process_definition_id: instance.process_definition_id,
        submitted_date: format_millis(instance.submitted_at),
        submitted_by: instance.submitted_by,
        form_values_id: instance.form_values_id,
        tenant_id: instance.tenant_id,
    }
}

fn submitted_by_from_basic_auth(headers: &HeaderMap) -> Option<String> {
    let auth_header = headers
        .get(header::AUTHORIZATION)
        .and_then(|header| header.to_str().ok())?;
    let encoded = auth_header.strip_prefix("Basic ")?;
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .ok()?;
    let decoded = String::from_utf8(decoded).ok()?;
    let user_id = decoded.split_once(':')?.0.trim();
    if user_id.is_empty() {
        None
    } else {
        Some(user_id.to_string())
    }
}

fn format_millis(value: i64) -> String {
    Utc.timestamp_millis_opt(value)
        .single()
        .map(|timestamp| timestamp.to_rfc3339_opts(SecondsFormat::Millis, true))
        .unwrap_or_else(|| value.to_string())
}

// ── M41: Breadth endpoints ────────────────────────────────────────────

/// GET /form-repository/form-definitions/{id}/versions
pub async fn list_form_definition_versions(
    Extension(repository): Extension<DynFormRepository>,
    Path(form_definition_id): Path<String>,
) -> Result<Json<Vec<FormDefinitionVersionRecord>>, ApiError> {
    Ok(Json(
        repository.list_form_definition_versions(&form_definition_id)?,
    ))
}

/// GET /form-repository/form-definitions/{id}/layout
pub async fn get_form_definition_layout(
    Extension(repository): Extension<DynFormRepository>,
    Path(form_definition_id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    Ok(Json(
        repository.get_form_definition_layout(&form_definition_id)?,
    ))
}

/// GET /form-repository/form-definitions/{id}/outcomes
pub async fn get_form_definition_outcomes(
    Extension(repository): Extension<DynFormRepository>,
    Path(form_definition_id): Path<String>,
) -> Result<Json<Vec<FormOutcome>>, ApiError> {
    Ok(Json(
        repository.get_form_definition_outcomes(&form_definition_id)?,
    ))
}

/// DELETE /form-repository/form-definitions?deploymentId=X or ?key=X
pub async fn delete_form_definitions(
    Extension(repository): Extension<DynFormRepository>,
    uri: Uri,
) -> Result<Json<Value>, ApiError> {
    let query: FormDeleteQueryParams = parse_query(&uri)?;
    if query.deployment_id.is_some() && query.key.is_some() {
        return Err(ApiError::bad_request(
            "Only one of deploymentId or key can be provided",
        ));
    }
    let deleted = repository.delete_form_definitions(query.into())?;
    Ok(Json(json!({"deleted": deleted})))
}

/// PUT /form-repository/form-definitions/{id}/activation
pub async fn set_form_definition_activation(
    Extension(repository): Extension<DynFormRepository>,
    Path(form_definition_id): Path<String>,
    body: String,
) -> Result<Json<FormDefinitionRecord>, ApiError> {
    let payload: ActivationRequest =
        serde_json::from_str(&body).map_err(|err| ApiError::bad_request(err.to_string()))?;
    Ok(Json(repository.set_form_definition_activation(
        &form_definition_id,
        payload.active,
    )?))
}

#[cfg(test)]
mod tests {
    use super::{MAX_SQL_LIKE_LEN, sql_like_matches};

    /// Semantic pin tests for in-memory SQL-LIKE (`%` / `_` / literal).
    /// Argument order in forms.rs is `(value, pattern)`.
    #[test]
    fn sql_like_semantic_pins() {
        // empty
        assert!(sql_like_matches("", ""));
        assert!(!sql_like_matches("a", ""));
        assert!(!sql_like_matches("", "a"));
        assert!(sql_like_matches("", "%"));
        assert!(sql_like_matches("", "%%"));
        assert!(!sql_like_matches("", "_"));

        // literal
        assert!(sql_like_matches("abc", "abc"));
        assert!(!sql_like_matches("abc", "ab"));
        assert!(!sql_like_matches("ab", "abc"));
        assert!(!sql_like_matches("abc", "Abc")); // case-sensitive
        assert!(!sql_like_matches("abc", "abd"));

        // `%` any sequence
        assert!(sql_like_matches("hello", "%"));
        assert!(sql_like_matches("hello", "h%"));
        assert!(sql_like_matches("hello", "%o"));
        assert!(sql_like_matches("hello", "%ell%"));
        assert!(sql_like_matches("hello", "h%o"));
        assert!(sql_like_matches("hello", "%%"));
        assert!(sql_like_matches("hello", "%h%e%l%o%"));
        assert!(!sql_like_matches("hello", "x%"));
        assert!(!sql_like_matches("hello", "%x"));

        // `_` single character
        assert!(sql_like_matches("a", "_"));
        assert!(sql_like_matches("ab", "a_"));
        assert!(sql_like_matches("ab", "_b"));
        assert!(sql_like_matches("abc", "a_c"));
        assert!(!sql_like_matches("ab", "_"));
        assert!(!sql_like_matches("a", "__"));
        assert!(!sql_like_matches("", "_"));

        // mixed + pattern longer than value
        assert!(sql_like_matches("ab", "%_%"));
        assert!(sql_like_matches("x", "%%_%%"));
        assert!(!sql_like_matches("ab", "a_c"));
        assert!(!sql_like_matches("a", "a_"));
        assert!(!sql_like_matches("ab", "abc%"));

        // Unicode is one char for `_`
        assert!(sql_like_matches("你", "_"));
        assert!(sql_like_matches("你好", "你_"));
        assert!(!sql_like_matches("你好", "_"));
    }

    #[test]
    fn sql_like_rejects_oversized_without_huge_allocation() {
        let long_value = "v".repeat(MAX_SQL_LIKE_LEN + 1);
        let long_pattern = "%".repeat(MAX_SQL_LIKE_LEN + 1);
        // Must not allocate O(n×m) matrix (~huge); oversize is non-matching.
        assert!(!sql_like_matches(&long_value, &long_pattern));
        assert!(!sql_like_matches(&long_value, "%"));
        assert!(!sql_like_matches("ok", &long_pattern));
        // Boundary at the cap still works.
        let at_cap_v = "a".repeat(MAX_SQL_LIKE_LEN);
        let at_cap_p = "%".repeat(MAX_SQL_LIKE_LEN);
        assert!(sql_like_matches(&at_cap_v, &at_cap_p));
        assert!(sql_like_matches(&at_cap_v, "%"));
    }
}
