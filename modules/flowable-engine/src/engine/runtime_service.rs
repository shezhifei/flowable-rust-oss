use crate::agenda::FlowableEngineAgenda;
use crate::agenda::continue_process_operation::{
    ASYNC_CONTINUATION_JOB_STATE, ASYNC_CONTINUATION_JOB_TYPE_MARKER, find_flow_element,
    flow_element_id, flow_element_type,
};
use crate::cmd::correlate_message_cmd::{
    CorrelateMessageCmd, CorrelateMessageOptions, CorrelateMessageResult,
};
use crate::cmd::process_definition_suspension::{
    ExecuteScheduledProcessDefinitionActionCmd, scheduled_process_definition_suspended,
};
use crate::cmd::record_failed_timer_work_cmd::{
    FailedJobExecutionOrigin, RecordFailedTimerWorkCmd,
};
use crate::cmd::run_due_timers_cmd::{ReleaseAcquiredTimerWorkLockCmd, ReleaseTimerJobLockCmd};
use crate::cmd::start_process_instance_cmd::StartProcessInstanceCmd;
use crate::cmd::suspend_process_instances_by_definition_cmd::SuspendProcessInstancesByDefinitionCmd;
use crate::cmd::trigger_boundary_event_cmd::TriggerBoundaryEventByEventRefCmd;
use crate::cmd::trigger_boundary_event_cmd::TriggerBoundaryEventByMessageRefCmd;
use crate::cmd::trigger_boundary_event_cmd::TriggerBoundaryEventBySignalRefCmd;
use crate::cmd::trigger_boundary_event_cmd::TriggerBoundaryEventCmd;
use crate::cmd::trigger_boundary_event_cmd::TriggerTimerBoundaryEventCmd;
use crate::cmd::trigger_intermediate_catch_event_cmd::TriggerEventIntermediateCatchCmd;
use crate::cmd::trigger_intermediate_catch_event_cmd::TriggerIntermediateCatchEventCmd;
use crate::cmd::trigger_intermediate_catch_event_cmd::TriggerTimerIntermediateCatchEventCmd;
use crate::cmd::trigger_send_event_service_task_cmd::TriggerSendEventServiceTaskCmd;
use crate::cmd::update_process_instance_fields_cmd::UpdateProcessInstanceFieldsCmd;

use crate::cmd::trigger_start_event_subscription_cmd::{
    TriggerEventSubprocessByEventCmd, TriggerProcessStartByEventCmd,
};
use crate::cmd::unlock_owned_jobs_cmd::UnlockOwnedJobsCmd;
use crate::el::expression::SimpleExpression;
use crate::engine::event_dispatcher::{EngineEvent, EngineEventType};
use crate::engine::task_service::{EventWaitState, QueryEventWaitStatesByProcessInstanceIdCmd};
use crate::engine::timer_worker::{TimerCoordinationMetrics, TimerWork};
use crate::interceptor::command_executor::{CommandExecutor, DefaultCommandExecutor};
use crate::persistence::runtime_store::{
    EventSubprocessEventSubscription, EventSubprocessTimerSubscription, EventSubscriptionKind,
    RuntimeTimerJobState,
};
use crate::repository::process_definition::ProcessDefinition;
use crate::runtime::execution::Execution;
use crate::runtime::process_instance::{ProcessInstance, ProcessInstanceUpdate};
use crate::runtime::process_instance_builder::ProcessInstanceBuilder;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// Type alias retained so existing callers can import `MessageStyleWaitState` from
/// runtime_service still compile.
pub use crate::engine::task_service::MessageStyleWaitState;

use crate::engine::query::{Direction, Query, QueryState};
use crate::interceptor::command::Command;
use crate::interceptor::command_context::CommandContext;
use crate::task::Task;
use flowable_bpmn_model::model::{EventDefinitionEnum, FlowElementEnum, SequenceFlow, StartEvent};
use uuid::Uuid;

pub struct EventSubscription {
    pub id: String,
    pub event_name: String,
    pub event_kind: String,
}

impl EventSubscription {
    pub fn id(&self) -> &String {
        &self.id
    }

    pub fn event_name(&self) -> &String {
        &self.event_name
    }
}

pub struct EventSubscriptionQuery {
    state: QueryState<EventSubscription>,
    event_name: Option<String>,
}

impl EventSubscriptionQuery {
    pub fn new(command_executor: Arc<DefaultCommandExecutor>) -> Self {
        Self {
            state: QueryState::new(command_executor),
            event_name: None,
        }
    }

    pub fn event_name(mut self, event_name: String) -> Self {
        self.event_name = Some(event_name);
        self
    }
}

pub struct EventSubscriptionQueryCmd {
    query: EventSubscriptionQuery,
}

impl EventSubscriptionQueryCmd {
    pub fn new(query: EventSubscriptionQuery) -> Self {
        Self { query }
    }
}

impl Command<Vec<EventSubscription>> for EventSubscriptionQueryCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<Vec<EventSubscription>, crate::error::FlowableError> {
        let mut rows = command_context
            .session()
            .find_raw_all("event_subscriptions")?;

        if let Some(name) = &self.query.event_name {
            rows.retain(|r| {
                r.extras.get("event_name").and_then(|v| v.as_deref()) == Some(name.as_str())
            });
        }

        rows.sort_by(|a, b| a.id.cmp(&b.id));

        Ok(rows
            .into_iter()
            .map(|r| EventSubscription {
                id: r.id,
                event_name: r
                    .extras
                    .get("event_name")
                    .cloned()
                    .flatten()
                    .unwrap_or_default(),
                event_kind: r
                    .extras
                    .get("event_kind")
                    .cloned()
                    .flatten()
                    .unwrap_or_default(),
            })
            .collect())
    }
}

impl Query<EventSubscription, EventSubscriptionQuery> for EventSubscriptionQuery {
    fn list(&self) -> Result<Vec<EventSubscription>, crate::error::FlowableError> {
        let query_clone = EventSubscriptionQuery {
            state: QueryState {
                command_executor: Arc::clone(&self.state.command_executor),
                phantom: std::marker::PhantomData,
                order_by: self.state.order_by.clone(),
                direction: match self.state.direction {
                    Direction::Asc => Direction::Asc,
                    Direction::Desc => Direction::Desc,
                },
            },
            event_name: self.event_name.clone(),
        };
        let cmd = EventSubscriptionQueryCmd::new(query_clone);
        self.state.command_executor.execute(&cmd)
    }

    fn single_result(&self) -> Result<Option<EventSubscription>, crate::error::FlowableError> {
        let mut list = self.list()?;
        Ok(list.pop())
    }

    fn count(&self) -> Result<i64, crate::error::FlowableError> {
        let list = self.list()?;
        Ok(list.len() as i64)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivityMigrationMapping {
    pub from_activity_id: String,
    pub to_activity_ids: Vec<String>,
}

pub struct MigrateProcessInstanceCmd {
    process_instance_id: String,
    target_process_definition_id: String,
    activity_migration_mappings: Vec<ActivityMigrationMapping>,
}

impl MigrateProcessInstanceCmd {
    pub fn new(
        process_instance_id: String,
        target_process_definition_id: String,
        activity_migration_mappings: Vec<ActivityMigrationMapping>,
    ) -> Self {
        Self {
            process_instance_id,
            target_process_definition_id,
            activity_migration_mappings,
        }
    }
}

impl Command<()> for MigrateProcessInstanceCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<(), crate::error::FlowableError> {
        let mut process_instance =
            validate_change_state_process_instance(command_context, &self.process_instance_id)?;
        let target_definition = {
            let (dm, session) = command_context.dm_and_session();
            dm.get_process_definitions(session)
                .remove(&self.target_process_definition_id)
                .ok_or_else(|| {
                    crate::error::FlowableError::NotFound(format!(
                        "Process definition '{}' was not found",
                        self.target_process_definition_id
                    ))
                })?
        };

        let active_executions =
            active_process_instance_executions(command_context, &self.process_instance_id);
        let activity_mappings = migration_mapping_lookup(&self.activity_migration_mappings)?;

        for execution in &active_executions {
            let Some(source_activity_id) = execution.activity_id.as_deref() else {
                continue;
            };
            let target_activity_ids = activity_mappings
                .get(source_activity_id)
                .cloned()
                .unwrap_or_else(|| vec![source_activity_id.to_string()]);
            for target_activity_id in target_activity_ids {
                ensure_migratable_user_task_wait_state(
                    command_context,
                    &target_definition.id,
                    execution,
                    &target_activity_id,
                )?;
            }
        }

        update_process_instance_definition_metadata(&mut process_instance, &target_definition);
        {
            let (store, session) = command_context.store_and_session();
            store.update_process_instance(&process_instance, session);
        }

        let single_activity_mappings = single_target_activity_mappings(&activity_mappings);
        let multi_target_execution_ids = active_executions
            .iter()
            .filter_map(|execution| {
                let source_activity_id = execution.activity_id.as_deref()?;
                let target_activity_ids = activity_mappings.get(source_activity_id)?;
                (target_activity_ids.len() > 1).then(|| execution.id.clone())
            })
            .collect::<HashSet<_>>();

        let active_execution_ids = active_executions
            .iter()
            .map(|execution| execution.id.clone())
            .collect::<HashSet<_>>();
        let executions_to_update: Vec<_> = {
            let (store, session) = command_context.store_and_session();
            store
                .snapshot_executions(session)
                .into_values()
                .filter(|execution| {
                    execution.process_instance_id.as_deref() == Some(&self.process_instance_id)
                })
                .collect()
        };
        for mut execution in executions_to_update {
            if multi_target_execution_ids.contains(&execution.id) {
                continue;
            }
            let source_activity_id = execution.activity_id.clone();
            let target_activity_id = source_activity_id
                .as_deref()
                .and_then(|activity_id| single_activity_mappings.get(activity_id))
                .cloned();
            update_execution_definition_metadata(&mut execution, &target_definition);
            if active_execution_ids.contains(&execution.id)
                && let Some(target_activity_id) = target_activity_id
            {
                execution.activity_id = Some(target_activity_id.clone());
                execution.activity_name =
                    activity_name(command_context, &target_definition.id, &target_activity_id);
            }
            command_context
                .execution_entity_manager
                .update(&execution, &mut command_context.session);
        }

        for execution in active_executions
            .into_iter()
            .filter(|execution| multi_target_execution_ids.contains(&execution.id))
        {
            let Some(source_activity_id) = execution.activity_id.as_deref() else {
                continue;
            };
            let Some(target_activity_ids) = activity_mappings.get(source_activity_id) else {
                continue;
            };
            move_executions_to_activity_ids(
                command_context,
                &process_instance,
                vec![execution],
                target_activity_ids,
            )?;
        }

        update_migrated_tasks(
            command_context,
            &self.process_instance_id,
            &target_definition,
            &single_activity_mappings,
        );
        update_migrated_wait_states(
            command_context,
            &self.process_instance_id,
            &target_definition,
            &single_activity_mappings,
        );
        update_migrated_historic_state(
            command_context,
            &self.process_instance_id,
            &target_definition,
            &single_activity_mappings,
        );

        command_context.history_manager.record_audit_event(
            "process-instance-migration",
            Some(&self.process_instance_id),
            Some(&target_definition.id),
            Some(&format!(
                "Migrated process instance {} to process definition {}",
                self.process_instance_id, target_definition.id
            )),
            &mut command_context.session,
        );

        Ok(())
    }
}

/// Changes the process definition version of an existing process instance
/// without any migration magic, mirroring Java `SetProcessDefinitionVersionCmd`:
///   - the target version MUST belong to the same definition key/tenant;
///   - every execution's current activity MUST exist in the new version;
///   - runtime executions, the historic process instance and tasks switch to
///     the new definition id in one transaction.
pub struct SetProcessDefinitionVersionCmd {
    process_instance_id: String,
    process_definition_version: i32,
}

// =====================================================================
// P56: migration validation framework + batch + callback.
// Java reference: `ProcessInstanceMigrationManagerImpl.java` (validation,
// pre/post listeners, `ProcessInstanceMigrationBuilder`). The runtime
// migration subset here is constrained to user-task wait states; the
// additions are: (a) a plan/issue/report vocabulary, (b) batch migration,
// and (c) a per-PI callback hook. Java's pre/post listener firing on
// migration is delegated to the engine's existing execution listener
// system — the migration itself swaps definition metadata, so the
// listeners on the target definition still fire naturally on subsequent
// activity entry.
// =====================================================================

/// Java-parity builder for a single migration. Compared to the positional
/// `MigrateProcessInstanceCmd::new(...)`, a `MigrationPlan` carries a
/// human-readable plan name and is the unit of input for batch migration,
/// validation and the per-PI callback hook.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationPlan {
    pub name: Option<String>,
    pub process_instance_id: String,
    pub target_process_definition_id: String,
    pub activity_migration_mappings: Vec<ActivityMigrationMapping>,
}

impl MigrationPlan {
    pub fn new(
        process_instance_id: impl Into<String>,
        target_process_definition_id: impl Into<String>,
    ) -> Self {
        Self {
            name: None,
            process_instance_id: process_instance_id.into(),
            target_process_definition_id: target_process_definition_id.into(),
            activity_migration_mappings: Vec::new(),
        }
    }

    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    pub fn add_activity_migration(
        mut self,
        from_activity_id: impl Into<String>,
        to_activity_ids: Vec<String>,
    ) -> Self {
        self.activity_migration_mappings
            .push(ActivityMigrationMapping {
                from_activity_id: from_activity_id.into(),
                to_activity_ids,
            });
        self
    }
}

/// Severity of a single validation issue, mirroring the
/// `MigrationInstructionValidator` failures collected in Java
/// `MigrationValidationReport`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MigrationValidationSeverity {
    Error,
    Warning,
}

/// A single validation issue. The `code` is a stable identifier so callers
/// can branch on a known list (e.g. `unknown-target-activity`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationValidationIssue {
    pub severity: MigrationValidationSeverity,
    pub code: String,
    pub message: String,
}

impl MigrationValidationIssue {
    pub fn error(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            severity: MigrationValidationSeverity::Error,
            code: code.into(),
            message: message.into(),
        }
    }

    pub fn warning(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            severity: MigrationValidationSeverity::Warning,
            code: code.into(),
            message: message.into(),
        }
    }
}

/// Aggregate report from `validate_migration_plan`. Java semantics: a
/// report with one or more `Error` issues blocks migration; warnings do
/// not.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MigrationValidationReport {
    pub issues: Vec<MigrationValidationIssue>,
}

impl MigrationValidationReport {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, issue: MigrationValidationIssue) {
        self.issues.push(issue);
    }

    pub fn has_errors(&self) -> bool {
        self.issues
            .iter()
            .any(|issue| issue.severity == MigrationValidationSeverity::Error)
    }

    pub fn is_empty(&self) -> bool {
        self.issues.is_empty()
    }
}

/// Per-PI outcome inside a batch migration run. `Ok(())` is success;
/// `Err(message)` is the textual error from the failed plan, matching
/// the row-oriented report emitted by Java
/// `ProcessInstanceMigrationManager.batchMigrateProcessInstances`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationBatchEntryResult {
    pub process_instance_id: String,
    pub plan_name: Option<String>,
    pub outcome: Result<(), String>,
}

/// Batch result, returned to the caller once every plan has been
/// processed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MigrationBatchResult {
    pub results: Vec<MigrationBatchEntryResult>,
}

impl MigrationBatchResult {
    pub fn all_succeeded(&self) -> bool {
        self.results.iter().all(|entry| entry.outcome.is_ok())
    }

    pub fn failures(&self) -> impl Iterator<Item = &MigrationBatchEntryResult> {
        self.results.iter().filter(|entry| entry.outcome.is_err())
    }
}

/// Per-PI migration callback. Mirrors Java's
/// `ProcessInstanceMigrationManagerImpl.MigrationCompletedListener`:
/// `pre_migration` runs after validation passes but before any state
/// mutation; `post_migration` runs after the migration commit. Both
/// receive the original plan and the per-step outcome (or `Ok(())` for
/// pre_migration). Implementations are best-effort: an error returned
/// from the callback is captured in the batch result but does NOT
/// abort the migration.
pub trait MigrationCallback: Send + Sync {
    fn pre_migration(
        &self,
        plan: &MigrationPlan,
        command_context: &mut CommandContext,
    ) -> Result<(), crate::error::FlowableError> {
        let _ = command_context;
        let _ = plan;
        Ok(())
    }

    fn post_migration(
        &self,
        plan: &MigrationPlan,
        result: Result<(), String>,
        command_context: &mut CommandContext,
    ) -> Result<(), crate::error::FlowableError> {
        let _ = command_context;
        let _ = plan;
        let _ = result;
        Ok(())
    }
}

/// Command that runs a list of migration plans in sequence. Each plan
/// gets its own `MigrateProcessInstanceCmd` execution; if one plan
/// fails, the rest still proceed (mirrors Java batch behavior). The
/// optional callback is fired around each individual plan.
pub struct BatchMigrateProcessInstancesCmd {
    plans: Vec<MigrationPlan>,
    callback: Option<Arc<dyn MigrationCallback>>,
}

impl BatchMigrateProcessInstancesCmd {
    pub fn new(plans: Vec<MigrationPlan>) -> Self {
        Self {
            plans,
            callback: None,
        }
    }

    pub fn with_callback(mut self, callback: Arc<dyn MigrationCallback>) -> Self {
        self.callback = Some(callback);
        self
    }
}

impl Command<MigrationBatchResult> for BatchMigrateProcessInstancesCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<MigrationBatchResult, crate::error::FlowableError> {
        let mut results = Vec::with_capacity(self.plans.len());
        for plan in &self.plans {
            if let Some(cb) = &self.callback {
                let _ = cb.pre_migration(plan, command_context);
            }
            let outcome = match MigrateProcessInstanceCmd::new(
                plan.process_instance_id.clone(),
                plan.target_process_definition_id.clone(),
                plan.activity_migration_mappings.clone(),
            )
            .execute(command_context)
            {
                Ok(()) => Ok(()),
                Err(error) => Err(error.to_string()),
            };
            if let Some(cb) = &self.callback {
                let _ = cb.post_migration(plan, outcome.clone(), command_context);
            }
            results.push(MigrationBatchEntryResult {
                process_instance_id: plan.process_instance_id.clone(),
                plan_name: plan.name.clone(),
                outcome,
            });
        }
        Ok(MigrationBatchResult { results })
    }
}

/// Command that wraps [`validate_migration_plan`] so callers can run
/// validation through the standard `CommandExecutor` pipeline
/// (matches Java's `ProcessInstanceMigrationManagerImpl.validateMigration`
/// being callable from any service layer).
pub struct ValidateMigrationPlanCmd {
    plan: MigrationPlan,
}

impl ValidateMigrationPlanCmd {
    pub fn new(plan: MigrationPlan) -> Self {
        Self { plan }
    }
}

impl Command<MigrationValidationReport> for ValidateMigrationPlanCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<MigrationValidationReport, crate::error::FlowableError> {
        Ok(validate_migration_plan(command_context, &self.plan))
    }
}

/// Validate a migration plan WITHOUT applying it. Java
/// `ProcessInstanceMigrationManagerImpl.validateMigration` collects
/// errors from every step (mapping shape, target definition lookup,
/// activity presence, user-task wait-state checks) and returns a
/// `MigrationValidationReport`. We mirror the same shape: all issues
/// are reported, not just the first.
pub(crate) fn validate_migration_plan(
    command_context: &mut CommandContext,
    plan: &MigrationPlan,
) -> MigrationValidationReport {
    let mut report = MigrationValidationReport::new();

    if plan.process_instance_id.trim().is_empty() {
        report.push(MigrationValidationIssue::error(
            "blank-process-instance-id",
            "process instance id must be a non-empty string",
        ));
    }
    if plan.target_process_definition_id.trim().is_empty() {
        report.push(MigrationValidationIssue::error(
            "blank-target-definition-id",
            "target process definition id must be a non-empty string",
        ));
    }

    // Mapping shape errors. `migration_mapping_lookup` already returns
    // a `Result` — re-run it and translate each failure into an
    // issue rather than aborting on the first.
    match migration_mapping_lookup(&plan.activity_migration_mappings) {
        Ok(_) => {}
        Err(crate::error::FlowableError::DeploymentValidationError(message)) => {
            report.push(MigrationValidationIssue::error(
                "invalid-activity-migration-mapping",
                message,
            ));
        }
        Err(other) => {
            report.push(MigrationValidationIssue::error(
                "invalid-activity-migration-mapping",
                other.to_string(),
            ));
        }
    }

    // Target definition lookup.
    let target_definition_exists = {
        let (dm, session) = command_context.dm_and_session();
        dm.get_process_definitions(session)
            .contains_key(&plan.target_process_definition_id)
    };
    if !target_definition_exists {
        report.push(MigrationValidationIssue::error(
            "unknown-target-definition",
            format!(
                "Process definition '{}' was not found",
                plan.target_process_definition_id
            ),
        ));
        return report;
    }

    // Process instance lookup and state.
    let process_instance = command_context
        .runtime_store
        .find_process_instance(&plan.process_instance_id, &mut command_context.session);
    let process_instance = match process_instance {
        Some(pi) if !pi.is_ended => pi,
        Some(_) => {
            report.push(MigrationValidationIssue::error(
                "process-instance-ended",
                format!(
                    "Process instance '{}' has already ended and cannot be migrated",
                    plan.process_instance_id
                ),
            ));
            return report;
        }
        None => {
            report.push(MigrationValidationIssue::error(
                "unknown-process-instance",
                format!(
                    "Process instance '{}' was not found",
                    plan.process_instance_id
                ),
            ));
            return report;
        }
    };

    if process_instance.is_suspended {
        report.push(MigrationValidationIssue::warning(
            "process-instance-suspended",
            format!(
                "Process instance '{}' is suspended; migration will resume it",
                plan.process_instance_id
            ),
        ));
    }

    // Activity mapping checks. Build the lookup so the duplicate /
    // empty check errors are reported once, then iterate.
    let activity_mappings = match migration_mapping_lookup(&plan.activity_migration_mappings) {
        Ok(lookup) => lookup,
        Err(_) => {
            // Already reported as a single error above.
            return report;
        }
    };

    let active_executions =
        active_process_instance_executions(command_context, &plan.process_instance_id);
    if active_executions.is_empty() {
        report.push(MigrationValidationIssue::warning(
            "no-active-executions",
            format!(
                "Process instance '{}' has no active executions; only definition metadata will be updated",
                plan.process_instance_id
            ),
        ));
    }
    for execution in &active_executions {
        let Some(source_activity_id) = execution.activity_id.as_deref() else {
            continue;
        };
        let target_activity_ids = activity_mappings
            .get(source_activity_id)
            .cloned()
            .unwrap_or_else(|| vec![source_activity_id.to_string()]);
        for target_activity_id in &target_activity_ids {
            // Skip the per-execution check if we've already collected the
            // mapping-shape error above.
            if report
                .issues
                .iter()
                .any(|issue| issue.code == "invalid-activity-migration-mapping")
            {
                continue;
            }
            if find_flow_element_in_definition(
                command_context,
                &plan.target_process_definition_id,
                target_activity_id,
            )
            .is_err()
            {
                report.push(MigrationValidationIssue::error(
                    "unknown-target-activity",
                    format!(
                        "Target definition '{}' has no activity '{}'",
                        plan.target_process_definition_id, target_activity_id
                    ),
                ));
                continue;
            }
            if let Err(error) = ensure_migratable_user_task_wait_state(
                command_context,
                &plan.target_process_definition_id,
                execution,
                target_activity_id,
            ) {
                report.push(MigrationValidationIssue::error(
                    "non-migratable-execution",
                    format!(
                        "Execution '{}' cannot be migrated to '{}': {}",
                        execution.id, target_activity_id, error
                    ),
                ));
            }
        }
    }

    report
}

impl SetProcessDefinitionVersionCmd {
    /// Java constructor parity: blank instance id and non-positive versions
    /// are rejected with `FlowableIllegalArgumentException` (BadRequest).
    pub fn new(
        process_instance_id: String,
        process_definition_version: i32,
    ) -> Result<Self, crate::error::FlowableError> {
        if process_instance_id.is_empty() {
            return Err(crate::error::FlowableError::BadRequest(format!(
                "The process instance id is mandatory, but '{}' has been provided.",
                process_instance_id
            )));
        }
        if process_definition_version < 1 {
            return Err(crate::error::FlowableError::BadRequest(format!(
                "The process definition version must be positive, but '{}' has been provided.",
                process_definition_version
            )));
        }
        Ok(Self {
            process_instance_id,
            process_definition_version,
        })
    }
}

impl Command<()> for SetProcessDefinitionVersionCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<(), crate::error::FlowableError> {
        let process_instance = {
            let (store, session) = command_context.store_and_session();
            store.find_process_instance(&self.process_instance_id, session)
        };
        let Some(mut process_instance) = process_instance else {
            // Java: an existing child execution id yields IllegalArgument, an
            // unknown id yields ObjectNotFound.
            let child_execution = {
                let (store, session) = command_context.store_and_session();
                store.find_execution(&self.process_instance_id, session)
            };
            if let Some(execution) = child_execution {
                return Err(crate::error::FlowableError::BadRequest(format!(
                    "A process instance id is required, but the provided id '{}' points to a child execution of process instance '{}'. Please invoke the SetProcessDefinitionVersionCmd with a root execution id.",
                    self.process_instance_id,
                    execution.process_instance_id.as_deref().unwrap_or_default()
                )));
            }
            return Err(crate::error::FlowableError::NotFound(format!(
                "No process instance found for id = '{}'.",
                self.process_instance_id
            )));
        };

        // Resolve the target version within the same key + tenant. Java
        // `DeploymentManager.findDeployedProcessDefinitionByKeyAndVersionAndTenantId`
        // raises FlowableObjectNotFoundException when absent.
        let target_definition = {
            let (dm, session) = command_context.dm_and_session();
            dm.get_process_definitions(session)
                .into_values()
                .find(|definition| {
                    definition.key == process_instance.process_definition_key
                        && definition.version == self.process_definition_version
                        && definition.tenant_id == process_instance.tenant_id
                })
        };
        let Some(target_definition) = target_definition else {
            return Err(crate::error::FlowableError::NotFound(format!(
                "no processes deployed with key = '{}' and version = '{}'",
                process_instance.process_definition_key, self.process_definition_version
            )));
        };

        // Validate that each execution's current activity exists in the new
        // version before switching anything (Java validateAndSwitchVersionOfExecution).
        let executions: Vec<_> = {
            let (store, session) = command_context.store_and_session();
            store
                .snapshot_executions(session)
                .into_values()
                .filter(|execution| {
                    execution.process_instance_id.as_deref() == Some(&self.process_instance_id)
                })
                .collect()
        };
        for execution in &executions {
            let Some(activity_id) = execution.activity_id.as_deref() else {
                continue;
            };
            if find_flow_element_in_definition(command_context, &target_definition.id, activity_id)
                .is_err()
            {
                return Err(crate::error::FlowableError::ExecutionError(format!(
                    "The new process definition (key = '{}') does not contain the current activity (id = '{}') of the process instance (id = '{}').",
                    target_definition.key, activity_id, self.process_instance_id
                )));
            }
        }

        update_process_instance_definition_metadata(&mut process_instance, &target_definition);
        {
            let (store, session) = command_context.store_and_session();
            store.update_process_instance(&process_instance, session);
        }
        for mut execution in executions {
            update_execution_definition_metadata(&mut execution, &target_definition);
            command_context
                .execution_entity_manager
                .update(&execution, &mut command_context.session);
        }

        // Java `HistoryManager.recordProcessDefinitionChange`.
        if let Some(mut historic_instance) = command_context
            .runtime_store
            .get_historic_process_instance(&self.process_instance_id, &mut command_context.session)
        {
            historic_instance.process_definition_id = target_definition.id.clone();
            command_context
                .runtime_store
                .update_historic_process_instance(&historic_instance, &mut command_context.session);
        }

        Ok(())
    }
}

pub struct BulkDeleteProcessInstancesCmd {
    process_instance_ids: Vec<String>,
    delete_reason: Option<String>,
    /// Mirrors Flowable Java's `cascade` flag on
    /// `ExecutionEntityManager.deleteProcessInstanceCascade(deleteHistory)`:
    /// - `false` (default, REST DELETE /runtime/process-instances/{id}):
    ///   historic PI/task rows are kept and merely marked ended via
    ///   `recordProcessInstanceEnd` / `recordActivityTaskEnd`.
    /// - `true` (cascade deployment deletion path): historic PI, historic
    ///   tasks, historic activity instances, historic variables, etc. are
    ///   *deleted* via `recordProcessInstanceDeleted` /
    ///   `TaskHelper.deleteHistoricTask`.
    cascade: bool,
}

impl BulkDeleteProcessInstancesCmd {
    /// Defaults to `cascade=false`, matching Flowable Java's
    /// `DeleteProcessInstanceCmd` (REST DELETE /runtime/process-instances/{id}).
    pub fn new(process_instance_ids: Vec<String>, delete_reason: Option<String>) -> Self {
        Self {
            process_instance_ids,
            delete_reason,
            cascade: false,
        }
    }

    /// `cascade=true` mirrors Flowable Java's cascade path used by
    /// `DeleteDeploymentCmd` → `DeploymentEntityManagerImpl.deleteDeployment`
    /// → `deleteProcessInstancesByProcessDefinition(..., cascade=true)`.
    pub fn new_with_cascade(
        process_instance_ids: Vec<String>,
        delete_reason: Option<String>,
        cascade: bool,
    ) -> Self {
        Self {
            process_instance_ids,
            delete_reason,
            cascade,
        }
    }
}

impl Command<()> for BulkDeleteProcessInstancesCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<(), crate::error::FlowableError> {
        let mut callback_targets: Vec<(String, String, String)> = Vec::new();
        for process_instance_id in &self.process_instance_ids {
            let exists = {
                let (store, session) = command_context.store_and_session();
                store
                    .find_process_instance(process_instance_id, session)
                    .is_some()
            };
            if !exists {
                return Err(crate::error::FlowableError::NotFound(format!(
                    "Process instance '{}' was not found",
                    process_instance_id
                )));
            }
        }

        for process_instance_id in &self.process_instance_ids {
            let process_instance = {
                let (store, session) = command_context.store_and_session();
                store.find_process_instance(process_instance_id, session)
            };
            let Some(process_instance) = process_instance else {
                tracing::error!(
                    process_instance_id = %process_instance_id,
                    "process instance not found despite existence check"
                );
                continue;
            };
            if crate::engine::cmmn_process_task_callback::is_cmmn_process_task_callback(
                &process_instance,
            ) {
                let default_message = format!(
                    "BPMN child process instance '{}' was deleted",
                    process_instance_id
                );
                let message = self.delete_reason.clone().unwrap_or(default_message);
                callback_targets.push((
                    process_instance.id.clone(),
                    process_instance.callback_type.clone().unwrap_or_default(),
                    message,
                ));
            }
        }

        for process_instance_id in &self.process_instance_ids {
            let delete_reason = self.delete_reason.as_deref();
            let tasks = command_context
                .task_entity_manager
                .find_by_process_instance_id(process_instance_id, &mut command_context.session);
            for task in tasks {
                // Java's `TaskHelper.handleTaskHistory`:
                // - cascade=false → `recordActivityTaskEnd` (mark historic task ended)
                // - cascade=true   → `deleteHistoricTask` (delete the historic task row
                //   plus its events and log entries)
                if self.cascade {
                    let (store, session) = command_context.store_and_session();
                    store.delete_historic_task_instance_cascade(&task.id, session);
                } else {
                    command_context.history_manager.record_task_end(
                        &task.id,
                        delete_reason,
                        &mut command_context.session,
                    );
                }
                command_context
                    .task_entity_manager
                    .delete(&task.id, &mut command_context.session);
            }

            // P134/P125: Java ExecutionEntityManagerImpl.java:1050-1077
            // (`deleteEventSubScriptions`) — when the dispatcher is enabled,
            // message subscriptions fire ACTIVITY_MESSAGE_CANCELLED before
            // the bulk delete of event wait rows.
            {
                use crate::persistence::runtime_store::EventSubscriptionKind;
                let waits = command_context
                    .runtime_store
                    .find_event_wait_states_by_process_instance_id(
                        process_instance_id,
                        &mut command_context.session,
                    );
                let process_definition_id = command_context
                    .runtime_store
                    .find_process_instance(process_instance_id, &mut command_context.session)
                    .map(|pi| pi.process_definition_id);
                let message_cancels: Vec<(String, String, String)> = waits
                    .into_iter()
                    .filter_map(|wait| {
                        let sub = wait.event_subscription.as_ref()?;
                        if sub.kind != EventSubscriptionKind::Message {
                            return None;
                        }
                        Some((
                            wait.activity_id.unwrap_or_default(),
                            sub.event_ref.clone(),
                            wait.execution_id,
                        ))
                    })
                    .collect();
                for (activity_id, event_ref, execution_id) in message_cancels {
                    crate::engine::event_dispatcher::dispatch_activity_message_cancelled(
                        command_context,
                        &activity_id,
                        &event_ref,
                        Some(process_instance_id),
                        Some(&execution_id),
                        process_definition_id.as_deref(),
                    );
                }
            }

            {
                let (store, session) = command_context.store_and_session();
                let executions: Vec<_> = store
                    .snapshot_executions(session)
                    .into_values()
                    .filter(|execution| {
                        execution.process_instance_id.as_deref() == Some(process_instance_id)
                    })
                    .collect();
                for execution in executions {
                    store.delete_execution(&execution.id, session);
                }

                store.delete_variables_by_process_instance_id(process_instance_id, session);
                store.delete_event_wait_states_by_process_instance_id(process_instance_id, session);
                store.delete_boundary_event_states_by_process_instance_id(
                    process_instance_id,
                    session,
                );
                store.delete_timer_job_states_by_process_instance_id(process_instance_id, session);
                store.delete_event_subprocess_timer_subscriptions_by_process_instance_id(
                    process_instance_id,
                    session,
                );
                store.delete_event_subprocess_event_subscriptions_by_process_instance_id(
                    process_instance_id,
                    session,
                );
                store.delete_compensation_subscriptions_by_process_instance_id(
                    process_instance_id,
                    session,
                );
            }

            // Java's `deleteProcessInstanceCascade`:
            // - deleteHistory=false → `recordProcessInstanceEnd` (mark historic PI
            //   ended: end_time, delete_reason, state, end_user_id)
            // - deleteHistory=true  → `recordProcessInstanceDeleted` (delete the
            //   historic PI row and its cascade: historic activity instances,
            //   historic tasks, historic variables, historic details, historic
            //   comments, historic task log entries, identity links)
            if self.cascade {
                let (store, session) = command_context.store_and_session();
                store.delete_historic_process_instance_cascade(process_instance_id, session);
            } else {
                command_context.history_manager.record_process_instance_end(
                    process_instance_id,
                    delete_reason,
                    &mut command_context.session,
                );
            }
            {
                let (store, session) = command_context.store_and_session();
                store.delete_process_instance(process_instance_id, session);
            }
        }

        for (process_instance_id, callback_type, message) in callback_targets {
            crate::engine::cmmn_process_task_callback::notify_cmmn_process_task_callback(
                command_context,
                &process_instance_id,
                Some(&callback_type),
                crate::engine::cmmn_process_task_callback::CmmnProcessTaskCallbackOutcome::Failed {
                    failure_message: message,
                },
            )?;
        }

        Ok(())
    }
}

pub struct InjectUserTaskCmd {
    process_instance_id: String,
    task_id: String,
    name: String,
    assignee: Option<String>,
}

impl InjectUserTaskCmd {
    pub fn new(
        process_instance_id: String,
        task_id: String,
        name: String,
        assignee: Option<String>,
    ) -> Self {
        Self {
            process_instance_id,
            task_id,
            name,
            assignee,
        }
    }
}

impl Command<()> for InjectUserTaskCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<(), crate::error::FlowableError> {
        let process_instance = {
            let (store, session) = command_context.store_and_session();
            store
                .find_process_instance(&self.process_instance_id, session)
                .ok_or_else(|| {
                    crate::error::FlowableError::NotFound(format!(
                        "Process instance '{}' was not found",
                        self.process_instance_id
                    ))
                })?
        };

        let execution_id = {
            let (store, session) = command_context.store_and_session();
            store
                .snapshot_executions(session)
                .into_values()
                .filter(|execution| {
                    execution.process_instance_id.as_deref() == Some(&self.process_instance_id)
                        && !execution.is_ended
                })
                .map(|execution| execution.id)
                .min()
                .unwrap_or_else(|| self.process_instance_id.clone())
        };

        let mut task = Task::new(
            self.task_id.clone(),
            self.process_instance_id.clone(),
            execution_id.clone(),
            self.task_id.clone(),
            self.name.clone(),
        );
        task.assignee = self.assignee.clone();
        task.tenant_id = process_instance.tenant_id.clone();
        {
            let (store, session) = command_context.store_and_session();
            store.insert_task(&task, session);
        }
        command_context
            .history_manager
            .record_task_created(&task, &mut command_context.session);
        command_context.history_manager.record_audit_event(
            "inject-task",
            Some(&self.process_instance_id),
            None,
            Some(&format!(
                "Injected task {} into process instance {}",
                self.task_id, process_instance.id
            )),
            &mut command_context.session,
        );
        Ok(())
    }
}

pub struct InjectSubprocessActivityCmd {
    process_instance_id: String,
    activity_id: String,
}

impl InjectSubprocessActivityCmd {
    pub fn new(process_instance_id: String, activity_id: String) -> Self {
        Self {
            process_instance_id,
            activity_id,
        }
    }
}

impl Command<()> for InjectSubprocessActivityCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<(), crate::error::FlowableError> {
        let process_instance =
            validate_change_state_process_instance(command_context, &self.process_instance_id)?;

        match find_flow_element_in_definition(
            command_context,
            &process_instance.process_definition_id,
            &self.activity_id,
        )? {
            FlowElementEnum::SubProcess(_) => {}
            other => {
                let element_type =
                    crate::agenda::continue_process_operation::flow_element_type(&other);
                return Err(crate::error::FlowableError::DeploymentValidationError(
                    format!(
                        "Subprocess injection only supports modeled subProcess activities; activity '{}' is {}",
                        self.activity_id, element_type
                    ),
                ));
            }
        }

        let parent_execution_id = {
            let (store, session) = command_context.store_and_session();
            let found = store.find_execution(&process_instance.id, session);
            if let Some(execution) = found {
                Some(execution.id)
            } else {
                store
                    .snapshot_executions(session)
                    .into_values()
                    .filter(|execution| {
                        execution.process_instance_id.as_deref() == Some(&process_instance.id)
                            && !execution.is_ended
                            && !execution.is_suspended
                    })
                    .map(|execution| execution.id)
                    .min()
            }
        };

        let execution = Execution {
            id: Uuid::new_v4().to_string(),
            parent_id: parent_execution_id,
            super_execution_id: process_instance.super_execution_id.clone(),
            root_process_instance_id: process_instance
                .root_process_instance_id
                .clone()
                .or_else(|| Some(process_instance.id.clone())),
            process_instance_id: Some(process_instance.id.clone()),
            process_definition_id: Some(process_instance.process_definition_id.clone()),
            process_definition_key: Some(process_instance.process_definition_key.clone()),
            process_definition_name: process_instance.process_definition_name.clone(),
            process_definition_version: Some(process_instance.process_definition_version),
            activity_id: Some(self.activity_id.clone()),
            activity_name: activity_name(
                command_context,
                &process_instance.process_definition_id,
                &self.activity_id,
            ),
            name: None,
            description: None,
            is_suspended: false,
            is_ended: false,
            is_active: true,
            is_concurrent: true,
            is_scope: false,
            is_multi_instance_root: false,
            tenant_id: process_instance.tenant_id.clone(),
            ..Default::default()
        };

        command_context
            .execution_entity_manager
            .insert(&execution, &mut command_context.session);
        command_context
            .agenda
            .plan_continue_process_operation(execution);
        command_context.history_manager.record_audit_event(
            "inject-subprocess",
            Some(&process_instance.id),
            Some(&process_instance.process_definition_id),
            Some(&format!(
                "Injected subprocess activity {} into process instance {}",
                self.activity_id, process_instance.id
            )),
            &mut command_context.session,
        );

        Ok(())
    }
}

pub struct InjectStartAfterActivityCmd {
    process_instance_id: String,
    activity_id: String,
}

impl InjectStartAfterActivityCmd {
    pub fn new(process_instance_id: String, activity_id: String) -> Self {
        Self {
            process_instance_id,
            activity_id,
        }
    }
}

impl Command<()> for InjectStartAfterActivityCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<(), crate::error::FlowableError> {
        let process_instance =
            validate_change_state_process_instance(command_context, &self.process_instance_id)?;
        let source_execution =
            find_single_active_execution_for_start_after(command_context, &process_instance.id)?;
        let active_activity_id = source_execution.activity_id.clone().ok_or_else(|| {
            crate::error::FlowableError::DeploymentValidationError(format!(
                "Process instance '{}' has no active activity to move for startAfter injection",
                process_instance.id
            ))
        })?;
        if active_activity_id != self.activity_id {
            return Err(crate::error::FlowableError::DeploymentValidationError(
                format!(
                    "Process instance '{}' is active at activity '{}' and cannot startAfter activity '{}'",
                    process_instance.id, active_activity_id, self.activity_id
                ),
            ));
        }

        let target_activity_id = start_after_target_activity_id(
            command_context,
            &process_instance.process_definition_id,
            &self.activity_id,
        )?;
        move_executions_to_activity_ids(
            command_context,
            &process_instance,
            vec![source_execution],
            &[target_activity_id],
        )
    }
}

pub struct EvaluateConditionalEventsCmd {
    process_instance_id: String,
    variables: HashMap<String, Value>,
}

impl EvaluateConditionalEventsCmd {
    pub fn new(process_instance_id: String, variables: HashMap<String, Value>) -> Self {
        Self {
            process_instance_id,
            variables,
        }
    }
}

impl Command<()> for EvaluateConditionalEventsCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<(), crate::error::FlowableError> {
        let process_instance = {
            let (store, session) = command_context.store_and_session();
            store
                .find_process_instance(&self.process_instance_id, session)
                .ok_or_else(|| {
                    crate::error::FlowableError::NotFound(format!(
                        "Process instance '{}' was not found",
                        self.process_instance_id
                    ))
                })?
        };

        // Java parity: EvaluateConditionalEventsCmd extends
        // NeedsActiveExecutionCmd, which raises FlowableException (500) for a
        // suspended execution.
        if process_instance.is_suspended {
            return Err(crate::error::FlowableError::ExecutionError(format!(
                "Cannot evaluate conditions for a suspended execution '{}'",
                self.process_instance_id
            )));
        }

        // Java parity: an ended instance has no runtime execution anymore, so
        // NeedsActiveExecutionCmd raises FlowableObjectNotFoundException (404).
        if process_instance.is_ended {
            return Err(crate::error::FlowableError::NotFound(format!(
                "execution {} doesn't exist",
                self.process_instance_id
            )));
        }

        if !self.variables.is_empty() {
            let mut root_execution = {
                let (store, session) = command_context.store_and_session();
                store
                    .find_execution(&self.process_instance_id, session)
                    .ok_or_else(|| {
                        crate::error::FlowableError::NotFound(format!(
                            "Root execution for process instance '{}' was not found",
                            self.process_instance_id
                        ))
                    })?
            };
            for (name, value) in &self.variables {
                root_execution.set_process_variable(name.clone(), value.clone());
            }
            command_context
                .execution_entity_manager
                .update(&root_execution, &mut command_context.session);
        }

        let conditional_waits = {
            let (store, session) = command_context.store_and_session();
            store
                .find_event_wait_states_by_process_instance_id(&self.process_instance_id, session)
                .into_iter()
                .filter(|wait| {
                    wait.event_subscription
                        .as_ref()
                        .is_some_and(|sub| sub.kind == EventSubscriptionKind::Conditional)
                })
                .collect::<Vec<_>>()
        };

        let mut intermediate_triggers = Vec::new();
        for wait in conditional_waits {
            let Some(sub) = wait.event_subscription.as_ref() else {
                continue;
            };
            let execution = {
                let (store, session) = command_context.store_and_session();
                store.find_execution(&wait.execution_id, session)
            };
            let Some(execution) = execution else {
                continue;
            };
            let evaluation_execution =
                execution_with_process_variables(command_context, &process_instance, &execution);
            if condition_is_true(&sub.event_ref, &evaluation_execution)? {
                intermediate_triggers.push((wait.execution_id.clone(), sub.event_ref.clone()));
            }
        }

        let mut boundary_triggers = Vec::new();
        let mut boundary_states = {
            let (store, session) = command_context.store_and_session();
            store.find_boundary_event_states_by_process_instance_id(
                &self.process_instance_id,
                session,
            )
        };
        boundary_states.sort_by(|left, right| left.boundary_event_id.cmp(&right.boundary_event_id));
        for boundary_state in boundary_states {
            if boundary_state.event_subscription.kind != EventSubscriptionKind::Conditional {
                continue;
            }
            let host_execution = {
                let (store, session) = command_context.store_and_session();
                store.find_execution(&boundary_state.host_execution_id, session)
            };
            let Some(host_execution) = host_execution else {
                continue;
            };
            let evaluation_execution = execution_with_process_variables(
                command_context,
                &process_instance,
                &host_execution,
            );
            if condition_is_true(
                &boundary_state.event_subscription.event_ref,
                &evaluation_execution,
            )? {
                boundary_triggers.push(boundary_state.boundary_event_id.clone());
            }
        }

        for boundary_event_id in boundary_triggers {
            let cmd =
                TriggerBoundaryEventCmd::new(boundary_event_id, self.process_instance_id.clone());
            cmd.execute(command_context)?;
        }

        for (execution_id, event_ref) in intermediate_triggers {
            let cmd = TriggerEventIntermediateCatchCmd::new(
                EventSubscriptionKind::Conditional,
                event_ref,
                execution_id,
            );
            cmd.execute(command_context)?;
        }

        // --- Event Subprocess evaluation (Java parity) ---
        // Java: EvaluateConditionalEventsOperation.run() lines 52-73
        //   - Finds EventSubProcess elements at the process level and evaluates
        //     their conditional start events.
        //   - For each child execution whose current element is a SubProcess,
        //     evaluates nested event subprocesses.
        let process_definition_id = process_instance.process_definition_id.clone();
        if let Some(bpmn_model) = command_context
            .deployment_manager
            .get_bpmn_model(&process_definition_id)
        {
            if let Some(ref process) = bpmn_model.main_process {
                // Evaluate process-level event subprocesses
                // Java: EvaluateConditionalEventsOperation.run() line 56
                evaluate_event_subprocesses(
                    command_context,
                    &process.flow_elements,
                    &self.process_instance_id,
                    &process_instance,
                )?;

                // Evaluate nested event subprocesses within child SubProcess executions
                // Java: EvaluateConditionalEventsOperation.run() lines 70-73
                let child_executions = command_context
                    .execution_entity_manager
                    .find_executions_by_process_instance_id(
                        &self.process_instance_id,
                        &mut command_context.session,
                    );
                for child_exec in child_executions {
                    if let Some(ref activity_id) = child_exec.activity_id {
                        if let Some(flow_element) = find_flow_element(process, activity_id) {
                            if let FlowElementEnum::SubProcess(sub_process) = flow_element {
                                evaluate_event_subprocesses(
                                    command_context,
                                    &sub_process.flow_elements,
                                    &child_exec.id,
                                    &process_instance,
                                )?;
                            }
                        }
                    }
                }
            }
        }

        Ok(())
    }
}

/// Variables carried by a change-activity-state request.
///
/// Java parity: `ChangeActivityStateBuilder#processVariables` /
/// `ChangeActivityStateBuilder#localVariables`, consumed by
/// `AbstractDynamicStateManager#doMoveExecutionState`. Process variables are written to
/// the process instance execution before the move is actioned so they are visible to the
/// activities the move starts; local variables are keyed by target activity id and written
/// as execution-local variables on the executions created at that activity.
///
/// `local_variables` are persisted in the execution-local scope of the execution the move
/// starts, so they outlive the command and are readable via
/// [`RuntimeService::get_variables_local`]. A key that matches no started activity is ignored.
#[derive(Debug, Clone, Default)]
pub struct ChangeActivityStateVariables {
    pub process_variables: HashMap<String, Value>,
    pub local_variables: HashMap<String, HashMap<String, Value>>,
}

impl ChangeActivityStateVariables {
    pub fn new(
        process_variables: HashMap<String, Value>,
        local_variables: HashMap<String, HashMap<String, Value>>,
    ) -> Self {
        Self {
            process_variables,
            local_variables,
        }
    }
}

pub struct ChangeProcessInstanceActivityStateCmd {
    process_instance_id: String,
    cancel_activity_ids: Vec<String>,
    start_activity_ids: Vec<String>,
    variables: ChangeActivityStateVariables,
}

impl ChangeProcessInstanceActivityStateCmd {
    pub fn new(
        process_instance_id: String,
        cancel_activity_ids: Vec<String>,
        start_activity_ids: Vec<String>,
    ) -> Self {
        Self::with_variables(
            process_instance_id,
            cancel_activity_ids,
            start_activity_ids,
            ChangeActivityStateVariables::default(),
        )
    }

    pub fn with_variables(
        process_instance_id: String,
        cancel_activity_ids: Vec<String>,
        start_activity_ids: Vec<String>,
        variables: ChangeActivityStateVariables,
    ) -> Self {
        Self {
            process_instance_id,
            cancel_activity_ids,
            start_activity_ids,
            variables,
        }
    }
}

impl Command<()> for ChangeProcessInstanceActivityStateCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<(), crate::error::FlowableError> {
        let process_instance =
            validate_change_state_process_instance(command_context, &self.process_instance_id)?;
        validate_change_state_shape(&self.cancel_activity_ids, &self.start_activity_ids)?;

        // Java parity: process variables are applied first "so they are available during
        // the change state". This must also precede reading the executions to move, since
        // the process instance root execution can itself be one of them.
        apply_change_state_process_variables(
            command_context,
            &process_instance.id,
            &self.variables.process_variables,
        )?;

        let cancel_executions = find_active_executions_for_activity_ids(
            command_context,
            &process_instance.id,
            &self.cancel_activity_ids,
        )?;

        if self.start_activity_ids.is_empty() {
            cancel_activity_executions(command_context, &process_instance, cancel_executions)
        } else {
            move_executions_to_activity_ids_with_variables(
                command_context,
                &process_instance,
                cancel_executions,
                &self.start_activity_ids,
                &self.variables.local_variables,
            )
        }
    }
}

pub struct ChangeExecutionActivityStateCmd {
    execution_id: String,
    cancel_activity_ids: Vec<String>,
    start_activity_ids: Vec<String>,
    variables: ChangeActivityStateVariables,
}

impl ChangeExecutionActivityStateCmd {
    pub fn new(
        execution_id: String,
        cancel_activity_ids: Vec<String>,
        start_activity_ids: Vec<String>,
    ) -> Self {
        Self::with_variables(
            execution_id,
            cancel_activity_ids,
            start_activity_ids,
            ChangeActivityStateVariables::default(),
        )
    }

    pub fn with_variables(
        execution_id: String,
        cancel_activity_ids: Vec<String>,
        start_activity_ids: Vec<String>,
        variables: ChangeActivityStateVariables,
    ) -> Self {
        Self {
            execution_id,
            cancel_activity_ids,
            start_activity_ids,
            variables,
        }
    }
}

impl Command<()> for ChangeExecutionActivityStateCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<(), crate::error::FlowableError> {
        if self.cancel_activity_ids.is_empty() && self.start_activity_ids.is_empty() {
            return Err(crate::error::FlowableError::DeploymentValidationError(
                "At least one of cancelActivityIds or startActivityIds must contain an activity id"
                    .to_string(),
            ));
        }
        if self.cancel_activity_ids.len() > 1 {
            return Err(crate::error::FlowableError::DeploymentValidationError(
                "Execution-level change-state supports only one cancelActivityId".to_string(),
            ));
        }
        if !self.cancel_activity_ids.is_empty()
            && self.cancel_activity_ids.len() > 1
            && self.start_activity_ids.len() > 1
        {
            return Err(crate::error::FlowableError::DeploymentValidationError(
                "Only single-to-many or many-to-single activity state changes are supported"
                    .to_string(),
            ));
        }

        let execution = {
            let (store, session) = command_context.store_and_session();
            store
                .find_execution(&self.execution_id, session)
                .ok_or_else(|| {
                    crate::error::FlowableError::NotFound(format!(
                        "Execution '{}' was not found",
                        self.execution_id
                    ))
                })?
        };
        if execution.is_ended {
            return Err(crate::error::FlowableError::NotFound(format!(
                "Execution '{}' was not found",
                self.execution_id
            )));
        }
        if execution.is_suspended {
            return Err(crate::error::FlowableError::ExecutionError(format!(
                "Cannot change state for suspended execution '{}'",
                self.execution_id
            )));
        }

        let process_instance_id = execution.process_instance_id.clone().ok_or_else(|| {
            crate::error::FlowableError::DeploymentValidationError(format!(
                "Execution '{}' is not attached to a process instance",
                self.execution_id
            ))
        })?;
        let process_instance =
            validate_change_state_process_instance(command_context, &process_instance_id)?;

        if let Some(cancel_activity_id) = self.cancel_activity_ids.first()
            && execution.activity_id.as_deref() != Some(cancel_activity_id.as_str())
        {
            return Err(crate::error::FlowableError::DeploymentValidationError(
                format!(
                    "Execution '{}' is not active at activity '{}'",
                    self.execution_id, cancel_activity_id
                ),
            ));
        }

        // Java parity: process variables are applied before the move is actioned. The
        // execution is re-read afterwards because it can be the process instance root
        // execution that just received the variables.
        apply_change_state_process_variables(
            command_context,
            &process_instance.id,
            &self.variables.process_variables,
        )?;
        let execution = if self.variables.process_variables.is_empty() {
            execution
        } else {
            let (store, session) = command_context.store_and_session();
            store
                .find_execution(&self.execution_id, session)
                .ok_or_else(|| {
                    crate::error::FlowableError::NotFound(format!(
                        "Execution '{}' was not found",
                        self.execution_id
                    ))
                })?
        };

        if self.start_activity_ids.is_empty() {
            cancel_activity_executions(command_context, &process_instance, vec![execution])
        } else {
            move_executions_to_activity_ids_with_variables(
                command_context,
                &process_instance,
                vec![execution],
                &self.start_activity_ids,
                &self.variables.local_variables,
            )
        }
    }
}

/// Java parity: `ChangeActivityStateBuilder#moveExecutionToActivityId`.
///
/// True execution-level move: the source execution row keeps its id, parent linkage,
/// and local variable map. Runtime artefacts attached to the previous activity
/// (task, wait state, boundary/timer jobs) are cancelled, then the same execution
/// is continued at the target activity. This is distinct from activityId-level
/// change-state which may cancel one execution and start another when fan-out /
/// fan-in reshapes the tree.
pub struct MoveExecutionToActivityIdCmd {
    execution_id: String,
    activity_id: String,
    variables: ChangeActivityStateVariables,
}

impl MoveExecutionToActivityIdCmd {
    pub fn new(execution_id: String, activity_id: String) -> Self {
        Self::with_variables(
            execution_id,
            activity_id,
            ChangeActivityStateVariables::default(),
        )
    }

    pub fn with_variables(
        execution_id: String,
        activity_id: String,
        variables: ChangeActivityStateVariables,
    ) -> Self {
        Self {
            execution_id,
            activity_id,
            variables,
        }
    }
}

impl Command<()> for MoveExecutionToActivityIdCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<(), crate::error::FlowableError> {
        if self.activity_id.trim().is_empty() {
            return Err(crate::error::FlowableError::DeploymentValidationError(
                "moveExecutionToActivityId requires a non-empty activity id".to_string(),
            ));
        }

        let execution = {
            let (store, session) = command_context.store_and_session();
            store
                .find_execution(&self.execution_id, session)
                .ok_or_else(|| {
                    crate::error::FlowableError::NotFound(format!(
                        "Execution '{}' was not found",
                        self.execution_id
                    ))
                })?
        };
        if execution.is_ended {
            return Err(crate::error::FlowableError::NotFound(format!(
                "Execution '{}' was not found",
                self.execution_id
            )));
        }
        if execution.is_suspended {
            return Err(crate::error::FlowableError::ExecutionError(format!(
                "Cannot change state for suspended execution '{}'",
                self.execution_id
            )));
        }
        // Rust keeps the primary wait-state token on the process-instance root
        // execution for flat processes (parent_id is None). Java always moves a
        // child execution; both shapes are accepted here as long as the token is
        // active at an activity. Reject only a pure root with no activity.
        if execution
            .activity_id
            .as_deref()
            .is_none_or(|id| id.trim().is_empty())
        {
            return Err(crate::error::FlowableError::DeploymentValidationError(
                format!(
                    "Cannot move execution '{}' with no active activity id",
                    self.execution_id
                ),
            ));
        }

        let process_instance_id = execution.process_instance_id.clone().ok_or_else(|| {
            crate::error::FlowableError::DeploymentValidationError(format!(
                "Execution '{}' is not attached to a process instance",
                self.execution_id
            ))
        })?;
        let process_instance =
            validate_change_state_process_instance(command_context, &process_instance_id)?;

        apply_change_state_process_variables(
            command_context,
            &process_instance.id,
            &self.variables.process_variables,
        )?;
        let execution = if self.variables.process_variables.is_empty() {
            execution
        } else {
            let (store, session) = command_context.store_and_session();
            store
                .find_execution(&self.execution_id, session)
                .ok_or_else(|| {
                    crate::error::FlowableError::NotFound(format!(
                        "Execution '{}' was not found",
                        self.execution_id
                    ))
                })?
        };

        move_executions_to_activity_ids_with_variables(
            command_context,
            &process_instance,
            vec![execution],
            &[self.activity_id.clone()],
            &self.variables.local_variables,
        )
    }
}

/// Java parity: `ChangeActivityStateBuilder#enableEventSubProcessStartEvent`
/// (`ChangeActivityStateBuilderImpl.java:177-182`,
/// `AbstractDynamicStateManager#doMoveExecutionState` enable loop).
///
/// Registers a single event-subprocess start-event subscription on the process
/// instance scope so a previously cancelled (e.g. interrupting) or never-armed
/// start event can be triggered again. Does not start the event subprocess
/// itself — the next matching message/signal/timer does.
pub struct EnableEventSubProcessStartEventCmd {
    process_instance_id: String,
    start_event_id: String,
}

impl EnableEventSubProcessStartEventCmd {
    pub fn new(process_instance_id: String, start_event_id: String) -> Self {
        Self {
            process_instance_id,
            start_event_id,
        }
    }
}

impl Command<()> for EnableEventSubProcessStartEventCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<(), crate::error::FlowableError> {
        if self.start_event_id.trim().is_empty() {
            return Err(crate::error::FlowableError::DeploymentValidationError(
                "enableEventSubProcessStartEvent requires a non-empty start event id".to_string(),
            ));
        }
        let process_instance =
            validate_change_state_process_instance(command_context, &self.process_instance_id)?;
        enable_event_subprocess_start_event(
            command_context,
            &process_instance,
            &self.start_event_id,
        )
    }
}

pub struct ActivateExecutionActivityCmd {
    execution_id: String,
    activity_id: String,
}

impl ActivateExecutionActivityCmd {
    pub fn new(execution_id: String, activity_id: String) -> Self {
        Self {
            execution_id,
            activity_id,
        }
    }
}

impl Command<()> for ActivateExecutionActivityCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<(), crate::error::FlowableError> {
        let parent_execution = {
            let (store, session) = command_context.store_and_session();
            store
                .find_execution(&self.execution_id, session)
                .ok_or_else(|| {
                    crate::error::FlowableError::NotFound(format!(
                        "Execution '{}' was not found",
                        self.execution_id
                    ))
                })?
        };

        if parent_execution.is_ended {
            return Err(crate::error::FlowableError::DeploymentValidationError(
                format!(
                    "Cannot activate activity under ended execution '{}'",
                    self.execution_id
                ),
            ));
        }

        let mut parent_execution = parent_execution;
        if !parent_execution.is_scope {
            parent_execution.is_scope = true;
            command_context
                .execution_entity_manager
                .update(&parent_execution, &mut command_context.session);
        }

        let child_execution = Execution {
            id: Uuid::new_v4().to_string(),
            parent_id: Some(parent_execution.id.clone()),
            super_execution_id: None,
            root_process_instance_id: parent_execution.root_process_instance_id.clone(),
            process_instance_id: parent_execution.process_instance_id.clone(),
            process_definition_id: parent_execution.process_definition_id.clone(),
            process_definition_key: parent_execution.process_definition_key.clone(),
            process_definition_name: parent_execution.process_definition_name.clone(),
            process_definition_version: parent_execution.process_definition_version,
            activity_id: Some(self.activity_id.clone()),
            activity_name: activity_name(
                command_context,
                parent_execution
                    .process_definition_id
                    .as_deref()
                    .unwrap_or_default(),
                &self.activity_id,
            ),
            name: None,
            description: None,
            is_suspended: false,
            is_ended: false,
            is_active: true,
            is_concurrent: false,
            is_scope: false,
            is_multi_instance_root: false,
            tenant_id: parent_execution.tenant_id.clone(),
            ..Default::default()
        };

        command_context
            .execution_entity_manager
            .insert(&child_execution, &mut command_context.session);

        command_context
            .agenda
            .plan_continue_process_operation(child_execution);

        Ok(())
    }
}

pub struct ActivateAdhocTaskCmd {
    execution_id: String,
    task_id: String,
}

impl ActivateAdhocTaskCmd {
    pub fn new(execution_id: String, task_id: String) -> Self {
        Self {
            execution_id,
            task_id,
        }
    }
}

impl Command<()> for ActivateAdhocTaskCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<(), crate::error::FlowableError> {
        let execution = {
            let (store, session) = command_context.store_and_session();
            store
                .find_execution(&self.execution_id, session)
                .ok_or_else(|| {
                    crate::error::FlowableError::NotFound(format!(
                        "Execution '{}' was not found",
                        self.execution_id
                    ))
                })?
        };

        if execution.is_ended {
            return Err(crate::error::FlowableError::DeploymentValidationError(
                format!(
                    "Cannot activate task under ended execution '{}'",
                    self.execution_id
                ),
            ));
        }

        let behavior = crate::bpmn::behavior::adhoc_subprocess_activity_behavior::AdhocSubProcessActivityBehavior::new();
        behavior.activate_task(&mut execution.clone(), command_context, &self.task_id)
    }
}

pub struct CompleteAdhocTaskCmd {
    execution_id: String,
    task_id: String,
}

impl CompleteAdhocTaskCmd {
    pub fn new(execution_id: String, task_id: String) -> Self {
        Self {
            execution_id,
            task_id,
        }
    }
}

impl Command<()> for CompleteAdhocTaskCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<(), crate::error::FlowableError> {
        let execution = command_context
            .runtime_store
            .find_execution(&self.execution_id, &mut command_context.session)
            .ok_or_else(|| {
                crate::error::FlowableError::NotFound(format!(
                    "Execution '{}' was not found",
                    self.execution_id
                ))
            })?;

        if execution.is_ended {
            return Err(crate::error::FlowableError::DeploymentValidationError(
                format!(
                    "Cannot complete task under ended execution '{}'",
                    self.execution_id
                ),
            ));
        }

        let behavior = crate::bpmn::behavior::adhoc_subprocess_activity_behavior::AdhocSubProcessActivityBehavior::new();
        behavior.complete_task(&mut execution.clone(), command_context, &self.task_id)
    }
}

/// Java `GetActiveAdhocSubProcessesCmd`.
pub struct GetAdhocSubProcessExecutionsCmd {
    process_instance_id: String,
}

impl GetAdhocSubProcessExecutionsCmd {
    pub fn new(process_instance_id: String) -> Self {
        Self {
            process_instance_id,
        }
    }
}

impl Command<Vec<Execution>> for GetAdhocSubProcessExecutionsCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<Vec<Execution>, crate::error::FlowableError> {
        let executions = command_context
            .execution_entity_manager
            .find_executions_by_process_instance_id(
                &self.process_instance_id,
                &mut command_context.session,
            );
        let mut adhoc = Vec::new();
        for execution in executions {
            let Some(activity_id) = execution.activity_id.as_deref() else {
                continue;
            };
            let Some(process_definition_id) = execution.process_definition_id.as_deref() else {
                continue;
            };
            let Some(bpmn_model) = command_context
                .deployment_manager
                .get_bpmn_model(process_definition_id)
            else {
                continue;
            };
            let Some(process) = bpmn_model.main_process.as_ref() else {
                continue;
            };
            let is_adhoc = process.flow_elements.iter().any(|fe| {
                matches!(
                    fe,
                    FlowElementEnum::AdhocSubProcess(a)
                        if a.sub_process
                            .activity
                            .flow_node
                            .flow_element
                            .base_element
                            .id
                            .as_deref()
                            == Some(activity_id)
                )
            });
            if is_adhoc && !execution.is_ended {
                adhoc.push(execution);
            }
        }
        Ok(adhoc)
    }
}

/// Java `GetEnabledActivitiesForAdhocSubProcessCmd`.
pub struct GetEnabledActivitiesForAdhocSubProcessCmd {
    execution_id: String,
}

impl GetEnabledActivitiesForAdhocSubProcessCmd {
    pub fn new(execution_id: String) -> Self {
        Self { execution_id }
    }
}

impl Command<Vec<crate::bpmn::behavior::adhoc_subprocess_activity_behavior::EnabledAdhocActivity>>
    for GetEnabledActivitiesForAdhocSubProcessCmd
{
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<
        Vec<crate::bpmn::behavior::adhoc_subprocess_activity_behavior::EnabledAdhocActivity>,
        crate::error::FlowableError,
    > {
        let execution = command_context
            .runtime_store
            .find_execution(&self.execution_id, &mut command_context.session)
            .ok_or_else(|| {
                crate::error::FlowableError::NotFound(format!(
                    "No execution found for id '{}'",
                    self.execution_id
                ))
            })?;
        let behavior = crate::bpmn::behavior::adhoc_subprocess_activity_behavior::AdhocSubProcessActivityBehavior::new();
        behavior.get_enabled_activities(&execution, command_context)
    }
}

/// Java `ExecuteActivityForAdhocSubProcessCmd`.
pub struct ExecuteActivityForAdhocSubProcessCmd {
    execution_id: String,
    activity_id: String,
}

impl ExecuteActivityForAdhocSubProcessCmd {
    pub fn new(execution_id: String, activity_id: String) -> Self {
        Self {
            execution_id,
            activity_id,
        }
    }
}

impl Command<Execution> for ExecuteActivityForAdhocSubProcessCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<Execution, crate::error::FlowableError> {
        let execution = command_context
            .runtime_store
            .find_execution(&self.execution_id, &mut command_context.session)
            .ok_or_else(|| {
                crate::error::FlowableError::NotFound(format!(
                    "No execution found for id '{}'",
                    self.execution_id
                ))
            })?;
        let behavior = crate::bpmn::behavior::adhoc_subprocess_activity_behavior::AdhocSubProcessActivityBehavior::new();
        behavior.execute_activity(&mut execution.clone(), command_context, &self.activity_id)
    }
}

/// Java `CompleteAdhocSubProcessCmd`.
pub struct CompleteAdhocSubProcessCmd {
    execution_id: String,
}

impl CompleteAdhocSubProcessCmd {
    pub fn new(execution_id: String) -> Self {
        Self { execution_id }
    }
}

impl Command<()> for CompleteAdhocSubProcessCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<(), crate::error::FlowableError> {
        let execution = command_context
            .runtime_store
            .find_execution(&self.execution_id, &mut command_context.session)
            .ok_or_else(|| {
                crate::error::FlowableError::NotFound(format!(
                    "No execution found for id '{}'",
                    self.execution_id
                ))
            })?;
        let behavior = crate::bpmn::behavior::adhoc_subprocess_activity_behavior::AdhocSubProcessActivityBehavior::new();
        behavior.complete_adhoc_subprocess(&mut execution.clone(), command_context)
    }
}

fn migration_mapping_lookup(
    mappings: &[ActivityMigrationMapping],
) -> Result<HashMap<String, Vec<String>>, crate::error::FlowableError> {
    let mut lookup = HashMap::new();
    for mapping in mappings {
        let target_activity_ids = mapping
            .to_activity_ids
            .iter()
            .filter(|activity_id| !activity_id.trim().is_empty())
            .cloned()
            .collect::<Vec<_>>();
        if mapping.from_activity_id.trim().is_empty() || target_activity_ids.is_empty() {
            return Err(crate::error::FlowableError::DeploymentValidationError(
                "activityMigrationMappings must contain non-empty fromActivityId and toActivityId"
                    .to_string(),
            ));
        }
        if lookup
            .insert(mapping.from_activity_id.clone(), target_activity_ids)
            .is_some()
        {
            return Err(crate::error::FlowableError::DeploymentValidationError(
                format!(
                    "Duplicate activity migration mapping for '{}'",
                    mapping.from_activity_id
                ),
            ));
        }
    }
    Ok(lookup)
}

fn single_target_activity_mappings(
    activity_mappings: &HashMap<String, Vec<String>>,
) -> HashMap<String, String> {
    activity_mappings
        .iter()
        .filter_map(|(source_activity_id, target_activity_ids)| {
            match target_activity_ids.as_slice() {
                [target_activity_id] => {
                    Some((source_activity_id.clone(), target_activity_id.clone()))
                }
                _ => None,
            }
        })
        .collect()
}

fn active_process_instance_executions(
    command_context: &mut CommandContext,
    process_instance_id: &str,
) -> Vec<Execution> {
    let mut executions = command_context
        .runtime_store
        .snapshot_executions(&mut command_context.session)
        .into_values()
        .filter(|execution| {
            execution.process_instance_id.as_deref() == Some(process_instance_id)
                && execution.activity_id.is_some()
                && !execution.is_ended
                && !execution.is_suspended
        })
        .collect::<Vec<_>>();
    executions.sort_by(|left, right| left.id.cmp(&right.id));
    executions
}

fn ensure_migratable_user_task_wait_state(
    command_context: &mut CommandContext,
    target_process_definition_id: &str,
    execution: &Execution,
    target_activity_id: &str,
) -> Result<(), crate::error::FlowableError> {
    if command_context
        .runtime_store
        .find_task_by_execution_id(&execution.id, &mut command_context.session)
        .is_none()
    {
        return Err(crate::error::FlowableError::DeploymentValidationError(
            format!(
                "Execution '{}' is not waiting at a user task and cannot be migrated by the runtime migration subset",
                execution.id
            ),
        ));
    }

    match find_flow_element_in_definition(
        command_context,
        target_process_definition_id,
        target_activity_id,
    )? {
        FlowElementEnum::UserTask(_) => Ok(()),
        _ => Err(crate::error::FlowableError::DeploymentValidationError(
            format!(
                "Target activity '{}' must be a userTask for runtime wait-state migration",
                target_activity_id
            ),
        )),
    }
}

fn update_process_instance_definition_metadata(
    process_instance: &mut ProcessInstance,
    target_definition: &ProcessDefinition,
) {
    process_instance.process_definition_id = target_definition.id.clone();
    process_instance.process_definition_key = target_definition.key.clone();
    process_instance.process_definition_name = target_definition.name.clone();
    process_instance.process_definition_version = target_definition.version;
    process_instance.tenant_id = target_definition.tenant_id.clone();
}

fn update_execution_definition_metadata(
    execution: &mut Execution,
    target_definition: &ProcessDefinition,
) {
    execution.process_definition_id = Some(target_definition.id.clone());
    execution.process_definition_key = Some(target_definition.key.clone());
    execution.process_definition_name = target_definition.name.clone();
    execution.process_definition_version = Some(target_definition.version);
    execution.tenant_id = target_definition.tenant_id.clone();
}

fn update_migrated_tasks(
    command_context: &mut CommandContext,
    process_instance_id: &str,
    target_definition: &ProcessDefinition,
    activity_mappings: &HashMap<String, String>,
) {
    for mut task in command_context
        .runtime_store
        .find_tasks_by_process_instance_id(process_instance_id, &mut command_context.session)
    {
        if let Some(target_activity_id) = activity_mappings.get(&task.task_definition_key) {
            task.task_definition_key = target_activity_id.clone();
            task.name = activity_name(command_context, &target_definition.id, target_activity_id)
                .unwrap_or_else(|| target_activity_id.clone());
            // P97: migrated key/name must reach the historic row through the
            // HistoryManager (Java TaskEntityManager.update → recordTaskInfoChange),
            // not through insert_task's silent sync.
            command_context
                .history_manager
                .record_task_updated(&task, &mut command_context.session);
            command_context
                .task_entity_manager
                .update(&task, &mut command_context.session);
        }
    }
}

fn update_migrated_wait_states(
    command_context: &mut CommandContext,
    process_instance_id: &str,
    target_definition: &ProcessDefinition,
    activity_mappings: &HashMap<String, String>,
) {
    for mut wait_state in command_context
        .runtime_store
        .find_event_wait_states_by_process_instance_id(
            process_instance_id,
            &mut command_context.session,
        )
    {
        let Some(target_activity_id) = wait_state
            .activity_id
            .as_deref()
            .and_then(|activity_id| activity_mappings.get(activity_id))
            .cloned()
        else {
            continue;
        };
        command_context
            .runtime_store
            .delete_event_wait_state_by_execution_id(
                &wait_state.execution_id,
                &mut command_context.session,
            );
        wait_state.activity_id = Some(target_activity_id.clone());
        wait_state.display_name =
            activity_name(command_context, &target_definition.id, &target_activity_id);
        command_context
            .runtime_store
            .insert_event_wait_state(&wait_state, &mut command_context.session);
    }
}

fn update_migrated_historic_state(
    command_context: &mut CommandContext,
    process_instance_id: &str,
    target_definition: &ProcessDefinition,
    activity_mappings: &HashMap<String, String>,
) {
    if let Some(mut historic_instance) = command_context
        .runtime_store
        .get_historic_process_instance(process_instance_id, &mut command_context.session)
    {
        historic_instance.process_definition_id = target_definition.id.clone();
        command_context
            .runtime_store
            .update_historic_process_instance(&historic_instance, &mut command_context.session);
    }

    for mut activity in command_context
        .runtime_store
        .find_historic_activity_instances_by_process_instance_id(
            process_instance_id,
            &mut command_context.session,
        )
        .into_iter()
        .filter(|activity| activity.end_time.is_none())
    {
        let Some(target_activity_id) = activity_mappings.get(&activity.activity_id).cloned() else {
            continue;
        };
        activity.activity_id = target_activity_id.clone();
        activity.activity_name =
            activity_name(command_context, &target_definition.id, &target_activity_id);
        command_context
            .runtime_store
            .update_historic_activity_instance(activity, &mut command_context.session);
    }
}

fn validate_change_state_shape(
    cancel_activity_ids: &[String],
    start_activity_ids: &[String],
) -> Result<(), crate::error::FlowableError> {
    if cancel_activity_ids.is_empty() {
        return Err(crate::error::FlowableError::DeploymentValidationError(
            "cancelActivityIds must contain at least one activity id".to_string(),
        ));
    }
    if !start_activity_ids.is_empty()
        && cancel_activity_ids.len() > 1
        && start_activity_ids.len() > 1
    {
        return Err(crate::error::FlowableError::DeploymentValidationError(
            "Only single-to-many or many-to-single activity state changes are supported"
                .to_string(),
        ));
    }
    Ok(())
}

fn validate_change_state_process_instance(
    command_context: &mut CommandContext,
    process_instance_id: &str,
) -> Result<ProcessInstance, crate::error::FlowableError> {
    let process_instance = command_context
        .runtime_store
        .find_process_instance(process_instance_id, &mut command_context.session)
        .ok_or_else(|| {
            crate::error::FlowableError::NotFound(format!(
                "Process instance '{}' was not found",
                process_instance_id
            ))
        })?;
    if process_instance.is_ended {
        return Err(crate::error::FlowableError::NotFound(format!(
            "Process instance '{}' was not found",
            process_instance_id
        )));
    }
    if process_instance.is_suspended {
        return Err(crate::error::FlowableError::ExecutionError(format!(
            "Cannot change state for suspended process instance '{}'",
            process_instance_id
        )));
    }
    Ok(process_instance)
}

fn find_active_executions_for_activity_ids(
    command_context: &mut CommandContext,
    process_instance_id: &str,
    activity_ids: &[String],
) -> Result<Vec<Execution>, crate::error::FlowableError> {
    let mut executions = Vec::with_capacity(activity_ids.len());
    for activity_id in activity_ids {
        let matches = command_context
            .runtime_store
            .snapshot_executions(&mut command_context.session)
            .into_values()
            .filter(|execution| {
                execution.process_instance_id.as_deref() == Some(process_instance_id)
                    && execution.activity_id.as_deref() == Some(activity_id.as_str())
                    && !execution.is_ended
                    && !execution.is_suspended
            })
            .collect::<Vec<_>>();
        match matches.as_slice() {
            [execution] => executions.push(execution.clone()),
            [] => {
                return Err(crate::error::FlowableError::ExecutionError(format!(
                    "Active execution could not be found with activity id '{}' in process instance '{}'",
                    activity_id, process_instance_id,
                )));
            }
            _ => {
                return Err(crate::error::FlowableError::ExecutionError(format!(
                    "Activity '{}' has multiple active executions; execution-level change-state is required",
                    activity_id
                )));
            }
        }
    }
    Ok(executions)
}

fn find_single_active_execution_for_start_after(
    command_context: &mut CommandContext,
    process_instance_id: &str,
) -> Result<Execution, crate::error::FlowableError> {
    let matches = command_context
        .runtime_store
        .snapshot_executions(&mut command_context.session)
        .into_values()
        .filter(|execution| {
            execution.process_instance_id.as_deref() == Some(process_instance_id)
                && execution
                    .activity_id
                    .as_deref()
                    .is_some_and(|id| !id.trim().is_empty())
                && !execution.is_ended
                && !execution.is_suspended
        })
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [execution] => Ok(execution.clone()),
        [] => Err(crate::error::FlowableError::DeploymentValidationError(
            format!(
                "Process instance '{}' has no active activity to move for startAfter injection",
                process_instance_id
            ),
        )),
        _ => Err(crate::error::FlowableError::DeploymentValidationError(
            format!(
                "startAfter injection requires exactly one active activity in process instance '{}'",
                process_instance_id
            ),
        )),
    }
}

fn start_after_target_activity_id(
    command_context: &mut CommandContext,
    process_definition_id: &str,
    activity_id: &str,
) -> Result<String, crate::error::FlowableError> {
    let flow_element =
        find_flow_element_in_definition(command_context, process_definition_id, activity_id)?;
    let outgoing_flows = outgoing_flows(&flow_element).ok_or_else(|| {
        crate::error::FlowableError::DeploymentValidationError(format!(
            "Activity '{}' does not expose outgoing sequence flows for startAfter injection",
            activity_id
        ))
    })?;
    match outgoing_flows {
        [flow] => flow
            .target_ref
            .clone()
            .filter(|target| !target.trim().is_empty())
            .ok_or_else(|| {
                crate::error::FlowableError::DeploymentValidationError(format!(
                    "Outgoing sequence flow from activity '{}' has no targetRef",
                    activity_id
                ))
            }),
        [] => Err(crate::error::FlowableError::DeploymentValidationError(
            format!(
                "Activity '{}' has no outgoing sequence flow for startAfter injection",
                activity_id
            ),
        )),
        _ => Err(crate::error::FlowableError::DeploymentValidationError(
            format!(
                "Activity '{}' has multiple outgoing sequence flows; startAfter injection only supports a single resolvable successor",
                activity_id
            ),
        )),
    }
}

fn outgoing_flows(flow_element: &FlowElementEnum) -> Option<&[SequenceFlow]> {
    match flow_element {
        FlowElementEnum::Task(task) => Some(&task.activity.flow_node.outgoing_flows),
        FlowElementEnum::UserTask(task) => Some(&task.task.activity.flow_node.outgoing_flows),
        FlowElementEnum::ServiceTask(task) => Some(&task.task.activity.flow_node.outgoing_flows),
        FlowElementEnum::CaseServiceTask(task) => Some(&task.service_task.task.activity.flow_node.outgoing_flows),
        FlowElementEnum::SendTask(task) => {
            Some(&task.service_task.task.activity.flow_node.outgoing_flows)
        }
        FlowElementEnum::ScriptTask(task) => Some(&task.task.activity.flow_node.outgoing_flows),
        FlowElementEnum::ManualTask(task) => Some(&task.task.activity.flow_node.outgoing_flows),
        FlowElementEnum::ReceiveTask(task) => Some(&task.task.activity.flow_node.outgoing_flows),
        FlowElementEnum::BusinessRuleTask(task) => {
            Some(&task.task.activity.flow_node.outgoing_flows)
        }
        FlowElementEnum::StartEvent(event) => Some(&event.event.flow_node.outgoing_flows),
        FlowElementEnum::EndEvent(event) => Some(&event.event.flow_node.outgoing_flows),
        FlowElementEnum::ExclusiveGateway(gateway) => {
            Some(&gateway.gateway.flow_node.outgoing_flows)
        }
        FlowElementEnum::ParallelGateway(gateway) => {
            Some(&gateway.gateway.flow_node.outgoing_flows)
        }
        FlowElementEnum::InclusiveGateway(gateway) => {
            Some(&gateway.gateway.flow_node.outgoing_flows)
        }
        FlowElementEnum::EventBasedGateway(gateway) => {
            Some(&gateway.gateway.flow_node.outgoing_flows)
        }
        FlowElementEnum::ComplexGateway(gateway) => {
            Some(&gateway.gateway.flow_node.outgoing_flows)
        }
        FlowElementEnum::IntermediateCatchEvent(event) => {
            Some(&event.event.flow_node.outgoing_flows)
        }
        FlowElementEnum::IntermediateThrowEvent(event) => {
            Some(&event.event.flow_node.outgoing_flows)
        }
        FlowElementEnum::SubProcess(sub_process) => {
            Some(&sub_process.activity.flow_node.outgoing_flows)
        }
        FlowElementEnum::Transaction(transaction) => {
            Some(&transaction.sub_process.activity.flow_node.outgoing_flows)
        }
        FlowElementEnum::EventSubProcess(event_sub_process) => Some(
            &event_sub_process
                .sub_process
                .activity
                .flow_node
                .outgoing_flows,
        ),
        FlowElementEnum::AdhocSubProcess(sub_process) => {
            Some(&sub_process.sub_process.activity.flow_node.outgoing_flows)
        }
        FlowElementEnum::CallActivity(call_activity) => {
            Some(&call_activity.activity.flow_node.outgoing_flows)
        }
        FlowElementEnum::BoundaryEvent(boundary_event) => {
            Some(&boundary_event.event.flow_node.outgoing_flows)
        }
        FlowElementEnum::SequenceFlow(_) | FlowElementEnum::ValuedDataObject(_) => None,
    }
}

/// Writes change-state process variables onto the process instance root execution.
///
/// Java parity: `AbstractDynamicStateManager#doMoveExecutionState` sets the process
/// instance variables first "so they are available during the change state", i.e. before
/// any target activity is resolved or started.
fn apply_change_state_process_variables(
    command_context: &mut CommandContext,
    process_instance_id: &str,
    process_variables: &HashMap<String, Value>,
) -> Result<(), crate::error::FlowableError> {
    if process_variables.is_empty() {
        return Ok(());
    }
    let mut root_execution = {
        let (store, session) = command_context.store_and_session();
        store
            .find_execution(process_instance_id, session)
            .ok_or_else(|| {
                crate::error::FlowableError::NotFound(format!(
                    "Root execution for process instance '{}' was not found",
                    process_instance_id
                ))
            })?
    };
    for (name, value) in process_variables {
        root_execution.set_process_variable(name.clone(), value.clone());
    }
    command_context
        .execution_entity_manager
        .update(&root_execution, &mut command_context.session);
    Ok(())
}

fn move_executions_to_activity_ids(
    command_context: &mut CommandContext,
    process_instance: &ProcessInstance,
    source_executions: Vec<Execution>,
    start_activity_ids: &[String],
) -> Result<(), crate::error::FlowableError> {
    move_executions_to_activity_ids_with_variables(
        command_context,
        process_instance,
        source_executions,
        start_activity_ids,
        &HashMap::new(),
    )
}

/// True move of source execution(s) onto `start_activity_ids`.
///
/// Identity / variable rules (P55 / Java `AbstractDynamicStateManager` flat move):
/// - The first source execution keeps its **id**, **parent_id**, concurrent/scope flags,
///   and **local_variables** map (merged with any request-keyed locals for the target).
/// - Additional source executions (many→one) are cancelled and deleted after their
///   runtime state is cleaned; their local maps are not merged (MI/fan-in difference —
///   callers that need per-instance locals must use execution-level single moves).
/// - Additional targets (one→many) spawn new sibling executions under the first
///   execution's parent; only the first target reuses the source identity.
/// - Process-instance process variables are untouched here (applied by callers first).
fn move_executions_to_activity_ids_with_variables(
    command_context: &mut CommandContext,
    process_instance: &ProcessInstance,
    mut source_executions: Vec<Execution>,
    start_activity_ids: &[String],
    local_variables: &HashMap<String, HashMap<String, Value>>,
) -> Result<(), crate::error::FlowableError> {
    if source_executions.is_empty() {
        return Err(crate::error::FlowableError::DeploymentValidationError(
            "At least one source execution is required".to_string(),
        ));
    }
    if start_activity_ids.is_empty() {
        return Err(crate::error::FlowableError::DeploymentValidationError(
            "At least one start activity id is required for a move".to_string(),
        ));
    }

    for activity_id in start_activity_ids {
        ensure_activity_can_be_started(
            command_context,
            &process_instance.process_definition_id,
            activity_id,
        )?;
    }

    source_executions.sort_by(|left, right| left.id.cmp(&right.id));
    for execution in &source_executions {
        cancel_execution_runtime_state(command_context, execution);
    }

    // Preserve the first source as the template for identity + locals. Re-read after
    // cancel so any store-side side effects cannot drop the in-memory local map; the
    // cancel path only touches tasks / wait states / timers, not variables.
    let template_execution = source_executions[0].clone();
    for redundant_execution in source_executions.iter().skip(1) {
        command_context
            .execution_entity_manager
            .delete(&redundant_execution.id, &mut command_context.session);
    }

    for (index, activity_id) in start_activity_ids.iter().enumerate() {
        let mut execution = template_execution.clone();
        if index > 0 {
            // Fan-out: new sibling under the same parent, fresh id. Local map starts
            // empty except request-injected locals for this target activity.
            execution.id = Uuid::new_v4().to_string();
            execution.parent_id = template_execution
                .parent_id
                .clone()
                .or_else(|| Some(template_execution.id.clone()));
            execution.local_variables.clear();
            execution.is_concurrent = true;
        }
        execution.activity_id = Some(activity_id.clone());
        execution.activity_name = activity_name(
            command_context,
            &process_instance.process_definition_id,
            activity_id,
        );
        execution.process_instance_id = Some(process_instance.id.clone());
        execution.root_process_instance_id = process_instance
            .root_process_instance_id
            .clone()
            .or_else(|| Some(process_instance.id.clone()));
        execution.process_definition_id = Some(process_instance.process_definition_id.clone());
        execution.process_definition_key = Some(process_instance.process_definition_key.clone());
        execution.process_definition_name = process_instance.process_definition_name.clone();
        execution.process_definition_version = Some(process_instance.process_definition_version);
        execution.tenant_id = process_instance.tenant_id.clone();
        execution.is_ended = false;
        execution.is_suspended = false;
        execution.is_active = true;
        // Java parity: local variables are keyed by the moved-to activity id and applied
        // to the execution created at that activity (merged onto preserved locals).
        if let Some(activity_local_variables) = local_variables.get(activity_id) {
            for (name, value) in activity_local_variables {
                execution.set_local_variable(name.clone(), value.clone());
            }
        }
        command_context
            .execution_entity_manager
            .update(&execution, &mut command_context.session);
        command_context
            .agenda
            .plan_continue_process_operation(execution);
    }

    command_context.history_manager.record_audit_event(
        "change-state",
        Some(&process_instance.id),
        Some(&process_instance.process_definition_id),
        Some(&format!(
            "Changed process instance {} to activity ids {}",
            process_instance.id,
            start_activity_ids.join(",")
        )),
        &mut command_context.session,
    );
    Ok(())
}

/// Registers a single event-subprocess start-event subscription on the process
/// instance root scope. Java:
/// `ProcessInstanceHelper#processEventSubProcessStartEvent` invoked from
/// `AbstractDynamicStateManager` enable-activity containers.
fn enable_event_subprocess_start_event(
    command_context: &mut CommandContext,
    process_instance: &ProcessInstance,
    start_event_id: &str,
) -> Result<(), crate::error::FlowableError> {
    let bpmn_model = command_context
        .deployment_manager
        .get_bpmn_model(&process_instance.process_definition_id)
        .ok_or_else(|| {
            crate::error::FlowableError::DeploymentValidationError(format!(
                "Process definition '{}' was not found for enableEventSubProcessStartEvent",
                process_instance.process_definition_id
            ))
        })?;
    let process = bpmn_model.main_process.as_ref().ok_or_else(|| {
        crate::error::FlowableError::DeploymentValidationError(
            "Process definition has no main process".to_string(),
        )
    })?;

    let (event_subprocess_id, start_event) =
        find_event_subprocess_start_event(process, start_event_id).ok_or_else(|| {
            crate::error::FlowableError::DeploymentValidationError(format!(
                "could not find element for activity id {}",
                start_event_id
            ))
        })?;

    let existing_event_subs = command_context
        .runtime_store
        .find_event_subprocess_event_subscriptions_by_process_instance_id(
            &process_instance.id,
            &mut command_context.session,
        );
    let existing_timer_subs = command_context
        .runtime_store
        .find_event_subprocess_timer_subscriptions_by_process_instance_id(
            &process_instance.id,
            &mut command_context.session,
        );

    let scope_execution_id = process_instance.id.clone();
    let root_execution = command_context
        .runtime_store
        .find_execution(&process_instance.id, &mut command_context.session)
        .ok_or_else(|| {
            crate::error::FlowableError::NotFound(format!(
                "Root execution for process instance '{}' was not found",
                process_instance.id
            ))
        })?;

    // Event Registry event-subprocess start uses empty event defs + eventType
    // (ProcessInstanceHelper.java:371-398). Allow enable for that shape.
    if start_event.event.event_definitions.is_empty() {
        if let Some(event_type) =
            crate::bpmn::behavior::event_registry_event_support::resolve_event_type_extension(
                &start_event.event.flow_node.flow_element.base_element,
            )
        {
            if !existing_event_subs.iter().any(|s| {
                s.start_event_id == start_event_id
                    && s.event_kind == EventSubscriptionKind::EventRegistry
                    && s.event_ref == event_type
            }) {
                // P134/P125: Java ProcessInstanceHelper.java:343-358 —
                // ACTIVITY_MESSAGE/SIGNAL_WAITING on message/signal register.
                crate::engine::event_dispatcher::insert_event_subprocess_subscription_with_waiting(
                    command_context,
                        EventSubprocessEventSubscription {
                            subscription_id: Uuid::new_v4().to_string(),
                            process_instance_id: process_instance.id.clone(),
                            scope_execution_id: Some(scope_execution_id.clone()),
                            scope_activity_id: None,
                            event_subprocess_id: event_subprocess_id.clone(),
                            start_event_id: start_event_id.to_string(),
                            interrupting: start_event.interrupting,
                            event_kind: EventSubscriptionKind::EventRegistry,
                            event_ref: event_type,
                            // Event-subprocess event-registry correlation is
                            // not computed at runtime yet (P93 scope note).
                            configuration: None,
                        },
                    Some(process_instance.process_definition_id.as_str()),
                );
            }
            return Ok(());
        }
        return Err(crate::error::FlowableError::DeploymentValidationError(
            format!(
                "Event subprocess start event '{}' has no event definition to enable",
                start_event_id
            ),
        ));
    }

    for event_def in &start_event.event.event_definitions {
        match event_def {
            EventDefinitionEnum::MessageEventDefinition(msg_def) => {
                let Some(msg_ref) = msg_def
                    .message_ref
                    .as_ref()
                    .filter(|r| !r.trim().is_empty())
                else {
                    continue;
                };
                if existing_event_subs.iter().any(|s| {
                    s.start_event_id == start_event_id
                        && s.event_kind == EventSubscriptionKind::Message
                        && s.event_ref == *msg_ref
                }) {
                    continue;
                }
                // P134/P125: Java ProcessInstanceHelper.java:343-358 —
                // ACTIVITY_MESSAGE/SIGNAL_WAITING on message/signal register.
                crate::engine::event_dispatcher::insert_event_subprocess_subscription_with_waiting(
                    command_context,
                        EventSubprocessEventSubscription {
                            subscription_id: Uuid::new_v4().to_string(),
                            process_instance_id: process_instance.id.clone(),
                            scope_execution_id: Some(scope_execution_id.clone()),
                            scope_activity_id: None,
                            event_subprocess_id: event_subprocess_id.clone(),
                            start_event_id: start_event_id.to_string(),
                            interrupting: start_event.interrupting,
                            event_kind: EventSubscriptionKind::Message,
                            event_ref: msg_ref.clone(),
                            configuration: None,
                        },
                    Some(process_instance.process_definition_id.as_str()),
                );
            }
            EventDefinitionEnum::SignalEventDefinition(sig_def) => {
                let Some(sig_ref) = sig_def.signal_ref.as_ref().filter(|r| !r.trim().is_empty())
                else {
                    continue;
                };
                if existing_event_subs.iter().any(|s| {
                    s.start_event_id == start_event_id
                        && s.event_kind == EventSubscriptionKind::Signal
                        && s.event_ref == *sig_ref
                }) {
                    continue;
                }
                // P134/P125: Java ProcessInstanceHelper.java:343-358 —
                // ACTIVITY_MESSAGE/SIGNAL_WAITING on message/signal register.
                crate::engine::event_dispatcher::insert_event_subprocess_subscription_with_waiting(
                    command_context,
                        EventSubprocessEventSubscription {
                            subscription_id: Uuid::new_v4().to_string(),
                            process_instance_id: process_instance.id.clone(),
                            scope_execution_id: Some(scope_execution_id.clone()),
                            scope_activity_id: None,
                            event_subprocess_id: event_subprocess_id.clone(),
                            start_event_id: start_event_id.to_string(),
                            interrupting: start_event.interrupting,
                            event_kind: EventSubscriptionKind::Signal,
                            event_ref: sig_ref.clone(),
                            configuration: None,
                        },
                    Some(process_instance.process_definition_id.as_str()),
                );
            }
            EventDefinitionEnum::TimerEventDefinition(timer_def) => {
                if existing_timer_subs
                    .iter()
                    .any(|s| s.start_event_id == start_event_id)
                {
                    continue;
                }
                let now = chrono::Utc::now();
                let schedule = crate::bpmn::timer_util::resolve_timer_schedule(
                    timer_def.time_date.as_ref(),
                    timer_def.time_duration.as_ref(),
                    timer_def.time_cycle.as_ref(),
                    timer_def.end_date.as_ref(),
                    timer_def.calendar_name.as_ref(),
                    &root_execution,
                    &command_context.config.business_calendar_registry,
                    now,
                )?;
                let category = crate::bpmn::job_category::resolve_job_category(
                    &start_event.event.flow_node.flow_element.base_element,
                    &root_execution,
                );
                command_context
                    .runtime_store
                    .insert_event_subprocess_timer_subscription(
                        EventSubprocessTimerSubscription {
                            subscription_id: Uuid::new_v4().to_string(),
                            process_instance_id: process_instance.id.clone(),
                            event_subprocess_id: event_subprocess_id.clone(),
                            start_event_id: start_event_id.to_string(),
                            interrupting: start_event.interrupting,
                            time_duration: schedule.time_duration,
                            time_date: schedule.time_date,
                            time_cycle: schedule.time_cycle,
                            end_date: schedule.end_date,
                            calendar_name: schedule.calendar_name,
                            due_time: schedule.due_time,
                            lock_owner: None,
                            lock_time: None,
                            category,
                        },
                        &mut command_context.session,
                    );
            }
            _ => {
                return Err(crate::error::FlowableError::DeploymentValidationError(
                    format!(
                        "enableEventSubProcessStartEvent does not support event definition on '{}'",
                        start_event_id
                    ),
                ));
            }
        }
    }

    command_context.history_manager.record_audit_event(
        "change-state",
        Some(&process_instance.id),
        Some(&process_instance.process_definition_id),
        Some(&format!(
            "Enabled event subprocess start event {} on process instance {}",
            start_event_id, process_instance.id
        )),
        &mut command_context.session,
    );
    Ok(())
}

/// Locate the event subprocess that owns `start_event_id` and return
/// `(eventSubProcessId, startEvent)`.
fn find_event_subprocess_start_event<'a>(
    process: &'a flowable_bpmn_model::model::Process,
    start_event_id: &str,
) -> Option<(String, &'a StartEvent)> {
    fn search_elements<'a>(
        elements: &'a [FlowElementEnum],
        start_event_id: &str,
    ) -> Option<(String, &'a StartEvent)> {
        for element in elements {
            let (sub_process, sub_id) = match element {
                FlowElementEnum::EventSubProcess(esp) => {
                    let id = esp
                        .sub_process
                        .activity
                        .flow_node
                        .flow_element
                        .base_element
                        .id
                        .clone()
                        .unwrap_or_default();
                    (Some(&esp.sub_process), id)
                }
                FlowElementEnum::SubProcess(sub) if sub.triggered_by_event => {
                    let id = sub
                        .activity
                        .flow_node
                        .flow_element
                        .base_element
                        .id
                        .clone()
                        .unwrap_or_default();
                    (Some(sub), id)
                }
                FlowElementEnum::SubProcess(sub) => {
                    if let Some(found) = search_elements(&sub.flow_elements, start_event_id) {
                        return Some(found);
                    }
                    continue;
                }
                FlowElementEnum::Transaction(tx) => {
                    if let Some(found) =
                        search_elements(&tx.sub_process.flow_elements, start_event_id)
                    {
                        return Some(found);
                    }
                    continue;
                }
                FlowElementEnum::AdhocSubProcess(adhoc) => {
                    if let Some(found) =
                        search_elements(&adhoc.sub_process.flow_elements, start_event_id)
                    {
                        return Some(found);
                    }
                    continue;
                }
                _ => continue,
            };
            let Some(sub_process) = sub_process else {
                continue;
            };
            for inner in &sub_process.flow_elements {
                if let FlowElementEnum::StartEvent(start_event) = inner {
                    let id = start_event
                        .event
                        .flow_node
                        .flow_element
                        .base_element
                        .id
                        .as_deref();
                    if id == Some(start_event_id) {
                        return Some((sub_id, start_event));
                    }
                }
            }
            // Nested event subprocesses inside another event subprocess are rare but
            // walk children for completeness.
            if let Some(found) = search_elements(&sub_process.flow_elements, start_event_id) {
                return Some(found);
            }
        }
        None
    }

    search_elements(&process.flow_elements, start_event_id)
}

fn cancel_execution_runtime_state(command_context: &mut CommandContext, execution: &Execution) {
    if let Some(activity_id) = execution.activity_id.as_deref() {
        command_context.history_manager.record_activity_end(
            &execution.id,
            activity_id,
            None,
            &mut command_context.session,
        );
    }
    if let Some(task) = command_context
        .runtime_store
        .find_task_by_execution_id(&execution.id, &mut command_context.session)
    {
        command_context.history_manager.record_task_end(
            &task.id,
            Some("change-state"),
            &mut command_context.session,
        );
        command_context
            .task_entity_manager
            .delete(&task.id, &mut command_context.session);
    }
    command_context
        .runtime_store
        .delete_event_wait_state_by_execution_id(&execution.id, &mut command_context.session);
    command_context
        .runtime_store
        .delete_boundary_event_states_by_host_execution_id(
            &execution.id,
            &mut command_context.session,
        );
    command_context
        .runtime_store
        .delete_timer_job_states_by_execution_id(&execution.id, &mut command_context.session);
}

fn cancel_activity_executions(
    command_context: &mut CommandContext,
    process_instance: &ProcessInstance,
    mut executions: Vec<Execution>,
) -> Result<(), crate::error::FlowableError> {
    executions.sort_by(|left, right| left.id.cmp(&right.id));
    for execution in &executions {
        cancel_execution_runtime_state(command_context, execution);
        command_context
            .execution_entity_manager
            .delete(&execution.id, &mut command_context.session);
    }

    let mut ended_process_instance = None;
    if !process_instance_has_active_runtime_state(command_context, &process_instance.id) {
        let mut updated = process_instance.clone();
        updated.is_ended = true;
        command_context
            .runtime_store
            .update_process_instance(&updated, &mut command_context.session);
        command_context
            .runtime_store
            .delete_event_subprocess_event_subscriptions_by_process_instance_id(
                &process_instance.id,
                &mut command_context.session,
            );
        command_context.history_manager.record_process_instance_end(
            &process_instance.id,
            Some("change-state"),
            &mut command_context.session,
        );
        ended_process_instance = Some(updated);
    }

    command_context.history_manager.record_audit_event(
        "change-state",
        Some(&process_instance.id),
        Some(&process_instance.process_definition_id),
        Some(&format!(
            "Cancelled activity ids {} in process instance {}",
            executions
                .iter()
                .filter_map(|execution| execution.activity_id.as_deref())
                .collect::<Vec<_>>()
                .join(","),
            process_instance.id
        )),
        &mut command_context.session,
    );

    if let Some(ended) = ended_process_instance.as_ref()
        && crate::engine::cmmn_process_task_callback::is_cmmn_process_task_callback(ended)
    {
        let message = format!(
            "BPMN child process instance '{}' was forcibly cancelled via change-state",
            process_instance.id
        );
        crate::engine::cmmn_process_task_callback::notify_cmmn_process_task_callback(
            command_context,
            &ended.id,
            ended.callback_type.as_deref(),
            crate::engine::cmmn_process_task_callback::CmmnProcessTaskCallbackOutcome::Failed {
                failure_message: message,
            },
        )?;
    }

    Ok(())
}

fn process_instance_has_active_runtime_state(
    command_context: &mut CommandContext,
    process_instance_id: &str,
) -> bool {
    let (store, session) = command_context.store_and_session();
    store
        .snapshot_executions(session)
        .into_values()
        .any(|execution| {
            execution.process_instance_id.as_deref() == Some(process_instance_id)
                && !execution.is_ended
                && !execution.is_suspended
        })
        || store
            .find_tasks_by_process_instance_id(process_instance_id, session)
            .into_iter()
            .any(|task| !task.is_completed)
        || !store
            .find_event_wait_states_by_process_instance_id(process_instance_id, session)
            .is_empty()
        || !store
            .find_boundary_event_states_by_process_instance_id(process_instance_id, session)
            .is_empty()
        || !store
            .find_timer_job_states_by_process_instance_id(process_instance_id, session)
            .is_empty()
        || !store
            .find_event_subprocess_timer_subscriptions_by_process_instance_id(
                process_instance_id,
                session,
            )
            .is_empty()
        || !store
            .find_event_subprocess_event_subscriptions_by_process_instance_id(
                process_instance_id,
                session,
            )
            .is_empty()
        || !store
            .find_compensation_subscriptions_by_process_instance_id(process_instance_id, session)
            .is_empty()
}

fn ensure_activity_can_be_started(
    command_context: &mut CommandContext,
    process_definition_id: &str,
    activity_id: &str,
) -> Result<(), crate::error::FlowableError> {
    match find_flow_element_in_definition(command_context, process_definition_id, activity_id)? {
        FlowElementEnum::SequenceFlow(_) | FlowElementEnum::BoundaryEvent(_) => Err(
            crate::error::FlowableError::DeploymentValidationError(format!(
                "Activity '{}' cannot be used as a startActivityId for change-state",
                activity_id
            )),
        ),
        _ => Ok(()),
    }
}

fn activity_name(
    command_context: &mut CommandContext,
    process_definition_id: &str,
    activity_id: &str,
) -> Option<String> {
    let flow_element =
        find_flow_element_in_definition(command_context, process_definition_id, activity_id)
            .ok()?;
    flow_element_display_name(&flow_element).map(|s| s.to_string())
}

fn find_flow_element_in_definition(
    command_context: &mut CommandContext,
    process_definition_id: &str,
    activity_id: &str,
) -> Result<FlowElementEnum, crate::error::FlowableError> {
    let bpmn_model = command_context
        .deployment_manager
        .get_bpmn_model(process_definition_id);
    bpmn_model
        .as_ref()
        .and_then(|model| model.main_process.as_ref())
        .and_then(|process| find_flow_element(process, activity_id))
        .cloned()
        .ok_or_else(|| {
            crate::error::FlowableError::DeploymentValidationError(format!(
                "Activity '{}' was not found in process definition '{}'",
                activity_id, process_definition_id
            ))
        })
}

fn flow_element_display_name(flow_element: &FlowElementEnum) -> Option<&str> {
    match flow_element {
        FlowElementEnum::Task(task) => task.activity.flow_node.flow_element.name.as_deref(),
        FlowElementEnum::UserTask(task) => {
            task.task.activity.flow_node.flow_element.name.as_deref()
        }
        FlowElementEnum::ServiceTask(task) => {
            task.task.activity.flow_node.flow_element.name.as_deref()
        }
        FlowElementEnum::CaseServiceTask(task) => {
            task.service_task.task.activity.flow_node.flow_element.name.as_deref()
        }
        FlowElementEnum::SendTask(task) => {
            task.service_task.task.activity.flow_node.flow_element.name.as_deref()
        }
        FlowElementEnum::ScriptTask(task) => {
            task.task.activity.flow_node.flow_element.name.as_deref()
        }
        FlowElementEnum::ManualTask(task) => {
            task.task.activity.flow_node.flow_element.name.as_deref()
        }
        FlowElementEnum::ReceiveTask(task) => {
            task.task.activity.flow_node.flow_element.name.as_deref()
        }
        FlowElementEnum::BusinessRuleTask(task) => {
            task.task.activity.flow_node.flow_element.name.as_deref()
        }
        FlowElementEnum::StartEvent(event) => event.event.flow_node.flow_element.name.as_deref(),
        FlowElementEnum::EndEvent(event) => event.event.flow_node.flow_element.name.as_deref(),
        FlowElementEnum::IntermediateCatchEvent(event) => {
            event.event.flow_node.flow_element.name.as_deref()
        }
        FlowElementEnum::IntermediateThrowEvent(event) => {
            event.event.flow_node.flow_element.name.as_deref()
        }
        FlowElementEnum::BoundaryEvent(event) => event.event.flow_node.flow_element.name.as_deref(),
        FlowElementEnum::ExclusiveGateway(gateway) => {
            gateway.gateway.flow_node.flow_element.name.as_deref()
        }
        FlowElementEnum::ParallelGateway(gateway) => {
            gateway.gateway.flow_node.flow_element.name.as_deref()
        }
        FlowElementEnum::InclusiveGateway(gateway) => {
            gateway.gateway.flow_node.flow_element.name.as_deref()
        }
        FlowElementEnum::EventBasedGateway(gateway) => {
            gateway.gateway.flow_node.flow_element.name.as_deref()
        }
        FlowElementEnum::ComplexGateway(gateway) => {
            gateway.gateway.flow_node.flow_element.name.as_deref()
        }
        FlowElementEnum::SubProcess(sub_process) => {
            sub_process.activity.flow_node.flow_element.name.as_deref()
        }
        FlowElementEnum::Transaction(transaction) => transaction
            .sub_process
            .activity
            .flow_node
            .flow_element
            .name
            .as_deref(),
        FlowElementEnum::EventSubProcess(event_sub_process) => event_sub_process
            .sub_process
            .activity
            .flow_node
            .flow_element
            .name
            .as_deref(),
        FlowElementEnum::AdhocSubProcess(sub_process) => sub_process
            .sub_process
            .activity
            .flow_node
            .flow_element
            .name
            .as_deref(),
        FlowElementEnum::CallActivity(call_activity) => call_activity
            .activity
            .flow_node
            .flow_element
            .name
            .as_deref(),
        FlowElementEnum::SequenceFlow(sequence_flow) => sequence_flow.flow_element.name.as_deref(),
        FlowElementEnum::ValuedDataObject(_) => None,
    }
    .or_else(|| flow_element_id(flow_element))
}

/// Builds the execution a conditional-event condition is evaluated against.
///
/// Delegates to the shared in-flight EL evaluation entry
/// ([`crate::engine::variable_service::evaluation_execution`]), which merges
/// the parent scope chain and the process-instance scope row. The
/// `process_instance` argument is retained for call-site clarity; the helper
/// resolves the scope row via `execution.process_instance_id`.
pub(crate) fn execution_with_process_variables(
    command_context: &mut CommandContext,
    _process_instance: &ProcessInstance,
    execution: &Execution,
) -> Execution {
    crate::engine::variable_service::evaluation_execution(command_context, execution)
}

/// Evaluates a conditional-event expression against the given execution's
/// variable context. Shared by `EvaluateConditionalEventsCmd` and the
/// conditional gate on `TriggerBoundaryEventCmd` (Java:
/// `ConditionUtil.hasTrueCondition` / `BoundaryConditionalEventActivityBehavior.trigger`).
pub(crate) fn condition_is_true(
    expression_text: &str,
    execution: &Execution,
) -> Result<bool, crate::error::FlowableError> {
    use crate::el::condition::Condition;

    let expression = Box::new(SimpleExpression::new(expression_text.to_string()));
    crate::el::uel_expression_condition::UelExpressionCondition::new(expression)
        .evaluate(None, execution)
}

/// Evaluates conditional start events within event subprocesses.
///
/// Java parity: `EvaluateConditionalEventsOperation.evaluateEventSubProcesses()`
/// (lines 77-121). Iterates through flow elements to find EventSubProcess entries,
/// and for each StartEvent with a ConditionalEventDefinition, evaluates the
/// condition. If true, creates the event subprocess execution tree.
///
/// Non-interrupting event subprocesses are repeatable: each call with a true
/// condition creates a new subprocess instance (Java:
/// `ConditionalEventSubprocessTest.testNonInterruptingSubProcess`).
fn evaluate_event_subprocesses(
    command_context: &mut CommandContext,
    flow_elements: &[FlowElementEnum],
    parent_execution_id: &str,
    process_instance: &ProcessInstance,
) -> Result<(), crate::error::FlowableError> {
    for flow_element in flow_elements {
        if let FlowElementEnum::EventSubProcess(event_sub_process) = flow_element {
            let sub_process = &event_sub_process.sub_process;
            for child in &sub_process.flow_elements {
                if let FlowElementEnum::StartEvent(start_event) = child {
                    // Check if this start event has a conditional event definition
                    let conditional_def = start_event
                        .event
                        .event_definitions
                        .iter()
                        .find_map(|def| {
                            if let flowable_bpmn_model::model::EventDefinitionEnum::ConditionalEventDefinition(cond) = def {
                                Some(cond)
                            } else {
                                None
                            }
                        });

                    if let Some(conditional_def) = conditional_def {
                        // Evaluate the condition
                        let parent_execution = {
                            let (store, session) = command_context.store_and_session();
                            store.find_execution(parent_execution_id, session)
                        };

                        let Some(parent_execution) = parent_execution else {
                            continue;
                        };

                        let evaluation_execution = execution_with_process_variables(
                            command_context,
                            process_instance,
                            &parent_execution,
                        );

                        let condition = conditional_def
                            .condition_expression
                            .as_deref()
                            .unwrap_or("");
                        let condition_is_true = if condition.is_empty() {
                            true
                        } else {
                            condition_is_true(condition, &evaluation_execution)?
                        };

                        if condition_is_true {
                            // Java: interrupting startEvent → delete child executions of
                            // the parent (line 101-103). Non-interrupting: keep parent
                            // and its children intact.
                            if start_event.interrupting {
                                delete_child_executions(command_context, parent_execution_id);
                            }

                            // Create event subprocess scope execution
                            // Java: lines 105-108
                            let es_scope_id = Uuid::new_v4().to_string();
                            let es_scope_execution = Execution {
                                id: es_scope_id.clone(),
                                parent_id: Some(parent_execution_id.to_string()),
                                process_instance_id: Some(process_instance.id.clone()),
                                process_definition_id: parent_execution
                                    .process_definition_id
                                    .clone(),
                                activity_id: flow_element_id(flow_element).map(|s| s.to_string()),
                                is_active: true,
                                is_scope: true,
                                variables: parent_execution.variables.clone(),
                                ..Default::default()
                            };

                            command_context
                                .execution_entity_manager
                                .insert(&es_scope_execution, &mut command_context.session);

                            // Record activity start for the event subprocess
                            // Java: line 109
                            command_context.history_manager.record_activity_start(
                                &flow_element_id(flow_element).unwrap_or(""),
                                sub_process.activity.flow_node.flow_element.name.as_deref(),
                                flow_element_type(flow_element),
                                &process_instance.id,
                                &es_scope_id,
                                &mut command_context.session,
                            );

                            // Create start event execution
                            // Java: lines 111-112
                            let start_event_execution = Execution {
                                id: Uuid::new_v4().to_string(),
                                parent_id: Some(es_scope_id),
                                process_instance_id: Some(process_instance.id.clone()),
                                process_definition_id: parent_execution
                                    .process_definition_id
                                    .clone(),
                                activity_id: flow_element_id(child).map(|s| s.to_string()),
                                is_active: true,
                                is_scope: false,
                                variables: parent_execution.variables.clone(),
                                ..Default::default()
                            };

                            command_context
                                .execution_entity_manager
                                .insert(&start_event_execution, &mut command_context.session);

                            // Plan continue process operation from the start event
                            // Java: line 114
                            command_context
                                .agenda
                                .plan_continue_process_operation(start_event_execution);
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

/// Deletes all child executions of the given parent execution, including their
/// runtime state (tasks, event wait states, boundary event states, timer jobs).
///
/// Java parity: `ExecutionEntityManager.deleteChildExecutions(parentExecution,
/// null, true)` called from `EvaluateConditionalEventsOperation.evaluateEventSubProcesses`
/// for interrupting conditional start events.
///
/// In Rust, the execution tree is flatter than Java: when a single sequence flow
/// leads from the start event to a userTask, the execution is reused (same id)
/// rather than creating a child. This function therefore also cleans up the parent
/// execution's own runtime data (tasks, timers, etc.) — but does NOT delete the
/// parent execution row itself.
fn delete_child_executions(command_context: &mut CommandContext, parent_execution_id: &str) {
    let child_ids: Vec<String> = command_context
        .execution_entity_manager
        .find_child_executions_by_parent_execution_id(
            parent_execution_id,
            &mut command_context.session,
        )
        .into_iter()
        .map(|c| c.id)
        .collect();
    for child_id in child_ids {
        crate::bpmn::behavior::multi_instance_support::delete_execution_tree(
            command_context,
            &child_id,
        );
    }
    // Clean up the parent execution's own runtime data (tasks, timers, etc.).
    // In Rust's flat execution tree, child activities may reuse the parent
    // execution directly instead of creating a new child execution.
    crate::bpmn::behavior::multi_instance_support::delete_execution_related_runtime_data(
        command_context,
        parent_execution_id,
    );
}

struct ExecuteAsyncDelegateCmd {
    process_instance_id: String,
    delegate_name: String,
    result_variable: String,
    fields: serde_json::Map<String, Value>,
}

impl Command<Value> for ExecuteAsyncDelegateCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<Value, crate::error::FlowableError> {
        crate::bpmn::behavior::async_delegate_activity_behavior::execute_async_delegate_on_process_instance(
            command_context,
            &self.process_instance_id,
            &self.delegate_name,
            &self.result_variable,
            self.fields.clone(),
        )
    }
}

/// Typed outcome of rejecting an acquired async job, distinguishing fatal
/// listener errors (which should short-circuit the batch, matching Java
/// `offerJobs` semantics) from release infrastructure failures (which should
/// be aggregated so remaining rejected jobs can still be released).
#[derive(Debug)]
pub(crate) enum AsyncJobRejectOutcome {
    /// Job was successfully rejected and its lock released.
    Released,
    /// A fatal listener error occurred during `JOB_REJECTED` dispatch. The
    /// lock was NOT released. Matching Java semantics, this short-circuits
    /// the batch — remaining jobs stay locked and rely on reset-expired
    /// recovery.
    ListenerFatal(crate::error::FlowableError),
    /// Release infrastructure failure (DB error or CAS mismatch). The lock
    /// was NOT released. This error should be aggregated so remaining
    /// rejected jobs can still be released.
    ReleaseFailure(crate::error::FlowableError),
}

pub struct RuntimeService {
    command_executor: Arc<DefaultCommandExecutor>,
    timer_owner_id: Arc<str>,
    timer_metrics: Arc<TimerCoordinationMetrics>,
}

/// Result of a successful issuer profile update.
/// Carries both the old and new profile so the caller can perform
/// targeted cache invalidation when the issuer changes.
pub struct UpdateIssuerProfileResult {
    pub old_profile: crate::service::issuer_profile::IssuerProfile,
    pub new_profile: crate::service::issuer_profile::IssuerProfile,
}

impl RuntimeService {
    pub fn new(command_executor: Arc<DefaultCommandExecutor>, timer_owner_id: Arc<str>) -> Self {
        Self {
            command_executor,
            timer_owner_id,
            timer_metrics: Arc::new(TimerCoordinationMetrics::new()),
        }
    }

    pub fn create_event_subscription_query(&self) -> EventSubscriptionQuery {
        EventSubscriptionQuery::new(Arc::clone(&self.command_executor))
    }

    pub fn get_variable(
        &self,
        execution_id: String,
        name: String,
    ) -> Result<Option<serde_json::Value>, crate::error::FlowableError> {
        let cmd = crate::engine::variable_service::GetVariableCmd::new(execution_id, name);
        self.command_executor.execute(&cmd)
    }

    pub fn get_variables(
        &self,
        execution_id: String,
    ) -> Result<std::collections::HashMap<String, serde_json::Value>, crate::error::FlowableError>
    {
        let cmd = crate::engine::variable_service::GetVariablesCmd::new(execution_id);
        self.command_executor.execute(&cmd)
    }

    /// Java parity: `RuntimeService#setVariable`. Writes to the scope that already owns the
    /// name (walking up the parent chain), otherwise to the root process instance execution.
    pub fn set_variable(
        &self,
        execution_id: String,
        name: String,
        value: serde_json::Value,
    ) -> Result<(), crate::error::FlowableError> {
        let cmd = crate::engine::variable_service::SetVariableCmd::new(execution_id, name, value);
        self.command_executor.execute(&cmd)
    }

    /// Java parity: `RuntimeService#setVariableLocal`. Writes to this execution's own scope,
    /// shadowing any same-named variable in an ancestor scope without modifying it.
    pub fn set_variable_local(
        &self,
        execution_id: String,
        name: String,
        value: serde_json::Value,
    ) -> Result<(), crate::error::FlowableError> {
        let cmd = crate::engine::variable_service::SetVariablesLocalCmd::new(
            execution_id,
            std::collections::HashMap::from([(name, value)]),
        );
        self.command_executor.execute(&cmd)
    }

    /// Java parity: `RuntimeService#setVariablesLocal`.
    pub fn set_variables_local(
        &self,
        execution_id: String,
        variables: std::collections::HashMap<String, serde_json::Value>,
    ) -> Result<(), crate::error::FlowableError> {
        let cmd =
            crate::engine::variable_service::SetVariablesLocalCmd::new(execution_id, variables);
        self.command_executor.execute(&cmd)
    }

    /// Java parity: `RuntimeService#setVariableAsync`. The value is not written
    /// synchronously; a `set-async-variables` job applies it with owning-scope
    /// (`setVariable`) resolution when the async executor runs it.
    pub fn set_variable_async(
        &self,
        execution_id: String,
        name: String,
        value: serde_json::Value,
    ) -> Result<(), crate::error::FlowableError> {
        let cmd = crate::engine::variable_service::SetAsyncExecutionVariablesCmd::new(
            execution_id,
            std::collections::HashMap::from([(name, value)]),
            false,
        );
        self.command_executor.execute(&cmd)
    }

    /// Java parity: `RuntimeService#setVariablesAsync`.
    pub fn set_variables_async(
        &self,
        execution_id: String,
        variables: std::collections::HashMap<String, serde_json::Value>,
    ) -> Result<(), crate::error::FlowableError> {
        let cmd = crate::engine::variable_service::SetAsyncExecutionVariablesCmd::new(
            execution_id,
            variables,
            false,
        );
        self.command_executor.execute(&cmd)
    }

    /// Java parity: `RuntimeService#setVariableLocalAsync`. The value is not written
    /// synchronously; a `set-async-variables` job applies it to this execution's own
    /// scope when the async executor runs it.
    pub fn set_variable_local_async(
        &self,
        execution_id: String,
        name: String,
        value: serde_json::Value,
    ) -> Result<(), crate::error::FlowableError> {
        let cmd = crate::engine::variable_service::SetAsyncExecutionVariablesCmd::new(
            execution_id,
            std::collections::HashMap::from([(name, value)]),
            true,
        );
        self.command_executor.execute(&cmd)
    }

    /// Java parity: `RuntimeService#setVariablesLocalAsync`.
    pub fn set_variables_local_async(
        &self,
        execution_id: String,
        variables: std::collections::HashMap<String, serde_json::Value>,
    ) -> Result<(), crate::error::FlowableError> {
        let cmd = crate::engine::variable_service::SetAsyncExecutionVariablesCmd::new(
            execution_id,
            variables,
            true,
        );
        self.command_executor.execute(&cmd)
    }

    /// Java parity: `RuntimeService#getVariableLocal`.
    pub fn get_variable_local(
        &self,
        execution_id: String,
        name: String,
    ) -> Result<Option<serde_json::Value>, crate::error::FlowableError> {
        let cmd = crate::engine::variable_service::GetVariableLocalCmd::new(execution_id, name);
        self.command_executor.execute(&cmd)
    }

    /// Java parity: `RuntimeService#getVariablesLocal`. Only this execution's own scope, with
    /// no parent-chain resolution.
    pub fn get_variables_local(
        &self,
        execution_id: String,
    ) -> Result<std::collections::HashMap<String, serde_json::Value>, crate::error::FlowableError>
    {
        let cmd = crate::engine::variable_service::GetVariablesLocalCmd::new(execution_id);
        self.command_executor.execute(&cmd)
    }

    /// Java parity: `RuntimeService#getDataObjects(executionId)`.
    pub fn get_data_objects(
        &self,
        execution_id: String,
    ) -> Result<
        std::collections::HashMap<String, crate::engine::data_object_service::DataObject>,
        crate::error::FlowableError,
    > {
        let cmd = crate::engine::data_object_service::GetDataObjectsCmd::new(execution_id, false);
        self.command_executor.execute(&cmd)
    }

    /// Java parity: `RuntimeService#getDataObjectsLocal(executionId)`.
    pub fn get_data_objects_local(
        &self,
        execution_id: String,
    ) -> Result<
        std::collections::HashMap<String, crate::engine::data_object_service::DataObject>,
        crate::error::FlowableError,
    > {
        let cmd = crate::engine::data_object_service::GetDataObjectsCmd::new(execution_id, true);
        self.command_executor.execute(&cmd)
    }

    /// Java parity: `RuntimeService#getDataObjects(executionId, names)`.
    pub fn get_data_objects_by_names(
        &self,
        execution_id: String,
        names: Vec<String>,
    ) -> Result<
        std::collections::HashMap<String, crate::engine::data_object_service::DataObject>,
        crate::error::FlowableError,
    > {
        let cmd = crate::engine::data_object_service::GetDataObjectsCmd::with_names(
            execution_id,
            names,
            false,
        );
        self.command_executor.execute(&cmd)
    }

    /// Java parity: `RuntimeService#getDataObjectsLocal(executionId, names)`.
    pub fn get_data_objects_local_by_names(
        &self,
        execution_id: String,
        names: Vec<String>,
    ) -> Result<
        std::collections::HashMap<String, crate::engine::data_object_service::DataObject>,
        crate::error::FlowableError,
    > {
        let cmd = crate::engine::data_object_service::GetDataObjectsCmd::with_names(
            execution_id,
            names,
            true,
        );
        self.command_executor.execute(&cmd)
    }

    /// Java parity: `RuntimeService#getDataObject(executionId, name)`.
    pub fn get_data_object(
        &self,
        execution_id: String,
        name: String,
    ) -> Result<Option<crate::engine::data_object_service::DataObject>, crate::error::FlowableError>
    {
        let cmd =
            crate::engine::data_object_service::GetDataObjectCmd::new(execution_id, name, false);
        self.command_executor.execute(&cmd)
    }

    /// Java parity: `RuntimeService#getDataObjectLocal(executionId, name)`.
    pub fn get_data_object_local(
        &self,
        execution_id: String,
        name: String,
    ) -> Result<Option<crate::engine::data_object_service::DataObject>, crate::error::FlowableError>
    {
        let cmd =
            crate::engine::data_object_service::GetDataObjectCmd::new(execution_id, name, true);
        self.command_executor.execute(&cmd)
    }

    /// Java parity: `RuntimeService#hasVariableLocal`.
    pub fn has_variable_local(
        &self,
        execution_id: String,
        name: String,
    ) -> Result<bool, crate::error::FlowableError> {
        let cmd = crate::engine::variable_service::HasVariableLocalCmd::new(execution_id, name);
        self.command_executor.execute(&cmd)
    }

    /// Java parity: `RuntimeService#hasVariable` — visibility including ancestor scopes.
    pub fn has_variable(
        &self,
        execution_id: String,
        name: String,
    ) -> Result<bool, crate::error::FlowableError> {
        let cmd = crate::engine::variable_service::HasVariableCmd::new(execution_id, name);
        self.command_executor.execute(&cmd)
    }

    /// Java parity: `RuntimeService#removeVariableLocal`.
    pub fn remove_variable_local(
        &self,
        execution_id: String,
        name: String,
    ) -> Result<(), crate::error::FlowableError> {
        let cmd =
            crate::engine::variable_service::RemoveVariablesLocalCmd::new(execution_id, vec![name]);
        self.command_executor.execute(&cmd)
    }

    /// Java parity: `RuntimeService#removeVariablesLocal`.
    pub fn remove_variables_local(
        &self,
        execution_id: String,
        names: Vec<String>,
    ) -> Result<(), crate::error::FlowableError> {
        let cmd =
            crate::engine::variable_service::RemoveVariablesLocalCmd::new(execution_id, names);
        self.command_executor.execute(&cmd)
    }

    /// Submit an async-capable local service-task delegate, wait for completion, and
    /// store the JSON result on the process instance under `result_variable`.
    ///
    /// Work is submitted to `future_task_executor` when configured; otherwise it runs
    /// synchronously on the command thread.
    pub fn execute_async_delegate(
        &self,
        process_instance_id: String,
        delegate_name: String,
        result_variable: String,
    ) -> Result<serde_json::Value, crate::error::FlowableError> {
        self.execute_async_delegate_with_fields(
            process_instance_id,
            delegate_name,
            result_variable,
            serde_json::Map::new(),
        )
    }

    /// Same as [`Self::execute_async_delegate`] with optional field extensions.
    pub fn execute_async_delegate_with_fields(
        &self,
        process_instance_id: String,
        delegate_name: String,
        result_variable: String,
        fields: serde_json::Map<String, serde_json::Value>,
    ) -> Result<serde_json::Value, crate::error::FlowableError> {
        let cmd = ExecuteAsyncDelegateCmd {
            process_instance_id,
            delegate_name,
            result_variable,
            fields,
        };
        self.command_executor.execute(&cmd)
    }

    pub fn timer_metrics(&self) -> Arc<crate::engine::runtime_service::TimerCoordinationMetrics> {
        Arc::clone(&self.timer_metrics)
    }

    // ── Issuer Profile admin operations ──
    //
    // These methods replace the former `get_store()` escape hatch for the
    // timer coordination service HTTP handlers. Each method encapsulates
    // session creation, commit/rollback, and returns enough context for the
    // caller to perform side effects (e.g. jwks_cache invalidation).

    pub fn list_issuer_profiles(&self) -> Vec<crate::service::issuer_profile::IssuerProfile> {
        let store = self.command_executor.runtime_store();
        let mut session = store.create_session().unwrap();
        let profiles = store.list_issuer_profiles(&mut session);
        session.rollback().unwrap();
        profiles
    }

    pub fn find_issuer_profile(
        &self,
        profile_id: &str,
    ) -> Option<crate::service::issuer_profile::IssuerProfile> {
        let store = self.command_executor.runtime_store();
        let mut session = store.create_session().unwrap();
        let found = store.find_issuer_profile(profile_id, &mut session);
        session.rollback().unwrap();
        found
    }

    pub fn insert_issuer_profile(
        &self,
        profile: crate::service::issuer_profile::IssuerProfile,
    ) -> crate::service::issuer_profile::IssuerProfile {
        let store = self.command_executor.runtime_store();
        let mut session = store.create_session().unwrap();
        store.insert_issuer_profile(profile.clone(), &mut session);
        session.flush_and_commit().unwrap();
        profile
    }

    /// Result of a successful issuer profile update.
    /// Carries both the old and new profile so the caller can perform
    /// targeted cache invalidation when the issuer changes.
    pub fn update_issuer_profile(
        &self,
        profile: crate::service::issuer_profile::IssuerProfile,
        expected_version: i64,
    ) -> Result<UpdateIssuerProfileResult, crate::persistence::StorageError> {
        let store = self.command_executor.runtime_store();
        let mut session = store.create_session().unwrap();
        let old_profile = store.find_issuer_profile(&profile.id, &mut session);
        match old_profile {
            Some(old) => {
                match store.update_issuer_profile(profile.clone(), expected_version, &mut session) {
                    Ok(()) => {
                        let new_profile = store
                            .find_issuer_profile(&profile.id, &mut session)
                            .unwrap_or(profile);
                        session.flush_and_commit().unwrap();
                        Ok(UpdateIssuerProfileResult {
                            old_profile: old,
                            new_profile,
                        })
                    }
                    Err(e) => {
                        session.rollback().unwrap();
                        Err(e)
                    }
                }
            }
            None => {
                session.rollback().unwrap();
                Err(crate::persistence::StorageError::Sql(
                    "Issuer profile not found".into(),
                ))
            }
        }
    }

    /// Returns the deleted profile (if any) so the caller can invalidate
    /// the jwks_cache for the affected issuer.
    pub fn delete_issuer_profile(
        &self,
        profile_id: &str,
    ) -> Option<crate::service::issuer_profile::IssuerProfile> {
        let store = self.command_executor.runtime_store();
        let mut session = store.create_session().unwrap();
        let found_profile = store.find_issuer_profile(profile_id, &mut session);
        store.delete_issuer_profile(profile_id, &mut session);
        session.flush_and_commit().unwrap();
        found_profile
    }

    // ── Identity runtime construction ──
    //
    // Replaces the former `get_store()` calls used by
    // `TimerCoordinationService` when building identity components.

    pub fn build_identity_runtime(
        &self,
        config: &crate::service::config::ServicePolicyConfig,
    ) -> crate::service::config::IdentityRuntimeComponents {
        let store = self.command_executor.runtime_store().clone();
        config.build_identity_runtime(store)
    }

    pub fn build_identity_runtime_with_components(
        &self,
        config: &crate::service::config::ServicePolicyConfig,
        profiles: Vec<crate::service::issuer_profile::IssuerProfile>,
        jwks_cache: Arc<crate::service::jwks::JwksCache>,
        revocation_registry: Arc<crate::service::revocation::TokenRevocationRegistry>,
    ) -> crate::service::config::IdentityRuntimeComponents {
        let store = self.command_executor.runtime_store().clone();
        config.build_identity_runtime_with_components(
            profiles,
            jwks_cache,
            revocation_registry,
            store,
        )
    }

    pub fn timer_owner_id(&self) -> &str {
        self.timer_owner_id.as_ref()
    }

    /// Starts creating a new process instance
    pub fn create_process_instance_builder(&self) -> ProcessInstanceBuilder {
        ProcessInstanceBuilder::new()
    }

    /// Starts a process instance directly by builder
    pub fn start_process_instance(
        &self,
        builder: ProcessInstanceBuilder,
    ) -> Result<ProcessInstance, crate::error::FlowableError> {
        let cmd = StartProcessInstanceCmd::new(builder);
        self.command_executor.execute(&cmd)
    }

    /// Starts a process instance asynchronously by creating an async-continuation
    /// job instead of executing the start event synchronously. The job will be
    /// picked up by the AsyncExecutor's acquisition thread.
    pub fn start_process_instance_async(
        &self,
        builder: ProcessInstanceBuilder,
    ) -> Result<ProcessInstance, crate::error::FlowableError> {
        let cmd =
            crate::cmd::start_process_instance_cmd::StartProcessInstanceAsyncCmd::new(builder);
        self.command_executor.execute(&cmd)
    }

    pub fn set_process_instances_suspended_by_definition_id(
        &self,
        process_definition_id: &str,
        suspended: bool,
    ) -> Result<usize, crate::error::FlowableError> {
        let cmd = SuspendProcessInstancesByDefinitionCmd::new(
            process_definition_id.to_string(),
            suspended,
        );
        self.command_executor.execute(&cmd)
    }

    pub fn bulk_delete_process_instances(
        &self,
        process_instance_ids: Vec<String>,
        delete_reason: Option<String>,
    ) -> Result<(), crate::error::FlowableError> {
        let cmd = BulkDeleteProcessInstancesCmd::new(process_instance_ids, delete_reason);
        self.command_executor.execute(&cmd)
    }

    pub fn delete_process_instance(
        &self,
        process_instance_id: String,
        delete_reason: Option<String>,
    ) -> Result<(), crate::error::FlowableError> {
        self.bulk_delete_process_instances(vec![process_instance_id], delete_reason)
    }

    pub fn update_process_instance(
        &self,
        process_instance_id: String,
        updates: ProcessInstanceUpdate,
    ) -> Result<ProcessInstance, crate::error::FlowableError> {
        let cmd = UpdateProcessInstanceFieldsCmd::new(process_instance_id, updates, None);
        self.command_executor.execute(&cmd)
    }

    pub fn suspend_process_instance(
        &self,
        process_instance_id: String,
        updates: ProcessInstanceUpdate,
    ) -> Result<ProcessInstance, crate::error::FlowableError> {
        let cmd = UpdateProcessInstanceFieldsCmd::new(process_instance_id, updates, Some(true));
        self.command_executor.execute(&cmd)
    }

    pub fn activate_process_instance(
        &self,
        process_instance_id: String,
        updates: ProcessInstanceUpdate,
    ) -> Result<ProcessInstance, crate::error::FlowableError> {
        let cmd = UpdateProcessInstanceFieldsCmd::new(process_instance_id, updates, Some(false));
        self.command_executor.execute(&cmd)
    }

    pub fn inject_user_task(
        &self,
        process_instance_id: String,
        task_id: String,
        name: String,
        assignee: Option<String>,
    ) -> Result<(), crate::error::FlowableError> {
        let cmd = InjectUserTaskCmd::new(process_instance_id, task_id, name, assignee);
        self.command_executor.execute(&cmd)
    }

    pub fn inject_subprocess_activity(
        &self,
        process_instance_id: String,
        activity_id: String,
    ) -> Result<(), crate::error::FlowableError> {
        let cmd = InjectSubprocessActivityCmd::new(process_instance_id, activity_id);
        self.command_executor.execute(&cmd)
    }

    pub fn inject_start_after_activity(
        &self,
        process_instance_id: String,
        activity_id: String,
    ) -> Result<(), crate::error::FlowableError> {
        let cmd = InjectStartAfterActivityCmd::new(process_instance_id, activity_id);
        self.command_executor.execute(&cmd)
    }

    pub fn evaluate_conditional_events(
        &self,
        process_instance_id: String,
        variables: HashMap<String, Value>,
    ) -> Result<(), crate::error::FlowableError> {
        let cmd = EvaluateConditionalEventsCmd::new(process_instance_id, variables);
        self.command_executor.execute(&cmd)
    }

    pub fn change_process_instance_activity_state(
        &self,
        process_instance_id: String,
        cancel_activity_ids: Vec<String>,
        start_activity_ids: Vec<String>,
    ) -> Result<(), crate::error::FlowableError> {
        let cmd = ChangeProcessInstanceActivityStateCmd::new(
            process_instance_id,
            cancel_activity_ids,
            start_activity_ids,
        );
        self.command_executor.execute(&cmd)
    }

    /// Change the activity state of a process instance, injecting variables as part of the
    /// same transaction.
    ///
    /// Java parity: `ChangeActivityStateBuilder#processVariables` /
    /// `#localVariables`. `process_variables` are written to the process instance before
    /// the move is actioned; `local_variables` are keyed by target activity id and applied
    /// as execution-local variables on the executions started by the move.
    pub fn change_process_instance_activity_state_with_variables(
        &self,
        process_instance_id: String,
        cancel_activity_ids: Vec<String>,
        start_activity_ids: Vec<String>,
        process_variables: HashMap<String, Value>,
        local_variables: HashMap<String, HashMap<String, Value>>,
    ) -> Result<(), crate::error::FlowableError> {
        let cmd = ChangeProcessInstanceActivityStateCmd::with_variables(
            process_instance_id,
            cancel_activity_ids,
            start_activity_ids,
            ChangeActivityStateVariables::new(process_variables, local_variables),
        );
        self.command_executor.execute(&cmd)
    }

    pub fn change_execution_activity_state(
        &self,
        execution_id: String,
        cancel_activity_ids: Vec<String>,
        start_activity_ids: Vec<String>,
    ) -> Result<(), crate::error::FlowableError> {
        let cmd = ChangeExecutionActivityStateCmd::new(
            execution_id,
            cancel_activity_ids,
            start_activity_ids,
        );
        self.command_executor.execute(&cmd)
    }

    /// Execution-level counterpart of
    /// [`Self::change_process_instance_activity_state_with_variables`].
    pub fn change_execution_activity_state_with_variables(
        &self,
        execution_id: String,
        cancel_activity_ids: Vec<String>,
        start_activity_ids: Vec<String>,
        process_variables: HashMap<String, Value>,
        local_variables: HashMap<String, HashMap<String, Value>>,
    ) -> Result<(), crate::error::FlowableError> {
        let cmd = ChangeExecutionActivityStateCmd::with_variables(
            execution_id,
            cancel_activity_ids,
            start_activity_ids,
            ChangeActivityStateVariables::new(process_variables, local_variables),
        );
        self.command_executor.execute(&cmd)
    }

    /// Java parity: `ChangeActivityStateBuilder#moveExecutionToActivityId`.
    ///
    /// True execution-level move: preserves the execution id, parent linkage, and
    /// existing local variables while relocating the token to `activity_id`.
    pub fn move_execution_to_activity_id(
        &self,
        execution_id: String,
        activity_id: String,
    ) -> Result<(), crate::error::FlowableError> {
        let cmd = MoveExecutionToActivityIdCmd::new(execution_id, activity_id);
        self.command_executor.execute(&cmd)
    }

    /// Variable-injecting variant of [`Self::move_execution_to_activity_id`].
    pub fn move_execution_to_activity_id_with_variables(
        &self,
        execution_id: String,
        activity_id: String,
        process_variables: HashMap<String, Value>,
        local_variables: HashMap<String, HashMap<String, Value>>,
    ) -> Result<(), crate::error::FlowableError> {
        let cmd = MoveExecutionToActivityIdCmd::with_variables(
            execution_id,
            activity_id,
            ChangeActivityStateVariables::new(process_variables, local_variables),
        );
        self.command_executor.execute(&cmd)
    }

    /// Java parity: `ChangeActivityStateBuilder#enableEventSubProcessStartEvent`.
    ///
    /// Arms the named event-subprocess start event on a running process instance so
    /// a subsequent matching message/signal/timer can trigger the event subprocess.
    pub fn enable_event_subprocess_start_event(
        &self,
        process_instance_id: String,
        start_event_id: String,
    ) -> Result<(), crate::error::FlowableError> {
        let cmd = EnableEventSubProcessStartEventCmd::new(process_instance_id, start_event_id);
        self.command_executor.execute(&cmd)
    }

    pub fn activate_execution_activity(
        &self,
        execution_id: String,
        activity_id: String,
    ) -> Result<(), crate::error::FlowableError> {
        let cmd = ActivateExecutionActivityCmd::new(execution_id, activity_id);
        self.command_executor.execute(&cmd)
    }

    pub fn activate_adhoc_task(
        &self,
        execution_id: &str,
        task_id: &str,
    ) -> Result<(), crate::error::FlowableError> {
        let cmd = ActivateAdhocTaskCmd::new(execution_id.to_string(), task_id.to_string());
        self.command_executor.execute(&cmd)
    }

    pub fn complete_adhoc_task(
        &self,
        execution_id: &str,
        task_id: &str,
    ) -> Result<(), crate::error::FlowableError> {
        let cmd = CompleteAdhocTaskCmd::new(execution_id.to_string(), task_id.to_string());
        self.command_executor.execute(&cmd)
    }

    /// Java parity: `RuntimeService#getAdhocSubProcessExecutions`.
    pub fn get_adhoc_subprocess_executions(
        &self,
        process_instance_id: &str,
    ) -> Result<Vec<Execution>, crate::error::FlowableError> {
        let cmd = GetAdhocSubProcessExecutionsCmd::new(process_instance_id.to_string());
        self.command_executor.execute(&cmd)
    }

    /// Java parity: `RuntimeService#getEnabledActivitiesFromAdhocSubProcess`.
    pub fn get_enabled_activities_from_adhoc_subprocess(
        &self,
        execution_id: &str,
    ) -> Result<
        Vec<crate::bpmn::behavior::adhoc_subprocess_activity_behavior::EnabledAdhocActivity>,
        crate::error::FlowableError,
    > {
        let cmd = GetEnabledActivitiesForAdhocSubProcessCmd::new(execution_id.to_string());
        self.command_executor.execute(&cmd)
    }

    /// Java parity: `RuntimeService#executeActivityInAdhocSubProcess`.
    pub fn execute_activity_in_adhoc_subprocess(
        &self,
        execution_id: &str,
        activity_id: &str,
    ) -> Result<Execution, crate::error::FlowableError> {
        let cmd = ExecuteActivityForAdhocSubProcessCmd::new(
            execution_id.to_string(),
            activity_id.to_string(),
        );
        self.command_executor.execute(&cmd)
    }

    /// Java parity: `RuntimeService#completeAdhocSubProcess`.
    pub fn complete_adhoc_subprocess(
        &self,
        execution_id: &str,
    ) -> Result<(), crate::error::FlowableError> {
        let cmd = CompleteAdhocSubProcessCmd::new(execution_id.to_string());
        self.command_executor.execute(&cmd)
    }

    pub fn migrate_process_instance(
        &self,
        process_instance_id: String,
        target_process_definition_id: String,
        activity_migration_mappings: Vec<ActivityMigrationMapping>,
    ) -> Result<(), crate::error::FlowableError> {
        let cmd = MigrateProcessInstanceCmd::new(
            process_instance_id,
            target_process_definition_id,
            activity_migration_mappings,
        );
        self.command_executor.execute(&cmd)
    }

    /// P56: validate a `MigrationPlan` without applying it. Returns a
    /// report aggregating every issue (Java
    /// `ProcessInstanceMigrationManagerImpl.validateMigration` parity).
    /// The report's `has_errors()` is the canonical "safe to migrate"
    /// gate; warnings do not block migration.
    pub fn validate_migration_plan(
        &self,
        plan: &MigrationPlan,
    ) -> Result<MigrationValidationReport, crate::error::FlowableError> {
        let cmd = ValidateMigrationPlanCmd::new(plan.clone());
        self.command_executor.execute(&cmd)
    }

    /// P56: batch migration. Every plan runs in sequence; one plan
    /// failing does not abort the rest. Use
    /// [`Self::migrate_process_instances_with_callback`] if you need
    /// per-PI observability.
    pub fn migrate_process_instances(
        &self,
        plans: Vec<MigrationPlan>,
    ) -> Result<MigrationBatchResult, crate::error::FlowableError> {
        let cmd = BatchMigrateProcessInstancesCmd::new(plans);
        self.command_executor.execute(&cmd)
    }

    /// P56: batch migration with a per-PI callback. The callback fires
    /// `pre_migration` before each plan and `post_migration` after each
    /// plan (regardless of success/failure). Callback errors do NOT
    /// abort the batch.
    pub fn migrate_process_instances_with_callback(
        &self,
        plans: Vec<MigrationPlan>,
        callback: Arc<dyn MigrationCallback>,
    ) -> Result<MigrationBatchResult, crate::error::FlowableError> {
        let cmd = BatchMigrateProcessInstancesCmd::new(plans).with_callback(callback);
        self.command_executor.execute(&cmd)
    }

    /// Switches a process instance to another version of its process
    /// definition. Mirrors Java `SetProcessDefinitionVersionCmd` (programmatic
    /// command; no REST endpoint in Flowable Java either).
    pub fn set_process_definition_version(
        &self,
        process_instance_id: &str,
        process_definition_version: i32,
    ) -> Result<(), crate::error::FlowableError> {
        let cmd = SetProcessDefinitionVersionCmd::new(
            process_instance_id.to_string(),
            process_definition_version,
        )?;
        self.command_executor.execute(&cmd)
    }

    /// Starts a process instance directly by process definition ID
    pub fn start_process_instance_by_id(
        &self,
        process_definition_id: String,
        business_key: Option<&str>,
    ) -> Result<ProcessInstance, crate::error::FlowableError> {
        let mut builder =
            ProcessInstanceBuilder::new().process_definition_id(process_definition_id);
        if let Some(bk) = business_key {
            builder = builder.business_key(bk.to_string());
        }
        self.start_process_instance(builder)
    }

    pub fn start_process_instance_by_key(
        &self,
        process_definition_key: &str,
    ) -> Result<ProcessInstance, crate::error::FlowableError> {
        let builder = ProcessInstanceBuilder::new()
            .process_definition_key(process_definition_key.to_string());
        self.start_process_instance(builder)
    }

    // ── Message/Signal Start Event API ──

    /// Starts a new process instance by triggering a message start event subscription.
    pub fn start_process_instance_by_message(
        &self,
        message_ref: String,
    ) -> Result<ProcessInstance, crate::error::FlowableError> {
        let cmd = TriggerProcessStartByEventCmd::new(EventSubscriptionKind::Message, message_ref);
        self.command_executor.execute(&cmd)
    }

    /// Starts a new process instance by triggering a signal start event subscription.
    pub fn start_process_instance_by_signal(
        &self,
        signal_ref: String,
    ) -> Result<ProcessInstance, crate::error::FlowableError> {
        let cmd = TriggerProcessStartByEventCmd::new(EventSubscriptionKind::Signal, signal_ref);
        self.command_executor.execute(&cmd)
    }

    // ── Event Subprocess Trigger API (message/signal) ──

    /// Triggers a message event subprocess within a running process instance.
    pub fn trigger_event_subprocess_by_message(
        &self,
        message_ref: String,
        process_instance_id: String,
    ) -> Vec<String> {
        let cmd = TriggerEventSubprocessByEventCmd::new(
            EventSubscriptionKind::Message,
            message_ref,
            process_instance_id,
        );
        self.command_executor.execute(&cmd).unwrap()
    }

    /// Triggers a signal event subprocess within a running process instance.
    pub fn trigger_event_subprocess_by_signal(
        &self,
        signal_ref: String,
        process_instance_id: String,
    ) -> Vec<String> {
        let cmd = TriggerEventSubprocessByEventCmd::new(
            EventSubscriptionKind::Signal,
            signal_ref,
            process_instance_id,
        );
        self.command_executor.execute(&cmd).unwrap()
    }

    // ── Unified event subscription trigger API ──

    /// Triggers a "none" intermediate catch event (no event definition) by process instance ID.
    pub fn trigger_intermediate_catch_event_by_process_instance_id(
        &self,
        process_instance_id: String,
    ) {
        let cmd = TriggerIntermediateCatchEventCmd::new(process_instance_id);
        self.command_executor.execute(&cmd).unwrap();
    }

    /// Unified: triggers an intermediate catch event by subscription kind + event_ref + execution_id.
    pub fn trigger_event_intermediate_catch(
        &self,
        subscription_kind: EventSubscriptionKind,
        event_ref: String,
        execution_id: String,
    ) {
        let cmd = TriggerEventIntermediateCatchCmd::new(subscription_kind, event_ref, execution_id);
        self.command_executor.execute(&cmd).unwrap();
    }

    /// Triggers a waiting send-event service task (triggerable send-and-receive).
    ///
    /// Java: `BpmnEventRegistryEventConsumer` → `runtimeService.trigger` →
    /// `TriggerCmd` / `TriggerExecutionOperation` →
    /// `SendEventTaskActivityBehavior#trigger` (`SendEventTaskActivityBehavior.java:230-265`).
    pub fn trigger_send_event_service_task(
        &self,
        execution_id: String,
        event_key: String,
        payload: Value,
    ) -> Result<(), crate::error::FlowableError> {
        let cmd = TriggerSendEventServiceTaskCmd::new(execution_id, event_key, payload);
        self.command_executor.execute(&cmd)
    }

    /// Same as [`Self::trigger_send_event_service_task`], associating the
    /// inbound pipeline delivery so the trigger path updates that row instead
    /// of inserting a second delivery (P134 dual-record merge).
    pub fn trigger_send_event_service_task_with_delivery(
        &self,
        execution_id: String,
        event_key: String,
        payload: Value,
        inbound_delivery_id: String,
    ) -> Result<(), crate::error::FlowableError> {
        let cmd = TriggerSendEventServiceTaskCmd::new(execution_id, event_key, payload)
            .with_inbound_delivery_id(inbound_delivery_id);
        self.command_executor.execute(&cmd)
    }

    pub fn trigger_event_intermediate_catch_with_variables(
        &self,
        subscription_kind: EventSubscriptionKind,
        event_ref: String,
        execution_id: String,
        variables: HashMap<String, Value>,
    ) -> Result<(), crate::error::FlowableError> {
        let cmd = TriggerEventIntermediateCatchCmd::with_variables(
            subscription_kind,
            event_ref,
            execution_id,
            variables,
        );
        self.command_executor.execute(&cmd)
    }

    /// Stable entry point for message intermediate catch.
    pub fn trigger_intermediate_catch_event_by_message_ref_and_execution_id(
        &self,
        message_ref: String,
        execution_id: String,
    ) {
        self.trigger_event_intermediate_catch(
            EventSubscriptionKind::Message,
            message_ref,
            execution_id,
        );
    }

    /// Stable entry point for signal intermediate catch.
    pub fn trigger_intermediate_catch_event_by_signal_ref_and_execution_id(
        &self,
        signal_ref: String,
        execution_id: String,
    ) {
        self.trigger_event_intermediate_catch(
            EventSubscriptionKind::Signal,
            signal_ref,
            execution_id,
        );
    }

    /// Global signal broadcast entry point. Java parity: `SignalEventReceivedCmd`
    /// with executionId == null does NOT check suspension.
    pub fn trigger_global_signal_intermediate_catch(
        &self,
        signal_ref: String,
        execution_id: String,
    ) {
        let cmd = TriggerEventIntermediateCatchCmd::new(
            EventSubscriptionKind::Signal,
            signal_ref,
            execution_id,
        )
        .without_suspension_check();
        self.command_executor.execute(&cmd).unwrap();
    }

    /// Triggers a timer intermediate catch event.
    pub fn trigger_timer_intermediate_catch_event(
        &self,
        execution_id: String,
    ) -> Result<(), crate::error::FlowableError> {
        let cmd = TriggerTimerIntermediateCatchEventCmd::new(execution_id);
        self.command_executor.execute(&cmd)
    }

    // ── Wait state query ──

    pub fn get_event_wait_states_by_process_instance_id(
        &self,
        process_instance_id: String,
    ) -> Vec<EventWaitState> {
        let cmd = QueryEventWaitStatesByProcessInstanceIdCmd::new(process_instance_id);
        self.command_executor.execute(&cmd).unwrap()
    }

    /// Type alias for callers that depend on the older name
    pub fn get_message_style_wait_states_by_process_instance_id(
        &self,
        process_instance_id: String,
    ) -> Vec<EventWaitState> {
        self.get_event_wait_states_by_process_instance_id(process_instance_id)
    }

    // ── Boundary event trigger API ──

    /// Triggers a boundary event by its exact boundary event ID within a process instance.
    pub fn trigger_boundary_event(
        &self,
        boundary_event_id: String,
        process_instance_id: String,
    ) -> Result<(), crate::error::FlowableError> {
        let cmd = TriggerBoundaryEventCmd::new(boundary_event_id, process_instance_id);
        self.command_executor.execute(&cmd)
    }

    /// Unified: triggers a boundary event by subscription kind + event_ref.
    pub fn trigger_boundary_event_by_event_ref(
        &self,
        subscription_kind: EventSubscriptionKind,
        event_ref: String,
        process_instance_id: String,
    ) {
        let cmd = TriggerBoundaryEventByEventRefCmd::new(
            subscription_kind,
            event_ref,
            process_instance_id,
        );
        self.command_executor.execute(&cmd).unwrap();
    }

    /// Stable entry point for message boundary trigger.
    pub fn trigger_boundary_event_by_message_ref(
        &self,
        message_ref: String,
        process_instance_id: String,
    ) {
        let cmd = TriggerBoundaryEventByMessageRefCmd::new(message_ref, process_instance_id);
        self.command_executor.execute(&cmd).unwrap();
    }

    /// Stable entry point for signal boundary trigger.
    pub fn trigger_boundary_event_by_signal_ref(
        &self,
        signal_ref: String,
        process_instance_id: String,
    ) {
        let cmd = TriggerBoundaryEventBySignalRefCmd::new(signal_ref, process_instance_id);
        self.command_executor.execute(&cmd).unwrap();
    }

    /// Triggers a timer boundary event by its ID.
    pub fn trigger_timer_boundary_event(
        &self,
        boundary_event_id: String,
        process_instance_id: String,
    ) {
        let cmd = TriggerTimerBoundaryEventCmd::new(boundary_event_id, process_instance_id);
        self.command_executor.execute(&cmd).unwrap();
    }

    // ── Unified Message Correlation API ──

    /// Correlates a message by name, searching across all running process instances.
    /// Returns `CorrelateMessageResult` indicating whether a match was found.
    pub fn correlate_message(
        &self,
        message_name: String,
    ) -> Result<CorrelateMessageResult, crate::error::FlowableError> {
        let cmd = CorrelateMessageCmd::new(message_name, CorrelateMessageOptions::default());
        self.command_executor.execute(&cmd)
    }

    /// Correlates a message with filters (process_instance_id, business_key, tenant_id).
    pub fn correlate_message_with_options(
        &self,
        message_name: String,
        options: CorrelateMessageOptions,
    ) -> Result<CorrelateMessageResult, crate::error::FlowableError> {
        let cmd = CorrelateMessageCmd::new(message_name, options);
        self.command_executor.execute(&cmd)
    }

    /// Correlates a message by name, targeting a specific process instance.
    pub fn correlate_message_to_process_instance(
        &self,
        message_name: String,
        process_instance_id: String,
    ) -> Result<CorrelateMessageResult, crate::error::FlowableError> {
        let options = CorrelateMessageOptions {
            process_instance_id: Some(process_instance_id),
            ..Default::default()
        };
        let cmd = CorrelateMessageCmd::new(message_name, options);
        self.command_executor.execute(&cmd)
    }

    /// Correlates a message by name, targeting by business key.
    pub fn correlate_message_by_business_key(
        &self,
        message_name: String,
        business_key: String,
    ) -> Result<CorrelateMessageResult, crate::error::FlowableError> {
        let options = CorrelateMessageOptions {
            business_key: Some(business_key),
            ..Default::default()
        };
        let cmd = CorrelateMessageCmd::new(message_name, options);
        self.command_executor.execute(&cmd)
    }

    /// Correlates a message and starts a new process instance if no match is found.
    pub fn correlate_message_or_start(
        &self,
        message_name: String,
    ) -> Result<CorrelateMessageResult, crate::error::FlowableError> {
        let options = CorrelateMessageOptions {
            start_new_if_no_match: true,
            ..Default::default()
        };
        let cmd = CorrelateMessageCmd::new(message_name, options);
        self.command_executor.execute(&cmd)
    }

    /// Correlates a message with payload variables written to the matched execution.
    pub fn correlate_message_with_variables(
        &self,
        message_name: String,
        variables: HashMap<String, Value>,
    ) -> Result<CorrelateMessageResult, crate::error::FlowableError> {
        let options = CorrelateMessageOptions {
            variables,
            ..Default::default()
        };
        let cmd = CorrelateMessageCmd::new(message_name, options);
        self.command_executor.execute(&cmd)
    }

    pub fn execute_timer_job_by_id(&self, job_id: &str) -> Result<(), crate::error::FlowableError> {
        let store = self.command_executor.runtime_store();
        let mut session = store
            .create_session()
            .map_err(|e| crate::error::FlowableError::Internal(e.to_string()))?;
        let timer_job = store
            .find_timer_job_state(job_id, &mut session)
            .ok_or_else(|| {
                crate::error::FlowableError::NotFound(format!("Job '{}' not found", job_id))
            })?;
        session
            .rollback()
            .map_err(|e| crate::error::FlowableError::Internal(e.to_string()))?;

        if timer_job.retries.unwrap_or(1) <= 0
            || matches!(
                timer_job.job_state.as_deref(),
                Some("deadletter" | "history" | "suspended")
            )
        {
            return Err(crate::error::FlowableError::NotFound(format!(
                "Executable job '{}' not found",
                job_id
            )));
        }

        if let Some(suspended) = scheduled_process_definition_suspended(&timer_job) {
            let cmd = ExecuteScheduledProcessDefinitionActionCmd::new(timer_job, suspended);
            return self.command_executor.execute(&cmd);
        }

        // P119: TIMER_FIRED + trigger in one command so post-agenda listeners
        // receive the event (Java TriggerTimerEventJobHandler.java:44-46).
        self.command_executor.execute(
            &crate::cmd::run_due_timers_cmd::ExecuteTimerJobWithFiredEventCmd::new(timer_job),
        )
    }

    // ── Timer scheduler API ──

    pub fn heartbeat_timer_node(
        &self,
        worker_type: &str,
    ) -> Result<(), crate::error::FlowableError> {
        let cmd = crate::cmd::run_due_timers_cmd::HeartbeatTimerNodeCmd::new(
            Arc::clone(&self.timer_owner_id),
            worker_type.to_string(),
        );
        self.command_executor.execute(&cmd).unwrap();
        Ok(())
    }

    pub fn acquire_coordinator_lease(
        &self,
        timeout_ms: u64,
    ) -> Result<Option<i64>, crate::error::FlowableError> {
        let cmd = crate::cmd::run_due_timers_cmd::AcquireCoordinatorLeaseCmd::new(
            Arc::clone(&self.timer_owner_id),
            timeout_ms,
        );
        self.command_executor.execute(&cmd)
    }

    pub fn release_coordinator_lease(
        &self,
        fencing_token: i64,
    ) -> Result<bool, crate::error::FlowableError> {
        let cmd = crate::cmd::run_due_timers_cmd::ReleaseCoordinatorLeaseCmd::new(
            Arc::clone(&self.timer_owner_id),
            fencing_token,
        );
        self.command_executor.execute(&cmd)
    }

    pub fn try_acquire_global_lock(
        &self,
        lock_name: &str,
        owner: &str,
        lease_ms: i64,
    ) -> Result<bool, crate::error::FlowableError> {
        let cmd = crate::cmd::run_due_timers_cmd::AcquireGlobalLockCmd::new(
            lock_name.to_string(),
            owner.to_string(),
            lease_ms,
        );
        self.command_executor.execute(&cmd)
    }

    pub(crate) fn try_acquire_executor_global_lock(
        &self,
        lock_name: &str,
        owner: &str,
        lease_ms: i64,
    ) -> Result<bool, crate::error::FlowableError> {
        let cmd = crate::cmd::run_due_timers_cmd::AcquireGlobalLockCmd::new_at(
            lock_name.to_string(),
            owner.to_string(),
            lease_ms,
            chrono::Utc::now().timestamp_millis(),
        );
        self.command_executor.execute(&cmd)
    }

    pub fn release_global_lock(
        &self,
        lock_name: &str,
        owner: &str,
    ) -> Result<bool, crate::error::FlowableError> {
        let cmd = crate::cmd::run_due_timers_cmd::ReleaseGlobalLockCmd::new(
            lock_name.to_string(),
            owner.to_string(),
        );
        self.command_executor.execute(&cmd)
    }

    pub fn acquire_async_jobs(
        &self,
        lock_duration_ms: i64,
        max_jobs: usize,
    ) -> Vec<RuntimeTimerJobState> {
        self.acquire_async_jobs_for_tenants(lock_duration_ms, max_jobs, &[], &[])
    }

    /// Acquire async jobs, optionally restricted to process instances whose
    /// tenant_id is in `tenant_ids`. Empty `tenant_ids` means all tenants.
    pub fn acquire_async_jobs_for_tenants(
        &self,
        lock_duration_ms: i64,
        max_jobs: usize,
        tenant_ids: &[String],
        enabled_job_categories: &[String],
    ) -> Vec<RuntimeTimerJobState> {
        self.try_acquire_async_jobs_for_tenants(
            lock_duration_ms,
            max_jobs,
            tenant_ids,
            enabled_job_categories,
        )
        .unwrap_or_default()
    }

    pub(crate) fn try_acquire_async_jobs_for_tenants(
        &self,
        lock_duration_ms: i64,
        max_jobs: usize,
        tenant_ids: &[String],
        enabled_job_categories: &[String],
    ) -> Result<Vec<RuntimeTimerJobState>, crate::error::FlowableError> {
        let cmd = crate::cmd::run_due_timers_cmd::AcquireAsyncJobsCmd::new(
            Arc::clone(&self.timer_owner_id),
            lock_duration_ms,
            max_jobs,
            Arc::clone(&self.timer_metrics),
        )
        .with_tenant_ids(tenant_ids.to_vec())
        .with_enabled_job_categories(enabled_job_categories.to_vec());
        self.command_executor.execute(&cmd)
    }

    pub(crate) fn acquire_async_jobs_global_for_tenants(
        &self,
        permit: &crate::engine::lock_manager::GlobalAcquirePermit<'_>,
        lock_duration_ms: i64,
        max_jobs: usize,
        tenant_ids: &[String],
        enabled_job_categories: &[String],
    ) -> Result<Vec<RuntimeTimerJobState>, crate::error::FlowableError> {
        permit.ensure_lock(crate::engine::lock_manager::ACQUIRE_ASYNC_JOBS_GLOBAL_LOCK)?;
        let cmd = crate::cmd::run_due_timers_cmd::AcquireAsyncJobsCmd::new(
            Arc::clone(&self.timer_owner_id),
            lock_duration_ms,
            max_jobs,
            Arc::clone(&self.timer_metrics),
        )
        .with_tenant_ids(tenant_ids.to_vec())
        .with_enabled_job_categories(enabled_job_categories.to_vec())
        .serialized_by_global_lock();
        self.command_executor.execute(&cmd)
    }

    pub fn acquire_history_jobs(
        &self,
        lock_duration_ms: i64,
        max_jobs: usize,
    ) -> Vec<RuntimeTimerJobState> {
        let cmd = crate::cmd::run_due_timers_cmd::AcquireHistoryJobsCmd::new(
            Arc::clone(&self.timer_owner_id),
            lock_duration_ms,
            max_jobs,
            Arc::clone(&self.timer_metrics),
        );
        self.command_executor.execute(&cmd).unwrap_or_default()
    }

    pub fn release_timer_job_lock(&self, timer_job_id: &str) -> bool {
        let cmd = crate::cmd::run_due_timers_cmd::ReleaseTimerJobLockCmd::new(
            timer_job_id.to_string(),
            Arc::clone(&self.timer_owner_id),
        );
        self.command_executor.execute(&cmd).unwrap_or(false)
    }

    /// Releases this executor owner's executable async jobs, optionally scoped
    /// to the configured Rust multi-tenant acquisition set. Empty means all
    /// tenants, while an empty-string entry matches tenant-less instances.
    pub fn unlock_owned_jobs(
        &self,
        tenant_ids: &[String],
    ) -> Result<usize, crate::error::FlowableError> {
        let cmd = UnlockOwnedJobsCmd::new(Arc::clone(&self.timer_owner_id))
            .with_tenant_ids(tenant_ids.to_vec());
        self.command_executor.execute(&cmd)
    }

    pub(crate) fn reject_acquired_async_job(
        &self,
        job: &RuntimeTimerJobState,
    ) -> Result<(), crate::error::FlowableError> {
        match self.try_reject_acquired_async_job(job) {
            AsyncJobRejectOutcome::Released => Ok(()),
            AsyncJobRejectOutcome::ListenerFatal(error) => Err(error),
            AsyncJobRejectOutcome::ReleaseFailure(error) => Err(error),
        }
    }

    /// Typed rejection outcome that distinguishes fatal listener errors (which
    /// should short-circuit the batch, matching Java `offerJobs` semantics)
    /// from release infrastructure failures (which should be aggregated so
    /// remaining rejected jobs can still be released).
    ///
    /// Ordering matches Java `DefaultAsyncJobExecutor.executeAsyncJob`:
    /// 1. dispatch `JOB_REJECTED` event — a fatal listener error returns
    ///    `ListenerFatal` and the lock is NOT released;
    /// 2. execute `ReleaseTimerJobLockCmd` — a DB error returns
    ///    `ReleaseFailure` and the lock is NOT released;
    /// 3. CAS mismatch (lock owner changed) returns `ReleaseFailure`.
    pub(crate) fn try_reject_acquired_async_job(
        &self,
        job: &RuntimeTimerJobState,
    ) -> AsyncJobRejectOutcome {
        let rejected_event = EngineEvent::Job {
            event_type: EngineEventType::JobRejected,
            job: job.clone(),
        };
        if let Err(error) = self
            .command_executor
            .config()
            .engine_event_dispatcher
            .dispatch(&rejected_event)
        {
            return AsyncJobRejectOutcome::ListenerFatal(error);
        }

        let release =
            ReleaseTimerJobLockCmd::new(job.timer_job_id.clone(), Arc::clone(&self.timer_owner_id));
        match self.command_executor.execute(&release) {
            Ok(true) => AsyncJobRejectOutcome::Released,
            Ok(false) => AsyncJobRejectOutcome::ReleaseFailure(
                crate::error::FlowableError::Internal(format!(
                    "failed to unacquire rejected async job {}",
                    job.timer_job_id
                )),
            ),
            Err(error) => AsyncJobRejectOutcome::ReleaseFailure(error),
        }
    }

    pub(crate) fn reject_acquired_timer_work(
        &self,
        work: &TimerWork,
    ) -> Result<(), crate::error::FlowableError> {
        if let TimerWork::RuntimeJob(job) = work {
            let rejected_event = EngineEvent::Job {
                event_type: EngineEventType::JobRejected,
                job: job.clone(),
            };
            self.command_executor
                .config()
                .engine_event_dispatcher
                .dispatch(&rejected_event)?;
        }

        let release =
            ReleaseAcquiredTimerWorkLockCmd::new(work.clone(), Arc::clone(&self.timer_owner_id));
        if self.command_executor.execute(&release)? {
            Ok(())
        } else {
            let work_id = match work {
                TimerWork::RuntimeJob(job) => &job.timer_job_id,
                TimerWork::ProcessStart(subscription) => &subscription.id,
                TimerWork::EventSubprocess(subscription) => &subscription.subscription_id,
            };
            Err(crate::error::FlowableError::Internal(format!(
                "failed to unacquire rejected timer work {work_id}"
            )))
        }
    }

    pub fn reset_expired_timer_job_locks(&self, page_size: usize) -> usize {
        let cmd = crate::cmd::run_due_timers_cmd::ResetExpiredTimerJobLocksCmd::new(page_size);
        self.command_executor.execute(&cmd).unwrap_or(0)
    }

    pub fn reset_expired_jobs_batch(
        &self,
        job_class: crate::persistence::runtime_store::ExpiredJobClass,
        page_size: usize,
    ) -> Result<
        crate::persistence::runtime_store::ResetExpiredJobsBatchOutcome,
        crate::error::FlowableError,
    > {
        self.reset_expired_jobs_batch_scoped(job_class, page_size, &[], &[])
    }

    pub fn reset_expired_jobs_batch_scoped(
        &self,
        job_class: crate::persistence::runtime_store::ExpiredJobClass,
        page_size: usize,
        tenant_ids: &[String],
        enabled_job_categories: &[String],
    ) -> Result<
        crate::persistence::runtime_store::ResetExpiredJobsBatchOutcome,
        crate::error::FlowableError,
    > {
        let cmd =
            crate::cmd::run_due_timers_cmd::ResetExpiredJobsBatchCmd::new(job_class, page_size)
                .with_tenant_ids(tenant_ids.to_vec())
                .with_enabled_job_categories(enabled_job_categories.to_vec());
        self.command_executor.execute(&cmd)
    }

    pub fn acquire_timer_work(
        &self,
        fencing_token: i64,
    ) -> Vec<crate::engine::timer_worker::TimerWork> {
        self.acquire_timer_work_for_tenants(fencing_token, &[], &[])
    }

    /// Acquire timer work, optionally restricted by process-instance tenant.
    /// Empty `tenant_ids` means all tenants.
    pub fn acquire_timer_work_for_tenants(
        &self,
        fencing_token: i64,
        tenant_ids: &[String],
        enabled_job_categories: &[String],
    ) -> Vec<crate::engine::timer_worker::TimerWork> {
        let cmd = crate::cmd::run_due_timers_cmd::AcquireTimerWorkCmd::new(
            Arc::clone(&self.timer_owner_id),
            fencing_token,
            Arc::clone(&self.timer_metrics),
        )
        .with_tenant_ids(tenant_ids.to_vec())
        .with_enabled_job_categories(enabled_job_categories.to_vec());
        self.command_executor.execute(&cmd).unwrap()
    }

    pub(crate) fn acquire_scheduled_timer_work_for_tenants(
        &self,
        fencing_token: i64,
        tenant_ids: &[String],
        enabled_job_categories: &[String],
        max_jobs: usize,
    ) -> Vec<crate::engine::timer_worker::TimerWork> {
        let cmd = crate::cmd::run_due_timers_cmd::AcquireTimerWorkCmd::new(
            Arc::clone(&self.timer_owner_id),
            fencing_token,
            Arc::clone(&self.timer_metrics),
        )
        .with_tenant_ids(tenant_ids.to_vec())
        .with_enabled_job_categories(enabled_job_categories.to_vec())
        .scheduled_timers_only()
        .with_max_jobs(max_jobs);
        self.command_executor.execute(&cmd).unwrap()
    }

    pub(crate) fn acquire_scheduled_timer_work_global_for_tenants(
        &self,
        permit: &crate::engine::lock_manager::GlobalAcquirePermit<'_>,
        fencing_token: i64,
        tenant_ids: &[String],
        enabled_job_categories: &[String],
        max_jobs: usize,
    ) -> Result<Vec<crate::engine::timer_worker::TimerWork>, crate::error::FlowableError> {
        permit.ensure_lock(crate::engine::lock_manager::ACQUIRE_TIMER_JOBS_GLOBAL_LOCK)?;
        let cmd = crate::cmd::run_due_timers_cmd::AcquireTimerWorkCmd::new(
            Arc::clone(&self.timer_owner_id),
            fencing_token,
            Arc::clone(&self.timer_metrics),
        )
        .with_tenant_ids(tenant_ids.to_vec())
        .with_enabled_job_categories(enabled_job_categories.to_vec())
        .scheduled_timers_only()
        .with_max_jobs(max_jobs)
        .serialized_by_global_lock();
        self.command_executor.execute(&cmd)
    }

    pub fn execute_timer_work(
        &self,
        work: &crate::engine::timer_worker::TimerWork,
        fencing_token: i64,
    ) -> Option<String> {
        // Java ExecuteAsyncRunnable.java:113-129: an exclusive job first takes
        // the process-instance scope lock in its own transaction; on conflict
        // (:239-258 lockJobFailed) the job row lock is released so another
        // executor (or the retry cycle) can pick it up, and execution is skipped.
        if !self.lock_exclusive_job_scope(work) {
            return None;
        }
        let cmd = crate::cmd::run_due_timers_cmd::ExecuteTimerWorkCmd::new(
            work.clone(),
            Arc::clone(&self.timer_owner_id),
            fencing_token,
            Arc::clone(&self.timer_metrics),
        );
        match self.command_executor.execute(&cmd) {
            Ok(executed_job_id) => executed_job_id,
            Err(error) => {
                self.record_failed_timer_work(work, &error);
                None
            }
        }
    }

    /// Java `ExecuteAsyncRunnable.run` (:113-129) + `lockJobIfNeeded`
    /// (`LockExclusiveJobCmd`): take the exclusive PI scope lock before an
    /// exclusive job executes. Returns `false` (job must not run) when another
    /// owner holds a live lock; the acquired job *row* lock is released in that
    /// case (Java :239-258 `unacquireJob`).
    fn lock_exclusive_job_scope(&self, work: &crate::engine::timer_worker::TimerWork) -> bool {
        let crate::engine::timer_worker::TimerWork::RuntimeJob(job) = work else {
            return true;
        };
        // History jobs are Java `HistoryJobEntity` (JobInfoEntity, *not*
        // JobEntity) — ExecuteAsyncRunnable.lockJobIfNeeded only locks
        // `job instanceof JobEntity && isExclusive()`, so no scope lock here.
        if job.job_state.as_deref() == Some("history") {
            return true;
        }
        if !job.exclusive || job.process_instance_id.is_empty() {
            return true;
        }
        let lock_cmd = crate::cmd::run_due_timers_cmd::LockExclusiveJobScopeCmd::new(
            job.clone(),
            Arc::clone(&self.timer_owner_id),
        );
        if matches!(self.command_executor.execute(&lock_cmd), Ok(true)) {
            return true;
        }
        // Lock failed: release the executor row lock so the job stays acquirable
        // (Java ExecuteAsyncRunnable.java:249-257 unacquireJob) and count the
        // conflict; the job is intentionally not executed now.
        let release_cmd = crate::cmd::run_due_timers_cmd::ReleaseTimerJobLockCmd::new(
            job.timer_job_id.clone(),
            Arc::clone(&self.timer_owner_id),
        );
        if let Err(error) = self.command_executor.execute(&release_cmd) {
            tracing::error!(
                job_id = %job.timer_job_id,
                "failed to release async job after exclusive-scope conflict: {error}"
            );
        }
        self.timer_metrics
            .acquire_conflicts
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        false
    }

    /// Execute a job delivered by a post-commit activation hint. The job was
    /// pre-locked by the *live* async executor (owner + expiration set inside
    /// the activating transaction), so it holds a valid executor row lock and
    /// bypasses the timer coordinator lease. `ExecuteTimerWorkCmd` re-reads the
    /// row and verifies the executor still owns a non-expired lock before
    /// running it — no fake `fencing_token == 0` sentinel is used.
    pub fn execute_timer_work_direct_hint(
        &self,
        work: &crate::engine::timer_worker::TimerWork,
    ) -> Option<String> {
        // Validate the executor-row lease before taking the separate exclusive
        // process-instance lock. An inactive executor may have observed an
        // unlocked persistent job, and a delayed hint may refer to a lease that
        // was reset and re-acquired under the same stable owner. In either case
        // taking the PI lock first can strand an orphan scope lock.
        let crate::engine::timer_worker::TimerWork::RuntimeJob(hinted_job) = work else {
            return None;
        };
        let store = self.command_executor.runtime_store();
        let mut session = store.create_session().ok()?;
        let current = store.find_timer_job_state(&hinted_job.timer_job_id, &mut session);
        let _ = session.rollback();
        let current = current?;
        let now = store.time_source().now().timestamp_millis();
        let same_live_lease = current.lock_owner.as_deref() == Some(self.timer_owner_id.as_ref())
            && current.lock_owner == hinted_job.lock_owner
            && current.lock_time == hinted_job.lock_time
            && current.lock_expiration_time == hinted_job.lock_expiration_time
            && current
                .lock_expiration_time
                .map(|expiration| expiration > now)
                .unwrap_or(false);
        if !same_live_lease {
            self.timer_metrics
                .acquire_conflicts
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            return None;
        }
        // Same exclusive PI scope gate as the acquire path
        // (Java ExecuteAsyncRunnable.java:113-129 applies to hinted jobs too).
        if !self.lock_exclusive_job_scope(work) {
            return None;
        }
        let cmd = crate::cmd::run_due_timers_cmd::ExecuteTimerWorkCmd::new_direct_hint(
            work.clone(),
            Arc::clone(&self.timer_owner_id),
            Arc::clone(&self.timer_metrics),
        );
        match self.command_executor.execute(&cmd) {
            Ok(executed_job_id) => executed_job_id,
            Err(error) => {
                self.record_failed_timer_work(work, &error);
                None
            }
        }
    }

    /// Offer a committed, pre-locked async job to the live executor.
    ///
    /// This is the post-commit half of the Java-compatible activation hint
    /// (`JobAddedTransactionListener` on `COMMITTED`). It runs *after* the
    /// activating command's transaction has committed:
    ///
    ///   1. re-read the current row. If the job was deleted (e.g. a concurrent
    ///      completion) nothing is submitted — Java's listener holds a stale
    ///      entity but the executor's own re-read drops it; we drop it here.
    ///   2. offer the job to the executor task pool via `execute_async_job`.
    ///   3. on rejection (queue full / shut down / no executor) dispatch
    ///      `JOB_REJECTED` and CAS-release the pre-lock so a later acquisition
    ///      can pick the job up again. A fatal `JOB_REJECTED` listener error is
    ///      returned to the caller and the lock is left in place (Java parity:
    ///      the DB transaction already committed, so this is a post-commit
    ///      lifecycle error, not a rollback).
    ///
    /// `submit` is the executor offer closure; it returns `true` when the pool
    /// accepted the job. Kept as a closure so the executor `Arc` is not a field
    /// of `RuntimeService`.
    pub(crate) fn submit_committed_async_hint(
        &self,
        job: &RuntimeTimerJobState,
        submit: &dyn Fn(RuntimeTimerJobState) -> bool,
    ) -> Result<(), crate::error::FlowableError> {
        // 1. Re-read: skip stale/deleted jobs.
        let current = {
            let store = self.command_executor.runtime_store();
            let mut session = match store.create_session() {
                Ok(session) => session,
                Err(error) => {
                    return Err(crate::error::FlowableError::Internal(error.to_string()));
                }
            };
            let found = store.find_timer_job_state(&job.timer_job_id, &mut session);
            let _ = session.rollback();
            found
        };
        let Some(current) = current else {
            return Ok(());
        };

        // 2. Offer to the executor.
        if submit(current.clone()) {
            return Ok(());
        }

        // 3. Rejected: JOB_REJECTED + CAS-release the pre-lock.
        match self.try_reject_acquired_async_job(&current) {
            AsyncJobRejectOutcome::Released => Ok(()),
            AsyncJobRejectOutcome::ListenerFatal(error) => Err(error),
            AsyncJobRejectOutcome::ReleaseFailure(error) => Err(error),
        }
    }

    fn record_failed_timer_work(
        &self,
        work: &crate::engine::timer_worker::TimerWork,
        error: &crate::error::FlowableError,
    ) {
        self.timer_metrics
            .execute_failures
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let cmd = RecordFailedTimerWorkCmd::new_with_origin(
            work.clone(),
            error,
            FailedJobExecutionOrigin::AutomaticExecutor,
        );
        if let Err(error) = self.command_executor.execute(&cmd) {
            tracing::error!("failed to persist automatic timer-work failure: {error}");
        }
    }

    pub fn renew_timer_lease(
        &self,
        work: &crate::engine::timer_worker::TimerWork,
        fencing_token: i64,
    ) {
        let cmd = crate::cmd::renew_timer_lease_cmd::RenewTimerLeaseCmd::new(
            work.clone(),
            Arc::clone(&self.timer_owner_id),
            fencing_token,
            Arc::clone(&self.timer_metrics),
        );
        self.command_executor.execute(&cmd).unwrap();
    }

    /// Acquires all due timers and executes them in batch.
    /// Returns the timer job IDs that were successfully triggered.
    pub fn run_due_timers(&self) -> Result<Vec<String>, crate::error::FlowableError> {
        let mut executed = Vec::new();
        // Match TimerWorker: publish liveness before competing for the lease so
        // concurrent one-shot callers are not treated as dead nodes (when a
        // heartbeat-based early-takeover path is active).
        let _ = self.heartbeat_timer_node("run_due_timers");
        if let Ok(Some(token)) = self.acquire_coordinator_lease(300_000) {
            let works = self.acquire_timer_work(token);
            for work in works {
                if let Some(id) = self.execute_timer_work(&work, token) {
                    executed.push(id);
                }
            }
            // Java parity: `AcquireTimerJobsRunnable` holds the global acquire
            // lock only for the acquisition cycle (`waitForLockRunAndRelease`).
            // A one-shot caller must not keep the 300s coordinator lease after
            // its batch — release so another node can take over immediately.
            // All work above executed synchronously, so the fencing token is no
            // longer needed; a failed release degrades to lease expiry.
            let _ = self.release_coordinator_lease(token);
        }
        Ok(executed)
    }

    // ── Control Surface API ──

    /// Get the current status of the timer coordinator
    pub fn get_timer_coordinator_status(
        &self,
    ) -> crate::persistence::runtime_store::TimerCoordinatorStatus {
        use crate::cmd::timer_coordination_control_cmd::{
            CoordinatorStatusResult, TimerCoordinationControlCmd,
        };
        let cmd = TimerCoordinationControlCmd::status();
        let result: CoordinatorStatusResult = self
            .command_executor
            .execute(&cmd as &dyn crate::interceptor::command::Command<CoordinatorStatusResult>)
            .unwrap();
        result.status
    }

    /// List all timer worker nodes with their status
    pub fn list_timer_nodes(
        &self,
    ) -> Result<Vec<crate::persistence::runtime_store::TimerNodeStatus>, crate::error::FlowableError>
    {
        use crate::cmd::timer_coordination_control_cmd::{
            NodesListResult, TimerCoordinationControlCmd,
        };
        let cmd = TimerCoordinationControlCmd::nodes();
        let result: NodesListResult = self
            .command_executor
            .execute(&cmd as &dyn crate::interceptor::command::Command<NodesListResult>)
            .unwrap();
        Ok(result.nodes)
    }

    /// Safely release leadership (must be called by current owner)
    pub fn release_leadership(
        &self,
        fencing_token: i64,
    ) -> Result<bool, crate::error::FlowableError> {
        use crate::cmd::timer_coordination_control_cmd::{
            ReleaseResult, TimerCoordinationControlCmd,
        };
        let cmd =
            TimerCoordinationControlCmd::release(Arc::clone(&self.timer_owner_id), fencing_token);
        let result: ReleaseResult = self
            .command_executor
            .execute(&cmd as &dyn crate::interceptor::command::Command<ReleaseResult>)
            .unwrap();
        Ok(result.success)
    }

    /// Admin step-down (force release, advance fencing token)
    pub fn admin_step_down(&self) -> Result<(bool, i64), crate::error::FlowableError> {
        use crate::cmd::timer_coordination_control_cmd::{
            StepDownResult, TimerCoordinationControlCmd,
        };
        let cmd = TimerCoordinationControlCmd::step_down();
        let result: StepDownResult = self
            .command_executor
            .execute(&cmd as &dyn crate::interceptor::command::Command<StepDownResult>)
            .unwrap();
        Ok((result.success, result.new_fencing_token))
    }

    /// Deregister a specific timer node (admin operation)
    pub fn deregister_timer_node(
        &self,
        node_id: &str,
    ) -> Result<bool, crate::error::FlowableError> {
        use crate::cmd::timer_coordination_control_cmd::{
            DeregisterResult, TimerCoordinationControlCmd,
        };
        let cmd = TimerCoordinationControlCmd::deregister(Arc::from(node_id));
        let result: DeregisterResult = self
            .command_executor
            .execute(&cmd as &dyn crate::interceptor::command::Command<DeregisterResult>)
            .unwrap();
        Ok(result.success)
    }

    /// Clean up expired timer nodes (admin operation)
    pub fn cleanup_expired_timer_nodes(&self) -> Result<usize, crate::error::FlowableError> {
        use crate::cmd::timer_coordination_control_cmd::{
            CleanupResult, TimerCoordinationControlCmd,
        };
        let cmd = TimerCoordinationControlCmd::cleanup();
        let result: CleanupResult = self
            .command_executor
            .execute(&cmd as &dyn crate::interceptor::command::Command<CleanupResult>)
            .unwrap();
        Ok(result.cleaned_count)
    }

    /// Record an admin audit action
    pub fn audit_admin_action(
        &self,
        input: crate::service::audit::TimerAdminAuditInput,
    ) -> Result<(), crate::error::FlowableError> {
        use crate::cmd::timer_coordination_control_cmd::{
            AuditAdminActionResult, TimerCoordinationControlCmd,
        };
        let cmd = TimerCoordinationControlCmd::audit(input);
        let _result: AuditAdminActionResult = self
            .command_executor
            .execute(&cmd as &dyn crate::interceptor::command::Command<AuditAdminActionResult>)
            .unwrap();
        Ok(())
    }

    pub fn move_deadletter_job_to_executable_job(
        &self,
        job_id: String,
        retries: i32,
    ) -> Result<(), crate::error::FlowableError> {
        let cmd = MoveDeadLetterJobToExecutableJobCmd::new(job_id, retries);
        self.command_executor.execute(&cmd)
    }

    pub fn delete_job(&self, job_id: String) -> Result<(), crate::error::FlowableError> {
        let cmd = DeleteJobCmd::new(job_id);
        self.command_executor.execute(&cmd)
    }

    pub fn add_entity_link(
        &self,
        scope_id: String,
        scope_type: String,
        reference_scope_id: String,
        reference_scope_type: String,
        link_type: String,
    ) -> Result<(), crate::error::FlowableError> {
        let cmd = AddEntityLinkCmd::new(
            scope_id,
            scope_type,
            reference_scope_id,
            reference_scope_type,
            link_type,
        );
        self.command_executor.execute(&cmd)
    }

    pub fn delete_entity_link(&self, link_id: String) -> Result<(), crate::error::FlowableError> {
        let cmd = DeleteEntityLinkCmd::new(link_id);
        self.command_executor.execute(&cmd)
    }

    pub fn get_entity_links_for_scope(
        &self,
        scope_id: String,
        scope_type: String,
    ) -> Result<Vec<crate::identity::entities::EntityLink>, crate::error::FlowableError> {
        let cmd = GetEntityLinksForScopeCmd::new(scope_id, scope_type);
        self.command_executor.execute(&cmd)
    }

    pub fn create_batch(
        &self,
        batch_type: String,
        status: String,
        total_items: i64,
    ) -> Result<crate::identity::entities::BatchEntity, crate::error::FlowableError> {
        let cmd = CreateBatchCmd::new(batch_type, status, total_items);
        self.command_executor.execute(&cmd)
    }

    pub fn get_batch(
        &self,
        batch_id: String,
    ) -> Result<Option<crate::identity::entities::BatchEntity>, crate::error::FlowableError> {
        let cmd = GetBatchCmd::new(batch_id);
        self.command_executor.execute(&cmd)
    }

    pub fn delete_batch(&self, batch_id: String) -> Result<(), crate::error::FlowableError> {
        let cmd = DeleteBatchCmd::new(batch_id);
        self.command_executor.execute(&cmd)
    }
}

pub struct MoveDeadLetterJobToExecutableJobCmd {
    job_id: String,
    retries: i32,
}

impl MoveDeadLetterJobToExecutableJobCmd {
    pub fn new(job_id: String, retries: i32) -> Self {
        Self { job_id, retries }
    }
}

impl Command<()> for MoveDeadLetterJobToExecutableJobCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<(), crate::error::FlowableError> {
        let mut job = command_context
            .runtime_store
            .find_timer_job_state(&self.job_id, &mut command_context.session)
            .ok_or_else(|| {
                crate::error::FlowableError::NotFound(format!("Job '{}' not found", self.job_id))
            })?;

        if job.job_state.as_deref() != Some("deadletter") {
            return Err(crate::error::FlowableError::Generic(format!(
                "Job '{}' is not in deadletter state",
                self.job_id
            )));
        }

        job.job_state = Some(executable_job_state_for_deadletter(&job).to_string());
        job.retries = Some(self.retries);
        job.error_message = None;
        job.error_details = None;
        job.lock_owner = None;
        job.lock_time = None;
        job.lock_expiration_time = None;

        command_context
            .runtime_store
            .insert_timer_job_state(&job, &mut command_context.session);
        Ok(())
    }
}

fn executable_job_state_for_deadletter(job: &RuntimeTimerJobState) -> &'static str {
    if job.time_duration.as_deref() == Some(ASYNC_CONTINUATION_JOB_TYPE_MARKER) {
        ASYNC_CONTINUATION_JOB_STATE
    } else {
        "timer"
    }
}

pub struct DeleteJobCmd {
    job_id: String,
}

impl DeleteJobCmd {
    pub fn new(job_id: String) -> Self {
        Self { job_id }
    }
}

impl Command<()> for DeleteJobCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<(), crate::error::FlowableError> {
        let job = command_context
            .runtime_store
            .find_timer_job_state(&self.job_id, &mut command_context.session)
            .ok_or_else(|| {
                crate::error::FlowableError::NotFound(format!("Job '{}' not found", self.job_id))
            })?;

        command_context
            .runtime_store
            .delete_timer_job_state(&job.timer_job_id, &mut command_context.session);
        Ok(())
    }
}

pub struct AddEntityLinkCmd {
    scope_id: String,
    scope_type: String,
    reference_scope_id: String,
    reference_scope_type: String,
    link_type: String,
}

impl AddEntityLinkCmd {
    pub fn new(
        scope_id: String,
        scope_type: String,
        reference_scope_id: String,
        reference_scope_type: String,
        link_type: String,
    ) -> Self {
        Self {
            scope_id,
            scope_type,
            reference_scope_id,
            reference_scope_type,
            link_type,
        }
    }
}

impl Command<()> for AddEntityLinkCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<(), crate::error::FlowableError> {
        let link = crate::identity::entities::EntityLink {
            id: uuid::Uuid::new_v4().to_string(),
            link_type: self.link_type.clone(),
            scope_id: Some(self.scope_id.clone()),
            scope_type: Some(self.scope_type.clone()),
            reference_scope_id: Some(self.reference_scope_id.clone()),
            reference_scope_type: Some(self.reference_scope_type.clone()),
            hierarchy_type: None,
        };
        command_context
            .runtime_store
            .insert_entity_link(link, &mut command_context.session);
        Ok(())
    }
}

pub struct DeleteEntityLinkCmd {
    link_id: String,
}

impl DeleteEntityLinkCmd {
    pub fn new(link_id: String) -> Self {
        Self { link_id }
    }
}

impl Command<()> for DeleteEntityLinkCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<(), crate::error::FlowableError> {
        command_context
            .runtime_store
            .delete_entity_link(&self.link_id, &mut command_context.session);
        Ok(())
    }
}

pub struct GetEntityLinksForScopeCmd {
    scope_id: String,
    #[allow(dead_code)]
    scope_type: String,
}

impl GetEntityLinksForScopeCmd {
    pub fn new(scope_id: String, scope_type: String) -> Self {
        Self {
            scope_id,
            scope_type,
        }
    }
}

impl Command<Vec<crate::identity::entities::EntityLink>> for GetEntityLinksForScopeCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<Vec<crate::identity::entities::EntityLink>, crate::error::FlowableError> {
        Ok(command_context
            .runtime_store
            .find_entity_links_by_scope(&self.scope_id, &mut command_context.session))
    }
}

pub struct CreateBatchCmd {
    batch_type: String,
    status: String,
    total_items: i64,
}

impl CreateBatchCmd {
    pub fn new(batch_type: String, status: String, total_items: i64) -> Self {
        Self {
            batch_type,
            status,
            total_items,
        }
    }
}

impl Command<crate::identity::entities::BatchEntity> for CreateBatchCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<crate::identity::entities::BatchEntity, crate::error::FlowableError> {
        let batch = crate::identity::entities::BatchEntity {
            id: uuid::Uuid::new_v4().to_string(),
            batch_type: self.batch_type.clone(),
            search_key: None,
            search_key2: None,
            status: self.status.clone(),
            total_items: self.total_items,
            items_processed: 0,
            create_time: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            end_time: None,
            tenant_id: None,
            batch_document_json: None,
        };
        command_context
            .runtime_store
            .insert_batch(batch.clone(), &mut command_context.session);
        Ok(batch)
    }
}

pub struct GetBatchCmd {
    batch_id: String,
}

impl GetBatchCmd {
    pub fn new(batch_id: String) -> Self {
        Self { batch_id }
    }
}

impl Command<Option<crate::identity::entities::BatchEntity>> for GetBatchCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<Option<crate::identity::entities::BatchEntity>, crate::error::FlowableError> {
        Ok(command_context
            .runtime_store
            .find_batch(&self.batch_id, &mut command_context.session))
    }
}

pub struct DeleteBatchCmd {
    batch_id: String,
}

impl DeleteBatchCmd {
    pub fn new(batch_id: String) -> Self {
        Self { batch_id }
    }
}

impl Command<()> for DeleteBatchCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<(), crate::error::FlowableError> {
        command_context
            .runtime_store
            .delete_batch(&self.batch_id, &mut command_context.session);
        Ok(())
    }
}
