use crate::agenda::FlowableEngineAgenda;
use crate::agenda::continue_process_operation::{
    ASYNC_CONTINUATION_JOB_STATE, ASYNC_CONTINUATION_JOB_TYPE_MARKER,
};
use crate::bpmn::fault::register_process_event_subprocesses;
use crate::bpmn::job_category::resolve_job_category;
use crate::engine::event_dispatcher::{
    dispatch_process_instance_created, dispatch_process_instance_initialized,
};
use crate::engine::variable_service::variable_type_name;
use crate::identity::entities::IdentityLink;
use crate::interceptor::command::Command;
use crate::interceptor::command_context::CommandContext;
use crate::persistence::runtime_store::RuntimeTimerJobState;
use crate::repository::process_definition::ProcessDefinition;
use crate::runtime::execution::Execution;
use crate::runtime::process_instance::ProcessInstance;
use crate::runtime::process_instance_builder::ProcessInstanceBuilder;
use chrono::Utc;
use flowable_bpmn_model::model::FlowElementEnum;

/// Creates Java's process-instance `starter` link inside the same command and
/// database transaction as the process instance itself.
fn insert_starter_identity_link(
    command_context: &mut CommandContext,
    process_instance_id: &str,
    start_user_id: Option<&str>,
) {
    let Some(start_user_id) = start_user_id else {
        return;
    };

    let link = IdentityLink {
        id: uuid::Uuid::new_v4().to_string(),
        link_type: "starter".to_string(),
        user_id: Some(start_user_id.to_string()),
        group_id: None,
        task_id: None,
        process_instance_id: Some(process_instance_id.to_string()),
        process_definition_id: None,
    };
    // P77: Java DefaultIdentityLinkInterceptor / IdentityLinkUtil
    // .createProcessInstanceIdentityLink → recordIdentityLinkCreated.
    command_context
        .history_manager
        .record_identity_link_created(&link, &mut command_context.session);
    command_context
        .runtime_store
        .insert_identity_link(link, &mut command_context.session);
}

fn resolve_start_event_id_optional(
    command_context: &CommandContext,
    process_definition_id: &str,
) -> Option<String> {
    let bpmn_model = command_context
        .deployment_manager
        .get_bpmn_model(process_definition_id)?;
    let main_process = bpmn_model.main_process.as_ref()?;

    main_process
        .flow_elements
        .iter()
        .find_map(|element| match element {
            FlowElementEnum::StartEvent(start_event) => start_event
                .event
                .flow_node
                .flow_element
                .base_element
                .id
                .clone(),
            _ => None,
        })
}

/// Java `ProcessInstanceHelper.java:197-199` — read StartEvent.initiator for the
/// start activity used to create this process instance.
fn resolve_start_event_initiator(
    command_context: &CommandContext,
    process_definition_id: &str,
    start_event_id: &str,
) -> Option<String> {
    let bpmn_model = command_context
        .deployment_manager
        .get_bpmn_model(process_definition_id)?;
    let main_process = bpmn_model.main_process.as_ref()?;
    let element =
        crate::agenda::continue_process_operation::find_flow_element(main_process, start_event_id)?;
    match element {
        FlowElementEnum::StartEvent(start_event) => start_event
            .initiator
            .as_ref()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty()),
        _ => None,
    }
}

/// Java `ExecutionEntityManagerImpl.java:298-300` — write authenticated user id
/// into the initiator variable on the PI scope execution.
///
/// **Rust deviation:** Java always writes (including null when unauthenticated).
/// Rust only writes when `start_user_id` is present (REST Basic Auth always has a
/// user; API callers that omit it skip the write).
fn apply_start_event_initiator(
    command_context: &CommandContext,
    process_definition_id: &str,
    start_event_id: &str,
    start_user_id: Option<&str>,
    root_execution: &mut Execution,
) -> Option<(String, serde_json::Value)> {
    let Some(user_id) = start_user_id.filter(|s| !s.is_empty()) else {
        return None;
    };
    let Some(initiator_name) =
        resolve_start_event_initiator(command_context, process_definition_id, start_event_id)
    else {
        return None;
    };
    let value = serde_json::Value::String(user_id.to_string());
    // PI scope: root execution is the process-instance row.
    root_execution.set_process_variable(initiator_name.clone(), value.clone());
    Some((initiator_name, value))
}

/// Record historic variable for initiator (mirrors builder.variables loop).
fn record_initiator_variable_history(
    command_context: &mut CommandContext,
    process_instance_id: &str,
    execution_id: &str,
    initiator: Option<(String, serde_json::Value)>,
) {
    let Some((name, value)) = initiator else {
        return;
    };
    // Skip if builder.variables already recorded the same name (unlikely but safe).
    let var_id = uuid::Uuid::new_v4().to_string();
    command_context.history_manager.record_variable_created(
        &var_id,
        &name,
        variable_type_name(&value),
        value,
        process_instance_id,
        Some(execution_id),
        None,
        &mut command_context.session,
    );
}

fn resolve_process_definition(
    command_context: &mut CommandContext,
    builder: &ProcessInstanceBuilder,
) -> Result<Option<ProcessDefinition>, crate::error::FlowableError> {
    let process_definitions = command_context
        .deployment_manager
        .get_process_definitions(&mut command_context.session);

    if let Some(process_definition_id) = builder.process_definition_id.as_deref() {
        return Ok(process_definitions.get(process_definition_id).cloned());
    }

    let Some(process_definition_key) = builder.process_definition_key.as_deref() else {
        return Ok(None);
    };

    Ok(process_definitions
        .values()
        .filter(|definition| definition.key == process_definition_key)
        .filter(|definition| definition.tenant_id.as_deref() == builder.tenant_id.as_deref())
        .max_by(|left, right| {
            left.version
                .cmp(&right.version)
                .then(left.id.cmp(&right.id))
        })
        .cloned())
}

pub struct StartProcessInstanceCmd {
    process_instance_builder: ProcessInstanceBuilder,
    override_start_event_id: Option<String>,
}

impl StartProcessInstanceCmd {
    pub fn new(process_instance_builder: ProcessInstanceBuilder) -> Self {
        Self {
            process_instance_builder,
            override_start_event_id: None,
        }
    }

    pub fn with_start_event_id(
        process_instance_builder: ProcessInstanceBuilder,
        start_event_id: String,
    ) -> Self {
        Self {
            process_instance_builder,
            override_start_event_id: Some(start_event_id),
        }
    }
}

impl Command<ProcessInstance> for StartProcessInstanceCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<ProcessInstance, crate::error::FlowableError> {
        let builder = self.process_instance_builder.clone();
        let process_definition = resolve_process_definition(command_context, &builder)?
            .ok_or_else(|| {
                crate::error::FlowableError::NotFound(
                    "No process definition found for process instance start request".to_string(),
                )
            })?;

        // Java parity: ProcessInstanceHelper lines 96-98 checks if definition is suspended
        if process_definition.is_suspended {
            return Err(crate::error::FlowableError::ExecutionError(format!(
                "Cannot start process instance. Process definition {} (id = {}) is suspended",
                process_definition.name.as_deref().unwrap_or("<unnamed>"),
                process_definition.id
            )));
        }

        let process_definition_id = process_definition.id.clone();

        let start_event_id = match self.override_start_event_id.clone() {
            Some(id) => id,
            None => {
                match resolve_start_event_id_optional(command_context, &process_definition_id) {
                    Some(id) => id,
                    None => {
                        return Err(crate::error::FlowableError::NotFound(format!(
                            "No StartEvent found in deployed model for process definition ID: {}",
                            process_definition_id
                        )));
                    }
                }
            }
        };

        let process_instance = ProcessInstance {
            id: uuid::Uuid::new_v4().to_string(),
            name: builder.name.clone(),
            process_definition_id: process_definition_id.clone(),
            process_definition_key: builder
                .process_definition_key
                .clone()
                .unwrap_or_else(|| process_definition.key.clone()),
            process_definition_name: process_definition.name.clone(),
            process_definition_version: process_definition.version,
            business_key: builder.business_key.clone(),
            business_status: builder.business_status.clone(),
            is_suspended: false,
            tenant_id: builder
                .tenant_id
                .clone()
                .or_else(|| process_definition.tenant_id.clone()),
            start_time: Some(Utc::now()),
            start_user_id: builder.start_user_id.clone(),
            callback_id: builder.callback_id.clone(),
            callback_type: builder.callback_type.clone(),
            reference_id: builder.reference_id.clone(),
            reference_type: builder.reference_type.clone(),
            is_ended: false,
            super_execution_id: builder.super_execution_id.clone(),
            root_process_instance_id: Some(process_definition_id.clone()), // we will use id below
        };

        // Fix root_process_instance_id after we have the id
        let mut process_instance = process_instance;
        process_instance.root_process_instance_id = Some(process_instance.id.clone());

        let mut root_execution = Execution {
            id: process_instance.id.clone(),
            parent_id: None,
            super_execution_id: builder.super_execution_id.clone(),
            root_process_instance_id: Some(process_instance.id.clone()),
            process_instance_id: Some(process_instance.id.clone()),
            process_definition_id: Some(process_instance.process_definition_id.clone()),
            process_definition_key: Some(process_instance.process_definition_key.clone()),
            process_definition_name: process_instance.process_definition_name.clone(),
            process_definition_version: Some(process_instance.process_definition_version),
            activity_id: Some(start_event_id.clone()),
            activity_name: None,
            name: builder.name,
            description: None,
            is_suspended: false,
            is_ended: false,
            is_active: true,
            is_concurrent: false,
            is_scope: true,
            is_multi_instance_root: false,
            tenant_id: process_instance.tenant_id.clone(),
            ..Default::default()
        };
        root_execution.set_process_variables(builder.variables.clone());
        // Java CallActivityBehavior#initializeTransientVariables — applied at
        // start so inheritVariables/in-parameter transient flags stick.
        for (name, value) in &builder.transient_variables {
            root_execution.set_transient_variable(name.clone(), value.clone());
        }

        // Initialize data objects as process variables
        {
            if let Some(bpmn_model) = command_context
                .deployment_manager
                .get_bpmn_model(&process_definition_id)
                && let Some(main_process) = bpmn_model.main_process.as_ref()
            {
                // Java ProcessInstanceHelper#processDataObjects: copy converter-typed
                // values as-is — no EL evaluation of `${...}`.
                for data_object in &main_process.data_objects {
                    if let Some(name) = &data_object.name {
                        if root_execution.process_variable(name).is_some() {
                            continue;
                        }
                        let value = data_object.value.clone().unwrap_or(serde_json::Value::Null);
                        root_execution.variables.insert(name.to_string(), value);
                    }
                }
            }
        }

        // Java ProcessInstanceHelper.java:197-199 + ExecutionEntityManagerImpl.java:298-300
        let initiator_var = apply_start_event_initiator(
            command_context,
            &process_definition_id,
            &start_event_id,
            builder.start_user_id.as_deref(),
            &mut root_execution,
        );

        command_context
            .execution_entity_manager
            .insert(&root_execution, &mut command_context.session);

        register_process_event_subprocesses(&root_execution, command_context)?;

        command_context
            .agenda
            .plan_continue_process_operation(root_execution.clone());

        command_context
            .runtime_store
            .insert_process_instance(&process_instance, &mut command_context.session);

        insert_starter_identity_link(
            command_context,
            &process_instance.id,
            process_instance.start_user_id.as_deref(),
        );

        // Java parity: ProcessInstanceHelper.java:227-275 dispatches
        // ENTITY_INITIALIZED + PROCESS_CREATED once the PI row is inserted
        // and execution row is set up, BEFORE the start event fires.
        dispatch_process_instance_initialized(
            command_context,
            &process_instance.id,
            &process_instance.process_definition_id,
        );
        dispatch_process_instance_created(
            command_context,
            &process_instance.id,
            &process_instance.process_definition_id,
        );

        command_context
            .history_manager
            .record_process_instance_start(
                &process_instance.id,
                &process_instance.process_definition_id,
                process_instance.business_key.as_deref(),
                process_instance.start_user_id.as_deref(),
                &mut command_context.session,
            );

        command_context.history_manager.record_audit_event(
            "start",
            Some(&process_instance.id),
            Some(&process_instance.process_definition_id),
            Some("Process instance started"),
            &mut command_context.session,
        );

        for (k, v) in &builder.variables {
            let var_id = uuid::Uuid::new_v4().to_string();
            command_context.history_manager.record_variable_created(
                &var_id,
                k,
                variable_type_name(v),
                v.clone(),
                &process_instance.id,
                Some(&root_execution.id),
                None,
                &mut command_context.session,
            );
        }
        // History for initiator variable (not in builder.variables).
        // Skip duplicate if start_user also passed the same name as a variable.
        if let Some((ref name, _)) = initiator_var {
            if !builder.variables.contains_key(name) {
                record_initiator_variable_history(
                    command_context,
                    &process_instance.id,
                    &root_execution.id,
                    initiator_var,
                );
            }
        }

        Ok(process_instance)
    }
}

/// Async variant of `StartProcessInstanceCmd`.
///
/// Creates the process instance and root execution but does NOT plan a
/// continue-process operation. Instead, an async-continuation job is inserted
/// so the `AsyncJobAcquisition` thread (or standalone timer worker) can pick
/// it up and execute the start event in a separate command context.
///
/// Java equivalent: `StartProcessInstanceAsyncCmd`.
pub struct StartProcessInstanceAsyncCmd {
    process_instance_builder: ProcessInstanceBuilder,
    override_start_event_id: Option<String>,
}

impl StartProcessInstanceAsyncCmd {
    pub fn new(process_instance_builder: ProcessInstanceBuilder) -> Self {
        Self {
            process_instance_builder,
            override_start_event_id: None,
        }
    }

    pub fn with_start_event_id(
        process_instance_builder: ProcessInstanceBuilder,
        start_event_id: String,
    ) -> Self {
        Self {
            process_instance_builder,
            override_start_event_id: Some(start_event_id),
        }
    }
}

impl Command<ProcessInstance> for StartProcessInstanceAsyncCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<ProcessInstance, crate::error::FlowableError> {
        let builder = self.process_instance_builder.clone();
        let process_definition = resolve_process_definition(command_context, &builder)?
            .ok_or_else(|| {
                crate::error::FlowableError::NotFound(
                    "No process definition found for async process instance start request"
                        .to_string(),
                )
            })?;
        let process_definition_id = process_definition.id.clone();

        let start_event_id = match self.override_start_event_id.clone() {
            Some(id) => id,
            None => {
                match resolve_start_event_id_optional(command_context, &process_definition_id) {
                    Some(id) => id,
                    None => {
                        return Err(crate::error::FlowableError::NotFound(format!(
                            "No StartEvent found in deployed model for process definition ID: {}",
                            process_definition_id
                        )));
                    }
                }
            }
        };

        let process_instance = ProcessInstance {
            id: uuid::Uuid::new_v4().to_string(),
            name: builder.name.clone(),
            process_definition_id: process_definition_id.clone(),
            process_definition_key: builder
                .process_definition_key
                .clone()
                .unwrap_or_else(|| process_definition.key.clone()),
            process_definition_name: process_definition.name.clone(),
            process_definition_version: process_definition.version,
            business_key: builder.business_key.clone(),
            business_status: builder.business_status.clone(),
            is_suspended: false,
            tenant_id: builder
                .tenant_id
                .clone()
                .or_else(|| process_definition.tenant_id.clone()),
            start_time: Some(Utc::now()),
            start_user_id: builder.start_user_id.clone(),
            callback_id: builder.callback_id.clone(),
            callback_type: builder.callback_type.clone(),
            reference_id: builder.reference_id.clone(),
            reference_type: builder.reference_type.clone(),
            is_ended: false,
            super_execution_id: builder.super_execution_id.clone(),
            root_process_instance_id: Some(process_definition_id.clone()),
        };

        let mut process_instance = process_instance;
        process_instance.root_process_instance_id = Some(process_instance.id.clone());

        let mut root_execution = Execution {
            id: process_instance.id.clone(),
            parent_id: None,
            super_execution_id: builder.super_execution_id.clone(),
            root_process_instance_id: Some(process_instance.id.clone()),
            process_instance_id: Some(process_instance.id.clone()),
            process_definition_id: Some(process_instance.process_definition_id.clone()),
            process_definition_key: Some(process_instance.process_definition_key.clone()),
            process_definition_name: process_instance.process_definition_name.clone(),
            process_definition_version: Some(process_instance.process_definition_version),
            activity_id: Some(start_event_id.clone()),
            activity_name: None,
            name: builder.name,
            description: None,
            is_suspended: false,
            is_ended: false,
            is_active: false, // inactive until async job resumes it
            is_concurrent: false,
            is_scope: true,
            is_multi_instance_root: false,
            tenant_id: process_instance.tenant_id.clone(),
            ..Default::default()
        };
        root_execution.set_process_variables(builder.variables.clone());
        for (name, value) in &builder.transient_variables {
            root_execution.set_transient_variable(name.clone(), value.clone());
        }

        // Initialize data objects as process variables
        {
            if let Some(bpmn_model) = command_context
                .deployment_manager
                .get_bpmn_model(&process_definition_id)
                && let Some(main_process) = bpmn_model.main_process.as_ref()
            {
                // Java ProcessInstanceHelper#processDataObjects: copy converter-typed
                // values as-is — no EL evaluation of `${...}`.
                for data_object in &main_process.data_objects {
                    if let Some(name) = &data_object.name {
                        if root_execution.process_variable(name).is_some() {
                            continue;
                        }
                        let value = data_object.value.clone().unwrap_or(serde_json::Value::Null);
                        root_execution.variables.insert(name.to_string(), value);
                    }
                }
            }
        }

        // Java ProcessInstanceHelper.java:197-199 + ExecutionEntityManagerImpl.java:298-300
        // (async start path — same initiator write as sync StartProcessInstanceCmd).
        let initiator_var = apply_start_event_initiator(
            command_context,
            &process_definition_id,
            &start_event_id,
            builder.start_user_id.as_deref(),
            &mut root_execution,
        );

        command_context
            .execution_entity_manager
            .insert(&root_execution, &mut command_context.session);

        register_process_event_subprocesses(&root_execution, command_context)?;

        // Create async-continuation job instead of planning continue-process operation.
        // The job will be acquired by AsyncJobAcquisition and executed in a separate
        // command context, which will set the resume flag and plan the continue operation.
        // Category is resolved from the process base element, matching Java
        // StartProcessInstanceAsyncCmd / JobUtil.createJob(execution, process, ...).
        let now = command_context
            .runtime_store
            .time_source()
            .now()
            .timestamp_millis();
        let category = command_context
            .deployment_manager
            .get_bpmn_model(&process_definition_id)
            .as_ref()
            .and_then(|model| model.main_process.as_ref())
            .and_then(|process| resolve_job_category(&process.base_element, &root_execution));
        command_context.runtime_store.insert_timer_job_state(
            &RuntimeTimerJobState {
                timer_job_id: uuid::Uuid::new_v4().to_string(),
                process_instance_id: process_instance.id.clone(),
                execution_id: root_execution.id.clone(),
                activity_id: start_event_id.clone(),
                job_state: Some(ASYNC_CONTINUATION_JOB_STATE.to_string()),
                is_boundary: false,
                attached_activity_id: None,
                cancel_activity: false,
                time_duration: Some(ASYNC_CONTINUATION_JOB_TYPE_MARKER.to_string()),
                time_date: None,
                time_cycle: None,
                end_date: None,
                due_time: Some(now),
                lock_owner: None,
                lock_time: None,
                lock_expiration_time: None,
                retries: Some(
                    command_context
                        .config
                        .async_executor
                        .number_of_retries
                        .max(0),
                ),
                error_message: None,
                error_details: None,
                category,
                // Java StartProcessInstanceAsyncCmd.java:71: createAsyncJob(job, false)
                // — the async start job is NOT exclusive.
                exclusive: false,
                ..Default::default()
            },
            &mut command_context.session,
        );

        command_context
            .runtime_store
            .insert_process_instance(&process_instance, &mut command_context.session);

        insert_starter_identity_link(
            command_context,
            &process_instance.id,
            process_instance.start_user_id.as_deref(),
        );

        command_context
            .history_manager
            .record_process_instance_start(
                &process_instance.id,
                &process_instance.process_definition_id,
                process_instance.business_key.as_deref(),
                process_instance.start_user_id.as_deref(),
                &mut command_context.session,
            );

        command_context.history_manager.record_audit_event(
            "start-async",
            Some(&process_instance.id),
            Some(&process_instance.process_definition_id),
            Some("Process instance started asynchronously"),
            &mut command_context.session,
        );

        for (k, v) in &builder.variables {
            let var_id = uuid::Uuid::new_v4().to_string();
            command_context.history_manager.record_variable_created(
                &var_id,
                k,
                variable_type_name(v),
                v.clone(),
                &process_instance.id,
                Some(&root_execution.id),
                None,
                &mut command_context.session,
            );
        }
        if let Some((ref name, _)) = initiator_var {
            if !builder.variables.contains_key(name) {
                record_initiator_variable_history(
                    command_context,
                    &process_instance.id,
                    &root_execution.id,
                    initiator_var,
                );
            }
        }

        Ok(process_instance)
    }
}
