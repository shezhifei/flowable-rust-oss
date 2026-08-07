use crate::db_session::DbSession;
use crate::entity::{Entity, EntityType, RevisionedEntity};
use crate::error::PersistenceError;
use crate::row::DbRow;
use crate::statement::StatementId;
use crate::value::DbParams;
use std::any::Any;

#[derive(Debug, Clone)]
pub struct CmmnCaseDefinitionEntity {
    pub id: String,
    pub case_key: String,
    pub deployment_id: String,
    pub tenant_id: Option<String>,
    pub category: Option<String>,
    pub version: i32,
    pub resource_name: String,
    pub diagram_resource_name: Option<String>,
    pub data: String,
}

impl CmmnCaseDefinitionEntity {
    pub fn new(
        id: String,
        case_key: String,
        deployment_id: String,
        version: i32,
        resource_name: String,
        data: String,
    ) -> Self {
        Self {
            id,
            case_key,
            deployment_id,
            tenant_id: None,
            category: None,
            version,
            resource_name,
            diagram_resource_name: None,
            data,
        }
    }

    pub fn set_tenant_id(&mut self, tenant_id: Option<String>) {
        self.tenant_id = tenant_id;
    }

    pub fn set_metadata(
        &mut self,
        category: Option<String>,
        diagram_resource_name: Option<String>,
    ) {
        self.category = category;
        self.diagram_resource_name = diagram_resource_name;
    }

    pub fn from_row(row: &DbRow) -> Result<Self, PersistenceError> {
        Ok(Self {
            id: row.get_text("ID_").ok_or_else(|| {
                PersistenceError::Deserialization(
                    "Missing ID_ in CmmnCaseDefinitionEntity".to_string(),
                )
            })?,
            case_key: row.get_text("CASE_KEY_").ok_or_else(|| {
                PersistenceError::Deserialization(
                    "Missing CASE_KEY_ in CmmnCaseDefinitionEntity".to_string(),
                )
            })?,
            deployment_id: row.get_text("DEPLOYMENT_ID_").ok_or_else(|| {
                PersistenceError::Deserialization(
                    "Missing DEPLOYMENT_ID_ in CmmnCaseDefinitionEntity".to_string(),
                )
            })?,
            tenant_id: row.get_text("TENANT_ID_"),
            category: row.get_text("CATEGORY_"),
            version: row.get_integer("VERSION_").unwrap_or(1) as i32,
            resource_name: row.get_text("RESOURCE_NAME_").ok_or_else(|| {
                PersistenceError::Deserialization(
                    "Missing RESOURCE_NAME_ in CmmnCaseDefinitionEntity".to_string(),
                )
            })?,
            diagram_resource_name: row.get_text("DIAGRAM_RESOURCE_NAME_"),
            data: row.get_text("DATA_").ok_or_else(|| {
                PersistenceError::Deserialization(
                    "Missing DATA_ in CmmnCaseDefinitionEntity".to_string(),
                )
            })?,
        })
    }
}

impl Entity for CmmnCaseDefinitionEntity {
    fn id(&self) -> &str {
        &self.id
    }

    fn set_id(&mut self, id: String) {
        self.id = id;
    }

    fn entity_type(&self) -> EntityType {
        EntityType::CmmnCaseDefinition
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn clone_box(&self) -> Box<dyn Entity> {
        Box::new(self.clone())
    }
}

impl RevisionedEntity for CmmnCaseDefinitionEntity {
    fn revision(&self) -> i32 {
        self.version
    }

    fn set_revision(&mut self, revision: i32) {
        self.version = revision;
    }
}

pub struct CmmnCaseDefinitionDataManager;

impl CmmnCaseDefinitionDataManager {
    pub fn new() -> Self {
        Self
    }

    pub fn insert(
        &self,
        session: &mut DbSession,
        entity: CmmnCaseDefinitionEntity,
    ) -> Result<(), PersistenceError> {
        let mut params = DbParams::new();
        params.push(entity.id.clone());
        params.push(entity.case_key.clone());
        params.push(entity.deployment_id.clone());
        params.push(entity.tenant_id.clone());
        params.push(entity.category.clone());
        params.push(entity.version as i64);
        params.push(entity.resource_name.clone());
        params.push(entity.diagram_resource_name.clone());
        params.push(entity.data.clone());

        session.insert(entity, StatementId::InsertCmmnCaseDefinition, params)
    }

    pub fn delete(
        &self,
        session: &mut DbSession,
        entity: &CmmnCaseDefinitionEntity,
    ) -> Result<(), PersistenceError> {
        let mut params = DbParams::new();
        params.push(entity.id.clone());

        session.delete(entity, StatementId::DeleteCmmnCaseDefinition, params)
    }

    pub fn find_by_id(
        &self,
        session: &mut DbSession,
        id: &str,
    ) -> Result<Option<CmmnCaseDefinitionEntity>, PersistenceError> {
        let mut params = DbParams::new();
        params.push(id);

        let row = session.select_one(StatementId::SelectCmmnCaseDefinitionById, params)?;
        match row {
            Some(row) => Ok(Some(CmmnCaseDefinitionEntity::from_row(&row)?)),
            None => Ok(None),
        }
    }

    pub fn update_category(
        &self,
        session: &mut DbSession,
        id: &str,
        category: Option<String>,
    ) -> Result<u64, PersistenceError> {
        let mut params = DbParams::new();
        params.push(category);
        params.push(id);
        Ok(session
            .execute(StatementId::UpdateCmmnCaseDefinitionCategory, params)?
            .rows_affected)
    }

    pub fn find_all(
        &self,
        session: &mut DbSession,
    ) -> Result<Vec<CmmnCaseDefinitionEntity>, PersistenceError> {
        let rows =
            session.select_list(StatementId::SelectAllCmmnCaseDefinitions, DbParams::new())?;
        rows.iter()
            .map(CmmnCaseDefinitionEntity::from_row)
            .collect()
    }

    pub fn find_by_deployment_id(
        &self,
        session: &mut DbSession,
        deployment_id: &str,
    ) -> Result<Vec<CmmnCaseDefinitionEntity>, PersistenceError> {
        let mut params = DbParams::new();
        params.push(deployment_id);

        let rows =
            session.select_list(StatementId::SelectCmmnCaseDefinitionsByDeploymentId, params)?;
        rows.iter()
            .map(CmmnCaseDefinitionEntity::from_row)
            .collect()
    }

    pub fn find_by_key(
        &self,
        session: &mut DbSession,
        case_key: &str,
    ) -> Result<Vec<CmmnCaseDefinitionEntity>, PersistenceError> {
        let mut params = DbParams::new();
        params.push(case_key);

        let rows = session.select_list(StatementId::SelectCmmnCaseDefinitionByKey, params)?;
        rows.iter()
            .map(CmmnCaseDefinitionEntity::from_row)
            .collect()
    }

    pub fn find_by_key_and_version(
        &self,
        session: &mut DbSession,
        case_key: &str,
        version: i32,
    ) -> Result<Vec<CmmnCaseDefinitionEntity>, PersistenceError> {
        let mut params = DbParams::new();
        params.push(case_key);
        params.push(version as i64);

        let rows =
            session.select_list(StatementId::SelectCmmnCaseDefinitionByKeyAndVersion, params)?;
        rows.iter()
            .map(CmmnCaseDefinitionEntity::from_row)
            .collect()
    }
}

impl Default for CmmnCaseDefinitionDataManager {
    fn default() -> Self {
        Self::new()
    }
}
