use thiserror::Error;

#[derive(Debug, Error)]
pub enum DbError {
    #[error("SQL error: {0}")]
    Sql(#[from] rusqlite::Error),
    #[error("Connection pool error: {0}")]
    Pool(String),
    #[error("Deserialization error for table {table}: {source}")]
    Deserialize {
        table: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("Serialization error: {0}")]
    Serialize(#[from] serde_json::Error),
}

impl From<r2d2::Error> for DbError {
    fn from(e: r2d2::Error) -> Self {
        DbError::Pool(e.to_string())
    }
}
