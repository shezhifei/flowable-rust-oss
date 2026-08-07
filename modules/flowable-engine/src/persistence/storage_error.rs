use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StorageError {
    Connection(String),
    Sql(String),
    Serialization(String),
    Deserialization(String),
    ClosedTransaction,
    InvalidTransactionHandle(String),
    OptimisticLockConflict,
    DuplicateEntity { entity_type: String, id: String },
    CorruptGlobalLockValue { lock_name: String, value: String },
    Persistence(String),
}

impl fmt::Display for StorageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StorageError::Connection(m) => write!(f, "storage connection error: {m}"),
            StorageError::Sql(m) => write!(f, "storage SQL error: {m}"),
            StorageError::Serialization(m) => write!(f, "storage serialization error: {m}"),
            StorageError::Deserialization(m) => write!(f, "storage deserialization error: {m}"),
            StorageError::ClosedTransaction => write!(f, "storage transaction is already closed"),
            StorageError::InvalidTransactionHandle(m) => {
                write!(f, "invalid transaction handle: {m}")
            }
            StorageError::OptimisticLockConflict => write!(f, "optimistic lock conflict"),
            StorageError::DuplicateEntity { entity_type, id } => {
                write!(
                    f,
                    "persistence error: Duplicate entity: {entity_type} id={id}"
                )
            }
            StorageError::CorruptGlobalLockValue { lock_name, value } => {
                write!(f, "corrupt global lock value for '{lock_name}': {value}")
            }
            StorageError::Persistence(m) => write!(f, "persistence error: {m}"),
        }
    }
}

impl std::error::Error for StorageError {}

impl From<rusqlite::Error> for StorageError {
    fn from(e: rusqlite::Error) -> Self {
        StorageError::Sql(e.to_string())
    }
}

impl From<r2d2::Error> for StorageError {
    fn from(e: r2d2::Error) -> Self {
        StorageError::Connection(e.to_string())
    }
}

impl From<serde_json::Error> for StorageError {
    fn from(e: serde_json::Error) -> Self {
        // serde_json serialization (to_string) errors surface as `Io` when they
        // occur at all; deserialization (from_str) produces Syntax/Data/Eof.
        match e.classify() {
            serde_json::error::Category::Io => StorageError::Serialization(e.to_string()),
            _ => StorageError::Deserialization(e.to_string()),
        }
    }
}

impl From<StorageError> for flowable_engine_common::FlowableError {
    fn from(e: StorageError) -> Self {
        flowable_engine_common::FlowableError::Internal(e.to_string())
    }
}

impl From<flowable_persistence::PersistenceError> for StorageError {
    fn from(e: flowable_persistence::PersistenceError) -> Self {
        match e {
            flowable_persistence::PersistenceError::DuplicateEntity { entity_type, id } => {
                StorageError::DuplicateEntity { entity_type, id }
            }
            other => StorageError::Persistence(other.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duplicate_entity_conversion_preserves_type_and_legacy_text() {
        let error = StorageError::from(flowable_persistence::PersistenceError::DuplicateEntity {
            entity_type: "engine property".to_string(),
            id: "acquireAsyncJobsLock".to_string(),
        });

        assert!(matches!(
            &error,
            StorageError::DuplicateEntity { entity_type, id }
                if entity_type == "engine property" && id == "acquireAsyncJobsLock"
        ));
        assert_eq!(
            error.to_string(),
            "persistence error: Duplicate entity: engine property id=acquireAsyncJobsLock"
        );
    }

    #[test]
    fn non_duplicate_persistence_errors_keep_the_legacy_mapping() {
        let error = StorageError::from(flowable_persistence::PersistenceError::Statement(
            "synthetic UNIQUE text".to_string(),
        ));
        assert_eq!(
            error,
            StorageError::Persistence("Statement error: synthetic UNIQUE text".to_string())
        );
    }
}
