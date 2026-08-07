use crate::entity::{Entity, EntityType};
use std::collections::HashMap;

#[derive(Debug, Default)]
pub struct EntityCache {
    entities: HashMap<(EntityType, String), Box<dyn Entity>>,
    inserted: HashMap<(EntityType, String), bool>,
    updated: HashMap<(EntityType, String), bool>,
    deleted: HashMap<(EntityType, String), bool>,
}

impl EntityCache {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get<T: Entity + Clone>(&self, entity_type: EntityType, id: &str) -> Option<T> {
        let key = (entity_type, id.to_string());
        self.entities
            .get(&key)
            .and_then(|e| e.as_any().downcast_ref::<T>().cloned())
    }

    pub fn put(&mut self, entity: Box<dyn Entity>) {
        let key = (entity.entity_type(), entity.id().to_string());
        self.entities.insert(key, entity);
    }

    pub fn mark_inserted(&mut self, entity_type: EntityType, id: &str) {
        self.inserted.insert((entity_type, id.to_string()), true);
    }

    pub fn mark_updated(&mut self, entity_type: EntityType, id: &str) {
        self.updated.insert((entity_type, id.to_string()), true);
    }

    pub fn mark_deleted(&mut self, entity_type: EntityType, id: &str) {
        self.deleted.insert((entity_type, id.to_string()), true);
    }

    pub fn is_inserted(&self, entity_type: EntityType, id: &str) -> bool {
        self.inserted
            .get(&(entity_type, id.to_string()))
            .copied()
            .unwrap_or(false)
    }

    pub fn is_updated(&self, entity_type: EntityType, id: &str) -> bool {
        self.updated
            .get(&(entity_type, id.to_string()))
            .copied()
            .unwrap_or(false)
    }

    pub fn is_deleted(&self, entity_type: EntityType, id: &str) -> bool {
        self.deleted
            .get(&(entity_type, id.to_string()))
            .copied()
            .unwrap_or(false)
    }

    pub fn get_updated_entities(&self) -> Vec<&dyn Entity> {
        self.updated
            .keys()
            .filter_map(|key| self.entities.get(key).map(|e| e.as_ref()))
            .collect()
    }

    pub fn clear(&mut self) {
        self.entities.clear();
        self.inserted.clear();
        self.updated.clear();
        self.deleted.clear();
    }
}
