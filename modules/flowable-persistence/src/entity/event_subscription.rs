use crate::db_session::DbSession;
use crate::entity::{Entity, EntityType, RevisionedEntity};
use crate::error::PersistenceError;
use crate::row::DbRow;
use crate::statement::StatementId;
use crate::value::DbParams;
use std::any::Any;

#[derive(Debug, Clone)]
pub struct EventSubscriptionEntity {
    pub id: String,
    pub revision: i32,
    pub event_type: Option<String>,
    pub event_name: Option<String>,
    pub execution_id: Option<String>,
    pub process_instance_id: Option<String>,
    pub activity_id: Option<String>,
    pub configuration: Option<String>,
    pub created: Option<i64>,
    pub process_definition_id: Option<String>,
    pub tenant_id: Option<String>,
    pub lock_owner: Option<String>,
    pub lock_time: Option<i64>,
}

impl EventSubscriptionEntity {
    pub fn new(id: String) -> Self {
        Self {
            id,
            revision: 1,
            event_type: None,
            event_name: None,
            execution_id: None,
            process_instance_id: None,
            activity_id: None,
            configuration: None,
            created: None,
            process_definition_id: None,
            tenant_id: None,
            lock_owner: None,
            lock_time: None,
        }
    }

    pub fn from_row(row: &DbRow) -> Result<Self, PersistenceError> {
        Ok(Self {
            id: row.get_text("ID_").ok_or_else(|| {
                PersistenceError::Deserialization(
                    "Missing ID_ in EventSubscriptionEntity".to_string(),
                )
            })?,
            revision: row.get_integer("REV_").unwrap_or(1) as i32,
            event_type: row.get_text("EVENT_TYPE_"),
            event_name: row.get_text("EVENT_NAME_"),
            execution_id: row.get_text("EXECUTION_ID_"),
            process_instance_id: row.get_text("PROC_INST_ID_"),
            activity_id: row.get_text("ACTIVITY_ID_"),
            configuration: row.get_text("CONFIGURATION_"),
            created: row.get_integer("CREATED_"),
            process_definition_id: row.get_text("PROC_DEF_ID_"),
            tenant_id: row.get_text("TENANT_ID_"),
            lock_owner: row.get_text("LOCK_OWNER_"),
            lock_time: row.get_integer("LOCK_TIME_"),
        })
    }
}

impl Entity for EventSubscriptionEntity {
    fn id(&self) -> &str {
        &self.id
    }

    fn set_id(&mut self, id: String) {
        self.id = id;
    }

    fn entity_type(&self) -> EntityType {
        EntityType::EventSubscription
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn clone_box(&self) -> Box<dyn Entity> {
        Box::new(self.clone())
    }
}

impl RevisionedEntity for EventSubscriptionEntity {
    fn revision(&self) -> i32 {
        self.revision
    }

    fn set_revision(&mut self, revision: i32) {
        self.revision = revision;
    }
}

pub struct EventSubscriptionDataManager;

impl EventSubscriptionDataManager {
    pub fn new() -> Self {
        Self
    }

    pub fn insert(
        &self,
        session: &mut DbSession,
        entity: EventSubscriptionEntity,
    ) -> Result<(), PersistenceError> {
        let mut params = DbParams::new();
        params.push(entity.id.clone());
        params.push(entity.revision as i64);
        params.push(entity.event_type.clone());
        params.push(entity.event_name.clone());
        params.push(entity.execution_id.clone());
        params.push(entity.process_instance_id.clone());
        params.push(entity.activity_id.clone());
        params.push(entity.configuration.clone());
        params.push(entity.created);
        params.push(entity.process_definition_id.clone());
        params.push(entity.tenant_id.clone());
        params.push(entity.lock_owner.clone());
        params.push(entity.lock_time);

        session.insert(entity, StatementId::InsertEventSubscription, params)
    }

    pub fn update(
        &self,
        session: &mut DbSession,
        entity: EventSubscriptionEntity,
    ) -> Result<(), PersistenceError> {
        let mut params = DbParams::new();
        params.push(entity.revision as i64);
        params.push(entity.event_type.clone());
        params.push(entity.event_name.clone());
        params.push(entity.execution_id.clone());
        params.push(entity.process_instance_id.clone());
        params.push(entity.activity_id.clone());
        params.push(entity.configuration.clone());
        params.push(entity.created);
        params.push(entity.process_definition_id.clone());
        params.push(entity.tenant_id.clone());
        params.push(entity.lock_owner.clone());
        params.push(entity.lock_time);
        params.push(entity.id.clone());
        params.push(entity.revision as i64);

        session.update(entity, StatementId::UpdateEventSubscription, params)
    }

    pub fn delete(
        &self,
        session: &mut DbSession,
        entity: &EventSubscriptionEntity,
    ) -> Result<(), PersistenceError> {
        let mut params = DbParams::new();
        params.push(entity.id.clone());

        session.delete(entity, StatementId::DeleteEventSubscription, params)
    }

    pub fn find_by_id(
        &self,
        session: &mut DbSession,
        id: &str,
    ) -> Result<Option<EventSubscriptionEntity>, PersistenceError> {
        let mut params = DbParams::new();
        params.push(id);

        let row = session.select_one(StatementId::SelectEventSubscriptionById, params)?;
        match row {
            Some(row) => Ok(Some(EventSubscriptionEntity::from_row(&row)?)),
            None => Ok(None),
        }
    }
}

impl Default for EventSubscriptionDataManager {
    fn default() -> Self {
        Self::new()
    }
}
