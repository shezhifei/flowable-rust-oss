use crate::db_session::DbSession;
use crate::entity::{Entity, EntityType};
use crate::error::PersistenceError;
use crate::row::DbRow;
use crate::statement::StatementId;
use crate::value::DbParams;
use std::any::Any;

#[derive(Debug, Clone)]
pub struct CmmnIdentityLinkEntity {
    pub id: String,
    pub scope_type: String,
    pub scope_id: String,
    pub link_type: String,
    pub user_id: Option<String>,
    pub group_id: Option<String>,
    pub data: String,
}

impl CmmnIdentityLinkEntity {
    pub fn new(
        id: String,
        scope_type: String,
        scope_id: String,
        link_type: String,
        data: String,
    ) -> Self {
        Self {
            id,
            scope_type,
            scope_id,
            link_type,
            user_id: None,
            group_id: None,
            data,
        }
    }

    pub fn set_user_id(&mut self, user_id: Option<String>) {
        self.user_id = user_id;
    }

    pub fn set_group_id(&mut self, group_id: Option<String>) {
        self.group_id = group_id;
    }

    pub fn from_row(row: &DbRow) -> Result<Self, PersistenceError> {
        Ok(Self {
            id: row.get_text("ID_").ok_or_else(|| {
                PersistenceError::Deserialization(
                    "Missing ID_ in CmmnIdentityLinkEntity".to_string(),
                )
            })?,
            scope_type: row.get_text("SCOPE_TYPE_").ok_or_else(|| {
                PersistenceError::Deserialization(
                    "Missing SCOPE_TYPE_ in CmmnIdentityLinkEntity".to_string(),
                )
            })?,
            scope_id: row.get_text("SCOPE_ID_").ok_or_else(|| {
                PersistenceError::Deserialization(
                    "Missing SCOPE_ID_ in CmmnIdentityLinkEntity".to_string(),
                )
            })?,
            link_type: row.get_text("LINK_TYPE_").ok_or_else(|| {
                PersistenceError::Deserialization(
                    "Missing LINK_TYPE_ in CmmnIdentityLinkEntity".to_string(),
                )
            })?,
            user_id: row.get_text("USER_ID_"),
            group_id: row.get_text("GROUP_ID_"),
            data: row.get_text("DATA_").ok_or_else(|| {
                PersistenceError::Deserialization(
                    "Missing DATA_ in CmmnIdentityLinkEntity".to_string(),
                )
            })?,
        })
    }
}

impl Entity for CmmnIdentityLinkEntity {
    fn id(&self) -> &str {
        &self.id
    }

    fn set_id(&mut self, id: String) {
        self.id = id;
    }

    fn entity_type(&self) -> EntityType {
        EntityType::CmmnIdentityLink
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn clone_box(&self) -> Box<dyn Entity> {
        Box::new(self.clone())
    }
}

pub struct CmmnIdentityLinkDataManager;

impl CmmnIdentityLinkDataManager {
    pub fn new() -> Self {
        Self
    }

    pub fn insert(
        &self,
        session: &mut DbSession,
        entity: CmmnIdentityLinkEntity,
    ) -> Result<(), PersistenceError> {
        let mut params = DbParams::new();
        params.push(entity.id.clone());
        params.push(entity.scope_type.clone());
        params.push(entity.scope_id.clone());
        params.push(entity.link_type.clone());
        params.push(entity.user_id.clone());
        params.push(entity.group_id.clone());
        params.push(entity.data.clone());

        session.insert(entity, StatementId::InsertCmmnIdentityLink, params)
    }

    pub fn delete(
        &self,
        session: &mut DbSession,
        entity: &CmmnIdentityLinkEntity,
    ) -> Result<(), PersistenceError> {
        let mut params = DbParams::new();
        params.push(entity.id.clone());

        session.delete(entity, StatementId::DeleteCmmnIdentityLink, params)
    }

    pub fn find_by_id(
        &self,
        session: &mut DbSession,
        id: &str,
    ) -> Result<Option<CmmnIdentityLinkEntity>, PersistenceError> {
        let mut params = DbParams::new();
        params.push(id);

        let row = session.select_one(StatementId::SelectCmmnIdentityLinkById, params)?;
        match row {
            Some(row) => Ok(Some(CmmnIdentityLinkEntity::from_row(&row)?)),
            None => Ok(None),
        }
    }

    pub fn find_by_scope(
        &self,
        session: &mut DbSession,
        scope_type: &str,
        scope_id: &str,
    ) -> Result<Vec<CmmnIdentityLinkEntity>, PersistenceError> {
        let mut params = DbParams::new();
        params.push(scope_type);
        params.push(scope_id);

        let rows = session.select_list(StatementId::SelectCmmnIdentityLinksByScope, params)?;
        rows.iter().map(CmmnIdentityLinkEntity::from_row).collect()
    }
}

impl Default for CmmnIdentityLinkDataManager {
    fn default() -> Self {
        Self::new()
    }
}
