use crate::common::{PagedResponse, PagingQuery, parse_query};
use crate::error::ApiError;
use crate::routes::{
    dmn::DecisionTableRecord, forms::FormDefinitionRecord, identity_links::RestIdentityLinkResponse,
};
use axum::{
    Extension, Json, Router,
    extract::Path,
    http::{StatusCode, Uri, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use flowable_bpmn_model::model::{BpmnModel, FlowElementEnum};
use flowable_dmn_engine::DmnEngine;
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::engine::query::Query;
use flowable_engine::identity::entities::IdentityLink;
use flowable_engine::identity::entities::{BatchEntity, BatchPartEntity};
use flowable_engine::repository::process_definition::ProcessDefinition;
use flowable_form_service::FlowableFormService;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::sync::Arc;

const PROCESS_DEFINITIONS_PATH: &str = "/repository/process-definitions";
const PROCESS_DEFINITION_PATH: &str = "/repository/process-definitions/:process_definition_id";
const PROCESS_DEFINITION_START_FORM_PATH: &str =
    "/repository/process-definitions/:process_definition_id/start-form";
const PROCESS_DEFINITION_FORM_DEFINITIONS_PATH: &str =
    "/repository/process-definitions/:process_definition_id/form-definitions";
const PROCESS_DEFINITION_DECISION_TABLES_PATH: &str =
    "/repository/process-definitions/:process_definition_id/decision-tables";
const PROCESS_DEFINITION_DECISIONS_PATH: &str =
    "/repository/process-definitions/:process_definition_id/decisions";
const PROCESS_DEFINITION_RESOURCE_DATA_PATH: &str =
    "/repository/process-definitions/:process_definition_id/resourcedata";
const PROCESS_DEFINITION_MODEL_PATH: &str =
    "/repository/process-definitions/:process_definition_id/model";
const PROCESS_DEFINITION_MIGRATE_PATH: &str =
    "/repository/process-definitions/:process_definition_id/migrate";
const PROCESS_DEFINITION_BATCH_MIGRATE_PATH: &str =
    "/repository/process-definitions/:process_definition_id/batch-migrate";
const PROCESS_DEFINITION_IDENTITY_LINKS_PATH: &str =
    "/repository/process-definitions/:process_definition_id/identitylinks";
const PROCESS_DEFINITION_IDENTITY_LINK_PATH: &str =
    "/repository/process-definitions/:process_definition_id/identitylinks/:family/:identity_id";

pub fn router() -> Router {
    router_with_prefix("")
}

fn router_with_prefix(prefix: &str) -> Router {
    Router::new()
        .route(
            &format!("{prefix}{PROCESS_DEFINITIONS_PATH}"),
            get(list_process_definitions),
        )
        .route(
            &format!("{prefix}{PROCESS_DEFINITION_PATH}"),
            get(get_process_definition).put(update_process_definition),
        )
        .route(
            &format!("{prefix}{PROCESS_DEFINITION_START_FORM_PATH}"),
            get(get_start_form),
        )
        .route(
            &format!("{prefix}{PROCESS_DEFINITION_FORM_DEFINITIONS_PATH}"),
            get(list_form_definitions_for_process_definition),
        )
        .route(
            &format!("{prefix}{PROCESS_DEFINITION_DECISION_TABLES_PATH}"),
            get(list_decisions_for_process_definition),
        )
        .route(
            &format!("{prefix}{PROCESS_DEFINITION_DECISIONS_PATH}"),
            get(list_decisions_for_process_definition),
        )
        .route(
            &format!("{prefix}{PROCESS_DEFINITION_RESOURCE_DATA_PATH}"),
            get(get_process_definition_resource_data),
        )
        .route(
            &format!("{prefix}{PROCESS_DEFINITION_MODEL_PATH}"),
            get(get_process_definition_model),
        )
        .route(
            &format!("{prefix}{PROCESS_DEFINITION_MIGRATE_PATH}"),
            post(migrate_process_definition_instances),
        )
        .route(
            &format!("{prefix}{PROCESS_DEFINITION_BATCH_MIGRATE_PATH}"),
            post(batch_migrate_process_definition_instances),
        )
        .route(
            &format!("{prefix}{PROCESS_DEFINITION_IDENTITY_LINKS_PATH}"),
            get(list_process_definition_identity_links)
                .post(create_process_definition_identity_link),
        )
        .route(
            &format!("{prefix}{PROCESS_DEFINITION_IDENTITY_LINK_PATH}"),
            get(get_process_definition_identity_link)
                .delete(delete_process_definition_identity_link),
        )
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct ProcessDefinitionListQuery {
    start: usize,
    size: Option<usize>,
    key: Option<String>,
    #[serde(rename = "keyLike")]
    key_like: Option<String>,
    name: Option<String>,
    #[serde(rename = "nameLike")]
    name_like: Option<String>,
    #[serde(rename = "nameLikeIgnoreCase")]
    name_like_ignore_case: Option<String>,
    category: Option<String>,
    #[serde(rename = "categoryLike")]
    category_like: Option<String>,
    #[serde(rename = "categoryNotEquals")]
    category_not_equals: Option<String>,
    #[serde(rename = "resourceName")]
    resource_name: Option<String>,
    #[serde(rename = "resourceNameLike")]
    resource_name_like: Option<String>,
    #[serde(rename = "deploymentId")]
    deployment_id: Option<String>,
    #[serde(rename = "parentDeploymentId")]
    parent_deployment_id: Option<String>,
    #[serde(rename = "startableByUser")]
    startable_by_user: Option<String>,
    version: Option<i32>,
    latest: Option<bool>,
    suspended: Option<bool>,
    #[serde(rename = "tenantId")]
    tenant_id: Option<String>,
    #[serde(rename = "tenantIdLike")]
    tenant_id_like: Option<String>,
    #[serde(rename = "withoutTenantId")]
    without_tenant_id: Option<bool>,
    sort: Option<String>,
    order: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct LinkedDefinitionListQuery {
    start: usize,
    size: Option<usize>,
}

impl LinkedDefinitionListQuery {
    fn paging(&self) -> PagingQuery {
        PagingQuery {
            start: self.start,
            size: self.size,
        }
    }
}

impl ProcessDefinitionListQuery {
    fn paging(&self) -> PagingQuery {
        PagingQuery {
            start: self.start,
            size: self.size,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ProcessDefinitionResponse {
    id: String,
    url: String,
    category: Option<String>,
    key: String,
    name: Option<String>,
    description: Option<String>,
    version: i32,
    resource_name: Option<String>,
    deployment_id: Option<String>,
    diagram_resource_name: Option<String>,
    suspended: bool,
    graphical_notation_defined: bool,
    start_form_defined: bool,
    tenant_id: Option<String>,
    engine_version: Option<String>,
    app_version: Option<i32>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct ProcessDefinitionActionRequest {
    action: Option<String>,
    category: Option<Option<String>>,
    include_process_instances: Option<bool>,
    date: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CreateProcessDefinitionIdentityLinkRequest {
    pub user: Option<String>,
    pub group: Option<String>,
    #[serde(rename = "type")]
    pub link_type: Option<String>,
}

fn to_process_definition_response(definition: ProcessDefinition) -> ProcessDefinitionResponse {
    let id = definition.id;
    ProcessDefinitionResponse {
        url: format!("/repository/process-definitions/{id}"),
        id,
        category: definition.category,
        key: definition.key,
        name: definition.name,
        description: definition.description,
        version: definition.version,
        resource_name: definition.resource_name,
        deployment_id: definition.deployment_id,
        diagram_resource_name: definition.diagram_resource_name,
        suspended: definition.is_suspended,
        graphical_notation_defined: definition.has_graphical_notation,
        start_form_defined: definition.has_start_form_key,
        tenant_id: definition.tenant_id,
        engine_version: definition.engine_version,
        app_version: definition.app_version,
    }
}

async fn list_process_definitions(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    uri: Uri,
) -> Result<Json<PagedResponse<ProcessDefinitionResponse>>, ApiError> {
    let query: ProcessDefinitionListQuery = parse_query(&uri)?;
    let repository_service = engine.get_repository_service();
    let mut definitions = engine.get_repository_service().get_process_definitions()?;

    if let Some(key) = query.key.as_deref() {
        definitions.retain(|definition| definition.key == key);
    }
    if let Some(key_like) = query.key_like.as_deref() {
        definitions.retain(|definition| sql_like_matches(&definition.key, key_like));
    }
    if let Some(name) = query.name.as_deref() {
        definitions.retain(|definition| definition.name.as_deref() == Some(name));
    }
    if let Some(name_like) = query.name_like.as_deref() {
        definitions.retain(|definition| {
            definition
                .name
                .as_deref()
                .is_some_and(|name| sql_like_matches(name, name_like))
        });
    }
    if let Some(name_like_ignore_case) = query.name_like_ignore_case.as_deref() {
        let pattern = name_like_ignore_case.to_lowercase();
        definitions.retain(|definition| {
            definition
                .name
                .as_deref()
                .is_some_and(|name| sql_like_matches(&name.to_lowercase(), &pattern))
        });
    }
    if let Some(category) = query.category.as_deref() {
        definitions.retain(|definition| definition.category.as_deref() == Some(category));
    }
    if let Some(category_like) = query.category_like.as_deref() {
        definitions.retain(|definition| {
            definition
                .category
                .as_deref()
                .is_some_and(|category| sql_like_matches(category, category_like))
        });
    }
    if let Some(category_not_equals) = query.category_not_equals.as_deref() {
        definitions
            .retain(|definition| definition.category.as_deref() != Some(category_not_equals));
    }
    if let Some(resource_name) = query.resource_name.as_deref() {
        definitions.retain(|definition| definition.resource_name.as_deref() == Some(resource_name));
    }
    if let Some(resource_name_like) = query.resource_name_like.as_deref() {
        definitions.retain(|definition| {
            definition
                .resource_name
                .as_deref()
                .is_some_and(|resource_name| sql_like_matches(resource_name, resource_name_like))
        });
    }
    if let Some(deployment_id) = query.deployment_id.as_deref() {
        definitions.retain(|definition| definition.deployment_id.as_deref() == Some(deployment_id));
    }
    if let Some(parent_deployment_id) = query.parent_deployment_id.as_deref() {
        definitions.retain(|definition| {
            process_definition_parent_deployment_id(&repository_service, definition).as_deref()
                == Some(parent_deployment_id)
        });
    }
    if let Some(startable_by_user) = query.startable_by_user.as_deref() {
        let startable_definition_ids =
            startable_process_definition_ids(&engine, startable_by_user)?;
        definitions.retain(|definition| startable_definition_ids.contains(&definition.id));
    }
    if let Some(version) = query.version {
        definitions.retain(|definition| definition.version == version);
    }
    if let Some(suspended) = query.suspended {
        definitions.retain(|definition| definition.is_suspended == suspended);
    }
    if query.without_tenant_id.unwrap_or(false) && query.tenant_id.is_some() {
        return Err(ApiError::BadRequest(
            "tenantId and withoutTenantId cannot be used together".to_string(),
        ));
    }
    if let Some(tenant_id) = query.tenant_id.as_deref() {
        definitions.retain(|definition| definition.tenant_id.as_deref() == Some(tenant_id));
    }
    if let Some(tenant_id_like) = query.tenant_id_like.as_deref() {
        definitions.retain(|definition| {
            definition
                .tenant_id
                .as_deref()
                .is_some_and(|tenant_id| sql_like_matches(tenant_id, tenant_id_like))
        });
    }
    if query.without_tenant_id.unwrap_or(false) {
        definitions.retain(|definition| definition.tenant_id.is_none());
    }
    if query.latest.unwrap_or(false) {
        let mut latest_by_key = BTreeMap::<(String, Option<String>), i32>::new();
        for definition in &definitions {
            let key = (definition.key.clone(), definition.tenant_id.clone());
            latest_by_key
                .entry(key)
                .and_modify(|version| *version = (*version).max(definition.version))
                .or_insert(definition.version);
        }
        definitions.retain(|definition| {
            latest_by_key
                .get(&(definition.key.clone(), definition.tenant_id.clone()))
                .is_some_and(|version| *version == definition.version)
        });
    }

    match query.sort.as_deref() {
        Some("name") => definitions.sort_by(|left, right| {
            left.name
                .cmp(&right.name)
                .then(left.key.cmp(&right.key))
                .then(left.id.cmp(&right.id))
        }),
        Some("key") => definitions.sort_by(|left, right| {
            left.key
                .cmp(&right.key)
                .then(left.version.cmp(&right.version))
                .then(left.id.cmp(&right.id))
        }),
        Some("category") => definitions.sort_by(|left, right| {
            left.category
                .cmp(&right.category)
                .then(left.key.cmp(&right.key))
                .then(left.id.cmp(&right.id))
        }),
        Some("version") => definitions.sort_by(|left, right| {
            left.version
                .cmp(&right.version)
                .then(left.key.cmp(&right.key))
                .then(left.id.cmp(&right.id))
        }),
        Some("deploymentId") => definitions.sort_by(|left, right| {
            left.deployment_id
                .cmp(&right.deployment_id)
                .then(left.key.cmp(&right.key))
                .then(left.id.cmp(&right.id))
        }),
        Some("tenantId") => definitions.sort_by(|left, right| {
            left.tenant_id
                .cmp(&right.tenant_id)
                .then(left.key.cmp(&right.key))
                .then(left.id.cmp(&right.id))
        }),
        Some("id") => definitions.sort_by(|left, right| left.id.cmp(&right.id)),
        None => definitions.sort_by(|left, right| {
            left.name
                .cmp(&right.name)
                .then(left.key.cmp(&right.key))
                .then(left.id.cmp(&right.id))
        }),
        Some(other) => {
            return Err(ApiError::BadRequest(format!(
                "Unsupported process definition sort field '{other}'"
            )));
        }
    }
    if query.order.as_deref() == Some("desc") {
        definitions.reverse();
    } else if !matches!(query.order.as_deref(), None | Some("asc")) {
        return Err(ApiError::BadRequest(format!(
            "Unsupported process definition sort order '{}'",
            query.order.as_deref().unwrap_or_default()
        )));
    }

    let data = definitions
        .into_iter()
        .map(to_process_definition_response)
        .collect();

    Ok(Json(query.paging().paginate(data)))
}

fn process_definition_parent_deployment_id(
    repository_service: &flowable_engine::engine::repository_service::RepositoryService,
    definition: &ProcessDefinition,
) -> Option<String> {
    let deployment_id = definition.deployment_id.as_deref()?;
    repository_service
        .get_deployment(deployment_id)
        .ok()
        .and_then(|deployment| deployment.parent_deployment_id)
}

/// Delegates to the shared O(pattern × value) matcher with the 512-char cap
/// (`routes::tasks::sql_like_matches`). Note this wrapper keeps its legacy
/// `(value, pattern)` parameter order.
fn sql_like_matches(value: &str, pattern: &str) -> bool {
    crate::routes::tasks::sql_like_matches(pattern, value)
}

fn invalid_process_definition_action(action: Option<&str>) -> ApiError {
    ApiError::BadRequest(format!("Invalid action: '{}'.", action.unwrap_or("null")))
}

fn startable_process_definition_ids(
    engine: &ProcessEngine,
    user_id: &str,
) -> Result<HashSet<String>, ApiError> {
    let group_ids = engine
        .get_identity_service()
        .get_groups_by_user(user_id)
        .into_iter()
        .map(|group| group.id)
        .collect::<HashSet<_>>();

    let links = engine
        .get_identity_link_service()
        .create_identity_link_query()
        .link_type("starter".to_string())
        .list()
        .map_err(ApiError::from)?;

    Ok(links
        .into_iter()
        .filter(|link| {
            link.user_id.as_deref() == Some(user_id)
                || link
                    .group_id
                    .as_deref()
                    .is_some_and(|group_id| group_ids.contains(group_id))
        })
        .filter_map(|link| link.process_definition_id)
        .collect())
}

async fn get_process_definition(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(process_definition_id): Path<String>,
) -> Result<Json<ProcessDefinitionResponse>, ApiError> {
    let definition = engine
        .get_repository_service()
        .get_process_definition(&process_definition_id)?;

    Ok(Json(to_process_definition_response(definition)))
}

async fn update_process_definition(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(process_definition_id): Path<String>,
    Json(request): Json<ProcessDefinitionActionRequest>,
) -> Result<Json<ProcessDefinitionResponse>, ApiError> {
    let repository_service = engine.get_repository_service();

    if let Some(category) = request.category.clone().flatten() {
        let definition = repository_service
            .update_process_definition_category(&process_definition_id, Some(category))?;
        return Ok(Json(to_process_definition_response(definition)));
    }

    let action = request.action.as_deref();
    let definition = match action {
        Some("suspend") => apply_process_definition_suspension_action(
            &engine,
            &process_definition_id,
            &request,
            true,
        )?,
        Some("activate") => apply_process_definition_suspension_action(
            &engine,
            &process_definition_id,
            &request,
            false,
        )?,
        other => return Err(invalid_process_definition_action(other)),
    };

    Ok(Json(to_process_definition_response(definition)))
}

fn apply_process_definition_suspension_action(
    engine: &ProcessEngine,
    process_definition_id: &str,
    request: &ProcessDefinitionActionRequest,
    suspended: bool,
) -> Result<ProcessDefinition, ApiError> {
    let repository_service = engine.get_repository_service();
    let include_process_instances = request.include_process_instances.unwrap_or(false);
    let definition = repository_service.get_process_definition(process_definition_id)?;

    if definition.is_suspended == suspended {
        return Err(ApiError::Conflict(format!(
            "Process definition with id '{}' is already {}",
            process_definition_id,
            if suspended { "suspended" } else { "active" }
        )));
    }

    if let Some(date) = request
        .date
        .as_deref()
        .map(str::trim)
        .filter(|date| !date.is_empty())
    {
        let scheduled_date = chrono::DateTime::parse_from_rfc3339(date)
            .map_err(|_| ApiError::bad_request("Invalid process definition action date"))?
            .with_timezone(&chrono::Utc);

        if scheduled_date > chrono::Utc::now() {
            repository_service.schedule_process_definition_suspended(
                process_definition_id,
                suspended,
                include_process_instances,
                scheduled_date.timestamp_millis(),
                scheduled_date.to_rfc3339(),
            )?;
            let mut response_definition = definition;
            response_definition.is_suspended = suspended;
            return Ok(response_definition);
        }
    }

    repository_service
        .set_process_definition_suspended_with_instances(
            process_definition_id,
            suspended,
            include_process_instances,
        )
        .map_err(ApiError::from)
}

async fn get_start_form(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(process_definition_id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let service = FlowableFormService::new(engine);
    let form_data = service.get_start_form_data(&process_definition_id)?;
    let definition = service.get_form_definition(&form_data.form_definition_id)?;
    Ok(Json(definition.form_payload))
}

async fn list_form_definitions_for_process_definition(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(process_definition_id): Path<String>,
    uri: Uri,
) -> Result<Json<PagedResponse<FormDefinitionRecord>>, ApiError> {
    let query: LinkedDefinitionListQuery = parse_query(&uri)?;
    let model = engine
        .get_repository_service()
        .get_bpmn_model(&process_definition_id)?;
    let form_keys = collect_form_keys(&model);
    let service = FlowableFormService::new(engine);
    let mut definitions = Vec::new();

    for form_key in form_keys {
        definitions.extend(
            service
                .create_form_definition_query()
                .key(form_key)
                .list()?
                .into_iter()
                .map(|definition| FormDefinitionRecord {
                    id: definition.id,
                    key: definition.key,
                    name: definition.name,
                    version: definition.version,
                    deployment_id: definition.deployment_id,
                    resource_name: definition.resource_name,
                    tenant_id: None,
                    active: definition.active,
                }),
        );
    }

    definitions.sort_by(|left, right| {
        left.key
            .cmp(&right.key)
            .then(right.version.cmp(&left.version))
            .then(left.id.cmp(&right.id))
    });

    Ok(Json(query.paging().paginate(definitions)))
}

async fn list_decisions_for_process_definition(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Extension(dmn_engine): Extension<Arc<DmnEngine>>,
    Path(process_definition_id): Path<String>,
    uri: Uri,
) -> Result<Json<PagedResponse<DecisionTableRecord>>, ApiError> {
    let query: LinkedDefinitionListQuery = parse_query(&uri)?;
    let model = engine
        .get_repository_service()
        .get_bpmn_model(&process_definition_id)?;
    let decision_keys = collect_decision_keys(&model);
    let repository_service = dmn_engine.repository_service();
    let mut definitions = BTreeMap::new();

    for decision_key in decision_keys {
        for decision in repository_service
            .create_decision_query()
            .key(decision_key)
            .list()?
        {
            definitions.insert(
                decision.id.clone(),
                DecisionTableRecord {
                    id: decision.id,
                    key: decision.key,
                    name: decision.name,
                    version: decision.version,
                    deployment_id: decision.deployment_id,
                    resource_name: decision.resource_name,
                    category: None,
                    description: None,
                    tenant_id: decision.tenant_id,
                    parent_deployment_id: None,
                },
            );
        }
    }

    let mut definitions = definitions.into_values().collect::<Vec<_>>();
    definitions.sort_by(|left, right| {
        left.key
            .cmp(&right.key)
            .then(right.version.cmp(&left.version))
            .then(left.id.cmp(&right.id))
    });

    Ok(Json(query.paging().paginate(definitions)))
}

async fn get_process_definition_resource_data(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(process_definition_id): Path<String>,
) -> Result<Response, ApiError> {
    let repository_service = engine.get_repository_service();
    let definition = repository_service.get_process_definition(&process_definition_id)?;
    let deployment_id = definition.deployment_id.as_deref().ok_or_else(|| {
        ApiError::NotFound(format!(
            "Deployment for process definition '{}' was not found",
            process_definition_id
        ))
    })?;
    let resource_name = definition.resource_name.as_deref().ok_or_else(|| {
        ApiError::NotFound(format!(
            "Resource for process definition '{}' was not found",
            process_definition_id
        ))
    })?;
    let resource = repository_service.get_deployment_resource(deployment_id, resource_name)?;

    Ok((
        StatusCode::OK,
        [(header::CONTENT_TYPE, resource.content_type)],
        resource.bytes,
    )
        .into_response())
}

async fn get_process_definition_model(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(process_definition_id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let model = engine
        .get_repository_service()
        .get_bpmn_model(&process_definition_id)?;
    Ok(Json(serde_json::to_value(model.as_ref()).map_err(
        |error| ApiError::InternalServerError(error.to_string()),
    )?))
}

async fn migrate_process_definition_instances(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(process_definition_id): Path<String>,
    body: String,
) -> Result<StatusCode, ApiError> {
    migrate_instances_for_process_definition(&engine, &process_definition_id, &body)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn batch_migrate_process_definition_instances(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(process_definition_id): Path<String>,
    body: String,
) -> Result<Json<crate::routes::batches::BatchResponse>, ApiError> {
    let source_definition = engine
        .get_repository_service()
        .get_process_definition(&process_definition_id)?;
    let migrated_ids =
        migrate_instances_for_process_definition(&engine, &process_definition_id, &body)?;
    let now = chrono::Utc::now().timestamp_millis().max(0) as u64;
    let batch_id = uuid::Uuid::new_v4().to_string();
    let target_definition_id = crate::routes::process_instances::target_process_definition_id(
        &crate::routes::process_instances::parse_migration_request(&body)?,
    )
    .unwrap_or_default()
    .to_string();

    let batch = BatchEntity {
        id: batch_id.clone(),
        batch_type: "process-instance-migration".to_string(),
        search_key: Some(process_definition_id.clone()),
        search_key2: Some(target_definition_id),
        status: "completed".to_string(),
        total_items: migrated_ids.len() as i64,
        items_processed: migrated_ids.len() as i64,
        create_time: now,
        end_time: Some(now),
        tenant_id: source_definition.tenant_id.clone(),
        batch_document_json: Some(body.clone()),
    };
    let batch_service = engine.get_batch_service();
    batch_service.create_batch(batch.clone());
    for process_instance_id in migrated_ids {
        batch_service.create_batch_part(BatchPartEntity {
            id: uuid::Uuid::new_v4().to_string(),
            batch_id: batch_id.clone(),
            batch_type: batch.batch_type.clone(),
            search_key: Some(process_definition_id.clone()),
            search_key2: batch.search_key2.clone(),
            scope_id: Some(process_instance_id),
            sub_scope_id: None,
            scope_type: Some("bpmn".to_string()),
            create_time: now,
            complete_time: Some(now),
            status: "completed".to_string(),
            tenant_id: None,
            batch_part_document_json: Some(body.clone()),
        });
    }

    Ok(Json(crate::routes::batches::BatchResponse::from(batch)))
}

fn migrate_instances_for_process_definition(
    engine: &ProcessEngine,
    process_definition_id: &str,
    body: &str,
) -> Result<Vec<String>, ApiError> {
    ensure_process_definition_exists(engine, process_definition_id)?;
    let payload = crate::routes::process_instances::parse_migration_request(body)?;
    let target_definition_id =
        crate::routes::process_instances::target_process_definition_id(&payload)
            .filter(|id| !id.trim().is_empty())
            .ok_or_else(|| {
                ApiError::BadRequest(
                    "migrateToProcessDefinitionId is required in the request or migrationDocument"
                        .to_string(),
                )
            })?
            .to_string();
    engine
        .get_repository_service()
        .get_process_definition(&target_definition_id)?;

    let runtime_store = engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let mut matching_instances = runtime_store
        .snapshot_process_instances(&mut session)
        .into_values()
        .filter(|instance| instance.process_definition_id == process_definition_id)
        .collect::<Vec<_>>();
    matching_instances.sort_by(|left, right| left.id.cmp(&right.id));

    let migrated_ids = matching_instances
        .iter()
        .map(|instance| instance.id.clone())
        .collect::<Vec<_>>();
    let activity_migration_mappings =
        crate::routes::process_instances::runtime_activity_migration_mappings(&payload)?;
    for process_instance_id in &migrated_ids {
        crate::routes::process_instances::migrate_process_instance_if_safe(
            engine,
            process_instance_id,
            &target_definition_id,
            activity_migration_mappings.clone(),
        )?;
    }

    Ok(migrated_ids)
}

async fn list_process_definition_identity_links(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(process_definition_id): Path<String>,
) -> Result<Json<Vec<RestIdentityLinkResponse>>, ApiError> {
    ensure_process_definition_exists(&engine, &process_definition_id)?;
    let links = process_definition_identity_links(&engine, &process_definition_id)?;
    Ok(Json(
        links
            .into_iter()
            .map(RestIdentityLinkResponse::from)
            .collect(),
    ))
}

async fn create_process_definition_identity_link(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path(process_definition_id): Path<String>,
    Json(request): Json<CreateProcessDefinitionIdentityLinkRequest>,
) -> Result<(StatusCode, Json<RestIdentityLinkResponse>), ApiError> {
    ensure_process_definition_exists(&engine, &process_definition_id)?;
    let link_type = request
        .link_type
        .ok_or_else(|| ApiError::BadRequest("The identity link type is required.".to_string()))?;
    let (user_id, group_id, family, identity_id) = match (request.user, request.group) {
        (Some(user), None) => (Some(user.clone()), None, "users", user),
        (None, Some(group)) => (None, Some(group.clone()), "groups", group),
        (None, None) => {
            return Err(ApiError::BadRequest(
                "Either user or group is required.".to_string(),
            ));
        }
        (Some(_), Some(_)) => {
            return Err(ApiError::BadRequest(
                "Only one of user or group is allowed.".to_string(),
            ));
        }
    };

    let link = IdentityLink {
        id: process_definition_identity_link_id(
            &process_definition_id,
            family,
            &identity_id,
            &link_type,
        ),
        link_type,
        user_id,
        group_id,
        task_id: None,
        process_instance_id: None,
        process_definition_id: Some(process_definition_id),
    };
    engine
        .get_identity_link_service()
        .add_identity_link(link.clone());

    Ok((
        StatusCode::CREATED,
        Json(RestIdentityLinkResponse::from(link)),
    ))
}

async fn get_process_definition_identity_link(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path((process_definition_id, family, identity_id)): Path<(String, String, String)>,
) -> Result<Json<RestIdentityLinkResponse>, ApiError> {
    ensure_process_definition_exists(&engine, &process_definition_id)?;
    let family = normalize_identity_link_family(&family)?;
    let link = process_definition_identity_links(&engine, &process_definition_id)?
        .into_iter()
        .find(|link| identity_link_matches_family(link, family, &identity_id))
        .ok_or_else(|| {
            ApiError::NotFound(format!(
                "Identity link 'process-definition:{process_definition_id}:{family}:{identity_id}' was not found"
            ))
        })?;

    Ok(Json(RestIdentityLinkResponse::from(link)))
}

async fn delete_process_definition_identity_link(
    Extension(engine): Extension<Arc<ProcessEngine>>,
    Path((process_definition_id, family, identity_id)): Path<(String, String, String)>,
) -> Result<StatusCode, ApiError> {
    ensure_process_definition_exists(&engine, &process_definition_id)?;
    let family = normalize_identity_link_family(&family)?;

    let matching_links = process_definition_identity_links(&engine, &process_definition_id)?
        .into_iter()
        .filter(|link| identity_link_matches_family(link, family, &identity_id))
        .collect::<Vec<_>>();

    if matching_links.is_empty() {
        return Err(ApiError::NotFound(format!(
            "Identity link 'process-definition:{process_definition_id}:{family}:{identity_id}' was not found"
        )));
    }

    let identity_link_service = engine.get_identity_link_service();
    for link in matching_links {
        identity_link_service.remove_identity_link(&link.id);
    }

    Ok(StatusCode::NO_CONTENT)
}

fn ensure_process_definition_exists(
    engine: &ProcessEngine,
    process_definition_id: &str,
) -> Result<(), ApiError> {
    engine
        .get_repository_service()
        .get_process_definition(process_definition_id)
        .map(|_| ())
        .map_err(ApiError::from)
}

fn process_definition_identity_links(
    engine: &ProcessEngine,
    process_definition_id: &str,
) -> Result<Vec<IdentityLink>, ApiError> {
    engine
        .get_identity_link_service()
        .create_identity_link_query()
        .process_definition_id(process_definition_id.to_string())
        .list()
        .map_err(|error| {
            ApiError::InternalServerError(format!("Identity link query failed: {}", error))
        })
}

fn normalize_identity_link_family(family: &str) -> Result<&'static str, ApiError> {
    if family.eq_ignore_ascii_case("users") {
        Ok("users")
    } else if family.eq_ignore_ascii_case("groups") {
        Ok("groups")
    } else {
        Err(ApiError::BadRequest(format!(
            "Unsupported identity link family '{}'",
            family
        )))
    }
}

fn identity_link_matches_family(link: &IdentityLink, family: &str, identity_id: &str) -> bool {
    match family {
        "users" => link.user_id.as_deref() == Some(identity_id),
        "groups" => link.group_id.as_deref() == Some(identity_id),
        _ => false,
    }
}

fn process_definition_identity_link_id(
    process_definition_id: &str,
    family: &str,
    identity_id: &str,
    link_type: &str,
) -> String {
    format!("process-definition:{process_definition_id}:{family}:{identity_id}:type:{link_type}")
}

fn collect_form_keys(model: &BpmnModel) -> BTreeSet<String> {
    let mut keys = BTreeSet::new();
    if let Some(process) = model.main_process.as_ref() {
        collect_form_keys_from_elements(&process.flow_elements, &mut keys);
    }
    keys
}

fn collect_form_keys_from_elements(elements: &[FlowElementEnum], keys: &mut BTreeSet<String>) {
    for element in elements {
        match element {
            FlowElementEnum::StartEvent(event) => insert_non_empty(&event.form_key, keys),
            FlowElementEnum::UserTask(task) => insert_non_empty(&task.form_key, keys),
            FlowElementEnum::ServiceTask(task) => insert_non_empty(&task.form_key, keys),
            FlowElementEnum::CaseServiceTask(task) => insert_non_empty(&task.service_task.form_key, keys),
            FlowElementEnum::SubProcess(sub_process) => {
                collect_form_keys_from_elements(&sub_process.flow_elements, keys)
            }
            FlowElementEnum::Transaction(transaction) => {
                collect_form_keys_from_elements(&transaction.sub_process.flow_elements, keys)
            }
            FlowElementEnum::EventSubProcess(event_sub_process) => {
                collect_form_keys_from_elements(&event_sub_process.sub_process.flow_elements, keys)
            }
            FlowElementEnum::AdhocSubProcess(adhoc_sub_process) => {
                collect_form_keys_from_elements(&adhoc_sub_process.sub_process.flow_elements, keys)
            }
            _ => {}
        }
    }
}

fn collect_decision_keys(model: &BpmnModel) -> BTreeSet<String> {
    let mut keys = BTreeSet::new();
    if let Some(process) = model.main_process.as_ref() {
        collect_decision_keys_from_elements(&process.flow_elements, &mut keys);
    }
    keys
}

fn collect_decision_keys_from_elements(elements: &[FlowElementEnum], keys: &mut BTreeSet<String>) {
    for element in elements {
        match element {
            FlowElementEnum::BusinessRuleTask(task) => insert_non_empty(&task.decision_ref, keys),
            FlowElementEnum::SubProcess(sub_process) => {
                collect_decision_keys_from_elements(&sub_process.flow_elements, keys)
            }
            FlowElementEnum::Transaction(transaction) => {
                collect_decision_keys_from_elements(&transaction.sub_process.flow_elements, keys)
            }
            FlowElementEnum::EventSubProcess(event_sub_process) => {
                collect_decision_keys_from_elements(
                    &event_sub_process.sub_process.flow_elements,
                    keys,
                )
            }
            FlowElementEnum::AdhocSubProcess(adhoc_sub_process) => {
                collect_decision_keys_from_elements(
                    &adhoc_sub_process.sub_process.flow_elements,
                    keys,
                )
            }
            _ => {}
        }
    }
}

fn insert_non_empty(value: &Option<String>, values: &mut BTreeSet<String>) {
    if let Some(value) = value.as_ref().filter(|value| !value.trim().is_empty()) {
        values.insert(value.clone());
    }
}
