use crate::db_session::DbSession;
use crate::entity::{Entity, EntityType, RevisionedEntity};
use crate::error::PersistenceError;
use crate::row::DbRow;
use crate::statement::StatementId;
use crate::value::DbParams;
use std::any::Any;

#[derive(Debug, Clone)]
pub struct SuspendedJobEntity {
    pub id: String,
    pub revision: i32,
    pub job_type: Option<String>,
    pub process_definition_id: Option<String>,
    pub process_instance_id: Option<String>,
    pub execution_id: Option<String>,
    pub name: Option<String>,
    pub scope_type: Option<String>,
    pub scope_id: Option<String>,
    pub sub_scope_id: Option<String>,
    pub create_time: Option<i64>,
    pub lock_owner: Option<String>,
    pub lock_time: Option<i64>,
    pub exclusive: bool,
    pub execution: Option<String>,
    pub process_definition: Option<String>,
    pub retries: Option<i32>,
    pub exception_stack_id: Option<String>,
    pub exception_msg: Option<String>,
    pub duedate: Option<i64>,
    pub repeat: Option<String>,
    pub handler_type: Option<String>,
    pub tenant_id: Option<String>,
    pub custom_values_id: Option<String>,
    pub job_handler_type: Option<String>,
    pub job_handler_cfg: Option<String>,
    pub lock_exp_time: Option<i64>,
}

impl SuspendedJobEntity {
    pub fn new(id: String) -> Self {
        Self {
            id,
            revision: 1,
            job_type: None,
            process_definition_id: None,
            process_instance_id: None,
            execution_id: None,
            name: None,
            scope_type: None,
            scope_id: None,
            sub_scope_id: None,
            create_time: None,
            lock_owner: None,
            lock_time: None,
            exclusive: false,
            execution: None,
            process_definition: None,
            retries: None,
            exception_stack_id: None,
            exception_msg: None,
            duedate: None,
            repeat: None,
            handler_type: None,
            tenant_id: None,
            custom_values_id: None,
            job_handler_type: None,
            job_handler_cfg: None,
            lock_exp_time: None,
        }
    }

    pub fn from_row(row: &DbRow) -> Result<Self, PersistenceError> {
        Ok(Self {
            id: row.get_text("ID_").ok_or_else(|| {
                PersistenceError::Deserialization("Missing ID_ in SuspendedJobEntity".to_string())
            })?,
            revision: row.get_integer("REV_").unwrap_or(1) as i32,
            job_type: row.get_text("TYPE_"),
            process_definition_id: row.get_text("PROC_DEF_ID_"),
            process_instance_id: row.get_text("PROC_INST_ID_"),
            execution_id: row.get_text("EXECUTION_ID_"),
            name: row.get_text("NAME_"),
            scope_type: row.get_text("SCOPE_TYPE_"),
            scope_id: row.get_text("SCOPE_ID_"),
            sub_scope_id: row.get_text("SUB_SCOPE_ID_"),
            create_time: row.get_integer("CREATE_TIME_"),
            lock_owner: row.get_text("LOCK_OWNER_"),
            lock_time: row.get_integer("LOCK_TIME_"),
            exclusive: row
                .get_integer("EXCLUSIVE_")
                .map(|v| v != 0)
                .unwrap_or(false),
            execution: row.get_text("EXECUTION_"),
            process_definition: row.get_text("PROCESS_DEFINITION_"),
            retries: row.get_integer("RETRIES_").map(|v| v as i32),
            exception_stack_id: row.get_text("EXCEPTION_STACK_ID_"),
            exception_msg: row.get_text("EXCEPTION_MSG_"),
            duedate: row.get_integer("DUEDATE_"),
            repeat: row.get_text("REPEAT_"),
            handler_type: row.get_text("HANDLER_TYPE_"),
            tenant_id: row.get_text("TENANT_ID_"),
            custom_values_id: row.get_text("CUSTOM_VALUES_ID_"),
            job_handler_type: row.get_text("JOB_HANDLER_TYPE_"),
            job_handler_cfg: row.get_text("JOB_HANDLER_CFG_"),
            lock_exp_time: row.get_integer("LOCK_EXP_TIME_"),
        })
    }
}

impl Entity for SuspendedJobEntity {
    fn id(&self) -> &str {
        &self.id
    }

    fn set_id(&mut self, id: String) {
        self.id = id;
    }

    fn entity_type(&self) -> EntityType {
        EntityType::SuspendedJob
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn clone_box(&self) -> Box<dyn Entity> {
        Box::new(self.clone())
    }
}

impl RevisionedEntity for SuspendedJobEntity {
    fn revision(&self) -> i32 {
        self.revision
    }

    fn set_revision(&mut self, revision: i32) {
        self.revision = revision;
    }
}

pub struct SuspendedJobDataManager;

impl SuspendedJobDataManager {
    pub fn new() -> Self {
        Self
    }

    pub fn insert(
        &self,
        session: &mut DbSession,
        entity: SuspendedJobEntity,
    ) -> Result<(), PersistenceError> {
        let mut params = DbParams::new();
        params.push(entity.id.clone());
        params.push(entity.revision as i64);
        params.push(entity.job_type.clone());
        params.push(entity.process_definition_id.clone());
        params.push(entity.process_instance_id.clone());
        params.push(entity.execution_id.clone());
        params.push(entity.name.clone());
        params.push(entity.scope_type.clone());
        params.push(entity.scope_id.clone());
        params.push(entity.sub_scope_id.clone());
        params.push(entity.create_time);
        params.push(entity.lock_owner.clone());
        params.push(entity.lock_time);
        params.push(if entity.exclusive { 1i64 } else { 0i64 });
        params.push(entity.execution.clone());
        params.push(entity.process_definition.clone());
        params.push(entity.retries.map(|v| v as i64));
        params.push(entity.exception_stack_id.clone());
        params.push(entity.exception_msg.clone());
        params.push(entity.duedate);
        params.push(entity.repeat.clone());
        params.push(entity.handler_type.clone());
        params.push(entity.tenant_id.clone());
        params.push(entity.custom_values_id.clone());
        params.push(entity.job_handler_type.clone());
        params.push(entity.job_handler_cfg.clone());
        params.push(entity.lock_exp_time);

        session.insert(entity, StatementId::InsertSuspendedJob, params)
    }

    pub fn update(
        &self,
        session: &mut DbSession,
        entity: SuspendedJobEntity,
    ) -> Result<(), PersistenceError> {
        let mut params = DbParams::new();
        params.push(entity.revision as i64);
        params.push(entity.job_type.clone());
        params.push(entity.process_definition_id.clone());
        params.push(entity.process_instance_id.clone());
        params.push(entity.execution_id.clone());
        params.push(entity.name.clone());
        params.push(entity.scope_type.clone());
        params.push(entity.scope_id.clone());
        params.push(entity.sub_scope_id.clone());
        params.push(entity.create_time);
        params.push(entity.lock_owner.clone());
        params.push(entity.lock_time);
        params.push(if entity.exclusive { 1i64 } else { 0i64 });
        params.push(entity.execution.clone());
        params.push(entity.process_definition.clone());
        params.push(entity.retries.map(|v| v as i64));
        params.push(entity.exception_stack_id.clone());
        params.push(entity.exception_msg.clone());
        params.push(entity.duedate);
        params.push(entity.repeat.clone());
        params.push(entity.handler_type.clone());
        params.push(entity.tenant_id.clone());
        params.push(entity.custom_values_id.clone());
        params.push(entity.job_handler_type.clone());
        params.push(entity.job_handler_cfg.clone());
        params.push(entity.lock_exp_time);
        params.push(entity.id.clone());
        params.push(entity.revision as i64);

        session.update(entity, StatementId::UpdateSuspendedJob, params)
    }

    pub fn delete(
        &self,
        session: &mut DbSession,
        entity: &SuspendedJobEntity,
    ) -> Result<(), PersistenceError> {
        let mut params = DbParams::new();
        params.push(entity.id.clone());

        session.delete(entity, StatementId::DeleteSuspendedJob, params)
    }

    pub fn find_by_id(
        &self,
        session: &mut DbSession,
        id: &str,
    ) -> Result<Option<SuspendedJobEntity>, PersistenceError> {
        let mut params = DbParams::new();
        params.push(id);

        let row = session.select_one(StatementId::SelectSuspendedJobById, params)?;
        match row {
            Some(row) => Ok(Some(SuspendedJobEntity::from_row(&row)?)),
            None => Ok(None),
        }
    }
}

impl Default for SuspendedJobDataManager {
    fn default() -> Self {
        Self::new()
    }
}
