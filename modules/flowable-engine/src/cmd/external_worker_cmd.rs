use crate::agenda::FlowableEngineAgenda;
use crate::cmd::trigger_boundary_event_cmd::{
    TriggerBoundaryEventByEventRefCmd, TriggerTimerBoundaryEventCmd,
};
use crate::cmd::trigger_intermediate_catch_event_cmd::TriggerTimerIntermediateCatchEventCmd;
use crate::cmd::trigger_start_event_subscription_cmd::TriggerEventSubprocessByEventCmd;
use crate::el::expression::{Expression, SimpleExpression};
use crate::engine::external_worker_service::{
    ExternalWorkerBpmnErrorRequest, ExternalWorkerCmmnTerminateRequest,
    ExternalWorkerFailureRequest, ExternalWorkerFetchAndLockRequest, ExternalWorkerJob,
    ExternalWorkerJobKind, is_external_worker_service_task_job,
};
use crate::error::FlowableError;
use crate::interceptor::command::Command;
use crate::interceptor::command_context::CommandContext;
use crate::persistence::runtime_store::{EventSubscriptionKind, RuntimeTimerJobState};

fn map_runtime_timer_job(
    timer_job: RuntimeTimerJobState,
    variables: std::collections::HashMap<String, serde_json::Value>,
) -> ExternalWorkerJob {
    ExternalWorkerJob {
        id: timer_job.timer_job_id,
        job_kind: ExternalWorkerJobKind::RuntimeTimer,
        process_instance_id: timer_job.process_instance_id,
        execution_id: timer_job.execution_id,
        activity_id: timer_job.activity_id,
        is_boundary: timer_job.is_boundary,
        worker_id: timer_job.lock_owner.unwrap_or_default(),
        due_time: timer_job.due_time,
        lock_expiration_time: timer_job.lock_expiration_time.unwrap_or_default(),
        retries: timer_job.retries.unwrap_or(1),
        error_message: timer_job.error_message,
        error_details: timer_job.error_details,
        // Topic is stored on the job at create time as job_handler_configuration
        // (Java ExternalWorkerTaskActivityBehavior.java:119-121).
        topic: timer_job.job_handler_configuration.clone(),
        variables,
    }
}

fn load_locked_runtime_timer_job(
    command_context: &mut CommandContext,
    job_id: &str,
    worker_id: &str,
) -> Result<RuntimeTimerJobState, FlowableError> {
    let (store, session) = command_context.store_and_session();
    let timer_job = store.find_timer_job_state(job_id, session).ok_or_else(|| {
        FlowableError::NotFound(format!(
            "external worker job {} was not found in the supported runtime timer subset",
            job_id
        ))
    })?;

    // Java parity (`AbstractExternalWorkerJobCmd.resolveJob`): only worker
    // ownership is validated. Lock expiration is NOT checked — a worker whose
    // lock has expired can still complete/fail/unlock until the background
    // reset clears the owner. Mutation paths that update the job use a CAS to
    // protect against the lock changing after this lookup.
    match timer_job.lock_owner.as_deref() {
        Some(owner) if owner == worker_id => {}
        Some(_) => {
            return Err(FlowableError::Forbidden(format!(
                "external worker job {} is locked by a different worker",
                job_id
            )));
        }
        None => {
            return Err(FlowableError::BadRequest(format!(
                "external worker job {} is not locked",
                job_id
            )));
        }
    }

    Ok(timer_job)
}

fn delete_external_worker_timer_source(
    command_context: &mut CommandContext,
    timer_job: &RuntimeTimerJobState,
) {
    {
        let (store, session) = command_context.store_and_session();
        store.delete_event_wait_state_by_execution_id(&timer_job.execution_id, session);
        store.delete_timer_job_state(&timer_job.timer_job_id, session);
        store.delete_boundary_event_states_by_host_execution_id(&timer_job.execution_id, session);
    }
    // Service-task wait tokens must not be deleted here — boundary/event-subprocess
    // handling owns cancellation of the host execution. Only legacy timer-backed
    // intermediate-catch tokens are removed with the job.
    if is_external_worker_service_task_job(timer_job) {
        return;
    }
    let entity_mgr = &mut command_context.execution_entity_manager;
    let session = &mut command_context.session;
    entity_mgr.delete(&timer_job.execution_id, session);
}

fn delete_external_worker_job_only(
    command_context: &mut CommandContext,
    timer_job: &RuntimeTimerJobState,
) {
    let (store, session) = command_context.store_and_session();
    store.delete_timer_job_state(&timer_job.timer_job_id, session);
}

/// Leave a service-task external-worker wait state (Java complete → message job →
/// `ExternalWorkerTaskCompleteJobHandler` → `planTriggerExecutionOperation`).
fn leave_external_worker_service_task(
    command_context: &mut CommandContext,
    timer_job: &RuntimeTimerJobState,
) -> Result<(), FlowableError> {
    delete_external_worker_job_only(command_context, timer_job);
    let execution_id = timer_job.execution_id.clone();
    let mut execution = command_context
        .runtime_store
        .find_execution(&execution_id, &mut command_context.session)
        .ok_or_else(|| {
            FlowableError::NotFound(format!(
                "external worker job execution {} was not found",
                execution_id
            ))
        })?;
    command_context
        .agenda
        .plan_take_outgoing_sequence_flows_operation(execution.clone());
    // Persist any prior wait-state flags before agenda ops run.
    execution.is_active = true;
    execution.is_ended = false;
    command_context
        .execution_entity_manager
        .update(&execution, &mut command_context.session);
    Ok(())
}

pub struct FetchAndLockExternalWorkerJobsCmd {
    request: ExternalWorkerFetchAndLockRequest,
}

impl FetchAndLockExternalWorkerJobsCmd {
    pub fn new(request: ExternalWorkerFetchAndLockRequest) -> Self {
        Self { request }
    }
}

impl Command<Vec<ExternalWorkerJob>> for FetchAndLockExternalWorkerJobsCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<Vec<ExternalWorkerJob>, FlowableError> {
        if self.request.max_jobs == 0 {
            return Err(FlowableError::BadRequest(
                "external worker fetch requires max_jobs > 0".to_string(),
            ));
        }
        if self.request.lock_duration_ms <= 0 {
            return Err(FlowableError::BadRequest(
                "external worker fetch requires lock_duration_ms > 0".to_string(),
            ));
        }

        let now = command_context
            .runtime_store
            .time_source()
            .now()
            .timestamp_millis();
        let topic = self.request.topic.as_deref();
        let (store, session) = command_context.store_and_session();
        let locked = store.fetch_and_lock_external_worker_timer_jobs(
            &self.request.worker_id,
            now,
            self.request.max_jobs,
            self.request.lock_duration_ms,
            topic,
            session,
        );

        let mut jobs = Vec::with_capacity(locked.len());
        for timer_job in locked {
            // Java `AcquireExternalWorkerJobsCmd` →
            // `InternalJobManager#resolveVariablesForExternalWorkerJob`
            // (DefaultInternalJobManager.java:79-109): project in-parameters
            // or full process variables onto the acquired job payload.
            let variables = resolve_variables_for_external_worker_job(command_context, &timer_job);
            jobs.push(map_runtime_timer_job(timer_job, variables));
        }

        Ok(jobs)
    }
}

pub struct CompleteExternalWorkerJobCmd {
    job_id: String,
    worker_id: String,
    /// Java `ExternalWorkerJobCompleteCmd.variables` — written back on complete.
    variables: Option<std::collections::HashMap<String, serde_json::Value>>,
}

impl CompleteExternalWorkerJobCmd {
    pub fn new(
        job_id: String,
        worker_id: String,
        variables: Option<std::collections::HashMap<String, serde_json::Value>>,
    ) -> Self {
        Self {
            job_id,
            worker_id,
            variables,
        }
    }
}

impl Command<()> for CompleteExternalWorkerJobCmd {
    fn execute(&self, command_context: &mut CommandContext) -> Result<(), FlowableError> {
        let timer_job =
            load_locked_runtime_timer_job(command_context, &self.job_id, &self.worker_id)?;

        // Java ExternalWorkerJobCompleteCmd.java:49-82 — out-parameter mapping
        // or direct variable writeback before the job is made executable.
        apply_external_worker_complete_variables(
            command_context,
            &timer_job,
            self.variables.as_ref(),
        )?;

        // Service-task external-worker jobs leave the activity; legacy timer-backed
        // jobs continue via intermediate-catch / boundary timer triggers.
        if is_external_worker_service_task_job(&timer_job) {
            return leave_external_worker_service_task(command_context, &timer_job);
        }

        if timer_job.is_boundary {
            TriggerTimerBoundaryEventCmd::new(
                timer_job.activity_id.clone(),
                timer_job.process_instance_id.clone(),
            )
            .execute(command_context)?;
        } else {
            TriggerTimerIntermediateCatchEventCmd::new(timer_job.execution_id.clone())
                .execute(command_context)?;
        }

        Ok(())
    }
}

/// Java `DefaultInternalJobManager#resolveVariablesForExternalWorkerJobInternal`
/// (`DefaultInternalJobManager.java:79-109`).
fn resolve_variables_for_external_worker_job(
    command_context: &mut CommandContext,
    timer_job: &RuntimeTimerJobState,
) -> std::collections::HashMap<String, serde_json::Value> {
    let execution = match command_context
        .runtime_store
        .find_execution(&timer_job.execution_id, &mut command_context.session)
    {
        Some(e) => e,
        None => return std::collections::HashMap::new(),
    };

    if let Some(service_task) = find_external_worker_service_task(command_context, &execution) {
        // Java DefaultInternalJobManager.resolveVariablesForExternalWorkerJobInternal
        // (DefaultInternalJobManager.java:87-106):
        // 1) non-empty in-parameters → mapped subset (in-params win over the flag)
        // 2) else if doNotIncludeVariables → empty map
        // 3) else → full process variables
        if !service_task.in_parameters.is_empty() {
            let mut variables = std::collections::HashMap::new();
            for param in &service_task.in_parameters {
                let value = if let Some(source) = param.source.as_ref() {
                    execution.process_variable(source)
                } else if let Some(expr) = param.source_expression.as_ref() {
                    SimpleExpression::new(expr.clone()).get_value(&execution)
                } else {
                    None
                };
                if let Some(target) = param.target.as_ref() {
                    if let Some(value) = value {
                        variables.insert(target.clone(), value);
                    }
                }
            }
            return variables;
        }
        if service_task.do_not_include_variables {
            return std::collections::HashMap::new();
        }
    }

    // Full process-visible variables (Java executionEntity.getVariables()).
    execution.process_variables()
}

/// Java `ExternalWorkerJobCompleteCmd#runJobLogic` out-parameter / variables writeback
/// (`ExternalWorkerJobCompleteCmd.java:59-82`). Variables land on the process-instance
/// scope so they are visible after the token leaves the service task.
fn apply_external_worker_complete_variables(
    command_context: &mut CommandContext,
    timer_job: &RuntimeTimerJobState,
    variables: Option<&std::collections::HashMap<String, serde_json::Value>>,
) -> Result<(), FlowableError> {
    let Some(variables) = variables else {
        return Ok(());
    };
    if variables.is_empty() {
        return Ok(());
    }

    let execution = command_context
        .runtime_store
        .find_execution(&timer_job.execution_id, &mut command_context.session)
        .ok_or_else(|| {
            FlowableError::NotFound(format!(
                "external worker job execution {} was not found",
                timer_job.execution_id
            ))
        })?;

    let process_instance_id = execution
        .process_instance_id
        .clone()
        .unwrap_or_else(|| execution.id.clone());

    let to_write: std::collections::HashMap<String, serde_json::Value> =
        if let Some(service_task) = find_external_worker_service_task(command_context, &execution) {
            if !service_task.out_parameters.is_empty() {
                // Temporary container = complete-request variables
                // (Java VariableContainerWrapper(variables)).
                let mut mapped = std::collections::HashMap::new();
                for param in &service_task.out_parameters {
                    let value = if let Some(source) = param.source.as_ref() {
                        variables.get(source).cloned()
                    } else if let Some(expr) = param.source_expression.as_ref() {
                        // Evaluate expression against a synthetic container of
                        // the worker-supplied variables (limited EL: ${varName}).
                        let temp = temporary_variable_execution(variables);
                        SimpleExpression::new(expr.clone()).get_value(&temp)
                    } else {
                        None
                    };
                    if let Some(target) = param.target.as_ref() {
                        if let Some(value) = value {
                            mapped.insert(target.clone(), value);
                        }
                    }
                }
                mapped
            } else {
                variables.clone()
            }
        } else {
            variables.clone()
        };

    if to_write.is_empty() {
        return Ok(());
    }

    if let Some(mut root_execution) = command_context
        .runtime_store
        .find_execution(&process_instance_id, &mut command_context.session)
    {
        for (name, value) in to_write {
            root_execution.set_process_variable(name, value);
        }
        command_context
            .execution_entity_manager
            .update(&root_execution, &mut command_context.session);
    }
    Ok(())
}

fn temporary_variable_execution(
    variables: &std::collections::HashMap<String, serde_json::Value>,
) -> crate::runtime::execution::Execution {
    let mut execution = crate::runtime::execution::Execution {
        id: "temp-external-worker-vars".to_string(),
        ..Default::default()
    };
    for (name, value) in variables {
        execution.set_process_variable(name.clone(), value.clone());
    }
    execution
}

fn find_external_worker_service_task(
    command_context: &CommandContext,
    execution: &crate::runtime::execution::Execution,
) -> Option<flowable_bpmn_model::model::ServiceTask> {
    let pd_id = execution.process_definition_id.as_deref()?;
    let activity_id = execution.activity_id.as_deref()?;
    let model = command_context.deployment_manager.get_bpmn_model(pd_id)?;
    let process = model.main_process.as_ref()?;
    let flow_element = process.flow_element_map.get(activity_id)?;
    match flow_element {
        flowable_bpmn_model::model::FlowElementEnum::ServiceTask(st)
            if st.task_type.as_deref() == Some("external-worker") =>
        {
            Some(st.clone())
        }
        _ => None,
    }
}

pub struct CompleteExternalWorkerJobWithBpmnErrorCmd {
    job_id: String,
    request: ExternalWorkerBpmnErrorRequest,
}

impl CompleteExternalWorkerJobWithBpmnErrorCmd {
    pub fn new(job_id: String, request: ExternalWorkerBpmnErrorRequest) -> Self {
        Self { job_id, request }
    }
}

impl Command<()> for CompleteExternalWorkerJobWithBpmnErrorCmd {
    fn execute(&self, command_context: &mut CommandContext) -> Result<(), FlowableError> {
        if self.request.error_code.trim().is_empty() {
            return Err(FlowableError::BadRequest(
                "external worker BPMN error requires errorCode".to_string(),
            ));
        }

        let timer_job =
            load_locked_runtime_timer_job(command_context, &self.job_id, &self.request.worker_id)?;

        let source_execution_id = timer_job.execution_id.clone();
        let process_instance_id = timer_job.process_instance_id.clone();
        let error_code = self.request.error_code.clone();

        if let Some(variables) = &self.request.variables
            && !variables.is_empty()
        {
            // Java parity: `ExternalWorkerJobBpmnErrorCmd` stores the variables
            // with scopeId = processInstanceId, i.e. on the process instance
            // scope — which in Rust is the process-instance scope execution row
            // (the single process-level variable store).
            if let Some(mut root_execution) = command_context
                .runtime_store
                .find_execution(&process_instance_id, &mut command_context.session)
            {
                for (name, value) in variables {
                    root_execution.set_process_variable(name.clone(), value.clone());
                }
                command_context
                    .execution_entity_manager
                    .update(&root_execution, &mut command_context.session);
            }
        }

        let event_subprocess_cmd = TriggerEventSubprocessByEventCmd::with_source_execution(
            EventSubscriptionKind::Error,
            error_code.clone(),
            process_instance_id.clone(),
            source_execution_id.clone(),
        );
        let triggered_event_subprocesses = event_subprocess_cmd.execute(command_context)?;
        if !triggered_event_subprocesses.is_empty() {
            delete_external_worker_timer_source(command_context, &timer_job);
            return Ok(());
        }

        let boundary_cmd = TriggerBoundaryEventByEventRefCmd::with_source_execution(
            EventSubscriptionKind::Error,
            error_code.clone(),
            process_instance_id,
            source_execution_id,
        );
        if boundary_cmd.execute_with_trigger_result(command_context)? {
            delete_external_worker_timer_source(command_context, &timer_job);
            return Ok(());
        }

        Err(FlowableError::BadRequest(format!(
            "external worker BPMN error was not handled: No matching BPMN error handler found for errorCode {}",
            error_code
        )))
    }
}

pub struct HandleExternalWorkerFailureCmd {
    job_id: String,
    request: ExternalWorkerFailureRequest,
}

pub struct TerminateExternalWorkerJobWithCmmnCmd {
    job_id: String,
    request: ExternalWorkerCmmnTerminateRequest,
}

impl TerminateExternalWorkerJobWithCmmnCmd {
    pub fn new(job_id: String, request: ExternalWorkerCmmnTerminateRequest) -> Self {
        Self { job_id, request }
    }
}

impl Command<()> for TerminateExternalWorkerJobWithCmmnCmd {
    fn execute(&self, command_context: &mut CommandContext) -> Result<(), FlowableError> {
        let (store, session) = command_context.store_and_session();
        if store.find_timer_job_state(&self.job_id, session).is_some() {
            return Err(FlowableError::BadRequest(format!(
                "external worker job {} is a BPMN runtime timer job and cannot be terminated with the CMMN terminate transition",
                self.job_id
            )));
        }

        let cmmn_engine = command_context.config.cmmn_engine.clone().ok_or_else(|| {
            FlowableError::BadRequest(
                "external worker CMMN terminate requires a configured CMMN engine".to_string(),
            )
        })?;

        if self.request.terminate {
            cmmn_engine
                .terminate_external_worker_job(&self.job_id, &self.request.worker_id)
                .map_err(map_cmmn_external_worker_error)
        } else {
            cmmn_engine
                .management_service()
                .delete_job(&self.job_id)
                .map_err(map_cmmn_external_worker_error)
        }
    }
}

fn map_cmmn_external_worker_error(error: flowable_cmmn_engine::CmmnError) -> FlowableError {
    match error {
        flowable_cmmn_engine::CmmnError::NotFound { message } => FlowableError::NotFound(message),
        flowable_cmmn_engine::CmmnError::Storage { message } => FlowableError::Internal(message),
        flowable_cmmn_engine::CmmnError::Validation { message }
        | flowable_cmmn_engine::CmmnError::UnsupportedModel { message, .. }
        | flowable_cmmn_engine::CmmnError::Execution { message } => {
            FlowableError::BadRequest(message)
        }
        flowable_cmmn_engine::CmmnError::Conflict { message } => FlowableError::Forbidden(message),
        flowable_cmmn_engine::CmmnError::NonUniqueResult { query, count } => {
            FlowableError::BadRequest(format!("non-unique result for {query}: {count} matches"))
        }
    }
}

impl HandleExternalWorkerFailureCmd {
    pub fn new(job_id: String, request: ExternalWorkerFailureRequest) -> Self {
        Self { job_id, request }
    }
}

impl Command<()> for HandleExternalWorkerFailureCmd {
    fn execute(&self, command_context: &mut CommandContext) -> Result<(), FlowableError> {
        // Java ExternalWorkerJobFailCmd:
        //   retries >= 0  -> use the provided value
        //   retries < 0   -> decrement the job's current retries by one
        //   newRetries > 0 -> requeue with unlock / retry timeout
        //   otherwise      -> move to dead letter
        if self.request.retry_duration_ms < 0 {
            return Err(FlowableError::BadRequest(
                "external worker failure requires retry_duration_ms >= 0".to_string(),
            ));
        }

        let now = command_context
            .runtime_store
            .time_source()
            .now()
            .timestamp_millis();
        let mut timer_job =
            load_locked_runtime_timer_job(command_context, &self.job_id, &self.request.worker_id)?;
        let expected_lock_expiration_time = timer_job.lock_expiration_time;

        let new_retries = if self.request.retries >= 0 {
            self.request.retries
        } else {
            timer_job.retries.unwrap_or(1).saturating_sub(1)
        };

        timer_job.error_message = self.request.error_message.clone();
        timer_job.error_details = self.request.error_details.clone();
        timer_job.lock_owner = None;
        timer_job.lock_time = None;
        timer_job.lock_expiration_time = None;

        if new_retries > 0 {
            timer_job.retries = Some(new_retries);
            timer_job.due_time = Some(now + self.request.retry_duration_ms);
        } else {
            // Exhausted retries: explicit deadletter state (not just retries=0).
            timer_job.retries = Some(0);
            timer_job.job_state = Some("deadletter".to_string());
        }

        let (store, session) = command_context.store_and_session();
        let updated = store.replace_timer_job_state_if_lock_matches(
            &timer_job,
            &self.request.worker_id,
            expected_lock_expiration_time,
            session,
        );
        if !updated {
            return Err(FlowableError::BadRequest(format!(
                "external worker job {} lock changed while applying failure",
                self.job_id
            )));
        }

        Ok(())
    }
}

pub struct UnlockExternalWorkerJobCmd {
    job_id: String,
    worker_id: String,
}

impl UnlockExternalWorkerJobCmd {
    pub fn new(job_id: String, worker_id: String) -> Self {
        Self { job_id, worker_id }
    }
}

impl Command<()> for UnlockExternalWorkerJobCmd {
    fn execute(&self, command_context: &mut CommandContext) -> Result<(), FlowableError> {
        let mut timer_job =
            load_locked_runtime_timer_job(command_context, &self.job_id, &self.worker_id)?;
        let expected_lock_expiration_time = timer_job.lock_expiration_time;

        timer_job.lock_owner = None;
        timer_job.lock_time = None;
        timer_job.lock_expiration_time = None;

        let (store, session) = command_context.store_and_session();
        let updated = store.replace_timer_job_state_if_lock_matches(
            &timer_job,
            &self.worker_id,
            expected_lock_expiration_time,
            session,
        );
        if !updated {
            return Err(FlowableError::BadRequest(format!(
                "external worker job {} lock changed while unlocking",
                self.job_id
            )));
        }

        Ok(())
    }
}
