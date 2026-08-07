use crate::db_session::DbSession;
use crate::entity::{Entity, EntityType};
use crate::error::PersistenceError;
use crate::row::DbRow;
use crate::statement::StatementId;
use crate::value::DbParams;
use std::any::Any;

#[derive(Debug, Clone)]
pub struct CmmnHumanTaskHistoryEntity {
    pub task_id: String,
    pub case_instance_id: String,
    pub case_definition_id: String,
    pub case_key: String,
    pub stage_instance_id: Option<String>,
    pub state: String,
    pub activated_at: String,
    pub completed_at: Option<String>,
    pub data: String,
}

impl CmmnHumanTaskHistoryEntity {
    pub fn new(
        task_id: String,
        case_instance_id: String,
        case_definition_id: String,
        case_key: String,
        state: String,
        activated_at: String,
        data: String,
    ) -> Self {
        Self {
            task_id,
            case_instance_id,
            case_definition_id,
            case_key,
            stage_instance_id: None,
            state,
            activated_at,
            completed_at: None,
            data,
        }
    }

    pub fn set_stage_instance_id(&mut self, stage_instance_id: Option<String>) {
        self.stage_instance_id = stage_instance_id;
    }

    pub fn set_completed_at(&mut self, completed_at: Option<String>) {
        self.completed_at = completed_at;
    }

    pub fn from_row(row: &DbRow) -> Result<Self, PersistenceError> {
        Ok(Self {
            task_id: row.get_text("TASK_ID_").ok_or_else(|| {
                PersistenceError::Deserialization(
                    "Missing TASK_ID_ in CmmnHumanTaskHistoryEntity".to_string(),
                )
            })?,
            case_instance_id: row.get_text("CASE_INSTANCE_ID_").ok_or_else(|| {
                PersistenceError::Deserialization(
                    "Missing CASE_INSTANCE_ID_ in CmmnHumanTaskHistoryEntity".to_string(),
                )
            })?,
            case_definition_id: row.get_text("CASE_DEFINITION_ID_").ok_or_else(|| {
                PersistenceError::Deserialization(
                    "Missing CASE_DEFINITION_ID_ in CmmnHumanTaskHistoryEntity".to_string(),
                )
            })?,
            case_key: row.get_text("CASE_KEY_").ok_or_else(|| {
                PersistenceError::Deserialization(
                    "Missing CASE_KEY_ in CmmnHumanTaskHistoryEntity".to_string(),
                )
            })?,
            stage_instance_id: row.get_text("STAGE_INSTANCE_ID_"),
            state: row.get_text("STATE_").ok_or_else(|| {
                PersistenceError::Deserialization(
                    "Missing STATE_ in CmmnHumanTaskHistoryEntity".to_string(),
                )
            })?,
            activated_at: row.get_text("ACTIVATED_AT_").ok_or_else(|| {
                PersistenceError::Deserialization(
                    "Missing ACTIVATED_AT_ in CmmnHumanTaskHistoryEntity".to_string(),
                )
            })?,
            completed_at: row.get_text("COMPLETED_AT_"),
            data: row.get_text("DATA_").ok_or_else(|| {
                PersistenceError::Deserialization(
                    "Missing DATA_ in CmmnHumanTaskHistoryEntity".to_string(),
                )
            })?,
        })
    }
}

impl Entity for CmmnHumanTaskHistoryEntity {
    fn id(&self) -> &str {
        &self.task_id
    }

    fn set_id(&mut self, id: String) {
        self.task_id = id;
    }

    fn entity_type(&self) -> EntityType {
        EntityType::CmmnHumanTaskHistory
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn clone_box(&self) -> Box<dyn Entity> {
        Box::new(self.clone())
    }
}

pub struct CmmnHumanTaskHistoryDataManager;

impl CmmnHumanTaskHistoryDataManager {
    pub fn new() -> Self {
        Self
    }

    pub fn insert(
        &self,
        session: &mut DbSession,
        entity: CmmnHumanTaskHistoryEntity,
    ) -> Result<(), PersistenceError> {
        let mut params = DbParams::new();
        params.push(entity.task_id.clone());
        params.push(entity.case_instance_id.clone());
        params.push(entity.case_definition_id.clone());
        params.push(entity.case_key.clone());
        params.push(entity.stage_instance_id.clone());
        params.push(entity.state.clone());
        params.push(entity.activated_at.clone());
        params.push(entity.completed_at.clone());
        params.push(entity.data.clone());

        session.insert(entity, StatementId::InsertCmmnHumanTaskHistory, params)
    }

    pub fn delete(
        &self,
        session: &mut DbSession,
        entity: &CmmnHumanTaskHistoryEntity,
    ) -> Result<(), PersistenceError> {
        let mut params = DbParams::new();
        params.push(entity.task_id.clone());

        session.delete(entity, StatementId::DeleteCmmnHumanTaskHistory, params)
    }

    pub fn find_by_id(
        &self,
        session: &mut DbSession,
        id: &str,
    ) -> Result<Option<CmmnHumanTaskHistoryEntity>, PersistenceError> {
        let mut params = DbParams::new();
        params.push(id);

        let row = session.select_one(StatementId::SelectCmmnHumanTaskHistoryById, params)?;
        match row {
            Some(row) => Ok(Some(CmmnHumanTaskHistoryEntity::from_row(&row)?)),
            None => Ok(None),
        }
    }

    pub fn find_by_case_instance_id(
        &self,
        session: &mut DbSession,
        case_instance_id: &str,
    ) -> Result<Vec<CmmnHumanTaskHistoryEntity>, PersistenceError> {
        let mut params = DbParams::new();
        params.push(case_instance_id);

        let rows = session.select_list(
            StatementId::SelectCmmnHumanTaskHistoryByCaseInstanceId,
            params,
        )?;
        rows.iter()
            .map(CmmnHumanTaskHistoryEntity::from_row)
            .collect()
    }
}

impl Default for CmmnHumanTaskHistoryDataManager {
    fn default() -> Self {
        Self::new()
    }
}
