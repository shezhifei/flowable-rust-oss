use crate::db_session::DbSession;
use crate::entity::{Entity, EntityType, RevisionedEntity};
use crate::error::PersistenceError;
use crate::row::DbRow;
use crate::statement::StatementId;
use crate::value::DbParams;
use std::any::Any;

#[derive(Debug, Clone)]
pub struct ProcessDefinitionEntity {
    pub id: String,
    pub category: Option<String>,
    pub name: Option<String>,
    pub key: String,
    pub version: i32,
    pub deployment_id: Option<String>,
    pub resource_name: Option<String>,
    pub dgrm_resource_name: Option<String>,
    pub description: Option<String>,
    pub has_graphical_notation: bool,
    pub has_start_form_key: bool,
    pub suspension_state: i32,
    pub tenant_id: Option<String>,
    pub engine_version: Option<String>,
    pub app_version: Option<i32>,
    pub revision: i32,
}

impl ProcessDefinitionEntity {
    pub fn new(id: String, key: String, version: i32) -> Self {
        Self {
            id,
            category: None,
            name: None,
            key,
            version,
            deployment_id: None,
            resource_name: None,
            dgrm_resource_name: None,
            description: None,
            has_graphical_notation: false,
            has_start_form_key: false,
            suspension_state: 1,
            tenant_id: None,
            engine_version: None,
            app_version: None,
            revision: 1,
        }
    }

    pub fn from_row(row: &DbRow) -> Result<Self, PersistenceError> {
        Ok(Self {
            id: row.get_text("ID_").ok_or_else(|| {
                PersistenceError::Deserialization(
                    "Missing ID_ in ProcessDefinitionEntity".to_string(),
                )
            })?,
            category: row.get_text("CATEGORY_"),
            name: row.get_text("NAME_"),
            key: row.get_text("KEY_").ok_or_else(|| {
                PersistenceError::Deserialization(
                    "Missing KEY_ in ProcessDefinitionEntity".to_string(),
                )
            })?,
            version: row.get_integer("VERSION_").unwrap_or(1) as i32,
            deployment_id: row.get_text("DEPLOYMENT_ID_"),
            resource_name: row.get_text("RESOURCE_NAME_"),
            dgrm_resource_name: row.get_text("DGRM_RESOURCE_NAME_"),
            description: row.get_text("DESCRIPTION_"),
            has_graphical_notation: row.get_integer("HAS_GRAPHICAL_NOTATION_").unwrap_or(0) != 0,
            has_start_form_key: row.get_integer("HAS_START_FORM_KEY_").unwrap_or(0) != 0,
            suspension_state: row.get_integer("SUSPENSION_STATE_").unwrap_or(1) as i32,
            tenant_id: row.get_text("TENANT_ID_"),
            engine_version: row.get_text("ENGINE_VERSION_"),
            app_version: row.get_integer("APP_VERSION_").map(|v| v as i32),
            revision: row.get_integer("REV_").unwrap_or(1) as i32,
        })
    }
}

impl Entity for ProcessDefinitionEntity {
    fn id(&self) -> &str {
        &self.id
    }

    fn set_id(&mut self, id: String) {
        self.id = id;
    }

    fn entity_type(&self) -> EntityType {
        EntityType::ProcessDefinition
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn clone_box(&self) -> Box<dyn Entity> {
        Box::new(self.clone())
    }
}

impl RevisionedEntity for ProcessDefinitionEntity {
    fn revision(&self) -> i32 {
        self.revision
    }

    fn set_revision(&mut self, revision: i32) {
        self.revision = revision;
    }
}

pub struct ProcessDefinitionDataManager;

impl ProcessDefinitionDataManager {
    pub fn new() -> Self {
        Self
    }

    pub fn insert(
        &self,
        session: &mut DbSession,
        entity: ProcessDefinitionEntity,
    ) -> Result<(), PersistenceError> {
        let mut params = DbParams::new();
        params.push(entity.id.clone());
        params.push(entity.revision as i64);
        params.push(entity.category.clone());
        params.push(entity.name.clone());
        params.push(entity.key.clone());
        params.push(entity.version as i64);
        params.push(entity.deployment_id.clone());
        params.push(entity.resource_name.clone());
        params.push(entity.dgrm_resource_name.clone());
        params.push(entity.description.clone());
        // Schema stores these as INTEGER (0/1) on all backends; PG rejects bool binds.
        params.push(if entity.has_graphical_notation {
            1i64
        } else {
            0i64
        });
        params.push(if entity.has_start_form_key { 1i64 } else { 0i64 });
        params.push(entity.suspension_state as i64);
        params.push(entity.tenant_id.clone());
        params.push(entity.engine_version.clone());
        params.push(entity.app_version.map(|v| v as i64));

        session.insert(entity, StatementId::InsertProcessDefinition, params)
    }

    pub fn update(
        &self,
        session: &mut DbSession,
        entity: ProcessDefinitionEntity,
    ) -> Result<(), PersistenceError> {
        let mut params = DbParams::new();
        params.push(entity.revision as i64);
        params.push(entity.category.clone());
        params.push(entity.name.clone());
        params.push(entity.key.clone());
        params.push(entity.version as i64);
        params.push(entity.deployment_id.clone());
        params.push(entity.resource_name.clone());
        params.push(entity.dgrm_resource_name.clone());
        params.push(entity.description.clone());
        params.push(if entity.has_graphical_notation {
            1i64
        } else {
            0i64
        });
        params.push(if entity.has_start_form_key { 1i64 } else { 0i64 });
        params.push(entity.suspension_state as i64);
        params.push(entity.tenant_id.clone());
        params.push(entity.engine_version.clone());
        params.push(entity.app_version.map(|v| v as i64));
        params.push(entity.id.clone());
        params.push(entity.revision as i64);

        session.update(entity, StatementId::UpdateProcessDefinition, params)
    }

    pub fn delete(
        &self,
        session: &mut DbSession,
        entity: &ProcessDefinitionEntity,
    ) -> Result<(), PersistenceError> {
        let mut params = DbParams::new();
        params.push(entity.id.clone());

        session.delete(entity, StatementId::DeleteProcessDefinition, params)
    }

    pub fn find_by_id(
        &self,
        session: &mut DbSession,
        id: &str,
    ) -> Result<Option<ProcessDefinitionEntity>, PersistenceError> {
        let mut params = DbParams::new();
        params.push(id);

        let row = session.select_one(StatementId::SelectProcessDefinitionById, params)?;
        match row {
            Some(row) => Ok(Some(ProcessDefinitionEntity::from_row(&row)?)),
            None => Ok(None),
        }
    }

    pub fn find_by_key(
        &self,
        session: &mut DbSession,
        key: &str,
    ) -> Result<Vec<ProcessDefinitionEntity>, PersistenceError> {
        let mut params = DbParams::new();
        params.push(key);

        let rows = session.select_list(StatementId::SelectProcessDefinitionByKey, params)?;
        rows.iter().map(ProcessDefinitionEntity::from_row).collect()
    }

    pub fn find_by_key_and_version(
        &self,
        session: &mut DbSession,
        key: &str,
        version: i32,
    ) -> Result<Option<ProcessDefinitionEntity>, PersistenceError> {
        let mut params = DbParams::new();
        params.push(key);
        params.push(version as i64);

        let row =
            session.select_one(StatementId::SelectProcessDefinitionByKeyAndVersion, params)?;
        match row {
            Some(row) => Ok(Some(ProcessDefinitionEntity::from_row(&row)?)),
            None => Ok(None),
        }
    }
}

impl Default for ProcessDefinitionDataManager {
    fn default() -> Self {
        Self::new()
    }
}
