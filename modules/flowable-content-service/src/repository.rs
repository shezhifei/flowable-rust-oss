use crate::models::ContentItem;
use flowable_engine::persistence::DbParams;
use flowable_engine::persistence::db_session::DbSession;
use flowable_engine::persistence::runtime_store::RuntimeStore;

const CONTENT_ITEMS_TABLE: &str = "m14_content_items";
const CONTENT_ITEM_DATA_TABLE: &str = "m14_content_item_data";

/// Column order of the fully-projected content-item upsert. Both insert paths
/// push their params in exactly this order.
const CONTENT_ITEM_COLUMNS: &[&str] = &[
    "id",
    "data",
    "name",
    "mime_type",
    "task_id",
    "process_instance_id",
    "scope_type",
    "scope_id",
    "field",
    "tenant_id",
    "created_by",
    "created_at",
    "updated_at",
    "expires_at",
];

pub fn ensure_schema(store: &RuntimeStore) {
    let mut session = store.db_store().create_session().unwrap();

    let id = session.dialect().varchar_type(255);
    let short = session.dialect().varchar_type(255);
    let text = session.dialect().text_type();
    let blob = session.dialect().blob_type();
    let big = session.dialect().bigint_type();

    // execute_raw_sql 只能处理单条语句，逐条执行 DDL
    session
        .execute_raw_sql(&format!(
            "CREATE TABLE IF NOT EXISTS {CONTENT_ITEMS_TABLE} (id {id} PRIMARY KEY, data {text} NOT NULL, name {short} NOT NULL, mime_type {short}, task_id {short}, process_instance_id {short}, scope_type {short}, scope_id {short}, field {short}, tenant_id {short}, created_by {short}, created_at {big} NOT NULL, updated_at {big} NOT NULL, expires_at {big})"
        ))
        .unwrap();
    session
        .execute_raw_sql(&format!(
            "CREATE TABLE IF NOT EXISTS {CONTENT_ITEM_DATA_TABLE} (content_item_id {id} PRIMARY KEY, payload {blob} NOT NULL)"
        ))
        .unwrap();

    create_index(&mut session, "idx_content_items_name", CONTENT_ITEMS_TABLE, "name");
    create_index(
        &mut session,
        "idx_content_items_mime_type",
        CONTENT_ITEMS_TABLE,
        "mime_type",
    );
    create_index(
        &mut session,
        "idx_content_items_task_id",
        CONTENT_ITEMS_TABLE,
        "task_id",
    );
    create_index(
        &mut session,
        "idx_content_items_process_instance_id",
        CONTENT_ITEMS_TABLE,
        "process_instance_id",
    );
    create_index(
        &mut session,
        "idx_content_items_scope",
        CONTENT_ITEMS_TABLE,
        "scope_type, scope_id",
    );
    create_index(
        &mut session,
        "idx_content_items_created_by",
        CONTENT_ITEMS_TABLE,
        "created_by",
    );
    create_index(
        &mut session,
        "idx_content_items_created_at",
        CONTENT_ITEMS_TABLE,
        "created_at",
    );
    create_index(
        &mut session,
        "idx_content_items_expires_at",
        CONTENT_ITEMS_TABLE,
        "expires_at",
    );
    create_index(
        &mut session,
        "idx_content_items_field",
        CONTENT_ITEMS_TABLE,
        "field",
    );
    create_index(
        &mut session,
        "idx_content_items_tenant_id",
        CONTENT_ITEMS_TABLE,
        "tenant_id",
    );

    migrate_content_item_columns(&mut session);

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

fn migrate_content_item_columns(session: &mut DbSession) {
    let column_names: std::collections::BTreeSet<String> = session
        .table_columns(CONTENT_ITEMS_TABLE)
        .unwrap()
        .into_iter()
        .map(|column| column.name)
        .collect();

    for (column, ddl_type) in [("field", "TEXT"), ("tenant_id", "TEXT")] {
        if !column_names.contains(column) {
            session
                .execute_raw_sql(&format!(
                    "ALTER TABLE {CONTENT_ITEMS_TABLE} ADD COLUMN {column} {ddl_type}"
                ))
                .unwrap();
        }
    }
}

pub fn insert_content_item(
    store: &RuntimeStore,
    item: ContentItem,
    payload: Option<&[u8]>,
) -> Result<(), flowable_engine::persistence::StorageError> {
    ensure_schema(store);
    let mut session = store.db_store().create_session()?;

    let mut params = DbParams::new();
    params.push(item.id.clone());
    params.push(serde_json::to_string(&item).unwrap());
    params.push(item.name.clone());
    params.push(item.mime_type.clone());
    params.push(item.task_id.clone());
    params.push(item.process_instance_id.clone());
    params.push(item.scope_type.clone());
    params.push(item.scope_id.clone());
    params.push(item.field.clone());
    params.push(item.tenant_id.clone());
    params.push(item.created_by.clone());
    params.push(item.created_at);
    params.push(item.updated_at);
    params.push(item.expires_at);

    session.upsert_raw(
        CONTENT_ITEMS_TABLE,
        "id",
        CONTENT_ITEM_COLUMNS,
        params,
    )?;

    if let Some(payload) = payload {
        let mut blob_params = DbParams::new();
        blob_params.push(item.id.clone());
        blob_params.push(payload.to_vec());
        session.upsert_raw(
            CONTENT_ITEM_DATA_TABLE,
            "content_item_id",
            &["content_item_id", "payload"],
            blob_params,
        )?;
    } else {
        let mut del_params = DbParams::new();
        del_params.push(item.id.clone());
        session.execute_raw(
            &format!("DELETE FROM {CONTENT_ITEM_DATA_TABLE} WHERE content_item_id = ?"),
            del_params,
        )?;
    }

    session.flush_and_commit()?;
    Ok(())
}

pub(crate) fn find_content_item(store: &RuntimeStore, id: &str) -> Option<ContentItem> {
    ensure_schema(store);
    store
        .db_store()
        .find_by_id(CONTENT_ITEMS_TABLE, id)
        .unwrap()
}

pub(crate) fn find_content_items_by_filter(
    store: &RuntimeStore,
    predicate: &str,
    params: DbParams,
) -> Vec<ContentItem> {
    ensure_schema(store);
    let mut session = store.db_store().create_session().unwrap();
    let sql = format!("SELECT data FROM {CONTENT_ITEMS_TABLE} WHERE {predicate}");
    let rows = session.raw_query(&sql, params).unwrap();
    rows.iter()
        .filter_map(|row| row.get_text("data"))
        .filter_map(|json| serde_json::from_str::<ContentItem>(json.as_str()).ok())
        .collect()
}

pub(crate) fn list_content_items(store: &RuntimeStore) -> Vec<ContentItem> {
    ensure_schema(store);
    store.db_store().find_all(CONTENT_ITEMS_TABLE).unwrap()
}

pub(crate) fn delete_content_item(store: &RuntimeStore, id: &str) -> bool {
    ensure_schema(store);
    let mut session = store.db_store().create_session().unwrap();

    let mut del_blob_params = DbParams::new();
    del_blob_params.push(id);
    session
        .execute_raw(
            &format!("DELETE FROM {CONTENT_ITEM_DATA_TABLE} WHERE content_item_id = ?"),
            del_blob_params,
        )
        .unwrap();

    let mut del_item_params = DbParams::new();
    del_item_params.push(id);
    let deleted = session
        .execute_raw(
            &format!("DELETE FROM {CONTENT_ITEMS_TABLE} WHERE id = ?"),
            del_item_params,
        )
        .unwrap()
        > 0;
    session.flush_and_commit().unwrap();
    deleted
}

pub(crate) fn delete_content_items_by_process_instance_id(
    store: &RuntimeStore,
    process_instance_id: &str,
) -> usize {
    let mut params = DbParams::new();
    params.push(process_instance_id);
    delete_content_items_by_filter(store, "process_instance_id = ?", params)
}

pub(crate) fn delete_content_items_by_task_id(store: &RuntimeStore, task_id: &str) -> usize {
    let mut params = DbParams::new();
    params.push(task_id);
    delete_content_items_by_filter(store, "task_id = ?", params)
}

pub(crate) fn delete_content_items_by_scope_id_and_scope_type(
    store: &RuntimeStore,
    scope_id: &str,
    scope_type: &str,
) -> usize {
    let mut params = DbParams::new();
    params.push(scope_id);
    params.push(scope_type);
    delete_content_items_by_filter(store, "scope_id = ? AND scope_type = ?", params)
}

fn delete_content_items_by_filter(
    store: &RuntimeStore,
    predicate: &str,
    params: DbParams,
) -> usize {
    ensure_schema(store);
    let mut session = store.db_store().create_session().unwrap();

    let sql = format!("SELECT id FROM {CONTENT_ITEMS_TABLE} WHERE {predicate}");
    let rows = session.raw_query(&sql, params).unwrap();
    let content_item_ids: Vec<String> = rows.iter().filter_map(|row| row.get_text("id")).collect();

    let deleted = content_item_ids.len();
    for content_item_id in &content_item_ids {
        let mut del_blob_params = DbParams::new();
        del_blob_params.push(content_item_id.as_str());
        session
            .execute_raw(
                &format!("DELETE FROM {CONTENT_ITEM_DATA_TABLE} WHERE content_item_id = ?"),
                del_blob_params,
            )
            .unwrap();

        let mut del_item_params = DbParams::new();
        del_item_params.push(content_item_id.as_str());
        session
            .execute_raw(
                &format!("DELETE FROM {CONTENT_ITEMS_TABLE} WHERE id = ?"),
                del_item_params,
            )
            .unwrap();
    }
    session.flush_and_commit().unwrap();
    deleted
}

pub(crate) fn find_expired_content_items(
    store: &RuntimeStore,
    now_millis: i64,
) -> Vec<ContentItem> {
    let mut params = DbParams::new();
    params.push(now_millis);
    find_content_items_by_filter(store, "expires_at IS NOT NULL AND expires_at <= ?", params)
}

/// Ensure content tables exist using the caller's session (no nested commit).
/// Preferred inside engine commands so schema + writes share one transaction when
/// the dialect supports transactional DDL; otherwise DDL may auto-commit.
pub fn ensure_schema_in_session(
    session: &mut DbSession,
) -> Result<(), flowable_engine::persistence::StorageError> {
    session.execute_raw_sql(&format!(
        "CREATE TABLE IF NOT EXISTS {CONTENT_ITEMS_TABLE} (id TEXT PRIMARY KEY, data TEXT NOT NULL, name TEXT NOT NULL, mime_type TEXT, task_id TEXT, process_instance_id TEXT, scope_type TEXT, scope_id TEXT, field TEXT, tenant_id TEXT, created_by TEXT, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL, expires_at INTEGER)"
    ))?;
    session.execute_raw_sql(&format!(
        "CREATE TABLE IF NOT EXISTS {CONTENT_ITEM_DATA_TABLE} (content_item_id TEXT PRIMARY KEY, payload BLOB NOT NULL)"
    ))?;
    let _ = session.execute_raw_sql(&format!(
        "ALTER TABLE {CONTENT_ITEMS_TABLE} ADD COLUMN field TEXT"
    ));
    let _ = session.execute_raw_sql(&format!(
        "ALTER TABLE {CONTENT_ITEMS_TABLE} ADD COLUMN tenant_id TEXT"
    ));
    Ok(())
}

/// Insert content item (+ optional binary payload) on the caller's session.
/// Used by `CreateTaskAttachmentCmd` so content row + task event share one commit.
pub fn insert_content_item_in_session(
    session: &mut DbSession,
    item: &ContentItem,
    payload: Option<&[u8]>,
) -> Result<(), flowable_engine::persistence::StorageError> {
    ensure_schema_in_session(session)?;

    let mut params = DbParams::new();
    params.push(item.id.clone());
    params.push(serde_json::to_string(item).unwrap());
    params.push(item.name.clone());
    params.push(item.mime_type.clone());
    params.push(item.task_id.clone());
    params.push(item.process_instance_id.clone());
    params.push(item.scope_type.clone());
    params.push(item.scope_id.clone());
    params.push(item.field.clone());
    params.push(item.tenant_id.clone());
    params.push(item.created_by.clone());
    params.push(item.created_at);
    params.push(item.updated_at);
    params.push(item.expires_at);

    session.upsert_raw(
        CONTENT_ITEMS_TABLE,
        "id",
        CONTENT_ITEM_COLUMNS,
        params,
    )?;

    if let Some(payload) = payload {
        let mut blob_params = DbParams::new();
        blob_params.push(item.id.clone());
        blob_params.push(payload.to_vec());
        session.upsert_raw(
            CONTENT_ITEM_DATA_TABLE,
            "content_item_id",
            &["content_item_id", "payload"],
            blob_params,
        )?;
    }

    Ok(())
}

/// Associate an existing content item with task/process/scope/field/tenant on the
/// caller's session (Java form upload association via ContentService.saveContentItem).
///
/// WARNING: unconditionally overwrites every ownership column including
/// `tenant_id`. Only intended for explicit administrative/seed operations;
/// form submissions must go through [`claim_content_item_for_field_in_session`]
/// which enforces strict tenant + ownership checks.
pub fn associate_content_item_in_session(
    session: &mut DbSession,
    content_item_id: &str,
    task_id: Option<&str>,
    process_instance_id: Option<&str>,
    scope_id: Option<&str>,
    scope_type: Option<&str>,
    field: Option<&str>,
    tenant_id: Option<&str>,
) -> Result<ContentItem, flowable_engine::persistence::StorageError> {
    let mut item = find_content_item_in_session(session, content_item_id)?.ok_or_else(|| {
        flowable_engine::persistence::StorageError::Persistence(format!(
            "Content item '{content_item_id}' was not found"
        ))
    })?;

    item.task_id = task_id.map(str::to_string);
    item.process_instance_id = process_instance_id.map(str::to_string);
    item.scope_id = scope_id.map(str::to_string);
    item.scope_type = scope_type.map(str::to_string);
    item.field = field.map(str::to_string);
    item.tenant_id = tenant_id.map(str::to_string);

    // Rewrite full JSON + physical association columns without touching payload.
    let mut params = DbParams::new();
    params.push(serde_json::to_string(&item).unwrap());
    params.push(item.task_id.clone().unwrap_or_default());
    params.push(item.process_instance_id.clone().unwrap_or_default());
    params.push(item.scope_type.clone().unwrap_or_default());
    params.push(item.scope_id.clone().unwrap_or_default());
    params.push(item.field.clone().unwrap_or_default());
    params.push(item.tenant_id.clone().unwrap_or_default());
    params.push(content_item_id);
    session.execute_raw(
        &format!(
            "UPDATE {CONTENT_ITEMS_TABLE}
             SET data = ?, task_id = ?, process_instance_id = ?, scope_type = ?,
                 scope_id = ?, field = ?, tenant_id = ?
             WHERE id = ?"
        ),
        params,
    )?;
    Ok(item)
}

/// Errors surfaced by [`claim_content_item_for_field_in_session`]; callers map
/// them onto their own API error space (form submit → NotFound/BadRequest/Conflict).
#[derive(Debug)]
pub enum ContentClaimError {
    /// No content item exists with the requested id.
    NotFound,
    /// The submitting context and the content item belong to different tenants
    /// (a tenant context may only reference content of the same tenant and a
    /// tenantless context may only reference tenantless content).
    TenantMismatch { item_tenant: Option<String> },
    /// The content item is already associated with another task/process/scope/field.
    AlreadyAssociated,
    /// Another transaction claimed the item between the read and the guarded update.
    ConcurrentClaim,
    /// Underlying storage failure.
    Storage(flowable_engine::persistence::StorageError),
}

impl From<flowable_engine::persistence::StorageError> for ContentClaimError {
    fn from(error: flowable_engine::persistence::StorageError) -> Self {
        ContentClaimError::Storage(error)
    }
}

/// Treat NULL and empty string as "no owner"/"no tenant" — physical columns may
/// hold '' from older association writes while the JSON data column holds None.
fn normalize_owner(value: Option<&str>) -> &str {
    value.unwrap_or("")
}

/// Claim an unowned content item for a form upload field on the caller's session.
///
/// Strict, symmetric ownership rules (tenant-isolation P1 fix):
/// - a tenant context may only reference content of the same tenant;
/// - a tenantless context may only reference tenantless content;
/// - content already associated with a task/process/scope/field is rejected
///   unless the requested target is exactly the same (idempotent resubmit);
/// - the UPDATE carries old-owner + tenant conditions so two concurrent
///   submissions cannot both claim the same item;
/// - `tenant_id` is never rewritten: first-time tenant adoption of tenantless
///   content must be an explicit create/claim operation, not a form submit.
pub fn claim_content_item_for_field_in_session(
    session: &mut DbSession,
    content_item_id: &str,
    task_id: Option<&str>,
    process_instance_id: Option<&str>,
    scope_id: Option<&str>,
    scope_type: Option<&str>,
    field: Option<&str>,
    requested_tenant: Option<&str>,
) -> Result<ContentItem, ContentClaimError> {
    let mut item = find_content_item_in_session(session, content_item_id)?
        .ok_or(ContentClaimError::NotFound)?;

    let item_tenant = normalize_owner(item.tenant_id.as_deref()).to_string();
    let requested_tenant_norm = normalize_owner(requested_tenant).to_string();
    if item_tenant != requested_tenant_norm {
        return Err(ContentClaimError::TenantMismatch {
            item_tenant: item.tenant_id.clone(),
        });
    }

    let current_owner: [String; 5] = [
        normalize_owner(item.task_id.as_deref()).to_string(),
        normalize_owner(item.process_instance_id.as_deref()).to_string(),
        normalize_owner(item.scope_id.as_deref()).to_string(),
        normalize_owner(item.scope_type.as_deref()).to_string(),
        normalize_owner(item.field.as_deref()).to_string(),
    ];
    let requested_owner: [String; 5] = [
        normalize_owner(task_id).to_string(),
        normalize_owner(process_instance_id).to_string(),
        normalize_owner(scope_id).to_string(),
        normalize_owner(scope_type).to_string(),
        normalize_owner(field).to_string(),
    ];
    if current_owner.iter().any(|part| !part.is_empty()) {
        if current_owner == requested_owner {
            // Idempotent resubmit of the exact same association.
            return Ok(item);
        }
        return Err(ContentClaimError::AlreadyAssociated);
    }

    item.task_id = task_id.map(str::to_string);
    item.process_instance_id = process_instance_id.map(str::to_string);
    item.scope_id = scope_id.map(str::to_string);
    item.scope_type = scope_type.map(str::to_string);
    item.field = field.map(str::to_string);
    // tenant_id deliberately untouched: claiming a field never moves tenants.

    let mut params = DbParams::new();
    params.push(serde_json::to_string(&item).unwrap());
    params.push(item.task_id.clone().unwrap_or_default());
    params.push(item.process_instance_id.clone().unwrap_or_default());
    params.push(item.scope_type.clone().unwrap_or_default());
    params.push(item.scope_id.clone().unwrap_or_default());
    params.push(item.field.clone().unwrap_or_default());
    params.push(content_item_id);
    params.push(requested_tenant_norm);
    let updated = session.execute_raw(
        &format!(
            "UPDATE {CONTENT_ITEMS_TABLE}
             SET data = ?, task_id = ?, process_instance_id = ?, scope_type = ?,
                 scope_id = ?, field = ?
             WHERE id = ?
               AND COALESCE(task_id, '') = ''
               AND COALESCE(process_instance_id, '') = ''
               AND COALESCE(scope_id, '') = ''
               AND COALESCE(scope_type, '') = ''
               AND COALESCE(field, '') = ''
               AND COALESCE(tenant_id, '') = ?"
        ),
        params,
    )?;
    if updated == 0 {
        return Err(ContentClaimError::ConcurrentClaim);
    }
    Ok(item)
}

/// Find content items by a set of ids on the caller's session.
pub fn find_content_items_by_ids_in_session(
    session: &mut DbSession,
    ids: &[String],
) -> Result<Vec<ContentItem>, flowable_engine::persistence::StorageError> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    ensure_schema_in_session(session)?;
    let mut items = Vec::with_capacity(ids.len());
    for id in ids {
        if let Some(item) = find_content_item_in_session(session, id)? {
            items.push(item);
        }
    }
    Ok(items)
}

/// Find a content item by id using the caller's session.
pub fn find_content_item_in_session(
    session: &mut DbSession,
    id: &str,
) -> Result<Option<ContentItem>, flowable_engine::persistence::StorageError> {
    ensure_schema_in_session(session)?;
    let mut params = DbParams::new();
    params.push(id);
    let rows = session.raw_query(
        &format!("SELECT data FROM {CONTENT_ITEMS_TABLE} WHERE id = ?"),
        params,
    )?;
    Ok(rows
        .into_iter()
        .filter_map(|row| row.get_text("data"))
        .find_map(|json| serde_json::from_str::<ContentItem>(&json).ok()))
}

/// List content items for a task using the caller's session.
pub fn find_content_items_by_task_id_in_session(
    session: &mut DbSession,
    task_id: &str,
) -> Result<Vec<ContentItem>, flowable_engine::persistence::StorageError> {
    ensure_schema_in_session(session)?;
    let mut params = DbParams::new();
    params.push(task_id);
    let rows = session.raw_query(
        &format!("SELECT data FROM {CONTENT_ITEMS_TABLE} WHERE task_id = ?"),
        params,
    )?;
    Ok(rows
        .into_iter()
        .filter_map(|row| row.get_text("data"))
        .filter_map(|json| serde_json::from_str::<ContentItem>(&json).ok())
        .collect())
}

/// List content items for a process instance using the caller's session.
/// Java `findAttachmentsByProcessInstanceId` — same physical table as task scope.
pub fn find_content_items_by_process_instance_id_in_session(
    session: &mut DbSession,
    process_instance_id: &str,
) -> Result<Vec<ContentItem>, flowable_engine::persistence::StorageError> {
    ensure_schema_in_session(session)?;
    let mut params = DbParams::new();
    params.push(process_instance_id);
    let rows = session.raw_query(
        &format!("SELECT data FROM {CONTENT_ITEMS_TABLE} WHERE process_instance_id = ?"),
        params,
    )?;
    Ok(rows
        .into_iter()
        .filter_map(|row| row.get_text("data"))
        .filter_map(|json| serde_json::from_str::<ContentItem>(&json).ok())
        .collect())
}

/// Load binary payload stored in the session-backed blob table (task-attachment path).
pub fn find_content_item_payload_in_session(
    session: &mut DbSession,
    content_item_id: &str,
) -> Result<Option<Vec<u8>>, flowable_engine::persistence::StorageError> {
    ensure_schema_in_session(session)?;
    let mut params = DbParams::new();
    params.push(content_item_id);
    let rows = session.raw_query(
        &format!("SELECT payload FROM {CONTENT_ITEM_DATA_TABLE} WHERE content_item_id = ?"),
        params,
    )?;
    Ok(rows.into_iter().find_map(|row| row.get_blob("payload")))
}

/// Delete content item + blob payload on the caller's session.
pub fn delete_content_item_in_session(
    session: &mut DbSession,
    id: &str,
) -> Result<bool, flowable_engine::persistence::StorageError> {
    ensure_schema_in_session(session)?;

    let mut del_blob_params = DbParams::new();
    del_blob_params.push(id);
    session.execute_raw(
        &format!("DELETE FROM {CONTENT_ITEM_DATA_TABLE} WHERE content_item_id = ?"),
        del_blob_params,
    )?;

    let mut del_item_params = DbParams::new();
    del_item_params.push(id);
    let deleted = session.execute_raw(
        &format!("DELETE FROM {CONTENT_ITEMS_TABLE} WHERE id = ?"),
        del_item_params,
    )?;
    Ok(deleted > 0)
}
