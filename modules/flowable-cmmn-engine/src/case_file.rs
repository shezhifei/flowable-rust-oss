use crate::{CmmnCaseFileItem, CmmnCaseFileItemState, CmmnError};

/// Domain service for the case-file instance network. Items are persisted as a
/// flat collection for compatibility, while identity, parent links and paths
/// provide a real graph boundary to the runtime.
pub struct CaseFileGraph<'a> {
    items: &'a mut Vec<CmmnCaseFileItem>,
}

impl<'a> CaseFileGraph<'a> {
    pub fn new(items: &'a mut Vec<CmmnCaseFileItem>) -> Result<Self, CmmnError> {
        let graph = Self { items };
        graph.validate()?;
        Ok(graph)
    }

    pub fn insert(&mut self, mut item: CmmnCaseFileItem) -> Result<(), CmmnError> {
        if self.items.iter().any(|candidate| candidate.id == item.id) {
            return Err(CmmnError::conflict(format!(
                "CMMN case-file instance '{}' already exists",
                item.id
            )));
        }
        if let Some(parent_id) = item.parent_id.as_deref() {
            let parent = self
                .items
                .iter()
                .find(|candidate| {
                    candidate.id == parent_id && candidate.state != CmmnCaseFileItemState::Removed
                })
                .ok_or_else(|| {
                    CmmnError::not_found(format!(
                        "CMMN case-file parent '{parent_id}' was not found"
                    ))
                })?;
            item.path = format!("{}/{}", parent.path, item.id);
        } else {
            item.path = format!("/{}", item.id);
        }
        item.version = 1;
        self.items.push(item);
        Ok(())
    }

    pub fn get(&self, instance_id: &str) -> Option<&CmmnCaseFileItem> {
        self.items.iter().find(|item| item.id == instance_id)
    }
    pub fn get_mut(&mut self, instance_id: &str) -> Option<&mut CmmnCaseFileItem> {
        self.items.iter_mut().find(|item| item.id == instance_id)
    }
    pub fn children(&self, parent_id: &str) -> Vec<CmmnCaseFileItem> {
        self.items
            .iter()
            .filter(|item| item.parent_id.as_deref() == Some(parent_id))
            .cloned()
            .collect()
    }
    pub fn descendants(&self, parent_id: &str) -> Vec<CmmnCaseFileItem> {
        let prefix = self
            .get(parent_id)
            .map(|item| format!("{}/", item.path))
            .unwrap_or_default();
        self.items
            .iter()
            .filter(|item| item.path.starts_with(&prefix))
            .cloned()
            .collect()
    }
    pub fn remove_subtree(&mut self, instance_id: &str) -> Result<Vec<String>, CmmnError> {
        let path = self
            .get(instance_id)
            .ok_or_else(|| {
                CmmnError::not_found(format!(
                    "CMMN case-file instance '{instance_id}' was not found"
                ))
            })?
            .path
            .clone();
        let prefix = format!("{path}/");
        let mut removed = Vec::new();
        for item in self
            .items
            .iter_mut()
            .filter(|item| item.path == path || item.path.starts_with(&prefix))
        {
            item.state = CmmnCaseFileItemState::Removed;
            item.version = item.version.saturating_add(1);
            removed.push(item.id.clone());
        }
        Ok(removed)
    }
    pub fn ancestry_definition_refs(&self, instance_id: &str) -> Vec<String> {
        let mut refs = Vec::new();
        let mut current = self.get(instance_id);
        while let Some(item) = current {
            refs.push(item.definition_ref.clone());
            current = item
                .parent_id
                .as_deref()
                .and_then(|parent| self.get(parent));
        }
        refs
    }

    fn validate(&self) -> Result<(), CmmnError> {
        for item in self.items.iter() {
            if let Some(parent_id) = item.parent_id.as_deref() {
                if parent_id == item.id {
                    return Err(CmmnError::conflict(format!(
                        "CMMN case-file instance '{}' cannot be its own parent",
                        item.id
                    )));
                }
                if !self.items.iter().any(|candidate| candidate.id == parent_id) {
                    return Err(CmmnError::not_found(format!(
                        "CMMN case-file parent '{parent_id}' was not found"
                    )));
                }
            }
        }
        Ok(())
    }
}
