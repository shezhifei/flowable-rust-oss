use crate::config::DatabaseKind;
use crate::entity::{Entity, EntityType};
use crate::entity_cache::EntityCache;
use crate::error::PersistenceError;
use crate::executor::{ExecuteResult, SqlExecutor};
use crate::row::DbRow;
use crate::statement::{RenderedStatement, StatementCatalog, StatementId};
use crate::value::DbParams;
use std::sync::Arc;

#[derive(Debug)]
enum PendingOperation {
    Insert {
        entity_type: EntityType,
        id: String,
        statement_id: StatementId,
        params: DbParams,
    },
    Update {
        entity_type: EntityType,
        id: String,
        expected_revision: i32,
        statement_id: StatementId,
        params: DbParams,
    },
    Delete {
        entity_type: EntityType,
        id: String,
        expected_revision: Option<i32>,
        statement_id: StatementId,
        params: DbParams,
    },
    BulkInsert {
        entity_type: EntityType,
        operations: Vec<(StatementId, DbParams)>,
    },
    BulkDelete {
        entity_type: EntityType,
        operations: Vec<(StatementId, DbParams)>,
    },
}

/// Metadata for one table column, as reported by the backend's catalog:
/// `PRAGMA table_info` on SQLite, `information_schema.columns` elsewhere.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColumnInfo {
    pub name: String,
    pub data_type: String,
    pub nullable: bool,
    pub primary_key: bool,
}

pub struct DbSession {
    executor: Box<dyn SqlExecutor>,
    catalog: Arc<dyn StatementCatalog>,
    cache: EntityCache,
    pending_inserts: Vec<PendingOperation>,
    pending_updates: Vec<PendingOperation>,
    pending_deletes: Vec<PendingOperation>,
    pending_bulk_inserts: Vec<PendingOperation>,
    pending_bulk_deletes: Vec<PendingOperation>,
}

impl DbSession {
    pub fn new(executor: Box<dyn SqlExecutor>, catalog: Arc<dyn StatementCatalog>) -> Self {
        Self {
            executor,
            catalog,
            cache: EntityCache::new(),
            pending_inserts: Vec::new(),
            pending_updates: Vec::new(),
            pending_deletes: Vec::new(),
            pending_bulk_inserts: Vec::new(),
            pending_bulk_deletes: Vec::new(),
        }
    }

    pub fn cache(&self) -> &EntityCache {
        &self.cache
    }

    pub fn cache_mut(&mut self) -> &mut EntityCache {
        &mut self.cache
    }

    pub fn insert<T: Entity + Clone>(
        &mut self,
        entity: T,
        statement_id: StatementId,
        params: DbParams,
    ) -> Result<(), PersistenceError> {
        let entity_type = entity.entity_type();
        let id = entity.id().to_string();

        self.cache.put(Box::new(entity.clone()));
        self.cache.mark_inserted(entity_type, &id);

        self.pending_inserts.push(PendingOperation::Insert {
            entity_type,
            id,
            statement_id,
            params,
        });

        Ok(())
    }

    pub fn update<T: Entity + Clone + crate::entity::RevisionedEntity>(
        &mut self,
        entity: T,
        statement_id: StatementId,
        params: DbParams,
    ) -> Result<(), PersistenceError> {
        let entity_type = entity.entity_type();
        let id = entity.id().to_string();
        let expected_revision = entity.revision();

        self.cache.put(Box::new(entity.clone()));
        self.cache.mark_updated(entity_type, &id);

        self.pending_updates.push(PendingOperation::Update {
            entity_type,
            id,
            expected_revision,
            statement_id,
            params,
        });

        Ok(())
    }

    pub fn delete<T: Entity>(
        &mut self,
        entity: &T,
        statement_id: StatementId,
        params: DbParams,
    ) -> Result<(), PersistenceError> {
        let entity_type = entity.entity_type();
        let id = entity.id().to_string();

        self.cache.mark_deleted(entity_type, &id);

        self.pending_deletes.push(PendingOperation::Delete {
            entity_type,
            id,
            expected_revision: None,
            statement_id,
            params,
        });

        Ok(())
    }

    pub fn delete_revisioned<T: Entity + crate::entity::RevisionedEntity>(
        &mut self,
        entity: &T,
        statement_id: StatementId,
        params: DbParams,
    ) -> Result<(), PersistenceError> {
        let entity_type = entity.entity_type();
        let id = entity.id().to_string();
        let expected_revision = entity.revision();

        self.cache.mark_deleted(entity_type, &id);

        self.pending_deletes.push(PendingOperation::Delete {
            entity_type,
            id,
            expected_revision: Some(expected_revision),
            statement_id,
            params,
        });

        Ok(())
    }

    pub fn bulk_insert(
        &mut self,
        entity_type: EntityType,
        operations: Vec<(StatementId, DbParams)>,
    ) -> Result<(), PersistenceError> {
        if !operations.is_empty() {
            self.pending_bulk_inserts
                .push(PendingOperation::BulkInsert {
                    entity_type,
                    operations,
                });
        }
        Ok(())
    }

    pub fn bulk_delete(
        &mut self,
        entity_type: EntityType,
        operations: Vec<(StatementId, DbParams)>,
    ) -> Result<(), PersistenceError> {
        if !operations.is_empty() {
            self.pending_bulk_deletes
                .push(PendingOperation::BulkDelete {
                    entity_type,
                    operations,
                });
        }
        Ok(())
    }

    pub fn flush_for_read(&mut self) -> Result<(), PersistenceError> {
        self.flush()
    }

    pub fn select_one(
        &mut self,
        statement_id: StatementId,
        params: DbParams,
    ) -> Result<Option<DbRow>, PersistenceError> {
        self.flush_for_read()?;
        let rendered = self
            .catalog
            .render(statement_id, self.catalog.dialect(), &params)?;
        self.executor.fetch_optional(rendered)
    }

    pub fn select_list(
        &mut self,
        statement_id: StatementId,
        params: DbParams,
    ) -> Result<Vec<DbRow>, PersistenceError> {
        self.flush_for_read()?;
        let rendered = self
            .catalog
            .render(statement_id, self.catalog.dialect(), &params)?;
        self.executor.fetch_all(rendered)
    }

    pub fn execute(
        &mut self,
        statement_id: StatementId,
        params: DbParams,
    ) -> Result<ExecuteResult, PersistenceError> {
        let rendered = self
            .catalog
            .render(statement_id, self.catalog.dialect(), &params)?;
        self.executor.execute(rendered)
    }

    pub fn execute_raw(
        &mut self,
        rendered: RenderedStatement,
    ) -> Result<ExecuteResult, PersistenceError> {
        self.executor.execute(rendered)
    }

    pub fn select_raw(
        &mut self,
        rendered: RenderedStatement,
    ) -> Result<Vec<DbRow>, PersistenceError> {
        self.flush_for_read()?;
        self.executor.fetch_all(rendered)
    }

    pub fn select_one_raw(
        &mut self,
        rendered: RenderedStatement,
    ) -> Result<Option<DbRow>, PersistenceError> {
        self.flush_for_read()?;
        self.executor.fetch_optional(rendered)
    }

    pub fn flush(&mut self) -> Result<(), PersistenceError> {
        // Preserve Flowable transaction semantics: inserts, then updates, then deletes.
        self.flush_inserts()?;
        self.flush_bulk_inserts()?;
        self.flush_updates()?;
        self.flush_deletes()?;
        self.flush_bulk_deletes()?;
        Ok(())
    }

    fn flush_inserts(&mut self) -> Result<(), PersistenceError> {
        let operations = std::mem::take(&mut self.pending_inserts);
        for op in operations {
            if let PendingOperation::Insert {
                entity_type,
                id,
                statement_id,
                params,
            } = op
            {
                let _ = (entity_type, id);
                let rendered =
                    self.catalog
                        .render(statement_id, self.catalog.dialect(), &params)?;
                self.executor.execute(rendered)?;
            }
        }
        Ok(())
    }

    fn flush_bulk_inserts(&mut self) -> Result<(), PersistenceError> {
        let operations = std::mem::take(&mut self.pending_bulk_inserts);
        for op in operations {
            if let PendingOperation::BulkInsert {
                entity_type,
                operations,
            } = op
            {
                let _ = entity_type;
                for (statement_id, params) in operations {
                    let rendered =
                        self.catalog
                            .render(statement_id, self.catalog.dialect(), &params)?;
                    self.executor.execute(rendered)?;
                }
            }
        }
        Ok(())
    }

    fn flush_updates(&mut self) -> Result<(), PersistenceError> {
        let operations = std::mem::take(&mut self.pending_updates);
        for op in operations {
            if let PendingOperation::Update {
                entity_type,
                id,
                expected_revision,
                statement_id,
                params,
            } = op
            {
                let rendered =
                    self.catalog
                        .render(statement_id, self.catalog.dialect(), &params)?;
                let result = self.executor.execute(rendered)?;

                if result.rows_affected == 0 {
                    return Err(PersistenceError::OptimisticLock {
                        entity_type: format!("{:?}", entity_type),
                        id,
                        expected: expected_revision,
                    });
                }
            }
        }
        Ok(())
    }

    fn flush_deletes(&mut self) -> Result<(), PersistenceError> {
        let operations = std::mem::take(&mut self.pending_deletes);
        for op in operations {
            if let PendingOperation::Delete {
                entity_type,
                id,
                expected_revision,
                statement_id,
                params,
            } = op
            {
                let rendered =
                    self.catalog
                        .render(statement_id, self.catalog.dialect(), &params)?;
                let result = self.executor.execute(rendered)?;

                if result.rows_affected == 0 {
                    return Err(PersistenceError::OptimisticLock {
                        entity_type: format!("{:?}", entity_type),
                        id,
                        expected: expected_revision.unwrap_or(-1),
                    });
                }
            }
        }
        Ok(())
    }

    fn flush_bulk_deletes(&mut self) -> Result<(), PersistenceError> {
        let operations = std::mem::take(&mut self.pending_bulk_deletes);
        for op in operations {
            if let PendingOperation::BulkDelete {
                entity_type,
                operations,
            } = op
            {
                let _ = entity_type;
                for (statement_id, params) in operations {
                    let rendered =
                        self.catalog
                            .render(statement_id, self.catalog.dialect(), &params)?;
                    self.executor.execute(rendered)?;
                }
            }
        }
        Ok(())
    }

    pub fn commit(&mut self) -> Result<(), PersistenceError> {
        self.flush()?;
        self.executor.commit()
    }

    pub fn rollback(&mut self) -> Result<(), PersistenceError> {
        self.pending_inserts.clear();
        self.pending_updates.clear();
        self.pending_deletes.clear();
        self.pending_bulk_inserts.clear();
        self.pending_bulk_deletes.clear();
        self.cache.clear();
        self.executor.rollback()
    }

    pub fn dialect(&self) -> &dyn crate::dialect::SqlDialect {
        self.catalog.dialect()
    }

    /// Column metadata for `table`, dispatched by backend: SQLite via
    /// `PRAGMA table_info`, Postgres/MySQL via `information_schema.columns`.
    /// The in-memory backend answers raw queries with empty results, matching
    /// how it already treats the `PRAGMA` callers this replaces.
    pub fn table_columns(&mut self, table: &str) -> Result<Vec<ColumnInfo>, PersistenceError> {
        match self.dialect().database_kind() {
            DatabaseKind::Memory | DatabaseKind::Sqlite => {
                let rows = self.select_raw(RenderedStatement::new(
                    format!("PRAGMA table_info({table})"),
                    DbParams::new(),
                ))?;
                Ok(rows
                    .iter()
                    .map(|row| ColumnInfo {
                        name: row.get_text("name").unwrap_or_default(),
                        data_type: row.get_text("type").unwrap_or_default(),
                        nullable: row.get_integer("notnull").unwrap_or(0) == 0,
                        primary_key: row.get_integer("pk").unwrap_or(0) != 0,
                    })
                    .collect())
            }
            DatabaseKind::Postgres => self.information_schema_columns(table, "current_schema()"),
            DatabaseKind::Mysql => self.information_schema_columns(table, "DATABASE()"),
        }
    }

    fn information_schema_columns(
        &mut self,
        table: &str,
        schema_expr: &str,
    ) -> Result<Vec<ColumnInfo>, PersistenceError> {
        let placeholder = self.dialect().placeholder(0);
        let sql = format!(
            "SELECT c.column_name, c.data_type, c.is_nullable, \
             EXISTS ( \
                 SELECT 1 FROM information_schema.table_constraints tc \
                 JOIN information_schema.key_column_usage kcu \
                   ON tc.constraint_name = kcu.constraint_name \
                  AND tc.table_schema = kcu.table_schema \
                  AND tc.table_name = kcu.table_name \
                 WHERE tc.constraint_type = 'PRIMARY KEY' \
                   AND tc.table_schema = {schema_expr} \
                   AND tc.table_name = c.table_name \
                   AND kcu.column_name = c.column_name \
             ) AS is_primary_key \
             FROM information_schema.columns c \
             WHERE c.table_schema = {schema_expr} AND c.table_name = {placeholder} \
             ORDER BY c.ordinal_position"
        );
        let mut params = DbParams::new();
        params.push(table);
        let rows = self.select_raw(RenderedStatement::new(sql, params))?;
        Ok(rows
            .iter()
            .map(|row| ColumnInfo {
                name: row.get_text("column_name").unwrap_or_default(),
                data_type: row.get_text("data_type").unwrap_or_default(),
                nullable: row.get_text("is_nullable").as_deref() == Some("YES"),
                // Postgres reports EXISTS as bool, MySQL as 1/0.
                primary_key: row
                    .get_boolean("is_primary_key")
                    .unwrap_or_else(|| row.get_integer("is_primary_key").unwrap_or(0) != 0),
            })
            .collect())
    }

    /// Upsert one row by primary key with an explicit column list, dispatched per
    /// backend: SQLite `INSERT OR REPLACE`, MySQL `REPLACE INTO`, Postgres
    /// `INSERT ... ON CONFLICT (pk) DO UPDATE`.
    ///
    /// `columns` must list every column in the same order as `params`, with
    /// `pk_column` among them; every other column is refreshed on conflict. For
    /// the `ID_`/`DATA_` JSON-entity shape prefer [`Self::json_insert`] — this is
    /// for the fully-projected tables (CMMN history, event subscriptions) whose
    /// columns are not a two-column pair.
    ///
    /// Writing `INSERT OR REPLACE` by hand instead is a portability bug: it is a
    /// syntax error on both MySQL and Postgres.
    pub fn upsert_raw(
        &mut self,
        table: &str,
        pk_column: &str,
        columns: &[&str],
        params: DbParams,
    ) -> Result<ExecuteResult, PersistenceError> {
        let sql = crate::dialect::render_upsert(self.dialect(), table, pk_column, columns);
        self.execute_raw(RenderedStatement::new(sql, params))
    }

    pub fn json_insert(
        &mut self,
        table: &str,
        id: &str,
        json: &str,
        extras: &[(&str, Option<&str>)],
    ) -> Result<(), PersistenceError> {
        let dialect = self.dialect();
        let mut columns = vec!["ID_", "DATA_"];
        let mut extra_cols: Vec<&str> = Vec::new();
        for (col, _) in extras {
            columns.push(col);
            extra_cols.push(col);
        }

        let mut placeholders = Vec::new();
        for i in 0..columns.len() {
            placeholders.push(dialect.placeholder(i));
        }

        let mut sql = format!(
            "{} {} ({}) VALUES ({})",
            dialect.insert_or_replace_into(),
            table,
            columns.join(", "),
            placeholders.join(", ")
        );

        if dialect.supports_on_conflict_update() {
            let mut update_cols = vec!["DATA_"];
            update_cols.extend(extra_cols.iter().copied());
            sql.push_str(&dialect.on_conflict_do_update_suffix("ID_", &update_cols));
        }

        let mut params = DbParams::new();
        params.push(id);
        params.push(json);
        for (_, val) in extras {
            match val {
                Some(v) => params.push(*v),
                None => params.push(Option::<String>::None),
            }
        }

        self.execute_raw(RenderedStatement::new(sql, params))?;
        Ok(())
    }

    pub fn json_delete(&mut self, table: &str, id: &str) -> Result<(), PersistenceError> {
        let dialect = self.dialect();
        let sql = format!(
            "DELETE FROM {} WHERE ID_ = {}",
            table,
            dialect.placeholder(0)
        );
        let mut params = DbParams::new();
        params.push(id);
        self.execute_raw(RenderedStatement::new(sql, params))?;
        Ok(())
    }

    pub fn json_delete_by(
        &mut self,
        table: &str,
        col: &str,
        val: &str,
    ) -> Result<(), PersistenceError> {
        let dialect = self.dialect();
        let sql = format!(
            "DELETE FROM {} WHERE {} = {}",
            table,
            col,
            dialect.placeholder(0)
        );
        let mut params = DbParams::new();
        params.push(val);
        self.execute_raw(RenderedStatement::new(sql, params))?;
        Ok(())
    }

    pub fn json_find(&mut self, table: &str, id: &str) -> Result<Option<String>, PersistenceError> {
        let dialect = self.dialect();
        let sql = format!(
            "SELECT DATA_ FROM {} WHERE ID_ = {}",
            table,
            dialect.placeholder(0)
        );
        let mut params = DbParams::new();
        params.push(id);
        let row = self.select_one_raw(RenderedStatement::new(sql, params))?;
        Ok(row.and_then(|r| r.get_text("DATA_")))
    }

    pub fn json_find_all(
        &mut self,
        table: &str,
    ) -> Result<Vec<(String, String)>, PersistenceError> {
        let sql = format!("SELECT ID_, DATA_ FROM {}", table);
        let params = DbParams::new();
        let rows = self.select_raw(RenderedStatement::new(sql, params))?;
        let mut result = Vec::new();
        for row in rows {
            if let (Some(id), Some(data)) = (row.get_text("ID_"), row.get_text("DATA_")) {
                result.push((id, data));
            }
        }
        Ok(result)
    }

    pub fn json_find_by(
        &mut self,
        table: &str,
        col: &str,
        val: &str,
    ) -> Result<Vec<(String, String)>, PersistenceError> {
        let dialect = self.dialect();
        let sql = format!(
            "SELECT ID_, DATA_ FROM {} WHERE {} = {}",
            table,
            col,
            dialect.placeholder(0)
        );
        let mut params = DbParams::new();
        params.push(val);
        let rows = self.select_raw(RenderedStatement::new(sql, params))?;
        let mut result = Vec::new();
        for row in rows {
            if let (Some(id), Some(data)) = (row.get_text("ID_"), row.get_text("DATA_")) {
                result.push((id, data));
            }
        }
        Ok(result)
    }

    pub fn json_find_by_two(
        &mut self,
        table: &str,
        col1: &str,
        val1: &str,
        col2: &str,
        val2: &str,
    ) -> Result<Vec<(String, String)>, PersistenceError> {
        let dialect = self.dialect();
        let sql = format!(
            "SELECT ID_, DATA_ FROM {} WHERE {} = {} AND {} = {}",
            table,
            col1,
            dialect.placeholder(0),
            col2,
            dialect.placeholder(1)
        );
        let mut params = DbParams::new();
        params.push(val1);
        params.push(val2);
        let rows = self.select_raw(RenderedStatement::new(sql, params))?;
        let mut result = Vec::new();
        for row in rows {
            if let (Some(id), Some(data)) = (row.get_text("ID_"), row.get_text("DATA_")) {
                result.push((id, data));
            }
        }
        Ok(result)
    }

    pub fn json_find_one_by(
        &mut self,
        table: &str,
        col: &str,
        val: &str,
    ) -> Result<Option<(String, String)>, PersistenceError> {
        let dialect = self.dialect();
        let sql = format!(
            "SELECT ID_, DATA_ FROM {} WHERE {} = {} {}",
            table,
            col,
            dialect.placeholder(0),
            dialect.limit_offset(Some(1), None)
        );
        let mut params = DbParams::new();
        params.push(val);
        let row = self.select_one_raw(RenderedStatement::new(sql, params))?;
        Ok(row.and_then(|r| {
            let id = r.get_text("ID_")?;
            let data = r.get_text("DATA_")?;
            Some((id, data))
        }))
    }

    pub fn json_blob_insert(
        &mut self,
        table: &str,
        id: &str,
        text_cols: &[(&str, &str)],
        blob_col: &str,
        blob: &[u8],
    ) -> Result<(), PersistenceError> {
        let dialect = self.dialect();
        let mut columns = vec!["ID_"];
        for (col, _) in text_cols {
            columns.push(col);
        }
        columns.push(blob_col);

        let mut placeholders = Vec::new();
        for i in 0..columns.len() {
            placeholders.push(dialect.placeholder(i));
        }

        let sql = format!(
            "{} {} ({}) VALUES ({})",
            dialect.insert_or_replace_into(),
            table,
            columns.join(", "),
            placeholders.join(", ")
        );

        let mut params = DbParams::new();
        params.push(id);
        for (_, val) in text_cols {
            params.push(*val);
        }
        params.push(blob.to_vec());

        self.execute_raw(RenderedStatement::new(sql, params))?;
        Ok(())
    }

    pub fn json_blob_find(
        &mut self,
        table: &str,
        col: &str,
        val: &str,
        blob_col: &str,
    ) -> Result<Option<Vec<u8>>, PersistenceError> {
        let dialect = self.dialect();
        let sql = format!(
            "SELECT {} FROM {} WHERE {} = {}",
            blob_col,
            table,
            col,
            dialect.placeholder(0)
        );
        let mut params = DbParams::new();
        params.push(val);
        let row = self.select_one_raw(RenderedStatement::new(sql, params))?;
        Ok(row.and_then(|r| r.get_blob(blob_col)))
    }

    pub fn json_blob_find_by_two(
        &mut self,
        table: &str,
        col1: &str,
        val1: &str,
        col2: &str,
        val2: &str,
        blob_col: &str,
    ) -> Result<Option<Vec<u8>>, PersistenceError> {
        let dialect = self.dialect();
        let sql = format!(
            "SELECT {} FROM {} WHERE {} = {} AND {} = {}",
            blob_col,
            table,
            col1,
            dialect.placeholder(0),
            col2,
            dialect.placeholder(1)
        );
        let mut params = DbParams::new();
        params.push(val1);
        params.push(val2);
        let row = self.select_one_raw(RenderedStatement::new(sql, params))?;
        Ok(row.and_then(|r| r.get_blob(blob_col)))
    }

    pub fn json_max(
        &mut self,
        table: &str,
        col: &str,
        conditions: &[(&str, &str)],
    ) -> Result<Option<i64>, PersistenceError> {
        let dialect = self.dialect();
        let mut sql = format!("SELECT MAX({}) AS RES_ FROM {}", col, table);
        let mut params = DbParams::new();

        if !conditions.is_empty() {
            sql.push_str(" WHERE ");
            let mut cond_parts = Vec::new();
            for (i, (c, _)) in conditions.iter().enumerate() {
                cond_parts.push(format!("{} = {}", c, dialect.placeholder(i)));
            }
            sql.push_str(&cond_parts.join(" AND "));

            for (_, v) in conditions {
                params.push(*v);
            }
        }

        let row = self.select_one_raw(RenderedStatement::new(sql, params))?;
        Ok(row.and_then(|r| r.get_integer("RES_")))
    }

    pub fn json_count(
        &mut self,
        table: &str,
        conditions: &[(&str, &str)],
    ) -> Result<i64, PersistenceError> {
        let dialect = self.dialect();
        let mut sql = format!("SELECT COUNT(*) AS RES_ FROM {}", table);
        let mut params = DbParams::new();

        if !conditions.is_empty() {
            sql.push_str(" WHERE ");
            let mut cond_parts = Vec::new();
            for (i, (c, _)) in conditions.iter().enumerate() {
                cond_parts.push(format!("{} = {}", c, dialect.placeholder(i)));
            }
            sql.push_str(&cond_parts.join(" AND "));

            for (_, v) in conditions {
                params.push(*v);
            }
        }

        let row = self.select_one_raw(RenderedStatement::new(sql, params))?;
        Ok(row.and_then(|r| r.get_integer("RES_")).unwrap_or(0))
    }

    pub fn generic_upsert(
        &mut self,
        table: &str,
        id: &str,
        text_cols: &[(&str, &str)],
        int_cols: &[(&str, i64)],
        blob_cols: &[(&str, &[u8])],
    ) -> Result<(), PersistenceError> {
        let dialect = self.dialect();
        let mut columns = vec!["ID_".to_string()];
        for (c, _) in text_cols {
            columns.push(c.to_string());
        }
        for (c, _) in int_cols {
            columns.push(c.to_string());
        }
        for (c, _) in blob_cols {
            columns.push(c.to_string());
        }

        let n = columns.len();
        let placeholders: Vec<String> = (0..n).map(|i| dialect.placeholder(i)).collect();

        let mut sql = format!(
            "{} {} ({}) VALUES ({})",
            dialect.insert_or_replace_into(),
            table,
            columns.join(", "),
            placeholders.join(", ")
        );

        if dialect.supports_on_conflict_update() {
            let update_cols: Vec<&str> = columns
                .iter()
                .filter(|c| *c != "ID_")
                .map(|s| s.as_str())
                .collect();
            sql.push_str(&dialect.on_conflict_do_update_suffix("ID_", &update_cols));
        }

        let mut params = DbParams::new();
        params.push(id);
        for (_, v) in text_cols {
            params.push(*v);
        }
        for (_, v) in int_cols {
            params.push(*v);
        }
        for (_, v) in blob_cols {
            params.push(v.to_vec());
        }

        self.execute_raw(RenderedStatement::new(sql, params))?;
        Ok(())
    }

    pub fn generic_update(
        &mut self,
        table: &str,
        set_text: &[(&str, &str)],
        set_int: &[(&str, i64)],
        set_blob: Option<(&str, &[u8])>,
        where_cols: &[(&str, &str)],
    ) -> Result<(), PersistenceError> {
        let dialect = self.dialect();
        let mut set_parts = Vec::new();
        let mut params = DbParams::new();
        let mut idx = 0usize;

        for (c, v) in set_text {
            set_parts.push(format!("{} = {}", c, dialect.placeholder(idx)));
            params.push(*v);
            idx += 1;
        }
        for (c, v) in set_int {
            set_parts.push(format!("{} = {}", c, dialect.placeholder(idx)));
            params.push(*v);
            idx += 1;
        }
        if let Some((c, v)) = set_blob {
            set_parts.push(format!("{} = {}", c, dialect.placeholder(idx)));
            params.push(v.to_vec());
            idx += 1;
        }

        let mut where_parts = Vec::new();
        for (c, v) in where_cols {
            where_parts.push(format!("{} = {}", c, dialect.placeholder(idx)));
            params.push(*v);
            idx += 1;
        }

        let sql = format!(
            "UPDATE {} SET {} WHERE {}",
            table,
            set_parts.join(", "),
            where_parts.join(" AND ")
        );

        self.execute_raw(RenderedStatement::new(sql, params))?;
        Ok(())
    }

    #[allow(clippy::type_complexity)]
    pub fn raw_query(
        &mut self,
        table: &str,
        select_text: &[&str],
        select_int: &[&str],
        select_blob: &[&str],
        conditions: &[(&str, &str)],
        order_by: Option<(&str, bool)>,
    ) -> Result<Vec<(Vec<String>, Vec<i64>, Vec<Vec<u8>>)>, PersistenceError> {
        let dialect = self.dialect();
        let mut all_cols: Vec<String> = Vec::new();
        for c in select_text {
            all_cols.push(c.to_string());
        }
        for c in select_int {
            all_cols.push(c.to_string());
        }
        for c in select_blob {
            all_cols.push(c.to_string());
        }

        if all_cols.is_empty() {
            all_cols.push("ID_".to_string());
        }

        let mut sql = format!("SELECT {} FROM {}", all_cols.join(", "), table);
        let mut params = DbParams::new();

        if !conditions.is_empty() {
            sql.push_str(" WHERE ");
            let mut cond_parts = Vec::new();
            for (i, (c, v)) in conditions.iter().enumerate() {
                cond_parts.push(format!("{} = {}", c, dialect.placeholder(i)));
                params.push(*v);
            }
            sql.push_str(&cond_parts.join(" AND "));
        }

        if let Some((col, asc)) = order_by {
            sql.push_str(&format!(
                " ORDER BY {} {}",
                col,
                if asc { "ASC" } else { "DESC" }
            ));
        }

        let rows = self.select_raw(RenderedStatement::new(sql, params))?;
        let mut results = Vec::with_capacity(rows.len());
        for row in rows {
            let mut texts = Vec::new();
            let mut ints = Vec::new();
            let mut blobs = Vec::new();
            for c in select_text {
                texts.push(row.get_text(c).unwrap_or_default());
            }
            for c in select_int {
                ints.push(row.get_integer(c).unwrap_or(0));
            }
            for c in select_blob {
                blobs.push(row.get_blob(c).unwrap_or_default());
            }
            results.push((texts, ints, blobs));
        }
        Ok(results)
    }

    pub fn cas_update(
        &mut self,
        table: &str,
        id: &str,
        json: &str,
        set_extras: &[(&str, Option<&str>)],
        conditions: &[(&str, Option<&str>)],
    ) -> Result<usize, PersistenceError> {
        let dialect = self.dialect();
        let mut set_parts = vec![format!("DATA_ = {}", dialect.placeholder(0))];
        let mut params = DbParams::new();
        params.push(json);
        let mut idx = 1usize;

        for (c, v) in set_extras {
            set_parts.push(format!("{} = {}", c, dialect.placeholder(idx)));
            match v {
                Some(val) => params.push(*val),
                None => params.push(Option::<String>::None),
            }
            idx += 1;
        }

        let mut where_parts = vec![format!("ID_ = {}", dialect.placeholder(idx))];
        params.push(id);
        idx += 1;

        for (c, v) in conditions {
            match v {
                Some(val) => {
                    where_parts.push(format!("{} = {}", c, dialect.placeholder(idx)));
                    params.push(*val);
                }
                None => {
                    where_parts.push(format!("{} IS NULL", c));
                }
            }
            idx += 1;
        }

        let sql = format!(
            "UPDATE {} SET {} WHERE {}",
            table,
            set_parts.join(", "),
            where_parts.join(" AND ")
        );

        let result = self.executor.execute(RenderedStatement::new(sql, params))?;
        Ok(result.rows_affected as usize)
    }

    pub fn filter_query_ids(
        &mut self,
        table: &str,
        filters: &[(String, FilterOp)],
        order_by: &str,
        ascending: bool,
        limit: Option<usize>,
        offset: Option<usize>,
    ) -> Result<Vec<String>, PersistenceError> {
        let dialect = self.dialect();
        let mut sql = format!("SELECT ID_ FROM {}", table);
        let mut params = DbParams::new();
        let mut idx = 0usize;

        if !filters.is_empty() {
            sql.push_str(" WHERE ");
            let mut parts = Vec::new();
            for (col, op) in filters {
                match op {
                    FilterOp::Eq(v) => {
                        parts.push(format!("{} = {}", col, dialect.placeholder(idx)));
                        params.push(v.as_str());
                        idx += 1;
                    }
                    FilterOp::Neq(v) => {
                        parts.push(format!("{} != {}", col, dialect.placeholder(idx)));
                        params.push(v.as_str());
                        idx += 1;
                    }
                    FilterOp::IsNull => {
                        parts.push(format!("{} IS NULL", col));
                    }
                    FilterOp::IsNotNull => {
                        parts.push(format!("{} IS NOT NULL", col));
                    }
                    FilterOp::Gt(v) => {
                        parts.push(format!("{} > {}", col, dialect.placeholder(idx)));
                        params.push(v.as_str());
                        idx += 1;
                    }
                    FilterOp::Lt(v) => {
                        parts.push(format!("{} < {}", col, dialect.placeholder(idx)));
                        params.push(v.as_str());
                        idx += 1;
                    }
                    FilterOp::Like(v) => {
                        parts.push(format!("{} LIKE {}", col, dialect.placeholder(idx)));
                        params.push(v.as_str());
                        idx += 1;
                    }
                    FilterOp::In(vals) => {
                        let placeholders: Vec<String> = vals
                            .iter()
                            .map(|_| {
                                let p = dialect.placeholder(idx);
                                idx += 1;
                                p
                            })
                            .collect();
                        parts.push(format!("{} IN ({})", col, placeholders.join(", ")));
                        for v in vals {
                            params.push(v.as_str());
                        }
                    }
                    FilterOp::GtInt(v) => {
                        parts.push(format!("{} > {}", col, dialect.placeholder(idx)));
                        params.push(*v);
                        idx += 1;
                    }
                    FilterOp::LtInt(v) => {
                        parts.push(format!("{} < {}", col, dialect.placeholder(idx)));
                        params.push(*v);
                        idx += 1;
                    }
                    FilterOp::GeInt(v) => {
                        parts.push(format!("{} >= {}", col, dialect.placeholder(idx)));
                        params.push(*v);
                        idx += 1;
                    }
                    FilterOp::LeInt(v) => {
                        parts.push(format!("{} <= {}", col, dialect.placeholder(idx)));
                        params.push(*v);
                        idx += 1;
                    }
                }
            }
            sql.push_str(&parts.join(" AND "));
        }

        sql.push_str(&format!(
            " ORDER BY {} {}",
            order_by,
            if ascending { "ASC" } else { "DESC" }
        ));

        if let Some(lim) = limit {
            sql.push_str(&format!(" LIMIT {}", lim));
        }
        if let Some(off) = offset {
            sql.push_str(&format!(" OFFSET {}", off));
        }

        let rows = self.select_raw(RenderedStatement::new(sql, params))?;
        Ok(rows.iter().filter_map(|r| r.get_text("ID_")).collect())
    }

    pub fn filter_query_data(
        &mut self,
        table: &str,
        filters: &[(String, FilterOp)],
        order_by: &str,
        ascending: bool,
        limit: Option<usize>,
        offset: Option<usize>,
    ) -> Result<Vec<DbRow>, PersistenceError> {
        let dialect = self.dialect();
        let mut sql = format!("SELECT * FROM {}", table);
        let mut params = DbParams::new();
        let mut idx = 0usize;

        if !filters.is_empty() {
            sql.push_str(" WHERE ");
            let mut parts = Vec::new();
            for (col, op) in filters {
                match op {
                    FilterOp::Eq(v) => {
                        parts.push(format!("{} = {}", col, dialect.placeholder(idx)));
                        params.push(v.as_str());
                        idx += 1;
                    }
                    FilterOp::Neq(v) => {
                        parts.push(format!("{} != {}", col, dialect.placeholder(idx)));
                        params.push(v.as_str());
                        idx += 1;
                    }
                    FilterOp::IsNull => {
                        parts.push(format!("{} IS NULL", col));
                    }
                    FilterOp::IsNotNull => {
                        parts.push(format!("{} IS NOT NULL", col));
                    }
                    FilterOp::Gt(v) => {
                        parts.push(format!("{} > {}", col, dialect.placeholder(idx)));
                        params.push(v.as_str());
                        idx += 1;
                    }
                    FilterOp::Lt(v) => {
                        parts.push(format!("{} < {}", col, dialect.placeholder(idx)));
                        params.push(v.as_str());
                        idx += 1;
                    }
                    FilterOp::Like(v) => {
                        parts.push(format!("{} LIKE {}", col, dialect.placeholder(idx)));
                        params.push(v.as_str());
                        idx += 1;
                    }
                    FilterOp::In(vals) => {
                        let placeholders: Vec<String> = vals
                            .iter()
                            .map(|_| {
                                let p = dialect.placeholder(idx);
                                idx += 1;
                                p
                            })
                            .collect();
                        parts.push(format!("{} IN ({})", col, placeholders.join(", ")));
                        for v in vals {
                            params.push(v.as_str());
                        }
                    }
                    FilterOp::GtInt(v) => {
                        parts.push(format!("{} > {}", col, dialect.placeholder(idx)));
                        params.push(*v);
                        idx += 1;
                    }
                    FilterOp::LtInt(v) => {
                        parts.push(format!("{} < {}", col, dialect.placeholder(idx)));
                        params.push(*v);
                        idx += 1;
                    }
                    FilterOp::GeInt(v) => {
                        parts.push(format!("{} >= {}", col, dialect.placeholder(idx)));
                        params.push(*v);
                        idx += 1;
                    }
                    FilterOp::LeInt(v) => {
                        parts.push(format!("{} <= {}", col, dialect.placeholder(idx)));
                        params.push(*v);
                        idx += 1;
                    }
                }
            }
            sql.push_str(&parts.join(" AND "));
        }

        sql.push_str(&format!(
            " ORDER BY {} {}",
            order_by,
            if ascending { "ASC" } else { "DESC" }
        ));

        if limit.is_some() || offset.is_some() {
            sql.push_str(&format!(" {}", dialect.limit_offset(limit, offset)));
        }

        let rows = self.select_raw(RenderedStatement::new(sql, params))?;
        Ok(rows)
    }

    /// Counts rows in `table` matching the supplied filters. Filters use the
    /// same `FilterOp` rendering as `filter_query_ids` / `filter_query_data`.
    pub fn count_query(
        &mut self,
        table: &str,
        filters: &[(String, FilterOp)],
    ) -> Result<i64, PersistenceError> {
        let dialect = self.dialect();
        let mut sql = format!("SELECT COUNT(*) AS CNT FROM {}", table);
        let mut params = DbParams::new();
        let mut idx = 0usize;

        if !filters.is_empty() {
            sql.push_str(" WHERE ");
            let mut parts = Vec::new();
            for (col, op) in filters {
                match op {
                    FilterOp::Eq(v) => {
                        parts.push(format!("{} = {}", col, dialect.placeholder(idx)));
                        params.push(v.as_str());
                        idx += 1;
                    }
                    FilterOp::Neq(v) => {
                        parts.push(format!("{} != {}", col, dialect.placeholder(idx)));
                        params.push(v.as_str());
                        idx += 1;
                    }
                    FilterOp::IsNull => {
                        parts.push(format!("{} IS NULL", col));
                    }
                    FilterOp::IsNotNull => {
                        parts.push(format!("{} IS NOT NULL", col));
                    }
                    FilterOp::Gt(v) => {
                        parts.push(format!("{} > {}", col, dialect.placeholder(idx)));
                        params.push(v.as_str());
                        idx += 1;
                    }
                    FilterOp::Lt(v) => {
                        parts.push(format!("{} < {}", col, dialect.placeholder(idx)));
                        params.push(v.as_str());
                        idx += 1;
                    }
                    FilterOp::Like(v) => {
                        parts.push(format!("{} LIKE {}", col, dialect.placeholder(idx)));
                        params.push(v.as_str());
                        idx += 1;
                    }
                    FilterOp::In(vals) => {
                        let placeholders: Vec<String> = vals
                            .iter()
                            .map(|_| {
                                let p = dialect.placeholder(idx);
                                idx += 1;
                                p
                            })
                            .collect();
                        parts.push(format!("{} IN ({})", col, placeholders.join(", ")));
                        for v in vals {
                            params.push(v.as_str());
                        }
                    }
                    FilterOp::GtInt(v) => {
                        parts.push(format!("{} > {}", col, dialect.placeholder(idx)));
                        params.push(*v);
                        idx += 1;
                    }
                    FilterOp::LtInt(v) => {
                        parts.push(format!("{} < {}", col, dialect.placeholder(idx)));
                        params.push(*v);
                        idx += 1;
                    }
                    FilterOp::GeInt(v) => {
                        parts.push(format!("{} >= {}", col, dialect.placeholder(idx)));
                        params.push(*v);
                        idx += 1;
                    }
                    FilterOp::LeInt(v) => {
                        parts.push(format!("{} <= {}", col, dialect.placeholder(idx)));
                        params.push(*v);
                        idx += 1;
                    }
                }
            }
            sql.push_str(&parts.join(" AND "));
        }

        let row = self.select_one_raw(RenderedStatement::new(sql, params))?;
        row.and_then(|r| r.get_integer("CNT"))
            .ok_or_else(|| PersistenceError::Database("COUNT(*) returned no row".to_string()))
    }

    pub fn property_get(&mut self, name: &str) -> Result<Option<(String, i32)>, PersistenceError> {
        let dialect = self.dialect();
        let sql = format!(
            "SELECT VALUE_, REV_ FROM ACT_GE_PROPERTY WHERE NAME_ = {}",
            dialect.placeholder(0)
        );
        let mut params = DbParams::new();
        params.push(name);
        let row = self.select_one_raw(RenderedStatement::new(sql, params))?;
        Ok(row.map(|r| {
            let value = r.get_text("VALUE_").unwrap_or_default();
            let revision = r.get_integer("REV_").unwrap_or(1) as i32;
            (value, revision)
        }))
    }

    pub fn property_insert(&mut self, name: &str, value: &str) -> Result<(), PersistenceError> {
        let dialect = self.dialect();
        let sql = format!(
            "INSERT INTO ACT_GE_PROPERTY (NAME_, VALUE_, REV_) VALUES ({}, {}, 1)",
            dialect.placeholder(0),
            dialect.placeholder(1)
        );
        let mut params = DbParams::new();
        params.push(name);
        params.push(value);
        self.execute_raw(RenderedStatement::new(sql, params))?;
        Ok(())
    }

    pub fn property_update(&mut self, name: &str, value: &str) -> Result<bool, PersistenceError> {
        let dialect = self.dialect();
        let sql = format!(
            "UPDATE ACT_GE_PROPERTY SET VALUE_ = {}, REV_ = REV_ + 1 WHERE NAME_ = {}",
            dialect.placeholder(0),
            dialect.placeholder(1)
        );
        let mut params = DbParams::new();
        params.push(value);
        params.push(name);
        let result = self.executor.execute(RenderedStatement::new(sql, params))?;
        Ok(result.rows_affected > 0)
    }




    /// Optimistic-locking property update: succeeds only when `REV_` matches.
    pub fn property_update_if_revision(
        &mut self,
        name: &str,
        value: &str,
        expected_rev: i32,
    ) -> Result<bool, PersistenceError> {
        let dialect = self.dialect();
        let sql = format!(
            "UPDATE ACT_GE_PROPERTY SET VALUE_ = {}, REV_ = REV_ + 1 WHERE NAME_ = {} AND REV_ = {}",
            dialect.placeholder(0),
            dialect.placeholder(1),
            dialect.placeholder(2)
        );
        let mut params = DbParams::new();
        params.push(value);
        params.push(name);
        params.push(expected_rev as i64);
        let result = self.executor.execute(RenderedStatement::new(sql, params))?;
        Ok(result.rows_affected > 0)
    }

    pub fn property_delete(&mut self, name: &str) -> Result<bool, PersistenceError> {
        let dialect = self.dialect();
        let sql = format!(
            "DELETE FROM ACT_GE_PROPERTY WHERE NAME_ = {}",
            dialect.placeholder(0)
        );
        let mut params = DbParams::new();
        params.push(name);
        let result = self.executor.execute(RenderedStatement::new(sql, params))?;
        Ok(result.rows_affected > 0)
    }

    pub fn property_list(&mut self) -> Result<Vec<(String, String, i32)>, PersistenceError> {
        let sql = "SELECT NAME_, VALUE_, REV_ FROM ACT_GE_PROPERTY".to_string();
        let rows = self.select_raw(RenderedStatement::new(sql, DbParams::new()))?;
        Ok(rows
            .iter()
            .filter_map(|r| {
                let n = r.get_text("NAME_")?;
                let v = r.get_text("VALUE_")?;
                let rev = r.get_integer("REV_").unwrap_or(1) as i32;
                Some((n, v, rev))
            })
            .collect())
    }
}

#[derive(Debug, Clone)]
pub enum FilterOp {
    Eq(String),
    Neq(String),
    IsNull,
    IsNotNull,
    Gt(String),
    Lt(String),
    Like(String),
    In(Vec<String>),
    /// Numeric greater-than comparison. The column value is compared to the
    /// integer stored as a parameter, which avoids string-based numeric
    /// comparisons that produce incorrect ordering (e.g. "10" < "2").
    GtInt(i64),
    /// Numeric less-than comparison.
    LtInt(i64),
    /// Numeric greater-than-or-equal comparison.
    GeInt(i64),
    /// Numeric less-than-or-equal comparison.
    LeInt(i64),
}

impl Drop for DbSession {
    fn drop(&mut self) {
        // Auto-rollback if not committed
        let _ = self.executor.rollback();
    }
}
