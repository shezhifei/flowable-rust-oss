use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DatabaseKind {
    Memory,
    Sqlite,
    Postgres,
    Mysql,
}

impl std::fmt::Display for DatabaseKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DatabaseKind::Memory => write!(f, "memory"),
            DatabaseKind::Sqlite => write!(f, "sqlite"),
            DatabaseKind::Postgres => write!(f, "postgres"),
            DatabaseKind::Mysql => write!(f, "mysql"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatabaseConfig {
    pub kind: DatabaseKind,
    pub url: String,
    pub pool_size: u32,
    pub schema_mode: SchemaMode,
    pub table_prefix: Option<String>,
    pub schema: Option<String>,
    pub catalog: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SchemaMode {
    False,
    True,
    Create,
    CreateDrop,
    DropCreate,
}

impl Default for DatabaseConfig {
    fn default() -> Self {
        Self {
            kind: DatabaseKind::Sqlite,
            url: "flowable.db".to_string(),
            pool_size: 8,
            schema_mode: SchemaMode::True,
            table_prefix: None,
            schema: None,
            catalog: None,
        }
    }
}
