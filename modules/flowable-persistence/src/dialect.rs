use crate::config::DatabaseKind;

pub trait SqlDialect: Send + Sync {
    fn database_kind(&self) -> DatabaseKind;
    fn placeholder(&self, index: usize) -> String;
    fn limit_offset(&self, limit: Option<usize>, offset: Option<usize>) -> String;
    fn bool_literal(&self, value: bool) -> &'static str;
    fn blob_type(&self) -> &'static str;
    /// Unbounded / large text. MySQL uses VARCHAR for PK-safe short strings via
    /// [`Self::varchar_type`]; this is for free-form payload columns.
    fn text_type(&self) -> &'static str {
        "TEXT"
    }
    /// Short string type safe for PRIMARY KEY / INDEX on all backends.
    fn varchar_type(&self, len: usize) -> String {
        format!("VARCHAR({len})")
    }
    fn integer_type(&self) -> &'static str;
    fn bigint_type(&self) -> &'static str;
    fn now_millis_expr(&self) -> &'static str;
    fn supports_returning(&self) -> bool;
    /// Whether this dialect can emit `FOR UPDATE SKIP LOCKED`.
    ///
    /// # C4 decision (2026-08-01): delete unused SKIP LOCKED acquire statements
    ///
    /// `StatementId::AcquireDueTimerJobs` / `AcquireDueAsyncJobs` and
    /// `TimerJobDataManager::acquire_due` were pure dead code (zero callers).
    /// Production acquisition already has two strategies on the engine runtime
    /// store path:
    /// 1. **Optimistic CAS** — per-row revision update (`AcquisitionWritePolicy::Optimistic`)
    /// 2. **SerializedByGlobalLock** — cluster-wide lock then bulk write
    ///
    /// Wiring SKIP LOCKED as a third strategy was rejected because:
    /// - It targeted ACT_RU_* relational rows, while acquire runs against the
    ///   dual-write JSON `timer_job_states` path (`runtime_store.rs`), so
    ///   "just wire it" would not reduce contention without a larger cutover.
    /// - The stub was incomplete (wrong param binding; SELECT without lock write).
    /// - High-contention waste is already addressed by global acquire lock.
    ///
    /// Dialect capability hooks remain for a future explicit design if/when
    /// acquisition moves fully onto ACT_RU_* with row-level locks.
    fn supports_skip_locked(&self) -> bool;
    fn for_update_skip_locked(&self) -> &'static str;
    fn quote_identifier(&self, identifier: &str) -> String;
    fn insert_or_replace_into(&self) -> &'static str;
    fn supports_on_conflict_update(&self) -> bool;
    fn on_conflict_do_update_suffix(&self, pk_column: &str, columns: &[&str]) -> String;
    /// CREATE INDEX statement. MySQL does not support `IF NOT EXISTS` on indexes
    /// in older versions; callers should treat duplicate-index errors as success.
    fn create_index_if_not_exists(&self, index_name: &str, table: &str, columns: &str) -> String {
        format!("CREATE INDEX IF NOT EXISTS {index_name} ON {table}({columns})")
    }
}

/// Render a single-row upsert keyed on `pk_column`, dispatched per backend:
/// SQLite `INSERT OR REPLACE`, MySQL `REPLACE INTO`, Postgres
/// `INSERT ... ON CONFLICT (pk) DO UPDATE`.
///
/// `columns` is the full column list, in the same order the caller binds its
/// parameters, and must contain `pk_column`; every other column is refreshed on
/// conflict. [`crate::DbSession::upsert_raw`] is the only caller — it lives here
/// so the SQL it emits is testable per backend without a database.
pub fn render_upsert(
    dialect: &dyn SqlDialect,
    table: &str,
    pk_column: &str,
    columns: &[&str],
) -> String {
    let placeholders = (0..columns.len())
        .map(|index| dialect.placeholder(index))
        .collect::<Vec<_>>();
    let mut sql = format!(
        "{} {} ({}) VALUES ({})",
        dialect.insert_or_replace_into(),
        table,
        columns.join(", "),
        placeholders.join(", ")
    );
    if dialect.supports_on_conflict_update() {
        let update_columns = columns
            .iter()
            .copied()
            .filter(|column| !column.eq_ignore_ascii_case(pk_column))
            .collect::<Vec<_>>();
        sql.push_str(&dialect.on_conflict_do_update_suffix(pk_column, &update_columns));
    }
    sql
}

#[derive(Debug, Clone, Copy)]
pub struct MemoryDialect;

impl SqlDialect for MemoryDialect {
    fn database_kind(&self) -> DatabaseKind {
        DatabaseKind::Memory
    }

    fn placeholder(&self, index: usize) -> String {
        format!("${}", index)
    }

    fn limit_offset(&self, limit: Option<usize>, offset: Option<usize>) -> String {
        match (limit, offset) {
            (Some(l), Some(o)) => format!("LIMIT {} OFFSET {}", l, o),
            (Some(l), None) => format!("LIMIT {}", l),
            (None, Some(o)) => format!("LIMIT -1 OFFSET {}", o),
            (None, None) => String::new(),
        }
    }

    fn bool_literal(&self, value: bool) -> &'static str {
        if value { "1" } else { "0" }
    }

    fn blob_type(&self) -> &'static str {
        "BLOB"
    }

    fn integer_type(&self) -> &'static str {
        "INTEGER"
    }

    fn bigint_type(&self) -> &'static str {
        "INTEGER"
    }

    fn now_millis_expr(&self) -> &'static str {
        "CURRENT_TIMESTAMP"
    }

    fn supports_returning(&self) -> bool {
        false
    }

    fn supports_skip_locked(&self) -> bool {
        false
    }

    fn for_update_skip_locked(&self) -> &'static str {
        ""
    }

    fn quote_identifier(&self, identifier: &str) -> String {
        format!("\"{}\"", identifier)
    }

    fn insert_or_replace_into(&self) -> &'static str {
        "INSERT OR REPLACE INTO"
    }

    fn supports_on_conflict_update(&self) -> bool {
        false
    }

    fn on_conflict_do_update_suffix(&self, _pk_column: &str, _columns: &[&str]) -> String {
        String::new()
    }
}

#[derive(Debug, Clone, Copy)]
pub struct SqliteDialect;

impl SqlDialect for SqliteDialect {
    fn database_kind(&self) -> DatabaseKind {
        DatabaseKind::Sqlite
    }

    fn placeholder(&self, _index: usize) -> String {
        "?".to_string()
    }

    fn limit_offset(&self, limit: Option<usize>, offset: Option<usize>) -> String {
        match (limit, offset) {
            (Some(l), Some(o)) => format!("LIMIT {} OFFSET {}", l, o),
            (Some(l), None) => format!("LIMIT {}", l),
            (None, Some(o)) => format!("LIMIT -1 OFFSET {}", o),
            (None, None) => String::new(),
        }
    }

    fn bool_literal(&self, value: bool) -> &'static str {
        if value { "1" } else { "0" }
    }

    fn blob_type(&self) -> &'static str {
        "BLOB"
    }

    fn integer_type(&self) -> &'static str {
        "INTEGER"
    }

    fn bigint_type(&self) -> &'static str {
        "INTEGER"
    }

    fn now_millis_expr(&self) -> &'static str {
        "(strftime('%s', 'now') * 1000)"
    }

    fn supports_returning(&self) -> bool {
        true
    }

    fn supports_skip_locked(&self) -> bool {
        false
    }

    fn for_update_skip_locked(&self) -> &'static str {
        ""
    }

    fn quote_identifier(&self, identifier: &str) -> String {
        format!("\"{}\"", identifier)
    }

    fn insert_or_replace_into(&self) -> &'static str {
        "INSERT OR REPLACE INTO"
    }

    fn supports_on_conflict_update(&self) -> bool {
        false
    }

    fn on_conflict_do_update_suffix(&self, _pk_column: &str, _columns: &[&str]) -> String {
        String::new()
    }
}

#[derive(Debug, Clone, Copy)]
pub struct PostgresDialect;

impl SqlDialect for PostgresDialect {
    fn database_kind(&self) -> DatabaseKind {
        DatabaseKind::Postgres
    }

    fn placeholder(&self, index: usize) -> String {
        format!("${}", index + 1)
    }

    fn limit_offset(&self, limit: Option<usize>, offset: Option<usize>) -> String {
        match (limit, offset) {
            (Some(l), Some(o)) => format!("LIMIT {} OFFSET {}", l, o),
            (Some(l), None) => format!("LIMIT {}", l),
            (None, Some(o)) => format!("OFFSET {}", o),
            (None, None) => String::new(),
        }
    }

    fn bool_literal(&self, value: bool) -> &'static str {
        if value { "TRUE" } else { "FALSE" }
    }

    fn blob_type(&self) -> &'static str {
        "BYTEA"
    }

    fn integer_type(&self) -> &'static str {
        "INTEGER"
    }

    fn bigint_type(&self) -> &'static str {
        "BIGINT"
    }

    fn now_millis_expr(&self) -> &'static str {
        "(EXTRACT(EPOCH FROM NOW()) * 1000)"
    }

    fn supports_returning(&self) -> bool {
        true
    }

    fn supports_skip_locked(&self) -> bool {
        true
    }

    fn for_update_skip_locked(&self) -> &'static str {
        "FOR UPDATE SKIP LOCKED"
    }

    fn quote_identifier(&self, identifier: &str) -> String {
        format!("\"{}\"", identifier)
    }

    fn insert_or_replace_into(&self) -> &'static str {
        "INSERT INTO"
    }

    fn supports_on_conflict_update(&self) -> bool {
        true
    }

    fn on_conflict_do_update_suffix(&self, pk_column: &str, columns: &[&str]) -> String {
        let set_clauses: Vec<String> = columns
            .iter()
            .map(|col| format!("{0} = EXCLUDED.{0}", col))
            .collect();
        format!(
            " ON CONFLICT ({}) DO UPDATE SET {}",
            pk_column,
            set_clauses.join(", ")
        )
    }
}

#[derive(Debug, Clone, Copy)]
pub struct MysqlDialect;

impl SqlDialect for MysqlDialect {
    fn database_kind(&self) -> DatabaseKind {
        DatabaseKind::Mysql
    }

    fn placeholder(&self, _index: usize) -> String {
        "?".to_string()
    }

    fn limit_offset(&self, limit: Option<usize>, offset: Option<usize>) -> String {
        match (limit, offset) {
            (Some(l), Some(o)) => format!("LIMIT {} OFFSET {}", l, o),
            (Some(l), None) => format!("LIMIT {}", l),
            (None, Some(o)) => format!("LIMIT 18446744073709551615 OFFSET {}", o),
            (None, None) => String::new(),
        }
    }

    fn bool_literal(&self, value: bool) -> &'static str {
        if value { "1" } else { "0" }
    }

    fn blob_type(&self) -> &'static str {
        "LONGBLOB"
    }

    fn integer_type(&self) -> &'static str {
        "INT"
    }

    fn bigint_type(&self) -> &'static str {
        "BIGINT"
    }

    fn now_millis_expr(&self) -> &'static str {
        "(UNIX_TIMESTAMP(NOW(3)) * 1000)"
    }

    fn supports_returning(&self) -> bool {
        false
    }

    fn supports_skip_locked(&self) -> bool {
        true
    }

    fn for_update_skip_locked(&self) -> &'static str {
        "FOR UPDATE SKIP LOCKED"
    }

    fn quote_identifier(&self, identifier: &str) -> String {
        format!("`{}`", identifier)
    }

    fn create_index_if_not_exists(&self, index_name: &str, table: &str, columns: &str) -> String {
        // MySQL 8.0 lacks CREATE INDEX IF NOT EXISTS; callers treat 1061 as success.
        format!("CREATE INDEX {index_name} ON {table}({columns})")
    }

    fn insert_or_replace_into(&self) -> &'static str {
        "REPLACE INTO"
    }

    fn supports_on_conflict_update(&self) -> bool {
        false
    }

    fn on_conflict_do_update_suffix(&self, _pk_column: &str, _columns: &[&str]) -> String {
        String::new()
    }
}
