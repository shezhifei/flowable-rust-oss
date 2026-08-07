use crate::cmd::deploy_cmd::{
    DeleteDeploymentCmd, DeployCmd, GetDeploymentCmd, GetDeploymentResourceCmd,
    GetDeploymentResourceNamesCmd, GetDeploymentResourcesCmd,
};
use crate::cmd::process_definition_suspension::set_process_definition_suspension_state;
use crate::cmd::repository_model_cmd::{
    CreateRepositoryModelCmd, DeleteRepositoryModelCmd, GetRepositoryModelCmd,
    GetRepositoryModelSourceCmd, GetRepositoryModelSourceExtraCmd, GetRepositoryModelsCmd,
    UpdateRepositoryModelCmd, UpdateRepositoryModelSourceCmd, UpdateRepositoryModelSourceExtraCmd,
};
use crate::interceptor::command::Command;
use crate::interceptor::command_context::CommandContext;
use crate::interceptor::command_executor::{CommandExecutor, DefaultCommandExecutor};
use crate::persistence::runtime_store::RuntimeTimerJobState;
use crate::repository::deployment::Deployment;
use crate::repository::deployment_builder::DeploymentBuilder;
use crate::repository::deployment_resource::DeploymentResource;
use crate::repository::model::{RepositoryModel, RepositoryModelBytes};
use crate::repository::process_definition::ProcessDefinition;
use flowable_bpmn_model::model::BpmnModel;
use std::sync::Arc;

pub const PROCESS_DEFINITION_SUSPEND_TIMER_ACTIVITY_ID: &str = "process-definition-suspend";
pub const PROCESS_DEFINITION_ACTIVATE_TIMER_ACTIVITY_ID: &str = "process-definition-activate";
pub const PROCESS_DEFINITION_TIMER_INCLUDE_INSTANCES: &str = "include-process-instances";

pub struct RepositoryService {
    command_executor: Arc<DefaultCommandExecutor>,
}

struct SetProcessDefinitionSuspensionCmd {
    process_definition_id: String,
    suspended: bool,
    include_process_instances: bool,
}

impl Command<ProcessDefinition> for SetProcessDefinitionSuspensionCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<ProcessDefinition, crate::error::FlowableError> {
        set_process_definition_suspension_state(
            command_context,
            &self.process_definition_id,
            self.suspended,
            self.include_process_instances,
        )
    }
}

impl RepositoryService {
    pub fn new(command_executor: Arc<DefaultCommandExecutor>) -> Self {
        Self { command_executor }
    }

    /// Starts creating a new deployment
    pub fn create_deployment(&self) -> DeploymentBuilder {
        DeploymentBuilder::new()
    }

    /// Deploys a deployment via a DeployCmd using the CommandExecutor
    pub fn deploy(
        &self,
        builder: DeploymentBuilder,
    ) -> Result<Deployment, crate::error::FlowableError> {
        let deploy_cmd = DeployCmd::new(builder);
        self.command_executor.execute(&deploy_cmd)
    }

    /// Deletes the given deployment
    pub fn delete_deployment(
        &self,
        deployment_id: &str,
    ) -> Result<(), crate::error::FlowableError> {
        self.delete_deployment_with_cascade(deployment_id, false)
    }

    /// Deletes the given deployment, honouring the REST cascade flag at the service boundary.
    pub fn delete_deployment_with_cascade(
        &self,
        deployment_id: &str,
        cascade: bool,
    ) -> Result<(), crate::error::FlowableError> {
        let delete_cmd = DeleteDeploymentCmd::new_with_cascade(deployment_id.to_string(), cascade);
        self.command_executor.execute(&delete_cmd)
    }

    pub fn get_deployment(
        &self,
        deployment_id: &str,
    ) -> Result<Deployment, crate::error::FlowableError> {
        let get_cmd = GetDeploymentCmd::new(deployment_id.to_string());
        self.command_executor.execute(&get_cmd)
    }

    /// Retrieves a list of deployment resources
    pub fn get_deployment_resource_names(
        &self,
        deployment_id: &str,
    ) -> Result<Vec<String>, crate::error::FlowableError> {
        let get_cmd = GetDeploymentResourceNamesCmd::new(deployment_id.to_string());
        self.command_executor.execute(&get_cmd)
    }

    pub fn get_deployment_resources(
        &self,
        deployment_id: &str,
    ) -> Result<Vec<DeploymentResource>, crate::error::FlowableError> {
        let get_cmd = GetDeploymentResourcesCmd::new(deployment_id.to_string());
        self.command_executor.execute(&get_cmd)
    }

    pub fn get_deployment_resource(
        &self,
        deployment_id: &str,
        resource_name: &str,
    ) -> Result<DeploymentResource, crate::error::FlowableError> {
        let get_cmd =
            GetDeploymentResourceCmd::new(deployment_id.to_string(), resource_name.to_string());
        self.command_executor.execute(&get_cmd)
    }

    /// Retrieves all process definition IDs deployed (for testing)
    pub fn get_process_definition_ids(&self) -> Result<Vec<String>, crate::error::FlowableError> {
        let dm = self.command_executor.deployment_manager();
        let mut session = dm.create_session().unwrap();
        let pds = dm.get_process_definitions(&mut session);
        let mut ids: Vec<String> = pds.into_keys().collect();
        ids.sort();
        let _ = session.rollback();
        Ok(ids)
    }

    pub fn get_process_definitions(
        &self,
    ) -> Result<Vec<ProcessDefinition>, crate::error::FlowableError> {
        let dm = self.command_executor.deployment_manager();
        let mut session = dm.create_session().unwrap();
        let mut definitions = dm
            .get_process_definitions(&mut session)
            .into_values()
            .collect::<Vec<_>>();
        definitions.sort_by(|left, right| left.key.cmp(&right.key).then(left.id.cmp(&right.id)));
        let _ = session.rollback();
        Ok(definitions)
    }

    /// Retrieves all deployments, mirroring Java
    /// `repositoryService.createDeploymentQuery()` as used by REST
    /// `GET /repository/deployments` (default ordering by id).
    pub fn get_deployments(&self) -> Result<Vec<Deployment>, crate::error::FlowableError> {
        let dm = self.command_executor.deployment_manager();
        let mut session = dm.create_session().unwrap();
        let mut deployments = dm
            .get_deployments(&mut session)
            .into_values()
            .collect::<Vec<_>>();
        deployments.sort_by(|left, right| left.id.cmp(&right.id));
        let _ = session.rollback();
        Ok(deployments)
    }

    pub fn get_process_definition(
        &self,
        process_definition_id: &str,
    ) -> Result<ProcessDefinition, crate::error::FlowableError> {
        let dm = self.command_executor.deployment_manager();
        let mut session = dm.create_session().unwrap();
        let result = dm
            .get_process_definitions(&mut session)
            .remove(process_definition_id)
            .ok_or_else(|| {
                crate::error::FlowableError::NotFound(format!(
                    "Process definition '{}' was not found",
                    process_definition_id
                ))
            });
        let _ = session.rollback();
        result
    }

    pub fn update_process_definition_category(
        &self,
        process_definition_id: &str,
        category: Option<String>,
    ) -> Result<ProcessDefinition, crate::error::FlowableError> {
        let mut definition = self.get_process_definition(process_definition_id)?;
        definition.category = category;
        self.update_process_definition(definition)
    }

    pub fn set_process_definition_suspended(
        &self,
        process_definition_id: &str,
        suspended: bool,
    ) -> Result<ProcessDefinition, crate::error::FlowableError> {
        self.set_process_definition_suspended_with_instances(
            process_definition_id,
            suspended,
            false,
        )
    }

    pub fn set_process_definition_suspended_with_instances(
        &self,
        process_definition_id: &str,
        suspended: bool,
        include_process_instances: bool,
    ) -> Result<ProcessDefinition, crate::error::FlowableError> {
        self.command_executor
            .execute(&SetProcessDefinitionSuspensionCmd {
                process_definition_id: process_definition_id.to_string(),
                suspended,
                include_process_instances,
            })
    }

    pub fn schedule_process_definition_suspended(
        &self,
        process_definition_id: &str,
        suspended: bool,
        include_process_instances: bool,
        due_time: i64,
        time_date: String,
    ) -> Result<RuntimeTimerJobState, crate::error::FlowableError> {
        self.get_process_definition(process_definition_id)?;

        let job = RuntimeTimerJobState {
            timer_job_id: format!(
                "process-definition-{}:{}",
                if suspended { "suspend" } else { "activate" },
                uuid::Uuid::new_v4()
            ),
            process_instance_id: String::new(),
            execution_id: process_definition_id.to_string(),
            activity_id: if suspended {
                PROCESS_DEFINITION_SUSPEND_TIMER_ACTIVITY_ID.to_string()
            } else {
                PROCESS_DEFINITION_ACTIVATE_TIMER_ACTIVITY_ID.to_string()
            },
            job_state: Some("timer".to_string()),
            is_boundary: false,
            attached_activity_id: include_process_instances
                .then(|| PROCESS_DEFINITION_TIMER_INCLUDE_INSTANCES.to_string()),
            cancel_activity: false,
            time_duration: None,
            time_date: Some(time_date),
            time_cycle: None,
            end_date: None,
            due_time: Some(due_time),
            lock_owner: None,
            lock_time: None,
            lock_expiration_time: None,
            retries: Some(3),
            error_message: None,
            error_details: None,
            category: None,
            ..Default::default()
        };
        let store = self.command_executor.runtime_store();
        let mut session = store.create_session().unwrap();
        store.insert_timer_job_state(&job, &mut session);
        session.flush_and_commit().unwrap();
        Ok(job)
    }

    fn update_process_definition(
        &self,
        definition: ProcessDefinition,
    ) -> Result<ProcessDefinition, crate::error::FlowableError> {
        let dm = self.command_executor.deployment_manager();
        let mut session = dm.create_session().unwrap();
        dm.update_process_definition(definition.clone(), &mut session)
            .ok_or_else(|| {
                crate::error::FlowableError::NotFound(format!(
                    "Process definition '{}' was not found",
                    definition.id
                ))
            })?;
        session.flush_and_commit().unwrap();
        Ok(definition)
    }

    pub fn latest_process_definition_by_key(
        &self,
        process_definition_key: &str,
        tenant_id: Option<&str>,
    ) -> Result<Option<ProcessDefinition>, crate::error::FlowableError> {
        let mut matches = self
            .get_process_definitions()?
            .into_iter()
            .filter(|definition| definition.key == process_definition_key)
            .filter(|definition| definition.tenant_id.as_deref() == tenant_id)
            .collect::<Vec<_>>();
        matches.sort_by(|left, right| {
            left.version
                .cmp(&right.version)
                .then(left.id.cmp(&right.id))
        });
        Ok(matches.into_iter().last())
    }

    pub fn get_bpmn_model(
        &self,
        process_definition_id: &str,
    ) -> Result<Arc<BpmnModel>, crate::error::FlowableError> {
        self.command_executor
            .deployment_manager()
            .get_bpmn_model(process_definition_id)
            .ok_or_else(|| {
                crate::error::FlowableError::NotFound(format!(
                    "BPMN model for process definition '{}' was not found",
                    process_definition_id
                ))
            })
    }

    pub fn get_repository_models(
        &self,
    ) -> Result<Vec<RepositoryModel>, crate::error::FlowableError> {
        self.command_executor.execute(&GetRepositoryModelsCmd)
    }

    pub fn create_repository_model(
        &self,
        model: RepositoryModel,
    ) -> Result<RepositoryModel, crate::error::FlowableError> {
        let create_cmd = CreateRepositoryModelCmd::new(model);
        self.command_executor.execute(&create_cmd)
    }

    pub fn get_repository_model(
        &self,
        model_id: &str,
    ) -> Result<RepositoryModel, crate::error::FlowableError> {
        let get_cmd = GetRepositoryModelCmd::new(model_id.to_string());
        self.command_executor.execute(&get_cmd)
    }

    pub fn update_repository_model(
        &self,
        model: RepositoryModel,
    ) -> Result<RepositoryModel, crate::error::FlowableError> {
        let update_cmd = UpdateRepositoryModelCmd::new(model);
        self.command_executor.execute(&update_cmd)
    }

    pub fn delete_repository_model(
        &self,
        model_id: &str,
    ) -> Result<(), crate::error::FlowableError> {
        let delete_cmd = DeleteRepositoryModelCmd::new(model_id.to_string());
        self.command_executor.execute(&delete_cmd)
    }

    pub fn get_repository_model_source(
        &self,
        model_id: &str,
    ) -> Result<RepositoryModelBytes, crate::error::FlowableError> {
        let get_cmd = GetRepositoryModelSourceCmd::new(model_id.to_string());
        self.command_executor.execute(&get_cmd)
    }

    pub fn get_repository_model_source_extra(
        &self,
        model_id: &str,
    ) -> Result<RepositoryModelBytes, crate::error::FlowableError> {
        let get_cmd = GetRepositoryModelSourceExtraCmd::new(model_id.to_string());
        self.command_executor.execute(&get_cmd)
    }

    pub fn update_repository_model_source(
        &self,
        model_id: &str,
        content_type: String,
        bytes: Vec<u8>,
    ) -> Result<(), crate::error::FlowableError> {
        let update_cmd =
            UpdateRepositoryModelSourceCmd::new(model_id.to_string(), content_type, bytes);
        self.command_executor.execute(&update_cmd)
    }

    pub fn update_repository_model_source_extra(
        &self,
        model_id: &str,
        content_type: String,
        bytes: Vec<u8>,
    ) -> Result<(), crate::error::FlowableError> {
        let update_cmd =
            UpdateRepositoryModelSourceExtraCmd::new(model_id.to_string(), content_type, bytes);
        self.command_executor.execute(&update_cmd)
    }
}
