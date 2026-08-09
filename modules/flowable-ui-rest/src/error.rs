//! Error shape for the UI REST surface.
//!
//! The AngularJS apps parse `{ message, messageKey }` (Java's
//! `org.flowable.ui.common.service.exception.ErrorInfo`, emitted by
//! `RestExceptionHandlerAdvice`). That is *not* the shape used by the engine
//! REST surface in `flowable-rest` (`{ code, message, details }`), so the UI
//! surface carries its own error type rather than reusing `ApiError`.
//!
//! Message keys are copied verbatim from `RestExceptionHandlerAdvice`; the
//! frontends use them for i18n lookups.

use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Serialize;

const UNAUTHORIZED_MESSAGE_KEY: &str = "GENERAL.ERROR.UNAUTHORIZED";
const NOT_FOUND_MESSAGE_KEY: &str = "GENERAL.ERROR.NOT-FOUND";
const BAD_REQUEST_MESSAGE_KEY: &str = "GENERAL.ERROR.BAD-REQUEST";
const INTERNAL_SERVER_ERROR_MESSAGE_KEY: &str = "GENERAL.ERROR.INTERNAL-SERVER_ERROR";
const FORBIDDEN_MESSAGE_KEY: &str = "GENERAL.ERROR.FORBIDDEN";

/// Java's `ErrorInfo`. `messageKey` is `@JsonInclude(NON_NULL)` there, so it is
/// skipped when absent here too.
#[derive(Debug, Serialize)]
pub struct ErrorInfo {
    /// `getMessage()` carries no `@JsonInclude`, so a null message is emitted as
    /// `"message": null` rather than omitted. `new NotFoundException()` — used by
    /// several idm endpoints — leaves it null, so this has to be nullable to
    /// match.
    pub message: Option<String>,
    #[serde(rename = "messageKey", skip_serializing_if = "Option::is_none")]
    pub message_key: Option<String>,
}

#[derive(Debug)]
pub enum UiError {
    /// HTTP 401 — Java throws `UnauthorizedException`.
    Unauthorized(String),
    /// HTTP 403 — Java throws `NotPermittedException`.
    Forbidden(String),
    /// HTTP 404 — Java throws `NotFoundException`; `None` is its no-argument
    /// constructor, which serialises `"message": null`.
    NotFound(Option<String>),
    /// HTTP 400 — Java throws `BadRequestException`.
    BadRequest(String),
    /// HTTP 409 — Java throws `ConflictingRequestException`, which carries a
    /// caller-supplied message key such as
    /// `ACCOUNT.SIGNUP.ERROR.ALREADY-REGISTERED` and falls back to the
    /// bad-request key.
    Conflict {
        message: String,
        message_key: Option<String>,
    },
    /// HTTP 500 — Java throws `InternalServerErrorException`.
    Internal(String),
    /// HTTP 404 with an empty body, for endpoints that declare a non-JSON content
    /// type (Java's `NonJsonResourceNotFoundException`, whose handler returns
    /// no body). Used by the profile-picture endpoint.
    NotFoundNoBody,
}

impl UiError {
    /// Java's `new NotFoundException()` — no message.
    pub fn not_found() -> Self {
        Self::NotFound(None)
    }

    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::BadRequest(message.into())
    }

    pub fn conflict(message: impl Into<String>, message_key: impl Into<String>) -> Self {
        Self::Conflict {
            message: message.into(),
            message_key: Some(message_key.into()),
        }
    }
}

impl IntoResponse for UiError {
    fn into_response(self) -> Response {
        let (status, message, message_key) = match self {
            UiError::Unauthorized(message) => (
                StatusCode::UNAUTHORIZED,
                Some(message),
                Some(UNAUTHORIZED_MESSAGE_KEY),
            ),
            UiError::Forbidden(message) => (
                StatusCode::FORBIDDEN,
                Some(message),
                Some(FORBIDDEN_MESSAGE_KEY),
            ),
            UiError::NotFound(message) => {
                (StatusCode::NOT_FOUND, message, Some(NOT_FOUND_MESSAGE_KEY))
            }
            UiError::BadRequest(message) => (
                StatusCode::BAD_REQUEST,
                Some(message),
                Some(BAD_REQUEST_MESSAGE_KEY),
            ),
            UiError::Conflict {
                message,
                message_key,
            } => {
                let key = message_key.unwrap_or_else(|| BAD_REQUEST_MESSAGE_KEY.to_string());
                return (
                    StatusCode::CONFLICT,
                    Json(ErrorInfo {
                        message: Some(message),
                        message_key: Some(key),
                    }),
                )
                    .into_response();
            }
            // Never echo internal error text to clients; log it and return a
            // fixed public message. Matches the engine REST surface's policy.
            UiError::Internal(message) => {
                tracing::error!(error = %message, "UI REST internal server error");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Some("Internal server error".to_string()),
                    Some(INTERNAL_SERVER_ERROR_MESSAGE_KEY),
                )
            }
            UiError::NotFoundNoBody => return StatusCode::NOT_FOUND.into_response(),
        };

        (
            status,
            Json(ErrorInfo {
                message,
                message_key: message_key.map(str::to_string),
            }),
        )
            .into_response()
    }
}
