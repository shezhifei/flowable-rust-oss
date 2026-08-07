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
            .unwrap()
            .insert(process_definition_id.to_string(), Arc::new(model));
    }

    pub fn get_bpmn_model(&self, process_definition_id: &str) -> Option<Arc<BpmnModel>> {
        self.bpmn_models
            .lock()
            .unwrap()
            .get(process_definition_id)
            .cloned()
    }

    pub fn contains_bpmn_model(&self, process_definition_id: &str) -> bool {
        self.bpmn_models
            .lock()
            .unwrap()
            .contains_key(process_definition_id)
    }

    pub fn remove_bpmn_model(&self, process_definition_id: &str) {
        self.bpmn_models
            .lock()
            .unwrap()
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
            session
                .insert_with_extra(
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
                )
                .unwrap();
        }
        let _ = session.flush();
    }

    pub fn get_timer_start_subscriptions(
        &self,
        session: &mut DbSession,
    ) -> Vec<ProcessTimerStartSubscription> {
        let mut rows = session
            .find_raw_all("process_timer_start_subscriptions")
            .unwrap();

        rows.sort_by(|a, b| a.id.cmp(&b.id));
        rows.into_iter()
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
            .collect()
    }

    pub fn acquire_due_process_timer_start_subscriptions(
        &self,
        owner: &str,
        now: i64,
        lock_timeout_ms: i64,
        session: &mut DbSession,
    ) -> (Vec<ProcessTimerStartSubscription>, usize, usize) {
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
    ) -> Vec<ProcessTimerStartSubscription> {
        // Expired locks require reset; acquisition only selects unlocked rows.
        let has_category_filter = category_filter.map(|f| !f.is_empty()).unwrap_or(false);
        let mut candidates: Vec<_> = self
            .get_timer_start_subscriptions(session)
            .into_iter()
            .filter(|t| t.due_time.is_some() && t.due_time.unwrap() <= now)
            .filter(|t| t.lock_owner.is_none())
            .filter(|t| {
                if !has_category_filter {
                    return true;
                }
                t.category
                    .as_ref()
                    .map(|cat| category_filter.unwrap().contains(cat))
                    .unwrap_or(false)
            })
            .collect();
        candidates.sort_by(|a, b| {
            a.due_time
                .unwrap()
                .cmp(&b.due_time.unwrap())
                .then(a.id.cmp(&b.id))
        });
        candidates
    }

    pub(crate) fn acquire_selected_process_timer_start_subscriptions(
        &self,
        owner: &str,
        now: i64,
        lock_timeout_ms: i64,
        selected_subscription_ids: &[String],
        category_filter: Option<&[String]>,
        session: &mut DbSession,
    ) -> (Vec<ProcessTimerStartSubscription>, usize, usize) {
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
        );
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
    ) -> (Vec<ProcessTimerStartSubscription>, usize, usize) {
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
    ) -> (Vec<ProcessTimerStartSubscription>, usize, usize) {
        let mut candidates = self.find_due_process_timer_start_subscription_candidates(
            now,
            lock_timeout_ms,
            category_filter,
            session,
        );
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

            let json = serde_json::to_string(&t).unwrap_or_else(|_| "{}".to_string());
            let affected = if let Some(old_owner) = old_lock_owner {
                session
                    .cas_update(
                        "process_timer_start_subscriptions",
                        &t.id,
                        &json,
                        &[
                            ("lock_owner".into(), Some(owner.to_string())),
                            ("lock_time".into(), Some(now.to_string())),
                        ],
                        &[
                            ("lock_owner".into(), Some(old_owner)),
                            ("lock_time".into(), Some(old_lock_time.unwrap().to_string())),
                        ],
                    )
                    .unwrap()
            } else {
                session
                    .cas_update(
                        "process_timer_start_subscriptions",
                        &t.id,
                        &json,
                        &[
                            ("lock_owner".into(), Some(owner.to_string())),
                            ("lock_time".into(), Some(now.to_string())),
                        ],
                        &[("lock_owner".into(), None)],
                    )
                    .unwrap()
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
        (acquired, recovered, conflicts)
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
        let json = serde_json::to_string(&updated_sub).unwrap_or_else(|_| "{}".to_string());
        if let (Some(owner), Some(lock_time)) = (sub.lock_owner.as_deref(), sub.lock_time) {
            session
                .cas_update(
                    "process_timer_start_subscriptions",
                    &sub.id,
                    &json,
                    &[("lock_owner".into(), None), ("lock_time".into(), None)],
                    &[
                        ("lock_owner".into(), Some(owner.to_string())),
                        ("lock_time".into(), Some(lock_time.to_string())),
                    ],
                )
                .unwrap();
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
        let json = serde_json::to_string(&updated_sub).unwrap_or_else(|_| "{}".to_string());
        if let (Some(owner), Some(lock_time)) = (sub.lock_owner.as_deref(), sub.lock_time) {
            session
                .cas_update(
                    "process_timer_start_subscriptions",
                    &sub.id,
                    &json,
                    &[("lock_owner".into(), None), ("lock_time".into(), None)],
                    &[
                        ("lock_owner".into(), Some(owner.to_string())),
                        ("lock_time".into(), Some(lock_time.to_string())),
                    ],
                )
                .unwrap();
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
        session
            .delete_by(
                "process_timer_start_subscriptions",
                "process_definition_id",
                process_definition_id,
            )
            .unwrap();
    }

    /// Java `TimerManager.removeObsoleteTimers`: cancel timer-start subscriptions
    /// for all versions of a process-definition key (and matching tenant).
    pub fn delete_timer_start_subscriptions_by_process_definition_key(
        &self,
        process_definition_key: &str,
        tenant_id: Option<&str>,
        session: &mut DbSession,
    ) {
        let defs = self.get_process_definitions(session);
        let to_delete: Vec<String> = self
            .get_timer_start_subscriptions(session)
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
            let _ = session.delete("process_timer_start_subscriptions", &id);
        }
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
            session
                .insert_with_extra(
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
                )
                .unwrap();
        }
        let _ = session.flush();
    }

    pub fn get_event_start_subscriptions(
        &self,
        session: &mut DbSession,
    ) -> Vec<ProcessEventStartSubscription> {
        session
            .find_all::<ProcessEventStartSubscription>("process_event_start_subscriptions")
            .unwrap_or_default()
    }

    pub fn delete_event_start_subscriptions_by_process_definition_id(
        &self,
        process_definition_id: &str,
        session: &mut DbSession,
    ) {
        session
            .delete_by(
                "process_event_start_subscriptions",
                "process_definition_id",
                process_definition_id,
            )
            .unwrap();
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
    ) {
        let rows = session
            .find_raw_all("process_event_start_subscriptions")
            .unwrap_or_default();
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
                let _ = session.delete("process_event_start_subscriptions", &row.id);
            }
        }
    }

    pub fn find_event_start_subscriptions_by_event_ref(
        &self,
        event_kind: &EventSubscriptionKind,
        event_ref: &str,
        session: &mut DbSession,
    ) -> Vec<ProcessEventStartSubscription> {
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
        session
            .find_by_two(
                "process_event_start_subscriptions",
                "event_kind",
                kind_str,
                "event_ref",
                event_ref,
            )
            .unwrap()
    }

    pub fn next_process_definition_version(
        &self,
        tenant_id: Option<&str>,
        process_key: &str,
        session: &mut DbSession,
    ) -> i32 {
        let tenant_str = tenant_id.unwrap_or("");
        session
            .next_process_definition_version(tenant_str, process_key)
            .unwrap_or_else(|error| {
                tracing::warn!("next_process_definition_version failed: {error}");
                1
            })
    }

    pub fn register_deployment(&self, deployment: Deployment, session: &mut DbSession) {
        let deployment_id = deployment.id.clone();
        let created_at = deployment
            .deployment_time
            .map(|value| value.timestamp_millis())
            .unwrap_or_default();

        let mut deployment_no_resources = deployment.clone();
        deployment_no_resources.resources.clear();
        session
            .insert("deployments", &deployment_id, &deployment_no_resources)
            .unwrap();

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
            session
                .inner_mut()
                .execute(StatementId::DeleteDeployment, params)
                .unwrap_or_else(|err| {
                    panic!(
                        "dual-write pre-delete ACT_RE_DEPLOYMENT failed for id={}: {err}",
                        entity.id
                    )
                });
        }
        flowable_persistence::DeploymentDataManager::new()
            .insert(session.inner_mut(), entity)
            .unwrap_or_else(|err| {
                panic!("dual-write ACT_RE_DEPLOYMENT insert failed for id={deployment_id}: {err}")
            });
        // DataManager insert only queues; flush so SQL failures surface as dual-write errors.
        session.inner_mut().flush().unwrap_or_else(|err| {
            panic!("dual-write ACT_RE_DEPLOYMENT flush failed for id={deployment_id}: {err}")
        });

        for (name, bytes) in &deployment.resources {
            let resource = DeploymentResource::new(
                deployment_id.clone(),
                name.clone(),
                bytes.clone(),
                created_at,
            );
            session
                .upsert_deployment_resource(
                    &resource.deployment_id,
                    &resource.resource_name,
                    &resource.resource_type,
                    &resource.content_type,
                    &resource.bytes,
                    resource.created_at,
                )
                .unwrap_or_else(|error| {
                    tracing::warn!("upsert_deployment_resource failed: {error}");
                });

            // Dual-write resource bytes into ACT_GE_BYTEARRAY / deployment resource statements.
            // 先删除可能存在的旧记录，避免 UNIQUE 约束冲突。
            flowable_persistence::DeploymentResourceDataManager::new()
                .delete_by_deployment_id_and_name(session.inner_mut(), &deployment_id, name)
                .unwrap_or_else(|err| {
                    panic!(
                        "dual-write pre-delete ACT_GE_BYTEARRAY failed for deployment={deployment_id} name={name}: {err}"
                    )
                });
            let mut byte_entity =
                flowable_persistence::ByteArrayEntity::new(format!("{deployment_id}:{name}"));
            byte_entity.name = Some(name.clone());
            byte_entity.deployment_id = Some(deployment_id.clone());
            byte_entity.bytes = Some(bytes.clone());
            flowable_persistence::DeploymentResourceDataManager::new()
                .insert(session.inner_mut(), byte_entity)
                .unwrap_or_else(|err| {
                    panic!(
                        "dual-write ACT_GE_BYTEARRAY insert failed for deployment={deployment_id} name={name}: {err}"
                    )
                });
            session.inner_mut().flush().unwrap_or_else(|err| {
                panic!(
                    "dual-write ACT_GE_BYTEARRAY flush failed for deployment={deployment_id} name={name}: {err}"
                )
            });

            let key = (deployment_id.clone(), name.clone());
            self.resource_cache
                .write()
                .unwrap()
                .insert(key, Arc::new(bytes.clone()));
        }
        // Hard-fail flush of remaining JSON-path work after dual-write (P73a).
        session.flush().unwrap_or_else(|err| {
            panic!("flush after dual-write deployment failed for id={deployment_id}: {err}")
        });
    }

    pub fn get_deployment(
        &self,
        deployment_id: &str,
        session: &mut DbSession,
    ) -> Option<Deployment> {
        self.get_deployments(session).remove(deployment_id)
    }

    pub fn get_deployment_resource_names(
        &self,
        deployment_id: &str,
        session: &mut DbSession,
    ) -> Vec<String> {
        session
            .list_deployment_resource_names(deployment_id)
            .unwrap_or_else(|error| {
                tracing::warn!("list_deployment_resource_names failed: {error}");
                Vec::new()
            })
    }

    pub fn get_deployment_resources(
        &self,
        deployment_id: &str,
        session: &mut DbSession,
    ) -> Vec<DeploymentResource> {
        session
            .list_deployment_resources(deployment_id)
            .unwrap_or_else(|error| {
                tracing::warn!("list_deployment_resources failed: {error}");
                Vec::new()
            })
    }

    pub fn get_deployment_resource(
        &self,
        deployment_id: &str,
        name: &str,
        session: &mut DbSession,
    ) -> Option<DeploymentResource> {
        session
            .find_deployment_resource(deployment_id, name)
            .unwrap_or_else(|error| {
                tracing::warn!("find_deployment_resource failed: {error}");
                None
            })
    }

    pub fn get_deployment_resource_bytes(
        &self,
        deployment_id: &str,
        name: &str,
        session: &mut DbSession,
    ) -> Option<Vec<u8>> {
        let key = (deployment_id.to_string(), name.to_string());
        {
            let read = self
                .resource_cache
                .read()
                .unwrap_or_else(|e| e.into_inner());
            if let Some(bytes) = read.get(&key) {
                return Some(bytes.as_ref().clone());
            }
        }
        let bytes = session
            .find_blob_by_two(
                "deployment_resources",
                "deployment_id",
                deployment_id,
                "name",
                name,
                "bytes",
            )
            .ok()??;
        self.resource_cache
            .write()
            .unwrap()
            .insert(key, Arc::new(bytes.clone()));
        Some(bytes)
    }

    pub fn get_deployments(&self, session: &mut DbSession) -> HashMap<String, Deployment> {
        let mut map: HashMap<String, Deployment> = session
            .find_all::<Deployment>("deployments")
            .unwrap_or_default()
            .into_iter()
            .map(|d| (d.id.clone(), d))
            .collect();
        let rows = session
            .iter_all_deployment_resource_bytes()
            .unwrap_or_else(|error| {
                tracing::warn!("iter_all_deployment_resource_bytes failed: {error}");
                Vec::new()
            });
        for (dep_id, name, bytes) in rows {
            if let Some(d) = map.get_mut(&dep_id) {
                d.resources.insert(name, bytes);
            }
        }
        map
    }

    pub fn get_process_definitions(
        &self,
        session: &mut DbSession,
    ) -> HashMap<String, ProcessDefinition> {
        let mut map = HashMap::new();
        for pd in session
            .find_all::<ProcessDefinition>("process_definitions")
            .unwrap_or_default()
        {
            map.insert(pd.id.clone(), pd);
        }
        map
    }

    pub fn insert_process_definition(&self, pd: ProcessDefinition, session: &mut DbSession) {
        session
            .insert_with_extra(
                "process_definitions",
                &pd.id,
                &pd,
                &[(
                    "deployment_id".into(),
                    Some(pd.deployment_id.clone().unwrap_or_default()),
                )],
            )
            .unwrap();

        // ADR-0001 Phase 5: dual-write normalized ACT_RE_PROCDEF via DataManager.
        // 立即执行 DELETE（flush 顺序 INSERT 先于 DELETE，queued delete 会导致 UNIQUE 冲突）。
        // Hard-fail dual-write errors (P73a).
        let entity = crate::persistence::entity_mapping::process_definition_to_entity(&pd);
        {
            use flowable_persistence::statement::StatementId;
            use flowable_persistence::value::DbParams;
            let mut params = DbParams::new();
            params.push(entity.id.clone());
            session
                .inner_mut()
                .execute(StatementId::DeleteProcessDefinition, params)
                .unwrap_or_else(|err| {
                    panic!(
                        "dual-write pre-delete ACT_RE_PROCDEF failed for id={}: {err}",
                        entity.id
                    )
                });
        }
        flowable_persistence::ProcessDefinitionDataManager::new()
            .insert(session.inner_mut(), entity)
            .unwrap_or_else(|err| {
                panic!("dual-write ACT_RE_PROCDEF insert failed for id={}: {err}", pd.id)
            });
        session.inner_mut().flush().unwrap_or_else(|err| {
            panic!(
                "dual-write ACT_RE_PROCDEF flush failed for id={}: {err}",
                pd.id
            )
        });

        session.flush().unwrap_or_else(|err| {
            panic!(
                "flush after dual-write process definition failed for id={}: {err}",
                pd.id
            )
        });
    }

    pub fn update_process_definition(
        &self,
        pd: ProcessDefinition,
        session: &mut DbSession,
    ) -> Option<()> {
        self.get_process_definitions(session).get(&pd.id)?;
        self.insert_process_definition(pd, session);
        Some(())
    }

    pub fn insert_repository_model(
        &self,
        model: RepositoryModel,
        source_bytes: Vec<u8>,
        source_extra_bytes: Vec<u8>,
        session: &mut DbSession,
    ) {
        let data_json = serde_json::to_string(&model).unwrap_or_else(|_| "{}".to_string());
        let dep_id = model.deployment_id.as_deref().unwrap_or("");
        let tenant = model.tenant_id.as_deref().unwrap_or("");
        session
            .insert_repository_model(
                &model.id,
                &data_json,
                dep_id,
                &model.key,
                tenant,
                &source_bytes,
                &source_extra_bytes,
            )
            .unwrap_or_else(|error| {
                tracing::warn!("insert_repository_model failed: {error}");
            });
    }

    pub fn get_repository_models(&self, session: &mut DbSession) -> Vec<RepositoryModel> {
        let mut models = session
            .find_all::<RepositoryModel>("repository_models")
            .unwrap_or_default();
        models.sort_by(|left, right| left.key.cmp(&right.key).then(left.id.cmp(&right.id)));
        models
    }

    pub fn get_repository_model(
        &self,
        model_id: &str,
        session: &mut DbSession,
    ) -> Option<RepositoryModel> {
        session.find("repository_models", model_id).unwrap()
    }

    pub fn update_repository_model(
        &self,
        model: RepositoryModel,
        session: &mut DbSession,
    ) -> Option<()> {
        self.get_repository_model(&model.id, session)?;
        let data_json = serde_json::to_string(&model).unwrap_or_else(|_| "{}".to_string());
        let dep_id = model.deployment_id.as_deref().unwrap_or("");
        let tenant = model.tenant_id.as_deref().unwrap_or("");
        session
            .update_repository_model_data(&model.id, &data_json, dep_id, &model.key, tenant)
            .unwrap_or_else(|error| {
                tracing::warn!("update_repository_model_data failed: {error}");
            });
        Some(())
    }

    pub fn update_repository_model_source(
        &self,
        model: RepositoryModel,
        source_bytes: Vec<u8>,
        session: &mut DbSession,
    ) -> Option<()> {
        self.update_repository_model_blob(session, model, RepositoryModelBlob::Source, source_bytes)
    }

    pub fn update_repository_model_source_extra(
        &self,
        model: RepositoryModel,
        source_extra_bytes: Vec<u8>,
        session: &mut DbSession,
    ) -> Option<()> {
        self.update_repository_model_blob(
            session,
            model,
            RepositoryModelBlob::SourceExtra,
            source_extra_bytes,
        )
    }

    pub fn delete_repository_model(&self, model_id: &str, session: &mut DbSession) -> bool {
        let prev = session
            .find::<RepositoryModel>("repository_models", model_id)
            .unwrap();
        let _ = session.delete("repository_models", model_id);
        let _ = session.flush();

        prev.is_some()
    }

    pub fn get_repository_model_source(
        &self,
        model_id: &str,
        session: &mut DbSession,
    ) -> Option<RepositoryModelBytes> {
        let model = self.get_repository_model(model_id, session)?;
        let bytes =
            self.get_repository_model_bytes(model_id, RepositoryModelBlob::Source, session)?;
        Some(RepositoryModelBytes {
            content_type: model.source_content_type,
            bytes,
        })
    }

    pub fn get_repository_model_source_extra(
        &self,
        model_id: &str,
        session: &mut DbSession,
    ) -> Option<RepositoryModelBytes> {
        let model = self.get_repository_model(model_id, session)?;
        let bytes =
            self.get_repository_model_bytes(model_id, RepositoryModelBlob::SourceExtra, session)?;
        Some(RepositoryModelBytes {
            content_type: model.source_extra_content_type,
            bytes,
        })
    }

    fn get_repository_model_bytes(
        &self,
        model_id: &str,
        blob: RepositoryModelBlob,
        session: &mut DbSession,
    ) -> Option<Vec<u8>> {
        let blob_col = match blob {
            RepositoryModelBlob::Source => "source_bytes",
            RepositoryModelBlob::SourceExtra => "source_extra_bytes",
        };
        session
            .find_blob("repository_models", "id", model_id, blob_col)
            .unwrap()
    }

    fn update_repository_model_blob(
        &self,
        session: &mut DbSession,
        model: RepositoryModel,
        blob: RepositoryModelBlob,
        bytes: Vec<u8>,
    ) -> Option<()> {
        self.get_repository_model(&model.id, session)?;
        let data_json = serde_json::to_string(&model).unwrap_or_else(|_| "{}".to_string());
        let dep_id = model.deployment_id.as_deref().unwrap_or("");
        let tenant = model.tenant_id.as_deref().unwrap_or("");
        let blob_col = match blob {
            RepositoryModelBlob::Source => "source_bytes",
            RepositoryModelBlob::SourceExtra => "source_extra_bytes",
        };
        session
            .update_repository_model_blob(
                &model.id, &data_json, dep_id, &model.key, tenant, blob_col, &bytes,
            )
            .unwrap_or_else(|error| {
                tracing::warn!("update_repository_model_blob failed: {error}");
            });
        Some(())
    }

    pub fn delete_deployment(&self, deployment_id: &str, session: &mut DbSession) {
        let _ = session.delete("deployments", deployment_id);
        session
            .delete_by("deployment_resources", "deployment_id", deployment_id)
            .unwrap();
        session
            .delete_by("repository_models", "deployment_id", deployment_id)
            .unwrap();

        // Dual-delete normalized ACT_* rows when present (P73a hard-fail on errors).
        flowable_persistence::DeploymentResourceDataManager::new()
            .delete_by_deployment_id(session.inner_mut(), deployment_id)
            .unwrap_or_else(|err| {
                panic!(
                    "dual-delete ACT_GE_BYTEARRAY by deployment failed for id={deployment_id}: {err}"
                )
            });
        match flowable_persistence::DeploymentDataManager::new()
            .find_by_id(session.inner_mut(), deployment_id)
        {
            Ok(Some(entity)) => {
                flowable_persistence::DeploymentDataManager::new()
                    .delete(session.inner_mut(), &entity)
                    .unwrap_or_else(|err| {
                        panic!(
                            "dual-delete ACT_RE_DEPLOYMENT failed for id={deployment_id}: {err}"
                        )
                    });
            }
            Ok(None) => {}
            Err(err) => {
                panic!(
                    "dual-delete ACT_RE_DEPLOYMENT find_by_id failed for id={deployment_id}: {err}"
                );
            }
        }

        let process_definitions: Vec<ProcessDefinition> = session
            .find_by("process_definitions", "deployment_id", deployment_id)
            .unwrap();

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
            session
                .delete("process_definitions", &process_definition_id)
                .unwrap();
            match flowable_persistence::ProcessDefinitionDataManager::new()
                .find_by_id(session.inner_mut(), &process_definition_id)
            {
                Ok(Some(entity)) => {
                    flowable_persistence::ProcessDefinitionDataManager::new()
                        .delete(session.inner_mut(), &entity)
                        .unwrap_or_else(|err| {
                            panic!(
                                "dual-delete ACT_RE_PROCDEF failed for id={process_definition_id}: {err}"
                            )
                        });
                }
                Ok(None) => {}
                Err(err) => {
                    panic!(
                        "dual-delete ACT_RE_PROCDEF find_by_id failed for id={process_definition_id}: {err}"
                    );
                }
            }
            self.remove_bpmn_model(&process_definition_id);
        }
        self.bpmn_model_cache.invalidate(deployment_id);
        self.resource_cache
            .write()
            .unwrap()
            .retain(|(dep_id, _), _| dep_id != deployment_id);
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

        let (acquired, _, _) = manager.acquire_due_process_timer_start_subscriptions(
            "owner-a",
            2_000,
            0,
            &mut session,
        );
        assert_eq!(acquired.len(), 1);
        let locked = acquired[0].clone();
        assert_eq!(locked.id, original.id);
        assert_eq!(locked.lock_owner.as_deref(), Some("owner-a"));

        let mut wrong_owner = locked.clone();
        wrong_owner.lock_owner = Some("owner-b".to_string());
        manager.release_process_timer_start_subscription(&wrong_owner, &mut session);

        let after_wrong_release = manager.get_timer_start_subscriptions(&mut session);
        assert_eq!(after_wrong_release.len(), 1);
        assert_eq!(after_wrong_release[0].id, original.id);
        assert_eq!(
            after_wrong_release[0].lock_owner.as_deref(),
            Some("owner-a")
        );

        manager.release_process_timer_start_subscription(&locked, &mut session);

        let after_correct_release = manager.get_timer_start_subscriptions(&mut session);
        assert_eq!(after_correct_release.len(), 1);
        assert_eq!(after_correct_release[0].id, original.id);
        assert!(after_correct_release[0].lock_owner.is_none());
        assert!(after_correct_release[0].lock_time.is_none());
        assert!(after_correct_release[0].due_time.is_none());
        session.rollback().unwrap();
    }
}
