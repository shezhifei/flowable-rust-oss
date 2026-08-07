use crate::common::parse_query;
use crate::error::ApiError;
use axum::{
    Extension, Json, Router,
    extract::Path,
    http::{StatusCode, Uri},
    routing::{delete, post},
};
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::engine::query::Query;
use flowable_engine::identity::entities::EntityLink;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

pub type EngineState = Extension<Arc<ProcessEngine>>;

const ENTITY_LINKS_PATH: &str = "/runtime/entity-links";
const ENTITY_LINK_PATH: &str = "/runtime/entity-links/:link_id";

pub fn router() -> Router {
    router_with_prefix("")
}

fn router_with_prefix(prefix: &str) -> Router {
    Router::new()
        .route(
            &format!("{prefix}{ENTITY_LINKS_PATH}"),
            post(create_entity_link).get(list_entity_links),
        )
        .route(
            &format!("{prefix}{ENTITY_LINK_PATH}"),
            delete(delete_entity_link),
        )
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EntityLinkResponse {
    pub id: String,
    pub link_type: String,
    pub scope_id: Option<String>,
    pub scope_type: Option<String>,
    pub reference_scope_id: Option<String>,
    pub reference_scope_type: Option<String>,
    pub hierarchy_type: Option<String>,
}

impl From<EntityLink> for EntityLinkResponse {
    fn from(l: EntityLink) -> Self {
        Self {
            id: l.id,
            link_type: l.link_type,
            scope_id: l.scope_id,
            scope_type: l.scope_type,
            reference_scope_id: l.reference_scope_id,
            reference_scope_type: l.reference_scope_type,
            hierarchy_type: l.hierarchy_type,
        }
    }
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
#[serde(rename_all = "camelCase")]
pub struct EntityLinkQueryParams {
    pub scope_id: Option<String>,
    pub scope_type: Option<String>,
    pub reference_scope_id: Option<String>,
    pub reference_scope_type: Option<String>,
    pub link_type: Option<String>,
}

pub async fn list_entity_links(
    engine: EngineState,
    uri: Uri,
) -> Result<Json<Vec<EntityLinkResponse>>, ApiError> {
    let params: EntityLinkQueryParams = parse_query(&uri)?;
    let service = engine.0.get_entity_link_service();
    let mut query = service.create_entity_link_query();
    if let Some(scope_id) = &params.scope_id {
        query = query.scope_id(scope_id.clone());
    }
    if let Some(scope_type) = &params.scope_type {
        query = query.scope_type(scope_type.clone());
    }
    if let Some(reference_scope_id) = &params.reference_scope_id {
        query = query.reference_scope_id(reference_scope_id.clone());
    }
    if let Some(reference_scope_type) = &params.reference_scope_type {
        query = query.reference_scope_type(reference_scope_type.clone());
    }
    if let Some(link_type) = &params.link_type {
        query = query.link_type(link_type.clone());
    }
    let links = query
        .list()
        .map_err(|e| ApiError::InternalServerError(format!("Entity link query failed: {}", e)))?;
    Ok(Json(
        links.into_iter().map(EntityLinkResponse::from).collect(),
    ))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateEntityLinkRequest {
    pub id: String,
    pub link_type: String,
    pub scope_id: Option<String>,
    pub scope_type: Option<String>,
    pub reference_scope_id: Option<String>,
    pub reference_scope_type: Option<String>,
    #[serde(default)]
    pub hierarchy_type: Option<String>,
}

pub async fn create_entity_link(
    engine: EngineState,
    Json(req): Json<CreateEntityLinkRequest>,
) -> Result<Json<EntityLinkResponse>, ApiError> {
    let link = EntityLink {
        id: req.id.clone(),
        link_type: req.link_type.clone(),
        scope_id: req.scope_id.clone(),
        scope_type: req.scope_type.clone(),
        reference_scope_id: req.reference_scope_id.clone(),
        reference_scope_type: req.reference_scope_type.clone(),
        hierarchy_type: req.hierarchy_type.clone(),
    };
    engine
        .0
        .get_entity_link_service()
        .add_entity_link(link.clone());
    Ok(Json(EntityLinkResponse::from(link)))
}

pub async fn delete_entity_link(
    engine: EngineState,
    Path(link_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    engine
        .0
        .get_entity_link_service()
        .remove_entity_link(&link_id);
    Ok(StatusCode::NO_CONTENT)
}
