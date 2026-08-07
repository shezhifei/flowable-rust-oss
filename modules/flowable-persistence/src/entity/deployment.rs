use crate::db_session::DbSession;
use crate::entity::{Entity, EntityType, RevisionedEntity};
use crate::error::PersistenceError;
use crate::row::DbRow;
use crate::statement::StatementId;
use crate::value::DbParams;
use std::any::Any;

#[derive(Debug, Clone)]
pub struct DeploymentEntity {
    pub id: String,
    pub name: String,
    pub category: Option<String>,
    pub key: Option<String>,
    pub tenant_id: Option<String>,
    pub deploy_time: Option<i64>,
    pub engine_version: Option<String>,
    pub revision: i32,
}

impl DeploymentEntity {
    pub fn new(id: String, name: String) -> Self {
        Self {
            id,
            name,
            category: None,
            key: None,
            tenant_id: None,
            deploy_time: None,
            engine_version: None,
            revision: 1,
        }
    }

    pub fn from_row(row: &DbRow) -> Result<Self, PersistenceError> {
        Ok(Self {
            id: row.get_text("ID_").ok_or_else(|| {
                PersistenceError::Deserialization("Missing ID_ in DeploymentEntity".to_string())
            })?,
            name: row.get_text("NAME_").ok_or_else(|| {
                PersistenceError::Deserialization("Missing NAME_ in DeploymentEntity".to_string())
            })?,
            category: row.get_text("CATEGORY_"),
            key: row.get_text("KEY_"),
            tenant_id: row.get_text("TENANT_ID_"),
            deploy_time: row.get_integer("DEPLOY_TIME_"),
            engine_version: row.get_text("ENGINE_VERSION_"),
            revision: row.get_integer("REV_").unwrap_or(1) as i32,
        })
    }
}

impl Entity for DeploymentEntity {
    fn id(&self) -> &str {
        &self.id
    }

    fn set_id(&mut self, id: String) {
        self.id = id;
    }

    fn entity_type(&self) -> EntityType {
        EntityType::Deployment
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn clone_box(&self) -> Box<dyn Entity> {
        Box::new(self.clone())
    }
}

impl RevisionedEntity for DeploymentEntity {
    fn revision(&self) -> i32 {
        self.revision
    }

    fn set_revision(&mut self, revision: i32) {
        self.revision = revision;
    }
}

pub struct DeploymentDataManager;

impl DeploymentDataManager {
    pub fn new() -> Self {
        Self
    }

    pub fn insert(
        &self,
        session: &mut DbSession,
        entity: DeploymentEntity,
    ) -> Result<(), PersistenceError> {
        let mut params = DbParams::new();
        params.push(entity.id.clone());
        params.push(entity.revision as i64);
        params.push(entity.name.clone());
        params.push(entity.category.clone());
        params.push(entity.key.clone());
        params.push(entity.tenant_id.clone());
        params.push(entity.deploy_time);
        params.push(entity.engine_version.clone());

        session.insert(entity, StatementId::InsertDeployment, params)
    }

    pub fn update(
        &self,
        session: &mut DbSession,
        entity: DeploymentEntity,
    ) -> Result<(), PersistenceError> {
        let mut params = DbParams::new();
        params.push(entity.revision as i64);
        params.push(entity.name.clone());
        params.push(entity.category.clone());
        params.push(entity.key.clone());
        params.push(entity.tenant_id.clone());
        params.push(entity.deploy_time);
        params.push(entity.engine_version.clone());
        params.push(entity.id.clone());
        params.push(entity.revision as i64);

        session.update(entity, StatementId::UpdateDeployment, params)
    }

    pub fn delete(
        &self,
        session: &mut DbSession,
        entity: &DeploymentEntity,
    ) -> Result<(), PersistenceError> {
        let mut params = DbParams::new();
        params.push(entity.id.clone());

        session.delete(entity, StatementId::DeleteDeployment, params)
    }

    pub fn find_by_id(
        &self,
        session: &mut DbSession,
        id: &str,
    ) -> Result<Option<DeploymentEntity>, PersistenceError> {
        let mut params = DbParams::new();
        params.push(id);

        let row = session.select_one(StatementId::SelectDeploymentById, params)?;
        match row {
            Some(row) => Ok(Some(DeploymentEntity::from_row(&row)?)),
            None => Ok(None),
        }
    }

    pub fn find_all(
        &self,
        session: &mut DbSession,
    ) -> Result<Vec<DeploymentEntity>, PersistenceError> {
        let params = DbParams::new();
        let rows = session.select_list(StatementId::SelectAllDeployments, params)?;
        rows.iter().map(DeploymentEntity::from_row).collect()
    }
}

impl Default for DeploymentDataManager {
    fn default() -> Self {
        Self::new()
    }
}
