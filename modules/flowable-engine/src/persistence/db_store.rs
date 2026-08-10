use crate::persistence::db_session::{DbParams, DbSession};
use crate::persistence::storage_error::StorageError;
use flowable_persistence::{
    DatabaseConfig, DatabaseKind, PersistenceError, SchemaMode, adapters::create_session_factory,
    adapters::rusqlite_pool::create_sqlite_session_factory,
    statement_catalog::FlowableStatementCatalog,
};
use std::sync::Arc;
use uuid::Uuid;

pub struct DbStore {
    session_factory: Arc<flowable_persistence::DbSessionFactory>,
}

fn persistence_to_rusqlite_err(e: PersistenceError) -> rusqlite::Error {
    rusqlite::Error::ToSqlConversionFailure(Box::new(std::io::Error::other(format!("{}", e))))
}

fn storage_to_rusqlite_err(e: StorageError) -> rusqlite::Error {
    rusqlite::Error::ToSqlConversionFailure(Box::new(std::io::Error::other(format!("{}", e))))
}

fn legacy_column_exists(
    session: &mut DbSession,
    table: &str,
    column: &str,
) -> Result<bool, StorageError> {
    // The in-memory backend keeps no catalog; legacy columns are assumed present.
    if matches!(session.dialect().database_kind(), DatabaseKind::Memory) {
        return Ok(true);
    }
    Ok(session
        .table_columns(table)?
        .iter()
        .any(|info| info.name == column))
}

/// Test-only re-export of the bootstrap migration used when a legacy
/// `timer_job_states` table is upgraded in place.
#[doc(hidden)]
pub fn ensure_legacy_tables_for_test(session: &mut DbSession) -> Result<(), StorageError> {
    ensure_legacy_tables(session)
}

fn ensure_legacy_tables(session: &mut DbSession) -> Result<(), StorageError> {
    let blob = session.dialect().blob_type();
    let big = session.dialect().bigint_type();
    // MySQL forbids TEXT/BLOB in key specs without a length; use VARCHAR for PK/index cols.
    // utf8mb4: 255 chars * 4 bytes = 1020; keep composite PKs under MySQL's 3072 limit.
    let id = session.dialect().varchar_type(255);
    let name = session.dialect().varchar_type(255);
    let text = session.dialect().text_type();
    let key_col = session.dialect().quote_identifier("key");

    let ddl_statements = vec![
        format!(
            "CREATE TABLE IF NOT EXISTS deployments (id {id} PRIMARY KEY, data {text} NOT NULL)"
        ),
        format!(
            "CREATE TABLE IF NOT EXISTS deployment_resources (deployment_id {id}, name {name}, resource_type {name}, content_type {name}, bytes {blob}, created_at {big}, PRIMARY KEY (deployment_id, name))"
        ),
        format!(
            "CREATE TABLE IF NOT EXISTS process_definitions (id {id} PRIMARY KEY, deployment_id {id}, {key_col} {name}, tenant_id {name}, data {text} NOT NULL)"
        ),
        format!(
            "CREATE TABLE IF NOT EXISTS process_definition_versions (tenant_id {name}, process_key {name}, version INTEGER, PRIMARY KEY (tenant_id, process_key))"
        ),
        format!(
            "CREATE TABLE IF NOT EXISTS repository_models (id {id} PRIMARY KEY, deployment_id {id}, model_key {name}, tenant_id {name}, source_bytes {blob} NOT NULL, source_extra_bytes {blob} NOT NULL, data {text} NOT NULL)"
        ),
        format!(
            "CREATE TABLE IF NOT EXISTS executions (id {id} PRIMARY KEY, process_instance_id {id}, data {text} NOT NULL)"
        ),
        format!(
            "CREATE TABLE IF NOT EXISTS process_instances (id {id} PRIMARY KEY, process_definition_id {id}, data {text} NOT NULL)"
        ),
        format!(
            "CREATE TABLE IF NOT EXISTS process_instance_locks (id {id} PRIMARY KEY, lock_owner {name}, lock_time {big}, data {text} NOT NULL)"
        ),
        format!(
            "CREATE TABLE IF NOT EXISTS event_registry_deployments (id {id} PRIMARY KEY, name {name}, deployed_at {big}, data {text} NOT NULL)"
        ),
        format!(
            "CREATE TABLE IF NOT EXISTS event_registry_channel_definitions (id {id} PRIMARY KEY, deployment_id {id}, {key_col} {name}, name {name}, channel_type {name}, resource_name {name}, data {text} NOT NULL)"
        ),
        format!(
            "CREATE TABLE IF NOT EXISTS event_registry_event_definitions (id {id} PRIMARY KEY, deployment_id {id}, {key_col} {name}, name {name}, event_type {name}, channel_key {name}, resource_name {name}, data {text} NOT NULL)"
        ),
        format!(
            "CREATE TABLE IF NOT EXISTS event_registry_event_instance_deliveries (id {id} PRIMARY KEY, event_definition_key {name}, event_type {name}, channel_key {name}, direction {name}, status {name}, created_at {big}, updated_at {big}, data {text} NOT NULL)"
        ),
        format!(
            "CREATE TABLE IF NOT EXISTS event_registry_change_records (id {id} PRIMARY KEY, revision {big}, change_type {name}, entity_type {name}, entity_key {name}, data {text} NOT NULL)"
        ),
        format!(
            "CREATE TABLE IF NOT EXISTS event_registry_change_revision_seq (id {id} PRIMARY KEY, next_revision {big} NOT NULL)"
        ),
        format!(
            "CREATE TABLE IF NOT EXISTS form_deployments (id {id} PRIMARY KEY, name {name}, deployed_at {big}, data {text} NOT NULL)"
        ),
        format!(
            "CREATE TABLE IF NOT EXISTS form_definitions (id {id} PRIMARY KEY, deployment_id {id}, {key_col} {name}, name {name}, version INTEGER, resource_name {name}, data {text} NOT NULL)"
        ),
        format!(
            "CREATE TABLE IF NOT EXISTS content_items (id {id} PRIMARY KEY, name {name}, mime_type {name}, created_at {big}, data {text} NOT NULL)"
        ),
        format!(
            "CREATE TABLE IF NOT EXISTS http_task_records (id {id} PRIMARY KEY, process_instance_id {id}, execution_id {id}, activity_id {name}, method {name}, url {text}, status {name}, created_at {big}, data {text} NOT NULL)"
        ),
        format!(
            "CREATE TABLE IF NOT EXISTS mail_outbox_records (id {id} PRIMARY KEY, process_instance_id {id}, execution_id {id}, activity_id {name}, recipient {name}, subject {text}, status {name}, created_at {big}, data {text} NOT NULL)"
        ),
        format!(
            "CREATE TABLE IF NOT EXISTS tasks (id {id} PRIMARY KEY, process_instance_id {id}, execution_id {id}, task_definition_key {name}, name {name}, assignee {name}, owner {name}, parent_task_id {id}, priority INTEGER, due_date {big}, data {text} NOT NULL)"
        ),
        format!(
            "CREATE TABLE IF NOT EXISTS variables (id {id} PRIMARY KEY, execution_id {id}, process_instance_id {id}, name {name}, data {text} NOT NULL)"
        ),
        format!(
            "CREATE TABLE IF NOT EXISTS event_subscriptions (id {id} PRIMARY KEY, execution_id {id}, process_instance_id {id}, event_name {name}, event_kind {name}, data {text} NOT NULL)"
        ),
        format!(
            "CREATE TABLE IF NOT EXISTS event_wait_states (id {id} PRIMARY KEY, process_instance_id {id}, data {text} NOT NULL)"
        ),
        format!(
            "CREATE TABLE IF NOT EXISTS boundary_event_states (id {id} PRIMARY KEY, process_instance_id {id}, host_execution_id {id}, data {text} NOT NULL)"
        ),
        format!(
            "CREATE TABLE IF NOT EXISTS timer_job_states (id {id} PRIMARY KEY, process_instance_id {id}, execution_id {id}, activity_id {name}, lock_owner {name}, lock_time {big}, lock_expiration_time {big}, retries INTEGER, error_message {text}, error_details {text}, due_time {big}, job_state {name}, job_type {name}, create_time {big}, correlation_id {name}, handler_type {name}, tenant_id {name}, process_definition_id {id}, element_name {name}, category {name}, scope_id {id}, sub_scope_id {id}, scope_type {name}, scope_definition_id {id}, data {text} NOT NULL)"
        ),
        format!(
            "CREATE TABLE IF NOT EXISTS process_timer_start_subscriptions (id {id} PRIMARY KEY, process_definition_id {id}, lock_owner {name}, lock_time {big}, data {text} NOT NULL)"
        ),
        format!(
            "CREATE TABLE IF NOT EXISTS process_event_start_subscriptions (id {id} PRIMARY KEY, process_definition_id {id}, event_kind {name}, event_ref {name}, data {text} NOT NULL)"
        ),
        format!(
            "CREATE TABLE IF NOT EXISTS event_subprocess_timer_subscriptions (id {id} PRIMARY KEY, process_instance_id {id}, lock_owner {name}, lock_time {big}, data {text} NOT NULL)"
        ),
        format!(
            "CREATE TABLE IF NOT EXISTS event_subprocess_event_subscriptions (id {id} PRIMARY KEY, process_instance_id {id}, event_kind {name}, event_ref {name}, data {text} NOT NULL)"
        ),
        format!(
            "CREATE TABLE IF NOT EXISTS timer_worker_nodes (id {id} PRIMARY KEY, last_heartbeat {big}, data {text} NOT NULL)"
        ),
        format!(
            "CREATE TABLE IF NOT EXISTS timer_coordinator_leases (id {id} PRIMARY KEY, owner_node_id {name}, expiry_time {big}, fencing_token {big}, data {text} NOT NULL)"
        ),
        format!(
            "CREATE TABLE IF NOT EXISTS timer_admin_audit_logs (id {id} PRIMARY KEY, request_id {name}, timestamp {big}, tenant_id {name}, issuer {name}, subject {name}, actor {name}, action {name}, target {name}, outcome {name}, profile_id {name}, data {text} NOT NULL)"
        ),
        format!(
            "CREATE TABLE IF NOT EXISTS token_revocations (id {id} PRIMARY KEY, issuer {name}, reason {text}, expires_at {big}, created_at {big}, data {text} NOT NULL)"
        ),
        format!(
            "CREATE TABLE IF NOT EXISTS timer_issuer_profiles (id {id} PRIMARY KEY, issuer {name}, version INTEGER, data {text} NOT NULL)"
        ),
        format!("CREATE TABLE IF NOT EXISTS users (id {id} PRIMARY KEY, data {text} NOT NULL)"),
        format!(
            "CREATE TABLE IF NOT EXISTS user_info (id {id} PRIMARY KEY, user_id {id} NOT NULL, info_key {name} NOT NULL, data {text} NOT NULL, UNIQUE (user_id, info_key))"
        ),
        format!(
            "CREATE TABLE IF NOT EXISTS user_pictures (id {id} PRIMARY KEY, user_id {id} NOT NULL UNIQUE, mime_type {name} NOT NULL, bytes {blob} NOT NULL, data {text} NOT NULL DEFAULT '')"
        ),
        format!(
            "CREATE TABLE IF NOT EXISTS {} (id {id} PRIMARY KEY, data {text} NOT NULL)",
            session.dialect().quote_identifier("groups")
        ),
        format!(
            "CREATE TABLE IF NOT EXISTS memberships (id {id} PRIMARY KEY, user_id {id}, group_id {id}, data {text} NOT NULL, UNIQUE (user_id, group_id))"
        ),
        format!(
            "CREATE TABLE IF NOT EXISTS privileges (id {id} PRIMARY KEY, name {name}, data {text} NOT NULL)"
        ),
        format!(
            "CREATE TABLE IF NOT EXISTS privilege_mappings (id {id} PRIMARY KEY, privilege_id {id}, user_id {id}, group_id {id}, data {text} NOT NULL)"
        ),
        format!(
            "CREATE TABLE IF NOT EXISTS tokens (id {id} PRIMARY KEY, token_value {name}, data {text} NOT NULL)"
        ),
        format!(
            "CREATE TABLE IF NOT EXISTS identity_links (id {id} PRIMARY KEY, link_type {name}, task_id {id}, process_instance_id {id}, process_definition_id {id}, user_id {id}, group_id {id}, data {text} NOT NULL)"
        ),
        // P77: historic identity links — Java ACT_HI_IDENTITYLINK
        // (flowable.postgres.all.create.sql:95-108).
        format!(
            "CREATE TABLE IF NOT EXISTS historic_identity_links (id {id} PRIMARY KEY, link_type {name}, task_id {id}, process_instance_id {id}, user_id {id}, group_id {id}, scope_id {id}, sub_scope_id {id}, scope_type {name}, scope_definition_id {id}, create_time {big}, data {text} NOT NULL)"
        ),
        format!(
            "CREATE TABLE IF NOT EXISTS entity_links (id {id} PRIMARY KEY, link_type {name}, scope_id {id}, scope_type {name}, reference_scope_id {id}, reference_scope_type {name}, data {text} NOT NULL)"
        ),
        format!(
            "CREATE TABLE IF NOT EXISTS batch_entities (id {id} PRIMARY KEY, batch_type {name}, status {name}, create_time {big}, tenant_id {name}, data {text} NOT NULL)"
        ),
        format!(
            "CREATE TABLE IF NOT EXISTS batch_part_entities (id {id} PRIMARY KEY, batch_id {id}, batch_type {name}, status {name}, create_time {big}, data {text} NOT NULL)"
        ),
        format!(
            "CREATE TABLE IF NOT EXISTS historic_process_instances (id {id} PRIMARY KEY, process_definition_id {id}, start_time_ms {big}, end_time_ms {big}, data {text} NOT NULL)"
        ),
        format!(
            "CREATE TABLE IF NOT EXISTS historic_activity_instances (id {id} PRIMARY KEY, process_instance_id {id}, execution_id {id}, activity_id {name}, delete_reason {name}, data {text} NOT NULL)"
        ),
        format!(
            "CREATE TABLE IF NOT EXISTS historic_task_instances (id {id} PRIMARY KEY, process_instance_id {id}, process_definition_id {id}, task_definition_key {name}, assignee {name}, owner {name}, claim_time {big}, tenant_id {name}, category {name}, form_key {name}, parent_task_id {id}, priority INTEGER, due_date {big}, data {text} NOT NULL)"
        ),
        format!(
            "CREATE TABLE IF NOT EXISTS historic_variable_instances (id {id} PRIMARY KEY, process_instance_id {id}, variable_name {name}, data {text} NOT NULL)"
        ),
        format!(
            "CREATE TABLE IF NOT EXISTS historic_details (id {id} PRIMARY KEY, process_instance_id {id}, execution_id {id}, task_id {id}, time {big}, detail_type {name}, variable_name {name}, property_id {name}, data {text} NOT NULL)"
        ),
        format!(
            "CREATE TABLE IF NOT EXISTS historic_audit_logs (id {id} PRIMARY KEY, data {text} NOT NULL)"
        ),
        format!(
            "CREATE TABLE IF NOT EXISTS historic_comments (id {id} PRIMARY KEY, task_id {id}, process_instance_id {id}, time {big}, comment_type {name}, data {text} NOT NULL)"
        ),
        format!(
            "CREATE TABLE IF NOT EXISTS historic_task_events (id {id} PRIMARY KEY, task_id {id}, time {big}, data {text} NOT NULL)"
        ),
        format!(
            "CREATE TABLE IF NOT EXISTS historic_task_log_entries (id {id} PRIMARY KEY, log_number INTEGER, task_id {id}, log_type {name}, process_instance_id {id}, process_definition_id {id}, timestamp {big}, data {text} NOT NULL)"
        ),
        format!(
            "CREATE TABLE IF NOT EXISTS cleanup_logs (id {id} PRIMARY KEY, data {text} NOT NULL)"
        ),
        format!(
            "CREATE TABLE IF NOT EXISTS cleanup_strategy_configs (id {id} PRIMARY KEY, data {text} NOT NULL)"
        ),
        format!(
            "CREATE TABLE IF NOT EXISTS compensation_subscriptions (id {id} PRIMARY KEY, process_instance_id {id}, execution_id {id}, activity_id {name}, compensation_activity_id {name}, subscription_order INTEGER, data {text} NOT NULL)"
        ),
    ];

    // Create legacy JSON-compat tables first.
    for (i, sql) in ddl_statements.iter().enumerate() {
        if let Err(e) = session.execute_raw_sql(sql) {
            let preview: String = sql.chars().take(120).collect();
            return Err(StorageError::Persistence(format!(
                "DDL statement {} failed: {} | SQL: {}",
                i, e, preview
            )));
        }
    }

    if !legacy_column_exists(session, "timer_job_states", "job_type")? {
        session.execute_raw_sql(&format!(
            "ALTER TABLE timer_job_states ADD COLUMN job_type {name}"
        ))?;
    }
    // P2-11: SQL pushdown filters on projected query columns whose values
    // legacy rows carry only inside the JSON payload. Track which projection
    // columns this bootstrap actually introduces so the one-time backfill
    // below hydrates every newly added column (not just activity_id); columns
    // that already existed keep their physical values untouched.
    let mut backfill_columns: Vec<&str> = Vec::new();
    for (column, sql_type) in [
        ("create_time", big),
        ("correlation_id", name.as_str()),
        ("handler_type", name.as_str()),
        ("tenant_id", name.as_str()),
        ("process_definition_id", id.as_str()),
        ("element_name", name.as_str()),
        ("category", name.as_str()),
        ("scope_id", id.as_str()),
        ("sub_scope_id", id.as_str()),
        ("scope_type", name.as_str()),
        ("scope_definition_id", id.as_str()),
        ("activity_id", name.as_str()),
    ] {
        if !legacy_column_exists(session, "timer_job_states", column)? {
            session.execute_raw_sql(&format!(
                "ALTER TABLE timer_job_states ADD COLUMN {column} {sql_type}"
            ))?;
            backfill_columns.push(column);
        }
    }
    if !backfill_columns.is_empty() {
        let rows = session.raw_query("SELECT id, data FROM timer_job_states", DbParams::new())?;
        for row in rows {
            let (Some(job_id), Some(json)) = (row.get_text("id"), row.get_text("data")) else {
                continue;
            };
            let Ok(state) = serde_json::from_str::<serde_json::Value>(&json) else {
                continue;
            };
            for column in &backfill_columns {
                let Some(field) = state.get(*column) else {
                    continue;
                };
                // Guarded by `IS NULL` so a physical value always wins over
                // the JSON copy; newly added columns are NULL on legacy rows.
                let mut params = DbParams::new();
                if let Some(text) = field.as_str() {
                    params.push(text);
                } else if let Some(number) = field.as_i64() {
                    params.push(number);
                } else {
                    continue;
                }
                params.push(job_id.as_str());
                session.execute_raw(
                    &format!(
                        "UPDATE timer_job_states SET {column} = ? WHERE id = ? AND {column} IS NULL"
                    ),
                    params,
                )?;
            }
        }
    }
    for (column, sql_type) in [
        ("claim_time", big),
        ("tenant_id", name.as_str()),
        ("category", name.as_str()),
        ("form_key", name.as_str()),
        ("parent_task_id", id.as_str()),
    ] {
        if !legacy_column_exists(session, "historic_task_instances", column)? {
            session.execute_raw_sql(&format!(
                "ALTER TABLE historic_task_instances ADD COLUMN {column} {sql_type}"
            ))?;
        }
    }
    // P65-comment-type: project Comment.type for indexed type queries. Existing
    // DBs created before this column need an ALTER; fresh CREATE includes it.
    if !legacy_column_exists(session, "historic_comments", "comment_type")? {
        session.execute_raw_sql(&format!(
            "ALTER TABLE historic_comments ADD COLUMN comment_type {name}"
        ))?;
    }
    // P71: project HistoricActivityInstance.deleteReason for SQL filtering.
    if !legacy_column_exists(session, "historic_activity_instances", "delete_reason")? {
        session.execute_raw_sql(&format!(
            "ALTER TABLE historic_activity_instances ADD COLUMN delete_reason {name}"
        ))?;
    }

    // Correctness-critical event-registry schema must exist on every dialect,
    // including MySQL which skips the secondary indexes below: the unique
    // revision index is what lets the single-revision poll cursor trust the
    // change log, and the allocator seed prevents two instances from racing
    // a missing-row reseed on their very first allocation.
    let unique_revision_sql = session
        .dialect()
        .create_index_if_not_exists(
            "idx_event_registry_change_revision_unique",
            "event_registry_change_records",
            "revision",
        )
        .replacen("CREATE INDEX", "CREATE UNIQUE INDEX", 1);
    let is_mysql = matches!(
        session.dialect().database_kind(),
        flowable_persistence::DatabaseKind::Mysql
    );
    if let Err(e) = session.execute_raw_sql(&unique_revision_sql) {
        let msg = e.to_string();
        if !(msg.contains("1061")
            || msg.contains("Duplicate key name")
            || msg.contains("already exists"))
        {
            return Err(StorageError::Persistence(format!(
                "event registry change revision unique index failed: {msg}"
            )));
        }
    }

    // Seed the event-registry revision allocator from the change-log high
    // water mark so databases that predate the allocator keep monotonic
    // revisions. The NOT EXISTS guard makes reruns no-ops; a concurrent
    // bootstrap losing the insert race is tolerated like duplicate indexes.
    let seed_sql = "INSERT INTO event_registry_change_revision_seq (id, next_revision) \
                    SELECT * FROM (SELECT 'event-registry' AS id, COALESCE(MAX(revision), 0) AS next_revision \
                    FROM event_registry_change_records) AS seed \
                    WHERE NOT EXISTS (SELECT 1 FROM event_registry_change_revision_seq WHERE id = 'event-registry')";
    if let Err(e) = session.execute_raw_sql(seed_sql) {
        let msg = e.to_string();
        if !msg.contains("Duplicate") && !msg.to_lowercase().contains("unique") {
            return Err(StorageError::Persistence(format!(
                "event registry revision allocator seed failed: {msg}"
            )));
        }
    }

    // MySQL: skip secondary indexes during bootstrap. Creating ~90 indexes over a
    // WSL-forwarded connection is fragile (EOF / pool timeouts). Indexes can be
    // added later via versioned schema scripts.
    if is_mysql {
        session.flush_and_commit()?;
        return Ok(());
    }
    let dialect = session.dialect();
    let index_defs: Vec<(&str, &str, &str)> = vec![
        (
            "idx_historic_comments_task_id",
            "historic_comments",
            "task_id",
        ),
        (
            "idx_historic_comments_process_instance_id",
            "historic_comments",
            "process_instance_id",
        ),
        ("idx_historic_comments_time", "historic_comments", "time"),
        (
            "idx_historic_comments_comment_type",
            "historic_comments",
            "comment_type",
        ),
        (
            "idx_historic_task_events_task_id",
            "historic_task_events",
            "task_id",
        ),
        (
            "idx_historic_task_events_time",
            "historic_task_events",
            "time",
        ),
        (
            "idx_historic_task_log_entries_log_number",
            "historic_task_log_entries",
            "log_number",
        ),
        (
            "idx_historic_task_log_entries_task_id",
            "historic_task_log_entries",
            "task_id",
        ),
        (
            "idx_historic_task_log_entries_type",
            "historic_task_log_entries",
            "log_type",
        ),
        (
            "idx_historic_task_log_entries_process_instance_id",
            "historic_task_log_entries",
            "process_instance_id",
        ),
        (
            "idx_historic_task_log_entries_process_definition_id",
            "historic_task_log_entries",
            "process_definition_id",
        ),
        (
            "idx_historic_task_log_entries_timestamp",
            "historic_task_log_entries",
            "timestamp",
        ),
        (
            "idx_timer_admin_audit_logs_request_id",
            "timer_admin_audit_logs",
            "request_id",
        ),
        (
            "idx_timer_admin_audit_logs_tenant_id",
            "timer_admin_audit_logs",
            "tenant_id",
        ),
        (
            "idx_timer_admin_audit_logs_actor",
            "timer_admin_audit_logs",
            "actor",
        ),
        (
            "idx_timer_admin_audit_logs_action",
            "timer_admin_audit_logs",
            "action",
        ),
        (
            "idx_token_revocations_expires_at",
            "token_revocations",
            "expires_at",
        ),
        (
            "idx_proc_inst_def",
            "process_instances",
            "process_definition_id",
        ),
        (
            "idx_repository_models_key",
            "repository_models",
            "model_key",
        ),
        (
            "idx_repository_models_deployment_id",
            "repository_models",
            "deployment_id",
        ),
        (
            "idx_repository_models_tenant_id",
            "repository_models",
            "tenant_id",
        ),
        (
            "idx_event_registry_channel_key",
            "event_registry_channel_definitions",
            "key",
        ),
        (
            "idx_event_registry_channel_type",
            "event_registry_channel_definitions",
            "channel_type",
        ),
        (
            "idx_event_registry_event_key",
            "event_registry_event_definitions",
            "key",
        ),
        (
            "idx_event_registry_event_type",
            "event_registry_event_definitions",
            "event_type",
        ),
        (
            "idx_event_registry_event_channel_key",
            "event_registry_event_definitions",
            "channel_key",
        ),
        (
            "idx_event_registry_delivery_direction",
            "event_registry_event_instance_deliveries",
            "direction",
        ),
        (
            "idx_event_registry_delivery_status",
            "event_registry_event_instance_deliveries",
            "status",
        ),
        (
            "idx_event_registry_delivery_event_type",
            "event_registry_event_instance_deliveries",
            "event_type",
        ),
        (
            "idx_event_registry_delivery_channel_key",
            "event_registry_event_instance_deliveries",
            "channel_key",
        ),
        (
            "idx_event_registry_change_revision",
            "event_registry_change_records",
            "revision",
        ),
        (
            "idx_event_registry_change_entity_key",
            "event_registry_change_records",
            "entity_key",
        ),
        ("idx_form_definitions_key", "form_definitions", "key"),
        (
            "idx_form_definitions_deployment_id",
            "form_definitions",
            "deployment_id",
        ),
        ("idx_content_items_name", "content_items", "name"),
        (
            "idx_content_items_created_at",
            "content_items",
            "created_at",
        ),
        (
            "idx_http_task_records_proc_inst",
            "http_task_records",
            "process_instance_id",
        ),
        (
            "idx_http_task_records_exec",
            "http_task_records",
            "execution_id",
        ),
        (
            "idx_http_task_records_activity_id",
            "http_task_records",
            "activity_id",
        ),
        (
            "idx_mail_outbox_records_proc_inst",
            "mail_outbox_records",
            "process_instance_id",
        ),
        (
            "idx_mail_outbox_records_exec",
            "mail_outbox_records",
            "execution_id",
        ),
        (
            "idx_mail_outbox_records_activity_id",
            "mail_outbox_records",
            "activity_id",
        ),
        ("idx_tasks_proc_inst", "tasks", "process_instance_id"),
        ("idx_tasks_exec", "tasks", "execution_id"),
        (
            "idx_tasks_task_definition_key",
            "tasks",
            "task_definition_key",
        ),
        ("idx_tasks_assignee", "tasks", "assignee"),
        ("idx_tasks_owner", "tasks", "owner"),
        ("idx_tasks_priority", "tasks", "priority"),
        ("idx_tasks_due_date", "tasks", "due_date"),
        ("idx_tasks_parent", "tasks", "parent_task_id"),
        (
            "idx_historic_task_instances_process_definition_id",
            "historic_task_instances",
            "process_definition_id",
        ),
        (
            "idx_historic_task_instances_task_definition_key",
            "historic_task_instances",
            "task_definition_key",
        ),
        (
            "idx_historic_task_instances_assignee",
            "historic_task_instances",
            "assignee",
        ),
        (
            "idx_historic_task_instances_owner",
            "historic_task_instances",
            "owner",
        ),
        (
            "idx_historic_task_instances_priority",
            "historic_task_instances",
            "priority",
        ),
        (
            "idx_historic_task_instances_due_date",
            "historic_task_instances",
            "due_date",
        ),
        ("idx_variables_exec", "variables", "execution_id"),
        (
            "idx_variables_proc_inst",
            "variables",
            "process_instance_id",
        ),
        ("idx_event_sub_name", "event_subscriptions", "event_name"),
        ("idx_event_sub_exec", "event_subscriptions", "execution_id"),
        (
            "idx_hist_proc_inst_id",
            "historic_activity_instances",
            "execution_id",
        ),
        ("idx_user_info_user_id", "user_info", "user_id"),
        ("idx_privileges_name", "privileges", "name"),
        (
            "idx_privilege_mappings_privilege_id",
            "privilege_mappings",
            "privilege_id",
        ),
        (
            "idx_privilege_mappings_user_id",
            "privilege_mappings",
            "user_id",
        ),
        (
            "idx_privilege_mappings_group_id",
            "privilege_mappings",
            "group_id",
        ),
        ("idx_tokens_value", "tokens", "token_value"),
        ("idx_identity_links_task_id", "identity_links", "task_id"),
        (
            "idx_identity_links_proc_inst",
            "identity_links",
            "process_instance_id",
        ),
        (
            "idx_identity_links_proc_def",
            "identity_links",
            "process_definition_id",
        ),
        ("idx_identity_links_user_id", "identity_links", "user_id"),
        ("idx_identity_links_group_id", "identity_links", "group_id"),
        (
            "idx_historic_identity_links_task_id",
            "historic_identity_links",
            "task_id",
        ),
        (
            "idx_historic_identity_links_proc_inst",
            "historic_identity_links",
            "process_instance_id",
        ),
        (
            "idx_historic_identity_links_user_id",
            "historic_identity_links",
            "user_id",
        ),
        (
            "idx_historic_identity_links_scope",
            "historic_identity_links",
            "scope_id, scope_type",
        ),
        ("idx_entity_links_scope", "entity_links", "scope_id"),
        (
            "idx_entity_links_ref_scope",
            "entity_links",
            "reference_scope_id",
        ),
        ("idx_batch_status", "batch_entities", "status"),
        ("idx_batch_tenant_id", "batch_entities", "tenant_id"),
        ("idx_batch_part_batch_id", "batch_part_entities", "batch_id"),
        ("idx_batch_part_status", "batch_part_entities", "status"),
        (
            "idx_compensation_subscriptions_proc_order",
            "compensation_subscriptions",
            "process_instance_id, subscription_order",
        ),
        (
            "idx_historic_details_process_instance_id",
            "historic_details",
            "process_instance_id",
        ),
        (
            "idx_historic_details_execution_id",
            "historic_details",
            "execution_id",
        ),
        (
            "idx_historic_details_task_id",
            "historic_details",
            "task_id",
        ),
        (
            "idx_historic_details_detail_type",
            "historic_details",
            "detail_type",
        ),
        (
            "idx_historic_details_variable_name",
            "historic_details",
            "variable_name",
        ),
        (
            "idx_historic_details_property_id",
            "historic_details",
            "property_id",
        ),
        ("idx_historic_details_time", "historic_details", "time"),
        ("idx_timer_job_due_time", "timer_job_states", "due_time"),
        ("idx_timer_job_state", "timer_job_states", "job_state"),
        ("idx_timer_job_type", "timer_job_states", "job_type"),
        ("idx_timer_job_lock_owner", "timer_job_states", "lock_owner"),
        (
            "idx_timer_job_lock_expiration",
            "timer_job_states",
            "lock_expiration_time",
        ),
        ("idx_timer_job_category", "timer_job_states", "category"),
        (
            "idx_timer_job_scope",
            "timer_job_states",
            "scope_id, scope_type, sub_scope_id",
        ),
        (
            "idx_timer_job_scope_definition",
            "timer_job_states",
            "scope_definition_id",
        ),
        (
            "idx_timer_job_correlation",
            "timer_job_states",
            "correlation_id",
        ),
        (
            "idx_timer_job_handler_type",
            "timer_job_states",
            "handler_type",
        ),
        (
            "idx_historic_pi_start_time_ms",
            "historic_process_instances",
            "start_time_ms",
        ),
    ];
    let index_sqls: Vec<String> = index_defs
        .iter()
        .map(|(idx, table, cols)| {
            let cols = if *cols == "key" {
                dialect.quote_identifier("key")
            } else {
                (*cols).to_string()
            };
            dialect.create_index_if_not_exists(idx, table, &cols)
        })
        .collect();

    for (i, sql) in index_sqls.iter().enumerate() {
        if let Err(e) = session.execute_raw_sql(sql) {
            let msg = e.to_string();
            // Index creation is best-effort — never block engine bootstrap.
            if msg.contains("1061")
                || msg.contains("Duplicate key name")
                || msg.contains("already exists")
            {
                continue;
            }
            let preview: String = sql.chars().take(120).collect();
            return Err(StorageError::Persistence(format!(
                "Index DDL statement {} failed: {} | SQL: {}",
                i, e, preview
            )));
        }
    }

    session.flush_and_commit()?;
    Ok(())
}

impl DbStore {
    pub fn new_in_memory() -> rusqlite::Result<Self> {
        Self::new_in_memory_with_size(8)
    }

    pub fn new_in_memory_with_size(pool_size: u32) -> rusqlite::Result<Self> {
        let uri = format!("file:flowable_{}?mode=memory&cache=shared", Uuid::new_v4());
        let config = DatabaseConfig {
            kind: DatabaseKind::Memory,
            url: uri,
            pool_size,
            schema_mode: SchemaMode::True,
            table_prefix: None,
            schema: None,
            catalog: None,
        };
        let catalog = Arc::new(FlowableStatementCatalog::new(Box::new(
            flowable_persistence::SqliteDialect,
        )));
        let factory =
            create_sqlite_session_factory(&config, catalog).map_err(persistence_to_rusqlite_err)?;
        let store = Self {
            session_factory: Arc::new(factory),
        };
        let mut session = store.create_session().map_err(storage_to_rusqlite_err)?;
        ensure_legacy_tables(&mut session).map_err(storage_to_rusqlite_err)?;
        Ok(store)
    }

    pub fn new_file(path: &str) -> rusqlite::Result<Self> {
        Self::new_file_with_size(path, 8)
    }

    pub fn new_file_with_size(path: &str, pool_size: u32) -> rusqlite::Result<Self> {
        let config = DatabaseConfig {
            kind: DatabaseKind::Sqlite,
            url: path.to_string(),
            pool_size,
            schema_mode: SchemaMode::True,
            table_prefix: None,
            schema: None,
            catalog: None,
        };
        let catalog = Arc::new(FlowableStatementCatalog::new(Box::new(
            flowable_persistence::SqliteDialect,
        )));
        let factory =
            create_sqlite_session_factory(&config, catalog).map_err(persistence_to_rusqlite_err)?;
        let store = Self {
            session_factory: Arc::new(factory),
        };
        let mut session = store.create_session().map_err(storage_to_rusqlite_err)?;
        ensure_legacy_tables(&mut session).map_err(storage_to_rusqlite_err)?;
        Ok(store)
    }

    pub fn from_config(config: DatabaseConfig) -> Result<Self, PersistenceError> {
        let factory = create_session_factory(&config)?;
        let store = Self {
            session_factory: Arc::new(factory),
        };
        let mut session = store
            .create_session()
            .map_err(|e| PersistenceError::Schema(e.to_string()))?;
        ensure_legacy_tables(&mut session).map_err(|e| PersistenceError::Schema(e.to_string()))?;
        Ok(store)
    }

    pub fn create_session(&self) -> Result<DbSession, StorageError> {
        let inner = self.session_factory.create_session()?;
        Ok(DbSession::new(inner))
    }

    pub fn find_all<T: serde::de::DeserializeOwned>(
        &self,
        table: &str,
    ) -> Result<Vec<T>, StorageError> {
        let mut session = self.create_session()?;
        let result = session.find_all(table)?;
        session.flush_and_commit()?;
        Ok(result)
    }

    pub fn find_by_id<T: serde::de::DeserializeOwned>(
        &self,
        table: &str,
        id: &str,
    ) -> Result<Option<T>, StorageError> {
        let mut session = self.create_session()?;
        let result = session.find(table, id)?;
        session.flush_and_commit()?;
        Ok(result)
    }

    pub fn find_all_by<T: serde::de::DeserializeOwned>(
        &self,
        table: &str,
        col: &str,
        val: &str,
    ) -> Result<Vec<T>, StorageError> {
        let mut session = self.create_session()?;
        let result = session.find_by(table, col, val)?;
        session.flush_and_commit()?;
        Ok(result)
    }

    pub fn insert_json_with_extra<T: serde::Serialize>(
        &self,
        table: &str,
        id: &str,
        value: &T,
        extra_cols: &str,
        extra_values: &[Option<String>],
    ) -> Result<(), StorageError> {
        let mut session = self.create_session()?;
        let cols: Vec<&str> = extra_cols.split(", ").collect();
        let mut extras: Vec<(String, Option<String>)> = Vec::new();
        for (i, col) in cols.iter().enumerate() {
            let val = extra_values.get(i).cloned().flatten();
            extras.push((col.to_string(), val));
        }
        session.insert_with_extra(table, id, value, &extras)?;
        session.flush_and_commit()?;
        Ok(())
    }

    pub fn next_process_definition_version(
        &self,
        tenant_id: &str,
        process_key: &str,
    ) -> Result<i32, StorageError> {
        let mut session = self.create_session()?;
        let result = session.next_process_definition_version(tenant_id, process_key)?;
        session.flush_and_commit()?;
        Ok(result)
    }

    pub fn insert_deployment_resource(
        &self,
        deployment_id: &str,
        name: &str,
        resource_type: &str,
        content_type: &str,
        bytes: &[u8],
        created_at: i64,
    ) -> Result<(), StorageError> {
        let mut session = self.create_session()?;
        session.upsert_deployment_resource(
            deployment_id,
            name,
            resource_type,
            content_type,
            bytes,
            created_at,
        )?;
        session.flush_and_commit()?;
        Ok(())
    }

    pub fn list_deployment_resource_names(
        &self,
        deployment_id: &str,
    ) -> Result<Vec<String>, StorageError> {
        let mut session = self.create_session()?;
        let result = session.list_deployment_resource_names(deployment_id)?;
        session.flush_and_commit()?;
        Ok(result)
    }

    pub fn find_deployment_resource(
        &self,
        deployment_id: &str,
        name: &str,
    ) -> Result<Option<crate::repository::deployment_resource::DeploymentResource>, StorageError>
    {
        let mut session = self.create_session()?;
        let result = session.find_deployment_resource(deployment_id, name)?;
        session.flush_and_commit()?;
        Ok(result)
    }

    pub fn list_deployment_resources(
        &self,
        deployment_id: &str,
    ) -> Result<Vec<crate::repository::deployment_resource::DeploymentResource>, StorageError> {
        let mut session = self.create_session()?;
        let result = session.list_deployment_resources(deployment_id)?;
        session.flush_and_commit()?;
        Ok(result)
    }

    pub fn iter_deployment_resource_bytes(
        &self,
    ) -> Result<Vec<(String, String, Vec<u8>)>, StorageError> {
        let mut session = self.create_session()?;
        let result = session.iter_all_deployment_resource_bytes()?;
        session.flush_and_commit()?;
        Ok(result)
    }
}

pub struct DbStoreExtra;

impl DbStoreExtra {
    pub fn insert_repository_model_blob(
        store: &DbStore,
        model: &crate::repository::model::RepositoryModel,
        source_bytes: &[u8],
        source_extra_bytes: &[u8],
    ) -> Result<(), StorageError> {
        let mut session = store.create_session()?;
        let json = serde_json::to_string(model)?;
        let dep_id = model.deployment_id.clone().unwrap_or_default();
        let tenant = model.tenant_id.clone().unwrap_or_default();
        session.insert_repository_model(
            &model.id,
            &json,
            &dep_id,
            &model.key,
            &tenant,
            source_bytes,
            source_extra_bytes,
        )?;
        session.flush_and_commit()?;
        Ok(())
    }

    pub fn update_repository_model_data(
        store: &DbStore,
        model: &crate::repository::model::RepositoryModel,
    ) -> Result<(), StorageError> {
        let mut session = store.create_session()?;
        let json = serde_json::to_string(model)?;
        let dep_id = model.deployment_id.clone().unwrap_or_default();
        let tenant = model.tenant_id.clone().unwrap_or_default();
        session.update_repository_model_data(&model.id, &json, &dep_id, &model.key, &tenant)?;
        session.flush_and_commit()?;
        Ok(())
    }

    pub fn update_repository_model_blob(
        store: &DbStore,
        model: &crate::repository::model::RepositoryModel,
        source_bytes: Option<&[u8]>,
        source_extra_bytes: Option<&[u8]>,
    ) -> Result<(), StorageError> {
        let mut session = store.create_session()?;
        let json = serde_json::to_string(model)?;
        let dep_id = model.deployment_id.clone().unwrap_or_default();
        let tenant = model.tenant_id.clone().unwrap_or_default();
        match (source_bytes, source_extra_bytes) {
            (Some(bytes), _) => {
                session.update_repository_model_blob(
                    &model.id,
                    &json,
                    &dep_id,
                    &model.key,
                    &tenant,
                    "source_bytes",
                    bytes,
                )?;
            }
            (None, Some(bytes)) => {
                session.update_repository_model_blob(
                    &model.id,
                    &json,
                    &dep_id,
                    &model.key,
                    &tenant,
                    "source_extra_bytes",
                    bytes,
                )?;
            }
            (None, None) => {
                session
                    .update_repository_model_data(&model.id, &json, &dep_id, &model.key, &tenant)?;
            }
        }
        session.flush_and_commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_timer_job_table_is_upgraded_with_typed_job_column() {
        let store = DbStore::new_in_memory().unwrap();
        let mut session = store.create_session().unwrap();
        session
            .execute_raw_sql("DROP TABLE timer_job_states")
            .unwrap();
        session
            .execute_raw_sql(
                "CREATE TABLE timer_job_states (id TEXT PRIMARY KEY, process_instance_id TEXT, execution_id TEXT, lock_owner TEXT, lock_time INTEGER, lock_expiration_time INTEGER, retries INTEGER, error_message TEXT, error_details TEXT, due_time INTEGER, job_state TEXT, data TEXT NOT NULL)",
            )
            .unwrap();
        session.flush_and_commit().unwrap();

        let mut session = store.create_session().unwrap();
        ensure_legacy_tables(&mut session).unwrap();
        drop(session);

        let mut session = store.create_session().unwrap();
        assert!(legacy_column_exists(&mut session, "timer_job_states", "job_type").unwrap());
        for column in [
            "create_time",
            "correlation_id",
            "handler_type",
            "tenant_id",
            "process_definition_id",
            "element_name",
            "category",
            "scope_id",
            "sub_scope_id",
            "scope_type",
            "scope_definition_id",
        ] {
            assert!(
                legacy_column_exists(&mut session, "timer_job_states", column).unwrap(),
                "migration must add {column}"
            );
        }
        session.rollback().unwrap();
    }
}
