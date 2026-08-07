use crate::error::PersistenceError;
use crate::executor::SqlExecutor;
use crate::statement::RenderedStatement;
use crate::value::DbParams;

pub trait SchemaManager: Send + Sync {
    fn create_schema(&mut self, executor: &mut dyn SqlExecutor) -> Result<(), PersistenceError>;
    fn update_schema(&mut self, executor: &mut dyn SqlExecutor) -> Result<(), PersistenceError>;
    fn drop_schema(&mut self, executor: &mut dyn SqlExecutor) -> Result<(), PersistenceError>;
    fn validate_schema(&mut self, executor: &mut dyn SqlExecutor) -> Result<(), PersistenceError>;
    fn get_schema_version(
        &mut self,
        executor: &mut dyn SqlExecutor,
    ) -> Result<Option<String>, PersistenceError>;
}

pub struct FlowableSchemaManager {
    scripts: Vec<SchemaScript>,
}

#[derive(Debug, Clone)]
pub struct SchemaScript {
    pub version: String,
    pub component: String,
    pub database_type: String,
    pub sql: String,
}

impl FlowableSchemaManager {
    pub fn new() -> Self {
        Self {
            scripts: Vec::new(),
        }
    }

    pub fn add_script(&mut self, script: SchemaScript) {
        self.scripts.push(script);
    }

    pub fn get_scripts_for_database(&self, database_type: &str) -> Vec<&SchemaScript> {
        self.scripts
            .iter()
            .filter(|s| s.database_type == database_type)
            .collect()
    }

    pub fn get_scripts_for_component(
        &self,
        component: &str,
        database_type: &str,
    ) -> Vec<&SchemaScript> {
        self.scripts
            .iter()
            .filter(|s| s.component == component && s.database_type == database_type)
            .collect()
    }

    /// Returns the highest version among all registered scripts.
    pub fn latest_version(&self) -> Option<String> {
        let mut versions: Vec<_> = self.scripts.iter().map(|s| &s.version).collect();
        versions.sort_by(|a, b| compare_versions(a, b));
        versions.last().cloned().cloned()
    }

    /// Returns all unique versions sorted ascending.
    pub fn all_versions(&self) -> Vec<String> {
        let mut versions: Vec<String> = self
            .scripts
            .iter()
            .map(|s| s.version.clone())
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();
        versions.sort_by(|a, b| compare_versions(a, b));
        versions
    }

    fn execute_scripts(
        &self,
        executor: &mut dyn SqlExecutor,
        scripts: &[&SchemaScript],
    ) -> Result<(), PersistenceError> {
        for (i, script) in scripts.iter().enumerate() {
            // Drivers prepare one statement at a time, so execute every DDL statement in a
            // versioned script rather than silently applying only its first statement.
            for sql in script
                .sql
                .split(';')
                .map(str::trim)
                .filter(|sql| !sql.is_empty())
            {
                let rendered = RenderedStatement::new(sql.to_string(), DbParams::new());
                if let Err(e) = executor.execute(rendered) {
                    return Err(PersistenceError::Schema(format!(
                        "Script {} (component={}, version={}) failed: {} | SQL: {}",
                        i,
                        script.component,
                        script.version,
                        e,
                        &sql[..std::cmp::min(200, sql.len())]
                    )));
                }
            }
        }
        Ok(())
    }

    fn get_current_version(
        &self,
        executor: &mut dyn SqlExecutor,
    ) -> Result<Option<String>, PersistenceError> {
        let db_kind = executor.dialect().database_kind();
        let sql = match db_kind {
            crate::config::DatabaseKind::Postgres => {
                "SELECT table_name FROM information_schema.tables WHERE table_schema = current_schema() AND table_name = 'act_ge_property'"
            }
            crate::config::DatabaseKind::Mysql => {
                "SELECT table_name FROM information_schema.tables WHERE table_schema = DATABASE() AND table_name = 'ACT_GE_PROPERTY'"
            }
            _ => "SELECT name FROM sqlite_master WHERE type='table' AND name='ACT_GE_PROPERTY'",
        };
        let rendered = RenderedStatement::new(sql.to_string(), DbParams::new());
        match executor.fetch_optional(rendered) {
            Ok(Some(_)) => {}
            _ => return Ok(None),
        }

        let sql = match db_kind {
            crate::config::DatabaseKind::Postgres => {
                "SELECT VALUE_ FROM ACT_GE_PROPERTY WHERE NAME_ = 'schema.version'"
            }
            crate::config::DatabaseKind::Mysql => {
                "SELECT VALUE_ FROM ACT_GE_PROPERTY WHERE NAME_ = 'schema.version'"
            }
            _ => "SELECT VALUE_ FROM ACT_GE_PROPERTY WHERE NAME_ = 'schema.version'",
        };
        let rendered = RenderedStatement::new(sql.to_string(), DbParams::new());
        match executor.fetch_optional(rendered) {
            Ok(Some(row)) => Ok(row.get_text("VALUE_")),
            Ok(None) => Ok(None),
            Err(_) => Ok(None),
        }
    }

    /// Inserts or updates the schema.version property record.
    /// Uses dialect-aware placeholders so it works on SQLite (?1) and PostgreSQL ($1).
    fn ensure_version_record(
        &self,
        executor: &mut dyn SqlExecutor,
        version: &str,
    ) -> Result<(), PersistenceError> {
        // Pre-compute placeholders so we don't hold a dialect borrow across executor calls.
        let (ph0, ph1, ph2) = {
            let d = executor.dialect();
            (d.placeholder(0), d.placeholder(1), d.placeholder(2))
        };

        // Try to update first.
        let update_sql = format!(
            "UPDATE ACT_GE_PROPERTY SET VALUE_ = {ph0}, REV_ = REV_ + 1 WHERE NAME_ = {ph1}",
        );
        let mut params = DbParams::new();
        params.push(version);
        params.push("schema.version");
        let rendered = RenderedStatement::new(update_sql, params);
        let result = executor.execute(rendered)?;

        if result.rows_affected == 0 {
            // No row to update — insert a fresh record.
            let insert_sql = format!(
                "INSERT INTO ACT_GE_PROPERTY (NAME_, VALUE_, REV_) VALUES ({ph0}, {ph1}, {ph2})",
            );
            let mut params = DbParams::new();
            params.push("schema.version");
            params.push(version);
            params.push(1i64);
            let rendered = RenderedStatement::new(insert_sql, params);
            executor.execute(rendered)?;
        }
        Ok(())
    }

    /// Checks whether ACT_GE_PROPERTY exists as a proxy for "schema has been created".
    fn schema_exists(&self, executor: &mut dyn SqlExecutor) -> Result<bool, PersistenceError> {
        let db_kind = executor.dialect().database_kind();
        let sql = match db_kind {
            crate::config::DatabaseKind::Postgres => {
                "SELECT table_name FROM information_schema.tables WHERE table_schema = current_schema() AND table_name = 'act_ge_property'"
            }
            crate::config::DatabaseKind::Mysql => {
                "SELECT table_name FROM information_schema.tables WHERE table_schema = DATABASE() AND table_name = 'ACT_GE_PROPERTY'"
            }
            _ => "SELECT name FROM sqlite_master WHERE type='table' AND name='ACT_GE_PROPERTY'",
        };
        let rendered = RenderedStatement::new(sql.to_string(), DbParams::new());
        match executor.fetch_optional(rendered) {
            Ok(Some(_)) => Ok(true),
            _ => Ok(false),
        }
    }
}

impl Default for FlowableSchemaManager {
    fn default() -> Self {
        Self::new()
    }
}

impl SchemaManager for FlowableSchemaManager {
    fn create_schema(&mut self, executor: &mut dyn SqlExecutor) -> Result<(), PersistenceError> {
        // No-op if schema already exists (repeated migration).
        if self.schema_exists(executor)? {
            return Ok(());
        }

        let database_type = executor.dialect().database_kind().to_string();
        let scripts = self.get_scripts_for_database(&database_type);
        self.execute_scripts(executor, &scripts)?;

        // Record the latest version in the property table.
        if let Some(version) = self.latest_version() {
            self.ensure_version_record(executor, &version)?;
        }
        Ok(())
    }

    fn update_schema(&mut self, executor: &mut dyn SqlExecutor) -> Result<(), PersistenceError> {
        let current_version = self.get_current_version(executor)?;
        let database_type = executor.dialect().database_kind().to_string();
        let scripts = self.get_scripts_for_database(&database_type);
        let all_versions = self.all_versions();

        if all_versions.is_empty() {
            return Ok(());
        }

        let latest = all_versions.last().unwrap().clone();

        // Fresh database — run everything.
        if current_version.is_none() {
            self.execute_scripts(executor, &scripts)?;
            self.ensure_version_record(executor, &latest)?;
            return Ok(());
        }

        let current = current_version.unwrap();

        // No-op if already at the latest version.
        if compare_versions(&current, &latest) == std::cmp::Ordering::Equal {
            return Ok(());
        }

        // Out-of-order: current version is newer than the latest known script.
        if compare_versions(&current, &latest) == std::cmp::Ordering::Greater {
            return Err(PersistenceError::Schema(format!(
                "Database schema version '{current}' is newer than the latest known version '{latest}'."
            )));
        }

        // Unknown version — database was created by a different release branch.
        if !all_versions.contains(&current) {
            return Err(PersistenceError::Schema(format!(
                "Database schema version '{current}' is not recognized. \
                 Known versions: {all_versions:?}. \
                 Manual migration may be required.",
            )));
        }

        // Normal forward migration — execute scripts newer than current.
        let scripts_to_run: Vec<_> = scripts
            .into_iter()
            .filter(|s| compare_versions(&s.version, &current) == std::cmp::Ordering::Greater)
            .collect();

        self.execute_scripts(executor, &scripts_to_run)?;
        self.ensure_version_record(executor, &latest)?;
        Ok(())
    }

    fn drop_schema(&mut self, executor: &mut dyn SqlExecutor) -> Result<(), PersistenceError> {
        let database_type = executor.dialect().database_kind().to_string();
        let scripts = self.get_scripts_for_database(&database_type);

        // Drop in reverse order to avoid FK constraint issues.
        for script in scripts.into_iter().rev() {
            let drop_sql = format!("DROP TABLE IF EXISTS {}", script.component);
            let rendered = RenderedStatement::new(drop_sql, DbParams::new());
            executor.execute(rendered)?;
        }
        Ok(())
    }

    fn validate_schema(&mut self, executor: &mut dyn SqlExecutor) -> Result<(), PersistenceError> {
        let database_type = executor.dialect().database_kind().to_string();
        let scripts = self.get_scripts_for_database(&database_type);

        if scripts.is_empty() {
            return Err(PersistenceError::Schema(
                "No schema scripts registered for this database type".to_string(),
            ));
        }

        // Verify every registered table can be queried.
        for script in &scripts {
            let sql = format!("SELECT 1 FROM {} LIMIT 1", script.component);
            let rendered = RenderedStatement::new(sql, DbParams::new());
            if let Err(e) = executor.fetch_optional(rendered) {
                return Err(PersistenceError::Schema(format!(
                    "Table {} is missing or inaccessible: {e}",
                    script.component
                )));
            }
        }
        Ok(())
    }

    fn get_schema_version(
        &mut self,
        executor: &mut dyn SqlExecutor,
    ) -> Result<Option<String>, PersistenceError> {
        self.get_current_version(executor)
    }
}

/// Compare two dot-separated version strings (e.g. "7.0.0" vs "7.0.1").
/// Falls back to string comparison on parse failure.
fn compare_versions(a: &str, b: &str) -> std::cmp::Ordering {
    fn parse(v: &str) -> Option<Vec<u32>> {
        v.split('.').map(|p| p.parse().ok()).collect()
    }
    match (parse(a), parse(b)) {
        (Some(va), Some(vb)) => va.cmp(&vb),
        _ => a.cmp(b),
    }
}
