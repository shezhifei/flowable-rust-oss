use crate::db_session::DbSession;
use crate::entity::{Entity, EntityType, RevisionedEntity};
use crate::error::PersistenceError;
use crate::row::DbRow;
use crate::statement::StatementId;
use crate::value::DbParams;
use std::any::Any;

#[derive(Debug, Clone)]
pub struct HistoryActivityEntity {
    pub id: String,
    pub revision: i32,
    pub process_definition_id: Option<String>,
    pub process_instance_id: Option<String>,
    pub execution_id: Option<String>,
    pub act_id: Option<String>,
    pub task_id: Option<String>,
    pub call_proc_inst_id: Option<String>,
    pub act_name: Option<String>,
    pub act_type: Option<String>,
    pub assignee: Option<String>,
    pub start_time: Option<i64>,
    pub end_time: Option<i64>,
    pub duration: Option<i64>,
    pub transaction_order: Option<i64>,
    pub delete_reason: Option<String>,
    pub tenant_id: Option<String>,
}

impl HistoryActivityEntity {
    pub fn new(id: String) -> Self {
        Self {
            id,
            revision: 1,
            process_definition_id: None,
            process_instance_id: None,
            execution_id: None,
            act_id: None,
            task_id: None,
            call_proc_inst_id: None,
            act_name: None,
            act_type: None,
            assignee: None,
            start_time: None,
            end_time: None,
            duration: None,
            transaction_order: None,
            delete_reason: None,
            tenant_id: None,
        }
    }

    pub fn from_row(row: &DbRow) -> Result<Self, PersistenceError> {
        Ok(Self {
            id: row.get_text("ID_").ok_or_else(|| {
                PersistenceError::Deserialization(
                    "Missing ID_ in HistoryActivityEntity".to_string(),
                )
            })?,
            revision: row.get_integer("REV_").unwrap_or(1) as i32,
            process_definition_id: row.get_text("PROC_DEF_ID_"),
            process_instance_id: row.get_text("PROC_INST_ID_"),
            execution_id: row.get_text("EXECUTION_ID_"),
            act_id: row.get_text("ACT_ID_"),
            task_id: row.get_text("TASK_ID_"),
            call_proc_inst_id: row.get_text("CALL_PROC_INST_ID_"),
            act_name: row.get_text("ACT_NAME_"),
            act_type: row.get_text("ACT_TYPE_"),
            assignee: row.get_text("ASSIGNEE_"),
            start_time: row.get_integer("START_TIME_"),
            end_time: row.get_integer("END_TIME_"),
            duration: row.get_integer("DURATION_"),
            transaction_order: row.get_integer("TRANSACTION_ORDER_"),
            delete_reason: row.get_text("DELETE_REASON_"),
            tenant_id: row.get_text("TENANT_ID_"),
        })
    }
}

impl Entity for HistoryActivityEntity {
    fn id(&self) -> &str {
        &self.id
    }

    fn set_id(&mut self, id: String) {
        self.id = id;
    }

    fn entity_type(&self) -> EntityType {
        EntityType::HistoryActivity
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn clone_box(&self) -> Box<dyn Entity> {
        Box::new(self.clone())
    }
}

impl RevisionedEntity for HistoryActivityEntity {
    fn revision(&self) -> i32 {
        self.revision
    }

    fn set_revision(&mut self, revision: i32) {
        self.revision = revision;
    }
}

pub struct HistoryActivityDataManager;

impl HistoryActivityDataManager {
    pub fn new() -> Self {
        Self
    }

    pub fn insert(
        &self,
        session: &mut DbSession,
        entity: HistoryActivityEntity,
    ) -> Result<(), PersistenceError> {
        let mut params = DbParams::new();
        params.push(entity.id.clone());
        params.push(entity.revision as i64);
        params.push(entity.process_definition_id.clone());
        params.push(entity.process_instance_id.clone());
        params.push(entity.execution_id.clone());
        params.push(entity.act_id.clone());
        params.push(entity.task_id.clone());
        params.push(entity.call_proc_inst_id.clone());
        params.push(entity.act_name.clone());
        params.push(entity.act_type.clone());
        params.push(entity.assignee.clone());
        params.push(entity.start_time);
        params.push(entity.end_time);
        params.push(entity.duration);
        params.push(entity.transaction_order);
        params.push(entity.delete_reason.clone());
        params.push(entity.tenant_id.clone());

        session.insert(entity, StatementId::InsertHistoryActivity, params)
    }

    pub fn update(
        &self,
        session: &mut DbSession,
        entity: HistoryActivityEntity,
    ) -> Result<(), PersistenceError> {
        let mut params = DbParams::new();
        params.push(entity.revision as i64);
        params.push(entity.process_definition_id.clone());
        params.push(entity.process_instance_id.clone());
        params.push(entity.execution_id.clone());
        params.push(entity.act_id.clone());
        params.push(entity.task_id.clone());
        params.push(entity.call_proc_inst_id.clone());
        params.push(entity.act_name.clone());
        params.push(entity.act_type.clone());
        params.push(entity.assignee.clone());
        params.push(entity.start_time);
        params.push(entity.end_time);
        params.push(entity.duration);
        params.push(entity.transaction_order);
        params.push(entity.delete_reason.clone());
        params.push(entity.tenant_id.clone());
        params.push(entity.id.clone());
        params.push(entity.revision as i64);

        session.update(entity, StatementId::UpdateHistoryActivity, params)
    }

    pub fn delete(
        &self,
        session: &mut DbSession,
        entity: &HistoryActivityEntity,
    ) -> Result<(), PersistenceError> {
        let mut params = DbParams::new();
        params.push(entity.id.clone());

        session.delete(entity, StatementId::DeleteHistoryActivity, params)
    }

    pub fn find_by_id(
        &self,
        session: &mut DbSession,
        id: &str,
    ) -> Result<Option<HistoryActivityEntity>, PersistenceError> {
        let mut params = DbParams::new();
        params.push(id);

        let row = session.select_one(StatementId::SelectHistoryActivityById, params)?;
        match row {
            Some(row) => Ok(Some(HistoryActivityEntity::from_row(&row)?)),
            None => Ok(None),
        }
    }
}

impl Default for HistoryActivityDataManager {
    fn default() -> Self {
        Self::new()
    }
}
