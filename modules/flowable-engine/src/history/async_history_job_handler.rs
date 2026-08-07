use crate::error::FlowableError;
use crate::history::historic_entities::HistoricTaskUpdate;
use crate::interceptor::command_context::CommandContext;
use crate::persistence::runtime_store::{RuntimeJobType, RuntimeTimerJobState};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// A single history record that was deferred during command execution.
/// When `AsyncHistoryConfiguration::enabled` is true, `record_*` methods
/// buffer these payloads instead of writing directly to the DB. After the
/// command's agenda drains, the batch is serialised into a single
/// `RuntimeTimerJobState` with `job_state = "history"`.
///
/// Java equivalent: each record_* call in `AsyncHistoryManager` produces
/// one entry in a JSON array that gets stored as the job's payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum HistoryJobPayload {
    ProcessInstanceStart {
        process_instance_id: String,
        process_definition_id: String,
        business_key: Option<String>,
        #[serde(default)]
        start_user_id: Option<String>,
        start_time: DateTime<Utc>,
    },
    ProcessInstanceEnd {
        process_instance_id: String,
        delete_reason: Option<String>,
        end_time: DateTime<Utc>,
    },
    ActivityStart {
        id: String,
        activity_id: String,
        activity_name: Option<String>,
        activity_type: String,
        process_instance_id: String,
        execution_id: String,
        start_time: DateTime<Utc>,
    },
    ActivityEnd {
        execution_id: String,
        activity_id: String,
        end_time: DateTime<Utc>,
        /// Java activity-end delete reason. `#[serde(default)]` keeps older
        /// history-job payloads (pre-P71) deserializable.
        #[serde(default)]
        delete_reason: Option<String>,
    },
    TaskCreated {
        id: String,
        process_instance_id: String,
        process_definition_id: Option<String>,
        execution_id: String,
        task_definition_key: Option<String>,
        name: Option<String>,
        #[serde(default)]
        description: Option<String>,
        assignee: Option<String>,
        owner: Option<String>,
        #[serde(default)]
        claim_time: Option<DateTime<Utc>>,
        #[serde(default)]
        tenant_id: Option<String>,
        #[serde(default)]
        category: Option<String>,
        #[serde(default)]
        form_key: Option<String>,
        #[serde(default)]
        parent_task_id: Option<String>,
        priority: Option<i32>,
        due_date: Option<DateTime<Utc>>,
        start_time: DateTime<Utc>,
    },
    TaskUpdated {
        update: HistoricTaskUpdate,
    },
    TaskEnd {
        task_id: String,
        delete_reason: Option<String>,
        end_time: DateTime<Utc>,
    },
    /// Claim/unclaim assignment event (`AddUserLink` / `DeleteUserLink`).
    /// P97: previously written synchronously and ungated from task_service.
    TaskEvent {
        task_id: String,
        action: String,
        message: Vec<String>,
        user_id: Option<String>,
        time: DateTime<Utc>,
    },
    VariableCreated {
        id: String,
        name: String,
        variable_type: String,
        value: serde_json::Value,
        process_instance_id: String,
        execution_id: Option<String>,
        task_id: Option<String>,
        create_time: DateTime<Utc>,
    },
    VariableUpdated {
        id: String,
        value: serde_json::Value,
        last_updated_time: DateTime<Utc>,
    },
    FormProperty {
        process_instance_id: String,
        task_id: Option<String>,
        property_id: String,
        property_value: serde_json::Value,
        time: DateTime<Utc>,
    },
    AuditEvent {
        id: String,
        event_type: String,
        process_instance_id: Option<String>,
        process_definition_id: Option<String>,
        details: Option<String>,
        timestamp: DateTime<Utc>,
    },
}

/// A batch of history payloads that will be serialised into a single
/// `RuntimeTimerJobState.time_duration` field.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryJobBatch {
    pub operations: Vec<HistoryJobPayload>,
}

/// Trait for handling async history jobs.
///
/// Java equivalent: `HistoryJobHandler` interface. When async history is enabled,
/// history record methods enqueue `RuntimeTimerJobState` entries with
/// `job_state = "history"` instead of writing synchronously. The async executor
/// acquires these jobs and dispatches them to a registered `HistoryJobHandler`.
///
/// This is the framework/skeleton – the actual async-ification of history
/// record methods is deferred to a follow-up effort.
pub trait HistoryJobHandler: Send + Sync {
    /// Execute the history job, replaying or persisting the deferred history data.
    fn execute(
        &self,
        job: &RuntimeTimerJobState,
        command_context: &mut CommandContext,
    ) -> Result<(), FlowableError>;

    /// Returns the handler type identifier (e.g. `"async-history"`).
    fn get_type(&self) -> &'static str;
}

/// Default no-op handler that simply deletes the history job.
/// This preserves the current `ManagementService::execute_history_job` behavior
/// when no custom handler is registered.
pub struct DefaultHistoryJobHandler;

impl HistoryJobHandler for DefaultHistoryJobHandler {
    fn execute(
        &self,
        job: &RuntimeTimerJobState,
        command_context: &mut CommandContext,
    ) -> Result<(), FlowableError> {
        let (store, session) = command_context.store_and_session();
        store.delete_timer_job_state(&job.timer_job_id, session);
        Ok(())
    }

    fn get_type(&self) -> &'static str {
        "default-history"
    }
}

/// Helper to create a history job entry from a serialized payload.
/// Callers that want to defer a history write can use this to enqueue a job
/// that will later be picked up by the async executor.
pub fn create_history_job(
    payload: String,
    handler_type: &str,
    command_context: &mut CommandContext,
) -> String {
    use uuid::Uuid;
    let job_id = Uuid::new_v4().to_string();
    let now = command_context
        .runtime_store
        .time_source()
        .now()
        .timestamp_millis();

    let retries = command_context
        .config
        .async_history
        .number_of_retries
        .max(0);
    let (store, session) = command_context.store_and_session();
    store.insert_timer_job_state_with_type(
        &RuntimeTimerJobState {
            timer_job_id: job_id.clone(),
            process_instance_id: String::new(),
            execution_id: String::new(),
            activity_id: handler_type.to_string(),
            job_state: Some("history".to_string()),
            is_boundary: false,
            attached_activity_id: None,
            cancel_activity: false,
            time_duration: Some(payload.clone()),
            time_date: None,
            time_cycle: None,
            end_date: None,
            due_time: Some(now),
            lock_owner: None,
            lock_time: None,
            lock_expiration_time: None,
            retries: Some(retries),
            error_message: None,
            error_details: None,
            category: None,
            create_time: Some(now),
            handler_type: Some(handler_type.to_string()),
            job_handler_configuration: None,
            advanced_job_handler_configuration: Some(payload),
            ..Default::default()
        },
        Some(&RuntimeJobType::History),
        session,
    );

    job_id
}

/// Type alias for a shared history job handler.
pub type SharedHistoryJobHandler = Arc<dyn HistoryJobHandler>;

/// Replay handler for deferred async history jobs.
///
/// When the async executor acquires a history job (`job_state = "history"`),
/// this handler deserialises the `HistoryJobBatch` from `time_duration` and
/// replays each operation in order against the runtime store (synchronously).
///
/// # Design decisions (from plan)
/// - **D3:** `next_historic_task_log_number()` is called at replay time, not buffer time.
/// - **D5:** operations within a batch are replayed in order; missing records cause retry.
/// - Read-modify-write operations (end/update) return `Err` when the target record
///   is not found, triggering the retry/deadletter mechanism (8J).
pub struct AsyncHistoryJobHandler;

impl HistoryJobHandler for AsyncHistoryJobHandler {
    fn execute(
        &self,
        job: &RuntimeTimerJobState,
        command_context: &mut CommandContext,
    ) -> Result<(), FlowableError> {
        let json = job.time_duration.as_deref().unwrap_or("");
        if json.is_empty() {
            let (store, session) = command_context.store_and_session();
            store.delete_timer_job_state(&job.timer_job_id, session);
            return Ok(());
        }
        let batch: HistoryJobBatch = match serde_json::from_str(json) {
            Ok(batch) => batch,
            Err(e) => {
                tracing::warn!("Failed to deserialize HistoryJobBatch: {e}");
                handle_failure(command_context, job, &format!("Invalid payload: {e}"));
                return Err(FlowableError::ExecutionError(format!(
                    "Invalid history job payload: {e}"
                )));
            }
        };

        for operation in &batch.operations {
            if let Err(e) = replay_operation(operation, command_context) {
                tracing::warn!("History job replay failed for operation: {e:?}");
                handle_failure(command_context, job, &format!("{e:?}"));
                return Err(e);
            }
        }

        let (store, session) = command_context.store_and_session();
        store.delete_timer_job_state(&job.timer_job_id, session);
        Ok(())
    }

    fn get_type(&self) -> &'static str {
        "async-history"
    }
}

/// 8J: Handle a failed history job replay — decrement retries and either
/// requeue with exponential backoff or move to deadletter.
fn handle_failure(
    command_context: &mut CommandContext,
    job: &RuntimeTimerJobState,
    error_message: &str,
) {
    let retries = job.retries.unwrap_or(0);
    let mut updated = job.clone();
    updated.lock_owner = None;
    updated.lock_time = None;
    updated.lock_expiration_time = None;
    updated.error_message = Some(error_message.to_string());

    if retries <= 1 {
        // Retries exhausted → deadletter
        updated.retries = Some(0);
        updated.job_state = Some("deadletter".to_string());
    } else {
        // Retry with exponential backoff: 5s, 10s, 15s based on attempt number
        let attempt = 3 - retries; // retries starts at 3, attempt 0 on first failure
        let backoff_ms = ((attempt + 1) as i64) * 5_000;
        let now = command_context
            .runtime_store
            .time_source()
            .now()
            .timestamp_millis();
        updated.retries = Some(retries - 1);
        updated.due_time = Some(now + backoff_ms);
    }
    let (store, session) = command_context.store_and_session();
    store.insert_timer_job_state(&updated, session);
}

fn replay_operation(op: &HistoryJobPayload, ctx: &mut CommandContext) -> Result<(), FlowableError> {
    match op {
        HistoryJobPayload::ProcessInstanceStart {
            process_instance_id,
            process_definition_id,
            business_key,
            start_user_id,
            start_time,
        } => {
            let (store, session) = ctx.store_and_session();
            store.insert_historic_process_instance(
                &crate::history::historic_entities::HistoricProcessInstance {
                    id: process_instance_id.clone(),
                    process_definition_id: process_definition_id.clone(),
                    business_key: business_key.clone(),
                    start_time: *start_time,
                    end_time: None,
                    duration_ms: None,
                    start_user_id: start_user_id.clone(),
                    delete_reason: None,
                },
                session,
            );
            // P119: fire HISTORIC_PROCESS_INSTANCE_CREATED on async replay
            // (Java DefaultHistoryManager.java:120-126 fires at record time).
            ctx.add_post_agenda_event(
                crate::engine::event_dispatcher::historic_process_instance_created_event(
                    process_instance_id,
                    process_definition_id,
                ),
            );
            Ok(())
        }

        HistoryJobPayload::ProcessInstanceEnd {
            process_instance_id,
            delete_reason,
            end_time,
        } => {
            let (store, session) = ctx.store_and_session();
            if let Some(mut instance) =
                store.get_historic_process_instance(process_instance_id, session)
            {
                let duration = end_time.signed_duration_since(instance.start_time);
                instance.end_time = Some(*end_time);
                instance.duration_ms = Some(duration.num_milliseconds());
                instance.delete_reason = delete_reason.clone();
                let pd_id = instance.process_definition_id.clone();
                store.update_historic_process_instance(&instance, session);
                // P119: HISTORIC_PROCESS_INSTANCE_ENDED on async replay.
                ctx.add_post_agenda_event(
                    crate::engine::event_dispatcher::historic_process_instance_ended_event(
                        process_instance_id,
                        Some(&pd_id),
                    ),
                );
                Ok(())
            } else {
                Err(FlowableError::NotFound(format!(
                    "HistoricProcessInstance '{}' not found for replay",
                    process_instance_id
                )))
            }
        }

        HistoryJobPayload::ActivityStart {
            id,
            activity_id,
            activity_name,
            activity_type,
            process_instance_id,
            execution_id,
            start_time,
        } => {
            let (store, session) = ctx.store_and_session();
            store.insert_historic_activity_instance(
                crate::history::historic_entities::HistoricActivityInstance {
                    id: id.clone(),
                    activity_id: activity_id.clone(),
                    activity_name: activity_name.clone(),
                    activity_type: activity_type.clone(),
                    process_instance_id: process_instance_id.clone(),
                    execution_id: execution_id.clone(),
                    start_time: *start_time,
                    end_time: None,
                    duration_ms: None,
                    assignee: None,
                    delete_reason: None,
                },
                session,
            );
            // P119: HISTORIC_ACTIVITY_INSTANCE_CREATED on async replay.
            ctx.add_post_agenda_event(
                crate::engine::event_dispatcher::historic_activity_instance_created_event(
                    id,
                    activity_id,
                    process_instance_id,
                    execution_id,
                ),
            );
            Ok(())
        }

        HistoryJobPayload::ActivityEnd {
            execution_id,
            activity_id,
            end_time,
            delete_reason,
        } => {
            let (store, session) = ctx.store_and_session();
            if let Some(mut instance) = store
                .get_historic_activity_instance_by_execution_and_activity(
                    execution_id,
                    activity_id,
                    session,
                )
            {
                let duration = end_time.signed_duration_since(instance.start_time);
                instance.end_time = Some(*end_time);
                instance.duration_ms = Some(duration.num_milliseconds());
                if let Some(reason) = delete_reason {
                    instance.delete_reason = Some(reason.clone());
                }
                let historic_id = instance.id.clone();
                let process_instance_id = instance.process_instance_id.clone();
                store.update_historic_activity_instance(instance, session);
                // P119: HISTORIC_ACTIVITY_INSTANCE_ENDED on async replay.
                ctx.add_post_agenda_event(
                    crate::engine::event_dispatcher::historic_activity_instance_ended_event(
                        &historic_id,
                        activity_id,
                        &process_instance_id,
                        execution_id,
                    ),
                );
                Ok(())
            } else {
                Err(FlowableError::NotFound(format!(
                    "HistoricActivityInstance exec={execution_id} act={activity_id} not found for replay"
                )))
            }
        }

        HistoryJobPayload::TaskCreated {
            id,
            process_instance_id,
            process_definition_id,
            execution_id,
            task_definition_key,
            name,
            description,
            assignee,
            owner,
            claim_time,
            tenant_id,
            category,
            form_key,
            parent_task_id,
            priority,
            due_date,
            start_time,
        } => {
            {
                let (store, session) = ctx.store_and_session();
                store.insert_historic_task_instance(
                    crate::history::historic_entities::HistoricTaskInstance {
                        id: id.clone(),
                        process_instance_id: process_instance_id.clone(),
                        process_definition_id: process_definition_id.clone(),
                        execution_id: execution_id.clone(),
                        task_definition_key: task_definition_key.clone(),
                        name: name.clone(),
                        description: description.clone(),
                        assignee: assignee.clone(),
                        owner: owner.clone(),
                        claim_time: *claim_time,
                        tenant_id: tenant_id.clone(),
                        category: category.clone(),
                        form_key: form_key.clone(),
                        parent_task_id: parent_task_id.clone(),
                        priority: *priority,
                        due_date: *due_date,
                        start_time: *start_time,
                        end_time: None,
                        duration_ms: None,
                        delete_reason: None,
                    },
                    session,
                );

                // D3: next_historic_task_log_number at replay time
                let process_instance = store.find_process_instance(process_instance_id, session);
                let log_number = store.next_historic_task_log_number(session);
                let log_entry = crate::history::historic_entities::HistoricTaskLogEntry {
                    id: uuid::Uuid::new_v4().to_string(),
                    log_number,
                    log_type: "USER_TASK_CREATED".to_string(),
                    task_id: id.clone(),
                    timestamp: *start_time,
                    user_id: None,
                    data: Some(format!("Task {id} created")),
                    execution_id: Some(execution_id.clone()),
                    process_instance_id: Some(process_instance_id.clone()),
                    process_definition_id: process_instance
                        .as_ref()
                        .map(|pi| pi.process_definition_id.clone()),
                    scope_id: None,
                    scope_definition_id: None,
                    sub_scope_id: None,
                    scope_type: None,
                    tenant_id: process_instance.and_then(|pi| pi.tenant_id),
                };
                store.insert_historic_task_log_entry(log_entry, session);

                store.insert_historic_task_event(
                    crate::history::historic_entities::HistoricTaskEvent {
                        id: uuid::Uuid::new_v4().to_string(),
                        task_id: id.clone(),
                        action: "userTaskCreated".to_string(),
                        message: vec![name.clone().unwrap_or_default()],
                        user_id: None,
                        time: *start_time,
                    },
                    session,
                );
            }

            // P90a — initial assignee/owner accumulating historic IL, symmetric
            // with sync `history_manager.rs:469-474`. Payload already carries
            // resolved assignee/owner (`history_manager.rs:417-418`); buffer
            // side intentionally does not write IL (would duplicate on replay).
            // Java: `HistoricTaskServiceImpl.createHistoricIdentityLink:265-273`
            // (OSS has no AsyncHistoryManager — end-state parity only).
            // P97: skip for standalone tasks (empty process_instance_id),
            // mirroring the sync gate (P90b pin).
            if !process_instance_id.is_empty() {
                if let Some(a) = assignee.as_deref() {
                    ctx.history_manager.record_task_assignment_identity_link(
                        id,
                        "assignee",
                        Some(a),
                        &mut ctx.session,
                    );
                }
                if let Some(o) = owner.as_deref() {
                    ctx.history_manager.record_task_assignment_identity_link(
                        id,
                        "owner",
                        Some(o),
                        &mut ctx.session,
                    );
                }
            }
            Ok(())
        }

        HistoryJobPayload::TaskUpdated { update } => {
            // P90a — Java `HistoricTaskServiceImpl.recordTaskInfoChange:142-152`:
            // compare assignee/owner against the *historic* row before
            // overwriting (sync path: `history_manager.rs:521-522`), then
            // append IL for each changed side (null userId on unclaim, no
            // null-check — `:150-151` / `createHistoricIdentityLink:265-273`).
            // Missing historic row remains Err → retry (not silent skip).
            let (assignee_changed, owner_changed) = {
                let (store, session) = ctx.store_and_session();
                if let Some(mut instance) = store.get_historic_task_instance(&update.id, session) {
                    let assignee_changed = instance.assignee != update.assignee;
                    let owner_changed = instance.owner != update.owner;
                    update.apply_to(&mut instance);
                    store.update_historic_task_instance(instance, session);
                    (assignee_changed, owner_changed)
                } else {
                    return Err(FlowableError::NotFound(format!(
                        "HistoricTaskInstance '{}' not found for replay",
                        update.id
                    )));
                }
            };
            if assignee_changed {
                ctx.history_manager.record_task_assignment_identity_link(
                    &update.id,
                    "assignee",
                    update.assignee.as_deref(),
                    &mut ctx.session,
                );
            }
            if owner_changed {
                ctx.history_manager.record_task_assignment_identity_link(
                    &update.id,
                    "owner",
                    update.owner.as_deref(),
                    &mut ctx.session,
                );
            }
            Ok(())
        }

        HistoryJobPayload::TaskEnd {
            task_id,
            delete_reason,
            end_time,
        } => {
            let (store, session) = ctx.store_and_session();
            if let Some(mut instance) = store.get_historic_task_instance(task_id, session) {
                let duration = end_time.signed_duration_since(instance.start_time);
                instance.end_time = Some(*end_time);
                instance.duration_ms = Some(duration.num_milliseconds());
                instance.delete_reason = delete_reason.clone();
                store.update_historic_task_instance(instance, session);

                if let Some(task) = store.get_historic_task_instance(task_id, session) {
                    let process_instance =
                        store.find_process_instance(&task.process_instance_id, session);
                    let log_number = store.next_historic_task_log_number(session);
                    let log_entry = crate::history::historic_entities::HistoricTaskLogEntry {
                        id: uuid::Uuid::new_v4().to_string(),
                        log_number,
                        log_type: "USER_TASK_COMPLETED".to_string(),
                        task_id: task_id.clone(),
                        timestamp: *end_time,
                        user_id: None,
                        data: Some(format!("Task {task_id} completed")),
                        execution_id: Some(task.execution_id.clone()),
                        process_instance_id: Some(task.process_instance_id.clone()),
                        process_definition_id: process_instance
                            .as_ref()
                            .map(|pi| pi.process_definition_id.clone()),
                        scope_id: None,
                        scope_definition_id: None,
                        sub_scope_id: None,
                        scope_type: None,
                        tenant_id: process_instance.and_then(|pi| pi.tenant_id),
                    };
                    store.insert_historic_task_log_entry(log_entry, session);

                    store.insert_historic_task_event(
                        crate::history::historic_entities::HistoricTaskEvent {
                            id: uuid::Uuid::new_v4().to_string(),
                            task_id: task_id.clone(),
                            action: "userTaskCompleted".to_string(),
                            message: vec![task.name.unwrap_or_default()],
                            user_id: None,
                            time: *end_time,
                        },
                        session,
                    );
                }
                Ok(())
            } else {
                Err(FlowableError::NotFound(format!(
                    "HistoricTaskInstance '{task_id}' not found for replay"
                )))
            }
        }

        HistoryJobPayload::TaskEvent {
            task_id,
            action,
            message,
            user_id,
            time,
        } => {
            let (store, session) = ctx.store_and_session();
            store.insert_historic_task_event(
                crate::history::historic_entities::HistoricTaskEvent {
                    id: uuid::Uuid::new_v4().to_string(),
                    task_id: task_id.clone(),
                    action: action.clone(),
                    message: message.clone(),
                    user_id: user_id.clone(),
                    time: *time,
                },
                session,
            );
            Ok(())
        }

        HistoryJobPayload::VariableCreated {
            id,
            name,
            variable_type,
            value,
            process_instance_id,
            execution_id,
            task_id,
            create_time,
        } => {
            // FULL-only historic detail (DefaultHistoryManager:347-348).
            // Gate uses engine config; VariableCreated is only enqueued when
            // ACTIVITY+ was effective at write time.
            let write_detail = ctx
                .config
                .history_level
                .is_at_least(crate::service::config::HistoryLevel::Full);
            let (store, session) = ctx.store_and_session();
            store.insert_historic_variable_instance(
                &crate::history::historic_entities::HistoricVariableInstance {
                    id: id.clone(),
                    process_instance_id: process_instance_id.clone(),
                    execution_id: execution_id.clone(),
                    task_id: task_id.clone(),
                    name: name.clone(),
                    variable_type: variable_type.clone(),
                    value: value.clone(),
                    create_time: *create_time,
                    last_updated_time: *create_time,
                },
                session,
            );
            if write_detail {
                store.insert_historic_detail(
                    crate::history::historic_entities::HistoricDetail {
                        id: uuid::Uuid::new_v4().to_string(),
                        process_instance_id: process_instance_id.clone(),
                        execution_id: execution_id.clone(),
                        activity_instance_id: None,
                        task_id: task_id.clone(),
                        time: *create_time,
                        detail_type: "variableUpdate".to_string(),
                        revision: Some(0),
                        variable_name: Some(name.clone()),
                        variable_type: Some(variable_type.clone()),
                        value: Some(value.clone()),
                        property_id: None,
                        property_value: None,
                    },
                    session,
                );
            }
            Ok(())
        }

        HistoryJobPayload::VariableUpdated {
            id,
            value,
            last_updated_time,
        } => {
            let write_detail = ctx
                .config
                .history_level
                .is_at_least(crate::service::config::HistoryLevel::Full);
            let (store, session) = ctx.store_and_session();
            if let Some(mut instance) = store.get_historic_variable_instance(id, session) {
                instance.value = value.clone();
                instance.last_updated_time = *last_updated_time;
                if write_detail {
                    store.insert_historic_detail(
                        crate::history::historic_entities::HistoricDetail {
                            id: uuid::Uuid::new_v4().to_string(),
                            process_instance_id: instance.process_instance_id.clone(),
                            execution_id: instance.execution_id.clone(),
                            activity_instance_id: None,
                            task_id: instance.task_id.clone(),
                            time: *last_updated_time,
                            detail_type: "variableUpdate".to_string(),
                            revision: Some(1),
                            variable_name: Some(instance.name.clone()),
                            variable_type: Some(instance.variable_type.clone()),
                            value: Some(value.clone()),
                            property_id: None,
                            property_value: None,
                        },
                        session,
                    );
                }
                store.insert_historic_variable_instance(&instance, session);
                Ok(())
            } else {
                Err(FlowableError::NotFound(format!(
                    "HistoricVariableInstance '{id}' not found for replay"
                )))
            }
        }

        HistoryJobPayload::FormProperty {
            process_instance_id,
            task_id,
            property_id,
            property_value,
            time,
        } => {
            let (store, session) = ctx.store_and_session();
            store.insert_historic_detail(
                crate::history::historic_entities::HistoricDetail {
                    id: uuid::Uuid::new_v4().to_string(),
                    process_instance_id: process_instance_id.clone(),
                    execution_id: None,
                    activity_instance_id: None,
                    task_id: task_id.clone(),
                    time: *time,
                    detail_type: "formProperty".to_string(),
                    revision: None,
                    variable_name: None,
                    variable_type: None,
                    value: None,
                    property_id: Some(property_id.clone()),
                    property_value: Some(property_value.clone()),
                },
                session,
            );
            Ok(())
        }

        HistoryJobPayload::AuditEvent {
            id,
            event_type,
            process_instance_id,
            process_definition_id,
            details,
            timestamp,
        } => {
            let (store, session) = ctx.store_and_session();
            store.insert_historic_audit_log(
                crate::history::historic_entities::HistoricAuditLog {
                    id: id.clone(),
                    event_type: event_type.clone(),
                    process_instance_id: process_instance_id.clone(),
                    process_definition_id: process_definition_id.clone(),
                    details: details.clone(),
                    timestamp: *timestamp,
                },
                session,
            );
            Ok(())
        }
    }
}
