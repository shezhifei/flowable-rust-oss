use crate::db_session::DbSession;
use crate::entity::{Entity, EntityType, RevisionedEntity};
use crate::error::PersistenceError;
use crate::row::DbRow;
use crate::statement::StatementId;
use crate::value::DbParams;
use std::any::Any;

// ============================================================================
// HistoryProcessInstanceEntity
// ============================================================================

#[derive(Debug, Clone)]
#[allow(non_snake_case)]
pub struct HistoryProcessInstanceEntity {
    pub id: String,
    pub revision: i32,
    pub process_definition_id: Option<String>,
    pub process_definition_key: Option<String>,
    pub process_definition_name: Option<String>,
    pub process_definition_version: Option<i32>,
    pub business_key: Option<String>,
    pub start_time: Option<i64>,
    pub end_time: Option<i64>,
    pub durationInMillis: Option<i64>,
    pub start_user_id: Option<String>,
    pub start_activity_id: Option<String>,
    pub end_activity_id: Option<String>,
    pub super_process_instance_id: Option<String>,
    pub delete_reason: Option<String>,
    pub tenant_id: Option<String>,
    pub name: Option<String>,
    pub description: Option<String>,
    pub callback_id: Option<String>,
    pub callback_type: Option<String>,
    pub reference_id: Option<String>,
    pub reference_type: Option<String>,
}

impl HistoryProcessInstanceEntity {
    pub fn new(id: String) -> Self {
        Self {
            id,
            revision: 1,
            process_definition_id: None,
            process_definition_key: None,
            process_definition_name: None,
            process_definition_version: None,
            business_key: None,
            start_time: None,
            end_time: None,
            durationInMillis: None,
            start_user_id: None,
            start_activity_id: None,
            end_activity_id: None,
            super_process_instance_id: None,
            delete_reason: None,
            tenant_id: None,
            name: None,
            description: None,
            callback_id: None,
            callback_type: None,
            reference_id: None,
            reference_type: None,
        }
    }

    pub fn from_row(row: &DbRow) -> Result<Self, PersistenceError> {
        Ok(Self {
            id: row.get_text("ID_").ok_or_else(|| {
                PersistenceError::Deserialization(
                    "Missing ID_ in HistoryProcessInstanceEntity".to_string(),
                )
            })?,
            revision: row.get_integer("REV_").unwrap_or(1) as i32,
            process_definition_id: row.get_text("PROC_DEF_ID_"),
            process_definition_key: row.get_text("PROC_DEF_KEY_"),
            process_definition_name: row.get_text("PROC_DEF_NAME_"),
            process_definition_version: row.get_integer("PROC_DEF_VERSION_").map(|v| v as i32),
            business_key: row.get_text("BUSINESS_KEY_"),
            start_time: row.get_integer("START_TIME_"),
            end_time: row.get_integer("END_TIME_"),
            durationInMillis: row.get_integer("DURATION_"),
            start_user_id: row.get_text("START_USER_ID_"),
            start_activity_id: row.get_text("START_ACT_ID_"),
            end_activity_id: row.get_text("END_ACT_ID_"),
            super_process_instance_id: row.get_text("SUPER_PROCESS_INSTANCE_ID_"),
            delete_reason: row.get_text("DELETE_REASON_"),
            tenant_id: row.get_text("TENANT_ID_"),
            name: row.get_text("NAME_"),
            description: row.get_text("DESCRIPTION_"),
            callback_id: row.get_text("CALLBACK_ID_"),
            callback_type: row.get_text("CALLBACK_TYPE_"),
            reference_id: row.get_text("REFERENCE_ID_"),
            reference_type: row.get_text("REFERENCE_TYPE_"),
        })
    }
}

impl Entity for HistoryProcessInstanceEntity {
    fn id(&self) -> &str {
        &self.id
    }

    fn set_id(&mut self, id: String) {
        self.id = id;
    }

    fn entity_type(&self) -> EntityType {
        EntityType::HistoryProcessInstance
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn clone_box(&self) -> Box<dyn Entity> {
        Box::new(self.clone())
    }
}

impl RevisionedEntity for HistoryProcessInstanceEntity {
    fn revision(&self) -> i32 {
        self.revision
    }

    fn set_revision(&mut self, revision: i32) {
        self.revision = revision;
    }
}

pub struct HistoryProcessInstanceDataManager;

impl HistoryProcessInstanceDataManager {
    pub fn new() -> Self {
        Self
    }

    pub fn insert(
        &self,
        session: &mut DbSession,
        entity: HistoryProcessInstanceEntity,
    ) -> Result<(), PersistenceError> {
        let mut params = DbParams::new();
        params.push(entity.id.clone());
        params.push(entity.revision as i64);
        params.push(entity.process_definition_id.clone());
        params.push(entity.process_definition_key.clone());
        params.push(entity.process_definition_name.clone());
        params.push(entity.process_definition_version.map(|v| v as i64));
        params.push(entity.business_key.clone());
        params.push(entity.start_time);
        params.push(entity.end_time);
        params.push(entity.durationInMillis);
        params.push(entity.start_user_id.clone());
        params.push(entity.start_activity_id.clone());
        params.push(entity.end_activity_id.clone());
        params.push(entity.super_process_instance_id.clone());
        params.push(entity.delete_reason.clone());
        params.push(entity.tenant_id.clone());
        params.push(entity.name.clone());
        params.push(entity.description.clone());
        params.push(entity.callback_id.clone());
        params.push(entity.callback_type.clone());
        params.push(entity.reference_id.clone());
        params.push(entity.reference_type.clone());

        session.insert(entity, StatementId::InsertHistoryProcessInstance, params)
    }

    pub fn update(
        &self,
        session: &mut DbSession,
        entity: HistoryProcessInstanceEntity,
    ) -> Result<(), PersistenceError> {
        let mut params = DbParams::new();
        params.push(entity.revision as i64);
        params.push(entity.process_definition_id.clone());
        params.push(entity.process_definition_key.clone());
        params.push(entity.process_definition_name.clone());
        params.push(entity.process_definition_version.map(|v| v as i64));
        params.push(entity.business_key.clone());
        params.push(entity.start_time);
        params.push(entity.end_time);
        params.push(entity.durationInMillis);
        params.push(entity.start_user_id.clone());
        params.push(entity.start_activity_id.clone());
        params.push(entity.end_activity_id.clone());
        params.push(entity.super_process_instance_id.clone());
        params.push(entity.delete_reason.clone());
        params.push(entity.tenant_id.clone());
        params.push(entity.name.clone());
        params.push(entity.description.clone());
        params.push(entity.callback_id.clone());
        params.push(entity.callback_type.clone());
        params.push(entity.reference_id.clone());
        params.push(entity.reference_type.clone());
        params.push(entity.id.clone());
        params.push(entity.revision as i64);

        session.update(entity, StatementId::UpdateHistoryProcessInstance, params)
    }

    pub fn delete(
        &self,
        session: &mut DbSession,
        entity: &HistoryProcessInstanceEntity,
    ) -> Result<(), PersistenceError> {
        let mut params = DbParams::new();
        params.push(entity.id.clone());

        session.delete(entity, StatementId::DeleteHistoryProcessInstance, params)
    }

    pub fn find_by_id(
        &self,
        session: &mut DbSession,
        id: &str,
    ) -> Result<Option<HistoryProcessInstanceEntity>, PersistenceError> {
        let mut params = DbParams::new();
        params.push(id);

        let row = session.select_one(StatementId::SelectHistoryProcessInstanceById, params)?;
        match row {
            Some(row) => Ok(Some(HistoryProcessInstanceEntity::from_row(&row)?)),
            None => Ok(None),
        }
    }
}

impl Default for HistoryProcessInstanceDataManager {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// HistoryTaskEntity
// ============================================================================

#[derive(Debug, Clone)]
#[allow(non_snake_case)]
pub struct HistoryTaskEntity {
    pub id: String,
    pub revision: i32,
    pub process_definition_id: Option<String>,
    pub process_instance_id: Option<String>,
    pub execution_id: Option<String>,
    pub name: Option<String>,
    pub parent_task_id: Option<String>,
    pub description: Option<String>,
    pub owner: Option<String>,
    pub assignee: Option<String>,
    pub start_time: Option<i64>,
    pub claim_time: Option<i64>,
    pub end_time: Option<i64>,
    pub durationInMillis: Option<i64>,
    pub delete_reason: Option<String>,
    pub priority: i32,
    pub due_date: Option<i64>,
    pub task_definition_key: Option<String>,
    pub category: Option<String>,
    pub form_key: Option<String>,
    pub tenant_id: Option<String>,
    pub app_version: Option<i32>,
}

impl HistoryTaskEntity {
    pub fn new(id: String) -> Self {
        Self {
            id,
            revision: 1,
            process_definition_id: None,
            process_instance_id: None,
            execution_id: None,
            name: None,
            parent_task_id: None,
            description: None,
            owner: None,
            assignee: None,
            start_time: None,
            claim_time: None,
            end_time: None,
            durationInMillis: None,
            delete_reason: None,
            priority: 50,
            due_date: None,
            task_definition_key: None,
            category: None,
            form_key: None,
            tenant_id: None,
            app_version: None,
        }
    }

    pub fn from_row(row: &DbRow) -> Result<Self, PersistenceError> {
        Ok(Self {
            id: row.get_text("ID_").ok_or_else(|| {
                PersistenceError::Deserialization("Missing ID_ in HistoryTaskEntity".to_string())
            })?,
            revision: row.get_integer("REV_").unwrap_or(1) as i32,
            process_definition_id: row.get_text("PROC_DEF_ID_"),
            process_instance_id: row.get_text("PROC_INST_ID_"),
            execution_id: row.get_text("EXECUTION_ID_"),
            name: row.get_text("NAME_"),
            parent_task_id: row.get_text("PARENT_TASK_ID_"),
            description: row.get_text("DESCRIPTION_"),
            owner: row.get_text("OWNER_"),
            assignee: row.get_text("ASSIGNEE_"),
            start_time: row.get_integer("START_TIME_"),
            claim_time: row.get_integer("CLAIM_TIME_"),
            end_time: row.get_integer("END_TIME_"),
            durationInMillis: row.get_integer("DURATION_"),
            delete_reason: row.get_text("DELETE_REASON_"),
            priority: row.get_integer("PRIORITY_").unwrap_or(50) as i32,
            due_date: row.get_integer("DUE_DATE_"),
            task_definition_key: row.get_text("TASK_DEF_KEY_"),
            category: row.get_text("CATEGORY_"),
            form_key: row.get_text("FORM_KEY_"),
            tenant_id: row.get_text("TENANT_ID_"),
            app_version: row.get_integer("APP_VERSION_").map(|v| v as i32),
        })
    }
}

impl Entity for HistoryTaskEntity {
    fn id(&self) -> &str {
        &self.id
    }

    fn set_id(&mut self, id: String) {
        self.id = id;
    }

    fn entity_type(&self) -> EntityType {
        EntityType::HistoryTask
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn clone_box(&self) -> Box<dyn Entity> {
        Box::new(self.clone())
    }
}

impl RevisionedEntity for HistoryTaskEntity {
    fn revision(&self) -> i32 {
        self.revision
    }

    fn set_revision(&mut self, revision: i32) {
        self.revision = revision;
    }
}

pub struct HistoryTaskDataManager;

impl HistoryTaskDataManager {
    pub fn new() -> Self {
        Self
    }

    pub fn insert(
        &self,
        session: &mut DbSession,
        entity: HistoryTaskEntity,
    ) -> Result<(), PersistenceError> {
        let mut params = DbParams::new();
        params.push(entity.id.clone());
        params.push(entity.revision as i64);
        params.push(entity.process_definition_id.clone());
        params.push(entity.process_instance_id.clone());
        params.push(entity.execution_id.clone());
        params.push(entity.name.clone());
        params.push(entity.parent_task_id.clone());
        params.push(entity.description.clone());
        params.push(entity.owner.clone());
        params.push(entity.assignee.clone());
        params.push(entity.start_time);
        params.push(entity.claim_time);
        params.push(entity.end_time);
        params.push(entity.durationInMillis);
        params.push(entity.delete_reason.clone());
        params.push(entity.priority as i64);
        params.push(entity.due_date);
        params.push(entity.task_definition_key.clone());
        params.push(entity.category.clone());
        params.push(entity.form_key.clone());
        params.push(entity.tenant_id.clone());
        params.push(entity.app_version.map(|v| v as i64));

        session.insert(entity, StatementId::InsertHistoryTask, params)
    }

    pub fn update(
        &self,
        session: &mut DbSession,
        entity: HistoryTaskEntity,
    ) -> Result<(), PersistenceError> {
        let mut params = DbParams::new();
        params.push(entity.revision as i64);
        params.push(entity.process_definition_id.clone());
        params.push(entity.process_instance_id.clone());
        params.push(entity.execution_id.clone());
        params.push(entity.name.clone());
        params.push(entity.parent_task_id.clone());
        params.push(entity.description.clone());
        params.push(entity.owner.clone());
        params.push(entity.assignee.clone());
        params.push(entity.start_time);
        params.push(entity.claim_time);
        params.push(entity.end_time);
        params.push(entity.durationInMillis);
        params.push(entity.delete_reason.clone());
        params.push(entity.priority as i64);
        params.push(entity.due_date);
        params.push(entity.task_definition_key.clone());
        params.push(entity.category.clone());
        params.push(entity.form_key.clone());
        params.push(entity.tenant_id.clone());
        params.push(entity.app_version.map(|v| v as i64));
        params.push(entity.id.clone());
        params.push(entity.revision as i64);

        session.update(entity, StatementId::UpdateHistoryTask, params)
    }

    pub fn delete(
        &self,
        session: &mut DbSession,
        entity: &HistoryTaskEntity,
    ) -> Result<(), PersistenceError> {
        let mut params = DbParams::new();
        params.push(entity.id.clone());

        session.delete(entity, StatementId::DeleteHistoryTask, params)
    }

    pub fn find_by_id(
        &self,
        session: &mut DbSession,
        id: &str,
    ) -> Result<Option<HistoryTaskEntity>, PersistenceError> {
        let mut params = DbParams::new();
        params.push(id);

        let row = session.select_one(StatementId::SelectHistoryTaskById, params)?;
        match row {
            Some(row) => Ok(Some(HistoryTaskEntity::from_row(&row)?)),
            None => Ok(None),
        }
    }
}

impl Default for HistoryTaskDataManager {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// HistoryVariableEntity
// ============================================================================

#[derive(Debug, Clone)]
pub struct HistoryVariableEntity {
    pub id: String,
    pub revision: i32,
    pub process_instance_id: Option<String>,
    pub execution_id: Option<String>,
    pub task_id: Option<String>,
    pub create_time: Option<i64>,
    pub last_updated_time: Option<i64>,
    pub name: Option<String>,
    pub variable_type: Option<String>,
    pub scope_type: Option<String>,
    pub scope_id: Option<String>,
    pub sub_scope_id: Option<String>,
    pub bytearray_id: Option<String>,
    pub double_value: Option<f64>,
    pub long_value: Option<i64>,
    pub text_value: Option<String>,
    pub text2_value: Option<String>,
}

impl HistoryVariableEntity {
    pub fn new(id: String) -> Self {
        Self {
            id,
            revision: 1,
            process_instance_id: None,
            execution_id: None,
            task_id: None,
            create_time: None,
            last_updated_time: None,
            name: None,
            variable_type: None,
            scope_type: None,
            scope_id: None,
            sub_scope_id: None,
            bytearray_id: None,
            double_value: None,
            long_value: None,
            text_value: None,
            text2_value: None,
        }
    }

    pub fn from_row(row: &DbRow) -> Result<Self, PersistenceError> {
        Ok(Self {
            id: row.get_text("ID_").ok_or_else(|| {
                PersistenceError::Deserialization(
                    "Missing ID_ in HistoryVariableEntity".to_string(),
                )
            })?,
            revision: row.get_integer("REV_").unwrap_or(1) as i32,
            process_instance_id: row.get_text("PROC_INST_ID_"),
            execution_id: row.get_text("EXECUTION_ID_"),
            task_id: row.get_text("TASK_ID_"),
            create_time: row.get_integer("CREATE_TIME_"),
            last_updated_time: row.get_integer("LAST_UPDATED_TIME_"),
            name: row.get_text("NAME_"),
            variable_type: row.get_text("VAR_TYPE_"),
            scope_type: row.get_text("SCOPE_TYPE_"),
            scope_id: row.get_text("SCOPE_ID_"),
            sub_scope_id: row.get_text("SUB_SCOPE_ID_"),
            bytearray_id: row.get_text("BYTEARRAY_ID_"),
            double_value: row.get_real("DOUBLE_"),
            long_value: row.get_integer("LONG_"),
            text_value: row.get_text("TEXT_"),
            text2_value: row.get_text("TEXT2_"),
        })
    }
}

impl Entity for HistoryVariableEntity {
    fn id(&self) -> &str {
        &self.id
    }

    fn set_id(&mut self, id: String) {
        self.id = id;
    }

    fn entity_type(&self) -> EntityType {
        EntityType::HistoryVariable
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn clone_box(&self) -> Box<dyn Entity> {
        Box::new(self.clone())
    }
}

impl RevisionedEntity for HistoryVariableEntity {
    fn revision(&self) -> i32 {
        self.revision
    }

    fn set_revision(&mut self, revision: i32) {
        self.revision = revision;
    }
}

pub struct HistoryVariableDataManager;

impl HistoryVariableDataManager {
    pub fn new() -> Self {
        Self
    }

    pub fn insert(
        &self,
        session: &mut DbSession,
        entity: HistoryVariableEntity,
    ) -> Result<(), PersistenceError> {
        let mut params = DbParams::new();
        params.push(entity.id.clone());
        params.push(entity.revision as i64);
        params.push(entity.process_instance_id.clone());
        params.push(entity.execution_id.clone());
        params.push(entity.task_id.clone());
        params.push(entity.create_time);
        params.push(entity.last_updated_time);
        params.push(entity.name.clone());
        params.push(entity.variable_type.clone());
        params.push(entity.scope_type.clone());
        params.push(entity.scope_id.clone());
        params.push(entity.sub_scope_id.clone());
        params.push(entity.bytearray_id.clone());
        params.push(entity.double_value);
        params.push(entity.long_value);
        params.push(entity.text_value.clone());
        params.push(entity.text2_value.clone());

        session.insert(entity, StatementId::InsertHistoryVariable, params)
    }

    pub fn update(
        &self,
        session: &mut DbSession,
        entity: HistoryVariableEntity,
    ) -> Result<(), PersistenceError> {
        let mut params = DbParams::new();
        params.push(entity.revision as i64);
        params.push(entity.process_instance_id.clone());
        params.push(entity.execution_id.clone());
        params.push(entity.task_id.clone());
        params.push(entity.create_time);
        params.push(entity.last_updated_time);
        params.push(entity.name.clone());
        params.push(entity.variable_type.clone());
        params.push(entity.scope_type.clone());
        params.push(entity.scope_id.clone());
        params.push(entity.sub_scope_id.clone());
        params.push(entity.bytearray_id.clone());
        params.push(entity.double_value);
        params.push(entity.long_value);
        params.push(entity.text_value.clone());
        params.push(entity.text2_value.clone());
        params.push(entity.id.clone());
        params.push(entity.revision as i64);

        session.update(entity, StatementId::UpdateHistoryVariable, params)
    }

    pub fn delete(
        &self,
        session: &mut DbSession,
        entity: &HistoryVariableEntity,
    ) -> Result<(), PersistenceError> {
        let mut params = DbParams::new();
        params.push(entity.id.clone());

        session.delete(entity, StatementId::DeleteHistoryVariable, params)
    }

    pub fn find_by_id(
        &self,
        session: &mut DbSession,
        id: &str,
    ) -> Result<Option<HistoryVariableEntity>, PersistenceError> {
        let mut params = DbParams::new();
        params.push(id);

        let row = session.select_one(StatementId::SelectHistoryVariableById, params)?;
        match row {
            Some(row) => Ok(Some(HistoryVariableEntity::from_row(&row)?)),
            None => Ok(None),
        }
    }
}

impl Default for HistoryVariableDataManager {
    fn default() -> Self {
        Self::new()
    }
}
