use crate::db_session::DbSession;
use crate::entity::{Entity, EntityType, RevisionedEntity};
use crate::error::PersistenceError;
use crate::row::DbRow;
use crate::statement::StatementId;
use crate::value::DbParams;
use std::any::Any;

#[derive(Debug, Clone)]
pub struct DmnDeploymentEntity {
    pub id: String,
    pub name: String,
    pub category: Option<String>,
    pub parent_deployment_id: Option<String>,
    pub tenant_id: Option<String>,
    pub deployed_at: String,
    pub data: String,
}

impl DmnDeploymentEntity {
    pub fn new(id: String, name: String, deployed_at: String, data: String) -> Self {
        Self {
            id,
            name,
            category: None,
            parent_deployment_id: None,
            tenant_id: None,
            deployed_at,
            data,
        }
    }

    pub fn set_category(&mut self, category: Option<String>) {
        self.category = category;
    }

    pub fn set_parent_deployment_id(&mut self, parent_deployment_id: Option<String>) {
        self.parent_deployment_id = parent_deployment_id;
    }

    pub fn set_tenant_id(&mut self, tenant_id: Option<String>) {
        self.tenant_id = tenant_id;
    }

    pub fn from_row(row: &DbRow) -> Result<Self, PersistenceError> {
        Ok(Self {
            id: row.get_text("ID_").ok_or_else(|| {
                PersistenceError::Deserialization("Missing ID_ in DmnDeploymentEntity".to_string())
            })?,
            name: row.get_text("NAME_").ok_or_else(|| {
                PersistenceError::Deserialization(
                    "Missing NAME_ in DmnDeploymentEntity".to_string(),
                )
            })?,
            category: row.get_text("CATEGORY_"),
            parent_deployment_id: row.get_text("PARENT_DEPLOYMENT_ID_"),
            tenant_id: row.get_text("TENANT_ID_"),
            deployed_at: row.get_text("DEPLOYED_AT_").ok_or_else(|| {
                PersistenceError::Deserialization(
                    "Missing DEPLOYED_AT_ in DmnDeploymentEntity".to_string(),
                )
            })?,
            data: row.get_text("DATA_").ok_or_else(|| {
                PersistenceError::Deserialization(
                    "Missing DATA_ in DmnDeploymentEntity".to_string(),
                )
            })?,
        })
    }
}

impl Entity for DmnDeploymentEntity {
    fn id(&self) -> &str {
        &self.id
    }

    fn set_id(&mut self, id: String) {
        self.id = id;
    }

    fn entity_type(&self) -> EntityType {
        EntityType::DmnDeployment
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn clone_box(&self) -> Box<dyn Entity> {
        Box::new(self.clone())
    }
}

impl RevisionedEntity for DmnDeploymentEntity {
    fn revision(&self) -> i32 {
        1
    }

    fn set_revision(&mut self, _revision: i32) {
        // DMN deployments don't have revision tracking
    }
}

pub struct DmnDeploymentDataManager;

impl DmnDeploymentDataManager {
    pub fn new() -> Self {
        Self
    }

    pub fn insert(
        &self,
        session: &mut DbSession,
        entity: DmnDeploymentEntity,
    ) -> Result<(), PersistenceError> {
        let mut params = DbParams::new();
        params.push(entity.id.clone());
        params.push(entity.name.clone());
        params.push(entity.category.clone());
        params.push(entity.parent_deployment_id.clone());
        params.push(entity.tenant_id.clone());
        params.push(entity.deployed_at.clone());
        params.push(entity.data.clone());

        session.insert(entity, StatementId::InsertDmnDeployment, params)
    }

    pub fn delete(
        &self,
        session: &mut DbSession,
        entity: &DmnDeploymentEntity,
    ) -> Result<(), PersistenceError> {
        let mut params = DbParams::new();
        params.push(entity.id.clone());

        session.delete(entity, StatementId::DeleteDmnDeployment, params)
    }

    pub fn find_by_id(
        &self,
        session: &mut DbSession,
        id: &str,
    ) -> Result<Option<DmnDeploymentEntity>, PersistenceError> {
        let mut params = DbParams::new();
        params.push(id);

        let row = session.select_one(StatementId::SelectDmnDeploymentById, params)?;
        match row {
            Some(row) => Ok(Some(DmnDeploymentEntity::from_row(&row)?)),
            None => Ok(None),
        }
    }

    pub fn find_all(
        &self,
        session: &mut DbSession,
    ) -> Result<Vec<DmnDeploymentEntity>, PersistenceError> {
        let params = DbParams::new();
        let rows = session.select_list(StatementId::SelectAllDmnDeployments, params)?;
        rows.iter().map(DmnDeploymentEntity::from_row).collect()
    }
}

impl Default for DmnDeploymentDataManager {
    fn default() -> Self {
        Self::new()
    }
}
