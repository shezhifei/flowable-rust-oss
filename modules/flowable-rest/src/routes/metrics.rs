use axum::{extract::Extension, http::StatusCode, response::IntoResponse};
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::engine::timer_worker::TimerCoordinationMetrics;
use flowable_engine::persistence::db_session::DbParams;
use std::sync::atomic::Ordering;
use std::sync::Arc;

pub async fn metrics(Extension(engine): Extension<Arc<ProcessEngine>>) -> impl IntoResponse {
    let runtime_store = engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();

    let process_instances: i64 = session
        .raw_query_one(
            "SELECT COUNT(*) AS RES_ FROM process_instances",
            DbParams::new(),
        )
        .unwrap()
        .and_then(|r| r.get_integer("RES_"))
        .unwrap_or(0);

    let tasks: i64 = session
        .raw_query_one(
            "SELECT COUNT(*) AS RES_ FROM historic_task_instances",
            DbParams::new(),
        )
        .unwrap()
        .and_then(|r| r.get_integer("RES_"))
        .unwrap_or(0);

    let timer = engine.get_runtime_service().timer_metrics();
    let job_metrics = format_job_lifecycle_metrics(timer.as_ref());

    let output = format!(
        "# HELP flowable_process_instances_total Total number of active process instances\n\
         # TYPE flowable_process_instances_total gauge\n\
         flowable_process_instances_total {}\n\
         # HELP flowable_tasks_total Total number of tasks\n\
         # TYPE flowable_tasks_total gauge\n\
         flowable_tasks_total {}\n\
         {}",
        process_instances, tasks, job_metrics
    );

    (StatusCode::OK, output)
}

/// Prometheus export of async/timer acquire + coordinator counters.
/// Mirrors management timer ledger metrics (`timer_metrics_response`) and the
/// Java `AcquireAsyncJobsDueLifecycleListener` hook surface (batch size,
/// conflicts, execute outcome).
fn format_job_lifecycle_metrics(m: &TimerCoordinationMetrics) -> String {
    let load = |c: &std::sync::atomic::AtomicUsize| c.load(Ordering::Relaxed);
    format!(
        "# HELP flowable_job_acquire_attempts_total Candidates considered during job acquire\n\
         # TYPE flowable_job_acquire_attempts_total counter\n\
         flowable_job_acquire_attempts_total {}\n\
         # HELP flowable_job_acquire_conflicts_total Acquire races lost (optimistic / exclusive scope)\n\
         # TYPE flowable_job_acquire_conflicts_total counter\n\
         flowable_job_acquire_conflicts_total {}\n\
         # HELP flowable_job_acquired_total Jobs successfully locked for execution\n\
         # TYPE flowable_job_acquired_total counter\n\
         flowable_job_acquired_total {}\n\
         # HELP flowable_job_acquire_batch_size Size of the most recent acquire batch\n\
         # TYPE flowable_job_acquire_batch_size gauge\n\
         flowable_job_acquire_batch_size {}\n\
         # HELP flowable_timer_lease_renew_successes_total Successful timer lease renewals\n\
         # TYPE flowable_timer_lease_renew_successes_total counter\n\
         flowable_timer_lease_renew_successes_total {}\n\
         # HELP flowable_timer_lease_renew_misses_total Missed timer lease renewals\n\
         # TYPE flowable_timer_lease_renew_misses_total counter\n\
         flowable_timer_lease_renew_misses_total {}\n\
         # HELP flowable_timer_expired_lease_recoveries_total Expired coordinator lease recoveries\n\
         # TYPE flowable_timer_expired_lease_recoveries_total counter\n\
         flowable_timer_expired_lease_recoveries_total {}\n\
         # HELP flowable_job_execute_total Successful job executions by kind\n\
         # TYPE flowable_job_execute_total counter\n\
         flowable_job_execute_total{{kind=\"runtime_job\"}} {}\n\
         flowable_job_execute_total{{kind=\"process_start\"}} {}\n\
         flowable_job_execute_total{{kind=\"event_subprocess\"}} {}\n\
         # HELP flowable_job_execute_failures_total Automatic executor job execution failures\n\
         # TYPE flowable_job_execute_failures_total counter\n\
         flowable_job_execute_failures_total {}\n",
        load(&m.acquire_attempts),
        load(&m.acquire_conflicts),
        load(&m.jobs_acquired),
        load(&m.last_acquire_batch_size),
        load(&m.renew_successes),
        load(&m.renew_misses),
        load(&m.expired_lease_recoveries),
        load(&m.execute_count_runtime_job),
        load(&m.execute_count_process_start),
        load(&m.execute_count_event_subprocess),
        load(&m.execute_failures),
    )
}
