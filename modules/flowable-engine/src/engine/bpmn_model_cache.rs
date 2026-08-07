use flowable_bpmn_converter::BpmnXMLConverter;
use flowable_bpmn_model::model::BpmnModel;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

pub struct BpmnModelCache {
    cache: Arc<RwLock<HashMap<String, Arc<BpmnModel>>>>,
}

impl Default for BpmnModelCache {
    fn default() -> Self {
        Self::new()
    }
}

impl BpmnModelCache {
    pub fn new() -> Self {
        Self {
            cache: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub fn get_or_parse(
        &self,
        deployment_id: &str,
        resource_name: &str,
        bytes: &[u8],
    ) -> Option<Arc<BpmnModel>> {
        let key = format!("{}#{}", deployment_id, resource_name);
        {
            let read = self.cache.read().unwrap_or_else(|e| e.into_inner());
            if let Some(model) = read.get(&key) {
                return Some(Arc::clone(model));
            }
        }
        let xml = std::str::from_utf8(bytes).ok()?;
        let model = Arc::new(
            BpmnXMLConverter::new()
                .try_convert_to_bpmn_model(xml)
                .ok()?,
        );
        self.cache
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(key, Arc::clone(&model));
        Some(model)
    }

    pub fn invalidate(&self, deployment_id: &str) {
        let prefix = format!("{}#", deployment_id);
        self.cache
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .retain(|k, _| !k.starts_with(&prefix));
    }

    pub fn clear(&self) {
        self.cache
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
    }

    pub fn len(&self) -> usize {
        self.cache.read().unwrap_or_else(|e| e.into_inner()).len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn contains_key(&self, deployment_id: &str, resource_name: &str) -> bool {
        let key = format!("{}#{}", deployment_id, resource_name);
        self.cache
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .contains_key(&key)
    }

    pub fn get_by_pd_id(&self, process_definition_id: &str) -> Option<Arc<BpmnModel>> {
        self.cache
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(process_definition_id)
            .map(Arc::clone)
    }

    pub fn insert_pd(&self, process_definition_id: String, model: Arc<BpmnModel>) {
        self.cache
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(process_definition_id, model);
    }
}
