use thiserror::Error;

#[derive(Error, Debug)]
pub enum PersistenceError {
    #[error("Database error: {0}")]
    Database(String),

    #[error("Connection error: {0}")]
    Connection(String),

    #[error("Transaction error: {0}")]
    Transaction(String),

    #[error("Optimistic locking conflict: {entity_type} id={id} expected_revision={expected}")]
    OptimisticLock {
        entity_type: String,
        id: String,
        expected: i32,
    },

    #[error("Entity not found: {entity_type} id={id}")]
    EntityNotFound { entity_type: String, id: String },

    #[error("Duplicate entity: {entity_type} id={id}")]
    DuplicateEntity { entity_type: String, id: String },

    #[error("Schema error: {0}")]
    Schema(String),

    #[error("Migration error: {0}")]
    Migration(String),

    #[error("Serialization error: {0}")]
    Serialization(String),

    #[error("Deserialization error: {0}")]
    Deserialization(String),

    #[error("Statement error: {0}")]
    Statement(String),

    #[error("Closed transaction")]
    ClosedTransaction,

    #[error("Pool error: {0}")]
    Pool(String),
}

impl From<sqlx::Error> for PersistenceError {
    fn from(err: sqlx::Error) -> Self {
        match err {
            sqlx::Error::RowNotFound => PersistenceError::EntityNotFound {
                entity_type: "unknown".to_string(),
                id: "unknown".to_string(),
            },
            sqlx::Error::Database(db_err) => {
                if db_err.is_unique_violation() {
                    PersistenceError::DuplicateEntity {
                        entity_type: "unknown".to_string(),
                        id: "unknown".to_string(),
                    }
                } else {
                    PersistenceError::Database(db_err.message().to_string())
                }
            }
            sqlx::Error::PoolTimedOut => PersistenceError::Pool("Pool timeout".to_string()),
            _ => PersistenceError::Database(err.to_string()),
        }
    }
}

impl From<serde_json::Error> for PersistenceError {
    fn from(err: serde_json::Error) -> Self {
        PersistenceError::Serialization(err.to_string())
    }
}

impl From<r2d2::Error> for PersistenceError {
    fn from(err: r2d2::Error) -> Self {
        PersistenceError::Pool(err.to_string())
    }
}

pub(crate) fn is_rusqlite_unique_violation(err: &rusqlite::Error) -> bool {
    matches!(
        err,
        rusqlite::Error::SqliteFailure(code, _)
            if code.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE
                || code.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_PRIMARYKEY
    )
}

impl From<rusqlite::Error> for PersistenceError {
    fn from(err: rusqlite::Error) -> Self {
        match err {
            rusqlite::Error::QueryReturnedNoRows => PersistenceError::EntityNotFound {
                entity_type: "unknown".to_string(),
                id: "unknown".to_string(),
            },
            err if is_rusqlite_unique_violation(&err) => PersistenceError::DuplicateEntity {
                entity_type: "unknown".to_string(),
                id: "unknown".to_string(),
            },
            err => PersistenceError::Database(err.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sqlite_failure(extended_code: i32) -> rusqlite::Error {
        rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error {
                code: rusqlite::ErrorCode::ConstraintViolation,
                extended_code,
            },
            None,
        )
    }

    #[test]
    fn rusqlite_unique_classifier_uses_typed_extended_codes() {
        assert!(is_rusqlite_unique_violation(&sqlite_failure(
            rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE,
        )));
        assert!(is_rusqlite_unique_violation(&sqlite_failure(
            rusqlite::ffi::SQLITE_CONSTRAINT_PRIMARYKEY,
        )));
        assert!(!is_rusqlite_unique_violation(&sqlite_failure(
            rusqlite::ffi::SQLITE_CONSTRAINT_NOTNULL,
        )));
        assert!(!is_rusqlite_unique_violation(&sqlite_failure(
            rusqlite::ffi::SQLITE_CONSTRAINT_FOREIGNKEY,
        )));
    }
}
