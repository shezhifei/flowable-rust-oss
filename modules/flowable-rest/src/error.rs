use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub code: String,
    pub message: String,
    pub details: Option<String>,
}

#[derive(Debug)]
pub enum ApiError {
    Unauthorized,
    Forbidden(String),
    NotFound(String),
    BadRequest(String),
    Conflict(String),
    InternalServerError(String),
}

impl ApiError {
    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::BadRequest(message.into())
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, code_str, error_message, details) = match self {
            ApiError::Unauthorized => (
                StatusCode::UNAUTHORIZED,
                "UNAUTHORIZED",
                "Unauthorized",
                None,
            ),
            ApiError::Forbidden(msg) => {
                (StatusCode::FORBIDDEN, "FORBIDDEN", "Forbidden", Some(msg))
            }
            ApiError::NotFound(msg) => (StatusCode::NOT_FOUND, "NOT_FOUND", "Not Found", Some(msg)),
            ApiError::BadRequest(msg) => (
                StatusCode::BAD_REQUEST,
                "BAD_REQUEST",
                "Bad Request",
                Some(msg),
            ),
            ApiError::Conflict(msg) => (StatusCode::CONFLICT, "CONFLICT", "Conflict", Some(msg)),
            ApiError::InternalServerError(msg) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "INTERNAL_SERVER_ERROR",
                "Internal Server Error",
                Some(msg),
            ),
        };

        let body = Json(ErrorResponse {
            code: code_str.to_string(),
            message: error_message.to_string(),
            details,
        });

        (status, body).into_response()
    }
}

impl From<flowable_engine::error::FlowableError> for ApiError {
    fn from(err: flowable_engine::error::FlowableError) -> Self {
        match err {
            flowable_engine::error::FlowableError::DeploymentValidationError(msg) => {
                ApiError::BadRequest(msg)
            }
            flowable_engine::error::FlowableError::BadRequest(msg) => ApiError::BadRequest(msg),
            flowable_engine::error::FlowableError::Forbidden(msg) => ApiError::Forbidden(msg),
            flowable_engine::error::FlowableError::Conflict(msg) => ApiError::Conflict(msg),
            flowable_engine::error::FlowableError::NotFound(msg) => ApiError::NotFound(msg),
            // Java parity: `FlowableException.getMessage()` is the raw message,
            // so the 500 details must not carry the Rust Display
            // `Execution error:` prefix.
            flowable_engine::error::FlowableError::ExecutionError(msg) => {
                ApiError::InternalServerError(msg)
            }
            _ => ApiError::InternalServerError(err.to_string()),
        }
    }
}

impl From<flowable_dmn_engine::DmnError> for ApiError {
    fn from(err: flowable_dmn_engine::DmnError) -> Self {
        match err {
            flowable_dmn_engine::DmnError::Validation { message }
            | flowable_dmn_engine::DmnError::UnsupportedModel { message, .. } => {
                ApiError::BadRequest(message)
            }
            flowable_dmn_engine::DmnError::NotFound { message } => ApiError::NotFound(message),
            flowable_dmn_engine::DmnError::Execution { message } => ApiError::BadRequest(message),
            flowable_dmn_engine::DmnError::Storage { message } => {
                ApiError::InternalServerError(message)
            }
        }
    }
}

impl From<flowable_cmmn_engine::CmmnError> for ApiError {
    fn from(err: flowable_cmmn_engine::CmmnError) -> Self {
        match err {
            flowable_cmmn_engine::CmmnError::Validation { message }
            | flowable_cmmn_engine::CmmnError::UnsupportedModel { message, .. }
            | flowable_cmmn_engine::CmmnError::Execution { message } => {
                ApiError::BadRequest(message)
            }
            flowable_cmmn_engine::CmmnError::Conflict { message } => ApiError::Conflict(message),
            flowable_cmmn_engine::CmmnError::NotFound { message } => ApiError::NotFound(message),
            flowable_cmmn_engine::CmmnError::Storage { message } => {
                ApiError::InternalServerError(message)
            }
            flowable_cmmn_engine::CmmnError::NonUniqueResult { query, count } => {
                ApiError::Conflict(format!("non-unique result for {query}: {count} matches"))
            }
        }
    }
}

impl From<flowable_app_engine::AppError> for ApiError {
    fn from(err: flowable_app_engine::AppError) -> Self {
        match err {
            flowable_app_engine::AppError::Validation { message }
            | flowable_app_engine::AppError::Execution { message } => ApiError::BadRequest(message),
            flowable_app_engine::AppError::Unsupported { .. } => {
                ApiError::BadRequest(err.to_string())
            }
            flowable_app_engine::AppError::NotFound { message } => ApiError::NotFound(message),
            flowable_app_engine::AppError::Storage { message } => {
                ApiError::InternalServerError(message)
            }
        }
    }
}

impl From<flowable_dmn_image_generator::DmnSvgGeneratorError> for ApiError {
    fn from(err: flowable_dmn_image_generator::DmnSvgGeneratorError) -> Self {
        match err {
            flowable_dmn_image_generator::DmnSvgGeneratorError::UnsupportedOptions { .. }
            | flowable_dmn_image_generator::DmnSvgGeneratorError::Structural(_) => {
                ApiError::BadRequest(err.to_string())
            }
            flowable_dmn_image_generator::DmnSvgGeneratorError::NotFound { .. } => {
                ApiError::NotFound(err.to_string())
            }
        }
    }
}

impl From<flowable_cmmn_image_generator::CmmnSvgGeneratorError> for ApiError {
    fn from(err: flowable_cmmn_image_generator::CmmnSvgGeneratorError) -> Self {
        match err {
            flowable_cmmn_image_generator::CmmnSvgGeneratorError::UnsupportedOptions { .. }
            | flowable_cmmn_image_generator::CmmnSvgGeneratorError::Structural(_) => {
                ApiError::BadRequest(err.to_string())
            }
            flowable_cmmn_image_generator::CmmnSvgGeneratorError::NotFound { .. } => {
                ApiError::NotFound(err.to_string())
            }
        }
    }
}

impl From<flowable_image_generator::ProcessDiagramSvgError> for ApiError {
    fn from(err: flowable_image_generator::ProcessDiagramSvgError) -> Self {
        match err {
            flowable_image_generator::ProcessDiagramSvgError::Layout(_) => {
                ApiError::InternalServerError(err.to_string())
            }
            flowable_image_generator::ProcessDiagramSvgError::UnsupportedOption { .. } => {
                ApiError::BadRequest(err.to_string())
            }
        }
    }
}

impl From<flowable_image_generator::SvgRasterizationError> for ApiError {
    fn from(err: flowable_image_generator::SvgRasterizationError) -> Self {
        ApiError::InternalServerError(err.to_string())
    }
}
