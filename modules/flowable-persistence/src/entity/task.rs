use crate::db_session::DbSession;
use crate::entity::{Entity, EntityType, RevisionedEntity};
use crate::error::PersistenceError;
use crate::row::DbRow;
use crate::statement::StatementId;
use crate::value::DbParams;
use std::any::Any;

#[derive(Debug, Clone)]
pub struct TaskEntity {
    pub id: String,
    pub revision: i32,
    pub execution_id: Option<String>,
    pub process_instance_id: Option<String>,
    pub process_definition_id: Option<String>,
    pub name: Option<String>,
    pub business_key: Option<String>,
    pub parent_task_id: Option<String>,
    pub description: Option<String>,
    pub task_definition_key: Option<String>,
    pub owner: Option<String>,
    pub assignee: Option<String>,
    pub delegation: Option<String>,
    pub priority: i32,
    pub create_time: Option<i64>,
    pub due_date: Option<i64>,
    pub category: Option<String>,
    pub suspension_state: i32,
    pub tenant_id: Option<String>,
    pub form_key: Option<String>,
    pub claim_time: Option<i64>,
    pub app_version: Option<i32>,
}

impl TaskEntity {
    pub fn new(id: String) -> Self {
        Self {
            id,
            revision: 1,
            execution_id: None,
            process_instance_id: None,
            process_definition_id: None,
            name: None,
            business_key: None,
            parent_task_id: None,
            description: None,
            task_definition_key: None,
            owner: None,
            assignee: None,
            delegation: None,
            priority: 50,
            create_time: None,
            due_date: None,
            category: None,
            suspension_state: 1,
            tenant_id: None,
            form_key: None,
            claim_time: None,
            app_version: None,
        }
    }

    pub fn from_row(row: &DbRow) -> Result<Self, PersistenceError> {
        Ok(Self {
            id: row.get_text("ID_").ok_or_else(|| {
                PersistenceError::Deserialization("Missing ID_ in TaskEntity".to_string())
            })?,
            revision: row.get_integer("REV_").unwrap_or(1) as i32,
            execution_id: row.get_text("EXECUTION_ID_"),
            process_instance_id: row.get_text("PROC_INST_ID_"),
            process_definition_id: row.get_text("PROC_DEF_ID_"),
            name: row.get_text("NAME_"),
            business_key: row.get_text("BUSINESS_KEY_"),
            parent_task_id: row.get_text("PARENT_TASK_ID_"),
            description: row.get_text("DESCRIPTION_"),
            task_definition_key: row.get_text("TASK_DEF_KEY_"),
            owner: row.get_text("OWNER_"),
            assignee: row.get_text("ASSIGNEE_"),
            delegation: row.get_text("DELEGATION_"),
            priority: row.get_integer("PRIORITY_").unwrap_or(50) as i32,
            create_time: row.get_integer("CREATE_TIME_"),
            due_date: row.get_integer("DUE_DATE_"),
            category: row.get_text("CATEGORY_"),
            suspension_state: row.get_integer("SUSPENSION_STATE_").unwrap_or(1) as i32,
            tenant_id: row.get_text("TENANT_ID_"),
            form_key: row.get_text("FORM_KEY_"),
            claim_time: row.get_integer("CLAIM_TIME_"),
            app_version: row.get_integer("APP_VERSION_").map(|v| v as i32),
        })
    }
}

impl Entity for TaskEntity {
    fn id(&self) -> &str {
        &self.id
    }

    fn set_id(&mut self, id: String) {
        self.id = id;
    }

    fn entity_type(&self) -> EntityType {
        EntityType::Task
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn clone_box(&self) -> Box<dyn Entity> {
        Box::new(self.clone())
    }
}

impl RevisionedEntity for TaskEntity {
    fn revision(&self) -> i32 {
        self.revision
    }

    fn set_revision(&mut self, revision: i32) {
        self.revision = revision;
    }
}

pub struct TaskDataManager;

impl TaskDataManager {
    pub fn new() -> Self {
        Self
    }

    pub fn insert(
        &self,
        session: &mut DbSession,
        entity: TaskEntity,
    ) -> Result<(), PersistenceError> {
        let mut params = DbParams::new();
        params.push(entity.id.clone());
        params.push(entity.revision as i64);
        params.push(entity.execution_id.clone());
        params.push(entity.process_instance_id.clone());
        params.push(entity.process_definition_id.clone());
        params.push(entity.name.clone());
        params.push(entity.business_key.clone());
        params.push(entity.parent_task_id.clone());
        params.push(entity.description.clone());
        params.push(entity.task_definition_key.clone());
        params.push(entity.owner.clone());
        params.push(entity.assignee.clone());
        params.push(entity.delegation.clone());
        params.push(entity.priority as i64);
        params.push(entity.create_time);
        params.push(entity.due_date);
        params.push(entity.category.clone());
        params.push(entity.suspension_state as i64);
        params.push(entity.tenant_id.clone());
        params.push(entity.form_key.clone());
        params.push(entity.claim_time);
        params.push(entity.app_version.map(|v| v as i64));

        session.insert(entity, StatementId::InsertTask, params)
    }

    pub fn update(
        &self,
        session: &mut DbSession,
        entity: TaskEntity,
    ) -> Result<(), PersistenceError> {
        let mut params = DbParams::new();
        params.push(entity.revision as i64);
        params.push(entity.execution_id.clone());
        params.push(entity.process_instance_id.clone());
        params.push(entity.process_definition_id.clone());
        params.push(entity.name.clone());
        params.push(entity.business_key.clone());
        params.push(entity.parent_task_id.clone());
        params.push(entity.description.clone());
        params.push(entity.task_definition_key.clone());
        params.push(entity.owner.clone());
        params.push(entity.assignee.clone());
        params.push(entity.delegation.clone());
        params.push(entity.priority as i64);
        params.push(entity.create_time);
        params.push(entity.due_date);
        params.push(entity.category.clone());
        params.push(entity.suspension_state as i64);
        params.push(entity.tenant_id.clone());
        params.push(entity.form_key.clone());
        params.push(entity.claim_time);
        params.push(entity.app_version.map(|v| v as i64));
        params.push(entity.id.clone());
        params.push(entity.revision as i64);

        session.update(entity, StatementId::UpdateTask, params)
    }

    pub fn delete(
        &self,
        session: &mut DbSession,
        entity: &TaskEntity,
    ) -> Result<(), PersistenceError> {
        let mut params = DbParams::new();
        params.push(entity.id.clone());

        session.delete(entity, StatementId::DeleteTask, params)
    }

    pub fn find_by_id(
        &self,
        session: &mut DbSession,
        id: &str,
    ) -> Result<Option<TaskEntity>, PersistenceError> {
        let mut params = DbParams::new();
        params.push(id);

        let row = session.select_one(StatementId::SelectTaskById, params)?;
        match row {
            Some(row) => Ok(Some(TaskEntity::from_row(&row)?)),
            None => Ok(None),
        }
    }

    pub fn find_by_execution_id(
        &self,
        session: &mut DbSession,
        execution_id: &str,
    ) -> Result<Vec<TaskEntity>, PersistenceError> {
        let mut params = DbParams::new();
        params.push(execution_id);

        let rows = session.select_list(StatementId::SelectTasksByExecutionId, params)?;
        rows.iter().map(TaskEntity::from_row).collect()
    }

    pub fn find_by_process_instance_id(
        &self,
        session: &mut DbSession,
        process_instance_id: &str,
    ) -> Result<Vec<TaskEntity>, PersistenceError> {
        let mut params = DbParams::new();
        params.push(process_instance_id);

        let rows = session.select_list(StatementId::SelectTasksByProcessInstanceId, params)?;
        rows.iter().map(TaskEntity::from_row).collect()
    }

    pub fn find_by_assignee(
        &self,
        session: &mut DbSession,
        assignee: &str,
    ) -> Result<Vec<TaskEntity>, PersistenceError> {
        let mut params = DbParams::new();
        params.push(assignee);

        let rows = session.select_list(StatementId::SelectTasksByAssignee, params)?;
        rows.iter().map(TaskEntity::from_row).collect()
    }
}

impl Default for TaskDataManager {
    fn default() -> Self {
        Self::new()
    }
}
