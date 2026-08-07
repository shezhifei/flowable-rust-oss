use crate::error::CmmnError;
use crate::models::CmmnIdentityLink;
use crate::store::CmmnStore;
use flowable_persistence::entity::cmmn_identity_link::{
    CmmnIdentityLinkDataManager, CmmnIdentityLinkEntity,
};

#[derive(Clone)]
pub struct CmmnIdentityLinkService {
    store: CmmnStore,
}

impl CmmnIdentityLinkService {
    pub(crate) fn new(store: CmmnStore) -> Self {
        Self { store }
    }

    pub fn add_identity_link(&self, link: CmmnIdentityLink) -> Result<(), CmmnError> {
        let mut session = self.store.create_session()?;
        let manager = CmmnIdentityLinkDataManager::new();
        let mut entity = CmmnIdentityLinkEntity::new(
            link.id.clone(),
            link.scope_type.clone(),
            link.scope_id.clone(),
            link.link_type.clone(),
            serde_json::to_string(&link)?,
        );
        entity.set_user_id(link.user_id.clone());
        entity.set_group_id(link.group_id.clone());
        manager.insert(&mut session, entity)?;
        session.commit()?;
        Ok(())
    }

    pub fn delete_identity_link(&self, link_id: &str) -> Result<(), CmmnError> {
        let mut session = self.store.create_session()?;
        let manager = CmmnIdentityLinkDataManager::new();
        if let Some(entity) = manager.find_by_id(&mut session, link_id)? {
            manager.delete(&mut session, &entity)?;
            session.commit()?;
        }
        Ok(())
    }

    pub fn list_identity_links(
        &self,
        scope_type: &str,
        scope_id: &str,
    ) -> Result<Vec<CmmnIdentityLink>, CmmnError> {
        let mut session = self.store.create_session()?;
        let manager = CmmnIdentityLinkDataManager::new();
        let entities = manager.find_by_scope(&mut session, scope_type, scope_id)?;
        entities
            .into_iter()
            .map(|entity| serde_json::from_str(&entity.data).map_err(Into::into))
            .collect()
    }
}
