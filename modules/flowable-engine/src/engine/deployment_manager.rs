// Pre-existing `unwrap()` call(s), grandfathered by the workspace clippy ratchet
// (`[workspace.lints.clippy] unwrap_used = "warn"` in the root Cargo.toml). These
// sites predate the ratchet and were NOT individually audited against Java. The
// exemption is scoped with `cfg_attr(test, ...)`, so it covers only this file's
// `#[cfg(test)]` code; a NEW unwrap() in production code is still surfaced.
// Do not add more without an audit note.
#![cfg_attr(test, allow(clippy::unwrap_used))]

use crate::persistence::db_session::{BulkJsonRowUpdate, DbSession};
use crate::persistence::db_store::DbStore;
use crate::persistence::runtime_store::{
    EventSubscriptionKind, ProcessEventStartSubscription, ProcessTimerStartSubscription,
};
use crate::persistence::storage_error::StorageError;
use crate::repository::deployment::Deployment;
use crate::repository::deployment_resource::DeploymentResource;
use crate::repository::model::{RepositoryModel, RepositoryModelBytes};
use crate::repository::process_definition::ProcessDefinition;
use flowable_bpmn_model::model::BpmnModel;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};

#[derive(Clone)]
#[allow(clippy::type_complexity)]
pub struct DeploymentManager {
    pub(crate) db_store: Arc<DbStore>,
    pub(crate) session_factory: Arc<dyn Fn() -> Result<DbSession, StorageError> + Send + Sync>,
    bpmn_models: Arc<Mutex<HashMap<String, Arc<BpmnModel>>>>,
    pub(crate) bpmn_model_cache: Arc<crate::engine::bpmn_model_cache::BpmnModelCache>,
    resource_cache: Arc<RwLock<HashMap<(String, String), Arc<Vec<u8>>>>>,
}

enum RepositoryModelBlob {
    Source,
    SourceExtra,
}

impl DeploymentManager {
    pub fn new(
        db_store: Arc<DbStore>,
        session_factory: Arc<dyn Fn() -> Result<DbSession, StorageError> + Send + Sync>,
    ) -> Self {
        Self {
            db_store,
            session_factory,
            bpmn_models: Arc::new(Mutex::new(HashMap::new())),
            bpmn_model_cache: Arc::new(crate::engine::bpmn_model_cache::BpmnModelCache::new()),
            resource_cache: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub fn new_with_memory_backend_for_test(db_store: Arc<DbStore>) -> Self {
        let session_factory = {
            let db_store = Arc::clone(&db_store);
            Arc::new(move || db_store.create_session())
                as Arc<dyn Fn() -> Result<DbSession, StorageError> + Send + Sync>
        };
        Self {
            db_store,
            session_factory,
            bpmn_models: Arc::new(Mutex::new(HashMap::new())),
            bpmn_model_cache: Arc::new(crate::engine::bpmn_model_cache::BpmnModelCache::new()),
            resource_cache: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub fn with_session_factory(
        db_store: Arc<DbStore>,
        session_factory: Arc<dyn Fn() -> Result<DbSession, StorageError> + Send + Sync>,
    ) -> Self {
        Self::new(db_store, session_factory)
    }

    pub fn create_session(&self) -> Result<DbSession, StorageError> {
        (self.session_factory)()
    }

    pub fn db_store(&self) -> &Arc<DbStore> {
        &self.db_store
    }

    pub fn invalidate_bpmn_model_cache(&self) {
        self.bpmn_models
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
    }

    pub fn insert_bpmn_model(&self, process_definition_id: &str, model: BpmnModel) {
        self.bpmn_models
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(process_definition_id.to_string(), Arc::new(model));
    }

    pub fn get_bpmn_model(&self, process_definition_id: &str) -> Option<Arc<BpmnModel>> {
        self.bpmn_models
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(process_definition_id)
            .cloned()
    }

    pub fn contains_bpmn_model(&self, process_definition_id: &str) -> bool {
        self.bpmn_models
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains_key(process_definition_id)
    }

    pub fn remove_bpmn_model(&self, process_definition_id: &str) {
        self.bpmn_models
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(process_definition_id);
    }

    pub fn with_bpmn_models<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&HashMap<String, Arc<BpmnModel>>) -> R,
    {
        let guard = self.bpmn_models.lock().unwrap_or_else(|e| e.into_inner());
        f(&guard)
    }

    #[allow(dead_code)]
    pub(crate) fn bpmn_models(&self) -> std::sync::MutexGuard<'_, HashMap<String, Arc<BpmnModel>>> {
        self.bpmn_models.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn register_timer_start_subscriptions(
        &self,
        subscriptions: Vec<ProcessTimerStartSubscription>,
        session: &mut DbSession,
    ) {
        for mut sub in subscriptions {
            if sub.id.is_empty() {
                sub.id = uuid::Uuid::new_v4().to_string();
            }
            if let Err(error) = session.insert_with_extra(
                "process_timer_start_subscriptions",
                &sub.id,
                &sub,
                &[
                    (
                        "process_definition_id".into(),
                        Some(sub.process_definition_id.clone()),
                    ),
                    ("lock_owner".into(), sub.lock_owner.clone()),
                    ("lock_time".into(), sub.lock_time.map(|v| v.to_string())),
                ],
            ) {
                session.note_write_error(error);
            }
        }
        // Java parity: TimerManager.scheduleTimers persists process-start timers
        // through the MyBatis session; a failed flush throws and aborts the command.
        // Re-note the flush error so it surfaces at commit instead of being silently
        // discarded (a bare `let _ = session.flush()` also *clears* the sticky write
        // error recorded by insert_with_extra, resurrecting the dropped-write bug).
        if let Err(error) = session.flush() {
            session.note_write_error(error);
        }
    }

    pub fn get_timer_start_subscriptions(
        &self,
        session: &mut DbSession,
    ) -> Result<Vec<ProcessTimerStartSubscription>, StorageError> {
        // Java parity: TimerManager/TimerJobEntityManager reads use MyBatis selectList;
        // SQL failures raise PersistenceException and never become an empty result.
        let mut rows = session.find_raw_all("process_timer_start_subscriptions")?;

        rows.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(rows
            .into_iter()
            .filter_map(|r| {
                let mut sub: ProcessTimerStartSubscription = match serde_json::from_str(&r.data) {
                    Ok(s) => s,
                    Err(error) => {
                        tracing::warn!(
                            "Corrupted timer start subscription skipped (id={}): {error}",
                            r.id
                        );
                        return None;
                    }
                };
                if sub.id.is_empty() {
                    sub.id = r.id;
                }
                Some(sub)
            })
            .collect())
    }

    pub fn acquire_due_process_timer_start_subscriptions(
        &self,
        owner: &str,
        now: i64,
        lock_timeout_ms: i64,
        session: &mut DbSession,
    ) -> Result<(Vec<ProcessTimerStartSubscription>, usize, usize), StorageError> {
        self.acquire_due_process_timer_start_subscriptions_selected(
            owner,
            now,
            lock_timeout_ms,
            None,
            None,
            session,
        )
    }

    pub(crate) fn find_due_process_timer_start_subscription_candidates(
        &self,
        now: i64,
        _lock_timeout_ms: i64,
        category_filter: Option<&[String]>,
        session: &mut DbSession,
    ) -> Result<Vec<ProcessTimerStartSubscription>, StorageError> {
        // Expired locks require reset; acquisition only selects unlocked rows.
        let has_category_filter = category_filter.map(|f| !f.is_empty()).unwrap_or(false);
        let mut candidates: Vec<_> = self
            .get_timer_start_subscriptions(session)?
            .into_iter()
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
        candidates.sort_by(|a, b| a.due_time.cmp(&b.due_time).then(a.id.cmp(&b.id)));
        Ok(candidates)
    }

    pub(crate) fn acquire_selected_process_timer_start_subscriptions(
        &self,
        owner: &str,
        now: i64,
        lock_timeout_ms: i64,
        selected_subscription_ids: &[String],
        category_filter: Option<&[String]>,
        session: &mut DbSession,
    ) -> Result<(Vec<ProcessTimerStartSubscription>, usize, usize), StorageError> {
        self.acquire_due_process_timer_start_subscriptions_selected(
            owner,
            now,
            lock_timeout_ms,
            Some(selected_subscription_ids),
            category_filter,
            session,
        )
    }

    pub(crate) fn acquire_selected_process_timer_start_subscriptions_global(
        &self,
        owner: &str,
        now: i64,
        lock_timeout_ms: i64,
        selected_subscription_ids: &[String],
        category_filter: Option<&[String]>,
        session: &mut DbSession,
    ) -> Result<(Vec<ProcessTimerStartSubscription>, usize, usize), StorageError> {
        let mut candidates = self.find_due_process_timer_start_subscription_candidates(
            now,
            lock_timeout_ms,
            category_filter,
            session,
        )?;
        candidates.retain(|candidate| {
            selected_subscription_ids.contains(&candidate.id) && candidate.lock_owner.is_none()
        });
        let mut serialized = Vec::with_capacity(candidates.len());
        for candidate in &mut candidates {
            candidate.lock_owner = Some(owner.to_string());
            candidate.lock_time = Some(now);
            serialized.push(serde_json::to_string(candidate)?);
        }
        let rows: Vec<_> = candidates
            .iter()
            .zip(serialized.iter())
            .map(|(subscription, json)| BulkJsonRowUpdate {
                id: &subscription.id,
                json,
            })
            .collect();
        let affected = session.bulk_update_json_and_columns_by_ids(
            "process_timer_start_subscriptions",
            &rows,
            &[
                ("lock_owner".into(), Some(owner.to_string())),
                ("lock_time".into(), Some(now.to_string())),
            ],
        )?;
        if affected != candidates.len() {
            return Err(StorageError::Persistence(format!(
                "serialized global process-start acquisition selected {} subscriptions but updated {affected}",
                candidates.len()
            )));
        }
        Ok((candidates, 0, 0))
    }

    pub(crate) fn acquire_due_process_timer_start_subscriptions_filtered(
        &self,
        owner: &str,
        now: i64,
        lock_timeout_ms: i64,
        category_filter: Option<&[String]>,
        session: &mut DbSession,
    ) -> Result<(Vec<ProcessTimerStartSubscription>, usize, usize), StorageError> {
        self.acquire_due_process_timer_start_subscriptions_selected(
            owner,
            now,
            lock_timeout_ms,
            None,
            category_filter,
            session,
        )
    }

    fn acquire_due_process_timer_start_subscriptions_selected(
        &self,
        owner: &str,
        now: i64,
        lock_timeout_ms: i64,
        selected_subscription_ids: Option<&[String]>,
        category_filter: Option<&[String]>,
        session: &mut DbSession,
    ) -> Result<(Vec<ProcessTimerStartSubscription>, usize, usize), StorageError> {
        let mut candidates = self.find_due_process_timer_start_subscription_candidates(
            now,
            lock_timeout_ms,
            category_filter,
            session,
        )?;
        if let Some(selected_subscription_ids) = selected_subscription_ids {
            candidates.retain(|candidate| selected_subscription_ids.contains(&candidate.id));
        }

        let mut acquired = Vec::new();
        let mut recovered = 0;
        let mut conflicts = 0;

        for mut t in candidates {
            let old_lock_owner = t.lock_owner.clone();
            let old_lock_time = t.lock_time;
            let was_recovered = old_lock_owner.is_some();

            t.lock_owner = Some(owner.to_string());
            t.lock_time = Some(now);

            // Java parity: a subscription row always serializes; a serialization
            // failure is data corruption, not an empty write. Sticky-record so the
            // acquisition command aborts at commit instead of persisting literal `{}`.
            let json = match serde_json::to_string(&t) {
                Ok(json) => json,
                Err(error) => {
                    session.note_write_error(StorageError::from(error));
                    continue;
                }
            };
            // Java parity: DeploymentManager treats stale/mismatched locks as conflicts
            // (FlowableOptimisticLockingException -> retry), never as a process abort.
            // A present owner without a timestamp is corrupt state -> count as conflict.
            // A real StorageError is NOT a conflict (DbSqlSession.flushUpdateEntity
            // throws PersistenceException on SQL failure vs. FlowableOptimisticLocking
            // on 0 rows): sticky-record it so the command aborts, while locally leaving
            // this row unacquired.
            let Some(old_owner) = old_lock_owner else {
                let affected = match session.cas_update(
                    "process_timer_start_subscriptions",
                    &t.id,
                    &json,
                    &[
                        ("lock_owner".into(), Some(owner.to_string())),
                        ("lock_time".into(), Some(now.to_string())),
                    ],
                    &[("lock_owner".into(), None)],
                ) {
                    Ok(affected) => affected,
                    Err(error) => {
                        session.note_write_error(error);
                        0
                    }
                };
                if affected > 0 {
                    acquired.push(t);
                } else {
                    conflicts += 1;
                }
                continue;
            };
            let Some(old_time) = old_lock_time else {
                conflicts += 1;
                continue;
            };
            let affected = match session.cas_update(
                "process_timer_start_subscriptions",
                &t.id,
                &json,
                &[
                    ("lock_owner".into(), Some(owner.to_string())),
                    ("lock_time".into(), Some(now.to_string())),
                ],
                &[
                    ("lock_owner".into(), Some(old_owner)),
                    ("lock_time".into(), Some(old_time.to_string())),
                ],
            ) {
                Ok(affected) => affected,
                Err(error) => {
                    session.note_write_error(error);
                    0
                }
            };
            if affected > 0 {
                acquired.push(t);
                if was_recovered {
                    recovered += 1;
                }
            } else {
                conflicts += 1;
            }
        }
        Ok((acquired, recovered, conflicts))
    }

    pub fn release_process_timer_start_subscription(
        &self,
        sub: &ProcessTimerStartSubscription,
        session: &mut DbSession,
    ) {
        let mut updated_sub = sub.clone();
        updated_sub.lock_owner = None;
        updated_sub.lock_time = None;
        updated_sub.due_time = None;
        // Java parity: a serialization failure is corruption, not a no-op; sticky-
        // record it so the release command aborts instead of writing literal `{}`.
        let json = match serde_json::to_string(&updated_sub) {
            Ok(json) => json,
            Err(error) => {
                session.note_write_error(StorageError::from(error));
                return;
            }
        };
        if let (Some(owner), Some(lock_time)) = (sub.lock_owner.as_deref(), sub.lock_time) {
            // Java parity: TimerJobEntityManagerImpl.delete/update goes through the
            // MyBatis session; a SQL failure throws and aborts. cas_update does not
            // sticky-record on its own, so record the error here.
            if let Err(error) = session.cas_update(
                "process_timer_start_subscriptions",
                &sub.id,
                &json,
                &[("lock_owner".into(), None), ("lock_time".into(), None)],
                &[
                    ("lock_owner".into(), Some(owner.to_string())),
                    ("lock_time".into(), Some(lock_time.to_string())),
                ],
            ) {
                session.note_write_error(error);
            }
        }
    }

    /// After a process-start timeCycle fires: clear the lock and either reschedule
    /// the next due (repeat remaining) or permanently retire (`due_time = None`).
    /// Java: `TimerJobSchedulerImpl.rescheduleTimerJobAfterExecution` +
    /// `TimerJobEntityManagerImpl.createAndCalculateNextTimer`.
    pub fn reschedule_or_release_process_timer_start_subscription(
        &self,
        sub: &ProcessTimerStartSubscription,
        next_cycle: Option<crate::engine::time_source::CycleSchedule>,
        session: &mut DbSession,
    ) {
        let mut updated_sub = sub.clone();
        updated_sub.lock_owner = None;
        updated_sub.lock_time = None;
        match next_cycle {
            Some(schedule) => {
                updated_sub.time_cycle = Some(schedule.cycle);
                updated_sub.due_time = Some(schedule.due_time_millis);
            }
            None => {
                updated_sub.due_time = None;
            }
        }
        // Java parity: a serialization failure is corruption, not a no-op; sticky-
        // record it so the reschedule command aborts instead of writing literal `{}`.
        let json = match serde_json::to_string(&updated_sub) {
            Ok(json) => json,
            Err(error) => {
                session.note_write_error(StorageError::from(error));
                return;
            }
        };
        if let (Some(owner), Some(lock_time)) = (sub.lock_owner.as_deref(), sub.lock_time) {
            // Java parity: the timer reschedule/release update goes through the MyBatis
            // session; a SQL failure throws and aborts. cas_update does not sticky-record
            // on its own, so record the error here.
            if let Err(error) = session.cas_update(
                "process_timer_start_subscriptions",
                &sub.id,
                &json,
                &[("lock_owner".into(), None), ("lock_time".into(), None)],
                &[
                    ("lock_owner".into(), Some(owner.to_string())),
                    ("lock_time".into(), Some(lock_time.to_string())),
                ],
            ) {
                session.note_write_error(error);
            }
        }
    }

    /// Releases an acquired timer-start subscription after task submission is
    /// rejected. Unlike successful timer execution, rejection must preserve
    /// the due date so the subscription remains eligible for reacquisition.
    pub fn release_process_timer_start_subscription_lock(
        &self,
        sub: &ProcessTimerStartSubscription,
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
            "process_timer_start_subscriptions",
            &sub.id,
            &json,
            &[("lock_owner".into(), None), ("lock_time".into(), None)],
            &[
                ("lock_owner".into(), Some(expected_owner.to_string())),
                ("lock_time".into(), Some(lock_time.to_string())),
            ],
        )? > 0)
    }

    pub fn delete_timer_start_subscriptions_by_process_definition_id(
        &self,
        process_definition_id: &str,
        session: &mut DbSession,
    ) {
        // Java parity: TimerManager.removeObsoleteTimers deletes process-start
        // timers via the MyBatis session; a SQL failure throws and aborts. delete_by
        // sticky-records internally; keep the abort explicit here too.
        if let Err(error) = session.delete_by(
            "process_timer_start_subscriptions",
            "process_definition_id",
            process_definition_id,
        ) {
            session.note_write_error(error);
        }
    }

    /// Java `TimerManager.removeObsoleteTimers`: cancel timer-start subscriptions
    /// for all versions of a process-definition key (and matching tenant).
    pub fn delete_timer_start_subscriptions_by_process_definition_key(
        &self,
        process_definition_key: &str,
        tenant_id: Option<&str>,
        session: &mut DbSession,
    ) -> Result<(), crate::error::FlowableError> {
        let defs = self.get_process_definitions(session)?;
        let to_delete: Vec<String> = self
            .get_timer_start_subscriptions(session)?
            .into_iter()
            .filter(|sub| {
                if sub.process_definition_key != process_definition_key {
                    return false;
                }
                let def_tenant = defs
                    .get(&sub.process_definition_id)
                    .and_then(|d| d.tenant_id.as_deref());
                def_tenant == tenant_id
            })
            .map(|sub| sub.id)
            .collect();
        for id in to_delete {
            // Java parity: TimerManager.removeObsoleteTimers funnels the delete
            // through the MyBatis session; a SQL failure throws and rolls the
            // command back. Sticky-record it so flush() re-raises instead of
            // silently reporting success.
            if let Err(error) = session.delete("process_timer_start_subscriptions", &id) {
                session.note_write_error(error);
            }
        }
        Ok(())
    }

    pub fn register_event_start_subscriptions(
        &self,
        subscriptions: Vec<ProcessEventStartSubscription>,
        session: &mut DbSession,
    ) {
        for sub in subscriptions {
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
            if let Err(error) = session.insert_with_extra(
                "process_event_start_subscriptions",
                &uuid::Uuid::new_v4().to_string(),
                &sub,
                &[
                    (
                        "process_definition_id".into(),
                        Some(sub.process_definition_id.clone()),
                    ),
                    ("event_kind".into(), Some(kind_str.to_string())),
                    ("event_ref".into(), Some(sub.event_ref.clone())),
                ],
            ) {
                session.note_write_error(error);
            }
        }
        // Java parity: EventSubscriptionManager.insertMessageEvent/insertSignalEvent
        // persist via the MyBatis session; a failed flush throws and aborts. Re-note
        // the flush error so it surfaces at commit instead of being silently cleared.
        if let Err(error) = session.flush() {
            session.note_write_error(error);
        }
    }

    pub fn get_event_start_subscriptions(
        &self,
        session: &mut DbSession,
    ) -> Result<Vec<ProcessEventStartSubscription>, crate::error::FlowableError> {
        let subs = session
            .find_all::<ProcessEventStartSubscription>("process_event_start_subscriptions")?;
        Ok(subs)
    }

    pub fn delete_event_start_subscriptions_by_process_definition_id(
        &self,
        process_definition_id: &str,
        session: &mut DbSession,
    ) {
        // Java parity: EventSubscriptionEntityManager.deleteEventSubscriptionsForProcessDefinition
        // deletes via the MyBatis session; a SQL failure throws and aborts. delete_by
        // sticky-records internally; keep the abort explicit here.
        if let Err(error) = session.delete_by(
            "process_event_start_subscriptions",
            "process_definition_id",
            process_definition_id,
        ) {
            session.note_write_error(error);
        }
    }

    /// Java `BpmnDeploymentHelper.addEventRegistrations` →
    /// `EventSubscriptionManager.removeObsoleteMessageEventSubscriptions` /
    /// `removeObsoleteSignalEventSubscription` (EventSubscriptionManager.java:55-67,122-133):
    /// on redeploy, message/signal start subscriptions of prior versions of the
    /// same process-definition key (and matching tenant) are removed before the
    /// new version registers its own. Symmetric to
    /// `delete_timer_start_subscriptions_by_process_definition_key`.
    pub fn delete_event_start_subscriptions_by_process_definition_key(
        &self,
        process_definition_key: &str,
        tenant_id: Option<&str>,
        session: &mut DbSession,
    ) -> Result<(), crate::error::FlowableError> {
        // Java parity: EventSubscriptionManager.removeObsolete{Message,Signal}EventSubscriptions
        // (EventSubscriptionManager.java:122-133) selectList the current subscriptions; the
        // query throws a PersistenceException on SQL failure. Swallowing the read to an
        // empty Vec would silently skip removing prior-version start subscriptions on
        // redeploy, so propagate the storage error instead.
        let rows = session.find_raw_all("process_event_start_subscriptions")?;
        for row in rows {
            let sub: ProcessEventStartSubscription = match serde_json::from_str(&row.data) {
                Ok(s) => s,
                Err(error) => {
                    tracing::warn!(
                        "Corrupted event start subscription skipped (id={}): {error}",
                        row.id
                    );
                    continue;
                }
            };
            if sub.process_definition_key == process_definition_key
                && sub.tenant_id.as_deref() == tenant_id
            {
                // delete() sticky-records write errors internally; the sticky error aborts
                // the command at commit (Java parity: a delete SQL failure throws).
                if let Err(error) = session.delete("process_event_start_subscriptions", &row.id) {
                    session.note_write_error(error);
                }
            }
        }
        Ok(())
    }

    pub fn find_event_start_subscriptions_by_event_ref(
        &self,
        event_kind: &EventSubscriptionKind,
        event_ref: &str,
        session: &mut DbSession,
    ) -> Result<Vec<ProcessEventStartSubscription>, crate::error::FlowableError> {
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
        // Java parity: EventSubscriptionEntityManagerImpl.findEventSubscriptionsByName*
        // (selectList) throws a PersistenceException on SQL failure; a storage error must
        // not be disguised as "no matching start subscriptions", which would silently
        // drop the message/signal start trigger.
        Ok(session.find_by_two(
            "process_event_start_subscriptions",
            "event_kind",
            kind_str,
            "event_ref",
            event_ref,
        )?)
    }

    pub fn next_process_definition_version(
        &self,
        tenant_id: Option<&str>,
        process_key: &str,
        session: &mut DbSession,
    ) -> Result<i32, crate::error::FlowableError> {
        let tenant_str = tenant_id.unwrap_or("");
        // Java parity: BpmnDeployer.setProcessDefinitionVersionsAndIds (L202-234) sets
        // version=1 (L208) and bumps it only when a latest version exists
        // (getMostRecentVersionOfProcessDefinition -> BpmnDeploymentHelper L106-120 ->
        // MybatisProcessDefinitionDataManager L52-58 selectOne). That query throws a
        // PersistenceException on SQL failure; the version-1 default is reachable ONLY
        // from a successful empty result, never from a storage error. Swallowing the
        // error to 1 would silently reuse version 1 and collide with an existing
        // definition, so propagate it instead.
        Ok(session.next_process_definition_version(tenant_str, process_key)?)
    }

    pub fn register_deployment(&self, deployment: Deployment, session: &mut DbSession) {
        let deployment_id = deployment.id.clone();
        let created_at = deployment
            .deployment_time
            .map(|value| value.timestamp_millis())
            .unwrap_or_default();

        let mut deployment_no_resources = deployment.clone();
        deployment_no_resources.resources.clear();
        // Java parity: DeploymentEntityManagerImpl.insert -> AbstractDataManager.insert
        // -> DbSqlSession.insert; a SQL failure throws and aborts the deploy command.
        // insert() sticky-records internally; keep the abort explicit here.
        if let Err(error) = session.insert("deployments", &deployment_id, &deployment_no_resources)
        {
            session.note_write_error(error);
        }

        // ADR-0001 Phase 5: dual-write normalized ACT_RE_DEPLOYMENT via DataManager.
        // 立即执行 DELETE（flush 顺序 INSERT 先于 DELETE，queued delete 会导致 UNIQUE 冲突）。
        // Hard-fail dual-write errors (P73a): do not swallow with `let _ =` — on PostgreSQL
        // a failed statement aborts the whole transaction and silent ACT_* divergence is worse.
        let entity =
            crate::persistence::entity_mapping::deployment_to_entity(&deployment_no_resources);
        {
            use flowable_persistence::statement::StatementId;
            use flowable_persistence::value::DbParams;
            let mut params = DbParams::new();
            params.push(entity.id.clone());
            // DELETE of a missing row is success (0 rows); real SQL errors must propagate.
            if let Err(err) = session
                .inner_mut()
                .execute(StatementId::DeleteDeployment, params)
            {
                session.note_write_error(crate::persistence::StorageError::Persistence(format!(
                    "dual-write pre-delete ACT_RE_DEPLOYMENT failed for id={}: {err}",
                    entity.id
                )));
            }
        }
        if let Err(err) =
            flowable_persistence::DeploymentDataManager::new().insert(session.inner_mut(), entity)
        {
            session.note_write_error(crate::persistence::StorageError::Persistence(format!(
                "dual-write ACT_RE_DEPLOYMENT insert failed for id={deployment_id}: {err}"
            )));
        }
        // DataManager insert only queues; flush so SQL failures surface as dual-write errors.
        if let Err(err) = session.inner_mut().flush() {
            session.note_write_error(crate::persistence::StorageError::Persistence(format!(
                "dual-write ACT_RE_DEPLOYMENT flush failed for id={deployment_id}: {err}"
            )));
        }

        for (name, bytes) in &deployment.resources {
            let resource = DeploymentResource::new(
                deployment_id.clone(),
                name.clone(),
                bytes.clone(),
                created_at,
            );
            if let Err(error) = session.upsert_deployment_resource(
                &resource.deployment_id,
                &resource.resource_name,
                &resource.resource_type,
                &resource.content_type,
                &resource.bytes,
                resource.created_at,
            ) {
                // Java parity: ResourceEntityManagerImpl persists deployment
                // resources through the MyBatis DbSqlSession; a failed INSERT/UPDATE
                // throws PersistenceException and aborts the deploy command. Sticky-
                // record so flush_and_commit fails instead of dropping resource bytes.
                session.note_write_error(crate::persistence::StorageError::Persistence(
                    format!(
                        "upsert_deployment_resource failed for deployment={deployment_id} name={name}: {error}"
                    ),
                ));
            }

            // Dual-write resource bytes into ACT_GE_BYTEARRAY / deployment resource statements.
            // 先删除可能存在的旧记录，避免 UNIQUE 约束冲突。
            if let Err(err) = flowable_persistence::DeploymentResourceDataManager::new()
                .delete_by_deployment_id_and_name(session.inner_mut(), &deployment_id, name)
            {
                session.note_write_error(crate::persistence::StorageError::Persistence(format!(
                    "dual-write pre-delete ACT_GE_BYTEARRAY failed for deployment={deployment_id} name={name}: {err}"
                )));
            }
            let mut byte_entity =
                flowable_persistence::ByteArrayEntity::new(format!("{deployment_id}:{name}"));
            byte_entity.name = Some(name.clone());
            byte_entity.deployment_id = Some(deployment_id.clone());
            byte_entity.bytes = Some(bytes.clone());
            if let Err(err) = flowable_persistence::DeploymentResourceDataManager::new()
                .insert(session.inner_mut(), byte_entity)
            {
                session.note_write_error(crate::persistence::StorageError::Persistence(format!(
                    "dual-write ACT_GE_BYTEARRAY insert failed for deployment={deployment_id} name={name}: {err}"
                )));
            }
            if let Err(err) = session.inner_mut().flush() {
                session.note_write_error(crate::persistence::StorageError::Persistence(format!(
                    "dual-write ACT_GE_BYTEARRAY flush failed for deployment={deployment_id} name={name}: {err}"
                )));
            }

            let key = (deployment_id.clone(), name.clone());
            self.resource_cache
                .write()
                .unwrap_or_else(|e| e.into_inner())
                .insert(key, Arc::new(bytes.clone()));
        }
        // Flush remaining JSON-path work after dual-write; sticky-note failures.
        if let Err(err) = session.flush() {
            session.note_write_error(crate::persistence::StorageError::Persistence(format!(
                "flush after dual-write deployment failed for id={deployment_id}: {err}"
            )));
        }
    }

    pub fn get_deployment(
        &self,
        deployment_id: &str,
        session: &mut DbSession,
    ) -> Result<Option<Deployment>, crate::error::FlowableError> {
        Ok(self.get_deployments(session)?.remove(deployment_id))
    }

    pub fn get_deployment_resource_names(
        &self,
        deployment_id: &str,
        session: &mut DbSession,
    ) -> Result<Vec<String>, crate::error::FlowableError> {
        // Java parity: MybatisDeploymentDataManager.getDeploymentResourceNames (L57-60)
        // selectList throws a PersistenceException on SQL failure; a storage error must
        // not be disguised as an empty resource-name list.
        Ok(session.list_deployment_resource_names(deployment_id)?)
    }

    pub fn get_deployment_resources(
        &self,
        deployment_id: &str,
        session: &mut DbSession,
    ) -> Result<Vec<DeploymentResource>, crate::error::FlowableError> {
        // Java parity: ResourceEntityManagerImpl.findResourcesByDeploymentId (selectList)
        // throws on SQL failure; a storage error must not be disguised as an empty list.
        Ok(session.list_deployment_resources(deployment_id)?)
    }

    pub fn get_deployment_resource(
        &self,
        deployment_id: &str,
        name: &str,
        session: &mut DbSession,
    ) -> Result<Option<DeploymentResource>, crate::error::FlowableError> {
        // Java parity: ResourceEntityManagerImpl.findResourceByDeploymentIdAndResourceName
        // (L38-46 selectOne) throws on SQL failure; a storage error must not be disguised
        // as a missing resource.
        Ok(session.find_deployment_resource(deployment_id, name)?)
    }

    pub fn get_deployment_resource_bytes(
        &self,
        deployment_id: &str,
        name: &str,
        session: &mut DbSession,
    ) -> Result<Option<Vec<u8>>, crate::error::FlowableError> {
        let key = (deployment_id.to_string(), name.to_string());
        {
            let read = self
                .resource_cache
                .read()
                .unwrap_or_else(|e| e.into_inner());
            if let Some(bytes) = read.get(&key) {
                return Ok(Some(bytes.as_ref().clone()));
            }
        }
        // Java parity: ResourceEntityManagerImpl.findResourceByDeploymentIdAndResourceName
        // (selectOne) throws on SQL failure. The prior `.ok()??` swallowed a storage error
        // into None, disguising a read failure as a missing resource; propagate it and
        // return None only for a genuinely absent blob.
        let Some(bytes) = session.find_blob_by_two(
            "deployment_resources",
            "deployment_id",
            deployment_id,
            "name",
            name,
            "bytes",
        )?
        else {
            return Ok(None);
        };
        self.resource_cache
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(key, Arc::new(bytes.clone()));
        Ok(Some(bytes))
    }

    pub fn get_deployments(
        &self,
        session: &mut DbSession,
    ) -> Result<HashMap<String, Deployment>, crate::error::FlowableError> {
        let mut map: HashMap<String, Deployment> = session
            .find_all::<Deployment>("deployments")?
            .into_iter()
            .map(|d| (d.id.clone(), d))
            .collect();
        let rows = session.iter_all_deployment_resource_bytes()?;
        for (dep_id, name, bytes) in rows {
            if let Some(d) = map.get_mut(&dep_id) {
                d.resources.insert(name, bytes);
            }
        }
        Ok(map)
    }

    pub fn get_process_definitions(
        &self,
        session: &mut DbSession,
    ) -> Result<HashMap<String, ProcessDefinition>, crate::error::FlowableError> {
        let mut map = HashMap::new();
        for pd in session.find_all::<ProcessDefinition>("process_definitions")? {
            map.insert(pd.id.clone(), pd);
        }
        Ok(map)
    }

    pub fn insert_process_definition(&self, pd: ProcessDefinition, session: &mut DbSession) {
        // Java parity: ProcessDefinitionEntityManager insert (BpmnDeployer
        // .persistProcessDefinitionsAndAuthorizations L288) -> DbSqlSession.insert; a
        // SQL failure throws and aborts the deploy. insert_with_extra sticky-records
        // internally; keep the abort explicit here.
        if let Err(error) = session.insert_with_extra(
            "process_definitions",
            &pd.id,
            &pd,
            &[(
                "deployment_id".into(),
                Some(pd.deployment_id.clone().unwrap_or_default()),
            )],
        ) {
            session.note_write_error(error);
        }

        // ADR-0001 Phase 5: dual-write normalized ACT_RE_PROCDEF via DataManager.
        // 立即执行 DELETE（flush 顺序 INSERT 先于 DELETE，queued delete 会导致 UNIQUE 冲突）。
        // Hard-fail dual-write errors (P73a).
        let entity = crate::persistence::entity_mapping::process_definition_to_entity(&pd);
        {
            use flowable_persistence::statement::StatementId;
            use flowable_persistence::value::DbParams;
            let mut params = DbParams::new();
            params.push(entity.id.clone());
            if let Err(err) = session
                .inner_mut()
                .execute(StatementId::DeleteProcessDefinition, params)
            {
                session.note_write_error(crate::persistence::StorageError::Persistence(format!(
                    "dual-write pre-delete ACT_RE_PROCDEF failed for id={}: {err}",
                    entity.id
                )));
            }
        }
        if let Err(err) = flowable_persistence::ProcessDefinitionDataManager::new()
            .insert(session.inner_mut(), entity)
        {
            session.note_write_error(crate::persistence::StorageError::Persistence(format!(
                "dual-write ACT_RE_PROCDEF insert failed for id={}: {err}",
                pd.id
            )));
        }
        if let Err(err) = session.inner_mut().flush() {
            session.note_write_error(crate::persistence::StorageError::Persistence(format!(
                "dual-write ACT_RE_PROCDEF flush failed for id={}: {err}",
                pd.id
            )));
        }

        if let Err(err) = session.flush() {
            session.note_write_error(crate::persistence::StorageError::Persistence(format!(
                "flush after dual-write process definition failed for id={}: {err}",
                pd.id
            )));
        }
    }

    pub fn update_process_definition(
        &self,
        pd: ProcessDefinition,
        session: &mut DbSession,
    ) -> Result<Option<()>, crate::error::FlowableError> {
        if !self.get_process_definitions(session)?.contains_key(&pd.id) {
            return Ok(None);
        }
        self.insert_process_definition(pd, session);
        Ok(Some(()))
    }

    pub fn insert_repository_model(
        &self,
        model: RepositoryModel,
        source_bytes: Vec<u8>,
        source_extra_bytes: Vec<u8>,
        session: &mut DbSession,
    ) {
        // Java parity: ModelEntityManagerImpl.insert (L42-48) -> DbSqlSession.insert; a
        // serialization failure is corruption and a SQL failure throws — both must
        // abort the command, never persist `{}` or silently drop the model.
        let data_json = match serde_json::to_string(&model) {
            Ok(json) => json,
            Err(error) => {
                session.note_write_error(StorageError::from(error));
                return;
            }
        };
        let dep_id = model.deployment_id.as_deref().unwrap_or("");
        let tenant = model.tenant_id.as_deref().unwrap_or("");
        if let Err(error) = session.insert_repository_model(
            &model.id,
            &data_json,
            dep_id,
            &model.key,
            tenant,
            &source_bytes,
            &source_extra_bytes,
        ) {
            session.note_write_error(error);
        }
    }

    pub fn get_repository_models(
        &self,
        session: &mut DbSession,
    ) -> Result<Vec<RepositoryModel>, crate::error::FlowableError> {
        let mut models = session.find_all::<RepositoryModel>("repository_models")?;
        models.sort_by(|left, right| left.key.cmp(&right.key).then(left.id.cmp(&right.id)));
        Ok(models)
    }

    pub fn get_repository_model(
        &self,
        model_id: &str,
        session: &mut DbSession,
    ) -> Result<Option<RepositoryModel>, crate::error::FlowableError> {
        // Java parity: ModelEntityManagerImpl.findById -> DbSqlSession selectOne throws a
        // PersistenceException on SQL failure; a storage error must not be disguised as a
        // missing model.
        Ok(session.find("repository_models", model_id)?)
    }

    pub fn update_repository_model(
        &self,
        model: RepositoryModel,
        session: &mut DbSession,
    ) -> Result<Option<()>, crate::error::FlowableError> {
        // Ok(None) means genuinely not-found (404); a storage error propagates instead of
        // being swallowed by the existence read.
        if self.get_repository_model(&model.id, session)?.is_none() {
            return Ok(None);
        }
        // Java parity: ModelEntityManagerImpl.updateModel (L50-54) -> DbSqlSession.update;
        // a serialization failure is corruption and a SQL failure throws — both must
        // abort the command. update_repository_model_data does not sticky-record, so
        // record it here (the sticky write error aborts the transaction at commit).
        let data_json = match serde_json::to_string(&model) {
            Ok(json) => json,
            Err(error) => {
                session.note_write_error(StorageError::from(error));
                return Ok(Some(()));
            }
        };
        let dep_id = model.deployment_id.as_deref().unwrap_or("");
        let tenant = model.tenant_id.as_deref().unwrap_or("");
        if let Err(error) =
            session.update_repository_model_data(&model.id, &data_json, dep_id, &model.key, tenant)
        {
            session.note_write_error(error);
        }
        Ok(Some(()))
    }

    pub fn update_repository_model_source(
        &self,
        model: RepositoryModel,
        source_bytes: Vec<u8>,
        session: &mut DbSession,
    ) -> Result<Option<()>, crate::error::FlowableError> {
        self.update_repository_model_blob(session, model, RepositoryModelBlob::Source, source_bytes)
    }

    pub fn update_repository_model_source_extra(
        &self,
        model: RepositoryModel,
        source_extra_bytes: Vec<u8>,
        session: &mut DbSession,
    ) -> Result<Option<()>, crate::error::FlowableError> {
        self.update_repository_model_blob(
            session,
            model,
            RepositoryModelBlob::SourceExtra,
            source_extra_bytes,
        )
    }

    pub fn delete_repository_model(
        &self,
        model_id: &str,
        session: &mut DbSession,
    ) -> Result<bool, crate::error::FlowableError> {
        // Java parity: ModelEntityManagerImpl.findById selectOne throws on SQL failure; a
        // storage error must not be disguised as a missing model (which would report a
        // spurious 404 and skip the delete).
        let prev = session.find::<RepositoryModel>("repository_models", model_id)?;
        if let Err(error) = session.delete("repository_models", model_id) {
            session.note_write_error(error);
        }
        // Java parity: ModelEntityManagerImpl.delete -> DbSqlSession.delete; a failed
        // flush throws and aborts. Re-note the flush error so it surfaces at commit
        // instead of being silently cleared (a bare `let _ = session.flush()` also
        // discards the sticky delete error, resurrecting the dropped-write bug).
        if let Err(error) = session.flush() {
            session.note_write_error(error);
        }

        Ok(prev.is_some())
    }

    pub fn get_repository_model_source(
        &self,
        model_id: &str,
        session: &mut DbSession,
    ) -> Result<Option<RepositoryModelBytes>, crate::error::FlowableError> {
        let Some(model) = self.get_repository_model(model_id, session)? else {
            return Ok(None);
        };
        let Some(bytes) =
            self.get_repository_model_bytes(model_id, RepositoryModelBlob::Source, session)?
        else {
            return Ok(None);
        };
        Ok(Some(RepositoryModelBytes {
            content_type: model.source_content_type,
            bytes,
        }))
    }

    pub fn get_repository_model_source_extra(
        &self,
        model_id: &str,
        session: &mut DbSession,
    ) -> Result<Option<RepositoryModelBytes>, crate::error::FlowableError> {
        let Some(model) = self.get_repository_model(model_id, session)? else {
            return Ok(None);
        };
        let Some(bytes) =
            self.get_repository_model_bytes(model_id, RepositoryModelBlob::SourceExtra, session)?
        else {
            return Ok(None);
        };
        Ok(Some(RepositoryModelBytes {
            content_type: model.source_extra_content_type,
            bytes,
        }))
    }

    fn get_repository_model_bytes(
        &self,
        model_id: &str,
        blob: RepositoryModelBlob,
        session: &mut DbSession,
    ) -> Result<Option<Vec<u8>>, crate::error::FlowableError> {
        let blob_col = match blob {
            RepositoryModelBlob::Source => "source_bytes",
            RepositoryModelBlob::SourceExtra => "source_extra_bytes",
        };
        // Java parity: model source blobs load via DbSqlSession selectOne, which throws on
        // SQL failure; a storage error must not be disguised as an absent blob.
        Ok(session.find_blob("repository_models", "id", model_id, blob_col)?)
    }

    fn update_repository_model_blob(
        &self,
        session: &mut DbSession,
        model: RepositoryModel,
        blob: RepositoryModelBlob,
        bytes: Vec<u8>,
    ) -> Result<Option<()>, crate::error::FlowableError> {
        if self.get_repository_model(&model.id, session)?.is_none() {
            return Ok(None);
        }
        // Java parity: ModelEntityManagerImpl.updateModel (source-blob update) ->
        // DbSqlSession.update; a serialization failure is corruption and a SQL failure
        // throws — both must abort. update_repository_model_blob does not sticky-record,
        // so record it here.
        let data_json = match serde_json::to_string(&model) {
            Ok(json) => json,
            Err(error) => {
                session.note_write_error(StorageError::from(error));
                return Ok(Some(()));
            }
        };
        let dep_id = model.deployment_id.as_deref().unwrap_or("");
        let tenant = model.tenant_id.as_deref().unwrap_or("");
        let blob_col = match blob {
            RepositoryModelBlob::Source => "source_bytes",
            RepositoryModelBlob::SourceExtra => "source_extra_bytes",
        };
        if let Err(error) = session.update_repository_model_blob(
            &model.id, &data_json, dep_id, &model.key, tenant, blob_col, &bytes,
        ) {
            session.note_write_error(error);
        }
        Ok(Some(()))
    }

    pub fn delete_deployment(
        &self,
        deployment_id: &str,
        session: &mut DbSession,
    ) -> Result<(), crate::error::FlowableError> {
        // Java parity: DeploymentEntityManagerImpl.deleteDeployment (L51-70) deletes the
        // deployment, its resources, and process definitions via DbSqlSession; each
        // failure throws and aborts. delete/delete_by sticky-record internally; keep the
        // abort explicit here.
        if let Err(error) = session.delete("deployments", deployment_id) {
            session.note_write_error(error);
        }
        if let Err(error) =
            session.delete_by("deployment_resources", "deployment_id", deployment_id)
        {
            session.note_write_error(error);
        }
        if let Err(error) = session.delete_by("repository_models", "deployment_id", deployment_id) {
            session.note_write_error(error);
        }

        // Dual-delete normalized ACT_* rows when present (P73a hard-fail on errors).
        if let Err(err) = flowable_persistence::DeploymentResourceDataManager::new()
            .delete_by_deployment_id(session.inner_mut(), deployment_id)
        {
            session.note_write_error(crate::persistence::StorageError::Persistence(format!(
                "dual-delete ACT_GE_BYTEARRAY by deployment failed for id={deployment_id}: {err}"
            )));
        }
        match flowable_persistence::DeploymentDataManager::new()
            .find_by_id(session.inner_mut(), deployment_id)
        {
            Ok(Some(entity)) => {
                if let Err(err) = flowable_persistence::DeploymentDataManager::new()
                    .delete(session.inner_mut(), &entity)
                {
                    session.note_write_error(crate::persistence::StorageError::Persistence(
                        format!(
                            "dual-delete ACT_RE_DEPLOYMENT failed for id={deployment_id}: {err}"
                        ),
                    ));
                }
            }
            Ok(None) => {}
            Err(err) => {
                session.note_write_error(crate::persistence::StorageError::Persistence(format!(
                    "dual-delete ACT_RE_DEPLOYMENT find_by_id failed for id={deployment_id}: {err}"
                )));
            }
        }

        // Java parity: DeploymentEntityManagerImpl.deleteDeployment (L51-70) loads the
        // deployment's process definitions (deleteProcessDefinitionsForDeployment, L67)
        // via selectList, which throws a PersistenceException on SQL failure. Swallowing
        // the read to an empty Vec would silently skip cascade deletion of timers, event
        // subscriptions and definition rows, so propagate the storage error instead.
        let process_definitions: Vec<ProcessDefinition> =
            session.find_by("process_definitions", "deployment_id", deployment_id)?;

        for pd in process_definitions {
            let process_definition_id = pd.id;
            self.delete_timer_start_subscriptions_by_process_definition_id(
                &process_definition_id,
                session,
            );
            self.delete_event_start_subscriptions_by_process_definition_id(
                &process_definition_id,
                session,
            );
            if let Err(error) = session.delete("process_definitions", &process_definition_id) {
                session.note_write_error(error);
            }
            match flowable_persistence::ProcessDefinitionDataManager::new()
                .find_by_id(session.inner_mut(), &process_definition_id)
            {
                Ok(Some(entity)) => {
                    if let Err(err) = flowable_persistence::ProcessDefinitionDataManager::new()
                        .delete(session.inner_mut(), &entity)
                    {
                        session.note_write_error(crate::persistence::StorageError::Persistence(
                            format!(
                                "dual-delete ACT_RE_PROCDEF failed for id={process_definition_id}: {err}"
                            ),
                        ));
                    }
                }
                Ok(None) => {}
                Err(err) => {
                    session.note_write_error(crate::persistence::StorageError::Persistence(
                        format!(
                            "dual-delete ACT_RE_PROCDEF find_by_id failed for id={process_definition_id}: {err}"
                        ),
                    ));
                }
            }
            self.remove_bpmn_model(&process_definition_id);
        }
        self.bpmn_model_cache.invalidate(deployment_id);
        self.resource_cache
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .retain(|(dep_id, _), _| dep_id != deployment_id);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::persistence::db_store::DbStore;

    fn sample_timer_subscription() -> ProcessTimerStartSubscription {
        ProcessTimerStartSubscription {
            id: "timer-sub-1".to_string(),
            process_definition_id: "process-def-1".to_string(),
            process_definition_key: "process-key-1".to_string(),
            start_event_id: "start-event-1".to_string(),
            start_event_name: Some("Timer Start".to_string()),
            interrupting: true,
            time_duration: Some("PT10S".to_string()),
            time_date: None,
            time_cycle: None,
            end_date: None,
            calendar_name: None,
            due_time: Some(1_000),
            lock_owner: None,
            lock_time: None,
            category: None,
        }
    }

    #[test]
    fn release_process_timer_start_subscription_requires_matching_owner() {
        let manager = DeploymentManager::new_with_memory_backend_for_test(Arc::new(
            DbStore::new_in_memory().unwrap(),
        ));
        let mut session = manager.create_session().unwrap();
        let original = sample_timer_subscription();
        manager.register_timer_start_subscriptions(vec![original.clone()], &mut session);

        let (acquired, _, _) = manager
            .acquire_due_process_timer_start_subscriptions("owner-a", 2_000, 0, &mut session)
            .expect("timer start subscription acquisition must read storage");
        assert_eq!(acquired.len(), 1);
        let locked = acquired[0].clone();
        assert_eq!(locked.id, original.id);
        assert_eq!(locked.lock_owner.as_deref(), Some("owner-a"));

        let mut wrong_owner = locked.clone();
        wrong_owner.lock_owner = Some("owner-b".to_string());
        manager.release_process_timer_start_subscription(&wrong_owner, &mut session);

        let after_wrong_release = manager
            .get_timer_start_subscriptions(&mut session)
            .expect("timer start subscription read must succeed");
        assert_eq!(after_wrong_release.len(), 1);
        assert_eq!(after_wrong_release[0].id, original.id);
        assert_eq!(
            after_wrong_release[0].lock_owner.as_deref(),
            Some("owner-a")
        );

        manager.release_process_timer_start_subscription(&locked, &mut session);

        let after_correct_release = manager
            .get_timer_start_subscriptions(&mut session)
            .expect("timer start subscription read must succeed");
        assert_eq!(after_correct_release.len(), 1);
        assert_eq!(after_correct_release[0].id, original.id);
        assert!(after_correct_release[0].lock_owner.is_none());
        assert!(after_correct_release[0].lock_time.is_none());
        assert!(after_correct_release[0].due_time.is_none());
        session.rollback().unwrap();
    }
}
