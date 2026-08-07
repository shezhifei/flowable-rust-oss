use crate::db_session::DbSession;
use crate::entity::{Entity, EntityType, RevisionedEntity};
use crate::error::PersistenceError;
use crate::row::DbRow;
use crate::statement::StatementId;
use crate::value::DbParams;
use std::any::Any;

#[derive(Debug, Clone)]
pub struct ByteArrayEntity {
    pub id: String,
    pub revision: i32,
    pub name: Option<String>,
    pub deployment_id: Option<String>,
    pub bytes: Option<Vec<u8>>,
    pub generated: bool,
}

impl ByteArrayEntity {
    pub fn new(id: String) -> Self {
        Self {
            id,
            revision: 1,
            name: None,
            deployment_id: None,
            bytes: None,
            generated: false,
        }
    }

    pub fn from_row(row: &DbRow) -> Result<Self, PersistenceError> {
        Ok(Self {
            id: row.get_text("ID_").ok_or_else(|| {
                PersistenceError::Deserialization("Missing ID_ in ByteArrayEntity".to_string())
            })?,
            revision: row.get_integer("REV_").unwrap_or(1) as i32,
            name: row.get_text("NAME_"),
            deployment_id: row.get_text("DEPLOYMENT_ID_"),
            bytes: row.get_blob("BYTES_"),
            generated: row
                .get_integer("GENERATED_")
                .map(|v| v != 0)
                .unwrap_or(false),
        })
    }
}

impl Entity for ByteArrayEntity {
    fn id(&self) -> &str {
        &self.id
    }

    fn set_id(&mut self, id: String) {
        self.id = id;
    }

    fn entity_type(&self) -> EntityType {
        EntityType::ByteArray
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn clone_box(&self) -> Box<dyn Entity> {
        Box::new(self.clone())
    }
}

impl RevisionedEntity for ByteArrayEntity {
    fn revision(&self) -> i32 {
        self.revision
    }

    fn set_revision(&mut self, revision: i32) {
        self.revision = revision;
    }
}

pub struct ByteArrayDataManager;

impl ByteArrayDataManager {
    pub fn new() -> Self {
        Self
    }

    pub fn insert(
        &self,
        session: &mut DbSession,
        entity: ByteArrayEntity,
    ) -> Result<(), PersistenceError> {
        let mut params = DbParams::new();
        params.push(entity.id.clone());
        params.push(entity.revision as i64);
        params.push(entity.name.clone());
        params.push(entity.deployment_id.clone());
        params.push(entity.bytes.clone());
        params.push(if entity.generated { 1i64 } else { 0i64 });

        session.insert(entity, StatementId::InsertByteArray, params)
    }

    pub fn update(
        &self,
        session: &mut DbSession,
        entity: ByteArrayEntity,
    ) -> Result<(), PersistenceError> {
        let mut params = DbParams::new();
        params.push(entity.revision as i64);
        params.push(entity.name.clone());
        params.push(entity.deployment_id.clone());
        params.push(entity.bytes.clone());
        params.push(if entity.generated { 1i64 } else { 0i64 });
        params.push(entity.id.clone());
        params.push(entity.revision as i64);

        session.update(entity, StatementId::UpdateByteArray, params)
    }

    pub fn delete(
        &self,
        session: &mut DbSession,
        entity: &ByteArrayEntity,
    ) -> Result<(), PersistenceError> {
        let mut params = DbParams::new();
        params.push(entity.id.clone());

        session.delete(entity, StatementId::DeleteByteArray, params)
    }

    pub fn find_by_id(
        &self,
        session: &mut DbSession,
        id: &str,
    ) -> Result<Option<ByteArrayEntity>, PersistenceError> {
        let mut params = DbParams::new();
        params.push(id);

        let row = session.select_one(StatementId::SelectByteArrayById, params)?;
        match row {
            Some(row) => Ok(Some(ByteArrayEntity::from_row(&row)?)),
            None => Ok(None),
        }
    }
}

impl Default for ByteArrayDataManager {
    fn default() -> Self {
        Self::new()
    }
}
