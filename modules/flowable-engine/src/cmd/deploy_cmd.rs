use crate::bpmn::event_registry_correlation::{
    correlation_key_from_base_element, extension_element_text, is_manual_subscription,
    ELEMENT_EVENT_TYPE,
};
use crate::bpmn::job_category::resolve_job_category;
use crate::bpmn::timer_util;
use crate::engine::deployer::bpmn_deployer::BpmnDeployer;
use crate::engine::runtime_service::BulkDeleteProcessInstancesCmd;
use crate::interceptor::command::Command;
use crate::interceptor::command_context::CommandContext;
use crate::persistence::runtime_store::{
    EventSubscriptionKind, ProcessEventStartSubscription, ProcessTimerStartSubscription,
};
use crate::repository::deployment::Deployment;
use crate::repository::deployment_builder::DeploymentBuilder;
use crate::repository::deployment_resource::DeploymentResource;
use crate::repository::model::RepositoryModel;
use crate::runtime::execution::Execution;
use flowable_bpmn_model::model::{EventDefinitionEnum, FlowElementEnum};

pub struct DeployCmd {
    deployment_builder: DeploymentBuilder,
}

impl DeployCmd {
    pub fn new(deployment_builder: DeploymentBuilder) -> Self {
        Self { deployment_builder }
    }

    fn find_duplicate_deployment(
        &self,
        candidate: &Deployment,
        command_context: &mut CommandContext,
    ) -> Option<Deployment> {
        let mut latest = command_context
            .deployment_manager
            .get_deployments(&mut command_context.session)
            .into_values()
            .filter(|deployment| {
                deployment.name == candidate.name && deployment.tenant_id == candidate.tenant_id
            })
            .max_by(|left, right| {
                left.deployment_time
                    .cmp(&right.deployment_time)
                    .then_with(|| left.id.cmp(&right.id))
            })?;

        if latest.resources != candidate.resources {
            return None;
        }

        // Java DeployCmd returns the persisted deployment without running the
        // deployer when duplicate filtering finds identical resource bytes.
        latest.is_new = false;
        Some(latest)
    }
}

/// Extract timer-start subscriptions from a BPMN model.
///
/// P17: evaluates `timeDate` / `timeDuration` / `timeCycle` / `endDate` as UEL
/// against an empty execution (deploy-time has no process variables; string
/// literals like `${'2036-…'}` still resolve). Evaluation failure aborts deploy.
pub(crate) fn extract_timer_start_subscriptions(
    process_definition_id: &str,
    process_definition_key: &str,
    models: &std::collections::HashMap<
        String,
        std::sync::Arc<flowable_bpmn_model::model::BpmnModel>,
    >,
    time_source: &dyn crate::engine::time_source::TimeSource,
    calendars: &crate::engine::business_calendar::BusinessCalendarRegistry,
) -> Result<Vec<ProcessTimerStartSubscription>, crate::error::FlowableError> {
    let mut subscriptions = Vec::new();

    let bpmn_model = match models.get(process_definition_id) {
        Some(m) => m,
        None => return Ok(subscriptions),
    };

    let process = match bpmn_model.main_process.as_ref() {
        Some(p) => p,
        None => return Ok(subscriptions),
    };

    let now = time_source.now();
    // Deploy-time: no process variables (Java DefinitionVariableContainer / empty scope).
    let empty_execution = Execution::default();

    for flow_element in &process.flow_elements {
        if let FlowElementEnum::StartEvent(start_event) = flow_element {
            for event_def in &start_event.event.event_definitions {
                if let EventDefinitionEnum::TimerEventDefinition(timer_def) = event_def {
                    let start_event_id = start_event
                        .event
                        .flow_node
                        .flow_element
                        .base_element
                        .id
                        .clone()
                        .unwrap_or_default();
                    let start_event_name = start_event.event.flow_node.flow_element.name.clone();

                    let category = resolve_job_category(
                        &start_event.event.flow_node.flow_element.base_element,
                        &empty_execution,
                    );
                    // EL first, then P16 prepare_repeat inside resolve_timer_schedule.
                    let schedule = timer_util::resolve_timer_schedule_for_start(
                        timer_def.time_date.as_ref(),
                        timer_def.time_duration.as_ref(),
                        timer_def.time_cycle.as_ref(),
                        timer_def.end_date.as_ref(),
                        timer_def.calendar_name.as_ref(),
                        &empty_execution,
                        calendars,
                        now,
                    )?;
                    subscriptions.push(ProcessTimerStartSubscription {
                        id: uuid::Uuid::new_v4().to_string(),
                        process_definition_id: process_definition_id.to_string(),
                        process_definition_key: process_definition_key.to_string(),
                        start_event_id,
                        start_event_name,
                        interrupting: start_event.interrupting,
                        time_duration: schedule.time_duration,
                        time_date: schedule.time_date,
                        time_cycle: schedule.time_cycle,
                        end_date: schedule.end_date,
                        calendar_name: schedule.calendar_name,
                        due_time: schedule.due_time,
                        lock_owner: None,
                        lock_time: None,
                        category,
                    });
                }
            }
        }
    }

    Ok(subscriptions)
}

fn extract_event_start_subscriptions(
    process_definition_id: &str,
    process_definition_key: &str,
    tenant_id: Option<&str>,
    models: &std::collections::HashMap<
        String,
        std::sync::Arc<flowable_bpmn_model::model::BpmnModel>,
    >,
) -> Vec<ProcessEventStartSubscription> {
    let mut subscriptions = Vec::new();

    let bpmn_model = match models.get(process_definition_id) {
        Some(m) => m,
        None => return subscriptions,
    };

    let process = match bpmn_model.main_process.as_ref() {
        Some(p) => p,
        None => return subscriptions,
    };

    for flow_element in &process.flow_elements {
        if let FlowElementEnum::StartEvent(start_event) = flow_element {
            let start_event_id = start_event
                .event
                .flow_node
                .flow_element
                .base_element
                .id
                .clone()
                .unwrap_or_default();
            let start_event_name = start_event.event.flow_node.flow_element.name.clone();
            let base_element = &start_event.event.flow_node.flow_element.base_element;

            let extensions = &start_event
                .event
                .flow_node
                .flow_element
                .base_element
                .extension_elements;
            // Deploy-time correlation key: CorrelationUtil with execution=null
            // stores raw value expressions (CorrelationUtil.java:53-54).
            let configuration = correlation_key_from_base_element(
                &start_event.event.flow_node.flow_element.base_element,
                None,
            );

            // Java EventSubscriptionManager.insertEventRegistryEvent:224-249 —
            // process-level start with flowable:eventType (and no event definitions).
            // Manual correlation configuration skips registration (:226-231).
            if start_event.event.event_definitions.is_empty() {
                if let Some(event_type) =
                    crate::bpmn::behavior::event_registry_event_support::resolve_event_type_extension(
                        base_element,
                    )
                {
                    if !crate::bpmn::behavior::event_registry_event_support::is_manual_event_registry_start_correlation(
                        base_element,
                    ) {
                        subscriptions.push(ProcessEventStartSubscription {
                            process_definition_id: process_definition_id.to_string(),
                            process_definition_key: process_definition_key.to_string(),
                            tenant_id: tenant_id.map(str::to_string),
                            start_event_id: start_event_id.clone(),
                            start_event_name: start_event_name.clone(),
                            event_kind: EventSubscriptionKind::EventRegistry,
                            event_ref: event_type,
                            configuration: configuration.clone(),
                        });
                    }
                }
                continue;
            }

            let mut registered_standard = false;
            for event_def in &start_event.event.event_definitions {
                match event_def {
                    EventDefinitionEnum::MessageEventDefinition(msg_def) => {
                        if let Some(ref msg_ref) = msg_def.message_ref {
                            // Resolve message name from model definition (consistent with catch events)
                            let event_ref = bpmn_model
                                .messages
                                .iter()
                                .find(|m| m.base_element.id.as_deref() == Some(msg_ref))
                                .and_then(|m| m.name.clone())
                                .unwrap_or_else(|| msg_ref.clone());

                            subscriptions.push(ProcessEventStartSubscription {
                                process_definition_id: process_definition_id.to_string(),
                                process_definition_key: process_definition_key.to_string(),
                                tenant_id: tenant_id.map(str::to_string),
                                start_event_id: start_event_id.clone(),
                                start_event_name: start_event_name.clone(),
                                event_kind: EventSubscriptionKind::Message,
                                event_ref,
                                configuration: configuration.clone(),
                            });
                            registered_standard = true;
                        }
                    }
                    EventDefinitionEnum::SignalEventDefinition(sig_def) => {
                        if let Some(ref sig_ref) = sig_def.signal_ref {
                            subscriptions.push(ProcessEventStartSubscription {
                                process_definition_id: process_definition_id.to_string(),
                                process_definition_key: process_definition_key.to_string(),
                                tenant_id: tenant_id.map(str::to_string),
                                start_event_id: start_event_id.clone(),
                                start_event_name: start_event_name.clone(),
                                event_kind: EventSubscriptionKind::Signal,
                                event_ref: sig_ref.clone(),
                                configuration: configuration.clone(),
                            });
                            registered_standard = true;
                        }
                    }
                    _ => {}
                }
            }

            // Event-registry start: `flowable:eventType` extension
            // (Java EventSubscriptionManager.insertEventRegistryEvent:224-248).
            // Reachable only when the start has event definitions but none of
            // them is message/signal (the empty-definitions case is handled by
            // the P92 block above). Uses the `EventRegistry` kind introduced by
            // P92 (P93's Message-kind fallback predates that variant).
            if !registered_standard
                && let Some(event_type) = extension_element_text(extensions, ELEMENT_EVENT_TYPE)
            {
                // manualSubscription skips auto-registration (:226-230).
                if !is_manual_subscription(extensions) {
                    subscriptions.push(ProcessEventStartSubscription {
                        process_definition_id: process_definition_id.to_string(),
                        process_definition_key: process_definition_key.to_string(),
                        tenant_id: tenant_id.map(str::to_string),
                        start_event_id: start_event_id.clone(),
                        start_event_name: start_event_name.clone(),
                        event_kind: EventSubscriptionKind::EventRegistry,
                        event_ref: event_type,
                        configuration,
                    });
                }
            }
        }
    }

    subscriptions
}

impl Command<Deployment> for DeployCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<Deployment, crate::error::FlowableError> {
        let deployment = self.deployment_builder.clone().deploy();
        if self.deployment_builder.duplicate_filtering_enabled()
            && let Some(existing) = self.find_duplicate_deployment(&deployment, command_context)
        {
            return Ok(existing);
        }

        let bpmn_deployer = BpmnDeployer::new();
        let process_definitions = bpmn_deployer.deploy(
            &deployment,
            &command_context.deployment_manager,
            &mut command_context.session,
        )?;

        let mut model_info = Vec::new();

        for (def, model) in process_definitions {
            crate::validation::structural_model_validator::StructuralModelValidator::validate(
                &model,
            )?;
            crate::validation::unsupported_model_validator::UnsupportedModelValidator::validate(
                &model,
                &command_context.config,
            )?;

            let id = def.id.clone();
            let key = def.key.clone();
            let tenant_id = def.tenant_id.clone();
            let resource_name = def.resource_name.clone();
            let deployed_at = deployment
                .deployment_time
                .map(|time| time.timestamp_millis())
                .unwrap_or_default();
            let source_bytes = resource_name
                .as_ref()
                .and_then(|name| deployment.resources.get(name))
                .cloned()
                .unwrap_or_default();
            let source_content_type = resource_name
                .as_ref()
                .map(|name| {
                    DeploymentResource::new(
                        deployment.id.clone(),
                        name.clone(),
                        Vec::new(),
                        deployed_at,
                    )
                    .content_type
                })
                .unwrap_or_else(|| "application/octet-stream".to_string());
            let source_extra_bytes = serde_json::to_vec(&model)
                .map_err(|error| crate::error::FlowableError::ExecutionError(error.to_string()))?;
            let repository_model = RepositoryModel {
                id: id.clone(),
                name: def.name.clone(),
                key: key.clone(),
                category: def.category.clone(),
                version: def.version,
                meta_info: Some(
                    serde_json::json!({
                        "name": def.name.clone(),
                        "description": def.description.clone(),
                    })
                    .to_string(),
                ),
                deployment_id: def.deployment_id.clone(),
                resource_name,
                process_definition_id: Some(id.clone()),
                tenant_id: def.tenant_id.clone(),
                create_time: deployed_at,
                last_update_time: deployed_at,
                source_content_type,
                source_extra_content_type: "application/json".to_string(),
            };
            command_context
                .deployment_manager
                .insert_process_definition(def, &mut command_context.session);
            command_context.deployment_manager.insert_repository_model(
                repository_model,
                source_bytes,
                source_extra_bytes,
                &mut command_context.session,
            );
            command_context
                .deployment_manager
                .insert_bpmn_model(&id, model);
            model_info.push((id, key, tenant_id));
        }

        // Java TimerManager.removeObsoleteTimers: cancel prior versions' timer
        // start jobs for the same process-definition key (+ tenant) before
        // registering the new version's subscriptions.
        // Java BpmnDeploymentHelper.addEventRegistrations →
        // EventSubscriptionManager.removeObsoleteMessageEventSubscriptions /
        // removeObsoleteSignalEventSubscription (EventSubscriptionManager.java:55-67,
        // 122-133): the same applies to message/signal start subscriptions.
        for (_id, key, tenant_id) in &model_info {
            command_context
                .deployment_manager
                .delete_timer_start_subscriptions_by_process_definition_key(
                    key,
                    tenant_id.as_deref(),
                    &mut command_context.session,
                );
            command_context
                .deployment_manager
                .delete_event_start_subscriptions_by_process_definition_key(
                    key,
                    tenant_id.as_deref(),
                    &mut command_context.session,
                );
        }

        let (all_timer_start_subscriptions, all_event_start_subscriptions) = {
            let time_source = command_context.runtime_store.time_source();
            let calendars = command_context.config.business_calendar_registry.clone();
            let mut extract_error: Option<crate::error::FlowableError> = None;
            let result = command_context.deployment_manager.with_bpmn_models(|models| {
                let mut all_timer = Vec::new();
                let mut all_event = Vec::new();
                for (id, key, tenant_id) in &model_info {
                    match extract_timer_start_subscriptions(
                        id,
                        key,
                        models,
                        time_source.as_ref(),
                            &calendars,
                    ) {
                        Ok(timer_subs) => all_timer.extend(timer_subs),
                        Err(e) => {
                            extract_error = Some(e);
                            return (Vec::new(), Vec::new());
                        }
                    }

                    let event_subs =
                        extract_event_start_subscriptions(id, key, tenant_id.as_deref(), models);
                    all_event.extend(event_subs);
                }
                (all_timer, all_event)
            });
            if let Some(e) = extract_error {
                return Err(e);
            }
            result
        };

        if !all_timer_start_subscriptions.is_empty() {
            command_context
                .deployment_manager
                .register_timer_start_subscriptions(
                    all_timer_start_subscriptions,
                    &mut command_context.session,
                );
        }

        if !all_event_start_subscriptions.is_empty() {
            command_context
                .deployment_manager
                .register_event_start_subscriptions(
                    all_event_start_subscriptions,
                    &mut command_context.session,
                );
        }

        command_context
            .deployment_manager
            .register_deployment(deployment.clone(), &mut command_context.session);

        Ok(deployment)
    }
}

pub struct DeleteDeploymentCmd {
    deployment_id: String,
    cascade: bool,
}

impl DeleteDeploymentCmd {
    pub fn new(deployment_id: String) -> Self {
        Self {
            deployment_id,
            cascade: false,
        }
    }

    pub fn new_with_cascade(deployment_id: String, cascade: bool) -> Self {
        Self {
            deployment_id,
            cascade,
        }
    }
}

impl Command<()> for DeleteDeploymentCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<(), crate::error::FlowableError> {
        if command_context
            .deployment_manager
            .get_deployment(&self.deployment_id, &mut command_context.session)
            .is_none()
        {
            return Err(crate::error::FlowableError::NotFound(format!(
                "Deployment '{}' was not found",
                self.deployment_id
            )));
        }

        if self.cascade {
            let process_definition_ids = command_context
                .deployment_manager
                .get_process_definitions(&mut command_context.session)
                .into_values()
                .filter(|definition| {
                    definition.deployment_id.as_deref() == Some(self.deployment_id.as_str())
                })
                .map(|definition| definition.id)
                .collect::<std::collections::HashSet<_>>();

            let process_instance_ids = command_context
                .runtime_store
                .snapshot_process_instances(&mut command_context.session)
                .into_values()
                .filter(|instance| {
                    process_definition_ids.contains(instance.process_definition_id.as_str())
                })
                .map(|instance| instance.id)
                .collect::<Vec<_>>();

            if !process_instance_ids.is_empty() {
                // Java's `DeploymentEntityManagerImpl.deleteDeployment` invokes
                // `deleteProcessInstancesForProcessDefinitions` (cascade=true) which
                // routes through `deleteProcessInstanceCascade(deleteHistory=true)`,
                // deleting historic PI/tasks/activities entirely rather than marking
                // them ended. Propagate the cascade flag so BulkDeleteProcessInstancesCmd
                // mirrors that path; without it the historic rows would linger as
                // "ended" instead of being purged.
                BulkDeleteProcessInstancesCmd::new_with_cascade(
                    process_instance_ids,
                    Some(format!("Deployment '{}' deleted", self.deployment_id)),
                    self.cascade,
                )
                .execute(command_context)?;
            }
        }

        // Capture restore plan before definitions/models are removed.
        // Java DeploymentProcessDefinitionDeletionManagerImpl:
        // restorePreviousStartEventsIfNeeded — only when deleting the latest version.
        let restore_plan = {
            let defs = command_context
                .deployment_manager
                .get_process_definitions(&mut command_context.session);
            let deleting: Vec<_> = defs
                .values()
                .filter(|d| d.deployment_id.as_deref() == Some(self.deployment_id.as_str()))
                .cloned()
                .collect();
            let mut restore_ids = Vec::new();
            for pd in &deleting {
                let same_key: Vec<_> = defs
                    .values()
                    .filter(|d| {
                        d.key == pd.key && d.tenant_id.as_deref() == pd.tenant_id.as_deref()
                    })
                    .collect();
                let is_latest = same_key
                    .iter()
                    .all(|d| d.version < pd.version || d.id == pd.id);
                if !is_latest {
                    continue;
                }
                // Previous version = highest version strictly below the deleted one.
                if let Some(prev) = same_key
                    .into_iter()
                    .filter(|d| d.version < pd.version)
                    .max_by_key(|d| d.version)
                {
                    restore_ids.push((
                        prev.id.clone(),
                        prev.key.clone(),
                        prev.tenant_id.clone(),
                    ));
                }
            }
            restore_ids
        };

        command_context
            .deployment_manager
            .delete_deployment(&self.deployment_id, &mut command_context.session);

        // Restore previous-version timer + message/signal start subscriptions
        // after latest is gone. Java DeploymentProcessDefinitionDeletionManagerImpl
        // .restorePreviousStartEventsIfNeeded (:111-155) re-registers timer (:127),
        // signal (:133) and message (:135) start events of the previous version.
        if !restore_plan.is_empty() {
            let time_source = command_context.runtime_store.time_source();
            let calendars = command_context.config.business_calendar_registry.clone();
            let mut restore_error: Option<crate::error::FlowableError> = None;
            let (restored, restored_events) =
                command_context.deployment_manager.with_bpmn_models(|models| {
                    let mut all = Vec::new();
                    let mut all_events = Vec::new();
                    for (id, key, tenant) in &restore_plan {
                        match extract_timer_start_subscriptions(
                            id,
                            key,
                            models,
                            time_source.as_ref(),
                                &calendars,
                        ) {
                            Ok(subs) => all.extend(subs),
                            Err(e) => {
                                restore_error = Some(e);
                                return (Vec::new(), Vec::new());
                            }
                        }
                        all_events.extend(extract_event_start_subscriptions(
                            id,
                            key,
                            tenant.as_deref(),
                            models,
                        ));
                    }
                    (all, all_events)
                });
            if let Some(e) = restore_error {
                return Err(e);
            }
            if !restored.is_empty() {
                command_context
                    .deployment_manager
                    .register_timer_start_subscriptions(restored, &mut command_context.session);
            }
            if !restored_events.is_empty() {
                command_context
                    .deployment_manager
                    .register_event_start_subscriptions(
                        restored_events,
                        &mut command_context.session,
                    );
            }
        }

        Ok(())
    }
}

pub struct GetDeploymentCmd {
    deployment_id: String,
}

impl GetDeploymentCmd {
    pub fn new(deployment_id: String) -> Self {
        Self { deployment_id }
    }
}

impl Command<Deployment> for GetDeploymentCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<Deployment, crate::error::FlowableError> {
        command_context
            .deployment_manager
            .get_deployment(&self.deployment_id, &mut command_context.session)
            .ok_or_else(|| {
                crate::error::FlowableError::NotFound(format!(
                    "Deployment '{}' was not found",
                    self.deployment_id
                ))
            })
    }
}

pub struct GetDeploymentResourceNamesCmd {
    deployment_id: String,
}

impl GetDeploymentResourceNamesCmd {
    pub fn new(deployment_id: String) -> Self {
        Self { deployment_id }
    }
}

impl Command<Vec<String>> for GetDeploymentResourceNamesCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<Vec<String>, crate::error::FlowableError> {
        Ok(command_context
            .deployment_manager
            .get_deployment_resource_names(&self.deployment_id, &mut command_context.session))
    }
}

pub struct GetDeploymentResourcesCmd {
    deployment_id: String,
}

impl GetDeploymentResourcesCmd {
    pub fn new(deployment_id: String) -> Self {
        Self { deployment_id }
    }
}

impl Command<Vec<DeploymentResource>> for GetDeploymentResourcesCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<Vec<DeploymentResource>, crate::error::FlowableError> {
        if command_context
            .deployment_manager
            .get_deployment(&self.deployment_id, &mut command_context.session)
            .is_none()
        {
            return Err(crate::error::FlowableError::NotFound(format!(
                "Deployment '{}' was not found",
                self.deployment_id
            )));
        }

        Ok(command_context
            .deployment_manager
            .get_deployment_resources(&self.deployment_id, &mut command_context.session))
    }
}

pub struct GetDeploymentResourceCmd {
    deployment_id: String,
    resource_name: String,
}

impl GetDeploymentResourceCmd {
    pub fn new(deployment_id: String, resource_name: String) -> Self {
        Self {
            deployment_id,
            resource_name,
        }
    }
}

impl Command<DeploymentResource> for GetDeploymentResourceCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<DeploymentResource, crate::error::FlowableError> {
        if command_context
            .deployment_manager
            .get_deployment(&self.deployment_id, &mut command_context.session)
            .is_none()
        {
            return Err(crate::error::FlowableError::NotFound(format!(
                "Deployment '{}' was not found",
                self.deployment_id
            )));
        }

        command_context
            .deployment_manager
            .get_deployment_resource(
                &self.deployment_id,
                &self.resource_name,
                &mut command_context.session,
            )
            .ok_or_else(|| {
                crate::error::FlowableError::NotFound(format!(
                    "Resource '{}' was not found in deployment '{}'",
                    self.resource_name, self.deployment_id
                ))
            })
    }
}
