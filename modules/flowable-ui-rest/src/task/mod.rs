//! Task UI app (`/app/rest/**`) — stream B aggregation layer.
//!
//! B0: health probe. B2: RestVariable contract + task/process aggregation.

mod rest_variable;

use axum::{routing::get, Json, Router};
use serde_json::json;

pub use rest_variable::{
    create_rest_variable, rest_variable_value, RestVariable, RestVariableScope,
};

/// Build the task router (mounted under `/app` by `ui_router`).
pub fn router() -> Router {
    Router::new()
        .route("/app/rest/health", get(health))
        // Placeholders so path inventory is visible; filled in B2.
        .route("/app/rest/tasks", axum::routing::post(tasks_not_implemented))
        .route(
            "/app/rest/tasks/:task_id",
            get(tasks_not_implemented),
        )
}

async fn health() -> Json<serde_json::Value> {
    Json(json!({ "status": "ok", "app": "task" }))
}

async fn tasks_not_implemented() -> (axum::http::StatusCode, Json<serde_json::Value>) {
    (
        axum::http::StatusCode::NOT_IMPLEMENTED,
        Json(json!({
            "message": "Task aggregation endpoints are implemented in B2 (in progress)"
        })),
    )
}
