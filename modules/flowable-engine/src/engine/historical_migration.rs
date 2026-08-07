use crate::engine::deployment_manager::DeploymentManager;

use crate::error::FlowableError;
use crate::history::historic_entities::{HistoricProcessInstance, HistoricVariableInstance};
use crate::persistence::db_session::DbSession;
use crate::persistence::runtime_store::{
    EventSubscription, EventSubscriptionKind, ProcessEventStartSubscription,
    ProcessTimerStartSubscription, RuntimeEventWaitKind, RuntimeEventWaitState, RuntimeStore,
    RuntimeTimerJobState,
};
use crate::repository::deployment::Deployment;
use crate::repository::process_definition::ProcessDefinition;
use crate::runtime::execution::Execution;
use crate::runtime::process_instance::ProcessInstance;
use crate::task::Task;
use chrono::{DateTime, TimeZone, Utc};
use flowable_bpmn_converter::BpmnXMLConverter;
use flowable_bpmn_model::model::{EventDefinitionEnum, FlowElementEnum};
use rusqlite::{
    Connection, Row, params_from_iter,
    types::{Value, ValueRef},
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};

const PORTABLE_BUNDLE_MANIFEST_NAME: &str = "manifest.json";
const PORTABLE_BUNDLE_FORMAT_V1: &str = "portable-historical-migration-bundle/v1";
const SOURCE_MANIFEST_FORMAT_V1: &str = "historical-migration-source-manifest/v1";

mod tables {
    pub const ACT_RE_DEPLOYMENT: &str = "ACT_RE_DEPLOYMENT";
    pub const ACT_GE_BYTEARRAY: &str = "ACT_GE_BYTEARRAY";
    pub const ACT_RE_PROCDEF: &str = "ACT_RE_PROCDEF";
    pub const ACT_RU_EXECUTION: &str = "ACT_RU_EXECUTION";
    pub const ACT_RU_TASK: &str = "ACT_RU_TASK";
    pub const ACT_RU_VARIABLE: &str = "ACT_RU_VARIABLE";
    pub const ACT_RU_TIMER_JOB: &str = "ACT_RU_TIMER_JOB";
    pub const ACT_RU_EVENT_SUBSCR: &str = "ACT_RU_EVENT_SUBSCR";
    pub const ACT_HI_PROCINST: &str = "ACT_HI_PROCINST";
    pub const ACT_HI_VARINST: &str = "ACT_HI_VARINST";
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HistoricalMigrationReport {
    pub source_path: String,
    pub present_tables: Vec<String>,
    pub deployment_count: usize,
    pub deployment_resource_count: usize,
    pub process_definition_count: usize,
    pub process_instance_count: usize,
    pub execution_count: usize,
    pub task_count: usize,
    pub variable_count: usize,
    pub timer_job_count: usize,
    pub event_subscription_count: usize,
    pub historic_process_instance_count: usize,
    pub historic_variable_count: usize,
    pub unsupported_variable_types: Vec<String>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HistoricalMigrationImportResult {
    pub report: HistoricalMigrationReport,
    pub imported_deployments: usize,
    pub imported_process_definitions: usize,
    pub imported_process_instances: usize,
    pub imported_executions: usize,
    pub imported_tasks: usize,
    pub imported_variables: usize,
    pub imported_timer_jobs: usize,
    pub imported_event_subscriptions: usize,
    pub imported_historic_process_instances: usize,
    pub imported_historic_variable_instances: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HistoricalMigrationBundleExportResult {
    pub bundle_path: String,
    pub manifest_path: String,
    pub sqlite_source_path: String,
    pub format: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum HistoricalMigrationRawDialect {
    Mysql,
    Postgres,
    H2,
}

impl HistoricalMigrationRawDialect {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Mysql => "mysql",
            Self::Postgres => "postgres",
            Self::H2 => "h2",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct PortableHistoricalBundleManifest {
    format: String,
    sqlite_source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct HistoricalMigrationSourceManifest {
    format: String,
    source: HistoricalMigrationSourceDescriptor,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "kebab-case")]
enum HistoricalMigrationSourceDescriptor {
    SqliteDb {
        path: String,
    },
    SqliteDump {
        path: String,
    },
    PortableBundle {
        path: String,
    },
    MysqlValuesDump {
        path: String,
    },
    PostgresValuesDump {
        path: String,
    },
    PostgresCopyDump {
        path: String,
    },
    /// Bounded live database URL inspect (sqlx/rusqlite adapters).
    /// `database_kind` is one of: sqlite, postgres, mysql.
    LiveSqlx {
        url: String,
        database_kind: String,
    },
}

#[derive(Debug)]
struct NormalizedSqlDumpEnvironment {
    sqlite_path: PathBuf,
    normalization_warning: String,
}

impl NormalizedSqlDumpEnvironment {
    fn decorate_report(&self, source_path: &Path, report: &mut HistoricalMigrationReport) {
        report.source_path = source_path.display().to_string();
        if !report
            .warnings
            .iter()
            .any(|warning| warning == &self.normalization_warning)
        {
            report.warnings.push(self.normalization_warning.clone());
        }
    }
}

impl Drop for NormalizedSqlDumpEnvironment {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.sqlite_path);
    }
}

#[derive(Debug, Clone, PartialEq)]
struct ParsedSqlInsert {
    table: String,
    columns: Vec<String>,
    rows: Vec<Vec<RawDumpValue>>,
}

#[derive(Debug, Clone, PartialEq)]
struct ParsedSqlCopy {
    table: String,
    columns: Vec<String>,
    rows: Vec<Vec<RawDumpValue>>,
}

#[derive(Debug, Clone, PartialEq)]
enum ParsedSqlOperation {
    Insert(ParsedSqlInsert),
    Copy(ParsedSqlCopy),
}

#[derive(Debug, Clone, PartialEq)]
enum RawDumpValue {
    Null,
    Integer(i64),
    Float(f64),
    Text(String),
    Bytes(Vec<u8>),
}

pub fn inspect_historical_migration_sqlite(
    path: &Path,
) -> Result<HistoricalMigrationReport, FlowableError> {
    let conn = open_source_connection(path)?;
    let present_tables = list_tables(&conn)?;
    let supported_runtime_variable_types = supported_runtime_variable_types();
    let supported_historic_variable_types = supported_historic_variable_types();
    let mut unsupported_types = BTreeSet::new();
    let mut warnings = Vec::new();

    if !present_tables
        .iter()
        .any(|table| table == tables::ACT_RE_DEPLOYMENT)
    {
        warnings.push(
            "missing ACT_RE_DEPLOYMENT; repository migration baseline will be empty".to_string(),
        );
    }
    if !present_tables
        .iter()
        .any(|table| table == tables::ACT_RE_PROCDEF)
    {
        warnings.push(
            "missing ACT_RE_PROCDEF; process-definition migration baseline will be empty"
                .to_string(),
        );
    }
    if !present_tables
        .iter()
        .any(|table| table == tables::ACT_GE_BYTEARRAY)
    {
        warnings.push(
            "missing ACT_GE_BYTEARRAY; BPMN resource migration baseline will be empty".to_string(),
        );
    }

    if has_table(&conn, tables::ACT_RU_VARIABLE)? {
        for variable_type in list_distinct_text_values(&conn, tables::ACT_RU_VARIABLE, "TYPE_")? {
            if !supported_runtime_variable_types.contains(variable_type.as_str()) {
                unsupported_types.insert(variable_type);
            }
        }
    }

    if has_table(&conn, tables::ACT_HI_VARINST)? {
        for variable_type in list_distinct_text_values(&conn, tables::ACT_HI_VARINST, "VAR_TYPE_")?
        {
            if !supported_historic_variable_types.contains(variable_type.as_str()) {
                unsupported_types.insert(variable_type);
            }
        }
    }

    if has_table(&conn, tables::ACT_RU_TIMER_JOB)? {
        warnings.push(
            "ACT_RU_TIMER_JOB import is best-effort in M21 baseline and assumes handler configuration is already resolved for the owned subset"
                .to_string(),
        );
    }

    Ok(HistoricalMigrationReport {
        source_path: path.display().to_string(),
        present_tables,
        deployment_count: count_rows_if_present(&conn, tables::ACT_RE_DEPLOYMENT)?,
        deployment_resource_count: count_rows_if_present(&conn, tables::ACT_GE_BYTEARRAY)?,
        process_definition_count: count_rows_if_present(&conn, tables::ACT_RE_PROCDEF)?,
        process_instance_count: count_process_instances(&conn)?,
        execution_count: count_rows_if_present(&conn, tables::ACT_RU_EXECUTION)?,
        task_count: count_rows_if_present(&conn, tables::ACT_RU_TASK)?,
        variable_count: count_rows_if_present(&conn, tables::ACT_RU_VARIABLE)?,
        timer_job_count: count_rows_if_present(&conn, tables::ACT_RU_TIMER_JOB)?,
        event_subscription_count: count_rows_if_present(&conn, tables::ACT_RU_EVENT_SUBSCR)?,
        historic_process_instance_count: count_rows_if_present(&conn, tables::ACT_HI_PROCINST)?,
        historic_variable_count: count_rows_if_present(&conn, tables::ACT_HI_VARINST)?,
        unsupported_variable_types: unsupported_types.into_iter().collect(),
        warnings,
    })
}

pub fn inspect_historical_migration_bundle(
    path: &Path,
) -> Result<HistoricalMigrationReport, FlowableError> {
    let source_db = resolve_bundle_sqlite_source(path)?;
    let mut report = inspect_historical_migration_sqlite(&source_db)?;
    report.source_path = normalize_bundle_display_path(path);
    report.warnings.push(
        "portable bundle import uses the bounded SQLite-backed extract embedded in the bundle"
            .to_string(),
    );
    Ok(report)
}

pub fn inspect_historical_migration_source_manifest(
    path: &Path,
) -> Result<HistoricalMigrationReport, FlowableError> {
    let manifest = read_source_manifest(path)?;
    let base_dir = path.parent().unwrap_or_else(|| Path::new("."));

    let mut report = match &manifest.source {
        HistoricalMigrationSourceDescriptor::SqliteDb { path } => {
            inspect_historical_migration_sqlite(&resolve_manifest_source_path(base_dir, path))?
        }
        HistoricalMigrationSourceDescriptor::SqliteDump { path } => {
            inspect_historical_migration_sqlite_dump(&resolve_manifest_source_path(base_dir, path))?
        }
        HistoricalMigrationSourceDescriptor::PortableBundle { path } => {
            inspect_historical_migration_bundle(&resolve_manifest_source_path(base_dir, path))?
        }
        HistoricalMigrationSourceDescriptor::MysqlValuesDump { path } => {
            inspect_historical_migration_sql_dump(
                &resolve_manifest_source_path(base_dir, path),
                HistoricalMigrationRawDialect::Mysql,
            )?
        }
        HistoricalMigrationSourceDescriptor::PostgresValuesDump { path }
        | HistoricalMigrationSourceDescriptor::PostgresCopyDump { path } => {
            inspect_historical_migration_sql_dump(
                &resolve_manifest_source_path(base_dir, path),
                HistoricalMigrationRawDialect::Postgres,
            )?
        }
        HistoricalMigrationSourceDescriptor::LiveSqlx { url, database_kind } => {
            inspect_historical_migration_live_url(url, database_kind)?
        }
    };

    decorate_source_manifest_report(path, &manifest.source, &mut report);
    Ok(report)
}

/// Bounded live-database historical inspect.
///
/// Connects to `url` using the adapters available for `kind`
/// (`sqlite` | `postgres` | `mysql`) and returns table presence plus row counts
/// for the same ACT_* set used by SQLite dump/db inspect.
///
/// - `sqlite` always works (rusqlite path or `sqlite:` URL).
/// - `postgres` requires `--features postgres`.
/// - `mysql` requires `--features mysql`.
///
/// This is inspect-only (no full remote import in the MVS baseline).
pub fn inspect_historical_migration_live_url(
    url: &str,
    kind: &str,
) -> Result<HistoricalMigrationReport, FlowableError> {
    let kind_normalized = normalize_live_database_kind(kind)?;
    match kind_normalized.as_str() {
        "sqlite" => inspect_live_sqlite_url(url),
        "postgres" => inspect_live_remote_url(url, flowable_persistence::DatabaseKind::Postgres),
        "mysql" => inspect_live_remote_url(url, flowable_persistence::DatabaseKind::Mysql),
        other => Err(FlowableError::Internal(format!(
            "unsupported live historical migration database kind: {other}"
        ))),
    }
}

pub fn inspect_historical_migration_sql_dump(
    path: &Path,
    dialect: HistoricalMigrationRawDialect,
) -> Result<HistoricalMigrationReport, FlowableError> {
    let normalized = normalize_sql_dump_to_sqlite(path, dialect)?;
    let mut report = inspect_historical_migration_sqlite(&normalized.sqlite_path)?;
    normalized.decorate_report(path, &mut report);
    Ok(report)
}

pub fn export_historical_migration_bundle(
    source_db: &Path,
    bundle_path: &Path,
) -> Result<HistoricalMigrationBundleExportResult, FlowableError> {
    if !source_db.is_file() {
        return Err(FlowableError::Internal(format!(
            "historical migration environment source database does not exist: {}",
            source_db.display()
        )));
    }

    std::fs::create_dir_all(bundle_path).map_err(|error| {
        FlowableError::Internal(format!(
            "failed to create portable historical migration bundle directory {}: {}",
            bundle_path.display(),
            error
        ))
    })?;

    let sqlite_file_name = source_db.file_name().ok_or_else(|| {
        FlowableError::Internal(format!(
            "historical migration environment source database has no file name: {}",
            source_db.display()
        ))
    })?;
    let bundled_sqlite_path = bundle_path.join(sqlite_file_name);
    if source_db != bundled_sqlite_path {
        std::fs::copy(source_db, &bundled_sqlite_path).map_err(|error| {
            FlowableError::Internal(format!(
                "failed to copy historical migration environment source {} into bundle {}: {}",
                source_db.display(),
                bundled_sqlite_path.display(),
                error
            ))
        })?;
    }

    let manifest = PortableHistoricalBundleManifest {
        format: PORTABLE_BUNDLE_FORMAT_V1.to_string(),
        sqlite_source: sqlite_file_name.to_string_lossy().into_owned(),
    };
    let manifest_path = bundle_manifest_path(bundle_path);
    let manifest_bytes = serde_json::to_vec_pretty(&manifest).map_err(|error| {
        FlowableError::Internal(format!(
            "failed to serialize portable historical migration bundle manifest {}: {}",
            manifest_path.display(),
            error
        ))
    })?;
    std::fs::write(&manifest_path, manifest_bytes).map_err(|error| {
        FlowableError::Internal(format!(
            "failed to write portable historical migration bundle manifest {}: {}",
            manifest_path.display(),
            error
        ))
    })?;

    Ok(HistoricalMigrationBundleExportResult {
        bundle_path: bundle_path.display().to_string(),
        manifest_path: manifest_path.display().to_string(),
        sqlite_source_path: bundled_sqlite_path.display().to_string(),
        format: PORTABLE_BUNDLE_FORMAT_V1.to_string(),
    })
}

pub fn import_historical_migration_sqlite(
    path: &Path,
    deployment_manager: &DeploymentManager,
    runtime_store: &RuntimeStore,
    calendars: &crate::engine::business_calendar::BusinessCalendarRegistry,
) -> Result<HistoricalMigrationImportResult, FlowableError> {
    let mut session = runtime_store
        .create_session()
        .map_err(|e| FlowableError::Internal(e.to_string()))?;
    ensure_target_is_empty(deployment_manager, runtime_store, &mut session)?;

    let report = inspect_historical_migration_sqlite(path)?;
    let conn = open_source_connection(path)?;

    let deployments = load_deployments(&conn)?;
    let resources = load_deployment_resources(&conn)?;
    let process_definitions = load_process_definitions(&conn)?;
    let process_definition_map: HashMap<String, ProcessDefinition> = process_definitions
        .iter()
        .map(|definition| (definition.id.clone(), definition.clone()))
        .collect();
    let runtime_variables = load_runtime_variables(&conn)?;
    let process_instances = load_process_instances(&conn, &process_definition_map)?;
    let executions = load_executions(&conn, &runtime_variables, &process_definition_map)?;
    let tasks = load_tasks(&conn)?;
    let timer_jobs = load_timer_jobs(&conn)?;
    let event_wait_states = load_event_wait_states(&conn)?;
    let historic_process_instances = load_historic_process_instances(&conn)?;
    let historic_variable_instances = load_historic_variable_instances(&conn)?;

    for mut deployment in deployments {
        deployment.resources = resources.get(&deployment.id).cloned().unwrap_or_default();
        deployment_manager.register_deployment(deployment, &mut session);
    }

    let converter = BpmnXMLConverter::new();
    let mut timer_start_subscriptions = Vec::new();
    let mut event_start_subscriptions = Vec::new();

    for process_definition in &process_definitions {
        deployment_manager.insert_process_definition(process_definition.clone(), &mut session);
        update_process_definition_version(process_definition, &mut session)?;

        if let Some(model) = load_bpmn_model_for_process_definition(
            &converter,
            deployment_manager,
            process_definition,
            &mut session,
        ) {
            deployment_manager.insert_bpmn_model(&process_definition.id, model.clone());
            timer_start_subscriptions.extend(extract_timer_start_subscriptions(
                process_definition,
                &model,
                runtime_store,
                calendars,
            )?);
            event_start_subscriptions.extend(extract_event_start_subscriptions(
                process_definition,
                &model,
            ));
        }
    }

    deployment_manager.register_timer_start_subscriptions(timer_start_subscriptions, &mut session);
    deployment_manager.register_event_start_subscriptions(event_start_subscriptions, &mut session);

    let imported_process_instances = process_instances.len();
    let imported_executions = executions.len();
    let imported_tasks = tasks.len();
    let imported_timer_jobs = timer_jobs.len();
    let imported_event_subscriptions = event_wait_states.len();
    let imported_historic_process_instances = historic_process_instances.len();
    let imported_historic_variable_instances = historic_variable_instances.len();

    for process_instance in &process_instances {
        runtime_store.insert_process_instance(process_instance, &mut session);
    }

    for execution in &executions {
        runtime_store.insert_execution(execution, &mut session);
    }

    for task in &tasks {
        runtime_store.insert_task(task, &mut session);
    }

    for timer_job in timer_jobs {
        runtime_store.insert_timer_job_state(&timer_job, &mut session);
    }

    for wait_state in event_wait_states {
        runtime_store.insert_event_wait_state(&wait_state, &mut session);
    }

    for instance in historic_process_instances {
        runtime_store.insert_historic_process_instance(&instance, &mut session);
    }

    for variable in historic_variable_instances {
        runtime_store.insert_historic_variable_instance(&variable, &mut session);
    }

    session.flush_and_commit().unwrap();

    Ok(HistoricalMigrationImportResult {
        imported_deployments: report.deployment_count,
        imported_process_definitions: process_definitions.len(),
        imported_process_instances,
        imported_executions,
        imported_tasks,
        imported_variables: report.variable_count,
        imported_timer_jobs,
        imported_event_subscriptions,
        imported_historic_process_instances,
        imported_historic_variable_instances,
        report,
    })
}

pub fn import_historical_migration_bundle(
    path: &Path,
    deployment_manager: &DeploymentManager,
    runtime_store: &RuntimeStore,
    calendars: &crate::engine::business_calendar::BusinessCalendarRegistry,
) -> Result<HistoricalMigrationImportResult, FlowableError> {
    let source_db = resolve_bundle_sqlite_source(path)?;
    let mut result = import_historical_migration_sqlite(
        &source_db,
        deployment_manager,
        runtime_store,
        calendars,
    )?;
    result.report.source_path = normalize_bundle_display_path(path);
    if !result.report.warnings.iter().any(|warning| {
        warning
            == "portable bundle import uses the bounded SQLite-backed extract embedded in the bundle"
    }) {
        result.report.warnings.push(
            "portable bundle import uses the bounded SQLite-backed extract embedded in the bundle"
                .to_string(),
        );
    }
    Ok(result)
}

pub fn import_historical_migration_source_manifest(
    path: &Path,
    deployment_manager: &DeploymentManager,
    runtime_store: &RuntimeStore,
    calendars: &crate::engine::business_calendar::BusinessCalendarRegistry,
) -> Result<HistoricalMigrationImportResult, FlowableError> {
    let manifest = read_source_manifest(path)?;
    let base_dir = path.parent().unwrap_or_else(|| Path::new("."));

    let mut result = match &manifest.source {
        HistoricalMigrationSourceDescriptor::SqliteDb { path } => {
            import_historical_migration_sqlite(
                &resolve_manifest_source_path(base_dir, path),
                deployment_manager,
                runtime_store,
                calendars,
            )?
        }
        HistoricalMigrationSourceDescriptor::SqliteDump { path } => {
            import_historical_migration_sqlite_dump(
                &resolve_manifest_source_path(base_dir, path),
                deployment_manager,
                runtime_store,
                calendars,
            )?
        }
        HistoricalMigrationSourceDescriptor::PortableBundle { path } => {
            import_historical_migration_bundle(
                &resolve_manifest_source_path(base_dir, path),
                deployment_manager,
                runtime_store,
                calendars,
            )?
        }
        HistoricalMigrationSourceDescriptor::MysqlValuesDump { path } => {
            import_historical_migration_sql_dump(
                &resolve_manifest_source_path(base_dir, path),
                HistoricalMigrationRawDialect::Mysql,
                deployment_manager,
                runtime_store,
                calendars,
            )?
        }
        HistoricalMigrationSourceDescriptor::PostgresValuesDump { path }
        | HistoricalMigrationSourceDescriptor::PostgresCopyDump { path } => {
            import_historical_migration_sql_dump(
                &resolve_manifest_source_path(base_dir, path),
                HistoricalMigrationRawDialect::Postgres,
                deployment_manager,
                runtime_store,
                calendars,
            )?
        }
        HistoricalMigrationSourceDescriptor::LiveSqlx { url, database_kind } => {
            import_historical_migration_live_url(
                url,
                database_kind,
                deployment_manager,
                runtime_store,
                calendars,
            )?
        }
    };

    decorate_source_manifest_report(path, &manifest.source, &mut result.report);
    Ok(result)
}

pub fn import_historical_migration_live_url(
    url: &str,
    database_kind: &str,
    deployment_manager: &DeploymentManager,
    runtime_store: &RuntimeStore,
    calendars: &crate::engine::business_calendar::BusinessCalendarRegistry,
) -> Result<HistoricalMigrationImportResult, FlowableError> {
    let kind = match database_kind.trim().to_ascii_lowercase().as_str() {
        "postgres" | "postgresql" => flowable_persistence::DatabaseKind::Postgres,
        "mysql" => flowable_persistence::DatabaseKind::Mysql,
        "sqlite" => flowable_persistence::DatabaseKind::Sqlite,
        other => {
            return Err(FlowableError::Internal(format!(
                "unsupported live historical database kind '{other}'"
            )));
        }
    };
    if kind == flowable_persistence::DatabaseKind::Sqlite {
        let path = sqlite_path_from_live_url(url)?;
        return import_historical_migration_sqlite(
            &path,
            deployment_manager,
            runtime_store,
            calendars,
        );
    }

    let bridge_path =
        std::env::temp_dir().join(format!("flowable-live-import-{}.db", uuid::Uuid::new_v4()));
    let extraction = extract_live_source_to_sqlite(url, kind, &bridge_path);
    if let Err(error) = extraction {
        let _ = std::fs::remove_file(&bridge_path);
        return Err(error);
    }
    let result = import_historical_migration_sqlite(
        &bridge_path,
        deployment_manager,
        runtime_store,
        calendars,
    );
    let _ = std::fs::remove_file(&bridge_path);
    result.map(|mut imported| {
        imported.report.source_path = url.to_string();
        imported
            .report
            .warnings
            .retain(|warning| !warning.contains("inspect-only"));
        imported.report.warnings.push(format!(
            "live {kind} historical source imported through the typed SQLx extraction pipeline"
        ));
        imported
    })
}

fn extract_live_source_to_sqlite(
    url: &str,
    kind: flowable_persistence::DatabaseKind,
    bridge_path: &Path,
) -> Result<(), FlowableError> {
    use flowable_persistence::{DbValue, LiveSqlProbe};
    let mut probe = LiveSqlProbe::connect(kind, url).map_err(|error| {
        FlowableError::Internal(format!(
            "failed to connect live historical source ({kind}): {error}"
        ))
    })?;
    let mut target = Connection::open(bridge_path).map_err(sql_to_error)?;
    let transaction = target.transaction().map_err(sql_to_error)?;

    for (table, columns) in live_import_table_specs() {
        if !probe.table_exists(table).map_err(|error| {
            FlowableError::Internal(format!("failed to inspect live table {table}: {error}"))
        })? {
            continue;
        }
        transaction
            .execute_batch(raw_supported_table_schema(table).ok_or_else(|| {
                FlowableError::Internal(format!("missing SQLite bridge schema for {table}"))
            })?)
            .map_err(sql_to_error)?;
        let projection = columns
            .iter()
            .map(|column| live_column_projection(kind, column))
            .collect::<Vec<_>>()
            .join(", ");
        let rows = probe
            .fetch_rows(format!("SELECT {projection} FROM {table} ORDER BY ID_"))
            .map_err(|error| {
                FlowableError::Internal(format!("failed to extract live table {table}: {error}"))
            })?;
        let placeholders = (0..columns.len())
            .map(|_| "?")
            .collect::<Vec<_>>()
            .join(", ");
        let insert = format!(
            "INSERT INTO {table} ({}) VALUES ({placeholders})",
            columns.join(", ")
        );
        let mut statement = transaction.prepare(&insert).map_err(sql_to_error)?;
        for row in rows {
            let values = columns
                .iter()
                .map(
                    |column| match row.get(column).cloned().unwrap_or(DbValue::Null) {
                        DbValue::Null
                        | DbValue::NullInteger
                        | DbValue::NullBoolean
                        | DbValue::NullBlob => rusqlite::types::Value::Null,
                        DbValue::Text(value) => rusqlite::types::Value::Text(value),
                        DbValue::Integer(value) => rusqlite::types::Value::Integer(value),
                        DbValue::Real(value) => rusqlite::types::Value::Real(value),
                        DbValue::Boolean(value) => {
                            rusqlite::types::Value::Integer(i64::from(value))
                        }
                        DbValue::Blob(value) => rusqlite::types::Value::Blob(value),
                    },
                )
                .collect::<Vec<_>>();
            statement
                .execute(rusqlite::params_from_iter(values))
                .map_err(sql_to_error)?;
        }
    }
    transaction.commit().map_err(sql_to_error)?;
    Ok(())
}

fn live_column_projection(kind: flowable_persistence::DatabaseKind, column: &str) -> String {
    const TIMESTAMPS: &[&str] = &[
        "DEPLOY_TIME_",
        "START_TIME_",
        "CREATE_TIME_",
        "DUEDATE_",
        "LOCK_EXP_TIME_",
        "END_TIME_",
        "LAST_UPDATED_TIME_",
    ];
    if TIMESTAMPS.contains(&column) {
        return match kind {
            flowable_persistence::DatabaseKind::Postgres => {
                format!("CAST(EXTRACT(EPOCH FROM {column}) * 1000 AS BIGINT) AS {column}")
            }
            flowable_persistence::DatabaseKind::Mysql => {
                format!("CAST(UNIX_TIMESTAMP({column}) * 1000 AS SIGNED) AS {column}")
            }
            _ => column.to_string(),
        };
    }
    column.to_string()
}

fn live_import_table_specs() -> Vec<(&'static str, Vec<&'static str>)> {
    vec![
        (
            tables::ACT_RE_DEPLOYMENT,
            vec![
                "ID_",
                "NAME_",
                "CATEGORY_",
                "KEY_",
                "TENANT_ID_",
                "PARENT_DEPLOYMENT_ID_",
                "DERIVED_FROM_",
                "DERIVED_FROM_ROOT_",
                "ENGINE_VERSION_",
                "DEPLOY_TIME_",
            ],
        ),
        (
            tables::ACT_GE_BYTEARRAY,
            vec!["ID_", "NAME_", "DEPLOYMENT_ID_", "BYTES_"],
        ),
        (
            tables::ACT_RE_PROCDEF,
            vec![
                "ID_",
                "CATEGORY_",
                "NAME_",
                "KEY_",
                "DESCRIPTION_",
                "VERSION_",
                "RESOURCE_NAME_",
                "DEPLOYMENT_ID_",
                "DGRM_RESOURCE_NAME_",
                "HAS_START_FORM_KEY_",
                "HAS_GRAPHICAL_NOTATION_",
                "SUSPENSION_STATE_",
                "TENANT_ID_",
                "ENGINE_VERSION_",
                "APP_VERSION_",
            ],
        ),
        (
            tables::ACT_RU_EXECUTION,
            vec![
                "ID_",
                "PARENT_ID_",
                "SUPER_EXEC_",
                "ROOT_PROC_INST_ID_",
                "PROC_INST_ID_",
                "PROC_DEF_ID_",
                "ACT_ID_",
                "IS_ACTIVE_",
                "IS_CONCURRENT_",
                "IS_SCOPE_",
                "IS_MI_ROOT_",
                "SUSPENSION_STATE_",
                "TENANT_ID_",
                "NAME_",
                "BUSINESS_KEY_",
                "START_USER_ID_",
                "START_TIME_",
            ],
        ),
        (
            tables::ACT_RU_TASK,
            vec![
                "ID_",
                "PROC_INST_ID_",
                "EXECUTION_ID_",
                "TASK_DEF_KEY_",
                "NAME_",
                "PARENT_TASK_ID_",
                "CREATE_TIME_",
            ],
        ),
        (
            tables::ACT_RU_VARIABLE,
            vec![
                "ID_",
                "EXECUTION_ID_",
                "PROC_INST_ID_",
                "NAME_",
                "TYPE_",
                "TEXT_",
                "TEXT2_",
                "LONG_",
                "DOUBLE_",
            ],
        ),
        (
            tables::ACT_RU_TIMER_JOB,
            vec![
                "ID_",
                "PROC_INST_ID_",
                "EXECUTION_ID_",
                "HANDLER_CFG_",
                "DUEDATE_",
                "LOCK_OWNER_",
                "LOCK_EXP_TIME_",
                "RETRIES_",
                "EXCEPTION_MSG_",
                "JOB_HANDLER_TYPE_",
            ],
        ),
        (
            tables::ACT_RU_EVENT_SUBSCR,
            vec![
                "ID_",
                "EXECUTION_ID_",
                "PROC_INST_ID_",
                "EVENT_TYPE_",
                "EVENT_NAME_",
                "ACTIVITY_ID_",
            ],
        ),
        (
            tables::ACT_HI_PROCINST,
            vec![
                "ID_",
                "PROC_DEF_ID_",
                "BUSINESS_KEY_",
                "START_TIME_",
                "END_TIME_",
                "DURATION_",
                "START_USER_ID_",
                "DELETE_REASON_",
            ],
        ),
        (
            tables::ACT_HI_VARINST,
            vec![
                "ID_",
                "PROC_INST_ID_",
                "EXECUTION_ID_",
                "TASK_ID_",
                "NAME_",
                "VAR_TYPE_",
                "TEXT_",
                "TEXT2_",
                "LONG_",
                "DOUBLE_",
                "CREATE_TIME_",
                "LAST_UPDATED_TIME_",
            ],
        ),
    ]
}

pub fn import_historical_migration_sql_dump(
    path: &Path,
    dialect: HistoricalMigrationRawDialect,
    deployment_manager: &DeploymentManager,
    runtime_store: &RuntimeStore,
    calendars: &crate::engine::business_calendar::BusinessCalendarRegistry,
) -> Result<HistoricalMigrationImportResult, FlowableError> {
    let normalized = normalize_sql_dump_to_sqlite(path, dialect)?;
    let mut result = import_historical_migration_sqlite(
        &normalized.sqlite_path,
        deployment_manager,
        runtime_store,
        calendars,
    )?;
    normalized.decorate_report(path, &mut result.report);
    Ok(result)
}

pub fn inspect_historical_migration_sqlite_dump(
    path: &Path,
) -> Result<HistoricalMigrationReport, FlowableError> {
    let normalized = normalize_sqlite_dump_to_sqlite(path)?;
    let mut report = inspect_historical_migration_sqlite(&normalized.sqlite_path)?;
    normalized.decorate_report(path, &mut report);
    Ok(report)
}

pub fn import_historical_migration_sqlite_dump(
    path: &Path,
    deployment_manager: &DeploymentManager,
    runtime_store: &RuntimeStore,
    calendars: &crate::engine::business_calendar::BusinessCalendarRegistry,
) -> Result<HistoricalMigrationImportResult, FlowableError> {
    let normalized = normalize_sqlite_dump_to_sqlite(path)?;
    let mut result = import_historical_migration_sqlite(
        &normalized.sqlite_path,
        deployment_manager,
        runtime_store,
        calendars,
    )?;
    normalized.decorate_report(path, &mut result.report);
    Ok(result)
}

fn ensure_target_is_empty(
    deployment_manager: &DeploymentManager,
    runtime_store: &RuntimeStore,
    session: &mut DbSession,
) -> Result<(), FlowableError> {
    let has_existing_data = !deployment_manager.get_deployments(session).is_empty()
        || !deployment_manager
            .get_process_definitions(session)
            .is_empty()
        || !runtime_store.snapshot_process_instances(session).is_empty()
        || !runtime_store.snapshot_executions(session).is_empty()
        || !runtime_store.snapshot_tasks(session).is_empty();

    if has_existing_data {
        return Err(FlowableError::Internal(
            "historical migration environment import requires an empty target engine baseline"
                .to_string(),
        ));
    }

    Ok(())
}

fn open_source_connection(path: &Path) -> Result<Connection, FlowableError> {
    Connection::open(path).map_err(|error| {
        FlowableError::Internal(format!(
            "failed to open historical migration environment source {}: {}",
            path.display(),
            error
        ))
    })
}

fn resolve_bundle_sqlite_source(bundle_path: &Path) -> Result<PathBuf, FlowableError> {
    let manifest_path = bundle_manifest_path(bundle_path);
    let manifest = read_bundle_manifest(&manifest_path)?;
    if manifest.format != PORTABLE_BUNDLE_FORMAT_V1 {
        return Err(FlowableError::Internal(format!(
            "unsupported portable historical migration bundle format at {}: {}",
            manifest_path.display(),
            manifest.format
        )));
    }

    let base_dir = manifest_path.parent().ok_or_else(|| {
        FlowableError::Internal(format!(
            "portable historical migration bundle manifest has no parent directory: {}",
            manifest_path.display()
        ))
    })?;
    let source_path = base_dir.join(manifest.sqlite_source);
    if !source_path.exists() {
        return Err(FlowableError::Internal(format!(
            "portable historical migration bundle source does not exist: {}",
            source_path.display()
        )));
    }

    Ok(source_path)
}

fn bundle_manifest_path(bundle_path: &Path) -> PathBuf {
    if bundle_path
        .file_name()
        .is_some_and(|name| name == PORTABLE_BUNDLE_MANIFEST_NAME)
    {
        bundle_path.to_path_buf()
    } else {
        bundle_path.join(PORTABLE_BUNDLE_MANIFEST_NAME)
    }
}

fn read_bundle_manifest(
    manifest_path: &Path,
) -> Result<PortableHistoricalBundleManifest, FlowableError> {
    let bytes = std::fs::read(manifest_path).map_err(|error| {
        FlowableError::Internal(format!(
            "failed to read portable historical migration bundle manifest {}: {}",
            manifest_path.display(),
            error
        ))
    })?;
    serde_json::from_slice(&bytes).map_err(|error| {
        FlowableError::Internal(format!(
            "failed to parse portable historical migration bundle manifest {}: {}",
            manifest_path.display(),
            error
        ))
    })
}

fn normalize_bundle_display_path(path: &Path) -> String {
    if path
        .file_name()
        .is_some_and(|name| name == PORTABLE_BUNDLE_MANIFEST_NAME)
    {
        path.parent().unwrap_or(path).display().to_string()
    } else {
        path.display().to_string()
    }
}

fn normalize_sqlite_dump_to_sqlite(
    path: &Path,
) -> Result<NormalizedSqlDumpEnvironment, FlowableError> {
    if !path.is_file() {
        return Err(FlowableError::Internal(format!(
            "historical migration sqlite dump source does not exist: {}",
            path.display()
        )));
    }

    let dump_text = std::fs::read_to_string(path).map_err(|error| {
        FlowableError::Internal(format!(
            "failed to read historical migration sqlite dump source {}: {}",
            path.display(),
            error
        ))
    })?;

    let sqlite_path = std::env::temp_dir().join(format!(
        "flowable-historical-migration-sqlite-dump-{}.sqlite",
        uuid::Uuid::new_v4()
    ));
    let conn = Connection::open(&sqlite_path).map_err(|error| {
        FlowableError::Internal(format!(
            "failed to create temporary sqlite bridge for sqlite dump {}: {}",
            sqlite_path.display(),
            error
        ))
    })?;
    conn.execute_batch(&dump_text).map_err(|error| {
        FlowableError::Internal(format!(
            "failed to execute sqlite dump source {} into temporary bridge {}: {}",
            path.display(),
            sqlite_path.display(),
            error
        ))
    })?;

    let supported_rows = [
        tables::ACT_RE_DEPLOYMENT,
        tables::ACT_GE_BYTEARRAY,
        tables::ACT_RE_PROCDEF,
        tables::ACT_RU_EXECUTION,
        tables::ACT_RU_TASK,
        tables::ACT_RU_VARIABLE,
        tables::ACT_RU_TIMER_JOB,
        tables::ACT_RU_EVENT_SUBSCR,
        tables::ACT_HI_PROCINST,
        tables::ACT_HI_VARINST,
    ]
    .into_iter()
    .map(|table| count_rows_if_present(&conn, table))
    .collect::<Result<Vec<_>, _>>()?
    .into_iter()
    .sum::<usize>();

    if supported_rows == 0 {
        return Err(FlowableError::Internal(format!(
            "sqlite dump source {} did not contain any supported ACT_* rows for the M37 historical import envelope",
            path.display()
        )));
    }

    Ok(NormalizedSqlDumpEnvironment {
        sqlite_path,
        normalization_warning:
            "sqlite dump import uses direct sqlite execute_batch normalization into a temporary SQLite bridge"
                .to_string(),
    })
}

fn normalize_sql_dump_to_sqlite(
    path: &Path,
    dialect: HistoricalMigrationRawDialect,
) -> Result<NormalizedSqlDumpEnvironment, FlowableError> {
    if !path.is_file() {
        return Err(FlowableError::Internal(format!(
            "historical migration sql dump source does not exist: {}",
            path.display()
        )));
    }

    let dump_text = std::fs::read_to_string(path).map_err(|error| {
        FlowableError::Internal(format!(
            "failed to read historical migration sql dump source {}: {}",
            path.display(),
            error
        ))
    })?;

    let sqlite_path = std::env::temp_dir().join(format!(
        "flowable-historical-migration-raw-{}-{}.sqlite",
        dialect.as_str(),
        uuid::Uuid::new_v4()
    ));
    let conn = Connection::open(&sqlite_path).map_err(|error| {
        FlowableError::Internal(format!(
            "failed to create temporary normalized sqlite bridge {}: {}",
            sqlite_path.display(),
            error
        ))
    })?;

    let mut inserted_supported_rows = 0usize;
    let mut created_tables = HashSet::new();

    for operation in parse_sql_dump_operations(&dump_text, dialect)? {
        let (table, columns, rows) = match operation {
            ParsedSqlOperation::Insert(insert) => (insert.table, insert.columns, insert.rows),
            ParsedSqlOperation::Copy(copy) => (copy.table, copy.columns, copy.rows),
        };

        let Some(schema_sql) = raw_supported_table_schema(&table) else {
            continue;
        };

        if created_tables.insert(table.clone()) {
            conn.execute_batch(schema_sql).map_err(|error| {
                FlowableError::Internal(format!(
                    "failed to create normalized table {} for raw {} dump source {}: {}",
                    table,
                    dialect.as_str(),
                    path.display(),
                    error
                ))
            })?;
        }

        inserted_supported_rows +=
            insert_rows_into_normalized_sqlite(&conn, &table, &columns, &rows)?;
    }

    if inserted_supported_rows == 0 {
        return Err(FlowableError::Internal(format!(
            "raw {} dump source {} did not contain any supported ACT_* INSERT rows for the bounded M32 import subset",
            dialect.as_str(),
            path.display()
        )));
    }

    Ok(NormalizedSqlDumpEnvironment {
        sqlite_path,
        normalization_warning: format!(
            "raw {} dump import uses bounded normalization into a temporary SQLite bridge",
            dialect.as_str()
        ),
    })
}

fn parse_sql_dump_operations(
    input: &str,
    dialect: HistoricalMigrationRawDialect,
) -> Result<Vec<ParsedSqlOperation>, FlowableError> {
    match dialect {
        HistoricalMigrationRawDialect::Mysql | HistoricalMigrationRawDialect::H2 => {
            split_sql_dump_statements(input)?
                .into_iter()
                .filter_map(|statement| parse_sql_dump_insert_statement(&statement).transpose())
                .map(|result| result.map(ParsedSqlOperation::Insert))
                .collect()
        }
        HistoricalMigrationRawDialect::Postgres => parse_postgres_dump_operations(input),
    }
}

fn parse_postgres_dump_operations(input: &str) -> Result<Vec<ParsedSqlOperation>, FlowableError> {
    let mut operations = Vec::new();
    let mut buffered_sql = String::new();
    let mut lines = input.lines();

    while let Some(line) = lines.next() {
        let trimmed = line.trim();
        if trimmed.starts_with("COPY ") && trimmed.contains("FROM STDIN") {
            if !buffered_sql.trim().is_empty() {
                append_insert_operations_from_sql(&buffered_sql, &mut operations)?;
                buffered_sql.clear();
            }

            let mut copy_block = String::new();
            copy_block.push_str(line);
            copy_block.push('\n');
            let mut terminated = false;
            for data_line in lines.by_ref() {
                copy_block.push_str(data_line);
                copy_block.push('\n');
                if data_line.trim() == "\\." {
                    terminated = true;
                    break;
                }
            }
            if !terminated {
                return Err(FlowableError::Internal(
                    "COPY-based raw historical import has unterminated STDIN block".to_string(),
                ));
            }

            if let Some(copy) = parse_sql_dump_copy_block(&copy_block)? {
                operations.push(ParsedSqlOperation::Copy(copy));
            }
            continue;
        }

        buffered_sql.push_str(line);
        buffered_sql.push('\n');
    }

    if !buffered_sql.trim().is_empty() {
        append_insert_operations_from_sql(&buffered_sql, &mut operations)?;
    }

    Ok(operations)
}

fn append_insert_operations_from_sql(
    sql: &str,
    operations: &mut Vec<ParsedSqlOperation>,
) -> Result<(), FlowableError> {
    for statement in split_sql_dump_statements(sql)? {
        if let Some(insert) = parse_sql_dump_insert_statement(&statement)? {
            operations.push(ParsedSqlOperation::Insert(insert));
        }
    }
    Ok(())
}

fn split_sql_dump_statements(input: &str) -> Result<Vec<String>, FlowableError> {
    let bytes = input.as_bytes();
    let mut statements = Vec::new();
    let mut current = String::new();
    let mut i = 0usize;
    let mut quote: Option<u8> = None;

    while i < bytes.len() {
        let byte = bytes[i];

        if let Some(active_quote) = quote {
            current.push(byte as char);
            if byte == active_quote {
                if i + 1 < bytes.len() && bytes[i + 1] == active_quote {
                    current.push(bytes[i + 1] as char);
                    i += 1;
                } else {
                    quote = None;
                }
            }
            i += 1;
            continue;
        }

        if byte == b'-' && i + 1 < bytes.len() && bytes[i + 1] == b'-' {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }

        if byte == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'*' {
            i += 2;
            while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                i += 1;
            }
            if i + 1 >= bytes.len() {
                return Err(FlowableError::Internal(
                    "unterminated block comment in raw historical migration sql dump".to_string(),
                ));
            }
            i += 2;
            continue;
        }

        if matches!(byte, b'\'' | b'"' | b'`') {
            quote = Some(byte);
            current.push(byte as char);
            i += 1;
            continue;
        }

        if byte == b';' {
            let trimmed = current.trim();
            if !trimmed.is_empty() {
                statements.push(trimmed.to_string());
            }
            current.clear();
            i += 1;
            continue;
        }

        current.push(byte as char);
        i += 1;
    }

    let trimmed = current.trim();
    if !trimmed.is_empty() {
        statements.push(trimmed.to_string());
    }

    Ok(statements)
}

fn parse_sql_dump_insert_statement(
    statement: &str,
) -> Result<Option<ParsedSqlInsert>, FlowableError> {
    let trimmed = statement.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }

    let uppercase = trimmed.to_ascii_uppercase();
    if !uppercase.starts_with("INSERT INTO ") {
        return Ok(None);
    }

    let values_index = find_keyword_outside_quotes(trimmed, "VALUES").ok_or_else(|| {
        FlowableError::Internal(format!(
            "raw dump INSERT statement is missing VALUES clause: {trimmed}"
        ))
    })?;
    let head = trimmed["INSERT INTO ".len()..values_index].trim();
    let values_segment = trimmed[values_index + "VALUES".len()..].trim();
    let column_start = find_char_outside_quotes(head, '(').ok_or_else(|| {
        FlowableError::Internal(format!(
            "raw dump INSERT statement is missing column list: {trimmed}"
        ))
    })?;
    let column_end = find_matching_paren(head, column_start).ok_or_else(|| {
        FlowableError::Internal(format!(
            "raw dump INSERT statement has unbalanced column list: {trimmed}"
        ))
    })?;
    let table = normalize_sql_identifier(&head[..column_start]);
    let columns = split_top_level_csv(&head[column_start + 1..column_end])?
        .into_iter()
        .map(|column| normalize_sql_identifier(&column))
        .collect::<Vec<_>>();
    let rows = split_value_tuples(values_segment)?
        .into_iter()
        .map(|tuple| {
            split_top_level_csv(&tuple)?
                .into_iter()
                .map(|value| parse_raw_dump_value(&value))
                .collect::<Result<Vec<_>, _>>()
        })
        .collect::<Result<Vec<_>, _>>()?;

    Ok(Some(ParsedSqlInsert {
        table,
        columns,
        rows,
    }))
}

fn parse_sql_dump_copy_block(statement: &str) -> Result<Option<ParsedSqlCopy>, FlowableError> {
    let mut lines = statement.lines();
    let Some(header) = lines.next() else {
        return Ok(None);
    };
    let header = header.trim();
    if !header.to_ascii_uppercase().starts_with("COPY ") {
        return Ok(None);
    }

    let from_index = find_keyword_outside_quotes(header, "FROM STDIN").ok_or_else(|| {
        FlowableError::Internal(format!(
            "raw dump COPY statement is missing FROM STDIN clause: {header}"
        ))
    })?;
    let head = header["COPY ".len()..from_index].trim();
    let column_start = find_char_outside_quotes(head, '(').ok_or_else(|| {
        FlowableError::Internal(format!(
            "raw dump COPY statement is missing column list: {header}"
        ))
    })?;
    let column_end = find_matching_paren(head, column_start).ok_or_else(|| {
        FlowableError::Internal(format!(
            "raw dump COPY statement has unbalanced column list: {header}"
        ))
    })?;
    let table = normalize_sql_identifier(&head[..column_start]);
    let columns = split_top_level_csv(&head[column_start + 1..column_end])?
        .into_iter()
        .map(|column| normalize_sql_identifier(&column))
        .collect::<Vec<_>>();

    let mut rows = Vec::new();
    for line in lines {
        let trimmed = line.trim_end();
        if trimmed.trim() == "\\." {
            break;
        }
        if trimmed.is_empty() {
            continue;
        }
        rows.push(
            trimmed
                .split('\t')
                .map(parse_copy_dump_value)
                .collect::<Result<Vec<_>, _>>()?,
        );
    }

    Ok(Some(ParsedSqlCopy {
        table,
        columns,
        rows,
    }))
}

fn find_keyword_outside_quotes(input: &str, keyword: &str) -> Option<usize> {
    let bytes = input.as_bytes();
    let keyword_upper = keyword.as_bytes();
    let mut i = 0usize;
    let mut quote: Option<u8> = None;

    while i < bytes.len() {
        let byte = bytes[i];
        if let Some(active_quote) = quote {
            if byte == active_quote {
                if i + 1 < bytes.len() && bytes[i + 1] == active_quote {
                    i += 2;
                    continue;
                }
                quote = None;
            }
            i += 1;
            continue;
        }

        if matches!(byte, b'\'' | b'"' | b'`') {
            quote = Some(byte);
            i += 1;
            continue;
        }

        if i + keyword_upper.len() <= bytes.len()
            && input[i..i + keyword_upper.len()].eq_ignore_ascii_case(keyword)
        {
            return Some(i);
        }

        i += 1;
    }

    None
}

fn find_char_outside_quotes(input: &str, target: char) -> Option<usize> {
    let bytes = input.as_bytes();
    let mut i = 0usize;
    let mut quote: Option<u8> = None;

    while i < bytes.len() {
        let byte = bytes[i];
        if let Some(active_quote) = quote {
            if byte == active_quote {
                if i + 1 < bytes.len() && bytes[i + 1] == active_quote {
                    i += 2;
                    continue;
                }
                quote = None;
            }
            i += 1;
            continue;
        }

        if matches!(byte, b'\'' | b'"' | b'`') {
            quote = Some(byte);
            i += 1;
            continue;
        }

        if byte == target as u8 {
            return Some(i);
        }

        i += 1;
    }

    None
}

fn find_matching_paren(input: &str, open_index: usize) -> Option<usize> {
    let bytes = input.as_bytes();
    let mut i = open_index;
    let mut depth = 0i32;
    let mut quote: Option<u8> = None;

    while i < bytes.len() {
        let byte = bytes[i];
        if let Some(active_quote) = quote {
            if byte == active_quote {
                if i + 1 < bytes.len() && bytes[i + 1] == active_quote {
                    i += 2;
                    continue;
                }
                quote = None;
            }
            i += 1;
            continue;
        }

        if matches!(byte, b'\'' | b'"' | b'`') {
            quote = Some(byte);
            i += 1;
            continue;
        }

        if byte == b'(' {
            depth += 1;
        } else if byte == b')' {
            depth -= 1;
            if depth == 0 {
                return Some(i);
            }
        }

        i += 1;
    }

    None
}

fn split_value_tuples(values_segment: &str) -> Result<Vec<String>, FlowableError> {
    let bytes = values_segment.as_bytes();
    let mut tuples = Vec::new();
    let mut current = String::new();
    let mut i = 0usize;
    let mut quote: Option<u8> = None;
    let mut depth = 0i32;

    while i < bytes.len() {
        let byte = bytes[i];

        if let Some(active_quote) = quote {
            current.push(byte as char);
            if byte == active_quote {
                if i + 1 < bytes.len() && bytes[i + 1] == active_quote {
                    current.push(bytes[i + 1] as char);
                    i += 1;
                } else {
                    quote = None;
                }
            }
            i += 1;
            continue;
        }

        if matches!(byte, b'\'' | b'"' | b'`') {
            quote = Some(byte);
            current.push(byte as char);
            i += 1;
            continue;
        }

        match byte {
            b'(' => {
                if depth > 0 {
                    current.push('(');
                }
                depth += 1;
            }
            b')' => {
                depth -= 1;
                if depth < 0 {
                    return Err(FlowableError::Internal(
                        "raw dump VALUES tuple list has unbalanced parentheses".to_string(),
                    ));
                }
                if depth == 0 {
                    tuples.push(current.trim().to_string());
                    current.clear();
                } else {
                    current.push(')');
                }
            }
            b',' if depth == 0 => {}
            _ => {
                if depth == 0 {
                    if !(byte as char).is_whitespace() {
                        return Err(FlowableError::Internal(format!(
                            "raw dump VALUES tuple list contains unsupported token outside tuples: {}",
                            values_segment
                        )));
                    }
                } else {
                    current.push(byte as char);
                }
            }
        }

        i += 1;
    }

    if depth != 0 {
        return Err(FlowableError::Internal(
            "raw dump VALUES tuple list has unterminated tuple".to_string(),
        ));
    }

    Ok(tuples)
}

fn split_top_level_csv(input: &str) -> Result<Vec<String>, FlowableError> {
    let bytes = input.as_bytes();
    let mut items = Vec::new();
    let mut current = String::new();
    let mut i = 0usize;
    let mut quote: Option<u8> = None;
    let mut paren_depth = 0i32;

    while i < bytes.len() {
        let byte = bytes[i];

        if let Some(active_quote) = quote {
            current.push(byte as char);
            if byte == active_quote {
                if i + 1 < bytes.len() && bytes[i + 1] == active_quote {
                    current.push(bytes[i + 1] as char);
                    i += 1;
                } else {
                    quote = None;
                }
            }
            i += 1;
            continue;
        }

        if matches!(byte, b'\'' | b'"' | b'`') {
            quote = Some(byte);
            current.push(byte as char);
            i += 1;
            continue;
        }

        match byte {
            b'(' => {
                paren_depth += 1;
                current.push('(');
            }
            b')' => {
                paren_depth -= 1;
                if paren_depth < 0 {
                    return Err(FlowableError::Internal(format!(
                        "raw dump CSV segment has unbalanced parentheses: {input}"
                    )));
                }
                current.push(')');
            }
            b',' if paren_depth == 0 => {
                items.push(current.trim().to_string());
                current.clear();
            }
            _ => current.push(byte as char),
        }

        i += 1;
    }

    if paren_depth != 0 {
        return Err(FlowableError::Internal(format!(
            "raw dump CSV segment has unterminated nested expression: {input}"
        )));
    }

    if !current.trim().is_empty() {
        items.push(current.trim().to_string());
    }

    Ok(items)
}

fn parse_raw_dump_value(value: &str) -> Result<RawDumpValue, FlowableError> {
    let stripped = strip_postgres_cast(value.trim());
    if stripped.eq_ignore_ascii_case("NULL") {
        return Ok(RawDumpValue::Null);
    }
    if stripped.eq_ignore_ascii_case("TRUE") {
        return Ok(RawDumpValue::Integer(1));
    }
    if stripped.eq_ignore_ascii_case("FALSE") {
        return Ok(RawDumpValue::Integer(0));
    }
    if let Some(hex_body) = stripped
        .strip_prefix("x'")
        .or_else(|| stripped.strip_prefix("X'"))
        .and_then(|hex_literal| hex_literal.strip_suffix('\''))
    {
        return Ok(RawDumpValue::Bytes(decode_hex_bytes(hex_body)?));
    }
    if stripped.starts_with('\'') || stripped.starts_with("E'") || stripped.starts_with("e'") {
        return Ok(RawDumpValue::Text(parse_sql_string_literal(stripped)?));
    }
    if let Ok(integer) = stripped.parse::<i64>() {
        return Ok(RawDumpValue::Integer(integer));
    }
    if let Ok(float) = stripped.parse::<f64>() {
        return Ok(RawDumpValue::Float(float));
    }
    Err(FlowableError::Internal(format!(
        "unsupported raw dump literal in bounded M32 parser: {value}"
    )))
}

fn parse_copy_dump_value(value: &str) -> Result<RawDumpValue, FlowableError> {
    if value == "\\N" {
        return Ok(RawDumpValue::Null);
    }

    let unescaped = value
        .replace("\\t", "\t")
        .replace("\\n", "\n")
        .replace("\\r", "\r")
        .replace("\\\\", "\\");

    if unescaped.eq_ignore_ascii_case("TRUE") {
        return Ok(RawDumpValue::Integer(1));
    }
    if unescaped.eq_ignore_ascii_case("FALSE") {
        return Ok(RawDumpValue::Integer(0));
    }
    if let Ok(integer) = unescaped.parse::<i64>() {
        return Ok(RawDumpValue::Integer(integer));
    }
    if let Ok(float) = unescaped.parse::<f64>() {
        return Ok(RawDumpValue::Float(float));
    }
    Ok(RawDumpValue::Text(unescaped))
}

fn strip_postgres_cast(value: &str) -> &str {
    let bytes = value.as_bytes();
    let mut quote: Option<u8> = None;
    let mut paren_depth = 0i32;
    let mut i = 0usize;

    while i + 1 < bytes.len() {
        let byte = bytes[i];
        if let Some(active_quote) = quote {
            if byte == active_quote {
                if i + 1 < bytes.len() && bytes[i + 1] == active_quote {
                    i += 2;
                    continue;
                }
                quote = None;
            }
            i += 1;
            continue;
        }

        if matches!(byte, b'\'' | b'"' | b'`') {
            quote = Some(byte);
            i += 1;
            continue;
        }
        if byte == b'(' {
            paren_depth += 1;
        } else if byte == b')' && paren_depth > 0 {
            paren_depth -= 1;
        } else if byte == b':' && bytes[i + 1] == b':' && paren_depth == 0 {
            return value[..i].trim_end();
        }
        i += 1;
    }

    value
}

fn parse_sql_string_literal(value: &str) -> Result<String, FlowableError> {
    let (body, escape_backslashes) = if value.starts_with("E'") || value.starts_with("e'") {
        (&value[1..], true)
    } else {
        (value, false)
    };

    if !body.starts_with('\'') || !body.ends_with('\'') || body.len() < 2 {
        return Err(FlowableError::Internal(format!(
            "unsupported raw dump string literal: {value}"
        )));
    }

    let mut result = String::new();
    let bytes = body.as_bytes();
    let mut i = 1usize;
    while i + 1 < bytes.len() {
        let byte = bytes[i];
        if byte == b'\'' {
            if i + 1 < bytes.len() - 1 && bytes[i + 1] == b'\'' {
                result.push('\'');
                i += 2;
                continue;
            }
            return Err(FlowableError::Internal(format!(
                "unsupported raw dump string literal quoting: {value}"
            )));
        }

        if escape_backslashes && byte == b'\\' && i + 1 < bytes.len() - 1 {
            let escaped = match bytes[i + 1] {
                b'n' => '\n',
                b'r' => '\r',
                b't' => '\t',
                b'\\' => '\\',
                b'\'' => '\'',
                other => other as char,
            };
            result.push(escaped);
            i += 2;
            continue;
        }

        result.push(byte as char);
        i += 1;
    }

    Ok(result)
}

fn decode_hex_bytes(hex: &str) -> Result<Vec<u8>, FlowableError> {
    if !hex.len().is_multiple_of(2) {
        return Err(FlowableError::Internal(format!(
            "raw dump hex literal has odd length: {hex}"
        )));
    }

    let mut bytes = Vec::with_capacity(hex.len() / 2);
    let chars: Vec<char> = hex.chars().collect();
    let mut i = 0usize;
    while i < chars.len() {
        let high = chars[i].to_digit(16).ok_or_else(|| {
            FlowableError::Internal(format!("invalid raw dump hex literal: {hex}"))
        })?;
        let low = chars[i + 1].to_digit(16).ok_or_else(|| {
            FlowableError::Internal(format!("invalid raw dump hex literal: {hex}"))
        })?;
        bytes.push(((high << 4) + low) as u8);
        i += 2;
    }
    Ok(bytes)
}

fn normalize_sql_identifier(identifier: &str) -> String {
    identifier
        .trim()
        .rsplit('.')
        .next()
        .unwrap_or(identifier)
        .trim()
        .trim_matches('"')
        .trim_matches('`')
        .to_string()
}

fn raw_supported_table_schema(table: &str) -> Option<&'static str> {
    match table {
        tables::ACT_RE_DEPLOYMENT => Some(
            "CREATE TABLE IF NOT EXISTS ACT_RE_DEPLOYMENT (
                ID_ TEXT PRIMARY KEY,
                NAME_ TEXT,
                CATEGORY_ TEXT,
                KEY_ TEXT,
                TENANT_ID_ TEXT,
                PARENT_DEPLOYMENT_ID_ TEXT,
                DERIVED_FROM_ TEXT,
                DERIVED_FROM_ROOT_ TEXT,
                ENGINE_VERSION_ TEXT,
                DEPLOY_TIME_ INTEGER
            )",
        ),
        tables::ACT_GE_BYTEARRAY => Some(
            "CREATE TABLE IF NOT EXISTS ACT_GE_BYTEARRAY (
                ID_ TEXT PRIMARY KEY,
                NAME_ TEXT,
                DEPLOYMENT_ID_ TEXT,
                BYTES_ BLOB
            )",
        ),
        tables::ACT_RE_PROCDEF => Some(
            "CREATE TABLE IF NOT EXISTS ACT_RE_PROCDEF (
                ID_ TEXT PRIMARY KEY,
                CATEGORY_ TEXT,
                NAME_ TEXT,
                KEY_ TEXT,
                DESCRIPTION_ TEXT,
                VERSION_ INTEGER,
                RESOURCE_NAME_ TEXT,
                DEPLOYMENT_ID_ TEXT,
                DGRM_RESOURCE_NAME_ TEXT,
                HAS_START_FORM_KEY_ INTEGER,
                HAS_GRAPHICAL_NOTATION_ INTEGER,
                SUSPENSION_STATE_ INTEGER,
                TENANT_ID_ TEXT,
                ENGINE_VERSION_ TEXT,
                APP_VERSION_ INTEGER
            )",
        ),
        tables::ACT_RU_EXECUTION => Some(
            "CREATE TABLE IF NOT EXISTS ACT_RU_EXECUTION (
                ID_ TEXT PRIMARY KEY,
                PARENT_ID_ TEXT,
                SUPER_EXEC_ TEXT,
                ROOT_PROC_INST_ID_ TEXT,
                PROC_INST_ID_ TEXT,
                PROC_DEF_ID_ TEXT,
                ACT_ID_ TEXT,
                IS_ACTIVE_ INTEGER,
                IS_CONCURRENT_ INTEGER,
                IS_SCOPE_ INTEGER,
                IS_MI_ROOT_ INTEGER,
                SUSPENSION_STATE_ INTEGER,
                TENANT_ID_ TEXT,
                NAME_ TEXT,
                BUSINESS_KEY_ TEXT,
                START_USER_ID_ TEXT,
                START_TIME_ INTEGER
            )",
        ),
        tables::ACT_RU_TASK => Some(
            "CREATE TABLE IF NOT EXISTS ACT_RU_TASK (
                ID_ TEXT PRIMARY KEY,
                PROC_INST_ID_ TEXT,
                EXECUTION_ID_ TEXT,
                TASK_DEF_KEY_ TEXT,
                NAME_ TEXT,
                PARENT_TASK_ID_ TEXT,
                CREATE_TIME_ INTEGER
            )",
        ),
        tables::ACT_RU_VARIABLE => Some(
            "CREATE TABLE IF NOT EXISTS ACT_RU_VARIABLE (
                ID_ TEXT PRIMARY KEY,
                EXECUTION_ID_ TEXT,
                PROC_INST_ID_ TEXT,
                NAME_ TEXT,
                TYPE_ TEXT,
                TEXT_ TEXT,
                TEXT2_ TEXT,
                LONG_ INTEGER,
                DOUBLE_ REAL
            )",
        ),
        tables::ACT_RU_TIMER_JOB => Some(
            "CREATE TABLE IF NOT EXISTS ACT_RU_TIMER_JOB (
                ID_ TEXT PRIMARY KEY,
                PROC_INST_ID_ TEXT,
                EXECUTION_ID_ TEXT,
                HANDLER_CFG_ TEXT,
                DUEDATE_ INTEGER,
                LOCK_OWNER_ TEXT,
                LOCK_EXP_TIME_ INTEGER,
                RETRIES_ INTEGER,
                EXCEPTION_MSG_ TEXT,
                JOB_HANDLER_TYPE_ TEXT
            )",
        ),
        tables::ACT_RU_EVENT_SUBSCR => Some(
            "CREATE TABLE IF NOT EXISTS ACT_RU_EVENT_SUBSCR (
                ID_ TEXT PRIMARY KEY,
                EXECUTION_ID_ TEXT,
                PROC_INST_ID_ TEXT,
                EVENT_TYPE_ TEXT,
                EVENT_NAME_ TEXT,
                ACTIVITY_ID_ TEXT
            )",
        ),
        tables::ACT_HI_PROCINST => Some(
            "CREATE TABLE IF NOT EXISTS ACT_HI_PROCINST (
                ID_ TEXT PRIMARY KEY,
                PROC_DEF_ID_ TEXT,
                BUSINESS_KEY_ TEXT,
                START_TIME_ INTEGER,
                END_TIME_ INTEGER,
                DURATION_ INTEGER,
                START_USER_ID_ TEXT,
                DELETE_REASON_ TEXT
            )",
        ),
        tables::ACT_HI_VARINST => Some(
            "CREATE TABLE IF NOT EXISTS ACT_HI_VARINST (
                ID_ TEXT PRIMARY KEY,
                PROC_INST_ID_ TEXT,
                EXECUTION_ID_ TEXT,
                TASK_ID_ TEXT,
                NAME_ TEXT,
                VAR_TYPE_ TEXT,
                TEXT_ TEXT,
                TEXT2_ TEXT,
                LONG_ INTEGER,
                DOUBLE_ REAL,
                CREATE_TIME_ INTEGER,
                LAST_UPDATED_TIME_ INTEGER
            )",
        ),
        _ => None,
    }
}

fn insert_rows_into_normalized_sqlite(
    conn: &Connection,
    table: &str,
    columns: &[String],
    rows: &[Vec<RawDumpValue>],
) -> Result<usize, FlowableError> {
    if columns.is_empty() {
        return Err(FlowableError::Internal(format!(
            "raw dump INSERT for table {} does not contain columns",
            table
        )));
    }

    let placeholders = (0..columns.len())
        .map(|_| "?")
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "INSERT INTO {} ({}) VALUES ({})",
        table,
        columns.join(", "),
        placeholders
    );

    let mut inserted = 0usize;
    for row in rows {
        if row.len() != columns.len() {
            return Err(FlowableError::Internal(format!(
                "raw dump INSERT row for table {} has {} values but {} columns",
                table,
                row.len(),
                columns.len()
            )));
        }
        let params = columns
            .iter()
            .zip(row.iter())
            .map(|(column, value)| raw_dump_value_to_sql_value(table, column, value))
            .collect::<Result<Vec<_>, _>>()?;
        conn.execute(&sql, params_from_iter(params))
            .map_err(|error| {
                FlowableError::Internal(format!(
                    "failed to insert normalized row into {} from raw sql dump: {}",
                    table, error
                ))
            })?;
        inserted += 1;
    }

    Ok(inserted)
}

fn raw_dump_value_to_sql_value(
    table: &str,
    column: &str,
    value: &RawDumpValue,
) -> Result<Value, FlowableError> {
    match value {
        RawDumpValue::Null => Ok(Value::Null),
        RawDumpValue::Integer(integer) => Ok(Value::Integer(*integer)),
        RawDumpValue::Float(float) => Ok(Value::Real(*float)),
        RawDumpValue::Bytes(bytes) => Ok(Value::Blob(bytes.clone())),
        RawDumpValue::Text(text) if table == tables::ACT_GE_BYTEARRAY && column == "BYTES_" => {
            Ok(Value::Blob(text.as_bytes().to_vec()))
        }
        RawDumpValue::Text(text) => Ok(Value::Text(text.clone())),
    }
}

fn read_source_manifest(path: &Path) -> Result<HistoricalMigrationSourceManifest, FlowableError> {
    let bytes = std::fs::read(path).map_err(|error| {
        FlowableError::Internal(format!(
            "failed to read historical migration source manifest {}: {}",
            path.display(),
            error
        ))
    })?;
    let manifest: HistoricalMigrationSourceManifest =
        serde_json::from_slice(&bytes).map_err(|error| {
            FlowableError::Internal(format!(
                "failed to parse historical migration source manifest {}: {}",
                path.display(),
                error
            ))
        })?;
    if manifest.format != SOURCE_MANIFEST_FORMAT_V1 {
        return Err(FlowableError::Internal(format!(
            "unsupported historical migration source manifest format at {}: {}",
            path.display(),
            manifest.format
        )));
    }
    Ok(manifest)
}

fn resolve_manifest_source_path(base_dir: &Path, raw_path: &str) -> PathBuf {
    let candidate = PathBuf::from(raw_path);
    if candidate.is_absolute() {
        candidate
    } else {
        base_dir.join(candidate)
    }
}

fn decorate_source_manifest_report(
    manifest_path: &Path,
    source: &HistoricalMigrationSourceDescriptor,
    report: &mut HistoricalMigrationReport,
) {
    report.source_path = manifest_path.display().to_string();
    let warnings = [
        format!(
            "source manifest import uses declared historical source kind '{}'",
            source_manifest_kind_name(source)
        ),
        format!(
            "source manifest {} remains on the owned historical inspect/import pipeline",
            manifest_path.display()
        ),
    ];
    for warning in warnings {
        if !report.warnings.iter().any(|existing| existing == &warning) {
            report.warnings.push(warning);
        }
    }
}

fn source_manifest_kind_name(source: &HistoricalMigrationSourceDescriptor) -> &'static str {
    match source {
        HistoricalMigrationSourceDescriptor::SqliteDb { .. } => "sqlite-db",
        HistoricalMigrationSourceDescriptor::SqliteDump { .. } => "sqlite-dump",
        HistoricalMigrationSourceDescriptor::PortableBundle { .. } => "portable-bundle",
        HistoricalMigrationSourceDescriptor::MysqlValuesDump { .. } => "mysql-values-dump",
        HistoricalMigrationSourceDescriptor::PostgresValuesDump { .. } => "postgres-values-dump",
        HistoricalMigrationSourceDescriptor::PostgresCopyDump { .. } => "postgres-copy-dump",
        HistoricalMigrationSourceDescriptor::LiveSqlx { .. } => "live-sqlx",
    }
}

fn normalize_live_database_kind(kind: &str) -> Result<String, FlowableError> {
    match kind.trim().to_ascii_lowercase().as_str() {
        "sqlite" | "sqlite3" => Ok("sqlite".to_string()),
        "postgres" | "postgresql" | "pg" => Ok("postgres".to_string()),
        "mysql" | "mariadb" => Ok("mysql".to_string()),
        other => Err(FlowableError::Internal(format!(
            "unsupported live historical migration database kind '{other}' \
             (expected sqlite, postgres, or mysql)"
        ))),
    }
}

fn sqlite_path_from_live_url(url: &str) -> Result<PathBuf, FlowableError> {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return Err(FlowableError::Internal(
            "live sqlite historical migration url must not be empty".to_string(),
        ));
    }

    if let Some(rest) = trimmed.strip_prefix("sqlite:") {
        // Accept sqlite:path, sqlite://path, and sqlite:///abs/path.
        let rest = rest.strip_prefix("//").unwrap_or(rest);
        let path = if cfg!(windows) {
            // sqlite:///C:/data.db -> /C:/data.db after // strip; drop leading slash.
            let candidate = rest.strip_prefix('/').unwrap_or(rest);
            PathBuf::from(candidate)
        } else {
            PathBuf::from(rest)
        };
        return Ok(path);
    }

    Ok(PathBuf::from(trimmed))
}

fn inspect_live_sqlite_url(url: &str) -> Result<HistoricalMigrationReport, FlowableError> {
    let path = sqlite_path_from_live_url(url)?;
    if !path.exists() {
        return Err(FlowableError::Internal(format!(
            "failed to open live historical migration sqlite source {}: path does not exist",
            path.display()
        )));
    }
    let mut report = inspect_historical_migration_sqlite(&path)?;
    report.source_path = url.to_string();
    report.warnings.push(
        "live SQLite inspect uses a bounded direct connection (table presence and counts only for remote kinds; local SQLite reuses the full inspect pipeline)"
            .to_string(),
    );
    Ok(report)
}

fn inspect_live_remote_url(
    url: &str,
    kind: flowable_persistence::DatabaseKind,
) -> Result<HistoricalMigrationReport, FlowableError> {
    use flowable_persistence::LiveSqlProbe;

    let mut probe = LiveSqlProbe::connect(kind, url).map_err(|error| {
        FlowableError::Internal(format!(
            "failed to connect live historical migration source ({kind}): {error}"
        ))
    })?;

    let candidate_tables = [
        tables::ACT_RE_DEPLOYMENT,
        tables::ACT_GE_BYTEARRAY,
        tables::ACT_RE_PROCDEF,
        tables::ACT_RU_EXECUTION,
        tables::ACT_RU_TASK,
        tables::ACT_RU_VARIABLE,
        tables::ACT_RU_TIMER_JOB,
        tables::ACT_RU_EVENT_SUBSCR,
        tables::ACT_HI_PROCINST,
        tables::ACT_HI_VARINST,
    ];

    let mut present_tables = Vec::new();
    for table in candidate_tables {
        match probe.table_exists(table) {
            Ok(true) => present_tables.push(table.to_string()),
            Ok(false) => {}
            Err(error) => {
                return Err(FlowableError::Internal(format!(
                    "failed to probe live historical migration table {table}: {error}"
                )));
            }
        }
    }
    present_tables.sort();

    let mut warnings = Vec::new();
    if !present_tables
        .iter()
        .any(|table| table == tables::ACT_RE_DEPLOYMENT)
    {
        warnings.push(
            "missing ACT_RE_DEPLOYMENT; repository migration baseline will be empty".to_string(),
        );
    }
    if !present_tables
        .iter()
        .any(|table| table == tables::ACT_RE_PROCDEF)
    {
        warnings.push(
            "missing ACT_RE_PROCDEF; process-definition migration baseline will be empty"
                .to_string(),
        );
    }
    if !present_tables
        .iter()
        .any(|table| table == tables::ACT_GE_BYTEARRAY)
    {
        warnings.push(
            "missing ACT_GE_BYTEARRAY; BPMN resource migration baseline will be empty".to_string(),
        );
    }

    let supported_runtime_variable_types = supported_runtime_variable_types();
    let supported_historic_variable_types = supported_historic_variable_types();
    let mut unsupported_types = BTreeSet::new();

    if present_tables
        .iter()
        .any(|table| table == tables::ACT_RU_VARIABLE)
    {
        for variable_type in probe
            .list_distinct_text_values(tables::ACT_RU_VARIABLE, "TYPE_")
            .map_err(|error| {
                FlowableError::Internal(format!(
                    "failed to list live runtime variable types: {error}"
                ))
            })?
        {
            if !supported_runtime_variable_types.contains(variable_type.as_str()) {
                unsupported_types.insert(variable_type);
            }
        }
    }

    if present_tables
        .iter()
        .any(|table| table == tables::ACT_HI_VARINST)
    {
        for variable_type in probe
            .list_distinct_text_values(tables::ACT_HI_VARINST, "VAR_TYPE_")
            .map_err(|error| {
                FlowableError::Internal(format!(
                    "failed to list live historic variable types: {error}"
                ))
            })?
        {
            if !supported_historic_variable_types.contains(variable_type.as_str()) {
                unsupported_types.insert(variable_type);
            }
        }
    }

    if present_tables
        .iter()
        .any(|table| table == tables::ACT_RU_TIMER_JOB)
    {
        warnings.push(
            "ACT_RU_TIMER_JOB import is best-effort in M21 baseline and assumes handler configuration is already resolved for the owned subset"
                .to_string(),
        );
    }

    warnings.push(format!(
        "live {kind} historical inspect is bounded (table presence and counts only; full remote import is not performed)"
    ));

    let map_count_err = |table: &str, error: flowable_persistence::PersistenceError| {
        FlowableError::Internal(format!(
            "failed to count live historical migration table {table}: {error}"
        ))
    };

    let deployment_count = probe
        .count_rows(tables::ACT_RE_DEPLOYMENT)
        .map_err(|error| map_count_err(tables::ACT_RE_DEPLOYMENT, error))?;
    let deployment_resource_count = probe
        .count_rows(tables::ACT_GE_BYTEARRAY)
        .map_err(|error| map_count_err(tables::ACT_GE_BYTEARRAY, error))?;
    let process_definition_count = probe
        .count_rows(tables::ACT_RE_PROCDEF)
        .map_err(|error| map_count_err(tables::ACT_RE_PROCDEF, error))?;
    let process_instance_count = probe
        .count_process_instances(tables::ACT_RU_EXECUTION)
        .map_err(|error| {
            FlowableError::Internal(format!(
                "failed to count live historical process instances: {error}"
            ))
        })?;
    let execution_count = probe
        .count_rows(tables::ACT_RU_EXECUTION)
        .map_err(|error| map_count_err(tables::ACT_RU_EXECUTION, error))?;
    let task_count = probe
        .count_rows(tables::ACT_RU_TASK)
        .map_err(|error| map_count_err(tables::ACT_RU_TASK, error))?;
    let variable_count = probe
        .count_rows(tables::ACT_RU_VARIABLE)
        .map_err(|error| map_count_err(tables::ACT_RU_VARIABLE, error))?;
    let timer_job_count = probe
        .count_rows(tables::ACT_RU_TIMER_JOB)
        .map_err(|error| map_count_err(tables::ACT_RU_TIMER_JOB, error))?;
    let event_subscription_count = probe
        .count_rows(tables::ACT_RU_EVENT_SUBSCR)
        .map_err(|error| map_count_err(tables::ACT_RU_EVENT_SUBSCR, error))?;
    let historic_process_instance_count = probe
        .count_rows(tables::ACT_HI_PROCINST)
        .map_err(|error| map_count_err(tables::ACT_HI_PROCINST, error))?;
    let historic_variable_count = probe
        .count_rows(tables::ACT_HI_VARINST)
        .map_err(|error| map_count_err(tables::ACT_HI_VARINST, error))?;

    Ok(HistoricalMigrationReport {
        source_path: url.to_string(),
        present_tables,
        deployment_count,
        deployment_resource_count,
        process_definition_count,
        process_instance_count,
        execution_count,
        task_count,
        variable_count,
        timer_job_count,
        event_subscription_count,
        historic_process_instance_count,
        historic_variable_count,
        unsupported_variable_types: unsupported_types.into_iter().collect(),
        warnings,
    })
}

fn list_tables(conn: &Connection) -> Result<Vec<String>, FlowableError> {
    let mut stmt = conn
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
        .map_err(sql_to_error)?;
    let rows = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(sql_to_error)?;
    Ok(rows.filter_map(Result::ok).collect())
}

fn has_table(conn: &Connection, table: &str) -> Result<bool, FlowableError> {
    let mut stmt = conn
        .prepare("SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1")
        .map_err(sql_to_error)?;
    let exists = stmt
        .query_row([table], |_| Ok(()))
        .map(|_| true)
        .or_else(|error| match error {
            rusqlite::Error::QueryReturnedNoRows => Ok(false),
            other => Err(other),
        })
        .map_err(sql_to_error)?;
    Ok(exists)
}

fn has_column(conn: &Connection, table: &str, column: &str) -> Result<bool, FlowableError> {
    if !has_table(conn, table)? {
        return Ok(false);
    }

    let mut stmt = conn
        .prepare(&format!("PRAGMA table_info({table})"))
        .map_err(sql_to_error)?;
    let rows = stmt
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(sql_to_error)?;

    for row in rows {
        if row.map_err(sql_to_error)?.eq_ignore_ascii_case(column) {
            return Ok(true);
        }
    }

    Ok(false)
}

fn count_rows_if_present(conn: &Connection, table: &str) -> Result<usize, FlowableError> {
    if !has_table(conn, table)? {
        return Ok(0);
    }

    let query = format!("SELECT COUNT(*) FROM {}", table);
    let count = conn
        .query_row(&query, [], |row| row.get::<_, i64>(0))
        .map_err(sql_to_error)?;
    Ok(count.max(0) as usize)
}

fn count_process_instances(conn: &Connection) -> Result<usize, FlowableError> {
    if !has_table(conn, tables::ACT_RU_EXECUTION)? {
        return Ok(0);
    }

    let count = conn
        .query_row(
            "SELECT COUNT(*) FROM ACT_RU_EXECUTION WHERE ID_ = PROC_INST_ID_",
            [],
            |row| row.get::<_, i64>(0),
        )
        .map_err(sql_to_error)?;
    Ok(count.max(0) as usize)
}

fn list_distinct_text_values(
    conn: &Connection,
    table: &str,
    column: &str,
) -> Result<Vec<String>, FlowableError> {
    let query = format!(
        "SELECT DISTINCT {} FROM {} WHERE {} IS NOT NULL ORDER BY {}",
        column, table, column, column
    );
    let mut stmt = conn.prepare(&query).map_err(sql_to_error)?;
    let rows = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(sql_to_error)?;
    Ok(rows.filter_map(Result::ok).collect())
}

fn load_deployments(conn: &Connection) -> Result<Vec<Deployment>, FlowableError> {
    if !has_table(conn, tables::ACT_RE_DEPLOYMENT)? {
        return Ok(Vec::new());
    }

    let mut stmt = conn
        .prepare(
            "SELECT ID_, NAME_, CATEGORY_, KEY_, TENANT_ID_, PARENT_DEPLOYMENT_ID_, DERIVED_FROM_, DERIVED_FROM_ROOT_, ENGINE_VERSION_, DEPLOY_TIME_ FROM ACT_RE_DEPLOYMENT ORDER BY ID_",
        )
        .map_err(sql_to_error)?;
    let rows = stmt
        .query_map([], |row| Ok(map_deployment(row)))
        .map_err(sql_to_error)?;

    Ok(rows.filter_map(Result::ok).collect())
}

fn map_deployment(row: &Row<'_>) -> Deployment {
    Deployment {
        id: row.get::<_, String>(0).unwrap_or_default(),
        name: row.get::<_, Option<String>>(1).unwrap_or(None),
        category: row.get::<_, Option<String>>(2).unwrap_or(None),
        key: row.get::<_, Option<String>>(3).unwrap_or(None),
        tenant_id: row.get::<_, Option<String>>(4).unwrap_or(None),
        parent_deployment_id: row.get::<_, Option<String>>(5).unwrap_or(None),
        derived_from: row.get::<_, Option<String>>(6).unwrap_or(None),
        derived_from_root: row.get::<_, Option<String>>(7).unwrap_or(None),
        engine_version: row.get::<_, Option<String>>(8).unwrap_or(None),
        deployment_time: optional_datetime(row, 9),
        is_new: false,
        resources: HashMap::new(),
    }
}

fn load_deployment_resources(
    conn: &Connection,
) -> Result<HashMap<String, HashMap<String, Vec<u8>>>, FlowableError> {
    if !has_table(conn, tables::ACT_GE_BYTEARRAY)? {
        return Ok(HashMap::new());
    }

    let mut resources: HashMap<String, HashMap<String, Vec<u8>>> = HashMap::new();
    let mut stmt = conn
        .prepare(
            "SELECT DEPLOYMENT_ID_, NAME_, BYTES_ FROM ACT_GE_BYTEARRAY WHERE DEPLOYMENT_ID_ IS NOT NULL ORDER BY DEPLOYMENT_ID_, NAME_",
        )
        .map_err(sql_to_error)?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0).unwrap_or_default(),
                row.get::<_, String>(1).unwrap_or_default(),
                row.get::<_, Option<Vec<u8>>>(2)
                    .unwrap_or(None)
                    .unwrap_or_default(),
            ))
        })
        .map_err(sql_to_error)?;

    for (deployment_id, name, bytes) in rows.filter_map(Result::ok) {
        resources
            .entry(deployment_id)
            .or_default()
            .insert(name, bytes);
    }

    Ok(resources)
}

fn load_process_definitions(conn: &Connection) -> Result<Vec<ProcessDefinition>, FlowableError> {
    if !has_table(conn, tables::ACT_RE_PROCDEF)? {
        return Ok(Vec::new());
    }

    let mut stmt = conn
        .prepare(
            "SELECT ID_, CATEGORY_, NAME_, KEY_, DESCRIPTION_, VERSION_, RESOURCE_NAME_, DEPLOYMENT_ID_, DGRM_RESOURCE_NAME_, HAS_START_FORM_KEY_, HAS_GRAPHICAL_NOTATION_, SUSPENSION_STATE_, TENANT_ID_, ENGINE_VERSION_, APP_VERSION_ FROM ACT_RE_PROCDEF ORDER BY ID_",
        )
        .map_err(sql_to_error)?;
    let rows = stmt
        .query_map([], |row| {
            Ok(ProcessDefinition {
                id: row.get::<_, String>(0).unwrap_or_default(),
                category: row.get::<_, Option<String>>(1).unwrap_or(None),
                name: row.get::<_, Option<String>>(2).unwrap_or(None),
                key: row.get::<_, String>(3).unwrap_or_default(),
                description: row.get::<_, Option<String>>(4).unwrap_or(None),
                version: row.get::<_, i32>(5).unwrap_or(1),
                resource_name: row.get::<_, Option<String>>(6).unwrap_or(None),
                deployment_id: row.get::<_, Option<String>>(7).unwrap_or(None),
                diagram_resource_name: row.get::<_, Option<String>>(8).unwrap_or(None),
                has_start_form_key: optional_bool(row, 9).unwrap_or(false),
                has_graphical_notation: optional_bool(row, 10).unwrap_or(false),
                is_suspended: matches!(row.get::<_, Option<i64>>(11).unwrap_or(None), Some(value) if value != 1),
                tenant_id: row.get::<_, Option<String>>(12).unwrap_or(None),
                engine_version: row.get::<_, Option<String>>(13).unwrap_or(None),
                app_version: row.get::<_, Option<i32>>(14).unwrap_or(None),
            history_level: None,
            })
        })
        .map_err(sql_to_error)?;

    Ok(rows.filter_map(Result::ok).collect())
}

fn load_process_instances(
    conn: &Connection,
    process_definition_map: &HashMap<String, ProcessDefinition>,
) -> Result<Vec<ProcessInstance>, FlowableError> {
    if !has_table(conn, tables::ACT_RU_EXECUTION)? {
        return Ok(Vec::new());
    }

    let mut instances = Vec::new();
    let mut stmt = conn
        .prepare(
            "SELECT ID_, PROC_INST_ID_, PROC_DEF_ID_, BUSINESS_KEY_, NAME_, START_USER_ID_, START_TIME_, SUSPENSION_STATE_, TENANT_ID_, ROOT_PROC_INST_ID_ FROM ACT_RU_EXECUTION WHERE ID_ = PROC_INST_ID_ ORDER BY ID_",
        )
        .map_err(sql_to_error)?;
    let rows = stmt
        .query_map([], |row| {
            let id = row.get::<_, String>(0).unwrap_or_default();
            let proc_def_id = row.get::<_, Option<String>>(2).unwrap_or(None).unwrap_or_default();
            let process_definition = process_definition_map.get(&proc_def_id);
            Ok(ProcessInstance {
                id,
                name: row.get::<_, Option<String>>(4).unwrap_or(None),
                process_definition_id: proc_def_id.clone(),
                process_definition_key: process_definition
                    .map(|definition| definition.key.clone())
                    .unwrap_or_default(),
                process_definition_name: process_definition.and_then(|definition| definition.name.clone()),
                process_definition_version: process_definition
                    .map(|definition| definition.version)
                    .unwrap_or(1),
                business_key: row.get::<_, Option<String>>(3).unwrap_or(None),
                business_status: None,
                is_suspended: matches!(row.get::<_, Option<i64>>(7).unwrap_or(None), Some(value) if value != 1),
                tenant_id: row.get::<_, Option<String>>(8).unwrap_or(None),
                start_time: optional_datetime(row, 6),
                start_user_id: row.get::<_, Option<String>>(5).unwrap_or(None),
                callback_id: None,
                callback_type: None,
                reference_id: None,
                reference_type: None,
                is_ended: false,
                super_execution_id: None,
                root_process_instance_id: row.get::<_, Option<String>>(9).unwrap_or(None),
            })
        })
        .map_err(sql_to_error)?;

    for instance in rows.filter_map(Result::ok) {
        instances.push(instance);
    }

    Ok(instances)
}

fn load_executions(
    conn: &Connection,
    runtime_variables: &HashMap<String, HashMap<String, serde_json::Value>>,
    process_definition_map: &HashMap<String, ProcessDefinition>,
) -> Result<Vec<Execution>, FlowableError> {
    if !has_table(conn, tables::ACT_RU_EXECUTION)? {
        return Ok(Vec::new());
    }

    let mut stmt = conn
        .prepare(
            "SELECT ID_, PARENT_ID_, SUPER_EXEC_, ROOT_PROC_INST_ID_, PROC_INST_ID_, PROC_DEF_ID_, ACT_ID_, IS_ACTIVE_, IS_CONCURRENT_, IS_SCOPE_, IS_MI_ROOT_, SUSPENSION_STATE_, TENANT_ID_, NAME_ FROM ACT_RU_EXECUTION ORDER BY ID_",
        )
        .map_err(sql_to_error)?;
    let rows = stmt
        .query_map([], |row| {
            let id = row.get::<_, String>(0).unwrap_or_default();
            let proc_def_id = row.get::<_, Option<String>>(5).unwrap_or(None);
            let process_definition = proc_def_id
                .as_ref()
                .and_then(|definition_id| process_definition_map.get(definition_id));

            Ok(Execution {
                id: id.clone(),
                parent_id: row.get::<_, Option<String>>(1).unwrap_or(None),
                super_execution_id: row.get::<_, Option<String>>(2).unwrap_or(None),
                root_process_instance_id: row.get::<_, Option<String>>(3).unwrap_or(None),
                process_instance_id: row.get::<_, Option<String>>(4).unwrap_or(None),
                process_definition_id: proc_def_id.clone(),
                process_definition_key: process_definition.map(|definition| definition.key.clone()),
                process_definition_name: process_definition.and_then(|definition| definition.name.clone()),
                process_definition_version: process_definition.map(|definition| definition.version),
                activity_id: row.get::<_, Option<String>>(6).unwrap_or(None),
                activity_name: None,
                name: row.get::<_, Option<String>>(13).unwrap_or(None),
                description: None,
                is_suspended: matches!(row.get::<_, Option<i64>>(11).unwrap_or(None), Some(value) if value != 1),
                is_ended: false,
                is_active: optional_bool(row, 7).unwrap_or(true),
                is_concurrent: optional_bool(row, 8).unwrap_or(false),
                is_scope: optional_bool(row, 9).unwrap_or(true),
                is_multi_instance_root: optional_bool(row, 10).unwrap_or(false),
                tenant_id: row.get::<_, Option<String>>(12).unwrap_or(None),
                reference_id: None,
                reference_type: None,
                variables: runtime_variables.get(&id).cloned().unwrap_or_default(),
                local_variables: HashMap::new(),
                transient_variables: HashMap::new(),
                non_interrupting_event_subprocess_path: false,
            })
        })
        .map_err(sql_to_error)?;

    Ok(rows.filter_map(Result::ok).collect())
}

fn load_tasks(conn: &Connection) -> Result<Vec<Task>, FlowableError> {
    if !has_table(conn, tables::ACT_RU_TASK)? {
        return Ok(Vec::new());
    }

    let parent_task_column = if has_column(conn, tables::ACT_RU_TASK, "PARENT_TASK_ID_")? {
        "PARENT_TASK_ID_"
    } else {
        "NULL AS PARENT_TASK_ID_"
    };
    let query = format!(
        "SELECT ID_, PROC_INST_ID_, EXECUTION_ID_, TASK_DEF_KEY_, NAME_, {parent_task_column}, CREATE_TIME_ FROM ACT_RU_TASK ORDER BY ID_"
    );
    let mut stmt = conn.prepare(&query).map_err(sql_to_error)?;
    let rows = stmt
        .query_map([], |row| {
            Ok(Task {
                id: row.get::<_, String>(0).unwrap_or_default(),
                process_instance_id: row
                    .get::<_, Option<String>>(1)
                    .unwrap_or(None)
                    .unwrap_or_default(),
                execution_id: row
                    .get::<_, Option<String>>(2)
                    .unwrap_or(None)
                    .unwrap_or_default(),
                task_definition_key: row
                    .get::<_, Option<String>>(3)
                    .unwrap_or(None)
                    .unwrap_or_default(),
                name: row
                    .get::<_, Option<String>>(4)
                    .unwrap_or(None)
                    .unwrap_or_default(),
                description: None,
                assignee: None,
                owner: None,
                delegation_state: None,
                parent_task_id: row.get::<_, Option<String>>(5).unwrap_or(None),
                priority: None,
                due_date: None,
                category: None,
                form_key: None,
                tenant_id: None,
                is_completed: false,
                created_time: optional_datetime(row, 6),
                completed_time: None,
                claim_time: None,
                state: "created".to_string(),
                suspension_state: 0,
                local_variables: HashMap::new(),
            })
        })
        .map_err(sql_to_error)?;

    Ok(rows.filter_map(Result::ok).collect())
}

fn load_runtime_variables(
    conn: &Connection,
) -> Result<HashMap<String, HashMap<String, serde_json::Value>>, FlowableError> {
    if !has_table(conn, tables::ACT_RU_VARIABLE)? {
        return Ok(HashMap::new());
    }

    let mut variables_by_execution = HashMap::new();
    let mut stmt = conn
        .prepare(
            "SELECT EXECUTION_ID_, NAME_, TYPE_, TEXT_, TEXT2_, LONG_, DOUBLE_ FROM ACT_RU_VARIABLE ORDER BY ID_",
        )
        .map_err(sql_to_error)?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, Option<String>>(0)
                    .unwrap_or(None)
                    .unwrap_or_default(),
                row.get::<_, String>(1).unwrap_or_default(),
                row.get::<_, Option<String>>(2).unwrap_or(None),
                row.get::<_, Option<String>>(3).unwrap_or(None),
                row.get::<_, Option<String>>(4).unwrap_or(None),
                row.get::<_, Option<i64>>(5).unwrap_or(None),
                row.get::<_, Option<f64>>(6).unwrap_or(None),
            ))
        })
        .map_err(sql_to_error)?;

    for row in rows {
        let (
            execution_id,
            name,
            variable_type,
            text_value,
            alt_text_value,
            long_value,
            double_value,
        ) = row.map_err(sql_to_error)?;
        if !is_supported_runtime_variable_type(variable_type.as_deref()) {
            continue;
        }
        let value = parse_variable_value_from_columns(
            variable_type.as_deref(),
            text_value,
            alt_text_value,
            long_value,
            double_value,
        )?;
        variables_by_execution
            .entry(execution_id)
            .or_insert_with(HashMap::new)
            .insert(name, value);
    }

    Ok(variables_by_execution)
}

fn load_timer_jobs(conn: &Connection) -> Result<Vec<RuntimeTimerJobState>, FlowableError> {
    if !has_table(conn, tables::ACT_RU_TIMER_JOB)? {
        return Ok(Vec::new());
    }

    let mut stmt = conn
        .prepare(
            "SELECT ID_, PROC_INST_ID_, EXECUTION_ID_, HANDLER_CFG_, DUEDATE_, LOCK_OWNER_, LOCK_EXP_TIME_, RETRIES_, EXCEPTION_MSG_, JOB_HANDLER_TYPE_ FROM ACT_RU_TIMER_JOB ORDER BY ID_",
        )
        .map_err(sql_to_error)?;
    let rows = stmt
        .query_map([], |row| {
            let handler_cfg = row.get::<_, Option<String>>(3).unwrap_or(None);
            let activity_id = handler_cfg
                .clone()
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| {
                    row.get::<_, Option<String>>(9)
                        .unwrap_or(None)
                        .unwrap_or_else(|| "historicalImportedTimer".to_string())
                });
            Ok(RuntimeTimerJobState {
                timer_job_id: row.get::<_, String>(0).unwrap_or_default(),
                process_instance_id: row
                    .get::<_, Option<String>>(1)
                    .unwrap_or(None)
                    .unwrap_or_default(),
                execution_id: row
                    .get::<_, Option<String>>(2)
                    .unwrap_or(None)
                    .unwrap_or_default(),
                activity_id,
                job_state: Some("timer".to_string()),
                is_boundary: false,
                attached_activity_id: None,
                cancel_activity: false,
                time_duration: None,
                time_date: None,
                time_cycle: None,
                end_date: None,
                due_time: optional_timestamp_millis(row, 4),
                lock_owner: row.get::<_, Option<String>>(5).unwrap_or(None),
                lock_time: None,
                lock_expiration_time: optional_timestamp_millis(row, 6),
                retries: row.get::<_, Option<i32>>(7).unwrap_or(None),
                error_message: row.get::<_, Option<String>>(8).unwrap_or(None),
                error_details: None,
                category: None,
                ..Default::default()
            })
        })
        .map_err(sql_to_error)?;

    Ok(rows.filter_map(Result::ok).collect())
}

fn load_event_wait_states(conn: &Connection) -> Result<Vec<RuntimeEventWaitState>, FlowableError> {
    if !has_table(conn, tables::ACT_RU_EVENT_SUBSCR)? {
        return Ok(Vec::new());
    }

    let mut states = Vec::new();
    let mut stmt = conn
        .prepare(
            "SELECT ID_, EXECUTION_ID_, PROC_INST_ID_, EVENT_TYPE_, EVENT_NAME_, ACTIVITY_ID_ FROM ACT_RU_EVENT_SUBSCR ORDER BY ID_",
        )
        .map_err(sql_to_error)?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0).unwrap_or_default(),
                row.get::<_, Option<String>>(1)
                    .unwrap_or(None)
                    .unwrap_or_default(),
                row.get::<_, Option<String>>(2)
                    .unwrap_or(None)
                    .unwrap_or_default(),
                row.get::<_, Option<String>>(3)
                    .unwrap_or(None)
                    .unwrap_or_default(),
                row.get::<_, Option<String>>(4)
                    .unwrap_or(None)
                    .unwrap_or_default(),
                row.get::<_, Option<String>>(5).unwrap_or(None),
            ))
        })
        .map_err(sql_to_error)?;

    for row in rows {
        let (_id, execution_id, process_instance_id, event_type, event_name, activity_id) =
            row.map_err(sql_to_error)?;

        let event_subscription = match map_event_subscription_kind(&event_type) {
            Some(kind) if !event_name.is_empty() => Some(EventSubscription {
                kind,
                event_ref: event_name.clone(),
            }),
            _ => None,
        };

        if let Some(subscription) = event_subscription {
            states.push(RuntimeEventWaitState {
                wait_kind: match subscription.kind {
                    EventSubscriptionKind::Signal => {
                        RuntimeEventWaitKind::SignalIntermediateCatchEvent
                    }
                    EventSubscriptionKind::Conditional => {
                        RuntimeEventWaitKind::ConditionalIntermediateCatchEvent
                    }
                    EventSubscriptionKind::Error => {
                        RuntimeEventWaitKind::ErrorIntermediateCatchEvent
                    }
                    EventSubscriptionKind::Cancel => {
                        RuntimeEventWaitKind::CancelIntermediateCatchEvent
                    }
                    EventSubscriptionKind::Compensate => {
                        RuntimeEventWaitKind::CompensateIntermediateCatchEvent
                    }
                    EventSubscriptionKind::Escalation => {
                        RuntimeEventWaitKind::EscalationIntermediateCatchEvent
                    }
                    _ => RuntimeEventWaitKind::MessageIntermediateCatchEvent,
                },
                process_instance_id,
                execution_id,
                task_id: None,
                activity_id,
                display_name: None,
                event_subscription: Some(subscription),
                configuration: None,
            });
        }
    }

    Ok(states)
}

fn load_historic_process_instances(
    conn: &Connection,
) -> Result<Vec<HistoricProcessInstance>, FlowableError> {
    if !has_table(conn, tables::ACT_HI_PROCINST)? {
        return Ok(Vec::new());
    }

    let mut stmt = conn
        .prepare(
            "SELECT ID_, PROC_DEF_ID_, BUSINESS_KEY_, START_TIME_, END_TIME_, DURATION_, START_USER_ID_, DELETE_REASON_ FROM ACT_HI_PROCINST ORDER BY ID_",
        )
        .map_err(sql_to_error)?;
    let rows = stmt
        .query_map([], |row| {
            let start_time = optional_datetime(row, 3).unwrap_or_else(Utc::now);
            let end_time = optional_datetime(row, 4);
            let duration_ms = row.get::<_, Option<i64>>(5).unwrap_or(None).or_else(|| {
                end_time.map(|end| end.timestamp_millis() - start_time.timestamp_millis())
            });
            Ok(HistoricProcessInstance {
                id: row.get::<_, String>(0).unwrap_or_default(),
                process_definition_id: row
                    .get::<_, Option<String>>(1)
                    .unwrap_or(None)
                    .unwrap_or_default(),
                business_key: row.get::<_, Option<String>>(2).unwrap_or(None),
                start_time,
                end_time,
                duration_ms,
                start_user_id: row.get::<_, Option<String>>(6).unwrap_or(None),
                delete_reason: row.get::<_, Option<String>>(7).unwrap_or(None),
            })
        })
        .map_err(sql_to_error)?;

    Ok(rows.filter_map(Result::ok).collect())
}

fn load_historic_variable_instances(
    conn: &Connection,
) -> Result<Vec<HistoricVariableInstance>, FlowableError> {
    if !has_table(conn, tables::ACT_HI_VARINST)? {
        return Ok(Vec::new());
    }

    let mut stmt = conn
        .prepare(
            "SELECT ID_, PROC_INST_ID_, EXECUTION_ID_, TASK_ID_, NAME_, VAR_TYPE_, TEXT_, TEXT2_, LONG_, DOUBLE_, CREATE_TIME_, LAST_UPDATED_TIME_ FROM ACT_HI_VARINST ORDER BY ID_",
        )
        .map_err(sql_to_error)?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0).unwrap_or_default(),
                row.get::<_, Option<String>>(1)
                    .unwrap_or(None)
                    .unwrap_or_default(),
                row.get::<_, Option<String>>(2).unwrap_or(None),
                row.get::<_, Option<String>>(3).unwrap_or(None),
                row.get::<_, String>(4).unwrap_or_default(),
                row.get::<_, Option<String>>(5)
                    .unwrap_or(None)
                    .unwrap_or_default(),
                row.get::<_, Option<String>>(6).unwrap_or(None),
                row.get::<_, Option<String>>(7).unwrap_or(None),
                row.get::<_, Option<i64>>(8).unwrap_or(None),
                row.get::<_, Option<f64>>(9).unwrap_or(None),
                optional_datetime(row, 10),
                optional_datetime(row, 11),
            ))
        })
        .map_err(sql_to_error)?;

    let mut result = Vec::new();
    for row in rows {
        let (
            id,
            process_instance_id,
            execution_id,
            task_id,
            name,
            variable_type,
            text_value,
            alt_text_value,
            long_value,
            double_value,
            create_time,
            last_updated_time,
        ) = row.map_err(sql_to_error)?;
        if !is_supported_historic_variable_type(Some(variable_type.as_str())) {
            continue;
        }
        result.push(HistoricVariableInstance {
            id,
            process_instance_id,
            execution_id,
            task_id,
            name,
            value: parse_variable_value_from_columns(
                Some(variable_type.as_str()),
                text_value,
                alt_text_value,
                long_value,
                double_value,
            )?,
            variable_type,
            create_time: create_time.unwrap_or_else(Utc::now),
            last_updated_time: last_updated_time.unwrap_or_else(Utc::now),
        });
    }

    Ok(result)
}

fn load_bpmn_model_for_process_definition(
    converter: &BpmnXMLConverter,
    deployment_manager: &DeploymentManager,
    process_definition: &ProcessDefinition,
    session: &mut DbSession,
) -> Option<flowable_bpmn_model::model::BpmnModel> {
    let deployment_id = process_definition.deployment_id.as_ref()?;
    let resource_name = process_definition.resource_name.as_ref()?;
    let bytes =
        deployment_manager.get_deployment_resource_bytes(deployment_id, resource_name, session)?;
    let xml = std::str::from_utf8(&bytes).ok()?;
    converter.try_convert_to_bpmn_model(xml).ok()
}

/// P64: import-time timer-start extraction resolves through the same business
/// calendar registry as deploy, so a migrated definition schedules identically
/// to a freshly deployed one.
fn extract_timer_start_subscriptions(
    process_definition: &ProcessDefinition,
    bpmn_model: &flowable_bpmn_model::model::BpmnModel,
    runtime_store: &RuntimeStore,
    calendars: &crate::engine::business_calendar::BusinessCalendarRegistry,
) -> Result<Vec<ProcessTimerStartSubscription>, crate::error::FlowableError> {
    let Some(process) = bpmn_model.main_process.as_ref() else {
        return Ok(Vec::new());
    };
    let now = runtime_store.time_source().now();
    let empty_execution = crate::runtime::execution::Execution::default();
    let mut subscriptions = Vec::new();

    for flow_element in &process.flow_elements {
        if let FlowElementEnum::StartEvent(start_event) = flow_element {
            for event_def in &start_event.event.event_definitions {
                if let EventDefinitionEnum::TimerEventDefinition(timer_def) = event_def {
                    let schedule = crate::bpmn::timer_util::resolve_timer_schedule_for_start(
                        timer_def.time_date.as_ref(),
                        timer_def.time_duration.as_ref(),
                        timer_def.time_cycle.as_ref(),
                        timer_def.end_date.as_ref(),
                        timer_def.calendar_name.as_ref(),
                        &empty_execution,
                        calendars,
                        now,
                    )?;
                    subscriptions.push(ProcessTimerStartSubscription {
                        id: uuid::Uuid::new_v4().to_string(),
                        process_definition_id: process_definition.id.clone(),
                        process_definition_key: process_definition.key.clone(),
                        start_event_id: start_event
                            .event
                            .flow_node
                            .flow_element
                            .base_element
                            .id
                            .clone()
                            .unwrap_or_default(),
                        start_event_name: start_event.event.flow_node.flow_element.name.clone(),
                        interrupting: start_event.interrupting,
                        time_duration: schedule.time_duration,
                        time_date: schedule.time_date,
                        time_cycle: schedule.time_cycle,
                        end_date: schedule.end_date,
                        calendar_name: schedule.calendar_name,
                        due_time: schedule.due_time,
                        lock_owner: None,
                        lock_time: None,
                        category: None,
                    });
                }
            }
        }
    }

    Ok(subscriptions)
}

fn extract_event_start_subscriptions(
    process_definition: &ProcessDefinition,
    bpmn_model: &flowable_bpmn_model::model::BpmnModel,
) -> Vec<ProcessEventStartSubscription> {
    let Some(process) = bpmn_model.main_process.as_ref() else {
        return Vec::new();
    };
    let mut subscriptions = Vec::new();

    for flow_element in &process.flow_elements {
        if let FlowElementEnum::StartEvent(start_event) = flow_element {
            let start_event_id = start_event
                .event
                .flow_node
                .flow_element
                .base_element
                .id
                .clone()
                .unwrap_or_default();
            let start_event_name = start_event.event.flow_node.flow_element.name.clone();
            let extensions = &start_event
                .event
                .flow_node
                .flow_element
                .base_element
                .extension_elements;
            // P93: deploy-time correlation key (CorrelationUtil.java:53-54).
            let configuration = crate::bpmn::event_registry_correlation::correlation_key_from_base_element(
                &start_event.event.flow_node.flow_element.base_element,
                None,
            );

            let mut registered_standard = false;
            for event_def in &start_event.event.event_definitions {
                match event_def {
                    EventDefinitionEnum::MessageEventDefinition(definition) => {
                        if let Some(message_ref) = &definition.message_ref {
                            subscriptions.push(ProcessEventStartSubscription {
                                process_definition_id: process_definition.id.clone(),
                                process_definition_key: process_definition.key.clone(),
                                tenant_id: process_definition.tenant_id.clone(),
                                start_event_id: start_event_id.clone(),
                                start_event_name: start_event_name.clone(),
                                event_kind: EventSubscriptionKind::Message,
                                event_ref: message_ref.clone(),
                                configuration: configuration.clone(),
                            });
                            registered_standard = true;
                        }
                    }
                    EventDefinitionEnum::SignalEventDefinition(definition) => {
                        if let Some(signal_ref) = &definition.signal_ref {
                            subscriptions.push(ProcessEventStartSubscription {
                                process_definition_id: process_definition.id.clone(),
                                process_definition_key: process_definition.key.clone(),
                                tenant_id: process_definition.tenant_id.clone(),
                                start_event_id: start_event_id.clone(),
                                start_event_name: start_event_name.clone(),
                                event_kind: EventSubscriptionKind::Signal,
                                event_ref: signal_ref.clone(),
                                configuration: configuration.clone(),
                            });
                            registered_standard = true;
                        }
                    }
                    _ => {}
                }
            }

            if !registered_standard
                && let Some(event_type) = crate::bpmn::event_registry_correlation::extension_element_text(
                    extensions,
                    crate::bpmn::event_registry_correlation::ELEMENT_EVENT_TYPE,
                )
            {
                if !crate::bpmn::event_registry_correlation::is_manual_subscription(extensions) {
                    subscriptions.push(ProcessEventStartSubscription {
                        process_definition_id: process_definition.id.clone(),
                        process_definition_key: process_definition.key.clone(),
                        tenant_id: process_definition.tenant_id.clone(),
                        start_event_id: start_event_id.clone(),
                        start_event_name: start_event_name.clone(),
                        event_kind: EventSubscriptionKind::Message,
                        event_ref: event_type,
                        configuration,
                    });
                }
            }
        }
    }

    subscriptions
}

fn update_process_definition_version(
    process_definition: &ProcessDefinition,
    session: &mut DbSession,
) -> Result<(), FlowableError> {
    let tenant_id = process_definition.tenant_id.as_deref().unwrap_or("");
    session
        .insert_process_definition_version(
            tenant_id,
            &process_definition.key,
            process_definition.version,
        )
        .map_err(|e| FlowableError::Internal(e.to_string()))?;
    Ok(())
}

fn supported_runtime_variable_types() -> BTreeSet<&'static str> {
    ["string", "long", "integer", "double", "boolean", "json"]
        .into_iter()
        .collect()
}

fn supported_historic_variable_types() -> BTreeSet<&'static str> {
    supported_runtime_variable_types()
}

fn is_supported_runtime_variable_type(variable_type: Option<&str>) -> bool {
    supported_runtime_variable_types().contains(variable_type.unwrap_or("string"))
}

fn is_supported_historic_variable_type(variable_type: Option<&str>) -> bool {
    supported_historic_variable_types().contains(variable_type.unwrap_or("string"))
}

fn parse_variable_value_from_columns(
    variable_type: Option<&str>,
    text_value: Option<String>,
    alt_text_value: Option<String>,
    long_value: Option<i64>,
    double_value: Option<f64>,
) -> Result<serde_json::Value, FlowableError> {
    match variable_type.unwrap_or("string") {
        "string" => Ok(text_value
            .or(alt_text_value)
            .map(serde_json::Value::String)
            .unwrap_or(serde_json::Value::Null)),
        "json" => {
            let raw = text_value
                .or(alt_text_value)
                .unwrap_or_else(|| "null".to_string());
            serde_json::from_str(&raw).map_err(|error| {
                FlowableError::Internal(format!(
                    "failed to parse historical migration environment json variable: {}",
                    error
                ))
            })
        }
        "long" | "integer" => Ok(long_value
            .map(serde_json::Value::from)
            .or_else(|| {
                text_value
                    .and_then(|value| value.parse::<i64>().ok())
                    .map(serde_json::Value::from)
            })
            .unwrap_or(serde_json::Value::Null)),
        "double" => Ok(double_value
            .map(serde_json::Value::from)
            .or_else(|| {
                text_value
                    .and_then(|value| value.parse::<f64>().ok())
                    .map(serde_json::Value::from)
            })
            .unwrap_or(serde_json::Value::Null)),
        "boolean" => Ok(parse_boolean_value(long_value, text_value.as_deref())
            .map(serde_json::Value::from)
            .unwrap_or(serde_json::Value::Null)),
        unsupported => Err(FlowableError::Internal(format!(
            "unsupported historical migration environment variable type: {}",
            unsupported
        ))),
    }
}

fn parse_boolean_value(long_value: Option<i64>, text_value: Option<&str>) -> Option<bool> {
    long_value.map(|value| value != 0).or(match text_value {
        Some("true") | Some("TRUE") | Some("1") => Some(true),
        Some("false") | Some("FALSE") | Some("0") => Some(false),
        _ => None,
    })
}

fn map_event_subscription_kind(event_type: &str) -> Option<EventSubscriptionKind> {
    match event_type {
        "message" => Some(EventSubscriptionKind::Message),
        "signal" => Some(EventSubscriptionKind::Signal),
        "conditional" => Some(EventSubscriptionKind::Conditional),
        _ => None,
    }
}

fn optional_bool(row: &Row<'_>, index: usize) -> Option<bool> {
    match row.get_ref(index) {
        Ok(ValueRef::Integer(value)) => Some(value != 0),
        Ok(ValueRef::Text(value)) => match std::str::from_utf8(value).ok()? {
            "true" | "TRUE" | "1" => Some(true),
            "false" | "FALSE" | "0" => Some(false),
            _ => None,
        },
        _ => None,
    }
}

fn optional_timestamp_millis(row: &Row<'_>, index: usize) -> Option<i64> {
    match row.get_ref(index) {
        Ok(ValueRef::Integer(value)) => Some(value),
        Ok(ValueRef::Text(value)) => {
            let text = std::str::from_utf8(value).ok()?;
            parse_datetime_text(text).map(|date_time| date_time.timestamp_millis())
        }
        _ => None,
    }
}

fn optional_datetime(row: &Row<'_>, index: usize) -> Option<DateTime<Utc>> {
    match row.get_ref(index) {
        Ok(ValueRef::Integer(value)) => Utc.timestamp_millis_opt(value).single(),
        Ok(ValueRef::Text(value)) => {
            let text = std::str::from_utf8(value).ok()?;
            parse_datetime_text(text)
        }
        _ => None,
    }
}

fn parse_datetime_text(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .map(|date_time| date_time.with_timezone(&Utc))
        .ok()
        .or_else(|| {
            chrono::NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S")
                .ok()
                .map(|date_time| Utc.from_utc_datetime(&date_time))
        })
        .or_else(|| {
            value
                .parse::<i64>()
                .ok()
                .and_then(|millis| Utc.timestamp_millis_opt(millis).single())
        })
}

fn sql_to_error(error: rusqlite::Error) -> FlowableError {
    FlowableError::Internal(format!(
        "historical migration environment migration failure: {}",
        error
    ))
}
