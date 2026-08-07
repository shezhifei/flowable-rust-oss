use rusqlite::Connection;

pub fn init_schema(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS deployments (
            id TEXT PRIMARY KEY,
            data TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS deployment_resources (
            deployment_id TEXT,
            name TEXT,
            bytes BLOB,
            PRIMARY KEY (deployment_id, name)
        );

        CREATE TABLE IF NOT EXISTS process_definitions (
            id TEXT PRIMARY KEY,
            deployment_id TEXT,
            key TEXT,
            tenant_id TEXT,
            data TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS process_definition_versions (
            tenant_id TEXT,
            process_key TEXT,
            version INTEGER,
            PRIMARY KEY (tenant_id, process_key)
        );

        CREATE TABLE IF NOT EXISTS repository_models (
            id TEXT PRIMARY KEY,
            deployment_id TEXT,
            model_key TEXT,
            tenant_id TEXT,
            source_bytes BLOB NOT NULL,
            source_extra_bytes BLOB NOT NULL,
            data TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS executions (
            id TEXT PRIMARY KEY,
            process_instance_id TEXT,
            data TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS process_instances (
            id TEXT PRIMARY KEY,
            process_definition_id TEXT,
            data TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS process_instance_locks (
            id TEXT PRIMARY KEY,
            lock_owner TEXT,
            lock_time INTEGER,
            data TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS event_registry_deployments (
            id TEXT PRIMARY KEY,
            name TEXT,
            deployed_at INTEGER,
            data TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS event_registry_channel_definitions (
            id TEXT PRIMARY KEY,
            deployment_id TEXT,
            key TEXT,
            name TEXT,
            channel_type TEXT,
            resource_name TEXT,
            data TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS event_registry_event_definitions (
            id TEXT PRIMARY KEY,
            deployment_id TEXT,
            key TEXT,
            name TEXT,
            event_type TEXT,
            channel_key TEXT,
            resource_name TEXT,
            data TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS event_registry_event_instance_deliveries (
            id TEXT PRIMARY KEY,
            event_definition_key TEXT,
            event_type TEXT,
            channel_key TEXT,
            direction TEXT,
            status TEXT,
            created_at INTEGER,
            updated_at INTEGER,
            data TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS event_registry_change_records (
            id TEXT PRIMARY KEY,
            revision INTEGER,
            change_type TEXT,
            entity_type TEXT,
            entity_key TEXT,
            data TEXT NOT NULL
        );

        CREATE UNIQUE INDEX IF NOT EXISTS idx_event_registry_change_revision
            ON event_registry_change_records(revision);

        CREATE TABLE IF NOT EXISTS event_registry_change_revision_seq (
            id TEXT PRIMARY KEY,
            next_revision INTEGER NOT NULL
        );

        INSERT OR IGNORE INTO event_registry_change_revision_seq (id, next_revision)
            SELECT 'event-registry', COALESCE(MAX(revision), 0)
            FROM event_registry_change_records;

        CREATE TABLE IF NOT EXISTS form_deployments (
            id TEXT PRIMARY KEY,
            name TEXT,
            deployed_at INTEGER,
            data TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS form_definitions (
            id TEXT PRIMARY KEY,
            deployment_id TEXT,
            key TEXT,
            name TEXT,
            version INTEGER,
            resource_name TEXT,
            data TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS content_items (
            id TEXT PRIMARY KEY,
            name TEXT,
            mime_type TEXT,
            created_at INTEGER,
            data TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS http_task_records (
            id TEXT PRIMARY KEY,
            process_instance_id TEXT,
            execution_id TEXT,
            activity_id TEXT,
            method TEXT,
            url TEXT,
            status TEXT,
            created_at INTEGER,
            data TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS mail_outbox_records (
            id TEXT PRIMARY KEY,
            process_instance_id TEXT,
            execution_id TEXT,
            activity_id TEXT,
            recipient TEXT,
            subject TEXT,
            status TEXT,
            created_at INTEGER,
            data TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS tasks (
            id TEXT PRIMARY KEY,
            process_instance_id TEXT,
            execution_id TEXT,
            task_definition_key TEXT,
            name TEXT,
            assignee TEXT,
            owner TEXT,
            parent_task_id TEXT,
            priority INTEGER,
            due_date INTEGER,
            data TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS variables (
            id TEXT PRIMARY KEY,
            execution_id TEXT,
            process_instance_id TEXT,
            name TEXT,
            data TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS event_subscriptions (
            id TEXT PRIMARY KEY,
            execution_id TEXT,
            process_instance_id TEXT,
            event_name TEXT,
            event_kind TEXT,
            data TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS event_wait_states (
            id TEXT PRIMARY KEY,
            process_instance_id TEXT,
            data TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS boundary_event_states (
            id TEXT PRIMARY KEY,
            process_instance_id TEXT,
            host_execution_id TEXT,
            data TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS timer_job_states (
            id TEXT PRIMARY KEY,
            process_instance_id TEXT,
            execution_id TEXT,
            lock_owner TEXT,
            lock_time INTEGER,
            lock_expiration_time INTEGER,
            retries INTEGER,
            error_message TEXT,
            error_details TEXT,
            job_type TEXT,
            create_time INTEGER,
            correlation_id TEXT,
            handler_type TEXT,
            tenant_id TEXT,
            process_definition_id TEXT,
            element_name TEXT,
            category TEXT,
            scope_id TEXT,
            sub_scope_id TEXT,
            scope_type TEXT,
            scope_definition_id TEXT,
            data TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS process_timer_start_subscriptions (
            id TEXT PRIMARY KEY,
            process_definition_id TEXT,
            lock_owner TEXT,
            lock_time INTEGER,
            data TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS process_event_start_subscriptions (
            id TEXT PRIMARY KEY,
            process_definition_id TEXT,
            event_kind TEXT,
            event_ref TEXT,
            data TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS event_subprocess_timer_subscriptions (
            id TEXT PRIMARY KEY,
            process_instance_id TEXT,
            lock_owner TEXT,
            lock_time INTEGER,
            data TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS event_subprocess_event_subscriptions (
            id TEXT PRIMARY KEY,
            process_instance_id TEXT,
            event_kind TEXT,
            event_ref TEXT,
            data TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS timer_worker_nodes (
            id TEXT PRIMARY KEY,
            last_heartbeat INTEGER,
            data TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS timer_coordinator_leases (
            id TEXT PRIMARY KEY,
            owner_node_id TEXT,
            expiry_time INTEGER,
            fencing_token INTEGER,
            data TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS timer_admin_audit_logs (
            id TEXT PRIMARY KEY,
            request_id TEXT,
            timestamp INTEGER,
            tenant_id TEXT,
            issuer TEXT,
            subject TEXT,
            actor TEXT,
            action TEXT,
            target TEXT,
            outcome TEXT,
            data TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS token_revocations (
            id TEXT PRIMARY KEY,
            issuer TEXT,
            reason TEXT,
            expires_at INTEGER,
            created_at INTEGER,
            data TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS timer_issuer_profiles (
            id TEXT PRIMARY KEY,
            issuer TEXT,
            data TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS users (
            id TEXT PRIMARY KEY,
            data TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS user_info (
            id TEXT PRIMARY KEY,
            user_id TEXT NOT NULL,
            info_key TEXT NOT NULL,
            data TEXT NOT NULL,
            UNIQUE (user_id, info_key)
        );

        CREATE TABLE IF NOT EXISTS user_pictures (
            id TEXT PRIMARY KEY,
            user_id TEXT NOT NULL UNIQUE,
            mime_type TEXT NOT NULL,
            bytes BLOB NOT NULL,
            data TEXT NOT NULL DEFAULT ''
        );

        CREATE TABLE IF NOT EXISTS groups (
            id TEXT PRIMARY KEY,
            data TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS memberships (
            id TEXT PRIMARY KEY,
            user_id TEXT,
            group_id TEXT,
            data TEXT NOT NULL,
            UNIQUE (user_id, group_id)
        );

        CREATE TABLE IF NOT EXISTS privileges (
            id TEXT PRIMARY KEY,
            name TEXT,
            data TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS privilege_mappings (
            id TEXT PRIMARY KEY,
            privilege_id TEXT,
            user_id TEXT,
            group_id TEXT,
            data TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS tokens (
            id TEXT PRIMARY KEY,
            token_value TEXT,
            data TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS identity_links (
            id TEXT PRIMARY KEY,
            link_type TEXT,
            task_id TEXT,
            process_instance_id TEXT,
            process_definition_id TEXT,
            user_id TEXT,
            group_id TEXT,
            data TEXT NOT NULL
        );

        -- P77: historic identity links (Java ACT_HI_IDENTITYLINK).
        -- Columns mirror Java create SQL (TYPE_/USER_ID_/GROUP_ID_/TASK_ID_/
        -- PROC_INST_ID_/SCOPE_*/CREATE_TIME_) projected for filter pushdown;
        -- JSON `data` remains the source of truth (legacy RuntimeStore pattern).
        CREATE TABLE IF NOT EXISTS historic_identity_links (
            id TEXT PRIMARY KEY,
            link_type TEXT,
            task_id TEXT,
            process_instance_id TEXT,
            user_id TEXT,
            group_id TEXT,
            scope_id TEXT,
            sub_scope_id TEXT,
            scope_type TEXT,
            scope_definition_id TEXT,
            create_time INTEGER,
            data TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS entity_links (
            id TEXT PRIMARY KEY,
            link_type TEXT,
            scope_id TEXT,
            scope_type TEXT,
            reference_scope_id TEXT,
            reference_scope_type TEXT,
            data TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS batch_entities (
            id TEXT PRIMARY KEY,
            batch_type TEXT,
            status TEXT,
            create_time INTEGER,
            tenant_id TEXT,
            data TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS batch_part_entities (
            id TEXT PRIMARY KEY,
            batch_id TEXT,
            batch_type TEXT,
            status TEXT,
            create_time INTEGER,
            data TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS ACT_GE_PROPERTY (
            NAME_ TEXT PRIMARY KEY,
            VALUE_ TEXT,
            REV_ INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS historic_process_instances (
            id TEXT PRIMARY KEY,
            process_definition_id TEXT,
            data TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS historic_activity_instances (
            id TEXT PRIMARY KEY,
            process_instance_id TEXT,
            execution_id TEXT,
            activity_id TEXT,
            delete_reason TEXT,
            data TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS historic_task_instances (
            id TEXT PRIMARY KEY,
            process_instance_id TEXT,
            process_definition_id TEXT,
            task_definition_key TEXT,
            assignee TEXT,
            owner TEXT,
            claim_time INTEGER,
            tenant_id TEXT,
            category TEXT,
            form_key TEXT,
            parent_task_id TEXT,
            priority INTEGER,
            due_date INTEGER,
            data TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS historic_variable_instances (
            id TEXT PRIMARY KEY,
            process_instance_id TEXT,
            variable_name TEXT,
            data TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS historic_details (
            id TEXT PRIMARY KEY,
            process_instance_id TEXT,
            execution_id TEXT,
            task_id TEXT,
            time INTEGER,
            detail_type TEXT,
            variable_name TEXT,
            property_id TEXT,
            data TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS historic_audit_logs (
            id TEXT PRIMARY KEY,
            data TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS historic_comments (
            id TEXT PRIMARY KEY,
            task_id TEXT,
            process_instance_id TEXT,
            time INTEGER,
            comment_type TEXT,
            data TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS historic_task_events (
            id TEXT PRIMARY KEY,
            task_id TEXT,
            time INTEGER,
            data TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS historic_task_log_entries (
            id TEXT PRIMARY KEY,
            log_number INTEGER,
            task_id TEXT,
            log_type TEXT,
            process_instance_id TEXT,
            process_definition_id TEXT,
            timestamp INTEGER,
            data TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS cleanup_logs (
            id TEXT PRIMARY KEY,
            data TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS cleanup_strategy_configs (
            id TEXT PRIMARY KEY,
            data TEXT NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_historic_comments_task_id
            ON historic_comments(task_id);
        CREATE INDEX IF NOT EXISTS idx_historic_comments_process_instance_id
            ON historic_comments(process_instance_id);
        CREATE INDEX IF NOT EXISTS idx_historic_comments_time
            ON historic_comments(time);
        CREATE INDEX IF NOT EXISTS idx_historic_comments_comment_type
            ON historic_comments(comment_type);
        CREATE INDEX IF NOT EXISTS idx_historic_task_events_task_id
            ON historic_task_events(task_id);
        CREATE INDEX IF NOT EXISTS idx_historic_task_events_time
            ON historic_task_events(time);
        CREATE INDEX IF NOT EXISTS idx_historic_task_log_entries_log_number
            ON historic_task_log_entries(log_number);
        CREATE INDEX IF NOT EXISTS idx_historic_task_log_entries_task_id
            ON historic_task_log_entries(task_id);
        CREATE INDEX IF NOT EXISTS idx_historic_task_log_entries_type
            ON historic_task_log_entries(log_type);
        CREATE INDEX IF NOT EXISTS idx_historic_task_log_entries_process_instance_id
            ON historic_task_log_entries(process_instance_id);
        CREATE INDEX IF NOT EXISTS idx_historic_task_log_entries_process_definition_id
            ON historic_task_log_entries(process_definition_id);
        CREATE INDEX IF NOT EXISTS idx_historic_task_log_entries_timestamp
            ON historic_task_log_entries(timestamp);

        CREATE TABLE IF NOT EXISTS compensation_subscriptions (
            id TEXT PRIMARY KEY,
            process_instance_id TEXT,
            execution_id TEXT,
            activity_id TEXT,
            compensation_activity_id TEXT,
            subscription_order INTEGER,
            data TEXT NOT NULL
        );
        ",
    )?;

    ensure_column(conn, "timer_admin_audit_logs", "request_id", "TEXT")?;
    ensure_column(conn, "timer_admin_audit_logs", "tenant_id", "TEXT")?;
    ensure_column(conn, "timer_admin_audit_logs", "issuer", "TEXT")?;
    ensure_column(conn, "timer_admin_audit_logs", "subject", "TEXT")?;
    ensure_column(conn, "timer_admin_audit_logs", "actor", "TEXT")?;
    ensure_column(conn, "timer_admin_audit_logs", "action", "TEXT")?;
    ensure_column(conn, "timer_admin_audit_logs", "target", "TEXT")?;
    ensure_column(conn, "timer_admin_audit_logs", "outcome", "TEXT")?;
    ensure_column(conn, "timer_admin_audit_logs", "profile_id", "TEXT")?;
    ensure_column(conn, "token_revocations", "created_at", "INTEGER")?;
    ensure_column(conn, "timer_job_states", "lock_expiration_time", "INTEGER")?;
    ensure_column(conn, "timer_job_states", "retries", "INTEGER")?;
    ensure_column(conn, "timer_job_states", "error_message", "TEXT")?;
    ensure_column(conn, "timer_job_states", "error_details", "TEXT")?;
    ensure_column(conn, "timer_job_states", "due_time", "INTEGER")?;
    ensure_column(conn, "timer_job_states", "job_state", "TEXT")?;
    ensure_column(conn, "timer_job_states", "job_type", "TEXT")?;
    // P2-11: track which projected job-query columns this upgrade introduces
    // so legacy rows get their JSON-only values backfilled (mirrors the
    // generic DB bootstrap in db_store.rs). Existing physical values win.
    let mut job_backfill_columns: Vec<&str> = Vec::new();
    for (column, column_type) in [
        ("create_time", "INTEGER"),
        ("correlation_id", "TEXT"),
        ("handler_type", "TEXT"),
        ("tenant_id", "TEXT"),
        ("process_definition_id", "TEXT"),
        ("element_name", "TEXT"),
        ("category", "TEXT"),
        ("scope_id", "TEXT"),
        ("sub_scope_id", "TEXT"),
        ("scope_type", "TEXT"),
        ("scope_definition_id", "TEXT"),
        ("activity_id", "TEXT"),
    ] {
        if ensure_column(conn, "timer_job_states", column, column_type)? {
            job_backfill_columns.push(column);
        }
    }
    if !job_backfill_columns.is_empty() {
        let rows: Vec<(String, String)> = {
            let mut stmt = conn.prepare("SELECT id, data FROM timer_job_states")?;
            let mapped = stmt.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?;
            mapped.collect::<Result<_, _>>()?
        };
        for (job_id, json) in rows {
            let Ok(state) = serde_json::from_str::<serde_json::Value>(&json) else {
                continue;
            };
            for column in &job_backfill_columns {
                let Some(field) = state.get(*column) else {
                    continue;
                };
                let sql = format!(
                    "UPDATE timer_job_states SET {column} = ?1 WHERE id = ?2 AND {column} IS NULL"
                );
                if let Some(text) = field.as_str() {
                    conn.execute(&sql, rusqlite::params![text, job_id])?;
                } else if let Some(number) = field.as_i64() {
                    conn.execute(&sql, rusqlite::params![number, job_id])?;
                }
            }
        }
    }
    ensure_column(conn, "timer_issuer_profiles", "version", "INTEGER")?;
    ensure_column(
        conn,
        "historic_activity_instances",
        "process_instance_id",
        "TEXT",
    )?;
    ensure_column(conn, "deployment_resources", "resource_type", "TEXT")?;
    ensure_column(conn, "deployment_resources", "content_type", "TEXT")?;
    ensure_column(conn, "deployment_resources", "created_at", "INTEGER")?;
    ensure_column(conn, "repository_models", "deployment_id", "TEXT")?;
    ensure_column(conn, "repository_models", "model_key", "TEXT")?;
    ensure_column(conn, "repository_models", "tenant_id", "TEXT")?;
    ensure_column(conn, "repository_models", "source_bytes", "BLOB")?;
    ensure_column(conn, "repository_models", "source_extra_bytes", "BLOB")?;
    ensure_column(conn, "tasks", "parent_task_id", "TEXT")?;
    ensure_column(conn, "tasks", "task_definition_key", "TEXT")?;
    ensure_column(conn, "tasks", "assignee", "TEXT")?;
    ensure_column(conn, "tasks", "owner", "TEXT")?;
    ensure_column(conn, "tasks", "priority", "INTEGER")?;
    ensure_column(conn, "tasks", "due_date", "INTEGER")?;
    ensure_column(
        conn,
        "historic_task_instances",
        "process_definition_id",
        "TEXT",
    )?;
    ensure_column(
        conn,
        "historic_task_instances",
        "task_definition_key",
        "TEXT",
    )?;
    ensure_column(conn, "historic_task_instances", "assignee", "TEXT")?;
    ensure_column(conn, "historic_task_instances", "owner", "TEXT")?;
    ensure_column(conn, "historic_task_instances", "claim_time", "INTEGER")?;
    ensure_column(conn, "historic_task_instances", "tenant_id", "TEXT")?;
    ensure_column(conn, "historic_task_instances", "category", "TEXT")?;
    ensure_column(conn, "historic_task_instances", "form_key", "TEXT")?;
    ensure_column(conn, "historic_task_instances", "parent_task_id", "TEXT")?;
    ensure_column(conn, "historic_task_instances", "priority", "INTEGER")?;
    ensure_column(conn, "historic_task_instances", "due_date", "INTEGER")?;
    ensure_column(conn, "historic_comments", "task_id", "TEXT")?;
    ensure_column(conn, "historic_comments", "process_instance_id", "TEXT")?;
    ensure_column(conn, "historic_comments", "time", "INTEGER")?;
    ensure_column(conn, "historic_comments", "comment_type", "TEXT")?;
    // P71: historic activity deleteReason projection (event-gateway cancel etc.).
    ensure_column(conn, "historic_activity_instances", "delete_reason", "TEXT")?;
    ensure_column(conn, "historic_task_events", "task_id", "TEXT")?;
    ensure_column(conn, "historic_task_events", "time", "INTEGER")?;
    ensure_column(conn, "historic_task_log_entries", "log_number", "INTEGER")?;
    ensure_column(conn, "historic_task_log_entries", "task_id", "TEXT")?;
    ensure_column(conn, "historic_task_log_entries", "log_type", "TEXT")?;
    ensure_column(
        conn,
        "historic_task_log_entries",
        "process_instance_id",
        "TEXT",
    )?;
    ensure_column(
        conn,
        "historic_task_log_entries",
        "process_definition_id",
        "TEXT",
    )?;
    ensure_column(conn, "historic_task_log_entries", "timestamp", "INTEGER")?;
    ensure_column(conn, "identity_links", "process_definition_id", "TEXT")?;
    ensure_column(conn, "historic_details", "process_instance_id", "TEXT")?;
    ensure_column(conn, "historic_details", "execution_id", "TEXT")?;
    ensure_column(conn, "historic_details", "task_id", "TEXT")?;
    ensure_column(conn, "historic_details", "time", "INTEGER")?;
    ensure_column(conn, "historic_details", "detail_type", "TEXT")?;
    ensure_column(conn, "historic_details", "variable_name", "TEXT")?;
    ensure_column(conn, "historic_details", "property_id", "TEXT")?;
    ensure_column(conn, "batch_entities", "batch_type", "TEXT")?;
    ensure_column(conn, "batch_entities", "status", "TEXT")?;
    ensure_column(conn, "batch_entities", "create_time", "INTEGER")?;
    ensure_column(conn, "batch_entities", "tenant_id", "TEXT")?;
    ensure_column(conn, "batch_part_entities", "batch_id", "TEXT")?;
    ensure_column(conn, "batch_part_entities", "batch_type", "TEXT")?;
    ensure_column(conn, "batch_part_entities", "status", "TEXT")?;
    ensure_column(conn, "batch_part_entities", "create_time", "INTEGER")?;
    ensure_column(
        conn,
        "compensation_subscriptions",
        "subscription_order",
        "INTEGER",
    )?;

    conn.execute(
        "UPDATE compensation_subscriptions \
         SET subscription_order = rowid \
         WHERE subscription_order IS NULL OR subscription_order = 0",
        [],
    )?;

    conn.execute_batch(
        "
        CREATE INDEX IF NOT EXISTS idx_timer_admin_audit_logs_request_id ON timer_admin_audit_logs(request_id);
        CREATE INDEX IF NOT EXISTS idx_timer_admin_audit_logs_tenant_id ON timer_admin_audit_logs(tenant_id);
        CREATE INDEX IF NOT EXISTS idx_timer_admin_audit_logs_actor ON timer_admin_audit_logs(actor);
        CREATE INDEX IF NOT EXISTS idx_timer_admin_audit_logs_action ON timer_admin_audit_logs(action);
        CREATE INDEX IF NOT EXISTS idx_token_revocations_expires_at ON token_revocations(expires_at);
        CREATE INDEX IF NOT EXISTS idx_proc_inst_def ON process_instances(process_definition_id);
        CREATE INDEX IF NOT EXISTS idx_repository_models_key ON repository_models(model_key);
        CREATE INDEX IF NOT EXISTS idx_repository_models_deployment_id ON repository_models(deployment_id);
        CREATE INDEX IF NOT EXISTS idx_repository_models_tenant_id ON repository_models(tenant_id);
        CREATE INDEX IF NOT EXISTS idx_event_registry_channel_key ON event_registry_channel_definitions(key);
        CREATE INDEX IF NOT EXISTS idx_event_registry_channel_type ON event_registry_channel_definitions(channel_type);
        CREATE INDEX IF NOT EXISTS idx_event_registry_event_key ON event_registry_event_definitions(key);
        CREATE INDEX IF NOT EXISTS idx_event_registry_event_type ON event_registry_event_definitions(event_type);
        CREATE INDEX IF NOT EXISTS idx_event_registry_event_channel_key ON event_registry_event_definitions(channel_key);
        CREATE INDEX IF NOT EXISTS idx_event_registry_delivery_direction ON event_registry_event_instance_deliveries(direction);
        CREATE INDEX IF NOT EXISTS idx_event_registry_delivery_status ON event_registry_event_instance_deliveries(status);
        CREATE INDEX IF NOT EXISTS idx_event_registry_delivery_event_type ON event_registry_event_instance_deliveries(event_type);
        CREATE INDEX IF NOT EXISTS idx_event_registry_delivery_channel_key ON event_registry_event_instance_deliveries(channel_key);
        CREATE INDEX IF NOT EXISTS idx_event_registry_change_revision ON event_registry_change_records(revision);
        CREATE INDEX IF NOT EXISTS idx_event_registry_change_entity_key ON event_registry_change_records(entity_key);
        CREATE INDEX IF NOT EXISTS idx_form_definitions_key ON form_definitions(key);
        CREATE INDEX IF NOT EXISTS idx_form_definitions_deployment_id ON form_definitions(deployment_id);
        CREATE INDEX IF NOT EXISTS idx_content_items_name ON content_items(name);
        CREATE INDEX IF NOT EXISTS idx_content_items_created_at ON content_items(created_at);
        CREATE INDEX IF NOT EXISTS idx_http_task_records_proc_inst ON http_task_records(process_instance_id);
        CREATE INDEX IF NOT EXISTS idx_http_task_records_exec ON http_task_records(execution_id);
        CREATE INDEX IF NOT EXISTS idx_http_task_records_activity_id ON http_task_records(activity_id);
        CREATE INDEX IF NOT EXISTS idx_mail_outbox_records_proc_inst ON mail_outbox_records(process_instance_id);
        CREATE INDEX IF NOT EXISTS idx_mail_outbox_records_exec ON mail_outbox_records(execution_id);
        CREATE INDEX IF NOT EXISTS idx_mail_outbox_records_activity_id ON mail_outbox_records(activity_id);
        CREATE INDEX IF NOT EXISTS idx_tasks_proc_inst ON tasks(process_instance_id);
        CREATE INDEX IF NOT EXISTS idx_tasks_exec ON tasks(execution_id);
        CREATE INDEX IF NOT EXISTS idx_tasks_task_definition_key ON tasks(task_definition_key);
        CREATE INDEX IF NOT EXISTS idx_tasks_assignee ON tasks(assignee);
        CREATE INDEX IF NOT EXISTS idx_tasks_owner ON tasks(owner);
        CREATE INDEX IF NOT EXISTS idx_tasks_priority ON tasks(priority);
        CREATE INDEX IF NOT EXISTS idx_tasks_due_date ON tasks(due_date);
        CREATE INDEX IF NOT EXISTS idx_tasks_parent ON tasks(parent_task_id);
        CREATE INDEX IF NOT EXISTS idx_historic_task_instances_process_definition_id ON historic_task_instances(process_definition_id);
        CREATE INDEX IF NOT EXISTS idx_historic_task_instances_task_definition_key ON historic_task_instances(task_definition_key);
        CREATE INDEX IF NOT EXISTS idx_historic_task_instances_assignee ON historic_task_instances(assignee);
        CREATE INDEX IF NOT EXISTS idx_historic_task_instances_owner ON historic_task_instances(owner);
        CREATE INDEX IF NOT EXISTS idx_historic_task_instances_priority ON historic_task_instances(priority);
        CREATE INDEX IF NOT EXISTS idx_historic_task_instances_due_date ON historic_task_instances(due_date);
        CREATE INDEX IF NOT EXISTS idx_variables_exec ON variables(execution_id);
        CREATE INDEX IF NOT EXISTS idx_variables_proc_inst ON variables(process_instance_id);
        CREATE INDEX IF NOT EXISTS idx_event_sub_name ON event_subscriptions(event_name);
        CREATE INDEX IF NOT EXISTS idx_event_sub_exec ON event_subscriptions(execution_id);
        CREATE INDEX IF NOT EXISTS idx_hist_proc_inst_id ON historic_activity_instances(execution_id);
        CREATE INDEX IF NOT EXISTS idx_user_info_user_id ON user_info(user_id);
        CREATE INDEX IF NOT EXISTS idx_privileges_name ON privileges(name);
        CREATE INDEX IF NOT EXISTS idx_privilege_mappings_privilege_id ON privilege_mappings(privilege_id);
        CREATE INDEX IF NOT EXISTS idx_privilege_mappings_user_id ON privilege_mappings(user_id);
        CREATE INDEX IF NOT EXISTS idx_privilege_mappings_group_id ON privilege_mappings(group_id);
        CREATE INDEX IF NOT EXISTS idx_tokens_value ON tokens(token_value);
        CREATE INDEX IF NOT EXISTS idx_identity_links_task_id ON identity_links(task_id);
        CREATE INDEX IF NOT EXISTS idx_identity_links_proc_inst ON identity_links(process_instance_id);
        CREATE INDEX IF NOT EXISTS idx_identity_links_proc_def ON identity_links(process_definition_id);
        CREATE INDEX IF NOT EXISTS idx_identity_links_user_id ON identity_links(user_id);
        CREATE INDEX IF NOT EXISTS idx_identity_links_group_id ON identity_links(group_id);
        CREATE INDEX IF NOT EXISTS idx_historic_identity_links_task_id ON historic_identity_links(task_id);
        CREATE INDEX IF NOT EXISTS idx_historic_identity_links_proc_inst ON historic_identity_links(process_instance_id);
        CREATE INDEX IF NOT EXISTS idx_historic_identity_links_user_id ON historic_identity_links(user_id);
        CREATE INDEX IF NOT EXISTS idx_historic_identity_links_scope ON historic_identity_links(scope_id, scope_type);
        CREATE INDEX IF NOT EXISTS idx_entity_links_scope ON entity_links(scope_id);
        CREATE INDEX IF NOT EXISTS idx_entity_links_ref_scope ON entity_links(reference_scope_id);
        CREATE INDEX IF NOT EXISTS idx_batch_status ON batch_entities(status);
        CREATE INDEX IF NOT EXISTS idx_batch_tenant_id ON batch_entities(tenant_id);
        CREATE INDEX IF NOT EXISTS idx_batch_part_batch_id ON batch_part_entities(batch_id);
        CREATE INDEX IF NOT EXISTS idx_batch_part_status ON batch_part_entities(status);
        CREATE INDEX IF NOT EXISTS idx_compensation_subscriptions_proc_order ON compensation_subscriptions(process_instance_id, subscription_order);
        CREATE INDEX IF NOT EXISTS idx_historic_details_process_instance_id ON historic_details(process_instance_id);
        CREATE INDEX IF NOT EXISTS idx_historic_details_execution_id ON historic_details(execution_id);
        CREATE INDEX IF NOT EXISTS idx_historic_details_task_id ON historic_details(task_id);
        CREATE INDEX IF NOT EXISTS idx_historic_details_detail_type ON historic_details(detail_type);
        CREATE INDEX IF NOT EXISTS idx_historic_details_variable_name ON historic_details(variable_name);
        CREATE INDEX IF NOT EXISTS idx_historic_details_property_id ON historic_details(property_id);
        CREATE INDEX IF NOT EXISTS idx_historic_details_time ON historic_details(time);
        CREATE INDEX IF NOT EXISTS idx_timer_job_due_time ON timer_job_states(due_time);
        CREATE INDEX IF NOT EXISTS idx_timer_job_state ON timer_job_states(job_state);
        CREATE INDEX IF NOT EXISTS idx_timer_job_type ON timer_job_states(job_type);
        CREATE INDEX IF NOT EXISTS idx_timer_job_lock_owner ON timer_job_states(lock_owner);
        CREATE INDEX IF NOT EXISTS idx_timer_job_lock_expiration ON timer_job_states(lock_expiration_time);
        CREATE INDEX IF NOT EXISTS idx_timer_job_category ON timer_job_states(category);
        CREATE INDEX IF NOT EXISTS idx_timer_job_scope ON timer_job_states(scope_id, scope_type, sub_scope_id);
        CREATE INDEX IF NOT EXISTS idx_timer_job_scope_definition ON timer_job_states(scope_definition_id);
        CREATE INDEX IF NOT EXISTS idx_timer_job_correlation ON timer_job_states(correlation_id);
        CREATE INDEX IF NOT EXISTS idx_timer_job_handler_type ON timer_job_states(handler_type);
        "
    )?;

    // Task 10: add start_time_ms / end_time_ms columns to historic_process_instances
    // so cleanup_batch can push date filtering into SQL instead of loading all rows.
    // Idempotent via ensure_column; existing rows keep NULL (matched by legacy path).
    ensure_column(
        conn,
        "historic_process_instances",
        "start_time_ms",
        "INTEGER",
    )?;
    ensure_column(conn, "historic_process_instances", "end_time_ms", "INTEGER")?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_historic_pi_start_time_ms ON historic_process_instances(start_time_ms)",
        [],
    )?;

    // P77: ensure historic_identity_links exists on upgraded DBs (CREATE IF NOT
    // EXISTS above only runs for fresh schemas; ensure_column is a no-op when
    // the table was just created with the full column list).
    conn.execute(
        "CREATE TABLE IF NOT EXISTS historic_identity_links (
            id TEXT PRIMARY KEY,
            link_type TEXT,
            task_id TEXT,
            process_instance_id TEXT,
            user_id TEXT,
            group_id TEXT,
            scope_id TEXT,
            sub_scope_id TEXT,
            scope_type TEXT,
            scope_definition_id TEXT,
            create_time INTEGER,
            data TEXT NOT NULL
        )",
        [],
    )?;
    ensure_column(conn, "historic_identity_links", "link_type", "TEXT")?;
    ensure_column(conn, "historic_identity_links", "task_id", "TEXT")?;
    ensure_column(conn, "historic_identity_links", "process_instance_id", "TEXT")?;
    ensure_column(conn, "historic_identity_links", "user_id", "TEXT")?;
    ensure_column(conn, "historic_identity_links", "group_id", "TEXT")?;
    ensure_column(conn, "historic_identity_links", "scope_id", "TEXT")?;
    ensure_column(conn, "historic_identity_links", "sub_scope_id", "TEXT")?;
    ensure_column(conn, "historic_identity_links", "scope_type", "TEXT")?;
    ensure_column(conn, "historic_identity_links", "scope_definition_id", "TEXT")?;
    ensure_column(conn, "historic_identity_links", "create_time", "INTEGER")?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_historic_identity_links_task_id ON historic_identity_links(task_id)",
        [],
    )?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_historic_identity_links_proc_inst ON historic_identity_links(process_instance_id)",
        [],
    )?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_historic_identity_links_user_id ON historic_identity_links(user_id)",
        [],
    )?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_historic_identity_links_scope ON historic_identity_links(scope_id, scope_type)",
        [],
    )?;

    Ok(())
}

fn ensure_column(
    conn: &Connection,
    table: &str,
    column: &str,
    column_type: &str,
) -> rusqlite::Result<bool> {
    let pragma = format!("PRAGMA table_info({})", table);
    let mut stmt = conn.prepare(&pragma)?;
    let existing_columns = stmt.query_map([], |row| row.get::<_, String>(1))?;

    let has_column = existing_columns
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .any(|existing| existing == column);

    if !has_column {
        let alter = format!(
            "ALTER TABLE {} ADD COLUMN {} {}",
            table, column, column_type
        );
        conn.execute(&alter, [])?;
    }

    Ok(!has_column)
}
