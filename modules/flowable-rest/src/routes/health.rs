use axum::response::IntoResponse;

pub async fn health() -> impl IntoResponse {
    "UP"
}

pub async fn ready() -> impl IntoResponse {
    "READY"
}
