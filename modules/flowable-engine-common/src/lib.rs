pub mod el;
pub mod like;

pub use el::{
    Expression, ExpressionMethodRegistry, MapVariableContainer, SimpleExpression,
    VariableContainer, with_expression_method_registry,
};

use std::borrow::Cow;

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub enum FlowableError {
    InvalidBpmnXml {
        position: u64,
        message: String,
    },
    UnsupportedElement {
        element_type: String,
        activity_id: String,
    },
    DeploymentValidationError(String),
    /// A caller supplied an invalid request or an operation is not valid for
    /// the current engine state. REST adapters expose this as HTTP 400.
    BadRequest(String),
    /// The caller is authenticated but does not own the locked resource.
    /// REST adapters expose this as HTTP 403.
    Forbidden(String),
    /// The request conflicts with the current engine state (e.g. creating a
    /// variable that already exists). Equivalent of Flowable Java's
    /// `FlowableConflictException`; REST adapters expose this as HTTP 409.
    Conflict(String),
    ExecutionError(String),
    /// Equivalent of Flowable Java's `FlowableUnrecoverableJobException`.
    /// Async job execution moves directly to dead-letter without consuming
    /// the remaining retry schedule.
    UnrecoverableJobError(String),
    NotFound(String),
    Internal(String),
    Generic(String),
    Caused(FlowableErrorCause),
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct FlowableErrorCause {
    outer: Box<FlowableError>,
    cause: Box<FlowableError>,
}

impl FlowableErrorCause {
    pub fn outer(&self) -> &FlowableError {
        &self.outer
    }

    pub fn cause(&self) -> &FlowableError {
        &self.cause
    }
}

impl FlowableError {
    pub fn caused_by(self, cause: FlowableError) -> Self {
        Self::Caused(FlowableErrorCause {
            outer: Box::new(self),
            cause: Box::new(cause),
        })
    }

    pub fn primary_error(&self) -> &Self {
        match self {
            Self::Caused(chain) => chain.outer().primary_error(),
            _ => self,
        }
    }

    pub fn raw_primary_message(&self) -> Cow<'_, str> {
        let primary = self.primary_error();
        match primary {
            Self::InvalidBpmnXml { message, .. }
            | Self::DeploymentValidationError(message)
            | Self::BadRequest(message)
            | Self::Forbidden(message)
            | Self::Conflict(message)
            | Self::ExecutionError(message)
            | Self::UnrecoverableJobError(message)
            | Self::NotFound(message)
            | Self::Internal(message)
            | Self::Generic(message) => Cow::Borrowed(message),
            Self::UnsupportedElement { .. } => Cow::Owned(primary.to_string()),
            Self::Caused(_) => unreachable!("primary_error removes cause wrappers"),
        }
    }

    pub fn is_unrecoverable_job_failure(&self) -> bool {
        match self {
            Self::UnrecoverableJobError(_) => true,
            Self::Caused(chain) => {
                chain.outer().is_unrecoverable_job_failure()
                    || chain.cause().is_unrecoverable_job_failure()
            }
            _ => false,
        }
    }
}

impl std::fmt::Display for FlowableError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidBpmnXml { position, message } => write!(
                f,
                "Invalid BPMN XML at byte position {}: {}",
                position, message
            ),
            Self::UnsupportedElement {
                element_type,
                activity_id,
            } => write!(
                f,
                "unsupported activity behavior: {} (activity id: {})",
                element_type, activity_id
            ),
            Self::DeploymentValidationError(msg) => {
                write!(f, "Deployment validation error: {}", msg)
            }
            // These variants refine transport classification without changing
            // the legacy service-level error text produced by ExecutionError.
            Self::BadRequest(msg) | Self::Forbidden(msg) | Self::ExecutionError(msg) => {
                write!(f, "Execution error: {}", msg)
            }
            Self::UnrecoverableJobError(msg) => write!(f, "Unrecoverable job error: {}", msg),
            Self::Conflict(msg) => write!(f, "Conflict: {}", msg),
            Self::NotFound(msg) => write!(f, "Not found: {}", msg),
            Self::Internal(msg) => write!(f, "Internal error: {}", msg),
            Self::Generic(msg) => write!(f, "Generic error: {}", msg),
            Self::Caused(chain) => std::fmt::Display::fmt(chain.outer(), f),
        }
    }
}

impl std::error::Error for FlowableError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Caused(chain) => Some(chain.cause()),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::error::Error;

    #[test]
    fn request_classification_preserves_legacy_display_and_serde() {
        for error in [
            FlowableError::BadRequest("invalid request".to_string()),
            FlowableError::Forbidden("wrong owner".to_string()),
        ] {
            assert!(error.to_string().starts_with("Execution error: "));
            let encoded = serde_json::to_string(&error).expect("typed error should serialize");
            let decoded: FlowableError =
                serde_json::from_str(&encoded).expect("typed error should deserialize");
            assert_eq!(decoded.to_string(), error.to_string());
        }
    }

    fn nested_unrecoverable_error() -> FlowableError {
        FlowableError::ExecutionError("outer response handling failure".to_string()).caused_by(
            FlowableError::Generic("intermediate handler failure".to_string()).caused_by(
                FlowableError::UnrecoverableJobError(
                    "response payload cannot be safely processed".to_string(),
                ),
            ),
        )
    }

    #[test]
    fn caused_error_displays_outer_and_exposes_typed_source() {
        let error = FlowableError::ExecutionError("outer failure".to_string()).caused_by(
            FlowableError::UnrecoverableJobError("terminal cause".to_string()),
        );

        assert_eq!(error.to_string(), "Execution error: outer failure");
        assert_eq!(error.raw_primary_message(), "outer failure");

        let source = error
            .source()
            .expect("caused error should expose its source");
        let source = source
            .downcast_ref::<FlowableError>()
            .expect("source should remain a typed FlowableError");
        assert!(matches!(
            source,
            FlowableError::UnrecoverableJobError(message) if message == "terminal cause"
        ));
    }

    #[test]
    fn serde_round_trip_preserves_typed_cause_chain() {
        let error = nested_unrecoverable_error();
        let encoded = serde_json::to_string(&error).expect("serialize caused FlowableError");
        let decoded: FlowableError =
            serde_json::from_str(&encoded).expect("deserialize caused FlowableError");

        assert_eq!(
            decoded.raw_primary_message(),
            "outer response handling failure"
        );
        assert!(decoded.is_unrecoverable_job_failure());
        assert!(decoded.source().is_some());
    }

    #[test]
    fn unrecoverable_classification_walks_the_typed_chain() {
        assert!(nested_unrecoverable_error().is_unrecoverable_job_failure());
        assert!(
            FlowableError::UnrecoverableJobError("direct".to_string())
                .is_unrecoverable_job_failure()
        );
        assert!(
            !FlowableError::ExecutionError("recoverable".to_string())
                .caused_by(FlowableError::Internal("ordinary cause".to_string()))
                .is_unrecoverable_job_failure()
        );
    }
}
