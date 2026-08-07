//! Java-compatible CMMN job parent state resolver.
//!
//! Baseline: `org.flowable.cmmn.engine.DefaultCmmnJobParentStateResolver`.

use crate::error::CmmnError;
use crate::models::{CMMN_SCOPE_TYPE, CmmnCaseInstanceState, CmmnJob};
use crate::runtime::load_case_instance_session;
use flowable_persistence::db_session::DbSession;

/// Typed parent-not-CMMN error (Java `FlowableIllegalArgumentException`).
/// Callers must match on [`CmmnError::Validation`], not message substrings alone.
pub fn parent_not_cmmn_error(job_id: &str) -> CmmnError {
    CmmnError::validation(format!("Job {job_id} parent is not CMMN case"))
}

/// Typed parent-suspended error (Java `JobServiceImpl.activateSuspendedJob`).
pub fn parent_suspended_error(job_id: &str) -> CmmnError {
    CmmnError::validation(format!(
        "Can not activate job {job_id}. Parent is suspended."
    ))
}

/// Java `DefaultCmmnJobParentStateResolver.isSuspended`.
///
/// - `scopeType` must be CMMN (missing defaults to CMMN for legacy rows).
/// - `scopeId` must be non-empty.
/// - Missing case instance is treated as parent-not-CMMN (typed validation error).
/// - Active/completed/terminated cases are not suspended.
/// - Suspended cases reject activation.
pub fn is_cmmn_job_parent_suspended(
    session: &mut DbSession,
    job: &CmmnJob,
) -> Result<bool, CmmnError> {
    let scope_type = job.scope_type.as_deref().unwrap_or(CMMN_SCOPE_TYPE);
    let scope_id = job.scope_id.as_deref().unwrap_or("").trim();
    if scope_type != CMMN_SCOPE_TYPE || scope_id.is_empty() {
        return Err(parent_not_cmmn_error(&job.id));
    }

    let case_instance = load_case_instance_session(session, scope_id)?.ok_or_else(|| {
        // Missing parent cannot be proven as a live CMMN case.
        parent_not_cmmn_error(&job.id)
    })?;

    Ok(case_instance.state == CmmnCaseInstanceState::Suspended)
}

/// Ensures the parent case allows suspended-job activation.
pub fn ensure_cmmn_job_parent_allows_activation(
    session: &mut DbSession,
    job: &CmmnJob,
) -> Result<(), CmmnError> {
    if is_cmmn_job_parent_suspended(session, job)? {
        return Err(parent_suspended_error(&job.id));
    }
    Ok(())
}
