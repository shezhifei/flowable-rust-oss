use crate::db_session::DbSession;
use crate::entity::{Entity, EntityType, RevisionedEntity};
use crate::error::PersistenceError;
use crate::row::DbRow;
use crate::statement::StatementId;
use crate::value::DbParams;
use std::any::Any;

#[derive(Debug, Clone)]
pub struct DmnDecisionDefinitionEntity {
    pub id: String,
    pub decision_key: String,
    pub deployment_id: String,
    pub tenant_id: Option<String>,
    pub version: i32,
    pub resource_name: String,
    pub data: String,
}

impl DmnDecisionDefinitionEntity {
    pub fn new(
        id: String,
        decision_key: String,
        deployment_id: String,
        version: i32,
        resource_name: String,
        data: String,
    ) -> Self {
        Self {
            id,
            decision_key,
            deployment_id,
            tenant_id: None,
            version,
            resource_name,
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
                    "Missing ID_ in DmnDecisionDefinitionEntity".to_string(),
                )
            })?,
            decision_key: row.get_text("DECISION_KEY_").ok_or_else(|| {
                PersistenceError::Deserialization(
                    "Missing DECISION_KEY_ in DmnDecisionDefinitionEntity".to_string(),
                )
            })?,
            deployment_id: row.get_text("DEPLOYMENT_ID_").ok_or_else(|| {
                PersistenceError::Deserialization(
                    "Missing DEPLOYMENT_ID_ in DmnDecisionDefinitionEntity".to_string(),
                )
            })?,
            tenant_id: row.get_text("TENANT_ID_"),
            version: row.get_integer("VERSION_").unwrap_or(1) as i32,
            resource_name: row.get_text("RESOURCE_NAME_").ok_or_else(|| {
                PersistenceError::Deserialization(
                    "Missing RESOURCE_NAME_ in DmnDecisionDefinitionEntity".to_string(),
                )
            })?,
            data: row.get_text("DATA_").ok_or_else(|| {
                PersistenceError::Deserialization(
                    "Missing DATA_ in DmnDecisionDefinitionEntity".to_string(),
                )
            })?,
        })
    }
}

impl Entity for DmnDecisionDefinitionEntity {
    fn id(&self) -> &str {
        &self.id
    }

    fn set_id(&mut self, id: String) {
        self.id = id;
    }

    fn entity_type(&self) -> EntityType {
        EntityType::DmnDecisionDefinition
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn clone_box(&self) -> Box<dyn Entity> {
        Box::new(self.clone())
    }
}

impl RevisionedEntity for DmnDecisionDefinitionEntity {
    fn revision(&self) -> i32 {
        self.version
    }

    fn set_revision(&mut self, revision: i32) {
        self.version = revision;
    }
}

pub struct DmnDecisionDefinitionDataManager;

impl DmnDecisionDefinitionDataManager {
    pub fn new() -> Self {
        Self
    }

    pub fn insert(
        &self,
        session: &mut DbSession,
        entity: DmnDecisionDefinitionEntity,
    ) -> Result<(), PersistenceError> {
        let mut params = DbParams::new();
        params.push(entity.id.clone());
        params.push(entity.decision_key.clone());
        params.push(entity.deployment_id.clone());
        params.push(entity.tenant_id.clone());
        params.push(entity.version as i64);
        params.push(entity.resource_name.clone());
        params.push(entity.data.clone());

        session.insert(entity, StatementId::InsertDmnDecisionDefinition, params)
    }

    pub fn delete(
        &self,
        session: &mut DbSession,
        entity: &DmnDecisionDefinitionEntity,
    ) -> Result<(), PersistenceError> {
        let mut params = DbParams::new();
        params.push(entity.id.clone());

        session.delete(entity, StatementId::DeleteDmnDecisionDefinition, params)
    }

    pub fn find_by_id(
        &self,
        session: &mut DbSession,
        id: &str,
    ) -> Result<Option<DmnDecisionDefinitionEntity>, PersistenceError> {
        let mut params = DbParams::new();
        params.push(id);

        let row = session.select_one(StatementId::SelectDmnDecisionDefinitionById, params)?;
        match row {
            Some(row) => Ok(Some(DmnDecisionDefinitionEntity::from_row(&row)?)),
            None => Ok(None),
        }
    }

    pub fn find_by_deployment_id(
        &self,
        session: &mut DbSession,
        deployment_id: &str,
    ) -> Result<Vec<DmnDecisionDefinitionEntity>, PersistenceError> {
        let mut params = DbParams::new();
        params.push(deployment_id);

        let rows = session.select_list(
            StatementId::SelectDmnDecisionDefinitionsByDeploymentId,
            params,
        )?;
        rows.iter()
            .map(DmnDecisionDefinitionEntity::from_row)
            .collect()
    }
}

impl Default for DmnDecisionDefinitionDataManager {
    fn default() -> Self {
        Self::new()
    }
}
