use crate::dialect::SqlDialect;
use crate::error::PersistenceError;
use crate::row::DbRow;
use crate::statement::RenderedStatement;

#[derive(Debug, Clone)]
pub struct ExecuteResult {
    pub rows_affected: u64,
}

pub trait SqlExecutor: Send {
    fn execute(&mut self, statement: RenderedStatement) -> Result<ExecuteResult, PersistenceError>;

    fn fetch_optional(
        &mut self,
        statement: RenderedStatement,
    ) -> Result<Option<DbRow>, PersistenceError>;

    fn fetch_all(&mut self, statement: RenderedStatement) -> Result<Vec<DbRow>, PersistenceError>;

    fn commit(&mut self) -> Result<(), PersistenceError>;
    fn rollback(&mut self) -> Result<(), PersistenceError>;

    fn dialect(&self) -> &dyn SqlDialect;
}
