use crate::cmd::external_worker_cmd::{
    CompleteExternalWorkerJobCmd, CompleteExternalWorkerJobWithBpmnErrorCmd,
    FetchAndLockExternalWorkerJobsCmd, HandleExternalWorkerFailureCmd,
    TerminateExternalWorkerJobWithCmmnCmd, UnlockExternalWorkerJobCmd,
};
use crate::el::expression::{Expression, SimpleExpression};
use crate::error::FlowableError;
use crate::interceptor::command_context::CommandContext;
use crate::interceptor::command_executor::{CommandExecutor, DefaultCommandExecutor};
use crate::persistence::runtime_store::{
    RuntimeJobType, RuntimeTimerJobState, job_handler_types, stamp_new_job_metadata,
};
use crate::runtime::execution::Execution;
use flowable_bpmn_model::model::ServiceTask;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExternalWorkerJobKind {
    RuntimeTimer,
}

// ── CreateExternalWorkerJob interceptor (Java parity) ───────────────────────

/// Mutable context passed to [`CreateExternalWorkerJobInterceptor::before_create`].
///
/// Mirrors Java `CreateExternalWorkerJobBeforeContext`: interceptors may override
/// `job_category` and/or `job_topic_expression` before the job row is inserted.
#[derive(Debug)]
pub struct CreateExternalWorkerJobBeforeContext<'a> {
    pub service_task: &'a ServiceTask,
    pub execution: &'a Execution,
    /// Resolved category text from the first `flowable:jobCategory` extension
    /// (may still be an expression string; evaluated after the before hook).
    pub job_category: Option<String>,
    /// When set by the interceptor, replaces the BPMN `topic` expression.
    pub job_topic_expression: Option<String>,
}

/// Context passed to [`CreateExternalWorkerJobInterceptor::after_create`].
#[derive(Debug)]
pub struct CreateExternalWorkerJobAfterContext<'a> {
    pub service_task: &'a ServiceTask,
    pub job: &'a RuntimeTimerJobState,
    pub execution: &'a Execution,
}

/// Java `CreateExternalWorkerJobInterceptor` — optional hook around external-worker
/// job creation for `flowable:type="external-worker"` service tasks.
pub trait CreateExternalWorkerJobInterceptor: Send + Sync {
    fn before_create(&self, context: &mut CreateExternalWorkerJobBeforeContext<'_>);
    fn after_create(&self, context: &CreateExternalWorkerJobAfterContext<'_>);
}

/// Object-safe handle stored on [`crate::service::config::ProcessEngineConfiguration`].
pub type CreateExternalWorkerJobInterceptorHandle =
    Arc<dyn CreateExternalWorkerJobInterceptor + Send + Sync>;

impl fmt::Debug for dyn CreateExternalWorkerJobInterceptor + Send + Sync {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("CreateExternalWorkerJobInterceptor")
    }
}

/// Create an external-worker wait job for a service task (Java
/// `ExternalWorkerTaskActivityBehavior#execute` job-creation half).
///
/// Covers the three S5 items on the create path:
/// - **skipExpression** is evaluated by the caller (ServiceTask common path) before
///   this is invoked; when skip is true the caller leaves without creating a job.
/// - **jobCategory** is read from the first `flowable:jobCategory` extension, may be
///   overridden by the before interceptor, then evaluated against the execution.
/// - **interceptor** `before`/`after` hooks run around insert when configured.
///
/// Returns without leaving the activity (wait-state).
pub(crate) fn create_external_worker_service_task_job(
    service_task: &ServiceTask,
    execution: &mut Execution,
    command_context: &mut CommandContext,
) -> Result<(), FlowableError> {
    let evaluation_execution =
        crate::engine::variable_service::evaluation_execution(command_context, execution);
    let element_id = service_task
        .task
        .activity
        .flow_node
        .flow_element
        .base_element
        .id
        .clone()
        .unwrap_or_else(|| execution.activity_id.clone().unwrap_or_default());
    let element_name = service_task
        .task
        .activity
        .flow_node
        .flow_element
        .name
        .clone();

    // Seed before-context with the raw extension text (Java passes getJobCategory
    // extension text, not the already-evaluated value) so interceptors can replace
    // the expression before evaluation.
    let mut before = CreateExternalWorkerJobBeforeContext {
        service_task,
        execution: &evaluation_execution,
        job_category: first_job_category_extension_text(service_task).map(str::to_string),
        job_topic_expression: None,
    };

    if let Some(interceptor) = command_context
        .config
        .create_external_worker_job_interceptor
        .as_ref()
    {
        interceptor.before_create(&mut before);
    }

    let category = before
        .job_category
        .as_deref()
        .and_then(|text| evaluate_job_category_text(text, &evaluation_execution));

    let topic_expression = before
        .job_topic_expression
        .as_deref()
        .or(service_task.topic.as_deref())
        .ok_or_else(|| {
            FlowableError::ExecutionError(format!(
                "External worker service task '{}' requires flowable:topic",
                element_id
            ))
        })?;

    let topic_value = evaluate_topic_expression(topic_expression, &evaluation_execution)?;
    if topic_value.is_empty() {
        return Err(FlowableError::ExecutionError(format!(
            "Expression {} did not evaluate to a valid value (non empty String). Was: {}. For execution {}",
            topic_expression, topic_value, evaluation_execution.id
        )));
    }

    let process_instance_id = execution
        .process_instance_id
        .clone()
        .unwrap_or_else(|| execution.id.clone());
    let store = command_context.runtime_store_handle();
    let now = store.time_source().now().timestamp_millis();
    let retries = command_context
        .config
        .async_executor
        .number_of_retries
        .max(0);

    let mut job = RuntimeTimerJobState {
        timer_job_id: Uuid::new_v4().to_string(),
        process_instance_id,
        execution_id: execution.id.clone(),
        activity_id: element_id.clone(),
        // Family isolation + fetch-and-lock require job_state=timer for typed EW rows.
        job_state: Some("timer".to_string()),
        is_boundary: false,
        attached_activity_id: None,
        cancel_activity: false,
        time_duration: None,
        time_date: None,
        time_cycle: None,
        end_date: None,
        // Immediately acquirable (Java external-worker jobs have no due-date gate).
        due_time: Some(now),
        lock_owner: None,
        lock_time: None,
        lock_expiration_time: None,
        retries: Some(retries),
        error_message: None,
        error_details: None,
        category,
        job_handler_configuration: Some(topic_value),
        ..Default::default()
    };
    stamp_new_job_metadata(
        &mut job,
        now,
        job_handler_types::EXTERNAL_WORKER_COMPLETE,
        execution.tenant_id.clone(),
        execution.process_definition_id.clone(),
        element_name,
    );

    store.insert_timer_job_state_with_type(
        &job,
        Some(&RuntimeJobType::ExternalWorker),
        &mut command_context.session,
    );

    // Wait-state: keep the token on the service task until complete/trigger.
    execution.is_active = true;
    execution.is_ended = false;
    command_context
        .execution_entity_manager
        .update(execution, &mut command_context.session);

    if let Some(interceptor) = command_context
        .config
        .create_external_worker_job_interceptor
        .as_ref()
    {
        interceptor.after_create(&CreateExternalWorkerJobAfterContext {
            service_task,
            job: &job,
            execution,
        });
    }

    Ok(())
}

fn first_job_category_extension_text(service_task: &ServiceTask) -> Option<&str> {
    let elements = service_task
        .task
        .activity
        .flow_node
        .flow_element
        .base_element
        .extension_elements
        .get("jobCategory")?;
    let text = elements.first()?.element_text.as_deref()?.trim();
    if text.is_empty() { None } else { Some(text) }
}

fn evaluate_job_category_text(text: &str, execution: &Execution) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.starts_with("${") && trimmed.ends_with('}') {
        let value = SimpleExpression::new(trimmed.to_string()).get_value(execution)?;
        return match value {
            Value::String(s) => {
                let t = s.trim();
                if t.is_empty() {
                    None
                } else {
                    Some(t.to_string())
                }
            }
            Value::Number(n) => Some(n.to_string()),
            Value::Bool(b) => Some(b.to_string()),
            Value::Null => None,
            _ => None,
        };
    }
    Some(trimmed.to_string())
}

fn evaluate_topic_expression(
    expression: &str,
    execution: &Execution,
) -> Result<String, FlowableError> {
    let trimmed = expression.trim();
    if trimmed.is_empty() {
        return Ok(String::new());
    }
    if trimmed.starts_with("${") && trimmed.ends_with('}') {
        match SimpleExpression::new(trimmed.to_string()).get_value(execution) {
            Some(Value::String(s)) => Ok(s),
            Some(Value::Number(n)) => Ok(n.to_string()),
            Some(Value::Bool(b)) => Ok(b.to_string()),
            Some(other) => Ok(other.to_string()),
            None => Ok(String::new()),
        }
    } else {
        Ok(trimmed.to_string())
    }
}

/// Whether a persisted timer row is a service-task external-worker wait job
/// (handler type `external-worker-complete`), as opposed to the legacy
/// timer-backed external-worker subset.
pub(crate) fn is_external_worker_service_task_job(job: &RuntimeTimerJobState) -> bool {
    job.handler_type.as_deref() == Some(job_handler_types::EXTERNAL_WORKER_COMPLETE)
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalWorkerFetchAndLockRequest {
    pub worker_id: String,
    pub max_jobs: usize,
    pub lock_duration_ms: i64,
    /// Java `AcquireExternalWorkerJobsCmd` / `ExternalWorkerJobAcquireBuilder#topic`
    /// (`AcquireExternalWorkerJobsCmd.java:55-58`): when set, only jobs whose
    /// `job_handler_configuration` (topic) matches are acquired. `None` keeps
    /// the pre-P68 unfiltered acquire path used by legacy timer-backed tests.
    #[serde(default)]
    pub topic: Option<String>,
}

impl ExternalWorkerFetchAndLockRequest {
    pub fn new(worker_id: impl Into<String>, max_jobs: usize, lock_duration_ms: i64) -> Self {
        Self {
            worker_id: worker_id.into(),
            max_jobs,
            lock_duration_ms,
            topic: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalWorkerFailureRequest {
    pub worker_id: String,
    pub error_message: Option<String>,
    pub error_details: Option<String>,
    pub retries: i32,
    pub retry_duration_ms: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ExternalWorkerBpmnErrorRequest {
    pub worker_id: String,
    pub error_code: String,
    pub error_message: Option<String>,
    pub variables: Option<HashMap<String, serde_json::Value>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalWorkerCmmnTerminateRequest {
    pub worker_id: String,
    pub terminate: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ExternalWorkerJob {
    pub id: String,
    pub job_kind: ExternalWorkerJobKind,
    pub process_instance_id: String,
    pub execution_id: String,
    pub activity_id: String,
    pub is_boundary: bool,
    pub worker_id: String,
    pub due_time: Option<i64>,
    pub lock_expiration_time: i64,
    pub retries: i32,
    pub error_message: Option<String>,
    pub error_details: Option<String>,
    /// Topic stored as `job_handler_configuration` at create time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub topic: Option<String>,
    /// Java `AcquiredExternalWorkerJob#variables` — in-parameter projection
    /// (or full process variables when no in-parameters are defined).
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub variables: HashMap<String, serde_json::Value>,
}

pub struct ExternalWorkerService {
    command_executor: Arc<DefaultCommandExecutor>,
}

impl ExternalWorkerService {
    pub fn new(command_executor: Arc<DefaultCommandExecutor>) -> Self {
        Self { command_executor }
    }

    pub fn fetch_and_lock(
        &self,
        request: ExternalWorkerFetchAndLockRequest,
    ) -> Result<Vec<ExternalWorkerJob>, FlowableError> {
        self.command_executor
            .execute(&FetchAndLockExternalWorkerJobsCmd::new(request))
    }

    pub fn complete(&self, job_id: &str, worker_id: &str) -> Result<(), FlowableError> {
        self.complete_with_variables(job_id, worker_id, None)
    }

    /// Java `ExternalWorkerJobCompleteCmd(externalJobId, workerId, variables, ...)`.
    /// When `variables` is set, they are written back to the process (via out-
    /// parameters when defined, otherwise as process variables).
    pub fn complete_with_variables(
        &self,
        job_id: &str,
        worker_id: &str,
        variables: Option<HashMap<String, serde_json::Value>>,
    ) -> Result<(), FlowableError> {
        self.command_executor
            .execute(&CompleteExternalWorkerJobCmd::new(
                job_id.to_string(),
                worker_id.to_string(),
                variables,
            ))
    }

    pub fn complete_with_bpmn_error(
        &self,
        job_id: &str,
        request: ExternalWorkerBpmnErrorRequest,
    ) -> Result<(), FlowableError> {
        self.command_executor
            .execute(&CompleteExternalWorkerJobWithBpmnErrorCmd::new(
                job_id.to_string(),
                request,
            ))
    }

    pub fn cmmn_terminate(
        &self,
        job_id: &str,
        request: ExternalWorkerCmmnTerminateRequest,
    ) -> Result<(), FlowableError> {
        self.command_executor
            .execute(&TerminateExternalWorkerJobWithCmmnCmd::new(
                job_id.to_string(),
                request,
            ))
    }

    pub fn handle_failure(
        &self,
        job_id: &str,
        request: ExternalWorkerFailureRequest,
    ) -> Result<(), FlowableError> {
        self.command_executor
            .execute(&HandleExternalWorkerFailureCmd::new(
                job_id.to_string(),
                request,
            ))
    }

    pub fn unlock(&self, job_id: &str, worker_id: &str) -> Result<(), FlowableError> {
        self.command_executor
            .execute(&UnlockExternalWorkerJobCmd::new(
                job_id.to_string(),
                worker_id.to_string(),
            ))
    }

    /// Active external-worker family only (`job_type=externalWorker`, `job_state=timer`,
    /// parent not suspended). Shared by REST list/get — not a second state pipeline.
    pub fn list_active_timer_jobs(&self) -> Vec<RuntimeTimerJobState> {
        let store = self.command_executor.runtime_store();
        let mut session = store.create_session().unwrap();
        let mut jobs: Vec<_> = store
            .snapshot_timer_job_states(&mut session)
            .into_values()
            .filter(|job| store.is_active_external_worker_job(job, &mut session))
            .collect();
        jobs.sort_by(|left, right| left.timer_job_id.cmp(&right.timer_job_id));
        jobs
    }

    /// Same family isolation as [`Self::list_active_timer_jobs`]. Non-family / unknown → None.
    pub fn find_active_timer_job(&self, job_id: &str) -> Option<RuntimeTimerJobState> {
        let store = self.command_executor.runtime_store();
        let mut session = store.create_session().unwrap();
        store
            .find_timer_job_state(job_id, &mut session)
            .filter(|job| store.is_active_external_worker_job(job, &mut session))
    }
}
