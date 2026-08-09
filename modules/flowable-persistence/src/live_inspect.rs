//! Bounded live SQL probes for historical migration inspect.
//!
//! Connects via the existing sqlx adapter path and reports table presence /
//! row counts for a fixed candidate list. Does not pull full table contents.

use crate::adapters::sqlx_executor::{SqlxExecutorFactory, shared_runtime};
use crate::config::{DatabaseConfig, DatabaseKind, SchemaMode};
use crate::error::PersistenceError;
use crate::executor::SqlExecutor;
use crate::row::DbRow;
use crate::statement::RenderedStatement;
use crate::value::DbParams;
use std::sync::Arc;
use tokio::runtime::Runtime;

/// Read-only probe over a live Flowable-compatible database.
pub struct LiveSqlProbe {
    _runtime: Arc<Runtime>,
    _factory: SqlxExecutorFactory,
    executor: Box<dyn SqlExecutor>,
    kind: DatabaseKind,
}

impl LiveSqlProbe {
    /// Open a short-lived connection pool and acquire one executor.
    ///
    /// Postgres/MySQL require the corresponding crate features; without them
    /// this returns a structured [`PersistenceError::Connection`].
    pub fn connect(kind: DatabaseKind, url: &str) -> Result<Self, PersistenceError> {
        match kind {
            DatabaseKind::Postgres | DatabaseKind::Mysql | DatabaseKind::Sqlite => {}
            DatabaseKind::Memory => {
                return Err(PersistenceError::Connection(
                    "live SQL probe does not support in-memory database kind".to_string(),
                ));
            }
        }

        // Shares the process-wide bridge runtime, so a LiveInspector dropped
        // inside an async context cannot take a runtime down with it.
        let runtime = shared_runtime()?;
        let config = DatabaseConfig {
            kind,
            url: url.to_string(),
            pool_size: 1,
            schema_mode: SchemaMode::False,
            table_prefix: None,
            schema: None,
            catalog: None,
        };
        let factory = SqlxExecutorFactory::new(&config, Arc::clone(&runtime))?;
        let executor = factory.create_executor()?;
        Ok(Self {
            _runtime: runtime,
            _factory: factory,
            executor,
            kind,
        })
    }

    pub fn database_kind(&self) -> DatabaseKind {
        self.kind
    }

    pub fn table_exists(&mut self, table: &str) -> Result<bool, PersistenceError> {
        let sql = match self.kind {
            DatabaseKind::Postgres => "SELECT 1 AS present FROM information_schema.tables \
                 WHERE table_schema = current_schema() \
                 AND lower(table_name) = lower($1) \
                 LIMIT 1"
                .to_string(),
            DatabaseKind::Mysql => "SELECT 1 AS present FROM information_schema.tables \
                 WHERE table_schema = DATABASE() \
                 AND table_name = ? \
                 LIMIT 1"
                .to_string(),
            DatabaseKind::Sqlite | DatabaseKind::Memory => {
                "SELECT 1 AS present FROM sqlite_master \
                 WHERE type = 'table' AND name = ?1 \
                 LIMIT 1"
                    .to_string()
            }
        };

        let mut params = DbParams::new();
        params.push(table);
        let row = self
            .executor
            .fetch_optional(RenderedStatement::new(sql, params))?;
        Ok(row.is_some())
    }

    pub fn count_rows(&mut self, table: &str) -> Result<usize, PersistenceError> {
        if !self.table_exists(table)? {
            return Ok(0);
        }
        let sql = format!("SELECT COUNT(*) AS cnt FROM {table}");
        self.fetch_count(sql)
    }

    /// Count process-instance roots: `ID_ = PROC_INST_ID_` when the execution
    /// table is present (mirrors SQLite historical inspect).
    pub fn count_process_instances(
        &mut self,
        execution_table: &str,
    ) -> Result<usize, PersistenceError> {
        if !self.table_exists(execution_table)? {
            return Ok(0);
        }
        let sql =
            format!("SELECT COUNT(*) AS cnt FROM {execution_table} WHERE ID_ = PROC_INST_ID_");
        self.fetch_count(sql)
    }

    pub fn list_distinct_text_values(
        &mut self,
        table: &str,
        column: &str,
    ) -> Result<Vec<String>, PersistenceError> {
        if !self.table_exists(table)? {
            return Ok(Vec::new());
        }
        let sql = format!(
            "SELECT DISTINCT {column} AS value FROM {table} WHERE {column} IS NOT NULL ORDER BY {column}"
        );
        let rows = self
            .executor
            .fetch_all(RenderedStatement::new(sql, DbParams::new()))?;
        Ok(rows
            .into_iter()
            .filter_map(|row| row.get_text("value").or_else(|| row.get_text(column)))
            .collect())
    }

    /// Execute a read-only extraction query assembled from engine-owned static
    /// table specifications. Live migration uses this to page full source rows;
    /// callers must not pass user-provided SQL.
    pub fn fetch_rows(&mut self, sql: impl Into<String>) -> Result<Vec<DbRow>, PersistenceError> {
        let sql = sql.into();
        if !sql.trim_start().to_ascii_uppercase().starts_with("SELECT ") {
            return Err(PersistenceError::Database(
                "live extraction only accepts SELECT statements".to_string(),
            ));
        }
        self.executor
            .fetch_all(RenderedStatement::new(sql, DbParams::new()))
    }

    fn fetch_count(&mut self, sql: String) -> Result<usize, PersistenceError> {
        let row = self
            .executor
            .fetch_optional(RenderedStatement::new(sql, DbParams::new()))?
            .ok_or_else(|| {
                PersistenceError::Database("COUNT(*) query returned no rows".to_string())
            })?;
        let count = row
            .get_integer("cnt")
            .or_else(|| row.get_integer("COUNT(*)"))
            .or_else(|| row.get_integer("count"))
            .unwrap_or(0);
        Ok(count.max(0) as usize)
    }
}
