use crate::db_session::DbSession;
use crate::entity::{Entity, EntityType, RevisionedEntity};
use crate::error::PersistenceError;
use crate::row::DbRow;
use crate::statement::StatementId;
use crate::value::DbParams;
use std::any::Any;

#[derive(Debug, Clone)]
pub struct ExecutionEntity {
    pub id: String,
    pub revision: i32,
    pub process_instance_id: Option<String>,
    pub business_key: Option<String>,
    pub parent_id: Option<String>,
    pub process_definition_id: Option<String>,
    pub super_execution_id: Option<String>,
    pub root_process_instance_id: Option<String>,
    pub activity_id: Option<String>,
    pub is_active: bool,
    pub is_concurrent: bool,
    pub is_scope: bool,
    pub is_event_scope: bool,
    pub is_mi_root: bool,
    pub suspension_state: i32,
    pub cached_ent_state: Option<i32>,
    pub tenant_id: Option<String>,
    pub name: Option<String>,
    pub start_act_id: Option<String>,
    pub start_time: Option<i64>,
    pub start_user_id: Option<String>,
    pub lock_time: Option<i64>,
    pub is_count_enabled: bool,
    pub evt_subscr_count: i32,
    pub task_count: i32,
    pub job_count: i32,
    pub timer_job_count: i32,
    pub susp_job_count: i32,
    pub deadletter_job_count: i32,
    pub var_count: i32,
    pub id_link_count: i32,
}

impl ExecutionEntity {
    pub fn new(id: String) -> Self {
        Self {
            id,
            revision: 1,
            process_instance_id: None,
            business_key: None,
            parent_id: None,
            process_definition_id: None,
            super_execution_id: None,
            root_process_instance_id: None,
            activity_id: None,
            is_active: true,
            is_concurrent: false,
            is_scope: false,
            is_event_scope: false,
            is_mi_root: false,
            suspension_state: 1,
            cached_ent_state: None,
            tenant_id: None,
            name: None,
            start_act_id: None,
            start_time: None,
            start_user_id: None,
            lock_time: None,
            is_count_enabled: false,
            evt_subscr_count: 0,
            task_count: 0,
            job_count: 0,
            timer_job_count: 0,
            susp_job_count: 0,
            deadletter_job_count: 0,
            var_count: 0,
            id_link_count: 0,
        }
    }

    pub fn from_row(row: &DbRow) -> Result<Self, PersistenceError> {
        Ok(Self {
            id: row.get_text("ID_").ok_or_else(|| {
                PersistenceError::Deserialization("Missing ID_ in ExecutionEntity".to_string())
            })?,
            revision: row.get_integer("REV_").unwrap_or(1) as i32,
            process_instance_id: row.get_text("PROC_INST_ID_"),
            business_key: row.get_text("BUSINESS_KEY_"),
            parent_id: row.get_text("PARENT_ID_"),
            process_definition_id: row.get_text("PROC_DEF_ID_"),
            super_execution_id: row.get_text("SUPER_EXEC_"),
            root_process_instance_id: row.get_text("ROOT_PROC_INST_ID_"),
            activity_id: row.get_text("ACT_ID_"),
            is_active: row.get_integer("IS_ACTIVE_").unwrap_or(0) != 0,
            is_concurrent: row.get_integer("IS_CONCURRENT_").unwrap_or(0) != 0,
            is_scope: row.get_integer("IS_SCOPE_").unwrap_or(0) != 0,
            is_event_scope: row.get_integer("IS_EVENT_SCOPE_").unwrap_or(0) != 0,
            is_mi_root: row.get_integer("IS_MI_ROOT_").unwrap_or(0) != 0,
            suspension_state: row.get_integer("SUSPENSION_STATE_").unwrap_or(1) as i32,
            cached_ent_state: row.get_integer("CACHED_ENT_STATE_").map(|v| v as i32),
            tenant_id: row.get_text("TENANT_ID_"),
            name: row.get_text("NAME_"),
            start_act_id: row.get_text("START_ACT_ID_"),
            start_time: row.get_integer("START_TIME_"),
            start_user_id: row.get_text("START_USER_ID_"),
            lock_time: row.get_integer("LOCK_TIME_"),
            is_count_enabled: row.get_integer("IS_COUNT_ENABLED_").unwrap_or(0) != 0,
            evt_subscr_count: row.get_integer("EVT_SUBSCR_COUNT_").unwrap_or(0) as i32,
            task_count: row.get_integer("TASK_COUNT_").unwrap_or(0) as i32,
            job_count: row.get_integer("JOB_COUNT_").unwrap_or(0) as i32,
            timer_job_count: row.get_integer("TIMER_JOB_COUNT_").unwrap_or(0) as i32,
            susp_job_count: row.get_integer("SUSP_JOB_COUNT_").unwrap_or(0) as i32,
            deadletter_job_count: row.get_integer("DEADLETTER_JOB_COUNT_").unwrap_or(0) as i32,
            var_count: row.get_integer("VAR_COUNT_").unwrap_or(0) as i32,
            id_link_count: row.get_integer("ID_LINK_COUNT_").unwrap_or(0) as i32,
        })
    }
}

impl Entity for ExecutionEntity {
    fn id(&self) -> &str {
        &self.id
    }

    fn set_id(&mut self, id: String) {
        self.id = id;
    }

    fn entity_type(&self) -> EntityType {
        EntityType::Execution
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn clone_box(&self) -> Box<dyn Entity> {
        Box::new(self.clone())
    }
}

impl RevisionedEntity for ExecutionEntity {
    fn revision(&self) -> i32 {
        self.revision
    }

    fn set_revision(&mut self, revision: i32) {
        self.revision = revision;
    }
}

pub struct ExecutionDataManager;

impl ExecutionDataManager {
    pub fn new() -> Self {
        Self
    }

    pub fn insert(
        &self,
        session: &mut DbSession,
        entity: ExecutionEntity,
    ) -> Result<(), PersistenceError> {
        let mut params = DbParams::new();
        params.push(entity.id.clone());
        params.push(entity.revision as i64);
        params.push(entity.process_instance_id.clone());
        params.push(entity.business_key.clone());
        params.push(entity.parent_id.clone());
        params.push(entity.process_definition_id.clone());
        params.push(entity.super_execution_id.clone());
        params.push(entity.root_process_instance_id.clone());
        params.push(entity.activity_id.clone());
        params.push(entity.is_active);
        params.push(entity.is_concurrent);
        params.push(entity.is_scope);
        params.push(entity.is_event_scope);
        params.push(entity.is_mi_root);
        params.push(entity.suspension_state as i64);
        params.push(entity.cached_ent_state.map(|v| v as i64));
        params.push(entity.tenant_id.clone());
        params.push(entity.name.clone());
        params.push(entity.start_act_id.clone());
        params.push(entity.start_time);
        params.push(entity.start_user_id.clone());
        params.push(entity.lock_time);
        params.push(entity.is_count_enabled);
        params.push(entity.evt_subscr_count as i64);
        params.push(entity.task_count as i64);
        params.push(entity.job_count as i64);
        params.push(entity.timer_job_count as i64);
        params.push(entity.susp_job_count as i64);
        params.push(entity.deadletter_job_count as i64);
        params.push(entity.var_count as i64);
        params.push(entity.id_link_count as i64);

        session.insert(entity, StatementId::InsertExecution, params)
    }

    pub fn update(
        &self,
        session: &mut DbSession,
        entity: ExecutionEntity,
    ) -> Result<(), PersistenceError> {
        let mut params = DbParams::new();
        params.push(entity.revision as i64);
        params.push(entity.process_instance_id.clone());
        params.push(entity.business_key.clone());
        params.push(entity.parent_id.clone());
        params.push(entity.process_definition_id.clone());
        params.push(entity.super_execution_id.clone());
        params.push(entity.root_process_instance_id.clone());
        params.push(entity.activity_id.clone());
        params.push(entity.is_active);
        params.push(entity.is_concurrent);
        params.push(entity.is_scope);
        params.push(entity.is_event_scope);
        params.push(entity.is_mi_root);
        params.push(entity.suspension_state as i64);
        params.push(entity.cached_ent_state.map(|v| v as i64));
        params.push(entity.tenant_id.clone());
        params.push(entity.name.clone());
        params.push(entity.start_act_id.clone());
        params.push(entity.start_time);
        params.push(entity.start_user_id.clone());
        params.push(entity.lock_time);
        params.push(entity.is_count_enabled);
        params.push(entity.evt_subscr_count as i64);
        params.push(entity.task_count as i64);
        params.push(entity.job_count as i64);
        params.push(entity.timer_job_count as i64);
        params.push(entity.susp_job_count as i64);
        params.push(entity.deadletter_job_count as i64);
        params.push(entity.var_count as i64);
        params.push(entity.id_link_count as i64);
        params.push(entity.id.clone());
        params.push(entity.revision as i64);

        session.update(entity, StatementId::UpdateExecution, params)
    }

    pub fn delete(
        &self,
        session: &mut DbSession,
        entity: &ExecutionEntity,
    ) -> Result<(), PersistenceError> {
        let mut params = DbParams::new();
        params.push(entity.id.clone());

        session.delete(entity, StatementId::DeleteExecution, params)
    }

    pub fn find_by_id(
        &self,
        session: &mut DbSession,
        id: &str,
    ) -> Result<Option<ExecutionEntity>, PersistenceError> {
        let mut params = DbParams::new();
        params.push(id);

        let row = session.select_one(StatementId::SelectExecutionById, params)?;
        match row {
            Some(row) => Ok(Some(ExecutionEntity::from_row(&row)?)),
            None => Ok(None),
        }
    }

    pub fn find_by_process_instance_id(
        &self,
        session: &mut DbSession,
        process_instance_id: &str,
    ) -> Result<Vec<ExecutionEntity>, PersistenceError> {
        let mut params = DbParams::new();
        params.push(process_instance_id);

        let rows = session.select_list(StatementId::SelectExecutionsByProcessInstanceId, params)?;
        rows.iter().map(ExecutionEntity::from_row).collect()
    }

    pub fn find_by_parent_execution_id(
        &self,
        session: &mut DbSession,
        parent_id: &str,
    ) -> Result<Vec<ExecutionEntity>, PersistenceError> {
        let mut params = DbParams::new();
        params.push(parent_id);

        let rows = session.select_list(StatementId::SelectExecutionsByParentExecutionId, params)?;
        rows.iter().map(ExecutionEntity::from_row).collect()
    }

    pub fn bulk_insert(
        &self,
        session: &mut DbSession,
        entities: Vec<ExecutionEntity>,
    ) -> Result<(), PersistenceError> {
        let mut operations = Vec::new();
        for entity in entities {
            let mut params = DbParams::new();
            params.push(entity.id.clone());
            params.push(entity.revision as i64);
            params.push(entity.process_instance_id.clone());
            params.push(entity.business_key.clone());
            params.push(entity.parent_id.clone());
            params.push(entity.process_definition_id.clone());
            params.push(entity.super_execution_id.clone());
            params.push(entity.root_process_instance_id.clone());
            params.push(entity.activity_id.clone());
            params.push(if entity.is_active { 1i64 } else { 0i64 });
            params.push(if entity.is_concurrent { 1i64 } else { 0i64 });
            params.push(if entity.is_scope { 1i64 } else { 0i64 });
            params.push(if entity.is_event_scope { 1i64 } else { 0i64 });
            params.push(if entity.is_mi_root { 1i64 } else { 0i64 });
            params.push(entity.suspension_state as i64);
            params.push(entity.cached_ent_state.map(|v| v as i64));
            params.push(entity.tenant_id.clone());
            params.push(entity.name.clone());
            params.push(entity.start_act_id.clone());
            params.push(entity.start_time);
            params.push(entity.start_user_id.clone());
            params.push(entity.lock_time);
            params.push(if entity.is_count_enabled { 1i64 } else { 0i64 });
            params.push(entity.evt_subscr_count as i64);
            params.push(entity.task_count as i64);
            params.push(entity.job_count as i64);
            params.push(entity.timer_job_count as i64);
            params.push(entity.susp_job_count as i64);
            params.push(entity.deadletter_job_count as i64);
            params.push(entity.var_count as i64);
            params.push(entity.id_link_count as i64);

            operations.push((StatementId::BulkInsertExecution, params));
        }

        session.bulk_insert(EntityType::Execution, operations)
    }
}

impl Default for ExecutionDataManager {
    fn default() -> Self {
        Self::new()
    }
}
