use std::error::Error;
use std::fmt::{Display, Formatter};

#[derive(Debug)]
pub enum CmmnError {
    Validation { message: String },
    UnsupportedModel { feature: String, message: String },
    NotFound { message: String },
    Execution { message: String },
    Conflict { message: String },
    Storage { message: String },
    NonUniqueResult { query: &'static str, count: usize },
}

impl CmmnError {
    pub fn validation(message: impl Into<String>) -> Self {
        Self::Validation {
            message: message.into(),
        }
    }

    pub fn unsupported(feature: impl Into<String>, message: impl Into<String>) -> Self {
        Self::UnsupportedModel {
            feature: feature.into(),
            message: message.into(),
        }
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::NotFound {
            message: message.into(),
        }
    }

    pub fn execution(message: impl Into<String>) -> Self {
        Self::Execution {
            message: message.into(),
        }
    }

    pub fn conflict(message: impl Into<String>) -> Self {
        Self::Conflict {
            message: message.into(),
        }
    }

    pub fn storage(message: impl Into<String>) -> Self {
        Self::Storage {
            message: message.into(),
        }
    }
}

impl Display for CmmnError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Validation { message }
            | Self::NotFound { message }
            | Self::Execution { message }
            | Self::Conflict { message }
            | Self::Storage { message } => formatter.write_str(message),
            Self::UnsupportedModel { feature, message } => {
                write!(formatter, "Unsupported CMMN {feature}: {message}")
            }
            Self::NonUniqueResult { query, count } => {
                write!(
                    formatter,
                    "non-unique result for {query}: expected at most 1, found {count}"
                )
            }
        }
    }
}

impl Error for CmmnError {}

impl From<rusqlite::Error> for CmmnError {
    fn from(value: rusqlite::Error) -> Self {
        Self::storage(format!("SQLite error: {value}"))
    }
}

impl From<serde_json::Error> for CmmnError {
    fn from(value: serde_json::Error) -> Self {
        Self::storage(format!("JSON error: {value}"))
    }
}

impl From<flowable_persistence::error::PersistenceError> for CmmnError {
    fn from(value: flowable_persistence::error::PersistenceError) -> Self {
        Self::storage(format!("Persistence error: {value}"))
    }
}
