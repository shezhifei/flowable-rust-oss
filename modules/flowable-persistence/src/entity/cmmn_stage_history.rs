use crate::db_session::DbSession;
use crate::entity::{Entity, EntityType};
use crate::error::PersistenceError;
use crate::row::DbRow;
use crate::statement::StatementId;
use crate::value::DbParams;
use std::any::Any;

#[derive(Debug, Clone)]
pub struct CmmnStageHistoryEntity {
    pub id: String,
    pub case_instance_id: String,
    pub case_definition_id: String,
    pub parent_stage_instance_id: Option<String>,
    pub stage_definition_id: String,
    pub state: String,
    pub activated_at: String,
    pub ended_at: Option<String>,
    pub data: String,
}

impl CmmnStageHistoryEntity {
    pub fn new(
        id: String,
        case_instance_id: String,
        case_definition_id: String,
        stage_definition_id: String,
        state: String,
        activated_at: String,
        data: String,
    ) -> Self {
        Self {
            id,
            case_instance_id,
            case_definition_id,
            parent_stage_instance_id: None,
            stage_definition_id,
            state,
            activated_at,
            ended_at: None,
            data,
        }
    }

    pub fn set_parent_stage_instance_id(&mut self, parent_stage_instance_id: Option<String>) {
        self.parent_stage_instance_id = parent_stage_instance_id;
    }

    pub fn set_ended_at(&mut self, ended_at: Option<String>) {
        self.ended_at = ended_at;
    }

    pub fn from_row(row: &DbRow) -> Result<Self, PersistenceError> {
        Ok(Self {
            id: row.get_text("ID_").ok_or_else(|| {
                PersistenceError::Deserialization(
                    "Missing ID_ in CmmnStageHistoryEntity".to_string(),
                )
            })?,
            case_instance_id: row.get_text("CASE_INSTANCE_ID_").ok_or_else(|| {
                PersistenceError::Deserialization(
                    "Missing CASE_INSTANCE_ID_ in CmmnStageHistoryEntity".to_string(),
                )
            })?,
            case_definition_id: row.get_text("CASE_DEFINITION_ID_").ok_or_else(|| {
                PersistenceError::Deserialization(
                    "Missing CASE_DEFINITION_ID_ in CmmnStageHistoryEntity".to_string(),
                )
            })?,
            parent_stage_instance_id: row.get_text("PARENT_STAGE_INSTANCE_ID_"),
            stage_definition_id: row.get_text("STAGE_DEFINITION_ID_").ok_or_else(|| {
                PersistenceError::Deserialization(
                    "Missing STAGE_DEFINITION_ID_ in CmmnStageHistoryEntity".to_string(),
                )
            })?,
            state: row.get_text("STATE_").ok_or_else(|| {
                PersistenceError::Deserialization(
                    "Missing STATE_ in CmmnStageHistoryEntity".to_string(),
                )
            })?,
            activated_at: row.get_text("ACTIVATED_AT_").ok_or_else(|| {
                PersistenceError::Deserialization(
                    "Missing ACTIVATED_AT_ in CmmnStageHistoryEntity".to_string(),
                )
            })?,
            ended_at: row.get_text("ENDED_AT_"),
            data: row.get_text("DATA_").ok_or_else(|| {
                PersistenceError::Deserialization(
                    "Missing DATA_ in CmmnStageHistoryEntity".to_string(),
                )
            })?,
        })
    }
}

impl Entity for CmmnStageHistoryEntity {
    fn id(&self) -> &str {
        &self.id
    }

    fn set_id(&mut self, id: String) {
        self.id = id;
    }

    fn entity_type(&self) -> EntityType {
        EntityType::CmmnStageHistory
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn clone_box(&self) -> Box<dyn Entity> {
        Box::new(self.clone())
    }
}

pub struct CmmnStageHistoryDataManager;

impl CmmnStageHistoryDataManager {
    pub fn new() -> Self {
        Self
    }

    pub fn insert(
        &self,
        session: &mut DbSession,
        entity: CmmnStageHistoryEntity,
    ) -> Result<(), PersistenceError> {
        let mut params = DbParams::new();
        params.push(entity.id.clone());
        params.push(entity.case_instance_id.clone());
        params.push(entity.case_definition_id.clone());
        params.push(entity.parent_stage_instance_id.clone());
        params.push(entity.stage_definition_id.clone());
        params.push(entity.state.clone());
        params.push(entity.activated_at.clone());
        params.push(entity.ended_at.clone());
        params.push(entity.data.clone());

        session.insert(entity, StatementId::InsertCmmnStageHistory, params)
    }

    pub fn delete(
        &self,
        session: &mut DbSession,
        entity: &CmmnStageHistoryEntity,
    ) -> Result<(), PersistenceError> {
        let mut params = DbParams::new();
        params.push(entity.id.clone());

        session.delete(entity, StatementId::DeleteCmmnStageHistory, params)
    }

    pub fn find_by_id(
        &self,
        session: &mut DbSession,
        id: &str,
    ) -> Result<Option<CmmnStageHistoryEntity>, PersistenceError> {
        let mut params = DbParams::new();
        params.push(id);

        let row = session.select_one(StatementId::SelectCmmnStageHistoryById, params)?;
        match row {
            Some(row) => Ok(Some(CmmnStageHistoryEntity::from_row(&row)?)),
            None => Ok(None),
        }
    }

    pub fn find_by_case_instance_id(
        &self,
        session: &mut DbSession,
        case_instance_id: &str,
    ) -> Result<Vec<CmmnStageHistoryEntity>, PersistenceError> {
        let mut params = DbParams::new();
        params.push(case_instance_id);

        let rows =
            session.select_list(StatementId::SelectCmmnStageHistoryByCaseInstanceId, params)?;
        rows.iter().map(CmmnStageHistoryEntity::from_row).collect()
    }
}

impl Default for CmmnStageHistoryDataManager {
    fn default() -> Self {
        Self::new()
    }
}
