use crate::db_session::DbSession;
use crate::entity::{Entity, EntityType};
use crate::error::PersistenceError;
use crate::row::DbRow;
use crate::statement::StatementId;
use crate::value::DbParams;
use std::any::Any;

#[derive(Debug, Clone)]
pub struct DmnDecisionRequirementsDiagramEntity {
    pub id: String,
    pub name: String,
    pub deployment_id: String,
    pub resource_name: String,
    pub data: String,
}

impl DmnDecisionRequirementsDiagramEntity {
    pub fn new(
        id: String,
        name: String,
        deployment_id: String,
        resource_name: String,
        data: String,
    ) -> Self {
        Self {
            id,
            name,
            deployment_id,
            resource_name,
            data,
        }
    }

    pub fn from_row(row: &DbRow) -> Result<Self, PersistenceError> {
        Ok(Self {
            id: row.get_text("ID_").ok_or_else(|| {
                PersistenceError::Deserialization(
                    "Missing ID_ in DmnDecisionRequirementsDiagramEntity".to_string(),
                )
            })?,
            name: row.get_text("NAME_").ok_or_else(|| {
                PersistenceError::Deserialization(
                    "Missing NAME_ in DmnDecisionRequirementsDiagramEntity".to_string(),
                )
            })?,
            deployment_id: row.get_text("DEPLOYMENT_ID_").ok_or_else(|| {
                PersistenceError::Deserialization(
                    "Missing DEPLOYMENT_ID_ in DmnDecisionRequirementsDiagramEntity".to_string(),
                )
            })?,
            resource_name: row.get_text("RESOURCE_NAME_").ok_or_else(|| {
                PersistenceError::Deserialization(
                    "Missing RESOURCE_NAME_ in DmnDecisionRequirementsDiagramEntity".to_string(),
                )
            })?,
            data: row.get_text("DATA_").ok_or_else(|| {
                PersistenceError::Deserialization(
                    "Missing DATA_ in DmnDecisionRequirementsDiagramEntity".to_string(),
                )
            })?,
        })
    }
}

impl Entity for DmnDecisionRequirementsDiagramEntity {
    fn id(&self) -> &str {
        &self.id
    }

    fn set_id(&mut self, id: String) {
        self.id = id;
    }

    fn entity_type(&self) -> EntityType {
        EntityType::DmnDecisionRequirementsDiagram
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn clone_box(&self) -> Box<dyn Entity> {
        Box::new(self.clone())
    }
}

pub struct DmnDecisionRequirementsDiagramDataManager;

impl DmnDecisionRequirementsDiagramDataManager {
    pub fn new() -> Self {
        Self
    }

    pub fn insert(
        &self,
        session: &mut DbSession,
        entity: DmnDecisionRequirementsDiagramEntity,
    ) -> Result<(), PersistenceError> {
        let mut params = DbParams::new();
        params.push(entity.id.clone());
        params.push(entity.name.clone());
        params.push(entity.deployment_id.clone());
        params.push(entity.resource_name.clone());
        params.push(entity.data.clone());

        session.insert(
            entity,
            StatementId::InsertDmnDecisionRequirementsDiagram,
            params,
        )
    }

    pub fn delete(
        &self,
        session: &mut DbSession,
        entity: &DmnDecisionRequirementsDiagramEntity,
    ) -> Result<(), PersistenceError> {
        let mut params = DbParams::new();
        params.push(entity.id.clone());

        session.delete(
            entity,
            StatementId::DeleteDmnDecisionRequirementsDiagram,
            params,
        )
    }

    pub fn find_by_id(
        &self,
        session: &mut DbSession,
        id: &str,
    ) -> Result<Option<DmnDecisionRequirementsDiagramEntity>, PersistenceError> {
        let mut params = DbParams::new();
        params.push(id);

        let row = session.select_one(
            StatementId::SelectDmnDecisionRequirementsDiagramById,
            params,
        )?;
        match row {
            Some(row) => Ok(Some(DmnDecisionRequirementsDiagramEntity::from_row(&row)?)),
            None => Ok(None),
        }
    }

    pub fn find_by_deployment_id(
        &self,
        session: &mut DbSession,
        deployment_id: &str,
    ) -> Result<Vec<DmnDecisionRequirementsDiagramEntity>, PersistenceError> {
        let mut params = DbParams::new();
        params.push(deployment_id);

        let rows = session.select_list(
            StatementId::SelectDmnDecisionRequirementsDiagramsByDeploymentId,
            params,
        )?;
        rows.iter()
            .map(DmnDecisionRequirementsDiagramEntity::from_row)
            .collect()
    }
}

impl Default for DmnDecisionRequirementsDiagramDataManager {
    fn default() -> Self {
        Self::new()
    }
}
