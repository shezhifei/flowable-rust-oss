use crate::engine::event_dispatcher::EngineEvent;
use crate::history::async_history_job_handler::HistoryJobPayload;
use crate::history::historic_entities::*;
use crate::persistence::db_session::DbSession;
use crate::persistence::runtime_store::{RuntimeJobType, RuntimeStore, RuntimeTimerJobState};
use crate::repository::process_definition::ProcessDefinition;
use crate::service::config::HistoryLevel;
use crate::task::Task;
use chrono::{DateTime, Utc};
use std::cell::RefCell;

#[derive(Debug, Clone)]
pub struct CleanupResult {
    pub deleted_count: usize,
    pub batch_number: usize,
    pub has_more: bool,
    pub duration_ms: u64,
}

pub struct HistoryManager {
    runtime_store: RuntimeStore,
    async_history_enabled: bool,
    /// Engine-level history (Java `ProcessEngineConfiguration.historyLevel`).
    history_level: HistoryLevel,
    /// Java `enableProcessDefinitionHistoryLevel` (default false).
    enable_process_definition_history_level: bool,
    /// Java `asyncHistoryExecutorNumberOfRetries` for newly created history jobs.
    async_history_number_of_retries: i32,
    buffer: RefCell<Vec<HistoryJobPayload>>,
    pending_jobs: RefCell<Vec<String>>,
    /// P119: HISTORIC_* engine events buffered during `record_*` (sync path).
    /// Drained into the command's post-agenda event stream so listeners see
    /// them without requiring `CommandContext` on every history call.
    pending_events: RefCell<Vec<EngineEvent>>,
}

impl HistoryManager {
    pub fn new(runtime_store: RuntimeStore, async_history_enabled: bool) -> Self {
        Self {
            runtime_store,
            async_history_enabled,
            // Java ProcessEngineConfiguration.history default = "audit"
            // (ProcessEngineConfiguration.java:88).
            history_level: HistoryLevel::Audit,
            enable_process_definition_history_level: false,
            async_history_number_of_retries: 10,
            buffer: RefCell::new(Vec::new()),
            pending_jobs: RefCell::new(Vec::new()),
            pending_events: RefCell::new(Vec::new()),
        }
    }

    /// Drain historic engine events queued by sync `record_*` methods.
    pub fn take_pending_events(&self) -> Vec<EngineEvent> {
        self.pending_events.borrow_mut().drain(..).collect()
    }

    fn queue_event(&self, event: EngineEvent) {
        self.pending_events.borrow_mut().push(event);
    }

    pub fn with_history_level(mut self, level: HistoryLevel) -> Self {
        self.history_level = level;
        self
    }

    pub fn with_enable_process_definition_history_level(mut self, enable: bool) -> Self {
        self.enable_process_definition_history_level = enable;
        self
    }

    pub fn with_async_history_number_of_retries(mut self, retries: i32) -> Self {
        self.async_history_number_of_retries = retries.max(0);
        self
    }

    /// Engine-level only (no process-definition override).
    /// Java `DefaultHistoryConfigurationSettings.isHistoryEnabled:54-57`.
    fn history_disabled(&self) -> bool {
        self.history_level == HistoryLevel::None
    }

    /// Java `HistoryManager.isHistoryEnabled()` / `AttachmentEntityManagerImpl.checkHistoryEnabled`.
    pub fn is_history_enabled(&self) -> bool {
        !self.history_disabled()
    }

    /// Resolve effective history level for a process definition.
    ///
    /// Java `DefaultHistoryConfigurationSettings.isHistoryLevelAtLeast:118-141`:
    /// when `enableProcessDefinitionHistoryLevel` is on and a definition id is
    /// present, the definition's `flowable:historyLevel` **replaces** the engine
    /// level (not max/min). Illegal keys are ignored
    /// (`getProcessDefinitionHistoryLevel:75-77`) and fall back to engine level.
    fn effective_level(
        &self,
        process_definition_id: Option<&str>,
        session: &mut DbSession,
    ) -> HistoryLevel {
        if self.enable_process_definition_history_level {
            if let Some(pd_id) = process_definition_id {
                if let Some(level) = self.process_definition_history_level(pd_id, session) {
                    return level;
                }
            }
        }
        self.history_level
    }

    fn process_definition_history_level(
        &self,
        process_definition_id: &str,
        session: &mut DbSession,
    ) -> Option<HistoryLevel> {
        let pd: ProcessDefinition = session
            .find("process_definitions", process_definition_id)
            .ok()
            .flatten()?;
        let key = pd.history_level.as_deref()?.trim();
        if key.is_empty() {
            return None;
        }
        // Illegal values swallowed (DefaultHistoryConfigurationSettings:75-77).
        HistoryLevel::parse(key).ok()
    }

    fn process_definition_id_for_instance(
        &self,
        process_instance_id: &str,
        session: &mut DbSession,
    ) -> Option<String> {
        self.runtime_store
            .find_process_instance(process_instance_id, session)
            .map(|pi| pi.process_definition_id)
            .or_else(|| {
                self.runtime_store
                    .get_historic_process_instance(process_instance_id, session)
                    .map(|h| h.process_definition_id)
            })
    }

    /// Java `isHistoryLevelAtLeast(level, processDefinitionId)`.
    pub fn is_history_level_at_least(
        &self,
        required: HistoryLevel,
        process_definition_id: Option<&str>,
        session: &mut DbSession,
    ) -> bool {
        self.effective_level(process_definition_id, session)
            .is_at_least(required)
    }

    /// Java `isHistoryEnabled(processDefinitionId)`.
    pub fn is_history_enabled_for_definition(
        &self,
        process_definition_id: Option<&str>,
        session: &mut DbSession,
    ) -> bool {
        self.effective_level(process_definition_id, session) != HistoryLevel::None
    }

    /// Java `isHistoryEnabledForProcessInstance` — INSTANCE+
    /// (`DefaultHistoryConfigurationSettings.java:145-147`).
    fn is_history_enabled_for_process_instance(
        &self,
        process_definition_id: Option<&str>,
        session: &mut DbSession,
    ) -> bool {
        self.is_history_level_at_least(HistoryLevel::Instance, process_definition_id, session)
    }

    /// Java `isHistoryEnabledForActivity` — ACTIVITY+
    /// (`DefaultHistoryConfigurationSettings.java:155-196`; includeInHistory
    /// flow-element override is intentionally out of scope for P112).
    fn is_history_enabled_for_activity(
        &self,
        process_definition_id: Option<&str>,
        session: &mut DbSession,
    ) -> bool {
        self.is_history_level_at_least(HistoryLevel::Activity, process_definition_id, session)
    }

    /// Java `hasTaskHistoryLevel` (`DefaultHistoryConfigurationSettings.java:258-268`):
    /// exact TASK **or** AUDIT+ (so ACTIVITY alone does **not** record tasks).
    fn is_history_enabled_for_user_task(
        &self,
        process_definition_id: Option<&str>,
        session: &mut DbSession,
    ) -> bool {
        let level = self.effective_level(process_definition_id, session);
        level == HistoryLevel::Task || level.is_at_least(HistoryLevel::Audit)
    }

    /// Java `isHistoryEnabledForVariableInstance` — ACTIVITY+
    /// (`DefaultHistoryConfigurationSettings.java:281-283`).
    fn is_history_enabled_for_variable(
        &self,
        process_definition_id: Option<&str>,
        session: &mut DbSession,
    ) -> bool {
        self.is_history_level_at_least(HistoryLevel::Activity, process_definition_id, session)
    }

    /// FULL-only historic detail variable updates
    /// (`DefaultHistoryManager.recordHistoricDetailVariableCreate:347-348`).
    fn is_history_enabled_for_full_detail(
        &self,
        process_definition_id: Option<&str>,
        session: &mut DbSession,
    ) -> bool {
        self.is_history_level_at_least(HistoryLevel::Full, process_definition_id, session)
    }

    fn record_full_extras(
        &self,
        process_definition_id: Option<&str>,
        session: &mut DbSession,
    ) -> bool {
        self.is_history_enabled_for_full_detail(process_definition_id, session)
    }

    /// Java `isHistoryEnabledForIdentityLink` — AUDIT+
    /// (`DefaultHistoryConfigurationSettings.java:291-294`).
    fn is_history_enabled_for_identity_link(
        &self,
        process_definition_id: Option<&str>,
        session: &mut DbSession,
    ) -> bool {
        self.is_history_level_at_least(HistoryLevel::Audit, process_definition_id, session)
    }

    /// Java `DefaultHistoryManager.recordIdentityLinkCreated:396-410`.
    /// Skips process-definition-only links (no taskId and no processInstanceId).
    /// Always synchronous (Java does not enqueue async history for IL create).
    pub fn record_identity_link_created(
        &self,
        link: &crate::identity::entities::IdentityLink,
        session: &mut DbSession,
    ) {
        let pd_id = link
            .process_instance_id
            .as_deref()
            .and_then(|pi| self.process_definition_id_for_instance(pi, session));
        if !self.is_history_enabled_for_identity_link(pd_id.as_deref(), session) {
            return;
        }
        // Java: only when processInstanceId != null || taskId != null
        if link.process_instance_id.is_none() && link.task_id.is_none() {
            return;
        }
        let historic =
            crate::history::historic_entities::HistoricIdentityLink::from_runtime(link);
        self.runtime_store
            .insert_historic_identity_link(&historic, session);
    }

    /// Java `DefaultHistoryManager.recordIdentityLinkDeleted:414-417` —
    /// deletes the historic row with the same id when AUDIT history is on.
    pub fn record_identity_link_deleted(&self, link_id: &str, session: &mut DbSession) {
        // No processDefinitionId available on delete-by-id path; use engine level.
        // Java resolves via the live IdentityLinkEntity (DefaultHistoryManager:414-417).
        if !self.is_history_enabled_for_identity_link(None, session) {
            return;
        }
        self.runtime_store
            .delete_historic_identity_link(link_id, session);
    }

    /// Java `HistoricTaskServiceImpl.createHistoricIdentityLink:265-273`.
    ///
    /// P86a: the assignee/owner trail is **accumulating**, not mirrored. Every
    /// change inserts a fresh row with a new id and never deletes the previous
    /// one — the opposite of the participant/candidate handling in
    /// `record_identity_link_created` / `record_identity_link_deleted` (P77).
    ///
    /// Java sets only taskId/type/userId/createTime (`:268-271`) — notably no
    /// processInstanceId and no groupId, so these rows are reachable by task id
    /// only and are cleaned up through the per-task cascade.
    ///
    /// `user_id` is `None` when the assignee/owner is being cleared (unclaim /
    /// `deleteUserIdentityLink`); Java writes that null row too, since
    /// `createHistoricIdentityLink` applies no null check.
    ///
    /// `pub(crate)` so async history replay (`async_history_job_handler`) can
    /// emit the same accumulating rows after replaying TaskCreated /
    /// TaskUpdated (P90a). Buffer side stays untouched — writing IL there
    /// would duplicate on replay.
    pub(crate) fn record_task_assignment_identity_link(
        &self,
        task_id: &str,
        link_type: &str,
        user_id: Option<&str>,
        session: &mut DbSession,
    ) {
        // Called only from task history paths that already passed the task gate;
        // still enforce AUDIT+ identity-link rule (DefaultHistoryConfigurationSettings:291-294).
        if !self.is_history_enabled_for_identity_link(None, session) {
            return;
        }
        let historic = HistoricIdentityLink {
            id: uuid::Uuid::new_v4().to_string(),
            link_type: link_type.to_string(),
            user_id: user_id.map(|value| value.to_string()),
            group_id: None,
            task_id: Some(task_id.to_string()),
            process_instance_id: None,
            scope_id: None,
            sub_scope_id: None,
            scope_type: None,
            scope_definition_id: None,
            create_time: Some(Utc::now()),
        };
        self.runtime_store
            .insert_historic_identity_link(&historic, session);
    }

    /// P97 — claim/unclaim assignment events (`AddUserLink`/`DeleteUserLink`),
    /// gated like the rest of the HistoryManager. Previously task_service wrote
    /// these synchronously with no history_disabled check and no async buffer,
    /// so `history=None` still produced rows and async mode wrote out of order.
    pub fn record_task_assignment_event(
        &self,
        task_id: &str,
        action: &str,
        assignee: &str,
        session: &mut DbSession,
    ) {
        // Java AbstractHistoryManager.createIdentityLinkComment:93 — AUDIT+.
        if !self.is_history_level_at_least(HistoryLevel::Audit, None, session) {
            return;
        }
        let time = Utc::now();
        if self.async_history_enabled {
            self.buffer
                .borrow_mut()
                .push(HistoryJobPayload::TaskEvent {
                    task_id: task_id.to_string(),
                    action: action.to_string(),
                    message: vec![assignee.to_string(), "assignee".to_string()],
                    user_id: Some(assignee.to_string()),
                    time,
                });
            return;
        }
        self.runtime_store.insert_historic_task_event(
            crate::history::historic_entities::HistoricTaskEvent {
                id: uuid::Uuid::new_v4().to_string(),
                task_id: task_id.to_string(),
                action: action.to_string(),
                message: vec![assignee.to_string(), "assignee".to_string()],
                user_id: Some(assignee.to_string()),
                time,
            },
            session,
        );
    }

    pub fn flush_history(&self, session: &mut DbSession) {
        if !self.async_history_enabled {
            return;
        }
        let payloads: Vec<HistoryJobPayload> = self.buffer.borrow_mut().drain(..).collect();
        if payloads.is_empty() {
            return;
        }
        let batch = crate::history::async_history_job_handler::HistoryJobBatch {
            operations: payloads,
        };
        let json = serde_json::to_string(&batch).unwrap_or_default();
        if json.is_empty() {
            return;
        }
        let job_id = uuid::Uuid::new_v4().to_string();
        let now = self.runtime_store.time_source().now().timestamp_millis();
        // Java HistoryJobEntity stores the batch payload in advanced config
        // (byte array) and exposes it via getHistoryJobHistoryJson; the
        // inline jobHandlerConfiguration is typically empty for async history.
        self.runtime_store.insert_timer_job_state_with_type(
            &RuntimeTimerJobState {
                timer_job_id: job_id.clone(),
                process_instance_id: String::new(),
                execution_id: String::new(),
                activity_id: "async-history".to_string(),
                job_state: Some("history".to_string()),
                is_boundary: false,
                attached_activity_id: None,
                cancel_activity: false,
                time_duration: Some(json.clone()),
                time_date: None,
                time_cycle: None,
                end_date: None,
                calendar_name: None,
                due_time: Some(now),
                lock_owner: None,
                lock_time: None,
                lock_expiration_time: None,
                retries: Some(self.async_history_number_of_retries),
                error_message: None,
                error_details: None,
                category: None,
                create_time: Some(now),
                correlation_id: None,
                handler_type: Some(
                    crate::persistence::runtime_store::job_handler_types::ASYNC_HISTORY.to_string(),
                ),
                job_handler_configuration: None,
                advanced_job_handler_configuration: Some(json),
                custom_values: None,
                scope_type: None,
                scope_id: None,
                sub_scope_id: None,
                scope_definition_id: None,
                tenant_id: None,
                process_definition_id: None,
                element_name: None,
                // Java HistoryJobEntity is not a JobEntity: the exclusive PI
                // scope lock never applies to async history jobs (P48).
                exclusive: false,
            },
            Some(&RuntimeJobType::History),
            session,
        );
        self.pending_jobs.borrow_mut().push(job_id);
    }

    pub fn take_pending_jobs(&self) -> Vec<String> {
        self.pending_jobs.borrow_mut().drain(..).collect()
    }

    pub fn record_process_instance_start(
        &self,
        process_instance_id: &str,
        process_definition_id: &str,
        business_key: Option<&str>,
        start_user_id: Option<&str>,
        session: &mut DbSession,
    ) {
        // Java DefaultHistoryManager.recordProcessInstanceStart:113-114 /
        // isHistoryEnabledForProcessInstance → INSTANCE+
        // (DefaultHistoryConfigurationSettings.java:145-147).
        if !self.is_history_enabled_for_process_instance(Some(process_definition_id), session) {
            return;
        }
        if self.async_history_enabled {
            self.buffer
                .borrow_mut()
                .push(HistoryJobPayload::ProcessInstanceStart {
                    process_instance_id: process_instance_id.to_string(),
                    process_definition_id: process_definition_id.to_string(),
                    business_key: business_key.map(|s| s.to_string()),
                    start_user_id: start_user_id.map(str::to_string),
                    start_time: Utc::now(),
                });
            return;
        }
        let instance = HistoricProcessInstance {
            id: process_instance_id.to_string(),
            process_definition_id: process_definition_id.to_string(),
            business_key: business_key.map(|s| s.to_string()),
            start_time: Utc::now(),
            end_time: None,
            duration_ms: None,
            start_user_id: start_user_id.map(str::to_string),
            delete_reason: None,
        };
        self.runtime_store
            .insert_historic_process_instance(&instance, session);
        // P119: HISTORIC_PROCESS_INSTANCE_CREATED —
        // Java DefaultHistoryManager.java:120-126.
        self.queue_event(
            crate::engine::event_dispatcher::historic_process_instance_created_event(
                process_instance_id,
                process_definition_id,
            ),
        );
    }

    pub fn record_process_instance_end(
        &self,
        process_instance_id: &str,
        delete_reason: Option<&str>,
        session: &mut DbSession,
    ) {
        // Java DefaultHistoryManager.recordProcessInstanceEnd:78-79 — INSTANCE+.
        let pd_id = self.process_definition_id_for_instance(process_instance_id, session);
        if !self.is_history_enabled_for_process_instance(pd_id.as_deref(), session) {
            return;
        }
        if self.async_history_enabled {
            self.buffer
                .borrow_mut()
                .push(HistoryJobPayload::ProcessInstanceEnd {
                    process_instance_id: process_instance_id.to_string(),
                    delete_reason: delete_reason.map(|s| s.to_string()),
                    end_time: Utc::now(),
                });
            return;
        }
        if let Some(mut instance) = self
            .runtime_store
            .get_historic_process_instance(process_instance_id, session)
        {
            let now = Utc::now();
            let duration = now.signed_duration_since(instance.start_time);
            instance.end_time = Some(now);
            instance.duration_ms = Some(duration.num_milliseconds());
            instance.delete_reason = delete_reason.map(|s| s.to_string());
            self.runtime_store
                .update_historic_process_instance(&instance, session);
            // P119: HISTORIC_PROCESS_INSTANCE_ENDED —
            // Java DefaultHistoryManager.java:90-95.
            self.queue_event(
                crate::engine::event_dispatcher::historic_process_instance_ended_event(
                    process_instance_id,
                    pd_id.as_deref(),
                ),
            );
        }
    }

    pub fn record_activity_start(
        &self,
        activity_id: &str,
        activity_name: Option<&str>,
        activity_type: &str,
        process_instance_id: &str,
        execution_id: &str,
        session: &mut DbSession,
    ) {
        // Java DefaultHistoryManager.recordActivityStart:208-209 — ACTIVITY+.
        let pd_id = self.process_definition_id_for_instance(process_instance_id, session);
        if !self.is_history_enabled_for_activity(pd_id.as_deref(), session) {
            return;
        }
        if self.async_history_enabled {
            self.buffer
                .borrow_mut()
                .push(HistoryJobPayload::ActivityStart {
                    id: uuid::Uuid::new_v4().to_string(),
                    activity_id: activity_id.to_string(),
                    activity_name: activity_name.map(|s| s.to_string()),
                    activity_type: activity_type.to_string(),
                    process_instance_id: process_instance_id.to_string(),
                    execution_id: execution_id.to_string(),
                    start_time: Utc::now(),
                });
            return;
        }
        let historic_id = uuid::Uuid::new_v4().to_string();
        let instance = HistoricActivityInstance {
            id: historic_id.clone(),
            activity_id: activity_id.to_string(),
            activity_name: activity_name.map(|s| s.to_string()),
            activity_type: activity_type.to_string(),
            process_instance_id: process_instance_id.to_string(),
            execution_id: execution_id.to_string(),
            start_time: Utc::now(),
            end_time: None,
            duration_ms: None,
            assignee: None,
            delete_reason: None,
        };
        self.runtime_store
            .insert_historic_activity_instance(instance, session);
        // P119: HISTORIC_ACTIVITY_INSTANCE_CREATED —
        // Java DefaultHistoryManager.java:215-218.
        self.queue_event(
            crate::engine::event_dispatcher::historic_activity_instance_created_event(
                &historic_id,
                activity_id,
                process_instance_id,
                execution_id,
            ),
        );
    }

    /// Ends the open historic activity for `(execution_id, activity_id)`.
    ///
    /// `delete_reason` maps to Java `ActivityInstanceEntityManager.recordActivityEnd`
    /// (e.g. event-based gateway cancel). Pass `None` for a normal leave.
    pub fn record_activity_end(
        &self,
        execution_id: &str,
        activity_id: &str,
        delete_reason: Option<&str>,
        session: &mut DbSession,
    ) {
        // Java DefaultHistoryManager.recordActivityEnd:225-226 — ACTIVITY+.
        // Prefer existing open historic activity's process_instance → definition.
        let pd_id = self
            .runtime_store
            .get_historic_activity_instance_by_execution_and_activity(
                execution_id,
                activity_id,
                session,
            )
            .map(|a| a.process_instance_id)
            .and_then(|pi| self.process_definition_id_for_instance(&pi, session));
        if !self.is_history_enabled_for_activity(pd_id.as_deref(), session) {
            return;
        }
        if self.async_history_enabled {
            self.buffer
                .borrow_mut()
                .push(HistoryJobPayload::ActivityEnd {
                    execution_id: execution_id.to_string(),
                    activity_id: activity_id.to_string(),
                    end_time: Utc::now(),
                    delete_reason: delete_reason.map(|s| s.to_string()),
                });
            return;
        }
        if let Some(mut instance) = self
            .runtime_store
            .get_historic_activity_instance_by_execution_and_activity(
                execution_id,
                activity_id,
                session,
            )
        {
            let now = Utc::now();
            let duration = now.signed_duration_since(instance.start_time);
            instance.end_time = Some(now);
            instance.duration_ms = Some(duration.num_milliseconds());
            if let Some(reason) = delete_reason {
                instance.delete_reason = Some(reason.to_string());
            }
            let historic_id = instance.id.clone();
            let process_instance_id = instance.process_instance_id.clone();
            self.runtime_store
                .update_historic_activity_instance(instance, session);
            // P119: HISTORIC_ACTIVITY_INSTANCE_ENDED —
            // Java DefaultHistoryManager.java:234-237.
            self.queue_event(
                crate::engine::event_dispatcher::historic_activity_instance_ended_event(
                    &historic_id,
                    activity_id,
                    &process_instance_id,
                    execution_id,
                ),
            );
        }
    }

    pub fn record_task_created(&self, task: &Task, session: &mut DbSession) {
        let process_instance = self
            .runtime_store
            .find_process_instance(&task.process_instance_id, session);
        let process_definition_id = process_instance
            .as_ref()
            .map(|instance| instance.process_definition_id.clone());
        // Java DefaultHistoryManager.recordTaskCreated:258-259 /
        // isHistoryEnabledForUserTask (hasTaskHistoryLevel: TASK or AUDIT+).
        if !self.is_history_enabled_for_user_task(process_definition_id.as_deref(), session) {
            return;
        }
        let task_definition_key =
            (!task.task_definition_key.is_empty()).then(|| task.task_definition_key.clone());
        let task_definition_key_ref = task_definition_key.as_deref().unwrap_or_default();
        let props = self.runtime_store.resolve_user_task_properties(
            &task.process_instance_id,
            &task.execution_id,
            task_definition_key_ref,
            session,
        );
        let resolved_assignee = task.assignee.clone().or(props.assignee);
        let resolved_owner = task.owner.clone().or(props.owner);
        let priority = task.priority.or(props.priority);
        let due_date = task.due_date.or(props.due_date);
        let category = task.category.clone().or(props.category);
        let form_key = task.form_key.clone().or(props.form_key);
        let tenant_id = task.tenant_id.clone().or_else(|| {
            process_instance
                .as_ref()
                .and_then(|instance| instance.tenant_id.clone())
        });

        if self.async_history_enabled {
            self.buffer
                .borrow_mut()
                .push(HistoryJobPayload::TaskCreated {
                    id: task.id.clone(),
                    process_instance_id: task.process_instance_id.clone(),
                    process_definition_id: process_definition_id.clone(),
                    execution_id: task.execution_id.clone(),
                    task_definition_key,
                    name: Some(task.name.clone()),
                    description: task.description.clone(),
                    assignee: resolved_assignee,
                    owner: resolved_owner,
                    claim_time: task.claim_time,
                    tenant_id,
                    category,
                    form_key,
                    parent_task_id: task.parent_task_id.clone(),
                    priority,
                    due_date,
                    start_time: Utc::now(),
                });
            return;
        }

        let assignee_for_identity_link = resolved_assignee.clone();
        let owner_for_identity_link = resolved_owner.clone();
        let instance = HistoricTaskInstance {
            id: task.id.clone(),
            process_instance_id: task.process_instance_id.clone(),
            process_definition_id: process_definition_id.clone(),
            execution_id: task.execution_id.clone(),
            task_definition_key,
            name: Some(task.name.clone()),
            description: task.description.clone(),
            assignee: resolved_assignee,
            owner: resolved_owner,
            claim_time: task.claim_time,
            tenant_id,
            category,
            form_key,
            parent_task_id: task.parent_task_id.clone(),
            priority,
            due_date,
            start_time: Utc::now(),
            end_time: None,
            duration_ms: None,
            delete_reason: None,
        };
        self.runtime_store
            .insert_historic_task_instance(instance, session);
        // P86a — initial assignee/owner also produce an accumulating historic
        // identity link, matching Java's observable end state for a BPMN user
        // task. Java reaches it by a different route: `TaskHelper.insertTask`
        // runs *before* `handleAssignments`
        // (`UserTaskActivityBehavior.java:163-173`, "Handling assignments need
        // to be done after the task is inserted, to have an id"), so
        // `recordTaskCreated` stores a null assignee and the subsequent
        // `TaskEntityManagerImpl.changeTaskAssignee:118-128` →
        // `recordTaskInfoChange` sees null → value and writes the row. Rust
        // resolves the assignee before `record_task_created`, so the diff in
        // `record_task_updated` would find nothing; emitting here reproduces
        // the same single row per initially-assigned task.
        // P97: skip for standalone tasks (empty process_instance_id) — Java's
        // standalone createTask/saveTask route (`TaskEntityManagerImpl
        // .createTask:66-67,77,96`) never runs changeTaskAssignee, so no
        // historic IL exists there (P90b pin).
        if !task.process_instance_id.is_empty() {
            if let Some(assignee) = assignee_for_identity_link.as_deref() {
                self.record_task_assignment_identity_link(&task.id, "assignee", Some(assignee), session);
            }
            if let Some(owner) = owner_for_identity_link.as_deref() {
                self.record_task_assignment_identity_link(&task.id, "owner", Some(owner), session);
            }
        }
        // Java task log 由独立开关 `enableHistoricTaskLogging` 控制、与
        // HistoryLevel 无关（TaskEntityManagerImpl.java:255-260）;
        // "userTaskCreated" task event 是 Rust 扩展、Java 无对应 comment 事件。
        // 二者与 record_task_end 的 USER_TASK_COMPLETED(未按 FULL 门控)同级,
        // 正确门控就是本函数顶部的 user-task history 级;按 FULL 门控会在
        // Java 默认 AUDIT 下隐藏它们,破坏既有 REST 可观测契约。
        {
            self.record_task_log_entry(
                "USER_TASK_CREATED",
                &task.id,
                &task.process_instance_id,
                Some(&task.execution_id),
                Some(&format!("Task {} created", task.id)),
                session,
            );
            let event = HistoricTaskEvent {
                id: uuid::Uuid::new_v4().to_string(),
                task_id: task.id.clone(),
                action: "userTaskCreated".to_string(),
                message: vec![task.name.clone()],
                user_id: None,
                time: Utc::now(),
            };
            self.runtime_store
                .insert_historic_task_event(event, session);
        }
    }

    /// Records the mutable task projection after listeners, assignment, claim,
    /// or metadata updates. Async history must enqueue this snapshot instead of
    /// assuming that the TaskCreated payload has already been replayed.
    pub fn record_task_updated(&self, task: &Task, session: &mut DbSession) {
        let process_definition_id = self
            .runtime_store
            .find_process_instance(&task.process_instance_id, session)
            .map(|pi| pi.process_definition_id);
        // Java DefaultHistoryManager.recordTaskInfoChange:293 — hasTaskHistoryLevel.
        if !self.is_history_enabled_for_user_task(process_definition_id.as_deref(), session) {
            return;
        }
        if self.async_history_enabled {
            self.buffer
                .borrow_mut()
                .push(HistoryJobPayload::TaskUpdated {
                    update: HistoricTaskUpdate::from_runtime_task(task),
                });
            return;
        }
        if let Some(mut instance) = self
            .runtime_store
            .get_historic_task_instance(&task.id, session)
        {
            // P86a — Java `HistoricTaskServiceImpl.recordTaskInfoChange:142-152`:
            // the assignee/owner comparison is made against the *historic* row
            // (not the previous runtime value) and must therefore happen before
            // the row is overwritten below. Each side that changed appends one
            // historic identity link.
            let assignee_changed = instance.assignee != task.assignee;
            let owner_changed = instance.owner != task.owner;
            instance.update_from_runtime_task(task);
            self.runtime_store
                .update_historic_task_instance(instance, session);
            if assignee_changed {
                self.record_task_assignment_identity_link(
                    &task.id,
                    "assignee",
                    task.assignee.as_deref(),
                    session,
                );
            }
            if owner_changed {
                self.record_task_assignment_identity_link(
                    &task.id,
                    "owner",
                    task.owner.as_deref(),
                    session,
                );
            }
        }
    }

    pub fn record_task_end(
        &self,
        task_id: &str,
        delete_reason: Option<&str>,
        session: &mut DbSession,
    ) {
        // Prefer historic task row for process_definition_id when available.
        let process_definition_id = self
            .runtime_store
            .get_historic_task_instance(task_id, session)
            .and_then(|t| t.process_definition_id);
        // Java DefaultHistoryManager.recordTaskEnd:279-280 — hasTaskHistoryLevel.
        if !self.is_history_enabled_for_user_task(process_definition_id.as_deref(), session) {
            return;
        }
        if self.async_history_enabled {
            self.buffer.borrow_mut().push(HistoryJobPayload::TaskEnd {
                task_id: task_id.to_string(),
                delete_reason: delete_reason.map(|s| s.to_string()),
                end_time: Utc::now(),
            });
            return;
        }
        if let Some(mut instance) = self
            .runtime_store
            .get_historic_task_instance(task_id, session)
        {
            let now = Utc::now();
            let duration = now.signed_duration_since(instance.start_time);
            instance.end_time = Some(now);
            instance.duration_ms = Some(duration.num_milliseconds());
            instance.delete_reason = delete_reason.map(|s| s.to_string());
            self.runtime_store
                .update_historic_task_instance(instance, session);
            if let Some(task) = self
                .runtime_store
                .get_historic_task_instance(task_id, session)
            {
                self.record_task_log_entry(
                    "USER_TASK_COMPLETED",
                    task_id,
                    &task.process_instance_id,
                    Some(&task.execution_id),
                    Some(&format!("Task {task_id} completed")),
                    session,
                );
                let event = HistoricTaskEvent {
                    id: uuid::Uuid::new_v4().to_string(),
                    task_id: task_id.to_string(),
                    action: "userTaskCompleted".to_string(),
                    message: vec![task.name.unwrap_or_default()],
                    user_id: None,
                    time: Utc::now(),
                };
                self.runtime_store
                    .insert_historic_task_event(event, session);
            }
        }
    }

    pub fn record_task_suspension_state_change(
        &self,
        task_id: &str,
        previous_suspension_state: i32,
        new_suspension_state: i32,
        task: &crate::task::Task,
        session: &mut DbSession,
    ) {
        let data = serde_json::json!({
            "previousSuspensionState": previous_suspension_state,
            "newSuspensionState": new_suspension_state,
        });
        self.record_task_log_entry(
            "USER_TASK_SUSPENSIONSTATE_CHANGED",
            task_id,
            &task.process_instance_id,
            Some(&task.execution_id),
            Some(&data.to_string()),
            session,
        );
    }

    fn record_task_log_entry(
        &self,
        log_type: &str,
        task_id: &str,
        process_instance_id: &str,
        execution_id: Option<&str>,
        data: Option<&str>,
        session: &mut DbSession,
    ) {
        let process_instance = self
            .runtime_store
            .find_process_instance(process_instance_id, session);
        let entry = HistoricTaskLogEntry {
            id: uuid::Uuid::new_v4().to_string(),
            log_number: self.runtime_store.next_historic_task_log_number(session),
            log_type: log_type.to_string(),
            task_id: task_id.to_string(),
            timestamp: Utc::now(),
            user_id: None,
            data: data.map(str::to_string),
            execution_id: execution_id.map(str::to_string),
            process_instance_id: Some(process_instance_id.to_string()),
            process_definition_id: process_instance
                .as_ref()
                .map(|instance| instance.process_definition_id.clone()),
            scope_id: None,
            scope_definition_id: None,
            sub_scope_id: None,
            scope_type: None,
            tenant_id: process_instance.and_then(|instance| instance.tenant_id),
        };
        self.runtime_store
            .insert_historic_task_log_entry(entry, session);
    }

    #[allow(clippy::too_many_arguments)]
    pub fn record_variable_created(
        &self,
        id: &str,
        name: &str,
        variable_type: &str,
        value: serde_json::Value,
        process_instance_id: &str,
        execution_id: Option<&str>,
        task_id: Option<&str>,
        session: &mut DbSession,
    ) {
        // Java DefaultHistoryManager.recordVariableCreate:336-338 — ACTIVITY+.
        let pd_id = self.process_definition_id_for_instance(process_instance_id, session);
        if !self.is_history_enabled_for_variable(pd_id.as_deref(), session) {
            return;
        }
        let now = Utc::now();
        if self.async_history_enabled {
            self.buffer
                .borrow_mut()
                .push(HistoryJobPayload::VariableCreated {
                    id: id.to_string(),
                    name: name.to_string(),
                    variable_type: variable_type.to_string(),
                    value,
                    process_instance_id: process_instance_id.to_string(),
                    execution_id: execution_id.map(|s| s.to_string()),
                    task_id: task_id.map(|s| s.to_string()),
                    create_time: now,
                });
            return;
        }
        let instance = HistoricVariableInstance {
            id: id.to_string(),
            process_instance_id: process_instance_id.to_string(),
            execution_id: execution_id.map(|s| s.to_string()),
            task_id: task_id.map(|s| s.to_string()),
            name: name.to_string(),
            variable_type: variable_type.to_string(),
            value: value.clone(),
            create_time: now,
            last_updated_time: now,
        };
        self.runtime_store
            .insert_historic_variable_instance(&instance, session);
        // Historic detail variable update is FULL-only
        // (DefaultHistoryManager.recordHistoricDetailVariableCreate:347-348).
        if self.is_history_enabled_for_full_detail(pd_id.as_deref(), session) {
            self.runtime_store.insert_historic_detail(
                HistoricDetail {
                    id: uuid::Uuid::new_v4().to_string(),
                    process_instance_id: process_instance_id.to_string(),
                    execution_id: execution_id.map(str::to_string),
                    activity_instance_id: None,
                    task_id: task_id.map(str::to_string),
                    time: now,
                    detail_type: "variableUpdate".to_string(),
                    revision: Some(0),
                    variable_name: Some(name.to_string()),
                    variable_type: Some(variable_type.to_string()),
                    value: Some(value),
                    property_id: None,
                    property_value: None,
                },
                session,
            );
        }
    }

    pub fn record_variable_updated(
        &self,
        id: &str,
        value: serde_json::Value,
        session: &mut DbSession,
    ) {
        // Variable update itself is ACTIVITY+; detail rows FULL-only.
        // When async, the job is only enqueued if an existing historic variable
        // would exist (i.e. ACTIVITY+ was on at create). Gate on engine/default
        // via existing row's process instance when present.
        if self.async_history_enabled {
            if let Some(existing) = self
                .runtime_store
                .get_historic_variable_instance(id, session)
            {
                let pd_id =
                    self.process_definition_id_for_instance(&existing.process_instance_id, session);
                if !self.is_history_enabled_for_variable(pd_id.as_deref(), session) {
                    return;
                }
                self.buffer
                    .borrow_mut()
                    .push(HistoryJobPayload::VariableUpdated {
                        id: id.to_string(),
                        value,
                        last_updated_time: Utc::now(),
                    });
            }
            return;
        }
        if let Some(mut instance) = self
            .runtime_store
            .get_historic_variable_instance(id, session)
        {
            let pd_id =
                self.process_definition_id_for_instance(&instance.process_instance_id, session);
            // Java DefaultHistoryManager.recordVariableUpdate:366-368 — ACTIVITY+.
            if !self.is_history_enabled_for_variable(pd_id.as_deref(), session) {
                return;
            }
            let now = Utc::now();
            instance.value = value.clone();
            instance.last_updated_time = now;
            if self.is_history_enabled_for_full_detail(pd_id.as_deref(), session) {
                self.runtime_store.insert_historic_detail(
                    HistoricDetail {
                        id: uuid::Uuid::new_v4().to_string(),
                        process_instance_id: instance.process_instance_id.clone(),
                        execution_id: instance.execution_id.clone(),
                        activity_instance_id: None,
                        task_id: instance.task_id.clone(),
                        time: now,
                        detail_type: "variableUpdate".to_string(),
                        revision: Some(1),
                        variable_name: Some(instance.name.clone()),
                        variable_type: Some(instance.variable_type.clone()),
                        value: Some(value),
                        property_id: None,
                        property_value: None,
                    },
                    session,
                );
            }
            self.runtime_store
                .insert_historic_variable_instance(&instance, session);
        }
    }

    pub fn record_form_property(
        &self,
        process_instance_id: &str,
        task_id: Option<&str>,
        property_id: &str,
        property_value: serde_json::Value,
        session: &mut DbSession,
    ) {
        // Java DefaultHistoryManager.recordFormPropertiesSubmitted:380-381 — AUDIT+.
        let pd_id = self.process_definition_id_for_instance(process_instance_id, session);
        if !self.is_history_level_at_least(HistoryLevel::Audit, pd_id.as_deref(), session) {
            return;
        }
        if self.async_history_enabled {
            self.buffer
                .borrow_mut()
                .push(HistoryJobPayload::FormProperty {
                    process_instance_id: process_instance_id.to_string(),
                    task_id: task_id.map(|s| s.to_string()),
                    property_id: property_id.to_string(),
                    property_value,
                    time: Utc::now(),
                });
            return;
        }
        self.runtime_store.insert_historic_detail(
            HistoricDetail {
                id: uuid::Uuid::new_v4().to_string(),
                process_instance_id: process_instance_id.to_string(),
                execution_id: None,
                activity_instance_id: None,
                task_id: task_id.map(str::to_string),
                time: Utc::now(),
                detail_type: "formProperty".to_string(),
                revision: None,
                variable_name: None,
                variable_type: None,
                value: None,
                property_id: Some(property_id.to_string()),
                property_value: Some(property_value),
            },
            session,
        );
    }

    /// Deletes the historic variable instance row.
    ///
    /// Java parity (P70 verification): `DefaultHistoryManager#recordVariableRemoved`
    /// (`DefaultHistoryManager.java:373-376`) always delegates to
    /// `HistoricVariableServiceImpl#recordVariableRemoved` (`:80-89`), which
    /// deletes the historic row **synchronously**. There is no async-history job
    /// type for variable removal in OSS Flowable (`HistoryJobPayload` has
    /// VariableCreated/Updated but not Removed — same shape as Java's OSS
    /// history manager). Keeping this path synchronous when
    /// `async_history_enabled` is therefore aligned, not a gap.
    pub fn record_variable_removed(&self, id: &str, session: &mut DbSession) {
        // Only delete when variable history is enabled (ACTIVITY+).
        // If no historic row exists, delete is a no-op.
        if let Some(existing) = self
            .runtime_store
            .get_historic_variable_instance(id, session)
        {
            let pd_id =
                self.process_definition_id_for_instance(&existing.process_instance_id, session);
            if !self.is_history_enabled_for_variable(pd_id.as_deref(), session) {
                return;
            }
        } else if !self.is_history_enabled_for_variable(None, session) {
            return;
        }
        self.runtime_store
            .delete_historic_variable_instance(id, session);
    }

    pub fn record_audit_event(
        &self,
        event_type: &str,
        process_instance_id: Option<&str>,
        process_definition_id: Option<&str>,
        details: Option<&str>,
        session: &mut DbSession,
    ) {
        // Audit log rows require history != NONE (engine or definition).
        if !self.is_history_enabled_for_definition(process_definition_id, session) {
            return;
        }
        if self.async_history_enabled {
            self.buffer
                .borrow_mut()
                .push(HistoryJobPayload::AuditEvent {
                    id: uuid::Uuid::new_v4().to_string(),
                    event_type: event_type.to_string(),
                    process_instance_id: process_instance_id.map(|s| s.to_string()),
                    process_definition_id: process_definition_id.map(|s| s.to_string()),
                    details: details.map(|s| s.to_string()),
                    timestamp: Utc::now(),
                });
            return;
        }
        let instance = HistoricAuditLog {
            id: uuid::Uuid::new_v4().to_string(),
            event_type: event_type.to_string(),
            process_instance_id: process_instance_id.map(|s| s.to_string()),
            process_definition_id: process_definition_id.map(|s| s.to_string()),
            details: details.map(|s| s.to_string()),
            timestamp: Utc::now(),
        };
        self.runtime_store
            .insert_historic_audit_log(instance, session);
    }

    pub fn cleanup_batch(
        &self,
        before_date: DateTime<Utc>,
        batch_size: usize,
        cleanup_type: &str,
        batch_number: usize,
        session: &mut DbSession,
    ) -> CleanupResult {
        let start_time = std::time::Instant::now();
        let before_millis = before_date.timestamp_millis();

        let instances_to_delete: Vec<String> = match cleanup_type {
            "completed" | "all" | "" => self
                .runtime_store
                .find_historic_process_instance_ids_for_cleanup(
                    before_millis,
                    cleanup_type,
                    batch_size,
                    batch_number,
                    session,
                )
                .unwrap_or_else(|| {
                    self.legacy_cleanup_filter(
                        before_millis,
                        cleanup_type,
                        batch_size,
                        batch_number,
                        session,
                    )
                }),
            _ => self.legacy_cleanup_filter(
                before_millis,
                cleanup_type,
                batch_size,
                batch_number,
                session,
            ),
        };

        let deleted_count = instances_to_delete.len();
        let has_more = deleted_count == batch_size;

        for id in &instances_to_delete {
            self.runtime_store
                .delete_historic_process_instance_cascade(id, session);
        }

        let duration = start_time.elapsed();
        CleanupResult {
            deleted_count,
            batch_number,
            has_more,
            duration_ms: duration.as_millis() as u64,
        }
    }

    fn legacy_cleanup_filter(
        &self,
        before_millis: i64,
        cleanup_type: &str,
        batch_size: usize,
        batch_number: usize,
        session: &mut DbSession,
    ) -> Vec<String> {
        let all_instances = self.runtime_store.list_historic_process_instances(session);
        all_instances
            .iter()
            .filter(|instance| {
                // P133: cutoff on end_time (Java DefaultHistoryCleaningManager.java:36
                // finishedBefore). Running instances have no end_time → never deleted.
                let date_match = instance
                    .end_time
                    .is_some_and(|end| end.timestamp_millis() < before_millis);
                let type_match = match cleanup_type {
                    "completed" => instance.end_time.is_some(),
                    "terminated" => {
                        instance.end_time.is_some()
                            && instance
                                .delete_reason
                                .as_deref()
                                .is_some_and(|r| r.contains("terminated"))
                    }
                    // "all" / "": still only finished instances (date_match requires end_time)
                    _ => true,
                };
                date_match && type_match
            })
            .skip(batch_number * batch_size)
            .take(batch_size)
            .map(|instance| instance.id.clone())
            .collect()
    }

    pub fn log_cleanup(&self, log: CleanupLog, session: &mut DbSession) {
        self.runtime_store.insert_cleanup_log(log, session);
    }

    pub fn get_cleanup_logs(&self, session: &mut DbSession) -> Vec<CleanupLog> {
        self.runtime_store.list_cleanup_logs(session)
    }
}
