use crate::db_session::DbSession;
use crate::entity::{Entity, EntityType};
use crate::error::PersistenceError;
use crate::row::DbRow;
use crate::statement::StatementId;
use crate::value::DbParams;
use std::any::Any;

#[derive(Debug, Clone)]
pub struct CmmnDeploymentEntity {
    pub id: String,
    pub name: String,
    pub category: Option<String>,
    pub key: Option<String>,
    pub tenant_id: Option<String>,
    pub parent_deployment_id: Option<String>,
    pub deployed_at: String,
    pub data: String,
}

impl CmmnDeploymentEntity {
    pub fn new(id: String, name: String, deployed_at: String, data: String) -> Self {
        Self {
            id,
            name,
            category: None,
            key: None,
            tenant_id: None,
            parent_deployment_id: None,
            deployed_at,
            data,
        }
    }

    pub fn set_tenant_id(&mut self, tenant_id: Option<String>) {
        self.tenant_id = tenant_id;
    }

    pub fn set_metadata(
        &mut self,
        category: Option<String>,
        key: Option<String>,
        parent_deployment_id: Option<String>,
    ) {
        self.category = category;
        self.key = key;
        self.parent_deployment_id = parent_deployment_id;
    }

    pub fn from_row(row: &DbRow) -> Result<Self, PersistenceError> {
        Ok(Self {
            id: row.get_text("ID_").ok_or_else(|| {
                PersistenceError::Deserialization("Missing ID_ in CmmnDeploymentEntity".to_string())
            })?,
            name: row.get_text("NAME_").ok_or_else(|| {
                PersistenceError::Deserialization(
                    "Missing NAME_ in CmmnDeploymentEntity".to_string(),
                )
            })?,
            category: row.get_text("CATEGORY_"),
            key: row.get_text("KEY_"),
            tenant_id: row.get_text("TENANT_ID_"),
            parent_deployment_id: row.get_text("PARENT_DEPLOYMENT_ID_"),
            deployed_at: row.get_text("DEPLOYED_AT_").ok_or_else(|| {
                PersistenceError::Deserialization(
                    "Missing DEPLOYED_AT_ in CmmnDeploymentEntity".to_string(),
                )
            })?,
            data: row.get_text("DATA_").ok_or_else(|| {
                PersistenceError::Deserialization(
                    "Missing DATA_ in CmmnDeploymentEntity".to_string(),
                )
            })?,
        })
    }
}

impl Entity for CmmnDeploymentEntity {
    fn id(&self) -> &str {
        &self.id
    }

    fn set_id(&mut self, id: String) {
        self.id = id;
    }

    fn entity_type(&self) -> EntityType {
        EntityType::CmmnDeployment
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn clone_box(&self) -> Box<dyn Entity> {
        Box::new(self.clone())
    }
}

pub struct CmmnDeploymentDataManager;

impl CmmnDeploymentDataManager {
    pub fn new() -> Self {
        Self
    }

    pub fn insert(
        &self,
        session: &mut DbSession,
        entity: CmmnDeploymentEntity,
    ) -> Result<(), PersistenceError> {
        let mut params = DbParams::new();
        params.push(entity.id.clone());
        params.push(entity.name.clone());
        params.push(entity.category.clone());
        params.push(entity.key.clone());
        params.push(entity.tenant_id.clone());
        params.push(entity.parent_deployment_id.clone());
        params.push(entity.deployed_at.clone());
        params.push(entity.data.clone());

        session.insert(entity, StatementId::InsertCmmnDeployment, params)
    }

    pub fn delete(
        &self,
        session: &mut DbSession,
        entity: &CmmnDeploymentEntity,
    ) -> Result<(), PersistenceError> {
        let mut params = DbParams::new();
        params.push(entity.id.clone());

        session.delete(entity, StatementId::DeleteCmmnDeployment, params)
    }

    pub fn find_by_id(
        &self,
        session: &mut DbSession,
        id: &str,
    ) -> Result<Option<CmmnDeploymentEntity>, PersistenceError> {
        let mut params = DbParams::new();
        params.push(id);

        let row = session.select_one(StatementId::SelectCmmnDeploymentById, params)?;
        match row {
            Some(row) => Ok(Some(CmmnDeploymentEntity::from_row(&row)?)),
            None => Ok(None),
        }
    }

    pub fn update_parent_id(
        &self,
        session: &mut DbSession,
        id: &str,
        parent_deployment_id: Option<String>,
    ) -> Result<u64, PersistenceError> {
        let mut params = DbParams::new();
        params.push(parent_deployment_id);
        params.push(id);
        Ok(session
            .execute(StatementId::UpdateCmmnDeploymentParentId, params)?
            .rows_affected)
    }

    pub fn find_all(
        &self,
        session: &mut DbSession,
    ) -> Result<Vec<CmmnDeploymentEntity>, PersistenceError> {
        let params = DbParams::new();
        let rows = session.select_list(StatementId::SelectAllCmmnDeployments, params)?;
        rows.iter().map(CmmnDeploymentEntity::from_row).collect()
    }
}

impl Default for CmmnDeploymentDataManager {
    fn default() -> Self {
        Self::new()
    }
}
