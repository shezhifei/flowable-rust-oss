use crate::db_session::DbSession;
use crate::entity::{Entity, EntityType};
use crate::error::PersistenceError;
use crate::row::DbRow;
use crate::statement::StatementId;
use crate::value::DbParams;
use std::any::Any;

#[derive(Debug, Clone)]
pub struct CmmnCaseInstanceEntity {
    pub id: String,
    pub case_definition_id: String,
    pub case_key: String,
    pub tenant_id: Option<String>,
    pub business_key: Option<String>,
    pub state: String,
    pub started_at: String,
    pub data: String,
}

impl CmmnCaseInstanceEntity {
    pub fn new(
        id: String,
        case_definition_id: String,
        case_key: String,
        state: String,
        started_at: String,
        data: String,
    ) -> Self {
        Self {
            id,
            case_definition_id,
            case_key,
            tenant_id: None,
            business_key: None,
            state,
            started_at,
            data,
        }
    }

    pub fn set_tenant_id(&mut self, tenant_id: Option<String>) {
        self.tenant_id = tenant_id;
    }

    pub fn set_business_key(&mut self, business_key: Option<String>) {
        self.business_key = business_key;
    }

    pub fn from_row(row: &DbRow) -> Result<Self, PersistenceError> {
        Ok(Self {
            id: row.get_text("ID_").ok_or_else(|| {
                PersistenceError::Deserialization(
                    "Missing ID_ in CmmnCaseInstanceEntity".to_string(),
                )
            })?,
            case_definition_id: row.get_text("CASE_DEFINITION_ID_").ok_or_else(|| {
                PersistenceError::Deserialization(
                    "Missing CASE_DEFINITION_ID_ in CmmnCaseInstanceEntity".to_string(),
                )
            })?,
            case_key: row.get_text("CASE_KEY_").ok_or_else(|| {
                PersistenceError::Deserialization(
                    "Missing CASE_KEY_ in CmmnCaseInstanceEntity".to_string(),
                )
            })?,
            tenant_id: row.get_text("TENANT_ID_"),
            business_key: row.get_text("BUSINESS_KEY_"),
            state: row.get_text("STATE_").ok_or_else(|| {
                PersistenceError::Deserialization(
                    "Missing STATE_ in CmmnCaseInstanceEntity".to_string(),
                )
            })?,
            started_at: row.get_text("STARTED_AT_").ok_or_else(|| {
                PersistenceError::Deserialization(
                    "Missing STARTED_AT_ in CmmnCaseInstanceEntity".to_string(),
                )
            })?,
            data: row.get_text("DATA_").ok_or_else(|| {
                PersistenceError::Deserialization(
                    "Missing DATA_ in CmmnCaseInstanceEntity".to_string(),
                )
            })?,
        })
    }
}

impl Entity for CmmnCaseInstanceEntity {
    fn id(&self) -> &str {
        &self.id
    }

    fn set_id(&mut self, id: String) {
        self.id = id;
    }

    fn entity_type(&self) -> EntityType {
        EntityType::CmmnCaseInstance
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn clone_box(&self) -> Box<dyn Entity> {
        Box::new(self.clone())
    }
}

pub struct CmmnCaseInstanceDataManager;

impl CmmnCaseInstanceDataManager {
    pub fn new() -> Self {
        Self
    }

    pub fn insert(
        &self,
        session: &mut DbSession,
        entity: CmmnCaseInstanceEntity,
    ) -> Result<(), PersistenceError> {
        let mut params = DbParams::new();
        params.push(entity.id.clone());
        params.push(entity.case_definition_id.clone());
        params.push(entity.case_key.clone());
        params.push(entity.tenant_id.clone());
        params.push(entity.business_key.clone());
        params.push(entity.state.clone());
        params.push(entity.started_at.clone());
        params.push(entity.data.clone());

        session.insert(entity, StatementId::InsertCmmnCaseInstance, params)
    }

    pub fn delete(
        &self,
        session: &mut DbSession,
        entity: &CmmnCaseInstanceEntity,
    ) -> Result<(), PersistenceError> {
        let mut params = DbParams::new();
        params.push(entity.id.clone());

        session.delete(entity, StatementId::DeleteCmmnCaseInstance, params)
    }

    pub fn find_by_id(
        &self,
        session: &mut DbSession,
        id: &str,
    ) -> Result<Option<CmmnCaseInstanceEntity>, PersistenceError> {
        let mut params = DbParams::new();
        params.push(id);

        let row = session.select_one(StatementId::SelectCmmnCaseInstanceById, params)?;
        match row {
            Some(row) => Ok(Some(CmmnCaseInstanceEntity::from_row(&row)?)),
            None => Ok(None),
        }
    }

    pub fn find_by_case_definition_id(
        &self,
        session: &mut DbSession,
        case_definition_id: &str,
    ) -> Result<Vec<CmmnCaseInstanceEntity>, PersistenceError> {
        let mut params = DbParams::new();
        params.push(case_definition_id);

        let rows = session.select_list(
            StatementId::SelectCmmnCaseInstancesByCaseDefinitionId,
            params,
        )?;
        rows.iter().map(CmmnCaseInstanceEntity::from_row).collect()
    }
}

impl Default for CmmnCaseInstanceDataManager {
    fn default() -> Self {
        Self::new()
    }
}
