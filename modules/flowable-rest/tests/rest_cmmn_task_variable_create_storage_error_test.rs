//! Java `TaskVariableCollectionResource.createTaskVariable` (:174-176) checks
//! `hasVariableOnScope`, which calls `taskService.hasVariableLocal` /
//! `runtimeService.hasVariable`. Neither catches: a failed variable query
//! escapes the request as a 500 and nothing is written.
//!
//! The Rust handler used `load_plan_item_variable(..).is_ok()` as that probe, so
//! a storage failure while listing variables read as "not present" and the
//! handler went on to write the variables — the opposite of Java.

use axum::extract::{Extension, Path};
use flowable_rest::common::PagedResponse;
use flowable_rest::error::ApiError;
use flowable_rest::routes::cmmn::*;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

/// A CMMN runtime whose task exists but whose variable reads fail.
#[derive(Default)]
struct FailingVariableReads {
    writes: AtomicUsize,
}

fn storage_failure() -> ApiError {
    ApiError::InternalServerError("variable table unavailable".to_string())
}

impl CmmnRuntimeApi for FailingVariableReads {
    fn start_case_instance(
        &self,
        _command: StartCaseInstanceCommand,
    ) -> Result<CaseInstanceRecord, ApiError> {
        Err(storage_failure())
    }

    fn list_case_instances(
        &self,
        _query: CaseInstanceQuery,
    ) -> Result<PagedResponse<CaseInstanceRecord>, ApiError> {
        Err(storage_failure())
    }

    fn list_plan_item_instances(
        &self,
        _query: PlanItemInstanceQuery,
    ) -> Result<PagedResponse<PlanItemInstanceRecord>, ApiError> {
        let task = PlanItemInstanceRecord {
            id: "task-1".to_string(),
            case_instance_id: "case-1".to_string(),
            case_definition_id: "def-1".to_string(),
            plan_item_definition_id: "humanTask".to_string(),
            plan_item_definition_type: "humantask".to_string(),
            element_id: "planItemTask".to_string(),
            stage_instance_id: None,
            stage: false,
            name: "Task".to_string(),
            state: "active".to_string(),
            occurred_time: None,
            assignee: None,
            owner: None,
            priority: None,
            due_date: None,
            category: None,
            delegation_state: None,
            variables: Vec::new(),
            tenant_id: None,
            created_at: "2026-01-01T00:00:00.000Z".to_string(),
            ended_at: None,
        };
        Ok(PagedResponse {
            start: 0,
            size: 1,
            total: 1,
            sort: None,
            order: None,
            data: vec![task],
        })
    }

    fn complete_plan_item_instance(&self, _plan_item_instance_id: &str) -> Result<(), ApiError> {
        Err(storage_failure())
    }

    fn list_task_variables_local(
        &self,
        _plan_item_instance_id: &str,
    ) -> Result<Vec<VariableInstanceRecord>, ApiError> {
        Err(storage_failure())
    }

    fn set_task_variables_local(
        &self,
        _plan_item_instance_id: &str,
        _variables: Vec<CmmnVariableUpdate>,
    ) -> Result<(), ApiError> {
        self.writes.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

#[tokio::test]
async fn variable_lookup_failure_aborts_create_instead_of_reading_as_absent() {
    let runtime = Arc::new(FailingVariableReads::default());
    let dyn_runtime: DynCmmnRuntime = runtime.clone();

    let result = create_task_variables(
        Extension(dyn_runtime),
        Path("task-1".to_string()),
        r#"[{"name":"approved","type":"boolean","value":true,"scope":"local"}]"#.to_string(),
    )
    .await;

    match result {
        Err(ApiError::InternalServerError(message)) => {
            assert!(message.contains("variable table unavailable"), "{message}");
        }
        Err(other) => panic!("expected the storage failure, got {other:?}"),
        Ok(_) => panic!("a failed duplicate probe must not create the variable"),
    }
    assert_eq!(
        runtime.writes.load(Ordering::SeqCst),
        0,
        "no variable may be written after the existence check failed"
    );
}
