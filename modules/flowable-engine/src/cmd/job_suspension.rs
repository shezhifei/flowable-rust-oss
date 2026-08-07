use crate::agenda::continue_process_operation::{
    ASYNC_AFTER_JOB_TYPE_MARKER, ASYNC_CONTINUATION_JOB_TYPE_MARKER,
};
use crate::error::FlowableError;
use crate::interceptor::command_context::CommandContext;
use crate::persistence::runtime_store::{RuntimeJobType, RuntimeTimerJobState};

#[derive(Clone, Copy)]
pub(crate) enum SuspendedJobActivation {
    Java,
    Extension { retries: i32 },
}

pub(crate) fn suspend_jobs_for_process_instance(
    command_context: &mut CommandContext,
    process_instance_id: &str,
) -> Result<usize, FlowableError> {
    let store = command_context.runtime_store_handle();
    let jobs = store
        .snapshot_timer_job_states(&mut command_context.session)
        .into_values()
        .filter(|job| job.process_instance_id == process_instance_id && is_suspendable_job(job))
        .collect::<Vec<_>>();

    for job in &jobs {
        suspend_job(command_context, job.clone())?;
    }
    Ok(jobs.len())
}

pub(crate) fn activate_suspended_jobs_for_process_instance(
    command_context: &mut CommandContext,
    process_instance_id: &str,
) -> Result<usize, FlowableError> {
    let store = command_context.runtime_store_handle();
    let jobs = store
        .snapshot_timer_job_states(&mut command_context.session)
        .into_values()
        .filter(|job| job.process_instance_id == process_instance_id && is_suspended_job(job))
        .collect::<Vec<_>>();

    for job in &jobs {
        activate_suspended_job(command_context, job.clone(), SuspendedJobActivation::Java)?;
    }
    Ok(jobs.len())
}

pub(crate) fn activate_suspended_job(
    command_context: &mut CommandContext,
    mut job: RuntimeTimerJobState,
    activation: SuspendedJobActivation,
) -> Result<RuntimeTimerJobState, FlowableError> {
    let store = command_context.runtime_store_handle();
    let job_type = resolve_job_type(&store, &mut command_context.session, &job);
    // Whether the reactivated job is eligible for the async executor (Java only
    // hints/pre-locks executable message jobs; timer and external-worker jobs
    // are picked up by their own acquisition paths). Determined by the resolved
    // activation state, mirroring `activateSuspendedJob` branching on job type.
    let mut async_eligible = false;
    match activation {
        SuspendedJobActivation::Java => {
            let state = activation_state(&job, job_type.as_ref());
            async_eligible = matches!(state, "async" | "async-after" | "executable");
            job.job_state = Some(state.to_string());
        }
        SuspendedJobActivation::Extension { retries } => {
            // Existing Rust extension: force executable + override retries.
            // Preserves the original extension contract (no coordinator
            // pre-lock or hint; the polling acquisition owns this path).
            job.job_state = Some("executable".to_string());
            job.retries = Some(retries);
        }
    }
    clear_job_lock(&mut job);

    // Java parity (`DefaultJobManager.activateSuspendedJob` ->
    // `createExecutableJobFromOtherJob` + `triggerExecutorIfNeeded`): when the
    // async executor is *live*, pre-lock the row inside this transaction so the
    // executor owns it on commit. The pre-lock happens regardless of category
    // (Java locks based on `isAsyncExecutorActive()` alone). Category only
    // decides whether the committed job is *hinted* to the executor.
    let mut hint_job: Option<RuntimeTimerJobState> = None;
    if let SuspendedJobActivation::Java = activation {
        if async_eligible {
            let coordinator = command_context.activation_coordinator().clone();
            if coordinator.is_active() && coordinator.tenant_enabled(None) {
                let now = store.time_source().now().timestamp_millis();
                job.lock_owner = Some(coordinator.lock_owner());
                job.lock_time = Some(now);
                job.lock_expiration_time =
                    Some(now.saturating_add(coordinator.async_job_lock_ms()));
                // Hint only when the job category is enabled (Java
                // `isJobApplicableForExecutorExecution`); a category mismatch
                // still pre-locks but leaves the hint to another node.
                if coordinator.category_enabled_for_hint(job.category.as_deref()) {
                    hint_job = Some(job.clone());
                }
            }
        }
    }

    store.insert_timer_job_state_with_type(&job, job_type.as_ref(), &mut command_context.session);

    // Register the post-commit hint only after the row is persisted. The
    // command executor drains pending hints once the transaction commits,
    // mirroring Java's `COMMITTED` `JobAddedTransactionListener`; on rollback
    // the hints are discarded with the command context.
    if let Some(hint) = hint_job {
        command_context.register_pending_async_hint(hint);
    }
    Ok(job)
}

fn suspend_job(
    command_context: &mut CommandContext,
    mut job: RuntimeTimerJobState,
) -> Result<(), FlowableError> {
    let store = command_context.runtime_store_handle();
    let job_type = resolve_job_type(&store, &mut command_context.session, &job);
    job.job_state = Some("suspended".to_string());
    clear_job_lock(&mut job);
    store.insert_timer_job_state_with_type(&job, job_type.as_ref(), &mut command_context.session);
    Ok(())
}

fn resolve_job_type(
    store: &crate::persistence::runtime_store::RuntimeStore,
    session: &mut crate::persistence::db_session::DbSession,
    job: &RuntimeTimerJobState,
) -> Option<RuntimeJobType> {
    // Prefer the persisted job_type column. Never infer ExternalWorker from
    // event-wait alone — that misclassified ordinary intermediate timers.
    // Untyped suspended/deadletter rows stay untyped (activation falls back to
    // executable); real timers are typed as Timer while still in timer state
    // before suspension.
    store
        .find_timer_job_type(&job.timer_job_id, session)
        .or_else(|| match job.job_state.as_deref() {
            None | Some("timer") => Some(RuntimeJobType::Timer),
            Some("executable") | Some("async") | Some("async-after") => {
                Some(RuntimeJobType::Other("message".to_string()))
            }
            Some(_) => None,
        })
}

fn activation_state(job: &RuntimeTimerJobState, job_type: Option<&RuntimeJobType>) -> &'static str {
    match job_type {
        Some(RuntimeJobType::Timer | RuntimeJobType::ExternalWorker) => "timer",
        _ if job.time_duration.as_deref() == Some(ASYNC_CONTINUATION_JOB_TYPE_MARKER) => "async",
        _ if job.time_duration.as_deref() == Some(ASYNC_AFTER_JOB_TYPE_MARKER) => "async-after",
        _ => "executable",
    }
}

fn is_suspendable_job(job: &RuntimeTimerJobState) -> bool {
    matches!(
        job.job_state.as_deref(),
        None | Some("timer") | Some("executable") | Some("async") | Some("async-after")
    )
}

fn is_suspended_job(job: &RuntimeTimerJobState) -> bool {
    job.job_state.as_deref() == Some("suspended")
}

fn clear_job_lock(job: &mut RuntimeTimerJobState) {
    job.lock_owner = None;
    job.lock_time = None;
    job.lock_expiration_time = None;
}
