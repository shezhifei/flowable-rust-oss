use crate::db_session::DbSession;
use crate::entity::{Entity, EntityType};
use crate::error::PersistenceError;
use crate::row::DbRow;
use crate::statement::StatementId;
use crate::value::DbParams;
use std::any::Any;

#[derive(Debug, Clone)]
pub struct AppDeploymentEntity {
    pub id: String,
    pub name: String,
    pub category: Option<String>,
    pub tenant_id: Option<String>,
    pub deployed_at: String,
    pub data: String,
}

impl AppDeploymentEntity {
    pub fn new(id: String, name: String, deployed_at: String, data: String) -> Self {
        Self {
            id,
            name,
            category: None,
            tenant_id: None,
            deployed_at,
            data,
        }
    }

    pub fn set_category(&mut self, category: Option<String>) {
        self.category = category;
    }

    pub fn set_tenant_id(&mut self, tenant_id: Option<String>) {
        self.tenant_id = tenant_id;
    }

    pub fn from_row(row: &DbRow) -> Result<Self, PersistenceError> {
        Ok(Self {
            id: row.get_text("ID_").ok_or_else(|| {
                PersistenceError::Deserialization("Missing ID_ in AppDeploymentEntity".to_string())
            })?,
            name: row.get_text("NAME_").ok_or_else(|| {
                PersistenceError::Deserialization(
                    "Missing NAME_ in AppDeploymentEntity".to_string(),
                )
            })?,
            category: row.get_text("CATEGORY_"),
            tenant_id: row.get_text("TENANT_ID_"),
            deployed_at: row.get_text("DEPLOYED_AT_").ok_or_else(|| {
                PersistenceError::Deserialization(
                    "Missing DEPLOYED_AT_ in AppDeploymentEntity".to_string(),
                )
            })?,
            data: row.get_text("DATA_").ok_or_else(|| {
                PersistenceError::Deserialization(
                    "Missing DATA_ in AppDeploymentEntity".to_string(),
                )
            })?,
        })
    }
}

impl Entity for AppDeploymentEntity {
    fn id(&self) -> &str {
        &self.id
    }

    fn set_id(&mut self, id: String) {
        self.id = id;
    }

    fn entity_type(&self) -> EntityType {
        EntityType::AppDeployment
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn clone_box(&self) -> Box<dyn Entity> {
        Box::new(self.clone())
    }
}

pub struct AppDeploymentDataManager;

impl AppDeploymentDataManager {
    pub fn new() -> Self {
        Self
    }

    pub fn insert(
        &self,
        session: &mut DbSession,
        entity: AppDeploymentEntity,
    ) -> Result<(), PersistenceError> {
        let mut params = DbParams::new();
        params.push(entity.id.clone());
        params.push(entity.name.clone());
        params.push(entity.category.clone());
        params.push(entity.tenant_id.clone());
        params.push(entity.deployed_at.clone());
        params.push(entity.data.clone());

        session.insert(entity, StatementId::InsertAppDeployment, params)
    }

    pub fn delete(
        &self,
        session: &mut DbSession,
        entity: &AppDeploymentEntity,
    ) -> Result<(), PersistenceError> {
        let mut params = DbParams::new();
        params.push(entity.id.clone());

        session.delete(entity, StatementId::DeleteAppDeployment, params)
    }

    pub fn find_by_id(
        &self,
        session: &mut DbSession,
        id: &str,
    ) -> Result<Option<AppDeploymentEntity>, PersistenceError> {
        let mut params = DbParams::new();
        params.push(id);

        let row = session.select_one(StatementId::SelectAppDeploymentById, params)?;
        match row {
            Some(row) => Ok(Some(AppDeploymentEntity::from_row(&row)?)),
            None => Ok(None),
        }
    }
}

impl Default for AppDeploymentDataManager {
    fn default() -> Self {
        Self::new()
    }
}
