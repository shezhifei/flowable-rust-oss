use crate::{config::RestAuthConfig, error::ApiError};
use axum::{
    extract::{Extension, Request, State},
    http::header,
    middleware::Next,
    response::Response,
};
use base64::Engine;
use flowable_engine::engine::process_engine::ProcessEngine;
use std::sync::Arc;

pub async fn auth_middleware(
    State(auth): State<Arc<RestAuthConfig>>,
    Extension(engine): Extension<Arc<ProcessEngine>>,
    req: Request,
    next: Next,
) -> Result<Response, ApiError> {
    if !auth.mode.is_enforced() {
        return Ok(next.run(req).await);
    }

    let auth_header = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("");

    if !auth_header.starts_with("Basic ") {
        return Err(ApiError::Unauthorized);
    }

    let encoded = auth_header.trim_start_matches("Basic ");
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|_| ApiError::Unauthorized)?;
    let decoded = String::from_utf8(decoded).map_err(|_| ApiError::Unauthorized)?;

    let mut parts = decoded.splitn(2, ':');
    let user_id = parts.next().unwrap_or_default();
    let password = parts.next().unwrap_or_default();
    if user_id.is_empty() || password.is_empty() {
        return Err(ApiError::Unauthorized);
    }

    if engine
        .get_identity_service()
        .check_password(user_id, password)
    {
        Ok(next.run(req).await)
    } else {
        Err(ApiError::Unauthorized)
    }
}
