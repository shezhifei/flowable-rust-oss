use crate::db_session::DbSession;
use crate::entity::{Entity, EntityType};
use crate::error::PersistenceError;
use crate::row::DbRow;
use crate::statement::StatementId;
use crate::value::DbParams;
use std::any::Any;

#[derive(Debug, Clone)]
pub struct AppResolvedCompositionEntity {
    pub id: String,
    pub app_definition_id: String,
    pub app_key: String,
    pub deployment_id: String,
    pub tenant_id: Option<String>,
    pub data: String,
}

impl AppResolvedCompositionEntity {
    pub fn new(
        id: String,
        app_definition_id: String,
        app_key: String,
        deployment_id: String,
        data: String,
    ) -> Self {
        Self {
            id,
            app_definition_id,
            app_key,
            deployment_id,
            tenant_id: None,
            data,
        }
    }

    pub fn set_tenant_id(&mut self, tenant_id: Option<String>) {
        self.tenant_id = tenant_id;
    }

    pub fn from_row(row: &DbRow) -> Result<Self, PersistenceError> {
        Ok(Self {
            id: row.get_text("ID_").ok_or_else(|| {
                PersistenceError::Deserialization(
                    "Missing ID_ in AppResolvedCompositionEntity".to_string(),
                )
            })?,
            app_definition_id: row.get_text("APP_DEFINITION_ID_").ok_or_else(|| {
                PersistenceError::Deserialization(
                    "Missing APP_DEFINITION_ID_ in AppResolvedCompositionEntity".to_string(),
                )
            })?,
            app_key: row.get_text("APP_KEY_").ok_or_else(|| {
                PersistenceError::Deserialization(
                    "Missing APP_KEY_ in AppResolvedCompositionEntity".to_string(),
                )
            })?,
            deployment_id: row.get_text("DEPLOYMENT_ID_").ok_or_else(|| {
                PersistenceError::Deserialization(
                    "Missing DEPLOYMENT_ID_ in AppResolvedCompositionEntity".to_string(),
                )
            })?,
            tenant_id: row.get_text("TENANT_ID_"),
            data: row.get_text("DATA_").ok_or_else(|| {
                PersistenceError::Deserialization(
                    "Missing DATA_ in AppResolvedCompositionEntity".to_string(),
                )
            })?,
        })
    }
}

impl Entity for AppResolvedCompositionEntity {
    fn id(&self) -> &str {
        &self.id
    }

    fn set_id(&mut self, id: String) {
        self.id = id;
    }

    fn entity_type(&self) -> EntityType {
        EntityType::AppResolvedComposition
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn clone_box(&self) -> Box<dyn Entity> {
        Box::new(self.clone())
    }
}

pub struct AppResolvedCompositionDataManager;

impl AppResolvedCompositionDataManager {
    pub fn new() -> Self {
        Self
    }

    pub fn insert(
        &self,
        session: &mut DbSession,
        entity: AppResolvedCompositionEntity,
    ) -> Result<(), PersistenceError> {
        let mut params = DbParams::new();
        params.push(entity.id.clone());
        params.push(entity.app_definition_id.clone());
        params.push(entity.app_key.clone());
        params.push(entity.deployment_id.clone());
        params.push(entity.tenant_id.clone());
        params.push(entity.data.clone());

        session.insert(entity, StatementId::InsertAppResolvedComposition, params)
    }

    pub fn delete(
        &self,
        session: &mut DbSession,
        entity: &AppResolvedCompositionEntity,
    ) -> Result<(), PersistenceError> {
        let mut params = DbParams::new();
        params.push(entity.id.clone());

        session.delete(entity, StatementId::DeleteAppResolvedComposition, params)
    }

    pub fn find_by_id(
        &self,
        session: &mut DbSession,
        id: &str,
    ) -> Result<Option<AppResolvedCompositionEntity>, PersistenceError> {
        let mut params = DbParams::new();
        params.push(id);

        let row = session.select_one(StatementId::SelectAppResolvedCompositionById, params)?;
        match row {
            Some(row) => Ok(Some(AppResolvedCompositionEntity::from_row(&row)?)),
            None => Ok(None),
        }
    }

    pub fn find_by_app_definition_id(
        &self,
        session: &mut DbSession,
        app_definition_id: &str,
    ) -> Result<Option<AppResolvedCompositionEntity>, PersistenceError> {
        let mut params = DbParams::new();
        params.push(app_definition_id);

        let row = session.select_one(
            StatementId::SelectAppResolvedCompositionByAppDefinitionId,
            params,
        )?;
        match row {
            Some(row) => Ok(Some(AppResolvedCompositionEntity::from_row(&row)?)),
            None => Ok(None),
        }
    }

    pub fn find_by_deployment_id(
        &self,
        session: &mut DbSession,
        deployment_id: &str,
    ) -> Result<Vec<AppResolvedCompositionEntity>, PersistenceError> {
        let mut params = DbParams::new();
        params.push(deployment_id);

        let rows = session.select_list(
            StatementId::SelectAppResolvedCompositionsByDeploymentId,
            params,
        )?;
        rows.iter()
            .map(AppResolvedCompositionEntity::from_row)
            .collect()
    }
}

impl Default for AppResolvedCompositionDataManager {
    fn default() -> Self {
        Self::new()
    }
}
