use crate::error::ApiError;
use axum::extract::Query as AxumQuery;
use axum::{
    Extension, Json, Router,
    body::{Body, Bytes},
    extract::Path,
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{delete, get, post},
};
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::engine::query::Query;
use flowable_engine::identity::entities::{Group, Membership, Privilege, Token, User, UserInfo};
use serde::{Deserialize, Deserializer, Serialize};
use std::sync::Arc;

pub type EngineState = Extension<Arc<ProcessEngine>>;

const USERS_PATH: &str = "/identity/users";
const USER_PATH: &str = "/identity/users/:user_id";
const USER_INFO_PATH: &str = "/identity/users/:user_id/info";
const USER_INFO_KEY_PATH: &str = "/identity/users/:user_id/info/:key";
const USER_PICTURE_PATH: &str = "/identity/users/:user_id/picture";
const GROUPS_PATH: &str = "/identity/groups";
const GROUP_PATH: &str = "/identity/groups/:group_id";
const GROUP_MEMBERS_PATH: &str = "/identity/groups/:group_id/members";
const GROUP_MEMBER_PATH: &str = "/identity/groups/:group_id/members/:user_id";
const USER_MEMBERSHIPS_PATH: &str = "/identity/users/:user_id/memberships";
const MEMBERSHIPS_PATH: &str = "/identity/memberships";
const MEMBERSHIP_PATH: &str = "/identity/memberships/:user_id/:group_id";
const PRIVILEGES_PATH: &str = "/identity/privileges";
const PRIVILEGE_PATH: &str = "/identity/privileges/:privilege_id";
const TOKENS_PATH: &str = "/identity/tokens";
const TOKEN_PATH: &str = "/identity/tokens/:token_id";
const REST_USERS_PATH: &str = "/users";
const REST_USER_PATH: &str = "/users/:user_id";
const REST_GROUPS_PATH: &str = "/groups";
const REST_GROUP_PATH: &str = "/groups/:group_id";
const REST_GROUP_MEMBERS_PATH: &str = "/groups/:group_id/members";
const REST_GROUP_MEMBER_PATH: &str = "/groups/:group_id/members/:user_id";
const REST_PRIVILEGES_PATH: &str = "/privileges";
const REST_PRIVILEGE_PATH: &str = "/privileges/:privilege_id";
const REST_PRIVILEGE_USERS_PATH: &str = "/privileges/:privilege_id/users";
const REST_PRIVILEGE_USER_PATH: &str = "/privileges/:privilege_id/users/:user_id";
const REST_PRIVILEGE_GROUPS_PATH: &str = "/privileges/:privilege_id/groups";
const REST_PRIVILEGE_GROUP_PATH: &str = "/privileges/:privilege_id/group/:group_id";
const IDM_ENGINE_PATH: &str = "/idm-management/engine";

pub fn router() -> Router {
    router_with_prefix("")
}

fn router_with_prefix(prefix: &str) -> Router {
    Router::new()
        .route(
            &format!("{prefix}{USERS_PATH}"),
            post(create_user).get(list_users),
        )
        .route(
            &format!("{prefix}{USER_PATH}"),
            get(get_user).put(update_user).delete(delete_user),
        )
        .route(
            &format!("{prefix}{USER_INFO_PATH}"),
            get(list_user_info).post(create_user_info),
        )
        .route(
            &format!("{prefix}{USER_INFO_KEY_PATH}"),
            get(get_user_info)
                .put(update_user_info)
                .delete(delete_user_info),
        )
        .route(
            &format!("{prefix}{USER_PICTURE_PATH}"),
            get(get_user_picture)
                .post(create_user_picture)
                .put(update_user_picture)
                .delete(delete_user_picture),
        )
        .route(
            &format!("{prefix}{GROUPS_PATH}"),
            post(create_group).get(list_groups),
        )
        .route(
            &format!("{prefix}{GROUP_PATH}"),
            get(get_group).put(update_group).delete(delete_group),
        )
        .route(
            &format!("{prefix}{GROUP_MEMBERS_PATH}"),
            get(list_group_members).post(create_rest_group_membership),
        )
        .route(
            &format!("{prefix}{GROUP_MEMBER_PATH}"),
            delete(delete_rest_group_membership),
        )
        .route(
            &format!("{prefix}{USER_MEMBERSHIPS_PATH}"),
            get(list_memberships_for_user),
        )
        .route(
            &format!("{prefix}{MEMBERSHIPS_PATH}"),
            post(create_membership),
        )
        .route(
            &format!("{prefix}{MEMBERSHIP_PATH}"),
            delete(delete_membership),
        )
        .route(
            &format!("{prefix}{PRIVILEGES_PATH}"),
            post(create_privilege).get(list_privileges),
        )
        .route(
            &format!("{prefix}{PRIVILEGE_PATH}"),
            get(get_privilege).delete(delete_privilege),
        )
        .route(
            &format!("{prefix}{TOKENS_PATH}"),
            post(create_token).get(list_tokens),
        )
        .route(&format!("{prefix}{TOKEN_PATH}"), delete(delete_token))
        .route(
            &format!("{prefix}{REST_USERS_PATH}"),
            post(create_rest_user).get(list_rest_users),
        )
        .route(
            &format!("{prefix}{REST_USER_PATH}"),
            get(get_rest_user).put(update_rest_user).delete(delete_user),
        )
        .route(
            &format!("{prefix}{REST_GROUPS_PATH}"),
            post(create_group).get(list_groups),
        )
        .route(
            &format!("{prefix}{REST_GROUP_PATH}"),
            get(get_group).put(update_group).delete(delete_group),
        )
        .route(
            &format!("{prefix}{REST_GROUP_MEMBERS_PATH}"),
            get(list_group_members).post(create_rest_group_membership),
        )
        .route(
            &format!("{prefix}{REST_GROUP_MEMBER_PATH}"),
            delete(delete_rest_group_membership),
        )
        .route(
            &format!("{prefix}{REST_PRIVILEGES_PATH}"),
            get(list_privileges),
        )
        .route(
            &format!("{prefix}{REST_PRIVILEGE_PATH}"),
            get(get_privilege),
        )
        .route(
            &format!("{prefix}{REST_PRIVILEGE_USERS_PATH}"),
            get(list_privilege_users).post(add_user_privilege),
        )
        .route(
            &format!("{prefix}{REST_PRIVILEGE_USER_PATH}"),
            delete(delete_user_privilege),
        )
        .route(
            &format!("{prefix}{REST_PRIVILEGE_GROUPS_PATH}"),
            get(list_privilege_groups).post(add_group_privilege),
        )
        .route(
            &format!("{prefix}{REST_PRIVILEGE_GROUP_PATH}"),
            delete(delete_group_privilege),
        )
        .route(&format!("{prefix}{IDM_ENGINE_PATH}"), get(get_idm_engine))
}

#[derive(Deserialize)]
#[serde(default, rename_all = "camelCase")]
#[derive(Default)]
pub struct UserQueryParams {
    pub id: Option<String>,
    #[serde(alias = "first_name")]
    pub first_name: Option<String>,
    #[serde(rename = "firstNameLike", alias = "first_name_contains")]
    pub first_name_like: Option<String>,
    #[serde(alias = "last_name")]
    pub last_name: Option<String>,
    #[serde(rename = "lastNameLike", alias = "last_name_contains")]
    pub last_name_like: Option<String>,
    // P110: Java UserCollectionResource.java:111,123 `userDisplayName` /
    // `userDisplayNameLike`; the display name is computed from first+last name.
    #[serde(alias = "display_name")]
    pub display_name: Option<String>,
    #[serde(rename = "displayNameLike", alias = "display_name_contains")]
    pub display_name_like: Option<String>,
    pub email: Option<String>,
    #[serde(rename = "emailLike", alias = "email_contains")]
    pub email_like: Option<String>,
    #[serde(rename = "memberOfGroup", alias = "member_of_group_id")]
    pub member_of_group_id: Option<String>,
    // P110: Java UserCollectionResource.java:132 `tenantId`.
    #[serde(alias = "tenant_id")]
    pub tenant_id: Option<String>,
    pub start: usize,
    pub size: Option<usize>,
    pub sort: Option<String>,
    pub order: Option<String>,
}

impl UserQueryParams {
    fn paging(&self) -> PagingParams {
        PagingParams {
            start: self.start,
            size: self.size,
        }
    }
}

#[derive(Serialize)]
pub struct UserResponse {
    pub id: String,
    #[serde(rename = "firstName")]
    pub first_name: Option<String>,
    #[serde(rename = "lastName")]
    pub last_name: Option<String>,
    #[serde(rename = "displayName")]
    pub display_name: String,
    pub email: Option<String>,
    pub url: String,
    #[serde(rename = "tenantId", skip_serializing_if = "Option::is_none")]
    pub tenant_id: Option<String>,
}

impl UserResponse {
    fn from_user(user: User, base_url: &str) -> Self {
        let display_name = user_display_name(&user);
        Self {
            id: user.id.clone(),
            first_name: user.first_name,
            last_name: user.last_name,
            display_name,
            email: user.email,
            url: user_url(base_url, &user.id),
            tenant_id: user.tenant_id,
        }
    }
}

#[derive(Serialize)]
pub struct RestUserResponse {
    pub id: String,
    #[serde(rename = "firstName")]
    pub first_name: Option<String>,
    #[serde(rename = "lastName")]
    pub last_name: Option<String>,
    #[serde(rename = "displayName")]
    pub display_name: String,
    pub email: Option<String>,
    pub url: String,
    #[serde(rename = "tenantId")]
    pub tenant_id: Option<String>,
    #[serde(rename = "pictureUrl")]
    pub picture_url: Option<String>,
}

impl RestUserResponse {
    fn from_user(user: User, base_url: &str, has_picture: bool) -> Self {
        let display_name = user_display_name(&user);
        let url = user_url(base_url, &user.id);
        let picture_url = has_picture.then(|| user_picture_url(base_url, &user.id));
        Self {
            id: user.id,
            first_name: user.first_name,
            last_name: user.last_name,
            display_name,
            email: user.email,
            url,
            tenant_id: user.tenant_id,
            picture_url,
        }
    }
}

#[derive(Serialize)]
pub struct DataResponse<T> {
    pub data: Vec<T>,
    pub total: usize,
    pub start: usize,
    pub sort: String,
    pub order: String,
    pub size: usize,
}

async fn list_users(
    engine: EngineState,
    Extension(directory_state): Extension<Arc<crate::DirectoryReadState>>,
    params: AxumQuery<UserQueryParams>,
) -> Result<Json<DataResponse<UserResponse>>, ApiError> {
    let users = list_user_entities(&engine.0, &directory_state, &params)?;
    let base_url = request_base_url(None);
    Ok(Json(paged_response(
        users
            .into_iter()
            .map(|user| UserResponse::from_user(user, &base_url))
            .collect(),
        &params.paging(),
        params.sort.as_deref().unwrap_or("id"),
        params.order.as_deref().unwrap_or("asc"),
    )))
}

async fn list_rest_users(
    engine: EngineState,
    Extension(directory_state): Extension<Arc<crate::DirectoryReadState>>,
    params: AxumQuery<UserQueryParams>,
) -> Result<Json<DataResponse<RestUserResponse>>, ApiError> {
    let users = list_user_entities(&engine.0, &directory_state, &params)?;
    let base_url = request_base_url(None);
    Ok(Json(paged_response(
        users
            .into_iter()
            .map(|user| {
                let has_picture = engine
                    .0
                    .get_identity_service()
                    .get_user_picture(&user.id)
                    .is_some();
                RestUserResponse::from_user(user, &base_url, has_picture)
            })
            .collect(),
        &params.paging(),
        params.sort.as_deref().unwrap_or("id"),
        params.order.as_deref().unwrap_or("asc"),
    )))
}

async fn get_user(
    engine: EngineState,
    Extension(directory_state): Extension<Arc<crate::DirectoryReadState>>,
    Path(user_id): Path<String>,
) -> Result<Json<UserResponse>, ApiError> {
    let base_url = request_base_url(None);
    if let Some(user) = query_live_user_by_id(&directory_state, &user_id)? {
        return Ok(Json(UserResponse::from_user(user, &base_url)));
    }

    let user = engine.0.get_identity_service().find_user_by_id(&user_id);
    match user {
        Some(u) => Ok(Json(UserResponse::from_user(u, &base_url))),
        None => Err(ApiError::NotFound(format!("User '{}' not found", user_id))),
    }
}

async fn get_rest_user(
    engine: EngineState,
    Extension(directory_state): Extension<Arc<crate::DirectoryReadState>>,
    Path(user_id): Path<String>,
) -> Result<Json<RestUserResponse>, ApiError> {
    let user = get_user_entity(&engine.0, &directory_state, &user_id)?;
    let has_picture = engine
        .0
        .get_identity_service()
        .get_user_picture(&user.id)
        .is_some();
    let base_url = request_base_url(None);
    Ok(Json(RestUserResponse::from_user(user, &base_url, has_picture)))
}

#[derive(Deserialize)]
pub struct CreateUserRequest {
    pub id: String,
    #[serde(rename = "firstName", alias = "first_name")]
    pub first_name: Option<String>,
    #[serde(rename = "lastName", alias = "last_name")]
    pub last_name: Option<String>,
    pub email: Option<String>,
    pub password: Option<String>,
    #[serde(rename = "tenantId", alias = "tenant_id")]
    pub tenant_id: Option<String>,
}

#[derive(Deserialize)]
pub struct UpdateUserRequest {
    #[serde(default, rename = "firstName", alias = "first_name")]
    pub first_name: NullableStringUpdate,
    #[serde(default, rename = "lastName", alias = "last_name")]
    pub last_name: NullableStringUpdate,
    #[serde(default)]
    pub email: NullableStringUpdate,
    #[serde(default)]
    pub password: NullableStringUpdate,
    #[serde(default, rename = "tenantId", alias = "tenant_id")]
    pub tenant_id: NullableStringUpdate,
}

#[derive(Default)]
pub enum NullableStringUpdate {
    #[default]
    Missing,
    Set(Option<String>),
}

impl<'de> Deserialize<'de> for NullableStringUpdate {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Option::<String>::deserialize(deserializer).map(Self::Set)
    }
}

async fn create_user(
    engine: EngineState,
    Extension(directory_state): Extension<Arc<crate::DirectoryReadState>>,
    Json(req): Json<CreateUserRequest>,
) -> Result<(StatusCode, Json<UserResponse>), ApiError> {
    let base_url = request_base_url(None);
    let user = save_created_user(&engine.0, &directory_state, req)?;
    Ok((
        StatusCode::CREATED,
        Json(UserResponse::from_user(user, &base_url)),
    ))
}

async fn create_rest_user(
    engine: EngineState,
    Extension(directory_state): Extension<Arc<crate::DirectoryReadState>>,
    Json(req): Json<CreateUserRequest>,
) -> Result<(StatusCode, Json<RestUserResponse>), ApiError> {
    let user = save_created_user(&engine.0, &directory_state, req)?;
    let has_picture = engine
        .0
        .get_identity_service()
        .get_user_picture(&user.id)
        .is_some();
    let base_url = request_base_url(None);
    // Security deviation from Java: the 201 response never echoes the
    // password (plaintext or hash); the password field was removed from
    // RestUserResponse entirely.
    Ok((
        StatusCode::CREATED,
        Json(RestUserResponse::from_user(user, &base_url, has_picture)),
    ))
}

async fn update_user(
    engine: EngineState,
    Extension(directory_state): Extension<Arc<crate::DirectoryReadState>>,
    Path(user_id): Path<String>,
    Json(req): Json<UpdateUserRequest>,
) -> Result<Json<UserResponse>, ApiError> {
    let base_url = request_base_url(None);
    let user = save_updated_user(&engine.0, &directory_state, &user_id, req)?;
    Ok(Json(UserResponse::from_user(user, &base_url)))
}

async fn update_rest_user(
    engine: EngineState,
    Extension(directory_state): Extension<Arc<crate::DirectoryReadState>>,
    Path(user_id): Path<String>,
    Json(req): Json<UpdateUserRequest>,
) -> Result<Json<RestUserResponse>, ApiError> {
    let user = save_updated_user(&engine.0, &directory_state, &user_id, req)?;
    let has_picture = engine
        .0
        .get_identity_service()
        .get_user_picture(&user.id)
        .is_some();
    let base_url = request_base_url(None);
    Ok(Json(RestUserResponse::from_user(user, &base_url, has_picture)))
}

async fn delete_user(
    engine: EngineState,
    Extension(directory_state): Extension<Arc<crate::DirectoryReadState>>,
    Path(user_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    if let Some(deleted) = directory_state.delete_live_user(&user_id)?
        && deleted
    {
        return Ok(StatusCode::NO_CONTENT);
    }
    engine.0.get_identity_service().delete_user(&user_id);
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct CreateUserInfoRequest {
    pub key: Option<String>,
    pub value: Option<String>,
}

#[derive(Deserialize)]
pub struct UpdateUserInfoRequest {
    pub value: Option<String>,
}

#[derive(Serialize)]
pub struct UserInfoResponse {
    pub key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    pub url: String,
}

async fn list_user_info(
    engine: EngineState,
    Extension(directory_state): Extension<Arc<crate::DirectoryReadState>>,
    Path(user_id): Path<String>,
) -> Result<Json<Vec<UserInfoResponse>>, ApiError> {
    get_user_entity(&engine.0, &directory_state, &user_id)?;
    let service = engine.0.get_identity_service();
    let base_url = request_base_url(None);
    let info = service
        .get_user_info_keys(&user_id)
        .into_iter()
        .map(|key| UserInfoResponse {
            url: user_info_url(&base_url, &user_id, &key),
            key,
            value: None,
        })
        .collect();
    Ok(Json(info))
}

async fn create_user_info(
    engine: EngineState,
    Extension(directory_state): Extension<Arc<crate::DirectoryReadState>>,
    Path(user_id): Path<String>,
    Json(req): Json<CreateUserInfoRequest>,
) -> Result<Response, ApiError> {
    get_user_entity(&engine.0, &directory_state, &user_id)?;
    let key = required_body_field(req.key, "key")?;
    let value = required_body_field(req.value, "value")?;
    let service = engine.0.get_identity_service();
    if service.get_user_info(&user_id, &key).is_some() {
        return Ok(conflict_response(format!(
            "User '{}' already has info for key '{}'",
            user_id, key
        )));
    }
    let base_url = request_base_url(None);
    let info = service.set_user_info(user_id, key, value);
    Ok((
        StatusCode::CREATED,
        Json(UserInfoResponse::from_user_info(info, &base_url)),
    )
        .into_response())
}

async fn get_user_info(
    engine: EngineState,
    Extension(directory_state): Extension<Arc<crate::DirectoryReadState>>,
    Path((user_id, key)): Path<(String, String)>,
) -> Result<Json<UserInfoResponse>, ApiError> {
    get_user_entity(&engine.0, &directory_state, &user_id)?;
    let info = engine
        .0
        .get_identity_service()
        .get_user_info(&user_id, &key)
        .ok_or_else(|| {
            ApiError::NotFound(format!("User '{}' has no info for key '{}'", user_id, key))
        })?;
    let base_url = request_base_url(None);
    Ok(Json(UserInfoResponse::from_user_info(info, &base_url)))
}

async fn update_user_info(
    engine: EngineState,
    Extension(directory_state): Extension<Arc<crate::DirectoryReadState>>,
    Path((user_id, key)): Path<(String, String)>,
    Json(req): Json<UpdateUserInfoRequest>,
) -> Result<Json<UserInfoResponse>, ApiError> {
    get_user_entity(&engine.0, &directory_state, &user_id)?;
    let value = required_body_field(req.value, "value")?;
    if engine
        .0
        .get_identity_service()
        .get_user_info(&user_id, &key)
        .is_none()
    {
        return Err(ApiError::NotFound(format!(
            "User '{}' has no info for key '{}'",
            user_id, key
        )));
    }
    let base_url = request_base_url(None);
    let info = engine
        .0
        .get_identity_service()
        .set_user_info(user_id, key, value);
    Ok(Json(UserInfoResponse::from_user_info(info, &base_url)))
}

async fn delete_user_info(
    engine: EngineState,
    Extension(directory_state): Extension<Arc<crate::DirectoryReadState>>,
    Path((user_id, key)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    get_user_entity(&engine.0, &directory_state, &user_id)?;
    if !engine
        .0
        .get_identity_service()
        .delete_user_info(&user_id, &key)
    {
        return Err(ApiError::NotFound(format!(
            "User '{}' has no info for key '{}'",
            user_id, key
        )));
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn get_user_picture(
    engine: EngineState,
    Extension(directory_state): Extension<Arc<crate::DirectoryReadState>>,
    Path(user_id): Path<String>,
) -> Result<Response, ApiError> {
    get_user_entity(&engine.0, &directory_state, &user_id)?;
    let picture = engine
        .0
        .get_identity_service()
        .get_user_picture(&user_id)
        .ok_or_else(|| ApiError::NotFound(format!("User '{}' has no picture", user_id)))?;
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, picture.mime_type)
        .body(Body::from(picture.bytes))
        .map_err(|err| ApiError::InternalServerError(err.to_string()))
}

async fn update_user_picture(
    engine: EngineState,
    Extension(directory_state): Extension<Arc<crate::DirectoryReadState>>,
    Path(user_id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<StatusCode, ApiError> {
    get_user_entity(&engine.0, &directory_state, &user_id)?;
    let upload = parse_picture_upload(&headers, &body)?;
    engine
        .0
        .get_identity_service()
        .set_user_picture(user_id, upload.mime_type, upload.bytes);
    Ok(StatusCode::NO_CONTENT)
}

async fn create_user_picture(
    engine: EngineState,
    Extension(directory_state): Extension<Arc<crate::DirectoryReadState>>,
    Path(user_id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<StatusCode, ApiError> {
    get_user_entity(&engine.0, &directory_state, &user_id)?;
    let upload = parse_picture_upload(&headers, &body)?;
    engine
        .0
        .get_identity_service()
        .set_user_picture(user_id, upload.mime_type, upload.bytes);
    Ok(StatusCode::CREATED)
}

async fn delete_user_picture(
    engine: EngineState,
    Extension(directory_state): Extension<Arc<crate::DirectoryReadState>>,
    Path(user_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    get_user_entity(&engine.0, &directory_state, &user_id)?;
    if !engine
        .0
        .get_identity_service()
        .delete_user_picture(&user_id)
    {
        return Err(ApiError::NotFound(format!(
            "User '{}' has no picture",
            user_id
        )));
    }
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
#[serde(default, rename_all = "camelCase")]
#[derive(Default)]
pub struct GroupQueryParams {
    pub id: Option<String>,
    pub name: Option<String>,
    #[serde(rename = "nameLike", alias = "name_contains")]
    pub name_like: Option<String>,
    #[serde(rename = "type", alias = "group_type")]
    pub group_type: Option<String>,
    #[serde(alias = "group_type_contains")]
    pub group_type_like: Option<String>,
    #[serde(rename = "member", alias = "member_user_id")]
    pub member_user_id: Option<String>,
    // P110: Java GroupCollectionResource.java:80 documents `potentialStarter`
    // in Swagger. Engine semantics (`IdentityService.java:102` +
    // `GetPotentialStarterGroupsCmd.java`): a group is a potential starter for
    // a process definition when a group identity link references that
    // definition. `tenantId` is deliberately NOT added here: Java `Group` has
    // no tenant id and `GroupCollectionResource.getGroups()` never wires it
    // (recorded deviation — see report).
    #[serde(alias = "potential_starter")]
    pub potential_starter: Option<String>,
    pub start: usize,
    pub size: Option<usize>,
    pub sort: Option<String>,
    pub order: Option<String>,
}

impl GroupQueryParams {
    fn paging(&self) -> PagingParams {
        PagingParams {
            start: self.start,
            size: self.size,
        }
    }
}

#[derive(Serialize)]
pub struct GroupResponse {
    pub id: String,
    pub url: String,
    pub name: String,
    #[serde(rename = "type")]
    pub group_type: Option<String>,
}

impl From<Group> for GroupResponse {
    fn from(g: Group) -> Self {
        let url = group_url("http://localhost", &g.id);
        Self {
            id: g.id,
            url,
            name: g.name,
            group_type: g.group_type,
        }
    }
}

async fn list_groups(
    engine: EngineState,
    Extension(directory_state): Extension<Arc<crate::DirectoryReadState>>,
    params: AxumQuery<GroupQueryParams>,
) -> Result<Json<DataResponse<GroupResponse>>, ApiError> {
    let stored_groups = query_engine_groups(&engine.0, &params)?;
    let live_groups = query_live_groups(&directory_state, &params)?;
    let mut groups = crate::merge_groups(stored_groups, live_groups);
    // P110: `potentialStarter` filters to groups referenced by the process
    // definition's identity links (Java GetPotentialStarterGroupsCmd).
    // Applied after merge so live-directory groups match too.
    if let Some(process_definition_id) = params.potential_starter.as_deref() {
        let starter_group_ids = potential_starter_group_ids(&engine.0, process_definition_id)?;
        groups.retain(|group| starter_group_ids.contains(&group.id));
    }
    sort_groups(&mut groups, params.sort.as_deref(), params.order.as_deref())?;
    Ok(Json(paged_response(
        groups.into_iter().map(GroupResponse::from).collect(),
        &params.paging(),
        params.sort.as_deref().unwrap_or("id"),
        params.order.as_deref().unwrap_or("asc"),
    )))
}

async fn get_group(
    engine: EngineState,
    Extension(directory_state): Extension<Arc<crate::DirectoryReadState>>,
    Path(group_id): Path<String>,
) -> Result<Json<GroupResponse>, ApiError> {
    if let Some(group) = query_live_group_by_id(&directory_state, &group_id)? {
        return Ok(Json(GroupResponse::from(group)));
    }

    let group = engine.0.get_identity_service().find_group_by_id(&group_id);
    match group {
        Some(g) => Ok(Json(GroupResponse::from(g))),
        None => Err(ApiError::NotFound(format!(
            "Group '{}' not found",
            group_id
        ))),
    }
}

#[derive(Deserialize)]
pub struct CreateGroupRequest {
    pub id: String,
    pub name: String,
    #[serde(rename = "type", alias = "group_type")]
    pub group_type: Option<String>,
}

#[derive(Deserialize)]
pub struct UpdateGroupRequest {
    pub name: Option<String>,
    #[serde(rename = "type", alias = "group_type")]
    pub group_type: Option<String>,
}

async fn create_group(
    engine: EngineState,
    Extension(directory_state): Extension<Arc<crate::DirectoryReadState>>,
    Json(req): Json<CreateGroupRequest>,
) -> Result<(StatusCode, Json<GroupResponse>), ApiError> {
    let group = Group {
        id: req.id.clone(),
        name: req.name.clone(),
        group_type: req.group_type.clone(),
    };
    if directory_state.has_live_provider() {
        if engine
            .0
            .get_identity_service()
            .find_group_by_id(&group.id)
            .is_some()
        {
            return Err(ApiError::bad_request(format!(
                "Cannot create live LDAP group '{}': the owned identity store already contains the same id",
                group.id
            )));
        }
        if let Some(saved_group) = directory_state.save_live_group(group.clone())? {
            return Ok((StatusCode::CREATED, Json(GroupResponse::from(saved_group))));
        }
    }
    engine.0.get_identity_service().save_group(group.clone());
    Ok((StatusCode::CREATED, Json(GroupResponse::from(group))))
}

async fn update_group(
    engine: EngineState,
    Extension(directory_state): Extension<Arc<crate::DirectoryReadState>>,
    Path(group_id): Path<String>,
    Json(req): Json<UpdateGroupRequest>,
) -> Result<Json<GroupResponse>, ApiError> {
    let mut group = get_group_entity(&engine.0, &directory_state, &group_id)?;
    if let Some(name) = req.name {
        group.name = name;
    }
    if let Some(group_type) = req.group_type {
        group.group_type = Some(group_type);
    }

    if directory_state.has_live_provider()
        && let Some(saved_group) = directory_state.save_live_group(group.clone())?
    {
        return Ok(Json(GroupResponse::from(saved_group)));
    }
    engine.0.get_identity_service().save_group(group.clone());
    Ok(Json(GroupResponse::from(group)))
}

async fn delete_group(
    engine: EngineState,
    Extension(directory_state): Extension<Arc<crate::DirectoryReadState>>,
    Path(group_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    if let Some(deleted) = directory_state.delete_live_group(&group_id)?
        && deleted
    {
        return Ok(StatusCode::NO_CONTENT);
    }
    engine.0.get_identity_service().delete_group(&group_id);
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Serialize)]
pub struct MembershipResponse {
    #[serde(rename = "userId")]
    pub user_id: String,
    #[serde(rename = "groupId")]
    pub group_id: String,
    pub url: String,
}

#[derive(Deserialize)]
pub struct CreateMembershipRequest {
    #[serde(rename = "userId", alias = "user_id")]
    pub user_id: String,
    #[serde(rename = "groupId", alias = "group_id")]
    pub group_id: String,
}

#[derive(Deserialize)]
pub struct RestCreateMembershipRequest {
    #[serde(rename = "userId", alias = "user_id")]
    pub user_id: String,
}

async fn list_memberships_for_user(
    engine: EngineState,
    Extension(directory_state): Extension<Arc<crate::DirectoryReadState>>,
    Path(user_id): Path<String>,
) -> Result<Json<Vec<MembershipResponse>>, ApiError> {
    let stored_memberships = engine
        .0
        .get_identity_service()
        .get_groups_by_user(&user_id)
        .into_iter()
        .map(|group| Membership {
            user_id: user_id.clone(),
            group_id: group.id,
        })
        .collect::<Vec<_>>();
    let live_memberships = query_live_memberships_for_user(&directory_state, &user_id)?;
    let memberships = crate::merge_memberships(stored_memberships, live_memberships);
    Ok(Json(
        memberships
            .into_iter()
            .map(|membership| MembershipResponse {
                user_id: membership.user_id.clone(),
                group_id: membership.group_id.clone(),
                url: membership_url(
                    "http://localhost",
                    &membership.group_id,
                    &membership.user_id,
                ),
            })
            .collect(),
    ))
}

async fn list_group_members(
    engine: EngineState,
    Extension(directory_state): Extension<Arc<crate::DirectoryReadState>>,
    Path(group_id): Path<String>,
    params: AxumQuery<UserQueryParams>,
) -> Result<Json<DataResponse<UserResponse>>, ApiError> {
    get_group_entity(&engine.0, &directory_state, &group_id)?;
    let mut params = params.0;
    if let Some(query_group_id) = params.member_of_group_id.as_deref()
        && query_group_id != group_id
    {
        return Err(ApiError::bad_request(format!(
            "memberOfGroup query parameter '{}' does not match group path id '{}'",
            query_group_id, group_id
        )));
    }
    params.member_of_group_id = Some(group_id);

    let users = list_user_entities(&engine.0, &directory_state, &params)?;
    let base_url = request_base_url(None);
    Ok(Json(paged_response(
        users
            .into_iter()
            .map(|user| UserResponse::from_user(user, &base_url))
            .collect(),
        &params.paging(),
        params.sort.as_deref().unwrap_or("id"),
        params.order.as_deref().unwrap_or("asc"),
    )))
}

async fn create_membership(
    engine: EngineState,
    Extension(directory_state): Extension<Arc<crate::DirectoryReadState>>,
    Json(req): Json<CreateMembershipRequest>,
) -> Result<Response, ApiError> {
    get_user_entity(&engine.0, &directory_state, &req.user_id)?;
    get_group_entity(&engine.0, &directory_state, &req.group_id)?;
    if membership_exists(&engine.0, &directory_state, &req.user_id, &req.group_id)? {
        return Ok(conflict_response(format!(
            "User '{}' is already part of group '{}'.",
            req.user_id, req.group_id
        )));
    }
    let base_url = request_base_url(None);
    if directory_state.has_live_provider() {
        directory_state.create_live_membership(&req.user_id, &req.group_id)?;
        return Ok(created_membership_response(
            &base_url,
            req.user_id,
            req.group_id,
        ));
    }
    engine
        .0
        .get_identity_service()
        .create_membership(req.user_id.clone(), req.group_id.clone());
    Ok(created_membership_response(
        &base_url,
        req.user_id,
        req.group_id,
    ))
}

async fn create_rest_group_membership(
    engine: EngineState,
    Extension(directory_state): Extension<Arc<crate::DirectoryReadState>>,
    Path(group_id): Path<String>,
    Json(req): Json<RestCreateMembershipRequest>,
) -> Result<Response, ApiError> {
    create_membership(
        engine,
        Extension(directory_state),
        Json(CreateMembershipRequest {
            user_id: req.user_id,
            group_id,
        }),
    )
    .await
}

async fn delete_membership(
    engine: EngineState,
    Extension(directory_state): Extension<Arc<crate::DirectoryReadState>>,
    Path((user_id, group_id)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    if directory_state.has_live_provider() {
        directory_state.delete_live_membership(&user_id, &group_id)?;
        return Ok(StatusCode::NO_CONTENT);
    }
    engine
        .0
        .get_identity_service()
        .delete_membership(&user_id, &group_id);
    Ok(StatusCode::NO_CONTENT)
}

async fn delete_rest_group_membership(
    engine: EngineState,
    Extension(directory_state): Extension<Arc<crate::DirectoryReadState>>,
    Path((group_id, user_id)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    delete_membership(
        engine,
        Extension(directory_state),
        Path((user_id, group_id)),
    )
    .await
}

#[derive(Serialize)]
pub struct PrivilegeResponse {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub users: Option<Vec<UserResponse>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub groups: Option<Vec<GroupResponse>>,
}

impl From<Privilege> for PrivilegeResponse {
    fn from(p: Privilege) -> Self {
        Self {
            id: p.id,
            name: p.name,
            users: None,
            groups: None,
        }
    }
}

pub async fn list_privileges(
    engine: EngineState,
    params: AxumQuery<PrivilegeQueryParams>,
) -> Result<Json<DataResponse<PrivilegeResponse>>, ApiError> {
    let store = engine.0.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let mut privileges = store.list_privileges(&mut session);
    if let Some(id) = params.id.as_deref() {
        privileges.retain(|privilege| privilege.id == id);
    }
    if let Some(name) = params.name.as_deref() {
        privileges.retain(|privilege| privilege.name == name);
    }
    if let Some(user_id) = params.user_id.as_deref() {
        let user_privileges = engine
            .0
            .get_identity_service()
            .get_privileges_for_user_in_session(user_id, &mut session);
        privileges.retain(|privilege| {
            user_privileges
                .iter()
                .any(|candidate| candidate.id == privilege.id)
        });
    }
    if let Some(group_id) = params.group_id.as_deref() {
        let group_privileges = engine
            .0
            .get_identity_service()
            .get_privileges_for_group_in_session(group_id, &mut session);
        privileges.retain(|privilege| {
            group_privileges
                .iter()
                .any(|candidate| candidate.id == privilege.id)
        });
    }
    sort_privileges(
        &mut privileges,
        params.sort.as_deref(),
        params.order.as_deref(),
    )?;
    let _ = session.rollback();
    Ok(Json(paged_response(
        privileges
            .into_iter()
            .map(PrivilegeResponse::from)
            .collect(),
        &params.paging(),
        params.sort.as_deref().unwrap_or("id"),
        params.order.as_deref().unwrap_or("asc"),
    )))
}

pub async fn get_privilege(
    engine: EngineState,
    Path(privilege_id): Path<String>,
) -> Result<Json<PrivilegeResponse>, ApiError> {
    let store = engine.0.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let privilege = engine
        .0
        .get_identity_service()
        .find_privilege_by_id_in_session(&privilege_id, &mut session)
        .ok_or_else(|| ApiError::NotFound(format!("Privilege '{}' not found", privilege_id)))?;
    let mut users = store
        .find_privilege_mappings_by_privilege(&privilege_id, &mut session)
        .into_iter()
        .filter_map(|mapping| mapping.user_id)
        .filter_map(|user_id| store.find_user(&user_id, &mut session))
        .collect::<Vec<_>>();
    users.sort_by(|left, right| left.id.cmp(&right.id));

    let mut groups = store
        .find_privilege_mappings_by_privilege(&privilege_id, &mut session)
        .into_iter()
        .filter_map(|mapping| mapping.group_id)
        .filter_map(|group_id| store.find_group(&group_id, &mut session))
        .collect::<Vec<_>>();
    groups.sort_by(|left, right| left.id.cmp(&right.id));
    let _ = session.rollback();

    let mut response = PrivilegeResponse::from(privilege);
    response.users = Some(
        users
            .into_iter()
            .map(|user| UserResponse::from_user(user, "http://localhost"))
            .collect(),
    );
    response.groups = Some(groups.into_iter().map(GroupResponse::from).collect());
    Ok(Json(response))
}

pub async fn create_privilege(
    engine: EngineState,
    Json(req): Json<CreatePrivilegeRequest>,
) -> Result<Json<PrivilegeResponse>, ApiError> {
    let privilege = Privilege {
        id: req.id.clone(),
        name: req.name.clone(),
    };
    engine
        .0
        .get_identity_service()
        .save_privilege(privilege.clone());
    Ok(Json(PrivilegeResponse::from(privilege)))
}

pub async fn delete_privilege(
    engine: EngineState,
    Path(privilege_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    engine
        .0
        .get_identity_service()
        .delete_privilege(&privilege_id);
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
#[serde(default, rename_all = "camelCase")]
#[derive(Default)]
pub struct PrivilegeQueryParams {
    pub id: Option<String>,
    pub name: Option<String>,
    #[serde(rename = "userId", alias = "user_id")]
    pub user_id: Option<String>,
    #[serde(rename = "groupId", alias = "group_id")]
    pub group_id: Option<String>,
    pub start: usize,
    pub size: Option<usize>,
    pub sort: Option<String>,
    pub order: Option<String>,
}

impl PrivilegeQueryParams {
    fn paging(&self) -> PagingParams {
        PagingParams {
            start: self.start,
            size: self.size,
        }
    }
}

#[derive(Deserialize)]
pub struct AddUserPrivilegeRequest {
    #[serde(rename = "userId", alias = "user_id")]
    pub user_id: String,
}

#[derive(Deserialize)]
pub struct AddGroupPrivilegeRequest {
    #[serde(rename = "groupId", alias = "group_id")]
    pub group_id: String,
}

async fn list_privilege_users(
    engine: EngineState,
    Path(privilege_id): Path<String>,
) -> Result<Json<Vec<UserResponse>>, ApiError> {
    ensure_privilege_exists(&engine.0, &privilege_id)?;
    let store = engine.0.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let mut users = store
        .find_privilege_mappings_by_privilege(&privilege_id, &mut session)
        .into_iter()
        .filter_map(|mapping| mapping.user_id)
        .filter_map(|user_id| store.find_user(&user_id, &mut session))
        .collect::<Vec<_>>();
    users.sort_by(|left, right| left.id.cmp(&right.id));
    Ok(Json(
        users
            .into_iter()
            .map(|user| UserResponse::from_user(user, "http://localhost"))
            .collect(),
    ))
}

async fn add_user_privilege(
    engine: EngineState,
    Path(privilege_id): Path<String>,
    Json(req): Json<AddUserPrivilegeRequest>,
) -> Result<StatusCode, ApiError> {
    ensure_privilege_exists(&engine.0, &privilege_id)?;
    ensure_user_exists(&engine.0, &req.user_id)?;
    engine
        .0
        .get_identity_service()
        .add_user_privilege_mapping(privilege_id, req.user_id);
    Ok(StatusCode::OK)
}

async fn delete_user_privilege(
    engine: EngineState,
    Path((privilege_id, user_id)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    ensure_privilege_exists(&engine.0, &privilege_id)?;
    engine
        .0
        .get_identity_service()
        .delete_user_privilege_mapping(&privilege_id, &user_id);
    Ok(StatusCode::OK)
}

async fn list_privilege_groups(
    engine: EngineState,
    Path(privilege_id): Path<String>,
) -> Result<Json<Vec<GroupResponse>>, ApiError> {
    ensure_privilege_exists(&engine.0, &privilege_id)?;
    let store = engine.0.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let mut groups = store
        .find_privilege_mappings_by_privilege(&privilege_id, &mut session)
        .into_iter()
        .filter_map(|mapping| mapping.group_id)
        .filter_map(|group_id| store.find_group(&group_id, &mut session))
        .collect::<Vec<_>>();
    groups.sort_by(|left, right| left.id.cmp(&right.id));
    Ok(Json(groups.into_iter().map(GroupResponse::from).collect()))
}

async fn add_group_privilege(
    engine: EngineState,
    Path(privilege_id): Path<String>,
    Json(req): Json<AddGroupPrivilegeRequest>,
) -> Result<StatusCode, ApiError> {
    ensure_privilege_exists(&engine.0, &privilege_id)?;
    ensure_group_exists(&engine.0, &req.group_id)?;
    engine
        .0
        .get_identity_service()
        .add_group_privilege_mapping(privilege_id, req.group_id);
    Ok(StatusCode::OK)
}

async fn delete_group_privilege(
    engine: EngineState,
    Path((privilege_id, group_id)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    ensure_privilege_exists(&engine.0, &privilege_id)?;
    engine
        .0
        .get_identity_service()
        .delete_group_privilege_mapping(&privilege_id, &group_id);
    Ok(StatusCode::OK)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct IdmEngineInfoResponse {
    name: String,
    version: String,
    resource_url: Option<String>,
    exception: Option<String>,
}

async fn get_idm_engine(engine: EngineState) -> Json<IdmEngineInfoResponse> {
    Json(IdmEngineInfoResponse {
        name: engine.0.get_name().to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        resource_url: None,
        exception: None,
    })
}

#[derive(Deserialize)]
pub struct CreatePrivilegeRequest {
    pub id: String,
    pub name: String,
}

#[derive(Serialize)]
pub struct TokenResponse {
    pub id: String,
    pub token_value: String,
    pub user_id: Option<String>,
}

impl From<Token> for TokenResponse {
    fn from(t: Token) -> Self {
        Self {
            id: t.id,
            token_value: t.token_value,
            user_id: t.user_id,
        }
    }
}

#[derive(Deserialize)]
pub struct TokenQueryParams {
    pub token_value: Option<String>,
    pub user_id: Option<String>,
}

pub async fn list_tokens(
    engine: EngineState,
    params: AxumQuery<TokenQueryParams>,
) -> Result<Json<Vec<TokenResponse>>, ApiError> {
    let service = engine.0.get_identity_service();
    let mut query = service.create_token_query();
    if let Some(token_value) = &params.token_value {
        query = query.token_value(token_value.clone());
    }
    if let Some(user_id) = &params.user_id {
        query = query.user_id(user_id.clone());
    }
    let tokens = query
        .list()
        .map_err(|e| ApiError::InternalServerError(format!("Token query failed: {}", e)))?;
    Ok(Json(tokens.into_iter().map(TokenResponse::from).collect()))
}

pub async fn create_token(
    engine: EngineState,
    Json(req): Json<CreateTokenRequest>,
) -> Result<Json<TokenResponse>, ApiError> {
    let token = Token {
        id: req.id.clone(),
        token_value: req.token_value.clone(),
        user_id: req.user_id.clone(),
        token_date: Some(chrono::Utc::now().timestamp_millis()),
        ip_address: None,
        user_agent: None,
    };
    engine.0.get_identity_service().save_token(token.clone());
    Ok(Json(TokenResponse::from(token)))
}

pub async fn delete_token(
    engine: EngineState,
    Path(token_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    engine.0.get_identity_service().delete_token(&token_id);
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct CreateTokenRequest {
    pub id: String,
    pub token_value: String,
    pub user_id: Option<String>,
}

fn list_user_entities(
    engine: &Arc<ProcessEngine>,
    directory_state: &Arc<crate::DirectoryReadState>,
    params: &UserQueryParams,
) -> Result<Vec<User>, ApiError> {
    let stored_users = query_engine_users(engine, params)?;
    let live_users = query_live_users(directory_state, params)?;
    let mut users = crate::merge_users(stored_users, live_users);
    sort_users(&mut users, params.sort.as_deref(), params.order.as_deref())?;
    Ok(users)
}

fn save_created_user(
    engine: &Arc<ProcessEngine>,
    directory_state: &Arc<crate::DirectoryReadState>,
    req: CreateUserRequest,
) -> Result<User, ApiError> {
    let user = User {
        id: req.id,
        first_name: req.first_name,
        last_name: req.last_name,
        email: req.email,
        password: req.password,
        tenant_id: req.tenant_id,
    };
    if directory_state.has_live_provider() {
        if engine
            .get_identity_service()
            .find_user_by_id(&user.id)
            .is_some()
        {
            return Err(ApiError::bad_request(format!(
                "Cannot create live LDAP user '{}': the owned identity store already contains the same id",
                user.id
            )));
        }
        if let Some(saved_user) = directory_state.save_live_user(user.clone())? {
            return Ok(saved_user);
        }
    }
    engine.get_identity_service().save_user(user.clone());
    Ok(user)
}

fn save_updated_user(
    engine: &Arc<ProcessEngine>,
    directory_state: &Arc<crate::DirectoryReadState>,
    user_id: &str,
    req: UpdateUserRequest,
) -> Result<User, ApiError> {
    let mut user = get_user_entity(engine, directory_state, user_id)?;
    if let NullableStringUpdate::Set(first_name) = req.first_name {
        user.first_name = first_name;
    }
    if let NullableStringUpdate::Set(last_name) = req.last_name {
        user.last_name = last_name;
    }
    if let NullableStringUpdate::Set(email) = req.email {
        user.email = email;
    }
    if let NullableStringUpdate::Set(password) = req.password {
        user.password = password;
    }
    if let NullableStringUpdate::Set(tenant_id) = req.tenant_id {
        user.tenant_id = tenant_id;
    }

    if directory_state.has_live_provider()
        && let Some(saved_user) = directory_state.save_live_user(user.clone())?
    {
        return Ok(saved_user);
    }
    engine.get_identity_service().save_user(user.clone());
    Ok(user)
}

fn query_engine_users(
    engine: &Arc<ProcessEngine>,
    params: &UserQueryParams,
) -> Result<Vec<User>, ApiError> {
    let service = engine.get_identity_service();
    let mut query = service.create_user_query();
    if let Some(first_name) = &params.first_name {
        query = query.first_name(first_name.clone());
    }
    if let Some(last_name) = &params.last_name {
        query = query.last_name(last_name.clone());
    }
    if let Some(email) = &params.email {
        query = query.email(email.clone());
    }
    if let Some(group_id) = &params.member_of_group_id {
        query = query.member_of_group_id(group_id.clone());
    }
    let users = query
        .list()
        .map_err(|error| ApiError::InternalServerError(format!("User query failed: {}", error)))?;
    Ok(users
        .into_iter()
        .filter(|user| user_matches(user, params))
        .collect())
}

fn query_live_users(
    directory_state: &Arc<crate::DirectoryReadState>,
    params: &UserQueryParams,
) -> Result<Vec<User>, ApiError> {
    Ok(directory_state
        .load_live_snapshot()?
        .map(|snapshot| {
            let memberships = snapshot.memberships.clone();
            snapshot
                .users
                .into_iter()
                .filter(|user| user_matches(user, params))
                .filter(|user| user_membership_matches(user, Some(&memberships), params))
                .collect()
        })
        .unwrap_or_default())
}

fn query_live_user_by_id(
    directory_state: &Arc<crate::DirectoryReadState>,
    user_id: &str,
) -> Result<Option<User>, ApiError> {
    Ok(directory_state
        .load_live_snapshot()?
        .and_then(|snapshot| snapshot.users.into_iter().find(|user| user.id == user_id)))
}

fn query_engine_groups(
    engine: &Arc<ProcessEngine>,
    params: &GroupQueryParams,
) -> Result<Vec<Group>, ApiError> {
    let service = engine.get_identity_service();
    let mut query = service.create_group_query();
    if let Some(name) = &params.name {
        query = query.name(name.clone());
    }
    if let Some(group_type) = &params.group_type {
        query = query.group_type(group_type.clone());
    }
    let groups = query
        .list()
        .map_err(|error| ApiError::InternalServerError(format!("Group query failed: {}", error)))?;
    Ok(groups
        .into_iter()
        .filter(|group| group_matches(group, params))
        .filter(|group| group_membership_matches(engine, group, params))
        .collect())
}

fn query_live_groups(
    directory_state: &Arc<crate::DirectoryReadState>,
    params: &GroupQueryParams,
) -> Result<Vec<Group>, ApiError> {
    Ok(directory_state
        .load_live_snapshot()?
        .map(|snapshot| {
            let memberships = snapshot.memberships.clone();
            snapshot
                .groups
                .into_iter()
                .filter(|group| group_matches(group, params))
                .filter(|group| live_group_membership_matches(group, &memberships, params))
                .collect()
        })
        .unwrap_or_default())
}

fn query_live_group_by_id(
    directory_state: &Arc<crate::DirectoryReadState>,
    group_id: &str,
) -> Result<Option<Group>, ApiError> {
    Ok(directory_state.load_live_snapshot()?.and_then(|snapshot| {
        snapshot
            .groups
            .into_iter()
            .find(|group| group.id == group_id)
    }))
}

fn query_live_memberships_for_user(
    directory_state: &Arc<crate::DirectoryReadState>,
    user_id: &str,
) -> Result<Vec<Membership>, ApiError> {
    Ok(directory_state
        .load_live_snapshot()?
        .map(|snapshot| {
            snapshot
                .memberships
                .into_iter()
                .filter(|membership| membership.user_id == user_id)
                .collect()
        })
        .unwrap_or_default())
}

fn membership_exists(
    engine: &Arc<ProcessEngine>,
    directory_state: &Arc<crate::DirectoryReadState>,
    user_id: &str,
    group_id: &str,
) -> Result<bool, ApiError> {
    if engine
        .get_identity_service()
        .membership_exists(user_id, group_id)
    {
        return Ok(true);
    }
    Ok(query_live_memberships_for_user(directory_state, user_id)?
        .into_iter()
        .any(|membership| membership.group_id == group_id))
}

fn get_user_entity(
    engine: &Arc<ProcessEngine>,
    directory_state: &Arc<crate::DirectoryReadState>,
    user_id: &str,
) -> Result<User, ApiError> {
    if let Some(user) = query_live_user_by_id(directory_state, user_id)? {
        return Ok(user);
    }
    engine
        .get_identity_service()
        .find_user_by_id(user_id)
        .ok_or_else(|| ApiError::NotFound(format!("User '{}' not found", user_id)))
}

fn get_group_entity(
    engine: &Arc<ProcessEngine>,
    directory_state: &Arc<crate::DirectoryReadState>,
    group_id: &str,
) -> Result<Group, ApiError> {
    if let Some(group) = query_live_group_by_id(directory_state, group_id)? {
        return Ok(group);
    }
    engine
        .get_identity_service()
        .find_group_by_id(group_id)
        .ok_or_else(|| ApiError::NotFound(format!("Group '{}' not found", group_id)))
}

fn ensure_privilege_exists(
    engine: &Arc<ProcessEngine>,
    privilege_id: &str,
) -> Result<Privilege, ApiError> {
    engine
        .get_identity_service()
        .find_privilege_by_id(privilege_id)
        .ok_or_else(|| ApiError::NotFound(format!("Privilege '{}' not found", privilege_id)))
}

fn ensure_user_exists(engine: &Arc<ProcessEngine>, user_id: &str) -> Result<(), ApiError> {
    engine
        .get_identity_service()
        .find_user_by_id(user_id)
        .map(|_| ())
        .ok_or_else(|| ApiError::NotFound(format!("User '{}' not found", user_id)))
}

fn ensure_group_exists(engine: &Arc<ProcessEngine>, group_id: &str) -> Result<(), ApiError> {
    engine
        .get_identity_service()
        .find_group_by_id(group_id)
        .map(|_| ())
        .ok_or_else(|| ApiError::NotFound(format!("Group '{}' not found", group_id)))
}

fn user_matches(user: &User, params: &UserQueryParams) -> bool {
    matches_optional_field(Some(user.id.as_str()), params.id.as_deref())
        && matches_optional_field(user.first_name.as_deref(), params.first_name.as_deref())
        && matches_contains_field(
            user.first_name.as_deref(),
            params.first_name_like.as_deref(),
        )
        && matches_optional_field(user.last_name.as_deref(), params.last_name.as_deref())
        && matches_contains_field(user.last_name.as_deref(), params.last_name_like.as_deref())
        && matches_optional_field(user.email.as_deref(), params.email.as_deref())
        && matches_contains_field(user.email.as_deref(), params.email_like.as_deref())
        // P110: Java UserCollectionResource.java:132 `tenantId`.
        && matches_optional_field(user.tenant_id.as_deref(), params.tenant_id.as_deref())
        // P110: Java UserCollectionResource.java:111,123 `userDisplayName` /
        // `userDisplayNameLike` — computed from first+last name.
        && display_name_matches(user, params)
}

/// Matches the computed display name (first+last) against the exact
/// `displayName` and `displayNameLike` params, mirroring Java's
/// `userDisplayName` / `userDisplayNameLike` (UserCollectionResource.java:111,123).
fn display_name_matches(user: &User, params: &UserQueryParams) -> bool {
    match (&params.display_name, &params.display_name_like) {
        (None, None) => true,
        _ => {
            let display_name = user_display_name(user);
            matches_optional_field(Some(display_name.as_str()), params.display_name.as_deref())
                && matches_contains_field(
                    Some(display_name.as_str()),
                    params.display_name_like.as_deref(),
                )
        }
    }
}

fn group_matches(group: &Group, params: &GroupQueryParams) -> bool {
    matches_optional_field(Some(group.id.as_str()), params.id.as_deref())
        && matches_optional_field(Some(group.name.as_str()), params.name.as_deref())
        && matches_contains_field(Some(group.name.as_str()), params.name_like.as_deref())
        && matches_optional_field(group.group_type.as_deref(), params.group_type.as_deref())
        && matches_contains_field(
            group.group_type.as_deref(),
            params.group_type_like.as_deref(),
        )
}

fn matches_optional_field(value: Option<&str>, expected: Option<&str>) -> bool {
    match expected {
        Some(expected) => value == Some(expected),
        None => true,
    }
}

fn matches_contains_field(value: Option<&str>, expected: Option<&str>) -> bool {
    match expected {
        Some(expected) => value
            .map(|value| {
                value
                    .to_ascii_lowercase()
                    .contains(&expected.to_ascii_lowercase())
            })
            .unwrap_or(false),
        None => true,
    }
}

fn user_membership_matches(
    user: &User,
    memberships: Option<&[Membership]>,
    params: &UserQueryParams,
) -> bool {
    match (&params.member_of_group_id, memberships) {
        (Some(group_id), Some(memberships)) => memberships
            .iter()
            .any(|membership| membership.user_id == user.id && membership.group_id == *group_id),
        (Some(_), None) => false,
        (None, _) => true,
    }
}

/// P110: Java `IdentityService.getPotentialStarterGroups` /
/// `GetPotentialStarterGroupsCmd.java:50-75` — collect the group ids a process
/// definition references through its identity links (its candidate-starter
/// groups). The Rust `identity_links` table and
/// `find_identity_links_by_process_definition` (`runtime_store.rs:2839`) exist;
/// no BPMN code currently populates definition-level candidate links, so the
/// filter only matches when such links are present.
fn potential_starter_group_ids(
    engine: &Arc<ProcessEngine>,
    process_definition_id: &str,
) -> Result<std::collections::HashSet<String>, ApiError> {
    let store = engine.get_runtime_store();
    let mut session = store
        .create_session()
        .map_err(|error| ApiError::InternalServerError(error.to_string()))?;
    let links = store.find_identity_links_by_process_definition(process_definition_id, &mut session);
    let _ = session.rollback();
    Ok(links.into_iter().filter_map(|link| link.group_id).collect())
}

fn group_membership_matches(
    engine: &Arc<ProcessEngine>,
    group: &Group,
    params: &GroupQueryParams,
) -> bool {
    match &params.member_user_id {
        Some(user_id) => engine
            .get_identity_service()
            .get_groups_by_user(user_id)
            .into_iter()
            .any(|candidate| candidate.id == group.id),
        None => true,
    }
}

fn live_group_membership_matches(
    group: &Group,
    memberships: &[Membership],
    params: &GroupQueryParams,
) -> bool {
    match &params.member_user_id {
        Some(user_id) => memberships
            .iter()
            .any(|membership| membership.group_id == group.id && membership.user_id == *user_id),
        None => true,
    }
}

impl UserInfoResponse {
    fn from_user_info(info: UserInfo, base_url: &str) -> Self {
        Self {
            url: user_info_url(base_url, &info.user_id, &info.key),
            key: info.key,
            value: Some(info.value),
        }
    }
}

struct PictureUpload {
    mime_type: String,
    bytes: Vec<u8>,
}

struct PagingParams {
    start: usize,
    size: Option<usize>,
}

fn paged_response<T>(
    items: Vec<T>,
    paging: &PagingParams,
    sort: &str,
    order: &str,
) -> DataResponse<T> {
    let total = items.len();
    let requested_size = paging.size.unwrap_or(10);
    let data = items
        .into_iter()
        .skip(paging.start)
        .take(requested_size)
        .collect::<Vec<_>>();
    let size = data.len();
    DataResponse {
        data,
        total,
        start: paging.start,
        sort: sort.to_string(),
        order: order.to_string(),
        size,
    }
}

fn sort_users(users: &mut [User], sort: Option<&str>, order: Option<&str>) -> Result<(), ApiError> {
    match sort {
        None | Some("id") => users.sort_by(|left, right| left.id.cmp(&right.id)),
        Some("firstName") => users.sort_by(|left, right| left.first_name.cmp(&right.first_name)),
        Some("lastName") => users.sort_by(|left, right| left.last_name.cmp(&right.last_name)),
        Some("email") => users.sort_by(|left, right| left.email.cmp(&right.email)),
        Some("displayName") => users.sort_by_key(user_display_name),
        Some(other) => {
            return Err(ApiError::bad_request(format!(
                "Invalid sort property '{}'",
                other
            )));
        }
    }
    apply_order(users, order)?;
    Ok(())
}

fn sort_groups(
    groups: &mut [Group],
    sort: Option<&str>,
    order: Option<&str>,
) -> Result<(), ApiError> {
    match sort {
        None | Some("id") => groups.sort_by(|left, right| left.id.cmp(&right.id)),
        Some("name") => groups.sort_by(|left, right| left.name.cmp(&right.name)),
        Some("type") => groups.sort_by(|left, right| left.group_type.cmp(&right.group_type)),
        Some(other) => {
            return Err(ApiError::bad_request(format!(
                "Invalid sort property '{}'",
                other
            )));
        }
    }
    apply_order(groups, order)?;
    Ok(())
}

fn sort_privileges(
    privileges: &mut [Privilege],
    sort: Option<&str>,
    order: Option<&str>,
) -> Result<(), ApiError> {
    match sort {
        None | Some("id") => privileges.sort_by(|left, right| left.id.cmp(&right.id)),
        Some("name") => privileges.sort_by(|left, right| left.name.cmp(&right.name)),
        Some(other) => {
            return Err(ApiError::bad_request(format!(
                "Invalid sort property '{}'",
                other
            )));
        }
    }
    apply_order(privileges, order)?;
    Ok(())
}

fn apply_order<T>(items: &mut [T], order: Option<&str>) -> Result<(), ApiError> {
    match order {
        None | Some("asc") => Ok(()),
        Some("desc") => {
            items.reverse();
            Ok(())
        }
        Some(other) => Err(ApiError::bad_request(format!(
            "Invalid sort order '{}'",
            other
        ))),
    }
}

fn user_display_name(user: &User) -> String {
    match (&user.first_name, &user.last_name) {
        (Some(first), Some(last)) => format!("{first} {last}"),
        (Some(first), None) => first.clone(),
        (None, Some(last)) => last.clone(),
        (None, None) => String::new(),
    }
}

fn required_body_field(value: Option<String>, field: &str) -> Result<String, ApiError> {
    match value {
        Some(value) if !value.is_empty() => Ok(value),
        _ => Err(ApiError::bad_request(format!(
            "Request body must include '{}'",
            field
        ))),
    }
}

fn request_base_url(base_override: Option<&str>) -> String {
    base_override.unwrap_or("http://localhost").to_string()
}

fn user_info_url(base_url: &str, user_id: &str, key: &str) -> String {
    format!("{base_url}/identity/users/{user_id}/info/{key}")
}

fn user_url(base_url: &str, user_id: &str) -> String {
    format!("{base_url}/identity/users/{user_id}")
}

fn user_picture_url(base_url: &str, user_id: &str) -> String {
    format!("{base_url}/identity/users/{user_id}/picture")
}

fn group_url(base_url: &str, group_id: &str) -> String {
    format!("{base_url}/identity/groups/{group_id}")
}

fn membership_url(base_url: &str, group_id: &str, user_id: &str) -> String {
    format!("{base_url}/identity/groups/{group_id}/members/{user_id}")
}

fn created_membership_response(base_url: &str, user_id: String, group_id: String) -> Response {
    (
        StatusCode::CREATED,
        Json(MembershipResponse {
            url: membership_url(base_url, &group_id, &user_id),
            user_id,
            group_id,
        }),
    )
        .into_response()
}

fn conflict_response(message: String) -> Response {
    (
        StatusCode::CONFLICT,
        Json(crate::error::ErrorResponse {
            code: "CONFLICT".to_string(),
            message: "Conflict".to_string(),
            details: Some(message),
        }),
    )
        .into_response()
}

fn parse_picture_upload(headers: &HeaderMap, body: &Bytes) -> Result<PictureUpload, ApiError> {
    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| ApiError::bad_request("Missing Content-Type header"))?;
    let boundary = multipart_boundary(content_type)
        .ok_or_else(|| ApiError::bad_request("Expected multipart/form-data request"))?;
    let parts = parse_multipart_parts(body.as_ref(), &boundary)?;

    let mut file_bytes = None;
    let mut mime_type = None;

    for part in parts {
        match part.name.as_deref() {
            Some("mimeType") => {
                mime_type = Some(String::from_utf8(part.body).map_err(|err| {
                    ApiError::bad_request(format!("Invalid mimeType field: {err}"))
                })?);
            }
            Some(_) if file_bytes.is_none() && part.filename.is_some() => {
                file_bytes = Some(part.body);
            }
            _ => {}
        }
    }

    let bytes = file_bytes.ok_or_else(|| {
        ApiError::bad_request("Multipart request must contain one file part with picture bytes")
    })?;
    if bytes.is_empty() {
        return Err(ApiError::bad_request("Picture file part must not be empty"));
    }

    Ok(PictureUpload {
        mime_type: mime_type
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "image/jpeg".to_string()),
        bytes,
    })
}

struct MultipartPart {
    name: Option<String>,
    filename: Option<String>,
    body: Vec<u8>,
}

fn multipart_boundary(content_type: &str) -> Option<String> {
    content_type
        .split(';')
        .map(str::trim)
        .find_map(|part| part.strip_prefix("boundary="))
        .map(|boundary| boundary.trim_matches('"').to_string())
        .filter(|boundary| !boundary.is_empty())
}

fn parse_multipart_parts(body: &[u8], boundary: &str) -> Result<Vec<MultipartPart>, ApiError> {
    let delimiter = format!("--{boundary}").into_bytes();
    let mut positions = Vec::new();
    let mut cursor = 0;
    while let Some(offset) = find_subslice(&body[cursor..], &delimiter) {
        let position = cursor + offset;
        positions.push(position);
        cursor = position + delimiter.len();
    }

    let mut parts = Vec::new();
    for pair in positions.windows(2) {
        let mut start = pair[0] + delimiter.len();
        if body.get(start..start + 2) == Some(b"--") {
            break;
        }
        if body.get(start..start + 2) == Some(b"\r\n") {
            start += 2;
        }

        let mut end = pair[1];
        if end >= 2 && body.get(end - 2..end) == Some(b"\r\n") {
            end -= 2;
        }
        if start >= end {
            continue;
        }

        let part = &body[start..end];
        let header_end = find_subslice(part, b"\r\n\r\n")
            .ok_or_else(|| ApiError::bad_request("Malformed multipart part"))?;
        let headers = std::str::from_utf8(&part[..header_end])
            .map_err(|err| ApiError::bad_request(format!("Invalid multipart headers: {err}")))?;
        let content_disposition = headers
            .lines()
            .find(|line| {
                line.to_ascii_lowercase()
                    .starts_with("content-disposition:")
            })
            .ok_or_else(|| ApiError::bad_request("Multipart part missing Content-Disposition"))?;
        parts.push(MultipartPart {
            name: disposition_param(content_disposition, "name"),
            filename: disposition_param(content_disposition, "filename"),
            body: part[header_end + 4..].to_vec(),
        });
    }

    Ok(parts)
}

fn disposition_param(content_disposition: &str, name: &str) -> Option<String> {
    content_disposition
        .split(';')
        .map(str::trim)
        .find_map(|part| part.strip_prefix(&format!("{name}=")))
        .map(|value| value.trim_matches('"').to_string())
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

#[cfg(test)]
mod tests {
    use super::*;
    use flowable_engine::engine::process_engine::ProcessEngine;
    use flowable_engine::identity::entities::{Group, Privilege, PrivilegeMapping, User};
    use std::sync::Arc;

    #[test]
    fn user_response_keeps_tenant_and_absolute_url() {
        let response = UserResponse::from_user(
            User {
                id: "u1".to_string(),
                first_name: Some("Ada".to_string()),
                last_name: Some("Lovelace".to_string()),
                email: Some("ada@example.com".to_string()),
                password: None,
                tenant_id: Some("tenant-a".to_string()),
            },
            "https://example.org/api",
        );
        assert_eq!(response.url, "https://example.org/api/identity/users/u1");
        assert_eq!(response.tenant_id.as_deref(), Some("tenant-a"));
    }

    #[test]
    fn rest_user_response_keeps_tenant_and_picture_url() {
        let response = RestUserResponse::from_user(
            User {
                id: "u1".to_string(),
                first_name: None,
                last_name: None,
                email: None,
                password: Some("secret".to_string()),
                tenant_id: Some("tenant-b".to_string()),
            },
            "https://example.org/base",
            true,
        );
        assert_eq!(response.url, "https://example.org/base/identity/users/u1");
        assert_eq!(
            response.picture_url.as_deref(),
            Some("https://example.org/base/identity/users/u1/picture")
        );
        assert_eq!(response.tenant_id.as_deref(), Some("tenant-b"));
    }

    #[test]
    fn privilege_lookup_includes_group_assignments() {
        let engine = Arc::new(ProcessEngine::new("idm-privilege-test".to_string()));
        let identity = engine.get_identity_service();
        let mut session = engine.get_runtime_store().create_session().unwrap();
        identity.save_user_in_session(
            User {
                id: "u1".to_string(),
                first_name: None,
                last_name: None,
                email: None,
                password: None,
                tenant_id: None,
            },
            &mut session,
        );
        identity.save_group_in_session(
            Group {
                id: "g1".to_string(),
                name: "Group 1".to_string(),
                group_type: None,
            },
            &mut session,
        );
        identity.create_membership_in_session("u1".to_string(), "g1".to_string(), &mut session);
        identity.save_privilege_in_session(
            Privilege {
                id: "p1".to_string(),
                name: "Privilege 1".to_string(),
            },
            &mut session,
        );
        engine.get_runtime_store().insert_privilege_mapping(
            PrivilegeMapping {
                id: "m1".to_string(),
                privilege_id: "p1".to_string(),
                user_id: None,
                group_id: Some("g1".to_string()),
            },
            &mut session,
        );
        session.flush_and_commit().unwrap();
        let mut session = engine.get_runtime_store().create_session().unwrap();
        let privileges = identity.get_privileges_for_user_in_session("u1", &mut session);
        assert_eq!(
            privileges.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(),
            vec!["p1"]
        );
    }
}
