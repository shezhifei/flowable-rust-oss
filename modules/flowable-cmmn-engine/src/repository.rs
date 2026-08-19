use crate::deployment::is_cmmn_resource;
use crate::error::CmmnError;
use crate::event_registry_correlation::generate_correlation_key;
use crate::models::{
    CmmnCase, CmmnCaseDefinition, CmmnCaseFileItemOnPart, CmmnCaseTask, CmmnDecisionTask,
    CmmnDeployment, CmmnDeploymentRequest, CmmnEventListener, CmmnEventSubscription, CmmnHumanTask,
    CmmnIdentityLink, CmmnMilestone, CmmnModel, CmmnPlanItem, CmmnPlanItemOnPart, CmmnPlanningTable,
    CmmnProcessTask, CmmnSentry, CmmnSentryIfPartExpression, CmmnSentryIfPartLiteral, CmmnStage,
    CmmnTaskAssociationKind, PagedResult, START_EVENT_CORRELATION_MANUAL,
    is_supported_number_literal,
};
use crate::process_cleanup::ProcessInstanceCleanup;
use crate::store::CmmnStore;
use chrono::Utc;
use flowable_persistence::db_session::{DbSession, FilterOp};
use flowable_persistence::entity::cmmn_case_definition::{
    CmmnCaseDefinitionDataManager, CmmnCaseDefinitionEntity,
};
use flowable_persistence::entity::cmmn_case_instance::CmmnCaseInstanceDataManager;
use flowable_persistence::entity::cmmn_deployment::{
    CmmnDeploymentDataManager, CmmnDeploymentEntity,
};
use flowable_persistence::entity::cmmn_deployment_resource::{
    CmmnDeploymentResourceDataManager, CmmnDeploymentResourceEntity,
};
use flowable_persistence::entity::cmmn_human_task::CmmnHumanTaskDataManager;
use flowable_persistence::entity::cmmn_human_task_history::CmmnHumanTaskHistoryDataManager;
use flowable_persistence::entity::cmmn_identity_link::{
    CmmnIdentityLinkDataManager, CmmnIdentityLinkEntity,
};
use flowable_persistence::entity::cmmn_task_instance_association::CmmnTaskInstanceAssociationDataManager;
use flowable_persistence::error::PersistenceError;
use flowable_persistence::statement::{RenderedStatement, StatementId};
use flowable_persistence::value::DbParams;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::Arc;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct CmmnDeploymentResourceData {
    pub deployment_id: String,
    pub resource_name: String,
    pub resource_type: String,
    pub content_type: String,
    pub bytes: Vec<u8>,
    pub created_at: i64,
}

impl CmmnDeploymentResourceData {
    pub fn new(
        deployment_id: String,
        resource_name: String,
        bytes: Vec<u8>,
        created_at: i64,
    ) -> Self {
        Self {
            deployment_id,
            resource_type: cmmn_resource_type_for_name(&resource_name).to_string(),
            content_type: cmmn_content_type_for_name(&resource_name).to_string(),
            resource_name,
            bytes,
            created_at,
        }
    }
}

pub fn cmmn_content_type_for_name(resource_name: &str) -> &'static str {
    let lower_name = resource_name.to_ascii_lowercase();
    if lower_name.ends_with(".cmmn") || lower_name.ends_with(".xml") {
        "application/xml"
    } else if lower_name.ends_with(".json") {
        "application/json"
    } else if lower_name.ends_with(".svg") {
        "image/svg+xml"
    } else if lower_name.ends_with(".png") {
        "image/png"
    } else if lower_name.ends_with(".jpg") || lower_name.ends_with(".jpeg") {
        "image/jpeg"
    } else if lower_name.ends_with(".gif") {
        "image/gif"
    } else if lower_name.ends_with(".txt") {
        "text/plain"
    } else {
        "application/octet-stream"
    }
}

fn cmmn_resource_type_for_name(resource_name: &str) -> &'static str {
    let lower_name = resource_name.to_ascii_lowercase();
    if lower_name.ends_with(".cmmn") {
        "caseDefinition"
    } else {
        "resource"
    }
}

/// Lightweight DTO representing a resolved DMN decision referenced by a
/// case definition. This type lives in the CMMN engine boundary so the
/// CMMN engine does not depend on the DMN engine crate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReferencedDecision {
    pub id: String,
    pub key: String,
    pub name: String,
    pub version: i32,
    pub deployment_id: String,
    pub tenant_id: Option<String>,
    pub resource_name: String,
}

/// Lightweight DTO representing a resolved form definition referenced by a
/// case definition. This type lives in the CMMN engine boundary so the
/// CMMN engine does not depend on the form service crate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReferencedFormDefinition {
    pub id: String,
    pub key: String,
    pub name: String,
    pub version: i32,
    pub deployment_id: String,
    pub resource_name: String,
}

/// Resolver trait for DMN decisions referenced by a case definition.
///
/// Implementors should apply parent-deployment scoping when
/// `parent_deployment_id` is `Some`: only return decisions whose DMN
/// deployment has that parent deployment ID. When `parent_deployment_id`
/// is `None`, fall back to the latest version of the decision.
///
/// Returns `Ok(None)` when the decision key cannot be resolved, matching
/// Java's behavior of silently omitting missing references.
pub trait CmmnDecisionResolver: Send + Sync {
    fn resolve_decision(
        &self,
        key: &str,
        tenant_id: Option<&str>,
        parent_deployment_id: Option<&str>,
    ) -> Result<Option<ReferencedDecision>, CmmnError>;
}

/// Resolver trait for form definitions referenced by a case definition.
///
/// Same scoping semantics as [`CmmnDecisionResolver`]. Returns `Ok(None)`
/// when the form key cannot be resolved, matching Java's behavior of
/// silently omitting missing references.
pub trait CmmnFormResolver: Send + Sync {
    fn resolve_form(
        &self,
        key: &str,
        tenant_id: Option<&str>,
        parent_deployment_id: Option<&str>,
    ) -> Result<Option<ReferencedFormDefinition>, CmmnError>;
}

#[derive(Clone)]
pub struct CmmnRepositoryService {
    store: CmmnStore,
    process_instance_cleanup: Option<Arc<dyn ProcessInstanceCleanup>>,
}

impl CmmnRepositoryService {
    pub(crate) fn new(
        store: CmmnStore,
        process_instance_cleanup: Option<Arc<dyn ProcessInstanceCleanup>>,
    ) -> Self {
        Self {
            store,
            process_instance_cleanup,
        }
    }

    pub fn deploy(&self, mut request: CmmnDeploymentRequest) -> Result<CmmnDeployment, CmmnError> {
        request.name = normalized_optional_key(request.name.as_deref());
        request.category = normalized_optional_key(request.category.as_deref());
        request.key = normalized_optional_key(request.key.as_deref());
        request.tenant_id = normalized_optional_key(request.tenant_id.as_deref());
        request.parent_deployment_id =
            normalized_optional_key(request.parent_deployment_id.as_deref());
        validate_deployment_request(&request)?;

        if request.enable_duplicate_filtering
            && let Some(existing) = self.find_duplicate_deployment(&request)?
        {
            return Ok(existing);
        }

        let deployment_id = format!("cmmn-deployment:{}", Uuid::new_v4());
        let deployed_at = Utc::now();
        let deployment = CmmnDeployment {
            id: deployment_id.clone(),
            name: request.name.clone(),
            category: request.category.clone(),
            key: request.key.clone(),
            tenant_id: request.tenant_id.clone(),
            parent_deployment_id: request.parent_deployment_id.clone(),
            resource_names: request
                .resources
                .iter()
                .map(|resource| resource.resource_name.clone())
                .collect(),
            deployed_at,
        };

        let mut session = self.store.create_session()?;
        let deployment_manager = CmmnDeploymentDataManager::new();
        let resource_manager = CmmnDeploymentResourceDataManager::new();
        let definition_manager = CmmnCaseDefinitionDataManager::new();
        let resource_names: Vec<_> = request
            .resources
            .iter()
            .map(|resource| resource.resource_name.clone())
            .collect();
        let mut deployment_entity = CmmnDeploymentEntity::new(
            deployment.id.clone(),
            deployment.name.clone().unwrap_or_default(),
            deployment.deployed_at.to_rfc3339(),
            serde_json::to_string(&deployment)?,
        );
        deployment_entity.set_tenant_id(deployment.tenant_id.clone());
        deployment_entity.set_metadata(
            deployment.category.clone(),
            deployment.key.clone(),
            deployment.parent_deployment_id.clone(),
        );
        deployment_manager.insert(&mut session, deployment_entity)?;

        for resource in request.resources {
            let created_at = deployed_at.timestamp_millis();
            let resource_data = CmmnDeploymentResourceData::new(
                deployment_id.clone(),
                resource.resource_name.clone(),
                resource.resource_bytes.clone(),
                created_at,
            );
            resource_manager.insert(
                &mut session,
                CmmnDeploymentResourceEntity::new(
                    deployment_id.clone(),
                    resource.resource_name.clone(),
                    resource_data.resource_type,
                    resource_data.content_type,
                    resource.resource_bytes.clone(),
                    created_at,
                ),
            )?;

            for case_model in resource.model.cases {
                let version =
                    next_version(&mut session, &case_model.key, request.tenant_id.as_deref())?;
                // Capture previous version before insert for event-subscription migration
                // (CmmnDeployer.updateEventSubscriptions.java:194-224).
                let previous_definition = if version > 1 {
                    find_previous_case_definition_session(
                        &mut session,
                        &case_model.key,
                        request.tenant_id.as_deref(),
                        version,
                    )?
                } else {
                    None
                };
                let definition = CmmnCaseDefinition {
                    id: format!("cmmn-case-definition:{}:{}", deployment_id, case_model.key),
                    case_id: case_model.id.clone(),
                    deployment_id: deployment_id.clone(),
                    key: case_model.key.clone(),
                    name: case_model.name.clone(),
                    version,
                    category: request.category.clone(),
                    tenant_id: request.tenant_id.clone(),
                    resource_name: resource.resource_name.clone(),
                    diagram_resource_name: find_case_diagram_resource_name(
                        &resource_names,
                        &resource.resource_name,
                        &case_model.key,
                    ),
                    model: case_model,
                };
                let mut definition_entity = CmmnCaseDefinitionEntity::new(
                    definition.id.clone(),
                    definition.key.clone(),
                    definition.deployment_id.clone(),
                    definition.version,
                    definition.resource_name.clone(),
                    serde_json::to_string(&definition)?,
                );
                definition_entity.set_tenant_id(definition.tenant_id.clone());
                definition_entity.set_metadata(
                    definition.category.clone(),
                    definition.diagram_resource_name.clone(),
                );
                definition_manager.insert(&mut session, definition_entity)?;

                // P136: definition-level event-registry start subscriptions
                // (CmmnDeployer.java:194-224).
                update_event_subscriptions_for_case_definition(
                    &mut session,
                    &definition,
                    previous_definition.as_ref(),
                )?;
            }
        }

        session.commit()?;
        Ok(deployment)
    }

    pub fn create_deployment_query(&self) -> CmmnDeploymentQuery {
        CmmnDeploymentQuery::new(self.store.clone())
    }

    pub fn new_deployment(&self) -> crate::deployment::CmmnDeploymentBuilder {
        crate::deployment::CmmnDeploymentBuilder::new(self.clone())
    }

    fn find_duplicate_deployment(
        &self,
        request: &CmmnDeploymentRequest,
    ) -> Result<Option<CmmnDeployment>, CmmnError> {
        let mut requested_resources: Vec<_> = request
            .resources
            .iter()
            .map(|resource| (&resource.resource_name, &resource.resource_bytes))
            .collect();
        requested_resources.sort_by(|left, right| left.0.cmp(right.0));

        for deployment in self.create_deployment_query().list()? {
            if deployment.name != request.name
                || deployment.category != request.category
                || deployment.key != request.key
                || deployment.tenant_id != request.tenant_id
                || deployment.parent_deployment_id != request.parent_deployment_id
            {
                continue;
            }
            let mut existing_names = deployment.resource_names.clone();
            existing_names.sort();
            if existing_names.len() != requested_resources.len()
                || existing_names
                    .iter()
                    .zip(&requested_resources)
                    .any(|(name, resource)| name != resource.0)
            {
                continue;
            }
            if requested_resources.iter().all(|(name, bytes)| {
                self.get_deployment_resource_bytes(&deployment.id, name)
                    .is_ok_and(|existing| existing == **bytes)
            }) {
                return Ok(Some(deployment));
            }
        }
        Ok(None)
    }

    pub fn get_deployment(&self, deployment_id: &str) -> Result<CmmnDeployment, CmmnError> {
        let mut session = self.store.create_session()?;
        CmmnDeploymentDataManager::new()
            .find_by_id(&mut session, deployment_id)?
            .map(deployment_from_entity)
            .transpose()?
            .ok_or_else(|| {
                CmmnError::not_found(format!("CMMN deployment '{deployment_id}' was not found"))
            })
    }

    pub fn get_deployment_resource_bytes(
        &self,
        deployment_id: &str,
        resource_name: &str,
    ) -> Result<Vec<u8>, CmmnError> {
        let mut session = self.store.create_session()?;
        CmmnDeploymentResourceDataManager::new()
            .find_by_id(&mut session, deployment_id, resource_name)?
            .map(|entity| entity.bytes)
            .ok_or_else(|| {
                CmmnError::not_found(format!(
                    "CMMN deployment resource '{}' was not found in deployment '{}'",
                    resource_name, deployment_id
                ))
            })
    }

    pub fn get_deployment_resource_data(
        &self,
        deployment_id: &str,
        resource_name: &str,
    ) -> Result<CmmnDeploymentResourceData, CmmnError> {
        let mut session = self.store.create_session()?;
        CmmnDeploymentResourceDataManager::new()
            .find_by_id(&mut session, deployment_id, resource_name)?
            .map(resource_entity_to_data)
            .ok_or_else(|| {
                CmmnError::not_found(format!(
                    "CMMN deployment resource '{}' was not found in deployment '{}'",
                    resource_name, deployment_id
                ))
            })
    }

    pub fn get_deployment_resources(
        &self,
        deployment_id: &str,
    ) -> Result<Vec<CmmnDeploymentResourceData>, CmmnError> {
        self.get_deployment(deployment_id)?;
        let mut session = self.store.create_session()?;
        let mut resources = CmmnDeploymentResourceDataManager::new()
            .find_by_deployment_id(&mut session, deployment_id)?
            .into_iter()
            .map(resource_entity_to_data)
            .collect::<Vec<_>>();
        resources.sort_by(|left, right| left.resource_name.cmp(&right.resource_name));
        Ok(resources)
    }

    pub fn delete_deployment(&self, deployment_id: &str, cascade: bool) -> Result<(), CmmnError> {
        self.get_deployment(deployment_id)?;

        let definitions = self
            .create_case_definition_query()
            .deployment_id(deployment_id.to_string())
            .list()?;
        let definition_ids: Vec<String> = definitions
            .iter()
            .map(|definition| definition.id.clone())
            .collect();

        let mut session = self.store.create_session()?;
        let case_instance_manager = CmmnCaseInstanceDataManager::new();

        if !cascade {
            for definition_id in &definition_ids {
                let active_instances = case_instance_manager
                    .find_by_case_definition_id(&mut session, definition_id)?;
                if !active_instances.is_empty() {
                    return Err(CmmnError::validation(format!(
                        "CMMN deployment '{deployment_id}' cannot be deleted because it has active case instances"
                    )));
                }
            }
        }

        if cascade {
            let mut visited_child_instances: HashSet<String> = HashSet::new();
            let mut visited_process_instances: HashSet<String> = HashSet::new();
            for definition_id in &definition_ids {
                cascade_purge_definition(
                    &mut session,
                    definition_id,
                    &mut visited_child_instances,
                    &mut visited_process_instances,
                    self.process_instance_cleanup.as_ref(),
                )?;
            }
        } else {
            // Java always deletes event subscriptions for the definition even without cascade
            // (CmmnDeploymentEntityManagerImpl.java:63-64).
            for definition_id in &definition_ids {
                let mut dp = DbParams::new();
                dp.push(definition_id.as_str());
                session.execute(
                    StatementId::DeleteCmmnEventSubscriptionsByCaseDefinitionId,
                    dp,
                )?;
            }
        }

        let mut p = DbParams::new();
        p.push(deployment_id);

        // Snapshot definitions before delete so we can restore previous-version start
        // subscriptions (CmmnDeploymentEntityManagerImpl.java:72-75, :81-108).
        let definitions_to_restore = definitions.clone();

        for definition_id in &definition_ids {
            let mut dp = DbParams::new();
            dp.push(definition_id.as_str());
            session.execute(StatementId::DeleteCmmnCaseDefinition, dp)?;
        }

        session.execute(
            StatementId::DeleteCmmnDeploymentResourcesByDeploymentId,
            p.clone(),
        )?;
        let deployment_affected = session.execute(StatementId::DeleteCmmnDeployment, p)?;
        if deployment_affected.rows_affected != 1 {
            return Err(CmmnError::conflict(format!(
                "CMMN deployment '{deployment_id}' metadata deletion affected {} rows",
                deployment_affected.rows_affected
            )));
        }

        // After removal, if the deleted def was the latest, restore start events on the
        // new latest (previous version). Java: restorePreviousStartEventsIfNeeded.
        for definition in &definitions_to_restore {
            restore_previous_start_events_if_needed(&mut session, definition)?;
        }

        session.commit()?;
        Ok(())
    }

    pub fn create_case_definition_query(&self) -> CmmnCaseDefinitionQuery {
        CmmnCaseDefinitionQuery::new(self.store.clone())
    }

    pub fn get_case_definition(
        &self,
        case_definition_id: &str,
    ) -> Result<CmmnCaseDefinition, CmmnError> {
        let mut session = self.store.create_session()?;
        CmmnCaseDefinitionDataManager::new()
            .find_by_id(&mut session, case_definition_id)?
            .map(case_definition_from_entity)
            .transpose()?
            .ok_or_else(|| {
                CmmnError::not_found(format!(
                    "CMMN case definition '{case_definition_id}' was not found"
                ))
            })
    }

    pub fn get_case_definition_resource_bytes(
        &self,
        case_definition_id: &str,
    ) -> Result<Vec<u8>, CmmnError> {
        let definition = self.get_case_definition(case_definition_id)?;
        self.get_deployment_resource_bytes(&definition.deployment_id, &definition.resource_name)
    }

    pub fn get_case_diagram(
        &self,
        case_definition_id: &str,
    ) -> Result<Option<CmmnDeploymentResourceData>, CmmnError> {
        let definition = self.get_case_definition(case_definition_id)?;
        definition
            .diagram_resource_name
            .map(|resource_name| {
                self.get_deployment_resource_data(&definition.deployment_id, &resource_name)
            })
            .transpose()
    }

    pub fn list_case_definition_decision_keys(
        &self,
        case_definition_id: &str,
    ) -> Result<Vec<String>, CmmnError> {
        let definition = self.get_case_definition(case_definition_id)?;
        Ok(case_definition_decision_keys(&definition.model))
    }

    pub fn list_case_definition_form_keys(
        &self,
        case_definition_id: &str,
    ) -> Result<Vec<String>, CmmnError> {
        let definition = self.get_case_definition(case_definition_id)?;
        Ok(case_definition_form_keys(&definition.model))
    }

    pub fn get_case_definition_start_form_key(
        &self,
        case_definition_id: &str,
    ) -> Result<Option<String>, CmmnError> {
        let definition = self.get_case_definition(case_definition_id)?;
        Ok(normalized_optional_key(
            definition.model.case_plan_model.start_form_key.as_deref(),
        ))
    }

    /// Resolves all DMN decisions referenced by a case definition into
    /// complete objects via the injected [`CmmnDecisionResolver`].
    ///
    /// Mirrors Java's `getDecisionsForCaseDefinition`: silently omits
    /// references that cannot be resolved (the resolver returns `None`).
    /// Uses parent-deployment scoping when the case definition's deployment
    /// has a parent deployment ID; otherwise falls back to the latest version.
    ///
    /// Results are sorted by key then version for deterministic ordering.
    pub fn list_referenced_decisions(
        &self,
        case_definition_id: &str,
        resolver: &dyn CmmnDecisionResolver,
    ) -> Result<Vec<ReferencedDecision>, CmmnError> {
        let definition = self.get_case_definition(case_definition_id)?;
        let parent_deployment_id = self
            .get_deployment(&definition.deployment_id)?
            .parent_deployment_id;
        let decision_keys = case_definition_decision_keys(&definition.model);
        let mut results = Vec::new();
        for key in decision_keys {
            if let Some(decision) = resolver.resolve_decision(
                &key,
                definition.tenant_id.as_deref(),
                parent_deployment_id.as_deref(),
            )? {
                results.push(decision);
            }
        }
        results.sort_by(|a, b| {
            a.key
                .cmp(&b.key)
                .then(a.version.cmp(&b.version))
                .then(a.id.cmp(&b.id))
        });
        Ok(results)
    }

    /// Resolves all form definitions referenced by a case definition into
    /// complete objects via the injected [`CmmnFormResolver`].
    ///
    /// Mirrors Java's `getFormDefinitionsForCaseDefinition`: silently omits
    /// references that cannot be resolved (the resolver returns `None`).
    /// Uses parent-deployment scoping when the case definition's deployment
    /// has a parent deployment ID; otherwise falls back to the latest version.
    ///
    /// Results are sorted by key then version for deterministic ordering.
    pub fn list_referenced_form_definitions(
        &self,
        case_definition_id: &str,
        resolver: &dyn CmmnFormResolver,
    ) -> Result<Vec<ReferencedFormDefinition>, CmmnError> {
        let definition = self.get_case_definition(case_definition_id)?;
        let parent_deployment_id = self
            .get_deployment(&definition.deployment_id)?
            .parent_deployment_id;
        let form_keys = case_definition_form_keys(&definition.model);
        let mut results = Vec::new();
        for key in form_keys {
            if let Some(form) = resolver.resolve_form(
                &key,
                definition.tenant_id.as_deref(),
                parent_deployment_id.as_deref(),
            )? {
                results.push(form);
            }
        }
        results.sort_by(|a, b| {
            a.key
                .cmp(&b.key)
                .then(a.version.cmp(&b.version))
                .then(a.id.cmp(&b.id))
        });
        Ok(results)
    }

    /// Sets the category of a case definition. Pass `None` to clear the
    /// category. Mirrors Java's `setCaseDefinitionCategory`.
    ///
    /// Returns `NotFound` if `case_definition_id` does not refer to a persisted
    /// case definition.
    pub fn set_case_definition_category(
        &self,
        case_definition_id: &str,
        category: Option<&str>,
    ) -> Result<(), CmmnError> {
        let normalized = normalized_optional_key(category);
        let mut session = self.store.create_session()?;
        let manager = CmmnCaseDefinitionDataManager::new();
        if manager
            .find_by_id(&mut session, case_definition_id)?
            .is_none()
        {
            return Err(CmmnError::not_found(format!(
                "CMMN case definition '{case_definition_id}' was not found"
            )));
        }
        manager.update_category(&mut session, case_definition_id, normalized)?;
        session.commit()?;
        Ok(())
    }

    /// Changes the parent deployment ID of a deployment. Pass `None` to clear
    /// the parent. Mirrors Java's `changeDeploymentParentDeploymentId`.
    ///
    /// Returns `NotFound` if `deployment_id` does not refer to a persisted
    /// deployment. The new parent deployment ID is not validated against
    /// existing deployments, matching Java's permissive behavior.
    pub fn set_deployment_parent_id(
        &self,
        deployment_id: &str,
        parent_deployment_id: Option<&str>,
    ) -> Result<(), CmmnError> {
        let normalized = normalized_optional_key(parent_deployment_id);
        let mut session = self.store.create_session()?;
        let manager = CmmnDeploymentDataManager::new();
        if manager.find_by_id(&mut session, deployment_id)?.is_none() {
            return Err(CmmnError::not_found(format!(
                "CMMN deployment '{deployment_id}' was not found"
            )));
        }
        manager.update_parent_id(&mut session, deployment_id, normalized)?;
        session.commit()?;
        Ok(())
    }

    /// Adds a candidate starter user to a case definition. Mirrors Java's
    /// `addCandidateStarterUser`. Idempotent: adding the same user twice does
    /// not create a duplicate link.
    ///
    /// Returns `NotFound` if the case definition does not exist.
    pub fn add_candidate_starter_user(
        &self,
        case_definition_id: &str,
        user_id: &str,
    ) -> Result<(), CmmnError> {
        self.add_candidate_starter_link(case_definition_id, Some(user_id), None)
    }

    /// Adds a candidate starter group to a case definition. Mirrors Java's
    /// `addCandidateStarterGroup`. Idempotent: adding the same group twice does
    /// not create a duplicate link.
    ///
    /// Returns `NotFound` if the case definition does not exist.
    pub fn add_candidate_starter_group(
        &self,
        case_definition_id: &str,
        group_id: &str,
    ) -> Result<(), CmmnError> {
        self.add_candidate_starter_link(case_definition_id, None, Some(group_id))
    }

    fn add_candidate_starter_link(
        &self,
        case_definition_id: &str,
        user_id: Option<&str>,
        group_id: Option<&str>,
    ) -> Result<(), CmmnError> {
        let mut session = self.store.create_session()?;
        let def_manager = CmmnCaseDefinitionDataManager::new();
        if def_manager
            .find_by_id(&mut session, case_definition_id)?
            .is_none()
        {
            return Err(CmmnError::not_found(format!(
                "CMMN case definition '{case_definition_id}' was not found"
            )));
        }

        let link_manager = CmmnIdentityLinkDataManager::new();
        // Check for existing link to enforce idempotency.
        let existing =
            link_manager.find_by_scope(&mut session, "definition", case_definition_id)?;
        for link in &existing {
            if link.link_type == "candidate"
                && link.user_id.as_deref() == user_id
                && link.group_id.as_deref() == group_id
            {
                // Duplicate link already exists; no-op.
                return Ok(());
            }
        }

        let link_id = format!("cmmn-starter:{}", Uuid::new_v4());
        let link = CmmnIdentityLink {
            id: link_id.clone(),
            scope_type: "definition".to_string(),
            scope_id: case_definition_id.to_string(),
            link_type: "candidate".to_string(),
            user_id: user_id.map(str::to_string),
            group_id: group_id.map(str::to_string),
        };
        let mut entity = CmmnIdentityLinkEntity::new(
            link.id.clone(),
            link.scope_type.clone(),
            link.scope_id.clone(),
            link.link_type.clone(),
            serde_json::to_string(&link)?,
        );
        entity.set_user_id(link.user_id.clone());
        entity.set_group_id(link.group_id.clone());
        link_manager.insert(&mut session, entity)?;
        session.commit()?;
        Ok(())
    }

    /// Removes a candidate starter user from a case definition. Mirrors Java's
    /// `deleteCandidateStarterUser`. No-op if the link does not exist.
    pub fn delete_candidate_starter_user(
        &self,
        case_definition_id: &str,
        user_id: &str,
    ) -> Result<(), CmmnError> {
        self.delete_candidate_starter_link(case_definition_id, Some(user_id), None)
    }

    /// Removes a candidate starter group from a case definition. Mirrors Java's
    /// `deleteCandidateStarterGroup`. No-op if the link does not exist.
    pub fn delete_candidate_starter_group(
        &self,
        case_definition_id: &str,
        group_id: &str,
    ) -> Result<(), CmmnError> {
        self.delete_candidate_starter_link(case_definition_id, None, Some(group_id))
    }

    fn delete_candidate_starter_link(
        &self,
        case_definition_id: &str,
        user_id: Option<&str>,
        group_id: Option<&str>,
    ) -> Result<(), CmmnError> {
        let mut session = self.store.create_session()?;
        let link_manager = CmmnIdentityLinkDataManager::new();
        let links = link_manager.find_by_scope(&mut session, "definition", case_definition_id)?;
        for link in &links {
            if link.link_type == "candidate"
                && link.user_id.as_deref() == user_id
                && link.group_id.as_deref() == group_id
            {
                link_manager.delete(&mut session, link)?;
            }
        }
        session.commit()?;
        Ok(())
    }

    /// Lists all identity links for a case definition. Mirrors Java's
    /// `getIdentityLinksForCaseDefinition`.
    pub fn get_identity_links_for_case_definition(
        &self,
        case_definition_id: &str,
    ) -> Result<Vec<CmmnIdentityLink>, CmmnError> {
        let mut session = self.store.create_session()?;
        let link_manager = CmmnIdentityLinkDataManager::new();
        let links = link_manager.find_by_scope(&mut session, "definition", case_definition_id)?;
        links
            .into_iter()
            .map(|entity| serde_json::from_str(&entity.data).map_err(Into::into))
            .collect()
    }

    pub(crate) fn latest_case_definition_by_key(
        &self,
        case_definition_key: &str,
        tenant_id: Option<&str>,
    ) -> Result<CmmnCaseDefinition, CmmnError> {
        self.create_case_definition_query()
            .key(case_definition_key)
            .tenant_id_optional(tenant_id)
            .single_result()?
            .ok_or_else(|| {
                CmmnError::not_found(format!(
                    "CMMN case definition '{case_definition_key}' was not found"
                ))
            })
    }
}

fn deployment_from_entity(entity: CmmnDeploymentEntity) -> Result<CmmnDeployment, CmmnError> {
    let mut deployment: CmmnDeployment = serde_json::from_str(&entity.data)?;
    deployment.name = normalized_optional_key(Some(&entity.name));
    deployment.category = entity.category;
    deployment.key = entity.key;
    deployment.tenant_id = entity.tenant_id;
    deployment.parent_deployment_id = entity.parent_deployment_id;
    Ok(deployment)
}

fn case_definition_from_entity(
    entity: CmmnCaseDefinitionEntity,
) -> Result<CmmnCaseDefinition, CmmnError> {
    let mut definition: CmmnCaseDefinition = serde_json::from_str(&entity.data)?;
    definition.category = entity.category;
    definition.tenant_id = entity.tenant_id;
    definition.diagram_resource_name = entity.diagram_resource_name;
    Ok(definition)
}

/// Sort direction for repository queries. Mirrors Java's `asc()` / `desc()`
/// ordering direction without reproducing the builder chain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SortDirection {
    #[default]
    Asc,
    Desc,
}

/// Sortable fields for `CmmnDeploymentQuery`, matching Java's
/// `orderByDeploymentId/Name/Time/TenantId` methods.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeploymentSortField {
    Id,
    Name,
    DeployedTime,
    TenantId,
}

impl DeploymentSortField {
    fn column(self) -> &'static str {
        match self {
            DeploymentSortField::Id => "ID_",
            DeploymentSortField::Name => "NAME_",
            DeploymentSortField::DeployedTime => "DEPLOYED_AT_",
            DeploymentSortField::TenantId => "TENANT_ID_",
        }
    }
}

/// Sortable fields for `CmmnCaseDefinitionQuery`, matching Java's
/// `orderByCaseDefinitionCategory/Key/Id/Version/Name/DeploymentId/TenantId`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaseDefinitionSortField {
    Id,
    Key,
    Category,
    Version,
    Name,
    DeploymentId,
    TenantId,
}

impl CaseDefinitionSortField {
    fn column(self) -> &'static str {
        match self {
            CaseDefinitionSortField::Id => "ID_",
            CaseDefinitionSortField::Key => "CASE_KEY_",
            CaseDefinitionSortField::Category => "CATEGORY_",
            CaseDefinitionSortField::Version => "VERSION_",
            // NAME_ is not a persisted column on ACT_CMMN_CASE_DEFINITION;
            // name-based ordering falls back to a stable id tie-breaker.
            CaseDefinitionSortField::Name => "ID_",
            CaseDefinitionSortField::DeploymentId => "DEPLOYMENT_ID_",
            CaseDefinitionSortField::TenantId => "TENANT_ID_",
        }
    }
}

pub struct CmmnDeploymentQuery {
    store: CmmnStore,
    id: Option<String>,
    ids: Vec<String>,
    name: Option<String>,
    name_like: Option<String>,
    category: Option<String>,
    category_not_equals: Option<String>,
    key: Option<String>,
    tenant_id: Option<String>,
    tenant_id_like: Option<String>,
    without_tenant_id: bool,
    parent_deployment_id: Option<String>,
    parent_deployment_id_like: Option<String>,
    parent_deployment_ids: Vec<String>,
    latest: bool,
    resource_name: Option<String>,
    sort_field: Option<DeploymentSortField>,
    sort_direction: SortDirection,
    start: usize,
    size: Option<usize>,
}

impl CmmnDeploymentQuery {
    fn new(store: CmmnStore) -> Self {
        Self {
            store,
            id: None,
            ids: Vec::new(),
            name: None,
            name_like: None,
            category: None,
            category_not_equals: None,
            key: None,
            tenant_id: None,
            tenant_id_like: None,
            without_tenant_id: false,
            parent_deployment_id: None,
            parent_deployment_id_like: None,
            parent_deployment_ids: Vec::new(),
            latest: false,
            resource_name: None,
            sort_field: None,
            sort_direction: SortDirection::default(),
            start: 0,
            size: None,
        }
    }

    pub fn id(mut self, id: impl Into<String>) -> Self {
        self.id = Some(id.into());
        self
    }

    pub fn ids(mut self, ids: Vec<String>) -> Self {
        self.ids = ids;
        self
    }

    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    pub fn name_like(mut self, name_like: impl Into<String>) -> Self {
        self.name_like = Some(name_like.into());
        self
    }

    pub fn category(mut self, category: impl Into<String>) -> Self {
        self.category = Some(category.into());
        self
    }

    pub fn category_not_equals(mut self, category: impl Into<String>) -> Self {
        self.category_not_equals = Some(category.into());
        self
    }

    pub fn key(mut self, key: impl Into<String>) -> Self {
        self.key = Some(key.into());
        self
    }

    pub fn tenant_id(mut self, tenant_id: impl Into<String>) -> Self {
        self.tenant_id = Some(tenant_id.into());
        self
    }

    pub fn tenant_id_like(mut self, tenant_id_like: impl Into<String>) -> Self {
        self.tenant_id_like = Some(tenant_id_like.into());
        self
    }

    pub fn without_tenant_id(mut self) -> Self {
        self.without_tenant_id = true;
        self
    }

    pub fn parent_deployment_id(mut self, parent_deployment_id: impl Into<String>) -> Self {
        self.parent_deployment_id = Some(parent_deployment_id.into());
        self
    }

    pub fn parent_deployment_id_like(
        mut self,
        parent_deployment_id_like: impl Into<String>,
    ) -> Self {
        self.parent_deployment_id_like = Some(parent_deployment_id_like.into());
        self
    }

    pub fn parent_deployment_ids(mut self, parent_deployment_ids: Vec<String>) -> Self {
        self.parent_deployment_ids = parent_deployment_ids;
        self
    }

    /// Selects the deployment whose `deployed_at` is the latest for the given
    /// key. Java contract: "Can only be used together with the deployment key."
    pub fn latest(mut self) -> Self {
        self.latest = true;
        self
    }

    pub fn resource_name(mut self, resource_name: impl Into<String>) -> Self {
        self.resource_name = Some(resource_name.into());
        self
    }

    pub fn order_by(mut self, field: DeploymentSortField, direction: SortDirection) -> Self {
        self.sort_field = Some(field);
        self.sort_direction = direction;
        self
    }

    pub fn page(mut self, start: usize, size: usize) -> Self {
        self.start = start;
        self.size = Some(size);
        self
    }

    fn build_filters(&self) -> Vec<(String, FilterOp)> {
        let mut filters = Vec::new();
        if let Some(id) = &self.id {
            filters.push(("ID_".to_string(), FilterOp::Eq(id.clone())));
        }
        if !self.ids.is_empty() {
            filters.push(("ID_".to_string(), FilterOp::In(self.ids.clone())));
        }
        if let Some(name) = &self.name {
            filters.push(("NAME_".to_string(), FilterOp::Eq(name.clone())));
        }
        if let Some(name_like) = &self.name_like {
            filters.push(("NAME_".to_string(), FilterOp::Like(name_like.clone())));
        }
        if let Some(category) = &self.category {
            filters.push(("CATEGORY_".to_string(), FilterOp::Eq(category.clone())));
        }
        if let Some(category_neq) = &self.category_not_equals {
            filters.push(("CATEGORY_".to_string(), FilterOp::Neq(category_neq.clone())));
        }
        if let Some(key) = &self.key {
            filters.push(("KEY_".to_string(), FilterOp::Eq(key.clone())));
        }
        if let Some(tenant_id) = &self.tenant_id {
            filters.push(("TENANT_ID_".to_string(), FilterOp::Eq(tenant_id.clone())));
        }
        if let Some(tenant_id_like) = &self.tenant_id_like {
            filters.push((
                "TENANT_ID_".to_string(),
                FilterOp::Like(tenant_id_like.clone()),
            ));
        }
        if self.without_tenant_id {
            filters.push(("TENANT_ID_".to_string(), FilterOp::IsNull));
        }
        if let Some(parent) = &self.parent_deployment_id {
            filters.push((
                "PARENT_DEPLOYMENT_ID_".to_string(),
                FilterOp::Eq(parent.clone()),
            ));
        }
        if let Some(parent_like) = &self.parent_deployment_id_like {
            filters.push((
                "PARENT_DEPLOYMENT_ID_".to_string(),
                FilterOp::Like(parent_like.clone()),
            ));
        }
        if !self.parent_deployment_ids.is_empty() {
            filters.push((
                "PARENT_DEPLOYMENT_ID_".to_string(),
                FilterOp::In(self.parent_deployment_ids.clone()),
            ));
        }
        filters
    }

    fn resolve_latest_filter(
        &self,
        session: &mut DbSession,
    ) -> Result<Option<FilterOp>, CmmnError> {
        if !self.latest {
            return Ok(None);
        }
        let key = self
            .key
            .as_deref()
            .ok_or_else(|| CmmnError::validation("latest() requires a deployment key to be set"))?;
        let dialect = session.dialect();
        let sql = format!(
            "SELECT MAX(DEPLOYED_AT_) AS MAX_AT FROM ACT_CMMN_DEPLOYMENT WHERE KEY_ = {}",
            dialect.placeholder(0)
        );
        let mut params = DbParams::new();
        params.push(key);
        let row = session
            .select_one_raw(RenderedStatement::new(sql, params))
            .map_err(|e| CmmnError::storage(format!("latest deployment lookup failed: {e}")))?;
        let max_at = row
            .and_then(|r| r.get_text("MAX_AT"))
            .ok_or_else(|| CmmnError::not_found("no deployment found for latest() key"))?;
        Ok(Some(FilterOp::Eq(max_at)))
    }

    pub fn list(&self) -> Result<Vec<CmmnDeployment>, CmmnError> {
        let mut session = self.store.create_session()?;
        let mut filters = self.build_filters();
        if let Some(latest_filter) = self.resolve_latest_filter(&mut session)? {
            filters.push(("DEPLOYED_AT_".to_string(), latest_filter));
        }

        let (order_col, ascending) = match self.sort_field {
            Some(field) => (field.column(), self.sort_direction == SortDirection::Asc),
            None => ("DEPLOYED_AT_", true),
        };

        let rows = session
            .filter_query_data(
                "ACT_CMMN_DEPLOYMENT",
                &filters,
                order_col,
                ascending,
                None,
                None,
            )
            .map_err(|e| CmmnError::storage(format!("deployment query failed: {e}")))?;

        let mut deployments: Vec<CmmnDeployment> = rows
            .iter()
            .map(|row| {
                CmmnDeploymentEntity::from_row(row)
                    .map_err(|e| PersistenceError::Deserialization(e.to_string()))
                    .and_then(|entity| {
                        deployment_from_entity(entity)
                            .map_err(|e| PersistenceError::Deserialization(e.to_string()))
                    })
            })
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| CmmnError::storage(format!("deployment hydration failed: {e}")))?;

        // Apply deterministic in-memory sort using the requested field as
        // primary key and ID as tie-breaker. This ensures consistent ordering
        // across database backends with different collation rules.
        let sort_field = self.sort_field;
        let direction = self.sort_direction;
        deployments.sort_by(|a, b| {
            let primary = match sort_field {
                Some(DeploymentSortField::Id) => a.id.cmp(&b.id),
                Some(DeploymentSortField::Name) => a.name.cmp(&b.name),
                Some(DeploymentSortField::DeployedTime) => a.deployed_at.cmp(&b.deployed_at),
                Some(DeploymentSortField::TenantId) => a.tenant_id.cmp(&b.tenant_id),
                None => a.deployed_at.cmp(&b.deployed_at),
            };
            if direction == SortDirection::Desc {
                primary.reverse().then(a.id.cmp(&b.id))
            } else {
                primary.then(a.id.cmp(&b.id))
            }
        });

        if let Some(resource_name) = &self.resource_name {
            deployments.retain(|item| item.resource_names.iter().any(|name| name == resource_name));
        }

        Ok(deployments)
    }

    pub fn count(&self) -> Result<i64, CmmnError> {
        let mut session = self.store.create_session()?;
        let mut filters = self.build_filters();
        if let Some(latest_filter) = self.resolve_latest_filter(&mut session)? {
            filters.push(("DEPLOYED_AT_".to_string(), latest_filter));
        }
        let count = session
            .count_query("ACT_CMMN_DEPLOYMENT", &filters)
            .map_err(|e| CmmnError::storage(format!("deployment count failed: {e}")))?;
        Ok(count)
    }

    pub fn single_result(&self) -> Result<Option<CmmnDeployment>, CmmnError> {
        // When post-filters (resource_name) are active, fall back to full list.
        if self.resource_name.is_some() {
            let results = self.list()?;
            if results.len() > 1 {
                return Err(CmmnError::NonUniqueResult {
                    query: "CmmnDeploymentQuery",
                    count: results.len(),
                });
            }
            return Ok(results.into_iter().next());
        }

        // Fetch at most 2 rows to detect multiplicity efficiently.
        let mut session = self.store.create_session()?;
        let mut filters = self.build_filters();
        if let Some(latest_filter) = self.resolve_latest_filter(&mut session)? {
            filters.push(("DEPLOYED_AT_".to_string(), latest_filter));
        }

        let (order_col, ascending) = match self.sort_field {
            Some(field) => (field.column(), self.sort_direction == SortDirection::Asc),
            None => ("DEPLOYED_AT_", true),
        };

        let rows = session
            .filter_query_data(
                "ACT_CMMN_DEPLOYMENT",
                &filters,
                order_col,
                ascending,
                Some(2),
                None,
            )
            .map_err(|e| CmmnError::storage(format!("deployment query failed: {e}")))?;

        let deployments: Vec<CmmnDeployment> = rows
            .iter()
            .map(|row| {
                CmmnDeploymentEntity::from_row(row)
                    .map_err(|e| PersistenceError::Deserialization(e.to_string()))
                    .and_then(|entity| {
                        deployment_from_entity(entity)
                            .map_err(|e| PersistenceError::Deserialization(e.to_string()))
                    })
            })
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| CmmnError::storage(format!("deployment hydration failed: {e}")))?;

        if deployments.len() > 1 {
            return Err(CmmnError::NonUniqueResult {
                query: "CmmnDeploymentQuery",
                count: deployments.len(),
            });
        }
        Ok(deployments.into_iter().next())
    }

    pub fn list_page(&self) -> Result<PagedResult<CmmnDeployment>, CmmnError> {
        Ok(page_items(self.list()?, self.start, self.size))
    }
}

pub struct CmmnCaseDefinitionQuery {
    store: CmmnStore,
    id: Option<String>,
    ids: Vec<String>,
    key: Option<String>,
    key_like: Option<String>,
    category: Option<String>,
    category_like: Option<String>,
    category_not_equals: Option<String>,
    name: Option<String>,
    name_like: Option<String>,
    name_like_ignore_case: Option<String>,
    deployment_id: Option<String>,
    deployment_ids: Vec<String>,
    parent_deployment_id: Option<String>,
    version: Option<i32>,
    version_gt: Option<i32>,
    version_gte: Option<i32>,
    version_lt: Option<i32>,
    version_lte: Option<i32>,
    latest_version: bool,
    resource_name: Option<String>,
    resource_name_like: Option<String>,
    tenant_id: Option<String>,
    tenant_id_like: Option<String>,
    without_tenant_id: bool,
    sort_field: Option<CaseDefinitionSortField>,
    sort_direction: SortDirection,
    start: usize,
    size: Option<usize>,
    startable_user: Option<String>,
    startable_groups: Vec<String>,
    startable_filter_active: bool,
}

impl CmmnCaseDefinitionQuery {
    fn new(store: CmmnStore) -> Self {
        Self {
            store,
            id: None,
            ids: Vec::new(),
            key: None,
            key_like: None,
            category: None,
            category_like: None,
            category_not_equals: None,
            name: None,
            name_like: None,
            name_like_ignore_case: None,
            deployment_id: None,
            deployment_ids: Vec::new(),
            parent_deployment_id: None,
            version: None,
            version_gt: None,
            version_gte: None,
            version_lt: None,
            version_lte: None,
            latest_version: false,
            resource_name: None,
            resource_name_like: None,
            tenant_id: None,
            tenant_id_like: None,
            without_tenant_id: false,
            sort_field: None,
            sort_direction: SortDirection::default(),
            start: 0,
            size: None,
            startable_user: None,
            startable_groups: Vec::new(),
            startable_filter_active: false,
        }
    }

    pub fn id(mut self, id: impl Into<String>) -> Self {
        self.id = Some(id.into());
        self
    }

    pub fn ids(mut self, ids: Vec<String>) -> Self {
        self.ids = ids;
        self
    }

    pub fn key(mut self, key: impl Into<String>) -> Self {
        self.key = Some(key.into());
        self
    }

    pub fn key_like(mut self, key_like: impl Into<String>) -> Self {
        self.key_like = Some(key_like.into());
        self
    }

    pub fn category(mut self, category: impl Into<String>) -> Self {
        self.category = Some(category.into());
        self
    }

    pub fn category_like(mut self, category_like: impl Into<String>) -> Self {
        self.category_like = Some(category_like.into());
        self
    }

    pub fn category_not_equals(mut self, category: impl Into<String>) -> Self {
        self.category_not_equals = Some(category.into());
        self
    }

    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    pub fn name_like(mut self, name_like: impl Into<String>) -> Self {
        self.name_like = Some(name_like.into());
        self
    }

    pub fn name_like_ignore_case(mut self, name_like: impl Into<String>) -> Self {
        self.name_like_ignore_case = Some(name_like.into());
        self
    }

    pub fn deployment_id(mut self, deployment_id: impl Into<String>) -> Self {
        self.deployment_id = Some(deployment_id.into());
        self
    }

    pub fn deployment_ids(mut self, deployment_ids: Vec<String>) -> Self {
        self.deployment_ids = deployment_ids;
        self
    }

    pub fn parent_deployment_id(mut self, parent_deployment_id: impl Into<String>) -> Self {
        self.parent_deployment_id = Some(parent_deployment_id.into());
        self
    }

    pub fn tenant_id(mut self, tenant_id: impl Into<String>) -> Self {
        self.tenant_id = Some(tenant_id.into());
        self
    }

    pub(crate) fn tenant_id_optional(mut self, tenant_id: Option<&str>) -> Self {
        self.tenant_id = tenant_id.map(str::to_string);
        self
    }

    pub fn tenant_id_like(mut self, tenant_id_like: impl Into<String>) -> Self {
        self.tenant_id_like = Some(tenant_id_like.into());
        self
    }

    pub fn without_tenant_id(mut self) -> Self {
        self.without_tenant_id = true;
        self
    }

    pub fn resource_name(mut self, resource_name: impl Into<String>) -> Self {
        self.resource_name = Some(resource_name.into());
        self
    }

    pub fn resource_name_like(mut self, resource_name_like: impl Into<String>) -> Self {
        self.resource_name_like = Some(resource_name_like.into());
        self
    }

    pub fn version(mut self, version: i32) -> Self {
        self.version = Some(version);
        self
    }

    pub fn version_gt(mut self, version: i32) -> Self {
        self.version_gt = Some(version);
        self
    }

    pub fn version_gte(mut self, version: i32) -> Self {
        self.version_gte = Some(version);
        self
    }

    pub fn version_lt(mut self, version: i32) -> Self {
        self.version_lt = Some(version);
        self
    }

    pub fn version_lte(mut self, version: i32) -> Self {
        self.version_lte = Some(version);
        self
    }

    /// Mirrors Java's `latestVersion()`: keep only the highest-version case
    /// definition per `(key, tenant_id)` pair.
    pub fn latest_version(mut self) -> Self {
        self.latest_version = true;
        self
    }

    pub fn order_by(mut self, field: CaseDefinitionSortField, direction: SortDirection) -> Self {
        self.sort_field = Some(field);
        self.sort_direction = direction;
        self
    }

    pub fn page(mut self, start: usize, size: usize) -> Self {
        self.start = start;
        self.size = Some(size);
        self
    }

    /// Mirrors Java's `startableByUser(userId)`: only definitions that have a
    /// candidate starter identity link for `user_id`.
    pub fn startable_by_user(mut self, user_id: impl Into<String>) -> Self {
        self.startable_user = Some(user_id.into());
        self.startable_groups = Vec::new();
        self.startable_filter_active = true;
        self
    }

    /// Mirrors Java's `startableByUserOrGroups(userId, groupIds)`: definitions
    /// that have a candidate starter link matching the user OR any of the
    /// groups. Pass `None` for user to query by groups only.
    pub fn startable_by_user_or_groups(
        mut self,
        user_id: Option<&str>,
        group_ids: &[&str],
    ) -> Self {
        self.startable_user = user_id.map(str::to_string);
        self.startable_groups = group_ids.iter().map(|s| (*s).to_string()).collect();
        self.startable_filter_active = true;
        self
    }

    fn build_filters(&self) -> Vec<(String, FilterOp)> {
        let mut filters = Vec::new();
        if let Some(id) = &self.id {
            filters.push(("ID_".to_string(), FilterOp::Eq(id.clone())));
        }
        if !self.ids.is_empty() {
            filters.push(("ID_".to_string(), FilterOp::In(self.ids.clone())));
        }
        if let Some(key) = &self.key {
            filters.push(("CASE_KEY_".to_string(), FilterOp::Eq(key.clone())));
        }
        if let Some(key_like) = &self.key_like {
            filters.push(("CASE_KEY_".to_string(), FilterOp::Like(key_like.clone())));
        }
        if let Some(category) = &self.category {
            filters.push(("CATEGORY_".to_string(), FilterOp::Eq(category.clone())));
        }
        if let Some(category_like) = &self.category_like {
            filters.push((
                "CATEGORY_".to_string(),
                FilterOp::Like(category_like.clone()),
            ));
        }
        if let Some(category_neq) = &self.category_not_equals {
            filters.push(("CATEGORY_".to_string(), FilterOp::Neq(category_neq.clone())));
        }
        if let Some(deployment_id) = &self.deployment_id {
            filters.push((
                "DEPLOYMENT_ID_".to_string(),
                FilterOp::Eq(deployment_id.clone()),
            ));
        }
        if !self.deployment_ids.is_empty() {
            filters.push((
                "DEPLOYMENT_ID_".to_string(),
                FilterOp::In(self.deployment_ids.clone()),
            ));
        }
        if let Some(tenant_id) = &self.tenant_id {
            filters.push(("TENANT_ID_".to_string(), FilterOp::Eq(tenant_id.clone())));
        }
        if let Some(tenant_id_like) = &self.tenant_id_like {
            filters.push((
                "TENANT_ID_".to_string(),
                FilterOp::Like(tenant_id_like.clone()),
            ));
        }
        if self.without_tenant_id {
            filters.push(("TENANT_ID_".to_string(), FilterOp::IsNull));
        }
        if let Some(resource_name) = &self.resource_name {
            filters.push((
                "RESOURCE_NAME_".to_string(),
                FilterOp::Eq(resource_name.clone()),
            ));
        }
        if let Some(resource_name_like) = &self.resource_name_like {
            filters.push((
                "RESOURCE_NAME_".to_string(),
                FilterOp::Like(resource_name_like.clone()),
            ));
        }
        if let Some(version) = self.version {
            filters.push(("VERSION_".to_string(), FilterOp::Eq(version.to_string())));
        }
        if let Some(version_gt) = self.version_gt {
            filters.push(("VERSION_".to_string(), FilterOp::GtInt(version_gt as i64)));
        }
        if let Some(version_gte) = self.version_gte {
            filters.push(("VERSION_".to_string(), FilterOp::GeInt(version_gte as i64)));
        }
        if let Some(version_lt) = self.version_lt {
            filters.push(("VERSION_".to_string(), FilterOp::LtInt(version_lt as i64)));
        }
        if let Some(version_lte) = self.version_lte {
            filters.push(("VERSION_".to_string(), FilterOp::LeInt(version_lte as i64)));
        }
        filters
    }

    /// Resolves `parent_deployment_id` into a concrete `DEPLOYMENT_ID_`
    /// IN-list by querying `ACT_CMMN_DEPLOYMENT`. Returns `None` when no
    /// parent filter is active. Returns `Some(In(empty))` when the parent
    /// matches no deployments, which yields an empty result set.
    fn resolve_parent_deployment_filter(
        &self,
        session: &mut DbSession,
    ) -> Result<Option<FilterOp>, CmmnError> {
        let Some(parent_id) = &self.parent_deployment_id else {
            return Ok(None);
        };
        let parent_filters = vec![(
            "PARENT_DEPLOYMENT_ID_".to_string(),
            FilterOp::Eq(parent_id.clone()),
        )];
        let rows = session
            .filter_query_data(
                "ACT_CMMN_DEPLOYMENT",
                &parent_filters,
                "ID_",
                true,
                None,
                None,
            )
            .map_err(|e| CmmnError::storage(format!("parent deployment lookup failed: {e}")))?;
        let deployment_ids: Vec<String> =
            rows.iter().filter_map(|row| row.get_text("ID_")).collect();
        Ok(Some(FilterOp::In(deployment_ids)))
    }

    /// Resolves `startable_by_user` / `startable_by_user_or_groups` into a
    /// concrete list of matching case definition IDs by querying
    /// `ACT_CMMN_IDENTITY_LINK` for candidate-starter links.
    ///
    /// Returns `None` when no starter filter is active.
    /// Returns `Some(vec)` when a starter filter is active; `vec` may be empty
    /// if no definitions match, in which case callers must short-circuit to
    /// an empty result to avoid invalid `IN ()` SQL.
    fn resolve_starter_filter(
        &self,
        session: &mut DbSession,
    ) -> Result<Option<Vec<String>>, CmmnError> {
        if !self.startable_filter_active {
            return Ok(None);
        }

        let mut definition_ids: BTreeSet<String> = BTreeSet::new();
        let base_filters = vec![
            (
                "SCOPE_TYPE_".to_string(),
                FilterOp::Eq("definition".to_string()),
            ),
            (
                "LINK_TYPE_".to_string(),
                FilterOp::Eq("candidate".to_string()),
            ),
        ];

        // Query by user_id
        if let Some(user) = &self.startable_user {
            let mut filters = base_filters.clone();
            filters.push(("USER_ID_".to_string(), FilterOp::Eq(user.clone())));
            let rows = session
                .filter_query_data(
                    "ACT_CMMN_IDENTITY_LINK",
                    &filters,
                    "SCOPE_ID_",
                    true,
                    None,
                    None,
                )
                .map_err(|e| CmmnError::storage(format!("starter user lookup failed: {e}")))?;
            for row in &rows {
                if let Some(id) = row.get_text("SCOPE_ID_") {
                    definition_ids.insert(id);
                }
            }
        }

        // Query by group_ids
        if !self.startable_groups.is_empty() {
            let mut filters = base_filters.clone();
            filters.push((
                "GROUP_ID_".to_string(),
                FilterOp::In(self.startable_groups.clone()),
            ));
            let rows = session
                .filter_query_data(
                    "ACT_CMMN_IDENTITY_LINK",
                    &filters,
                    "SCOPE_ID_",
                    true,
                    None,
                    None,
                )
                .map_err(|e| CmmnError::storage(format!("starter group lookup failed: {e}")))?;
            for row in &rows {
                if let Some(id) = row.get_text("SCOPE_ID_") {
                    definition_ids.insert(id);
                }
            }
        }

        Ok(Some(definition_ids.into_iter().collect()))
    }

    /// Returns true when in-memory post-filters (name, latest_version) are
    /// active and cannot be pushed down to SQL.
    fn has_post_filters(&self) -> bool {
        self.name.is_some()
            || self.name_like.is_some()
            || self.name_like_ignore_case.is_some()
            || self.latest_version
    }

    pub fn list(&self) -> Result<Vec<CmmnCaseDefinition>, CmmnError> {
        let mut session = self.store.create_session()?;
        let mut filters = self.build_filters();
        if let Some(parent_filter) = self.resolve_parent_deployment_filter(&mut session)? {
            filters.push(("DEPLOYMENT_ID_".to_string(), parent_filter));
        }
        if let Some(starter_ids) = self.resolve_starter_filter(&mut session)? {
            if starter_ids.is_empty() {
                return Ok(Vec::new());
            }
            filters.push(("ID_".to_string(), FilterOp::In(starter_ids)));
        }

        let (order_col, ascending) = match self.sort_field {
            Some(field) => (field.column(), self.sort_direction == SortDirection::Asc),
            None => ("CASE_KEY_", true),
        };

        let rows = session
            .filter_query_data(
                "ACT_CMMN_CASE_DEFINITION",
                &filters,
                order_col,
                ascending,
                None,
                None,
            )
            .map_err(|e| CmmnError::storage(format!("case definition query failed: {e}")))?;

        let mut definitions: Vec<CmmnCaseDefinition> = rows
            .iter()
            .map(|row| {
                CmmnCaseDefinitionEntity::from_row(row)
                    .map_err(|e| PersistenceError::Deserialization(e.to_string()))
                    .and_then(|entity| {
                        case_definition_from_entity(entity)
                            .map_err(|e| PersistenceError::Deserialization(e.to_string()))
                    })
            })
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| CmmnError::storage(format!("case definition hydration failed: {e}")))?;

        // Post-filters for fields stored inside DATA_ JSON (no dedicated column).
        if let Some(name) = &self.name {
            definitions.retain(|item| &item.name == name);
        }
        if let Some(name_like) = &self.name_like {
            definitions.retain(|item| like_match(&item.name, name_like));
        }
        if let Some(name_like_icase) = &self.name_like_ignore_case {
            let lower_pattern = name_like_icase.to_ascii_lowercase();
            definitions.retain(|item| like_match(&item.name.to_ascii_lowercase(), &lower_pattern));
        }

        if self.latest_version {
            retain_latest_version_per_key_and_tenant(&mut definitions);
        }

        // Deterministic ordering: key asc, version desc, id asc.
        definitions.sort_by(|left, right| {
            left.key
                .cmp(&right.key)
                .then_with(|| right.version.cmp(&left.version))
                .then_with(|| left.id.cmp(&right.id))
        });

        Ok(definitions)
    }

    pub fn count(&self) -> Result<i64, CmmnError> {
        // When post-filters are active (name, latest_version), the count must
        // be computed in memory because those fields are not SQL columns.
        if self.has_post_filters() {
            return Ok(self.list()?.len() as i64);
        }
        let mut session = self.store.create_session()?;
        let mut filters = self.build_filters();
        if let Some(parent_filter) = self.resolve_parent_deployment_filter(&mut session)? {
            filters.push(("DEPLOYMENT_ID_".to_string(), parent_filter));
        }
        if let Some(starter_ids) = self.resolve_starter_filter(&mut session)? {
            if starter_ids.is_empty() {
                return Ok(0);
            }
            filters.push(("ID_".to_string(), FilterOp::In(starter_ids)));
        }
        let count = session
            .count_query("ACT_CMMN_CASE_DEFINITION", &filters)
            .map_err(|e| CmmnError::storage(format!("case definition count failed: {e}")))?;
        Ok(count)
    }

    pub fn single_result(&self) -> Result<Option<CmmnCaseDefinition>, CmmnError> {
        // When post-filters (name, latest_version) are active, fall back to
        // full list because those fields are not SQL columns.
        if self.has_post_filters() {
            let results = self.list()?;
            if results.len() > 1 {
                return Err(CmmnError::NonUniqueResult {
                    query: "CmmnCaseDefinitionQuery",
                    count: results.len(),
                });
            }
            return Ok(results.into_iter().next());
        }

        // Fetch at most 2 rows to detect multiplicity efficiently.
        let mut session = self.store.create_session()?;
        let mut filters = self.build_filters();
        if let Some(parent_filter) = self.resolve_parent_deployment_filter(&mut session)? {
            filters.push(("DEPLOYMENT_ID_".to_string(), parent_filter));
        }
        if let Some(starter_ids) = self.resolve_starter_filter(&mut session)? {
            if starter_ids.is_empty() {
                return Ok(None);
            }
            filters.push(("ID_".to_string(), FilterOp::In(starter_ids)));
        }

        let (order_col, ascending) = match self.sort_field {
            Some(field) => (field.column(), self.sort_direction == SortDirection::Asc),
            None => ("CASE_KEY_", true),
        };

        let rows = session
            .filter_query_data(
                "ACT_CMMN_CASE_DEFINITION",
                &filters,
                order_col,
                ascending,
                Some(2),
                None,
            )
            .map_err(|e| CmmnError::storage(format!("case definition query failed: {e}")))?;

        let definitions: Vec<CmmnCaseDefinition> = rows
            .iter()
            .map(|row| {
                CmmnCaseDefinitionEntity::from_row(row)
                    .map_err(|e| PersistenceError::Deserialization(e.to_string()))
                    .and_then(|entity| {
                        case_definition_from_entity(entity)
                            .map_err(|e| PersistenceError::Deserialization(e.to_string()))
                    })
            })
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| CmmnError::storage(format!("case definition hydration failed: {e}")))?;

        if definitions.len() > 1 {
            return Err(CmmnError::NonUniqueResult {
                query: "CmmnCaseDefinitionQuery",
                count: definitions.len(),
            });
        }
        Ok(definitions.into_iter().next())
    }

    pub fn list_page(&self) -> Result<PagedResult<CmmnCaseDefinition>, CmmnError> {
        Ok(page_items(self.list()?, self.start, self.size))
    }
}

/// SQL `LIKE` pattern matching supporting `%` (any sequence) and `_` (single
/// char) wildcards. Case-sensitive, matching Java's `LIKE` semantics.
/// Local signature is `(haystack, pattern)`; shared impl is `(pattern, value)`.
fn like_match(haystack: &str, pattern: &str) -> bool {
    // Delegates to flowable_engine_common::like::sql_like_matches (P143 unified LIKE, O(m)+512 cap).
    flowable_engine_common::like::sql_like_matches(pattern, haystack)
}

/// Keeps only the highest-version definition per `(key, tenant_id)` pair,
/// mirroring Java's `latestVersion()` semantics.
fn retain_latest_version_per_key_and_tenant(definitions: &mut Vec<CmmnCaseDefinition>) {
    let mut best: HashMap<(String, Option<String>), usize> = HashMap::new();
    for (idx, def) in definitions.iter().enumerate() {
        let key = (def.key.clone(), def.tenant_id.clone());
        best.entry(key)
            .and_modify(|prev| {
                if definitions[*prev].version < def.version {
                    *prev = idx;
                }
            })
            .or_insert(idx);
    }
    let kept: HashSet<usize> = best.into_values().collect();
    let mut i = 0;
    definitions.retain(|_| {
        let keep = kept.contains(&i);
        i += 1;
        keep
    });
}

fn find_case_diagram_resource_name(
    resource_names: &[String],
    cmmn_resource_name: &str,
    case_key: &str,
) -> Option<String> {
    let lower_name = cmmn_resource_name.to_ascii_lowercase();
    let base = if lower_name.ends_with(".cmmn.xml") {
        &cmmn_resource_name[..cmmn_resource_name.len() - ".cmmn.xml".len()]
    } else {
        &cmmn_resource_name[..cmmn_resource_name.len() - ".cmmn".len()]
    };
    for suffix in ["png", "jpg", "gif", "svg"] {
        for candidate in [
            format!("{base}{case_key}.{suffix}"),
            format!("{base}{suffix}"),
        ] {
            if resource_names.iter().any(|name| name == &candidate) {
                return Some(candidate);
            }
        }
    }
    None
}

fn validate_deployment_request(request: &CmmnDeploymentRequest) -> Result<(), CmmnError> {
    if request.resources.is_empty() {
        return Err(CmmnError::validation(
            "CMMN deployment resources are required",
        ));
    }

    let mut seen_case_keys = BTreeSet::new();
    for resource in &request.resources {
        if resource.resource_name.trim().is_empty() {
            return Err(CmmnError::validation(
                "CMMN deployment resource name is required",
            ));
        }
        if !is_cmmn_resource(&resource.resource_name) {
            continue;
        }
        validate_model(
            &resource.model,
            &resource.resource_name,
            &mut seen_case_keys,
        )?;
    }

    if seen_case_keys.is_empty() {
        return Err(CmmnError::validation(
            "A CMMN deployment requires at least one CMMN model resource",
        ));
    }

    Ok(())
}

fn validate_model(
    model: &CmmnModel,
    resource_name: &str,
    seen_case_keys: &mut BTreeSet<String>,
) -> Result<(), CmmnError> {
    if model.cases.is_empty() {
        return Err(CmmnError::validation(format!(
            "CMMN resource '{resource_name}' does not contain cases"
        )));
    }

    for case_model in &model.cases {
        validate_case(case_model, resource_name)?;
        if !seen_case_keys.insert(case_model.key.clone()) {
            return Err(CmmnError::validation(format!(
                "CMMN case key '{}' appears multiple times in one deployment",
                case_model.key
            )));
        }
    }

    Ok(())
}

fn validate_case(case_model: &CmmnCase, resource_name: &str) -> Result<(), CmmnError> {
    if case_model.id.trim().is_empty() {
        return Err(CmmnError::validation(format!(
            "CMMN resource '{resource_name}' contains a case without id"
        )));
    }
    if case_model.key.trim().is_empty() {
        return Err(CmmnError::validation(format!(
            "CMMN resource '{resource_name}' contains a case without key"
        )));
    }
    if case_model.name.trim().is_empty() {
        return Err(CmmnError::validation(format!(
            "CMMN case '{}' must declare a name",
            case_model.key
        )));
    }
    if case_model.case_plan_model.id.trim().is_empty() {
        return Err(CmmnError::validation(format!(
            "CMMN case '{}' must declare a casePlanModel id",
            case_model.key
        )));
    }
    if case_model.case_plan_model.name.trim().is_empty() {
        return Err(CmmnError::validation(format!(
            "CMMN case '{}' must declare a casePlanModel name",
            case_model.key
        )));
    }

    let mut global_ids = HashSet::new();
    if !global_ids.insert(case_model.id.clone()) {
        return Err(CmmnError::validation(format!(
            "CMMN case '{}' contains duplicate id '{}'",
            case_model.key, case_model.id
        )));
    }
    if !global_ids.insert(case_model.case_plan_model.id.clone()) {
        return Err(CmmnError::validation(format!(
            "CMMN case '{}' contains duplicate id '{}'",
            case_model.key, case_model.case_plan_model.id
        )));
    }
    validate_container(
        case_model.key.as_str(),
        case_model.case_plan_model.name.as_str(),
        &case_model.case_plan_model.plan_items,
        &case_model.case_plan_model.stages,
        &case_model.case_plan_model.human_tasks,
        &case_model.case_plan_model.decision_tasks,
        &case_model.case_plan_model.process_tasks,
        &case_model.case_plan_model.case_tasks,
        &case_model.case_plan_model.milestones,
        &case_model.case_plan_model.event_listeners,
        &case_model.case_plan_model.sentries,
        &case_model.case_plan_model.planning_tables,
        &mut global_ids,
    )
}

#[allow(clippy::too_many_arguments)]
fn validate_container(
    case_key: &str,
    container_name: &str,
    plan_items: &[CmmnPlanItem],
    stages: &[CmmnStage],
    human_tasks: &[CmmnHumanTask],
    decision_tasks: &[CmmnDecisionTask],
    process_tasks: &[CmmnProcessTask],
    case_tasks: &[CmmnCaseTask],
    milestones: &[CmmnMilestone],
    event_listeners: &[CmmnEventListener],
    sentries: &[CmmnSentry],
    planning_tables: &[CmmnPlanningTable],
    global_ids: &mut HashSet<String>,
) -> Result<(), CmmnError> {
    let stage_ids = stages
        .iter()
        .map(|stage| stage.id.as_str())
        .collect::<HashSet<_>>();
    let task_ids = human_tasks
        .iter()
        .map(|human_task| human_task.id.as_str())
        .collect::<HashSet<_>>();
    let decision_task_ids = decision_tasks
        .iter()
        .map(|decision_task| decision_task.id.as_str())
        .collect::<HashSet<_>>();
    let process_task_ids = process_tasks
        .iter()
        .map(|process_task| process_task.id.as_str())
        .collect::<HashSet<_>>();
    let case_task_ids = case_tasks
        .iter()
        .map(|case_task| case_task.id.as_str())
        .collect::<HashSet<_>>();
    let milestone_ids = milestones
        .iter()
        .map(|milestone| milestone.id.as_str())
        .collect::<HashSet<_>>();
    let event_listener_ids = event_listeners
        .iter()
        .map(|event_listener| event_listener.id.as_str())
        .collect::<HashSet<_>>();
    let event_listener_plan_item_ids = plan_items
        .iter()
        .filter(|plan_item| event_listener_ids.contains(plan_item.definition_ref.as_str()))
        .map(|plan_item| plan_item.id.as_str())
        .collect::<HashSet<_>>();
    let human_task_plan_item_ids = plan_items
        .iter()
        .filter(|plan_item| task_ids.contains(plan_item.definition_ref.as_str()))
        .map(|plan_item| plan_item.id.as_str())
        .collect::<HashSet<_>>();
    let stage_plan_item_ids = plan_items
        .iter()
        .filter(|plan_item| stage_ids.contains(plan_item.definition_ref.as_str()))
        .map(|plan_item| plan_item.id.as_str())
        .collect::<HashSet<_>>();
    let milestone_plan_item_ids = plan_items
        .iter()
        .filter(|plan_item| milestone_ids.contains(plan_item.definition_ref.as_str()))
        .map(|plan_item| plan_item.id.as_str())
        .collect::<HashSet<_>>();
    let plan_item_ids = plan_items
        .iter()
        .map(|plan_item| plan_item.id.as_str())
        .collect::<HashSet<_>>();
    let sentry_ids = sentries
        .iter()
        .map(|sentry| sentry.id.as_str())
        .collect::<HashSet<_>>();

    for stage in stages {
        validate_stage(case_key, stage, global_ids)?;
    }

    for planning_table in planning_tables {
        validate_planning_table(
            case_key,
            container_name,
            planning_table,
            &task_ids,
            global_ids,
        )?;
    }

    for sentry in sentries {
        if sentry.id.trim().is_empty() {
            return Err(CmmnError::validation(format!(
                "CMMN case '{case_key}' contains a sentry without id in '{container_name}'"
            )));
        }
        if !global_ids.insert(sentry.id.clone()) {
            return Err(CmmnError::validation(format!(
                "CMMN case '{case_key}' contains duplicate id '{}'",
                sentry.id
            )));
        }
        // P118: caseFileItemOnPart alone is a valid sentry (CMMN11CaseModel.xsd:1027-1042);
        // runtime evaluates it via handle_case_file_item_on_part.
        if sentry.plan_item_on_parts.is_empty()
            && sentry.case_file_item_on_parts.is_empty()
            && sentry.if_part.is_none()
        {
            return Err(CmmnError::validation(format!(
                "CMMN case '{case_key}' sentry '{}' must declare at least one plan item on-part, case file item on-part, or ifPart",
                sentry.id
            )));
        }
        // In this customized M16 subset, we allow both entry and exit criteria to be ifPart-only.
        for on_part in &sentry.case_file_item_on_parts {
            if on_part.id.trim().is_empty() {
                return Err(CmmnError::validation(format!(
                    "CMMN case '{case_key}' sentry '{}' contains a case file item on-part without id",
                    sentry.id
                )));
            }
            if !global_ids.insert(on_part.id.clone()) {
                return Err(CmmnError::validation(format!(
                    "CMMN case '{case_key}' contains duplicate id '{}'",
                    on_part.id
                )));
            }
            if on_part.case_file_item_ref.trim().is_empty() {
                return Err(CmmnError::validation(format!(
                    "CMMN case '{case_key}' sentry '{}' case file item on-part '{}' is missing sourceRef/case_file_item_ref",
                    sentry.id, on_part.id
                )));
            }
            if !CmmnCaseFileItemOnPart::is_supported_standard_event(&on_part.standard_event) {
                return Err(CmmnError::unsupported(
                    "case file item on-part standard event",
                    format!(
                        "case '{case_key}' sentry '{}' declared case file item standard event '{}', but only create, update, delete, and complete are supported",
                        sentry.id, on_part.standard_event
                    ),
                ));
            }
        }
        for on_part in &sentry.plan_item_on_parts {
            if on_part.id.trim().is_empty() {
                return Err(CmmnError::validation(format!(
                    "CMMN case '{case_key}' sentry '{}' contains a plan item on-part without id",
                    sentry.id
                )));
            }
            if !global_ids.insert(on_part.id.clone()) {
                return Err(CmmnError::validation(format!(
                    "CMMN case '{case_key}' contains duplicate id '{}'",
                    on_part.id
                )));
            }
            if !plan_item_ids.contains(on_part.source_ref.as_str()) {
                return Err(CmmnError::validation(format!(
                    "CMMN case '{case_key}' sentry '{}' references unknown source plan item '{}'",
                    sentry.id, on_part.source_ref
                )));
            }
            if !CmmnPlanItemOnPart::is_supported_standard_event(&on_part.standard_event) {
                return Err(CmmnError::unsupported(
                    "sentry standard event",
                    format!(
                        "case '{case_key}' sentry '{}' declared standard event '{}', but only complete, occur, terminate, start, enable, disable, and exit are supported",
                        sentry.id, on_part.standard_event
                    ),
                ));
            }
            if on_part.standard_event == CmmnPlanItemOnPart::STANDARD_EVENT_OCCUR
                && !event_listener_plan_item_ids.contains(on_part.source_ref.as_str())
                && !milestone_plan_item_ids.contains(on_part.source_ref.as_str())
            {
                return Err(CmmnError::unsupported(
                    "sentry standard event",
                    format!(
                        "case '{case_key}' sentry '{}' declared standard event 'occur' for source '{}', but occur is only supported for event listener and milestone plan items",
                        sentry.id, on_part.source_ref
                    ),
                ));
            }
            if matches!(
                on_part.standard_event.as_str(),
                CmmnPlanItemOnPart::STANDARD_EVENT_ENABLE
                    | CmmnPlanItemOnPart::STANDARD_EVENT_DISABLE
            ) && !human_task_plan_item_ids.contains(on_part.source_ref.as_str())
            {
                return Err(CmmnError::unsupported(
                    "sentry standard event",
                    format!(
                        "case '{case_key}' sentry '{}' declared standard event '{}' for source '{}', but enable and disable are only supported for human task plan items",
                        sentry.id, on_part.standard_event, on_part.source_ref
                    ),
                ));
            }
            if on_part.standard_event == CmmnPlanItemOnPart::STANDARD_EVENT_START
                && !human_task_plan_item_ids.contains(on_part.source_ref.as_str())
                && !stage_plan_item_ids.contains(on_part.source_ref.as_str())
            {
                return Err(CmmnError::unsupported(
                    "sentry standard event",
                    format!(
                        "case '{case_key}' sentry '{}' declared standard event 'start' for source '{}', but start is only supported for human task and stage plan items",
                        sentry.id, on_part.source_ref
                    ),
                ));
            }
            if on_part.standard_event == CmmnPlanItemOnPart::STANDARD_EVENT_EXIT
                && !human_task_plan_item_ids.contains(on_part.source_ref.as_str())
                && !stage_plan_item_ids.contains(on_part.source_ref.as_str())
            {
                return Err(CmmnError::unsupported(
                    "sentry standard event",
                    format!(
                        "case '{case_key}' sentry '{}' declared standard event 'exit' for source '{}', but exit is only supported for human task and stage plan items",
                        sentry.id, on_part.source_ref
                    ),
                ));
            }
        }
        if let Some(condition) = sentry.if_part.as_ref() {
            validate_if_part_condition(case_key, &sentry.id, condition)?;
        }
    }

    for human_task in human_tasks {
        if human_task.id.trim().is_empty() {
            return Err(CmmnError::validation(format!(
                "CMMN case '{case_key}' contains a human task without id in '{container_name}'"
            )));
        }
        if human_task.name.trim().is_empty() {
            return Err(CmmnError::validation(format!(
                "CMMN case '{case_key}' human task '{}' must declare a name",
                human_task.id
            )));
        }
        if !global_ids.insert(human_task.id.clone()) {
            return Err(CmmnError::validation(format!(
                "CMMN case '{case_key}' contains duplicate id '{}'",
                human_task.id
            )));
        }
    }

    for decision_task in decision_tasks {
        if decision_task.id.trim().is_empty() {
            return Err(CmmnError::validation(format!(
                "CMMN case '{case_key}' contains a decision task without id in '{container_name}'"
            )));
        }
        if decision_task.name.trim().is_empty() {
            return Err(CmmnError::validation(format!(
                "CMMN case '{case_key}' decision task '{}' must declare a name",
                decision_task.id
            )));
        }
        if let Some(decision_ref) = decision_task.decision_ref.as_deref()
            && decision_ref.trim().is_empty()
        {
            return Err(CmmnError::validation(format!(
                "CMMN case '{case_key}' decision task '{}' declares an empty decision reference",
                decision_task.id
            )));
        }
        if !global_ids.insert(decision_task.id.clone()) {
            return Err(CmmnError::validation(format!(
                "CMMN case '{case_key}' contains duplicate id '{}'",
                decision_task.id
            )));
        }
    }

    for process_task in process_tasks {
        if process_task.id.trim().is_empty() {
            return Err(CmmnError::validation(format!(
                "CMMN case '{case_key}' contains a process task without id in '{container_name}'"
            )));
        }
        if process_task.name.trim().is_empty() {
            return Err(CmmnError::validation(format!(
                "CMMN case '{case_key}' process task '{}' must declare a name",
                process_task.id
            )));
        }
        match process_task.process_ref.as_deref() {
            Some(process_ref) if !process_ref.trim().is_empty() => {}
            _ => {
                return Err(CmmnError::validation(format!(
                    "CMMN case '{case_key}' process task '{}' declares an empty process reference",
                    process_task.id
                )));
            }
        }
        if !global_ids.insert(process_task.id.clone()) {
            return Err(CmmnError::validation(format!(
                "CMMN case '{case_key}' contains duplicate id '{}'",
                process_task.id
            )));
        }
    }

    for case_task in case_tasks {
        if case_task.id.trim().is_empty() {
            return Err(CmmnError::validation(format!(
                "CMMN case '{case_key}' contains a case task without id in '{container_name}'"
            )));
        }
        if case_task.name.trim().is_empty() {
            return Err(CmmnError::validation(format!(
                "CMMN case '{case_key}' case task '{}' must declare a name",
                case_task.id
            )));
        }
        match case_task.case_ref.as_deref() {
            Some(case_ref) if !case_ref.trim().is_empty() => {}
            _ => {
                return Err(CmmnError::validation(format!(
                    "CMMN case '{case_key}' case task '{}' declares an empty case reference",
                    case_task.id
                )));
            }
        }
        if !global_ids.insert(case_task.id.clone()) {
            return Err(CmmnError::validation(format!(
                "CMMN case '{case_key}' contains duplicate id '{}'",
                case_task.id
            )));
        }
    }

    for milestone in milestones {
        if milestone.id.trim().is_empty() {
            return Err(CmmnError::validation(format!(
                "CMMN case '{case_key}' contains a milestone without id in '{container_name}'"
            )));
        }
        if milestone.name.trim().is_empty() {
            return Err(CmmnError::validation(format!(
                "CMMN case '{case_key}' milestone '{}' must declare a name",
                milestone.id
            )));
        }
        if !global_ids.insert(milestone.id.clone()) {
            return Err(CmmnError::validation(format!(
                "CMMN case '{case_key}' contains duplicate id '{}'",
                milestone.id
            )));
        }
    }

    for event_listener in event_listeners {
        if event_listener.id.trim().is_empty() {
            return Err(CmmnError::validation(format!(
                "CMMN case '{case_key}' contains an event listener without id in '{container_name}'"
            )));
        }
        if event_listener.event_type.trim().is_empty() {
            return Err(CmmnError::validation(format!(
                "CMMN case '{case_key}' event listener '{}' must declare an event type",
                event_listener.id
            )));
        }
        if let Some(event_name) = event_listener.event_name.as_deref()
            && event_name.trim().is_empty()
        {
            return Err(CmmnError::validation(format!(
                "CMMN case '{case_key}' event listener '{}' declares an empty event name",
                event_listener.id
            )));
        }
        if !global_ids.insert(event_listener.id.clone()) {
            return Err(CmmnError::validation(format!(
                "CMMN case '{case_key}' contains duplicate id '{}'",
                event_listener.id
            )));
        }
    }

    for plan_item in plan_items {
        if plan_item.id.trim().is_empty() {
            return Err(CmmnError::validation(format!(
                "CMMN case '{case_key}' contains a plan item without id in '{container_name}'"
            )));
        }
        if !global_ids.insert(plan_item.id.clone()) {
            return Err(CmmnError::validation(format!(
                "CMMN case '{case_key}' contains duplicate id '{}'",
                plan_item.id
            )));
        }
        for criterion_id in &plan_item.entry_criterion_ids {
            if criterion_id.trim().is_empty() {
                return Err(CmmnError::validation(format!(
                    "CMMN case '{case_key}' plan item '{}' declares an empty entry criterion reference",
                    plan_item.id
                )));
            }
            if !sentry_ids.contains(criterion_id.as_str()) {
                return Err(CmmnError::validation(format!(
                    "CMMN case '{case_key}' plan item '{}' references unknown entry criterion sentry '{}'",
                    plan_item.id, criterion_id
                )));
            }
        }
        for criterion_id in &plan_item.exit_criterion_ids {
            if criterion_id.trim().is_empty() {
                return Err(CmmnError::validation(format!(
                    "CMMN case '{case_key}' plan item '{}' declares an empty exit criterion reference",
                    plan_item.id
                )));
            }
            if !sentry_ids.contains(criterion_id.as_str()) {
                return Err(CmmnError::validation(format!(
                    "CMMN case '{case_key}' plan item '{}' references unknown exit criterion sentry '{}'",
                    plan_item.id, criterion_id
                )));
            }
        }
        let Some(definition_type) = plan_item_definition_type(
            plan_item.definition_ref.as_str(),
            &stage_ids,
            &task_ids,
            &decision_task_ids,
            &process_task_ids,
            &case_task_ids,
            &milestone_ids,
            &event_listener_ids,
        ) else {
            return Err(CmmnError::validation(format!(
                "CMMN case '{case_key}' plan item '{}' references unknown definition '{}'",
                plan_item.id, plan_item.definition_ref
            )));
        };
        validate_plan_item_control_rules(case_key, plan_item, definition_type)?;
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn plan_item_definition_type(
    definition_ref: &str,
    stage_ids: &HashSet<&str>,
    task_ids: &HashSet<&str>,
    decision_task_ids: &HashSet<&str>,
    process_task_ids: &HashSet<&str>,
    case_task_ids: &HashSet<&str>,
    milestone_ids: &HashSet<&str>,
    event_listener_ids: &HashSet<&str>,
) -> Option<&'static str> {
    if task_ids.contains(definition_ref) {
        Some("humanTask")
    } else if stage_ids.contains(definition_ref) {
        Some("stage")
    } else if decision_task_ids.contains(definition_ref) {
        Some("decisionTask")
    } else if process_task_ids.contains(definition_ref) {
        Some("processTask")
    } else if case_task_ids.contains(definition_ref) {
        Some("caseTask")
    } else if milestone_ids.contains(definition_ref) {
        Some("milestone")
    } else if event_listener_ids.contains(definition_ref) {
        Some("eventListener")
    } else {
        None
    }
}

fn validate_plan_item_control_rules(
    case_key: &str,
    plan_item: &CmmnPlanItem,
    definition_type: &str,
) -> Result<(), CmmnError> {
    // manualActivationRule is now supported on all types.
    // repetitionRule is supported on humanTask, stage, and decisionTask.
    if plan_item.repetition_rule.is_some()
        && !matches!(definition_type, "humanTask" | "stage" | "decisionTask")
    {
        return Err(CmmnError::unsupported(
            "plan item control",
            format!(
                "case '{case_key}' plan item '{}' references a {definition_type} definition '{}', but repetitionRule is outside the supported bounded subset",
                plan_item.id, plan_item.definition_ref
            ),
        ));
    }
    // requiredRule is supported on all types; validate the expression if present.
    if let Some(ref required_rule) = plan_item.required_rule {
        validate_if_part_condition(
            case_key,
            &format!("{}-requiredRule", plan_item.id),
            required_rule,
        )?;
    }
    Ok(())
}

fn validate_if_part_condition(
    case_key: &str,
    sentry_id: &str,
    expression: &CmmnSentryIfPartExpression,
) -> Result<(), CmmnError> {
    let condition = match expression {
        CmmnSentryIfPartExpression::Comparison(condition) => condition,
        CmmnSentryIfPartExpression::Logical { operands, .. } => {
            for operand in operands {
                validate_if_part_condition(case_key, sentry_id, operand)?;
            }
            return Ok(());
        }
        CmmnSentryIfPartExpression::Not { operand } => {
            validate_if_part_condition(case_key, sentry_id, operand)?;
            return Ok(());
        }
        CmmnSentryIfPartExpression::Empty { variable_name } => {
            if !is_supported_if_part_variable_name(variable_name) {
                return Err(CmmnError::unsupported(
                    "sentry ifPart",
                    format!(
                        "case '{case_key}' sentry '{sentry_id}' declared ifPart empty variable '{}', but only case variable paths are supported",
                        variable_name
                    ),
                ));
            }
            return Ok(());
        }
        CmmnSentryIfPartExpression::Contains {
            collection_variable_name,
            value,
            ..
        } => {
            if !is_supported_if_part_comparison_left_operand(collection_variable_name) {
                return Err(CmmnError::unsupported(
                    "sentry ifPart",
                    format!(
                        "case '{case_key}' sentry '{sentry_id}' declared ifPart contains collection '{}', but only case variable paths and value expressions are supported",
                        collection_variable_name
                    ),
                ));
            }
            validate_if_part_literal(case_key, sentry_id, value)?;
            return Ok(());
        }
        CmmnSentryIfPartExpression::StartsWith { variable_name, .. } => {
            if !is_supported_if_part_variable_name(variable_name) {
                return Err(CmmnError::unsupported(
                    "sentry ifPart",
                    format!(
                        "case '{case_key}' sentry '{sentry_id}' declared ifPart startsWith variable '{}', but only case variable paths are supported",
                        variable_name
                    ),
                ));
            }
            return Ok(());
        }
        CmmnSentryIfPartExpression::EndsWith { variable_name, .. } => {
            if !is_supported_if_part_variable_name(variable_name) {
                return Err(CmmnError::unsupported(
                    "sentry ifPart",
                    format!(
                        "case '{case_key}' sentry '{sentry_id}' declared ifPart endsWith variable '{}', but only case variable paths are supported",
                        variable_name
                    ),
                ));
            }
            return Ok(());
        }
        CmmnSentryIfPartExpression::Matches { variable_name, .. } => {
            if !is_supported_if_part_variable_name(variable_name) {
                return Err(CmmnError::unsupported(
                    "sentry ifPart",
                    format!(
                        "case '{case_key}' sentry '{sentry_id}' declared ifPart matches variable '{}', but only case variable paths are supported",
                        variable_name
                    ),
                ));
            }
            return Ok(());
        }
        CmmnSentryIfPartExpression::Size {
            collection_variable_name,
            ..
        } => {
            if !is_supported_if_part_comparison_left_operand(collection_variable_name) {
                return Err(CmmnError::unsupported(
                    "sentry ifPart",
                    format!(
                        "case '{case_key}' sentry '{sentry_id}' declared ifPart size collection '{}', but only case variable paths and value expressions are supported",
                        collection_variable_name
                    ),
                ));
            }
            return Ok(());
        }
        CmmnSentryIfPartExpression::Length { variable_name, .. } => {
            if !is_supported_if_part_comparison_left_operand(variable_name) {
                return Err(CmmnError::unsupported(
                    "sentry ifPart",
                    format!(
                        "case '{case_key}' sentry '{sentry_id}' declared ifPart length variable '{}', but only case variable paths and value expressions are supported",
                        variable_name
                    ),
                ));
            }
            return Ok(());
        }
        CmmnSentryIfPartExpression::MethodCall { object, args, .. } => {
            if let Some(obj) = object
                && !is_supported_if_part_variable_name(obj)
            {
                return Err(CmmnError::unsupported(
                    "sentry ifPart",
                    format!(
                        "case '{case_key}' sentry '{sentry_id}' declared ifPart method object '{}', but only case variable paths are supported",
                        obj
                    ),
                ));
            }
            for arg in args {
                validate_if_part_condition(case_key, sentry_id, arg)?;
            }
            return Ok(());
        }
        CmmnSentryIfPartExpression::Arithmetic { left, right, .. } => {
            validate_if_part_condition(case_key, sentry_id, left)?;
            validate_if_part_condition(case_key, sentry_id, right)?;
            return Ok(());
        }
        CmmnSentryIfPartExpression::Ternary {
            condition,
            true_expr,
            false_expr,
        } => {
            validate_if_part_condition(case_key, sentry_id, condition)?;
            validate_if_part_condition(case_key, sentry_id, true_expr)?;
            validate_if_part_condition(case_key, sentry_id, false_expr)?;
            return Ok(());
        }
        CmmnSentryIfPartExpression::PropertyAccess { object, .. } => {
            validate_if_part_condition(case_key, sentry_id, object)?;
            return Ok(());
        }
        CmmnSentryIfPartExpression::IndexAccess { object, index } => {
            validate_if_part_condition(case_key, sentry_id, object)?;
            validate_if_part_condition(case_key, sentry_id, index)?;
            return Ok(());
        }
        CmmnSentryIfPartExpression::Literal(lit) => {
            validate_if_part_literal(case_key, sentry_id, lit)?;
            return Ok(());
        }
    };

    if !is_supported_if_part_comparison_left_operand(&condition.variable_name) {
        return Err(CmmnError::unsupported(
            "sentry ifPart",
            format!(
                "case '{case_key}' sentry '{sentry_id}' declared ifPart variable '{}', but only case variable paths, size(path), and length(path) are supported",
                condition.variable_name
            ),
        ));
    }

    validate_if_part_literal(case_key, sentry_id, &condition.literal)?;

    Ok(())
}

fn validate_if_part_literal(
    case_key: &str,
    sentry_id: &str,
    literal: &CmmnSentryIfPartLiteral,
) -> Result<(), CmmnError> {
    match literal {
        CmmnSentryIfPartLiteral::Number(number) => {
            if !is_supported_number_literal(number) {
                return Err(CmmnError::unsupported(
                    "sentry ifPart",
                    format!(
                        "case '{case_key}' sentry '{sentry_id}' declared ifPart number literal '{number}', but only integer or decimal number literals are supported"
                    ),
                ));
            }
        }
        CmmnSentryIfPartLiteral::Variable(variable_name) => {
            if !is_supported_if_part_comparison_left_operand(variable_name) {
                return Err(CmmnError::unsupported(
                    "sentry ifPart",
                    format!(
                        "case '{case_key}' sentry '{sentry_id}' declared ifPart right-hand variable '{}', but only case variable paths and value expressions are supported",
                        variable_name
                    ),
                ));
            }
        }
        CmmnSentryIfPartLiteral::Boolean(_)
        | CmmnSentryIfPartLiteral::String(_)
        | CmmnSentryIfPartLiteral::Null => {}
    }

    Ok(())
}

fn is_supported_if_part_comparison_left_operand(value: &str) -> bool {
    if flowable_cmmn_model::parse_sentry_value_expression(value).is_ok() {
        return true;
    }

    if let Some(variable_name) = supported_if_part_sizing_operand(value) {
        return is_supported_if_part_variable_name(variable_name);
    }

    is_supported_if_part_variable_name(value)
}

fn supported_if_part_sizing_operand(value: &str) -> Option<&str> {
    value
        .strip_prefix("size(")
        .or_else(|| value.strip_prefix("length("))
        .and_then(|value| value.strip_suffix(')'))
}

fn is_supported_if_part_variable_name(value: &str) -> bool {
    let bytes = value.as_bytes();
    let Some(first) = bytes.first() else {
        return false;
    };
    if !is_if_part_identifier_start_byte(*first) {
        return false;
    }

    let mut index = 1;
    while index < bytes.len() && is_if_part_identifier_byte(bytes[index]) {
        index += 1;
    }

    while index < bytes.len() {
        match bytes[index] {
            b'.' => {
                index += 1;
                if !bytes
                    .get(index)
                    .is_some_and(|candidate| is_if_part_identifier_start_byte(*candidate))
                {
                    return false;
                }
                index += 1;
                while index < bytes.len() && is_if_part_identifier_byte(bytes[index]) {
                    index += 1;
                }
            }
            b'[' => {
                index += 1;
                match bytes.get(index) {
                    Some(candidate) if candidate.is_ascii_digit() => {
                        while index < bytes.len() && bytes[index].is_ascii_digit() {
                            index += 1;
                        }
                        if bytes.get(index) != Some(&b']') {
                            return false;
                        }
                        index += 1;
                    }
                    Some(quote @ (b'\'' | b'"')) => {
                        let quote = *quote;
                        index += 1;
                        let key_start = index;
                        while index < bytes.len() && bytes[index] != quote {
                            if bytes[index] == b'\\' {
                                return false;
                            }
                            index += 1;
                        }
                        if key_start == index
                            || bytes.get(index) != Some(&quote)
                            || bytes.get(index + 1) != Some(&b']')
                        {
                            return false;
                        }
                        index += 2;
                    }
                    _ => return false,
                }
            }
            _ => return false,
        }
    }

    true
}

fn is_if_part_identifier_start_byte(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_'
}

fn is_if_part_identifier_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

fn validate_stage(
    case_key: &str,
    stage: &CmmnStage,
    global_ids: &mut HashSet<String>,
) -> Result<(), CmmnError> {
    if stage.id.trim().is_empty() {
        return Err(CmmnError::validation(format!(
            "CMMN case '{case_key}' contains a stage without id"
        )));
    }
    if stage.name.trim().is_empty() {
        return Err(CmmnError::validation(format!(
            "CMMN case '{case_key}' stage '{}' must declare a name",
            stage.id
        )));
    }
    if !global_ids.insert(stage.id.clone()) {
        return Err(CmmnError::validation(format!(
            "CMMN case '{case_key}' contains duplicate id '{}'",
            stage.id
        )));
    }

    validate_container(
        case_key,
        stage.name.as_str(),
        &stage.plan_items,
        &stage.stages,
        &stage.human_tasks,
        &stage.decision_tasks,
        &stage.process_tasks,
        &stage.case_tasks,
        &stage.milestones,
        &stage.event_listeners,
        &stage.sentries,
        &stage.planning_tables,
        global_ids,
    )
}

fn validate_planning_table(
    case_key: &str,
    container_name: &str,
    planning_table: &CmmnPlanningTable,
    human_task_ids: &HashSet<&str>,
    global_ids: &mut HashSet<String>,
) -> Result<(), CmmnError> {
    if planning_table.id.trim().is_empty() {
        return Err(CmmnError::validation(format!(
            "CMMN case '{case_key}' contains a planning table without id in '{container_name}'"
        )));
    }
    if planning_table.name.trim().is_empty() {
        return Err(CmmnError::validation(format!(
            "CMMN case '{case_key}' planning table '{}' must declare a name",
            planning_table.id
        )));
    }
    if !global_ids.insert(planning_table.id.clone()) {
        return Err(CmmnError::validation(format!(
            "CMMN case '{case_key}' contains duplicate id '{}'",
            planning_table.id
        )));
    }

    for discretionary_item in &planning_table.discretionary_items {
        if discretionary_item.id.trim().is_empty() {
            return Err(CmmnError::validation(format!(
                "CMMN case '{case_key}' planning table '{}' contains a discretionary item without id",
                planning_table.id
            )));
        }
        if discretionary_item.name.trim().is_empty() {
            return Err(CmmnError::validation(format!(
                "CMMN case '{case_key}' discretionary item '{}' must declare a name",
                discretionary_item.id
            )));
        }
        if discretionary_item.definition_ref.trim().is_empty() {
            return Err(CmmnError::validation(format!(
                "CMMN case '{case_key}' discretionary item '{}' declares an empty definition reference",
                discretionary_item.id
            )));
        }
        if !human_task_ids.contains(discretionary_item.definition_ref.as_str()) {
            return Err(CmmnError::validation(format!(
                "CMMN case '{case_key}' discretionary item '{}' references unknown human task definition '{}'",
                discretionary_item.id, discretionary_item.definition_ref
            )));
        }
        if let Some(item_planning_table) = discretionary_item.planning_table.as_deref()
            && item_planning_table != planning_table.id
        {
            return Err(CmmnError::validation(format!(
                "CMMN case '{case_key}' discretionary item '{}' declares planning table '{}', expected '{}'",
                discretionary_item.id, item_planning_table, planning_table.id
            )));
        }
        if !global_ids.insert(discretionary_item.id.clone()) {
            return Err(CmmnError::validation(format!(
                "CMMN case '{case_key}' contains duplicate id '{}'",
                discretionary_item.id
            )));
        }
    }

    Ok(())
}

fn case_definition_decision_keys(case_model: &CmmnCase) -> Vec<String> {
    let mut keys = BTreeSet::new();
    collect_decision_keys_from_container(
        &case_model.case_plan_model.decision_tasks,
        &case_model.case_plan_model.stages,
        &mut keys,
    );
    keys.into_iter().collect()
}

fn collect_decision_keys_from_container(
    decision_tasks: &[CmmnDecisionTask],
    stages: &[CmmnStage],
    keys: &mut BTreeSet<String>,
) {
    for decision_task in decision_tasks {
        insert_non_empty(decision_task.decision_ref.as_deref(), keys);
    }
    for stage in stages {
        collect_decision_keys_from_container(&stage.decision_tasks, &stage.stages, keys);
    }
}

fn case_definition_form_keys(case_model: &CmmnCase) -> Vec<String> {
    let mut keys = BTreeSet::new();
    insert_non_empty(
        case_model.case_plan_model.start_form_key.as_deref(),
        &mut keys,
    );
    collect_form_keys_from_container(
        &case_model.case_plan_model.human_tasks,
        &case_model.case_plan_model.stages,
        &mut keys,
    );
    keys.into_iter().collect()
}

fn collect_form_keys_from_container(
    human_tasks: &[CmmnHumanTask],
    stages: &[CmmnStage],
    keys: &mut BTreeSet<String>,
) {
    for human_task in human_tasks {
        insert_non_empty(human_task.form_key.as_deref(), keys);
    }
    for stage in stages {
        collect_form_keys_from_container(&stage.human_tasks, &stage.stages, keys);
    }
}

fn insert_non_empty(value: Option<&str>, keys: &mut BTreeSet<String>) {
    if let Some(value) = normalized_optional_key(value) {
        keys.insert(value);
    }
}

fn normalized_optional_key(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn next_version(
    session: &mut DbSession,
    case_key: &str,
    tenant_id: Option<&str>,
) -> Result<i32, CmmnError> {
    let current = CmmnCaseDefinitionDataManager::new()
        .find_by_key(session, case_key)?
        .into_iter()
        .filter(|definition| definition.tenant_id.as_deref() == tenant_id)
        .map(|definition| definition.version)
        .max()
        .unwrap_or(0);
    Ok(current + 1)
}

/// Previous version for the same key+tenant (max version < `current_version`).
fn find_previous_case_definition_session(
    session: &mut DbSession,
    case_key: &str,
    tenant_id: Option<&str>,
    current_version: i32,
) -> Result<Option<CmmnCaseDefinition>, CmmnError> {
    let mut candidates = CmmnCaseDefinitionDataManager::new()
        .find_by_key(session, case_key)?
        .into_iter()
        .filter(|definition| definition.tenant_id.as_deref() == tenant_id)
        .filter(|definition| definition.version < current_version)
        .collect::<Vec<_>>();
    candidates.sort_by_key(|d| d.version);
    candidates
        .pop()
        .map(case_definition_from_entity)
        .transpose()
}

/// Java `CmmnDeployer.updateEventSubscriptions` (CmmnDeployer.java:194-224).
fn update_event_subscriptions_for_case_definition(
    session: &mut DbSession,
    new_definition: &CmmnCaseDefinition,
    previous: Option<&CmmnCaseDefinition>,
) -> Result<(), CmmnError> {
    if let Some(previous) = previous {
        if is_manual_correlation_subscription(&previous.model) {
            // Manual: keep registered subscriptions, retarget scopeDefinitionId
            // (CmmnDeployer.java:200-204, :226-229).
            if let Some(start_event_type) = previous.model.start_event_type.as_deref() {
                update_definition_level_subscription_scope(
                    session,
                    &previous.id,
                    &new_definition.id,
                    start_event_type,
                )?;
            }
        } else {
            // Static: drop old definition-level start subscriptions
            // (CmmnDeployer.java:205-208 — NullScopeId only).
            delete_definition_level_event_subscriptions(session, &previous.id)?;
        }
    }

    // Create new static start subscription when configured
    // (CmmnDeployer.java:211-222).
    if let Some(start_event_type) = new_definition.model.start_event_type.as_deref() {
        if !is_manual_correlation_subscription(&new_definition.model) {
            create_definition_level_start_subscription(
                session,
                new_definition,
                start_event_type,
                start_event_correlation_key(&new_definition.model),
            )?;
        }
    }
    Ok(())
}

/// Java `isManualCorrelationSubscriptionConfiguration` (CmmnDeployer.java:240-248).
fn is_manual_correlation_subscription(case_model: &CmmnCase) -> bool {
    case_model
        .start_correlation_configuration
        .as_deref()
        .is_some_and(|cfg| cfg == START_EVENT_CORRELATION_MANUAL)
}

/// Java `CmmnCorrelationUtil.getCorrelationKey` for case start
/// (CmmnCorrelationUtil.java:29-46) — static name/value, no expression eval.
fn start_event_correlation_key(case_model: &CmmnCase) -> Option<String> {
    if case_model.start_correlation_parameters.is_empty() {
        return None;
    }
    let mut params = std::collections::BTreeMap::new();
    for param in &case_model.start_correlation_parameters {
        params.insert(param.name.clone(), Some(param.value.clone()));
    }
    Some(generate_correlation_key(&params))
}

fn create_definition_level_start_subscription(
    session: &mut DbSession,
    definition: &CmmnCaseDefinition,
    event_type: &str,
    configuration: Option<String>,
) -> Result<(), CmmnError> {
    // Align field semantics with instance-level event-registry subscriptions
    // (EventRegistryEventListenerActivityBehaviour.java:146 stores event key as eventType;
    // cmmn_consumer.rs queries event_type by event definition key).
    let subscription = CmmnEventSubscription {
        id: format!("cmmn-event-subscription:{}", Uuid::new_v4()),
        event_type: event_type.to_string(),
        event_name: None,
        activity_id: None,
        case_instance_id: None,
        case_definition_id: Some(definition.id.clone()),
        plan_item_instance_id: None,
        tenant_id: definition.tenant_id.clone(),
        configuration,
        created_at: Utc::now(),
    };
    persist_event_subscription_in_session(session, &subscription)
}

/// Definition-level only: case_instance_id IS NULL
/// (Java deleteEventSubscriptionsForScopeDefinitionIdAndTypeAndNullScopeId).
fn delete_definition_level_event_subscriptions(
    session: &mut DbSession,
    case_definition_id: &str,
) -> Result<(), CmmnError> {
    let mut params = DbParams::new();
    params.push(case_definition_id);
    session.execute_raw(RenderedStatement::new(
        "DELETE FROM ACT_CMMN_EVENT_SUBSCRIPTION WHERE CASE_DEFINITION_ID_ = ? AND CASE_INSTANCE_ID_ IS NULL".to_string(),
        params,
    ))?;
    Ok(())
}

/// Retarget definition-level subscriptions from old → new scope definition id
/// (Java updateEventSubscriptionScopeDefinitionId).
fn update_definition_level_subscription_scope(
    session: &mut DbSession,
    old_definition_id: &str,
    new_definition_id: &str,
    event_type: &str,
) -> Result<(), CmmnError> {
    let sql = "SELECT DATA_ FROM ACT_CMMN_EVENT_SUBSCRIPTION \
               WHERE CASE_DEFINITION_ID_ = ? AND CASE_INSTANCE_ID_ IS NULL AND EVENT_TYPE_ = ?"
        .to_string();
    let mut params = DbParams::new();
    params.push(old_definition_id);
    params.push(event_type);
    let rows = session.select_raw(RenderedStatement::new(sql, params))?;
    for row in rows {
        let json = row.get_text("DATA_").ok_or_else(|| {
            CmmnError::storage("Missing DATA_ in CMMN event subscription for scope update")
        })?;
        let mut subscription: CmmnEventSubscription =
            serde_json::from_str(&json).map_err(CmmnError::from)?;
        subscription.case_definition_id = Some(new_definition_id.to_string());
        persist_event_subscription_in_session(session, &subscription)?;
    }
    Ok(())
}

fn persist_event_subscription_in_session(
    session: &mut DbSession,
    subscription: &CmmnEventSubscription,
) -> Result<(), CmmnError> {
    let mut params = DbParams::new();
    params.push(subscription.id.as_str());
    params.push(subscription.event_type.as_str());
    params.push(subscription.event_name.clone());
    params.push(subscription.activity_id.clone());
    params.push(subscription.case_instance_id.clone());
    params.push(subscription.case_definition_id.clone());
    params.push(subscription.plan_item_instance_id.clone());
    params.push(subscription.tenant_id.clone());
    params.push(subscription.configuration.clone());
    params.push(subscription.created_at.to_rfc3339());
    params.push(serde_json::to_string(subscription)?);
    session.upsert_raw(
        "ACT_CMMN_EVENT_SUBSCRIPTION",
        "ID_",
        &[
            "ID_",
            "EVENT_TYPE_",
            "EVENT_NAME_",
            "ACTIVITY_ID_",
            "CASE_INSTANCE_ID_",
            "CASE_DEFINITION_ID_",
            "PLAN_ITEM_INSTANCE_ID_",
            "TENANT_ID_",
            "CONFIGURATION_",
            "CREATED_AT_",
            "DATA_",
        ],
        params,
    )?;
    Ok(())
}

/// Java `restorePreviousStartEventsIfNeeded` (CmmnDeploymentEntityManagerImpl.java:81-108).
///
/// After deleting `deleted`, if it was the latest version, rebuild a definition-level
/// start subscription for the new latest (previous version) when that previous version
/// itself has a non-null startEventType.
///
/// Note: Java's implementation reads startEventType from the *deleted* model (:88-92);
/// the surrounding comment says "previous". We use the previous version's own model
/// (comment intent) so rollback restores the correct event type/correlation for that
/// version.
fn restore_previous_start_events_if_needed(
    session: &mut DbSession,
    deleted: &CmmnCaseDefinition,
) -> Result<(), CmmnError> {
    // Was deleted the latest? After removal, find current max version for key+tenant.
    let remaining = CmmnCaseDefinitionDataManager::new()
        .find_by_key(session, &deleted.key)?
        .into_iter()
        .filter(|d| d.tenant_id.as_deref() == deleted.tenant_id.as_deref())
        .collect::<Vec<_>>();

    // Restore only when the deleted definition was the latest version. Selecting an older
    // candidate before checking the remaining maximum would incorrectly restore (for example)
    // v1 when deleting v2 while v3 still exists.
    let Some(previous_entity) = remaining.into_iter().max_by_key(|d| d.version) else {
        return Ok(());
    };
    if previous_entity.version >= deleted.version {
        return Ok(());
    }

    let previous = case_definition_from_entity(previous_entity)?;
    if let Some(start_event_type) = previous.model.start_event_type.as_deref() {
        // Do not auto-create for manualSubscription configs (Java restore always creates
        // if startEventType != null, including manual — CmmnDeploymentEntityManagerImpl.java:91-93.
        // Align with Java: restore whenever startEventType is non-null).
        create_definition_level_start_subscription(
            session,
            &previous,
            start_event_type,
            start_event_correlation_key(&previous.model),
        )?;
    }
    Ok(())
}

fn resource_entity_to_data(entity: CmmnDeploymentResourceEntity) -> CmmnDeploymentResourceData {
    CmmnDeploymentResourceData {
        deployment_id: entity.deployment_id,
        resource_name: entity.resource_name,
        resource_type: entity.resource_type,
        content_type: entity.content_type,
        bytes: entity.bytes,
        created_at: entity.created_at,
    }
}

fn page_items<T>(items: Vec<T>, start: usize, size: Option<usize>) -> PagedResult<T> {
    let total = items.len();
    let start = start.min(total);
    let page_size = size.unwrap_or(total.saturating_sub(start));
    let data = items
        .into_iter()
        .skip(start)
        .take(page_size)
        .collect::<Vec<_>>();

    PagedResult {
        start,
        size: data.len(),
        total,
        data,
    }
}

fn load_runtime_case_instance_ids(
    session: &mut DbSession,
    definition_id: &str,
) -> Result<Vec<String>, CmmnError> {
    let mut params = DbParams::new();
    params.push(definition_id);
    let rows = session.select_list(
        StatementId::SelectCmmnCaseInstanceIdsByCaseDefinitionId,
        params,
    )?;
    rows.into_iter()
        .map(|row| {
            row.get_text("ID_").ok_or_else(|| {
                CmmnError::storage("Missing ID_ in runtime case instance query result")
            })
        })
        .collect::<Result<Vec<_>, _>>()
}

fn load_historic_only_case_instance_ids(
    session: &mut DbSession,
    runtime_ids: &[String],
    definition_id: &str,
) -> Result<Vec<String>, CmmnError> {
    let mut params = DbParams::new();
    params.push(definition_id);
    let rows = session.select_list(
        StatementId::SelectHistoricCmmnCaseInstanceIdsByCaseDefinitionId,
        params,
    )?;
    let runtime_set: HashSet<&str> = runtime_ids.iter().map(String::as_str).collect();
    Ok(rows
        .into_iter()
        .filter_map(|row| row.get_text("CASE_INSTANCE_ID_"))
        .filter(|id| !runtime_set.contains(id.as_str()))
        .collect())
}

fn delete_identity_links_for_case_instance(
    session: &mut DbSession,
    case_instance_id: &str,
) -> Result<(), CmmnError> {
    let mut params = DbParams::new();
    params.push(case_instance_id);
    session.execute(StatementId::DeleteCmmnIdentityLinksByCaseInstanceId, params)?;
    Ok(())
}

fn delete_identity_links_for_human_task(
    session: &mut DbSession,
    task_id: &str,
) -> Result<(), CmmnError> {
    let mut params = DbParams::new();
    params.push(task_id);
    session.execute(StatementId::DeleteCmmnIdentityLinksByTaskId, params)?;
    Ok(())
}

fn delete_identity_links_for_definition(
    session: &mut DbSession,
    definition_id: &str,
) -> Result<(), CmmnError> {
    let mut params = DbParams::new();
    params.push(definition_id);
    session.execute(
        StatementId::DeleteCmmnIdentityLinksByScopeDefinitionId,
        params,
    )?;
    Ok(())
}

fn case_human_task_ids_for_instance(
    session: &mut DbSession,
    case_instance_id: &str,
) -> Result<Vec<String>, CmmnError> {
    let manager = CmmnHumanTaskDataManager::new();
    let tasks = manager
        .find_by_case_instance_id(session, case_instance_id)
        .map_err(CmmnError::from)?;
    Ok(tasks.into_iter().map(|task| task.id).collect())
}

fn case_historic_human_task_ids_for_instance(
    session: &mut DbSession,
    case_instance_id: &str,
) -> Result<Vec<String>, CmmnError> {
    let manager = CmmnHumanTaskHistoryDataManager::new();
    let tasks = manager
        .find_by_case_instance_id(session, case_instance_id)
        .map_err(CmmnError::from)?;
    Ok(tasks.into_iter().map(|task| task.task_id).collect())
}

fn purge_runtime_case_instance_data(
    session: &mut DbSession,
    case_instance_id: &str,
    visited_children: &mut HashSet<String>,
    visited_process_instances: &mut HashSet<String>,
    process_instance_cleanup: Option<&Arc<dyn ProcessInstanceCleanup>>,
) -> Result<(), CmmnError> {
    let human_task_ids = case_human_task_ids_for_instance(session, case_instance_id)?;

    delete_identity_links_for_case_instance(session, case_instance_id)?;
    for task_id in &human_task_ids {
        delete_identity_links_for_human_task(session, task_id)?;
    }

    let mut params = DbParams::new();
    params.push(case_instance_id);
    session.execute(StatementId::DeleteCmmnJobsByScopeId, params.clone())?;
    session.execute(StatementId::DeleteCmmnJobsBySubScopeId, params.clone())?;
    session.execute(
        StatementId::DeleteCmmnEventSubscriptionsByCaseInstanceId,
        params.clone(),
    )?;
    session.execute(
        StatementId::DeleteCmmnPlanItemEventsByCaseInstanceId,
        params.clone(),
    )?;

    // Associations are the only durable source of child instance IDs, so inspect
    // them before removing the parent association rows. BPMN process children are
    // deleted first (via injected cleanup) so they cannot be left as orphans.
    let child_case_instances = collect_and_cleanup_child_instances(
        session,
        case_instance_id,
        visited_process_instances,
        process_instance_cleanup,
    )?;
    purge_child_cmmn_instances(
        session,
        child_case_instances,
        visited_children,
        visited_process_instances,
        process_instance_cleanup,
    )?;

    session.execute(
        StatementId::DeleteCmmnTaskInstanceAssociationsByCaseInstanceId,
        params.clone(),
    )?;

    session.execute(
        StatementId::DeleteCmmnHumanTasksByCaseInstanceId,
        params.clone(),
    )?;
    session.execute(
        StatementId::DeleteCmmnStageInstancesByCaseInstanceId,
        params.clone(),
    )?;
    session.execute(StatementId::DeleteCmmnCaseInstance, params)?;
    Ok(())
}

fn purge_history_for_instance(
    session: &mut DbSession,
    case_instance_id: &str,
) -> Result<(), CmmnError> {
    let historic_task_ids = case_historic_human_task_ids_for_instance(session, case_instance_id)?;
    for task_id in &historic_task_ids {
        delete_identity_links_for_human_task(session, task_id)?;
    }

    let mut params = DbParams::new();
    params.push(case_instance_id);
    session.execute(
        StatementId::DeleteCmmnMilestoneHistoryByCaseInstanceId,
        params.clone(),
    )?;
    session.execute(
        StatementId::DeleteCmmnStageHistoryByCaseInstanceId,
        params.clone(),
    )?;
    session.execute(
        StatementId::DeleteCmmnHumanTaskHistoryByCaseInstanceId,
        params.clone(),
    )?;
    session.execute(StatementId::DeleteCmmnCaseHistory, params)?;
    Ok(())
}

/// Collect CMMN child case instance IDs and purge BPMN process-task children.
///
/// Process children are never returned as CMMN children. When cleanup is not
/// injected, a deterministic conflict is raised so cascade cannot leave orphans.
fn collect_and_cleanup_child_instances(
    session: &mut DbSession,
    case_instance_id: &str,
    visited_process_instances: &mut HashSet<String>,
    process_instance_cleanup: Option<&Arc<dyn ProcessInstanceCleanup>>,
) -> Result<Vec<String>, CmmnError> {
    let manager = CmmnTaskInstanceAssociationDataManager::new();
    let associations = manager.find_by_case_instance_id(session, case_instance_id)?;
    let mut children = Vec::new();
    for association in associations {
        let child_id = association.child_instance_id.trim();
        if child_id.is_empty() {
            continue;
        }
        if association.kind == CmmnTaskAssociationKind::ProcessTask.as_str() {
            let Some(cleanup) = process_instance_cleanup else {
                return Err(CmmnError::unsupported(
                    "process task child cleanup",
                    format!(
                        "cannot cascade delete BPMN child instance '{child_id}' without an injected process cleanup service"
                    ),
                ));
            };
            // Recursion / multi-parent safety: only delete each process once.
            if visited_process_instances.insert(child_id.to_string()) {
                cleanup.delete_process_instance_cascade(child_id)?;
            }
            continue;
        }
        children.push(child_id.to_string());
    }
    Ok(children)
}

fn purge_child_cmmn_instances(
    session: &mut DbSession,
    child_ids: Vec<String>,
    visited: &mut HashSet<String>,
    visited_process_instances: &mut HashSet<String>,
    process_instance_cleanup: Option<&Arc<dyn ProcessInstanceCleanup>>,
) -> Result<(), CmmnError> {
    for child_id in child_ids {
        if visited.insert(child_id.clone()) {
            purge_runtime_case_instance_data(
                session,
                &child_id,
                visited,
                visited_process_instances,
                process_instance_cleanup,
            )?;
            purge_history_for_instance(session, &child_id)?;
        }
    }
    Ok(())
}

fn cascade_purge_definition(
    session: &mut DbSession,
    definition_id: &str,
    visited_children: &mut HashSet<String>,
    visited_process_instances: &mut HashSet<String>,
    process_instance_cleanup: Option<&Arc<dyn ProcessInstanceCleanup>>,
) -> Result<(), CmmnError> {
    let runtime_ids = load_runtime_case_instance_ids(session, definition_id)?;
    for instance_id in &runtime_ids {
        if visited_children.insert(instance_id.clone()) {
            purge_runtime_case_instance_data(
                session,
                instance_id,
                visited_children,
                visited_process_instances,
                process_instance_cleanup,
            )?;
        }
    }

    let historic_only_ids =
        load_historic_only_case_instance_ids(session, &runtime_ids, definition_id)?;
    for instance_id in runtime_ids.iter().chain(historic_only_ids.iter()) {
        purge_history_for_instance(session, instance_id)?;
    }

    let mut params = DbParams::new();
    params.push(definition_id);
    session.execute(
        StatementId::DeleteCmmnEventSubscriptionsByCaseDefinitionId,
        params.clone(),
    )?;
    session.execute(
        StatementId::DeleteCmmnJobsByScopeDefinitionId,
        params.clone(),
    )?;
    delete_identity_links_for_definition(session, definition_id)?;

    Ok(())
}
