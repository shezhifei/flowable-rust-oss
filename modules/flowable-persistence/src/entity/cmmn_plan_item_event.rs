use crate::db_session::DbSession;
use crate::entity::{Entity, EntityType};
use crate::error::PersistenceError;
use crate::row::DbRow;
use crate::statement::StatementId;
use crate::value::DbParams;
use std::any::Any;

#[derive(Debug, Clone)]
pub struct CmmnPlanItemEventEntity {
    pub id: String,
    pub case_instance_id: String,
    pub plan_item_id: String,
    pub standard_event: String,
    pub occurred_at: i64,
}

impl CmmnPlanItemEventEntity {
    pub fn new(
        id: String,
        case_instance_id: String,
        plan_item_id: String,
        standard_event: String,
        occurred_at: i64,
    ) -> Self {
        Self {
            id,
            case_instance_id,
            plan_item_id,
            standard_event,
            occurred_at,
        }
    }

    pub fn from_row(row: &DbRow) -> Result<Self, PersistenceError> {
        Ok(Self {
            id: row.get_text("ID_").ok_or_else(|| {
                PersistenceError::Deserialization(
                    "Missing ID_ in CmmnPlanItemEventEntity".to_string(),
                )
            })?,
            case_instance_id: row.get_text("CASE_INSTANCE_ID_").ok_or_else(|| {
                PersistenceError::Deserialization(
                    "Missing CASE_INSTANCE_ID_ in CmmnPlanItemEventEntity".to_string(),
                )
            })?,
            plan_item_id: row.get_text("PLAN_ITEM_ID_").ok_or_else(|| {
                PersistenceError::Deserialization(
                    "Missing PLAN_ITEM_ID_ in CmmnPlanItemEventEntity".to_string(),
                )
            })?,
            standard_event: row.get_text("STANDARD_EVENT_").ok_or_else(|| {
                PersistenceError::Deserialization(
                    "Missing STANDARD_EVENT_ in CmmnPlanItemEventEntity".to_string(),
                )
            })?,
            occurred_at: row.get_integer("OCCURRED_AT_").unwrap_or(0),
        })
    }
}

impl Entity for CmmnPlanItemEventEntity {
    fn id(&self) -> &str {
        &self.id
    }

    fn set_id(&mut self, id: String) {
        self.id = id;
    }

    fn entity_type(&self) -> EntityType {
        EntityType::CmmnPlanItemEvent
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn clone_box(&self) -> Box<dyn Entity> {
        Box::new(self.clone())
    }
}

pub struct CmmnPlanItemEventDataManager;

impl CmmnPlanItemEventDataManager {
    pub fn new() -> Self {
        Self
    }

    pub fn insert(
        &self,
        session: &mut DbSession,
        entity: CmmnPlanItemEventEntity,
    ) -> Result<(), PersistenceError> {
        let mut params = DbParams::new();
        params.push(entity.id.clone());
        params.push(entity.case_instance_id.clone());
        params.push(entity.plan_item_id.clone());
        params.push(entity.standard_event.clone());
        params.push(entity.occurred_at);

        session.insert(entity, StatementId::InsertCmmnPlanItemEvent, params)
    }

    pub fn delete(
        &self,
        session: &mut DbSession,
        entity: &CmmnPlanItemEventEntity,
    ) -> Result<(), PersistenceError> {
        let mut params = DbParams::new();
        params.push(entity.id.clone());

        session.delete(entity, StatementId::DeleteCmmnPlanItemEvent, params)
    }

    pub fn find_by_id(
        &self,
        session: &mut DbSession,
        id: &str,
    ) -> Result<Option<CmmnPlanItemEventEntity>, PersistenceError> {
        let mut params = DbParams::new();
        params.push(id);

        let row = session.select_one(StatementId::SelectCmmnPlanItemEventById, params)?;
        match row {
            Some(row) => Ok(Some(CmmnPlanItemEventEntity::from_row(&row)?)),
            None => Ok(None),
        }
    }

    pub fn find_by_case_instance_id(
        &self,
        session: &mut DbSession,
        case_instance_id: &str,
    ) -> Result<Vec<CmmnPlanItemEventEntity>, PersistenceError> {
        let mut params = DbParams::new();
        params.push(case_instance_id);

        let rows = session.select_list(
            StatementId::SelectCmmnPlanItemEventsByCaseInstanceId,
            params,
        )?;
        rows.iter().map(CmmnPlanItemEventEntity::from_row).collect()
    }
}

impl Default for CmmnPlanItemEventDataManager {
    fn default() -> Self {
        Self::new()
    }
}
