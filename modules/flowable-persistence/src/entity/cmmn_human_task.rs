use crate::db_session::DbSession;
use crate::entity::{Entity, EntityType};
use crate::error::PersistenceError;
use crate::row::DbRow;
use crate::statement::StatementId;
use crate::value::DbParams;
use std::any::Any;

#[derive(Debug, Clone)]
pub struct CmmnHumanTaskEntity {
    pub id: String,
    pub case_instance_id: String,
    pub case_definition_id: String,
    pub case_key: String,
    pub stage_instance_id: Option<String>,
    pub state: String,
    pub activated_at: String,
    pub data: String,
}

impl CmmnHumanTaskEntity {
    pub fn new(
        id: String,
        case_instance_id: String,
        case_definition_id: String,
        case_key: String,
        state: String,
        activated_at: String,
        data: String,
    ) -> Self {
        Self {
            id,
            case_instance_id,
            case_definition_id,
            case_key,
            stage_instance_id: None,
            state,
            activated_at,
            data,
        }
    }

    pub fn set_stage_instance_id(&mut self, stage_instance_id: Option<String>) {
        self.stage_instance_id = stage_instance_id;
    }

    pub fn from_row(row: &DbRow) -> Result<Self, PersistenceError> {
        Ok(Self {
            id: row.get_text("ID_").ok_or_else(|| {
                PersistenceError::Deserialization("Missing ID_ in CmmnHumanTaskEntity".to_string())
            })?,
            case_instance_id: row.get_text("CASE_INSTANCE_ID_").ok_or_else(|| {
                PersistenceError::Deserialization(
                    "Missing CASE_INSTANCE_ID_ in CmmnHumanTaskEntity".to_string(),
                )
            })?,
            case_definition_id: row.get_text("CASE_DEFINITION_ID_").ok_or_else(|| {
                PersistenceError::Deserialization(
                    "Missing CASE_DEFINITION_ID_ in CmmnHumanTaskEntity".to_string(),
                )
            })?,
            case_key: row.get_text("CASE_KEY_").ok_or_else(|| {
                PersistenceError::Deserialization(
                    "Missing CASE_KEY_ in CmmnHumanTaskEntity".to_string(),
                )
            })?,
            stage_instance_id: row.get_text("STAGE_INSTANCE_ID_"),
            state: row.get_text("STATE_").ok_or_else(|| {
                PersistenceError::Deserialization(
                    "Missing STATE_ in CmmnHumanTaskEntity".to_string(),
                )
            })?,
            activated_at: row.get_text("ACTIVATED_AT_").ok_or_else(|| {
                PersistenceError::Deserialization(
                    "Missing ACTIVATED_AT_ in CmmnHumanTaskEntity".to_string(),
                )
            })?,
            data: row.get_text("DATA_").ok_or_else(|| {
                PersistenceError::Deserialization(
                    "Missing DATA_ in CmmnHumanTaskEntity".to_string(),
                )
            })?,
        })
    }
}

impl Entity for CmmnHumanTaskEntity {
    fn id(&self) -> &str {
        &self.id
    }

    fn set_id(&mut self, id: String) {
        self.id = id;
    }

    fn entity_type(&self) -> EntityType {
        EntityType::CmmnHumanTask
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn clone_box(&self) -> Box<dyn Entity> {
        Box::new(self.clone())
    }
}

pub struct CmmnHumanTaskDataManager;

impl CmmnHumanTaskDataManager {
    pub fn new() -> Self {
        Self
    }

    pub fn insert(
        &self,
        session: &mut DbSession,
        entity: CmmnHumanTaskEntity,
    ) -> Result<(), PersistenceError> {
        let mut params = DbParams::new();
        params.push(entity.id.clone());
        params.push(entity.case_instance_id.clone());
        params.push(entity.case_definition_id.clone());
        params.push(entity.case_key.clone());
        params.push(entity.stage_instance_id.clone());
        params.push(entity.state.clone());
        params.push(entity.activated_at.clone());
        params.push(entity.data.clone());

        session.insert(entity, StatementId::InsertCmmnHumanTask, params)
    }

    pub fn delete(
        &self,
        session: &mut DbSession,
        entity: &CmmnHumanTaskEntity,
    ) -> Result<(), PersistenceError> {
        let mut params = DbParams::new();
        params.push(entity.id.clone());

        session.delete(entity, StatementId::DeleteCmmnHumanTask, params)
    }

    pub fn find_by_id(
        &self,
        session: &mut DbSession,
        id: &str,
    ) -> Result<Option<CmmnHumanTaskEntity>, PersistenceError> {
        let mut params = DbParams::new();
        params.push(id);

        let row = session.select_one(StatementId::SelectCmmnHumanTaskById, params)?;
        match row {
            Some(row) => Ok(Some(CmmnHumanTaskEntity::from_row(&row)?)),
            None => Ok(None),
        }
    }

    pub fn find_by_case_instance_id(
        &self,
        session: &mut DbSession,
        case_instance_id: &str,
    ) -> Result<Vec<CmmnHumanTaskEntity>, PersistenceError> {
        let mut params = DbParams::new();
        params.push(case_instance_id);

        let rows =
            session.select_list(StatementId::SelectCmmnHumanTasksByCaseInstanceId, params)?;
        rows.iter().map(CmmnHumanTaskEntity::from_row).collect()
    }
}

impl Default for CmmnHumanTaskDataManager {
    fn default() -> Self {
        Self::new()
    }
}
