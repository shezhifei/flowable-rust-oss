//! Mapping between engine domain types and flowable-persistence normalized entities.
//! Used for dual-write: JSON compatibility tables remain the read path while ACT_* tables
//! receive StatementId / DataManager writes (ADR-0001 Phase 5 vertical slice).

use crate::repository::deployment::Deployment;
use crate::repository::process_definition::ProcessDefinition;
use crate::runtime::execution::Execution;
use flowable_persistence::{DeploymentEntity, ExecutionEntity, ProcessDefinitionEntity};

pub fn deployment_to_entity(deployment: &Deployment) -> DeploymentEntity {
    let mut entity = DeploymentEntity::new(
        deployment.id.clone(),
        deployment.name.clone().unwrap_or_default(),
    );
    entity.category = deployment.category.clone();
    entity.key = deployment.key.clone();
    entity.tenant_id = deployment.tenant_id.clone();
    entity.deploy_time = deployment.deployment_time.map(|t| t.timestamp_millis());
    entity.engine_version = deployment.engine_version.clone();
    entity
}

pub fn process_definition_to_entity(pd: &ProcessDefinition) -> ProcessDefinitionEntity {
    let mut entity = ProcessDefinitionEntity::new(pd.id.clone(), pd.key.clone(), pd.version);
    entity.category = pd.category.clone();
    entity.name = pd.name.clone();
    entity.deployment_id = pd.deployment_id.clone();
    entity.resource_name = pd.resource_name.clone();
    entity.dgrm_resource_name = pd.diagram_resource_name.clone();
    entity.description = pd.description.clone();
    entity.has_graphical_notation = pd.has_graphical_notation;
    entity.has_start_form_key = pd.has_start_form_key;
    entity.suspension_state = if pd.is_suspended { 2 } else { 1 };
    entity.tenant_id = pd.tenant_id.clone();
    entity.engine_version = pd.engine_version.clone();
    entity.app_version = pd.app_version;
    entity
}

pub fn execution_to_entity(execution: &Execution) -> ExecutionEntity {
    let mut entity = ExecutionEntity::new(execution.id.clone());
    entity.process_instance_id = execution.process_instance_id.clone();
    entity.parent_id = execution.parent_id.clone();
    entity.process_definition_id = execution.process_definition_id.clone();
    entity.super_execution_id = execution.super_execution_id.clone();
    entity.root_process_instance_id = execution.root_process_instance_id.clone();
    entity.activity_id = execution.activity_id.clone();
    entity.is_active = execution.is_active;
    entity.is_concurrent = execution.is_concurrent;
    entity.is_scope = execution.is_scope;
    entity.is_mi_root = execution.is_multi_instance_root;
    entity.suspension_state = if execution.is_suspended { 2 } else { 1 };
    entity.tenant_id = execution.tenant_id.clone();
    entity.name = execution.name.clone();
    entity
}
