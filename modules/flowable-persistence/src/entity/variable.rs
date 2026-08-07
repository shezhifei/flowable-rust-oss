use crate::db_session::DbSession;
use crate::entity::{Entity, EntityType, RevisionedEntity};
use crate::error::PersistenceError;
use crate::row::DbRow;
use crate::statement::StatementId;
use crate::value::DbParams;
use std::any::Any;

#[derive(Debug, Clone)]
pub struct VariableEntity {
    pub id: String,
    pub revision: i32,
    pub variable_type: Option<String>,
    pub name: Option<String>,
    pub execution_id: Option<String>,
    pub process_instance_id: Option<String>,
    pub task_id: Option<String>,
    pub scope_type: Option<String>,
    pub scope_id: Option<String>,
    pub sub_scope_id: Option<String>,
    pub bytearray_id: Option<String>,
    pub double_value: Option<f64>,
    pub long_value: Option<i64>,
    pub text_value: Option<String>,
    pub text2_value: Option<String>,
    pub is_initial: Option<bool>,
}

impl VariableEntity {
    pub fn new(id: String) -> Self {
        Self {
            id,
            revision: 1,
            variable_type: None,
            name: None,
            execution_id: None,
            process_instance_id: None,
            task_id: None,
            scope_type: None,
            scope_id: None,
            sub_scope_id: None,
            bytearray_id: None,
            double_value: None,
            long_value: None,
            text_value: None,
            text2_value: None,
            is_initial: None,
        }
    }

    pub fn from_row(row: &DbRow) -> Result<Self, PersistenceError> {
        Ok(Self {
            id: row.get_text("ID_").ok_or_else(|| {
                PersistenceError::Deserialization("Missing ID_ in VariableEntity".to_string())
            })?,
            revision: row.get_integer("REV_").unwrap_or(1) as i32,
            variable_type: row.get_text("TYPE_"),
            name: row.get_text("NAME_"),
            execution_id: row.get_text("EXECUTION_ID_"),
            process_instance_id: row.get_text("PROC_INST_ID_"),
            task_id: row.get_text("TASK_ID_"),
            scope_type: row.get_text("SCOPE_TYPE_"),
            scope_id: row.get_text("SCOPE_ID_"),
            sub_scope_id: row.get_text("SUB_SCOPE_ID_"),
            bytearray_id: row.get_text("BYTEARRAY_ID_"),
            double_value: row.get_real("DOUBLE_"),
            long_value: row.get_integer("LONG_"),
            text_value: row.get_text("TEXT_"),
            text2_value: row.get_text("TEXT2_"),
            is_initial: row.get_integer("IS_INITIAL_").map(|v| v != 0),
        })
    }
}

impl Entity for VariableEntity {
    fn id(&self) -> &str {
        &self.id
    }

    fn set_id(&mut self, id: String) {
        self.id = id;
    }

    fn entity_type(&self) -> EntityType {
        EntityType::Variable
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn clone_box(&self) -> Box<dyn Entity> {
        Box::new(self.clone())
    }
}

impl RevisionedEntity for VariableEntity {
    fn revision(&self) -> i32 {
        self.revision
    }

    fn set_revision(&mut self, revision: i32) {
        self.revision = revision;
    }
}

pub struct VariableDataManager;

impl VariableDataManager {
    pub fn new() -> Self {
        Self
    }

    pub fn insert(
        &self,
        session: &mut DbSession,
        entity: VariableEntity,
    ) -> Result<(), PersistenceError> {
        let mut params = DbParams::new();
        params.push(entity.id.clone());
        params.push(entity.revision as i64);
        params.push(entity.variable_type.clone());
        params.push(entity.name.clone());
        params.push(entity.execution_id.clone());
        params.push(entity.process_instance_id.clone());
        params.push(entity.task_id.clone());
        params.push(entity.scope_type.clone());
        params.push(entity.scope_id.clone());
        params.push(entity.sub_scope_id.clone());
        params.push(entity.bytearray_id.clone());
        params.push(entity.double_value);
        params.push(entity.long_value);
        params.push(entity.text_value.clone());
        params.push(entity.text2_value.clone());
        params.push(entity.is_initial.map(|v| if v { 1i64 } else { 0i64 }));

        session.insert(entity, StatementId::InsertVariable, params)
    }

    pub fn update(
        &self,
        session: &mut DbSession,
        entity: VariableEntity,
    ) -> Result<(), PersistenceError> {
        let mut params = DbParams::new();
        params.push(entity.revision as i64);
        params.push(entity.variable_type.clone());
        params.push(entity.name.clone());
        params.push(entity.execution_id.clone());
        params.push(entity.process_instance_id.clone());
        params.push(entity.task_id.clone());
        params.push(entity.scope_type.clone());
        params.push(entity.scope_id.clone());
        params.push(entity.sub_scope_id.clone());
        params.push(entity.bytearray_id.clone());
        params.push(entity.double_value);
        params.push(entity.long_value);
        params.push(entity.text_value.clone());
        params.push(entity.text2_value.clone());
        params.push(entity.is_initial.map(|v| if v { 1i64 } else { 0i64 }));
        params.push(entity.id.clone());
        params.push(entity.revision as i64);

        session.update(entity, StatementId::UpdateVariable, params)
    }

    pub fn delete(
        &self,
        session: &mut DbSession,
        entity: &VariableEntity,
    ) -> Result<(), PersistenceError> {
        let mut params = DbParams::new();
        params.push(entity.id.clone());

        session.delete(entity, StatementId::DeleteVariable, params)
    }

    pub fn find_by_id(
        &self,
        session: &mut DbSession,
        id: &str,
    ) -> Result<Option<VariableEntity>, PersistenceError> {
        let mut params = DbParams::new();
        params.push(id);

        let row = session.select_one(StatementId::SelectVariableById, params)?;
        match row {
            Some(row) => Ok(Some(VariableEntity::from_row(&row)?)),
            None => Ok(None),
        }
    }

    pub fn find_by_execution_id(
        &self,
        session: &mut DbSession,
        execution_id: &str,
    ) -> Result<Vec<VariableEntity>, PersistenceError> {
        let mut params = DbParams::new();
        params.push(execution_id);

        let rows = session.select_list(StatementId::SelectVariablesByExecutionId, params)?;
        rows.iter().map(VariableEntity::from_row).collect()
    }

    pub fn find_by_task_id(
        &self,
        session: &mut DbSession,
        task_id: &str,
    ) -> Result<Vec<VariableEntity>, PersistenceError> {
        let mut params = DbParams::new();
        params.push(task_id);

        let rows = session.select_list(StatementId::SelectVariablesByTaskId, params)?;
        rows.iter().map(VariableEntity::from_row).collect()
    }
}

impl Default for VariableDataManager {
    fn default() -> Self {
        Self::new()
    }
}
