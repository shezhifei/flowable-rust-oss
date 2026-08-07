use crate::db_session::DbSession;
use crate::entity::{Entity, EntityType};
use crate::error::PersistenceError;
use crate::row::DbRow;
use crate::statement::StatementId;
use crate::value::DbParams;
use std::any::Any;

#[derive(Debug, Clone)]
pub struct CmmnTaskInstanceAssociationEntity {
    pub id: String,
    pub kind: String,
    pub state: String,
    pub case_instance_id: String,
    pub case_definition_id: String,
    pub case_key: String,
    pub stage_instance_id: Option<String>,
    pub plan_item_id: String,
    pub task_definition_id: String,
    pub child_definition_key: String,
    pub child_instance_id: String,
    pub created_at: i64,
    pub completed_at: Option<String>,
    pub failure_message: Option<String>,
    pub data: String,
}

impl CmmnTaskInstanceAssociationEntity {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: String,
        kind: String,
        state: String,
        case_instance_id: String,
        case_definition_id: String,
        case_key: String,
        plan_item_id: String,
        task_definition_id: String,
        child_definition_key: String,
        child_instance_id: String,
        created_at: i64,
        data: String,
    ) -> Self {
        Self {
            id,
            kind,
            state,
            case_instance_id,
            case_definition_id,
            case_key,
            stage_instance_id: None,
            plan_item_id,
            task_definition_id,
            child_definition_key,
            child_instance_id,
            created_at,
            completed_at: None,
            failure_message: None,
            data,
        }
    }

    pub fn set_stage_instance_id(&mut self, stage_instance_id: Option<String>) {
        self.stage_instance_id = stage_instance_id;
    }

    pub fn set_completed_at(&mut self, completed_at: Option<String>) {
        self.completed_at = completed_at;
    }

    pub fn set_failure_message(&mut self, failure_message: Option<String>) {
        self.failure_message = failure_message;
    }

    pub fn from_row(row: &DbRow) -> Result<Self, PersistenceError> {
        Ok(Self {
            id: row.get_text("ID_").ok_or_else(|| {
                PersistenceError::Deserialization(
                    "Missing ID_ in CmmnTaskInstanceAssociationEntity".to_string(),
                )
            })?,
            kind: row.get_text("KIND_").ok_or_else(|| {
                PersistenceError::Deserialization(
                    "Missing KIND_ in CmmnTaskInstanceAssociationEntity".to_string(),
                )
            })?,
            state: row.get_text("STATE_").ok_or_else(|| {
                PersistenceError::Deserialization(
                    "Missing STATE_ in CmmnTaskInstanceAssociationEntity".to_string(),
                )
            })?,
            case_instance_id: row.get_text("CASE_INSTANCE_ID_").ok_or_else(|| {
                PersistenceError::Deserialization(
                    "Missing CASE_INSTANCE_ID_ in CmmnTaskInstanceAssociationEntity".to_string(),
                )
            })?,
            case_definition_id: row.get_text("CASE_DEFINITION_ID_").ok_or_else(|| {
                PersistenceError::Deserialization(
                    "Missing CASE_DEFINITION_ID_ in CmmnTaskInstanceAssociationEntity".to_string(),
                )
            })?,
            case_key: row.get_text("CASE_KEY_").ok_or_else(|| {
                PersistenceError::Deserialization(
                    "Missing CASE_KEY_ in CmmnTaskInstanceAssociationEntity".to_string(),
                )
            })?,
            stage_instance_id: row.get_text("STAGE_INSTANCE_ID_"),
            plan_item_id: row.get_text("PLAN_ITEM_ID_").ok_or_else(|| {
                PersistenceError::Deserialization(
                    "Missing PLAN_ITEM_ID_ in CmmnTaskInstanceAssociationEntity".to_string(),
                )
            })?,
            task_definition_id: row.get_text("TASK_DEFINITION_ID_").ok_or_else(|| {
                PersistenceError::Deserialization(
                    "Missing TASK_DEFINITION_ID_ in CmmnTaskInstanceAssociationEntity".to_string(),
                )
            })?,
            child_definition_key: row.get_text("CHILD_DEFINITION_KEY_").ok_or_else(|| {
                PersistenceError::Deserialization(
                    "Missing CHILD_DEFINITION_KEY_ in CmmnTaskInstanceAssociationEntity"
                        .to_string(),
                )
            })?,
            child_instance_id: row.get_text("CHILD_INSTANCE_ID_").ok_or_else(|| {
                PersistenceError::Deserialization(
                    "Missing CHILD_INSTANCE_ID_ in CmmnTaskInstanceAssociationEntity".to_string(),
                )
            })?,
            created_at: row.get_integer("CREATED_AT_").unwrap_or(0),
            completed_at: row.get_text("COMPLETED_AT_"),
            failure_message: row.get_text("FAILURE_MESSAGE_"),
            data: row.get_text("DATA_").ok_or_else(|| {
                PersistenceError::Deserialization(
                    "Missing DATA_ in CmmnTaskInstanceAssociationEntity".to_string(),
                )
            })?,
        })
    }
}

impl Entity for CmmnTaskInstanceAssociationEntity {
    fn id(&self) -> &str {
        &self.id
    }

    fn set_id(&mut self, id: String) {
        self.id = id;
    }

    fn entity_type(&self) -> EntityType {
        EntityType::CmmnTaskInstanceAssociation
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn clone_box(&self) -> Box<dyn Entity> {
        Box::new(self.clone())
    }
}

pub struct CmmnTaskInstanceAssociationDataManager;

impl CmmnTaskInstanceAssociationDataManager {
    pub fn new() -> Self {
        Self
    }

    pub fn insert(
        &self,
        session: &mut DbSession,
        entity: CmmnTaskInstanceAssociationEntity,
    ) -> Result<(), PersistenceError> {
        let mut params = DbParams::new();
        params.push(entity.id.clone());
        params.push(entity.kind.clone());
        params.push(entity.state.clone());
        params.push(entity.case_instance_id.clone());
        params.push(entity.case_definition_id.clone());
        params.push(entity.case_key.clone());
        params.push(entity.stage_instance_id.clone());
        params.push(entity.plan_item_id.clone());
        params.push(entity.task_definition_id.clone());
        params.push(entity.child_definition_key.clone());
        params.push(entity.child_instance_id.clone());
        params.push(entity.created_at);
        params.push(entity.completed_at.clone());
        params.push(entity.failure_message.clone());
        params.push(entity.data.clone());

        session.insert(
            entity,
            StatementId::InsertCmmnTaskInstanceAssociation,
            params,
        )
    }

    pub fn delete(
        &self,
        session: &mut DbSession,
        entity: &CmmnTaskInstanceAssociationEntity,
    ) -> Result<(), PersistenceError> {
        let mut params = DbParams::new();
        params.push(entity.id.clone());

        session.delete(
            entity,
            StatementId::DeleteCmmnTaskInstanceAssociation,
            params,
        )
    }

    pub fn find_by_id(
        &self,
        session: &mut DbSession,
        id: &str,
    ) -> Result<Option<CmmnTaskInstanceAssociationEntity>, PersistenceError> {
        let mut params = DbParams::new();
        params.push(id);

        let row = session.select_one(StatementId::SelectCmmnTaskInstanceAssociationById, params)?;
        match row {
            Some(row) => Ok(Some(CmmnTaskInstanceAssociationEntity::from_row(&row)?)),
            None => Ok(None),
        }
    }

    pub fn find_by_case_instance_id(
        &self,
        session: &mut DbSession,
        case_instance_id: &str,
    ) -> Result<Vec<CmmnTaskInstanceAssociationEntity>, PersistenceError> {
        let mut params = DbParams::new();
        params.push(case_instance_id);

        let rows = session.select_list(
            StatementId::SelectCmmnTaskInstanceAssociationsByCaseInstanceId,
            params,
        )?;
        rows.iter()
            .map(CmmnTaskInstanceAssociationEntity::from_row)
            .collect()
    }
}

impl Default for CmmnTaskInstanceAssociationDataManager {
    fn default() -> Self {
        Self::new()
    }
}
