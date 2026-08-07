use crate::db_session::DbSession;
use crate::entity::{Entity, EntityType};
use crate::error::PersistenceError;
use crate::row::DbRow;
use crate::statement::StatementId;
use crate::value::DbParams;
use std::any::Any;

#[derive(Debug, Clone)]
pub struct CmmnJobEntity {
    pub id: String,
    pub family: String,
    pub state: String,
    pub scope_id: String,
    pub sub_scope_id: Option<String>,
    pub scope_definition_id: String,
    pub element_id: String,
    pub tenant_id: Option<String>,
    pub due_date: Option<String>,
    pub lock_owner: Option<String>,
    pub retries: i32,
    pub exception_message: Option<String>,
    pub exception_stacktrace: Option<String>,
    pub created_at: i64,
    pub data: String,
}

impl CmmnJobEntity {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: String,
        family: String,
        state: String,
        scope_id: String,
        scope_definition_id: String,
        element_id: String,
        retries: i32,
        created_at: i64,
        data: String,
    ) -> Self {
        Self {
            id,
            family,
            state,
            scope_id,
            sub_scope_id: None,
            scope_definition_id,
            element_id,
            tenant_id: None,
            due_date: None,
            lock_owner: None,
            retries,
            exception_message: None,
            exception_stacktrace: None,
            created_at,
            data,
        }
    }

    pub fn set_sub_scope_id(&mut self, sub_scope_id: Option<String>) {
        self.sub_scope_id = sub_scope_id;
    }

    pub fn set_tenant_id(&mut self, tenant_id: Option<String>) {
        self.tenant_id = tenant_id;
    }

    pub fn set_due_date(&mut self, due_date: Option<String>) {
        self.due_date = due_date;
    }

    pub fn set_lock_owner(&mut self, lock_owner: Option<String>) {
        self.lock_owner = lock_owner;
    }

    pub fn set_exception_message(&mut self, exception_message: Option<String>) {
        self.exception_message = exception_message;
    }

    pub fn set_exception_stacktrace(&mut self, exception_stacktrace: Option<String>) {
        self.exception_stacktrace = exception_stacktrace;
    }

    pub fn from_row(row: &DbRow) -> Result<Self, PersistenceError> {
        Ok(Self {
            id: row.get_text("ID_").ok_or_else(|| {
                PersistenceError::Deserialization("Missing ID_ in CmmnJobEntity".to_string())
            })?,
            family: row.get_text("FAMILY_").ok_or_else(|| {
                PersistenceError::Deserialization("Missing FAMILY_ in CmmnJobEntity".to_string())
            })?,
            state: row.get_text("STATE_").ok_or_else(|| {
                PersistenceError::Deserialization("Missing STATE_ in CmmnJobEntity".to_string())
            })?,
            scope_id: row.get_text("SCOPE_ID_").ok_or_else(|| {
                PersistenceError::Deserialization("Missing SCOPE_ID_ in CmmnJobEntity".to_string())
            })?,
            sub_scope_id: row.get_text("SUB_SCOPE_ID_"),
            scope_definition_id: row.get_text("SCOPE_DEFINITION_ID_").ok_or_else(|| {
                PersistenceError::Deserialization(
                    "Missing SCOPE_DEFINITION_ID_ in CmmnJobEntity".to_string(),
                )
            })?,
            element_id: row.get_text("ELEMENT_ID_").ok_or_else(|| {
                PersistenceError::Deserialization(
                    "Missing ELEMENT_ID_ in CmmnJobEntity".to_string(),
                )
            })?,
            tenant_id: row.get_text("TENANT_ID_"),
            due_date: row.get_text("DUE_DATE_"),
            lock_owner: row.get_text("LOCK_OWNER_"),
            retries: row.get_integer("RETRIES_").unwrap_or(0) as i32,
            exception_message: row.get_text("EXCEPTION_MESSAGE_"),
            exception_stacktrace: row.get_text("EXCEPTION_STACKTRACE_"),
            created_at: row.get_integer("CREATED_AT_").unwrap_or(0),
            data: row.get_text("DATA_").ok_or_else(|| {
                PersistenceError::Deserialization("Missing DATA_ in CmmnJobEntity".to_string())
            })?,
        })
    }
}

impl Entity for CmmnJobEntity {
    fn id(&self) -> &str {
        &self.id
    }

    fn set_id(&mut self, id: String) {
        self.id = id;
    }

    fn entity_type(&self) -> EntityType {
        EntityType::CmmnJob
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn clone_box(&self) -> Box<dyn Entity> {
        Box::new(self.clone())
    }
}

pub struct CmmnJobDataManager;

impl CmmnJobDataManager {
    pub fn new() -> Self {
        Self
    }

    pub fn insert(
        &self,
        session: &mut DbSession,
        entity: CmmnJobEntity,
    ) -> Result<(), PersistenceError> {
        let mut params = DbParams::new();
        params.push(entity.id.clone());
        params.push(entity.family.clone());
        params.push(entity.state.clone());
        params.push(entity.scope_id.clone());
        params.push(entity.sub_scope_id.clone());
        params.push(entity.scope_definition_id.clone());
        params.push(entity.element_id.clone());
        params.push(entity.tenant_id.clone());
        params.push(entity.due_date.clone());
        params.push(entity.lock_owner.clone());
        params.push(entity.retries as i64);
        params.push(entity.exception_message.clone());
        params.push(entity.exception_stacktrace.clone());
        params.push(entity.created_at);
        params.push(entity.data.clone());

        session.insert(entity, StatementId::InsertCmmnJob, params)
    }

    pub fn delete(
        &self,
        session: &mut DbSession,
        entity: &CmmnJobEntity,
    ) -> Result<(), PersistenceError> {
        let mut params = DbParams::new();
        params.push(entity.id.clone());

        session.delete(entity, StatementId::DeleteCmmnJob, params)
    }

    pub fn find_by_id(
        &self,
        session: &mut DbSession,
        id: &str,
    ) -> Result<Option<CmmnJobEntity>, PersistenceError> {
        let mut params = DbParams::new();
        params.push(id);

        let row = session.select_one(StatementId::SelectCmmnJobById, params)?;
        match row {
            Some(row) => Ok(Some(CmmnJobEntity::from_row(&row)?)),
            None => Ok(None),
        }
    }

    pub fn find_by_scope_id(
        &self,
        session: &mut DbSession,
        scope_id: &str,
    ) -> Result<Vec<CmmnJobEntity>, PersistenceError> {
        let mut params = DbParams::new();
        params.push(scope_id);

        let rows = session.select_list(StatementId::SelectCmmnJobsByScopeId, params)?;
        rows.iter().map(CmmnJobEntity::from_row).collect()
    }
}

impl Default for CmmnJobDataManager {
    fn default() -> Self {
        Self::new()
    }
}
