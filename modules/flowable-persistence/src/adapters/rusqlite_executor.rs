use crate::dialect::SqliteDialect;
use crate::error::{PersistenceError, is_rusqlite_unique_violation};
use crate::executor::{ExecuteResult, SqlExecutor};
use crate::row::DbRow;
use crate::statement::RenderedStatement;
use crate::value::DbValue;
use rusqlite::Connection;
use std::ops::{Deref, DerefMut};
use std::thread;
use std::time::Duration;

const SQLITE_BUSY_RETRIES: u32 = 50;
const SQLITE_BUSY_BASE_DELAY_MS: u64 = 2;
const SQLITE_BUSY_MAX_DELAY_MS: u64 = 100;

fn is_busy_persistence_error(err: &PersistenceError) -> bool {
    let msg = err.to_string();
    msg.contains("database is locked")
        || msg.contains("database table is locked")
        || msg.contains("DatabaseBusy")
}

fn with_busy_retry<T, F>(mut operation: F) -> Result<T, PersistenceError>
where
    F: FnMut() -> Result<T, PersistenceError>,
{
    let mut last_error = None;
    for attempt in 0..SQLITE_BUSY_RETRIES {
        match operation() {
            Ok(result) => return Ok(result),
            Err(err) => {
                if !is_busy_persistence_error(&err) {
                    return Err(err);
                }
                last_error = Some(err);
                let delay_ms = std::cmp::min(
                    SQLITE_BUSY_BASE_DELAY_MS * 2_u64.pow(attempt),
                    SQLITE_BUSY_MAX_DELAY_MS,
                );
                thread::sleep(Duration::from_millis(delay_ms));
            }
        }
    }
    Err(PersistenceError::Transaction(format!(
        "SQLite database busy after {} retries: {:?}",
        SQLITE_BUSY_RETRIES, last_error
    )))
}

enum ConnectionHolder {
    Direct(Connection),
    Pooled(Box<dyn DerefMut<Target = Connection> + Send + 'static>),
}

impl Deref for ConnectionHolder {
    type Target = Connection;
    fn deref(&self) -> &Connection {
        match self {
            ConnectionHolder::Direct(c) => c,
            ConnectionHolder::Pooled(p) => p.deref(),
        }
    }
}

impl DerefMut for ConnectionHolder {
    fn deref_mut(&mut self) -> &mut Connection {
        match self {
            ConnectionHolder::Direct(c) => c,
            ConnectionHolder::Pooled(p) => p.deref_mut(),
        }
    }
}

pub struct RusqliteExecutor {
    connection: Option<ConnectionHolder>,
    in_transaction: bool,
    dialect: SqliteDialect,
}

impl RusqliteExecutor {
    pub fn new(connection: Connection) -> Result<Self, PersistenceError> {
        Ok(Self {
            connection: Some(ConnectionHolder::Direct(connection)),
            in_transaction: false,
            dialect: SqliteDialect,
        })
    }

    pub fn new_pooled<P>(pooled_conn: P) -> Result<Self, PersistenceError>
    where
        P: DerefMut<Target = Connection> + Send + 'static,
    {
        Ok(Self {
            connection: Some(ConnectionHolder::Pooled(Box::new(pooled_conn))),
            in_transaction: false,
            dialect: SqliteDialect,
        })
    }

    pub fn with_transaction(connection: Connection) -> Result<Self, PersistenceError> {
        let conn = connection;
        conn.execute_batch("BEGIN IMMEDIATE").map_err(|e| {
            PersistenceError::Transaction(format!("Failed to begin transaction: {}", e))
        })?;
        Ok(Self {
            connection: Some(ConnectionHolder::Direct(conn)),
            in_transaction: true,
            dialect: SqliteDialect,
        })
    }

    fn ensure_transaction(&mut self) -> Result<(), PersistenceError> {
        if !self.in_transaction
            && let Some(conn) = self.connection.as_mut()
        {
            let c = conn.deref_mut();
            with_busy_retry(|| {
                c.execute_batch("BEGIN IMMEDIATE").map_err(|e| {
                    PersistenceError::Transaction(format!("Failed to begin transaction: {}", e))
                })
            })?;
            self.in_transaction = true;
        }
        Ok(())
    }

    fn conn_for_read(&self) -> Result<&Connection, PersistenceError> {
        self.connection
            .as_deref()
            .ok_or(PersistenceError::ClosedTransaction)
    }

    fn conn_for_write(&mut self) -> Result<&mut Connection, PersistenceError> {
        self.ensure_transaction()?;
        self.connection
            .as_deref_mut()
            .ok_or(PersistenceError::ClosedTransaction)
    }
}

impl SqlExecutor for RusqliteExecutor {
    fn execute(&mut self, statement: RenderedStatement) -> Result<ExecuteResult, PersistenceError> {
        let conn = self.conn_for_write()?;

        let mut stmt = conn.prepare(&statement.sql).map_err(|e| {
            PersistenceError::Statement(format!("Failed to prepare statement: {}", e))
        })?;

        let params: Vec<Box<dyn rusqlite::types::ToSql>> = statement
            .params
            .values
            .iter()
            .map(|v| -> Box<dyn rusqlite::types::ToSql> {
                match v {
                    DbValue::Null
                    | DbValue::NullInteger
                    | DbValue::NullBoolean
                    | DbValue::NullBlob => Box::new(rusqlite::types::Null),
                    DbValue::Text(s) => Box::new(s.clone()),
                    DbValue::Integer(i) => Box::new(*i),
                    DbValue::Real(f) => Box::new(*f),
                    DbValue::Boolean(b) => Box::new(*b),
                    DbValue::Blob(b) => Box::new(b.clone()),
                }
            })
            .collect();

        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            params.iter().map(|p| p.as_ref()).collect();

        let rows_affected = stmt.execute(param_refs.as_slice()).map_err(|e| {
            if is_rusqlite_unique_violation(&e) {
                PersistenceError::DuplicateEntity {
                    entity_type: "unknown".to_string(),
                    id: "unknown".to_string(),
                }
            } else {
                PersistenceError::Statement(format!("Failed to execute statement: {}", e))
            }
        })?;

        Ok(ExecuteResult {
            rows_affected: rows_affected as u64,
        })
    }

    fn fetch_optional(
        &mut self,
        statement: RenderedStatement,
    ) -> Result<Option<DbRow>, PersistenceError> {
        let conn = self.conn_for_read()?;

        let mut stmt = conn.prepare(&statement.sql).map_err(|e| {
            PersistenceError::Statement(format!("Failed to prepare statement: {}", e))
        })?;

        let column_names: Vec<String> = (0..stmt.column_count())
            .map(|i| stmt.column_name(i).unwrap_or("").to_string())
            .collect();

        let params: Vec<Box<dyn rusqlite::types::ToSql>> = statement
            .params
            .values
            .iter()
            .map(|v| -> Box<dyn rusqlite::types::ToSql> {
                match v {
                    DbValue::Null
                    | DbValue::NullInteger
                    | DbValue::NullBoolean
                    | DbValue::NullBlob => Box::new(rusqlite::types::Null),
                    DbValue::Text(s) => Box::new(s.clone()),
                    DbValue::Integer(i) => Box::new(*i),
                    DbValue::Real(f) => Box::new(*f),
                    DbValue::Boolean(b) => Box::new(*b),
                    DbValue::Blob(b) => Box::new(b.clone()),
                }
            })
            .collect();

        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            params.iter().map(|p| p.as_ref()).collect();

        let mut rows = stmt
            .query(param_refs.as_slice())
            .map_err(|e| PersistenceError::Statement(format!("Failed to query: {}", e)))?;

        if let Some(row) = rows
            .next()
            .map_err(|e| PersistenceError::Statement(format!("Failed to get row: {}", e)))?
        {
            let mut db_row = DbRow::new();
            for (i, column) in column_names.iter().enumerate() {
                let value: rusqlite::types::Value = row.get(i).map_err(|e| {
                    PersistenceError::Statement(format!("Failed to get column {}: {}", i, e))
                })?;
                let db_value = match value {
                    rusqlite::types::Value::Null => DbValue::Null,
                    rusqlite::types::Value::Integer(i) => DbValue::Integer(i),
                    rusqlite::types::Value::Real(f) => DbValue::Real(f),
                    rusqlite::types::Value::Text(s) => DbValue::Text(s),
                    rusqlite::types::Value::Blob(b) => DbValue::Blob(b),
                };
                db_row.insert(column.clone(), db_value);
            }
            Ok(Some(db_row))
        } else {
            Ok(None)
        }
    }

    fn fetch_all(&mut self, statement: RenderedStatement) -> Result<Vec<DbRow>, PersistenceError> {
        let conn = self.conn_for_read()?;

        let mut stmt = conn.prepare(&statement.sql).map_err(|e| {
            PersistenceError::Statement(format!("Failed to prepare statement: {}", e))
        })?;

        let column_names: Vec<String> = (0..stmt.column_count())
            .map(|i| stmt.column_name(i).unwrap_or("").to_string())
            .collect();

        let params: Vec<Box<dyn rusqlite::types::ToSql>> = statement
            .params
            .values
            .iter()
            .map(|v| -> Box<dyn rusqlite::types::ToSql> {
                match v {
                    DbValue::Null
                    | DbValue::NullInteger
                    | DbValue::NullBoolean
                    | DbValue::NullBlob => Box::new(rusqlite::types::Null),
                    DbValue::Text(s) => Box::new(s.clone()),
                    DbValue::Integer(i) => Box::new(*i),
                    DbValue::Real(f) => Box::new(*f),
                    DbValue::Boolean(b) => Box::new(*b),
                    DbValue::Blob(b) => Box::new(b.clone()),
                }
            })
            .collect();

        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            params.iter().map(|p| p.as_ref()).collect();

        let mut rows = stmt
            .query(param_refs.as_slice())
            .map_err(|e| PersistenceError::Statement(format!("Failed to query: {}", e)))?;

        let mut results = Vec::new();
        while let Some(row) = rows
            .next()
            .map_err(|e| PersistenceError::Statement(format!("Failed to get row: {}", e)))?
        {
            let mut db_row = DbRow::new();
            for (i, column) in column_names.iter().enumerate() {
                let value: rusqlite::types::Value = row.get(i).map_err(|e| {
                    PersistenceError::Statement(format!("Failed to get column {}: {}", i, e))
                })?;
                let db_value = match value {
                    rusqlite::types::Value::Null => DbValue::Null,
                    rusqlite::types::Value::Integer(i) => DbValue::Integer(i),
                    rusqlite::types::Value::Real(f) => DbValue::Real(f),
                    rusqlite::types::Value::Text(s) => DbValue::Text(s),
                    rusqlite::types::Value::Blob(b) => DbValue::Blob(b),
                };
                db_row.insert(column.clone(), db_value);
            }
            results.push(db_row);
        }

        Ok(results)
    }

    fn commit(&mut self) -> Result<(), PersistenceError> {
        if self.in_transaction {
            if let Some(conn) = self.connection.as_mut() {
                conn.deref_mut().execute_batch("COMMIT").map_err(|e| {
                    PersistenceError::Transaction(format!("Failed to commit: {}", e))
                })?;
            }
            self.in_transaction = false;
        }
        Ok(())
    }

    fn rollback(&mut self) -> Result<(), PersistenceError> {
        if self.in_transaction {
            if let Some(conn) = self.connection.as_mut() {
                conn.deref_mut().execute_batch("ROLLBACK").map_err(|e| {
                    PersistenceError::Transaction(format!("Failed to rollback: {}", e))
                })?;
            }
            self.in_transaction = false;
        }
        Ok(())
    }

    fn dialect(&self) -> &dyn crate::dialect::SqlDialect {
        &self.dialect
    }
}

impl Drop for RusqliteExecutor {
    fn drop(&mut self) {
        if self.in_transaction {
            let _ = self.rollback();
        }
    }
}
