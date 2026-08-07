//! Contract tests for CMMN suspended-job activation/delete and parent resolver.
//!
//! Java baselines:
//! - `CmmnManagementService.moveSuspendedJobToExecutableJob` / `deleteSuspendedJob`
//! - `DefaultCmmnJobParentStateResolver`
//! - `MoveSuspendedJobToExecutableJobCmd` / `DeleteSuspendedJobCmd`

use chrono::{TimeZone, Utc};
use flowable_cmmn_engine::{
    CMMN_SCOPE_TYPE, CmmnCase, CmmnCaseInstanceStartRequest, CmmnCaseInstanceState,
    CmmnCasePlanModel, CmmnDeploymentRequest, CmmnEngine, CmmnError, CmmnHumanTask, CmmnJob,
    CmmnJobFamily, CmmnModel, CmmnPlanItem,
};

fn engine() -> CmmnEngine {
    CmmnEngine::new_in_memory().expect("cmmn engine")
}

fn simple_case_model(key: &str) -> CmmnModel {
    let plan_model = CmmnCasePlanModel::new("casePlanModel", "Case Plan Model")
        .with_human_task(CmmnHumanTask::new("reviewTask", "Review"))
        .with_plan_item(CmmnPlanItem::new("planItemReview", "reviewTask"));
    CmmnModel::new(vec![CmmnCase::new(
        format!("{key}Definition"),
        key,
        "Suspended Job Case",
        plan_model,
    )])
}

fn start_case(engine: &CmmnEngine, key: &str) -> String {
    engine
        .deploy(
            CmmnDeploymentRequest::new(format!("dep-{key}"))
                .with_resource(format!("{key}.cmmn"), simple_case_model(key)),
        )
        .expect("deploy");
    engine
        .start_case_instance_by_key(key, CmmnCaseInstanceStartRequest::new())
        .expect("start case")
        .id
}

fn full_suspended_job(id: &str, scope_id: Option<&str>) -> CmmnJob {
    let due = Utc.with_ymd_and_hms(2026, 5, 1, 12, 0, 0).unwrap();
    let created = Utc.with_ymd_and_hms(2026, 4, 30, 8, 0, 0).unwrap();
    let mut job = CmmnJob::new(id, CmmnJobFamily::Suspended);
    job.scope_id = scope_id.map(str::to_string);
    job.sub_scope_id = Some("plan-item-1".to_string());
    job.scope_definition_id = Some("case-def-1".to_string());
    job.element_id = Some("timerElement".to_string());
    job.tenant_id = Some("tenant-a".to_string());
    job.due_date = Some(due);
    job.created_at = created;
    job.retries = 0;
    job.exception_message = Some("preserved failure".to_string());
    job.exception_stacktrace = Some("stack line 1\nstack line 2".to_string());
    job.handler_type = Some("cmmn-trigger-timer".to_string());
    job.configuration = Some(r#"{"cfg":true}"#.to_string());
    job.lock_owner = Some("stale-worker".to_string());
    job.scope_type = Some(CMMN_SCOPE_TYPE.to_string());
    job
}

#[test]
fn active_case_activates_suspended_job_preserving_all_fields_including_zero_retries() {
    let engine = engine();
    let case_id = start_case(&engine, "activate-zero-retries");
    let mut job = full_suspended_job("susp-zero", Some(&case_id));
    job.retries = 0;
    engine
        .management_service()
        .insert_job(job.clone())
        .expect("insert");

    let activated = engine
        .management_service()
        .move_suspended_job_to_executable_job("susp-zero")
        .expect("activate");

    assert_eq!(activated.family, CmmnJobFamily::Executable);
    assert_eq!(activated.state, "executable");
    assert_eq!(activated.retries, 0);
    assert_eq!(activated.due_date, job.due_date);
    assert_eq!(activated.handler_type, job.handler_type);
    assert_eq!(activated.configuration, job.configuration);
    assert_eq!(activated.exception_message, job.exception_message);
    assert_eq!(activated.exception_stacktrace, job.exception_stacktrace);
    assert_eq!(activated.scope_id, job.scope_id);
    assert_eq!(activated.sub_scope_id, job.sub_scope_id);
    assert_eq!(activated.scope_definition_id, job.scope_definition_id);
    assert_eq!(activated.tenant_id, job.tenant_id);
    assert_eq!(activated.element_id, job.element_id);
    assert!(activated.lock_owner.is_none());

    // Source suspended gone; target executable present.
    let listed_suspended = engine
        .management_service()
        .create_job_query()
        .family(CmmnJobFamily::Suspended)
        .id("susp-zero")
        .list()
        .expect("query suspended");
    assert!(listed_suspended.is_empty());

    let listed_executable = engine
        .management_service()
        .create_job_query()
        .family(CmmnJobFamily::Executable)
        .id("susp-zero")
        .list()
        .expect("query executable");
    assert_eq!(listed_executable.len(), 1);
}

#[test]
fn negative_retries_are_preserved_on_activation() {
    let engine = engine();
    let case_id = start_case(&engine, "activate-neg-retries");
    let mut job = full_suspended_job("susp-neg", Some(&case_id));
    job.retries = -3;
    engine.management_service().insert_job(job).expect("insert");

    let activated = engine
        .management_service()
        .move_suspended_job_to_executable_job("susp-neg")
        .expect("activate");
    assert_eq!(activated.retries, -3);
    assert_eq!(activated.family, CmmnJobFamily::Executable);
}

#[test]
fn suspended_case_rejects_activation_without_mutating_job() {
    let engine = engine();
    let case_id = start_case(&engine, "suspended-parent");
    engine
        .runtime_service()
        .set_case_instance_state(&case_id, CmmnCaseInstanceState::Suspended)
        .expect("suspend case");

    let mut job = full_suspended_job("blocked-susp", Some(&case_id));
    job.retries = 2;
    job.lock_owner = Some("keep-lock".to_string());
    engine
        .management_service()
        .insert_job(job.clone())
        .expect("insert");

    let err = engine
        .management_service()
        .move_suspended_job_to_executable_job("blocked-susp")
        .expect_err("suspended parent must reject");
    assert!(matches!(err, CmmnError::Validation { .. }));
    assert_eq!(
        err.to_string(),
        "Can not activate job blocked-susp. Parent is suspended."
    );

    let still = engine
        .management_service()
        .get_job("blocked-susp")
        .expect("job still present");
    assert_eq!(still.family, CmmnJobFamily::Suspended);
    assert_eq!(still.retries, 2);
    assert_eq!(still.lock_owner.as_deref(), Some("keep-lock"));
}

#[test]
fn non_cmmn_scope_and_empty_scope_return_typed_parent_not_cmmn_error() {
    let engine = engine();

    let mut non_cmmn = full_suspended_job("non-cmmn-scope", Some("case-x"));
    non_cmmn.scope_type = Some("bpmn".to_string());
    engine
        .management_service()
        .insert_job(non_cmmn)
        .expect("insert");
    let err = engine
        .management_service()
        .move_suspended_job_to_executable_job("non-cmmn-scope")
        .expect_err("non-cmmn");
    assert!(matches!(err, CmmnError::Validation { .. }));
    assert_eq!(
        err.to_string(),
        "Job non-cmmn-scope parent is not CMMN case"
    );

    let mut empty_scope = full_suspended_job("empty-scope", None);
    empty_scope.scope_id = Some("  ".to_string());
    engine
        .management_service()
        .insert_job(empty_scope)
        .expect("insert");
    let err = engine
        .management_service()
        .move_suspended_job_to_executable_job("empty-scope")
        .expect_err("empty scope");
    assert!(matches!(err, CmmnError::Validation { .. }));
    assert_eq!(err.to_string(), "Job empty-scope parent is not CMMN case");
}

#[test]
fn missing_parent_case_returns_typed_parent_not_cmmn_error() {
    let engine = engine();
    let job = full_suspended_job("missing-parent", Some("no-such-case"));
    engine.management_service().insert_job(job).expect("insert");
    let err = engine
        .management_service()
        .move_suspended_job_to_executable_job("missing-parent")
        .expect_err("missing parent");
    assert!(matches!(err, CmmnError::Validation { .. }));
    assert_eq!(
        err.to_string(),
        "Job missing-parent parent is not CMMN case"
    );
}

#[test]
fn activate_missing_id_is_not_found() {
    let engine = engine();
    let err = engine
        .management_service()
        .move_suspended_job_to_executable_job("no-such-job")
        .expect_err("missing");
    assert!(matches!(err, CmmnError::NotFound { .. }));
}

#[test]
fn activate_non_suspended_family_is_not_found_and_leaves_row() {
    let engine = engine();
    let case_id = start_case(&engine, "family-mismatch-activate");
    let mut job = CmmnJob::new("exec-1", CmmnJobFamily::Executable);
    job.scope_id = Some(case_id);
    engine.management_service().insert_job(job).expect("insert");

    let err = engine
        .management_service()
        .move_suspended_job_to_executable_job("exec-1")
        .expect_err("not suspended");
    assert!(matches!(err, CmmnError::NotFound { .. }));
    assert!(
        engine
            .management_service()
            .get_job("exec-1")
            .expect("still there")
            .family
            == CmmnJobFamily::Executable
    );
}

#[test]
fn delete_suspended_job_succeeds_for_suspended_family_only() {
    let engine = engine();
    let case_id = start_case(&engine, "delete-susp");
    engine
        .management_service()
        .insert_job(full_suspended_job("to-delete", Some(&case_id)))
        .expect("insert");

    engine
        .management_service()
        .delete_suspended_job("to-delete")
        .expect("delete");
    assert!(matches!(
        engine.management_service().get_job("to-delete"),
        Err(CmmnError::NotFound { .. })
    ));
}

#[test]
fn delete_suspended_rejects_other_families_and_unknown_without_mutation() {
    let engine = engine();
    let case_id = start_case(&engine, "delete-family-guard");

    for (id, family) in [
        ("del-exec", CmmnJobFamily::Executable),
        ("del-timer", CmmnJobFamily::Timer),
        ("del-dl", CmmnJobFamily::Deadletter),
        ("del-hist", CmmnJobFamily::History),
    ] {
        let mut job = CmmnJob::new(id, family.clone());
        job.scope_id = Some(case_id.clone());
        engine.management_service().insert_job(job).expect("insert");
        let err = engine
            .management_service()
            .delete_suspended_job(id)
            .expect_err("must 404-equivalent");
        assert!(matches!(err, CmmnError::NotFound { .. }), "{id}");
        assert_eq!(
            engine
                .management_service()
                .get_job(id)
                .expect("unchanged")
                .family,
            family
        );
    }

    let err = engine
        .management_service()
        .delete_suspended_job("unknown-id")
        .expect_err("unknown");
    assert!(matches!(err, CmmnError::NotFound { .. }));
}

#[test]
fn activation_outer_transaction_rollback_restores_suspended_row() {
    let engine = engine();
    let case_id = start_case(&engine, "activate-rollback");
    let original = full_suspended_job("rollback-job", Some(&case_id));
    engine
        .management_service()
        .insert_job(original.clone())
        .expect("insert");

    let err = engine
        .management_service()
        .in_transaction(|session| {
            engine
                .management_service()
                .move_suspended_job_to_executable_job_in_session(session, "rollback-job")?;
            Err::<CmmnJob, _>(CmmnError::execution(
                "forced rollback after activation".to_string(),
            ))
        })
        .expect_err("forced");
    assert_eq!(err.to_string(), "forced rollback after activation");

    let restored = engine
        .management_service()
        .get_job("rollback-job")
        .expect("restored");
    assert_eq!(restored.family, CmmnJobFamily::Suspended);
    assert_eq!(restored.retries, original.retries);
    assert_eq!(restored.lock_owner, original.lock_owner);
    assert_eq!(restored.exception_message, original.exception_message);
}

#[test]
fn delete_outer_transaction_rollback_keeps_suspended_row() {
    let engine = engine();
    let case_id = start_case(&engine, "delete-rollback");
    engine
        .management_service()
        .insert_job(full_suspended_job("rollback-del", Some(&case_id)))
        .expect("insert");

    let _ = engine
        .management_service()
        .in_transaction(|session| {
            engine
                .management_service()
                .delete_suspended_job_in_session(session, "rollback-del")?;
            Err::<(), _>(CmmnError::execution("forced rollback after delete"))
        })
        .expect_err("forced");

    let still = engine
        .management_service()
        .get_job("rollback-del")
        .expect("kept");
    assert_eq!(still.family, CmmnJobFamily::Suspended);
}

#[test]
fn case_state_serde_preserves_existing_variants_and_suspended() {
    for (state, expected) in [
        (CmmnCaseInstanceState::Active, "\"Active\""),
        (CmmnCaseInstanceState::Completed, "\"Completed\""),
        (CmmnCaseInstanceState::Terminated, "\"Terminated\""),
        (CmmnCaseInstanceState::Suspended, "\"Suspended\""),
    ] {
        let json = serde_json::to_string(&state).expect("ser");
        assert_eq!(json, expected);
        let back: CmmnCaseInstanceState = serde_json::from_str(&json).expect("de");
        assert_eq!(back, state);
    }
}
