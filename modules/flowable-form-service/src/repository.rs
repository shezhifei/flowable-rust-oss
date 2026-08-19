use crate::models::{FormDefinition, FormDeployment, FormInstance};
use flowable_engine::error::FlowableError;
use flowable_engine::persistence::DbParams;
use flowable_engine::persistence::db_session::DbSession;
use flowable_engine::persistence::runtime_store::RuntimeStore;
use flowable_engine::persistence::StorageError;

const FORM_DEPLOYMENTS_TABLE: &str = "m14_form_deployments";
const FORM_DEFINITIONS_TABLE: &str = "m14_form_definitions";
const FORM_INSTANCES_TABLE: &str = "m40_form_instances";

pub(crate) fn ensure_schema(store: &RuntimeStore) {
    let mut session = store.db_store().create_session().unwrap();

    let id = session.dialect().varchar_type(255);
    let short = session.dialect().varchar_type(255);
    let text = session.dialect().text_type();
    let int = session.dialect().integer_type();
    let big = session.dialect().bigint_type();

    // execute_raw_sql 只能处理单条语句，逐条执行 DDL
    session
        .execute_raw_sql(&format!(
            "CREATE TABLE IF NOT EXISTS {FORM_DEPLOYMENTS_TABLE} (id {id} PRIMARY KEY, data {text} NOT NULL, name {short} NOT NULL, deployed_at {big} NOT NULL)"
        ))
        .unwrap();
    session
        .execute_raw_sql(&format!(
            "CREATE TABLE IF NOT EXISTS {FORM_DEFINITIONS_TABLE} (id {id} PRIMARY KEY, data {text} NOT NULL, deployment_id {short} NOT NULL, form_key {short} NOT NULL, name {short} NOT NULL, version {int} NOT NULL, resource_name {short} NOT NULL, active {int} NOT NULL DEFAULT 1)"
        ))
        .unwrap();
    session
        .execute_raw_sql(&format!(
            "CREATE TABLE IF NOT EXISTS {FORM_INSTANCES_TABLE} (id {id} PRIMARY KEY, data {text} NOT NULL, form_definition_id {short} NOT NULL, form_definition_key {short} NOT NULL, process_definition_id {short}, process_instance_id {short}, task_id {short}, scope_type {short} NOT NULL, scope_id {short} NOT NULL, scope_definition_id {short}, submitted_at {big} NOT NULL, submitted_by {short}, tenant_id {short}, form_values_id {short})"
        ))
        .unwrap();

    create_index(
        &mut session,
        "idx_form_deployments_name",
        FORM_DEPLOYMENTS_TABLE,
        "name",
    );
    create_index(
        &mut session,
        "idx_form_deployments_deployed_at",
        FORM_DEPLOYMENTS_TABLE,
        "deployed_at",
    );
    create_index(
        &mut session,
        "idx_form_definitions_key",
        FORM_DEFINITIONS_TABLE,
        "form_key",
    );
    create_index(
        &mut session,
        "idx_form_definitions_deployment_id",
        FORM_DEFINITIONS_TABLE,
        "deployment_id",
    );
    create_index(
        &mut session,
        "idx_form_definitions_name",
        FORM_DEFINITIONS_TABLE,
        "name",
    );
    create_index(
        &mut session,
        "idx_form_definitions_resource_name",
        FORM_DEFINITIONS_TABLE,
        "resource_name",
    );
    create_index(
        &mut session,
        "idx_form_instances_definition_id",
        FORM_INSTANCES_TABLE,
        "form_definition_id",
    );
    create_index(
        &mut session,
        "idx_form_instances_definition_key",
        FORM_INSTANCES_TABLE,
        "form_definition_key",
    );
    create_index(
        &mut session,
        "idx_form_instances_process_definition_id",
        FORM_INSTANCES_TABLE,
        "process_definition_id",
    );
    create_index(
        &mut session,
        "idx_form_instances_process_instance_id",
        FORM_INSTANCES_TABLE,
        "process_instance_id",
    );
    create_index(
        &mut session,
        "idx_form_instances_task_id",
        FORM_INSTANCES_TABLE,
        "task_id",
    );
    create_index(
        &mut session,
        "idx_form_instances_scope",
        FORM_INSTANCES_TABLE,
        "scope_type, scope_id",
    );
    create_index(
        &mut session,
        "idx_form_instances_scope_definition_id",
        FORM_INSTANCES_TABLE,
        "scope_definition_id",
    );
    create_index(
        &mut session,
        "idx_form_instances_tenant_id",
        FORM_INSTANCES_TABLE,
        "tenant_id",
    );
    create_index(
        &mut session,
        "idx_form_instances_form_values_id",
        FORM_INSTANCES_TABLE,
        "form_values_id",
    );
    create_index(
        &mut session,
        "idx_form_instances_submitted_at",
        FORM_INSTANCES_TABLE,
        "submitted_at",
    );

    migrate_form_instance_columns(&mut session);

    create_index(
        &mut session,
        "idx_form_instances_submitted_by",
        FORM_INSTANCES_TABLE,
        "submitted_by",
    );

    session.flush_and_commit().unwrap();
}

/// MySQL 8.0 没有 CREATE INDEX IF NOT EXISTS，重复索引按成功处理（对齐 engine db_store.rs）。
fn create_index(session: &mut DbSession, name: &str, table: &str, columns: &str) {
    let sql = session.dialect().create_index_if_not_exists(name, table, columns);
    if let Err(error) = session.execute_raw_sql(&sql) {
        let message = error.to_string();
        if message.contains("1061")
            || message.contains("Duplicate key name")
            || message.contains("already exists")
        {
            return;
        }
        panic!("index DDL failed: {error} | SQL: {sql}");
    }
}

fn migrate_form_instance_columns(session: &mut DbSession) {
    let column_names: std::collections::BTreeSet<String> = session
        .table_columns(FORM_INSTANCES_TABLE)
        .unwrap()
        .into_iter()
        .map(|column| column.name)
        .collect();

    for (column, ddl_type) in [
        ("submitted_by", "TEXT"),
        ("scope_definition_id", "TEXT"),
        ("tenant_id", "TEXT"),
        ("form_values_id", "TEXT"),
    ] {
        if !column_names.contains(column) {
            session
                .execute_raw_sql(&format!(
                    "ALTER TABLE {FORM_INSTANCES_TABLE} ADD COLUMN {column} {ddl_type}"
                ))
                .unwrap();
        }
    }
}

pub(crate) fn insert_form_deployment(store: &RuntimeStore, deployment: FormDeployment) {
    store
        .db_store()
        .insert_json_with_extra(
            FORM_DEPLOYMENTS_TABLE,
            &deployment.id,
            &deployment,
            "name, deployed_at",
            &[
                Some(deployment.name.clone()),
                Some(deployment.deployed_at.to_string()),
            ],
        )
        .unwrap();
}

pub(crate) fn insert_form_definition(store: &RuntimeStore, definition: FormDefinition) {
    let active_val = if definition.active.unwrap_or(true) {
        1
    } else {
        0
    };
    store
        .db_store()
        .insert_json_with_extra(
            FORM_DEFINITIONS_TABLE,
            &definition.id,
            &definition,
            "deployment_id, form_key, name, version, resource_name, active",
            &[
                Some(definition.deployment_id.clone()),
                Some(definition.key.clone()),
                Some(definition.name.clone()),
                Some(definition.version.to_string()),
                Some(definition.resource_name.clone()),
                Some(active_val.to_string()),
            ],
        )
        .unwrap();
}

pub(crate) fn find_form_definition(store: &RuntimeStore, id: &str) -> Option<FormDefinition> {
    store
        .db_store()
        .find_by_id(FORM_DEFINITIONS_TABLE, id)
        .unwrap()
}

pub(crate) fn list_form_definitions(store: &RuntimeStore) -> Vec<FormDefinition> {
    store.db_store().find_all(FORM_DEFINITIONS_TABLE).unwrap()
}

pub(crate) fn list_form_definitions_by_key(store: &RuntimeStore, key: &str) -> Vec<FormDefinition> {
    store
        .db_store()
        .find_all_by(FORM_DEFINITIONS_TABLE, "form_key", key)
        .unwrap()
}

pub(crate) fn find_form_definitions_by_key(
    store: &RuntimeStore,
    key: &str,
) -> Result<Vec<FormDefinition>, FlowableError> {
    let mut session = store
        .db_store()
        .create_session()
        .map_err(map_storage_error)?;
    let query = format!(
        "SELECT data FROM {} WHERE form_key = ? ORDER BY version DESC",
        FORM_DEFINITIONS_TABLE
    );
    let mut params = DbParams::new();
    params.push(key);
    let rows = session
        .raw_query(&query, params)
        .map_err(map_storage_error)?;
    let mut result = Vec::new();
    for row in &rows {
        if let Some(data) = row.get_text("data") {
            let def: FormDefinition = serde_json::from_str(&data).map_err(|e| {
                FlowableError::Internal(format!("Failed to deserialize form definition: {}", e))
            })?;
            result.push(def);
        }
    }
    session.flush_and_commit().map_err(map_storage_error)?;
    Ok(result)
}

pub(crate) fn find_form_definition_by_key_and_version(
    store: &RuntimeStore,
    key: &str,
    version: i32,
) -> Result<Option<FormDefinition>, FlowableError> {
    let mut session = store
        .db_store()
        .create_session()
        .map_err(map_storage_error)?;
    let query = format!(
        "SELECT data FROM {} WHERE form_key = ? AND version = ?",
        FORM_DEFINITIONS_TABLE
    );
    let mut params = DbParams::new();
    params.push(key);
    params.push(version as i64);
    let row = session
        .raw_query_one(&query, params)
        .map_err(map_storage_error)?;
    let result =
        match row {
            Some(r) => {
                let data = r.get_text("data").ok_or_else(|| {
                    FlowableError::Internal("Failed to get data column".to_string())
                })?;
                Some(serde_json::from_str(&data).map_err(|e| {
                    FlowableError::Internal(format!("Failed to deserialize: {}", e))
                })?)
            }
            None => None,
        };
    session.flush_and_commit().map_err(map_storage_error)?;
    Ok(result)
}

pub(crate) fn delete_form_definitions_by_deployment_id(
    store: &RuntimeStore,
    deployment_id: &str,
) -> Result<usize, FlowableError> {
    let mut session = store
        .db_store()
        .create_session()
        .map_err(map_storage_error)?;

    let def_ids_query = format!(
        "SELECT id FROM {} WHERE deployment_id = ?",
        FORM_DEFINITIONS_TABLE
    );
    let mut def_params = DbParams::new();
    def_params.push(deployment_id);
    let def_id_rows = session
        .raw_query(&def_ids_query, def_params)
        .map_err(map_storage_error)?;
    let def_ids: Vec<String> = def_id_rows
        .iter()
        .filter_map(|row| row.get_text("id"))
        .collect();

    for def_id in &def_ids {
        let mut del_inst_params = DbParams::new();
        del_inst_params.push(def_id.as_str());
        session
            .execute_raw(
                &format!(
                    "DELETE FROM {} WHERE form_definition_id = ?",
                    FORM_INSTANCES_TABLE
                ),
                del_inst_params,
            )
            .map_err(map_storage_error)?;
    }

    let mut del_def_params = DbParams::new();
    del_def_params.push(deployment_id);
    let count = session
        .execute_raw(
            &format!(
                "DELETE FROM {} WHERE deployment_id = ?",
                FORM_DEFINITIONS_TABLE
            ),
            del_def_params,
        )
        .map_err(map_storage_error)? as usize;

    session.flush_and_commit().map_err(map_storage_error)?;
    Ok(count)
}

pub(crate) fn delete_form_definitions_by_key(
    store: &RuntimeStore,
    key: &str,
) -> Result<usize, FlowableError> {
    let mut session = store
        .db_store()
        .create_session()
        .map_err(map_storage_error)?;

    let def_ids_query = format!(
        "SELECT id FROM {} WHERE form_key = ?",
        FORM_DEFINITIONS_TABLE
    );
    let mut def_params = DbParams::new();
    def_params.push(key);
    let def_id_rows = session
        .raw_query(&def_ids_query, def_params)
        .map_err(map_storage_error)?;
    let def_ids: Vec<String> = def_id_rows
        .iter()
        .filter_map(|row| row.get_text("id"))
        .collect();

    for def_id in &def_ids {
        let mut del_inst_params = DbParams::new();
        del_inst_params.push(def_id.as_str());
        session
            .execute_raw(
                &format!(
                    "DELETE FROM {} WHERE form_definition_id = ?",
                    FORM_INSTANCES_TABLE
                ),
                del_inst_params,
            )
            .map_err(map_storage_error)?;
    }

    let mut del_def_params = DbParams::new();
    del_def_params.push(key);
    let count = session
        .execute_raw(
            &format!("DELETE FROM {} WHERE form_key = ?", FORM_DEFINITIONS_TABLE),
            del_def_params,
        )
        .map_err(map_storage_error)? as usize;

    session.flush_and_commit().map_err(map_storage_error)?;
    Ok(count)
}

pub(crate) fn update_form_definition_activation(
    store: &RuntimeStore,
    id: &str,
    active: bool,
) -> Result<(), FlowableError> {
    let mut session = store
        .db_store()
        .create_session()
        .map_err(map_storage_error)?;

    let select_query = format!("SELECT data FROM {} WHERE id = ?", FORM_DEFINITIONS_TABLE);
    let mut sel_params = DbParams::new();
    sel_params.push(id);
    let row = session
        .raw_query_one(&select_query, sel_params)
        .map_err(map_storage_error)?;
    let data = match row {
        Some(r) => r
            .get_text("data")
            .ok_or_else(|| FlowableError::Internal("Failed to get data column".to_string()))?,
        None => {
            return Err(FlowableError::NotFound(format!(
                "Form definition '{}' was not found",
                id
            )));
        }
    };

    let mut definition: FormDefinition = serde_json::from_str(&data)
        .map_err(|e| FlowableError::Internal(format!("Failed to deserialize definition: {}", e)))?;
    definition.active = Some(active);
    let updated_json = serde_json::to_string(&definition)
        .map_err(|e| FlowableError::Internal(format!("Failed to serialize definition: {}", e)))?;

    let active_int: i64 = if active { 1 } else { 0 };
    let query = format!(
        "UPDATE {} SET active = ?, data = ? WHERE id = ?",
        FORM_DEFINITIONS_TABLE
    );
    let mut upd_params = DbParams::new();
    upd_params.push(active_int);
    upd_params.push(updated_json.as_str());
    upd_params.push(id);
    session
        .execute_raw(&query, upd_params)
        .map_err(map_storage_error)?;

    session.flush_and_commit().map_err(map_storage_error)?;
    Ok(())
}

#[allow(dead_code)]
pub(crate) fn insert_form_instance(store: &RuntimeStore, form_instance: FormInstance) {
    store.db_store().insert_json_with_extra(
        FORM_INSTANCES_TABLE,
        &form_instance.id,
        &form_instance,
        "form_definition_id, form_definition_key, process_definition_id, process_instance_id, task_id, scope_type, scope_id, scope_definition_id, submitted_at, submitted_by, tenant_id, form_values_id",
        &[
            Some(form_instance.form_definition_id.clone()),
            Some(form_instance.form_definition_key.clone()),
            Some(form_instance.process_definition_id.clone().unwrap_or_default()),
            Some(form_instance.process_instance_id.clone().unwrap_or_default()),
            Some(form_instance.task_id.clone().unwrap_or_default()),
            Some(form_instance.scope_type.clone()),
            Some(form_instance.scope_id.clone()),
            Some(form_instance.scope_definition_id.clone().unwrap_or_default()),
            Some(form_instance.submitted_at.to_string()),
            Some(form_instance.submitted_by.clone().unwrap_or_default()),
            Some(form_instance.tenant_id.clone().unwrap_or_default()),
            Some(form_instance.form_values_id.clone().unwrap_or_default()),
        ],
    )
    .unwrap();
}

/// Ensure form tables exist on the caller's session (no nested commit).
/// Used by `CompleteTaskWithFormCmd` so form instance + task complete share one TX.
pub fn ensure_form_schema_in_session(session: &mut DbSession) -> Result<(), StorageError> {
    session.execute_raw_sql(&format!(
        "CREATE TABLE IF NOT EXISTS {FORM_DEPLOYMENTS_TABLE} (id TEXT PRIMARY KEY, data TEXT NOT NULL, name TEXT NOT NULL, deployed_at INTEGER NOT NULL)"
    ))?;
    session.execute_raw_sql(&format!(
        "CREATE TABLE IF NOT EXISTS {FORM_DEFINITIONS_TABLE} (id TEXT PRIMARY KEY, data TEXT NOT NULL, deployment_id TEXT NOT NULL, form_key TEXT NOT NULL, name TEXT NOT NULL, version INTEGER NOT NULL, resource_name TEXT NOT NULL, active INTEGER NOT NULL DEFAULT 1)"
    ))?;
    session.execute_raw_sql(&format!(
        "CREATE TABLE IF NOT EXISTS {FORM_INSTANCES_TABLE} (id TEXT PRIMARY KEY, data TEXT NOT NULL, form_definition_id TEXT NOT NULL, form_definition_key TEXT NOT NULL, process_definition_id TEXT, process_instance_id TEXT, task_id TEXT, scope_type TEXT NOT NULL, scope_id TEXT NOT NULL, scope_definition_id TEXT, submitted_at INTEGER NOT NULL, submitted_by TEXT, tenant_id TEXT, form_values_id TEXT)"
    ))?;
    // Best-effort migrations for sessions that opened an older table shape.
    // DDL may auto-commit depending on dialect; still keeps new columns available.
    let _ = session.execute_raw_sql(&format!(
        "ALTER TABLE {FORM_INSTANCES_TABLE} ADD COLUMN scope_definition_id TEXT"
    ));
    let _ = session.execute_raw_sql(&format!(
        "ALTER TABLE {FORM_INSTANCES_TABLE} ADD COLUMN tenant_id TEXT"
    ));
    let _ = session.execute_raw_sql(&format!(
        "ALTER TABLE {FORM_INSTANCES_TABLE} ADD COLUMN form_values_id TEXT"
    ));
    let _ = session.execute_raw_sql(&format!(
        "ALTER TABLE {FORM_INSTANCES_TABLE} ADD COLUMN submitted_by TEXT"
    ));
    Ok(())
}

/// Insert form instance on the caller's session (Java `saveFormInstance` in same command).
pub fn insert_form_instance_in_session(
    session: &mut DbSession,
    form_instance: &FormInstance,
) -> Result<(), StorageError> {
    ensure_form_schema_in_session(session)?;

    let mut params = DbParams::new();
    params.push(form_instance.id.clone());
    params.push(serde_json::to_string(form_instance).unwrap());
    params.push(form_instance.form_definition_id.clone());
    params.push(form_instance.form_definition_key.clone());
    params.push(
        form_instance
            .process_definition_id
            .clone()
            .unwrap_or_default(),
    );
    params.push(
        form_instance
            .process_instance_id
            .clone()
            .unwrap_or_default(),
    );
    params.push(form_instance.task_id.clone().unwrap_or_default());
    params.push(form_instance.scope_type.clone());
    params.push(form_instance.scope_id.clone());
    params.push(
        form_instance
            .scope_definition_id
            .clone()
            .unwrap_or_default(),
    );
    params.push(form_instance.submitted_at);
    params.push(form_instance.submitted_by.clone().unwrap_or_default());
    params.push(form_instance.tenant_id.clone().unwrap_or_default());
    params.push(form_instance.form_values_id.clone().unwrap_or_default());

    session.upsert_raw(
        FORM_INSTANCES_TABLE,
        "id",
        &[
            "id",
            "data",
            "form_definition_id",
            "form_definition_key",
            "process_definition_id",
            "process_instance_id",
            "task_id",
            "scope_type",
            "scope_id",
            "scope_definition_id",
            "submitted_at",
            "submitted_by",
            "tenant_id",
            "form_values_id",
        ],
        params,
    )?;
    Ok(())
}

/// Find form definition by id using the caller's session.
pub fn find_form_definition_in_session(
    session: &mut DbSession,
    id: &str,
) -> Result<Option<FormDefinition>, StorageError> {
    ensure_form_schema_in_session(session)?;
    let mut params = DbParams::new();
    params.push(id);
    let rows = session.raw_query(
        &format!("SELECT data FROM {FORM_DEFINITIONS_TABLE} WHERE id = ?"),
        params,
    )?;
    Ok(rows
        .into_iter()
        .filter_map(|row| row.get_text("data"))
        .find_map(|json| serde_json::from_str::<FormDefinition>(&json).ok()))
}

/// List form instances for a task using the caller's session (tests / rollback checks).
pub fn find_form_instances_by_task_id_in_session(
    session: &mut DbSession,
    task_id: &str,
) -> Result<Vec<FormInstance>, StorageError> {
    ensure_form_schema_in_session(session)?;
    let mut params = DbParams::new();
    params.push(task_id);
    let rows = session.raw_query(
        &format!("SELECT data FROM {FORM_INSTANCES_TABLE} WHERE task_id = ?"),
        params,
    )?;
    Ok(rows
        .into_iter()
        .filter_map(|row| row.get_text("data"))
        .filter_map(|json| serde_json::from_str::<FormInstance>(&json).ok())
        .collect())
}

pub(crate) fn find_form_instance(store: &RuntimeStore, id: &str) -> Option<FormInstance> {
    store
        .db_store()
        .find_by_id(FORM_INSTANCES_TABLE, id)
        .unwrap()
}

/// Physical-column filters pushed into the repository (Java query parity).
/// Like-filters that require wildcards still run in the service query layer.
#[derive(Clone, Debug, Default)]
pub(crate) struct FormInstanceListFilter<'a> {
    pub id: Option<&'a str>,
    pub form_definition_id: Option<&'a str>,
    pub form_definition_key: Option<&'a str>,
    pub process_definition_id: Option<&'a str>,
    pub process_instance_id: Option<&'a str>,
    pub task_id: Option<&'a str>,
    pub without_task_id: bool,
    pub scope_id: Option<&'a str>,
    pub scope_type: Option<&'a str>,
    pub scope_definition_id: Option<&'a str>,
    pub tenant_id: Option<&'a str>,
    pub without_tenant_id: bool,
    pub submitted_date: Option<i64>,
    pub submitted_date_before: Option<i64>,
    pub submitted_date_after: Option<i64>,
    pub submitted_by: Option<&'a str>,
}

pub(crate) fn list_form_instances_filtered(
    store: &RuntimeStore,
    filter: FormInstanceListFilter<'_>,
) -> Vec<FormInstance> {
    let mut session = store.db_store().create_session().unwrap();
    // ensure columns exist for filtered queries
    let _ = ensure_form_schema_in_session(&mut session);

    let mut clauses = Vec::new();
    let mut params = DbParams::new();

    if let Some(id) = filter.id {
        clauses.push("id = ?".to_string());
        params.push(id);
    }
    if let Some(form_definition_id) = filter.form_definition_id {
        clauses.push("form_definition_id = ?".to_string());
        params.push(form_definition_id);
    }
    if let Some(form_definition_key) = filter.form_definition_key {
        clauses.push("form_definition_key = ?".to_string());
        params.push(form_definition_key);
    }
    if let Some(process_definition_id) = filter.process_definition_id {
        clauses.push("process_definition_id = ?".to_string());
        params.push(process_definition_id);
    }
    if let Some(process_instance_id) = filter.process_instance_id {
        clauses.push("process_instance_id = ?".to_string());
        params.push(process_instance_id);
    }
    if filter.without_task_id {
        clauses.push("(task_id IS NULL OR task_id = '')".to_string());
    } else if let Some(task_id) = filter.task_id {
        clauses.push("task_id = ?".to_string());
        params.push(task_id);
    }
    if let Some(scope_id) = filter.scope_id {
        clauses.push("scope_id = ?".to_string());
        params.push(scope_id);
    }
    if let Some(scope_type) = filter.scope_type {
        clauses.push("scope_type = ?".to_string());
        params.push(scope_type);
    }
    if let Some(scope_definition_id) = filter.scope_definition_id {
        clauses.push("scope_definition_id = ?".to_string());
        params.push(scope_definition_id);
    }
    if filter.without_tenant_id {
        clauses.push("(tenant_id IS NULL OR tenant_id = '')".to_string());
    } else if let Some(tenant_id) = filter.tenant_id {
        clauses.push("tenant_id = ?".to_string());
        params.push(tenant_id);
    }
    if let Some(submitted_date) = filter.submitted_date {
        clauses.push("submitted_at = ?".to_string());
        params.push(submitted_date);
    }
    if let Some(before) = filter.submitted_date_before {
        clauses.push("submitted_at < ?".to_string());
        params.push(before);
    }
    if let Some(after) = filter.submitted_date_after {
        clauses.push("submitted_at > ?".to_string());
        params.push(after);
    }
    if let Some(submitted_by) = filter.submitted_by {
        clauses.push("submitted_by = ?".to_string());
        params.push(submitted_by);
    }

    let where_clause = if clauses.is_empty() {
        String::new()
    } else {
        format!(" WHERE {}", clauses.join(" AND "))
    };
    let sql = format!("SELECT data FROM {FORM_INSTANCES_TABLE}{where_clause}");
    let rows = session.raw_query(&sql, params).unwrap();
    let instances = rows
        .into_iter()
        .filter_map(|row| row.get_text("data"))
        .filter_map(|json| serde_json::from_str::<FormInstance>(&json).ok())
        .collect();
    // Read-only query: release the session lock without writing.
    session.rollback().ok();
    instances
}

pub(crate) fn delete_form_instance(
    store: &RuntimeStore,
    form_instance_id: &str,
) -> Result<bool, FlowableError> {
    let mut session = store
        .db_store()
        .create_session()
        .map_err(map_storage_error)?;
    ensure_form_schema_in_session(&mut session).map_err(map_storage_error)?;
    let mut params = DbParams::new();
    params.push(form_instance_id);
    let deleted = session
        .execute_raw(
            &format!("DELETE FROM {FORM_INSTANCES_TABLE} WHERE id = ?"),
            params,
        )
        .map_err(map_storage_error)?
        > 0;
    session.flush_and_commit().map_err(map_storage_error)?;
    Ok(deleted)
}

pub(crate) fn delete_form_instances_by_form_definition(
    store: &RuntimeStore,
    form_definition_id: &str,
) -> Result<usize, FlowableError> {
    delete_form_instances_by_column(store, "form_definition_id", form_definition_id)
}

pub(crate) fn delete_form_instances_by_process_definition(
    store: &RuntimeStore,
    process_definition_id: &str,
) -> Result<usize, FlowableError> {
    delete_form_instances_by_column(store, "process_definition_id", process_definition_id)
}

pub(crate) fn delete_form_instances_by_scope_definition(
    store: &RuntimeStore,
    scope_definition_id: &str,
) -> Result<usize, FlowableError> {
    delete_form_instances_by_column(store, "scope_definition_id", scope_definition_id)
}

fn delete_form_instances_by_column(
    store: &RuntimeStore,
    column: &str,
    value: &str,
) -> Result<usize, FlowableError> {
    let mut session = store
        .db_store()
        .create_session()
        .map_err(map_storage_error)?;
    ensure_form_schema_in_session(&mut session).map_err(map_storage_error)?;
    let mut params = DbParams::new();
    params.push(value);
    let deleted = session
        .execute_raw(
            &format!("DELETE FROM {FORM_INSTANCES_TABLE} WHERE {column} = ?"),
            params,
        )
        .map_err(map_storage_error)? as usize;
    session.flush_and_commit().map_err(map_storage_error)?;
    Ok(deleted)
}

fn map_storage_error(error: flowable_engine::persistence::StorageError) -> FlowableError {
    FlowableError::Internal(format!("Database error: {}", error))
}

#[cfg(test)]
mod tests {
    #[test]
    fn repository_operations_do_not_repeat_schema_ddl() {
        let source = include_str!("repository.rs");
        assert_eq!(
            source
                .matches(&["    ensure_", "schema(store);"].concat())
                .count(),
            0
        );
    }
}
