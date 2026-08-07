use std::error::Error;
use std::fmt::{Display, Formatter};

#[derive(Debug)]
pub enum AppError {
    Validation { message: String },
    Unsupported { feature: String, message: String },
    NotFound { message: String },
    Execution { message: String },
    Storage { message: String },
}

impl AppError {
    pub fn validation(message: impl Into<String>) -> Self {
        Self::Validation {
            message: message.into(),
        }
    }

    pub fn unsupported(feature: impl Into<String>, message: impl Into<String>) -> Self {
        Self::Unsupported {
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

    pub fn storage(message: impl Into<String>) -> Self {
        Self::Storage {
            message: message.into(),
        }
    }
}

impl Display for AppError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Validation { message }
            | Self::NotFound { message }
            | Self::Execution { message }
            | Self::Storage { message } => formatter.write_str(message),
            Self::Unsupported { feature, message } => {
                write!(formatter, "Unsupported app {feature}: {message}")
            }
        }
    }
}

impl Error for AppError {}

impl From<rusqlite::Error> for AppError {
    fn from(value: rusqlite::Error) -> Self {
        Self::storage(format!("SQLite error: {value}"))
    }
}

impl From<serde_json::Error> for AppError {
    fn from(value: serde_json::Error) -> Self {
        Self::storage(format!("JSON error: {value}"))
    }
}

impl From<flowable_persistence::error::PersistenceError> for AppError {
    fn from(value: flowable_persistence::error::PersistenceError) -> Self {
        Self::storage(format!("Persistence error: {value}"))
    }
}
