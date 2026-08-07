use crate::db_session::DbSession;
use crate::entity::{Entity, EntityType, RevisionedEntity};
use crate::error::PersistenceError;
use crate::row::DbRow;
use crate::statement::StatementId;
use crate::value::DbParams;
use std::any::Any;

#[derive(Debug, Clone)]
pub struct IdentityLinkEntity {
    pub id: String,
    pub revision: i32,
    pub group_id: Option<String>,
    pub link_type: Option<String>,
    pub user_id: Option<String>,
    pub task_id: Option<String>,
    pub process_instance_id: Option<String>,
    pub process_definition_id: Option<String>,
    pub scope_id: Option<String>,
    pub scope_type: Option<String>,
    pub scope_definition_id: Option<String>,
    pub sub_scope_id: Option<String>,
}

impl IdentityLinkEntity {
    pub fn new(id: String) -> Self {
        Self {
            id,
            revision: 1,
            group_id: None,
            link_type: None,
            user_id: None,
            task_id: None,
            process_instance_id: None,
            process_definition_id: None,
            scope_id: None,
            scope_type: None,
            scope_definition_id: None,
            sub_scope_id: None,
        }
    }

    pub fn from_row(row: &DbRow) -> Result<Self, PersistenceError> {
        Ok(Self {
            id: row.get_text("ID_").ok_or_else(|| {
                PersistenceError::Deserialization("Missing ID_ in IdentityLinkEntity".to_string())
            })?,
            revision: row.get_integer("REV_").unwrap_or(1) as i32,
            group_id: row.get_text("GROUP_ID_"),
            link_type: row.get_text("TYPE_"),
            user_id: row.get_text("USER_ID_"),
            task_id: row.get_text("TASK_ID_"),
            process_instance_id: row.get_text("PROC_INST_ID_"),
            process_definition_id: row.get_text("PROC_DEF_ID_"),
            scope_id: row.get_text("SCOPE_ID_"),
            scope_type: row.get_text("SCOPE_TYPE_"),
            scope_definition_id: row.get_text("SCOPE_DEFINITION_ID_"),
            sub_scope_id: row.get_text("SUB_SCOPE_ID_"),
        })
    }
}

impl Entity for IdentityLinkEntity {
    fn id(&self) -> &str {
        &self.id
    }

    fn set_id(&mut self, id: String) {
        self.id = id;
    }

    fn entity_type(&self) -> EntityType {
        EntityType::IdentityLink
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn clone_box(&self) -> Box<dyn Entity> {
        Box::new(self.clone())
    }
}

impl RevisionedEntity for IdentityLinkEntity {
    fn revision(&self) -> i32 {
        self.revision
    }

    fn set_revision(&mut self, revision: i32) {
        self.revision = revision;
    }
}

pub struct IdentityLinkDataManager;

impl IdentityLinkDataManager {
    pub fn new() -> Self {
        Self
    }

    pub fn insert(
        &self,
        session: &mut DbSession,
        entity: IdentityLinkEntity,
    ) -> Result<(), PersistenceError> {
        let mut params = DbParams::new();
        params.push(entity.id.clone());
        params.push(entity.revision as i64);
        params.push(entity.group_id.clone());
        params.push(entity.link_type.clone());
        params.push(entity.user_id.clone());
        params.push(entity.task_id.clone());
        params.push(entity.process_instance_id.clone());
        params.push(entity.process_definition_id.clone());
        params.push(entity.scope_id.clone());
        params.push(entity.scope_type.clone());
        params.push(entity.scope_definition_id.clone());
        params.push(entity.sub_scope_id.clone());

        session.insert(entity, StatementId::InsertIdentityLink, params)
    }

    pub fn delete(
        &self,
        session: &mut DbSession,
        entity: &IdentityLinkEntity,
    ) -> Result<(), PersistenceError> {
        let mut params = DbParams::new();
        params.push(entity.id.clone());

        session.delete(entity, StatementId::DeleteIdentityLink, params)
    }

    pub fn find_by_task_id(
        &self,
        session: &mut DbSession,
        task_id: &str,
    ) -> Result<Vec<IdentityLinkEntity>, PersistenceError> {
        let mut params = DbParams::new();
        params.push(task_id);

        let rows = session.select_list(StatementId::SelectIdentityLinksByTaskId, params)?;
        rows.iter().map(IdentityLinkEntity::from_row).collect()
    }

    pub fn find_by_execution_id(
        &self,
        session: &mut DbSession,
        execution_id: &str,
    ) -> Result<Vec<IdentityLinkEntity>, PersistenceError> {
        let mut params = DbParams::new();
        params.push(execution_id);

        let rows = session.select_list(StatementId::SelectIdentityLinksByExecutionId, params)?;
        rows.iter().map(IdentityLinkEntity::from_row).collect()
    }

    pub fn find_by_process_instance_id(
        &self,
        session: &mut DbSession,
        process_instance_id: &str,
    ) -> Result<Vec<IdentityLinkEntity>, PersistenceError> {
        let mut params = DbParams::new();
        params.push(process_instance_id);

        let rows =
            session.select_list(StatementId::SelectIdentityLinksByProcessInstanceId, params)?;
        rows.iter().map(IdentityLinkEntity::from_row).collect()
    }
}

impl Default for IdentityLinkDataManager {
    fn default() -> Self {
        Self::new()
    }
}
