use crate::db_session::DbSession;
use crate::entity::{Entity, EntityType};
use crate::error::PersistenceError;
use crate::row::DbRow;
use crate::statement::StatementId;
use crate::value::DbParams;
use std::any::Any;

#[derive(Debug, Clone)]
pub struct CmmnEventSubscriptionEntity {
    pub id: String,
    pub event_type: String,
    pub event_name: String,
    pub activity_id: String,
    pub case_instance_id: String,
    pub case_definition_id: String,
    pub plan_item_instance_id: Option<String>,
    pub tenant_id: Option<String>,
    pub configuration: Option<String>,
    pub created_at: i64,
    pub data: String,
}

impl CmmnEventSubscriptionEntity {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: String,
        event_type: String,
        event_name: String,
        activity_id: String,
        case_instance_id: String,
        case_definition_id: String,
        created_at: i64,
        data: String,
    ) -> Self {
        Self {
            id,
            event_type,
            event_name,
            activity_id,
            case_instance_id,
            case_definition_id,
            plan_item_instance_id: None,
            tenant_id: None,
            configuration: None,
            created_at,
            data,
        }
    }

    pub fn set_plan_item_instance_id(&mut self, plan_item_instance_id: Option<String>) {
        self.plan_item_instance_id = plan_item_instance_id;
    }

    pub fn set_tenant_id(&mut self, tenant_id: Option<String>) {
        self.tenant_id = tenant_id;
    }

    pub fn set_configuration(&mut self, configuration: Option<String>) {
        self.configuration = configuration;
    }

    pub fn from_row(row: &DbRow) -> Result<Self, PersistenceError> {
        Ok(Self {
            id: row.get_text("ID_").ok_or_else(|| {
                PersistenceError::Deserialization(
                    "Missing ID_ in CmmnEventSubscriptionEntity".to_string(),
                )
            })?,
            event_type: row.get_text("EVENT_TYPE_").ok_or_else(|| {
                PersistenceError::Deserialization(
                    "Missing EVENT_TYPE_ in CmmnEventSubscriptionEntity".to_string(),
                )
            })?,
            event_name: row.get_text("EVENT_NAME_").ok_or_else(|| {
                PersistenceError::Deserialization(
                    "Missing EVENT_NAME_ in CmmnEventSubscriptionEntity".to_string(),
                )
            })?,
            activity_id: row.get_text("ACTIVITY_ID_").ok_or_else(|| {
                PersistenceError::Deserialization(
                    "Missing ACTIVITY_ID_ in CmmnEventSubscriptionEntity".to_string(),
                )
            })?,
            case_instance_id: row.get_text("CASE_INSTANCE_ID_").ok_or_else(|| {
                PersistenceError::Deserialization(
                    "Missing CASE_INSTANCE_ID_ in CmmnEventSubscriptionEntity".to_string(),
                )
            })?,
            case_definition_id: row.get_text("CASE_DEFINITION_ID_").ok_or_else(|| {
                PersistenceError::Deserialization(
                    "Missing CASE_DEFINITION_ID_ in CmmnEventSubscriptionEntity".to_string(),
                )
            })?,
            plan_item_instance_id: row.get_text("PLAN_ITEM_INSTANCE_ID_"),
            tenant_id: row.get_text("TENANT_ID_"),
            configuration: row.get_text("CONFIGURATION_"),
            created_at: row.get_integer("CREATED_AT_").unwrap_or(0),
            data: row.get_text("DATA_").ok_or_else(|| {
                PersistenceError::Deserialization(
                    "Missing DATA_ in CmmnEventSubscriptionEntity".to_string(),
                )
            })?,
        })
    }
}

impl Entity for CmmnEventSubscriptionEntity {
    fn id(&self) -> &str {
        &self.id
    }

    fn set_id(&mut self, id: String) {
        self.id = id;
    }

    fn entity_type(&self) -> EntityType {
        EntityType::CmmnEventSubscription
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn clone_box(&self) -> Box<dyn Entity> {
        Box::new(self.clone())
    }
}

pub struct CmmnEventSubscriptionDataManager;

impl CmmnEventSubscriptionDataManager {
    pub fn new() -> Self {
        Self
    }

    pub fn insert(
        &self,
        session: &mut DbSession,
        entity: CmmnEventSubscriptionEntity,
    ) -> Result<(), PersistenceError> {
        let mut params = DbParams::new();
        params.push(entity.id.clone());
        params.push(entity.event_type.clone());
        params.push(entity.event_name.clone());
        params.push(entity.activity_id.clone());
        params.push(entity.case_instance_id.clone());
        params.push(entity.case_definition_id.clone());
        params.push(entity.plan_item_instance_id.clone());
        params.push(entity.tenant_id.clone());
        params.push(entity.configuration.clone());
        params.push(entity.created_at);
        params.push(entity.data.clone());

        session.insert(entity, StatementId::InsertCmmnEventSubscription, params)
    }

    pub fn delete(
        &self,
        session: &mut DbSession,
        entity: &CmmnEventSubscriptionEntity,
    ) -> Result<(), PersistenceError> {
        let mut params = DbParams::new();
        params.push(entity.id.clone());

        session.delete(entity, StatementId::DeleteCmmnEventSubscription, params)
    }

    pub fn find_by_id(
        &self,
        session: &mut DbSession,
        id: &str,
    ) -> Result<Option<CmmnEventSubscriptionEntity>, PersistenceError> {
        let mut params = DbParams::new();
        params.push(id);

        let row = session.select_one(StatementId::SelectCmmnEventSubscriptionById, params)?;
        match row {
            Some(row) => Ok(Some(CmmnEventSubscriptionEntity::from_row(&row)?)),
            None => Ok(None),
        }
    }

    pub fn find_by_case_instance_id(
        &self,
        session: &mut DbSession,
        case_instance_id: &str,
    ) -> Result<Vec<CmmnEventSubscriptionEntity>, PersistenceError> {
        let mut params = DbParams::new();
        params.push(case_instance_id);

        let rows = session.select_list(
            StatementId::SelectCmmnEventSubscriptionsByCaseInstanceId,
            params,
        )?;
        rows.iter()
            .map(CmmnEventSubscriptionEntity::from_row)
            .collect()
    }
}

impl Default for CmmnEventSubscriptionDataManager {
    fn default() -> Self {
        Self::new()
    }
}
