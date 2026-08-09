#![allow(clippy::collapsible_match)]
use crate::dialect::{MemoryDialect, MysqlDialect, PostgresDialect, SqlDialect, SqliteDialect};
use crate::error::PersistenceError;
use crate::executor::{ExecuteResult, SqlExecutor};
use crate::row::DbRow;
use crate::statement::RenderedStatement;
use crate::value::DbValue;
use sqlx::{Column, Row};
use std::future::Future;
use std::sync::Arc;
use tokio::runtime::{Runtime, RuntimeFlavor};

/// The one tokio runtime the sqlx backends bridge onto, for the life of the
/// process.
///
/// A runtime per session factory would be dropped whenever its engine is, and
/// dropping a `Runtime` blocks until its threads wind down — which tokio forbids
/// on a thread that is driving tasks. An engine released inside a handler or at
/// `#[tokio::main]` shutdown would panic on the way out. One shared runtime is
/// never dropped, so that end of the bridge cannot fail; it also stops each
/// engine from standing up its own thread pool, which is the behaviour a server
/// wants anyway.
pub fn shared_runtime() -> Result<Arc<Runtime>, PersistenceError> {
    static SHARED: std::sync::OnceLock<Result<Arc<Runtime>, String>> = std::sync::OnceLock::new();
    SHARED
        .get_or_init(|| {
            Runtime::new()
                .map(Arc::new)
                .map_err(|error| error.to_string())
        })
        .clone()
        .map_err(PersistenceError::Connection)
}

/// Drives `future` to completion on `runtime` from synchronous code, including
/// when the caller is already inside *another* tokio runtime.
///
/// The store API is synchronous while sqlx is not, so every statement here
/// bridges the two with `block_on`. Called bare, that bridge panics with
/// "Cannot start a runtime from within a runtime" whenever a runtime is already
/// on the thread — which is every axum handler and anything under
/// `#[tokio::main]` or `#[tokio::test]`. That left the sqlx backends unusable
/// from a server: `flowable-rest` panicked on the first request against Postgres
/// or MySQL, and its bootstrap panicked before reaching one.
///
/// Each case gets the cheapest escape that works:
/// * no runtime on the thread — block directly, as before;
/// * multi-thread runtime — `block_in_place` hands this worker's queued tasks to
///   a sibling and marks the thread blocking, which lifts the restriction;
/// * anything else — `block_in_place` is only valid on the multi-thread flavour,
///   so the work goes to a scoped thread that has no runtime of its own. That
///   covers current-thread and, because `RuntimeFlavor` is `#[non_exhaustive]`,
///   any flavour added later: the fallback works everywhere, so an unfamiliar
///   runtime gets the safe path rather than an assumption. It costs a thread
///   spawn per statement, which is why it is the last resort — a server runs
///   multi-thread and never lands here, while the shape that does is test code.
///
/// Every branch is reached only where the bare call would have panicked, so no
/// path that works today changes behaviour.
fn block_on<F>(runtime: &Runtime, future: F) -> F::Output
where
    F: Future + Send,
    F::Output: Send,
{
    match tokio::runtime::Handle::try_current() {
        Err(_) => runtime.block_on(future),
        Ok(handle) => match handle.runtime_flavor() {
            RuntimeFlavor::MultiThread => {
                tokio::task::block_in_place(|| runtime.block_on(future))
            }
            _ => std::thread::scope(|scope| {
                scope
                    .spawn(|| runtime.block_on(future))
                    .join()
                    .unwrap_or_else(|payload| std::panic::resume_unwind(payload))
            }),
        },
    }
}

/// Convert a sqlx row to a `DbRow` by trying each `DbValue` variant in order.
/// Implemented as a macro to avoid complex generic trait bounds on `Row::try_get`.
macro_rules! row_to_db_row {
    ($row:expr) => {{
        let row = $row;
        let mut db_row = DbRow::new();
        for (i, column) in row.columns().iter().enumerate() {
            let name = column.name().to_string();
            if let Ok(v) = row.try_get::<Option<String>, _>(i) {
                if let Some(s) = v {
                    db_row.insert(name, DbValue::Text(s));
                    continue;
                }
            }
            if let Ok(v) = row.try_get::<Option<i64>, _>(i) {
                if let Some(i) = v {
                    db_row.insert(name, DbValue::Integer(i));
                    continue;
                }
            }
            if let Ok(v) = row.try_get::<Option<i32>, _>(i) {
                if let Some(i) = v {
                    db_row.insert(name, DbValue::Integer(i as i64));
                    continue;
                }
            }
            if let Ok(v) = row.try_get::<Option<f64>, _>(i) {
                if let Some(f) = v {
                    db_row.insert(name, DbValue::Real(f));
                    continue;
                }
            }
            if let Ok(v) = row.try_get::<Option<f32>, _>(i) {
                if let Some(f) = v {
                    db_row.insert(name, DbValue::Real(f as f64));
                    continue;
                }
            }
            if let Ok(v) = row.try_get::<Option<bool>, _>(i) {
                if let Some(b) = v {
                    db_row.insert(name, DbValue::Boolean(b));
                    continue;
                }
            }
            if let Ok(v) = row.try_get::<Option<Vec<u8>>, _>(i) {
                if let Some(b) = v {
                    db_row.insert(name, DbValue::Blob(b));
                    continue;
                }
            }
            db_row.insert(name, DbValue::Null);
        }
        db_row
    }};
}

/// SQLite-specific sqlx executor with manual transaction control.
///
/// Stores a `PoolConnection<Sqlite>` and dereferences it to `&mut SqliteConnection`
/// when executing queries, because sqlx 0.8 only implements `Executor` for
/// `&mut SqliteConnection` (not for `&mut PoolConnection<Sqlite>`).
pub struct SqlxSqliteExecutor {
    runtime: Arc<Runtime>,
    connection: Option<sqlx::pool::PoolConnection<sqlx::Sqlite>>,
    in_transaction: bool,
}

impl SqlxSqliteExecutor {
    pub fn new(
        runtime: Arc<Runtime>,
        mut connection: sqlx::pool::PoolConnection<sqlx::Sqlite>,
    ) -> Result<Self, PersistenceError> {
        match block_on(&runtime, async { sqlx::query("BEGIN").execute(&mut *connection).await }) {
            Ok(_) => Ok(Self {
                runtime,
                connection: Some(connection),
                in_transaction: true,
            }),
            Err(e) => {
                {
                    let _enter = runtime.enter();
                    drop(connection);
                }
                Err(PersistenceError::Transaction(e.to_string()))
            }
        }
    }
}

impl SqlExecutor for SqlxSqliteExecutor {
    fn execute(&mut self, statement: RenderedStatement) -> Result<ExecuteResult, PersistenceError> {
        let runtime = &self.runtime;
        let conn = self
            .connection
            .as_deref_mut()
            .ok_or(PersistenceError::ClosedTransaction)?;
        let result = block_on(runtime, async {
            let mut query = sqlx::query(&statement.sql);
            for value in &statement.params.values {
                query = match value {
                    DbValue::Null => query.bind(None::<String>),
                    DbValue::NullInteger => query.bind(None::<i64>),
                    DbValue::NullBoolean => query.bind(None::<bool>),
                    DbValue::NullBlob => query.bind(None::<Vec<u8>>),
                    DbValue::Text(s) => query.bind(s.as_str()),
                    DbValue::Integer(i) => query.bind(*i),
                    DbValue::Real(f) => query.bind(*f),
                    DbValue::Boolean(b) => query.bind(*b),
                    DbValue::Blob(b) => query.bind(b.as_slice()),
                };
            }
            query.execute(conn).await
        })?;
        Ok(ExecuteResult {
            rows_affected: result.rows_affected(),
        })
    }

    fn fetch_optional(
        &mut self,
        statement: RenderedStatement,
    ) -> Result<Option<DbRow>, PersistenceError> {
        let runtime = &self.runtime;
        let conn = self
            .connection
            .as_deref_mut()
            .ok_or(PersistenceError::ClosedTransaction)?;
        let result = block_on(runtime, async {
            let mut query = sqlx::query(&statement.sql);
            for value in &statement.params.values {
                query = match value {
                    DbValue::Null => query.bind(None::<String>),
                    DbValue::NullInteger => query.bind(None::<i64>),
                    DbValue::NullBoolean => query.bind(None::<bool>),
                    DbValue::NullBlob => query.bind(None::<Vec<u8>>),
                    DbValue::Text(s) => query.bind(s.as_str()),
                    DbValue::Integer(i) => query.bind(*i),
                    DbValue::Real(f) => query.bind(*f),
                    DbValue::Boolean(b) => query.bind(*b),
                    DbValue::Blob(b) => query.bind(b.as_slice()),
                };
            }
            query.fetch_optional(conn).await
        })?;
        Ok(result.map(|row| row_to_db_row!(&row)))
    }

    fn fetch_all(&mut self, statement: RenderedStatement) -> Result<Vec<DbRow>, PersistenceError> {
        let runtime = &self.runtime;
        let conn = self
            .connection
            .as_deref_mut()
            .ok_or(PersistenceError::ClosedTransaction)?;
        let result = block_on(runtime, async {
            let mut query = sqlx::query(&statement.sql);
            for value in &statement.params.values {
                query = match value {
                    DbValue::Null => query.bind(None::<String>),
                    DbValue::NullInteger => query.bind(None::<i64>),
                    DbValue::NullBoolean => query.bind(None::<bool>),
                    DbValue::NullBlob => query.bind(None::<Vec<u8>>),
                    DbValue::Text(s) => query.bind(s.as_str()),
                    DbValue::Integer(i) => query.bind(*i),
                    DbValue::Real(f) => query.bind(*f),
                    DbValue::Boolean(b) => query.bind(*b),
                    DbValue::Blob(b) => query.bind(b.as_slice()),
                };
            }
            query.fetch_all(conn).await
        })?;
        Ok(result.iter().map(|row| row_to_db_row!(row)).collect())
    }

    fn commit(&mut self) -> Result<(), PersistenceError> {
        if self.in_transaction {
            if let Some(conn) = self.connection.as_mut() {
                block_on(&self.runtime, async { sqlx::query("COMMIT").execute(&mut **conn).await })
                    .map_err(|e| PersistenceError::Transaction(e.to_string()))?;
            }
            self.in_transaction = false;
        }
        Ok(())
    }

    fn rollback(&mut self) -> Result<(), PersistenceError> {
        if self.in_transaction {
            if let Some(conn) = self.connection.as_mut() {
                block_on(&self.runtime, async { sqlx::query("ROLLBACK").execute(&mut **conn).await })
                    .map_err(|e| PersistenceError::Transaction(e.to_string()))?;
            }
            self.in_transaction = false;
        }
        Ok(())
    }

    fn dialect(&self) -> &dyn SqlDialect {
        &SqliteDialect
    }
}

impl Drop for SqlxSqliteExecutor {
    fn drop(&mut self) {
        if self.in_transaction {
            let _ = self.rollback();
        }
        // PoolConnection::drop requires a tokio Handle on the current thread.
        if let Some(conn) = self.connection.take() {
            let _enter = self.runtime.enter();
            drop(conn);
        }
    }
}

/// PostgreSQL-specific sqlx executor with manual transaction control.
#[cfg(feature = "postgres")]
pub struct SqlxPostgresExecutor {
    runtime: Arc<Runtime>,
    connection: Option<sqlx::pool::PoolConnection<sqlx::Postgres>>,
    in_transaction: bool,
}

#[cfg(feature = "postgres")]
impl SqlxPostgresExecutor {
    pub fn new(
        runtime: Arc<Runtime>,
        mut connection: sqlx::pool::PoolConnection<sqlx::Postgres>,
    ) -> Result<Self, PersistenceError> {
        match block_on(&runtime, async { sqlx::query("BEGIN").execute(&mut *connection).await }) {
            Ok(_) => Ok(Self {
                runtime,
                connection: Some(connection),
                in_transaction: true,
            }),
            Err(e) => {
                {
                    let _enter = runtime.enter();
                    drop(connection);
                }
                Err(PersistenceError::Transaction(e.to_string()))
            }
        }
    }
}

#[cfg(feature = "postgres")]
impl SqlExecutor for SqlxPostgresExecutor {
    fn execute(&mut self, statement: RenderedStatement) -> Result<ExecuteResult, PersistenceError> {
        let runtime = &self.runtime;
        let conn = self
            .connection
            .as_deref_mut()
            .ok_or(PersistenceError::ClosedTransaction)?;
        let result = block_on(&runtime, async {
            let mut query = sqlx::query(&statement.sql);
            for value in &statement.params.values {
                query = match value {
                    DbValue::Null => query.bind(None::<String>),
                    DbValue::NullInteger => query.bind(None::<i64>),
                    DbValue::NullBoolean => query.bind(None::<bool>),
                    DbValue::NullBlob => query.bind(None::<Vec<u8>>),
                    DbValue::Text(s) => query.bind(s.as_str()),
                    DbValue::Integer(i) => query.bind(*i),
                    DbValue::Real(f) => query.bind(*f),
                    DbValue::Boolean(b) => query.bind(*b),
                    DbValue::Blob(b) => query.bind(b.as_slice()),
                };
            }
            query.execute(conn).await
        })?;
        Ok(ExecuteResult {
            rows_affected: result.rows_affected(),
        })
    }

    fn fetch_optional(
        &mut self,
        statement: RenderedStatement,
    ) -> Result<Option<DbRow>, PersistenceError> {
        let runtime = &self.runtime;
        let conn = self
            .connection
            .as_deref_mut()
            .ok_or(PersistenceError::ClosedTransaction)?;
        let result = block_on(&runtime, async {
            let mut query = sqlx::query(&statement.sql);
            for value in &statement.params.values {
                query = match value {
                    DbValue::Null => query.bind(None::<String>),
                    DbValue::NullInteger => query.bind(None::<i64>),
                    DbValue::NullBoolean => query.bind(None::<bool>),
                    DbValue::NullBlob => query.bind(None::<Vec<u8>>),
                    DbValue::Text(s) => query.bind(s.as_str()),
                    DbValue::Integer(i) => query.bind(*i),
                    DbValue::Real(f) => query.bind(*f),
                    DbValue::Boolean(b) => query.bind(*b),
                    DbValue::Blob(b) => query.bind(b.as_slice()),
                };
            }
            query.fetch_optional(conn).await
        })?;
        Ok(result.map(|row| row_to_db_row!(&row)))
    }

    fn fetch_all(&mut self, statement: RenderedStatement) -> Result<Vec<DbRow>, PersistenceError> {
        let runtime = &self.runtime;
        let conn = self
            .connection
            .as_deref_mut()
            .ok_or(PersistenceError::ClosedTransaction)?;
        let result = block_on(&runtime, async {
            let mut query = sqlx::query(&statement.sql);
            for value in &statement.params.values {
                query = match value {
                    DbValue::Null => query.bind(None::<String>),
                    DbValue::NullInteger => query.bind(None::<i64>),
                    DbValue::NullBoolean => query.bind(None::<bool>),
                    DbValue::NullBlob => query.bind(None::<Vec<u8>>),
                    DbValue::Text(s) => query.bind(s.as_str()),
                    DbValue::Integer(i) => query.bind(*i),
                    DbValue::Real(f) => query.bind(*f),
                    DbValue::Boolean(b) => query.bind(*b),
                    DbValue::Blob(b) => query.bind(b.as_slice()),
                };
            }
            query.fetch_all(conn).await
        })?;
        Ok(result.iter().map(|row| row_to_db_row!(row)).collect())
    }

    fn commit(&mut self) -> Result<(), PersistenceError> {
        if self.in_transaction {
            if let Some(conn) = self.connection.as_mut() {
                block_on(&self.runtime, async { sqlx::query("COMMIT").execute(&mut **conn).await })
                    .map_err(|e| PersistenceError::Transaction(e.to_string()))?;
            }
            self.in_transaction = false;
        }
        Ok(())
    }

    fn rollback(&mut self) -> Result<(), PersistenceError> {
        if self.in_transaction {
            if let Some(conn) = self.connection.as_mut() {
                block_on(&self.runtime, async { sqlx::query("ROLLBACK").execute(&mut **conn).await })
                    .map_err(|e| PersistenceError::Transaction(e.to_string()))?;
            }
            self.in_transaction = false;
        }
        Ok(())
    }

    fn dialect(&self) -> &dyn SqlDialect {
        &PostgresDialect
    }
}

#[cfg(feature = "postgres")]
impl Drop for SqlxPostgresExecutor {
    fn drop(&mut self) {
        if self.in_transaction {
            let _ = self.rollback();
        }
        if let Some(conn) = self.connection.take() {
            let _enter = self.runtime.enter();
            drop(conn);
        }
    }
}

/// MySQL-specific sqlx executor with manual transaction control.
#[cfg(feature = "mysql")]
pub struct SqlxMySqlExecutor {
    runtime: Arc<Runtime>,
    connection: Option<sqlx::pool::PoolConnection<sqlx::MySql>>,
    in_transaction: bool,
}

#[cfg(feature = "mysql")]
impl SqlxMySqlExecutor {
    pub fn new(
        runtime: Arc<Runtime>,
        connection: sqlx::pool::PoolConnection<sqlx::MySql>,
    ) -> Result<Self, PersistenceError> {
        // Do not auto-start a transaction: MySQL DDL (CREATE/ALTER) silently
        // commits any open transaction and leaves the connection in a state that
        // breaks subsequent COMMIT/ROLLBACK. Command sessions call begin() when
        // they need transactional semantics.
        Ok(Self {
            runtime,
            connection: Some(connection),
            in_transaction: false,
        })
    }

    pub fn begin(&mut self) -> Result<(), PersistenceError> {
        if self.in_transaction {
            return Ok(());
        }
        let conn = self
            .connection
            .as_mut()
            .ok_or(PersistenceError::ClosedTransaction)?;
        block_on(&self.runtime, async {
                use sqlx::Executor;
                // Simple query protocol — prepared BEGIN is rejected (error 1295).
                conn.execute("START TRANSACTION").await
            })
            .map_err(|e| PersistenceError::Transaction(e.to_string()))?;
        self.in_transaction = true;
        Ok(())
    }
}

#[cfg(feature = "mysql")]
impl SqlxMySqlExecutor {
    fn ensure_transaction_for_dml(&mut self, sql: &str) -> Result<(), PersistenceError> {
        // DDL auto-commits on MySQL — keep it outside a transaction.
        let upper = sql.trim_start().to_ascii_uppercase();
        let is_ddl = upper.starts_with("CREATE")
            || upper.starts_with("ALTER")
            || upper.starts_with("DROP")
            || upper.starts_with("TRUNCATE")
            || upper.starts_with("RENAME");
        if !is_ddl {
            self.begin()?;
        }
        Ok(())
    }
}

#[cfg(feature = "mysql")]
impl SqlExecutor for SqlxMySqlExecutor {
    fn execute(&mut self, statement: RenderedStatement) -> Result<ExecuteResult, PersistenceError> {
        self.ensure_transaction_for_dml(&statement.sql)?;
        let runtime = &self.runtime;
        let conn = self
            .connection
            .as_deref_mut()
            .ok_or(PersistenceError::ClosedTransaction)?;
        let result = block_on(&runtime, async {
            let mut query = sqlx::query(&statement.sql);
            for value in &statement.params.values {
                query = match value {
                    DbValue::Null => query.bind(None::<String>),
                    DbValue::NullInteger => query.bind(None::<i64>),
                    DbValue::NullBoolean => query.bind(None::<bool>),
                    DbValue::NullBlob => query.bind(None::<Vec<u8>>),
                    DbValue::Text(s) => query.bind(s.as_str()),
                    DbValue::Integer(i) => query.bind(*i),
                    DbValue::Real(f) => query.bind(*f),
                    DbValue::Boolean(b) => query.bind(*b),
                    DbValue::Blob(b) => query.bind(b.as_slice()),
                };
            }
            query.execute(conn).await
        })?;
        Ok(ExecuteResult {
            rows_affected: result.rows_affected(),
        })
    }

    fn fetch_optional(
        &mut self,
        statement: RenderedStatement,
    ) -> Result<Option<DbRow>, PersistenceError> {
        self.ensure_transaction_for_dml(&statement.sql)?;
        let runtime = &self.runtime;
        let conn = self
            .connection
            .as_deref_mut()
            .ok_or(PersistenceError::ClosedTransaction)?;
        let result = block_on(&runtime, async {
            let mut query = sqlx::query(&statement.sql);
            for value in &statement.params.values {
                query = match value {
                    DbValue::Null => query.bind(None::<String>),
                    DbValue::NullInteger => query.bind(None::<i64>),
                    DbValue::NullBoolean => query.bind(None::<bool>),
                    DbValue::NullBlob => query.bind(None::<Vec<u8>>),
                    DbValue::Text(s) => query.bind(s.as_str()),
                    DbValue::Integer(i) => query.bind(*i),
                    DbValue::Real(f) => query.bind(*f),
                    DbValue::Boolean(b) => query.bind(*b),
                    DbValue::Blob(b) => query.bind(b.as_slice()),
                };
            }
            query.fetch_optional(conn).await
        })?;
        Ok(result.map(|row| row_to_db_row!(&row)))
    }

    fn fetch_all(&mut self, statement: RenderedStatement) -> Result<Vec<DbRow>, PersistenceError> {
        self.ensure_transaction_for_dml(&statement.sql)?;
        let runtime = &self.runtime;
        let conn = self
            .connection
            .as_deref_mut()
            .ok_or(PersistenceError::ClosedTransaction)?;
        let result = block_on(&runtime, async {
            let mut query = sqlx::query(&statement.sql);
            for value in &statement.params.values {
                query = match value {
                    DbValue::Null => query.bind(None::<String>),
                    DbValue::NullInteger => query.bind(None::<i64>),
                    DbValue::NullBoolean => query.bind(None::<bool>),
                    DbValue::NullBlob => query.bind(None::<Vec<u8>>),
                    DbValue::Text(s) => query.bind(s.as_str()),
                    DbValue::Integer(i) => query.bind(*i),
                    DbValue::Real(f) => query.bind(*f),
                    DbValue::Boolean(b) => query.bind(*b),
                    DbValue::Blob(b) => query.bind(b.as_slice()),
                };
            }
            query.fetch_all(conn).await
        })?;
        Ok(result.iter().map(|row| row_to_db_row!(row)).collect())
    }

    fn commit(&mut self) -> Result<(), PersistenceError> {
        if !self.in_transaction {
            return Ok(());
        }
        if let Some(conn) = self.connection.as_mut() {
            // DDL may have already auto-committed; tolerate "no transaction" errors.
            let result = block_on(&self.runtime, async {
                use sqlx::Executor;
                conn.execute("COMMIT").await
            });
            if let Err(e) = result {
                let msg = e.to_string();
                if !msg.contains("1305")
                    && !msg.contains("not locked")
                    && !msg.to_ascii_lowercase().contains("no transaction")
                {
                    return Err(PersistenceError::Transaction(msg));
                }
            }
        }
        self.in_transaction = false;
        Ok(())
    }

    fn rollback(&mut self) -> Result<(), PersistenceError> {
        if !self.in_transaction {
            return Ok(());
        }
        if let Some(conn) = self.connection.as_mut() {
            let result = block_on(&self.runtime, async {
                use sqlx::Executor;
                conn.execute("ROLLBACK").await
            });
            if let Err(e) = result {
                let msg = e.to_string();
                if !msg.contains("1305")
                    && !msg.contains("not locked")
                    && !msg.to_ascii_lowercase().contains("no transaction")
                {
                    return Err(PersistenceError::Transaction(msg));
                }
            }
        }
        self.in_transaction = false;
        Ok(())
    }

    fn dialect(&self) -> &dyn SqlDialect {
        &MysqlDialect
    }
}

#[cfg(feature = "mysql")]
impl Drop for SqlxMySqlExecutor {
    fn drop(&mut self) {
        if self.in_transaction {
            let _ = self.rollback();
        }
        if let Some(conn) = self.connection.take() {
            let _enter = self.runtime.enter();
            drop(conn);
        }
    }
}

/// Factory sharing a tokio runtime and per-backend connection pool.
pub struct SqlxExecutorFactory {
    runtime: Arc<Runtime>,
    sqlite_pool: Option<sqlx::Pool<sqlx::Sqlite>>,
    #[cfg(feature = "postgres")]
    postgres_pool: Option<sqlx::Pool<sqlx::Postgres>>,
    #[cfg(feature = "mysql")]
    mysql_pool: Option<sqlx::Pool<sqlx::MySql>>,
    database_kind: crate::config::DatabaseKind,
}

impl SqlxExecutorFactory {
    pub fn new(
        config: &crate::config::DatabaseConfig,
        runtime: Arc<Runtime>,
    ) -> Result<Self, PersistenceError> {
        match config.kind {
            crate::config::DatabaseKind::Sqlite => {
                let pool = block_on(&runtime, async {
                        sqlx::sqlite::SqlitePoolOptions::new()
                            .max_connections(config.pool_size)
                            .connect(&config.url)
                            .await
                    })
                    .map_err(|e| PersistenceError::Connection(e.to_string()))?;
                Ok(Self {
                    runtime,
                    sqlite_pool: Some(pool),
                    #[cfg(feature = "postgres")]
                    postgres_pool: None,
                    #[cfg(feature = "mysql")]
                    mysql_pool: None,
                    database_kind: config.kind,
                })
            }
            #[cfg(feature = "postgres")]
            crate::config::DatabaseKind::Postgres => {
                let pool = block_on(&runtime, async {
                        sqlx::postgres::PgPoolOptions::new()
                            .max_connections(config.pool_size)
                            .connect(&config.url)
                            .await
                    })
                    .map_err(|e| PersistenceError::Connection(e.to_string()))?;
                Ok(Self {
                    runtime,
                    sqlite_pool: None,
                    postgres_pool: Some(pool),
                    #[cfg(feature = "mysql")]
                    mysql_pool: None,
                    database_kind: config.kind,
                })
            }
            #[cfg(not(feature = "postgres"))]
            crate::config::DatabaseKind::Postgres => Err(PersistenceError::Connection(
                "PostgreSQL support requires --features postgres".to_string(),
            )),
            #[cfg(feature = "mysql")]
            crate::config::DatabaseKind::Mysql => {
                let pool = block_on(&runtime, async {
                        sqlx::mysql::MySqlPoolOptions::new()
                            .max_connections(config.pool_size.max(4))
                            .acquire_timeout(std::time::Duration::from_secs(60))
                            .idle_timeout(Some(std::time::Duration::from_secs(60)))
                            .test_before_acquire(true)
                            .connect(&config.url)
                            .await
                    })
                    .map_err(|e| PersistenceError::Connection(e.to_string()))?;
                Ok(Self {
                    runtime,
                    sqlite_pool: None,
                    #[cfg(feature = "postgres")]
                    postgres_pool: None,
                    mysql_pool: Some(pool),
                    database_kind: config.kind,
                })
            }
            #[cfg(not(feature = "mysql"))]
            crate::config::DatabaseKind::Mysql => Err(PersistenceError::Connection(
                "MySQL support requires --features mysql".to_string(),
            )),
            other => Err(PersistenceError::Connection(format!(
                "Unsupported database kind for sqlx: {other}"
            ))),
        }
    }

    pub fn create_executor(&self) -> Result<Box<dyn SqlExecutor>, PersistenceError> {
        match self.database_kind {
            crate::config::DatabaseKind::Sqlite => {
                let pool = self.sqlite_pool.as_ref().ok_or_else(|| {
                    PersistenceError::Pool("SQLite pool not initialized".to_string())
                })?;
                let conn = block_on(&self.runtime, async { pool.acquire().await })
                    .map_err(|e| PersistenceError::Connection(e.to_string()))?;
                Ok(Box::new(SqlxSqliteExecutor::new(
                    Arc::clone(&self.runtime),
                    conn,
                )?))
            }
            #[cfg(feature = "postgres")]
            crate::config::DatabaseKind::Postgres => {
                let pool = self.postgres_pool.as_ref().ok_or_else(|| {
                    PersistenceError::Pool("Postgres pool not initialized".to_string())
                })?;
                let conn = block_on(&self.runtime, async { pool.acquire().await })
                    .map_err(|e| PersistenceError::Connection(e.to_string()))?;
                Ok(Box::new(SqlxPostgresExecutor::new(
                    Arc::clone(&self.runtime),
                    conn,
                )?))
            }
            #[cfg(not(feature = "postgres"))]
            crate::config::DatabaseKind::Postgres => Err(PersistenceError::Connection(
                "PostgreSQL support requires --features postgres".to_string(),
            )),
            #[cfg(feature = "mysql")]
            crate::config::DatabaseKind::Mysql => {
                let pool = self.mysql_pool.as_ref().ok_or_else(|| {
                    PersistenceError::Pool("MySQL pool not initialized".to_string())
                })?;
                let conn = block_on(&self.runtime, async { pool.acquire().await })
                    .map_err(|e| PersistenceError::Connection(e.to_string()))?;
                Ok(Box::new(SqlxMySqlExecutor::new(
                    Arc::clone(&self.runtime),
                    conn,
                )?))
            }
            #[cfg(not(feature = "mysql"))]
            crate::config::DatabaseKind::Mysql => Err(PersistenceError::Connection(
                "MySQL support requires --features mysql".to_string(),
            )),
            other => Err(PersistenceError::Connection(format!(
                "Unsupported database kind: {other}"
            ))),
        }
    }

    pub fn clone_for_session(&self) -> SqlxExecutorFactory {
        SqlxExecutorFactory {
            runtime: Arc::clone(&self.runtime),
            sqlite_pool: self.sqlite_pool.clone(),
            #[cfg(feature = "postgres")]
            postgres_pool: self.postgres_pool.clone(),
            #[cfg(feature = "mysql")]
            mysql_pool: self.mysql_pool.clone(),
            database_kind: self.database_kind,
        }
    }
}

pub fn dialect_for(kind: crate::config::DatabaseKind) -> Box<dyn SqlDialect> {
    match kind {
        crate::config::DatabaseKind::Memory => Box::new(MemoryDialect),
        crate::config::DatabaseKind::Sqlite => Box::new(SqliteDialect),
        crate::config::DatabaseKind::Postgres => Box::new(PostgresDialect),
        crate::config::DatabaseKind::Mysql => Box::new(MysqlDialect),
    }
}

// Re-export the concrete executor types for tests that need to name them.
pub use SqlxSqliteExecutor as SqlxExecutor;
