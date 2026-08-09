//! `/idm-app/rest/**` — the idm app's REST surface.
//!
//! Java blueprint: `$J/modules/flowable-ui/flowable-ui-idm-rest/.../idm/rest/app/`
//! for the endpoints, `$J/modules/flowable-ui/flowable-ui-idm-logic/.../service/`
//! for the behaviour behind them, and
//! `$J/modules/flowable-ui/flowable-ui-common/.../model/` for the wire types.
//!
//! The `/api/idm/**` surface (`ApiUsersResource`, `ApiGroupsResource`,
//! `ApiTokensResource`) is deliberately absent: the engine REST layer already
//! exposes equivalent Basic-authenticated endpoints under `/identity/**`
//! (`flowable-rest::routes::idm`).
//!
//! Response bodies are matched field for field against the Java
//! representations. Two things follow from Jackson's defaults, which the UI apps
//! never override: `null` fields are **emitted** (`Include.ALWAYS`), and
//! property names come from the getters, so they are camelCase. `Option` fields
//! here therefore carry no `skip_serializing_if`.
//!
//! Deviations from Java, all deliberate and each marked at its site:
//!
//! * `GET /rest/admin/groups/{id}/users` defaults `page`/`pageSize` to `0`/`50`
//!   instead of dereferencing them. The Java resource computes
//!   `page * pageSize` on the boxed parameters before its service layer applies
//!   those same defaults, so a request omitting them — which the frontend does
//!   emit, both parameters being conditional — throws `NullPointerException`.
//! * `POST /rest/admin/profile-password` compares the current password through
//!   argon2id verification. Java compares `user.getPassword().equals(...)`
//!   against a plaintext column; this port hashes passwords, so string equality
//!   would reject every correct password.
//! * Filtering, sorting and paging happen in this layer. The engine's
//!   `UserQuery`/`GroupQuery` support neither `LIKE` nor paging, and the port
//!   does not extend them for a UI concern.

use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{Multipart, Path, Query},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::engine::query::Query as _;
use flowable_engine::identity::entities::{Group, User};
use flowable_engine_common::like::sql_like_matches;
use serde::{Deserialize, Serialize};

use crate::auth::UiAuth;
use crate::error::UiError;

/// Java `GroupTypes.TYPE_ASSIGNMENT`, the fallback when a group is created
/// without a type.
const TYPE_ASSIGNMENT: &str = "assignment";

/// Java `UserServiceImpl.MAX_USER_SIZE`. The user list is capped at this many
/// rows per page; the resource has no page-size parameter.
const MAX_USER_SIZE: usize = 100;

/// Java `GroupServiceImpl.getGroupUsers` default page size.
const DEFAULT_GROUP_USERS_PAGE_SIZE: usize = 50;

type EngineState = axum::Extension<Arc<ProcessEngine>>;

// ── Wire types ──

/// Java `UserRepresentation`.
#[derive(Debug, Serialize)]
pub struct UserRepresentation {
    pub id: String,
    #[serde(rename = "firstName")]
    pub first_name: Option<String>,
    #[serde(rename = "lastName")]
    pub last_name: Option<String>,
    pub email: Option<String>,
    #[serde(rename = "fullName")]
    pub full_name: String,
    #[serde(rename = "tenantId")]
    pub tenant_id: Option<String>,
    pub groups: Vec<GroupRepresentation>,
    pub privileges: Vec<String>,
}

impl UserRepresentation {
    /// Java's `UserRepresentation(User)` constructor.
    ///
    /// `fullName` is `first + " " + last` with each null half replaced by the
    /// empty string, so a user with no names serialises as a single space rather
    /// than an empty string. `tenantId` is normalised to null when empty by the
    /// setter.
    fn from_user(user: User) -> Self {
        let full_name = format!(
            "{} {}",
            user.first_name.clone().unwrap_or_default(),
            user.last_name.clone().unwrap_or_default()
        );
        Self {
            id: user.id,
            first_name: user.first_name,
            last_name: user.last_name,
            email: user.email,
            full_name,
            tenant_id: user.tenant_id.filter(|tenant| !tenant.is_empty()),
            groups: Vec::new(),
            privileges: Vec::new(),
        }
    }

    fn with_groups_and_privileges(
        mut self,
        groups: Vec<Group>,
        privileges: Vec<String>,
    ) -> Self {
        self.groups = groups
            .into_iter()
            .map(GroupRepresentation::from_group)
            .collect();
        self.privileges = privileges;
        self
    }
}

/// Java `GroupRepresentation`.
#[derive(Debug, Serialize, Deserialize)]
pub struct GroupRepresentation {
    pub id: Option<String>,
    pub name: Option<String>,
    #[serde(rename = "type")]
    pub group_type: Option<String>,
}

impl GroupRepresentation {
    fn from_group(group: Group) -> Self {
        Self {
            id: Some(group.id),
            name: Some(group.name),
            group_type: group.group_type,
        }
    }
}

/// Java `ResultListDataRepresentation`.
#[derive(Debug, Serialize)]
pub struct ResultListDataRepresentation<T> {
    pub size: i32,
    pub total: i64,
    pub start: i32,
    pub data: Vec<T>,
}

/// Java `PrivilegeRepresentation`.
///
/// `users` and `groups` stay null on the list endpoint (Java constructs those
/// entries with the two-argument constructor, leaving both fields unset) and are
/// populated on the single-privilege endpoint.
#[derive(Debug, Serialize)]
pub struct PrivilegeRepresentation {
    pub id: String,
    pub name: String,
    pub users: Option<Vec<UserRepresentation>>,
    pub groups: Option<Vec<GroupRepresentation>>,
}

pub fn router() -> Router {
    Router::new()
        .route("/idm-app/rest/authenticate", get(authenticate))
        .route("/idm-app/rest/account", get(account))
        .route(
            "/idm-app/rest/admin/users",
            get(list_users).post(create_user).put(bulk_update_users),
        )
        .route(
            "/idm-app/rest/admin/users/:user_id",
            axum::routing::put(update_user).delete(delete_user),
        )
        .route(
            "/idm-app/rest/admin/groups",
            get(list_groups).post(create_group),
        )
        .route(
            "/idm-app/rest/admin/groups/:group_id",
            get(get_group)
                .put(update_group)
                .delete(delete_group),
        )
        .route("/idm-app/rest/admin/groups/:group_id/users", get(group_users))
        .route(
            "/idm-app/rest/admin/groups/:group_id/members/:user_id",
            post(add_group_member).delete(delete_group_member),
        )
        .route("/idm-app/rest/admin/privileges", get(list_privileges))
        .route(
            "/idm-app/rest/admin/privileges/:privilege_id",
            get(get_privilege),
        )
        .route(
            "/idm-app/rest/admin/privileges/:privilege_id/users",
            get(privilege_users).post(add_user_privilege),
        )
        .route(
            "/idm-app/rest/admin/privileges/:privilege_id/users/:user_id",
            axum::routing::delete(delete_user_privilege),
        )
        .route(
            "/idm-app/rest/admin/privileges/:privilege_id/groups",
            get(privilege_groups).post(add_group_privilege),
        )
        .route(
            "/idm-app/rest/admin/privileges/:privilege_id/groups/:group_id",
            axum::routing::delete(delete_group_privilege),
        )
        .route("/idm-app/rest/admin/profile", get(get_profile).post(update_profile))
        .route("/idm-app/rest/admin/profile-password", post(change_password))
        .route(
            "/idm-app/rest/admin/profile-picture",
            get(get_profile_picture).post(upload_profile_picture),
        )
}

// ── Shared helpers ──

/// Java `UserServiceImpl.getUserInformation`: the user plus their groups plus the
/// **names** of every privilege reachable directly or through a group.
///
/// Returns `None` when the user row is gone; every caller turns that into a 404,
/// which is what Java's `NotFoundException` does.
fn user_information(engine: &Arc<ProcessEngine>, user_id: &str) -> Option<UserRepresentation> {
    let identity = engine.get_identity_service();
    let user = identity.find_user_by_id(user_id)?;
    let groups = identity.get_groups_by_user(user_id);

    // Java collects into a `HashSet<String>` of names, so duplicates across the
    // user's own grants and their groups' collapse. Sorted here because a set's
    // iteration order is not something to replicate.
    let mut privileges: Vec<String> = identity
        .get_privileges_for_user(user_id)
        .into_iter()
        .map(|privilege| privilege.name)
        .collect();
    privileges.sort();
    privileges.dedup();

    Some(UserRepresentation::from_user(user).with_groups_and_privileges(groups, privileges))
}

/// Java `UserQuery.userFullNameLikeIgnoreCase("%" + filter + "%")`.
///
/// The engine's `UserQuery` has no full-name or `LIKE` support, so the match runs
/// here over the same concatenation Java's SQL uses —
/// `lower(CONCAT(CONCAT(FIRST_, ' '), LAST_)) like ?` — with `to_lowercase` on
/// both sides standing in for `IgnoreCase`.
///
/// A null name half is coerced to the empty string. Java's behaviour for that
/// case is *database dependent*: MySQL's `CONCAT` returns null if any argument is
/// null, so a user with no surname can never be found by first name, while
/// PostgreSQL's `CONCAT` skips nulls and the same user matches. This follows
/// PostgreSQL — the same query answering differently per backend is not
/// behaviour worth reproducing, and being unable to find a half-named user is
/// the less useful of the two readings.
fn full_name_matches(user: &User, filter: &str) -> bool {
    let full_name = format!(
        "{} {}",
        user.first_name.clone().unwrap_or_default(),
        user.last_name.clone().unwrap_or_default()
    );
    sql_like_matches(
        &format!("%{}%", filter.to_lowercase()),
        &full_name.to_lowercase(),
    )
}

/// Java `UserServiceImpl.createUserQuery`'s `sort` parameter. Anything else —
/// including a missing value — leaves the order as the store returned it, which
/// is what Java does when no `orderBy` is applied.
fn sort_users(users: &mut [User], sort: Option<&str>) {
    match sort {
        Some("idAsc") => users.sort_by(|left, right| left.id.cmp(&right.id)),
        Some("idDesc") => users.sort_by(|left, right| right.id.cmp(&left.id)),
        Some("emailAsc") => users.sort_by(|left, right| {
            left.email
                .as_deref()
                .unwrap_or_default()
                .cmp(right.email.as_deref().unwrap_or_default())
        }),
        Some("emailDesc") => users.sort_by(|left, right| {
            right
                .email
                .as_deref()
                .unwrap_or_default()
                .cmp(left.email.as_deref().unwrap_or_default())
        }),
        _ => {}
    }
}

/// `listPage(start, size)` semantics: skip `start`, take `size`, tolerating a
/// `start` past the end.
fn page_slice<T>(items: Vec<T>, start: usize, size: usize) -> Vec<T> {
    items.into_iter().skip(start).take(size).collect()
}

// ── Account ──

/// `GET /idm-app/rest/authenticate`.
///
/// Java returns `{"login": <remoteUser>}` and nothing else. The path is public in
/// the security chain but the handler throws `UnauthorizedException` when there is
/// no user, so the extractor's rejection is the correct behaviour.
async fn authenticate(auth: UiAuth) -> Json<serde_json::Value> {
    Json(serde_json::json!({ "login": auth.scope().login }))
}

/// `GET /idm-app/rest/account`.
///
/// The `CurrentUserProvider` chain Java consults first is for externally managed
/// identities (Keycloak); with the default identity service none of the providers
/// support the authentication, so this falls straight through to
/// `getCurrentUserRepresentation`.
async fn account(
    auth: UiAuth,
    axum::Extension(engine): EngineState,
) -> Result<Json<UserRepresentation>, UiError> {
    user_information(&engine, auth.user_id())
        .map(Json)
        .ok_or_else(UiError::not_found)
}

// ── Users ──

#[derive(Debug, Deserialize)]
pub struct ListUsersQuery {
    filter: Option<String>,
    sort: Option<String>,
    start: Option<i32>,
    /// Accepted and ignored, exactly as Java does: `getUserCount` takes a
    /// `groupId` and never reads it, and the list query never receives it at all.
    #[allow(dead_code)]
    group_id: Option<String>,
}

/// `GET /idm-app/rest/admin/users`.
///
/// `total` is the count *before* paging but *after* filtering, and there is no
/// page-size parameter — the page is always at most [`MAX_USER_SIZE`].
async fn list_users(
    _auth: UiAuth,
    axum::Extension(engine): EngineState,
    Query(query): Query<ListUsersQuery>,
) -> Result<Json<ResultListDataRepresentation<UserRepresentation>>, UiError> {
    let start = query.start.unwrap_or(0).max(0);
    let mut users = engine
        .get_identity_service()
        .create_user_query()
        .list()
        .map_err(|error| UiError::Internal(format!("Could not list users: {error}")))?;

    if let Some(filter) = query.filter.as_deref().filter(|value| !value.is_empty()) {
        users.retain(|user| full_name_matches(user, filter));
    }
    sort_users(&mut users, query.sort.as_deref());

    let total = users.len() as i64;
    let page = page_slice(users, start as usize, MAX_USER_SIZE);

    Ok(Json(ResultListDataRepresentation {
        size: page.len() as i32,
        total,
        start,
        data: page.into_iter().map(UserRepresentation::from_user).collect(),
    }))
}

/// Java `CreateUserRepresentation`.
#[derive(Debug, Deserialize)]
pub struct CreateUserRequest {
    id: Option<String>,
    #[serde(rename = "firstName")]
    first_name: Option<String>,
    #[serde(rename = "lastName")]
    last_name: Option<String>,
    email: Option<String>,
    password: Option<String>,
    #[serde(rename = "tenantId")]
    tenant_id: Option<String>,
}

/// `POST /idm-app/rest/admin/users`.
///
/// Java returns the *pre-save* entity, so the response carries no password and
/// reflects exactly the fields that were set. The duplicate checks are two
/// separate 409s with the same message key.
async fn create_user(
    _auth: UiAuth,
    axum::Extension(engine): EngineState,
    Json(request): Json<CreateUserRequest>,
) -> Result<Json<UserRepresentation>, UiError> {
    let id = request.id.unwrap_or_default();
    let password = request.password.unwrap_or_default();
    let first_name = request.first_name.unwrap_or_default();

    // Java's `StringUtils.isBlank`: null, empty, or whitespace only.
    if id.trim().is_empty() || password.trim().is_empty() || first_name.trim().is_empty() {
        return Err(UiError::bad_request(
            "Id, password and first name are required",
        ));
    }

    let identity = engine.get_identity_service();

    if let Some(email) = request
        .email
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        let existing = identity
            .create_user_query()
            .email(email.to_string())
            .list()
            .map_err(|error| UiError::Internal(format!("Could not query users by email: {error}")))?;
        if !existing.is_empty() {
            return Err(UiError::conflict(
                "User already registered",
                "ACCOUNT.SIGNUP.ERROR.ALREADY-REGISTERED",
            ));
        }
    }

    if identity.find_user_by_id(&id).is_some() {
        return Err(UiError::conflict(
            "User already registered",
            "ACCOUNT.SIGNUP.ERROR.ALREADY-REGISTERED",
        ));
    }

    let user = User {
        id: id.clone(),
        first_name: Some(first_name),
        last_name: request.last_name,
        email: request.email,
        password: Some(password),
        tenant_id: request.tenant_id,
    };
    identity.save_user(user.clone());

    // Java hands back the in-memory entity, which still holds the plaintext
    // password field; the representation does not expose it, so the only
    // difference here is that the stored row is hashed.
    Ok(Json(UserRepresentation::from_user(User {
        password: None,
        ..user
    })))
}

/// Java `UpdateUsersRepresentation`, shared by the single and bulk endpoints.
#[derive(Debug, Deserialize)]
pub struct UpdateUsersRequest {
    #[serde(rename = "firstName")]
    first_name: Option<String>,
    #[serde(rename = "lastName")]
    last_name: Option<String>,
    email: Option<String>,
    #[serde(rename = "tenantId")]
    tenant_id: Option<String>,
    password: Option<String>,
    #[serde(default)]
    users: Vec<String>,
}

/// `PUT /idm-app/rest/admin/users/{userId}`.
///
/// Every field is overwritten with the request value, including with null — Java
/// calls the setters unconditionally. A missing user is silently ignored (the
/// service's `if (user != null)`), so this returns 200 either way.
async fn update_user(
    _auth: UiAuth,
    axum::Extension(engine): EngineState,
    Path(user_id): Path<String>,
    Json(request): Json<UpdateUsersRequest>,
) -> StatusCode {
    let identity = engine.get_identity_service();
    if let Some(user) = identity.find_user_by_id(&user_id) {
        identity.save_user(User {
            first_name: request.first_name,
            last_name: request.last_name,
            email: request.email,
            tenant_id: request.tenant_id,
            ..user
        });
    }
    StatusCode::OK
}

/// `PUT /idm-app/rest/admin/users`.
///
/// Bulk password reset. Java iterates the id list and skips ids that do not
/// resolve, so a partially valid list partially succeeds and still returns 200.
async fn bulk_update_users(
    _auth: UiAuth,
    axum::Extension(engine): EngineState,
    Json(request): Json<UpdateUsersRequest>,
) -> StatusCode {
    let identity = engine.get_identity_service();
    let Some(password) = request.password else {
        return StatusCode::OK;
    };
    for user_id in request.users {
        if let Some(user) = identity.find_user_by_id(&user_id) {
            identity.save_user(User {
                password: Some(password.clone()),
                ..user
            });
        }
    }
    StatusCode::OK
}

/// `DELETE /idm-app/rest/admin/users/{userId}`.
///
/// Cascades privilege mappings and group memberships first, mirroring
/// `UserServiceImpl.deleteUser`.
async fn delete_user(
    _auth: UiAuth,
    axum::Extension(engine): EngineState,
    Path(user_id): Path<String>,
) -> StatusCode {
    let identity = engine.get_identity_service();

    // Direct grants only. Java's `createPrivilegeQuery().userId(userId)` does not
    // include privileges the user merely inherits from a group, and those must
    // survive the user's deletion — they belong to the group.
    for privilege in identity.get_direct_privileges_for_user(&user_id) {
        identity.delete_user_privilege_mapping(&privilege.id, &user_id);
    }
    for group in identity.get_groups_by_user(&user_id) {
        identity.delete_membership(&user_id, &group.id);
    }
    identity.delete_user(&user_id);

    StatusCode::OK
}

// ── Groups ──

#[derive(Debug, Deserialize)]
pub struct FilterQuery {
    filter: Option<String>,
}

/// `GET /idm-app/rest/admin/groups`.
///
/// A bare array, not a `ResultListDataRepresentation` — this endpoint has no
/// paging. Always ordered by name ascending.
async fn list_groups(
    _auth: UiAuth,
    axum::Extension(engine): EngineState,
    Query(query): Query<FilterQuery>,
) -> Result<Json<Vec<GroupRepresentation>>, UiError> {
    let mut groups = engine
        .get_identity_service()
        .create_group_query()
        .list()
        .map_err(|error| UiError::Internal(format!("Could not list groups: {error}")))?;

    if let Some(filter) = query.filter.as_deref().filter(|value| !value.is_empty()) {
        let pattern = format!("%{}%", filter.to_lowercase());
        groups.retain(|group| sql_like_matches(&pattern, &group.name.to_lowercase()));
    }
    groups.sort_by(|left, right| left.name.cmp(&right.name));

    Ok(Json(
        groups
            .into_iter()
            .map(GroupRepresentation::from_group)
            .collect(),
    ))
}

/// `GET /idm-app/rest/admin/groups/{groupId}`.
///
/// Java wraps the service result unconditionally, so a missing group throws
/// `NullPointerException` inside the representation constructor and surfaces as a
/// 500. This returns a 404 instead: the frontend's error handling treats any
/// non-2xx the same way, and a 500 for a routine missing row is not behaviour
/// worth reproducing.
async fn get_group(
    _auth: UiAuth,
    axum::Extension(engine): EngineState,
    Path(group_id): Path<String>,
) -> Result<Json<GroupRepresentation>, UiError> {
    engine
        .get_identity_service()
        .find_group_by_id(&group_id)
        .map(|group| Json(GroupRepresentation::from_group(group)))
        .ok_or_else(UiError::not_found)
}

#[derive(Debug, Deserialize)]
pub struct GroupUsersQuery {
    filter: Option<String>,
    page: Option<i32>,
    #[serde(rename = "pageSize")]
    page_size: Option<i32>,
}

/// `GET /idm-app/rest/admin/groups/{groupId}/users`.
///
/// `page`/`pageSize` default to `0`/`50` here. Java's resource computes
/// `page * pageSize` on the boxed parameters to fill in `start`, before the
/// service layer applies those same defaults — so a request omitting either one
/// throws `NullPointerException`. The frontend does emit such requests, both
/// parameters being conditional, so the defaults are applied first instead.
async fn group_users(
    _auth: UiAuth,
    axum::Extension(engine): EngineState,
    Path(group_id): Path<String>,
    Query(query): Query<GroupUsersQuery>,
) -> Json<ResultListDataRepresentation<UserRepresentation>> {
    let page = query.page.unwrap_or(0).max(0);
    let page_size = query
        .page_size
        .unwrap_or(DEFAULT_GROUP_USERS_PAGE_SIZE as i32)
        .max(0);

    let mut users = engine.get_identity_service().get_users_by_group(&group_id);
    if let Some(filter) = query.filter.as_deref().filter(|value| !value.is_empty()) {
        users.retain(|user| full_name_matches(user, filter));
    }

    let total = users.len() as i64;
    let start = page.saturating_mul(page_size);
    let slice = page_slice(users, start as usize, page_size as usize);

    Json(ResultListDataRepresentation {
        size: slice.len() as i32,
        total,
        start,
        data: slice.into_iter().map(UserRepresentation::from_user).collect(),
    })
}

/// `POST /idm-app/rest/admin/groups`.
///
/// A null `type` becomes [`TYPE_ASSIGNMENT`]; a null `id` is stored as given,
/// which is what `identityService.newGroup(null)` produces.
async fn create_group(
    _auth: UiAuth,
    axum::Extension(engine): EngineState,
    Json(request): Json<GroupRepresentation>,
) -> Result<Json<GroupRepresentation>, UiError> {
    let Some(name) = request.name.filter(|value| !value.trim().is_empty()) else {
        return Err(UiError::bad_request("Group name required"));
    };

    let group = Group {
        id: request.id.unwrap_or_default(),
        name,
        group_type: Some(request.group_type.unwrap_or_else(|| TYPE_ASSIGNMENT.to_string())),
    };
    engine.get_identity_service().save_group(group.clone());

    Ok(Json(GroupRepresentation::from_group(group)))
}

/// `PUT /idm-app/rest/admin/groups/{groupId}`.
///
/// Only the name is updatable; the type is not reachable through this endpoint.
async fn update_group(
    _auth: UiAuth,
    axum::Extension(engine): EngineState,
    Path(group_id): Path<String>,
    Json(request): Json<GroupRepresentation>,
) -> Result<Json<GroupRepresentation>, UiError> {
    let Some(name) = request.name.filter(|value| !value.trim().is_empty()) else {
        return Err(UiError::bad_request("Group name required"));
    };

    let identity = engine.get_identity_service();
    let Some(group) = identity.find_group_by_id(&group_id) else {
        return Err(UiError::not_found());
    };

    let updated = Group { name, ..group };
    identity.save_group(updated.clone());

    Ok(Json(GroupRepresentation::from_group(updated)))
}

/// `DELETE /idm-app/rest/admin/groups/{groupId}`.
async fn delete_group(
    _auth: UiAuth,
    axum::Extension(engine): EngineState,
    Path(group_id): Path<String>,
) -> Result<StatusCode, UiError> {
    let identity = engine.get_identity_service();
    if identity.find_group_by_id(&group_id).is_none() {
        return Err(UiError::not_found());
    }
    identity.delete_group(&group_id);
    Ok(StatusCode::OK)
}

/// `POST /idm-app/rest/admin/groups/{groupId}/members/{userId}`.
///
/// 404 unless both the group and the user exist.
async fn add_group_member(
    _auth: UiAuth,
    axum::Extension(engine): EngineState,
    Path((group_id, user_id)): Path<(String, String)>,
) -> Result<StatusCode, UiError> {
    let identity = engine.get_identity_service();
    if identity.find_group_by_id(&group_id).is_none()
        || identity.find_user_by_id(&user_id).is_none()
    {
        return Err(UiError::not_found());
    }
    identity.create_membership(user_id, group_id);
    Ok(StatusCode::OK)
}

/// `DELETE /idm-app/rest/admin/groups/{groupId}/members/{userId}`.
async fn delete_group_member(
    _auth: UiAuth,
    axum::Extension(engine): EngineState,
    Path((group_id, user_id)): Path<(String, String)>,
) -> Result<StatusCode, UiError> {
    let identity = engine.get_identity_service();
    if identity.find_group_by_id(&group_id).is_none()
        || identity.find_user_by_id(&user_id).is_none()
    {
        return Err(UiError::not_found());
    }
    identity.delete_membership(&user_id, &group_id);
    Ok(StatusCode::OK)
}

// ── Privileges ──

/// `GET /idm-app/rest/admin/privileges`.
///
/// `users` and `groups` are null on every entry: Java builds these with the
/// two-argument constructor and never populates the collections.
async fn list_privileges(
    _auth: UiAuth,
    axum::Extension(engine): EngineState,
) -> Json<Vec<PrivilegeRepresentation>> {
    Json(
        engine
            .get_identity_service()
            .list_privileges()
            .into_iter()
            .map(|privilege| PrivilegeRepresentation {
                id: privilege.id,
                name: privilege.name,
                users: None,
                groups: None,
            })
            .collect(),
    )
}

/// Java's `getPrivilege`, shared by three endpoints — the single-privilege view
/// and the two collection views, which Java implements by calling it and
/// returning one field.
fn privilege_detail(
    engine: &Arc<ProcessEngine>,
    privilege_id: &str,
) -> Result<PrivilegeRepresentation, UiError> {
    let identity = engine.get_identity_service();
    let Some(privilege) = identity.find_privilege_by_id(privilege_id) else {
        return Err(UiError::not_found());
    };

    let (user_ids, group_ids) = identity.get_privilege_mapping_ids(privilege_id);

    // Mappings can outlive the rows they point at; Java's join drops those, so
    // unresolvable ids are skipped rather than surfacing as empty entries.
    let users = user_ids
        .into_iter()
        .filter_map(|user_id| identity.find_user_by_id(&user_id))
        .map(UserRepresentation::from_user)
        .collect();
    let groups = group_ids
        .into_iter()
        .filter_map(|group_id| identity.find_group_by_id(&group_id))
        .map(GroupRepresentation::from_group)
        .collect();

    Ok(PrivilegeRepresentation {
        id: privilege.id,
        name: privilege.name,
        users: Some(users),
        groups: Some(groups),
    })
}

/// `GET /idm-app/rest/admin/privileges/{privilegeId}`.
async fn get_privilege(
    _auth: UiAuth,
    axum::Extension(engine): EngineState,
    Path(privilege_id): Path<String>,
) -> Result<Json<PrivilegeRepresentation>, UiError> {
    privilege_detail(&engine, &privilege_id).map(Json)
}

/// `GET /idm-app/rest/admin/privileges/{privilegeId}/users`.
async fn privilege_users(
    _auth: UiAuth,
    axum::Extension(engine): EngineState,
    Path(privilege_id): Path<String>,
) -> Result<Json<Vec<UserRepresentation>>, UiError> {
    Ok(Json(
        privilege_detail(&engine, &privilege_id)?
            .users
            .unwrap_or_default(),
    ))
}

/// `GET /idm-app/rest/admin/privileges/{privilegeId}/groups`.
async fn privilege_groups(
    _auth: UiAuth,
    axum::Extension(engine): EngineState,
    Path(privilege_id): Path<String>,
) -> Result<Json<Vec<GroupRepresentation>>, UiError> {
    Ok(Json(
        privilege_detail(&engine, &privilege_id)?
            .groups
            .unwrap_or_default(),
    ))
}

/// Java `AddUserPrivilegeRepresentation`.
#[derive(Debug, Deserialize)]
pub struct AddUserPrivilegeRequest {
    #[serde(rename = "userId")]
    user_id: Option<String>,
}

/// Java `AddGroupPrivilegeRepresentation`.
#[derive(Debug, Deserialize)]
pub struct AddGroupPrivilegeRequest {
    #[serde(rename = "groupId")]
    group_id: Option<String>,
}

/// `POST /idm-app/rest/admin/privileges/{privilegeId}/users`.
///
/// Java's `isUserPrivilege` throws `IllegalArgumentException("Invalid user id")`
/// for an unknown user, which the advice does not handle — it escapes as a 500.
/// A 400 is returned instead; the id came from the request, so the fault is the
/// caller's.
///
/// Granting an already-granted privilege is a no-op, not an error.
async fn add_user_privilege(
    _auth: UiAuth,
    axum::Extension(engine): EngineState,
    Path(privilege_id): Path<String>,
    Json(request): Json<AddUserPrivilegeRequest>,
) -> Result<StatusCode, UiError> {
    let identity = engine.get_identity_service();
    let user_id = request.user_id.unwrap_or_default();

    if identity.find_user_by_id(&user_id).is_none() {
        return Err(UiError::bad_request("Invalid user id"));
    }

    let (existing_users, _) = identity.get_privilege_mapping_ids(&privilege_id);
    if !existing_users.contains(&user_id) {
        identity.add_user_privilege_mapping(privilege_id, user_id);
    }
    Ok(StatusCode::OK)
}

/// `DELETE /idm-app/rest/admin/privileges/{privilegeId}/users/{userId}`.
async fn delete_user_privilege(
    _auth: UiAuth,
    axum::Extension(engine): EngineState,
    Path((privilege_id, user_id)): Path<(String, String)>,
) -> Result<StatusCode, UiError> {
    let identity = engine.get_identity_service();
    if identity.find_user_by_id(&user_id).is_none() {
        return Err(UiError::bad_request("Invalid user id"));
    }
    identity.delete_user_privilege_mapping(&privilege_id, &user_id);
    Ok(StatusCode::OK)
}

/// `POST /idm-app/rest/admin/privileges/{privilegeId}/groups`.
async fn add_group_privilege(
    _auth: UiAuth,
    axum::Extension(engine): EngineState,
    Path(privilege_id): Path<String>,
    Json(request): Json<AddGroupPrivilegeRequest>,
) -> Result<StatusCode, UiError> {
    let identity = engine.get_identity_service();
    let group_id = request.group_id.unwrap_or_default();

    if identity.find_group_by_id(&group_id).is_none() {
        return Err(UiError::bad_request("Invalid group id"));
    }

    let (_, existing_groups) = identity.get_privilege_mapping_ids(&privilege_id);
    if !existing_groups.contains(&group_id) {
        identity.add_group_privilege_mapping(privilege_id, group_id);
    }
    Ok(StatusCode::OK)
}

/// `DELETE /idm-app/rest/admin/privileges/{privilegeId}/groups/{groupId}`.
async fn delete_group_privilege(
    _auth: UiAuth,
    axum::Extension(engine): EngineState,
    Path((privilege_id, group_id)): Path<(String, String)>,
) -> Result<StatusCode, UiError> {
    let identity = engine.get_identity_service();
    if identity.find_group_by_id(&group_id).is_none() {
        return Err(UiError::bad_request("Invalid group id"));
    }
    identity.delete_group_privilege_mapping(&privilege_id, &group_id);
    Ok(StatusCode::OK)
}

// ── Profile ──

/// `GET /idm-app/rest/admin/profile`.
///
/// Same body as `/rest/account`, for the current user. Both exist because the
/// profile screen and the shell load independently.
async fn get_profile(
    auth: UiAuth,
    axum::Extension(engine): EngineState,
) -> Result<Json<UserRepresentation>, UiError> {
    user_information(&engine, auth.user_id())
        .map(Json)
        .ok_or_else(UiError::not_found)
}

/// `POST /idm-app/rest/admin/profile`.
///
/// Updates the *current* user only — the id in the body is ignored. `tenantId` is
/// not updatable here, unlike through the admin users endpoint.
async fn update_profile(
    auth: UiAuth,
    axum::Extension(engine): EngineState,
    Json(request): Json<UpdateUsersRequest>,
) -> Result<Json<UserRepresentation>, UiError> {
    // The email doubles as a login identifier for locally managed users, so an
    // empty one is rejected before anything is written.
    let Some(email) = request.email.filter(|value| !value.is_empty()) else {
        return Err(UiError::bad_request("Empty email is not allowed"));
    };

    let identity = engine.get_identity_service();
    let Some(user) = identity.find_user_by_id(auth.user_id()) else {
        return Err(UiError::not_found());
    };

    let updated = User {
        first_name: request.first_name,
        last_name: request.last_name,
        email: Some(email),
        ..user
    };
    identity.save_user(updated.clone());

    Ok(Json(UserRepresentation::from_user(updated)))
}

/// Java `ChangePasswordRepresentation`.
#[derive(Debug, Deserialize)]
pub struct ChangePasswordRequest {
    #[serde(rename = "originalPassword")]
    original_password: Option<String>,
    #[serde(rename = "newPassword")]
    new_password: Option<String>,
}

/// `POST /idm-app/rest/admin/profile-password`.
///
/// A wrong current password is a **404**, not a 401 or 403 — Java throws
/// `NotFoundException` there, and the frontend keys its error message off that
/// status.
///
/// The comparison goes through argon2id verification. Java compares
/// `user.getPassword().equals(originalPassword)` against a plaintext column; this
/// port stores hashes, so string equality would reject every correct password.
async fn change_password(
    auth: UiAuth,
    axum::Extension(engine): EngineState,
    Json(request): Json<ChangePasswordRequest>,
) -> Result<StatusCode, UiError> {
    let identity = engine.get_identity_service();
    let Some(user) = identity.find_user_by_id(auth.user_id()) else {
        return Err(UiError::not_found());
    };

    let original = request.original_password.unwrap_or_default();
    if !identity.check_password(auth.user_id(), &original) {
        return Err(UiError::not_found());
    }

    identity.save_user(User {
        password: Some(request.new_password.unwrap_or_default()),
        ..user
    });
    Ok(StatusCode::OK)
}

/// `GET /idm-app/rest/admin/profile-picture`.
///
/// Streams the stored bytes under their recorded content type. Java has no
/// picture at all for a fresh user, so the 404 path is the common one.
///
/// The 404 carries a JSON `ErrorInfo` body: Java throws the ordinary
/// `NotFoundException` here, not `NonJsonResourceNotFoundException`, even though
/// the success path writes raw bytes.
async fn get_profile_picture(
    auth: UiAuth,
    axum::Extension(engine): EngineState,
) -> Result<Response, UiError> {
    let Some(picture) = engine.get_identity_service().get_user_picture(auth.user_id()) else {
        return Err(UiError::not_found());
    };

    let content_type = header::HeaderValue::from_str(&picture.mime_type)
        .unwrap_or_else(|_| header::HeaderValue::from_static("application/octet-stream"));

    Ok(([(header::CONTENT_TYPE, content_type)], picture.bytes).into_response())
}

/// `POST /idm-app/rest/admin/profile-picture`.
///
/// Multipart, field name `file`. Java reads the part's own content type and
/// stores it alongside the bytes.
async fn upload_profile_picture(
    auth: UiAuth,
    axum::Extension(engine): EngineState,
    mut multipart: Multipart,
) -> Result<StatusCode, UiError> {
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|error| UiError::Internal(format!("Could not read multipart body: {error}")))?
    {
        if field.name() != Some("file") {
            continue;
        }

        let content_type = field
            .content_type()
            .unwrap_or("application/octet-stream")
            .to_string();
        let bytes = field
            .bytes()
            .await
            .map_err(|error| UiError::Internal(format!("Could not read uploaded file: {error}")))?;

        engine.get_identity_service().set_user_picture(
            auth.user_id().to_string(),
            content_type,
            bytes.to_vec(),
        );
        return Ok(StatusCode::OK);
    }

    // Java's `@RequestParam("file")` is required, so a body without that part is
    // a binding failure rather than a silent success.
    Err(UiError::bad_request("No file content was found in request"))
}