use crate::engine::timer_worker::TimerWork;
use crate::interceptor::command::Command;
use crate::interceptor::command_context::CommandContext;

use crate::engine::timer_worker::TimerCoordinationMetrics;
use std::sync::Arc;

pub struct RenewTimerLeaseCmd {
    work: TimerWork,
    owner_id: Arc<str>,
    fencing_token: i64,
    metrics: Arc<TimerCoordinationMetrics>,
}

impl RenewTimerLeaseCmd {
    pub fn new(
        work: TimerWork,
        owner_id: Arc<str>,
        fencing_token: i64,
        metrics: Arc<TimerCoordinationMetrics>,
    ) -> Self {
        Self {
            work,
            owner_id,
            fencing_token,
            metrics,
        }
    }
}

impl Command<()> for RenewTimerLeaseCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<(), crate::error::FlowableError> {
        let store = command_context.runtime_store_handle();
        let _dm = command_context.deployment_manager_handle();
        // Extend the authoritative expiration with the configured lock duration so
        // reset does not treat a renewed lease as still expired.
        let lock_duration_ms = command_context.config.async_executor.timer_lock_time_ms as i64;
        let session = command_context.session();

        let lease_opt = store.find_timer_coordinator_lease("timer-coordinator", session);
        if let Some(lease) = lease_opt {
            if lease.fencing_token != self.fencing_token
                || lease.owner_node_id != self.owner_id.as_ref()
            {
                self.metrics
                    .renew_misses
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                return Ok(());
            }
        } else {
            self.metrics
                .renew_misses
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            return Ok(());
        }

        let now = store.time_source().now().timestamp_millis();
        let new_expiration = now.saturating_add(lock_duration_ms);

        let affected = match &self.work {
            TimerWork::RuntimeJob(job) => {
                let mut updated = job.clone();
                updated.lock_time = Some(now);
                updated.lock_expiration_time = Some(new_expiration);
                let json = serde_json::to_string(&updated).unwrap();
                session.cas_update(
                    "timer_job_states",
                    &job.timer_job_id,
                    &json,
                    &[
                        ("lock_time".into(), Some(now.to_string())),
                        (
                            "lock_expiration_time".into(),
                            Some(new_expiration.to_string()),
                        ),
                        ("due_time".into(), updated.due_time.map(|v| v.to_string())),
                        ("job_state".into(), updated.job_state.clone()),
                    ],
                    &[("lock_owner".into(), Some(self.owner_id.to_string()))],
                )
            }
            TimerWork::ProcessStart(sub) => {
                let mut updated = sub.clone();
                updated.lock_time = Some(now);
                let json = serde_json::to_string(&updated).unwrap();
                session.cas_update(
                    "process_timer_start_subscriptions",
                    &sub.id,
                    &json,
                    &[("lock_time".into(), Some(now.to_string()))],
                    &[("lock_owner".into(), Some(self.owner_id.to_string()))],
                )
            }
            TimerWork::EventSubprocess(sub) => {
                let mut updated = sub.clone();
                updated.lock_time = Some(now);
                let json = serde_json::to_string(&updated).unwrap();
                session.cas_update(
                    "event_subprocess_timer_subscriptions",
                    &sub.subscription_id,
                    &json,
                    &[("lock_time".into(), Some(now.to_string()))],
                    &[("lock_owner".into(), Some(self.owner_id.to_string()))],
                )
            }
        }
        .unwrap();

        if affected > 0 {
            self.metrics
                .renew_successes
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        } else {
            self.metrics
                .renew_misses
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        Ok(())
    }
}
