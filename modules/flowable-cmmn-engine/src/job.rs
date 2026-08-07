//! CMMN async job handlers aligned with Java `org.flowable.cmmn.engine.impl.job`.
//!
//! Java surface (handler TYPE strings):
//! 1. `cmmn-async-activate-plan-item-instance`
//! 2. `cmmn-async-leave-active-plan-item-instance`
//! 3. `cmmn-async-init-plan-model-instance`
//! 4. `cmmn-set-async-variables`
//! 5. `case-migration`
//! 6. `cmmn-trigger-timer`
//! 7. `cmmn-external-worker-complete`
//! 8. `cmmn-history-cleanup`
//! 9. `case-migration-status`
//! 10. `historic-case-migration`
//! 11. enable / disable / reactivate / complete / terminate plan-item helpers
//!     (exposed as first-class types for REST/async orchestration parity)

use crate::error::CmmnError;
use crate::history::CmmnHistoryService;
use crate::history_cleaning::CmmnHistoryCleaningConfiguration;
use crate::management::CmmnManagementService;
use crate::models::{
    CmmnChangePlanItemStateRequest, CmmnHumanTaskCompletionRequest, CmmnJob, CmmnJobFamily,
    CmmnMigrationDocument,
};
use crate::runtime::CmmnRuntimeService;
use chrono::Utc;
use serde_json::Value;
use std::collections::HashMap;

/// Java: `AsyncActivatePlanItemInstanceJobHandler.TYPE`
pub const TYPE_ASYNC_ACTIVATE_PLAN_ITEM: &str = "cmmn-async-activate-plan-item-instance";
/// Java: `AsyncLeaveActivePlanItemInstanceJobHandler.TYPE`
pub const TYPE_ASYNC_LEAVE_ACTIVE_PLAN_ITEM: &str = "cmmn-async-leave-active-plan-item-instance";
/// Java: `AsyncInitializePlanModelJobHandler.TYPE`
pub const TYPE_ASYNC_INIT_PLAN_MODEL: &str = "cmmn-async-init-plan-model-instance";
/// Java: `SetAsyncVariablesJobHandler.TYPE`
pub const TYPE_SET_ASYNC_VARIABLES: &str = "cmmn-set-async-variables";
/// Java: `CaseInstanceMigrationJobHandler.TYPE`
pub const TYPE_CASE_MIGRATION: &str = "case-migration";
/// Java: `TriggerTimerEventJobHandler.TYPE`
pub const TYPE_TRIGGER_TIMER: &str = "cmmn-trigger-timer";
/// Java: `ExternalWorkerTaskCompleteJobHandler.TYPE`
pub const TYPE_EXTERNAL_WORKER_COMPLETE: &str = "cmmn-external-worker-complete";
/// Java: `CmmnHistoryCleanupJobHandler.TYPE`
pub const TYPE_HISTORY_CLEANUP: &str = "cmmn-history-cleanup";
/// Java: `CaseInstanceMigrationStatusJobHandler.TYPE`
pub const TYPE_CASE_MIGRATION_STATUS: &str = "case-migration-status";
/// Java: `HistoricCaseInstanceMigrationJobHandler.TYPE`
pub const TYPE_HISTORIC_CASE_MIGRATION: &str = "historic-case-migration";

/// Async enable plan item (orchestration helper; maps to enable API).
pub const TYPE_ASYNC_ENABLE_PLAN_ITEM: &str = "cmmn-async-enable-plan-item-instance";
/// Async disable plan item (orchestration helper; maps to disable API).
pub const TYPE_ASYNC_DISABLE_PLAN_ITEM: &str = "cmmn-async-disable-plan-item-instance";
/// Async reactivate plan item (orchestration helper).
pub const TYPE_ASYNC_REACTIVATE_PLAN_ITEM: &str = "cmmn-async-reactivate-plan-item-instance";
/// Async complete human task / plan item.
pub const TYPE_ASYNC_COMPLETE_PLAN_ITEM: &str = "cmmn-async-complete-plan-item-instance";
/// Async terminate plan item or case.
pub const TYPE_ASYNC_TERMINATE: &str = "cmmn-async-terminate";
/// Async start case instance by key.
pub const TYPE_ASYNC_START_CASE: &str = "cmmn-async-start-case-instance";

/// All registered handler type strings (Java core + orchestration helpers).
pub const ALL_HANDLER_TYPES: &[&str] = &[
    TYPE_ASYNC_ACTIVATE_PLAN_ITEM,
    TYPE_ASYNC_LEAVE_ACTIVE_PLAN_ITEM,
    TYPE_ASYNC_INIT_PLAN_MODEL,
    TYPE_SET_ASYNC_VARIABLES,
    TYPE_CASE_MIGRATION,
    TYPE_TRIGGER_TIMER,
    TYPE_EXTERNAL_WORKER_COMPLETE,
    TYPE_HISTORY_CLEANUP,
    TYPE_CASE_MIGRATION_STATUS,
    TYPE_HISTORIC_CASE_MIGRATION,
    TYPE_ASYNC_ENABLE_PLAN_ITEM,
    TYPE_ASYNC_DISABLE_PLAN_ITEM,
    TYPE_ASYNC_REACTIVATE_PLAN_ITEM,
    TYPE_ASYNC_COMPLETE_PLAN_ITEM,
    TYPE_ASYNC_TERMINATE,
    TYPE_ASYNC_START_CASE,
];

/// Context available to job handlers during execution.
pub struct CmmnJobExecutionContext<'a> {
    pub runtime: &'a CmmnRuntimeService,
    pub history: &'a CmmnHistoryService,
    pub management: &'a CmmnManagementService,
    /// Engine history-cleaning config (P127). Shared so the cleanup handler can
    /// read batch size / retention without reaching back into `CmmnEngine`.
    pub history_cleaning: &'a CmmnHistoryCleaningConfiguration,
}

/// Migration batch status values written into status-job configuration JSON.
pub const MIGRATION_STATUS_IN_PROGRESS: &str = "inProgress";
pub const MIGRATION_STATUS_COMPLETED: &str = "completed";
pub const MIGRATION_STATUS_FAIL: &str = "fail";

/// Trait for CMMN job handlers (Java `JobHandler` equivalent).
pub trait CmmnJobHandler: Send + Sync {
    fn type_name(&self) -> &'static str;
    fn execute(
        &self,
        job: &CmmnJob,
        configuration: Option<&str>,
        ctx: &CmmnJobExecutionContext<'_>,
    ) -> Result<(), CmmnError>;
}

/// Registry of job handlers keyed by TYPE string.
pub struct CmmnJobHandlerRegistry {
    handlers: HashMap<&'static str, Box<dyn CmmnJobHandler>>,
}

impl Default for CmmnJobHandlerRegistry {
    fn default() -> Self {
        Self::with_default_handlers()
    }
}

impl CmmnJobHandlerRegistry {
    pub fn new() -> Self {
        Self {
            handlers: HashMap::new(),
        }
    }

    pub fn with_default_handlers() -> Self {
        let mut registry = Self::new();
        registry.register(Box::new(AsyncActivatePlanItemHandler));
        registry.register(Box::new(AsyncLeaveActivePlanItemHandler));
        registry.register(Box::new(AsyncInitPlanModelHandler));
        registry.register(Box::new(SetAsyncVariablesHandler));
        registry.register(Box::new(CaseMigrationHandler));
        registry.register(Box::new(TriggerTimerHandler));
        registry.register(Box::new(ExternalWorkerCompleteHandler));
        registry.register(Box::new(HistoryCleanupHandler));
        registry.register(Box::new(CaseMigrationStatusHandler));
        registry.register(Box::new(HistoricCaseMigrationHandler));
        registry.register(Box::new(AsyncEnablePlanItemHandler));
        registry.register(Box::new(AsyncDisablePlanItemHandler));
        registry.register(Box::new(AsyncReactivatePlanItemHandler));
        registry.register(Box::new(AsyncCompletePlanItemHandler));
        registry.register(Box::new(AsyncTerminateHandler));
        registry.register(Box::new(AsyncStartCaseHandler));
        registry
    }

    pub fn register(&mut self, handler: Box<dyn CmmnJobHandler>) {
        self.handlers.insert(handler.type_name(), handler);
    }

    pub fn has_handler(&self, type_name: &str) -> bool {
        self.handlers.contains_key(type_name)
    }

    pub fn registered_types(&self) -> Vec<&'static str> {
        let mut types: Vec<_> = self.handlers.keys().copied().collect();
        types.sort_unstable();
        types
    }

    pub fn execute(
        &self,
        job: &CmmnJob,
        ctx: &CmmnJobExecutionContext<'_>,
    ) -> Result<(), CmmnError> {
        let type_name = job.handler_type.as_deref().ok_or_else(|| {
            CmmnError::execution(format!("CMMN job '{}' is missing handler_type", job.id))
        })?;
        let handler = self.handlers.get(type_name).ok_or_else(|| {
            CmmnError::execution(format!(
                "No CMMN job handler registered for type '{type_name}'"
            ))
        })?;
        handler.execute(job, job.configuration.as_deref(), ctx)
    }
}

fn require_scope_id(job: &CmmnJob) -> Result<&str, CmmnError> {
    job.scope_id.as_deref().ok_or_else(|| {
        CmmnError::execution(format!(
            "CMMN job '{}' is missing scope_id (case instance id)",
            job.id
        ))
    })
}

fn require_element_or_sub_scope(job: &CmmnJob) -> Result<&str, CmmnError> {
    job.element_id
        .as_deref()
        .or(job.sub_scope_id.as_deref())
        .ok_or_else(|| {
            CmmnError::execution(format!(
                "CMMN job '{}' is missing element_id / sub_scope_id",
                job.id
            ))
        })
}

fn parse_json_object(
    configuration: Option<&str>,
) -> Result<serde_json::Map<String, Value>, CmmnError> {
    let Some(raw) = configuration.filter(|s| !s.trim().is_empty()) else {
        return Ok(serde_json::Map::new());
    };
    let value: Value = serde_json::from_str(raw)
        .map_err(|e| CmmnError::execution(format!("Invalid CMMN job configuration JSON: {e}")))?;
    match value {
        Value::Object(map) => Ok(map),
        _ => Err(CmmnError::execution(
            "CMMN job configuration must be a JSON object",
        )),
    }
}

// ── Handlers ──────────────────────────────────────────────────────────────

struct AsyncActivatePlanItemHandler;

impl CmmnJobHandler for AsyncActivatePlanItemHandler {
    fn type_name(&self) -> &'static str {
        TYPE_ASYNC_ACTIVATE_PLAN_ITEM
    }

    fn execute(
        &self,
        job: &CmmnJob,
        configuration: Option<&str>,
        ctx: &CmmnJobExecutionContext<'_>,
    ) -> Result<(), CmmnError> {
        let case_instance_id = require_scope_id(job)?;
        let definition_id = configuration
            .filter(|s| !s.is_empty())
            .or(job.element_id.as_deref())
            .ok_or_else(|| {
                CmmnError::execution(format!(
                    "async activate job '{}' needs plan item definition id",
                    job.id
                ))
            })?;
        ctx.runtime.change_plan_item_state(
            case_instance_id,
            CmmnChangePlanItemStateRequest {
                activate_plan_item_definition_ids: vec![definition_id.to_string()],
                ..Default::default()
            },
        )
    }
}

struct AsyncLeaveActivePlanItemHandler;

impl CmmnJobHandler for AsyncLeaveActivePlanItemHandler {
    fn type_name(&self) -> &'static str {
        TYPE_ASYNC_LEAVE_ACTIVE_PLAN_ITEM
    }

    fn execute(
        &self,
        job: &CmmnJob,
        configuration: Option<&str>,
        ctx: &CmmnJobExecutionContext<'_>,
    ) -> Result<(), CmmnError> {
        let case_instance_id = require_scope_id(job)?;
        let cfg = parse_json_object(configuration)?;
        let transition = cfg
            .get("transition")
            .and_then(|v| v.as_str())
            .unwrap_or("complete");
        let plan_item_def = job
            .element_id
            .as_deref()
            .or_else(|| cfg.get("planItemDefinitionId").and_then(|v| v.as_str()));

        match transition {
            "complete" => {
                let task_id = require_element_or_sub_scope(job)?;
                // Prefer completing human task by instance id; fall back to definition terminate.
                match ctx
                    .runtime
                    .complete_human_task(task_id, CmmnHumanTaskCompletionRequest::new())
                {
                    Ok(_) => Ok(()),
                    Err(_) if plan_item_def.is_some() => ctx.runtime.change_plan_item_state(
                        case_instance_id,
                        CmmnChangePlanItemStateRequest {
                            terminate_plan_item_definition_ids: vec![
                                plan_item_def.unwrap().to_string(),
                            ],
                            ..Default::default()
                        },
                    ),
                    Err(e) => Err(e),
                }
            }
            "exit" | "terminate" => {
                let def_id = plan_item_def.ok_or_else(|| {
                    CmmnError::execution(format!(
                        "leave job '{}' needs element_id for {transition}",
                        job.id
                    ))
                })?;
                ctx.runtime.change_plan_item_state(
                    case_instance_id,
                    CmmnChangePlanItemStateRequest {
                        terminate_plan_item_definition_ids: vec![def_id.to_string()],
                        ..Default::default()
                    },
                )
            }
            other => Err(CmmnError::execution(format!(
                "unsupported leave transition '{other}' for job '{}'",
                job.id
            ))),
        }
    }
}

struct AsyncInitPlanModelHandler;

impl CmmnJobHandler for AsyncInitPlanModelHandler {
    fn type_name(&self) -> &'static str {
        TYPE_ASYNC_INIT_PLAN_MODEL
    }

    fn execute(
        &self,
        job: &CmmnJob,
        _configuration: Option<&str>,
        ctx: &CmmnJobExecutionContext<'_>,
    ) -> Result<(), CmmnError> {
        // Java plans init-plan-model on the agenda. Here we verify the case exists
        // (plan model was already initialized on start; this is a no-op success).
        let case_instance_id = require_scope_id(job)?;
        let _ = ctx.runtime.get_case_instance(case_instance_id)?;
        Ok(())
    }
}

struct SetAsyncVariablesHandler;

impl CmmnJobHandler for SetAsyncVariablesHandler {
    fn type_name(&self) -> &'static str {
        TYPE_SET_ASYNC_VARIABLES
    }

    fn execute(
        &self,
        job: &CmmnJob,
        configuration: Option<&str>,
        ctx: &CmmnJobExecutionContext<'_>,
    ) -> Result<(), CmmnError> {
        let case_instance_id = require_scope_id(job)?;
        let map = parse_json_object(configuration)?;
        let variables: Vec<(String, Value)> = map.into_iter().collect();
        if variables.is_empty() {
            return Ok(());
        }
        ctx.runtime
            .set_case_instance_variables(case_instance_id, variables)
    }
}

struct CaseMigrationHandler;

impl CmmnJobHandler for CaseMigrationHandler {
    fn type_name(&self) -> &'static str {
        TYPE_CASE_MIGRATION
    }

    fn execute(
        &self,
        job: &CmmnJob,
        configuration: Option<&str>,
        ctx: &CmmnJobExecutionContext<'_>,
    ) -> Result<(), CmmnError> {
        let case_instance_id = require_scope_id(job)?;
        let cfg = parse_json_object(configuration)?;
        let target = cfg
            .get("targetCaseDefinitionId")
            .and_then(|v| v.as_str())
            .or(job.scope_definition_id.as_deref())
            .ok_or_else(|| {
                CmmnError::execution(format!(
                    "migration job '{}' needs targetCaseDefinitionId",
                    job.id
                ))
            })?;
        ctx.runtime.migrate_case_instance(
            case_instance_id,
            CmmnMigrationDocument {
                target_case_definition_id: target.to_string(),
            },
        )
    }
}

struct TriggerTimerHandler;

impl CmmnJobHandler for TriggerTimerHandler {
    fn type_name(&self) -> &'static str {
        TYPE_TRIGGER_TIMER
    }

    fn execute(
        &self,
        job: &CmmnJob,
        _configuration: Option<&str>,
        ctx: &CmmnJobExecutionContext<'_>,
    ) -> Result<(), CmmnError> {
        // Java TriggerTimerEventJobHandler.java:35-38: the job's subScopeId points at the
        // timer event listener's plan item; firing occurs the plan item (which fans out
        // to sentry onParts) and reschedules a repeating cycle. The fired job is deleted
        // by CmmnEngine::execute_job after the handler returns.
        ctx.runtime.fire_timer_event_listener(job)
    }
}

struct ExternalWorkerCompleteHandler;

impl CmmnJobHandler for ExternalWorkerCompleteHandler {
    fn type_name(&self) -> &'static str {
        TYPE_EXTERNAL_WORKER_COMPLETE
    }

    fn execute(
        &self,
        job: &CmmnJob,
        configuration: Option<&str>,
        ctx: &CmmnJobExecutionContext<'_>,
    ) -> Result<(), CmmnError> {
        // Apply any variables from configuration, then complete the plan item task.
        if let Some(cfg) = configuration.filter(|s| !s.is_empty()) {
            let map = parse_json_object(Some(cfg))?;
            if !map.is_empty() {
                let case_instance_id = require_scope_id(job)?;
                let variables: Vec<(String, Value)> = map.into_iter().collect();
                ctx.runtime
                    .set_case_instance_variables(case_instance_id, variables)?;
            }
        }
        let task_id = require_element_or_sub_scope(job)?;
        match ctx
            .runtime
            .complete_human_task(task_id, CmmnHumanTaskCompletionRequest::new())
        {
            Ok(_) => Ok(()),
            Err(_) => {
                // Fall back: terminate plan item by definition id.
                let case_instance_id = require_scope_id(job)?;
                ctx.runtime.change_plan_item_state(
                    case_instance_id,
                    CmmnChangePlanItemStateRequest {
                        terminate_plan_item_definition_ids: vec![task_id.to_string()],
                        ..Default::default()
                    },
                )
            }
        }
    }
}

struct HistoryCleanupHandler;

impl CmmnJobHandler for HistoryCleanupHandler {
    fn type_name(&self) -> &'static str {
        TYPE_HISTORY_CLEANUP
    }

    fn execute(
        &self,
        job: &CmmnJob,
        _configuration: Option<&str>,
        ctx: &CmmnJobExecutionContext<'_>,
    ) -> Result<(), CmmnError> {
        // Java CmmnHistoryCleanupJobHandler.execute (CmmnHistoryCleanupJobHandler.java:37-57).
        // Deviations: no in-progress batch skip; no batch-record cleanup; sync delete.
        let now = Utc::now();
        crate::history_cleaning::execute_history_cleanup(
            ctx.history,
            ctx.history_cleaning,
            now,
        )?;
        // Schedule next cron occurrence before the outer execute_job deletes this row.
        crate::history_cleaning::schedule_next_history_cleanup_timer(ctx.management, job, now)?;
        Ok(())
    }
}

struct CaseMigrationStatusHandler;

impl CmmnJobHandler for CaseMigrationStatusHandler {
    fn type_name(&self) -> &'static str {
        TYPE_CASE_MIGRATION_STATUS
    }

    fn execute(
        &self,
        job: &CmmnJob,
        configuration: Option<&str>,
        ctx: &CmmnJobExecutionContext<'_>,
    ) -> Result<(), CmmnError> {
        let mut cfg = parse_json_object(configuration)?;
        let batch_id = cfg
            .get("batchId")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .or_else(|| job.scope_id.clone());

        // Seeded counts cover finished work (successful migration jobs are deleted on execute).
        let seeded_completed = cfg
            .get("completedCount")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let seeded_failed = cfg.get("failedCount").and_then(|v| v.as_u64()).unwrap_or(0);
        let seeded_total = cfg.get("totalCount").and_then(|v| v.as_u64());

        let jobs = ctx.management.create_job_query().list()?;
        let mut pending = 0u64;
        let mut live_failed = 0u64;

        for other in &jobs {
            if other.id == job.id {
                continue;
            }
            if !job_matches_migration_batch(other, batch_id.as_deref(), job.scope_id.as_deref()) {
                continue;
            }
            let handler = other.handler_type.as_deref();
            let is_migration_part = matches!(
                handler,
                Some(TYPE_CASE_MIGRATION) | Some(TYPE_HISTORIC_CASE_MIGRATION)
            );
            if !is_migration_part {
                continue;
            }
            if other.family == CmmnJobFamily::Deadletter
                || other
                    .exception_message
                    .as_ref()
                    .is_some_and(|m| !m.is_empty())
            {
                live_failed += 1;
            } else {
                pending += 1;
            }
        }

        let failed = seeded_failed.saturating_add(live_failed);
        let total = seeded_total.unwrap_or_else(|| {
            seeded_completed
                .saturating_add(failed)
                .saturating_add(pending)
        });
        // Prefer explicit completed seed; otherwise derive from total - pending - failed.
        let completed = if seeded_completed > 0 || seeded_total.is_none() {
            seeded_completed
        } else {
            total.saturating_sub(pending.saturating_add(failed))
        };

        let status = if pending > 0 {
            MIGRATION_STATUS_IN_PROGRESS
        } else if failed > 0 {
            MIGRATION_STATUS_FAIL
        } else {
            MIGRATION_STATUS_COMPLETED
        };

        if let Some(batch_id) = batch_id.as_ref() {
            cfg.insert("batchId".to_string(), Value::String(batch_id.clone()));
        }
        cfg.insert("status".to_string(), Value::String(status.to_string()));
        cfg.insert(
            "completedCount".to_string(),
            Value::Number(completed.into()),
        );
        cfg.insert("failedCount".to_string(), Value::Number(failed.into()));
        cfg.insert("pendingCount".to_string(), Value::Number(pending.into()));
        cfg.insert("totalCount".to_string(), Value::Number(total.into()));
        cfg.insert("aggregated".to_string(), Value::Bool(true));

        let mut updated = job.clone();
        updated.configuration = Some(serde_json::to_string(&Value::Object(cfg)).map_err(|e| {
            CmmnError::execution(format!(
                "failed to serialize migration status for job '{}': {e}",
                job.id
            ))
        })?);
        updated.state = status.to_string();
        ctx.management.update_job(&updated)?;
        Ok(())
    }
}

fn job_matches_migration_batch(
    job: &CmmnJob,
    batch_id: Option<&str>,
    scope_id: Option<&str>,
) -> bool {
    if let Some(batch_id) = batch_id {
        if job_config_batch_id(job).as_deref() == Some(batch_id) {
            return true;
        }
        if job.scope_id.as_deref() == Some(batch_id) {
            return true;
        }
    }
    if let Some(scope_id) = scope_id {
        if job.scope_id.as_deref() == Some(scope_id) {
            return true;
        }
    }
    false
}

fn job_config_batch_id(job: &CmmnJob) -> Option<String> {
    let raw = job.configuration.as_deref()?;
    let value: Value = serde_json::from_str(raw).ok()?;
    value
        .get("batchId")
        .and_then(|v| v.as_str())
        .map(str::to_string)
}

struct HistoricCaseMigrationHandler;

impl CmmnJobHandler for HistoricCaseMigrationHandler {
    fn type_name(&self) -> &'static str {
        TYPE_HISTORIC_CASE_MIGRATION
    }

    fn execute(
        &self,
        job: &CmmnJob,
        configuration: Option<&str>,
        ctx: &CmmnJobExecutionContext<'_>,
    ) -> Result<(), CmmnError> {
        let case_instance_id = require_scope_id(job)?;
        let cfg = parse_json_object(configuration)?;
        let target = cfg
            .get("targetCaseDefinitionId")
            .and_then(|v| v.as_str())
            .or(job.scope_definition_id.as_deref())
            .ok_or_else(|| {
                CmmnError::execution(format!(
                    "historic migration job '{}' needs targetCaseDefinitionId",
                    job.id
                ))
            })?;
        ctx.history.migrate_historic_case_instance(
            case_instance_id,
            CmmnMigrationDocument {
                target_case_definition_id: target.to_string(),
            },
        )
    }
}

struct AsyncEnablePlanItemHandler;

impl CmmnJobHandler for AsyncEnablePlanItemHandler {
    fn type_name(&self) -> &'static str {
        TYPE_ASYNC_ENABLE_PLAN_ITEM
    }

    fn execute(
        &self,
        job: &CmmnJob,
        _configuration: Option<&str>,
        ctx: &CmmnJobExecutionContext<'_>,
    ) -> Result<(), CmmnError> {
        let plan_item_instance_id = require_element_or_sub_scope(job)?;
        ctx.runtime.enable_plan_item_instance(plan_item_instance_id)
    }
}

struct AsyncDisablePlanItemHandler;

impl CmmnJobHandler for AsyncDisablePlanItemHandler {
    fn type_name(&self) -> &'static str {
        TYPE_ASYNC_DISABLE_PLAN_ITEM
    }

    fn execute(
        &self,
        job: &CmmnJob,
        _configuration: Option<&str>,
        ctx: &CmmnJobExecutionContext<'_>,
    ) -> Result<(), CmmnError> {
        let plan_item_instance_id = require_element_or_sub_scope(job)?;
        ctx.runtime
            .disable_plan_item_instance(plan_item_instance_id)
    }
}

struct AsyncReactivatePlanItemHandler;

impl CmmnJobHandler for AsyncReactivatePlanItemHandler {
    fn type_name(&self) -> &'static str {
        TYPE_ASYNC_REACTIVATE_PLAN_ITEM
    }

    fn execute(
        &self,
        job: &CmmnJob,
        _configuration: Option<&str>,
        ctx: &CmmnJobExecutionContext<'_>,
    ) -> Result<(), CmmnError> {
        let plan_item_instance_id = require_element_or_sub_scope(job)?;
        ctx.runtime
            .reactivate_plan_item_instance(plan_item_instance_id)
    }
}

struct AsyncCompletePlanItemHandler;

impl CmmnJobHandler for AsyncCompletePlanItemHandler {
    fn type_name(&self) -> &'static str {
        TYPE_ASYNC_COMPLETE_PLAN_ITEM
    }

    fn execute(
        &self,
        job: &CmmnJob,
        _configuration: Option<&str>,
        ctx: &CmmnJobExecutionContext<'_>,
    ) -> Result<(), CmmnError> {
        let task_id = require_element_or_sub_scope(job)?;
        ctx.runtime
            .complete_human_task(task_id, CmmnHumanTaskCompletionRequest::new())
            .map(|_| ())
    }
}

struct AsyncTerminateHandler;

impl CmmnJobHandler for AsyncTerminateHandler {
    fn type_name(&self) -> &'static str {
        TYPE_ASYNC_TERMINATE
    }

    fn execute(
        &self,
        job: &CmmnJob,
        configuration: Option<&str>,
        ctx: &CmmnJobExecutionContext<'_>,
    ) -> Result<(), CmmnError> {
        let case_instance_id = require_scope_id(job)?;
        if let Some(def_id) = job
            .element_id
            .as_deref()
            .or_else(|| configuration.filter(|s| !s.is_empty()))
        {
            ctx.runtime.change_plan_item_state(
                case_instance_id,
                CmmnChangePlanItemStateRequest {
                    terminate_plan_item_definition_ids: vec![def_id.to_string()],
                    ..Default::default()
                },
            )
        } else {
            ctx.runtime.terminate_case_instance(case_instance_id)
        }
    }
}

struct AsyncStartCaseHandler;

impl CmmnJobHandler for AsyncStartCaseHandler {
    fn type_name(&self) -> &'static str {
        TYPE_ASYNC_START_CASE
    }

    fn execute(
        &self,
        job: &CmmnJob,
        configuration: Option<&str>,
        ctx: &CmmnJobExecutionContext<'_>,
    ) -> Result<(), CmmnError> {
        let cfg = parse_json_object(configuration)?;
        let key = cfg
            .get("caseDefinitionKey")
            .and_then(|v| v.as_str())
            .or(job.element_id.as_deref())
            .ok_or_else(|| {
                CmmnError::execution(format!(
                    "async start job '{}' needs caseDefinitionKey",
                    job.id
                ))
            })?;
        let mut request = crate::models::CmmnCaseInstanceStartRequest::new();
        if let Some(Value::Object(vars)) = cfg.get("variables").cloned() {
            request = request.with_variables(Value::Object(vars));
        }
        if let Some(bk) = cfg.get("businessKey").and_then(|v| v.as_str()) {
            request = request.with_business_key(bk);
        }
        ctx.runtime
            .start_case_instance_by_key(key, request)
            .map(|_| ())
    }
}
