use crate::db_session::DbSession;
use crate::entity::{Entity, EntityType};
use crate::error::PersistenceError;
use crate::row::DbRow;
use crate::statement::StatementId;
use crate::value::DbParams;
use std::any::Any;

#[derive(Debug, Clone)]
pub struct CmmnMilestoneHistoryEntity {
    pub id: String,
    pub case_instance_id: String,
    pub case_definition_id: String,
    pub case_key: String,
    pub milestone_id: String,
    pub time: String,
    pub data: String,
}

impl CmmnMilestoneHistoryEntity {
    pub fn new(
        id: String,
        case_instance_id: String,
        case_definition_id: String,
        case_key: String,
        milestone_id: String,
        time: String,
        data: String,
    ) -> Self {
        Self {
            id,
            case_instance_id,
            case_definition_id,
            case_key,
            milestone_id,
            time,
            data,
        }
    }

    pub fn from_row(row: &DbRow) -> Result<Self, PersistenceError> {
        Ok(Self {
            id: row.get_text("ID_").ok_or_else(|| {
                PersistenceError::Deserialization(
                    "Missing ID_ in CmmnMilestoneHistoryEntity".to_string(),
                )
            })?,
            case_instance_id: row.get_text("CASE_INSTANCE_ID_").ok_or_else(|| {
                PersistenceError::Deserialization(
                    "Missing CASE_INSTANCE_ID_ in CmmnMilestoneHistoryEntity".to_string(),
                )
            })?,
            case_definition_id: row.get_text("CASE_DEFINITION_ID_").ok_or_else(|| {
                PersistenceError::Deserialization(
                    "Missing CASE_DEFINITION_ID_ in CmmnMilestoneHistoryEntity".to_string(),
                )
            })?,
            case_key: row.get_text("CASE_KEY_").ok_or_else(|| {
                PersistenceError::Deserialization(
                    "Missing CASE_KEY_ in CmmnMilestoneHistoryEntity".to_string(),
                )
            })?,
            milestone_id: row.get_text("MILESTONE_ID_").ok_or_else(|| {
                PersistenceError::Deserialization(
                    "Missing MILESTONE_ID_ in CmmnMilestoneHistoryEntity".to_string(),
                )
            })?,
            time: row.get_text("TIME_").ok_or_else(|| {
                PersistenceError::Deserialization(
                    "Missing TIME_ in CmmnMilestoneHistoryEntity".to_string(),
                )
            })?,
            data: row.get_text("DATA_").ok_or_else(|| {
                PersistenceError::Deserialization(
                    "Missing DATA_ in CmmnMilestoneHistoryEntity".to_string(),
                )
            })?,
        })
    }
}

impl Entity for CmmnMilestoneHistoryEntity {
    fn id(&self) -> &str {
        &self.id
    }

    fn set_id(&mut self, id: String) {
        self.id = id;
    }

    fn entity_type(&self) -> EntityType {
        EntityType::CmmnMilestoneHistory
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn clone_box(&self) -> Box<dyn Entity> {
        Box::new(self.clone())
    }
}

pub struct CmmnMilestoneHistoryDataManager;

impl CmmnMilestoneHistoryDataManager {
    pub fn new() -> Self {
        Self
    }

    pub fn insert(
        &self,
        session: &mut DbSession,
        entity: CmmnMilestoneHistoryEntity,
    ) -> Result<(), PersistenceError> {
        let mut params = DbParams::new();
        params.push(entity.id.clone());
        params.push(entity.case_instance_id.clone());
        params.push(entity.case_definition_id.clone());
        params.push(entity.case_key.clone());
        params.push(entity.milestone_id.clone());
        params.push(entity.time.clone());
        params.push(entity.data.clone());

        session.insert(entity, StatementId::InsertCmmnMilestoneHistory, params)
    }

    pub fn delete(
        &self,
        session: &mut DbSession,
        entity: &CmmnMilestoneHistoryEntity,
    ) -> Result<(), PersistenceError> {
        let mut params = DbParams::new();
        params.push(entity.id.clone());

        session.delete(entity, StatementId::DeleteCmmnMilestoneHistory, params)
    }

    pub fn find_by_id(
        &self,
        session: &mut DbSession,
        id: &str,
    ) -> Result<Option<CmmnMilestoneHistoryEntity>, PersistenceError> {
        let mut params = DbParams::new();
        params.push(id);

        let row = session.select_one(StatementId::SelectCmmnMilestoneHistoryById, params)?;
        match row {
            Some(row) => Ok(Some(CmmnMilestoneHistoryEntity::from_row(&row)?)),
            None => Ok(None),
        }
    }

    pub fn find_by_case_instance_id(
        &self,
        session: &mut DbSession,
        case_instance_id: &str,
    ) -> Result<Vec<CmmnMilestoneHistoryEntity>, PersistenceError> {
        let mut params = DbParams::new();
        params.push(case_instance_id);

        let rows = session.select_list(
            StatementId::SelectCmmnMilestoneHistoryByCaseInstanceId,
            params,
        )?;
        rows.iter()
            .map(CmmnMilestoneHistoryEntity::from_row)
            .collect()
    }
}

impl Default for CmmnMilestoneHistoryDataManager {
    fn default() -> Self {
        Self::new()
    }
}
