use crate::error::AppError;
use crate::models::{DefinitionType, ResolvedDefinition};
use std::collections::BTreeMap;

/// Tenant matching policy for cross-engine definition resolution.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TenantResolutionPolicy {
    /// Prefer tenant-specific definitions, then fall back to no-tenant definitions.
    /// This is the historical/default behavior.
    #[default]
    FallbackToDefault,
    /// Require an exact tenant match. When `tenant_id` is `None`, only no-tenant
    /// definitions are eligible.
    Strict,
}

pub trait DefinitionCatalog: Send + Sync {
    /// Resolve a referenced definition using the default tenant fallback policy.
    fn resolve_definition(
        &self,
        definition_type: DefinitionType,
        definition_key: &str,
        tenant_id: Option<&str>,
    ) -> Result<Option<ResolvedDefinition>, AppError>;

    /// Resolve a referenced definition with an explicit tenant policy.
    ///
    /// Default implementation preserves `resolve_definition` behavior for
    /// [`TenantResolutionPolicy::FallbackToDefault`] and filters the result for
    /// [`TenantResolutionPolicy::Strict`].
    fn resolve_definition_with_policy(
        &self,
        definition_type: DefinitionType,
        definition_key: &str,
        tenant_id: Option<&str>,
        policy: TenantResolutionPolicy,
    ) -> Result<Option<ResolvedDefinition>, AppError> {
        let resolved = self.resolve_definition(definition_type, definition_key, tenant_id)?;
        match policy {
            TenantResolutionPolicy::FallbackToDefault => Ok(resolved),
            TenantResolutionPolicy::Strict => {
                Ok(resolved.filter(|definition| definition.tenant_id.as_deref() == tenant_id))
            }
        }
    }

    /// Resolve a definition pinned by its exact id (`AppReference::definition_id`).
    /// Returns `Ok(None)` when no definition of this type has the given id;
    /// callers must not fall back to latest-by-key for pinned references.
    fn resolve_definition_by_id(
        &self,
        definition_type: DefinitionType,
        definition_id: &str,
    ) -> Result<Option<ResolvedDefinition>, AppError>;
}

#[derive(Clone, Debug, Default)]
pub struct InMemoryDefinitionCatalog {
    definitions: BTreeMap<(Option<String>, DefinitionType, String), ResolvedDefinition>,
}

impl InMemoryDefinitionCatalog {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn builder() -> InMemoryDefinitionCatalogBuilder {
        InMemoryDefinitionCatalogBuilder::default()
    }
}

impl DefinitionCatalog for InMemoryDefinitionCatalog {
    fn resolve_definition(
        &self,
        definition_type: DefinitionType,
        definition_key: &str,
        tenant_id: Option<&str>,
    ) -> Result<Option<ResolvedDefinition>, AppError> {
        self.resolve_definition_with_policy(
            definition_type,
            definition_key,
            tenant_id,
            TenantResolutionPolicy::FallbackToDefault,
        )
    }

    fn resolve_definition_with_policy(
        &self,
        definition_type: DefinitionType,
        definition_key: &str,
        tenant_id: Option<&str>,
        policy: TenantResolutionPolicy,
    ) -> Result<Option<ResolvedDefinition>, AppError> {
        let requested_key = definition_key.to_string();
        let tenant_match = tenant_id.map(str::to_string);

        if let Some(resolved) =
            self.definitions
                .get(&(tenant_match.clone(), definition_type, requested_key.clone()))
        {
            return Ok(Some(resolved.clone()));
        }

        match policy {
            TenantResolutionPolicy::Strict => Ok(None),
            TenantResolutionPolicy::FallbackToDefault => {
                if tenant_match.is_some() {
                    Ok(self
                        .definitions
                        .get(&(None, definition_type, requested_key))
                        .cloned())
                } else {
                    Ok(None)
                }
            }
        }
    }

    fn resolve_definition_by_id(
        &self,
        definition_type: DefinitionType,
        definition_id: &str,
    ) -> Result<Option<ResolvedDefinition>, AppError> {
        Ok(self
            .definitions
            .values()
            .find(|definition| {
                definition.definition_type == definition_type
                    && definition.definition_id == definition_id
            })
            .cloned())
    }
}

#[derive(Clone, Debug, Default)]
pub struct InMemoryDefinitionCatalogBuilder {
    definitions: BTreeMap<(Option<String>, DefinitionType, String), ResolvedDefinition>,
}

impl InMemoryDefinitionCatalogBuilder {
    pub fn with_process_definition(
        self,
        definition_key: impl Into<String>,
        definition_name: impl Into<String>,
        version: i32,
        tenant_id: Option<&str>,
    ) -> Self {
        self.with_definition(
            DefinitionType::BpmnProcess,
            definition_key,
            definition_name,
            version,
            tenant_id,
        )
    }

    pub fn with_decision_definition(
        self,
        definition_key: impl Into<String>,
        definition_name: impl Into<String>,
        version: i32,
        tenant_id: Option<&str>,
    ) -> Self {
        self.with_definition(
            DefinitionType::DmnDecision,
            definition_key,
            definition_name,
            version,
            tenant_id,
        )
    }

    pub fn with_case_definition(
        self,
        definition_key: impl Into<String>,
        definition_name: impl Into<String>,
        version: i32,
        tenant_id: Option<&str>,
    ) -> Self {
        self.with_definition(
            DefinitionType::CmmnCase,
            definition_key,
            definition_name,
            version,
            tenant_id,
        )
    }

    pub fn with_event_definition(
        self,
        definition_key: impl Into<String>,
        definition_name: impl Into<String>,
        version: i32,
        tenant_id: Option<&str>,
    ) -> Self {
        self.with_definition(
            DefinitionType::EventRegistry,
            definition_key,
            definition_name,
            version,
            tenant_id,
        )
    }

    pub fn with_definition(
        mut self,
        definition_type: DefinitionType,
        definition_key: impl Into<String>,
        definition_name: impl Into<String>,
        version: i32,
        tenant_id: Option<&str>,
    ) -> Self {
        let definition_key = definition_key.into();
        let tenant_id = tenant_id.map(str::to_string);
        let slug = definition_type.slug();
        let resolved = ResolvedDefinition {
            definition_type,
            definition_id: format!("{slug}:{definition_key}:v{version}"),
            definition_key: definition_key.clone(),
            definition_name: definition_name.into(),
            deployment_id: format!("{slug}-deployment:{definition_key}:v{version}"),
            version,
            tenant_id: tenant_id.clone(),
        };
        self.definitions
            .insert((tenant_id, definition_type, definition_key), resolved);
        self
    }

    pub fn build(self) -> InMemoryDefinitionCatalog {
        InMemoryDefinitionCatalog {
            definitions: self.definitions,
        }
    }
}
