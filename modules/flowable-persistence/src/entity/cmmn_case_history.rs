use crate::db_session::DbSession;
use crate::entity::{Entity, EntityType};
use crate::error::PersistenceError;
use crate::row::DbRow;
use crate::statement::StatementId;
use crate::value::DbParams;
use std::any::Any;

#[derive(Debug, Clone)]
pub struct CmmnCaseHistoryEntity {
    pub case_instance_id: String,
    pub case_definition_id: String,
    pub case_key: String,
    pub tenant_id: Option<String>,
    pub business_key: Option<String>,
    pub state: String,
    pub started_at: String,
    pub completed_at: Option<String>,
    pub data: String,
}

impl CmmnCaseHistoryEntity {
    pub fn new(
        case_instance_id: String,
        case_definition_id: String,
        case_key: String,
        state: String,
        started_at: String,
        data: String,
    ) -> Self {
        Self {
            case_instance_id,
            case_definition_id,
            case_key,
            tenant_id: None,
            business_key: None,
            state,
            started_at,
            completed_at: None,
            data,
        }
    }

    pub fn set_tenant_id(&mut self, tenant_id: Option<String>) {
        self.tenant_id = tenant_id;
    }

    pub fn set_business_key(&mut self, business_key: Option<String>) {
        self.business_key = business_key;
    }

    pub fn set_completed_at(&mut self, completed_at: Option<String>) {
        self.completed_at = completed_at;
    }

    pub fn from_row(row: &DbRow) -> Result<Self, PersistenceError> {
        Ok(Self {
            case_instance_id: row.get_text("CASE_INSTANCE_ID_").ok_or_else(|| {
                PersistenceError::Deserialization(
                    "Missing CASE_INSTANCE_ID_ in CmmnCaseHistoryEntity".to_string(),
                )
            })?,
            case_definition_id: row.get_text("CASE_DEFINITION_ID_").ok_or_else(|| {
                PersistenceError::Deserialization(
                    "Missing CASE_DEFINITION_ID_ in CmmnCaseHistoryEntity".to_string(),
                )
            })?,
            case_key: row.get_text("CASE_KEY_").ok_or_else(|| {
                PersistenceError::Deserialization(
                    "Missing CASE_KEY_ in CmmnCaseHistoryEntity".to_string(),
                )
            })?,
            tenant_id: row.get_text("TENANT_ID_"),
            business_key: row.get_text("BUSINESS_KEY_"),
            state: row.get_text("STATE_").ok_or_else(|| {
                PersistenceError::Deserialization(
                    "Missing STATE_ in CmmnCaseHistoryEntity".to_string(),
                )
            })?,
            started_at: row.get_text("STARTED_AT_").ok_or_else(|| {
                PersistenceError::Deserialization(
                    "Missing STARTED_AT_ in CmmnCaseHistoryEntity".to_string(),
                )
            })?,
            completed_at: row.get_text("COMPLETED_AT_"),
            data: row.get_text("DATA_").ok_or_else(|| {
                PersistenceError::Deserialization(
                    "Missing DATA_ in CmmnCaseHistoryEntity".to_string(),
                )
            })?,
        })
    }
}

impl Entity for CmmnCaseHistoryEntity {
    fn id(&self) -> &str {
        &self.case_instance_id
    }

    fn set_id(&mut self, id: String) {
        self.case_instance_id = id;
    }

    fn entity_type(&self) -> EntityType {
        EntityType::CmmnCaseHistory
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn clone_box(&self) -> Box<dyn Entity> {
        Box::new(self.clone())
    }
}

pub struct CmmnCaseHistoryDataManager;

impl CmmnCaseHistoryDataManager {
    pub fn new() -> Self {
        Self
    }

    pub fn insert(
        &self,
        session: &mut DbSession,
        entity: CmmnCaseHistoryEntity,
    ) -> Result<(), PersistenceError> {
        let mut params = DbParams::new();
        params.push(entity.case_instance_id.clone());
        params.push(entity.case_definition_id.clone());
        params.push(entity.case_key.clone());
        params.push(entity.tenant_id.clone());
        params.push(entity.business_key.clone());
        params.push(entity.state.clone());
        params.push(entity.started_at.clone());
        params.push(entity.completed_at.clone());
        params.push(entity.data.clone());

        session.insert(entity, StatementId::InsertCmmnCaseHistory, params)
    }

    pub fn delete(
        &self,
        session: &mut DbSession,
        entity: &CmmnCaseHistoryEntity,
    ) -> Result<(), PersistenceError> {
        let mut params = DbParams::new();
        params.push(entity.case_instance_id.clone());

        session.delete(entity, StatementId::DeleteCmmnCaseHistory, params)
    }

    pub fn find_by_id(
        &self,
        session: &mut DbSession,
        id: &str,
    ) -> Result<Option<CmmnCaseHistoryEntity>, PersistenceError> {
        let mut params = DbParams::new();
        params.push(id);

        let row = session.select_one(StatementId::SelectCmmnCaseHistoryById, params)?;
        match row {
            Some(row) => Ok(Some(CmmnCaseHistoryEntity::from_row(&row)?)),
            None => Ok(None),
        }
    }
}

impl Default for CmmnCaseHistoryDataManager {
    fn default() -> Self {
        Self::new()
    }
}
