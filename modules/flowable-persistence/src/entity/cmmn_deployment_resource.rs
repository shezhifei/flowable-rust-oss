use crate::db_session::DbSession;
use crate::entity::{Entity, EntityType};
use crate::error::PersistenceError;
use crate::row::DbRow;
use crate::statement::StatementId;
use crate::value::DbParams;
use std::any::Any;

#[derive(Debug, Clone)]
pub struct CmmnDeploymentResourceEntity {
    pub deployment_id: String,
    pub resource_name: String,
    pub resource_type: String,
    pub content_type: String,
    pub bytes: Vec<u8>,
    pub created_at: i64,
}

impl CmmnDeploymentResourceEntity {
    pub fn new(
        deployment_id: String,
        resource_name: String,
        resource_type: String,
        content_type: String,
        bytes: Vec<u8>,
        created_at: i64,
    ) -> Self {
        Self {
            deployment_id,
            resource_name,
            resource_type,
            content_type,
            bytes,
            created_at,
        }
    }

    pub fn from_row(row: &DbRow) -> Result<Self, PersistenceError> {
        Ok(Self {
            deployment_id: row.get_text("DEPLOYMENT_ID_").ok_or_else(|| {
                PersistenceError::Deserialization(
                    "Missing DEPLOYMENT_ID_ in CmmnDeploymentResourceEntity".to_string(),
                )
            })?,
            resource_name: row.get_text("RESOURCE_NAME_").ok_or_else(|| {
                PersistenceError::Deserialization(
                    "Missing RESOURCE_NAME_ in CmmnDeploymentResourceEntity".to_string(),
                )
            })?,
            resource_type: row.get_text("RESOURCE_TYPE_").ok_or_else(|| {
                PersistenceError::Deserialization(
                    "Missing RESOURCE_TYPE_ in CmmnDeploymentResourceEntity".to_string(),
                )
            })?,
            content_type: row.get_text("CONTENT_TYPE_").ok_or_else(|| {
                PersistenceError::Deserialization(
                    "Missing CONTENT_TYPE_ in CmmnDeploymentResourceEntity".to_string(),
                )
            })?,
            bytes: row.get_blob("BYTES_").ok_or_else(|| {
                PersistenceError::Deserialization(
                    "Missing BYTES_ in CmmnDeploymentResourceEntity".to_string(),
                )
            })?,
            created_at: row.get_integer("CREATED_AT_").unwrap_or(0),
        })
    }
}

impl Entity for CmmnDeploymentResourceEntity {
    fn id(&self) -> &str {
        &self.deployment_id
    }

    fn set_id(&mut self, id: String) {
        self.deployment_id = id;
    }

    fn entity_type(&self) -> EntityType {
        EntityType::CmmnDeploymentResource
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn clone_box(&self) -> Box<dyn Entity> {
        Box::new(self.clone())
    }
}

pub struct CmmnDeploymentResourceDataManager;

impl CmmnDeploymentResourceDataManager {
    pub fn new() -> Self {
        Self
    }

    pub fn insert(
        &self,
        session: &mut DbSession,
        entity: CmmnDeploymentResourceEntity,
    ) -> Result<(), PersistenceError> {
        let mut params = DbParams::new();
        params.push(entity.deployment_id.clone());
        params.push(entity.resource_name.clone());
        params.push(entity.resource_type.clone());
        params.push(entity.content_type.clone());
        params.push(entity.bytes.clone());
        params.push(entity.created_at);

        session.insert(entity, StatementId::InsertCmmnDeploymentResource, params)
    }

    pub fn delete(
        &self,
        session: &mut DbSession,
        entity: &CmmnDeploymentResourceEntity,
    ) -> Result<(), PersistenceError> {
        let mut params = DbParams::new();
        params.push(entity.deployment_id.clone());
        params.push(entity.resource_name.clone());

        session.delete(entity, StatementId::DeleteCmmnDeploymentResource, params)
    }

    pub fn find_by_id(
        &self,
        session: &mut DbSession,
        deployment_id: &str,
        resource_name: &str,
    ) -> Result<Option<CmmnDeploymentResourceEntity>, PersistenceError> {
        let mut params = DbParams::new();
        params.push(deployment_id);
        params.push(resource_name);

        let row = session.select_one(StatementId::SelectCmmnDeploymentResourceById, params)?;
        match row {
            Some(row) => Ok(Some(CmmnDeploymentResourceEntity::from_row(&row)?)),
            None => Ok(None),
        }
    }

    pub fn find_by_deployment_id(
        &self,
        session: &mut DbSession,
        deployment_id: &str,
    ) -> Result<Vec<CmmnDeploymentResourceEntity>, PersistenceError> {
        let mut params = DbParams::new();
        params.push(deployment_id);

        let rows = session.select_list(
            StatementId::SelectCmmnDeploymentResourcesByDeploymentId,
            params,
        )?;
        rows.iter()
            .map(CmmnDeploymentResourceEntity::from_row)
            .collect()
    }
}

impl Default for CmmnDeploymentResourceDataManager {
    fn default() -> Self {
        Self::new()
    }
}
