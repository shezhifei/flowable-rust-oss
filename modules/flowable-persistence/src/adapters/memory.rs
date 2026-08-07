use crate::dialect::MemoryDialect;
use crate::error::PersistenceError;
use crate::executor::{ExecuteResult, SqlExecutor};
use crate::row::DbRow;
use crate::statement::RenderedStatement;
use std::collections::HashMap;

/// In-memory SQL executor for testing. Stores data in HashMaps.
#[allow(dead_code)]
pub struct MemoryExecutor {
    tables: HashMap<String, Vec<DbRow>>,
    committed: bool,
    dialect: MemoryDialect,
}

impl MemoryExecutor {
    pub fn new() -> Self {
        Self {
            tables: HashMap::new(),
            committed: false,
            dialect: MemoryDialect,
        }
    }
}

impl Default for MemoryExecutor {
    fn default() -> Self {
        Self::new()
    }
}

impl SqlExecutor for MemoryExecutor {
    fn execute(
        &mut self,
        _statement: RenderedStatement,
    ) -> Result<ExecuteResult, PersistenceError> {
        // Simple in-memory execution - just track that something happened
        Ok(ExecuteResult { rows_affected: 1 })
    }

    fn fetch_optional(
        &mut self,
        _statement: RenderedStatement,
    ) -> Result<Option<DbRow>, PersistenceError> {
        Ok(None)
    }

    fn fetch_all(&mut self, _statement: RenderedStatement) -> Result<Vec<DbRow>, PersistenceError> {
        Ok(Vec::new())
    }

    fn commit(&mut self) -> Result<(), PersistenceError> {
        Ok(())
    }

    fn rollback(&mut self) -> Result<(), PersistenceError> {
        Ok(())
    }

    fn dialect(&self) -> &dyn crate::dialect::SqlDialect {
        &self.dialect
    }
}
