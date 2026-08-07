use crate::models::FormDefinition;
use crate::repository;
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::error::FlowableError;
use std::sync::Arc;

/// Form management service providing administrative operations:
/// batch delete, version management, and activation/deactivation.
pub struct FormManagementService {
    engine: Arc<ProcessEngine>,
}

impl FormManagementService {
    pub fn new(engine: Arc<ProcessEngine>) -> Self {
        repository::ensure_schema(&engine.get_runtime_store());
        Self { engine }
    }

    /// Delete all form definitions (and their instances) for a given deployment.
    /// Returns the number of deleted definitions.
    pub fn delete_definitions_by_deployment_id(
        &self,
        deployment_id: &str,
    ) -> Result<usize, FlowableError> {
        let store = self.engine.get_runtime_store();
        repository::delete_form_definitions_by_deployment_id(&store, deployment_id)
    }

    /// Delete all form definitions (and their instances) for a given key.
    /// Returns the number of deleted definitions.
    pub fn delete_definitions_by_key(&self, key: &str) -> Result<usize, FlowableError> {
        let store = self.engine.get_runtime_store();
        repository::delete_form_definitions_by_key(&store, key)
    }

    /// List all versions of a form definition by key, ordered by version descending.
    pub fn list_versions(&self, key: &str) -> Result<Vec<FormDefinition>, FlowableError> {
        let store = self.engine.get_runtime_store();
        repository::find_form_definitions_by_key(&store, key)
    }

    /// Get the latest (highest version) active form definition for a key.
    /// Returns an error if no active version exists.
    pub fn get_latest_version(&self, key: &str) -> Result<FormDefinition, FlowableError> {
        let store = self.engine.get_runtime_store();
        let definitions = repository::find_form_definitions_by_key(&store, key)?;

        definitions
            .into_iter()
            .filter(|d| d.active.unwrap_or(true))
            .max_by(|left, right| {
                left.version
                    .cmp(&right.version)
                    .then(left.id.cmp(&right.id))
            })
            .ok_or_else(|| {
                FlowableError::NotFound(format!(
                    "No active form definition found for key '{}'",
                    key
                ))
            })
    }

    /// Get a specific version of a form definition by key and version number.
    pub fn get_version(&self, key: &str, version: i32) -> Result<FormDefinition, FlowableError> {
        let store = self.engine.get_runtime_store();
        repository::find_form_definition_by_key_and_version(&store, key, version)?.ok_or_else(
            || {
                FlowableError::NotFound(format!(
                    "Form definition not found for key '{}' version {}",
                    key, version
                ))
            },
        )
    }

    /// Activate or deactivate a form definition by its id.
    /// Returns the updated definition.
    pub fn set_activation(&self, id: &str, active: bool) -> Result<FormDefinition, FlowableError> {
        let store = self.engine.get_runtime_store();

        // Verify the definition exists
        let mut definition = repository::find_form_definition(&store, id).ok_or_else(|| {
            FlowableError::NotFound(format!("Form definition '{}' was not found", id))
        })?;

        // Update in database
        repository::update_form_definition_activation(&store, id, active)?;

        // Update in-memory and return
        definition.active = Some(active);
        Ok(definition)
    }
}
