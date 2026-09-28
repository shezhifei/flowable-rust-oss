use crate::el::expression::{Expression, SimpleExpression};
use crate::engine::query::Direction;
use crate::engine::runtime_job_query::{RuntimeJobFamily, RuntimeJobQueryCriteria};
use crate::engine::time_source::{SystemTimeSource, TimeSource, calculate_due_time};
use crate::error::FlowableError;
use crate::persistence::FilterOp;
use crate::persistence::db_session::{BulkJsonRowUpdate, DbParams, DbSession, DbValue};
use crate::persistence::db_store::DbStore;
use crate::persistence::storage_error::StorageError;
use crate::repository::process_definition::ProcessDefinition;
use crate::runtime::execution::Execution;
use crate::runtime::process_instance::ProcessInstance;
use chrono::{DateTime, TimeZone, Utc};
use flowable_bpmn_converter::BpmnXMLConverter;
use flowable_bpmn_model::model::FlowElementEnum;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

thread_local! {
    /// Execution ids written this command with non-empty `transient_variables`.
    /// P45 strips only these rows on commit (avoids full-table scans under load).
    static TRANSIENT_DIRTY_EXECUTION_IDS: RefCell<HashSet<String>> = RefCell::new(HashSet::new());
    /// P58: process instances whose executions were written/deleted this
    /// command → process definition id. Mirrors Java's involved-executions
    /// registration (CommandContextUtil.getInvolvedExecutions) so the
    /// end-of-command inactive-behavior scan only touches affected instances
    /// instead of snapshotting the whole executions table.
    static INVOLVED_PROCESS_INSTANCES: RefCell<HashMap<String, String>> =
        RefCell::new(HashMap::new());
}

/// Clear the per-command dirty set. Called at the start of each command.
pub fn clear_transient_dirty_execution_ids() {
    TRANSIENT_DIRTY_EXECUTION_IDS.with(|ids| ids.borrow_mut().clear());
}

/// P58: clear the per-command involved-process-instance set. Called at the
/// start of each command, next to `clear_transient_dirty_execution_ids`.
pub fn clear_involved_process_instances() {
    INVOLVED_PROCESS_INSTANCES.with(|map| map.borrow_mut().clear());
}

fn mark_involved_process_instance(execution: &Execution) {
    if let (Some(pi_id), Some(def_id)) = (
        execution.process_instance_id.as_deref(),
        execution.process_definition_id.as_deref(),
    ) {
        INVOLVED_PROCESS_INSTANCES.with(|map| {
            map.borrow_mut()
                .insert(pi_id.to_string(), def_id.to_string());
        });
    }
}

/// P58: drain the involved set (Java clears it after planning the
/// ExecuteInactiveBehaviorsOperation, CommandInvoker.java:86). Draining —
/// rather than reading — lets the fixpoint loop terminate: only writes made
/// after the previous scan re-mark an instance.
pub(crate) fn take_involved_process_instances() -> HashMap<String, String> {
    INVOLVED_PROCESS_INSTANCES.with(|map| std::mem::take(&mut *map.borrow_mut()))
}

fn mark_transient_dirty_execution(execution_id: &str) {
    TRANSIENT_DIRTY_EXECUTION_IDS.with(|ids| {
        ids.borrow_mut().insert(execution_id.to_string());
    });
}

fn take_transient_dirty_execution_ids() -> HashSet<String> {
    TRANSIENT_DIRTY_EXECUTION_IDS.with(|ids| std::mem::take(&mut *ids.borrow_mut()))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AcquisitionWritePolicy {
    Optimistic,
    SerializedByGlobalLock,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum JobLockEligibility {
    /// Only rows with no lock owner. Expired leases must be cleared by reset first.
    UnlockedOnly,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExpiredJobClass {
    Async,
    Timer,
    History,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RuntimeJobType {
    Timer,
    History,
    ExternalWorker,
    Other(String),
}

impl RuntimeJobType {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Timer => "timer",
            Self::History => "history",
            Self::ExternalWorker => "externalWorker",
            Self::Other(value) => value,
        }
    }

    pub fn from_persisted(value: &str) -> Self {
        match value {
            "timer" => Self::Timer,
            "history" => Self::History,
            "externalWorker" | "external-worker" => Self::ExternalWorker,
            other => Self::Other(other.to_string()),
        }
    }
}

impl ExpiredJobClass {
    pub const ALL: [Self; 3] = [Self::Async, Self::Timer, Self::History];

    fn job_states(self) -> &'static [&'static str] {
        match self {
            Self::Async => &["executable", "async", "async-after"],
            Self::Timer => &["timer"],
            Self::History => &["history"],
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ResetExpiredJobsBatchOutcome {
    pub scanned: usize,
    pub reset: usize,
    pub conflicts: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum PersistedPropertyLockState {
    Free,
    Held {
        owner: String,
        acquired_at_ms: i64,
        expiry_ms: i64,
    },
    Corrupt,
}

fn parse_task_due_date(value: &str, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    let value = value.trim();
    if let Ok(timestamp_millis) = value.parse::<i64>() {
        return Utc.timestamp_millis_opt(timestamp_millis).single();
    }

    let value = value.to_string();
    let due_millis = if value.starts_with('P') {
        calculate_due_time(None, Some(&value), None, now)
    } else {
        calculate_due_time(Some(&value), None, None, now)
    }?;
    Utc.timestamp_millis_opt(due_millis).single()
}

/// Resolve a BPMN user-task due-date using the same command clock and ISO
/// date/duration rules as timer scheduling.
///
/// Java parity: `UserTaskActivityBehavior#handleDueDate` evaluates the EL
/// first, accepts temporal values directly, and sends String values through
/// `DueDateBusinessCalendar`. Runtime variables are represented as JSON in
/// this engine, so temporal values map to epoch milliseconds or ISO strings.
pub(crate) fn evaluate_user_task_due_date(
    raw_due_date: Option<&str>,
    execution: &Execution,
    now: DateTime<Utc>,
) -> Result<Option<DateTime<Utc>>, FlowableError> {
    let Some(raw_due_date) = raw_due_date.filter(|value| !value.is_empty()) else {
        return Ok(None);
    };

    let value = if raw_due_date.trim().starts_with("${") && raw_due_date.trim().ends_with('}') {
        SimpleExpression::new(raw_due_date.trim().to_string())
            .get_value(execution)
            .unwrap_or(Value::Null)
    } else {
        Value::String(raw_due_date.to_string())
    };

    match value {
        Value::Null => Ok(None),
        Value::String(value) => parse_task_due_date(&value, now).map(Some).ok_or_else(|| {
            FlowableError::ExecutionError(format!("couldn't resolve duedate: {value}"))
        }),
        Value::Number(value) => value
            .as_i64()
            .and_then(|millis| Utc.timestamp_millis_opt(millis).single())
            .map(Some)
            .ok_or_else(|| invalid_due_date_expression(raw_due_date)),
        Value::Bool(_) | Value::Array(_) | Value::Object(_) => {
            Err(invalid_due_date_expression(raw_due_date))
        }
    }
}

fn invalid_due_date_expression(expression: &str) -> FlowableError {
    FlowableError::ExecutionError(format!(
        "Due date expression does not resolve to a Date, Instant, LocalDate, LocalDateTime or Date string: {expression}"
    ))
}

/// Process-definition schedule timers (suspend/activate definition). Never
/// external-worker candidates — activity ids match
/// `PROCESS_DEFINITION_*_TIMER_ACTIVITY_ID` in repository_service.
fn is_process_definition_schedule_timer(job: &RuntimeTimerJobState) -> bool {
    matches!(
        job.activity_id.as_str(),
        "process-definition-suspend" | "process-definition-activate"
    )
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventRegistryDeployment {
    pub id: String,
    pub name: String,
    pub deployed_at: i64,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub parent_deployment_id: Option<String>,
    #[serde(default)]
    pub tenant_id: Option<String>,
    pub resource_names: Vec<String>,
    #[serde(default)]
    pub resources: Vec<EventRegistryDeploymentResource>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventRegistryDeploymentResource {
    pub resource_name: String,
    pub resource: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EventRegistryChannelDefinition {
    pub id: String,
    pub deployment_id: String,
    pub key: String,
    pub name: String,
    pub description: Option<String>,
    #[serde(default)]
    pub category: Option<String>,
    pub channel_type: String,
    pub resource_name: String,
    #[serde(default = "default_event_registry_version")]
    pub version: i32,
    #[serde(default)]
    pub create_time: i64,
    #[serde(default)]
    pub tenant_id: Option<String>,
    #[serde(default)]
    pub parent_deployment_id: Option<String>,
    pub configuration: serde_json::Value,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EventRegistryEventDefinition {
    pub id: String,
    pub deployment_id: String,
    pub key: String,
    pub name: String,
    pub description: Option<String>,
    #[serde(default)]
    pub category: Option<String>,
    pub event_type: String,
    pub channel_key: String,
    pub resource_name: String,
    #[serde(default = "default_event_registry_version")]
    pub version: i32,
    #[serde(default)]
    pub tenant_id: Option<String>,
    #[serde(default)]
    pub parent_deployment_id: Option<String>,
    pub payload: serde_json::Value,
}

fn default_event_registry_version() -> i32 {
    1
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EventRegistryEventDirection {
    Inbound,
    Outbound,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum EventRegistryEventInstanceStatus {
    Created,
    Received,
    Processed,
    Published,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EventRegistryEventInstanceDelivery {
    pub id: String,
    pub event_definition_id: String,
    pub event_definition_key: String,
    pub event_type: String,
    pub channel_key: String,
    pub direction: EventRegistryEventDirection,
    pub status: EventRegistryEventInstanceStatus,
    pub status_history: Vec<EventRegistryEventInstanceStatus>,
    #[serde(default)]
    pub last_error: Option<String>,
    #[serde(default)]
    pub retry_count: u32,
    #[serde(default)]
    pub last_retry_at: Option<i64>,
    #[serde(default)]
    pub last_failure_at: Option<i64>,
    #[serde(default)]
    pub next_retry_at: Option<i64>,
    /// Stable idempotency/dispatch token assigned before external I/O so retries are diagnosable.
    #[serde(default)]
    pub dispatch_token: Option<String>,
    /// Channel definition that ran the original pipeline. Retries must replay
    /// against this exact version, not the current latest for the key.
    #[serde(default)]
    pub channel_definition_id: Option<String>,
    #[serde(default)]
    pub tenant_id: Option<String>,
    pub payload: serde_json::Value,
    pub created_at: i64,
    pub updated_at: i64,
}

/// Durable Event Registry change log entry for cross-instance cache reconciliation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventRegistryChangeRecord {
    pub id: String,
    pub revision: u64,
    /// `deploy`, `delete`, or `update`.
    pub change_type: String,
    /// `channel`, `event`, or `deployment`.
    pub entity_type: String,
    pub entity_id: String,
    pub entity_key: String,
    #[serde(default)]
    pub tenant_id: Option<String>,
    #[serde(default)]
    pub version: Option<i32>,
    #[serde(default)]
    pub deployment_id: Option<String>,
    pub created_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FormDeployment {
    pub id: String,
    pub name: String,
    pub deployed_at: i64,
    pub resource_names: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FormDefinition {
    pub id: String,
    pub deployment_id: String,
    pub key: String,
    pub name: String,
    pub version: i32,
    pub resource_name: String,
    pub form_json: serde_json::Value,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ContentItem {
    pub id: String,
    pub name: String,
    pub mime_type: Option<String>,
    pub created_at: i64,
    pub content: Vec<u8>,
    pub metadata: serde_json::Value,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum HttpTaskRecordStatus {
    Completed,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HttpTaskRecord {
    pub id: String,
    pub process_instance_id: String,
    pub execution_id: String,
    pub activity_id: String,
    pub method: String,
    pub url: String,
    pub request_body: Option<String>,
    pub response_status_code: Option<u16>,
    pub response_body: Option<String>,
    pub status: HttpTaskRecordStatus,
    pub created_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum MailOutboxStatus {
    Queued,
    Sent,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MailOutboxRecord {
    pub id: String,
    pub process_instance_id: String,
    pub execution_id: String,
    pub activity_id: String,
    pub recipient: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub recipients: Vec<String>,
    pub subject: String,
    pub body: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub html_body: Option<String>,
    pub status: MailOutboxStatus,
    pub created_at: i64,
}

/// Unified event subscription kind: replaces the previous parallel
/// `message_ref` / `signal_ref` optional fields with a single discriminated type.
///
/// `EventRegistry` is the BPMN wait-state kind for inbound Event Registry events
/// (`flowable:eventType` extension). Java stores these as event-type subscriptions
/// (`BpmnEventRegistryEventConsumer` / `EventSubscriptionManager.insertEventRegistryEvent`).
/// Serde unit-variant name is additive and backward-compatible for older rows.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum EventSubscriptionKind {
    Message,
    Signal,
    Conditional,
    Error,
    Cancel,
    Compensate,
    Escalation,
    /// Event Registry inbound event (`eventType` extension → event definition key).
    EventRegistry,
}

/// An active event subscription attached to an execution.
/// Replaces the old "message_ref + signal_ref" pair with a single
/// `(EventSubscriptionKind, String)` tuple.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventSubscription {
    pub kind: EventSubscriptionKind,
    pub event_ref: String,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum RuntimeEventWaitKind {
    ReceiveTask,
    MessageIntermediateCatchEvent,
    SignalIntermediateCatchEvent,
    ConditionalIntermediateCatchEvent,
    ErrorIntermediateCatchEvent,
    CancelIntermediateCatchEvent,
    CompensateIntermediateCatchEvent,
    EscalationIntermediateCatchEvent,
    /// Intermediate catch waiting on an Event Registry `eventType`.
    EventRegistryIntermediateCatchEvent,
    /// Send-event service task waiting for inbound trigger
    /// (Java `SendEventTaskActivityBehavior.java:140-151` EventSubscription +
    /// `:230-265` trigger path; P130).
    SendEventTask,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeEventWaitState {
    pub wait_kind: RuntimeEventWaitKind,
    pub process_instance_id: String,
    pub execution_id: String,
    pub task_id: Option<String>,
    pub activity_id: Option<String>,
    /// Human-readable BPMN activity name for visibility only.
    /// This is intentionally separate from the event subscription ref.
    pub display_name: Option<String>,
    /// Unified event subscription (kind + ref). `None` for "none" intermediate catch events
    /// and receive tasks without an explicit messageRef.
    pub event_subscription: Option<EventSubscription>,
    /// Event-registry correlation key (Java `EventSubscriptionEntity.configuration`).
    /// Populated from `flowable:eventCorrelationParameter` at subscription create time
    /// (`CorrelationUtil.java:30-67`). `None` matches any event correlation key
    /// (`BaseEventRegistryEventConsumer.java:163-174`). serde default keeps legacy rows loadable.
    #[serde(default)]
    pub configuration: Option<String>,
}

/// Runtime state for an active boundary event attached to a wait-state activity.
/// This represents a message or signal boundary event attached to a waiting user task/receive task.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeBoundaryEventState {
    pub boundary_event_id: String,
    pub attached_activity_id: String,
    pub process_instance_id: String,
    pub host_execution_id: String,
    /// Whether this boundary event cancels the host activity when triggered.
    /// true = interrupting (cancels host), false = non-interrupting (preserves host).
    pub cancel_activity: bool,
    /// Unified event subscription (kind + ref).
    pub event_subscription: EventSubscription,
    /// Event-registry correlation key (Java `EventSubscriptionEntity.configuration`).
    /// See `BoundaryEventRegistryEventActivityBehavior.java:68`. serde default for legacy rows.
    #[serde(default)]
    pub configuration: Option<String>,
}

// ── Timer State ──

/// Java-compatible job handler type constants (query param `handlerType`).
pub mod job_handler_types {
    pub const ASYNC_CONTINUATION: &str = "async-continuation";
    pub const ASYNC_AFTER: &str = "async-after";
    pub const TRIGGER_TIMER: &str = "trigger-timer";
    pub const ASYNC_HISTORY: &str = "async-history";
    /// Java `SetAsyncVariablesJobHandler.TYPE`.
    pub const SET_ASYNC_VARIABLES: &str = "set-async-variables";
    /// Java `AsyncCompleteCallActivityJobHandler.TYPE` — note the original
    /// Java misspelling "actiivty" is part of the wire contract.
    pub const ASYNC_COMPLETE_CALL_ACTIVITY: &str = "async-complete-call-actiivty";
    /// Java `ExternalWorkerTaskCompleteJobHandler.TYPE` — service-task wait jobs.
    pub const EXTERNAL_WORKER_COMPLETE: &str = "external-worker-complete";
    /// Java `BpmnHistoryCleanupJobHandler.TYPE` (`BpmnHistoryCleanupJobHandler.java:27`).
    pub const BPMN_HISTORY_CLEANUP: &str = "bpmn-history-cleanup";
}

/// Stamp create-time metadata on a newly created job.
///
/// `correlation_id` is generated when absent (Java assigns a stable UUID at insert).
/// Legacy/read paths leave these fields null and fall back to execution joins.
/// Query-visible dimensions already set by the caller (category, scope_*) are kept.
pub fn stamp_new_job_metadata(
    job: &mut RuntimeTimerJobState,
    now_ms: i64,
    handler_type: &str,
    tenant_id: Option<String>,
    process_definition_id: Option<String>,
    element_name: Option<String>,
) {
    if job.create_time.is_none() {
        job.create_time = Some(now_ms);
    }
    if job.correlation_id.is_none() {
        job.correlation_id = Some(uuid::Uuid::new_v4().to_string());
    }
    if job.handler_type.is_none() {
        job.handler_type = Some(handler_type.to_string());
    }
    if job.tenant_id.is_none() {
        job.tenant_id = tenant_id;
    }
    if job.process_definition_id.is_none() {
        job.process_definition_id = process_definition_id;
    }
    if job.element_name.is_none() {
        job.element_name = element_name;
    }
}

/// Copy every query-visible job dimension from `from` onto `to`.
///
/// Used by family transitions that rebuild a destination row (or when a
/// partial update must re-attach metadata). Existing non-empty values on
/// `to` win so callers can intentionally override a subset.
pub fn copy_job_query_metadata(from: &RuntimeTimerJobState, to: &mut RuntimeTimerJobState) {
    if to.category.is_none() {
        to.category = from.category.clone();
    }
    if to.correlation_id.is_none() {
        to.correlation_id = from.correlation_id.clone();
    }
    if to.handler_type.is_none() {
        to.handler_type = from.handler_type.clone();
    }
    if to.tenant_id.is_none() {
        to.tenant_id = from.tenant_id.clone();
    }
    if to.process_definition_id.is_none() {
        to.process_definition_id = from.process_definition_id.clone();
    }
    if to.element_name.is_none() {
        to.element_name = from.element_name.clone();
    }
    if to.scope_type.is_none() {
        to.scope_type = from.scope_type.clone();
    }
    if to.scope_id.is_none() {
        to.scope_id = from.scope_id.clone();
    }
    if to.sub_scope_id.is_none() {
        to.sub_scope_id = from.sub_scope_id.clone();
    }
    if to.scope_definition_id.is_none() {
        to.scope_definition_id = from.scope_definition_id.clone();
    }
    if to.create_time.is_none() {
        to.create_time = from.create_time;
    }
    if to.job_handler_configuration.is_none() {
        to.job_handler_configuration = from.job_handler_configuration.clone();
    }
    if to.advanced_job_handler_configuration.is_none() {
        to.advanced_job_handler_configuration = from.advanced_job_handler_configuration.clone();
    }
    if to.custom_values.is_none() {
        to.custom_values = from.custom_values.clone();
    }
}

/// Marker stored in `time_duration` for async continuation jobs
/// (mirrors `continue_process_operation::ASYNC_CONTINUATION_JOB_TYPE_MARKER`).
const ASYNC_CONTINUATION_MARKER: &str = "__flowable_async_continuation";
/// Marker stored in `time_duration` for async-after jobs.
const ASYNC_AFTER_MARKER: &str = "__flowable_async_after";

/// Infer a stable handler type when the caller did not set one.
///
/// Prefers Java-compatible names for async continuation; keeps family-style
/// names (`timer`, `history`, …) for timer/history so existing query filters
/// and tests that seed jobs without explicit handler_type continue to work.
fn infer_handler_type(job: &RuntimeTimerJobState) -> &'static str {
    if job.time_duration.as_deref() == Some(ASYNC_CONTINUATION_MARKER) {
        return job_handler_types::ASYNC_CONTINUATION;
    }
    if job.time_duration.as_deref() == Some(ASYNC_AFTER_MARKER) {
        return job_handler_types::ASYNC_AFTER;
    }
    match job.job_state.as_deref() {
        // Seeded/legacy rows without the async marker keep short family names so
        // existing handlerType=async/timer filters continue to match.
        Some("async") => "async",
        Some("async-after") => "async-after",
        Some("history") => "history",
        Some("suspended") => "suspended",
        Some("deadletter") => "deadletter",
        Some("executable") => "timer",
        Some("timer") | None => "timer",
        Some(_) => "timer",
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeTimerJobState {
    pub timer_job_id: String,
    pub process_instance_id: String,
    pub execution_id: String,
    pub activity_id: String,
    #[serde(default)]
    pub job_state: Option<String>,

    pub is_boundary: bool,
    pub attached_activity_id: Option<String>,
    pub cancel_activity: bool,

    pub time_duration: Option<String>,
    pub time_date: Option<String>,
    pub time_cycle: Option<String>,
    /// Optional cycle end bound from `flowable:endDate` / `activiti:endDate` on timeCycle.
    #[serde(default)]
    pub end_date: Option<String>,
    /// Raw `flowable:businessCalendarName` / `<calendar>` text (literal or `${…}`).
    /// P64/ADR-2: persisted unresolved so repeats and reschedules re-evaluate it.
    #[serde(default)]
    pub calendar_name: Option<String>,

    pub due_time: Option<i64>,
    pub lock_owner: Option<String>,
    pub lock_time: Option<i64>,
    #[serde(default)]
    pub lock_expiration_time: Option<i64>,
    #[serde(default)]
    pub retries: Option<i32>,
    #[serde(default)]
    pub error_message: Option<String>,
    #[serde(default)]
    pub error_details: Option<String>,
    #[serde(default)]
    pub category: Option<String>,
    /// Engine clock millis when the job row was created (Java `createTime`).
    #[serde(default)]
    pub create_time: Option<i64>,
    /// Stable correlation id across family moves (Java `correlationId`).
    #[serde(default)]
    pub correlation_id: Option<String>,
    /// Real job handler type (Java `jobHandlerType` / query `handlerType`).
    #[serde(default)]
    pub handler_type: Option<String>,
    /// Denormalized tenant id for direct filtering (legacy rows may be null).
    #[serde(default)]
    pub tenant_id: Option<String>,
    /// Denormalized process definition id for direct filtering.
    #[serde(default)]
    pub process_definition_id: Option<String>,
    /// BPMN element display name (Java `elementName`).
    #[serde(default)]
    pub element_name: Option<String>,
    /// Job handler configuration payload (history jobs: batch JSON / cfg string).
    #[serde(default)]
    pub job_handler_configuration: Option<String>,
    /// Advanced history handler configuration (Java `advancedJobHandlerConfiguration`).
    #[serde(default)]
    pub advanced_job_handler_configuration: Option<String>,
    /// Custom values JSON string (Java `customValues`).
    #[serde(default)]
    pub custom_values: Option<String>,
    /// History/CMMN scope type when applicable (Java `Job.scopeType`).
    #[serde(default)]
    pub scope_type: Option<String>,
    /// Non-process scope id (e.g. CMMN case instance id). Java `Job.scopeId`.
    #[serde(default)]
    pub scope_id: Option<String>,
    /// Sub-scope id (e.g. plan item instance id). Java `Job.subScopeId`.
    #[serde(default)]
    pub sub_scope_id: Option<String>,
    /// Scope definition id (e.g. case definition id). Java `Job.scopeDefinitionId`.
    #[serde(default)]
    pub scope_definition_id: Option<String>,
    /// Java `JobEntity.isExclusive` (AbstractJobEntityImpl.DEFAULT_EXCLUSIVE = true).
    /// Exclusive jobs serialize per process instance through the PI scope lock
    /// taken by the async executor before execution (P48).
    #[serde(default = "default_job_exclusive")]
    pub exclusive: bool,
}

fn default_job_exclusive() -> bool {
    true
}

impl Default for RuntimeTimerJobState {
    /// Manual impl so `exclusive` defaults to `true`, matching Java
    /// `AbstractJobEntityImpl.DEFAULT_EXCLUSIVE` (timers and async jobs are
    /// exclusive unless the creation site passes an explicit `false`, e.g.
    /// `StartProcessInstanceAsyncCmd.java:71`).
    fn default() -> Self {
        Self {
            timer_job_id: String::new(),
            process_instance_id: String::new(),
            execution_id: String::new(),
            activity_id: String::new(),
            job_state: None,
            is_boundary: false,
            attached_activity_id: None,
            cancel_activity: false,
            time_duration: None,
            time_date: None,
            time_cycle: None,
            end_date: None,
            calendar_name: None,
            due_time: None,
            lock_owner: None,
            lock_time: None,
            lock_expiration_time: None,
            retries: None,
            error_message: None,
            error_details: None,
            category: None,
            create_time: None,
            correlation_id: None,
            handler_type: None,
            tenant_id: None,
            process_definition_id: None,
            element_name: None,
            job_handler_configuration: None,
            advanced_job_handler_configuration: None,
            custom_values: None,
            scope_type: None,
            scope_id: None,
            sub_scope_id: None,
            scope_definition_id: None,
            exclusive: default_job_exclusive(),
        }
    }
}

// ── Process Instance exclusive-scope lock row ──

/// Exclusive-job scope lock of a process instance (P48).
/// Java: ACT_RU_EXECUTION.LOCK_OWNER_/LOCK_TIME_ on the PI execution row
/// (`DefaultInternalJobManager.lockJobScopeInternal`, 184-215). `lock_time`
/// stores the *expiration* instant in epoch millis, mirroring Java LOCK_TIME_.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcessInstanceLockState {
    pub process_instance_id: String,
    pub lock_owner: Option<String>,
    pub lock_time: Option<i64>,
}

// ── Process Timer Start Subscription ──

/// A process-level timer start subscription registered from a deployed process definition.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcessTimerStartSubscription {
    #[serde(default)]
    pub id: String,
    pub process_definition_id: String,
    pub process_definition_key: String,
    pub start_event_id: String,
    pub start_event_name: Option<String>,
    pub interrupting: bool,
    pub time_duration: Option<String>,
    pub time_date: Option<String>,
    pub time_cycle: Option<String>,
    /// Optional cycle end bound from `flowable:endDate` on the timer definition.
    #[serde(default)]
    pub end_date: Option<String>,
    /// Raw `flowable:businessCalendarName` / `<calendar>` text (literal or `${…}`).
    /// P64/ADR-2: persisted unresolved so repeats and reschedules re-evaluate it.
    #[serde(default)]
    pub calendar_name: Option<String>,
    pub due_time: Option<i64>,
    pub lock_owner: Option<String>,
    pub lock_time: Option<i64>,
    /// Resolved job category from the start-event `flowable:jobCategory` extension.
    /// `None` for uncategorized subscriptions (or legacy serialized rows).
    #[serde(default)]
    pub category: Option<String>,
}

// ── Event Subprocess Timer Subscription ──

/// A timer subscription for an event subprocess within a running process instance.
/// Registered when the process instance enters a scope that contains a timer-triggered event subprocess.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventSubprocessTimerSubscription {
    pub subscription_id: String,
    pub process_instance_id: String,
    pub event_subprocess_id: String,
    pub start_event_id: String,
    pub interrupting: bool,
    pub time_duration: Option<String>,
    pub time_date: Option<String>,
    pub time_cycle: Option<String>,
    /// Optional cycle end bound from `flowable:endDate` on the timer definition.
    #[serde(default)]
    pub end_date: Option<String>,
    /// Raw `flowable:businessCalendarName` / `<calendar>` text (literal or `${…}`).
    /// P64/ADR-2: persisted unresolved so repeats and reschedules re-evaluate it.
    #[serde(default)]
    pub calendar_name: Option<String>,
    pub due_time: Option<i64>,
    pub lock_owner: Option<String>,
    pub lock_time: Option<i64>,
    /// Resolved job category from the start-event `flowable:jobCategory` extension.
    /// `None` for uncategorized subscriptions (or legacy serialized rows).
    #[serde(default)]
    pub category: Option<String>,
}

// ── Process Event Start Subscription (message/signal) ──

/// A process-level message/signal start subscription registered from a deployed process definition.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcessEventStartSubscription {
    pub process_definition_id: String,
    pub process_definition_key: String,
    #[serde(default)]
    pub tenant_id: Option<String>,
    pub start_event_id: String,
    pub start_event_name: Option<String>,
    pub event_kind: EventSubscriptionKind,
    pub event_ref: String,
    /// Event-registry correlation key (Java `EventSubscriptionEntity.configuration`).
    /// Deploy-time static evaluation of `eventCorrelationParameter`
    /// (`EventSubscriptionManager.insertEventRegistryEvent:241` /
    /// `CorrelationUtil.java:53-54`). serde default for legacy rows.
    #[serde(default)]
    pub configuration: Option<String>,
}

// ── Event Subprocess Event Subscription (message/signal) ──

/// A message/signal subscription for an event subprocess within a running process instance.
/// Registered when the process instance enters a scope that contains a message/signal-triggered event subprocess.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventSubprocessEventSubscription {
    pub subscription_id: String,
    pub process_instance_id: String,
    #[serde(default)]
    pub scope_execution_id: Option<String>,
    #[serde(default)]
    pub scope_activity_id: Option<String>,
    pub event_subprocess_id: String,
    pub start_event_id: String,
    pub interrupting: bool,
    pub event_kind: EventSubscriptionKind,
    pub event_ref: String,
    /// Event-registry correlation key (Java `EventSubscriptionEntity.configuration`).
    /// See `ProcessInstanceHelper` event-subprocess registry path /
    /// `CorrelationUtil.java:30-67`. serde default for legacy rows.
    #[serde(default)]
    pub configuration: Option<String>,
}

// ── Timer Worker Coordination ──

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimerWorkerNode {
    pub node_id: String,
    pub last_heartbeat: i64,
    pub worker_type: String, // "embedded" or "standalone"
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimerCoordinatorLease {
    pub id: String, // e.g., "coordinator"
    pub owner_node_id: String,
    pub expiry_time: i64,
    pub fencing_token: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EngineProperty {
    #[serde(rename = "NAME_")]
    pub name: String,
    #[serde(rename = "VALUE_")]
    pub value: String,
    #[serde(rename = "REV_")]
    pub revision: i32,
}

// ── Token Revocation ──

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeTokenRevocation {
    pub jti: String,
    pub issuer: String,
    pub reason: String,
    pub expires_at: i64,
    #[serde(default)]
    pub created_at: i64,
}

// ── Public API structures for control surface ──

/// Status of the timer coordinator leadership
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CoordinatorLeadershipStatus {
    /// No leader currently holds the lease
    NoLeader,
    /// Current leader is active and lease is valid
    Active,
    /// Lease exists but has expired (leader may be dead)
    Expired,
}

/// Public view of timer coordinator status
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimerCoordinatorStatus {
    /// Current leader node ID, empty if no leader
    pub leader_node_id: String,
    /// Current fencing token (0 if no leader)
    pub fencing_token: i64,
    /// Lease expiry time in milliseconds since epoch
    pub lease_expiry_time: i64,
    /// Leadership status
    pub status: CoordinatorLeadershipStatus,
}

/// Status of a timer worker node
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum NodeStatus {
    /// Node has sent a heartbeat recently
    Active,
    /// Node's last heartbeat is older than the heartbeat timeout
    Expired,
}

/// Public view of a timer worker node
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimerNodeStatus {
    /// Node identifier
    pub node_id: String,
    /// Last heartbeat time in milliseconds since epoch
    pub last_heartbeat: i64,
    /// Type of worker: "embedded" or "standalone"
    pub worker_type: String,
    /// Current node status
    pub status: NodeStatus,
}

// ── Caller-stable type aliases ──
pub type RuntimeMessageStyleWaitKind = RuntimeEventWaitKind;
pub type RuntimeMessageStyleWaitState = RuntimeEventWaitState;

#[derive(Clone)]
pub struct RuntimeStore {
    pub(crate) db_store: Arc<DbStore>,
    pub(crate) session_factory: Arc<
        dyn Fn() -> Result<
                crate::persistence::db_session::DbSession,
                crate::persistence::storage_error::StorageError,
            > + Send
            + Sync,
    >,
    time_source: Arc<dyn TimeSource>,
    bpmn_model_cache: Option<Arc<crate::engine::bpmn_model_cache::BpmnModelCache>>,
}

/// All user-task properties resolvable from a single XML parse (Task 6).
/// Replaces separate resolve_user_task_* calls (12-16 DB queries + repeated XML parses)
/// with one pass (3-4 DB queries + 1 XML parse, cached thereafter).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UserTaskProperties {
    pub assignee: Option<String>,
    pub owner: Option<String>,
    pub priority: Option<i32>,
    pub due_date: Option<DateTime<Utc>>,
    pub category: Option<String>,
    pub form_key: Option<String>,
}

impl RuntimeStore {
    pub fn new_with_memory_backend_for_test(db_store: Arc<DbStore>) -> Self {
        let db_store_clone = Arc::clone(&db_store);
        let session_factory: Arc<dyn Fn() -> Result<DbSession, StorageError> + Send + Sync> =
            Arc::new(move || db_store_clone.create_session());
        Self {
            db_store,
            session_factory,
            time_source: Arc::new(SystemTimeSource),
            bpmn_model_cache: None,
        }
    }

    pub fn with_time_source_for_test(
        db_store: Arc<DbStore>,
        time_source: Arc<dyn TimeSource>,
    ) -> Self {
        let db_store_clone = Arc::clone(&db_store);
        let session_factory: Arc<dyn Fn() -> Result<DbSession, StorageError> + Send + Sync> =
            Arc::new(move || db_store_clone.create_session());
        Self {
            db_store,
            session_factory,
            time_source,
            bpmn_model_cache: None,
        }
    }

    pub fn with_backend(
        db_store: Arc<DbStore>,
        session_factory: Arc<dyn Fn() -> Result<DbSession, StorageError> + Send + Sync>,
        time_source: Arc<dyn TimeSource>,
    ) -> Self {
        Self {
            db_store,
            session_factory,
            time_source,
            bpmn_model_cache: None,
        }
    }

    pub fn create_session(&self) -> Result<DbSession, StorageError> {
        (self.session_factory)()
    }

    /// Builder: inject a BpmnModelCache so resolve_user_task_properties can reuse
    /// parsed BpmnModels across calls. When unset, falls back to direct XML parsing.
    pub fn with_bpmn_model_cache(
        mut self,
        cache: Arc<crate::engine::bpmn_model_cache::BpmnModelCache>,
    ) -> Self {
        self.bpmn_model_cache = Some(cache);
        self
    }

    pub fn time_source(&self) -> Arc<dyn TimeSource> {
        Arc::clone(&self.time_source)
    }

    pub fn db_store(&self) -> &Arc<DbStore> {
        &self.db_store
    }

    pub fn insert_execution(
        &self,
        execution: &Execution,
        session: &mut DbSession,
    ) -> Result<(), StorageError> {
        let process_instance_id = execution.process_instance_id.clone().unwrap_or_default();

        // P58: register the owning process instance for the end-of-command
        // inactive-behavior re-evaluation (Java involved executions).
        mark_involved_process_instance(execution);

        // P45: track rows that still carry in-command transient so commit can
        // strip them without scanning every execution in the store.
        if !execution.transient_variables.is_empty() {
            mark_transient_dirty_execution(&execution.id);
        }

        // Sweep before projecting so the table mirrors the two maps exactly:
        // names dropped from the maps since the last write (e.g. the data
        // input association wholesale replace) must not linger as orphan rows.
        // One extra delete_by per execution write — cheap next to the
        // normalized DataManager dual-write below.
        self.delete_variables_by_execution_id(&execution.id, session);

        // Project the Java-equivalent row-level LOCAL scope onto the runtime
        // `variables` table. One execution row is two maps in Rust
        // (`variables` ∪ `local_variables`); the projection key is
        // `{execution_id}:{name}`, so process variables go first and
        // `local_variables` overwrite on a name clash — the same precedence
        // `Execution::process_variable` applies. Without the local pass,
        // `SetVariablesLocalCmd` writes were invisible to
        // variable-instance queries (P4-1 patched only the REST mutation path).
        for (name, value) in &execution.variables {
            let id = format!("{}:{}", execution.id, name);
            session.insert_with_extra(
                "variables",
                &id,
                value,
                &[
                    ("execution_id".into(), Some(execution.id.clone())),
                    (
                        "process_instance_id".into(),
                        Some(process_instance_id.clone()),
                    ),
                    ("name".into(), Some(name.clone())),
                ],
            )?;
        }
        for (name, value) in &execution.local_variables {
            let id = format!("{}:{}", execution.id, name);
            session.insert_with_extra(
                "variables",
                &id,
                value,
                &[
                    ("execution_id".into(), Some(execution.id.clone())),
                    (
                        "process_instance_id".into(),
                        Some(process_instance_id.clone()),
                    ),
                    ("name".into(), Some(name.clone())),
                ],
            )?;
        }

        session.insert_with_extra(
            "executions",
            &execution.id,
            &execution,
            &[("process_instance_id".into(), Some(process_instance_id))],
        )?;

        // ADR-0001 Phase 5: dual-write normalized ACT_RU_EXECUTION via DataManager.
        // Prefer update when the row already exists (JSON path uses upsert semantics).
        //
        // Hard-fail on dual-write errors (P73a): DataManager insert/update only queue
        // until flush. We flush immediately so SQL failures surface here with dual-write
        // context instead of being deferred (or swallowed by a later `let _ = flush`).
        // On PostgreSQL a failed statement aborts the whole transaction — silent swallow
        // either pollutes later work or allows JSON primary writes to diverge from ACT_*.
        Self::dual_write_execution(session, execution);
        Ok(())
    }

    /// Queue + immediately flush ACT_RU_EXECUTION dual-write; sticky-notes failures
    /// so the session commit fails (Java command/transaction failure semantics).
    fn dual_write_execution(session: &mut DbSession, execution: &Execution) {
        let manager = flowable_persistence::ExecutionDataManager::new();
        match manager.find_by_id(session.inner_mut(), &execution.id) {
            Ok(Some(existing)) => {
                let mut entity = crate::persistence::entity_mapping::execution_to_entity(execution);
                entity.revision = existing.revision;
                if let Err(err) = manager.update(session.inner_mut(), entity) {
                    session.note_write_error(crate::persistence::StorageError::Persistence(
                        format!(
                            "dual-write ACT_RU_EXECUTION update failed for id={}: {err}",
                            execution.id
                        ),
                    ));
                }
            }
            Ok(None) => {
                let entity = crate::persistence::entity_mapping::execution_to_entity(execution);
                if let Err(err) = manager.insert(session.inner_mut(), entity) {
                    session.note_write_error(crate::persistence::StorageError::Persistence(
                        format!(
                            "dual-write ACT_RU_EXECUTION insert failed for id={}: {err}",
                            execution.id
                        ),
                    ));
                }
            }
            Err(err) => {
                session.note_write_error(crate::persistence::StorageError::Persistence(format!(
                    "dual-write ACT_RU_EXECUTION find_by_id failed for id={}: {err}",
                    execution.id
                )));
            }
        }
        // Force SQL now so failures are attributed to dual-write, not a later flush.
        if let Err(err) = session.inner_mut().flush() {
            session.note_write_error(crate::persistence::StorageError::Persistence(format!(
                "dual-write ACT_RU_EXECUTION flush failed for id={}: {err}",
                execution.id
            )));
        }
    }

    pub fn update_execution(
        &self,
        execution: &Execution,
        session: &mut DbSession,
    ) -> Result<(), StorageError> {
        self.insert_execution(execution, session)
    }

    /// Java parity (P45): `VariableScopeImpl.transientVariables` are pure memory
    /// and vanish when the command/transaction ends. Mid-command we keep them on
    /// the execution JSON so same-command reloads (call-activity inheritVariables,
    /// async resume flags, PENDING_FUTURE_ID) still work; this rewrites only
    /// executions written this command with non-empty transient, clearing the map
    /// immediately before commit.
    pub fn strip_transient_variables_before_commit(&self, session: &mut DbSession) {
        let dirty_ids = take_transient_dirty_execution_ids();
        for id in dirty_ids {
            let Some(mut execution) = self.find_execution(&id, session) else {
                continue;
            };
            if execution.transient_variables.is_empty() {
                continue;
            }
            execution.transient_variables.clear();
            // Rewrite only the execution JSON (and ACT_RU_EXECUTION dual-write).
            // Do not re-project the `variables` table — durable/local maps are
            // unchanged and a full insert_execution would needlessly delete+rewrite
            // every variable row.
            self.rewrite_execution_json(&execution, session);
        }
    }

    /// Upserts the execution row JSON + normalized ACT_RU_EXECUTION without
    /// touching the projected `variables` table.
    fn rewrite_execution_json(&self, execution: &Execution, session: &mut DbSession) {
        let process_instance_id = execution.process_instance_id.clone().unwrap_or_default();
        session
            .insert_with_extra(
                "executions",
                &execution.id,
                execution,
                &[("process_instance_id".into(), Some(process_instance_id))],
            )
            .unwrap_or_default();

        // Hard-fail dual-write (P73a): same rationale as insert_execution.
        Self::dual_write_execution(session, execution);
    }

    pub fn delete_execution(&self, id: &str, session: &mut DbSession) {
        // P58: a destroyed execution also marks its process instance as
        // involved — destroying a branch is exactly what can unblock a parked
        // inclusive join (Java registers deletes as involved executions too).
        if let Ok(Some(execution)) = session.find::<Execution>("executions", id) {
            mark_involved_process_instance(&execution);
        }
        self.delete_variables_by_execution_id(id, session);
        let _ = session.delete("executions", id);
        // Dual-delete ACT_RU_EXECUTION: queue + flush immediately; sticky-note failures.
        match flowable_persistence::ExecutionDataManager::new().find_by_id(session.inner_mut(), id)
        {
            Ok(Some(entity)) => {
                if let Err(err) = flowable_persistence::ExecutionDataManager::new()
                    .delete(session.inner_mut(), &entity)
                {
                    session.note_write_error(crate::persistence::StorageError::Persistence(
                        format!("dual-delete ACT_RU_EXECUTION failed for id={id}: {err}"),
                    ));
                }
                if let Err(err) = session.inner_mut().flush() {
                    session.note_write_error(crate::persistence::StorageError::Persistence(
                        format!("dual-delete ACT_RU_EXECUTION flush failed for id={id}: {err}"),
                    ));
                }
            }
            Ok(None) => {}
            Err(err) => {
                session.note_write_error(crate::persistence::StorageError::Persistence(format!(
                    "dual-delete ACT_RU_EXECUTION find_by_id failed for id={id}: {err}"
                )));
            }
        }
    }

    pub fn find_execution(&self, id: &str, session: &mut DbSession) -> Option<Execution> {
        match session.find("executions", id) {
            Ok(found) => found,
            Err(error) => {
                session.note_write_error(error);
                None
            }
        }
    }

    pub fn snapshot_executions(&self, session: &mut DbSession) -> HashMap<String, Execution> {
        {
            match session.find_all::<Execution>("executions") {
                Ok(found) => found,
                Err(error) => {
                    session.note_write_error(error);
                    Vec::new()
                }
            }
        }
        .into_iter()
        .map(|e| (e.id.clone(), e))
        .collect()
    }

    pub fn insert_process_instance(
        &self,
        process_instance: &ProcessInstance,
        session: &mut DbSession,
    ) {
        session
            .insert_with_extra(
                "process_instances",
                &process_instance.id,
                process_instance,
                &[(
                    "process_definition_id".into(),
                    Some(process_instance.process_definition_id.clone()),
                )],
            )
            .unwrap_or_default();
    }

    pub fn update_process_instance(
        &self,
        process_instance: &ProcessInstance,
        session: &mut DbSession,
    ) {
        self.insert_process_instance(process_instance, session);
    }

    pub fn delete_process_instance(&self, id: &str, session: &mut DbSession) {
        let _ = session.delete("process_instances", id);
        // A finished/removed PI must not leave a dangling exclusive-scope lock row.
        let _ = session.delete("process_instance_locks", id);
    }

    pub fn find_process_instance(
        &self,
        id: &str,
        session: &mut DbSession,
    ) -> Result<Option<ProcessInstance>, StorageError> {
        session.find("process_instances", id)
    }

    pub fn snapshot_process_instances(
        &self,
        session: &mut DbSession,
    ) -> HashMap<String, ProcessInstance> {
        {
            match session.find_all::<ProcessInstance>("process_instances") {
                Ok(found) => found,
                Err(error) => {
                    session.note_write_error(error);
                    Vec::new()
                }
            }
        }
        .into_iter()
        .map(|e| (e.id.clone(), e))
        .collect()
    }

    // ── Process Instance exclusive-scope lock (P48) ──
    //
    // Java stores the exclusive-job scope lock on the process-instance
    // execution row (ACT_RU_EXECUTION.LOCK_OWNER_/LOCK_TIME_, where LOCK_TIME_
    // holds the *expiration* instant). The Rust `process_instances` row is a
    // whole-document upsert, so the lock lives in a dedicated CAS-able side
    // row keyed by the process instance id.

    /// Try to lock the process-instance scope for an exclusive job.
    ///
    /// Java `MybatisExecutionDataManager.updateProcessInstanceLockTime`
    /// (302-313): conditional `UPDATE ... WHERE LOCK_TIME_ IS NULL OR
    /// LOCK_TIME_ < now`; zero affected rows raises
    /// `FlowableOptimisticLockingException`. Here the conflict is reported as
    /// `false` and the caller unacquires the job without executing it.
    pub fn lock_process_instance(
        &self,
        process_instance_id: &str,
        lock_owner: &str,
        lock_expiration_ms: i64,
        now: i64,
        session: &mut DbSession,
    ) -> bool {
        let existing: Option<ProcessInstanceLockState> = {
            match session.find("process_instance_locks", process_instance_id) {
                Ok(found) => found,
                Err(error) => {
                    session.note_write_error(error);
                    None
                }
            }
        };
        match existing {
            None => {
                let state = ProcessInstanceLockState {
                    process_instance_id: process_instance_id.to_string(),
                    lock_owner: Some(lock_owner.to_string()),
                    lock_time: Some(lock_expiration_ms),
                };
                // Plain INSERT: a concurrent first-locker must not be silently
                // overwritten (mirrors the 0-rows-updated optimistic conflict).
                session
                    .insert_exclusive_with_extra(
                        "process_instance_locks",
                        process_instance_id,
                        &state,
                        &[
                            ("lock_owner".into(), state.lock_owner.clone()),
                            ("lock_time".into(), state.lock_time.map(|v| v.to_string())),
                        ],
                    )
                    .is_ok()
            }
            Some(current) => {
                // Free or expired locks may be taken over (LOCK_TIME_ stores
                // the expiration instant, compared against `now`).
                let takeable = match current.lock_time {
                    None => true,
                    Some(expiration) => expiration < now,
                };
                if !takeable {
                    return false;
                }
                let updated = ProcessInstanceLockState {
                    process_instance_id: process_instance_id.to_string(),
                    lock_owner: Some(lock_owner.to_string()),
                    lock_time: Some(lock_expiration_ms),
                };
                let json = serde_json::to_string(&updated).unwrap_or_else(|_| "{}".to_string());
                let mut conditions: Vec<(String, Option<String>)> =
                    vec![("lock_owner".into(), current.lock_owner.clone())];
                conditions.push(("lock_time".into(), current.lock_time.map(|v| v.to_string())));
                session
                    .cas_update(
                        "process_instance_locks",
                        process_instance_id,
                        &json,
                        &[
                            ("lock_owner".into(), updated.lock_owner.clone()),
                            ("lock_time".into(), updated.lock_time.map(|v| v.to_string())),
                        ],
                        &conditions,
                    )
                    .map(|affected| affected > 0)
                    .unwrap_or(false)
            }
        }
    }

    /// Clear the process-instance scope lock.
    ///
    /// Java `MybatisExecutionDataManager.clearProcessInstanceLockTime`
    /// (321-325) nulls LOCK_TIME_/LOCK_OWNER_ unconditionally for the PI row;
    /// here the side row is removed, which is observably identical.
    pub fn clear_process_instance_lock(&self, process_instance_id: &str, session: &mut DbSession) {
        let _ = session.delete("process_instance_locks", process_instance_id);
    }

    /// Current exclusive-scope lock of a process instance (None = unlocked).
    pub fn find_process_instance_lock(
        &self,
        process_instance_id: &str,
        session: &mut DbSession,
    ) -> Option<ProcessInstanceLockState> {
        {
            match session.find("process_instance_locks", process_instance_id) {
                Ok(found) => found,
                Err(error) => {
                    session.note_write_error(error);
                    None
                }
            }
        }
    }

    // ── Event Registry methods ──

    pub fn insert_event_registry_deployment(
        &self,
        deployment: EventRegistryDeployment,
        session: &mut DbSession,
    ) {
        session
            .insert_with_extra(
                "event_registry_deployments",
                &deployment.id,
                &deployment,
                &[
                    ("name".into(), Some(deployment.name.clone())),
                    (
                        "deployed_at".into(),
                        Some(deployment.deployed_at.to_string()),
                    ),
                ],
            )
            .unwrap_or_default();
    }

    pub fn find_event_registry_deployment(
        &self,
        id: &str,
        session: &mut DbSession,
    ) -> Option<EventRegistryDeployment> {
        // Java parity: DbSqlSession.selectById throws on SQL error; None means a no-row query, not a storage failure.
        match session.find("event_registry_deployments", id) {
            Ok(found) => found,
            Err(error) => {
                session.note_write_error(error);
                None
            }
        }
    }

    pub fn delete_event_registry_deployment(&self, id: &str, session: &mut DbSession) {
        let _ = session.delete("event_registry_deployments", id);
        session
            .delete_by("event_registry_channel_definitions", "deployment_id", id)
            .unwrap_or_default();
        session
            .delete_by("event_registry_event_definitions", "deployment_id", id)
            .unwrap_or_default();
    }

    pub fn list_event_registry_deployments(
        &self,
        session: &mut DbSession,
    ) -> Vec<EventRegistryDeployment> {
        {
            match session.find_all("event_registry_deployments") {
                Ok(found) => found,
                Err(error) => {
                    session.note_write_error(error);
                    Vec::new()
                }
            }
        }
    }

    pub fn insert_event_registry_channel_definition(
        &self,
        definition: EventRegistryChannelDefinition,
        session: &mut DbSession,
    ) {
        session
            .insert_with_extra(
                "event_registry_channel_definitions",
                &definition.id,
                &definition,
                &[
                    (
                        "deployment_id".into(),
                        Some(definition.deployment_id.clone()),
                    ),
                    ("key".into(), Some(definition.key.clone())),
                    ("name".into(), Some(definition.name.clone())),
                    ("channel_type".into(), Some(definition.channel_type.clone())),
                    (
                        "resource_name".into(),
                        Some(definition.resource_name.clone()),
                    ),
                ],
            )
            .unwrap_or_default();
    }

    pub fn find_event_registry_channel_definition(
        &self,
        id: &str,
        session: &mut DbSession,
    ) -> Option<EventRegistryChannelDefinition> {
        // Java parity: DbSqlSession.selectById throws on SQL error; None means a no-row query, not a storage failure.
        match session.find("event_registry_channel_definitions", id) {
            Ok(found) => found,
            Err(error) => {
                session.note_write_error(error);
                None
            }
        }
    }

    pub fn find_event_registry_channel_definition_by_key(
        &self,
        key: &str,
        session: &mut DbSession,
    ) -> Option<EventRegistryChannelDefinition> {
        // Task 9: SQL WHERE pushdown on indexed `key` column replaces full-table load + memory filter.
        let definitions: Vec<EventRegistryChannelDefinition> =
            match session.find_by("event_registry_channel_definitions", "key", key) {
                Ok(found) => found,
                Err(error) => {
                    session.note_write_error(error);
                    Vec::new()
                }
            };
        definitions.into_iter().max_by(|left, right| {
            left.version
                .cmp(&right.version)
                .then(left.id.cmp(&right.id))
        })
    }

    pub fn find_event_registry_channel_definition_by_key_and_tenant(
        &self,
        key: &str,
        tenant_id: Option<&str>,
        session: &mut DbSession,
    ) -> Option<EventRegistryChannelDefinition> {
        // Task 9: SQL WHERE pushdown on `key` column; tenant filter remains in-memory (sparse).
        let definitions: Vec<EventRegistryChannelDefinition> =
            match session.find_by("event_registry_channel_definitions", "key", key) {
                Ok(found) => found,
                Err(error) => {
                    session.note_write_error(error);
                    Vec::new()
                }
            };

        if let Some(tenant_id) = tenant_id
            && let Some(definition) = definitions
                .iter()
                .filter(|definition| definition.tenant_id.as_deref() == Some(tenant_id))
                .cloned()
                .max_by(|left, right| {
                    left.version
                        .cmp(&right.version)
                        .then(left.id.cmp(&right.id))
                })
        {
            return Some(definition);
        }

        definitions
            .into_iter()
            .filter(|definition| definition.tenant_id.is_none())
            .max_by(|left, right| {
                left.version
                    .cmp(&right.version)
                    .then(left.id.cmp(&right.id))
            })
    }

    pub fn list_event_registry_channel_definitions(
        &self,
        session: &mut DbSession,
    ) -> Vec<EventRegistryChannelDefinition> {
        {
            match session.find_all("event_registry_channel_definitions") {
                Ok(found) => found,
                Err(error) => {
                    session.note_write_error(error);
                    Vec::new()
                }
            }
        }
    }

    pub fn insert_event_registry_event_definition(
        &self,
        definition: EventRegistryEventDefinition,
        session: &mut DbSession,
    ) {
        session
            .insert_with_extra(
                "event_registry_event_definitions",
                &definition.id,
                &definition,
                &[
                    (
                        "deployment_id".into(),
                        Some(definition.deployment_id.clone()),
                    ),
                    ("key".into(), Some(definition.key.clone())),
                    ("name".into(), Some(definition.name.clone())),
                    ("event_type".into(), Some(definition.event_type.clone())),
                    ("channel_key".into(), Some(definition.channel_key.clone())),
                    (
                        "resource_name".into(),
                        Some(definition.resource_name.clone()),
                    ),
                ],
            )
            .unwrap_or_default();
    }

    pub fn find_event_registry_event_definition(
        &self,
        id: &str,
        session: &mut DbSession,
    ) -> Option<EventRegistryEventDefinition> {
        {
            match session.find("event_registry_event_definitions", id) {
                Ok(found) => found,
                Err(error) => {
                    session.note_write_error(error);
                    None
                }
            }
        }
    }

    pub fn find_event_registry_event_definition_by_key(
        &self,
        key: &str,
        session: &mut DbSession,
    ) -> Option<EventRegistryEventDefinition> {
        self.list_event_registry_event_definitions(session)
            .into_iter()
            .filter(|definition| definition.key == key)
            .max_by(|left, right| {
                left.version
                    .cmp(&right.version)
                    .then(left.id.cmp(&right.id))
            })
    }

    pub fn find_event_registry_event_definition_by_key_and_tenant(
        &self,
        key: &str,
        tenant_id: Option<&str>,
        session: &mut DbSession,
    ) -> Option<EventRegistryEventDefinition> {
        let definitions = self.list_event_registry_event_definitions(session);

        if let Some(tenant_id) = tenant_id
            && let Some(definition) = definitions
                .iter()
                .filter(|definition| {
                    definition.key == key && definition.tenant_id.as_deref() == Some(tenant_id)
                })
                .cloned()
                .max_by(|left, right| {
                    left.version
                        .cmp(&right.version)
                        .then(left.id.cmp(&right.id))
                })
        {
            return Some(definition);
        }

        definitions
            .into_iter()
            .filter(|definition| definition.key == key && definition.tenant_id.is_none())
            .max_by(|left, right| {
                left.version
                    .cmp(&right.version)
                    .then(left.id.cmp(&right.id))
            })
    }

    pub fn find_event_registry_event_definitions_by_event_type(
        &self,
        event_type: &str,
        session: &mut DbSession,
    ) -> Vec<EventRegistryEventDefinition> {
        // Java parity: DbSqlSession.selectList throws on SQL error; an empty list means a no-row query, not a storage failure.
        match session.find_by("event_registry_event_definitions", "event_type", event_type) {
            Ok(found) => found,
            Err(error) => {
                session.note_write_error(error);
                Vec::new()
            }
        }
    }

    pub fn list_event_registry_event_definitions(
        &self,
        session: &mut DbSession,
    ) -> Vec<EventRegistryEventDefinition> {
        {
            match session.find_all("event_registry_event_definitions") {
                Ok(found) => found,
                Err(error) => {
                    session.note_write_error(error);
                    Vec::new()
                }
            }
        }
    }

    pub fn insert_event_registry_event_instance_delivery(
        &self,
        delivery: EventRegistryEventInstanceDelivery,
        session: &mut DbSession,
    ) -> Result<(), StorageError> {
        session.insert_with_extra(
            "event_registry_event_instance_deliveries",
            &delivery.id,
            &delivery,
            &[
                (
                    "event_definition_key".into(),
                    Some(delivery.event_definition_key.clone()),
                ),
                ("event_type".into(), Some(delivery.event_type.clone())),
                ("channel_key".into(), Some(delivery.channel_key.clone())),
                (
                    "direction".into(),
                    Some(
                        serde_json::to_string(&delivery.direction)
                            .map_err(StorageError::from)?
                            .trim_matches('"')
                            .to_string(),
                    ),
                ),
                (
                    "status".into(),
                    Some(
                        serde_json::to_string(&delivery.status)
                            .map_err(StorageError::from)?
                            .trim_matches('"')
                            .to_string(),
                    ),
                ),
                ("created_at".into(), Some(delivery.created_at.to_string())),
                ("updated_at".into(), Some(delivery.updated_at.to_string())),
            ],
        )
    }

    pub fn update_event_registry_event_instance_delivery(
        &self,
        delivery: EventRegistryEventInstanceDelivery,
        session: &mut DbSession,
    ) -> Result<(), StorageError> {
        self.insert_event_registry_event_instance_delivery(delivery, session)
    }

    pub fn update_event_registry_channel_definition(
        &self,
        definition: EventRegistryChannelDefinition,
        session: &mut DbSession,
    ) {
        self.insert_event_registry_channel_definition(definition, session);
    }

    pub fn update_event_registry_event_definition(
        &self,
        definition: EventRegistryEventDefinition,
        session: &mut DbSession,
    ) {
        self.insert_event_registry_event_definition(definition, session);
    }

    pub fn delete_event_registry_event_instance_delivery(
        &self,
        id: &str,
        session: &mut DbSession,
    ) -> Result<(), StorageError> {
        session.delete("event_registry_event_instance_deliveries", id)
    }

    pub fn find_event_registry_event_instance_delivery(
        &self,
        id: &str,
        session: &mut DbSession,
    ) -> Result<Option<EventRegistryEventInstanceDelivery>, StorageError> {
        session.find("event_registry_event_instance_deliveries", id)
    }

    pub fn list_event_registry_event_instance_deliveries(
        &self,
        session: &mut DbSession,
    ) -> Result<Vec<EventRegistryEventInstanceDelivery>, StorageError> {
        session.find_all("event_registry_event_instance_deliveries")
    }

    /// Allocates the next change revision from the single-row allocator in
    /// [`DbSession::next_event_registry_change_revision`]. Revisions are
    /// strictly monotonic and unique (enforced by a unique index), so change
    /// pollers can use a single-revision high water mark as their cursor.
    pub fn next_event_registry_change_revision(
        &self,
        session: &mut DbSession,
    ) -> Result<u64, StorageError> {
        session.next_event_registry_change_revision()
    }

    pub fn insert_event_registry_change_record(
        &self,
        record: EventRegistryChangeRecord,
        session: &mut DbSession,
    ) -> Result<(), StorageError> {
        // Change records are append-only. A plain INSERT keeps the unique
        // revision index effective: with INSERT OR REPLACE a revision clash
        // would silently delete the colliding record instead of failing.
        session.insert_exclusive_with_extra(
            "event_registry_change_records",
            &record.id,
            &record,
            &[
                ("revision".into(), Some(record.revision.to_string())),
                ("change_type".into(), Some(record.change_type.clone())),
                ("entity_type".into(), Some(record.entity_type.clone())),
                ("entity_key".into(), Some(record.entity_key.clone())),
            ],
        )
    }

    pub fn list_event_registry_change_records(
        &self,
        session: &mut DbSession,
    ) -> Vec<EventRegistryChangeRecord> {
        let mut records: Vec<EventRegistryChangeRecord> = match session.find_with_filters(
            "event_registry_change_records",
            &[],
            Some(("revision", true)),
            None,
        ) {
            Ok(found) => found,
            Err(error) => {
                session.note_write_error(error);
                Vec::new()
            }
        };
        // Stable tie-break for legacy rows that predate the unique revision index.
        records.sort_by(|left, right| {
            left.revision
                .cmp(&right.revision)
                .then(left.id.cmp(&right.id))
        });
        records
    }

    /// Bounded, resumable poll of changes strictly after `after_revision`,
    /// pushed down to SQL (`WHERE revision > ? ORDER BY revision LIMIT ?`).
    pub fn list_event_registry_change_records_after(
        &self,
        after_revision: u64,
        limit: usize,
        session: &mut DbSession,
    ) -> Vec<EventRegistryChangeRecord> {
        match session.find_with_filters(
            "event_registry_change_records",
            &[(
                "revision".to_string(),
                FilterOp::GreaterThan(after_revision as i64),
            )],
            Some(("revision", true)),
            Some(limit),
        ) {
            Ok(found) => found,
            Err(error) => {
                session.note_write_error(error);
                Vec::new()
            }
        }
    }

    // ── Forms methods ──

    pub fn insert_form_deployment(&self, deployment: FormDeployment, session: &mut DbSession) {
        session
            .insert_with_extra(
                "form_deployments",
                &deployment.id,
                &deployment,
                &[
                    ("name".into(), Some(deployment.name.clone())),
                    (
                        "deployed_at".into(),
                        Some(deployment.deployed_at.to_string()),
                    ),
                ],
            )
            .unwrap_or_default();
    }

    pub fn find_form_deployment(
        &self,
        id: &str,
        session: &mut DbSession,
    ) -> Option<FormDeployment> {
        {
            match session.find("form_deployments", id) {
                Ok(found) => found,
                Err(error) => {
                    session.note_write_error(error);
                    None
                }
            }
        }
    }

    pub fn list_form_deployments(&self, session: &mut DbSession) -> Vec<FormDeployment> {
        {
            match session.find_all("form_deployments") {
                Ok(found) => found,
                Err(error) => {
                    session.note_write_error(error);
                    Vec::new()
                }
            }
        }
    }

    pub fn insert_form_definition(&self, definition: FormDefinition, session: &mut DbSession) {
        session
            .insert_with_extra(
                "form_definitions",
                &definition.id,
                &definition,
                &[
                    (
                        "deployment_id".into(),
                        Some(definition.deployment_id.clone()),
                    ),
                    ("key".into(), Some(definition.key.clone())),
                    ("name".into(), Some(definition.name.clone())),
                    ("version".into(), Some(definition.version.to_string())),
                    (
                        "resource_name".into(),
                        Some(definition.resource_name.clone()),
                    ),
                ],
            )
            .unwrap_or_default();
    }

    pub fn find_form_definition(
        &self,
        id: &str,
        session: &mut DbSession,
    ) -> Option<FormDefinition> {
        {
            match session.find("form_definitions", id) {
                Ok(found) => found,
                Err(error) => {
                    session.note_write_error(error);
                    None
                }
            }
        }
    }

    pub fn find_form_definition_by_key(
        &self,
        key: &str,
        session: &mut DbSession,
    ) -> Option<FormDefinition> {
        // Java parity: DbSqlSession.selectList throws on SQL error; an empty list means a no-row query, not a storage failure.
        (match session.find_by::<FormDefinition>("form_definitions", "key", key) {
            Ok(found) => found,
            Err(error) => {
                session.note_write_error(error);
                Vec::new()
            }
        })
        .into_iter()
        .next()
    }

    pub fn list_form_definitions(&self, session: &mut DbSession) -> Vec<FormDefinition> {
        {
            match session.find_all("form_definitions") {
                Ok(found) => found,
                Err(error) => {
                    session.note_write_error(error);
                    Vec::new()
                }
            }
        }
    }

    // ── Content methods ──

    pub fn insert_content_item(&self, item: ContentItem, session: &mut DbSession) {
        session
            .insert_with_extra(
                "content_items",
                &item.id,
                &item,
                &[
                    ("name".into(), Some(item.name.clone())),
                    ("mime_type".into(), item.mime_type.clone()),
                    ("created_at".into(), Some(item.created_at.to_string())),
                ],
            )
            .unwrap_or_default();
    }

    pub fn update_content_item(&self, item: ContentItem, session: &mut DbSession) {
        self.insert_content_item(item, session);
    }

    pub fn find_content_item(&self, id: &str, session: &mut DbSession) -> Option<ContentItem> {
        {
            match session.find("content_items", id) {
                Ok(found) => found,
                Err(error) => {
                    session.note_write_error(error);
                    None
                }
            }
        }
    }

    pub fn list_content_items(&self, session: &mut DbSession) -> Vec<ContentItem> {
        {
            match session.find_all("content_items") {
                Ok(found) => found,
                Err(error) => {
                    session.note_write_error(error);
                    Vec::new()
                }
            }
        }
    }

    pub fn delete_content_item(&self, id: &str, session: &mut DbSession) {
        let _ = session.delete("content_items", id);
    }

    // ── Historic comment and task event methods ──

    pub fn insert_historic_comment(
        &self,
        comment: crate::history::historic_entities::HistoricComment,
        session: &mut DbSession,
    ) {
        // Project resolved type so typed queries hit the index even when the
        // JSON row still omits `comment_type` for legacy-compatible writes.
        let projected_type = comment.resolved_type().to_string();
        session
            .insert_with_extra(
                "historic_comments",
                &comment.id,
                &comment,
                &[
                    ("task_id".into(), comment.task_id.clone()),
                    (
                        "process_instance_id".into(),
                        comment.process_instance_id.clone(),
                    ),
                    (
                        "time".into(),
                        Some(comment.time.timestamp_millis().to_string()),
                    ),
                    ("comment_type".into(), Some(projected_type)),
                ],
            )
            .unwrap_or_default();
    }

    pub fn find_historic_comment(
        &self,
        comment_id: &str,
        session: &mut DbSession,
    ) -> Option<crate::history::historic_entities::HistoricComment> {
        {
            match session.find("historic_comments", comment_id) {
                Ok(found) => found,
                Err(error) => {
                    session.note_write_error(error);
                    None
                }
            }
        }
    }

    pub fn find_historic_comments_by_task_id(
        &self,
        task_id: &str,
        session: &mut DbSession,
    ) -> Vec<crate::history::historic_entities::HistoricComment> {
        // Java Comment.xml `selectCommentsByTaskId`: TYPE_ = 'comment', TIME_ desc.
        // Custom-type comments are excluded (use find_historic_comments_by_task_id_and_type).
        let mut comments: Vec<crate::history::historic_entities::HistoricComment> =
            match session.find_by("historic_comments", "task_id", task_id) {
                Ok(found) => found,
                Err(error) => {
                    session.note_write_error(error);
                    Vec::new()
                }
            };
        comments.retain(|comment| {
            comment.resolved_type()
                == crate::history::historic_entities::HistoricComment::TYPE_COMMENT
        });
        comments.sort_by(|left, right| right.time.cmp(&left.time).then(right.id.cmp(&left.id)));
        comments
    }

    pub fn find_historic_comments_by_task_id_and_type(
        &self,
        task_id: &str,
        comment_type: &str,
        session: &mut DbSession,
    ) -> Vec<crate::history::historic_entities::HistoricComment> {
        // Prefer the projected type index; fall back to resolved_type so legacy
        // rows with a NULL comment_type column still match.
        let mut comments: Vec<crate::history::historic_entities::HistoricComment> = match session
            .find_by_two(
                "historic_comments",
                "task_id",
                task_id,
                "comment_type",
                comment_type,
            ) {
            Ok(found) => found,
            Err(error) => {
                session.note_write_error(error);
                Vec::new()
            }
        };
        if comment_type == crate::history::historic_entities::HistoricComment::TYPE_COMMENT
            || comment_type == crate::history::historic_entities::HistoricComment::TYPE_EVENT
        {
            let by_task: Vec<crate::history::historic_entities::HistoricComment> =
                match session.find_by("historic_comments", "task_id", task_id) {
                    Ok(found) => found,
                    Err(error) => {
                        session.note_write_error(error);
                        Vec::new()
                    }
                };
            for comment in by_task {
                if comment.resolved_type() == comment_type
                    && !comments.iter().any(|existing| existing.id == comment.id)
                {
                    comments.push(comment);
                }
            }
        }
        comments.retain(|comment| comment.resolved_type() == comment_type);
        // Java Comment.xml `selectCommentsByTaskIdAndType`: order by TIME_ desc
        comments.sort_by(|left, right| right.time.cmp(&left.time).then(right.id.cmp(&left.id)));
        comments
    }

    pub fn find_historic_comments_by_process_instance_id(
        &self,
        process_instance_id: &str,
        session: &mut DbSession,
    ) -> Vec<crate::history::historic_entities::HistoricComment> {
        // Java `selectCommentsByProcessInstanceId` does not filter by type
        // (includes event-style comments such as identity-link audit rows).
        let mut comments: Vec<crate::history::historic_entities::HistoricComment> = match session
            .find_by(
                "historic_comments",
                "process_instance_id",
                process_instance_id,
            ) {
            Ok(found) => found,
            Err(error) => {
                session.note_write_error(error);
                Vec::new()
            }
        };
        // Java Comment.xml `selectCommentsByProcessInstanceId`: order by TIME_ desc
        comments.sort_by(|left, right| right.time.cmp(&left.time).then(right.id.cmp(&left.id)));
        comments
    }

    pub fn find_historic_comments_by_process_instance_id_and_type(
        &self,
        process_instance_id: &str,
        comment_type: &str,
        session: &mut DbSession,
    ) -> Vec<crate::history::historic_entities::HistoricComment> {
        let mut comments: Vec<crate::history::historic_entities::HistoricComment> = match session
            .find_by_two(
                "historic_comments",
                "process_instance_id",
                process_instance_id,
                "comment_type",
                comment_type,
            ) {
            Ok(found) => found,
            Err(error) => {
                session.note_write_error(error);
                Vec::new()
            }
        };
        if comment_type == crate::history::historic_entities::HistoricComment::TYPE_COMMENT
            || comment_type == crate::history::historic_entities::HistoricComment::TYPE_EVENT
        {
            let by_pi: Vec<crate::history::historic_entities::HistoricComment> = match session
                .find_by(
                    "historic_comments",
                    "process_instance_id",
                    process_instance_id,
                ) {
                Ok(found) => found,
                Err(error) => {
                    session.note_write_error(error);
                    Vec::new()
                }
            };
            for comment in by_pi {
                if comment.resolved_type() == comment_type
                    && !comments.iter().any(|existing| existing.id == comment.id)
                {
                    comments.push(comment);
                }
            }
        }
        comments.retain(|comment| comment.resolved_type() == comment_type);
        // Java Comment.xml `selectCommentsByProcessInstanceIdAndType`: TIME_ desc
        comments.sort_by(|left, right| right.time.cmp(&left.time).then(right.id.cmp(&left.id)));
        comments
    }

    pub fn find_historic_comments_by_type(
        &self,
        comment_type: &str,
        session: &mut DbSession,
    ) -> Vec<crate::history::historic_entities::HistoricComment> {
        // Index path for projected types.
        let mut comments: Vec<crate::history::historic_entities::HistoricComment> =
            match session.find_by("historic_comments", "comment_type", comment_type) {
                Ok(found) => found,
                Err(error) => {
                    session.note_write_error(error);
                    Vec::new()
                }
            };
        // Legacy JSON rows may lack both the projected column and the field;
        // include those whose resolved_type matches (Java TYPE_COMMENT/EVENT).
        if comment_type == crate::history::historic_entities::HistoricComment::TYPE_COMMENT
            || comment_type == crate::history::historic_entities::HistoricComment::TYPE_EVENT
        {
            let all: Vec<crate::history::historic_entities::HistoricComment> = {
                match session.find_all("historic_comments") {
                    Ok(found) => found,
                    Err(error) => {
                        session.note_write_error(error);
                        Vec::new()
                    }
                }
            };
            for comment in all {
                if comment.resolved_type() == comment_type
                    && !comments.iter().any(|existing| existing.id == comment.id)
                {
                    comments.push(comment);
                }
            }
        }
        comments.retain(|comment| comment.resolved_type() == comment_type);
        // Java Comment.xml `selectCommentsByType`: order by TIME_ desc
        comments.sort_by(|left, right| right.time.cmp(&left.time).then(right.id.cmp(&left.id)));
        comments
    }

    pub fn delete_historic_comment(&self, comment_id: &str, session: &mut DbSession) {
        let _ = session.delete("historic_comments", comment_id);
    }

    pub fn insert_historic_task_event(
        &self,
        event: crate::history::historic_entities::HistoricTaskEvent,
        session: &mut DbSession,
    ) {
        session
            .insert_with_extra(
                "historic_task_events",
                &event.id,
                &event,
                &[
                    ("task_id".into(), Some(event.task_id.clone())),
                    (
                        "time".into(),
                        Some(event.time.timestamp_millis().to_string()),
                    ),
                ],
            )
            .unwrap_or_default();
    }

    pub fn find_historic_task_event(
        &self,
        event_id: &str,
        session: &mut DbSession,
    ) -> Option<crate::history::historic_entities::HistoricTaskEvent> {
        {
            match session.find("historic_task_events", event_id) {
                Ok(found) => found,
                Err(error) => {
                    session.note_write_error(error);
                    None
                }
            }
        }
    }

    pub fn find_historic_task_events_by_task_id(
        &self,
        task_id: &str,
        session: &mut DbSession,
    ) -> Vec<crate::history::historic_entities::HistoricTaskEvent> {
        let mut events: Vec<crate::history::historic_entities::HistoricTaskEvent> =
            match session.find_by("historic_task_events", "task_id", task_id) {
                Ok(found) => found,
                Err(error) => {
                    session.note_write_error(error);
                    Vec::new()
                }
            };
        // Java Comment.xml `selectEventsByTaskId`: order by TIME_ desc
        events.sort_by(|left, right| right.time.cmp(&left.time).then(right.id.cmp(&left.id)));
        events
    }

    pub fn delete_historic_task_event(&self, event_id: &str, session: &mut DbSession) {
        let _ = session.delete("historic_task_events", event_id);
    }

    pub fn next_historic_task_log_number(&self, session: &mut DbSession) -> i64 {
        session
            .max("historic_task_log_entries", "log_number", &[])
            .unwrap_or_default()
            .unwrap_or(0)
            + 1
    }

    pub fn insert_historic_task_log_entry(
        &self,
        entry: crate::history::historic_entities::HistoricTaskLogEntry,
        session: &mut DbSession,
    ) {
        session
            .insert_with_extra(
                "historic_task_log_entries",
                &entry.id,
                &entry,
                &[
                    ("log_number".into(), Some(entry.log_number.to_string())),
                    ("task_id".into(), Some(entry.task_id.clone())),
                    ("log_type".into(), Some(entry.log_type.clone())),
                    (
                        "process_instance_id".into(),
                        entry.process_instance_id.clone(),
                    ),
                    (
                        "process_definition_id".into(),
                        entry.process_definition_id.clone(),
                    ),
                    (
                        "timestamp".into(),
                        Some(entry.timestamp.timestamp_millis().to_string()),
                    ),
                ],
            )
            .unwrap_or_default();
    }

    pub fn list_historic_task_log_entries(
        &self,
        session: &mut DbSession,
    ) -> Vec<crate::history::historic_entities::HistoricTaskLogEntry> {
        let mut entries = {
            match session.find_all::<crate::history::historic_entities::HistoricTaskLogEntry>(
                "historic_task_log_entries",
            ) {
                Ok(found) => found,
                Err(error) => {
                    session.note_write_error(error);
                    Vec::new()
                }
            }
        };
        entries.sort_by_key(|left| left.log_number);
        entries
    }

    // ── HTTP task methods ──

    pub fn insert_http_task_record(&self, record: HttpTaskRecord, session: &mut DbSession) {
        session
            .insert_with_extra(
                "http_task_records",
                &record.id,
                &record,
                &[
                    (
                        "process_instance_id".into(),
                        Some(record.process_instance_id.clone()),
                    ),
                    ("execution_id".into(), Some(record.execution_id.clone())),
                    ("activity_id".into(), Some(record.activity_id.clone())),
                    ("method".into(), Some(record.method.clone())),
                    ("url".into(), Some(record.url.clone())),
                    (
                        "status".into(),
                        Some(
                            serde_json::to_string(&record.status)
                                .unwrap_or_default()
                                .trim_matches('"')
                                .to_string(),
                        ),
                    ),
                    ("created_at".into(), Some(record.created_at.to_string())),
                ],
            )
            .unwrap_or_default();
    }

    pub fn list_http_task_records(&self, session: &mut DbSession) -> Vec<HttpTaskRecord> {
        {
            match session.find_all("http_task_records") {
                Ok(found) => found,
                Err(error) => {
                    session.note_write_error(error);
                    Vec::new()
                }
            }
        }
    }

    pub fn find_http_task_records_by_process_instance_id(
        &self,
        process_instance_id: &str,
        session: &mut DbSession,
    ) -> Vec<HttpTaskRecord> {
        // Java parity: DbSqlSession.selectList throws on SQL error; an empty list means a no-row query, not a storage failure.
        match session.find_by(
            "http_task_records",
            "process_instance_id",
            process_instance_id,
        ) {
            Ok(found) => found,
            Err(error) => {
                session.note_write_error(error);
                Vec::new()
            }
        }
    }

    // ── Mail outbox methods ──

    pub fn insert_mail_outbox_record(&self, record: MailOutboxRecord, session: &mut DbSession) {
        session
            .insert_with_extra(
                "mail_outbox_records",
                &record.id,
                &record,
                &[
                    (
                        "process_instance_id".into(),
                        Some(record.process_instance_id.clone()),
                    ),
                    ("execution_id".into(), Some(record.execution_id.clone())),
                    ("activity_id".into(), Some(record.activity_id.clone())),
                    ("recipient".into(), Some(record.recipient.clone())),
                    ("subject".into(), Some(record.subject.clone())),
                    (
                        "status".into(),
                        Some(
                            serde_json::to_string(&record.status)
                                .unwrap_or_default()
                                .trim_matches('"')
                                .to_string(),
                        ),
                    ),
                    ("created_at".into(), Some(record.created_at.to_string())),
                ],
            )
            .unwrap_or_default();
    }

    pub fn list_mail_outbox_records(&self, session: &mut DbSession) -> Vec<MailOutboxRecord> {
        {
            match session.find_all("mail_outbox_records") {
                Ok(found) => found,
                Err(error) => {
                    session.note_write_error(error);
                    Vec::new()
                }
            }
        }
    }

    pub fn find_mail_outbox_records_by_process_instance_id(
        &self,
        process_instance_id: &str,
        session: &mut DbSession,
    ) -> Vec<MailOutboxRecord> {
        // Java parity: DbSqlSession.selectList throws on SQL error; an empty list means a no-row query, not a storage failure.
        match session.find_by(
            "mail_outbox_records",
            "process_instance_id",
            process_instance_id,
        ) {
            Ok(found) => found,
            Err(error) => {
                session.note_write_error(error);
                Vec::new()
            }
        }
    }

    // ── Task methods ──

    pub fn insert_task(
        &self,
        task: &crate::task::Task,
        session: &mut DbSession,
    ) -> Result<(), StorageError> {
        let mut task = task.clone();
        // Task 6: single batched resolve replaces 4 separate XML parses.
        // Only resolve when at least one property is missing.
        let needs_resolve = task.assignee.is_none()
            || task.owner.is_none()
            || task.priority.is_none()
            || task.due_date.is_none()
            || task.category.is_none()
            || task.form_key.is_none();
        if needs_resolve {
            let props = self.resolve_user_task_properties(
                &task.process_instance_id,
                &task.execution_id,
                &task.task_definition_key,
                session,
            )?;
            if task.assignee.is_none() {
                task.assignee = props.assignee;
            }
            if task.owner.is_none() {
                task.owner = props.owner;
            }
            if task.priority.is_none() {
                task.priority = props.priority;
            }
            if task.due_date.is_none() {
                task.due_date = props.due_date;
            }
            if task.category.is_none() {
                task.category = props.category;
            }
            if task.form_key.is_none() {
                task.form_key = props.form_key;
            }
        }
        session.insert_with_extra(
            "tasks",
            &task.id,
            &task,
            &[
                (
                    "process_instance_id".into(),
                    Some(task.process_instance_id.clone()),
                ),
                ("execution_id".into(), Some(task.execution_id.clone())),
                (
                    "task_definition_key".into(),
                    Some(task.task_definition_key.clone()),
                ),
                ("name".into(), Some(task.name.clone())),
                ("assignee".into(), task.assignee.clone()),
                ("owner".into(), task.owner.clone()),
                ("parent_task_id".into(), task.parent_task_id.clone()),
                ("priority".into(), task.priority.map(|v| v.to_string())),
                (
                    "due_date".into(),
                    task.due_date
                        .map(|due_date| due_date.timestamp_millis().to_string()),
                ),
            ],
        )?;
        // P97: no silent historic sync here. History writes belong to the
        // HistoryManager (gating + async buffer + identity-link diff); syncing
        // the historic row in the store consumed the IL diff in
        // record_task_updated and bypassed history_disabled/async_history.
        Ok(())
    }

    pub fn update_task(
        &self,
        task: &crate::task::Task,
        session: &mut DbSession,
    ) -> Result<(), crate::persistence::StorageError> {
        self.insert_task(task, session)?;
        Ok(())
    }

    pub fn delete_task(&self, id: &str, session: &mut DbSession) {
        let _ = session.delete("tasks", id);
    }

    /// A successful query may have no row; a failed query must stop its caller.
    /// Java DbSqlSession.selectOne propagates the persistence exception here.
    pub fn find_task(
        &self,
        id: &str,
        session: &mut DbSession,
    ) -> Result<Option<crate::task::Task>, StorageError> {
        session.find("tasks", id)
    }

    pub fn find_tasks_by_process_instance_id(
        &self,
        process_instance_id: &str,
        session: &mut DbSession,
    ) -> Vec<crate::task::Task> {
        // Java parity: DbSqlSession.selectList throws on SQL error; an empty list means a no-row query, not a storage failure.
        match session.find_by("tasks", "process_instance_id", process_instance_id) {
            Ok(found) => found,
            Err(error) => {
                session.note_write_error(error);
                Vec::new()
            }
        }
    }

    pub fn find_task_by_execution_id(
        &self,
        execution_id: &str,
        session: &mut DbSession,
    ) -> Option<crate::task::Task> {
        // Java parity: DbSqlSession.selectList throws on SQL error; an empty list means a no-row query, not a storage failure.
        (match session.find_by::<crate::task::Task>("tasks", "execution_id", execution_id) {
            Ok(found) => found,
            Err(error) => {
                session.note_write_error(error);
                Vec::new()
            }
        })
        .into_iter()
        .next()
    }

    pub fn find_tasks_by_parent_task_id(
        &self,
        parent_task_id: &str,
        session: &mut DbSession,
    ) -> Vec<crate::task::Task> {
        // Java parity: DbSqlSession.selectList throws on SQL error; an empty list means a no-row query, not a storage failure.
        match session.find_by("tasks", "parent_task_id", parent_task_id) {
            Ok(found) => found,
            Err(error) => {
                session.note_write_error(error);
                Vec::new()
            }
        }
    }

    pub fn snapshot_tasks(&self, session: &mut DbSession) -> HashMap<String, crate::task::Task> {
        {
            match session.find_all::<crate::task::Task>("tasks") {
                Ok(found) => found,
                Err(error) => {
                    session.note_write_error(error);
                    Vec::new()
                }
            }
        }
        .into_iter()
        .map(|t| (t.id.clone(), t))
        .collect()
    }

    pub fn resolve_user_task_assignee(
        &self,
        process_instance_id: &str,
        execution_id: &str,
        task_definition_key: &str,
        session: &mut DbSession,
    ) -> Result<Option<String>, StorageError> {
        self.resolve_user_task_property(
            process_instance_id,
            execution_id,
            task_definition_key,
            |user_task| user_task.assignee.clone(),
            session,
        )
    }

    pub fn resolve_user_task_owner(
        &self,
        process_instance_id: &str,
        execution_id: &str,
        task_definition_key: &str,
        session: &mut DbSession,
    ) -> Result<Option<String>, StorageError> {
        self.resolve_user_task_property(
            process_instance_id,
            execution_id,
            task_definition_key,
            |user_task| user_task.owner.clone(),
            session,
        )
    }

    pub fn resolve_user_task_priority(
        &self,
        process_instance_id: &str,
        execution_id: &str,
        task_definition_key: &str,
        session: &mut DbSession,
    ) -> Result<Option<i32>, StorageError> {
        self.resolve_user_task_property(
            process_instance_id,
            execution_id,
            task_definition_key,
            |user_task| {
                user_task
                    .priority
                    .as_deref()
                    .and_then(|priority| priority.trim().parse::<i32>().ok())
            },
            session,
        )
    }

    pub fn resolve_user_task_due_date(
        &self,
        process_instance_id: &str,
        execution_id: &str,
        task_definition_key: &str,
        session: &mut DbSession,
    ) -> Result<Option<DateTime<Utc>>, StorageError> {
        let now = self.time_source.now();
        self.resolve_user_task_property(
            process_instance_id,
            execution_id,
            task_definition_key,
            |user_task| {
                user_task
                    .due_date
                    .as_deref()
                    .and_then(|value| parse_task_due_date(value, now))
            },
            session,
        )
    }

    /// Resolve all four user-task properties (assignee, owner, priority, due_date) in a
    /// single pass (Task 6). Performs ONE find_execution + ONE find_process_instance +
    /// ONE find_by_id process_definition + ONE deployment_resource_bytes + ONE XML parse
    /// (cached via BpmnModelCache when available). Replaces 4 calls to
    /// resolve_user_task_property which previously did 12-16 DB queries + 4 XML parses.
    pub fn resolve_user_task_properties(
        &self,
        process_instance_id: &str,
        execution_id: &str,
        task_definition_key: &str,
        session: &mut DbSession,
    ) -> Result<UserTaskProperties, StorageError> {
        let execution = self.find_execution(execution_id, session);
        let process_definition_id = match execution
            .as_ref()
            .and_then(|execution| execution.process_definition_id.clone())
        {
            Some(id) => Some(id),
            None => self
                .find_process_instance(process_instance_id, session)?
                .map(|instance| instance.process_definition_id),
        };
        let Some(process_definition_id) = process_definition_id else {
            return Ok(UserTaskProperties::default());
        };
        let activity_id = execution
            .as_ref()
            .and_then(|execution| execution.activity_id.clone())
            .unwrap_or_else(|| task_definition_key.to_string());
        let Some(process_definition) =
            session.find::<ProcessDefinition>("process_definitions", &process_definition_id)?
        else {
            return Ok(UserTaskProperties::default());
        };
        let deployment_id = match process_definition.deployment_id {
            Some(id) => id,
            None => return Ok(UserTaskProperties::default()),
        };
        let resource_name = match process_definition.resource_name {
            Some(name) => name,
            None => return Ok(UserTaskProperties::default()),
        };
        let bytes = match self.deployment_resource_bytes(&deployment_id, &resource_name, session) {
            Some(b) => b,
            None => return Ok(UserTaskProperties::default()),
        };

        // Prefer cache; fall back to direct parse for backward compatibility.
        // Java parity: the bytes were validated as UTF-8 BPMN XML at deployment time,
        // so once the resource row exists any decoding/parse failure means the stored
        // resource is corrupt and must surface as a 500 instead of default properties.
        let model: Arc<flowable_bpmn_model::model::BpmnModel> =
            if let Some(cache) = &self.bpmn_model_cache {
                // A cache hit always returns Some; None here can only mean the stored bytes
                // failed UTF-8 validation or BPMN conversion (i.e. corrupt resource).
                match cache.get_or_parse(&deployment_id, &resource_name, &bytes) {
                    Some(model) => model,
                    None => {
                        return Err(Self::corrupt_deployment_resource_error(
                            &deployment_id,
                            &resource_name,
                            "resource bytes are not valid UTF-8 or BPMN XML conversion failed",
                        ));
                    }
                }
            } else {
                let xml = match std::str::from_utf8(&bytes) {
                    Ok(xml) => xml,
                    Err(error) => {
                        return Err(Self::corrupt_deployment_resource_error(
                            &deployment_id,
                            &resource_name,
                            &format!("resource bytes are not valid UTF-8: {error}"),
                        ));
                    }
                };
                match BpmnXMLConverter::new().try_convert_to_bpmn_model(xml) {
                    Ok(model) => Arc::new(model),
                    Err(error) => {
                        return Err(Self::corrupt_deployment_resource_error(
                            &deployment_id,
                            &resource_name,
                            &format!("BPMN XML conversion failed: {error}"),
                        ));
                    }
                }
            };

        // Single pass over flow_elements to extract all properties.
        for process in &model.processes {
            for element in process.flow_elements.iter() {
                if let FlowElementEnum::UserTask(user_task) = element {
                    let id_matches = user_task
                        .task
                        .activity
                        .flow_node
                        .flow_element
                        .base_element
                        .id
                        .as_deref()
                        == Some(activity_id.as_str());
                    if id_matches {
                        let due_date = execution.as_ref().and_then(|execution| {
                            let mut evaluation_execution = execution.clone();
                            let variables =
                                crate::engine::variable_service::collect_execution_variables(
                                    self,
                                    session,
                                    &execution.id,
                                );
                            for (name, value) in variables {
                                evaluation_execution.variables.entry(name).or_insert(value);
                            }
                            evaluate_user_task_due_date(
                                user_task.due_date.as_deref(),
                                &evaluation_execution,
                                self.time_source.now(),
                            )
                            .ok()
                            .flatten()
                        });
                        return Ok(UserTaskProperties {
                            assignee: user_task.assignee.clone(),
                            owner: user_task.owner.clone(),
                            priority: user_task
                                .priority
                                .as_deref()
                                .and_then(|p| p.trim().parse::<i32>().ok()),
                            due_date,
                            category: user_task.category.clone(),
                            form_key: user_task.form_key.clone(),
                        });
                    }
                }
            }
        }
        Ok(UserTaskProperties::default())
    }

    fn resolve_user_task_property<T>(
        &self,
        process_instance_id: &str,
        execution_id: &str,
        task_definition_key: &str,
        property: impl Fn(&flowable_bpmn_model::model::UserTask) -> Option<T>,
        session: &mut DbSession,
    ) -> Result<Option<T>, StorageError> {
        let execution = self.find_execution(execution_id, session);
        let process_definition_id = match execution
            .as_ref()
            .and_then(|execution| execution.process_definition_id.clone())
        {
            Some(id) => Some(id),
            None => self
                .find_process_instance(process_instance_id, session)?
                .map(|instance| instance.process_definition_id),
        };
        let Some(process_definition_id) = process_definition_id else {
            return Ok(None);
        };
        let activity_id = execution
            .and_then(|execution| execution.activity_id)
            .unwrap_or_else(|| task_definition_key.to_string());
        let Some(process_definition) =
            session.find::<ProcessDefinition>("process_definitions", &process_definition_id)?
        else {
            return Ok(None);
        };
        let Some(deployment_id) = process_definition.deployment_id else {
            return Ok(None);
        };
        let Some(resource_name) = process_definition.resource_name else {
            return Ok(None);
        };
        // Java parity: a no-row blob SELECT legitimately yields None (absent resource,
        // sticky SQL errors are already recorded by `deployment_resource_bytes`).
        let Some(bytes) = self.deployment_resource_bytes(&deployment_id, &resource_name, session)
        else {
            return Ok(None);
        };
        // The bytes were validated as UTF-8 BPMN XML at deployment time, so once the row
        // exists a decoding/parse failure can only mean the stored resource is corrupt.
        // Java re-parsing such a resource throws FlowableException (HTTP 500); it must
        // never silently fall back to default task properties.
        let xml = match std::str::from_utf8(&bytes) {
            Ok(xml) => xml,
            Err(error) => {
                return Err(Self::corrupt_deployment_resource_error(
                    &deployment_id,
                    &resource_name,
                    &format!("resource bytes are not valid UTF-8: {error}"),
                ));
            }
        };
        let model = match BpmnXMLConverter::new().try_convert_to_bpmn_model(xml) {
            Ok(model) => model,
            Err(error) => {
                return Err(Self::corrupt_deployment_resource_error(
                    &deployment_id,
                    &resource_name,
                    &format!("BPMN XML conversion failed: {error}"),
                ));
            }
        };

        Ok(model
            .processes
            .iter()
            .flat_map(|process| process.flow_elements.iter())
            .find_map(|element| match element {
                FlowElementEnum::UserTask(user_task)
                    if user_task
                        .task
                        .activity
                        .flow_node
                        .flow_element
                        .base_element
                        .id
                        .as_deref()
                        == Some(activity_id.as_str()) =>
                {
                    property(user_task)
                }
                _ => None,
            }))
    }

    fn deployment_resource_bytes(
        &self,
        deployment_id: &str,
        name: &str,
        session: &mut DbSession,
    ) -> Option<Vec<u8>> {
        // Java parity: a blob SELECT throws on SQL error; None means a no-row query, not a storage failure.
        match session.find_blob_by_two(
            "deployment_resources",
            "deployment_id",
            deployment_id,
            "name",
            name,
            "bytes",
        ) {
            Ok(found) => found,
            Err(error) => {
                session.note_write_error(error);
                None
            }
        }
    }

    /// Build the error for a deployment resource row whose bytes cannot be decoded as
    /// UTF-8 BPMN XML. Such bytes were validated at deployment time, so this can only be
    /// a corrupt/stored-as-garbage resource; Java surfaces this as a FlowableException
    /// (HTTP 500) rather than silently falling back to default task properties.
    fn corrupt_deployment_resource_error(
        deployment_id: &str,
        resource_name: &str,
        reason: &str,
    ) -> StorageError {
        StorageError::Deserialization(format!(
            "corrupt deployment resource '{resource_name}' of deployment '{deployment_id}': {reason}"
        ))
    }

    // ── Identity-Link methods ──

    pub fn insert_identity_link(
        &self,
        link: crate::identity::entities::IdentityLink,
        session: &mut DbSession,
    ) {
        session
            .insert_with_extra(
                "identity_links",
                &link.id,
                &link,
                &[
                    ("link_type".into(), Some(link.link_type.clone())),
                    ("task_id".into(), link.task_id.clone()),
                    (
                        "process_instance_id".into(),
                        link.process_instance_id.clone(),
                    ),
                    (
                        "process_definition_id".into(),
                        link.process_definition_id.clone(),
                    ),
                    ("user_id".into(), link.user_id.clone()),
                    ("group_id".into(), link.group_id.clone()),
                ],
            )
            .unwrap_or_default();
    }

    pub fn delete_identity_link(&self, link_id: &str, session: &mut DbSession) {
        let _ = session.delete("identity_links", link_id);
    }

    pub fn find_identity_link(
        &self,
        link_id: &str,
        session: &mut DbSession,
    ) -> Option<crate::identity::entities::IdentityLink> {
        {
            match session.find("identity_links", link_id) {
                Ok(found) => found,
                Err(error) => {
                    session.note_write_error(error);
                    None
                }
            }
        }
    }

    pub fn find_identity_links_by_task(
        &self,
        task_id: &str,
        session: &mut DbSession,
    ) -> Vec<crate::identity::entities::IdentityLink> {
        match session.find_by("identity_links", "task_id", task_id) {
            Ok(found) => found,
            Err(error) => {
                session.note_write_error(error);
                Vec::new()
            }
        }
    }

    /// Batch-query identity links for multiple task IDs in a single SQL call.
    /// Avoids N+1 queries when filtering tasks by candidate user/group.
    pub fn find_identity_links_by_tasks(
        &self,
        task_ids: &[String],
        session: &mut DbSession,
    ) -> Vec<crate::identity::entities::IdentityLink> {
        if task_ids.is_empty() {
            return Vec::new();
        }
        let filters: Vec<(String, FilterOp)> =
            vec![("task_id".to_string(), FilterOp::In(task_ids.to_vec()))];
        match session.find_with_filters("identity_links", &filters, None, None) {
            Ok(found) => found,
            Err(error) => {
                session.note_write_error(error);
                Vec::new()
            }
        }
    }

    pub fn find_identity_links_by_process_instance(
        &self,
        process_instance_id: &str,
        session: &mut DbSession,
    ) -> Vec<crate::identity::entities::IdentityLink> {
        match session.find_by("identity_links", "process_instance_id", process_instance_id) {
            Ok(found) => found,
            Err(error) => {
                session.note_write_error(error);
                Vec::new()
            }
        }
    }

    pub fn find_identity_links_by_process_definition(
        &self,
        process_definition_id: &str,
        session: &mut DbSession,
    ) -> Vec<crate::identity::entities::IdentityLink> {
        match session.find_by(
            "identity_links",
            "process_definition_id",
            process_definition_id,
        ) {
            Ok(found) => found,
            Err(error) => {
                session.note_write_error(error);
                Vec::new()
            }
        }
    }

    pub fn find_identity_links_by_user(
        &self,
        user_id: &str,
        session: &mut DbSession,
    ) -> Vec<crate::identity::entities::IdentityLink> {
        match session.find_by("identity_links", "user_id", user_id) {
            Ok(found) => found,
            Err(error) => {
                session.note_write_error(error);
                Vec::new()
            }
        }
    }

    /// Returns distinct process-instance ids linked to `user_id`, regardless
    /// of identity-link type. The indexed user predicate is evaluated by the
    /// store; task-only links are excluded because they have no process id.
    pub fn find_process_instance_ids_by_involved_user(
        &self,
        user_id: &str,
        session: &mut DbSession,
    ) -> Vec<String> {
        let mut process_instance_ids: Vec<String> = self
            .find_identity_links_by_user(user_id, session)
            .into_iter()
            .filter_map(|link| link.process_instance_id)
            .collect();
        process_instance_ids.sort();
        process_instance_ids.dedup();
        process_instance_ids
    }

    pub fn find_identity_links_by_group(
        &self,
        group_id: &str,
        session: &mut DbSession,
    ) -> Vec<crate::identity::entities::IdentityLink> {
        match session.find_by("identity_links", "group_id", group_id) {
            Ok(found) => found,
            Err(error) => {
                session.note_write_error(error);
                Vec::new()
            }
        }
    }

    pub fn find_identity_links_by_type(
        &self,
        link_type: &str,
        session: &mut DbSession,
    ) -> Vec<crate::identity::entities::IdentityLink> {
        match session.find_by("identity_links", "link_type", link_type) {
            Ok(found) => found,
            Err(error) => {
                session.note_write_error(error);
                Vec::new()
            }
        }
    }

    pub fn list_identity_links(
        &self,
        session: &mut DbSession,
    ) -> Vec<crate::identity::entities::IdentityLink> {
        {
            match session.find_all("identity_links") {
                Ok(found) => found,
                Err(error) => {
                    session.note_write_error(error);
                    Vec::new()
                }
            }
        }
    }

    // ── Historic identity-link methods (P77 / ACT_HI_IDENTITYLINK) ──
    // Java: HistoricIdentityLinkServiceImpl + DefaultHistoryManager:391-417.

    pub fn insert_historic_identity_link(
        &self,
        link: &crate::history::historic_entities::HistoricIdentityLink,
        session: &mut DbSession,
    ) {
        let create_time_ms = link.create_time.map(|t| t.timestamp_millis());
        session
            .insert_with_extra(
                "historic_identity_links",
                &link.id,
                link,
                &[
                    ("link_type".into(), Some(link.link_type.clone())),
                    ("task_id".into(), link.task_id.clone()),
                    (
                        "process_instance_id".into(),
                        link.process_instance_id.clone(),
                    ),
                    ("user_id".into(), link.user_id.clone()),
                    ("group_id".into(), link.group_id.clone()),
                    ("scope_id".into(), link.scope_id.clone()),
                    ("sub_scope_id".into(), link.sub_scope_id.clone()),
                    ("scope_type".into(), link.scope_type.clone()),
                    (
                        "scope_definition_id".into(),
                        link.scope_definition_id.clone(),
                    ),
                    ("create_time".into(), create_time_ms.map(|v| v.to_string())),
                ],
            )
            .unwrap_or_default();
    }

    pub fn delete_historic_identity_link(&self, link_id: &str, session: &mut DbSession) {
        let _ = session.delete("historic_identity_links", link_id);
    }

    pub fn find_historic_identity_link(
        &self,
        link_id: &str,
        session: &mut DbSession,
    ) -> Option<crate::history::historic_entities::HistoricIdentityLink> {
        match session.find("historic_identity_links", link_id) {
            Ok(found) => found,
            Err(error) => {
                session.note_write_error(error);
                None
            }
        }
    }

    pub fn find_historic_identity_links_by_task(
        &self,
        task_id: &str,
        session: &mut DbSession,
    ) -> Vec<crate::history::historic_entities::HistoricIdentityLink> {
        match session.find_by("historic_identity_links", "task_id", task_id) {
            Ok(found) => found,
            Err(error) => {
                session.note_write_error(error);
                Vec::new()
            }
        }
    }

    pub fn find_historic_identity_links_by_process_instance(
        &self,
        process_instance_id: &str,
        session: &mut DbSession,
    ) -> Vec<crate::history::historic_entities::HistoricIdentityLink> {
        match session.find_by(
            "historic_identity_links",
            "process_instance_id",
            process_instance_id,
        ) {
            Ok(found) => found,
            Err(error) => {
                session.note_write_error(error);
                Vec::new()
            }
        }
    }

    pub fn find_historic_identity_links_by_user(
        &self,
        user_id: &str,
        session: &mut DbSession,
    ) -> Vec<crate::history::historic_entities::HistoricIdentityLink> {
        match session.find_by("historic_identity_links", "user_id", user_id) {
            Ok(found) => found,
            Err(error) => {
                session.note_write_error(error);
                Vec::new()
            }
        }
    }

    pub fn find_historic_identity_links_by_scope(
        &self,
        scope_id: &str,
        scope_type: Option<&str>,
        session: &mut DbSession,
    ) -> Vec<crate::history::historic_entities::HistoricIdentityLink> {
        let mut links: Vec<crate::history::historic_entities::HistoricIdentityLink> =
            match session.find_by("historic_identity_links", "scope_id", scope_id) {
                Ok(found) => found,
                Err(error) => {
                    session.note_write_error(error);
                    Vec::new()
                }
            };
        if let Some(scope_type) = scope_type {
            links.retain(|link| link.scope_type.as_deref() == Some(scope_type));
        }
        links
    }

    /// Process-instance ids that have a historic identity link for `user_id`.
    /// Java `HistoricProcessInstance.xml:903-904` uses ACT_HI_IDENTITYLINK.
    pub fn find_process_instance_ids_by_historic_involved_user(
        &self,
        user_id: &str,
        session: &mut DbSession,
    ) -> Vec<String> {
        let mut process_instance_ids: Vec<String> = self
            .find_historic_identity_links_by_user(user_id, session)
            .into_iter()
            .filter_map(|link| link.process_instance_id)
            .collect();
        process_instance_ids.sort();
        process_instance_ids.dedup();
        process_instance_ids
    }

    // ── Entity-Link methods ──

    pub fn insert_entity_link(
        &self,
        link: crate::identity::entities::EntityLink,
        session: &mut DbSession,
    ) {
        session
            .insert_with_extra(
                "entity_links",
                &link.id,
                &link,
                &[
                    ("link_type".into(), Some(link.link_type.clone())),
                    ("scope_id".into(), link.scope_id.clone()),
                    ("scope_type".into(), link.scope_type.clone()),
                    ("reference_scope_id".into(), link.reference_scope_id.clone()),
                    (
                        "reference_scope_type".into(),
                        link.reference_scope_type.clone(),
                    ),
                ],
            )
            .unwrap_or_default();
    }

    pub fn delete_entity_link(&self, link_id: &str, session: &mut DbSession) {
        let _ = session.delete("entity_links", link_id);
    }

    pub fn find_entity_links_by_scope(
        &self,
        scope_id: &str,
        session: &mut DbSession,
    ) -> Vec<crate::identity::entities::EntityLink> {
        // Java parity: DbSqlSession.selectList throws on SQL error; an empty list means a no-row query, not a storage failure.
        match session.find_by("entity_links", "scope_id", scope_id) {
            Ok(found) => found,
            Err(error) => {
                session.note_write_error(error);
                Vec::new()
            }
        }
    }

    pub fn find_entity_links_by_reference_scope(
        &self,
        reference_scope_id: &str,
        session: &mut DbSession,
    ) -> Vec<crate::identity::entities::EntityLink> {
        // Java parity: DbSqlSession.selectList throws on SQL error; an empty list means a no-row query, not a storage failure.
        match session.find_by("entity_links", "reference_scope_id", reference_scope_id) {
            Ok(found) => found,
            Err(error) => {
                session.note_write_error(error);
                Vec::new()
            }
        }
    }

    pub fn find_entity_links_by_link_type(
        &self,
        link_type: &str,
        session: &mut DbSession,
    ) -> Vec<crate::identity::entities::EntityLink> {
        // Java parity: DbSqlSession.selectList throws on SQL error; an empty list means a no-row query, not a storage failure.
        match session.find_by("entity_links", "link_type", link_type) {
            Ok(found) => found,
            Err(error) => {
                session.note_write_error(error);
                Vec::new()
            }
        }
    }

    pub fn list_entity_links(
        &self,
        session: &mut DbSession,
    ) -> Vec<crate::identity::entities::EntityLink> {
        {
            match session.find_all("entity_links") {
                Ok(found) => found,
                Err(error) => {
                    session.note_write_error(error);
                    Vec::new()
                }
            }
        }
    }

    // ── Batch methods ──

    pub fn insert_batch(
        &self,
        batch: crate::identity::entities::BatchEntity,
        session: &mut DbSession,
    ) {
        session
            .insert_with_extra(
                "batch_entities",
                &batch.id,
                &batch,
                &[
                    ("batch_type".into(), Some(batch.batch_type.clone())),
                    ("status".into(), Some(batch.status.clone())),
                    (
                        "create_time".into(),
                        Some((batch.create_time as i64).to_string()),
                    ),
                    ("tenant_id".into(), batch.tenant_id.clone()),
                ],
            )
            .unwrap_or_default();
    }

    pub fn list_engine_properties(
        &self,
        session: &mut DbSession,
    ) -> Result<Vec<EngineProperty>, StorageError> {
        Ok(session
            .list_engine_properties()?
            .into_iter()
            .map(|row| EngineProperty {
                name: row.name,
                value: row.value,
                revision: row.revision,
            })
            .collect())
    }

    pub fn find_engine_property(
        &self,
        name: &str,
        session: &mut DbSession,
    ) -> Result<Option<EngineProperty>, StorageError> {
        Ok(session
            .find_engine_property(name)?
            .map(|row| EngineProperty {
                name: row.name,
                value: row.value,
                revision: row.revision,
            }))
    }

    pub fn create_engine_property(
        &self,
        name: &str,
        value: &str,
        session: &mut DbSession,
    ) -> Result<(), StorageError> {
        session.create_engine_property(name, value)
    }

    pub fn update_engine_property(
        &self,
        name: &str,
        value: &str,
        session: &mut DbSession,
    ) -> Result<bool, StorageError> {
        session.update_engine_property(name, value)
    }

    pub fn update_engine_property_if_revision(
        &self,
        name: &str,
        value: &str,
        expected_rev: i32,
        session: &mut DbSession,
    ) -> Result<bool, StorageError> {
        session.update_engine_property_if_revision(name, value, expected_rev)
    }

    pub fn delete_engine_property(
        &self,
        name: &str,
        session: &mut DbSession,
    ) -> Result<bool, StorageError> {
        session.delete_engine_property(name)
    }

    // ── Global property-based lease locks (ACT_GE_PROPERTY) ──

    /// Encode a held lock value: `owner|acquired_at_ms|expiry_ms`.
    pub fn encode_property_lock_value(owner: &str, acquired_at_ms: i64, expiry_ms: i64) -> String {
        format!("{owner}|{acquired_at_ms}|{expiry_ms}")
    }

    /// Parse a lock property value. Empty/null means free.
    pub fn parse_property_lock_value(value: &str) -> Option<(String, i64, i64)> {
        match Self::parse_property_lock_state(value) {
            PersistedPropertyLockState::Held {
                owner,
                acquired_at_ms,
                expiry_ms,
            } => Some((owner, acquired_at_ms, expiry_ms)),
            PersistedPropertyLockState::Free | PersistedPropertyLockState::Corrupt => None,
        }
    }

    pub(crate) fn parse_property_lock_state(value: &str) -> PersistedPropertyLockState {
        let value = value.trim();
        if value.is_empty() {
            return PersistedPropertyLockState::Free;
        }
        let mut parts = value.splitn(3, '|');
        let Some(owner) = parts.next().filter(|owner| !owner.is_empty()) else {
            return PersistedPropertyLockState::Corrupt;
        };
        let Some(acquired_at_ms) = parts.next().and_then(|value| value.parse::<i64>().ok()) else {
            return PersistedPropertyLockState::Corrupt;
        };
        let Some(expiry_ms) = parts.next().and_then(|value| value.parse::<i64>().ok()) else {
            return PersistedPropertyLockState::Corrupt;
        };
        if owner.is_empty() {
            return PersistedPropertyLockState::Corrupt;
        }
        PersistedPropertyLockState::Held {
            owner: owner.to_string(),
            acquired_at_ms,
            expiry_ms,
        }
    }

    /// Try to acquire a DB lease lock stored in `ACT_GE_PROPERTY`.
    ///
    /// - Free or missing row -> claim with `owner` / `lease_ms`
    /// - Held by `owner` -> renew lease
    /// - Held by another and expired (`now_ms >= expiry`) -> force reclaim
    /// - Held by another and still valid -> returns `Ok(false)`
    pub fn try_acquire_property_lock(
        &self,
        lock_name: &str,
        owner: &str,
        now_ms: i64,
        lease_ms: i64,
        session: &mut DbSession,
    ) -> Result<bool, StorageError> {
        let expiry_ms = now_ms.saturating_add(lease_ms);
        let new_value = Self::encode_property_lock_value(owner, now_ms, expiry_ms);

        match self.find_engine_property(lock_name, session)? {
            None => match self.create_engine_property(lock_name, &new_value, session) {
                Ok(()) => Ok(true),
                Err(StorageError::DuplicateEntity { .. }) => Ok(false),
                Err(error) => Err(error),
            },
            Some(prop) => {
                let held = Self::parse_property_lock_state(&prop.value);
                match held {
                    PersistedPropertyLockState::Free => Ok(self
                        .update_engine_property_if_revision(
                            lock_name,
                            &new_value,
                            prop.revision,
                            session,
                        )?),
                    PersistedPropertyLockState::Held {
                        owner: current_owner,
                        acquired_at_ms: _,
                        expiry_ms: current_expiry,
                    } => {
                        if current_owner == owner {
                            Ok(self.update_engine_property_if_revision(
                                lock_name,
                                &new_value,
                                prop.revision,
                                session,
                            )?)
                        } else if now_ms >= current_expiry {
                            Ok(self.update_engine_property_if_revision(
                                lock_name,
                                &new_value,
                                prop.revision,
                                session,
                            )?)
                        } else {
                            Ok(false)
                        }
                    }
                    PersistedPropertyLockState::Corrupt => {
                        Err(StorageError::CorruptGlobalLockValue {
                            lock_name: lock_name.to_string(),
                            value: prop.value,
                        })
                    }
                }
            }
        }
    }

    /// Release a property lock if currently held by `owner`.
    pub fn release_property_lock(
        &self,
        lock_name: &str,
        owner: &str,
        session: &mut DbSession,
    ) -> Result<bool, StorageError> {
        let Some(prop) = self.find_engine_property(lock_name, session)? else {
            return Ok(false);
        };
        match Self::parse_property_lock_value(&prop.value) {
            Some((current_owner, _, _)) if current_owner == owner => Ok(
                self.update_engine_property_if_revision(lock_name, "", prop.revision, session)?
            ),
            _ => Ok(false),
        }
    }

    pub fn find_batch(
        &self,
        batch_id: &str,
        session: &mut DbSession,
    ) -> Option<crate::identity::entities::BatchEntity> {
        {
            match session.find("batch_entities", batch_id) {
                Ok(found) => found,
                Err(error) => {
                    session.note_write_error(error);
                    None
                }
            }
        }
    }

    pub fn delete_batch(&self, batch_id: &str, session: &mut DbSession) {
        let _ = session.delete("batch_entities", batch_id);
        session
            .delete_by("batch_part_entities", "batch_id", batch_id)
            .unwrap_or_default();
    }

    pub fn list_batches(
        &self,
        session: &mut DbSession,
    ) -> Vec<crate::identity::entities::BatchEntity> {
        {
            match session.find_all("batch_entities") {
                Ok(found) => found,
                Err(error) => {
                    session.note_write_error(error);
                    Vec::new()
                }
            }
        }
    }

    pub fn find_batches_by_status(
        &self,
        status: &str,
        session: &mut DbSession,
    ) -> Vec<crate::identity::entities::BatchEntity> {
        // Java parity: DbSqlSession.selectList throws on SQL error; an empty list means a no-row query, not a storage failure.
        match session.find_by("batch_entities", "status", status) {
            Ok(found) => found,
            Err(error) => {
                session.note_write_error(error);
                Vec::new()
            }
        }
    }

    pub fn find_batches_by_type(
        &self,
        batch_type: &str,
        session: &mut DbSession,
    ) -> Vec<crate::identity::entities::BatchEntity> {
        // Java parity: DbSqlSession.selectList throws on SQL error; an empty list means a no-row query, not a storage failure.
        match session.find_by("batch_entities", "batch_type", batch_type) {
            Ok(found) => found,
            Err(error) => {
                session.note_write_error(error);
                Vec::new()
            }
        }
    }

    pub fn find_batches_by_tenant_id(
        &self,
        tenant_id: &str,
        session: &mut DbSession,
    ) -> Vec<crate::identity::entities::BatchEntity> {
        // Java parity: DbSqlSession.selectList throws on SQL error; an empty list means a no-row query, not a storage failure.
        match session.find_by("batch_entities", "tenant_id", tenant_id) {
            Ok(found) => found,
            Err(error) => {
                session.note_write_error(error);
                Vec::new()
            }
        }
    }

    pub fn insert_batch_part(
        &self,
        batch_part: crate::identity::entities::BatchPartEntity,
        session: &mut DbSession,
    ) {
        session
            .insert_with_extra(
                "batch_part_entities",
                &batch_part.id,
                &batch_part,
                &[
                    ("batch_id".into(), Some(batch_part.batch_id.clone())),
                    ("batch_type".into(), Some(batch_part.batch_type.clone())),
                    ("status".into(), Some(batch_part.status.clone())),
                    (
                        "create_time".into(),
                        Some((batch_part.create_time as i64).to_string()),
                    ),
                ],
            )
            .unwrap_or_default();
    }

    pub fn find_batch_part(
        &self,
        batch_part_id: &str,
        session: &mut DbSession,
    ) -> Option<crate::identity::entities::BatchPartEntity> {
        {
            match session.find("batch_part_entities", batch_part_id) {
                Ok(found) => found,
                Err(error) => {
                    session.note_write_error(error);
                    None
                }
            }
        }
    }

    pub fn find_batch_parts_by_batch_id(
        &self,
        batch_id: &str,
        session: &mut DbSession,
    ) -> Vec<crate::identity::entities::BatchPartEntity> {
        // Java parity: DbSqlSession.selectList throws on SQL error; an empty list means a no-row query, not a storage failure.
        match session.find_by("batch_part_entities", "batch_id", batch_id) {
            Ok(found) => found,
            Err(error) => {
                session.note_write_error(error);
                Vec::new()
            }
        }
    }

    pub fn find_batch_parts_by_batch_id_and_status(
        &self,
        batch_id: &str,
        status: &str,
        session: &mut DbSession,
    ) -> Vec<crate::identity::entities::BatchPartEntity> {
        self.find_batch_parts_by_batch_id(batch_id, session)
            .into_iter()
            .filter(|part| part.status == status)
            .collect()
    }

    // ── Variable methods ──

    pub fn insert_variable(
        &self,
        execution_id: &str,
        process_instance_id: &str,
        name: &str,
        value: serde_json::Value,
        session: &mut DbSession,
    ) {
        let id = format!("{}:{}", execution_id, name);
        session
            .insert_with_extra(
                "variables",
                &id,
                &value,
                &[
                    ("execution_id".into(), Some(execution_id.to_string())),
                    (
                        "process_instance_id".into(),
                        Some(process_instance_id.to_string()),
                    ),
                    ("name".into(), Some(name.to_string())),
                ],
            )
            .unwrap_or_default();
    }

    pub fn find_variables_by_execution_id(
        &self,
        execution_id: &str,
        session: &mut DbSession,
    ) -> Result<HashMap<String, serde_json::Value>, StorageError> {
        let rows = session.find_raw_by("variables", "execution_id", execution_id)?;
        let mut out = HashMap::with_capacity(rows.len());
        for row in rows {
            let name = row
                .extras
                .get("name")
                .and_then(|n| n.clone())
                .unwrap_or_default();
            // Java parity: corrupt variable JSON is a persistence failure, not null.
            let value = serde_json::from_str(&row.data).map_err(|e| {
                StorageError::Deserialization(format!(
                    "variable '{name}' in execution '{execution_id}': {e}"
                ))
            })?;
            out.insert(name, value);
        }
        Ok(out)
    }

    pub fn delete_variables_by_execution_id(&self, execution_id: &str, session: &mut DbSession) {
        session
            .delete_by("variables", "execution_id", execution_id)
            .unwrap_or_default();
    }

    pub fn delete_variable_by_execution_id_and_name(
        &self,
        execution_id: &str,
        name: &str,
        session: &mut DbSession,
    ) {
        let id = format!("{}:{}", execution_id, name);
        let _ = session.delete("variables", &id);
    }

    pub fn delete_variables_by_process_instance_id(
        &self,
        process_instance_id: &str,
        session: &mut DbSession,
    ) {
        session
            .delete_by("variables", "process_instance_id", process_instance_id)
            .unwrap_or_default();
    }

    // ── Event Subscription methods ──

    pub fn insert_event_subscription(
        &self,
        execution_id: &str,
        process_instance_id: &str,
        event_name: &str,
        event_kind: &str,
        data: serde_json::Value,
        session: &mut DbSession,
    ) {
        let id = uuid::Uuid::new_v4().to_string();
        session
            .insert_with_extra(
                "event_subscriptions",
                &id,
                &data,
                &[
                    ("execution_id".into(), Some(execution_id.to_string())),
                    (
                        "process_instance_id".into(),
                        Some(process_instance_id.to_string()),
                    ),
                    ("event_name".into(), Some(event_name.to_string())),
                    ("event_kind".into(), Some(event_kind.to_string())),
                ],
            )
            .unwrap_or_default();
    }

    pub fn find_event_subscriptions_by_execution_id(
        &self,
        execution_id: &str,
        session: &mut DbSession,
    ) -> Vec<serde_json::Value> {
        // Java parity: DbSqlSession.selectList throws on SQL error; an empty list means a no-row query, not a storage failure.
        match session.find_by("event_subscriptions", "execution_id", execution_id) {
            Ok(found) => found,
            Err(error) => {
                session.note_write_error(error);
                Vec::new()
            }
        }
    }

    pub fn delete_event_subscriptions_by_execution_id(
        &self,
        execution_id: &str,
        session: &mut DbSession,
    ) {
        session
            .delete_by("event_subscriptions", "execution_id", execution_id)
            .unwrap_or_default();
    }

    // ── Event wait state methods ──

    pub fn insert_event_wait_state(
        &self,
        wait_state: &RuntimeEventWaitState,
        session: &mut DbSession,
    ) {
        if let Some(sub) = &wait_state.event_subscription {
            self.insert_event_subscription(
                &wait_state.execution_id,
                &wait_state.process_instance_id,
                &sub.event_ref,
                match sub.kind {
                    EventSubscriptionKind::Message => "message",
                    EventSubscriptionKind::Signal => "signal",
                    EventSubscriptionKind::Conditional => "conditional",
                    EventSubscriptionKind::Error => "error",
                    EventSubscriptionKind::Cancel => "cancel",
                    EventSubscriptionKind::Compensate => "compensate",
                    EventSubscriptionKind::Escalation => "escalation",
                    // Java event-subscription eventType for registry events.
                    EventSubscriptionKind::EventRegistry => "event-registry",
                },
                serde_json::to_value(wait_state).unwrap_or(serde_json::Value::Null),
                session,
            );
        }

        session
            .insert_with_extra(
                "event_wait_states",
                &wait_state.execution_id,
                &wait_state,
                &[(
                    "process_instance_id".into(),
                    Some(wait_state.process_instance_id.clone()),
                )],
            )
            .unwrap_or_default();
    }

    pub fn insert_message_style_wait_state(
        &self,
        wait_state: &RuntimeEventWaitState,
        session: &mut DbSession,
    ) {
        self.insert_event_wait_state(wait_state, session);
    }

    pub fn delete_event_wait_state_by_execution_id(
        &self,
        execution_id: &str,
        session: &mut DbSession,
    ) {
        self.delete_event_subscriptions_by_execution_id(execution_id, session);
        let _ = session.delete("event_wait_states", execution_id);
    }

    pub fn delete_message_style_wait_state_by_execution_id(
        &self,
        execution_id: &str,
        session: &mut DbSession,
    ) {
        self.delete_event_wait_state_by_execution_id(execution_id, session);
    }

    pub fn find_event_wait_state_by_execution_id(
        &self,
        execution_id: &str,
        session: &mut DbSession,
    ) -> Option<RuntimeEventWaitState> {
        {
            match session.find("event_wait_states", execution_id) {
                Ok(found) => found,
                Err(error) => {
                    session.note_write_error(error);
                    None
                }
            }
        }
    }

    pub fn find_message_style_wait_state_by_execution_id(
        &self,
        execution_id: &str,
        session: &mut DbSession,
    ) -> Option<RuntimeEventWaitState> {
        self.find_event_wait_state_by_execution_id(execution_id, session)
    }

    pub fn find_event_wait_states_by_process_instance_id(
        &self,
        process_instance_id: &str,
        session: &mut DbSession,
    ) -> Vec<RuntimeEventWaitState> {
        // Java parity: DbSqlSession.selectList throws on SQL error; an empty list means a no-row query, not a storage failure.
        match session.find_by(
            "event_wait_states",
            "process_instance_id",
            process_instance_id,
        ) {
            Ok(found) => found,
            Err(error) => {
                session.note_write_error(error);
                Vec::new()
            }
        }
    }

    pub fn delete_event_wait_states_by_process_instance_id(
        &self,
        process_instance_id: &str,
        session: &mut DbSession,
    ) {
        session
            .delete_by(
                "event_wait_states",
                "process_instance_id",
                process_instance_id,
            )
            .unwrap_or_default();
    }

    pub fn find_message_style_wait_states_by_process_instance_id(
        &self,
        process_instance_id: &str,
        session: &mut DbSession,
    ) -> Vec<RuntimeEventWaitState> {
        self.find_event_wait_states_by_process_instance_id(process_instance_id, session)
    }

    pub fn snapshot_event_wait_states(
        &self,
        session: &mut DbSession,
    ) -> HashMap<String, RuntimeEventWaitState> {
        {
            match session.find_all::<RuntimeEventWaitState>("event_wait_states") {
                Ok(found) => found,
                Err(error) => {
                    session.note_write_error(error);
                    Vec::new()
                }
            }
        }
        .into_iter()
        .map(|e| (e.execution_id.clone(), e))
        .collect()
    }

    pub fn snapshot_message_style_wait_states(
        &self,
        session: &mut DbSession,
    ) -> HashMap<String, RuntimeEventWaitState> {
        self.snapshot_event_wait_states(session)
    }

    // ── Boundary event state methods ──

    pub fn insert_boundary_event_state(
        &self,
        state: RuntimeBoundaryEventState,
        session: &mut DbSession,
    ) {
        let key = format!("{}:{}", state.process_instance_id, state.boundary_event_id);
        session
            .insert_with_extra(
                "boundary_event_states",
                &key,
                &state,
                &[
                    (
                        "process_instance_id".into(),
                        Some(state.process_instance_id.clone()),
                    ),
                    (
                        "host_execution_id".into(),
                        Some(state.host_execution_id.clone()),
                    ),
                ],
            )
            .unwrap_or_default();
    }

    pub fn delete_boundary_event_state(
        &self,
        boundary_event_id: &str,
        process_instance_id: &str,
        session: &mut DbSession,
    ) {
        let key = format!("{}:{}", process_instance_id, boundary_event_id);
        let _ = session.delete("boundary_event_states", &key);
    }

    pub fn delete_boundary_event_states_by_host_execution_id(
        &self,
        host_execution_id: &str,
        session: &mut DbSession,
    ) {
        session
            .delete_by(
                "boundary_event_states",
                "host_execution_id",
                host_execution_id,
            )
            .unwrap_or_default();
    }

    pub fn find_boundary_event_state(
        &self,
        boundary_event_id: &str,
        process_instance_id: &str,
        session: &mut DbSession,
    ) -> Option<RuntimeBoundaryEventState> {
        let key = format!("{}:{}", process_instance_id, boundary_event_id);
        {
            match session.find("boundary_event_states", &key) {
                Ok(found) => found,
                Err(error) => {
                    session.note_write_error(error);
                    None
                }
            }
        }
    }

    pub fn find_boundary_event_states_by_host_execution_id(
        &self,
        host_execution_id: &str,
        session: &mut DbSession,
    ) -> Vec<RuntimeBoundaryEventState> {
        // Java parity: DbSqlSession.selectList throws on SQL error; an empty list means a no-row query, not a storage failure.
        match session.find_by(
            "boundary_event_states",
            "host_execution_id",
            host_execution_id,
        ) {
            Ok(found) => found,
            Err(error) => {
                session.note_write_error(error);
                Vec::new()
            }
        }
    }

    pub fn find_boundary_event_states_by_process_instance_id(
        &self,
        process_instance_id: &str,
        session: &mut DbSession,
    ) -> Vec<RuntimeBoundaryEventState> {
        // Java parity: DbSqlSession.selectList throws on SQL error; an empty list means a no-row query, not a storage failure.
        match session.find_by(
            "boundary_event_states",
            "process_instance_id",
            process_instance_id,
        ) {
            Ok(found) => found,
            Err(error) => {
                session.note_write_error(error);
                Vec::new()
            }
        }
    }

    pub fn delete_boundary_event_states_by_process_instance_id(
        &self,
        process_instance_id: &str,
        session: &mut DbSession,
    ) {
        session
            .delete_by(
                "boundary_event_states",
                "process_instance_id",
                process_instance_id,
            )
            .unwrap_or_default();
    }

    pub fn snapshot_boundary_event_states(
        &self,
        session: &mut DbSession,
    ) -> HashMap<String, RuntimeBoundaryEventState> {
        {
            match session.find_all::<RuntimeBoundaryEventState>("boundary_event_states") {
                Ok(found) => found,
                Err(error) => {
                    session.note_write_error(error);
                    Vec::new()
                }
            }
        }
        .into_iter()
        .map(|e| {
            (
                format!("{}:{}", e.process_instance_id, e.boundary_event_id),
                e,
            )
        })
        .collect()
    }

    // ── Timer Job State methods ──

    pub fn insert_timer_job_state(
        &self,
        state: &RuntimeTimerJobState,
        session: &mut DbSession,
    ) -> Result<(), crate::persistence::StorageError> {
        let job_type = self.find_timer_job_type(&state.timer_job_id, session);
        self.insert_timer_job_state_with_type(state, job_type.as_ref(), session)?;
        Ok(())
    }

    pub fn insert_timer_job_state_with_type(
        &self,
        state: &RuntimeTimerJobState,
        job_type: Option<&RuntimeJobType>,
        session: &mut DbSession,
    ) -> Result<(), StorageError> {
        // Ensure create-time / correlation / handler / query-metadata defaults for
        // every write so updates (retries, family moves) keep stable dimensions.
        let mut state = state.clone();
        let now_ms = self.time_source().now().timestamp_millis();
        // Prefer existing row metadata when this is an update/move and the
        // incoming payload omitted query dimensions (common for family flips).
        if let Some(existing) = self.find_timer_job_state(&state.timer_job_id, session) {
            copy_job_query_metadata(&existing, &mut state);
        }
        if state.create_time.is_none() {
            state.create_time = Some(now_ms);
        }
        if state.correlation_id.is_none() {
            state.correlation_id = Some(uuid::Uuid::new_v4().to_string());
        }
        if state.handler_type.is_none() {
            state.handler_type = Some(infer_handler_type(&state).to_string());
        }
        // Best-effort denormalized tenant / process definition from execution/instance.
        if state.tenant_id.is_none() || state.process_definition_id.is_none() {
            if let Some(execution) = self.find_execution(&state.execution_id, session) {
                if state.tenant_id.is_none() {
                    state.tenant_id = execution.tenant_id.clone();
                }
                if state.process_definition_id.is_none() {
                    state.process_definition_id = execution.process_definition_id.clone();
                }
                if state.element_name.is_none()
                    && execution.activity_id.as_deref() == Some(state.activity_id.as_str())
                {
                    state.element_name = execution.activity_name.clone();
                }
            }
            if (state.tenant_id.is_none() || state.process_definition_id.is_none())
                && !state.process_instance_id.is_empty()
            {
                if let Some(instance) =
                    self.find_process_instance(&state.process_instance_id, session)?
                {
                    if state.tenant_id.is_none() {
                        state.tenant_id = instance.tenant_id.clone();
                    }
                    if state.process_definition_id.is_none() {
                        state.process_definition_id = Some(instance.process_definition_id.clone());
                    }
                }
            }
        }

        // Use typed extras (not insert_with_extra): on PostgreSQL ON CONFLICT DO
        // UPDATE only touches listed columns, and insert_with_extra drops None
        // values — so a re-insert that clears lock_time/lock_owner would leave the
        // previous bigint/text lock columns set (P73b shared-DB matrix).
        fn opt_text(value: &Option<String>) -> DbValue {
            match value {
                Some(s) => DbValue::Text(s.clone()),
                None => DbValue::Null,
            }
        }
        fn opt_i64(value: Option<i64>) -> DbValue {
            match value {
                Some(v) => DbValue::Integer(v),
                None => DbValue::NullInteger,
            }
        }
        session.insert_with_typed_extra(
            "timer_job_states",
            &state.timer_job_id,
            &state,
            &[
                (
                    "process_instance_id".into(),
                    DbValue::Text(state.process_instance_id.clone()),
                ),
                (
                    "execution_id".into(),
                    DbValue::Text(state.execution_id.clone()),
                ),
                (
                    "activity_id".into(),
                    DbValue::Text(state.activity_id.clone()),
                ),
                ("lock_owner".into(), opt_text(&state.lock_owner)),
                ("lock_time".into(), opt_i64(state.lock_time)),
                (
                    "lock_expiration_time".into(),
                    opt_i64(state.lock_expiration_time),
                ),
                ("retries".into(), opt_i64(state.retries.map(|v| v as i64))),
                ("error_message".into(), opt_text(&state.error_message)),
                ("error_details".into(), opt_text(&state.error_details)),
                ("due_time".into(), opt_i64(state.due_time)),
                ("job_state".into(), opt_text(&state.job_state)),
                (
                    "job_type".into(),
                    match job_type {
                        Some(jt) => DbValue::Text(jt.as_str().to_string()),
                        None => DbValue::Null,
                    },
                ),
                ("create_time".into(), opt_i64(state.create_time)),
                ("correlation_id".into(), opt_text(&state.correlation_id)),
                ("handler_type".into(), opt_text(&state.handler_type)),
                ("tenant_id".into(), opt_text(&state.tenant_id)),
                (
                    "process_definition_id".into(),
                    opt_text(&state.process_definition_id),
                ),
                ("element_name".into(), opt_text(&state.element_name)),
                ("category".into(), opt_text(&state.category)),
                ("scope_id".into(), opt_text(&state.scope_id)),
                ("sub_scope_id".into(), opt_text(&state.sub_scope_id)),
                ("scope_type".into(), opt_text(&state.scope_type)),
                (
                    "scope_definition_id".into(),
                    opt_text(&state.scope_definition_id),
                ),
            ],
        )?;
        Ok(())
    }

    pub fn find_timer_job_type(
        &self,
        timer_job_id: &str,
        session: &mut DbSession,
    ) -> Option<RuntimeJobType> {
        // Java parity: DbSqlSession.selectOne throws on SQL error; None means a no-row query, not a storage failure.
        (match session.find_raw("timer_job_states", timer_job_id) {
            Ok(found) => found,
            Err(error) => {
                session.note_write_error(error);
                None
            }
        })
        .and_then(|row| row.extras.get("job_type").cloned().flatten())
        .map(|value| RuntimeJobType::from_persisted(&value))
    }

    /// Java external-worker query family isolation:
    /// only `job_type == externalWorker` + active `job_state == timer` rows whose
    /// parent process is not suspended are visible on `/external-worker/jobs`.
    ///
    /// Suspended / deadletter / history / plain timer / executable / definition-
    /// suspension timers are excluded. Legacy untyped timer inference is intentionally
    /// *not* applied here — that remains fetch-and-lock only.
    pub fn is_active_external_worker_job(
        &self,
        job: &RuntimeTimerJobState,
        session: &mut DbSession,
    ) -> Result<bool, StorageError> {
        if self.find_timer_job_type(&job.timer_job_id, session)
            != Some(RuntimeJobType::ExternalWorker)
        {
            return Ok(false);
        }
        if job.job_state.as_deref() != Some("timer") {
            return Ok(false);
        }
        self.external_worker_parent_allows_visibility(job, session)
    }

    /// Fetch-and-lock candidate selection (broader than list/get).
    ///
    /// Accepts:
    /// - typed `externalWorker` rows in timer state;
    /// - **legacy untyped** timer rows (missing `job_type`) for the timer-backed
    ///   external-worker extension path used by existing acquire/complete tests.
    ///
    /// Rejects:
    /// - typed `timer` / `history` / message / other job types (must not be
    ///   reclassified as external-worker on lock);
    /// - definition-suspension/activation timers;
    /// - suspended parent process;
    /// - non-timer job states.
    ///
    /// Event-wait presence alone is **not** used as a classification signal —
    /// that was the misclassification path for ordinary intermediate timers.
    pub fn is_fetchable_external_worker_candidate(
        &self,
        job: &RuntimeTimerJobState,
        session: &mut DbSession,
    ) -> Result<bool, StorageError> {
        if !matches!(job.job_state.as_deref(), None | Some("timer")) {
            return Ok(false);
        }
        if is_process_definition_schedule_timer(job) {
            return Ok(false);
        }
        if !self.external_worker_parent_allows_visibility(job, session)? {
            return Ok(false);
        }
        Ok(match self.find_timer_job_type(&job.timer_job_id, session) {
            Some(RuntimeJobType::ExternalWorker) => true,
            // Legacy untyped promotion path — preserved, not deleted wholesale.
            None => true,
            // Explicit non-EW types must never be acquired or re-stamped.
            Some(RuntimeJobType::Timer | RuntimeJobType::History | RuntimeJobType::Other(_)) => {
                false
            }
        })
    }

    fn external_worker_parent_allows_visibility(
        &self,
        job: &RuntimeTimerJobState,
        session: &mut DbSession,
    ) -> Result<bool, StorageError> {
        if job.process_instance_id.is_empty() {
            return Ok(true);
        }
        Ok(!self
            .find_process_instance(&job.process_instance_id, session)?
            .is_some_and(|process_instance| process_instance.is_suspended))
    }

    pub fn delete_timer_job_state(&self, timer_job_id: &str, session: &mut DbSession) {
        let _ = session.delete("timer_job_states", timer_job_id);
    }

    pub fn delete_timer_job_states_by_execution_id(
        &self,
        execution_id: &str,
        session: &mut DbSession,
    ) {
        session
            .delete_by("timer_job_states", "execution_id", execution_id)
            .unwrap_or_default();
    }

    pub fn find_timer_job_state(
        &self,
        timer_job_id: &str,
        session: &mut DbSession,
    ) -> Option<RuntimeTimerJobState> {
        {
            match session.find("timer_job_states", timer_job_id) {
                Ok(found) => found,
                Err(error) => {
                    session.note_write_error(error);
                    None
                }
            }
        }
    }

    pub fn find_timer_job_states_by_execution_id(
        &self,
        execution_id: &str,
        session: &mut DbSession,
    ) -> Vec<RuntimeTimerJobState> {
        // Java parity: DbSqlSession.selectList throws on SQL error; an empty list means a no-row query, not a storage failure.
        match session.find_by("timer_job_states", "execution_id", execution_id) {
            Ok(found) => found,
            Err(error) => {
                session.note_write_error(error);
                Vec::new()
            }
        }
    }

    pub fn find_timer_job_states_by_process_instance_id(
        &self,
        process_instance_id: &str,
        session: &mut DbSession,
    ) -> Vec<RuntimeTimerJobState> {
        // Java parity: DbSqlSession.selectList throws on SQL error; an empty list means a no-row query, not a storage failure.
        match session.find_by(
            "timer_job_states",
            "process_instance_id",
            process_instance_id,
        ) {
            Ok(found) => found,
            Err(error) => {
                session.note_write_error(error);
                Vec::new()
            }
        }
    }

    pub fn delete_timer_job_states_by_process_instance_id(
        &self,
        process_instance_id: &str,
        session: &mut DbSession,
    ) {
        session
            .delete_by(
                "timer_job_states",
                "process_instance_id",
                process_instance_id,
            )
            .unwrap_or_default();
    }

    pub fn snapshot_timer_job_states(
        &self,
        session: &mut DbSession,
    ) -> HashMap<String, RuntimeTimerJobState> {
        {
            match session.find_all::<RuntimeTimerJobState>("timer_job_states") {
                Ok(found) => found,
                Err(error) => {
                    session.note_write_error(error);
                    Vec::new()
                }
            }
        }
        .into_iter()
        .map(|e| (e.timer_job_id.clone(), e))
        .collect()
    }

    /// Typed management job query: every criteria predicate, the sort order,
    /// the id tie-break, and offset/limit are pushed into SQL over the
    /// denormalized `timer_job_states` columns. The count uses the same WHERE
    /// clause as the page, and job-type filters read the projected `job_type`
    /// column instead of issuing a per-row lookup.
    pub fn query_runtime_jobs(
        &self,
        criteria: &RuntimeJobQueryCriteria,
        session: &mut DbSession,
    ) -> Result<(Vec<RuntimeTimerJobState>, usize), StorageError> {
        let mut conditions: Vec<String> = Vec::new();
        let mut params = DbParams::new();

        match criteria.family {
            RuntimeJobFamily::All => {}
            RuntimeJobFamily::Timer => {
                conditions.push("(job_state IS NULL OR job_state = 'timer')".into());
            }
            RuntimeJobFamily::Executable => {
                conditions.push("job_state IN ('executable', 'async', 'async-after')".into());
            }
            RuntimeJobFamily::Deadletter => conditions.push("job_state = 'deadletter'".into()),
            RuntimeJobFamily::Suspended => conditions.push("job_state = 'suspended'".into()),
            RuntimeJobFamily::History => conditions.push("job_state = 'history'".into()),
        }

        let push_eq =
            |column: &str, value: &str, conditions: &mut Vec<String>, params: &mut DbParams| {
                conditions.push(format!("{column} = ?"));
                params.push(value);
            };
        if let Some(id) = criteria.id.as_deref() {
            push_eq("id", id, &mut conditions, &mut params);
        }
        if let Some(pi) = criteria.process_instance_id.as_deref() {
            push_eq("process_instance_id", pi, &mut conditions, &mut params);
        }
        if criteria.without_process_instance_id {
            conditions.push("(process_instance_id IS NULL OR process_instance_id = '')".into());
        }
        if let Some(def) = criteria.process_definition_id.as_deref() {
            push_eq("process_definition_id", def, &mut conditions, &mut params);
        }
        if let Some(execution) = criteria.execution_id.as_deref() {
            push_eq("execution_id", execution, &mut conditions, &mut params);
        }
        if let Some(element) = criteria.element_id.as_deref() {
            push_eq("activity_id", element, &mut conditions, &mut params);
        }
        if let Some(name) = criteria.element_name.as_deref() {
            push_eq("element_name", name, &mut conditions, &mut params);
        }
        if let Some(handler) = criteria.handler_type.as_deref() {
            push_eq("handler_type", handler, &mut conditions, &mut params);
        }
        if !criteria.handler_types.is_empty() {
            let marks = vec!["?"; criteria.handler_types.len()].join(", ");
            conditions.push(format!("handler_type IN ({marks})"));
            for handler in &criteria.handler_types {
                params.push(handler.as_str());
            }
        }
        if let Some(category) = criteria.category.as_deref() {
            push_eq("category", category, &mut conditions, &mut params);
        }
        if let Some(pattern) = criteria.category_like.as_deref() {
            conditions.push("category LIKE ?".into());
            params.push(pattern);
        }
        if let Some(scope_id) = criteria.scope_id.as_deref() {
            push_eq("scope_id", scope_id, &mut conditions, &mut params);
        }
        if criteria.without_scope_id {
            conditions.push("(scope_id IS NULL OR TRIM(scope_id) = '')".into());
        }
        if let Some(sub_scope_id) = criteria.sub_scope_id.as_deref() {
            push_eq("sub_scope_id", sub_scope_id, &mut conditions, &mut params);
        }
        if let Some(scope_type) = criteria.scope_type.as_deref() {
            push_eq("scope_type", scope_type, &mut conditions, &mut params);
        }
        if criteria.without_scope_type {
            conditions.push("(scope_type IS NULL OR TRIM(scope_type) = '')".into());
        }
        if let Some(def) = criteria.scope_definition_id.as_deref() {
            push_eq("scope_definition_id", def, &mut conditions, &mut params);
        }
        if criteria.case_definition_key.is_some() {
            // Empty resolved ID list means no matching case definitions → no jobs.
            if criteria.case_definition_ids.is_empty() {
                conditions.push("1 = 0".into());
            } else {
                let marks = vec!["?"; criteria.case_definition_ids.len()].join(", ");
                conditions.push(format!("scope_definition_id IN ({marks})"));
                for id in &criteria.case_definition_ids {
                    params.push(id.as_str());
                }
            }
        }
        if let Some(correlation) = criteria.correlation_id.as_deref() {
            push_eq("correlation_id", correlation, &mut conditions, &mut params);
        }
        if criteria.external_workers {
            // Persisted job_type wins; canonical handler type is the fallback
            // for rows written before job_type was stamped.
            conditions.push(
                "(job_type IN ('externalWorker', 'external-worker') \
                 OR (job_type IS NULL AND handler_type = 'external-worker-complete'))"
                    .into(),
            );
        }
        if criteria.timers_only {
            conditions.push(
                "(job_type = 'timer' \
                 OR (job_type IS NULL AND (job_state IS NULL OR job_state IN ('timer', 'executable'))))"
                    .into(),
            );
        }
        if criteria.messages_only {
            conditions.push(
                "(job_type = 'message' \
                 OR (job_type IS NULL AND job_state IS NOT NULL \
                     AND job_state NOT IN ('timer', 'executable') \
                     AND (handler_type IS NULL OR handler_type <> 'external-worker-complete')))"
                    .into(),
            );
        }
        if criteria.with_retries_left {
            conditions.push("(retries IS NULL OR retries > 0)".into());
        }
        if criteria.no_retries_left {
            conditions.push("(retries IS NOT NULL AND retries <= 0)".into());
        }
        if criteria.executable {
            conditions.push("(due_time IS NOT NULL AND due_time <= ?)".into());
            params.push(criteria.now_millis.unwrap_or(0));
        }
        if let Some(bound) = criteria.due_before {
            conditions.push("due_time < ?".into());
            params.push(bound);
        }
        if let Some(bound) = criteria.due_after {
            conditions.push("due_time > ?".into());
            params.push(bound);
        }
        if criteria.with_exception {
            conditions.push("(error_message IS NOT NULL AND error_message <> '')".into());
        }
        if criteria.without_exception {
            conditions.push("(error_message IS NULL OR error_message = '')".into());
        }
        if let Some(message) = criteria.exception_message.as_deref() {
            push_eq("error_message", message, &mut conditions, &mut params);
        }
        if criteria.locked {
            conditions.push("lock_owner IS NOT NULL".into());
        }
        if criteria.unlocked {
            conditions.push("lock_owner IS NULL".into());
        }
        if let Some(tenant) = criteria.tenant_id.as_deref() {
            push_eq("tenant_id", tenant, &mut conditions, &mut params);
        }
        if let Some(pattern) = criteria.tenant_id_like.as_deref() {
            conditions.push("tenant_id LIKE ?".into());
            params.push(pattern);
        }
        if criteria.without_tenant_id {
            conditions.push("(tenant_id IS NULL OR TRIM(tenant_id) = '')".into());
        }

        let where_clause = if conditions.is_empty() {
            String::new()
        } else {
            format!(" WHERE {}", conditions.join(" AND "))
        };

        let count_sql = format!("SELECT COUNT(*) AS cnt FROM timer_job_states{where_clause}");
        let total = session
            .raw_query_one(&count_sql, params.clone())?
            .and_then(|row| row.get_integer("cnt"))
            .unwrap_or(0)
            .max(0) as usize;

        let dir = match criteria.direction {
            Direction::Asc => "ASC",
            Direction::Desc => "DESC",
        };
        // Nullable sort columns keep the in-memory Option ordering (None first
        // ascending, None last descending) across dialects via the CASE prefix.
        let order_clause = match criteria.sort.as_str() {
            "dueDate" => format!(
                "(CASE WHEN due_time IS NULL THEN 0 ELSE 1 END) {dir}, due_time {dir}, id ASC"
            ),
            "createTime" => format!(
                "(CASE WHEN create_time IS NULL THEN 0 ELSE 1 END) {dir}, create_time {dir}, id ASC"
            ),
            "executionId" => format!("execution_id {dir}, id ASC"),
            "processInstanceId" => format!("process_instance_id {dir}, id ASC"),
            "retries" => format!("COALESCE(retries, 1) {dir}, id ASC"),
            "tenantId" => format!(
                "(CASE WHEN tenant_id IS NULL THEN 0 ELSE 1 END) {dir}, tenant_id {dir}, id ASC"
            ),
            // default + unknown → id
            _ => format!("id {dir}"),
        };

        let start = criteria.start.min(total);
        let limit = criteria.size.unwrap_or_else(|| total.saturating_sub(start));
        let limit_clause = session.dialect().limit_offset(Some(limit), Some(start));
        let page_sql = format!(
            "SELECT data FROM timer_job_states{where_clause} ORDER BY {order_clause} {limit_clause}"
        );
        let rows = session.raw_query(&page_sql, params)?;
        let mut data = Vec::with_capacity(rows.len());
        for row in rows {
            let json = row.get_text("data").ok_or_else(|| {
                StorageError::Persistence("timer job query row missing data column".to_string())
            })?;
            data.push(
                serde_json::from_str::<RuntimeTimerJobState>(&json)
                    .map_err(|e| StorageError::Deserialization(e.to_string()))?,
            );
        }
        Ok((data, total))
    }

    /// Returns true when `tenant_filter` is None/empty (no filtering), or when the
    /// process instance's tenant matches an entry in the filter.
    /// Process instances with no tenant match the empty string `""` in the filter.
    fn job_matches_tenant_filter(
        &self,
        job: &RuntimeTimerJobState,
        tenant_filter: Option<&[String]>,
        session: &mut DbSession,
    ) -> Result<bool, StorageError> {
        let Some(filter) = tenant_filter else {
            return Ok(true);
        };
        if filter.is_empty() {
            return Ok(true);
        }
        let process_tenant = self
            .find_process_instance(&job.process_instance_id, session)?
            .and_then(|pi| pi.tenant_id)
            .unwrap_or_default();
        Ok(filter.iter().any(|t| t == &process_tenant))
    }

    pub fn acquire_due_timer_jobs(
        &self,
        owner: &str,
        now: i64,
        lock_timeout_ms: i64,
        session: &mut DbSession,
    ) -> (Vec<RuntimeTimerJobState>, usize, usize) {
        self.acquire_due_timer_jobs_filtered(owner, now, lock_timeout_ms, None, None, session)
    }

    /// Acquire due timer/async jobs, optionally restricted to process instances
    /// whose `tenant_id` is in `tenant_filter`. None or empty filter = all tenants.
    /// When `category_filter` is non-empty, only jobs whose category is in the list are acquired;
    /// jobs with NULL category are excluded (matching Java enabledJobCategories semantics).
    pub fn acquire_due_timer_jobs_filtered(
        &self,
        owner: &str,
        now: i64,
        lock_timeout_ms: i64,
        tenant_filter: Option<&[String]>,
        category_filter: Option<&[String]>,
        session: &mut DbSession,
    ) -> (Vec<RuntimeTimerJobState>, usize, usize) {
        self.try_acquire_due_timer_jobs_filtered(
            owner,
            now,
            lock_timeout_ms,
            tenant_filter,
            category_filter,
            session,
        )
        .unwrap_or_else(|error| {
            tracing::warn!("timer-job acquisition storage operation failed: {error:?}");
            (Vec::new(), 0, 0)
        })
    }

    pub(crate) fn try_acquire_due_timer_jobs_filtered(
        &self,
        owner: &str,
        now: i64,
        lock_timeout_ms: i64,
        tenant_filter: Option<&[String]>,
        category_filter: Option<&[String]>,
        session: &mut DbSession,
    ) -> Result<(Vec<RuntimeTimerJobState>, usize, usize), StorageError> {
        let max_jobs = 100;
        let has_category_filter = category_filter.map(|f| !f.is_empty()).unwrap_or(false);
        let buffer_size = if has_category_filter {
            max_jobs * 12
        } else {
            max_jobs * 4
        };
        let filters: Vec<(String, FilterOp)> = vec![
            ("due_time".to_string(), FilterOp::LessThanOrEqual(now)),
            (
                "job_state".to_string(),
                FilterOp::In(vec![
                    "executable".to_string(),
                    "timer".to_string(),
                    "async".to_string(),
                    "async-after".to_string(),
                ]),
            ),
            ("retries".to_string(), FilterOp::GreaterThan(0)),
        ];
        let mut candidates: Vec<RuntimeTimerJobState> = session
            .find_with_filters::<RuntimeTimerJobState>(
                "timer_job_states",
                &filters,
                Some(("due_time", true)),
                Some(buffer_size),
            )?;
        // Unlocked-only: expired locks must be cleared by reset before reacquire.
        candidates.retain(|job| job.lock_owner.is_none());
        if tenant_filter.map(|f| !f.is_empty()).unwrap_or(false) {
            let mut filtered = Vec::with_capacity(candidates.len());
            for job in candidates {
                if self.job_matches_tenant_filter(&job, tenant_filter, session)? {
                    filtered.push(job);
                }
            }
            candidates = filtered;
        }
        if has_category_filter {
            candidates.retain(|job| {
                job.category
                    .as_ref()
                    .map(|cat| category_filter.is_some_and(|f| f.contains(cat)))
                    .unwrap_or(false)
            });
        }
        candidates.sort_by(|a, b| {
            a.due_time
                .cmp(&b.due_time)
                .then_with(|| a.timer_job_id.cmp(&b.timer_job_id))
        });
        let candidates: Vec<RuntimeTimerJobState> = candidates.into_iter().take(max_jobs).collect();

        let mut acquired = Vec::new();
        let mut conflicts = 0;

        for mut t in candidates {
            let old_lock_owner = t.lock_owner.clone();
            let old_lock_time = t.lock_time;

            t.lock_owner = Some(owner.to_string());
            t.lock_time = Some(now);
            t.lock_expiration_time = Some(now.saturating_add(lock_timeout_ms));

            let json = serde_json::to_string(&t)?;
            let affected = {
                let mut conditions: Vec<(String, Option<String>)> =
                    vec![("lock_owner".into(), old_lock_owner.clone())];
                if let Some(old_time) = old_lock_time {
                    conditions.push(("lock_time".into(), Some(old_time.to_string())));
                }
                session.cas_update(
                    "timer_job_states",
                    &t.timer_job_id,
                    &json,
                    &[
                        ("lock_owner".into(), Some(owner.to_string())),
                        ("lock_time".into(), Some(now.to_string())),
                        (
                            "lock_expiration_time".into(),
                            t.lock_expiration_time.map(|v| v.to_string()),
                        ),
                        ("due_time".into(), t.due_time.map(|v| v.to_string())),
                        ("job_state".into(), t.job_state.clone()),
                    ],
                    &conditions,
                )?
            };
            if affected > 0 {
                acquired.push(t);
            } else {
                conflicts += 1;
            }
        }
        // Recoveries are performed by the reset path, not acquisition.
        Ok((acquired, 0, conflicts))
    }

    pub fn acquire_due_async_timer_jobs(
        &self,
        owner: &str,
        now: i64,
        lock_duration_ms: i64,
        max_jobs: usize,
        session: &mut DbSession,
    ) -> Result<(Vec<RuntimeTimerJobState>, usize, usize), StorageError> {
        self.acquire_due_async_timer_jobs_filtered(
            owner,
            now,
            lock_duration_ms,
            max_jobs,
            None,
            None,
            session,
        )
    }

    /// Acquire only scheduled timer jobs. Async and async-after jobs are
    /// intentionally excluded so a dedicated async acquisition thread cannot
    /// race the timer acquisition thread for the same executable job.
    pub fn acquire_due_scheduled_timer_jobs_filtered(
        &self,
        owner: &str,
        now: i64,
        lock_duration_ms: i64,
        max_jobs: usize,
        tenant_filter: Option<&[String]>,
        category_filter: Option<&[String]>,
        session: &mut DbSession,
    ) -> Result<(Vec<RuntimeTimerJobState>, usize, usize), StorageError> {
        self.acquire_due_timer_jobs_by_states(
            &["timer"],
            owner,
            now,
            lock_duration_ms,
            max_jobs,
            None,
            tenant_filter,
            category_filter,
            session,
            JobLockEligibility::UnlockedOnly,
            AcquisitionWritePolicy::Optimistic,
        )
    }

    pub(crate) fn find_due_scheduled_timer_job_candidates_filtered(
        &self,
        now: i64,
        _lock_duration_ms: i64,
        tenant_filter: Option<&[String]>,
        category_filter: Option<&[String]>,
        session: &mut DbSession,
    ) -> Result<Vec<RuntimeTimerJobState>, StorageError> {
        let has_category_filter = category_filter.map(|f| !f.is_empty()).unwrap_or(false);
        let filters: Vec<(String, FilterOp)> = vec![
            ("due_time".to_string(), FilterOp::LessThanOrEqual(now)),
            (
                "job_state".to_string(),
                FilterOp::In(vec!["timer".to_string()]),
            ),
            ("retries".to_string(), FilterOp::GreaterThan(0)),
        ];
        let mut candidates = session.find_with_filters::<RuntimeTimerJobState>(
            "timer_job_states",
            &filters,
            Some(("due_time", true)),
            None,
        )?;
        // Expired locks require the reset path; acquisition only sees unlocked jobs.
        candidates.retain(|job| job.lock_owner.is_none());
        if tenant_filter
            .map(|filter| !filter.is_empty())
            .unwrap_or(false)
        {
            let mut filtered = Vec::with_capacity(candidates.len());
            for job in candidates {
                if self.job_matches_tenant_filter(&job, tenant_filter, session)? {
                    filtered.push(job);
                }
            }
            candidates = filtered;
        }
        if has_category_filter {
            candidates.retain(|job| {
                job.category
                    .as_ref()
                    .map(|cat| category_filter.is_some_and(|f| f.contains(cat)))
                    .unwrap_or(false)
            });
        }
        candidates.sort_by(|left, right| {
            left.due_time
                .cmp(&right.due_time)
                .then_with(|| left.timer_job_id.cmp(&right.timer_job_id))
        });
        Ok(candidates)
    }

    pub(crate) fn acquire_selected_scheduled_timer_jobs_filtered(
        &self,
        owner: &str,
        now: i64,
        lock_duration_ms: i64,
        selected_job_ids: &[String],
        tenant_filter: Option<&[String]>,
        category_filter: Option<&[String]>,
        session: &mut DbSession,
    ) -> Result<(Vec<RuntimeTimerJobState>, usize, usize), StorageError> {
        self.acquire_due_timer_jobs_by_states(
            &["timer"],
            owner,
            now,
            lock_duration_ms,
            selected_job_ids.len(),
            Some(selected_job_ids),
            tenant_filter,
            category_filter,
            session,
            JobLockEligibility::UnlockedOnly,
            AcquisitionWritePolicy::Optimistic,
        )
    }

    pub(crate) fn acquire_selected_scheduled_timer_jobs_global_filtered(
        &self,
        owner: &str,
        now: i64,
        lock_duration_ms: i64,
        selected_job_ids: &[String],
        tenant_filter: Option<&[String]>,
        category_filter: Option<&[String]>,
        session: &mut DbSession,
    ) -> Result<(Vec<RuntimeTimerJobState>, usize, usize), StorageError> {
        self.acquire_due_timer_jobs_by_states(
            &["timer"],
            owner,
            now,
            lock_duration_ms,
            selected_job_ids.len(),
            Some(selected_job_ids),
            tenant_filter,
            category_filter,
            session,
            JobLockEligibility::UnlockedOnly,
            AcquisitionWritePolicy::SerializedByGlobalLock,
        )
    }

    /// Acquire due async jobs, optionally restricted by process-instance tenant.
    pub fn acquire_due_async_timer_jobs_filtered(
        &self,
        owner: &str,
        now: i64,
        lock_duration_ms: i64,
        max_jobs: usize,
        tenant_filter: Option<&[String]>,
        category_filter: Option<&[String]>,
        session: &mut DbSession,
    ) -> Result<(Vec<RuntimeTimerJobState>, usize, usize), StorageError> {
        self.acquire_due_timer_jobs_by_states(
            &["executable", "async", "async-after"],
            owner,
            now,
            lock_duration_ms,
            max_jobs,
            None,
            tenant_filter,
            category_filter,
            session,
            JobLockEligibility::UnlockedOnly,
            AcquisitionWritePolicy::Optimistic,
        )
    }

    pub(crate) fn acquire_due_async_timer_jobs_global_filtered(
        &self,
        owner: &str,
        now: i64,
        lock_duration_ms: i64,
        max_jobs: usize,
        tenant_filter: Option<&[String]>,
        category_filter: Option<&[String]>,
        session: &mut DbSession,
    ) -> Result<(Vec<RuntimeTimerJobState>, usize, usize), StorageError> {
        self.acquire_due_timer_jobs_by_states(
            &["executable", "async", "async-after"],
            owner,
            now,
            lock_duration_ms,
            max_jobs,
            None,
            tenant_filter,
            category_filter,
            session,
            JobLockEligibility::UnlockedOnly,
            AcquisitionWritePolicy::SerializedByGlobalLock,
        )
    }

    pub fn acquire_due_history_jobs(
        &self,
        owner: &str,
        now: i64,
        lock_duration_ms: i64,
        max_jobs: usize,
        session: &mut DbSession,
    ) -> Result<(Vec<RuntimeTimerJobState>, usize, usize), StorageError> {
        self.acquire_due_timer_jobs_by_states(
            &["history"],
            owner,
            now,
            lock_duration_ms,
            max_jobs,
            None,
            None,
            None,
            session,
            JobLockEligibility::UnlockedOnly,
            AcquisitionWritePolicy::Optimistic,
        )
    }

    fn acquire_due_timer_jobs_by_states(
        &self,
        job_states: &[&str],
        owner: &str,
        now: i64,
        lock_duration_ms: i64,
        max_jobs: usize,
        selected_job_ids: Option<&[String]>,
        tenant_filter: Option<&[String]>,
        category_filter: Option<&[String]>,
        session: &mut DbSession,
        eligibility: JobLockEligibility,
        write_policy: AcquisitionWritePolicy,
    ) -> Result<(Vec<RuntimeTimerJobState>, usize, usize), StorageError> {
        // Larger buffer when tenant-filtered or category-filtered so we still
        // fill max_jobs after discard.
        let has_tenant_filter = tenant_filter.map(|f| !f.is_empty()).unwrap_or(false);
        let has_category_filter = category_filter.map(|f| !f.is_empty()).unwrap_or(false);
        let buffer_multiplier = if has_tenant_filter || has_category_filter {
            4
        } else {
            2
        };
        let buffer_size = max_jobs.saturating_mul(buffer_multiplier).max(max_jobs);
        let query_limit = if selected_job_ids.is_some() {
            None
        } else {
            Some(buffer_size)
        };
        let filters: Vec<(String, FilterOp)> = vec![
            ("due_time".to_string(), FilterOp::LessThanOrEqual(now)),
            (
                "job_state".to_string(),
                FilterOp::In(
                    job_states
                        .iter()
                        .map(|state| (*state).to_string())
                        .collect(),
                ),
            ),
            ("retries".to_string(), FilterOp::GreaterThan(0)),
        ];
        let mut candidates: Vec<RuntimeTimerJobState> = session
            .find_with_filters::<RuntimeTimerJobState>(
                "timer_job_states",
                &filters,
                Some(("due_time", true)),
                query_limit,
            )?;
        if let Some(selected_job_ids) = selected_job_ids {
            candidates.retain(|job| selected_job_ids.contains(&job.timer_job_id));
        }
        // Lock duration is only used when writing a newly acquired lease.
        // Eligibility never re-derives expiry from current lock duration.
        candidates.retain(|job| match eligibility {
            JobLockEligibility::UnlockedOnly => job.lock_owner.is_none(),
        });
        if tenant_filter.map(|f| !f.is_empty()).unwrap_or(false) {
            let mut filtered = Vec::with_capacity(candidates.len());
            for job in candidates {
                if self.job_matches_tenant_filter(&job, tenant_filter, session)? {
                    filtered.push(job);
                }
            }
            candidates = filtered;
        }
        if has_category_filter {
            candidates.retain(|job| {
                job.category
                    .as_ref()
                    .map(|cat| category_filter.is_some_and(|f| f.contains(cat)))
                    .unwrap_or(false)
            });
        }
        candidates.sort_by(|a, b| {
            a.due_time
                .cmp(&b.due_time)
                .then_with(|| a.timer_job_id.cmp(&b.timer_job_id))
        });
        let candidates: Vec<RuntimeTimerJobState> = candidates.into_iter().take(max_jobs).collect();

        let mut acquired = Vec::new();
        let mut conflicts = 0;

        if matches!(write_policy, AcquisitionWritePolicy::SerializedByGlobalLock) {
            let mut locked_candidates = Vec::with_capacity(candidates.len());
            let mut serialized = Vec::with_capacity(candidates.len());
            for mut candidate in candidates {
                candidate.lock_owner = Some(owner.to_string());
                candidate.lock_time = Some(now);
                candidate.lock_expiration_time = Some(now.saturating_add(lock_duration_ms));
                serialized.push(serde_json::to_string(&candidate)?);
                locked_candidates.push(candidate);
            }
            let rows: Vec<_> = locked_candidates
                .iter()
                .zip(serialized.iter())
                .map(|(job, json)| BulkJsonRowUpdate {
                    id: &job.timer_job_id,
                    json,
                })
                .collect();
            let affected = session.bulk_update_json_and_columns_by_ids(
                "timer_job_states",
                &rows,
                &[
                    ("lock_owner".into(), Some(owner.to_string())),
                    ("lock_time".into(), Some(now.to_string())),
                    (
                        "lock_expiration_time".into(),
                        Some(now.saturating_add(lock_duration_ms).to_string()),
                    ),
                ],
            )?;
            if affected != locked_candidates.len() {
                return Err(StorageError::Persistence(format!(
                    "serialized global acquisition selected {} jobs but updated {affected}",
                    locked_candidates.len()
                )));
            }
            return Ok((locked_candidates, 0, 0));
        }

        for mut t in candidates {
            let old_lock_owner = t.lock_owner.clone();
            let old_lock_time = t.lock_time;

            t.lock_owner = Some(owner.to_string());
            t.lock_time = Some(now);
            t.lock_expiration_time = Some(now.saturating_add(lock_duration_ms));

            let json = serde_json::to_string(&t)?;
            let affected = {
                let mut conditions: Vec<(String, Option<String>)> =
                    vec![("lock_owner".into(), old_lock_owner.clone())];
                if let Some(old_time) = old_lock_time {
                    conditions.push(("lock_time".into(), Some(old_time.to_string())));
                }
                session.cas_update(
                    "timer_job_states",
                    &t.timer_job_id,
                    &json,
                    &[
                        ("lock_owner".into(), Some(owner.to_string())),
                        ("lock_time".into(), Some(now.to_string())),
                        (
                            "lock_expiration_time".into(),
                            t.lock_expiration_time.map(|v| v.to_string()),
                        ),
                        ("due_time".into(), t.due_time.map(|v| v.to_string())),
                        ("job_state".into(), t.job_state.clone()),
                    ],
                    &conditions,
                )?
            };
            if affected > 0 {
                acquired.push(t);
            } else {
                conflicts += 1;
            }
        }
        // Expired-lease recovery is owned by reset, not acquisition.
        Ok((acquired, 0, conflicts))
    }

    pub fn reset_expired_job_locks_batch(
        &self,
        now: i64,
        job_class: ExpiredJobClass,
        page_size: usize,
        tenant_filter: Option<&[String]>,
        category_filter: Option<&[String]>,
        session: &mut DbSession,
    ) -> Result<ResetExpiredJobsBatchOutcome, StorageError> {
        let candidates = self.find_expired_job_lock_candidates(
            now,
            job_class,
            page_size,
            tenant_filter,
            category_filter,
            session,
        )?;
        self.compare_and_reset_expired_job_locks(candidates, session)
    }

    /// Selects expired job rows for a typed class using the authoritative
    /// `lock_expiration_time < now` predicate. The owner may be absent on an
    /// inconsistent row that still needs repair.
    ///
    /// Tenant scope is resolved via the job's process instance. Category
    /// filtering applies only to `Async` and `Timer` classes; `History` ignores
    /// categories to match Java history manager behavior.
    pub(crate) fn find_expired_job_lock_candidates(
        &self,
        now: i64,
        job_class: ExpiredJobClass,
        page_size: usize,
        tenant_filter: Option<&[String]>,
        category_filter: Option<&[String]>,
        session: &mut DbSession,
    ) -> Result<Vec<RuntimeTimerJobState>, StorageError> {
        let apply_tenant = tenant_filter
            .map(|filter| !filter.is_empty())
            .unwrap_or(false);
        let apply_category = matches!(job_class, ExpiredJobClass::Async | ExpiredJobClass::Timer)
            && category_filter
                .map(|filter| !filter.is_empty())
                .unwrap_or(false);
        let select_limit = if apply_tenant || apply_category {
            page_size.saturating_mul(8).max(page_size)
        } else {
            page_size
        };

        let filters: Vec<(String, FilterOp)> = vec![
            ("lock_expiration_time".to_string(), FilterOp::LessThan(now)),
            (
                "job_state".to_string(),
                FilterOp::In(
                    job_class
                        .job_states()
                        .iter()
                        .map(|state| (*state).to_string())
                        .collect(),
                ),
            ),
        ];
        // Physical column is `id` (JSON payload holds `timer_job_id`).
        let mut candidates: Vec<RuntimeTimerJobState> = session
            .find_with_filters::<RuntimeTimerJobState>(
                "timer_job_states",
                &filters,
                Some(("id", true)),
                Some(select_limit),
            )?;
        if apply_tenant {
            let mut filtered = Vec::with_capacity(candidates.len());
            for job in candidates {
                if self.job_matches_tenant_filter(&job, tenant_filter, session)? {
                    filtered.push(job);
                }
            }
            candidates = filtered;
        }
        if apply_category {
            let categories = category_filter.unwrap_or_default();
            candidates.retain(|job| {
                job.category
                    .as_ref()
                    .map(|category| categories.iter().any(|enabled| enabled == category))
                    .unwrap_or(false)
            });
        }
        candidates.sort_by(|a, b| a.timer_job_id.cmp(&b.timer_job_id));
        candidates.truncate(page_size);
        Ok(candidates)
    }

    /// Clears locks for previously selected candidates using full compare-and-set
    /// on state, owner, lock time, and lock expiration.
    pub(crate) fn compare_and_reset_expired_job_locks(
        &self,
        candidates: Vec<RuntimeTimerJobState>,
        session: &mut DbSession,
    ) -> Result<ResetExpiredJobsBatchOutcome, StorageError> {
        let scanned = candidates.len();
        let mut outcome = ResetExpiredJobsBatchOutcome {
            scanned,
            ..ResetExpiredJobsBatchOutcome::default()
        };
        for mut job in candidates {
            let old_lock_owner = job.lock_owner.clone();
            let old_lock_time = job.lock_time;
            let old_lock_expiration_time = job.lock_expiration_time;
            let old_job_state = job.job_state.clone();
            job.lock_owner = None;
            job.lock_time = None;
            job.lock_expiration_time = None;
            let json = serde_json::to_string(&job)?;
            let affected = session.cas_update(
                "timer_job_states",
                &job.timer_job_id,
                &json,
                &[
                    ("lock_owner".into(), None),
                    ("lock_time".into(), None),
                    ("lock_expiration_time".into(), None),
                ],
                &[
                    ("lock_owner".into(), old_lock_owner),
                    (
                        "lock_time".into(),
                        old_lock_time.map(|value| value.to_string()),
                    ),
                    (
                        "lock_expiration_time".into(),
                        old_lock_expiration_time.map(|value| value.to_string()),
                    ),
                    ("job_state".into(), old_job_state),
                ],
            )?;
            if affected > 0 {
                outcome.reset += 1;
            } else {
                outcome.conflicts += 1;
            }
        }
        Ok(outcome)
    }

    /// Resets expired locks across every job class.
    ///
    /// Java parity: `ResetExpiredJobsCmd` / `JobExecutor` treat a SQL failure as
    /// `FlowableException` (retryable, logged by the acquisition loop). Folding
    /// a `StorageError` into `0` would report "no expired locks" and silently
    /// stop recovery, which is strictly worse than panicking or failing the command.
    pub fn reset_expired_timer_job_locks(
        &self,
        now: i64,
        page_size: usize,
        session: &mut DbSession,
    ) -> Result<usize, StorageError> {
        let mut total = 0usize;
        for job_class in ExpiredJobClass::ALL.iter().copied() {
            let outcome =
                self.reset_expired_job_locks_batch(now, job_class, page_size, None, None, session)?;
            total += outcome.reset;
        }
        Ok(total)
    }

    pub fn release_timer_job_lock(
        &self,
        timer_job_id: &str,
        expected_owner: &str,
        session: &mut DbSession,
    ) -> Result<bool, StorageError> {
        // Do not use `find_timer_job_state` here: that legacy convenience API
        // maps storage failures to `None`, which is indistinguishable from a
        // genuinely deleted job and prevents the executor from retrying a
        // required unacquire after a transient database lock.
        let Some(mut job) =
            session.find::<RuntimeTimerJobState>("timer_job_states", timer_job_id)?
        else {
            return Ok(false);
        };
        if job.lock_owner.as_deref() != Some(expected_owner) {
            return Ok(false);
        }
        job.lock_owner = None;
        job.lock_time = None;
        job.lock_expiration_time = None;
        let json = serde_json::to_string(&job)?;
        Ok(session.cas_update(
            "timer_job_states",
            timer_job_id,
            &json,
            &[
                ("lock_owner".into(), None),
                ("lock_time".into(), None),
                ("lock_expiration_time".into(), None),
            ],
            &[("lock_owner".into(), Some(expected_owner.to_string()))],
        )? > 0)
    }

    /// Releases executable jobs owned by one async executor. Tenant scope is
    /// resolved in the command transaction; owner, lock timestamps, and state
    /// are rechecked by CAS so a renewed or transitioned job is not unlocked.
    pub fn unlock_owned_executable_jobs(
        &self,
        expected_owner: &str,
        tenant_filter: Option<&[String]>,
        session: &mut DbSession,
    ) -> Result<usize, StorageError> {
        let filters = vec![
            (
                "lock_owner".to_string(),
                FilterOp::Eq(Arc::from(expected_owner)),
            ),
            (
                "job_state".to_string(),
                FilterOp::In(vec![
                    "executable".to_string(),
                    "async".to_string(),
                    "async-after".to_string(),
                ]),
            ),
        ];
        let mut candidates = session.find_with_filters::<RuntimeTimerJobState>(
            "timer_job_states",
            &filters,
            Some(("id", true)),
            None,
        )?;
        if tenant_filter
            .map(|filter| !filter.is_empty())
            .unwrap_or(false)
        {
            let mut filtered = Vec::with_capacity(candidates.len());
            for job in candidates {
                let Some(instance) =
                    self.find_process_instance(&job.process_instance_id, session)?
                else {
                    continue;
                };
                let tenant_id = instance.tenant_id.unwrap_or_default();
                if tenant_filter
                    .unwrap_or_default()
                    .iter()
                    .any(|expected| expected == &tenant_id)
                {
                    filtered.push(job);
                }
            }
            candidates = filtered;
        }
        candidates.sort_by(|left, right| left.timer_job_id.cmp(&right.timer_job_id));

        let mut unlocked = 0usize;
        for mut job in candidates {
            let old_lock_time = job.lock_time;
            let old_lock_expiration_time = job.lock_expiration_time;
            let old_job_state = job.job_state.clone();
            job.lock_owner = None;
            job.lock_time = None;
            job.lock_expiration_time = None;
            let json = serde_json::to_string(&job)?;
            let affected = session.cas_update(
                "timer_job_states",
                &job.timer_job_id,
                &json,
                &[
                    ("lock_owner".into(), None),
                    ("lock_time".into(), None),
                    ("lock_expiration_time".into(), None),
                ],
                &[
                    ("lock_owner".into(), Some(expected_owner.to_string())),
                    (
                        "lock_time".into(),
                        old_lock_time.map(|value| value.to_string()),
                    ),
                    (
                        "lock_expiration_time".into(),
                        old_lock_expiration_time.map(|value| value.to_string()),
                    ),
                    ("job_state".into(), old_job_state),
                ],
            )?;
            if affected > 0 {
                unlocked += 1;
            }
        }
        Ok(unlocked)
    }

    /// Acquire external-worker jobs. When `topic` is `Some`, only jobs whose
    /// `job_handler_configuration` equals the topic are eligible (Java
    /// `AcquireExternalWorkerJobsCmd` / `findExternalJobsToExecute` topic filter).
    pub fn fetch_and_lock_external_worker_timer_jobs(
        &self,
        owner: &str,
        now: i64,
        max_jobs: usize,
        lock_duration_ms: i64,
        topic: Option<&str>,
        session: &mut DbSession,
    ) -> Result<Vec<RuntimeTimerJobState>, StorageError> {
        let mut candidates = Vec::new();
        for job in session.find_all::<RuntimeTimerJobState>("timer_job_states")? {
            if job.due_time.is_some_and(|due| due <= now)
                && job.retries.is_some_and(|retries| retries > 0)
                && (job.lock_owner.is_none()
                    || job
                        .lock_expiration_time
                        .is_some_and(|expires| expires <= now))
                && self.is_fetchable_external_worker_candidate(&job, session)?
                && topic.is_none_or(|topic| job.job_handler_configuration.as_deref() == Some(topic))
            {
                candidates.push(job);
            }
        }
        candidates.sort_by(|a, b| {
            a.due_time
                .cmp(&b.due_time)
                .then_with(|| a.timer_job_id.cmp(&b.timer_job_id))
        });
        let candidates: Vec<RuntimeTimerJobState> = candidates.into_iter().take(max_jobs).collect();
        let mut locked_jobs = Vec::new();

        for mut timer_job in candidates {
            let old_lock_owner = timer_job.lock_owner.clone();
            let old_lock_expiration_time = timer_job.lock_expiration_time;

            timer_job.lock_owner = Some(owner.to_string());
            timer_job.lock_time = Some(now);
            timer_job.lock_expiration_time = Some(now + lock_duration_ms);
            if timer_job.retries.is_none() {
                timer_job.retries = Some(1);
            }
            // Promote legacy untyped rows to externalWorker on successful lock;
            // already-typed externalWorker rows keep their type.
            let job_type_extra = Some(RuntimeJobType::ExternalWorker.as_str().to_string());

            let json = serde_json::to_string(&timer_job)?;
            let affected = {
                if let (Some(ref old_owner), Some(ref old_exp_time)) =
                    (old_lock_owner, old_lock_expiration_time)
                {
                    session.cas_update(
                        "timer_job_states",
                        &timer_job.timer_job_id,
                        &json,
                        &[
                            ("lock_owner".into(), Some(owner.to_string())),
                            ("lock_time".into(), Some(now.to_string())),
                            (
                                "lock_expiration_time".into(),
                                timer_job.lock_expiration_time.map(|v| v.to_string()),
                            ),
                            ("retries".into(), timer_job.retries.map(|v| v.to_string())),
                            ("error_message".into(), timer_job.error_message.clone()),
                            ("error_details".into(), timer_job.error_details.clone()),
                            ("due_time".into(), timer_job.due_time.map(|v| v.to_string())),
                            ("job_state".into(), timer_job.job_state.clone()),
                            ("job_type".into(), job_type_extra.clone()),
                        ],
                        &[
                            ("lock_owner".into(), Some(old_owner.clone())),
                            (
                                "lock_expiration_time".into(),
                                Some(old_exp_time.to_string()),
                            ),
                        ],
                    )?
                } else {
                    session.cas_update(
                        "timer_job_states",
                        &timer_job.timer_job_id,
                        &json,
                        &[
                            ("lock_owner".into(), Some(owner.to_string())),
                            ("lock_time".into(), Some(now.to_string())),
                            (
                                "lock_expiration_time".into(),
                                timer_job.lock_expiration_time.map(|v| v.to_string()),
                            ),
                            ("retries".into(), timer_job.retries.map(|v| v.to_string())),
                            ("error_message".into(), timer_job.error_message.clone()),
                            ("error_details".into(), timer_job.error_details.clone()),
                            ("due_time".into(), timer_job.due_time.map(|v| v.to_string())),
                            ("job_state".into(), timer_job.job_state.clone()),
                            ("job_type".into(), job_type_extra),
                        ],
                        &[("lock_owner".into(), None)],
                    )?
                }
            };

            if affected > 0 {
                locked_jobs.push(timer_job);
            }
        }

        Ok(locked_jobs)
    }

    pub fn replace_timer_job_state_if_locked(
        &self,
        state: &RuntimeTimerJobState,
        expected_lock_owner: &str,
        expected_lock_expiration_time: i64,
        session: &mut DbSession,
    ) -> bool {
        self.replace_timer_job_state_if_lock_matches(
            state,
            expected_lock_owner,
            Some(expected_lock_expiration_time),
            session,
        )
    }

    /// Compare-and-replace an external-worker job using the complete persisted
    /// lock identity. Java validates worker ownership even for legacy rows that
    /// have no lock expiration timestamp, so the expected expiration remains
    /// optional here while still participating in the CAS predicate.
    pub fn replace_timer_job_state_if_lock_matches(
        &self,
        state: &RuntimeTimerJobState,
        expected_lock_owner: &str,
        expected_lock_expiration_time: Option<i64>,
        session: &mut DbSession,
    ) -> bool {
        let json = serde_json::to_string(state).unwrap_or_else(|_| "{}".to_string());
        let affected = session
            .cas_update(
                "timer_job_states",
                &state.timer_job_id,
                &json,
                &[
                    (
                        "process_instance_id".into(),
                        Some(state.process_instance_id.clone()),
                    ),
                    ("execution_id".into(), Some(state.execution_id.clone())),
                    ("lock_owner".into(), state.lock_owner.clone()),
                    ("lock_time".into(), state.lock_time.map(|v| v.to_string())),
                    (
                        "lock_expiration_time".into(),
                        state.lock_expiration_time.map(|v| v.to_string()),
                    ),
                    ("retries".into(), state.retries.map(|v| v.to_string())),
                    ("error_message".into(), state.error_message.clone()),
                    ("error_details".into(), state.error_details.clone()),
                    ("due_time".into(), state.due_time.map(|v| v.to_string())),
                    ("job_state".into(), state.job_state.clone()),
                ],
                &[
                    ("lock_owner".into(), Some(expected_lock_owner.to_string())),
                    (
                        "lock_expiration_time".into(),
                        expected_lock_expiration_time.map(|value| value.to_string()),
                    ),
                ],
            )
            .unwrap_or_default();
        affected > 0
    }

    // ── Event Subprocess Timer Subscription methods ──

    pub fn insert_event_subprocess_timer_subscription(
        &self,
        sub: EventSubprocessTimerSubscription,
        session: &mut DbSession,
    ) {
        session
            .insert_with_extra(
                "event_subprocess_timer_subscriptions",
                &sub.subscription_id,
                &sub,
                &[
                    (
                        "process_instance_id".into(),
                        Some(sub.process_instance_id.clone()),
                    ),
                    ("lock_owner".into(), sub.lock_owner.clone()),
                    ("lock_time".into(), sub.lock_time.map(|v| v.to_string())),
                ],
            )
            .unwrap_or_default();
    }

    pub fn delete_event_subprocess_timer_subscription(
        &self,
        subscription_id: &str,
        session: &mut DbSession,
    ) {
        session
            .delete("event_subprocess_timer_subscriptions", subscription_id)
            .unwrap_or_default();
    }

    pub fn delete_event_subprocess_timer_subscriptions_by_process_instance_id(
        &self,
        process_instance_id: &str,
        session: &mut DbSession,
    ) {
        session
            .delete_by(
                "event_subprocess_timer_subscriptions",
                "process_instance_id",
                process_instance_id,
            )
            .unwrap_or_default();
    }

    pub fn find_event_subprocess_timer_subscriptions_by_process_instance_id(
        &self,
        process_instance_id: &str,
        session: &mut DbSession,
    ) -> Vec<EventSubprocessTimerSubscription> {
        // Java parity: DbSqlSession.selectList throws on SQL error; an empty list means a no-row query, not a storage failure.
        match session.find_by(
            "event_subprocess_timer_subscriptions",
            "process_instance_id",
            process_instance_id,
        ) {
            Ok(found) => found,
            Err(error) => {
                session.note_write_error(error);
                Vec::new()
            }
        }
    }

    pub fn snapshot_event_subprocess_timer_subscriptions(
        &self,
        session: &mut DbSession,
    ) -> HashMap<String, EventSubprocessTimerSubscription> {
        {
            match session.find_all::<EventSubprocessTimerSubscription>(
                "event_subprocess_timer_subscriptions",
            ) {
                Ok(found) => found,
                Err(error) => {
                    session.note_write_error(error);
                    Vec::new()
                }
            }
        }
        .into_iter()
        .map(|e| (e.subscription_id.clone(), e))
        .collect()
    }

    pub fn acquire_due_event_subprocess_timer_subscriptions(
        &self,
        owner: &str,
        now: i64,
        lock_timeout_ms: i64,
        session: &mut DbSession,
    ) -> (Vec<EventSubprocessTimerSubscription>, usize, usize) {
        self.acquire_due_event_subprocess_timer_subscriptions_filtered(
            owner,
            now,
            lock_timeout_ms,
            None,
            session,
        )
    }

    pub(crate) fn acquire_due_event_subprocess_timer_subscriptions_filtered(
        &self,
        owner: &str,
        now: i64,
        lock_timeout_ms: i64,
        category_filter: Option<&[String]>,
        session: &mut DbSession,
    ) -> (Vec<EventSubprocessTimerSubscription>, usize, usize) {
        self.acquire_due_event_subprocess_timer_subscriptions_selected(
            owner,
            now,
            lock_timeout_ms,
            None,
            category_filter,
            session,
            JobLockEligibility::UnlockedOnly,
            AcquisitionWritePolicy::Optimistic,
        )
        .unwrap_or_default()
    }

    pub(crate) fn find_due_event_subprocess_timer_subscription_candidates(
        &self,
        now: i64,
        _lock_timeout_ms: i64,
        category_filter: Option<&[String]>,
        session: &mut DbSession,
    ) -> Vec<EventSubprocessTimerSubscription> {
        // Expired locks require reset; acquisition only selects unlocked rows.
        let has_category_filter = category_filter.map(|f| !f.is_empty()).unwrap_or(false);
        let mut candidates: Vec<_> = self
            .snapshot_event_subprocess_timer_subscriptions(session)
            .into_values()
            .filter(|t| t.due_time.is_some_and(|d| d <= now))
            .filter(|t| t.lock_owner.is_none())
            .filter(|t| {
                if !has_category_filter {
                    return true;
                }
                t.category
                    .as_ref()
                    .map(|cat| category_filter.is_some_and(|f| f.contains(cat)))
                    .unwrap_or(false)
            })
            .collect();
        // Deterministic ordering: due time, then id (for stability)
        candidates.sort_by(|a, b| {
            a.due_time
                .cmp(&b.due_time)
                .then(a.subscription_id.cmp(&b.subscription_id))
        });
        candidates
    }

    pub(crate) fn acquire_selected_event_subprocess_timer_subscriptions(
        &self,
        owner: &str,
        now: i64,
        lock_timeout_ms: i64,
        selected_subscription_ids: &[String],
        category_filter: Option<&[String]>,
        session: &mut DbSession,
    ) -> (Vec<EventSubprocessTimerSubscription>, usize, usize) {
        self.acquire_due_event_subprocess_timer_subscriptions_selected(
            owner,
            now,
            lock_timeout_ms,
            Some(selected_subscription_ids),
            category_filter,
            session,
            JobLockEligibility::UnlockedOnly,
            AcquisitionWritePolicy::Optimistic,
        )
        .unwrap_or_default()
    }

    pub(crate) fn acquire_selected_event_subprocess_timer_subscriptions_global(
        &self,
        owner: &str,
        now: i64,
        lock_timeout_ms: i64,
        selected_subscription_ids: &[String],
        category_filter: Option<&[String]>,
        session: &mut DbSession,
    ) -> Result<(Vec<EventSubprocessTimerSubscription>, usize, usize), StorageError> {
        self.acquire_due_event_subprocess_timer_subscriptions_selected(
            owner,
            now,
            lock_timeout_ms,
            Some(selected_subscription_ids),
            category_filter,
            session,
            JobLockEligibility::UnlockedOnly,
            AcquisitionWritePolicy::SerializedByGlobalLock,
        )
    }

    fn acquire_due_event_subprocess_timer_subscriptions_selected(
        &self,
        owner: &str,
        now: i64,
        lock_timeout_ms: i64,
        selected_subscription_ids: Option<&[String]>,
        category_filter: Option<&[String]>,
        session: &mut DbSession,
        eligibility: JobLockEligibility,
        write_policy: AcquisitionWritePolicy,
    ) -> Result<(Vec<EventSubprocessTimerSubscription>, usize, usize), StorageError> {
        let mut candidates = self.find_due_event_subprocess_timer_subscription_candidates(
            now,
            lock_timeout_ms,
            category_filter,
            session,
        );
        if let Some(selected_subscription_ids) = selected_subscription_ids {
            candidates
                .retain(|candidate| selected_subscription_ids.contains(&candidate.subscription_id));
        }
        candidates.retain(|candidate| match eligibility {
            JobLockEligibility::UnlockedOnly => candidate.lock_owner.is_none(),
        });

        let mut acquired = Vec::new();
        let mut conflicts = 0;

        if matches!(write_policy, AcquisitionWritePolicy::SerializedByGlobalLock) {
            let mut locked_candidates = Vec::with_capacity(candidates.len());
            let mut serialized = Vec::with_capacity(candidates.len());
            for mut candidate in candidates {
                candidate.lock_owner = Some(owner.to_string());
                candidate.lock_time = Some(now);
                serialized.push(serde_json::to_string(&candidate)?);
                locked_candidates.push(candidate);
            }
            let rows: Vec<_> = locked_candidates
                .iter()
                .zip(serialized.iter())
                .map(|(subscription, json)| BulkJsonRowUpdate {
                    id: &subscription.subscription_id,
                    json,
                })
                .collect();
            let affected = session.bulk_update_json_and_columns_by_ids(
                "event_subprocess_timer_subscriptions",
                &rows,
                &[
                    ("lock_owner".into(), Some(owner.to_string())),
                    ("lock_time".into(), Some(now.to_string())),
                ],
            )?;
            if affected != locked_candidates.len() {
                return Err(StorageError::Persistence(format!(
                    "serialized global event-subprocess acquisition selected {} subscriptions but updated {affected}",
                    locked_candidates.len()
                )));
            }
            return Ok((locked_candidates, 0, 0));
        }

        for mut t in candidates {
            let old_lock_owner = t.lock_owner.clone();
            let old_lock_time = t.lock_time;

            t.lock_owner = Some(owner.to_string());
            t.lock_time = Some(now);

            let json = serde_json::to_string(&t)?;
            let affected = {
                let mut conditions = vec![("lock_owner".into(), old_lock_owner.clone())];
                if let Some(old_time) = old_lock_time {
                    conditions.push(("lock_time".into(), Some(old_time.to_string())));
                }
                session.cas_update(
                    "event_subprocess_timer_subscriptions",
                    &t.subscription_id,
                    &json,
                    &[
                        ("lock_owner".into(), Some(owner.to_string())),
                        ("lock_time".into(), Some(now.to_string())),
                    ],
                    &conditions,
                )?
            };
            if affected > 0 {
                acquired.push(t);
            } else {
                conflicts += 1;
            }
        }
        Ok((acquired, 0, conflicts))
    }

    /// Releases an acquired event-subprocess timer after task submission is
    /// rejected, preserving its due date and all non-lock state.
    pub fn release_event_subprocess_timer_subscription_lock(
        &self,
        sub: &EventSubprocessTimerSubscription,
        expected_owner: &str,
        session: &mut DbSession,
    ) -> Result<bool, StorageError> {
        let Some(lock_time) = sub.lock_time else {
            return Ok(false);
        };
        let mut updated_sub = sub.clone();
        updated_sub.lock_owner = None;
        updated_sub.lock_time = None;
        let json = serde_json::to_string(&updated_sub)?;
        Ok(session.cas_update(
            "event_subprocess_timer_subscriptions",
            &sub.subscription_id,
            &json,
            &[("lock_owner".into(), None), ("lock_time".into(), None)],
            &[
                ("lock_owner".into(), Some(expected_owner.to_string())),
                ("lock_time".into(), Some(lock_time.to_string())),
            ],
        )? > 0)
    }

    // ── Event Subprocess Event Subscription methods (message/signal) ──

    pub fn insert_event_subprocess_event_subscription(
        &self,
        sub: EventSubprocessEventSubscription,
        session: &mut DbSession,
    ) {
        let kind_str = match sub.event_kind {
            EventSubscriptionKind::Message => "message",
            EventSubscriptionKind::Signal => "signal",
            EventSubscriptionKind::Conditional => "conditional",
            EventSubscriptionKind::Error => "error",
            EventSubscriptionKind::Cancel => "cancel",
            EventSubscriptionKind::Compensate => "compensate",
            EventSubscriptionKind::Escalation => "escalation",
            EventSubscriptionKind::EventRegistry => "event-registry",
        };
        session
            .insert_with_extra(
                "event_subprocess_event_subscriptions",
                &sub.subscription_id,
                &sub,
                &[
                    (
                        "process_instance_id".into(),
                        Some(sub.process_instance_id.clone()),
                    ),
                    ("event_kind".into(), Some(kind_str.to_string())),
                    ("event_ref".into(), Some(sub.event_ref.clone())),
                ],
            )
            .unwrap_or_default();
    }

    pub fn delete_event_subprocess_event_subscription(
        &self,
        subscription_id: &str,
        session: &mut DbSession,
    ) {
        session
            .delete("event_subprocess_event_subscriptions", subscription_id)
            .unwrap_or_default();
    }

    pub fn delete_event_subprocess_event_subscriptions_by_process_instance_id(
        &self,
        process_instance_id: &str,
        session: &mut DbSession,
    ) {
        session
            .delete_by(
                "event_subprocess_event_subscriptions",
                "process_instance_id",
                process_instance_id,
            )
            .unwrap_or_default();
    }

    pub fn delete_event_subprocess_event_subscriptions_by_scope_execution_id(
        &self,
        scope_execution_id: &str,
        session: &mut DbSession,
    ) {
        let subscriptions: Vec<_> = self
            .snapshot_event_subprocess_event_subscriptions(session)
            .into_values()
            .filter(|sub| sub.scope_execution_id.as_deref() == Some(scope_execution_id))
            .map(|sub| sub.subscription_id)
            .collect();

        for subscription_id in subscriptions {
            self.delete_event_subprocess_event_subscription(&subscription_id, session);
        }
    }

    pub fn find_event_subprocess_event_subscriptions_by_process_instance_id(
        &self,
        process_instance_id: &str,
        session: &mut DbSession,
    ) -> Vec<EventSubprocessEventSubscription> {
        // Java parity: DbSqlSession.selectList throws on SQL error; an empty list means a no-row query, not a storage failure.
        match session.find_by(
            "event_subprocess_event_subscriptions",
            "process_instance_id",
            process_instance_id,
        ) {
            Ok(found) => found,
            Err(error) => {
                session.note_write_error(error);
                Vec::new()
            }
        }
    }

    pub fn find_event_subprocess_event_subscriptions_by_event_ref(
        &self,
        event_kind: &EventSubscriptionKind,
        event_ref: &str,
        session: &mut DbSession,
    ) -> Vec<EventSubprocessEventSubscription> {
        let kind_str = match event_kind {
            EventSubscriptionKind::Message => "message",
            EventSubscriptionKind::Signal => "signal",
            EventSubscriptionKind::Conditional => "conditional",
            EventSubscriptionKind::Error => "error",
            EventSubscriptionKind::Cancel => "cancel",
            EventSubscriptionKind::Compensate => "compensate",
            EventSubscriptionKind::Escalation => "escalation",
            EventSubscriptionKind::EventRegistry => "event-registry",
        };
        // Java parity: DbSqlSession.selectList throws on SQL error; an empty list means a no-row query, not a storage failure.
        match session.find_by_two(
            "event_subprocess_event_subscriptions",
            "event_kind",
            kind_str,
            "event_ref",
            event_ref,
        ) {
            Ok(found) => found,
            Err(error) => {
                session.note_write_error(error);
                Vec::new()
            }
        }
    }

    pub fn snapshot_event_subprocess_event_subscriptions(
        &self,
        session: &mut DbSession,
    ) -> HashMap<String, EventSubprocessEventSubscription> {
        {
            match session.find_all::<EventSubprocessEventSubscription>(
                "event_subprocess_event_subscriptions",
            ) {
                Ok(found) => found,
                Err(error) => {
                    session.note_write_error(error);
                    Vec::new()
                }
            }
        }
        .into_iter()
        .map(|e| (e.subscription_id.clone(), e))
        .collect()
    }

    // ── Timer Worker Node methods ──

    pub fn insert_timer_worker_node(&self, node: TimerWorkerNode, session: &mut DbSession) {
        session
            .insert_with_extra(
                "timer_worker_nodes",
                &node.node_id,
                &node,
                &[(
                    "last_heartbeat".into(),
                    Some(node.last_heartbeat.to_string()),
                )],
            )
            .unwrap_or_default();
    }

    pub fn delete_timer_worker_node(&self, node_id: &str, session: &mut DbSession) {
        let _ = session.delete("timer_worker_nodes", node_id);
    }

    pub fn find_timer_worker_node(
        &self,
        node_id: &str,
        session: &mut DbSession,
    ) -> Option<TimerWorkerNode> {
        {
            match session.find("timer_worker_nodes", node_id) {
                Ok(found) => found,
                Err(error) => {
                    session.note_write_error(error);
                    None
                }
            }
        }
    }

    pub fn snapshot_timer_worker_nodes(
        &self,
        session: &mut DbSession,
    ) -> HashMap<String, TimerWorkerNode> {
        {
            match session.find_all::<TimerWorkerNode>("timer_worker_nodes") {
                Ok(found) => found,
                Err(error) => {
                    session.note_write_error(error);
                    Vec::new()
                }
            }
        }
        .into_iter()
        .map(|n| (n.node_id.clone(), n))
        .collect()
    }

    // ── Timer Coordinator Lease methods ──

    pub fn insert_timer_coordinator_lease(
        &self,
        lease: TimerCoordinatorLease,
        session: &mut DbSession,
    ) {
        session
            .insert_with_extra(
                "timer_coordinator_leases",
                &lease.id,
                &lease,
                &[
                    ("owner_node_id".into(), Some(lease.owner_node_id.clone())),
                    ("expiry_time".into(), Some(lease.expiry_time.to_string())),
                    (
                        "fencing_token".into(),
                        Some(lease.fencing_token.to_string()),
                    ),
                ],
            )
            .unwrap_or_default();
    }

    pub fn find_timer_coordinator_lease(
        &self,
        id: &str,
        session: &mut DbSession,
    ) -> Option<TimerCoordinatorLease> {
        {
            match session.find("timer_coordinator_leases", id) {
                Ok(found) => found,
                Err(error) => {
                    session.note_write_error(error);
                    None
                }
            }
        }
    }

    pub fn acquire_coordinator_lease(
        &self,
        lease_id: &str,
        owner_node_id: &str,
        now: i64,
        timeout_ms: i64,
        session: &mut DbSession,
    ) -> Option<i64> {
        let new_expiry = now + timeout_ms;

        let current_opt =
            match session.find::<TimerCoordinatorLease>("timer_coordinator_leases", lease_id) {
                Ok(found) => found,
                Err(error) => {
                    session.note_write_error(error);
                    None
                }
            };

        if let Some(current) = current_opt {
            if current.owner_node_id == owner_node_id {
                let lease = TimerCoordinatorLease {
                    id: lease_id.to_string(),
                    owner_node_id: owner_node_id.to_string(),
                    expiry_time: new_expiry,
                    fencing_token: current.fencing_token,
                };
                let json = serde_json::to_string(&lease).unwrap_or_else(|_| "{}".to_string());
                // Renew own lease
                if session
                    .cas_update(
                        "timer_coordinator_leases",
                        lease_id,
                        &json,
                        &[
                            ("owner_node_id".into(), Some(owner_node_id.to_string())),
                            ("expiry_time".into(), Some(new_expiry.to_string())),
                            (
                                "fencing_token".into(),
                                Some(current.fencing_token.to_string()),
                            ),
                        ],
                        &[("owner_node_id".into(), Some(owner_node_id.to_string()))],
                    )
                    .unwrap_or_default()
                    > 0
                {
                    Some(current.fencing_token)
                } else {
                    None
                }
            } else {
                // Empty owner (released lease) is treated as free for takeover when
                // the stored expiry is already in the past (release writes expiry-1).
                let owner_node_opt = if current.owner_node_id.is_empty() {
                    None
                } else {
                    match session
                        .find::<TimerWorkerNode>("timer_worker_nodes", &current.owner_node_id)
                    {
                        Ok(found) => found,
                        Err(error) => {
                            session.note_write_error(error);
                            None
                        }
                    }
                };
                let can_takeover = current.expiry_time < now;
                // Early takeover only when a registered node is demonstrably stale.
                // A *missing* node is NOT proof of death: one-shot callers such as
                // `run_due_timers` never register a worker node, and treating them
                // as dead let concurrent engines steal a still-valid lease, lock a
                // job under the stolen-from owner, and leave both sides with zero
                // executions (see concurrent_timer_acquisition_test flake).
                let is_current_owner_dead = match owner_node_opt {
                    Some(node) => node.last_heartbeat < now - timeout_ms,
                    None => false,
                };

                if can_takeover || is_current_owner_dead {
                    let new_token = current.fencing_token + 1;
                    let lease = TimerCoordinatorLease {
                        id: lease_id.to_string(),
                        owner_node_id: owner_node_id.to_string(),
                        expiry_time: new_expiry,
                        fencing_token: new_token,
                    };
                    let json = serde_json::to_string(&lease).unwrap_or_else(|_| "{}".to_string());
                    if session
                        .cas_update(
                            "timer_coordinator_leases",
                            lease_id,
                            &json,
                            &[
                                ("owner_node_id".into(), Some(owner_node_id.to_string())),
                                ("expiry_time".into(), Some(new_expiry.to_string())),
                                ("fencing_token".into(), Some(new_token.to_string())),
                            ],
                            &[
                                ("owner_node_id".into(), Some(current.owner_node_id.clone())),
                                (
                                    "fencing_token".into(),
                                    Some(current.fencing_token.to_string()),
                                ),
                            ],
                        )
                        .unwrap_or_default()
                        > 0
                    {
                        Some(new_token)
                    } else {
                        None
                    }
                } else {
                    None
                }
            }
        } else {
            // No lease exists: exclusive insert. INSERT OR REPLACE would let two
            // concurrent first-writers both observe "no row", both "succeed", and
            // silently overwrite each other with the same fencing token — the
            // loser can lock a job then fail the lease check on execute while the
            // winner cannot re-acquire the locked job (total_executed = 0).
            let lease = TimerCoordinatorLease {
                id: lease_id.to_string(),
                owner_node_id: owner_node_id.to_string(),
                expiry_time: new_expiry,
                fencing_token: 1,
            };
            match session.insert_exclusive_with_extra(
                "timer_coordinator_leases",
                lease_id,
                &lease,
                &[
                    ("owner_node_id".into(), Some(owner_node_id.to_string())),
                    ("expiry_time".into(), Some(new_expiry.to_string())),
                    ("fencing_token".into(), Some("1".to_string())),
                ],
            ) {
                Ok(()) => Some(1),
                Err(crate::persistence::storage_error::StorageError::DuplicateEntity {
                    ..
                }) => None,
                Err(error) => {
                    // Unexpected insert failure: surface via panic-free None so the
                    // acquire path stays fallible without aborting the worker.
                    tracing::error!(
                        "failed to create timer coordinator lease '{}': {error}",
                        lease_id
                    );
                    None
                }
            }
        }
    }

    pub fn release_coordinator_lease(
        &self,
        lease_id: &str,
        owner_node_id: &str,
        fencing_token: i64,
        session: &mut DbSession,
    ) -> bool {
        let current_opt =
            match session.find::<TimerCoordinatorLease>("timer_coordinator_leases", lease_id) {
                Ok(found) => found,
                Err(error) => {
                    session.note_write_error(error);
                    None
                }
            };

        let Some(current) = current_opt else {
            return false;
        };

        if current.owner_node_id != owner_node_id || current.fencing_token != fencing_token {
            return false;
        }

        let released_token = current.fencing_token + 1;
        let released_lease = TimerCoordinatorLease {
            id: lease_id.to_string(),
            owner_node_id: String::new(),
            expiry_time: self.time_source.now().timestamp_millis() - 1,
            fencing_token: released_token,
        };
        let json = serde_json::to_string(&released_lease).unwrap_or_else(|_| "{}".to_string());

        session
            .cas_update(
                "timer_coordinator_leases",
                lease_id,
                &json,
                &[
                    ("owner_node_id".into(), Some("".to_string())),
                    (
                        "expiry_time".into(),
                        Some(released_lease.expiry_time.to_string()),
                    ),
                    ("fencing_token".into(), Some(released_token.to_string())),
                ],
                &[
                    ("owner_node_id".into(), Some(owner_node_id.to_string())),
                    ("fencing_token".into(), Some(fencing_token.to_string())),
                ],
            )
            .unwrap_or_default()
            > 0
    }

    // ── Control Surface API ──

    /// Get the current status of the timer coordinator
    pub fn get_timer_coordinator_status(&self, session: &mut DbSession) -> TimerCoordinatorStatus {
        let now = self.time_source.now().timestamp_millis();
        let lease_opt = self.find_timer_coordinator_lease("timer-coordinator", session);

        match lease_opt {
            Some(lease) => {
                let status = if lease.owner_node_id.is_empty() {
                    CoordinatorLeadershipStatus::NoLeader
                } else if lease.expiry_time >= now {
                    CoordinatorLeadershipStatus::Active
                } else {
                    CoordinatorLeadershipStatus::Expired
                };

                TimerCoordinatorStatus {
                    leader_node_id: lease.owner_node_id.clone(),
                    fencing_token: lease.fencing_token,
                    lease_expiry_time: lease.expiry_time,
                    status,
                }
            }
            None => TimerCoordinatorStatus {
                leader_node_id: String::new(),
                fencing_token: 0,
                lease_expiry_time: 0,
                status: CoordinatorLeadershipStatus::NoLeader,
            },
        }
    }

    /// List all timer worker nodes with their status
    pub fn list_timer_nodes(&self, session: &mut DbSession) -> Vec<TimerNodeStatus> {
        let now = self.time_source.now().timestamp_millis();
        let heartbeat_timeout_ms = 300_000; // 5 minutes

        let nodes = {
            match session.find_all::<TimerWorkerNode>("timer_worker_nodes") {
                Ok(found) => found,
                Err(error) => {
                    session.note_write_error(error);
                    Vec::new()
                }
            }
        };

        nodes
            .into_iter()
            .map(|node| {
                let status = if node.last_heartbeat >= now - heartbeat_timeout_ms {
                    NodeStatus::Active
                } else {
                    NodeStatus::Expired
                };

                TimerNodeStatus {
                    node_id: node.node_id,
                    last_heartbeat: node.last_heartbeat,
                    worker_type: node.worker_type,
                    status,
                }
            })
            .collect()
    }

    /// Force step down the current leader (admin operation)
    /// This advances the fencing token and releases the lease
    pub fn force_step_down(&self, session: &mut DbSession) -> bool {
        let lease_opt = match session
            .find::<TimerCoordinatorLease>("timer_coordinator_leases", "timer-coordinator")
        {
            Ok(found) => found,
            Err(error) => {
                session.note_write_error(error);
                None
            }
        };

        let Some(current) = lease_opt else {
            return false;
        };

        if current.owner_node_id.is_empty() {
            return true; // Already no leader
        }

        let new_token = current.fencing_token + 1;
        let released_lease = TimerCoordinatorLease {
            id: "timer-coordinator".to_string(),
            owner_node_id: String::new(),
            expiry_time: self.time_source.now().timestamp_millis() - 1,
            fencing_token: new_token,
        };
        let json = serde_json::to_string(&released_lease).unwrap_or_else(|_| "{}".to_string());

        session
            .cas_update(
                "timer_coordinator_leases",
                "timer-coordinator",
                &json,
                &[
                    ("owner_node_id".into(), Some(String::new())),
                    (
                        "expiry_time".into(),
                        Some(released_lease.expiry_time.to_string()),
                    ),
                    ("fencing_token".into(), Some(new_token.to_string())),
                ],
                &[
                    ("owner_node_id".into(), Some(current.owner_node_id.clone())),
                    (
                        "fencing_token".into(),
                        Some(current.fencing_token.to_string()),
                    ),
                ],
            )
            .unwrap_or_default()
            > 0
    }

    /// Deregister a timer node (admin operation)
    pub fn deregister_timer_node(&self, node_id: &str, session: &mut DbSession) -> bool {
        let _ = session.delete("timer_worker_nodes", node_id);
        true
    }

    /// Clean up expired timer nodes (admin operation)
    pub fn cleanup_expired_timer_nodes(&self, session: &mut DbSession) -> usize {
        let now = self.time_source.now().timestamp_millis();
        let heartbeat_timeout_ms = 300_000; // 5 minutes

        let expired_nodes: Vec<TimerWorkerNode> = {
            match session.find_all::<TimerWorkerNode>("timer_worker_nodes") {
                Ok(found) => found,
                Err(error) => {
                    session.note_write_error(error);
                    Vec::new()
                }
            }
        }
        .into_iter()
        .filter(|node| node.last_heartbeat < now - heartbeat_timeout_ms)
        .collect();

        let mut cleaned = 0;
        for node in expired_nodes {
            let _ = session.delete("timer_worker_nodes", &node.node_id);
            cleaned += 1;
        }

        cleaned
    }

    /// Insert an audit record for timer administrative actions
    pub fn insert_timer_admin_audit_record(
        &self,
        record: crate::service::audit::TimerAdminAuditRecord,
        session: &mut DbSession,
    ) {
        session
            .insert_with_extra(
                "timer_admin_audit_logs",
                &record.id,
                &record,
                &[
                    ("request_id".into(), Some(record.request_id.clone())),
                    ("timestamp".into(), Some(record.timestamp.to_string())),
                    ("tenant_id".into(), record.tenant_id.clone()),
                    ("issuer".into(), Some(record.issuer.clone())),
                    ("subject".into(), Some(record.subject.clone())),
                    ("actor".into(), Some(record.actor.clone())),
                    ("action".into(), Some(record.action.clone())),
                    ("target".into(), Some(record.target.clone())),
                    ("outcome".into(), Some(record.outcome.clone())),
                    ("profile_id".into(), record.profile_id.clone()),
                ],
            )
            .unwrap_or_default();
    }

    /// Find timer admin audit records created since the given timestamp (inclusive).
    pub fn find_timer_admin_audit_records_since(
        &self,
        timestamp: i64,
        session: &mut DbSession,
    ) -> Vec<crate::service::audit::TimerAdminAuditRecord> {
        // Java parity: DbSqlSession.selectList throws on SQL error; an empty id
        // list means a no-row query, not a storage failure.
        let ids = match session.find_ids_by_filter(
            "timer_admin_audit_logs",
            &[("timestamp".into(), FilterOp::GreaterThan(timestamp - 1))],
            "timestamp",
            true,
            None,
            None,
        ) {
            Ok(found) => found,
            Err(error) => {
                session.note_write_error(error);
                Vec::new()
            }
        };
        ids.into_iter()
            .filter_map(|id| {
                match session.find::<crate::service::audit::TimerAdminAuditRecord>(
                    "timer_admin_audit_logs",
                    &id,
                ) {
                    Ok(found) => found,
                    Err(error) => {
                        session.note_write_error(error);
                        None
                    }
                }
            })
            .collect()
    }

    // ── Token Revocation methods ──

    pub fn insert_token_revocation(
        &self,
        revocation: RuntimeTokenRevocation,
        session: &mut DbSession,
    ) {
        // Java parity: `DbSqlSession.insert` propagates a SQL failure so the enclosing
        // transaction rolls back. Discarding the result here made `revoke` report success
        // for a row that was never written, so the admin control plane answered
        // `{"success": true}` while the token stayed live. Recording the failure lets the
        // caller's `flush_and_commit` fail the transaction instead.
        if let Err(error) = session.insert_with_extra(
            "token_revocations",
            &revocation.jti,
            &revocation,
            &[
                ("issuer".into(), Some(revocation.issuer.clone())),
                ("reason".into(), Some(revocation.reason.clone())),
                ("expires_at".into(), Some(revocation.expires_at.to_string())),
                ("created_at".into(), Some(revocation.created_at.to_string())),
            ],
        ) {
            session.note_write_error(error);
        }
    }

    pub fn delete_token_revocation(&self, jti: &str, session: &mut DbSession) -> bool {
        // Java parity: DbSqlSession.selectById throws on SQL error; None means a no-row query, not a storage failure.
        let existed = match session.find::<RuntimeTokenRevocation>("token_revocations", jti) {
            Ok(found) => found,
            Err(error) => {
                session.note_write_error(error);
                None
            }
        }
        .is_some();
        if existed {
            let _ = session.delete("token_revocations", jti);
        }
        existed
    }

    pub fn find_token_revocation(
        &self,
        jti: &str,
        session: &mut DbSession,
    ) -> Option<RuntimeTokenRevocation> {
        {
            match session.find("token_revocations", jti) {
                Ok(found) => found,
                Err(error) => {
                    session.note_write_error(error);
                    None
                }
            }
        }
    }

    pub fn cleanup_expired_token_revocations(&self, session: &mut DbSession) -> usize {
        let now = self.time_source.now().timestamp_millis();
        let expired: Vec<String> = match session.find_ids_by_filter(
            "token_revocations",
            &[("expires_at".into(), FilterOp::LessThan(now + 1))],
            "id",
            true,
            None,
            None,
        ) {
            Ok(found) => found,
            Err(error) => {
                session.note_write_error(error);
                Vec::new()
            }
        };
        let count = expired.len();
        for id in expired {
            let _ = session.delete("token_revocations", &id);
        }
        count
    }

    pub fn count_active_token_revocations(&self, session: &mut DbSession) -> usize {
        let now = self.time_source.now().timestamp_millis();
        (match session.find_ids_by_filter(
            "token_revocations",
            &[("expires_at".into(), FilterOp::GreaterThan(now))],
            "id",
            true,
            None,
            None,
        ) {
            Ok(found) => found,
            Err(error) => {
                session.note_write_error(error);
                Vec::new()
            }
        })
        .len()
    }

    pub fn list_active_token_revocations(
        &self,
        session: &mut DbSession,
    ) -> Vec<RuntimeTokenRevocation> {
        let now = self.time_source.now().timestamp_millis();
        let mut entries = {
            match session.find_all::<RuntimeTokenRevocation>("token_revocations") {
                Ok(found) => found,
                Err(error) => {
                    session.note_write_error(error);
                    Vec::new()
                }
            }
        };
        entries.retain(|entry| entry.expires_at > now);
        entries.sort_by(|left, right| {
            right
                .created_at
                .cmp(&left.created_at)
                .then(left.jti.cmp(&right.jti))
        });
        entries
    }

    // ── Issuer Profile methods ──

    pub fn insert_issuer_profile(
        &self,
        profile: crate::service::issuer_profile::IssuerProfile,
        session: &mut DbSession,
    ) {
        // Java parity: DbSqlSession.flushInserts (flowable-engine-common/.../impl/db/
        // DbSqlSession.java:495, reached from flush() L375) throws on a SQL failure and
        // CommandContext.close() (interceptor/CommandContext.java:66-115) rethrows it via
        // rethrowExceptionIfNeeded() (L112-114, L136-144). Record the failure on the
        // session so the transaction fails instead of reporting a successful write.
        if let Err(error) = session.insert_with_extra(
            "timer_issuer_profiles",
            &profile.id,
            &profile,
            &[
                ("issuer".into(), Some(profile.issuer.clone())),
                ("version".into(), Some(profile.version.to_string())),
            ],
        ) {
            session.note_write_error(error);
        }
    }

    pub fn update_issuer_profile(
        &self,
        profile: crate::service::issuer_profile::IssuerProfile,
        expected_version: i64,
        session: &mut DbSession,
    ) -> Result<(), StorageError> {
        let mut updated_profile = profile.clone();
        updated_profile.version = expected_version + 1;
        let json = serde_json::to_string(&updated_profile)?;

        let affected = session.cas_update(
            "timer_issuer_profiles",
            &profile.id,
            &json,
            &[("version".into(), Some(updated_profile.version.to_string()))],
            &[("version".into(), Some(expected_version.to_string()))],
        )?;

        if affected > 0 {
            Ok(())
        } else {
            Err(StorageError::OptimisticLockConflict)
        }
    }

    pub fn delete_issuer_profile(&self, profile_id: &str, session: &mut DbSession) -> bool {
        // Java parity: DbSqlSession.flushDeletes (DbSqlSession.java:636, from flush() L377)
        // throws on a SQL failure; the error is never dropped on the floor.
        if let Err(error) = session.delete("timer_issuer_profiles", profile_id) {
            session.note_write_error(error);
        }
        true
    }

    /// Java parity: `ModelEntityManagerImpl.findById` (flowable-engine/.../persistence/
    /// entity/ModelEntityManagerImpl.java:37-40) delegates to `DbSqlSession.selectById`,
    /// whose MyBatis `selectOne` throws on a SQL error. `None` is reachable ONLY from a
    /// successful no-row query, so the storage error must propagate.
    pub fn find_issuer_profile(
        &self,
        profile_id: &str,
        session: &mut DbSession,
    ) -> Result<Option<crate::service::issuer_profile::IssuerProfile>, StorageError> {
        session.find("timer_issuer_profiles", profile_id)
    }

    /// Java parity: `MybatisModelDataManager`/`MybatisDeploymentDataManager` query paths
    /// (`getDbSqlSession().selectList(...)`) let a SQL error throw instead of degrading to
    /// an empty result set — an empty list is only valid for a successful no-row query.
    pub fn list_issuer_profiles(
        &self,
        session: &mut DbSession,
    ) -> Result<Vec<crate::service::issuer_profile::IssuerProfile>, StorageError> {
        session.find_all("timer_issuer_profiles")
    }

    // ── Identity methods ──

    pub fn insert_user(&self, user: crate::identity::entities::User, session: &mut DbSession) {
        let _ = session.insert("users", &user.id, &user);
    }

    pub fn find_user(
        &self,
        user_id: &str,
        session: &mut DbSession,
    ) -> Option<crate::identity::entities::User> {
        {
            match session.find("users", user_id) {
                Ok(found) => found,
                Err(error) => {
                    session.note_write_error(error);
                    None
                }
            }
        }
    }

    pub fn delete_user(&self, user_id: &str, session: &mut DbSession) {
        let _ = session.delete_by("user_info", "user_id", user_id);
        session
            .delete_by("user_pictures", "user_id", user_id)
            .unwrap_or_default();
        let _ = session.delete("users", user_id);
    }

    pub fn insert_user_info(
        &self,
        info: crate::identity::entities::UserInfo,
        session: &mut DbSession,
    ) {
        let id = user_info_id(&info.user_id, &info.key);
        session
            .insert_with_extra(
                "user_info",
                &id,
                &info,
                &[
                    ("user_id".into(), Some(info.user_id.clone())),
                    ("info_key".into(), Some(info.key.clone())),
                ],
            )
            .unwrap_or_default();
    }

    pub fn find_user_info(
        &self,
        user_id: &str,
        key: &str,
        session: &mut DbSession,
    ) -> Option<crate::identity::entities::UserInfo> {
        let id = user_info_id(user_id, key);
        {
            match session.find("user_info", &id) {
                Ok(found) => found,
                Err(error) => {
                    session.note_write_error(error);
                    None
                }
            }
        }
    }

    pub fn list_user_info(
        &self,
        user_id: &str,
        session: &mut DbSession,
    ) -> Result<Vec<crate::identity::entities::UserInfo>, StorageError> {
        session
            .find_raw_by("user_info", "user_id", user_id)?
            .into_iter()
            .map(|r| {
                serde_json::from_str(&r.data).map_err(|e| {
                    StorageError::Deserialization(format!("user_info for '{user_id}': {e}"))
                })
            })
            .collect()
    }

    pub fn delete_user_info(&self, user_id: &str, key: &str, session: &mut DbSession) -> bool {
        let id = user_info_id(user_id, key);
        let _ = session.delete("user_info", &id);
        true
    }

    pub fn set_user_picture(
        &self,
        picture: crate::identity::entities::UserPicture,
        session: &mut DbSession,
    ) {
        session
            .insert_blob(
                "user_pictures",
                &picture.user_id,
                &[
                    ("user_id", &picture.user_id),
                    ("mime_type", &picture.mime_type),
                ],
                "bytes",
                &picture.bytes,
            )
            .unwrap_or_default();
    }

    pub fn get_user_picture(
        &self,
        user_id: &str,
        session: &mut DbSession,
    ) -> Option<crate::identity::entities::UserPicture> {
        // Java parity: DbSqlSession.selectOne throws on SQL error; None means a no-row query, not a storage failure.
        let mime_type = match session.find_raw("user_pictures", user_id) {
            Ok(found) => found,
            Err(error) => {
                session.note_write_error(error);
                None
            }
        }
        .and_then(|r| r.extras.get("mime_type").cloned())
        .flatten();
        let bytes = match session.find_blob("user_pictures", "id", user_id, "bytes") {
            Ok(found) => found,
            Err(error) => {
                session.note_write_error(error);
                None
            }
        };

        match (mime_type, bytes) {
            (Some(mime_type), Some(bytes)) => Some(crate::identity::entities::UserPicture {
                user_id: user_id.to_string(),
                mime_type,
                bytes,
                created_at: None,
            }),
            _ => None,
        }
    }

    pub fn delete_user_picture(&self, user_id: &str, session: &mut DbSession) -> bool {
        if self.get_user_picture(user_id, session).is_none() {
            return false;
        }
        let _ = session.delete("user_pictures", user_id);
        true
    }

    pub fn insert_group(&self, group: crate::identity::entities::Group, session: &mut DbSession) {
        let _ = session.insert("groups", &group.id, &group);
    }

    pub fn find_group(
        &self,
        group_id: &str,
        session: &mut DbSession,
    ) -> Option<crate::identity::entities::Group> {
        {
            match session.find("groups", group_id) {
                Ok(found) => found,
                Err(error) => {
                    session.note_write_error(error);
                    None
                }
            }
        }
    }

    pub fn delete_group(&self, group_id: &str, session: &mut DbSession) {
        let _ = session.delete("groups", group_id);
    }

    pub fn create_membership(&self, user_id: String, group_id: String, session: &mut DbSession) {
        session
            .insert_with_extra(
                "memberships",
                &format!("{}:{}", user_id, group_id),
                &crate::identity::entities::Membership {
                    user_id: user_id.clone(),
                    group_id: group_id.clone(),
                },
                &[
                    ("user_id".into(), Some(user_id)),
                    ("group_id".into(), Some(group_id)),
                ],
            )
            .unwrap_or_default();
    }

    pub fn delete_membership(&self, user_id: &str, group_id: &str, session: &mut DbSession) {
        session
            .delete("memberships", &format!("{}:{}", user_id, group_id))
            .unwrap_or_default();
    }

    pub fn get_groups_by_user(
        &self,
        user_id: &str,
        session: &mut DbSession,
    ) -> Vec<crate::identity::entities::Group> {
        // Java parity: DbSqlSession.selectList throws on SQL error; an empty list
        // means a no-row query, not a storage failure.
        (match session.find_raw_by("memberships", "user_id", user_id) {
            Ok(found) => found,
            Err(error) => {
                session.note_write_error(error);
                Vec::new()
            }
        })
        .into_iter()
        .filter_map(|m| {
            let gid = m.extras.get("group_id").cloned().flatten()?;
            {
                match session.find("groups", &gid) {
                    Ok(found) => found,
                    Err(error) => {
                        session.note_write_error(error);
                        None
                    }
                }
            }
        })
        .collect()
    }

    pub fn get_users_by_group(
        &self,
        group_id: &str,
        session: &mut DbSession,
    ) -> Vec<crate::identity::entities::User> {
        // Java parity: DbSqlSession.selectList throws on SQL error; an empty list
        // means a no-row query, not a storage failure.
        (match session.find_raw_by("memberships", "group_id", group_id) {
            Ok(found) => found,
            Err(error) => {
                session.note_write_error(error);
                Vec::new()
            }
        })
        .into_iter()
        .filter_map(|m| {
            let uid = m.extras.get("user_id").cloned().flatten()?;
            {
                match session.find("users", &uid) {
                    Ok(found) => found,
                    Err(error) => {
                        session.note_write_error(error);
                        None
                    }
                }
            }
        })
        .collect()
    }

    pub fn membership_exists(
        &self,
        user_id: &str,
        group_id: &str,
        session: &mut DbSession,
    ) -> bool {
        // Java parity: DbSqlSession.selectList throws on SQL error; an empty list
        // means a no-row query, not a storage failure.
        !(match session.find_raw_by_two("memberships", "user_id", user_id, "group_id", group_id) {
            Ok(found) => found,
            Err(error) => {
                session.note_write_error(error);
                Vec::new()
            }
        })
        .is_empty()
    }

    pub fn list_memberships(
        &self,
        session: &mut DbSession,
    ) -> Vec<crate::identity::entities::Membership> {
        // Java parity: DbSqlSession.selectList throws on SQL error; an empty list means a no-row query, not a storage failure.
        (match session.find_raw_all("memberships") {
            Ok(found) => found,
            Err(error) => {
                session.note_write_error(error);
                Vec::new()
            }
        })
        .into_iter()
        .filter_map(|r| {
            let user_id = r.extras.get("user_id").cloned().flatten()?;
            let group_id = r.extras.get("group_id").cloned().flatten()?;
            Some(crate::identity::entities::Membership { user_id, group_id })
        })
        .collect()
    }

    pub fn list_users(&self, session: &mut DbSession) -> Vec<crate::identity::entities::User> {
        {
            match session.find_all("users") {
                Ok(found) => found,
                Err(error) => {
                    session.note_write_error(error);
                    Vec::new()
                }
            }
        }
    }

    pub fn list_groups(&self, session: &mut DbSession) -> Vec<crate::identity::entities::Group> {
        {
            match session.find_all("groups") {
                Ok(found) => found,
                Err(error) => {
                    session.note_write_error(error);
                    Vec::new()
                }
            }
        }
    }

    // ── Privilege methods ──

    pub fn insert_privilege(
        &self,
        privilege: crate::identity::entities::Privilege,
        session: &mut DbSession,
    ) {
        session
            .insert_with_extra(
                "privileges",
                &privilege.id,
                &privilege,
                &[("name".into(), Some(privilege.name.clone()))],
            )
            .unwrap_or_default();
    }

    pub fn find_privilege(
        &self,
        privilege_id: &str,
        session: &mut DbSession,
    ) -> Option<crate::identity::entities::Privilege> {
        {
            match session.find("privileges", privilege_id) {
                Ok(found) => found,
                Err(error) => {
                    session.note_write_error(error);
                    None
                }
            }
        }
    }

    pub fn delete_privilege(&self, privilege_id: &str, session: &mut DbSession) {
        let _ = session.delete("privileges", privilege_id);
        session
            .delete_by("privilege_mappings", "privilege_id", privilege_id)
            .unwrap_or_default();
    }

    pub fn list_privileges(
        &self,
        session: &mut DbSession,
    ) -> Vec<crate::identity::entities::Privilege> {
        {
            match session.find_all("privileges") {
                Ok(found) => found,
                Err(error) => {
                    session.note_write_error(error);
                    Vec::new()
                }
            }
        }
    }

    pub fn insert_privilege_mapping(
        &self,
        mapping: crate::identity::entities::PrivilegeMapping,
        session: &mut DbSession,
    ) {
        session
            .insert_with_extra(
                "privilege_mappings",
                &mapping.id,
                &mapping,
                &[
                    ("privilege_id".into(), Some(mapping.privilege_id.clone())),
                    ("user_id".into(), mapping.user_id.clone()),
                    ("group_id".into(), mapping.group_id.clone()),
                ],
            )
            .unwrap_or_default();
    }

    pub fn delete_privilege_mapping(&self, mapping_id: &str, session: &mut DbSession) {
        let _ = session.delete("privilege_mappings", mapping_id);
    }

    pub fn find_privilege_mappings_by_privilege(
        &self,
        privilege_id: &str,
        session: &mut DbSession,
    ) -> Vec<crate::identity::entities::PrivilegeMapping> {
        // Java parity: DbSqlSession.selectList throws on SQL error; an empty list means a no-row query, not a storage failure.
        match session.find_by("privilege_mappings", "privilege_id", privilege_id) {
            Ok(found) => found,
            Err(error) => {
                session.note_write_error(error);
                Vec::new()
            }
        }
    }

    pub fn find_privilege_mappings_by_user(
        &self,
        user_id: &str,
        session: &mut DbSession,
    ) -> Vec<crate::identity::entities::PrivilegeMapping> {
        // Java parity: DbSqlSession.selectList throws on SQL error; an empty list means a no-row query, not a storage failure.
        match session.find_by("privilege_mappings", "user_id", user_id) {
            Ok(found) => found,
            Err(error) => {
                session.note_write_error(error);
                Vec::new()
            }
        }
    }

    pub fn find_privilege_mappings_by_group(
        &self,
        group_id: &str,
        session: &mut DbSession,
    ) -> Vec<crate::identity::entities::PrivilegeMapping> {
        // Java parity: DbSqlSession.selectList throws on SQL error; an empty list means a no-row query, not a storage failure.
        match session.find_by("privilege_mappings", "group_id", group_id) {
            Ok(found) => found,
            Err(error) => {
                session.note_write_error(error);
                Vec::new()
            }
        }
    }

    // ── Token methods ──

    pub fn insert_token(&self, token: crate::identity::entities::Token, session: &mut DbSession) {
        session
            .insert_with_extra(
                "tokens",
                &token.id,
                &token,
                &[("token_value".into(), Some(token.token_value.clone()))],
            )
            .unwrap_or_default();
    }

    pub fn find_token(
        &self,
        token_id: &str,
        session: &mut DbSession,
    ) -> Option<crate::identity::entities::Token> {
        {
            match session.find("tokens", token_id) {
                Ok(found) => found,
                Err(error) => {
                    session.note_write_error(error);
                    None
                }
            }
        }
    }

    pub fn find_token_by_value(
        &self,
        token_value: &str,
        session: &mut DbSession,
    ) -> Option<crate::identity::entities::Token> {
        // Java parity: DbSqlSession.selectList throws on SQL error; an empty list means a no-row query, not a storage failure.
        (match session.find_by::<crate::identity::entities::Token>(
            "tokens",
            "token_value",
            token_value,
        ) {
            Ok(found) => found,
            Err(error) => {
                session.note_write_error(error);
                Vec::new()
            }
        })
        .into_iter()
        .next()
    }

    pub fn delete_token(&self, token_id: &str, session: &mut DbSession) {
        let _ = session.delete("tokens", token_id);
    }

    pub fn list_tokens(&self, session: &mut DbSession) -> Vec<crate::identity::entities::Token> {
        {
            match session.find_all("tokens") {
                Ok(found) => found,
                Err(error) => {
                    session.note_write_error(error);
                    Vec::new()
                }
            }
        }
    }

    // ── History methods ──

    pub fn insert_historic_process_instance(
        &self,
        instance: &crate::history::historic_entities::HistoricProcessInstance,
        session: &mut DbSession,
    ) {
        // Task 10: dual-write start_time_ms/end_time_ms columns for SQL pushdown in cleanup_batch.
        let start_time_ms = instance.start_time.timestamp_millis();
        let end_time_ms = instance.end_time.map(|t| t.timestamp_millis());
        session
            .insert_with_extra(
                "historic_process_instances",
                &instance.id,
                instance,
                &[
                    (
                        "process_definition_id".into(),
                        Some(instance.process_definition_id.clone()),
                    ),
                    ("start_time_ms".into(), Some(start_time_ms.to_string())),
                    ("end_time_ms".into(), end_time_ms.map(|v| v.to_string())),
                ],
            )
            .unwrap_or_default();
    }

    pub fn update_historic_process_instance(
        &self,
        instance: &crate::history::historic_entities::HistoricProcessInstance,
        session: &mut DbSession,
    ) {
        self.insert_historic_process_instance(instance, session);
    }

    pub fn delete_historic_process_instance_cascade(
        &self,
        process_instance_id: &str,
        session: &mut DbSession,
    ) {
        // P86a: the assignee/owner historic identity links carry only a task_id
        // (Java `HistoricTaskServiceImpl.createHistoricIdentityLink:265-273`
        // sets no processInstanceId), so the by-process delete below cannot see
        // them. Java reaches them through the per-task cascade:
        // `DefaultHistoryManager.recordProcessInstanceDeleted:143` →
        // `TaskHelper.deleteHistoricTaskInstancesByProcessInstanceId:612-620` →
        // `deleteHistoricTask` → `deleteHistoricIdentityLinksByTaskId`.
        // Collect the ids before the historic task rows are removed.
        let historic_task_ids: Vec<String> =
            match session.find_by::<crate::history::historic_entities::HistoricTaskInstance>(
                "historic_task_instances",
                "process_instance_id",
                process_instance_id,
            ) {
                Ok(found) => found,
                Err(error) => {
                    session.note_write_error(error);
                    Vec::new()
                }
            }
            .into_iter()
            .map(|instance| instance.id)
            .collect();
        session
            .delete("historic_process_instances", process_instance_id)
            .unwrap_or_default();
        session
            .delete_by(
                "historic_activity_instances",
                "process_instance_id",
                process_instance_id,
            )
            .unwrap_or_default();
        session
            .delete_by(
                "historic_task_instances",
                "process_instance_id",
                process_instance_id,
            )
            .unwrap_or_default();
        session
            .delete_by(
                "historic_variable_instances",
                "process_instance_id",
                process_instance_id,
            )
            .unwrap_or_default();
        session
            .delete_by(
                "historic_details",
                "process_instance_id",
                process_instance_id,
            )
            .unwrap_or_default();
        session
            .delete_by(
                "historic_comments",
                "process_instance_id",
                process_instance_id,
            )
            .unwrap_or_default();
        session
            .delete_by(
                "historic_task_log_entries",
                "process_instance_id",
                process_instance_id,
            )
            .unwrap_or_default();
        // P77: cascade historic identity links (Java
        // DefaultHistoryManager delete path + HistoricIdentityLinkService
        // .deleteHistoricIdentityLinksByProcessInstanceId).
        session
            .delete_by(
                "historic_identity_links",
                "process_instance_id",
                process_instance_id,
            )
            .unwrap_or_default();
        for task_id in &historic_task_ids {
            session
                .delete_by("historic_identity_links", "task_id", task_id.as_str())
                .unwrap_or_default();
        }
        session
            .delete_by("identity_links", "process_instance_id", process_instance_id)
            .unwrap_or_default();
    }

    pub fn get_historic_process_instance(
        &self,
        id: &str,
        session: &mut DbSession,
    ) -> Option<crate::history::historic_entities::HistoricProcessInstance> {
        {
            match session.find("historic_process_instances", id) {
                Ok(found) => found,
                Err(error) => {
                    session.note_write_error(error);
                    None
                }
            }
        }
    }

    pub fn insert_historic_activity_instance(
        &self,
        instance: crate::history::historic_entities::HistoricActivityInstance,
        session: &mut DbSession,
    ) {
        // Project delete_reason for indexed/SQL filtering while the JSON blob
        // remains the source of truth (P65-style legacy-compatible column).
        session
            .insert_with_extra(
                "historic_activity_instances",
                &instance.id,
                &instance,
                &[
                    (
                        "process_instance_id".into(),
                        Some(instance.process_instance_id.clone()),
                    ),
                    ("execution_id".into(), Some(instance.execution_id.clone())),
                    ("activity_id".into(), Some(instance.activity_id.clone())),
                    ("delete_reason".into(), instance.delete_reason.clone()),
                ],
            )
            .unwrap_or_default();
    }

    pub fn update_historic_activity_instance(
        &self,
        instance: crate::history::historic_entities::HistoricActivityInstance,
        session: &mut DbSession,
    ) {
        self.insert_historic_activity_instance(instance, session);
    }

    pub fn get_historic_activity_instance_by_execution_and_activity(
        &self,
        execution_id: &str,
        activity_id: &str,
        session: &mut DbSession,
    ) -> Option<crate::history::historic_entities::HistoricActivityInstance> {
        // Java parity: DbSqlSession.selectList throws on SQL error; an empty list means a no-row query, not a storage failure.
        (match session.find_by_two(
            "historic_activity_instances",
            "execution_id",
            execution_id,
            "activity_id",
            activity_id,
        ) {
            Ok(found) => found,
            Err(error) => {
                session.note_write_error(error);
                Vec::new()
            }
        })
        .into_iter()
        .find(
            |inst: &crate::history::historic_entities::HistoricActivityInstance| {
                inst.end_time.is_none()
            },
        )
    }

    pub fn delete_historic_activity_instance(&self, id: &str, session: &mut DbSession) {
        let _ = session.delete("historic_activity_instances", id);
    }

    pub fn delete_open_historic_activity_instance(
        &self,
        execution_id: &str,
        activity_id: &str,
        session: &mut DbSession,
    ) {
        if let Some(inst) = self.get_historic_activity_instance_by_execution_and_activity(
            execution_id,
            activity_id,
            session,
        ) {
            self.delete_historic_activity_instance(&inst.id, session);
        }
    }

    pub fn find_historic_activity_instances_by_process_instance_id(
        &self,
        process_instance_id: &str,
        session: &mut DbSession,
    ) -> Vec<crate::history::historic_entities::HistoricActivityInstance> {
        // Java parity: DbSqlSession.selectList throws on SQL error; an empty list means a no-row query, not a storage failure.
        match session.find_by(
            "historic_activity_instances",
            "process_instance_id",
            process_instance_id,
        ) {
            Ok(found) => found,
            Err(error) => {
                session.note_write_error(error);
                Vec::new()
            }
        }
    }

    pub fn insert_historic_task_instance(
        &self,
        instance: crate::history::historic_entities::HistoricTaskInstance,
        session: &mut DbSession,
    ) {
        session
            .insert_with_typed_extra(
                "historic_task_instances",
                &instance.id,
                &instance,
                &[
                    (
                        "process_instance_id".into(),
                        DbValue::Text(instance.process_instance_id.clone()),
                    ),
                    (
                        "process_definition_id".into(),
                        DbValue::from(instance.process_definition_id.clone()),
                    ),
                    (
                        "task_definition_key".into(),
                        DbValue::from(instance.task_definition_key.clone()),
                    ),
                    ("assignee".into(), DbValue::from(instance.assignee.clone())),
                    ("owner".into(), DbValue::from(instance.owner.clone())),
                    (
                        "claim_time".into(),
                        DbValue::from(
                            instance
                                .claim_time
                                .map(|claim_time| claim_time.timestamp_millis()),
                        ),
                    ),
                    (
                        "tenant_id".into(),
                        DbValue::from(instance.tenant_id.clone()),
                    ),
                    ("category".into(), DbValue::from(instance.category.clone())),
                    ("form_key".into(), DbValue::from(instance.form_key.clone())),
                    (
                        "parent_task_id".into(),
                        DbValue::from(instance.parent_task_id.clone()),
                    ),
                    ("priority".into(), DbValue::from(instance.priority)),
                    (
                        "due_date".into(),
                        DbValue::from(
                            instance
                                .due_date
                                .map(|due_date| due_date.timestamp_millis()),
                        ),
                    ),
                ],
            )
            .unwrap_or_default();
    }

    pub fn update_historic_task_instance(
        &self,
        instance: crate::history::historic_entities::HistoricTaskInstance,
        session: &mut DbSession,
    ) {
        self.insert_historic_task_instance(instance, session);
    }

    pub fn get_historic_task_instance(
        &self,
        id: &str,
        session: &mut DbSession,
    ) -> Option<crate::history::historic_entities::HistoricTaskInstance> {
        {
            match session.find("historic_task_instances", id) {
                Ok(found) => found,
                Err(error) => {
                    session.note_write_error(error);
                    None
                }
            }
        }
    }

    pub fn delete_historic_task_instance_cascade(&self, task_id: &str, session: &mut DbSession) {
        let _ = session.delete("historic_task_instances", task_id);
        session
            .delete_by("historic_comments", "task_id", task_id)
            .unwrap_or_default();
        session
            .delete_by("historic_task_events", "task_id", task_id)
            .unwrap_or_default();
        session
            .delete_by("historic_task_log_entries", "task_id", task_id)
            .unwrap_or_default();
        // P77: cascade historic identity links by task
        // (Java HistoricIdentityLinkService.deleteHistoricIdentityLinksByTaskId).
        session
            .delete_by("historic_identity_links", "task_id", task_id)
            .unwrap_or_default();
        session
            .delete_by("identity_links", "task_id", task_id)
            .unwrap_or_default();
    }

    pub fn insert_historic_variable_instance(
        &self,
        instance: &crate::history::historic_entities::HistoricVariableInstance,
        session: &mut DbSession,
    ) {
        session
            .insert_with_extra(
                "historic_variable_instances",
                &instance.id,
                instance,
                &[
                    (
                        "process_instance_id".into(),
                        Some(instance.process_instance_id.clone()),
                    ),
                    ("variable_name".into(), Some(instance.name.clone())),
                ],
            )
            .unwrap_or_default();
    }

    pub fn get_historic_variable_instance(
        &self,
        id: &str,
        session: &mut DbSession,
    ) -> Option<crate::history::historic_entities::HistoricVariableInstance> {
        // Java parity: DbSqlSession.selectById throws on SQL error; None means a no-row query, not a storage failure.
        match session.find("historic_variable_instances", id) {
            Ok(found) => found,
            Err(error) => {
                session.note_write_error(error);
                None
            }
        }
    }

    pub fn delete_historic_variable_instance(&self, id: &str, session: &mut DbSession) {
        let _ = session.delete("historic_variable_instances", id);
    }

    pub fn insert_historic_detail(
        &self,
        detail: crate::history::historic_entities::HistoricDetail,
        session: &mut DbSession,
    ) {
        session
            .insert_with_extra(
                "historic_details",
                &detail.id,
                &detail,
                &[
                    (
                        "process_instance_id".into(),
                        Some(detail.process_instance_id.clone()),
                    ),
                    ("execution_id".into(), detail.execution_id.clone()),
                    ("task_id".into(), detail.task_id.clone()),
                    (
                        "time".into(),
                        Some(detail.time.timestamp_millis().to_string()),
                    ),
                    ("detail_type".into(), Some(detail.detail_type.clone())),
                    ("variable_name".into(), detail.variable_name.clone()),
                    ("property_id".into(), detail.property_id.clone()),
                ],
            )
            .unwrap_or_default();
    }

    pub fn get_historic_detail(
        &self,
        id: &str,
        session: &mut DbSession,
    ) -> Option<crate::history::historic_entities::HistoricDetail> {
        {
            match session.find("historic_details", id) {
                Ok(found) => found,
                Err(error) => {
                    session.note_write_error(error);
                    None
                }
            }
        }
    }

    pub fn list_historic_details(
        &self,
        session: &mut DbSession,
    ) -> Vec<crate::history::historic_entities::HistoricDetail> {
        let mut details = {
            match session
                .find_all::<crate::history::historic_entities::HistoricDetail>("historic_details")
            {
                Ok(found) => found,
                Err(error) => {
                    session.note_write_error(error);
                    Vec::new()
                }
            }
        };
        details.sort_by(|left, right| left.time.cmp(&right.time).then(left.id.cmp(&right.id)));
        details
    }

    pub fn insert_historic_audit_log(
        &self,
        instance: crate::history::historic_entities::HistoricAuditLog,
        session: &mut DbSession,
    ) {
        session
            .insert("historic_audit_logs", &instance.id, &instance)
            .unwrap_or_default();
    }

    pub fn get_historic_audit_log(
        &self,
        id: &str,
        session: &mut DbSession,
    ) -> Option<crate::history::historic_entities::HistoricAuditLog> {
        {
            match session.find("historic_audit_logs", id) {
                Ok(found) => found,
                Err(error) => {
                    session.note_write_error(error);
                    None
                }
            }
        }
    }

    // ── Compensation methods ──

    pub fn insert_compensation_subscription(
        &self,
        sub: crate::runtime::compensation::CompensationSubscription,
        session: &mut DbSession,
    ) {
        let mut sub = sub;

        if sub.subscription_order <= 0 {
            let existing_order = session
                .max(
                    "compensation_subscriptions",
                    "subscription_order",
                    &[("id".into(), sub.id.clone())],
                )
                .unwrap_or_default()
                .unwrap_or(0);

            sub.subscription_order = if existing_order > 0 {
                existing_order
            } else {
                let max_order = session
                    .max("compensation_subscriptions", "subscription_order", &[])
                    .unwrap_or_default()
                    .unwrap_or(0);
                max_order + 1
            };
        }

        session
            .insert_with_extra(
                "compensation_subscriptions",
                &sub.id,
                &sub,
                &[
                    (
                        "process_instance_id".into(),
                        Some(sub.process_instance_id.clone()),
                    ),
                    ("execution_id".into(), Some(sub.execution_id.clone())),
                    ("activity_id".into(), Some(sub.activity_id.clone())),
                    (
                        "compensation_activity_id".into(),
                        Some(sub.compensation_activity_id.clone()),
                    ),
                    (
                        "subscription_order".into(),
                        Some(sub.subscription_order.to_string()),
                    ),
                ],
            )
            .unwrap_or_default();
    }

    pub fn find_compensation_subscriptions_by_process_instance_id(
        &self,
        pi_id: &str,
        session: &mut DbSession,
    ) -> Vec<crate::runtime::compensation::CompensationSubscription> {
        // Java parity: DbSqlSession.selectList throws on SQL error; an empty list means a no-row query, not a storage failure.
        match session.find_by("compensation_subscriptions", "process_instance_id", pi_id) {
            Ok(found) => found,
            Err(error) => {
                session.note_write_error(error);
                Vec::new()
            }
        }
    }

    pub fn find_compensation_subscriptions_by_process_instance_id_newest_first(
        &self,
        pi_id: &str,
        session: &mut DbSession,
    ) -> Vec<crate::runtime::compensation::CompensationSubscription> {
        // Java parity: DbSqlSession.selectList throws on SQL error; an empty list means a no-row query, not a storage failure.
        let mut results = match session
            .find_by::<crate::runtime::compensation::CompensationSubscription>(
                "compensation_subscriptions",
                "process_instance_id",
                pi_id,
            ) {
            Ok(found) => found,
            Err(error) => {
                session.note_write_error(error);
                Vec::new()
            }
        };
        results.sort_by(|a, b| {
            b.subscription_order
                .cmp(&a.subscription_order)
                .then_with(|| b.id.cmp(&a.id))
        });
        results
    }

    pub fn delete_compensation_subscriptions_by_process_instance_id(
        &self,
        pi_id: &str,
        session: &mut DbSession,
    ) {
        session
            .delete_by("compensation_subscriptions", "process_instance_id", pi_id)
            .unwrap_or_default();
    }

    pub fn delete_compensation_subscription(&self, subscription_id: &str, session: &mut DbSession) {
        session
            .delete("compensation_subscriptions", subscription_id)
            .unwrap_or_default();
    }

    // ── Cleanup methods ──

    pub fn list_historic_process_instances(
        &self,
        session: &mut DbSession,
    ) -> Vec<crate::history::historic_entities::HistoricProcessInstance> {
        {
            match session.find_all::<crate::history::historic_entities::HistoricProcessInstance>(
                "historic_process_instances",
            ) {
                Ok(found) => found,
                Err(error) => {
                    session.note_write_error(error);
                    Vec::new()
                }
            }
        }
    }

    /// Task 10 / P133: SQL pushdown for cleanup_batch — returns IDs matching
    /// finished-before cutoff + type filter with LIMIT/OFFSET pagination.
    ///
    /// Cutoff is **end_time** (not start_time), aligning with engine auto-cleanup
    /// (`history_cleaning.rs` / Java `DefaultHistoryCleaningManager.java:36`
    /// finishedBefore). Both `"all"` and `"completed"` only select finished
    /// instances (`end_time_ms IS NOT NULL AND end_time_ms < before`) so
    /// long-running processes are never deleted.
    ///
    /// Returns None if columns missing (very old data), in which case caller
    /// falls back to legacy in-memory path.
    pub fn find_historic_process_instance_ids_for_cleanup(
        &self,
        before_millis: i64,
        cleanup_type: &str,
        batch_size: usize,
        batch_number: usize,
        session: &mut DbSession,
    ) -> Option<Vec<String>> {
        match cleanup_type {
            "completed" | "all" | "" => {}
            _ => return None, // terminated needs JSON delete_reason; use legacy path
        }
        let offset = batch_number * batch_size;
        // P133: end_time cutoff — Java DefaultHistoryCleaningManager.java:36 finishedBefore
        let filters = vec![
            ("end_time_ms".into(), FilterOp::IsNotNull),
            ("end_time_ms".into(), FilterOp::LessThan(before_millis)),
        ];
        // A SELECT failure here is NOT the documented "very old schema" `None`
        // path: record it in the session's sticky slot (and still report `None`
        // so the caller keeps its legacy fallback) instead of letting
        // `unwrap_or_default()` claim "no historic instances matched", which would
        // silently skip history cleanup for this batch.
        // Java parity: DbSqlSession.selectList propagates the SQL error
        // (flowable-engine-common/.../impl/db/DbSqlSession.java:282-299).
        let ids = match session.find_ids_by_filter(
            "historic_process_instances",
            &filters,
            "end_time_ms",
            true,
            Some(batch_size),
            Some(offset),
        ) {
            Ok(found) => found,
            Err(error) => {
                session.note_write_error(error);
                return None;
            }
        };
        Some(ids)
    }

    pub fn insert_cleanup_log(
        &self,
        log: crate::history::historic_entities::CleanupLog,
        session: &mut DbSession,
    ) {
        let _ = session.insert("cleanup_logs", &log.id, &log);
    }

    pub fn list_cleanup_logs(
        &self,
        session: &mut DbSession,
    ) -> Vec<crate::history::historic_entities::CleanupLog> {
        {
            match session.find_all::<crate::history::historic_entities::CleanupLog>("cleanup_logs")
            {
                Ok(found) => found,
                Err(error) => {
                    session.note_write_error(error);
                    Vec::new()
                }
            }
        }
    }

    pub fn set_cleanup_strategy_config(
        &self,
        config: &crate::history::historic_entities::CleanupStrategyConfig,
        session: &mut DbSession,
    ) {
        session
            .insert("cleanup_strategy_configs", "default", config)
            .unwrap_or_default();
    }

    pub fn get_cleanup_strategy_config(
        &self,
        session: &mut DbSession,
    ) -> Option<crate::history::historic_entities::CleanupStrategyConfig> {
        {
            match session.find("cleanup_strategy_configs", "default") {
                Ok(found) => found,
                Err(error) => {
                    session.note_write_error(error);
                    None
                }
            }
        }
    }
}

fn user_info_id(user_id: &str, key: &str) -> String {
    format!("{user_id}\u{1f}{key}")
}
