use crate::db_session::DbSession;
use crate::entity::{Entity, EntityType, RevisionedEntity};
use crate::error::PersistenceError;
use crate::row::DbRow;
use crate::statement::StatementId;
use crate::value::DbParams;
use std::any::Any;

#[derive(Debug, Clone)]
pub struct PropertyEntity {
    pub name: String,
    pub value: String,
    pub revision: i32,
}

impl PropertyEntity {
    pub fn new(name: String, value: String) -> Self {
        Self {
            name,
            value,
            revision: 1,
        }
    }

    pub fn from_row(row: &DbRow) -> Result<Self, PersistenceError> {
        Ok(Self {
            name: row.get_text("NAME_").ok_or_else(|| {
                PersistenceError::Deserialization("Missing NAME_ in PropertyEntity".to_string())
            })?,
            value: row.get_text("VALUE_").ok_or_else(|| {
                PersistenceError::Deserialization("Missing VALUE_ in PropertyEntity".to_string())
            })?,
            revision: row.get_integer("REV_").unwrap_or(1) as i32,
        })
    }
}

impl Entity for PropertyEntity {
    fn id(&self) -> &str {
        &self.name
    }

    fn set_id(&mut self, id: String) {
        self.name = id;
    }

    fn entity_type(&self) -> EntityType {
        EntityType::Property
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn clone_box(&self) -> Box<dyn Entity> {
        Box::new(self.clone())
    }
}

impl RevisionedEntity for PropertyEntity {
    fn revision(&self) -> i32 {
        self.revision
    }

    fn set_revision(&mut self, revision: i32) {
        self.revision = revision;
    }
}

pub struct PropertyDataManager;

impl PropertyDataManager {
    pub fn new() -> Self {
        Self
    }

    pub fn insert(
        &self,
        session: &mut DbSession,
        entity: PropertyEntity,
    ) -> Result<(), PersistenceError> {
        let mut params = DbParams::new();
        params.push(entity.name.clone());
        params.push(entity.value.clone());
        params.push(entity.revision as i64);

        session.insert(entity, StatementId::InsertProperty, params)
    }

    pub fn update(
        &self,
        session: &mut DbSession,
        entity: PropertyEntity,
    ) -> Result<(), PersistenceError> {
        let mut params = DbParams::new();
        params.push(entity.value.clone());
        params.push(entity.name.clone());
        params.push(entity.revision as i64);

        session.update(entity, StatementId::UpdateProperty, params)
    }

    pub fn delete(
        &self,
        session: &mut DbSession,
        entity: &PropertyEntity,
    ) -> Result<(), PersistenceError> {
        let mut params = DbParams::new();
        params.push(entity.name.clone());

        session.delete(entity, StatementId::DeleteProperty, params)
    }

    pub fn find_by_id(
        &self,
        session: &mut DbSession,
        name: &str,
    ) -> Result<Option<PropertyEntity>, PersistenceError> {
        let mut params = DbParams::new();
        params.push(name);

        let row = session.select_one(StatementId::SelectPropertyByName, params)?;
        match row {
            Some(row) => Ok(Some(PropertyEntity::from_row(&row)?)),
            None => Ok(None),
        }
    }

    pub fn find_all(
        &self,
        session: &mut DbSession,
    ) -> Result<Vec<PropertyEntity>, PersistenceError> {
        let params = DbParams::new();
        let rows = session.select_list(StatementId::SelectAllProperties, params)?;
        rows.iter().map(PropertyEntity::from_row).collect()
    }
}

impl Default for PropertyDataManager {
    fn default() -> Self {
        Self::new()
    }
}
