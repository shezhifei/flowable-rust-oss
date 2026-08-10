use crate::error::DmnError;
use crate::models::{
    CollectOperator, DecisionService, DmnComparisonOperator, DmnDecision, DmnDecisionDefinition,
    DmnDeployment, DmnDeploymentRequest, DmnHitPolicy, DmnModel, DmnStringFunction, DmnUnaryTest,
    PagedResult, is_static_output_literal, normalize_temporal_value, normalized_type_ref,
    number_to_i64, numeric_value, temporal_type_ref,
};
use crate::store::DmnStore;
use chrono::Utc;
use flowable_persistence::entity::dmn_decision_definition::{
    DmnDecisionDefinitionDataManager, DmnDecisionDefinitionEntity,
};
use flowable_persistence::entity::dmn_decision_requirements_diagram::{
    DmnDecisionRequirementsDiagramDataManager, DmnDecisionRequirementsDiagramEntity,
};
use flowable_persistence::entity::dmn_deployment::{DmnDeploymentDataManager, DmnDeploymentEntity};
use flowable_persistence::entity::dmn_deployment_resource::{
    DmnDeploymentResourceDataManager, DmnDeploymentResourceEntity,
};
use flowable_persistence::statement::{RenderedStatement, StatementId};
use flowable_persistence::value::DbParams;
use std::collections::BTreeSet;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct DmnDeploymentResourceData {
    pub deployment_id: String,
    pub resource_name: String,
    pub resource_type: String,
    pub content_type: String,
    pub bytes: Vec<u8>,
    pub created_at: i64,
}

impl DmnDeploymentResourceData {
    pub fn new(
        deployment_id: String,
        resource_name: String,
        bytes: Vec<u8>,
        created_at: i64,
    ) -> Self {
        Self {
            deployment_id,
            resource_type: dmn_resource_type_for_name(&resource_name).to_string(),
            content_type: dmn_content_type_for_name(&resource_name).to_string(),
            resource_name,
            bytes,
            created_at,
        }
    }
}

pub fn dmn_content_type_for_name(resource_name: &str) -> &'static str {
    let lower_name = resource_name.to_ascii_lowercase();
    if lower_name.ends_with(".dmn") || lower_name.ends_with(".xml") {
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

fn dmn_resource_type_for_name(resource_name: &str) -> &'static str {
    let lower_name = resource_name.to_ascii_lowercase();
    if lower_name.ends_with(".dmn") {
        "decisionDefinition"
    } else {
        "resource"
    }
}

#[derive(Clone)]
pub struct DmnRepositoryService {
    store: DmnStore,
}

impl DmnRepositoryService {
    pub(crate) fn new(store: DmnStore) -> Self {
        Self { store }
    }

    pub fn deploy(&self, mut request: DmnDeploymentRequest) -> Result<DmnDeployment, DmnError> {
        validate_and_normalize_deployment_request(&mut request)?;

        let deployment_id = format!("dmn-deployment:{}", Uuid::new_v4());
        let deployed_at = Utc::now();
        let deployment = DmnDeployment {
            id: deployment_id.clone(),
            name: request.name.clone(),
            category: request.category.clone(),
            parent_deployment_id: request.parent_deployment_id.clone(),
            tenant_id: request.tenant_id.clone(),
            resource_names: request
                .resources
                .iter()
                .map(|resource| resource.resource_name.clone())
                .collect(),
            deployed_at,
        };

        let mut session = self.store.create_session()?;
        let deployment_manager = DmnDeploymentDataManager::new();
        let resource_manager = DmnDeploymentResourceDataManager::new();
        let drd_manager = DmnDecisionRequirementsDiagramDataManager::new();
        let decision_manager = DmnDecisionDefinitionDataManager::new();

        let mut deployment_entity = DmnDeploymentEntity::new(
            deployment.id.clone(),
            deployment.name.clone(),
            deployment.deployed_at.to_rfc3339(),
            serde_json::to_string(&deployment)?,
        );
        deployment_entity.set_category(deployment.category.clone());
        deployment_entity.set_parent_deployment_id(deployment.parent_deployment_id.clone());
        deployment_entity.set_tenant_id(deployment.tenant_id.clone());
        deployment_manager.insert(&mut session, deployment_entity)?;

        let mut stored_drd = false;
        for resource in request.resources {
            let created_at = deployed_at.timestamp_millis();
            let resource_data = DmnDeploymentResourceData::new(
                deployment_id.clone(),
                resource.resource_name.clone(),
                resource.resource_bytes.clone(),
                created_at,
            );
            let resource_entity = DmnDeploymentResourceEntity::new(
                deployment_id.clone(),
                resource.resource_name.clone(),
                resource_data.resource_type.clone(),
                resource_data.content_type.clone(),
                resource.resource_bytes.clone(),
                created_at,
            );
            resource_manager.insert(&mut session, resource_entity)?;

            if !stored_drd {
                let mut drd_model = resource.model.clone();
                drd_model.id = deployment_id.clone();
                let drd_entity = DmnDecisionRequirementsDiagramEntity::new(
                    deployment_id.clone(),
                    drd_model.name.clone(),
                    deployment_id.clone(),
                    resource.resource_name.clone(),
                    serde_json::to_string(&drd_model)?,
                );
                drd_manager.insert(&mut session, drd_entity)?;
                stored_drd = true;
            }

            for decision in resource.model.decisions {
                let version =
                    next_version(&mut session, &decision.key, request.tenant_id.as_deref())?;
                let definition = DmnDecisionDefinition {
                    id: format!("dmn-decision:{}:{}", deployment_id, decision.key),
                    decision_id: decision.id,
                    deployment_id: deployment_id.clone(),
                    key: decision.key,
                    name: decision.name,
                    version,
                    category: request.category.clone(),
                    parent_deployment_id: request.parent_deployment_id.clone(),
                    tenant_id: request.tenant_id.clone(),
                    resource_name: resource.resource_name.clone(),
                    hit_policy: decision.hit_policy,
                    collect_operator: decision.collect_operator,
                    inputs: decision.inputs,
                    outputs: decision.outputs,
                    rules: decision.rules,
                    required_decisions: decision.required_decisions,
                };

                let mut decision_entity = DmnDecisionDefinitionEntity::new(
                    definition.id.clone(),
                    definition.key.clone(),
                    definition.deployment_id.clone(),
                    definition.version,
                    definition.resource_name.clone(),
                    serde_json::to_string(&definition)?,
                );
                decision_entity.set_tenant_id(definition.tenant_id.clone());
                decision_manager.insert(&mut session, decision_entity)?;
            }
        }

        session.commit()?;
        Ok(deployment)
    }

    pub fn create_deployment_query(&self) -> DmnDeploymentQuery {
        DmnDeploymentQuery::new(self.store.clone())
    }

    pub fn get_deployment(&self, deployment_id: &str) -> Result<DmnDeployment, DmnError> {
        let mut session = self.store.create_session()?;
        let manager = DmnDeploymentDataManager::new();
        let entity = manager
            .find_by_id(&mut session, deployment_id)?
            .ok_or_else(|| {
                DmnError::not_found(format!("DMN deployment '{}' was not found", deployment_id))
            })?;
        deployment_from_entity(&entity)
    }

    pub fn get_deployment_resource_bytes(
        &self,
        deployment_id: &str,
        resource_name: &str,
    ) -> Result<Vec<u8>, DmnError> {
        self.get_deployment_resource_data(deployment_id, resource_name)
            .map(|resource| resource.bytes)
    }

    pub fn get_deployment_resource_data(
        &self,
        deployment_id: &str,
        resource_name: &str,
    ) -> Result<DmnDeploymentResourceData, DmnError> {
        let mut session = self.store.create_session()?;
        let manager = DmnDeploymentResourceDataManager::new();
        let resources = manager.find_by_deployment_id(&mut session, deployment_id)?;
        resources
            .into_iter()
            .find(|r| r.resource_name == resource_name)
            .map(resource_entity_to_data)
            .ok_or_else(|| {
                DmnError::not_found(format!(
                    "DMN deployment resource '{}' was not found in deployment '{}'",
                    resource_name, deployment_id
                ))
            })
    }

    pub fn get_deployment_resources(
        &self,
        deployment_id: &str,
    ) -> Result<Vec<DmnDeploymentResourceData>, DmnError> {
        self.get_deployment(deployment_id)?;
        let mut session = self.store.create_session()?;
        let manager = DmnDeploymentResourceDataManager::new();
        let resources = manager.find_by_deployment_id(&mut session, deployment_id)?;
        Ok(resources.into_iter().map(resource_entity_to_data).collect())
    }

    pub fn delete_deployment(&self, deployment_id: &str, cascade: bool) -> Result<(), DmnError> {
        self.get_deployment(deployment_id)?;

        let mut session = self.store.create_session()?;
        let mut p = DbParams::new();
        p.push(deployment_id);
        session.execute(
            StatementId::DeleteDmnDeploymentResourcesByDeploymentId,
            p.clone(),
        )?;
        session.execute(
            StatementId::DeleteDmnDecisionRequirementsDiagramsByDeploymentId,
            p.clone(),
        )?;
        session.execute(
            StatementId::DeleteDmnDecisionDefinitionsByDeploymentId,
            p.clone(),
        )?;
        if cascade {
            session.execute(
                StatementId::DeleteDmnExecutionHistoriesByDeploymentId,
                p.clone(),
            )?;
        }
        session.execute(StatementId::DeleteDmnDeployment, p)?;
        session.commit()?;
        Ok(())
    }

    pub fn get_decision_resource_bytes(
        &self,
        decision_definition_id: &str,
    ) -> Result<Vec<u8>, DmnError> {
        let decision = self.get_decision(decision_definition_id)?;
        self.get_deployment_resource_bytes(&decision.deployment_id, &decision.resource_name)
    }

    pub fn create_decision_query(&self) -> DmnDecisionQuery {
        DmnDecisionQuery::new(self.store.clone())
    }

    pub fn get_decision(
        &self,
        decision_definition_id: &str,
    ) -> Result<DmnDecisionDefinition, DmnError> {
        let mut session = self.store.create_session()?;
        let manager = DmnDecisionDefinitionDataManager::new();
        let entity = manager
            .find_by_id(&mut session, decision_definition_id)?
            .ok_or_else(|| {
                DmnError::not_found(format!(
                    "DMN decision definition '{}' was not found",
                    decision_definition_id
                ))
            })?;
        serde_json::from_str(&entity.data).map_err(DmnError::from)
    }

    pub fn get_drd(&self, drd_id: &str) -> Result<DmnModel, DmnError> {
        let mut session = self.store.create_session()?;
        let manager = DmnDecisionRequirementsDiagramDataManager::new();
        let entity = manager
            .find_by_id(&mut session, drd_id)?
            .ok_or_else(|| DmnError::not_found(format!("DMN DRD '{}' was not found", drd_id)))?;
        serde_json::from_str(&entity.data).map_err(DmnError::from)
    }

    pub fn get_drd_with_deployment_info(
        &self,
        drd_id: &str,
    ) -> Result<(DmnDeployment, DmnModel), DmnError> {
        let mut session = self.store.create_session()?;
        let mut params = DbParams::new();
        params.push(drd_id);
        let rendered = RenderedStatement::new(
            "SELECT deployments.DATA_ AS DEPLOYMENT_DATA_, drds.DATA_ AS DRD_DATA_\n                 FROM ACT_DMN_DRD drds\n                 JOIN ACT_DMN_DEPLOYMENT deployments ON deployments.ID_ = drds.DEPLOYMENT_ID_\n                 WHERE drds.ID_ = ?1"
                .to_string(),
            params,
        );
        let row = session
            .select_one_raw(rendered)?
            .ok_or_else(|| DmnError::not_found(format!("DMN DRD '{}' was not found", drd_id)))?;
        let deployment_json = row
            .get_text("DEPLOYMENT_DATA_")
            .ok_or_else(|| DmnError::storage("Missing deployment data in DMN DRD query result"))?;
        let model_json = row
            .get_text("DRD_DATA_")
            .ok_or_else(|| DmnError::storage("Missing DRD data in DMN DRD query result"))?;
        let deployment = serde_json::from_str(&deployment_json)?;
        let model = serde_json::from_str(&model_json)?;
        Ok((deployment, model))
    }

    pub fn list_drds(&self) -> Result<Vec<DmnModel>, DmnError> {
        let mut session = self.store.create_session()?;
        let rendered = RenderedStatement::new(
            "SELECT DATA_ FROM ACT_DMN_DRD ORDER BY ID_ ASC".to_string(),
            DbParams::new(),
        );
        let rows = session.select_raw(rendered)?;
        rows.into_iter()
            .map(|row| {
                let json = row
                    .get_text("DATA_")
                    .ok_or_else(|| DmnError::storage("Missing DATA_ in DMN DRD query result"))?;
                Ok(serde_json::from_str(&json)?)
            })
            .collect()
    }

    pub(crate) fn latest_decision_service_by_key(
        &self,
        decision_service_key: &str,
        tenant_id: Option<&str>,
        parent_deployment_id: Option<&str>,
        fallback_to_default_tenant: bool,
    ) -> Result<Option<(DecisionService, DmnDeployment)>, DmnError> {
        // Mirror AbstractExecuteDecisionCmd parent-first + fallback-to-latest
        // (Java :119-137) for decision services.
        if let Some(parent_id) = parent_deployment_id
            && let Some(found) = self.find_decision_service_by_key_filtered(
                decision_service_key,
                tenant_id,
                Some(parent_id),
            )?
        {
            return Ok(Some(found));
        }
        if let Some(found) =
            self.find_decision_service_by_key_filtered(decision_service_key, tenant_id, None)?
        {
            return Ok(Some(found));
        }

        // Java :141-160 — default-tenant fallback; empty default tenant means
        // "look up without tenant" (see `latest_decision_by_key_with_fallback`).
        if fallback_to_default_tenant && tenant_id.is_some_and(|tenant| !tenant.is_empty()) {
            return self.find_decision_service_by_key_filtered(decision_service_key, None, None);
        }

        Ok(None)
    }

    fn find_decision_service_by_key_filtered(
        &self,
        decision_service_key: &str,
        tenant_id: Option<&str>,
        parent_deployment_id: Option<&str>,
    ) -> Result<Option<(DecisionService, DmnDeployment)>, DmnError> {
        let mut session = self.store.create_session()?;
        let rendered = RenderedStatement::new(
            "SELECT deployments.DATA_ AS DEPLOYMENT_DATA_, drds.DATA_ AS DRD_DATA_\n             FROM ACT_DMN_DRD drds\n             JOIN ACT_DMN_DEPLOYMENT deployments ON deployments.ID_ = drds.DEPLOYMENT_ID_\n             ORDER BY deployments.DEPLOYED_AT_ DESC, deployments.ID_ DESC"
                .to_string(),
            DbParams::new(),
        );
        let rows = session.select_raw(rendered)?;

        for row in rows {
            let deployment_json = row.get_text("DEPLOYMENT_DATA_").ok_or_else(|| {
                DmnError::storage("Missing deployment data in DMN decision service query result")
            })?;
            let model_json = row.get_text("DRD_DATA_").ok_or_else(|| {
                DmnError::storage("Missing DRD data in DMN decision service query result")
            })?;
            let deployment: DmnDeployment = serde_json::from_str(&deployment_json)?;
            if tenant_id.is_some_and(|tenant_id| deployment.tenant_id.as_deref() != Some(tenant_id))
            {
                continue;
            }
            if parent_deployment_id.is_some_and(|parent_deployment_id| {
                deployment.parent_deployment_id.as_deref() != Some(parent_deployment_id)
            }) {
                continue;
            }

            let model: DmnModel = serde_json::from_str(&model_json)?;
            if let Some(service) = model.decision_services.into_iter().find(|service| {
                service.id == decision_service_key || service.name == decision_service_key
            }) {
                return Ok(Some((service, deployment)));
            }
        }

        Ok(None)
    }

    pub fn latest_decision_by_key(
        &self,
        decision_key: &str,
        tenant_id: Option<&str>,
        parent_deployment_id: Option<&str>,
    ) -> Result<DmnDecisionDefinition, DmnError> {
        self.latest_decision_by_key_with_fallback(
            decision_key,
            tenant_id,
            parent_deployment_id,
            false,
        )
    }

    /// Java `AbstractExecuteDecisionCmd.resolveDefinition` (:68-163).
    ///
    /// Lookup order: parent-deployment scoped → latest by key+tenant →
    /// (when `fallback_to_default_tenant`) latest by key in the default tenant.
    /// The default tenant comes from `DefaultTenantProvider`, whose engine
    /// default returns `NO_TENANT_ID` = `""`
    /// (`AbstractEngineConfiguration.java:139,329`); Java then treats an empty
    /// default tenant as "look up without tenant"
    /// (`AbstractExecuteDecisionCmd.java:96-102,150-157`). Rust has no
    /// `DefaultTenantProvider` yet, so we take that empty-default branch:
    /// retry with no tenant filter.
    pub fn latest_decision_by_key_with_fallback(
        &self,
        decision_key: &str,
        tenant_id: Option<&str>,
        parent_deployment_id: Option<&str>,
        fallback_to_default_tenant: bool,
    ) -> Result<DmnDecisionDefinition, DmnError> {
        // Java AbstractExecuteDecisionCmd.java:119-137 — prefer parent-scoped
        // decision; if none, fall back to latest by key without parent filter.
        if let Some(parent_id) = parent_deployment_id {
            let deployments = self
                .create_deployment_query()
                .parent_deployment_id(parent_id)
                .list()?;
            let deployment_ids: std::collections::HashSet<String> =
                deployments.into_iter().map(|d| d.id).collect();
            if !deployment_ids.is_empty() {
                let definitions = self
                    .create_decision_query()
                    .key(decision_key)
                    .tenant_id_optional(tenant_id)
                    .list()?;
                if let Some(matched) = definitions
                    .into_iter()
                    .find(|d| deployment_ids.contains(&d.deployment_id))
                {
                    return Ok(matched);
                }
            }
            // Fall through to latest-by-key (Java :129-131).
        }

        let query = self
            .create_decision_query()
            .key(decision_key)
            .tenant_id_optional(tenant_id)
            .page(0, 1);

        if let Some(found) = query.single_result()? {
            return Ok(found);
        }

        // Java :131-160 — fallback only applies when a tenant was requested
        // (`StringUtils.isNotEmpty(tenantId)`); without one the previous query
        // already was the untenanted lookup.
        if fallback_to_default_tenant && tenant_id.is_some_and(|tenant| !tenant.is_empty()) {
            let fallback = self
                .create_decision_query()
                .key(decision_key)
                .page(0, 1)
                .single_result()?;
            if let Some(found) = fallback {
                return Ok(found);
            }
            // Java :98-101 — "no fall back decision found without tenant".
            return Err(DmnError::not_found(format!(
                "DMN decision '{}' was not found. There was also no fall back decision found without tenant",
                decision_key
            )));
        }

        Err(DmnError::not_found(format!(
            "DMN decision '{}' was not found",
            decision_key
        )))
    }
}

/// Run every side-effect-free deployment gate and normalize the owned request
/// exactly as [`DmnRepositoryService::deploy`] does before persistence begins.
/// Kept crate-visible so the editor boundary can validate canonical models
/// without creating an in-memory database or writing deployment state.
pub(crate) fn validate_and_normalize_deployment_request(
    request: &mut DmnDeploymentRequest,
) -> Result<(), DmnError> {
    // P82c: COLLECT+aggregation structural checks (multi-output, typeRef=number)
    // must run before output typeRef coercion so Java-aligned messages surface
    // instead of generic "incompatible value" from coerce_deployment_output_value.
    // Value-level numeric checks stay in validate_collect_operator after coercion
    // so string numbers (e.g. "1.5") can still be normalized to JSON numbers.
    // Java RuleEngineExecutorImpl.java:323-331 (runtime); Rust deploy-time.
    for resource in &request.resources {
        for decision in &resource.model.decisions {
            validate_collect_operator_structure(decision)?;
        }
    }
    normalize_input_type_refs(request)?;
    normalize_output_type_refs(request)?;
    validate_deployment_request(request)
}

fn normalize_input_type_refs(request: &mut DmnDeploymentRequest) -> Result<(), DmnError> {
    for resource in &mut request.resources {
        for decision in &mut resource.model.decisions {
            for input_index in 0..decision.inputs.len() {
                let Some(type_ref) = decision.inputs[input_index].type_ref.clone() else {
                    continue;
                };
                let input_variable = decision.inputs[input_index].input_variable.clone();

                for rule in &mut decision.rules {
                    let Some(input_entry) = rule.input_entries.get_mut(input_index) else {
                        continue;
                    };
                    normalize_typed_input_unary_test(
                        &decision.key,
                        &input_variable,
                        &type_ref,
                        &mut input_entry.expression,
                    )?;
                }
            }
        }
    }
    Ok(())
}

fn normalize_typed_input_unary_test(
    decision_key: &str,
    input_variable: &str,
    type_ref: &str,
    expression: &mut DmnUnaryTest,
) -> Result<(), DmnError> {
    if temporal_type_ref(type_ref).is_some() {
        return normalize_temporal_unary_test(decision_key, input_variable, type_ref, expression);
    }

    match normalized_type_ref(type_ref).as_str() {
        "integer" => normalize_numeric_unary_test(
            expression,
            type_ref,
            Some((i32::MIN as i64, i32::MAX as i64)),
        ),
        "long" => normalize_numeric_unary_test(expression, type_ref, Some((i64::MIN, i64::MAX))),
        "double" | "number" => normalize_numeric_unary_test(expression, type_ref, None),
        _ => Ok(()),
    }
}

fn normalize_temporal_unary_test(
    decision_key: &str,
    input_variable: &str,
    type_ref: &str,
    expression: &mut DmnUnaryTest,
) -> Result<(), DmnError> {
    let rendered_expression = render_unary_test(expression);
    match expression {
        DmnUnaryTest::Any => Ok(()),
        // Runtime-evaluated entries have no deploy-time literal to coerce; Java
        // likewise defers the whole expression to execution
        // (RuleEngineExecutorImpl.java:189).
        DmnUnaryTest::DeferredComparison { .. }
        | DmnUnaryTest::ElCondition { .. }
        | DmnUnaryTest::PropertyPath { .. } => Ok(()),
        DmnUnaryTest::Equals(value)
        | DmnUnaryTest::NotEquals(value)
        | DmnUnaryTest::GreaterThan(value)
        | DmnUnaryTest::GreaterThanOrEqual(value)
        | DmnUnaryTest::LessThan(value)
        | DmnUnaryTest::LessThanOrEqual(value) => {
            *value = normalize_temporal_value(type_ref, value).ok_or_else(|| {
                incompatible_input_type_ref_error(decision_key, input_variable, type_ref, value)
            })?;
            Ok(())
        }
        DmnUnaryTest::StringFunction { .. }
        | DmnUnaryTest::StringTransform { .. }
        | DmnUnaryTest::StringTransformComparison { .. } => {
            Err(unsupported_input_unary_test_error(&rendered_expression))
        }
        DmnUnaryTest::Range { start, end, .. } => {
            *start = normalize_temporal_value(type_ref, start).ok_or_else(|| {
                incompatible_input_type_ref_error(decision_key, input_variable, type_ref, start)
            })?;
            *end = normalize_temporal_value(type_ref, end).ok_or_else(|| {
                incompatible_input_type_ref_error(decision_key, input_variable, type_ref, end)
            })?;
            Ok(())
        }
        DmnUnaryTest::AnyOf(expressions) => {
            for expression in expressions {
                normalize_temporal_unary_test(decision_key, input_variable, type_ref, expression)?;
            }
            Ok(())
        }
        DmnUnaryTest::And(expressions) | DmnUnaryTest::Or(expressions) => {
            for expression in expressions {
                normalize_temporal_unary_test(decision_key, input_variable, type_ref, expression)?;
            }
            Ok(())
        }
        DmnUnaryTest::Not(expression) => {
            normalize_temporal_unary_test(decision_key, input_variable, type_ref, expression)
        }
        DmnUnaryTest::InstanceOf { .. }
        | DmnUnaryTest::Substring { .. }
        | DmnUnaryTest::Replace { .. }
        | DmnUnaryTest::ListContains { .. } => {
            Err(unsupported_input_unary_test_error(&rendered_expression))
        }
        DmnUnaryTest::InList { values } => {
            for value in values {
                *value = normalize_temporal_value(type_ref, value).ok_or_else(|| {
                    incompatible_input_type_ref_error(decision_key, input_variable, type_ref, value)
                })?;
            }
            Ok(())
        }
    }
}

fn normalize_numeric_unary_test(
    expression: &mut DmnUnaryTest,
    type_ref: &str,
    integer_bounds: Option<(i64, i64)>,
) -> Result<(), DmnError> {
    let rendered_expression = render_unary_test(expression);
    match expression {
        DmnUnaryTest::Any => Ok(()),
        // Deferred to execution — see normalize_temporal_unary_test.
        DmnUnaryTest::DeferredComparison { .. }
        | DmnUnaryTest::ElCondition { .. }
        | DmnUnaryTest::PropertyPath { .. } => Ok(()),
        DmnUnaryTest::Equals(value)
        | DmnUnaryTest::NotEquals(value)
        | DmnUnaryTest::GreaterThan(value)
        | DmnUnaryTest::GreaterThanOrEqual(value)
        | DmnUnaryTest::LessThan(value)
        | DmnUnaryTest::LessThanOrEqual(value) => {
            *value = coerce_deployment_input_numeric_value(type_ref, value, integer_bounds)
                .ok_or_else(|| unsupported_input_unary_test_error(&rendered_expression))?;
            Ok(())
        }
        DmnUnaryTest::StringFunction { .. }
        | DmnUnaryTest::StringTransform { .. }
        | DmnUnaryTest::StringTransformComparison { .. }
        | DmnUnaryTest::InstanceOf { .. }
        | DmnUnaryTest::Substring { .. }
        | DmnUnaryTest::Replace { .. }
        | DmnUnaryTest::ListContains { .. } => {
            Err(unsupported_input_unary_test_error(&rendered_expression))
        }
        DmnUnaryTest::Range { start, end, .. } => {
            *start = coerce_deployment_input_numeric_value(type_ref, start, integer_bounds)
                .ok_or_else(|| unsupported_input_unary_test_error(&rendered_expression))?;
            *end = coerce_deployment_input_numeric_value(type_ref, end, integer_bounds)
                .ok_or_else(|| unsupported_input_unary_test_error(&rendered_expression))?;
            Ok(())
        }
        DmnUnaryTest::AnyOf(expressions) => {
            for expression in expressions {
                normalize_numeric_unary_test(expression, type_ref, integer_bounds)?;
            }
            Ok(())
        }
        DmnUnaryTest::And(expressions) | DmnUnaryTest::Or(expressions) => {
            for expression in expressions {
                normalize_numeric_unary_test(expression, type_ref, integer_bounds)?;
            }
            Ok(())
        }
        DmnUnaryTest::Not(expression) => {
            normalize_numeric_unary_test(expression, type_ref, integer_bounds)
        }
        DmnUnaryTest::InList { values } => {
            for value in values {
                *value = coerce_deployment_input_numeric_value(type_ref, value, integer_bounds)
                    .ok_or_else(|| unsupported_input_unary_test_error(&rendered_expression))?;
            }
            Ok(())
        }
    }
}

fn coerce_deployment_input_numeric_value(
    type_ref: &str,
    value: &serde_json::Value,
    integer_bounds: Option<(i64, i64)>,
) -> Option<serde_json::Value> {
    if value.is_null() {
        return Some(serde_json::Value::Null);
    }

    let number = numeric_value(value)?;
    if let Some((min, max)) = integer_bounds {
        let integer = number_to_i64(&number)?;
        if integer < min || integer > max {
            return None;
        }
        return Some(serde_json::Value::from(integer));
    }

    match normalized_type_ref(type_ref).as_str() {
        "double" | "number" => Some(number),
        _ => None,
    }
}

fn unsupported_input_unary_test_error(expression: &str) -> DmnError {
    DmnError::unsupported(
        "unary test",
        format!("unsupported unary test '{expression}' in owned M15 subset",),
    )
}

fn render_unary_test(expression: &DmnUnaryTest) -> String {
    match expression {
        DmnUnaryTest::Any => "-".to_string(),
        DmnUnaryTest::DeferredComparison { operator, source } => {
            let operator = match operator {
                crate::models::DmnDeferredOperator::Equals => "==",
                crate::models::DmnDeferredOperator::NotEquals => "!=",
                crate::models::DmnDeferredOperator::GreaterThan => ">",
                crate::models::DmnDeferredOperator::GreaterThanOrEqual => ">=",
                crate::models::DmnDeferredOperator::LessThan => "<",
                crate::models::DmnDeferredOperator::LessThanOrEqual => "<=",
            };
            format!("{operator} {source}")
        }
        DmnUnaryTest::ElCondition { source } => format!("#{{{source}}}"),
        DmnUnaryTest::PropertyPath { path, test } => {
            format!(".{} {}", path.join("."), render_unary_test(test))
        }
        DmnUnaryTest::Equals(value) => render_literal(value),
        DmnUnaryTest::NotEquals(value) => format!("!= {}", render_literal(value)),
        DmnUnaryTest::StringFunction { function, needle } => {
            format!("{}(?, '{}')", render_string_function(*function), needle)
        }
        DmnUnaryTest::StringTransform {
            transform,
            expected,
        } => format!(
            "{}(?) = '{}'",
            render_string_transform(*transform),
            expected
        ),
        DmnUnaryTest::StringTransformComparison {
            transform,
            operator,
            expected,
        } => format!(
            "{}(?) {} {}",
            render_string_transform(*transform),
            render_comparison_operator(*operator),
            render_literal(expected)
        ),
        DmnUnaryTest::GreaterThan(value) => format!("> {}", render_literal(value)),
        DmnUnaryTest::GreaterThanOrEqual(value) => format!(">= {}", render_literal(value)),
        DmnUnaryTest::LessThan(value) => format!("< {}", render_literal(value)),
        DmnUnaryTest::LessThanOrEqual(value) => format!("<= {}", render_literal(value)),
        DmnUnaryTest::Range {
            start,
            end,
            start_inclusive,
            end_inclusive,
        } => format!(
            "{}{}..{}{}",
            if *start_inclusive { "[" } else { "(" },
            render_literal(start),
            render_literal(end),
            if *end_inclusive { "]" } else { ")" }
        ),
        DmnUnaryTest::AnyOf(expressions) => expressions
            .iter()
            .map(render_unary_test)
            .collect::<Vec<_>>()
            .join(", "),
        DmnUnaryTest::Not(expression) => format!("not({})", render_unary_test(expression)),
        DmnUnaryTest::And(expressions) => expressions
            .iter()
            .map(render_unary_test)
            .collect::<Vec<_>>()
            .join(" and "),
        DmnUnaryTest::Or(expressions) => expressions
            .iter()
            .map(render_unary_test)
            .collect::<Vec<_>>()
            .join(" or "),
        DmnUnaryTest::InstanceOf { type_name } => format!("instance of({type_name})"),
        DmnUnaryTest::Substring {
            start,
            length,
            expected,
        } => {
            if let Some(len) = length {
                format!("substring(?, {}, {}) = '{}'", start, len, expected)
            } else {
                format!("substring(?, {}) = '{}'", start, expected)
            }
        }
        DmnUnaryTest::Replace {
            pattern,
            replacement,
            flags,
            expected,
        } => {
            if let Some(f) = flags {
                format!(
                    "replace(?, '{}', '{}', '{}') = '{}'",
                    pattern, replacement, f, expected
                )
            } else {
                format!(
                    "replace(?, '{}', '{}') = '{}'",
                    pattern, replacement, expected
                )
            }
        }
        DmnUnaryTest::ListContains { needle } => match needle {
            crate::models::DmnListContainsNeedle::Literal(value) => {
                format!("list contains(?, {})", render_literal(value))
            }
            crate::models::DmnListContainsNeedle::Variable(name) => {
                format!("list contains(?, {name})")
            }
        },
        DmnUnaryTest::InList { values } => {
            let rendered = values
                .iter()
                .map(render_literal)
                .collect::<Vec<_>>()
                .join(", ");
            format!("? in ({rendered})")
        }
    }
}

fn render_string_function(function: DmnStringFunction) -> &'static str {
    match function {
        DmnStringFunction::Contains => "contains",
        DmnStringFunction::StartsWith => "starts with",
        DmnStringFunction::EndsWith => "ends with",
        DmnStringFunction::Matches => "matches",
    }
}

fn render_string_transform(transform: crate::models::DmnStringTransform) -> &'static str {
    match transform {
        crate::models::DmnStringTransform::LowerCase => "lower case",
        crate::models::DmnStringTransform::UpperCase => "upper case",
        crate::models::DmnStringTransform::StringLength => "string length",
    }
}

fn render_comparison_operator(operator: DmnComparisonOperator) -> &'static str {
    match operator {
        DmnComparisonOperator::GreaterThan => ">",
        DmnComparisonOperator::GreaterThanOrEqual => ">=",
        DmnComparisonOperator::LessThan => "<",
        DmnComparisonOperator::LessThanOrEqual => "<=",
    }
}

fn render_literal(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(value) => format!("'{value}'"),
        _ => value.to_string(),
    }
}

fn normalize_output_type_refs(request: &mut DmnDeploymentRequest) -> Result<(), DmnError> {
    for resource in &mut request.resources {
        for decision in &mut resource.model.decisions {
            for output_index in 0..decision.outputs.len() {
                let Some(type_ref) = decision.outputs[output_index].type_ref.clone() else {
                    continue;
                };
                let output_name = decision.outputs[output_index].name.clone();

                decision.outputs[output_index].output_values = decision.outputs[output_index]
                    .output_values
                    .iter()
                    .map(|value| {
                        coerce_deployment_output_value(
                            &decision.key,
                            &output_name,
                            &type_ref,
                            value,
                        )
                    })
                    .collect::<Result<Vec<_>, _>>()?;

                for rule in &mut decision.rules {
                    let Some(output_entry) = rule.output_entries.get_mut(output_index) else {
                        continue;
                    };
                    // Non-literal expressions are evaluated at runtime
                    // (Java RuleEngineExecutorImpl.java:253-254); skip deploy-time
                    // typeRef coerce so FEEL expressions like `price * 2` can deploy.
                    // Static literals keep deploy-time validation (existing type_ref tests).
                    if !output_entry.expression.trim().is_empty()
                        && !is_static_output_literal(&output_entry.expression)
                    {
                        continue;
                    }
                    output_entry.value = coerce_deployment_output_value(
                        &decision.key,
                        &output_name,
                        &type_ref,
                        &output_entry.value,
                    )?;
                }
            }
        }
    }
    Ok(())
}

fn coerce_deployment_output_value(
    decision_key: &str,
    output_name: &str,
    type_ref: &str,
    value: &serde_json::Value,
) -> Result<serde_json::Value, DmnError> {
    if value.is_null() {
        return Ok(serde_json::Value::Null);
    }

    match normalized_type_ref(type_ref).as_str() {
        "string" => value
            .as_str()
            .map(|value| serde_json::Value::String(value.to_string()))
            .ok_or_else(|| {
                incompatible_output_type_ref_error(decision_key, output_name, type_ref, value)
            }),
        "boolean" => value.as_bool().map(serde_json::Value::Bool).ok_or_else(|| {
            incompatible_output_type_ref_error(decision_key, output_name, type_ref, value)
        }),
        "integer" => coerce_deployment_integer_output(
            decision_key,
            output_name,
            type_ref,
            value,
            i32::MIN as i64,
            i32::MAX as i64,
        ),
        "long" => coerce_deployment_integer_output(
            decision_key,
            output_name,
            type_ref,
            value,
            i64::MIN,
            i64::MAX,
        ),
        "double" | "number" => numeric_value(value).ok_or_else(|| {
            incompatible_output_type_ref_error(decision_key, output_name, type_ref, value)
        }),
        "date" | "time" | "datetime" | "duration" | "daytimeduration" | "yearmonthduration" => {
            normalize_temporal_value(type_ref, value).ok_or_else(|| {
                incompatible_output_type_ref_error(decision_key, output_name, type_ref, value)
            })
        }
        "context" => value
            .as_object()
            .map(|value| serde_json::Value::Object(value.clone()))
            .ok_or_else(|| {
                incompatible_output_type_ref_error(decision_key, output_name, type_ref, value)
            }),
        "list" => value
            .as_array()
            .map(|value| serde_json::Value::Array(value.clone()))
            .ok_or_else(|| {
                incompatible_output_type_ref_error(decision_key, output_name, type_ref, value)
            }),
        _ => Err(DmnError::unsupported(
            "typeRef",
            format!(
                "unsupported output typeRef '{}' for decision '{}' output '{}'; supported output typeRefs are string, boolean, integer, long, double, number, date, time, dateTime, date and time, duration, dayTimeDuration, yearMonthDuration, context, and list",
                type_ref, decision_key, output_name
            ),
        )),
    }
}

fn coerce_deployment_integer_output(
    decision_key: &str,
    output_name: &str,
    type_ref: &str,
    value: &serde_json::Value,
    min: i64,
    max: i64,
) -> Result<serde_json::Value, DmnError> {
    let Some(number) = numeric_value(value) else {
        return Err(incompatible_output_type_ref_error(
            decision_key,
            output_name,
            type_ref,
            value,
        ));
    };
    let Some(integer) = number_to_i64(&number) else {
        return Err(incompatible_output_type_ref_error(
            decision_key,
            output_name,
            type_ref,
            value,
        ));
    };
    if integer < min || integer > max {
        return Err(incompatible_output_type_ref_error(
            decision_key,
            output_name,
            type_ref,
            value,
        ));
    }

    Ok(serde_json::Value::from(integer))
}

fn incompatible_input_type_ref_error(
    decision_key: &str,
    input_variable: &str,
    type_ref: &str,
    value: &serde_json::Value,
) -> DmnError {
    DmnError::validation(format!(
        "DMN decision '{}' input '{}' with typeRef '{}' has incompatible value {}",
        decision_key, input_variable, type_ref, value
    ))
}

fn incompatible_output_type_ref_error(
    decision_key: &str,
    output_name: &str,
    type_ref: &str,
    value: &serde_json::Value,
) -> DmnError {
    DmnError::validation(format!(
        "DMN decision '{}' output '{}' with typeRef '{}' has incompatible value {}",
        decision_key, output_name, type_ref, value
    ))
}

#[derive(Clone)]
pub struct DmnDeploymentQuery {
    store: DmnStore,
    id: Option<String>,
    name: Option<String>,
    name_like: Option<String>,
    category: Option<String>,
    category_not_equals: Option<String>,
    parent_deployment_id: Option<String>,
    parent_deployment_id_like: Option<String>,
    tenant_id: Option<String>,
    tenant_id_like: Option<String>,
    without_tenant_id: bool,
    resource_name: Option<String>,
    start: usize,
    size: Option<usize>,
}

impl DmnDeploymentQuery {
    fn new(store: DmnStore) -> Self {
        Self {
            store,
            id: None,
            name: None,
            name_like: None,
            category: None,
            category_not_equals: None,
            parent_deployment_id: None,
            parent_deployment_id_like: None,
            tenant_id: None,
            tenant_id_like: None,
            without_tenant_id: false,
            resource_name: None,
            start: 0,
            size: None,
        }
    }

    pub fn id(mut self, id: impl Into<String>) -> Self {
        self.id = Some(id.into());
        self
    }

    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    pub fn category(mut self, category: impl Into<String>) -> Self {
        self.category = Some(category.into());
        self
    }

    pub fn name_like(mut self, name_like: impl Into<String>) -> Self {
        self.name_like = Some(name_like.into());
        self
    }

    pub fn category_not_equals(mut self, category: impl Into<String>) -> Self {
        self.category_not_equals = Some(category.into());
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

    pub fn resource_name(mut self, resource_name: impl Into<String>) -> Self {
        self.resource_name = Some(resource_name.into());
        self
    }

    pub fn page(mut self, start: usize, size: usize) -> Self {
        self.start = start;
        self.size = Some(size);
        self
    }

    pub fn list(&self) -> Result<Vec<DmnDeployment>, DmnError> {
        let mut session = self.store.create_session()?;
        let mut sql = String::from(
            "SELECT ID_, NAME_, CATEGORY_, PARENT_DEPLOYMENT_ID_, TENANT_ID_, DEPLOYED_AT_, DATA_\n             FROM ACT_DMN_DEPLOYMENT WHERE 1=1",
        );
        let mut params = DbParams::new();
        if let Some(value) = &self.id {
            let index = params.len() + 1;
            sql.push_str(&format!(" AND ID_ = ?{index}"));
            params.push(value.clone());
        }
        if let Some(value) = &self.name {
            let index = params.len() + 1;
            sql.push_str(&format!(" AND NAME_ = ?{index}"));
            params.push(value.clone());
        }
        if let Some(value) = &self.name_like {
            let index = params.len() + 1;
            sql.push_str(&format!(" AND NAME_ LIKE ?{index}"));
            params.push(value.clone());
        }
        if let Some(value) = &self.category {
            let index = params.len() + 1;
            sql.push_str(&format!(" AND CATEGORY_ = ?{index}"));
            params.push(value.clone());
        }
        if let Some(value) = &self.category_not_equals {
            let index = params.len() + 1;
            sql.push_str(&format!(" AND CATEGORY_ <> ?{index}"));
            params.push(value.clone());
        }
        if let Some(value) = &self.parent_deployment_id {
            let index = params.len() + 1;
            sql.push_str(&format!(" AND PARENT_DEPLOYMENT_ID_ = ?{index}"));
            params.push(value.clone());
        }
        if let Some(value) = &self.parent_deployment_id_like {
            let index = params.len() + 1;
            sql.push_str(&format!(" AND PARENT_DEPLOYMENT_ID_ LIKE ?{index}"));
            params.push(value.clone());
        }
        if let Some(value) = &self.tenant_id {
            let index = params.len() + 1;
            sql.push_str(&format!(" AND TENANT_ID_ = ?{index}"));
            params.push(value.clone());
        }
        if let Some(value) = &self.tenant_id_like {
            let index = params.len() + 1;
            sql.push_str(&format!(" AND TENANT_ID_ LIKE ?{index}"));
            params.push(value.clone());
        }
        if self.without_tenant_id {
            sql.push_str(" AND TENANT_ID_ IS NULL");
        }
        if let Some(value) = &self.resource_name {
            let index = params.len() + 1;
            sql.push_str(&format!(
                " AND ID_ IN (SELECT DEPLOYMENT_ID_ FROM ACT_DMN_RESOURCE WHERE RESOURCE_NAME_ = ?{index})"
            ));
            params.push(value.clone());
        }
        sql.push_str(" ORDER BY DEPLOYED_AT_ ASC, ID_ ASC");

        let rendered = RenderedStatement::new(sql, params);
        let rows = session.select_raw(rendered)?;
        rows.into_iter()
            .map(|row| {
                let data = row.get_text("DATA_").ok_or_else(|| {
                    DmnError::storage("Missing DATA_ in DMN deployment query result")
                })?;
                let deployment: DmnDeployment = serde_json::from_str(&data)?;
                Ok(DmnDeployment {
                    id: row.get_text("ID_").unwrap_or_default(),
                    name: row.get_text("NAME_").unwrap_or_default(),
                    category: row.get_text("CATEGORY_"),
                    parent_deployment_id: row.get_text("PARENT_DEPLOYMENT_ID_"),
                    tenant_id: row.get_text("TENANT_ID_"),
                    resource_names: deployment.resource_names,
                    deployed_at: deployment.deployed_at,
                })
            })
            .collect()
    }

    pub fn single_result(&self) -> Result<Option<DmnDeployment>, DmnError> {
        Ok(self.list()?.into_iter().next())
    }

    pub fn list_page(&self) -> Result<PagedResult<DmnDeployment>, DmnError> {
        Ok(page_items(self.list()?, self.start, self.size))
    }
}

#[derive(Clone)]
pub struct DmnDecisionQuery {
    store: DmnStore,
    id: Option<String>,
    key: Option<String>,
    deployment_id: Option<String>,
    tenant_id: Option<String>,
    resource_name: Option<String>,
    version: Option<i32>,
    start: usize,
    size: Option<usize>,
}

impl DmnDecisionQuery {
    fn new(store: DmnStore) -> Self {
        Self {
            store,
            id: None,
            key: None,
            deployment_id: None,
            tenant_id: None,
            resource_name: None,
            version: None,
            start: 0,
            size: None,
        }
    }

    pub fn id(mut self, id: impl Into<String>) -> Self {
        self.id = Some(id.into());
        self
    }

    pub fn key(mut self, key: impl Into<String>) -> Self {
        self.key = Some(key.into());
        self
    }

    pub fn deployment_id(mut self, deployment_id: impl Into<String>) -> Self {
        self.deployment_id = Some(deployment_id.into());
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

    pub fn resource_name(mut self, resource_name: impl Into<String>) -> Self {
        self.resource_name = Some(resource_name.into());
        self
    }

    pub fn version(mut self, version: i32) -> Self {
        self.version = Some(version);
        self
    }

    pub fn page(mut self, start: usize, size: usize) -> Self {
        self.start = start;
        self.size = Some(size);
        self
    }

    pub fn list(&self) -> Result<Vec<DmnDecisionDefinition>, DmnError> {
        let mut session = self.store.create_session()?;
        let rendered = RenderedStatement::new(
            "SELECT DATA_ FROM ACT_DMN_DECISION ORDER BY DECISION_KEY_ ASC, VERSION_ DESC, ID_ ASC"
                .to_string(),
            DbParams::new(),
        );
        let rows = session.select_raw(rendered)?;
        let mut definitions = rows
            .into_iter()
            .map(|row| {
                let json = row.get_text("DATA_").ok_or_else(|| {
                    DmnError::storage("Missing DATA_ in DMN decision query result")
                })?;
                Ok(serde_json::from_str::<DmnDecisionDefinition>(&json)?)
            })
            .collect::<Result<Vec<_>, DmnError>>()?;

        definitions.retain(|item| matches_optional(&self.id, &item.id));
        definitions.retain(|item| matches_optional(&self.key, &item.key));
        definitions.retain(|item| matches_optional(&self.deployment_id, &item.deployment_id));
        definitions
            .retain(|item| matches_optional_option(&self.tenant_id, item.tenant_id.as_deref()));
        definitions.retain(|item| matches_optional(&self.resource_name, &item.resource_name));
        definitions.retain(|item| self.version.is_none_or(|value| item.version == value));
        Ok(definitions)
    }

    pub fn single_result(&self) -> Result<Option<DmnDecisionDefinition>, DmnError> {
        Ok(self.list()?.into_iter().next())
    }

    pub fn list_page(&self) -> Result<PagedResult<DmnDecisionDefinition>, DmnError> {
        Ok(page_items(self.list()?, self.start, self.size))
    }
}

fn validate_deployment_request(request: &DmnDeploymentRequest) -> Result<(), DmnError> {
    if request.name.trim().is_empty() {
        return Err(DmnError::validation("DMN deployment name is required"));
    }
    if request.resources.is_empty() {
        return Err(DmnError::validation(
            "DMN deployment resources are required",
        ));
    }

    let mut seen_keys = BTreeSet::new();
    for resource in &request.resources {
        if resource.resource_name.trim().is_empty() {
            return Err(DmnError::validation(
                "DMN deployment resource name is required",
            ));
        }
        if !resource.resource_name.ends_with(".dmn") {
            return Err(DmnError::validation(format!(
                "Unsupported DMN resource '{}'",
                resource.resource_name
            )));
        }
        if resource.model.decisions.is_empty() {
            return Err(DmnError::validation(format!(
                "DMN resource '{}' does not contain decisions",
                resource.resource_name
            )));
        }
        for decision in &resource.model.decisions {
            validate_decision(decision, &resource.resource_name)?;
            if !seen_keys.insert(decision.key.clone()) {
                return Err(DmnError::validation(format!(
                    "DMN decision key '{}' appears multiple times in one deployment",
                    decision.key
                )));
            }
        }
    }
    Ok(())
}

fn validate_decision(decision: &DmnDecision, resource_name: &str) -> Result<(), DmnError> {
    if decision.id.trim().is_empty() {
        return Err(DmnError::validation(format!(
            "DMN resource '{}' contains a decision without id",
            resource_name
        )));
    }
    if decision.key.trim().is_empty() {
        return Err(DmnError::validation(format!(
            "DMN resource '{}' contains a decision without key",
            resource_name
        )));
    }
    if decision.name.trim().is_empty() {
        return Err(DmnError::validation(format!(
            "DMN resource '{}' contains a decision without name",
            resource_name
        )));
    }
    if !matches!(
        decision.hit_policy,
        DmnHitPolicy::First
            | DmnHitPolicy::Unique
            | DmnHitPolicy::Any
            | DmnHitPolicy::RuleOrder
            | DmnHitPolicy::OutputOrder
            | DmnHitPolicy::Priority
            | DmnHitPolicy::Collect
            | DmnHitPolicy::Complete
            | DmnHitPolicy::Batch
    ) {
        return Err(DmnError::unsupported(
            "hit policy",
            format!(
                "resource '{}' decision '{}' declared {:?}, but only FIRST, UNIQUE, ANY, RULE ORDER, OUTPUT ORDER, PRIORITY, COLLECT, COMPLETE, and BATCH are supported",
                resource_name, decision.key, decision.hit_policy
            ),
        ));
    }
    validate_collect_operator(decision)?;
    if decision.outputs.is_empty() {
        return Err(DmnError::validation(format!(
            "DMN decision '{}' must declare at least one output",
            decision.key
        )));
    }
    if decision.rules.is_empty() {
        return Err(DmnError::validation(format!(
            "DMN decision '{}' must declare at least one rule",
            decision.key
        )));
    }

    let mut output_names = BTreeSet::new();
    for output in &decision.outputs {
        if output.name.trim().is_empty() {
            return Err(DmnError::validation(format!(
                "DMN decision '{}' contains an output without name",
                decision.key
            )));
        }
        if !output_names.insert(output.name.clone()) {
            return Err(DmnError::validation(format!(
                "DMN decision '{}' contains duplicate output '{}'",
                decision.key, output.name
            )));
        }
    }

    for rule in &decision.rules {
        if rule.input_entries.len() != decision.inputs.len() {
            return Err(DmnError::validation(format!(
                "DMN decision '{}' rule '{}' declared {} input entries but expected {}",
                decision.key,
                rule.id,
                rule.input_entries.len(),
                decision.inputs.len()
            )));
        }
        if rule.output_entries.len() != decision.outputs.len() {
            return Err(DmnError::validation(format!(
                "DMN decision '{}' rule '{}' declared {} output entries but expected {}",
                decision.key,
                rule.id,
                rule.output_entries.len(),
                decision.outputs.len()
            )));
        }
        for input_entry in &rule.input_entries {
            validate_unary_test(&input_entry.expression)?;
        }
    }

    validate_output_priority_policy(decision)?;
    Ok(())
}

fn validate_unary_test(expression: &DmnUnaryTest) -> Result<(), DmnError> {
    match expression {
        DmnUnaryTest::StringFunction {
            function: DmnStringFunction::Matches,
            needle,
        } => regex::Regex::new(needle).map(|_| ()).map_err(|error| {
            DmnError::validation(format!(
                "invalid matches regex '{needle}' in unary test '{}': {error}",
                render_unary_test(expression)
            ))
        }),
        DmnUnaryTest::AnyOf(expressions)
        | DmnUnaryTest::And(expressions)
        | DmnUnaryTest::Or(expressions) => {
            for expression in expressions {
                validate_unary_test(expression)?;
            }
            Ok(())
        }
        DmnUnaryTest::Not(expression) => validate_unary_test(expression),
        DmnUnaryTest::Replace { pattern, flags, .. } => {
            let mut builder = regex::RegexBuilder::new(pattern);
            if let Some(f) = flags {
                if f.contains('i') {
                    builder.case_insensitive(true);
                }
                if f.contains('s') {
                    builder.dot_matches_new_line(true);
                }
                if f.contains('m') {
                    builder.multi_line(true);
                }
                if f.contains('x') {
                    builder.ignore_whitespace(true);
                }
            }
            builder.build().map(|_| ()).map_err(|error| {
                DmnError::validation(format!(
                    "invalid replace regex pattern '{pattern}' in unary test '{}': {error}",
                    render_unary_test(expression)
                ))
            })
        }
        _ => Ok(()),
    }
}

/// Structural COLLECT+aggregation checks (multi-output, typeRef).
/// Safe to run before typeRef coercion of rule output literals.
fn validate_collect_operator_structure(decision: &DmnDecision) -> Result<(), DmnError> {
    let Some(operator) = &decision.collect_operator else {
        return Ok(());
    };

    if decision.hit_policy != DmnHitPolicy::Collect {
        return Err(DmnError::validation(format!(
            "DMN decision '{}' declares COLLECT aggregation for non-COLLECT hit policy",
            decision.key
        )));
    }
    if decision.outputs.is_empty() {
        return Err(DmnError::validation(format!(
            "DMN decision '{}' COLLECT aggregation requires at least one output",
            decision.key
        )));
    }

    // Java RuleEngineExecutorImpl.java:323-331 (runtime sanityCheckDecisionTable) —
    // Rust deploys this at validation time (400) instead of execution (Java 500).
    // Applies to ALL aggregations including COUNT.
    if decision.outputs.len() > 1 {
        return Err(DmnError::validation(format!(
            "HitPolicy: COLLECT has aggregation: {:?} and multiple outputs. This is not supported",
            operator
        )));
    }
    let type_ref = decision.outputs[0].type_ref.as_deref().unwrap_or("");
    // Java: !"number".equals(getTypeRef()) — COUNT included.
    if normalized_type_ref(type_ref) != "number" {
        return Err(DmnError::validation(format!(
            "HitPolicy: COLLECT has aggregation: {:?} needs output type number",
            operator
        )));
    }

    Ok(())
}

fn validate_collect_operator(decision: &DmnDecision) -> Result<(), DmnError> {
    validate_collect_operator_structure(decision)?;

    let Some(operator) = &decision.collect_operator else {
        return Ok(());
    };

    // After typeRef coercion: SUM/MIN/MAX require JSON numbers in rule outputs.
    if matches!(
        operator,
        CollectOperator::Sum | CollectOperator::Min | CollectOperator::Max
    ) {
        for rule in &decision.rules {
            for output in &rule.output_entries {
                // Runtime FEEL expressions are validated when evaluated
                // (Java RuleEngineExecutorImpl.java:253-254); only static
                // numeric snapshots are checked at deploy time.
                if !output.expression.trim().is_empty()
                    && !is_static_output_literal(&output.expression)
                {
                    continue;
                }
                if !output.value.is_number() {
                    return Err(DmnError::validation(format!(
                        "DMN decision '{}' COLLECT {:?} aggregation requires numeric output values; rule '{}' produced {}",
                        decision.key, operator, rule.id, output.value
                    )));
                }
            }
        }
    }

    Ok(())
}

fn validate_output_priority_policy(decision: &DmnDecision) -> Result<(), DmnError> {
    if !matches!(
        decision.hit_policy,
        DmnHitPolicy::OutputOrder | DmnHitPolicy::Priority
    ) {
        return Ok(());
    }

    if decision.outputs.len() != 1 {
        return Err(DmnError::unsupported(
            "hit policy",
            format!(
                "DMN decision '{}' {:?} hit policy requires a single output in the supported subset",
                decision.key, decision.hit_policy
            ),
        ));
    }

    // Empty outputValues is NOT a deploy-time rejection: Java only fails at
    // evaluation when PRIORITY/OUTPUT_ORDER need ranking
    // (`HitPolicyPriority.java:60-72`, `HitPolicyOutputOrder.java:53-60`), and
    // non-strict mode soft-fails with a validationMessage. Values absent from
    // outputValues are also never an error — `OutputOrderComparator.java:31-33`
    // ranks them first via indexOf = -1 (P89 runtime ranking).
    Ok(())
}

fn next_version(
    session: &mut flowable_persistence::db_session::DbSession,
    decision_key: &str,
    tenant_id: Option<&str>,
) -> Result<i32, DmnError> {
    let mut params = DbParams::new();
    params.push(decision_key);
    params.push(tenant_id);
    let rendered = RenderedStatement::new(
        "SELECT MAX(VERSION_) FROM ACT_DMN_DECISION\n             WHERE DECISION_KEY_ = ?1\n               AND ((TENANT_ID_ IS NULL AND ?2 IS NULL) OR TENANT_ID_ = ?2)"
            .to_string(),
        params,
    );
    let row = session.select_one_raw(rendered)?;
    let current: Option<i32> = row
        .and_then(|r| r.get_integer("MAX(VERSION_)"))
        .map(|v| v as i32);

    Ok(current.unwrap_or(0) + 1)
}

fn deployment_from_entity(entity: &DmnDeploymentEntity) -> Result<DmnDeployment, DmnError> {
    let mut deployment: DmnDeployment = serde_json::from_str(&entity.data)?;
    deployment.id = entity.id.clone();
    deployment.name = entity.name.clone();
    deployment.category = entity.category.clone();
    deployment.parent_deployment_id = entity.parent_deployment_id.clone();
    deployment.tenant_id = entity.tenant_id.clone();
    Ok(deployment)
}

fn resource_entity_to_data(entity: DmnDeploymentResourceEntity) -> DmnDeploymentResourceData {
    DmnDeploymentResourceData {
        deployment_id: entity.deployment_id,
        resource_name: entity.resource_name,
        resource_type: entity.resource_type,
        content_type: entity.content_type,
        bytes: entity.bytes,
        created_at: entity.created_at,
    }
}

fn matches_optional(filter: &Option<String>, actual: &str) -> bool {
    filter.as_ref().is_none_or(|value| value == actual)
}

fn matches_optional_option(filter: &Option<String>, actual: Option<&str>) -> bool {
    filter
        .as_ref()
        .is_none_or(|value| actual == Some(value.as_str()))
}

fn page_items<T>(items: Vec<T>, start: usize, size: Option<usize>) -> PagedResult<T> {
    let total = items.len();
    let start = start.min(total);
    let page_size = size.unwrap_or(total.saturating_sub(start));
    let data: Vec<T> = items.into_iter().skip(start).take(page_size).collect();

    PagedResult {
        start,
        size: data.len(),
        total,
        data,
    }
}
