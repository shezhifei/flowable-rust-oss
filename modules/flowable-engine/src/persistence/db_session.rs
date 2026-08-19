use crate::persistence::storage_error::StorageError;
pub use flowable_persistence::DbRow;
use flowable_persistence::statement::RenderedStatement;
pub use flowable_persistence::value::{DbParams, DbValue};
use flowable_persistence::{ColumnInfo, DbSession as InnerDbSession, SqlDialect};
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub enum FilterOp {
    Eq(Arc<str>),
    IsNull,
    IsNotNull,
    LessThan(i64),
    LessThanOrEqual(i64),
    GreaterThan(i64),
    GreaterThanOrEqual(i64),
    In(Vec<String>),
    Like(Arc<str>),
}

#[derive(Clone, Debug)]
pub struct RawRow {
    pub id: String,
    pub data: Arc<str>,
    pub extras: Arc<HashMap<String, Option<String>>>,
}

#[derive(Clone, Copy, Debug)]
pub struct BulkJsonRowUpdate<'a> {
    pub id: &'a str,
    pub json: &'a str,
}

#[derive(Debug, Clone)]
pub struct EnginePropertyRow {
    pub name: String,
    pub value: String,
    pub revision: i32,
}

fn map_table_name(name: &str) -> String {
    name.to_string()
}

fn map_col(name: &str) -> String {
    name.to_string()
}

/// Projection columns stored as INTEGER/BIGINT. Clearing them via untyped
/// `DbValue::Null` (text null) fails on PostgreSQL with "column is bigint but
/// expression is text" — use `NullInteger` instead (P73b / C2 typed nulls).
fn is_integer_projection_column(column: &str) -> bool {
    matches!(
        column,
        "lock_time"
            | "lock_expiration_time"
            | "due_time"
            | "retries"
            | "create_time"
            | "priority"
            | "due_date"
            | "created_at"
            | "updated_at"
            | "deployed_at"
            | "last_heartbeat"
            | "expiry_time"
            | "fencing_token"
            | "next_revision"
            | "revision"
            | "version"
            | "start_time_ms"
            | "end_time_ms"
    )
}

fn db_value_to_option_string(value: &DbValue) -> Option<String> {
    match value {
        DbValue::Null | DbValue::NullInteger | DbValue::NullBoolean | DbValue::NullBlob => None,
        DbValue::Text(s) => Some(s.clone()),
        DbValue::Integer(i) => Some(i.to_string()),
        DbValue::Real(f) => Some(f.to_string()),
        DbValue::Boolean(b) => Some(b.to_string()),
        DbValue::Blob(_) => None,
    }
}

fn row_to_raw_row(row: DbRow) -> Option<RawRow> {
    let id = row.get_text("id")?;
    let data = row.get_text("data")?;
    let mut extras = HashMap::new();
    for (col_name, col_value) in row.columns() {
        if col_name != "id" && col_name != "data" {
            extras.insert(col_name.clone(), db_value_to_option_string(col_value));
        }
    }
    Some(RawRow {
        id,
        data: Arc::from(data.into_boxed_str()),
        extras: Arc::new(extras),
    })
}

pub struct DbSession {
    inner: InnerDbSession,
    closed: bool,
}

impl DbSession {
    pub fn new(inner: InnerDbSession) -> Self {
        Self {
            inner,
            closed: false,
        }
    }

    /// Access the provider-backed persistence session for DataManager / StatementId paths.
    pub fn inner_mut(&mut self) -> &mut InnerDbSession {
        &mut self.inner
    }

    pub fn dialect(&self) -> &dyn SqlDialect {
        self.inner.dialect()
    }

    fn ensure_open(&self) -> Result<(), StorageError> {
        if self.closed {
            Err(StorageError::ClosedTransaction)
        } else {
            Ok(())
        }
    }

    pub fn insert<T: serde::Serialize>(
        &mut self,
        table: &str,
        id: &str,
        value: &T,
    ) -> Result<(), StorageError> {
        self.insert_with_extra(table, id, value, &[])
    }

    pub fn insert_with_extra<T: serde::Serialize>(
        &mut self,
        table: &str,
        id: &str,
        value: &T,
        extras: &[(String, Option<String>)],
    ) -> Result<(), StorageError> {
        let typed_extras = extras
            .iter()
            .filter_map(|(column, value)| {
                value.as_ref().map(|value| {
                    let value = value
                        .parse::<i64>()
                        .map(DbValue::Integer)
                        .unwrap_or_else(|_| DbValue::Text(value.clone()));
                    (column.clone(), value)
                })
            })
            .collect::<Vec<_>>();
        self.insert_with_typed_extra(table, id, value, &typed_extras)
    }

    /// Upserts a JSON entity together with explicitly typed projection columns.
    ///
    /// Unlike [`Self::insert_with_extra`], this method never infers SQL types
    /// from string contents and retains typed nulls. It is the appropriate path
    /// for projections that mix user-controlled text and numeric columns or
    /// need to clear an existing projected value on update.
    pub fn insert_with_typed_extra<T: serde::Serialize>(
        &mut self,
        table: &str,
        id: &str,
        value: &T,
        extras: &[(String, DbValue)],
    ) -> Result<(), StorageError> {
        self.ensure_open()?;
        let json = serde_json::to_string(value).map_err(StorageError::from)?;
        let table = map_table_name(table);
        let dialect = self.inner.dialect();

        let mapped_extras: Vec<(String, DbValue)> = extras
            .iter()
            .map(|(column, value)| (map_col(column), value.clone()))
            .collect();

        let mut column_refs: Vec<&str> = vec!["id", "data"];
        for (k, _) in &mapped_extras {
            column_refs.push(k.as_str());
        }

        let mut placeholders = Vec::new();
        for i in 0..column_refs.len() {
            placeholders.push(dialect.placeholder(i));
        }

        let mut sql = format!(
            "{} {} ({}) VALUES ({})",
            dialect.insert_or_replace_into(),
            table,
            column_refs.join(", "),
            placeholders.join(", ")
        );

        if dialect.supports_on_conflict_update() {
            let mut update_cols: Vec<&str> = vec!["data"];
            for (k, _) in &mapped_extras {
                update_cols.push(k.as_str());
            }
            sql.push_str(&dialect.on_conflict_do_update_suffix("id", &update_cols));
        }

        let mut params = DbParams::new();
        params.push(id);
        params.push(json.as_str());
        for (_, value) in &mapped_extras {
            params.values.push(value.clone());
        }

        self.inner
            .execute_raw(RenderedStatement::new(sql, params))?;
        Ok(())
    }

    /// Plain `INSERT` that fails with [`StorageError::DuplicateEntity`] when the
    /// primary key already exists. Unlike [`Self::insert_with_extra`], this never
    /// uses `INSERT OR REPLACE` / `ON CONFLICT DO UPDATE`, so concurrent first-
    /// writers of a shared row (e.g. coordinator lease) cannot silently overwrite
    /// each other.
    pub fn insert_exclusive_with_extra<T: serde::Serialize>(
        &mut self,
        table: &str,
        id: &str,
        value: &T,
        extras: &[(String, Option<String>)],
    ) -> Result<(), StorageError> {
        self.ensure_open()?;
        let json = serde_json::to_string(value).map_err(StorageError::from)?;
        let table = map_table_name(table);
        let dialect = self.inner.dialect();

        let mapped_extras: Vec<(String, String)> = extras
            .iter()
            .filter_map(|(k, v)| v.as_ref().map(|val| (map_col(k), val.clone())))
            .collect();

        let mut column_refs: Vec<&str> = vec!["id", "data"];
        for (k, _) in &mapped_extras {
            column_refs.push(k.as_str());
        }

        let mut placeholders = Vec::new();
        for i in 0..column_refs.len() {
            placeholders.push(dialect.placeholder(i));
        }

        let sql = format!(
            "INSERT INTO {} ({}) VALUES ({})",
            table,
            column_refs.join(", "),
            placeholders.join(", ")
        );

        let mut params = DbParams::new();
        params.push(id);
        params.push(json.as_str());
        for (_, val) in &mapped_extras {
            if let Ok(int_val) = val.parse::<i64>() {
                params.push(int_val);
            } else {
                params.push(val.as_str());
            }
        }

        self.inner
            .execute_raw(RenderedStatement::new(sql, params))?;
        Ok(())
    }

    pub fn find<T: serde::de::DeserializeOwned>(
        &mut self,
        table: &str,
        id: &str,
    ) -> Result<Option<T>, StorageError> {
        self.ensure_open()?;
        let table = map_table_name(table);
        let dialect = self.inner.dialect();
        let sql = format!(
            "SELECT data FROM {} WHERE id = {}",
            table,
            dialect.placeholder(0)
        );
        let mut params = DbParams::new();
        params.push(id);
        let row = self
            .inner
            .select_one_raw(RenderedStatement::new(sql, params))?;
        match row.and_then(|r| r.get_text("data")) {
            Some(json) => {
                let v = serde_json::from_str(&json)
                    .map_err(|e| StorageError::Deserialization(e.to_string()))?;
                Ok(Some(v))
            }
            None => Ok(None),
        }
    }

    pub fn find_by<T: serde::de::DeserializeOwned>(
        &mut self,
        table: &str,
        col: &str,
        val: &str,
    ) -> Result<Vec<T>, StorageError> {
        self.ensure_open()?;
        let table = map_table_name(table);
        let col = map_col(col);
        let dialect = self.inner.dialect();
        let sql = format!(
            "SELECT id, data FROM {} WHERE {} = {}",
            table,
            col,
            dialect.placeholder(0)
        );
        let mut params = DbParams::new();
        params.push(val);
        let rows = self.inner.select_raw(RenderedStatement::new(sql, params))?;
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            if let Some(data) = row.get_text("data") {
                out.push(
                    serde_json::from_str(&data)
                        .map_err(|e| StorageError::Deserialization(e.to_string()))?,
                );
            }
        }
        Ok(out)
    }

    pub fn find_raw_by(
        &mut self,
        table: &str,
        col: &str,
        val: &str,
    ) -> Result<Vec<RawRow>, StorageError> {
        self.ensure_open()?;
        let table = map_table_name(table);
        let col = map_col(col);
        let dialect = self.inner.dialect();
        let sql = format!(
            "SELECT * FROM {} WHERE {} = {}",
            table,
            col,
            dialect.placeholder(0)
        );
        let mut params = DbParams::new();
        params.push(val);
        let rows = self.inner.select_raw(RenderedStatement::new(sql, params))?;
        Ok(rows.into_iter().filter_map(row_to_raw_row).collect())
    }

    pub fn find_raw_by_two(
        &mut self,
        table: &str,
        col1: &str,
        val1: &str,
        col2: &str,
        val2: &str,
    ) -> Result<Vec<RawRow>, StorageError> {
        self.ensure_open()?;
        let table = map_table_name(table);
        let col1 = map_col(col1);
        let col2 = map_col(col2);
        let dialect = self.inner.dialect();
        let sql = format!(
            "SELECT * FROM {} WHERE {} = {} AND {} = {}",
            table,
            col1,
            dialect.placeholder(0),
            col2,
            dialect.placeholder(1)
        );
        let mut params = DbParams::new();
        params.push(val1);
        params.push(val2);
        let rows = self.inner.select_raw(RenderedStatement::new(sql, params))?;
        Ok(rows.into_iter().filter_map(row_to_raw_row).collect())
    }

    pub fn find_raw_all(&mut self, table: &str) -> Result<Vec<RawRow>, StorageError> {
        self.ensure_open()?;
        let table = map_table_name(table);
        let sql = format!("SELECT * FROM {}", table);
        let rows = self
            .inner
            .select_raw(RenderedStatement::new(sql, DbParams::new()))?;
        Ok(rows.into_iter().filter_map(row_to_raw_row).collect())
    }

    pub fn find_by_two<T: serde::de::DeserializeOwned>(
        &mut self,
        table: &str,
        col1: &str,
        val1: &str,
        col2: &str,
        val2: &str,
    ) -> Result<Vec<T>, StorageError> {
        self.ensure_open()?;
        let table = map_table_name(table);
        let col1 = map_col(col1);
        let col2 = map_col(col2);
        let dialect = self.inner.dialect();
        let sql = format!(
            "SELECT id, data FROM {} WHERE {} = {} AND {} = {}",
            table,
            col1,
            dialect.placeholder(0),
            col2,
            dialect.placeholder(1)
        );
        let mut params = DbParams::new();
        params.push(val1);
        params.push(val2);
        let rows = self.inner.select_raw(RenderedStatement::new(sql, params))?;
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            if let Some(data) = row.get_text("data") {
                out.push(
                    serde_json::from_str(&data)
                        .map_err(|e| StorageError::Deserialization(e.to_string()))?,
                );
            }
        }
        Ok(out)
    }

    pub fn find_all<T: serde::de::DeserializeOwned>(
        &mut self,
        table: &str,
    ) -> Result<Vec<T>, StorageError> {
        self.ensure_open()?;
        let table = map_table_name(table);
        let sql = format!("SELECT id, data FROM {}", table);
        let rows = self
            .inner
            .select_raw(RenderedStatement::new(sql, DbParams::new()))?;
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            if let Some(data) = row.get_text("data") {
                out.push(
                    serde_json::from_str(&data)
                        .map_err(|e| StorageError::Deserialization(e.to_string()))?,
                );
            }
        }
        Ok(out)
    }

    pub fn delete(&mut self, table: &str, id: &str) -> Result<(), StorageError> {
        self.ensure_open()?;
        let table = map_table_name(table);
        let dialect = self.inner.dialect();
        let sql = format!(
            "DELETE FROM {} WHERE id = {}",
            table,
            dialect.placeholder(0)
        );
        let mut params = DbParams::new();
        params.push(id);
        self.inner
            .execute_raw(RenderedStatement::new(sql, params))?;
        Ok(())
    }

    pub fn delete_by(&mut self, table: &str, col: &str, val: &str) -> Result<(), StorageError> {
        self.ensure_open()?;
        let table = map_table_name(table);
        let col = map_col(col);
        let dialect = self.inner.dialect();
        let sql = format!(
            "DELETE FROM {} WHERE {} = {}",
            table,
            col,
            dialect.placeholder(0)
        );
        let mut params = DbParams::new();
        params.push(val);
        self.inner
            .execute_raw(RenderedStatement::new(sql, params))?;
        Ok(())
    }

    pub fn cas_update(
        &mut self,
        table: &str,
        id: &str,
        json: &str,
        set_extras: &[(String, Option<String>)],
        conditions: &[(String, Option<String>)],
    ) -> Result<usize, StorageError> {
        self.ensure_open()?;
        let table = map_table_name(table);
        let dialect = self.inner.dialect();

        let mapped_set: Vec<(String, Option<String>)> = set_extras
            .iter()
            .map(|(k, v)| (map_col(k), v.clone()))
            .collect();
        let mapped_conds: Vec<(String, Option<String>)> = conditions
            .iter()
            .map(|(k, v)| (map_col(k), v.clone()))
            .collect();

        let mut set_parts = vec![format!("data = {}", dialect.placeholder(0))];
        let mut params = DbParams::new();
        params.push(json);
        let mut idx = 1usize;

        for (c, val) in &mapped_set {
            set_parts.push(format!("{} = {}", c, dialect.placeholder(idx)));
            match val {
                Some(val) => {
                    if let Ok(int_val) = val.parse::<i64>() {
                        params.push(int_val);
                    } else {
                        params.push(val.as_str());
                    }
                }
                None if is_integer_projection_column(c) => {
                    // PG requires typed null for bigint/integer columns.
                    params.values.push(DbValue::NullInteger);
                }
                None => params.push(Option::<String>::None),
            }
            idx += 1;
        }

        let mut where_parts = vec![format!("id = {}", dialect.placeholder(idx))];
        params.push(id);
        idx += 1;

        for (c, v) in &mapped_conds {
            match v {
                Some(val) => {
                    where_parts.push(format!("{} = {}", c, dialect.placeholder(idx)));
                    if let Ok(int_val) = val.parse::<i64>() {
                        params.push(int_val);
                    } else {
                        params.push(val.as_str());
                    }
                    idx += 1;
                }
                None => {
                    where_parts.push(format!("{} IS NULL", c));
                }
            }
        }

        let sql = format!(
            "UPDATE {} SET {} WHERE {}",
            table,
            set_parts.join(", "),
            where_parts.join(" AND ")
        );

        let result = self
            .inner
            .execute_raw(RenderedStatement::new(sql, params))?;
        Ok(result.rows_affected as usize)
    }

    /// Updates a selected ID set without revision/old-value predicates. This is
    /// reserved for callers that already hold a serialized global-acquisition
    /// permit. JSON payloads are updated with a CASE expression so the indexed
    /// columns and the serialized entity remain consistent.
    pub fn bulk_update_json_and_columns_by_ids(
        &mut self,
        table: &str,
        rows: &[BulkJsonRowUpdate<'_>],
        shared_columns: &[(String, Option<String>)],
    ) -> Result<usize, StorageError> {
        self.ensure_open()?;
        if rows.is_empty() {
            return Ok(0);
        }

        const ROWS_PER_CHUNK: usize = 100;
        let table = map_table_name(table);
        let mut total_affected = 0usize;

        for chunk in rows.chunks(ROWS_PER_CHUNK) {
            let (sql, params) = {
                let dialect = self.inner.dialect();
                let mut params = DbParams::new();
                let mut parameter_index = 0usize;
                let mut case_parts = Vec::with_capacity(chunk.len());
                for row in chunk {
                    case_parts.push(format!(
                        "WHEN {} THEN {}",
                        dialect.placeholder(parameter_index),
                        dialect.placeholder(parameter_index + 1)
                    ));
                    params.push(row.id);
                    params.push(row.json);
                    parameter_index += 2;
                }

                let mut set_parts = vec![format!(
                    "data = CASE id {} ELSE data END",
                    case_parts.join(" ")
                )];
                for (column, value) in shared_columns {
                    set_parts.push(format!(
                        "{} = {}",
                        map_col(column),
                        dialect.placeholder(parameter_index)
                    ));
                    match value {
                        Some(value) => {
                            if let Ok(integer) = value.parse::<i64>() {
                                params.push(integer);
                            } else {
                                params.push(value.as_str());
                            }
                        }
                        None => params.push(Option::<String>::None),
                    }
                    parameter_index += 1;
                }

                let mut id_placeholders = Vec::with_capacity(chunk.len());
                for row in chunk {
                    id_placeholders.push(dialect.placeholder(parameter_index));
                    params.push(row.id);
                    parameter_index += 1;
                }
                let sql = format!(
                    "UPDATE {} SET {} WHERE id IN ({})",
                    table,
                    set_parts.join(", "),
                    id_placeholders.join(", ")
                );
                (sql, params)
            };
            let result = self
                .inner
                .execute_raw(RenderedStatement::new(sql, params))?;
            total_affected += result.rows_affected as usize;
        }

        Ok(total_affected)
    }

    pub fn find_ids_by_filter(
        &mut self,
        table: &str,
        filters: &[(String, FilterOp)],
        order_by: &str,
        ascending: bool,
        limit: Option<usize>,
        offset: Option<usize>,
    ) -> Result<Vec<String>, StorageError> {
        self.ensure_open()?;
        let table = map_table_name(table);
        let order_by = map_col(order_by);
        let dialect = self.inner.dialect();
        let mut sql = format!("SELECT id FROM {}", table);
        let mut params = DbParams::new();
        let mut idx = 0usize;

        if !filters.is_empty() {
            sql.push_str(" WHERE ");
            let mut parts = Vec::new();
            for (col, op) in filters {
                let mapped_col = map_col(col);
                match op {
                    FilterOp::Eq(v) => {
                        parts.push(format!("{} = {}", mapped_col, dialect.placeholder(idx)));
                        params.push(v.as_ref());
                        idx += 1;
                    }
                    FilterOp::IsNull => {
                        parts.push(format!("{} IS NULL", mapped_col));
                    }
                    FilterOp::IsNotNull => {
                        parts.push(format!("{} IS NOT NULL", mapped_col));
                    }
                    // Bind integer filter operands as i64 so PG bigint columns
                    // (due_time / lock_time / …) compare correctly (P73b).
                    FilterOp::GreaterThan(v) => {
                        parts.push(format!("{} > {}", mapped_col, dialect.placeholder(idx)));
                        params.push(*v);
                        idx += 1;
                    }
                    FilterOp::GreaterThanOrEqual(v) => {
                        parts.push(format!("{} >= {}", mapped_col, dialect.placeholder(idx)));
                        params.push(*v);
                        idx += 1;
                    }
                    FilterOp::LessThan(v) => {
                        parts.push(format!("{} < {}", mapped_col, dialect.placeholder(idx)));
                        params.push(*v);
                        idx += 1;
                    }
                    FilterOp::LessThanOrEqual(v) => {
                        parts.push(format!("{} <= {}", mapped_col, dialect.placeholder(idx)));
                        params.push(*v);
                        idx += 1;
                    }
                    FilterOp::Like(v) => {
                        parts.push(format!("{} LIKE {}", mapped_col, dialect.placeholder(idx)));
                        params.push(v.as_ref());
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
                        parts.push(format!("{} IN ({})", mapped_col, placeholders.join(", ")));
                        for v in vals {
                            params.push(v.as_str());
                        }
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

        let rows = self.inner.select_raw(RenderedStatement::new(sql, params))?;
        Ok(rows.iter().filter_map(|r| r.get_text("id")).collect())
    }

    pub fn find_with_filters<T: serde::de::DeserializeOwned>(
        &mut self,
        table: &str,
        filters: &[(String, FilterOp)],
        order_by: Option<(&str, bool)>,
        limit: Option<usize>,
    ) -> Result<Vec<T>, StorageError> {
        self.ensure_open()?;
        let table = map_table_name(table);
        let (order_col, ascending) = order_by.unwrap_or(("id", true));
        let order_col = map_col(order_col);
        let dialect = self.inner.dialect();
        let mut sql = format!("SELECT id, data FROM {}", table);
        let mut params = DbParams::new();
        let mut idx = 0usize;

        if !filters.is_empty() {
            sql.push_str(" WHERE ");
            let mut parts = Vec::new();
            for (col, op) in filters {
                let mapped_col = map_col(col);
                match op {
                    FilterOp::Eq(v) => {
                        parts.push(format!("{} = {}", mapped_col, dialect.placeholder(idx)));
                        params.push(v.as_ref());
                        idx += 1;
                    }
                    FilterOp::IsNull => {
                        parts.push(format!("{} IS NULL", mapped_col));
                    }
                    FilterOp::IsNotNull => {
                        parts.push(format!("{} IS NOT NULL", mapped_col));
                    }
                    FilterOp::GreaterThan(v) => {
                        parts.push(format!("{} > {}", mapped_col, dialect.placeholder(idx)));
                        params.push(*v);
                        idx += 1;
                    }
                    FilterOp::GreaterThanOrEqual(v) => {
                        parts.push(format!("{} >= {}", mapped_col, dialect.placeholder(idx)));
                        params.push(*v);
                        idx += 1;
                    }
                    FilterOp::LessThan(v) => {
                        parts.push(format!("{} < {}", mapped_col, dialect.placeholder(idx)));
                        params.push(*v);
                        idx += 1;
                    }
                    FilterOp::LessThanOrEqual(v) => {
                        parts.push(format!("{} <= {}", mapped_col, dialect.placeholder(idx)));
                        params.push(*v);
                        idx += 1;
                    }
                    FilterOp::Like(v) => {
                        parts.push(format!("{} LIKE {}", mapped_col, dialect.placeholder(idx)));
                        params.push(v.as_ref());
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
                        parts.push(format!("{} IN ({})", mapped_col, placeholders.join(", ")));
                        for v in vals {
                            params.push(v.as_str());
                        }
                    }
                }
            }
            sql.push_str(&parts.join(" AND "));
        }

        sql.push_str(&format!(
            " ORDER BY {} {}",
            order_col,
            if ascending { "ASC" } else { "DESC" }
        ));

        if let Some(lim) = limit {
            sql.push_str(&format!(" {}", dialect.limit_offset(Some(lim), None)));
        }

        let rows = self.inner.select_raw(RenderedStatement::new(sql, params))?;
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            if let Some(data) = row.get_text("data") {
                out.push(
                    serde_json::from_str::<T>(&data)
                        .map_err(|e| StorageError::Deserialization(e.to_string()))?,
                );
            }
        }
        Ok(out)
    }

    pub fn find_raw(&mut self, table: &str, id: &str) -> Result<Option<RawRow>, StorageError> {
        self.ensure_open()?;
        let table = map_table_name(table);
        let dialect = self.inner.dialect();
        let sql = format!(
            "SELECT * FROM {} WHERE id = {}",
            table,
            dialect.placeholder(0)
        );
        let mut params = DbParams::new();
        params.push(id);
        let row = self
            .inner
            .select_one_raw(RenderedStatement::new(sql, params))?;
        Ok(row.and_then(row_to_raw_row))
    }

    pub fn insert_blob(
        &mut self,
        table: &str,
        id: &str,
        cols: &[(&str, &str)],
        blob_col: &str,
        blob: &[u8],
    ) -> Result<(), StorageError> {
        self.ensure_open()?;
        let table = map_table_name(table);
        let blob_col_mapped = map_col(blob_col);
        let dialect = self.inner.dialect();

        let mapped_cols: Vec<(String, String)> = cols
            .iter()
            .map(|(k, v)| (map_col(k), v.to_string()))
            .collect();

        let mut column_refs: Vec<&str> = vec!["id"];
        for (k, _) in &mapped_cols {
            column_refs.push(k.as_str());
        }
        column_refs.push(blob_col_mapped.as_str());

        let n = column_refs.len();
        let placeholders: Vec<String> = (0..n).map(|i| dialect.placeholder(i)).collect();

        let sql = format!(
            "{} {} ({}) VALUES ({})",
            dialect.insert_or_replace_into(),
            table,
            column_refs.join(", "),
            placeholders.join(", ")
        );

        let mut params = DbParams::new();
        params.push(id);
        for (_, v) in &mapped_cols {
            params.push(v.as_str());
        }
        params.push(blob.to_vec());

        self.inner
            .execute_raw(RenderedStatement::new(sql, params))?;
        Ok(())
    }

    pub fn find_blob(
        &mut self,
        table: &str,
        col: &str,
        val: &str,
        blob_col: &str,
    ) -> Result<Option<Vec<u8>>, StorageError> {
        self.ensure_open()?;
        let table = map_table_name(table);
        let col = map_col(col);
        let blob_col = map_col(blob_col);
        let dialect = self.inner.dialect();
        let sql = format!(
            "SELECT {} FROM {} WHERE {} = {}",
            blob_col,
            table,
            col,
            dialect.placeholder(0)
        );
        let mut params = DbParams::new();
        params.push(val);
        let row = self
            .inner
            .select_one_raw(RenderedStatement::new(sql, params))?;
        Ok(row.and_then(|r| r.get_blob(&blob_col)))
    }

    pub fn find_blob_by_two(
        &mut self,
        table: &str,
        col1: &str,
        val1: &str,
        col2: &str,
        val2: &str,
        blob_col: &str,
    ) -> Result<Option<Vec<u8>>, StorageError> {
        self.ensure_open()?;
        let table = map_table_name(table);
        let col1 = map_col(col1);
        let col2 = map_col(col2);
        let blob_col = map_col(blob_col);
        let dialect = self.inner.dialect();
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
        let row = self
            .inner
            .select_one_raw(RenderedStatement::new(sql, params))?;
        Ok(row.and_then(|r| r.get_blob(&blob_col)))
    }

    pub fn max(
        &mut self,
        table: &str,
        col: &str,
        conditions: &[(String, String)],
    ) -> Result<Option<i64>, StorageError> {
        self.ensure_open()?;
        let table = map_table_name(table);
        let col = map_col(col);
        let dialect = self.inner.dialect();

        let mapped_conds: Vec<(String, String)> = conditions
            .iter()
            .map(|(k, v)| (map_col(k), v.clone()))
            .collect();

        let mut sql = format!("SELECT MAX({}) AS RES_ FROM {}", col, table);
        let mut params = DbParams::new();

        if !mapped_conds.is_empty() {
            sql.push_str(" WHERE ");
            let mut cond_parts = Vec::new();
            for (i, (c, _)) in mapped_conds.iter().enumerate() {
                cond_parts.push(format!("{} = {}", c, dialect.placeholder(i)));
            }
            sql.push_str(&cond_parts.join(" AND "));

            for (_, v) in &mapped_conds {
                params.push(v.as_str());
            }
        }

        let row = self
            .inner
            .select_one_raw(RenderedStatement::new(sql, params))?;
        Ok(row.and_then(|r| r.get_integer("RES_")))
    }

    pub fn count(
        &mut self,
        table: &str,
        conditions: &[(String, String)],
    ) -> Result<i64, StorageError> {
        self.ensure_open()?;
        let table = map_table_name(table);
        let dialect = self.inner.dialect();

        let mapped_conds: Vec<(String, String)> = conditions
            .iter()
            .map(|(k, v)| (map_col(k), v.clone()))
            .collect();

        let mut sql = format!("SELECT COUNT(*) AS RES_ FROM {}", table);
        let mut params = DbParams::new();

        if !mapped_conds.is_empty() {
            sql.push_str(" WHERE ");
            let mut cond_parts = Vec::new();
            for (i, (c, _)) in mapped_conds.iter().enumerate() {
                cond_parts.push(format!("{} = {}", c, dialect.placeholder(i)));
            }
            sql.push_str(&cond_parts.join(" AND "));

            for (_, v) in &mapped_conds {
                params.push(v.as_str());
            }
        }

        let row = self
            .inner
            .select_one_raw(RenderedStatement::new(sql, params))?;
        Ok(row.and_then(|r| r.get_integer("RES_")).unwrap_or(0))
    }

    pub fn flush(&mut self) -> Result<(), StorageError> {
        self.ensure_open()?;
        self.inner.flush()?;
        Ok(())
    }

    pub fn flush_and_commit(&mut self) -> Result<(), StorageError> {
        self.ensure_open()?;
        self.inner.commit()?;
        self.closed = true;
        Ok(())
    }

    pub fn rollback(&mut self) -> Result<(), StorageError> {
        if self.closed {
            return Err(StorageError::ClosedTransaction);
        }
        self.inner.rollback()?;
        self.closed = true;
        Ok(())
    }

    pub fn execute_raw_sql(&mut self, sql: &str) -> Result<(), StorageError> {
        self.ensure_open()?;
        self.inner
            .execute_raw(RenderedStatement::new(sql.to_string(), DbParams::new()))?;
        Ok(())
    }

    pub fn execute_raw(&mut self, sql: &str, params: DbParams) -> Result<u64, StorageError> {
        self.ensure_open()?;
        let dialect = self.inner.dialect();
        let translated = translate_placeholders(sql, dialect);
        let result = self
            .inner
            .execute_raw(RenderedStatement::new(translated, params))?;
        Ok(result.rows_affected)
    }

    pub fn raw_query(&mut self, sql: &str, params: DbParams) -> Result<Vec<DbRow>, StorageError> {
        self.ensure_open()?;
        let dialect = self.inner.dialect();
        let translated = translate_placeholders(sql, dialect);
        let rows = self
            .inner
            .select_raw(RenderedStatement::new(translated, params))?;
        Ok(rows)
    }

    /// Upsert one row by primary key with an explicit column list, dispatched per
    /// backend: SQLite `INSERT OR REPLACE`, MySQL `REPLACE INTO`, Postgres
    /// `INSERT ... ON CONFLICT (pk) DO UPDATE`. Delegates to
    /// [`flowable_persistence::DbSession::upsert_raw`].
    ///
    /// `columns` must list every column in the same order as `params`, with
    /// `pk_column` among them. For the `id`/`data` JSON-entity shape prefer
    /// [`Self::insert_with_extra`]; this is for the fully-projected tables (content
    /// items, form instances) that have no `data`-plus-extras structure.
    ///
    /// Writing `INSERT OR REPLACE` by hand instead is a portability bug: it is a
    /// syntax error on both MySQL and Postgres.
    pub fn upsert_raw(
        &mut self,
        table: &str,
        pk_column: &str,
        columns: &[&str],
        params: DbParams,
    ) -> Result<u64, StorageError> {
        self.ensure_open()?;
        let result = self.inner.upsert_raw(table, pk_column, columns, params)?;
        Ok(result.rows_affected)
    }

    /// Column metadata for `table`, dispatched by backend; see
    /// `flowable_persistence::DbSession::table_columns`.
    pub fn table_columns(&mut self, table: &str) -> Result<Vec<ColumnInfo>, StorageError> {
        self.ensure_open()?;
        Ok(self.inner.table_columns(table)?)
    }

    pub fn raw_query_one(
        &mut self,
        sql: &str,
        params: DbParams,
    ) -> Result<Option<DbRow>, StorageError> {
        self.ensure_open()?;
        let dialect = self.inner.dialect();
        let translated = translate_placeholders(sql, dialect);
        let row = self
            .inner
            .select_one_raw(RenderedStatement::new(translated, params))?;
        Ok(row)
    }

    pub fn upsert_deployment_resource(
        &mut self,
        deployment_id: &str,
        name: &str,
        resource_type: &str,
        content_type: &str,
        bytes: &[u8],
        created_at: i64,
    ) -> Result<(), StorageError> {
        self.ensure_open()?;
        let table = map_table_name("deployment_resources");
        let dialect = self.inner.dialect();

        let sql = format!(
            "{} {} (deployment_id, name, resource_type, content_type, created_at, bytes) VALUES ({}, {}, {}, {}, {}, {})",
            dialect.insert_or_replace_into(),
            table,
            dialect.placeholder(0),
            dialect.placeholder(1),
            dialect.placeholder(2),
            dialect.placeholder(3),
            dialect.placeholder(4),
            dialect.placeholder(5),
        );

        let mut params = DbParams::new();
        params.push(deployment_id);
        params.push(name);
        params.push(resource_type);
        params.push(content_type);
        params.push(created_at);
        params.push(bytes.to_vec());

        self.inner
            .execute_raw(RenderedStatement::new(sql, params))?;
        Ok(())
    }

    pub fn list_deployment_resource_names(
        &mut self,
        deployment_id: &str,
    ) -> Result<Vec<String>, StorageError> {
        self.ensure_open()?;
        let table = map_table_name("deployment_resources");
        let dialect = self.inner.dialect();
        let sql = format!(
            "SELECT name FROM {} WHERE deployment_id = {} ORDER BY name ASC",
            table,
            dialect.placeholder(0)
        );
        let mut params = DbParams::new();
        params.push(deployment_id);
        let rows = self.inner.select_raw(RenderedStatement::new(sql, params))?;
        Ok(rows
            .into_iter()
            .filter_map(|r| r.get_text("name"))
            .collect())
    }

    pub fn find_deployment_resource(
        &mut self,
        deployment_id: &str,
        name: &str,
    ) -> Result<Option<crate::repository::deployment_resource::DeploymentResource>, StorageError>
    {
        self.ensure_open()?;
        let table = map_table_name("deployment_resources");
        let dialect = self.inner.dialect();
        let sql = format!(
            "SELECT deployment_id, name, resource_type, content_type, created_at, bytes FROM {} WHERE deployment_id = {} AND name = {}",
            table,
            dialect.placeholder(0),
            dialect.placeholder(1)
        );
        let mut params = DbParams::new();
        params.push(deployment_id);
        params.push(name);
        let row = self
            .inner
            .select_one_raw(RenderedStatement::new(sql, params))?;
        Ok(row.map(|r| {
            let dep_id = r.get_text("deployment_id").unwrap_or_default();
            let res_name = r.get_text("name").unwrap_or_default();
            let res_type = r.get_text("resource_type").filter(|s| !s.is_empty());
            let content_type = r.get_text("content_type").filter(|s| !s.is_empty());
            let created_at = r.get_integer("created_at").filter(|&v| v != 0);
            let bytes = r.get_blob("bytes").unwrap_or_default();
            crate::repository::deployment_resource::DeploymentResource::from_stored(
                dep_id,
                res_name,
                res_type,
                content_type,
                bytes,
                created_at,
            )
        }))
    }

    pub fn list_deployment_resources(
        &mut self,
        deployment_id: &str,
    ) -> Result<Vec<crate::repository::deployment_resource::DeploymentResource>, StorageError> {
        self.ensure_open()?;
        let table = map_table_name("deployment_resources");
        let dialect = self.inner.dialect();
        let sql = format!(
            "SELECT deployment_id, name, resource_type, content_type, created_at, bytes FROM {} WHERE deployment_id = {} ORDER BY name ASC",
            table,
            dialect.placeholder(0)
        );
        let mut params = DbParams::new();
        params.push(deployment_id);
        let rows = self.inner.select_raw(RenderedStatement::new(sql, params))?;
        Ok(rows
            .into_iter()
            .map(|r| {
                let dep_id = r.get_text("deployment_id").unwrap_or_default();
                let res_name = r.get_text("name").unwrap_or_default();
                let res_type = r.get_text("resource_type").filter(|s| !s.is_empty());
                let content_type = r.get_text("content_type").filter(|s| !s.is_empty());
                let created_at = r.get_integer("created_at").filter(|&v| v != 0);
                let bytes = r.get_blob("bytes").unwrap_or_default();
                crate::repository::deployment_resource::DeploymentResource::from_stored(
                    dep_id,
                    res_name,
                    res_type,
                    content_type,
                    bytes,
                    created_at,
                )
            })
            .collect())
    }

    pub fn iter_all_deployment_resource_bytes(
        &mut self,
    ) -> Result<Vec<(String, String, Vec<u8>)>, StorageError> {
        self.ensure_open()?;
        let table = map_table_name("deployment_resources");
        let sql = format!("SELECT deployment_id, name, bytes FROM {}", table);
        let rows = self
            .inner
            .select_raw(RenderedStatement::new(sql, DbParams::new()))?;
        Ok(rows
            .into_iter()
            .filter_map(|r| {
                let dep_id = r.get_text("deployment_id")?;
                let name = r.get_text("name")?;
                let bytes = r.get_blob("bytes")?;
                Some((dep_id, name, bytes))
            })
            .collect())
    }

    pub fn next_process_definition_version(
        &mut self,
        tenant_id: &str,
        process_key: &str,
    ) -> Result<i32, StorageError> {
        self.ensure_open()?;
        let table = map_table_name("process_definition_versions");

        let (ph0, ph1) = {
            let dialect = self.inner.dialect();
            (dialect.placeholder(0), dialect.placeholder(1))
        };
        let sql = format!(
            "SELECT MAX(version) AS RES_ FROM {} WHERE tenant_id = {} AND process_key = {}",
            table, ph0, ph1
        );
        let mut params = DbParams::new();
        params.push(tenant_id);
        params.push(process_key);

        let row = self
            .inner
            .select_one_raw(RenderedStatement::new(sql, params))?;
        let current = row.and_then(|r| r.get_integer("RES_")).unwrap_or(0);
        let next = current + 1;

        let dialect = self.inner.dialect();
        let (ph0, ph1, ph2) = (
            dialect.placeholder(0),
            dialect.placeholder(1),
            dialect.placeholder(2),
        );
        let mut upsert_sql = format!(
            "{} {} (tenant_id, process_key, version) VALUES ({}, {}, {})",
            dialect.insert_or_replace_into(),
            table,
            ph0,
            ph1,
            ph2
        );
        if dialect.supports_on_conflict_update() {
            upsert_sql.push_str(
                &dialect.on_conflict_do_update_suffix("tenant_id,process_key", &["version"]),
            );
        }
        let mut upsert_params = DbParams::new();
        upsert_params.push(tenant_id);
        upsert_params.push(process_key);
        upsert_params.push(next);
        self.inner
            .execute_raw(RenderedStatement::new(upsert_sql, upsert_params))?;
        Ok(next as i32)
    }

    /// Allocates the next event-registry change revision from the single-row
    /// allocator table `event_registry_change_revision_seq`. The `UPDATE` takes
    /// a write lock inside the session transaction, so concurrent allocators
    /// serialize instead of both reading the same `MAX(revision)` and
    /// colliding. Revisions are therefore strictly monotonic and unique.
    pub fn next_event_registry_change_revision(&mut self) -> Result<u64, StorageError> {
        self.ensure_open()?;
        const SEQ_ID: &str = "event-registry";
        let seq_table = map_table_name("event_registry_change_revision_seq");

        let mut params = DbParams::new();
        params.push(SEQ_ID);
        let updated = self.execute_raw(
            &format!("UPDATE {seq_table} SET next_revision = next_revision + 1 WHERE id = ?"),
            params,
        )?;
        if updated == 0 {
            // Databases created before the allocator seed existed: start from
            // the current change-log high water mark so revisions stay
            // monotonic relative to already persisted records.
            let records_table = map_table_name("event_registry_change_records");
            let row = self.raw_query_one(
                &format!("SELECT MAX(revision) AS RES_ FROM {records_table}"),
                DbParams::new(),
            )?;
            let current = row
                .and_then(|r| r.get_integer("RES_"))
                .unwrap_or(0)
                .max(0) as u64;
            let next = current + 1;
            let mut params = DbParams::new();
            params.push(SEQ_ID);
            params.push(next as i64);
            self.execute_raw(
                &format!("INSERT INTO {seq_table} (id, next_revision) VALUES (?, ?)"),
                params,
            )?;
            return Ok(next);
        }

        let mut params = DbParams::new();
        params.push(SEQ_ID);
        let row = self.raw_query_one(
            &format!("SELECT next_revision AS RES_ FROM {seq_table} WHERE id = ?"),
            params,
        )?;
        row.and_then(|r| r.get_integer("RES_"))
            .map(|value| value as u64)
            .ok_or_else(|| {
                StorageError::Persistence(
                    "event registry change revision allocator row missing after update".to_string(),
                )
            })
    }

    pub fn insert_process_definition_version(
        &mut self,
        tenant_id: &str,
        process_key: &str,
        version: i32,
    ) -> Result<(), StorageError> {
        self.ensure_open()?;
        let table = map_table_name("process_definition_versions");
        let dialect = self.inner.dialect();
        let mut sql = format!(
            "{} {} (tenant_id, process_key, version) VALUES ({}, {}, {})",
            dialect.insert_or_replace_into(),
            table,
            dialect.placeholder(0),
            dialect.placeholder(1),
            dialect.placeholder(2),
        );
        if dialect.supports_on_conflict_update() {
            sql.push_str(
                &dialect.on_conflict_do_update_suffix("tenant_id,process_key", &["version"]),
            );
        }
        let mut params = DbParams::new();
        params.push(tenant_id);
        params.push(process_key);
        params.push(version as i64);
        self.inner
            .execute_raw(RenderedStatement::new(sql, params))?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn insert_repository_model(
        &mut self,
        id: &str,
        data_json: &str,
        deployment_id: &str,
        model_key: &str,
        tenant_id: &str,
        source_bytes: &[u8],
        source_extra_bytes: &[u8],
    ) -> Result<(), StorageError> {
        self.ensure_open()?;
        let table = map_table_name("repository_models");
        let dialect = self.inner.dialect();
        let sql = format!(
            "{} {} (id, deployment_id, model_key, tenant_id, data, source_bytes, source_extra_bytes) VALUES ({}, {}, {}, {}, {}, {}, {})",
            dialect.insert_or_replace_into(),
            table,
            dialect.placeholder(0),
            dialect.placeholder(1),
            dialect.placeholder(2),
            dialect.placeholder(3),
            dialect.placeholder(4),
            dialect.placeholder(5),
            dialect.placeholder(6),
        );
        let mut params = DbParams::new();
        params.push(id);
        params.push(deployment_id);
        params.push(model_key);
        params.push(tenant_id);
        params.push(data_json);
        params.push(source_bytes.to_vec());
        params.push(source_extra_bytes.to_vec());
        self.inner
            .execute_raw(RenderedStatement::new(sql, params))?;
        Ok(())
    }

    pub fn update_repository_model_data(
        &mut self,
        id: &str,
        data_json: &str,
        deployment_id: &str,
        model_key: &str,
        tenant_id: &str,
    ) -> Result<(), StorageError> {
        self.ensure_open()?;
        let table = map_table_name("repository_models");
        let dialect = self.inner.dialect();
        let sql = format!(
            "UPDATE {} SET data = {}, deployment_id = {}, model_key = {}, tenant_id = {} WHERE id = {}",
            table,
            dialect.placeholder(0),
            dialect.placeholder(1),
            dialect.placeholder(2),
            dialect.placeholder(3),
            dialect.placeholder(4),
        );
        let mut params = DbParams::new();
        params.push(data_json);
        params.push(deployment_id);
        params.push(model_key);
        params.push(tenant_id);
        params.push(id);
        self.inner
            .execute_raw(RenderedStatement::new(sql, params))?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn update_repository_model_blob(
        &mut self,
        id: &str,
        data_json: &str,
        deployment_id: &str,
        model_key: &str,
        tenant_id: &str,
        blob_col: &str,
        blob: &[u8],
    ) -> Result<(), StorageError> {
        self.ensure_open()?;
        let table = map_table_name("repository_models");
        let blob_col_mapped = map_col(blob_col);
        let dialect = self.inner.dialect();
        let sql = format!(
            "UPDATE {} SET data = {}, deployment_id = {}, model_key = {}, tenant_id = {}, {} = {} WHERE id = {}",
            table,
            dialect.placeholder(0),
            dialect.placeholder(1),
            dialect.placeholder(2),
            dialect.placeholder(3),
            blob_col_mapped,
            dialect.placeholder(4),
            dialect.placeholder(5),
        );
        let mut params = DbParams::new();
        params.push(data_json);
        params.push(deployment_id);
        params.push(model_key);
        params.push(tenant_id);
        params.push(blob.to_vec());
        params.push(id);
        self.inner
            .execute_raw(RenderedStatement::new(sql, params))?;
        Ok(())
    }

    pub fn list_engine_properties(&mut self) -> Result<Vec<EnginePropertyRow>, StorageError> {
        self.ensure_open()?;
        let rows = self.inner.property_list()?;
        Ok(rows
            .into_iter()
            .map(|(name, value, revision)| EnginePropertyRow {
                name,
                value,
                revision,
            })
            .collect())
    }

    pub fn find_engine_property(
        &mut self,
        name: &str,
    ) -> Result<Option<EnginePropertyRow>, StorageError> {
        self.ensure_open()?;
        match self.inner.property_get(name)? {
            Some((value, revision)) => Ok(Some(EnginePropertyRow {
                name: name.to_string(),
                value,
                revision,
            })),
            None => Ok(None),
        }
    }

    pub fn create_engine_property(&mut self, name: &str, value: &str) -> Result<(), StorageError> {
        self.ensure_open()?;
        self.inner.property_insert(name, value)?;
        Ok(())
    }

    pub fn update_engine_property(
        &mut self,
        name: &str,
        value: &str,
    ) -> Result<bool, StorageError> {
        self.ensure_open()?;
        Ok(self.inner.property_update(name, value)?)
    }

    /// Optimistic-locking property update: succeeds only when `REV_` matches.
    pub fn update_engine_property_if_revision(
        &mut self,
        name: &str,
        value: &str,
        expected_rev: i32,
    ) -> Result<bool, StorageError> {
        self.ensure_open()?;
        Ok(self
            .inner
            .property_update_if_revision(name, value, expected_rev)?)
    }

    pub fn delete_engine_property(&mut self, name: &str) -> Result<bool, StorageError> {
        self.ensure_open()?;
        Ok(self.inner.property_delete(name)?)
    }
}

impl Drop for DbSession {
    fn drop(&mut self) {
        if !self.closed {
            let _ = self.inner.rollback();
        }
    }
}

fn translate_placeholders(sql: &str, dialect: &dyn SqlDialect) -> String {
    use flowable_persistence::config::DatabaseKind;
    match dialect.database_kind() {
        DatabaseKind::Postgres => {
            let mut result = String::with_capacity(sql.len());
            let mut idx = 1usize;
            let chars = sql.chars().peekable();
            let mut in_single_quote = false;
            for c in chars {
                if c == '\'' {
                    in_single_quote = !in_single_quote;
                    result.push(c);
                } else if c == '?' && !in_single_quote {
                    result.push_str(&format!("${}", idx));
                    idx += 1;
                } else {
                    result.push(c);
                }
            }
            result
        }
        _ => sql.to_string(),
    }
}
