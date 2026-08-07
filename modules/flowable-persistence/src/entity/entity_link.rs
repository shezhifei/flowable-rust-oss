use crate::db_session::DbSession;
use crate::entity::{Entity, EntityType, RevisionedEntity};
use crate::error::PersistenceError;
use crate::row::DbRow;
use crate::statement::StatementId;
use crate::value::DbParams;
use std::any::Any;

#[derive(Debug, Clone)]
pub struct EntityLinkEntity {
    pub id: String,
    pub revision: i32,
    pub create_time: Option<i64>,
    pub link_type: Option<String>,
    pub scope_id: Option<String>,
    pub scope_type: Option<String>,
    pub scope_definition_id: Option<String>,
    pub ref_scope_id: Option<String>,
    pub ref_scope_type: Option<String>,
    pub ref_scope_definition_id: Option<String>,
    pub hierarchy_type: Option<String>,
}

impl EntityLinkEntity {
    pub fn new(id: String) -> Self {
        Self {
            id,
            revision: 1,
            create_time: None,
            link_type: None,
            scope_id: None,
            scope_type: None,
            scope_definition_id: None,
            ref_scope_id: None,
            ref_scope_type: None,
            ref_scope_definition_id: None,
            hierarchy_type: None,
        }
    }

    pub fn from_row(row: &DbRow) -> Result<Self, PersistenceError> {
        Ok(Self {
            id: row.get_text("ID_").ok_or_else(|| {
                PersistenceError::Deserialization("Missing ID_ in EntityLinkEntity".to_string())
            })?,
            revision: row.get_integer("REV_").unwrap_or(1) as i32,
            create_time: row.get_integer("CREATE_TIME_"),
            link_type: row.get_text("LINK_TYPE_"),
            scope_id: row.get_text("SCOPE_ID_"),
            scope_type: row.get_text("SCOPE_TYPE_"),
            scope_definition_id: row.get_text("SCOPE_DEFINITION_ID_"),
            ref_scope_id: row.get_text("REF_SCOPE_ID_"),
            ref_scope_type: row.get_text("REF_SCOPE_TYPE_"),
            ref_scope_definition_id: row.get_text("REF_SCOPE_DEFINITION_ID_"),
            hierarchy_type: row.get_text("HIERARCHY_TYPE_"),
        })
    }
}

impl Entity for EntityLinkEntity {
    fn id(&self) -> &str {
        &self.id
    }

    fn set_id(&mut self, id: String) {
        self.id = id;
    }

    fn entity_type(&self) -> EntityType {
        EntityType::EntityLink
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn clone_box(&self) -> Box<dyn Entity> {
        Box::new(self.clone())
    }
}

impl RevisionedEntity for EntityLinkEntity {
    fn revision(&self) -> i32 {
        self.revision
    }

    fn set_revision(&mut self, revision: i32) {
        self.revision = revision;
    }
}

pub struct EntityLinkDataManager;

impl EntityLinkDataManager {
    pub fn new() -> Self {
        Self
    }

    pub fn insert(
        &self,
        session: &mut DbSession,
        entity: EntityLinkEntity,
    ) -> Result<(), PersistenceError> {
        let mut params = DbParams::new();
        params.push(entity.id.clone());
        params.push(entity.revision as i64);
        params.push(entity.create_time);
        params.push(entity.link_type.clone());
        params.push(entity.scope_id.clone());
        params.push(entity.scope_type.clone());
        params.push(entity.scope_definition_id.clone());
        params.push(entity.ref_scope_id.clone());
        params.push(entity.ref_scope_type.clone());
        params.push(entity.ref_scope_definition_id.clone());
        params.push(entity.hierarchy_type.clone());

        session.insert(entity, StatementId::InsertEntityLink, params)
    }

    pub fn delete(
        &self,
        session: &mut DbSession,
        entity: &EntityLinkEntity,
    ) -> Result<(), PersistenceError> {
        let mut params = DbParams::new();
        params.push(entity.id.clone());

        session.delete(entity, StatementId::DeleteEntityLink, params)
    }

    pub fn find_by_scope_id_and_type(
        &self,
        session: &mut DbSession,
        scope_id: &str,
        scope_type: &str,
    ) -> Result<Vec<EntityLinkEntity>, PersistenceError> {
        let mut params = DbParams::new();
        params.push(scope_id);
        params.push(scope_type);

        let rows = session.select_list(StatementId::SelectEntityLinksByScopeIdAndType, params)?;
        rows.iter().map(EntityLinkEntity::from_row).collect()
    }
}

impl Default for EntityLinkDataManager {
    fn default() -> Self {
        Self::new()
    }
}
