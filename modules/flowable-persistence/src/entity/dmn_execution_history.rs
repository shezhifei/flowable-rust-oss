use crate::db_session::DbSession;
use crate::entity::{Entity, EntityType};
use crate::error::PersistenceError;
use crate::row::DbRow;
use crate::statement::StatementId;
use crate::value::DbParams;
use std::any::Any;

#[derive(Debug, Clone)]
pub struct DmnExecutionHistoryEntity {
    pub execution_id: String,
    pub decision_key: String,
    pub decision_definition_id: String,
    pub deployment_id: String,
    pub business_key: Option<String>,
    pub tenant_id: Option<String>,
    /// `INSTANCE_ID_` — Java `PersistHistoricDecisionExecutionCmd.java:56`.
    pub instance_id: Option<String>,
    /// `SCOPE_EXECUTION_ID_` — Java's `EXECUTION_ID_`
    /// (`PersistHistoricDecisionExecutionCmd.java:57`). Renamed here because
    /// `EXECUTION_ID_` is already this table's primary key in the Rust schema.
    pub scope_execution_id: Option<String>,
    /// `ACTIVITY_ID_` — Java `PersistHistoricDecisionExecutionCmd.java:58`.
    pub activity_id: Option<String>,
    /// `SCOPE_TYPE_` — Java `PersistHistoricDecisionExecutionCmd.java:59`.
    pub scope_type: Option<String>,
    pub executed_at: String,
    pub data: String,
}

impl DmnExecutionHistoryEntity {
    pub fn new(
        execution_id: String,
        decision_key: String,
        decision_definition_id: String,
        deployment_id: String,
        executed_at: String,
        data: String,
    ) -> Self {
        Self {
            execution_id,
            decision_key,
            decision_definition_id,
            deployment_id,
            business_key: None,
            tenant_id: None,
            instance_id: None,
            scope_execution_id: None,
            activity_id: None,
            scope_type: None,
            executed_at,
            data,
        }
    }

    pub fn set_business_key(&mut self, business_key: Option<String>) {
        self.business_key = business_key;
    }

    pub fn set_tenant_id(&mut self, tenant_id: Option<String>) {
        self.tenant_id = tenant_id;
    }

    /// Java `PersistHistoricDecisionExecutionCmd.java:56-59`.
    pub fn set_scope_correlation(
        &mut self,
        instance_id: Option<String>,
        scope_execution_id: Option<String>,
        activity_id: Option<String>,
        scope_type: Option<String>,
    ) {
        self.instance_id = instance_id;
        self.scope_execution_id = scope_execution_id;
        self.activity_id = activity_id;
        self.scope_type = scope_type;
    }

    pub fn from_row(row: &DbRow) -> Result<Self, PersistenceError> {
        Ok(Self {
            execution_id: row.get_text("EXECUTION_ID_").ok_or_else(|| {
                PersistenceError::Deserialization(
                    "Missing EXECUTION_ID_ in DmnExecutionHistoryEntity".to_string(),
                )
            })?,
            decision_key: row.get_text("DECISION_KEY_").ok_or_else(|| {
                PersistenceError::Deserialization(
                    "Missing DECISION_KEY_ in DmnExecutionHistoryEntity".to_string(),
                )
            })?,
            decision_definition_id: row.get_text("DECISION_DEFINITION_ID_").ok_or_else(|| {
                PersistenceError::Deserialization(
                    "Missing DECISION_DEFINITION_ID_ in DmnExecutionHistoryEntity".to_string(),
                )
            })?,
            deployment_id: row.get_text("DEPLOYMENT_ID_").ok_or_else(|| {
                PersistenceError::Deserialization(
                    "Missing DEPLOYMENT_ID_ in DmnExecutionHistoryEntity".to_string(),
                )
            })?,
            business_key: row.get_text("BUSINESS_KEY_"),
            tenant_id: row.get_text("TENANT_ID_"),
            instance_id: row.get_text("INSTANCE_ID_"),
            scope_execution_id: row.get_text("SCOPE_EXECUTION_ID_"),
            activity_id: row.get_text("ACTIVITY_ID_"),
            scope_type: row.get_text("SCOPE_TYPE_"),
            executed_at: row.get_text("EXECUTED_AT_").ok_or_else(|| {
                PersistenceError::Deserialization(
                    "Missing EXECUTED_AT_ in DmnExecutionHistoryEntity".to_string(),
                )
            })?,
            data: row.get_text("DATA_").ok_or_else(|| {
                PersistenceError::Deserialization(
                    "Missing DATA_ in DmnExecutionHistoryEntity".to_string(),
                )
            })?,
        })
    }
}

impl Entity for DmnExecutionHistoryEntity {
    fn id(&self) -> &str {
        &self.execution_id
    }

    fn set_id(&mut self, id: String) {
        self.execution_id = id;
    }

    fn entity_type(&self) -> EntityType {
        EntityType::DmnExecutionHistory
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn clone_box(&self) -> Box<dyn Entity> {
        Box::new(self.clone())
    }
}

pub struct DmnExecutionHistoryDataManager;

impl DmnExecutionHistoryDataManager {
    pub fn new() -> Self {
        Self
    }

    pub fn insert(
        &self,
        session: &mut DbSession,
        entity: DmnExecutionHistoryEntity,
    ) -> Result<(), PersistenceError> {
        let mut params = DbParams::new();
        params.push(entity.execution_id.clone());
        params.push(entity.decision_key.clone());
        params.push(entity.decision_definition_id.clone());
        params.push(entity.deployment_id.clone());
        params.push(entity.business_key.clone());
        params.push(entity.tenant_id.clone());
        params.push(entity.instance_id.clone());
        params.push(entity.scope_execution_id.clone());
        params.push(entity.activity_id.clone());
        params.push(entity.scope_type.clone());
        params.push(entity.executed_at.clone());
        params.push(entity.data.clone());

        session.insert(entity, StatementId::InsertDmnExecutionHistory, params)
    }

    pub fn delete(
        &self,
        session: &mut DbSession,
        entity: &DmnExecutionHistoryEntity,
    ) -> Result<(), PersistenceError> {
        let mut params = DbParams::new();
        params.push(entity.execution_id.clone());

        session.delete(entity, StatementId::DeleteDmnExecutionHistory, params)
    }

    pub fn find_by_id(
        &self,
        session: &mut DbSession,
        execution_id: &str,
    ) -> Result<Option<DmnExecutionHistoryEntity>, PersistenceError> {
        let mut params = DbParams::new();
        params.push(execution_id);

        let row = session.select_one(StatementId::SelectDmnExecutionHistoryById, params)?;
        match row {
            Some(row) => Ok(Some(DmnExecutionHistoryEntity::from_row(&row)?)),
            None => Ok(None),
        }
    }

    pub fn find_by_deployment_id(
        &self,
        session: &mut DbSession,
        deployment_id: &str,
    ) -> Result<Vec<DmnExecutionHistoryEntity>, PersistenceError> {
        let mut params = DbParams::new();
        params.push(deployment_id);

        let rows = session.select_list(
            StatementId::SelectDmnExecutionHistoriesByDeploymentId,
            params,
        )?;
        rows.iter()
            .map(DmnExecutionHistoryEntity::from_row)
            .collect()
    }
}

impl Default for DmnExecutionHistoryDataManager {
    fn default() -> Self {
        Self::new()
    }
}
