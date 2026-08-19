use crate::CaseFileGraph;
use crate::error::CmmnError;
use crate::event_registry_correlation::{
    generate_correlation_key, json_value_to_correlation_string,
};
use crate::job::TYPE_TRIGGER_TIMER;
use crate::lifecycle_listener::{
    CmmnLifecycleListenerContext, CmmnLifecycleListenerHandler, CmmnLifecycleListenerRegistry,
    CmmnLifecycleScope, LifecycleListenerRegistryGuard, fire_matching_lifecycle_listeners,
};
use crate::management::insert_job_entity;
use crate::models::{
    CmmnCase, CmmnCaseDefinition, CmmnCaseFileItem, CmmnCaseFileItemOnPart, CmmnCaseFileItemState,
    CmmnCaseInstance, CmmnCaseInstanceStartRequest, CmmnCaseInstanceState, CmmnCasePlanModel,
    CmmnCaseTask, CmmnChangePlanItemStateRequest, CmmnDecisionTask, CmmnDelegationState,
    CmmnDiscretionaryItem, CmmnEventListener, CmmnEventOutParameter, CmmnEventSubscription,
    CmmnHistoricCaseInstance, CmmnHistoricHumanTaskInstance, CmmnHistoricMilestoneInstance,
    CmmnHumanTask, CmmnHumanTaskCompletionRequest, CmmnHumanTaskCompletionResult,
    CmmnHumanTaskInstance, CmmnHumanTaskState, CmmnHumanTaskUpdate, CmmnIOParameter,
    CmmnIdentityLink, CmmnJob, CmmnJobFamily, CmmnMigrationDocument, CmmnMigrationValidationResult,
    CmmnMilestone, CmmnPlanFragment, CmmnPlanItem, CmmnPlanItemDefinitionWithTargetIds,
    CmmnPlanItemInstance, CmmnPlanItemOnPart, CmmnPlanningTable, CmmnProcessTask,
    CmmnProcessTaskStartRequest, CmmnProcessTaskStartResult, CmmnSentry,
    CmmnSentryIfPartExpression, CmmnSentryIfPartLiteral, CmmnSentryIfPartLogicalOperator,
    CmmnSentryIfPartOperator, CmmnStage, CmmnStageInstance, CmmnStageInstanceState,
    CmmnStageOverview, CmmnTaskAssociationKind, CmmnTaskAssociationState,
    CmmnTaskInstanceAssociation, PagedResult,
};
use crate::repository::CmmnRepositoryService;
use crate::store::CmmnStore;
use crate::timer_util::{
    next_repeat_expression, prepare_repeat, resolve_next_due, resolve_timer_due,
};
use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
use flowable_engine_common::el::{Expression, MapVariableContainer, SimpleExpression};
use flowable_persistence::entity::cmmn_case_instance::{
    CmmnCaseInstanceDataManager, CmmnCaseInstanceEntity,
};
use flowable_persistence::entity::cmmn_human_task::{
    CmmnHumanTaskDataManager, CmmnHumanTaskEntity,
};
use flowable_persistence::entity::cmmn_identity_link::{
    CmmnIdentityLinkDataManager, CmmnIdentityLinkEntity,
};
use flowable_persistence::entity::cmmn_plan_item_instance::{
    CmmnPlanItemInstanceDataManager, CmmnPlanItemInstanceEntity,
};
use flowable_persistence::entity::cmmn_stage_history::{
    CmmnStageHistoryDataManager, CmmnStageHistoryEntity,
};
use flowable_persistence::entity::cmmn_stage_instance::{
    CmmnStageInstanceDataManager, CmmnStageInstanceEntity,
};
use flowable_persistence::entity::cmmn_task_instance_association::{
    CmmnTaskInstanceAssociationDataManager, CmmnTaskInstanceAssociationEntity,
};
use flowable_persistence::{DbParams, DbSession, RenderedStatement};
use serde_json::{Map, Value};
use std::cell::RefCell;
use std::collections::HashSet;
use std::sync::Arc;
use uuid::Uuid;

/// Java `PlanItemDefinitionType.EVENT_LISTENER` (`eventlistener`).
const PLAN_ITEM_DEFINITION_TYPE_EVENT_LISTENER: &str = "eventlistener";
/// Java `PlanItemDefinitionType.TIMER_EVENT_LISTENER`
/// (`TimerEventListener.class.getSimpleName().toLowerCase()`).
const PLAN_ITEM_DEFINITION_TYPE_TIMER_EVENT_LISTENER: &str = "timereventlistener";
/// Java `TimerEventListenerActivityBehaviour.java:182` uses
/// `asyncExecutorNumberOfRetries` (default 3) for the timer job retries.
const TIMER_JOB_RETRIES: i32 = 3;
/// Java `TimerEventListenerActivityBehaviour.java:180` sets `exclusive=true`; the CMMN
/// job model has no exclusive column, so the exclusive scope lock is implied for
/// timer jobs (they always run under the case instance's exclusive lock).
const TIMER_JOB_CONFIG_REPEAT_KEY: &str = "repeat";

/// Synthetic marker persisted in `ACT_CMMN_PLAN_ITEM_EVENT` (keyed by sentry
/// id) that plays the role of a Java `SentryPartInstanceEntity` with an
/// `ifPartId` set: it records that the ifPart of a multi-part sentry in
/// default trigger mode was satisfied in an earlier evaluation cycle
/// (`AbstractEvaluationCriteriaOperation.createSentryPartInstanceEntity`,
/// AbstractEvaluationCriteriaOperation.java:679-715 — inserted only when
/// `isDefaultTriggerMode`, :709-711). Not a CMMN standard event, so it can
/// never collide with real plan item lifecycle records.
const SENTRY_IF_PART_SATISFIED_EVENT: &str = "ifPartSatisfied";

#[derive(Clone)]
pub struct CmmnRuntimeService {
    store: CmmnStore,
    repository_service: CmmnRepositoryService,
    process_task_runner: Option<Arc<dyn CmmnProcessTaskRunner>>,
    /// Java `ChildBpmnCaseInstanceStateChangeCallback` equivalent — late-bound so
    /// the BPMN process engine can register after construction (P76).
    bpmn_case_task_callback: Arc<std::sync::RwLock<Option<Arc<dyn BpmnCaseTaskCallback>>>>,
    /// Name → handler registry for `class` / `delegateExpression` lifecycle listeners, and the
    /// method registry backing `expression` ones. Java resolves these through Spring / the bean
    /// registry (CmmnListenerNotificationHelper.java:162-169); Rust has no bean container, so
    /// handlers are registered on the engine.
    lifecycle_listener_registry: Arc<std::sync::RwLock<CmmnLifecycleListenerRegistry>>,
}

pub trait CmmnProcessTaskRunner: Send + Sync {
    fn start_process(
        &self,
        request: CmmnProcessTaskStartRequest,
    ) -> Result<CmmnProcessTaskStartResult, CmmnError>;
}

/// Java `ChildBpmnCaseInstanceStateChangeCallback` / `ProcessInstanceService#triggerCaseTask`
/// — notified when a child case started from BPMN `caseServiceTask` completes or terminates.
pub trait BpmnCaseTaskCallback: Send + Sync {
    fn on_child_case_completed(
        &self,
        execution_id: &str,
        case_instance_id: &str,
        variables: Map<String, Value>,
    ) -> Result<(), CmmnError>;

    fn on_child_case_terminated(
        &self,
        execution_id: &str,
        case_instance_id: &str,
    ) -> Result<(), CmmnError> {
        // Default: same as completed with empty out-map (Java still triggers leave).
        self.on_child_case_completed(execution_id, case_instance_id, Map::new())
    }
}

#[derive(Clone, Copy)]
struct ContainerView<'a> {
    plan_items: &'a [CmmnPlanItem],
    stages: &'a [CmmnStage],
    human_tasks: &'a [CmmnHumanTask],
    decision_tasks: &'a [CmmnDecisionTask],
    process_tasks: &'a [CmmnProcessTask],
    case_tasks: &'a [CmmnCaseTask],
    milestones: &'a [CmmnMilestone],
    event_listeners: &'a [CmmnEventListener],
    sentries: &'a [CmmnSentry],
}

impl<'a> ContainerView<'a> {
    fn from_case_plan_model(case_plan_model: &'a CmmnCasePlanModel) -> Self {
        Self {
            plan_items: &case_plan_model.plan_items,
            stages: &case_plan_model.stages,
            human_tasks: &case_plan_model.human_tasks,
            decision_tasks: &case_plan_model.decision_tasks,
            process_tasks: &case_plan_model.process_tasks,
            case_tasks: &case_plan_model.case_tasks,
            milestones: &case_plan_model.milestones,
            event_listeners: &case_plan_model.event_listeners,
            sentries: &case_plan_model.sentries,
        }
    }

    fn from_stage(stage: &'a CmmnStage) -> Self {
        Self {
            plan_items: &stage.plan_items,
            stages: &stage.stages,
            human_tasks: &stage.human_tasks,
            decision_tasks: &stage.decision_tasks,
            process_tasks: &stage.process_tasks,
            case_tasks: &stage.case_tasks,
            milestones: &stage.milestones,
            event_listeners: &stage.event_listeners,
            sentries: &stage.sentries,
        }
    }
}

impl CmmnRuntimeService {
    pub(crate) fn new(
        store: CmmnStore,
        repository_service: CmmnRepositoryService,
        process_task_runner: Option<Arc<dyn CmmnProcessTaskRunner>>,
    ) -> Self {
        Self {
            store,
            repository_service,
            process_task_runner,
            bpmn_case_task_callback: Arc::new(std::sync::RwLock::new(None)),
            lifecycle_listener_registry: Arc::new(std::sync::RwLock::new(
                CmmnLifecycleListenerRegistry::new(),
            )),
        }
    }

    /// Register a handler for a `class` / `delegateExpression` lifecycle listener. The name is
    /// the literal `class` attribute value, or the bean name inside a `${…}`
    /// `delegateExpression`.
    pub fn register_lifecycle_listener(
        &self,
        name: impl Into<String>,
        handler: Arc<dyn CmmnLifecycleListenerHandler>,
    ) {
        if let Ok(mut registry) = self.lifecycle_listener_registry.write() {
            registry.register(name, handler);
        }
    }

    /// Register a bean method callable from an `expression` lifecycle listener body
    /// (`${auditBean.record(...)}`), giving expression listeners a side-effect channel.
    pub fn register_lifecycle_listener_expression_method<F>(
        &self,
        bean: &str,
        method: &str,
        function: F,
    ) where
        F: Fn(&[serde_json::Value]) -> Result<serde_json::Value, String> + Send + Sync + 'static,
    {
        if let Ok(registry) = self.lifecycle_listener_registry.read() {
            registry
                .expression_methods()
                .register_bean_method(bean, method, function);
        }
    }

    /// Register the BPMN-side callback for `EXECUTION_CHILD_CASE` completion
    /// (Java `ChildBpmnCaseInstanceStateChangeCallback`).
    pub fn set_bpmn_case_task_callback(&self, callback: Arc<dyn BpmnCaseTaskCallback>) {
        if let Ok(mut guard) = self.bpmn_case_task_callback.write() {
            *guard = Some(callback);
        }
    }

    pub fn clear_bpmn_case_task_callback(&self) {
        if let Ok(mut guard) = self.bpmn_case_task_callback.write() {
            *guard = None;
        }
    }

    /// Java `ChildBpmnCaseInstanceStateChangeCallback#stateChanged` for COMPLETED/TERMINATED.
    /// Invoked after the CMMN transaction commits so the BPMN engine sees durable state.
    pub(crate) fn notify_bpmn_case_task_callback_if_needed(
        &self,
        case_instance: &CmmnCaseInstance,
    ) -> Result<(), CmmnError> {
        if case_instance.callback_type.as_deref()
            != Some(crate::CMMN_EXECUTION_CHILD_CASE_CALLBACK_TYPE)
        {
            return Ok(());
        }
        let Some(execution_id) = case_instance.callback_id.as_deref() else {
            return Ok(());
        };
        let callback = self
            .bpmn_case_task_callback
            .read()
            .ok()
            .and_then(|guard| guard.clone());
        let Some(callback) = callback else {
            return Ok(());
        };
        match case_instance.state {
            CmmnCaseInstanceState::Completed => callback.on_child_case_completed(
                execution_id,
                &case_instance.id,
                case_instance.variables.clone(),
            ),
            CmmnCaseInstanceState::Terminated => {
                callback.on_child_case_terminated(execution_id, &case_instance.id)
            }
            _ => Ok(()),
        }
    }

    pub fn case_file_item_service(&self) -> CmmnCaseFileItemService {
        CmmnCaseFileItemService::new(self.store.clone())
    }

    pub fn start_case_instance_by_key(
        &self,
        case_definition_key: &str,
        request: CmmnCaseInstanceStartRequest,
    ) -> Result<CmmnCaseInstance, CmmnError> {
        let case_definition = self
            .repository_service
            .latest_case_definition_by_key(case_definition_key, request.tenant_id.as_deref())?;
        self.start_case_instance(case_definition, request)
    }

    /// Start by case definition id (Java `CaseInstanceBuilder.caseDefinitionId`).
    /// Used by event-registry definition-level start
    /// (CmmnEventRegistryEventConsumer.startCaseInstance.java:241-278).
    pub fn start_case_instance_by_id(
        &self,
        case_definition_id: &str,
        request: CmmnCaseInstanceStartRequest,
    ) -> Result<CmmnCaseInstance, CmmnError> {
        let case_definition = self
            .repository_service
            .get_case_definition(case_definition_id)?;
        self.start_case_instance(case_definition, request)
    }

    pub fn create_case_instance_query(&self) -> CmmnCaseInstanceQuery {
        CmmnCaseInstanceQuery::new(self.store.clone())
    }

    pub fn get_case_instance(&self, case_instance_id: &str) -> Result<CmmnCaseInstance, CmmnError> {
        let mut session = self.store.create_session()?;
        load_case_instance_session(&mut session, case_instance_id)?.ok_or_else(|| {
            CmmnError::not_found(format!(
                "CMMN case instance '{case_instance_id}' was not found"
            ))
        })
    }

    /// Update the typed runtime case state (used by parent-resolver contracts and
    /// suspension-aware job management). Preserves all other case fields.
    pub fn set_case_instance_state(
        &self,
        case_instance_id: &str,
        state: CmmnCaseInstanceState,
    ) -> Result<CmmnCaseInstance, CmmnError> {
        // P126: install the engine's lifecycle listener registry for this call, so the
        // transition sites below (many of them free functions taking only a DbSession) can
        // resolve `class` / `delegateExpression` listeners. Java reaches the equivalent
        // registry through the CommandContext (CmmnListenerNotificationHelper.java:162-169).
        let _lifecycle_listener_guard =
            LifecycleListenerRegistryGuard::install(&self.lifecycle_listener_registry);
        let mut session = self.store.create_session()?;
        let mut case_instance = load_case_instance_session(&mut session, case_instance_id)?
            .ok_or_else(|| {
                CmmnError::not_found(format!(
                    "CMMN case instance '{case_instance_id}' was not found"
                ))
            })?;
        // P126: Java fires the case lifecycle listeners before the state field is assigned
        // (AbstractChangeCaseInstanceStateOperation.java:45,47).
        fire_case_lifecycle_listeners_session(
            &mut session,
            &case_instance,
            case_instance.state.as_str(),
            state.as_str(),
        )?;
        case_instance.state = state;
        persist_case_instance_session(&mut session, &case_instance)?;
        session.commit()?;
        Ok(case_instance)
    }

    pub fn delete_case_instance(&self, case_instance_id: &str) -> Result<(), CmmnError> {
        self.end_case_instances(&[case_instance_id.to_string()], None)
    }

    pub fn bulk_delete_case_instances(
        &self,
        case_instance_ids: &[String],
    ) -> Result<(), CmmnError> {
        self.end_case_instances(case_instance_ids, None)
    }

    pub fn terminate_case_instance(&self, case_instance_id: &str) -> Result<(), CmmnError> {
        self.end_case_instances(&[case_instance_id.to_string()], None)
    }

    /// Explicit counterpart to Java's thread-local authenticated finishing user
    /// (`DefaultCmmnHistoryManager.java:89-90`). Passing `None` preserves the
    /// unauthenticated engine/REST behaviour.
    pub fn terminate_case_instance_with_actor(
        &self,
        case_instance_id: &str,
        finished_by: Option<&str>,
    ) -> Result<(), CmmnError> {
        self.end_case_instances(&[case_instance_id.to_string()], finished_by)
    }

    pub fn bulk_terminate_case_instances(
        &self,
        case_instance_ids: &[String],
    ) -> Result<(), CmmnError> {
        self.end_case_instances(case_instance_ids, None)
    }

    pub fn create_human_task_query(&self) -> CmmnHumanTaskQuery {
        CmmnHumanTaskQuery::new(self.store.clone())
    }

    /// P116: unified plan-item-instance query (stage / milestone / event listener
    /// mirror rows). Java `CmmnRuntimeService.createPlanItemInstanceQuery`
    /// (CmmnRuntimeServiceImpl.java:357-358).
    pub fn create_plan_item_instance_query(&self) -> CmmnPlanItemInstanceQuery {
        CmmnPlanItemInstanceQuery::new(self.store.clone())
    }

    pub fn create_event_subscription_query(&self) -> CmmnEventSubscriptionQuery {
        CmmnEventSubscriptionQuery::new(self.store.clone())
    }

    pub fn create_task_association_query(&self) -> CmmnTaskAssociationQuery {
        CmmnTaskAssociationQuery::new(self.store.clone())
    }

    pub fn complete_process_task_child_instance(
        &self,
        process_instance_id: &str,
    ) -> Result<(), CmmnError> {
        self.complete_child_task_association(
            CmmnTaskAssociationKind::ProcessTask,
            process_instance_id,
            CmmnTaskAssociationState::Completed,
            None,
            None,
        )
    }

    // Java parity: ProcessTaskActivityBehavior.java:156 — on trigger the child process variables
    // feed the declared out-parameter mapping back into the parent case. The Rust process task
    // runner is one-way, so the completing side hands the child variables in explicitly.
    pub fn complete_process_task_child_instance_with_variables(
        &self,
        process_instance_id: &str,
        variables: Map<String, Value>,
    ) -> Result<(), CmmnError> {
        self.complete_child_task_association(
            CmmnTaskAssociationKind::ProcessTask,
            process_instance_id,
            CmmnTaskAssociationState::Completed,
            None,
            Some(variables),
        )
    }

    pub fn notify_process_task_child_instance_completed(
        &self,
        process_instance_id: &str,
    ) -> Result<bool, CmmnError> {
        self.try_complete_child_task_association(
            CmmnTaskAssociationKind::ProcessTask,
            process_instance_id,
            CmmnTaskAssociationState::Completed,
            None,
            None,
        )
    }

    pub fn fail_process_task_child_instance(
        &self,
        process_instance_id: &str,
        failure_message: impl Into<String>,
    ) -> Result<(), CmmnError> {
        self.complete_child_task_association(
            CmmnTaskAssociationKind::ProcessTask,
            process_instance_id,
            CmmnTaskAssociationState::Failed,
            Some(failure_message.into()),
            None,
        )
    }

    fn complete_child_task_association(
        &self,
        kind: CmmnTaskAssociationKind,
        child_instance_id: &str,
        target_state: CmmnTaskAssociationState,
        failure_message: Option<String>,
        child_variables: Option<Map<String, Value>>,
    ) -> Result<(), CmmnError> {
        // P126: install the engine's lifecycle listener registry for this call, so the
        // transition sites below (many of them free functions taking only a DbSession) can
        // resolve `class` / `delegateExpression` listeners. Java reaches the equivalent
        // registry through the CommandContext (CmmnListenerNotificationHelper.java:162-169).
        let _lifecycle_listener_guard =
            LifecycleListenerRegistryGuard::install(&self.lifecycle_listener_registry);
        let missing_message = format!(
            "CMMN {:?} association for child instance '{}' was not found",
            kind, child_instance_id
        );
        self.try_complete_child_task_association(
            kind,
            child_instance_id,
            target_state,
            failure_message,
            child_variables,
        )?
        .then_some(())
        .ok_or_else(|| CmmnError::not_found(missing_message))
    }

    fn try_complete_child_task_association(
        &self,
        kind: CmmnTaskAssociationKind,
        child_instance_id: &str,
        target_state: CmmnTaskAssociationState,
        failure_message: Option<String>,
        child_variables: Option<Map<String, Value>>,
    ) -> Result<bool, CmmnError> {
        // P126: install the engine's lifecycle listener registry for this call, so the
        // transition sites below (many of them free functions taking only a DbSession) can
        // resolve `class` / `delegateExpression` listeners. Java reaches the equivalent
        // registry through the CommandContext (CmmnListenerNotificationHelper.java:162-169).
        let _lifecycle_listener_guard =
            LifecycleListenerRegistryGuard::install(&self.lifecycle_listener_registry);
        let mut session = self.store.create_session()?;
        let mut association = match load_active_task_association_by_child_instance_session(
            &mut session,
            &kind,
            child_instance_id,
        )? {
            Some(association) => association,
            None => return Ok(false),
        };
        complete_task_association_session(
            &mut session,
            &mut association,
            target_state,
            failure_message,
            child_variables.as_ref(),
        )?;
        session.commit()?;
        Ok(true)
    }

    pub fn complete_event_subscription(
        &self,
        event_subscription_id: &str,
    ) -> Result<(), CmmnError> {
        self.occur_event_subscription(event_subscription_id)
    }

    pub fn occur_event_subscription(&self, event_subscription_id: &str) -> Result<(), CmmnError> {
        self.occur_event_subscription_with_payload(event_subscription_id, None)
    }

    /// Trigger a CMMN event subscription (occur transition), optionally applying
    /// event-registry payload → case-variable out-parameter mapping first.
    ///
    /// Java: `CmmnEventRegistryEventConsumer.handleEventSubscription` (:108-136)
    /// sets `EVENT_INSTANCE` transient then `trigger()`; out-params are applied in
    /// `EventRegistryEventListenerActivityBehaviour.handleEventInstance` →
    /// `EventInstanceCmmnUtil.handleEventInstanceOutParameters` (EventInstanceCmmnUtil.java:46-68).
    pub fn occur_event_subscription_with_payload(
        &self,
        event_subscription_id: &str,
        event_payload: Option<&Value>,
    ) -> Result<(), CmmnError> {
        // P126: install the engine's lifecycle listener registry for this call, so the
        // transition sites below (many of them free functions taking only a DbSession) can
        // resolve `class` / `delegateExpression` listeners. Java reaches the equivalent
        // registry through the CommandContext (CmmnListenerNotificationHelper.java:162-169).
        let _lifecycle_listener_guard =
            LifecycleListenerRegistryGuard::install(&self.lifecycle_listener_registry);
        let mut session = self.store.create_session()?;
        let subscription = load_event_subscription_session(&mut session, event_subscription_id)?
            .ok_or_else(|| {
                CmmnError::not_found(format!(
                    "CMMN event subscription '{event_subscription_id}' was not found"
                ))
            })?;

        // Apply out-parameter mappings before occur so entry criteria / tasks see variables.
        // Java EventInstanceCmmnUtil.java:46-68 (non-transient → setVariable).
        if let Some(payload) = event_payload
            && let (Some(case_instance_id), Some(case_definition_id), Some(activity_id)) = (
                subscription.case_instance_id.as_deref(),
                subscription.case_definition_id.as_deref(),
                subscription.activity_id.as_deref(),
            )
        {
            let case_definition =
                load_case_definition_session(&mut session, case_definition_id)?.ok_or_else(|| {
                    CmmnError::storage(format!(
                        "CMMN case definition '{case_definition_id}' disappeared during event occurrence"
                    ))
                })?;
            if let Some(listener) = find_event_listener_in_definition(&case_definition, activity_id)
            {
                apply_event_out_parameters_to_case(
                    &mut session,
                    case_instance_id,
                    &listener.event_out_parameters,
                    payload,
                )?;
            }
        }

        let mut p = DbParams::new();
        p.push(event_subscription_id);
        session.execute_raw(RenderedStatement::new(
            "DELETE FROM ACT_CMMN_EVENT_SUBSCRIPTION WHERE ID_ = ?".to_string(),
            p,
        ))?;

        if let (Some(case_instance_id), Some(plan_item_id)) = (
            subscription.case_instance_id.as_deref(),
            subscription.plan_item_instance_id.as_deref(),
        ) {
            record_plan_item_standard_event_session(
                &mut session,
                case_instance_id,
                plan_item_id,
                CmmnPlanItemOnPart::STANDARD_EVENT_OCCUR,
            )?;
            // P116/P139: mark the AVAILABLE event-listener mirror COMPLETED on occur
            // (OccurPlanItemInstanceOperation.java:34-61). Without this the mirror
            // remains AVAILABLE and blocks non-autocomplete case completion even after
            // the subscription row is deleted (PlanItemInstanceContainerUtil.java:143-146).
            complete_plan_item_instance_rows(
                &mut session,
                case_instance_id,
                plan_item_id,
                PLAN_ITEM_DEFINITION_TYPE_EVENT_LISTENER,
            )?;
            if let Some(case_definition_id) = subscription.case_definition_id.as_deref() {
                let case_definition =
                    load_case_definition_session(&mut session, case_definition_id)?.ok_or_else(|| {
                        CmmnError::storage(format!(
                            "CMMN case definition '{case_definition_id}' disappeared during event occurrence"
                        ))
                    })?;
                handle_plan_item_standard_event(
                    &mut session,
                    &case_definition,
                    case_instance_id,
                    plan_item_id,
                    CmmnPlanItemOnPart::STANDARD_EVENT_OCCUR,
                    None,
                )?;
            }
            maybe_complete_case(&mut session, case_instance_id)?;
        }
        session.commit()?;
        Ok(())
    }

    pub fn get_human_task(&self, task_id: &str) -> Result<CmmnHumanTaskInstance, CmmnError> {
        let mut session = self.store.create_session()?;
        load_human_task_session(&mut session, task_id)?.ok_or_else(|| {
            CmmnError::not_found(format!("CMMN human task '{task_id}' was not found"))
        })
    }

    /// Fire a due `cmmn-trigger-timer` job: occur the timer event listener's plan
    /// item (Java `TriggerTimerEventJobHandler.java:35-38` →
    /// `TriggerPlanItemInstanceOperation` → `TimerEventListenerActivityBehaviour.trigger`
    /// → `OccurPlanItemInstanceOperation`) and reschedule a repeating cycle — all in one
    /// transaction. The fired job row is deleted by `CmmnEngine::execute_job` afterwards.
    pub fn fire_timer_event_listener(&self, job: &CmmnJob) -> Result<(), CmmnError> {
        // P126: install the engine's lifecycle listener registry for this call, so the
        // transition sites below (many of them free functions taking only a DbSession) can
        // resolve `class` / `delegateExpression` listeners. Java reaches the equivalent
        // registry through the CommandContext (CmmnListenerNotificationHelper.java:162-169).
        let _lifecycle_listener_guard =
            LifecycleListenerRegistryGuard::install(&self.lifecycle_listener_registry);
        let case_instance_id = job.scope_id.as_deref().ok_or_else(|| {
            CmmnError::execution(format!("CMMN timer job '{}' has no scope_id", job.id))
        })?;
        let plan_item_id = job.sub_scope_id.as_deref().ok_or_else(|| {
            CmmnError::execution(format!(
                "CMMN timer job '{}' has no sub_scope_id (plan item id)",
                job.id
            ))
        })?;
        let mut session = self.store.create_session()?;
        let case_definition =
            load_case_definition_for_case_session(&mut session, case_instance_id)?;
        occur_timer_event_listener_in_session(
            &mut session,
            &case_definition,
            case_instance_id,
            plan_item_id,
        )?;
        if case_still_active(&mut session, case_instance_id)? {
            reschedule_timer_event_listener_job_in_session(&mut session, &case_definition, job)?;
        }
        session.commit()?;
        Ok(())
    }

    /// CMMN-side timer acquisition loop (Java `DefaultJobManager.executeTimerJob`):
    /// scan `ACT_CMMN_JOB` timer-family rows whose due date has passed, fire each via
    /// `occur_timer_event_listener_in_session`, reschedule repeating cycles and delete
    /// the fired row — all in one transaction. Returns the ids of the triggered jobs.
    pub fn run_due_timer_jobs(&self) -> Result<Vec<String>, CmmnError> {
        // P126: install the engine's lifecycle listener registry for this call, so the
        // transition sites below (many of them free functions taking only a DbSession) can
        // resolve `class` / `delegateExpression` listeners. Java reaches the equivalent
        // registry through the CommandContext (CmmnListenerNotificationHelper.java:162-169).
        let _lifecycle_listener_guard =
            LifecycleListenerRegistryGuard::install(&self.lifecycle_listener_registry);
        let mut session = self.store.create_session()?;
        let now = Utc::now();
        let mut params = DbParams::new();
        params.push(now.to_rfc3339());
        let rows = session.select_raw(RenderedStatement::new(
            "SELECT DATA_ FROM ACT_CMMN_JOB WHERE FAMILY_ = 'timer' AND DUE_DATE_ IS NOT NULL \
             AND DUE_DATE_ <= ? ORDER BY DUE_DATE_ ASC, ID_ ASC"
                .to_string(),
            params,
        ))?;
        let mut triggered = Vec::new();
        for row in rows {
            let data = row
                .get_text("DATA_")
                .ok_or_else(|| CmmnError::storage("Missing DATA_ in CMMN timer job row"))?;
            let job: CmmnJob = serde_json::from_str(&data).map_err(CmmnError::from)?;
            let case_instance_id = job.scope_id.as_deref().ok_or_else(|| {
                CmmnError::execution(format!("CMMN timer job '{}' has no scope_id", job.id))
            })?;
            let plan_item_id = job.sub_scope_id.as_deref().ok_or_else(|| {
                CmmnError::execution(format!(
                    "CMMN timer job '{}' has no sub_scope_id (plan item id)",
                    job.id
                ))
            })?;

            // A listener that already occurred / terminated / was dismissed must not fire
            // again; drop the stale job (Java TriggerPlanItemInstanceOperation.java:39-50
            // only triggers an EventListener in the AVAILABLE state).
            if !timer_listener_still_available(&mut session, case_instance_id, plan_item_id)? {
                delete_job_entity_if_exists(&mut session, &job.id)?;
                continue;
            }

            let case_definition =
                load_case_definition_for_case_session(&mut session, case_instance_id)?;
            occur_timer_event_listener_in_session(
                &mut session,
                &case_definition,
                case_instance_id,
                plan_item_id,
            )?;

            // Repeat rescheduling (Java DefaultJobManager.java:535 + TimerJobSchedulerImpl
            // rescheduleTimerJobAfterExecution). Skipped when the occur completed the case
            // (its jobs were cascade-deleted) or the fired job is already gone.
            if case_still_active(&mut session, case_instance_id)?
                && job_exists(&mut session, &job.id)?
            {
                reschedule_timer_event_listener_job_in_session(
                    &mut session,
                    &case_definition,
                    &job,
                )?;
            }
            delete_job_entity_if_exists(&mut session, &job.id)?;
            triggered.push(job.id);
        }
        session.commit()?;
        Ok(triggered)
    }

    pub fn reactivate_plan_item_instance(
        &self,
        plan_item_instance_id: &str,
    ) -> Result<(), CmmnError> {
        // P126: install the engine's lifecycle listener registry for this call, so the
        // transition sites below (many of them free functions taking only a DbSession) can
        // resolve `class` / `delegateExpression` listeners. Java reaches the equivalent
        // registry through the CommandContext (CmmnListenerNotificationHelper.java:162-169).
        let _lifecycle_listener_guard =
            LifecycleListenerRegistryGuard::install(&self.lifecycle_listener_registry);
        let mut session = self.store.create_session()?;

        // Try to load as historic human task
        let historic_task = load_historic_human_task_session(&mut session, plan_item_instance_id)?;
        if let Some(historic) = historic_task {
            // Verify the case instance is still active
            let case_instance =
                load_case_instance_session(&mut session, &historic.case_instance_id)?.ok_or_else(
                    || {
                        CmmnError::not_found(format!(
                            "CMMN case instance '{}' was not found",
                            historic.case_instance_id
                        ))
                    },
                )?;
            if case_instance.state != CmmnCaseInstanceState::Active {
                return Err(CmmnError::conflict(format!(
                    "CMMN case instance '{}' is not active",
                    historic.case_instance_id
                )));
            }

            // Load the case definition
            let case_definition =
                load_case_definition_session(&mut session, &historic.case_definition_id)?
                    .ok_or_else(|| {
                        CmmnError::not_found(format!(
                            "CMMN case definition '{}' was not found",
                            historic.case_definition_id
                        ))
                    })?;

            // Find the human task definition
            let _human_task = find_human_task_by_definition_id(
                &case_definition.model.case_plan_model.human_tasks,
                &historic.task_definition_id,
            )
            .ok_or_else(|| {
                CmmnError::not_found(format!(
                    "CMMN human task definition '{}' was not found",
                    historic.task_definition_id
                ))
            })?;

            // Find the plan item
            let _plan_item = find_plan_item_by_id(
                &case_definition.model.case_plan_model.plan_items,
                &historic.plan_item_id,
            )
            .ok_or_else(|| {
                CmmnError::not_found(format!(
                    "CMMN plan item '{}' was not found",
                    historic.plan_item_id
                ))
            })?;

            // Create a new active human task instance
            let new_task = CmmnHumanTaskInstance {
                id: format!("cmmn-human-task:{}", Uuid::new_v4()),
                case_instance_id: historic.case_instance_id.clone(),
                case_definition_id: historic.case_definition_id.clone(),
                case_definition_key: historic.case_definition_key.clone(),
                stage_instance_id: historic.stage_instance_id.clone(),
                plan_item_id: historic.plan_item_id.clone(),
                task_definition_id: historic.task_definition_id.clone(),
                name: historic.name.clone(),
                activated_at: Utc::now(),
                last_enabled_at: None,
                completed_at: None,
                completed_by: None,
                state: CmmnHumanTaskState::Active,
                assignee: historic.assignee.clone(),
                owner: historic.owner.clone(),
                priority: historic.priority.clone(),
                due_date: historic.due_date.clone(),
                category: historic.category.clone(),
                delegation_state: None,
                task_local_variables: Map::new(),
            };

            persist_human_task_session(&mut session, &new_task)?;
            persist_historic_human_task_session(
                &mut session,
                &CmmnHistoricHumanTaskInstance::from(&new_task),
            )?;

            session.commit()?;
            return Ok(());
        }

        Err(CmmnError::not_found(format!(
            "CMMN plan item instance '{plan_item_instance_id}' was not found"
        )))
    }

    pub fn disable_plan_item_instance(&self, plan_item_instance_id: &str) -> Result<(), CmmnError> {
        // P126: install the engine's lifecycle listener registry for this call, so the
        // transition sites below (many of them free functions taking only a DbSession) can
        // resolve `class` / `delegateExpression` listeners. Java reaches the equivalent
        // registry through the CommandContext (CmmnListenerNotificationHelper.java:162-169).
        let _lifecycle_listener_guard =
            LifecycleListenerRegistryGuard::install(&self.lifecycle_listener_registry);
        let mut session = self.store.create_session()?;

        let mut task =
            load_human_task_session(&mut session, plan_item_instance_id)?.ok_or_else(|| {
                CmmnError::not_found(format!(
                    "CMMN plan item instance '{plan_item_instance_id}' was not found"
                ))
            })?;

        // Java `DisablePlanItemInstanceCmd.java:44-45`: disable is legal only
        // while the plan item is ENABLED.
        if task.state != CmmnHumanTaskState::Enabled {
            return Err(CmmnError::conflict(format!(
                "CMMN plan item instance '{plan_item_instance_id}' cannot be disabled from state '{}'",
                task.state.as_str()
            )));
        }

        // P126: plan item lifecycle listeners fire before the new state is stored
        // (AbstractChangePlanItemInstanceStateOperation.java:64 calls the notification helper
        // before `planItemInstanceEntity.setState`).
        fire_plan_item_lifecycle_listeners_session(
            &mut session,
            &task.case_instance_id,
            Some(&task.id),
            &task.task_definition_id,
            Some("humantask"),
            task.state.as_str(),
            CmmnHumanTaskState::Disabled.as_str(),
        )?;
        task.state = CmmnHumanTaskState::Disabled;
        persist_human_task_session(&mut session, &task)?;
        persist_historic_human_task_session(
            &mut session,
            &CmmnHistoricHumanTaskInstance::from(&task),
        )?;
        let case_definition = load_case_definition_session(&mut session, &task.case_definition_id)?
            .ok_or_else(|| {
                CmmnError::storage(format!(
                    "CMMN case definition '{}' disappeared during plan item disable",
                    task.case_definition_id
                ))
            })?;
        record_plan_item_standard_event_session(
            &mut session,
            &task.case_instance_id,
            &task.plan_item_id,
            CmmnPlanItemOnPart::STANDARD_EVENT_DISABLE,
        )?;
        handle_plan_item_standard_event(
            &mut session,
            &case_definition,
            &task.case_instance_id,
            &task.plan_item_id,
            CmmnPlanItemOnPart::STANDARD_EVENT_DISABLE,
            task.stage_instance_id.as_deref(),
        )?;

        session.commit()?;
        Ok(())
    }

    pub fn enable_plan_item_instance(&self, plan_item_instance_id: &str) -> Result<(), CmmnError> {
        // P126: install the engine's lifecycle listener registry for this call, so the
        // transition sites below (many of them free functions taking only a DbSession) can
        // resolve `class` / `delegateExpression` listeners. Java reaches the equivalent
        // registry through the CommandContext (CmmnListenerNotificationHelper.java:162-169).
        let _lifecycle_listener_guard =
            LifecycleListenerRegistryGuard::install(&self.lifecycle_listener_registry);
        let mut session = self.store.create_session()?;

        let mut task =
            load_human_task_session(&mut session, plan_item_instance_id)?.ok_or_else(|| {
                CmmnError::not_found(format!(
                    "CMMN plan item instance '{plan_item_instance_id}' was not found"
                ))
            })?;

        // P132 deliberately keeps sentry-waiting AVAILABLE instances out of the
        // public command path. Once ENABLED is represented directly, re-enable is
        // the DISABLED -> ENABLED transition; Java records the target state and
        // timestamp in EnablePlanItemInstanceOperation.java:39-51.
        if task.state != CmmnHumanTaskState::Disabled {
            return Err(CmmnError::conflict(format!(
                "CMMN plan item instance '{plan_item_instance_id}' cannot be enabled from state '{}'",
                task.state.as_str()
            )));
        }

        let enabled_at = Utc::now();
        fire_plan_item_lifecycle_listeners_session(
            &mut session,
            &task.case_instance_id,
            Some(&task.id),
            &task.task_definition_id,
            Some("humantask"),
            task.state.as_str(),
            CmmnHumanTaskState::Enabled.as_str(),
        )?;
        task.state = CmmnHumanTaskState::Enabled;
        task.last_enabled_at = Some(enabled_at);
        persist_human_task_session(&mut session, &task)?;
        persist_historic_human_task_session(
            &mut session,
            &CmmnHistoricHumanTaskInstance::from(&task),
        )?;
        let case_definition = load_case_definition_session(&mut session, &task.case_definition_id)?
            .ok_or_else(|| {
                CmmnError::storage(format!(
                    "CMMN case definition '{}' disappeared during plan item enable",
                    task.case_definition_id
                ))
            })?;
        record_plan_item_standard_event_session(
            &mut session,
            &task.case_instance_id,
            &task.plan_item_id,
            CmmnPlanItemOnPart::STANDARD_EVENT_ENABLE,
        )?;
        handle_plan_item_standard_event(
            &mut session,
            &case_definition,
            &task.case_instance_id,
            &task.plan_item_id,
            CmmnPlanItemOnPart::STANDARD_EVENT_ENABLE,
            task.stage_instance_id.as_deref(),
        )?;

        session.commit()?;
        Ok(())
    }

    pub fn get_stage_overview(
        &self,
        case_instance_id: &str,
    ) -> Result<Vec<CmmnStageOverview>, CmmnError> {
        self.get_case_instance(case_instance_id)?;
        list_stage_overview(&self.store, case_instance_id)
    }

    pub fn validate_case_instance_migration(
        &self,
        case_instance_id: &str,
        document: CmmnMigrationDocument,
    ) -> Result<CmmnMigrationValidationResult, CmmnError> {
        let case_instance = self.get_case_instance(case_instance_id)?;
        self.repository_service
            .get_case_definition(&document.target_case_definition_id)?;
        validate_runtime_migration_state(&self.store, &case_instance, &document)
    }

    pub fn migrate_case_instance(
        &self,
        case_instance_id: &str,
        document: CmmnMigrationDocument,
    ) -> Result<(), CmmnError> {
        // P126: install the engine's lifecycle listener registry for this call, so the
        // transition sites below (many of them free functions taking only a DbSession) can
        // resolve `class` / `delegateExpression` listeners. Java reaches the equivalent
        // registry through the CommandContext (CmmnListenerNotificationHelper.java:162-169).
        let _lifecycle_listener_guard =
            LifecycleListenerRegistryGuard::install(&self.lifecycle_listener_registry);
        let validation =
            self.validate_case_instance_migration(case_instance_id, document.clone())?;
        if !validation.valid {
            return Err(CmmnError::conflict(
                validation
                    .validation_messages
                    .first()
                    .cloned()
                    .unwrap_or_else(|| "CMMN case instance migration is not valid".to_string()),
            ));
        }

        let target_definition = self
            .repository_service
            .get_case_definition(&document.target_case_definition_id)?;
        let mut session = self.store.create_session()?;
        let mut case_instance = load_case_instance_session(&mut session, case_instance_id)?
            .ok_or_else(|| {
                CmmnError::not_found(format!(
                    "CMMN case instance '{case_instance_id}' was not found"
                ))
            })?;
        if case_instance.case_definition_id != target_definition.id {
            apply_case_definition_to_runtime_case(&mut case_instance, &target_definition);
            persist_case_instance_session(&mut session, &case_instance)?;
            persist_historic_case_session(
                &mut session,
                &CmmnHistoricCaseInstance::from(&case_instance),
            )?;
        }
        session.commit()?;
        Ok(())
    }

    pub fn migrate_case_instances_of_case_definition(
        &self,
        case_definition_id: &str,
        document: CmmnMigrationDocument,
    ) -> Result<(), CmmnError> {
        self.repository_service
            .get_case_definition(case_definition_id)?;
        self.repository_service
            .get_case_definition(&document.target_case_definition_id)?;

        let case_instances = self
            .create_case_instance_query()
            .case_definition_id(case_definition_id)
            .list()?;
        for case_instance in case_instances {
            self.migrate_case_instance(&case_instance.id, document.clone())?;
        }
        Ok(())
    }

    pub fn change_plan_item_state(
        &self,
        case_instance_id: &str,
        request: CmmnChangePlanItemStateRequest,
    ) -> Result<(), CmmnError> {
        // P126: install the engine's lifecycle listener registry for this call, so the
        // transition sites below (many of them free functions taking only a DbSession) can
        // resolve `class` / `delegateExpression` listeners. Java reaches the equivalent
        // registry through the CommandContext (CmmnListenerNotificationHelper.java:162-169).
        let _lifecycle_listener_guard =
            LifecycleListenerRegistryGuard::install(&self.lifecycle_listener_registry);
        let operation_count = [
            !request.activate_plan_item_definition_ids.is_empty(),
            !request
                .move_to_available_plan_item_definition_ids
                .is_empty(),
            !request.terminate_plan_item_definition_ids.is_empty(),
            !request
                .add_waiting_for_repetition_plan_item_definition_ids
                .is_empty(),
            !request
                .remove_waiting_for_repetition_plan_item_definition_ids
                .is_empty(),
            !request.change_plan_item_ids.is_empty(),
            !request.change_plan_item_ids_with_definition_id.is_empty(),
            !request
                .change_plan_item_definitions_with_new_target_ids
                .is_empty(),
        ]
        .into_iter()
        .filter(|active| *active)
        .count();
        if operation_count != 1 {
            return Err(CmmnError::execution(
                "Exactly one supported CMMN change-state operation is required",
            ));
        }

        let mut case_instance = self.get_case_instance(case_instance_id)?;
        let adds_waiting_for_repetition = !request
            .add_waiting_for_repetition_plan_item_definition_ids
            .is_empty();
        let may_reopen_for_repetition =
            adds_waiting_for_repetition && case_instance.state == CmmnCaseInstanceState::Completed;
        if case_instance.state != CmmnCaseInstanceState::Active && !may_reopen_for_repetition {
            return Err(CmmnError::execution(format!(
                "CMMN case instance '{case_instance_id}' must be active before changing plan item state"
            )));
        }
        let case_definition = self
            .repository_service
            .get_case_definition(&case_instance.case_definition_id)?;

        let mut session = self.store.create_session()?;
        load_case_instance_session(&mut session, case_instance_id)?.ok_or_else(|| {
            CmmnError::not_found(format!(
                "CMMN case instance '{case_instance_id}' was not found"
            ))
        })?;
        if may_reopen_for_repetition {
            // P126: completed → active is a case instance state transition, so the case
            // lifecycle listeners fire before the assignment
            // (AbstractChangeCaseInstanceStateOperation.java:45,47).
            fire_case_lifecycle_listeners_session(
                &mut session,
                &case_instance,
                case_instance.state.as_str(),
                CmmnCaseInstanceState::Active.as_str(),
            )?;
            case_instance.state = CmmnCaseInstanceState::Active;
            case_instance.ended_at = None;
            persist_case_instance_session(&mut session, &case_instance)?;
            persist_historic_case_session(
                &mut session,
                &CmmnHistoricCaseInstance::from(&case_instance),
            )?;
        }

        let moved_to_available = !request
            .move_to_available_plan_item_definition_ids
            .is_empty();

        if !request.terminate_plan_item_definition_ids.is_empty() {
            let terminated_plan_items = terminate_human_tasks_by_definition_ids(
                &mut session,
                case_instance_id,
                &request.terminate_plan_item_definition_ids,
            )?;
            for (plan_item_id, parent_stage_instance_id) in terminated_plan_items {
                handle_plan_item_standard_event(
                    &mut session,
                    &case_definition,
                    case_instance_id,
                    &plan_item_id,
                    CmmnPlanItemOnPart::STANDARD_EVENT_TERMINATE,
                    parent_stage_instance_id.as_deref(),
                )?;
            }
        } else if moved_to_available {
            move_human_tasks_to_available_by_definition_ids(
                &mut session,
                case_instance_id,
                &request.move_to_available_plan_item_definition_ids,
            )?;
        } else if adds_waiting_for_repetition {
            add_waiting_for_repetition_human_tasks_by_definition_ids(
                &mut session,
                &case_definition,
                &case_instance,
                &request.add_waiting_for_repetition_plan_item_definition_ids,
            )?;
        } else if !request
            .remove_waiting_for_repetition_plan_item_definition_ids
            .is_empty()
        {
            remove_waiting_for_repetition_human_tasks_by_definition_ids(
                &mut session,
                case_instance_id,
                &request.remove_waiting_for_repetition_plan_item_definition_ids,
            )?;
        } else if !request.change_plan_item_ids.is_empty() {
            change_plan_item_instances_by_target_plan_item_ids(
                &mut session,
                &case_definition,
                &case_instance,
                &request.change_plan_item_ids,
            )?;
        } else if !request.change_plan_item_ids_with_definition_id.is_empty() {
            change_plan_item_instances_by_target_definition_ids(
                &mut session,
                &case_definition,
                &case_instance,
                &request.change_plan_item_ids_with_definition_id,
            )?;
        } else if !request
            .change_plan_item_definitions_with_new_target_ids
            .is_empty()
        {
            change_plan_item_definitions_with_new_target_ids(
                &mut session,
                &case_definition,
                &case_instance,
                &request.change_plan_item_definitions_with_new_target_ids,
            )?;
        } else {
            activate_plan_items_by_definition_ids(
                &mut session,
                &case_definition,
                &case_instance,
                &request.activate_plan_item_definition_ids,
            )?;
        }

        if !moved_to_available {
            maybe_complete_case(&mut session, case_instance_id)?;
        }
        session.commit()?;
        Ok(())
    }

    pub fn set_case_instance_variables(
        &self,
        case_instance_id: &str,
        variables: Vec<(String, serde_json::Value)>,
    ) -> Result<(), CmmnError> {
        // P126: install the engine's lifecycle listener registry for this call, so the
        // transition sites below (many of them free functions taking only a DbSession) can
        // resolve `class` / `delegateExpression` listeners. Java reaches the equivalent
        // registry through the CommandContext (CmmnListenerNotificationHelper.java:162-169).
        let _lifecycle_listener_guard =
            LifecycleListenerRegistryGuard::install(&self.lifecycle_listener_registry);
        let mut session = self.store.create_session()?;
        let mut case_instance = load_case_instance_session(&mut session, case_instance_id)?
            .ok_or_else(|| {
                CmmnError::not_found(format!(
                    "CMMN case instance '{case_instance_id}' was not found"
                ))
            })?;

        // Java: variable writes record their change type ("create" vs "update"), which
        // variable event listeners match against
        // (EvaluateVariableEventListenersOperation.java:93-95).
        let mut variable_changes: Vec<(String, &str)> = Vec::new();
        for (name, value) in variables {
            let change_type = if case_instance.variables.contains_key(&name) {
                CmmnEventListener::CHANGE_TYPE_UPDATE
            } else {
                CmmnEventListener::CHANGE_TYPE_CREATE
            };
            variable_changes.push((name.clone(), change_type));
            case_instance.variables.insert(name, value);
        }

        persist_case_instance_session(&mut session, &case_instance)?;
        persist_historic_case_session(
            &mut session,
            &CmmnHistoricCaseInstance::from(&case_instance),
        )?;
        let case_definition =
            load_case_definition_session(&mut session, &case_instance.case_definition_id)?
                .ok_or_else(|| {
                    CmmnError::storage(format!(
                        "CMMN case definition '{}' disappeared during variable update",
                        case_instance.case_definition_id
                    ))
                })?;
        // Java: availableCondition of event listeners is re-evaluated on every evaluation
        // cycle (AbstractEvaluationCriteriaOperation.java:584-604), moving listeners between
        // unavailable and available in both directions.
        reevaluate_event_listener_available_conditions(
            &mut session,
            &case_definition,
            &case_instance,
        )?;
        // Java: variable writes trigger matching "variable" event subscriptions
        // (EvaluateVariableEventListenersOperation.java:58-104).
        trigger_variable_event_listeners(
            &mut session,
            &case_definition,
            &case_instance,
            &variable_changes,
        )?;
        // Java: SetVariablesCmd plans an evaluate-criteria operation; in the
        // default trigger mode the ifPart of a multi-part sentry is evaluated
        // on every such cycle and persisted once satisfied
        // (AbstractEvaluationCriteriaOperation.java:550-566).
        record_satisfied_sentry_if_parts(&mut session, &case_definition, &case_instance)?;
        handle_if_part_only_exit_criteria(&mut session, &case_definition, &case_instance.id)?;
        maybe_complete_case(&mut session, &case_instance.id)?;
        session.commit()?;
        Ok(())
    }

    // Java parity: CmmnRuntimeService#updateBusinessStatus (used by
    // MilestoneActivityBehavior.java:59 when a reached milestone declares a businessStatus).
    pub fn update_business_status(
        &self,
        case_instance_id: &str,
        business_status: impl Into<String>,
    ) -> Result<(), CmmnError> {
        let mut session = self.store.create_session()?;
        let mut case_instance = load_case_instance_session(&mut session, case_instance_id)?
            .ok_or_else(|| {
                CmmnError::not_found(format!(
                    "CMMN case instance '{case_instance_id}' was not found"
                ))
            })?;
        case_instance.business_status = Some(business_status.into());
        persist_case_instance_session(&mut session, &case_instance)?;
        persist_historic_case_session(
            &mut session,
            &CmmnHistoricCaseInstance::from(&case_instance),
        )?;
        session.commit()?;
        Ok(())
    }

    // Java parity: CmmnRuntimeServiceImpl#setCaseInstanceName
    // (CmmnRuntimeServiceImpl.java:347) → SetCaseInstanceNameCmd.java:48-63.
    pub fn set_case_instance_name(
        &self,
        case_instance_id: &str,
        name: impl Into<String>,
    ) -> Result<(), CmmnError> {
        let mut session = self.store.create_session()?;
        let mut case_instance = load_case_instance_session(&mut session, case_instance_id)?
            .ok_or_else(|| {
                CmmnError::not_found(format!(
                    "CMMN case instance '{case_instance_id}' was not found"
                ))
            })?;
        case_instance.name = name.into();
        persist_case_instance_session(&mut session, &case_instance)?;
        persist_historic_case_session(
            &mut session,
            &CmmnHistoricCaseInstance::from(&case_instance),
        )?;
        session.commit()?;
        Ok(())
    }

    // Java parity: CmmnRuntimeServiceImpl#updateBusinessKey
    // (CmmnRuntimeServiceImpl.java:467) → SetCaseInstanceBusinessKeyCmd.java:55-72.
    pub fn update_business_key(
        &self,
        case_instance_id: &str,
        business_key: impl Into<String>,
    ) -> Result<(), CmmnError> {
        let mut session = self.store.create_session()?;
        let mut case_instance = load_case_instance_session(&mut session, case_instance_id)?
            .ok_or_else(|| {
                CmmnError::not_found(format!(
                    "CMMN case instance '{case_instance_id}' was not found"
                ))
            })?;
        case_instance.business_key = Some(business_key.into());
        persist_case_instance_session(&mut session, &case_instance)?;
        persist_historic_case_session(
            &mut session,
            &CmmnHistoricCaseInstance::from(&case_instance),
        )?;
        session.commit()?;
        Ok(())
    }

    // Java parity: CmmnRuntimeServiceImpl#removeVariable
    // (CmmnRuntimeServiceImpl.java:322) → RemoveVariableCmd.java:45-63.
    pub fn remove_variable(
        &self,
        case_instance_id: &str,
        variable_name: &str,
    ) -> Result<(), CmmnError> {
        self.remove_variables(case_instance_id, std::slice::from_ref(&variable_name))
    }

    // Java parity: CmmnRuntimeServiceImpl#removeVariables
    // (CmmnRuntimeServiceImpl.java:327).
    pub fn remove_variables(
        &self,
        case_instance_id: &str,
        variable_names: &[&str],
    ) -> Result<(), CmmnError> {
        let mut session = self.store.create_session()?;
        let mut case_instance = load_case_instance_session(&mut session, case_instance_id)?
            .ok_or_else(|| {
                CmmnError::not_found(format!(
                    "CMMN case instance '{case_instance_id}' was not found"
                ))
            })?;
        for name in variable_names {
            case_instance.variables.remove(*name);
        }
        persist_case_instance_session(&mut session, &case_instance)?;
        persist_historic_case_session(
            &mut session,
            &CmmnHistoricCaseInstance::from(&case_instance),
        )?;
        session.commit()?;
        Ok(())
    }

    // Java parity: CmmnRuntimeServiceImpl#evaluateCriteria
    // (CmmnRuntimeServiceImpl.java:202) → EvaluateCriteriaCmd.java:36-40
    // (plans EvaluateCriteriaOperation). Re-runs the evaluation-cycle sweep that
    // variable writes also schedule (record_satisfied_sentry_if_parts +
    // handle_if_part_only_exit_criteria + event-listener availableCondition).
    pub fn evaluate_criteria(&self, case_instance_id: &str) -> Result<(), CmmnError> {
        // P126: install the engine's lifecycle listener registry for this call, so the
        // transition sites below (many of them free functions taking only a DbSession) can
        // resolve `class` / `delegateExpression` listeners. Java reaches the equivalent
        // registry through the CommandContext (CmmnListenerNotificationHelper.java:162-169).
        let _lifecycle_listener_guard =
            LifecycleListenerRegistryGuard::install(&self.lifecycle_listener_registry);
        let mut session = self.store.create_session()?;
        let case_instance = load_case_instance_session(&mut session, case_instance_id)?
            .ok_or_else(|| {
                CmmnError::not_found(format!(
                    "CMMN case instance '{case_instance_id}' was not found"
                ))
            })?;
        let case_definition =
            load_case_definition_session(&mut session, &case_instance.case_definition_id)?
                .ok_or_else(|| {
                    CmmnError::storage(format!(
                        "CMMN case definition '{}' disappeared during evaluateCriteria",
                        case_instance.case_definition_id
                    ))
                })?;
        reevaluate_event_listener_available_conditions(
            &mut session,
            &case_definition,
            &case_instance,
        )?;
        record_satisfied_sentry_if_parts(&mut session, &case_definition, &case_instance)?;
        handle_if_part_only_exit_criteria(&mut session, &case_definition, &case_instance.id)?;
        maybe_complete_case(&mut session, &case_instance.id)?;
        session.commit()?;
        Ok(())
    }

    // Java parity: CmmnRuntimeServiceImpl#triggerPlanItemInstance
    // (CmmnRuntimeServiceImpl.java:142) → TriggerPlanItemInstanceCmd /
    // StartPlanItemInstanceCmd for manual-activation human tasks.
    // Java `StartPlanItemInstanceCmd.java:54-58`: trigger/start moves ENABLED to
    // ACTIVE and records the start lifecycle event.
    pub fn trigger_plan_item_instance(&self, plan_item_instance_id: &str) -> Result<(), CmmnError> {
        self.start_plan_item_instance(plan_item_instance_id)
    }

    // Java parity: CmmnRuntimeServiceImpl#startPlanItemInstance
    // (StartPlanItemInstanceCmd.java:54-58 — requires ENABLED).
    pub fn start_plan_item_instance(&self, plan_item_instance_id: &str) -> Result<(), CmmnError> {
        // P126: install the engine's lifecycle listener registry for this call, so the
        // transition sites below (many of them free functions taking only a DbSession) can
        // resolve `class` / `delegateExpression` listeners. Java reaches the equivalent
        // registry through the CommandContext (CmmnListenerNotificationHelper.java:162-169).
        let _lifecycle_listener_guard =
            LifecycleListenerRegistryGuard::install(&self.lifecycle_listener_registry);
        let mut session = self.store.create_session()?;

        let mut task =
            load_human_task_session(&mut session, plan_item_instance_id)?.ok_or_else(|| {
                CmmnError::not_found(format!(
                    "CMMN plan item instance '{plan_item_instance_id}' was not found"
                ))
            })?;

        if task.state != CmmnHumanTaskState::Enabled {
            return Err(CmmnError::conflict(format!(
                "CMMN plan item instance '{plan_item_instance_id}' cannot be started from state '{}'",
                task.state.as_str()
            )));
        }

        // P126/P132: enabled -> active (Java's `start` transition,
        // AbstractChangePlanItemInstanceStateOperation.java:64).
        fire_plan_item_lifecycle_listeners_session(
            &mut session,
            &task.case_instance_id,
            Some(&task.id),
            &task.task_definition_id,
            Some("humantask"),
            task.state.as_str(),
            CmmnHumanTaskState::Active.as_str(),
        )?;
        task.state = CmmnHumanTaskState::Active;
        persist_human_task_session(&mut session, &task)?;
        persist_historic_human_task_session(
            &mut session,
            &CmmnHistoricHumanTaskInstance::from(&task),
        )?;

        let case_definition = load_case_definition_session(&mut session, &task.case_definition_id)?
            .ok_or_else(|| {
                CmmnError::storage(format!(
                    "CMMN case definition '{}' disappeared during plan item start",
                    task.case_definition_id
                ))
            })?;

        if !plan_item_standard_event_occurred(
            &mut session,
            &task.case_instance_id,
            &task.plan_item_id,
            CmmnPlanItemOnPart::STANDARD_EVENT_START,
        )? {
            record_plan_item_standard_event_session(
                &mut session,
                &task.case_instance_id,
                &task.plan_item_id,
                CmmnPlanItemOnPart::STANDARD_EVENT_START,
            )?;
        }
        handle_plan_item_standard_event(
            &mut session,
            &case_definition,
            &task.case_instance_id,
            &task.plan_item_id,
            CmmnPlanItemOnPart::STANDARD_EVENT_START,
            task.stage_instance_id.as_deref(),
        )?;
        maybe_complete_case(&mut session, &task.case_instance_id)?;

        session.commit()?;
        Ok(())
    }

    fn end_case_instances(
        &self,
        case_instance_ids: &[String],
        finished_by: Option<&str>,
    ) -> Result<(), CmmnError> {
        // P126: install the engine's lifecycle listener registry for this call, so the
        // transition sites below (many of them free functions taking only a DbSession) can
        // resolve `class` / `delegateExpression` listeners. Java reaches the equivalent
        // registry through the CommandContext (CmmnListenerNotificationHelper.java:162-169).
        let _lifecycle_listener_guard =
            LifecycleListenerRegistryGuard::install(&self.lifecycle_listener_registry);
        let mut session = self.store.create_session()?;

        let mut unique_case_instance_ids = Vec::new();
        for case_instance_id in case_instance_ids {
            if !unique_case_instance_ids.contains(case_instance_id) {
                unique_case_instance_ids.push(case_instance_id.clone());
            }
        }

        let mut case_instances = Vec::with_capacity(unique_case_instance_ids.len());
        for case_instance_id in &unique_case_instance_ids {
            let case_instance = load_case_instance_session(&mut session, case_instance_id)?
                .ok_or_else(|| {
                    CmmnError::not_found(format!(
                        "CMMN case instance '{case_instance_id}' was not found"
                    ))
                })?;
            case_instances.push(case_instance);
        }

        let ended_at = Utc::now();
        for mut case_instance in case_instances {
            // Java fires the lifecycle listeners before setState
            // (AbstractChangeCaseInstanceStateOperation.java:45,47).
            fire_case_lifecycle_listeners_session(
                &mut session,
                &case_instance,
                case_instance.state.as_str(),
                CmmnCaseInstanceState::Terminated.as_str(),
            )?;
            case_instance.state = CmmnCaseInstanceState::Terminated;
            case_instance.ended_at = Some(ended_at);
            let mut historic_case = CmmnHistoricCaseInstance::from(&case_instance);
            // Java writes Authentication.getAuthenticatedUserId at case end
            // (DefaultCmmnHistoryManager.java:89-90); Rust receives it explicitly.
            historic_case.finished_by = finished_by.map(str::to_owned);
            persist_historic_case_session(&mut session, &historic_case)?;
            persist_ended_stage_history_for_case_session(
                &mut session,
                &case_instance.id,
                ended_at,
            )?;
            terminate_parent_case_task_associations_for_child_case(
                &mut session,
                &case_instance.id,
            )?;
            terminate_open_plan_item_instances_for_case_session(
                &mut session,
                &case_instance.id,
                ended_at,
            )?;
            delete_runtime_case_instance_session(&mut session, &case_instance.id)?;
        }

        session.commit()?;
        Ok(())
    }

    pub fn complete_human_task(
        &self,
        task_id: &str,
        request: CmmnHumanTaskCompletionRequest,
    ) -> Result<CmmnHumanTaskCompletionResult, CmmnError> {
        // P126: install the engine's lifecycle listener registry for this call, so the
        // transition sites below (many of them free functions taking only a DbSession) can
        // resolve `class` / `delegateExpression` listeners. Java reaches the equivalent
        // registry through the CommandContext (CmmnListenerNotificationHelper.java:162-169).
        let _lifecycle_listener_guard =
            LifecycleListenerRegistryGuard::install(&self.lifecycle_listener_registry);
        let case_definition_id = {
            let mut session = self.store.create_session()?;
            load_human_task_session(&mut session, task_id)?
                .ok_or_else(|| {
                    CmmnError::not_found(format!("CMMN human task '{task_id}' was not found"))
                })?
                .case_definition_id
        };
        let case_definition = self
            .repository_service
            .get_case_definition(&case_definition_id)?;

        let mut session = self.store.create_session()?;

        let mut task = load_human_task_session(&mut session, task_id)?.ok_or_else(|| {
            CmmnError::not_found(format!("CMMN human task '{task_id}' was not found"))
        })?;
        if task.state == CmmnHumanTaskState::Completed {
            return Err(CmmnError::execution(format!(
                "CMMN human task '{task_id}' is already completed"
            )));
        }
        if task.state != CmmnHumanTaskState::Active {
            return Err(CmmnError::execution(format!(
                "CMMN human task '{task_id}' must be active before it can be completed"
            )));
        }

        // P126: active → completed. Java routes the terminal transitions through
        // AbstractMovePlanItemInstanceToTerminalStateOperation.java:124, which likewise notifies
        // before the state is written.
        fire_plan_item_lifecycle_listeners_session(
            &mut session,
            &task.case_instance_id,
            Some(&task.id),
            &task.task_definition_id,
            Some("humantask"),
            task.state.as_str(),
            CmmnHumanTaskState::Completed.as_str(),
        )?;
        task.state = CmmnHumanTaskState::Completed;
        task.completed_at = Some(Utc::now());
        task.completed_by = request.completed_by;
        // Java parity: completing the human task deletes the TaskEntity and its
        // task-local variables (HumanTaskActivityBehavior.java:482 completeTask →
        // CMMN TaskHelper.internalDeleteTask.java:109-128). The Rust task row
        // survives with state COMPLETED, so clear the local scope here.
        task.task_local_variables.clear();
        persist_human_task_session(&mut session, &task)?;
        persist_historic_human_task_session(
            &mut session,
            &CmmnHistoricHumanTaskInstance::from(&task),
        )?;

        // Java parity: GLOBAL completion variables are written to the case
        // before the complete standard event, so the dependent sentry
        // evaluation observes them (CompleteTaskCmd.java:100-101 sets them on
        // the task entity, which is case-scoped for CMMN).
        if !request.variables.is_empty() {
            let mut case_instance =
                load_case_instance_session(&mut session, &task.case_instance_id)?.ok_or_else(
                    || {
                        CmmnError::storage(format!(
                            "CMMN case instance '{}' disappeared during task completion",
                            task.case_instance_id
                        ))
                    },
                )?;
            for (name, value) in &request.variables {
                case_instance.variables.insert(name.clone(), value.clone());
            }
            persist_case_instance_session(&mut session, &case_instance)?;
            persist_historic_case_session(
                &mut session,
                &CmmnHistoricCaseInstance::from(&case_instance),
            )?;
        }

        // Java parity: HumanTaskActivityBehavior.java:498-507 — on the complete
        // transition a declared taskCompleterVariableName stores the completing
        // user, before the dependent sentry evaluation observes the completion.
        if let Some((_, human_task)) = find_human_task_plan_item_by_plan_item_id(
            &case_definition.model.case_plan_model,
            &task.plan_item_id,
        ) && let Some(variable_name) = &human_task.task_completer_variable_name
        {
            let mut refreshed_case =
                load_case_instance_session(&mut session, &task.case_instance_id)?.ok_or_else(
                    || {
                        CmmnError::storage(format!(
                            "CMMN case instance '{}' disappeared while storing the task completer",
                            task.case_instance_id
                        ))
                    },
                )?;
            refreshed_case.variables.insert(
                variable_name.clone(),
                task.completed_by
                    .clone()
                    .map(Value::String)
                    .unwrap_or(Value::Null),
            );
            persist_case_instance_session(&mut session, &refreshed_case)?;
        }

        if !plan_item_standard_event_occurred(
            &mut session,
            &task.case_instance_id,
            &task.plan_item_id,
            CmmnPlanItemOnPart::STANDARD_EVENT_COMPLETE,
        )? {
            record_plan_item_standard_event_session(
                &mut session,
                &task.case_instance_id,
                &task.plan_item_id,
                CmmnPlanItemOnPart::STANDARD_EVENT_COMPLETE,
            )?;
        }
        handle_plan_item_standard_event(
            &mut session,
            &case_definition,
            &task.case_instance_id,
            &task.plan_item_id,
            CmmnPlanItemOnPart::STANDARD_EVENT_COMPLETE,
            task.stage_instance_id.as_deref(),
        )?;
        repeat_human_task_if_needed(&mut session, &case_definition, &task)?;
        if let Some(stage_instance_id) = task.stage_instance_id.as_deref() {
            maybe_complete_stage(&mut session, &case_definition, stage_instance_id)?;
        }
        maybe_complete_case(&mut session, &task.case_instance_id)?;

        let case_instance = load_case_instance_session(&mut session, &task.case_instance_id)?
            .ok_or_else(|| {
                CmmnError::storage(format!(
                    "CMMN case instance '{}' disappeared during task completion",
                    task.case_instance_id
                ))
            })?;

        session.commit()?;

        // Java ChildBpmnCaseInstanceStateChangeCallback — after durable complete.
        self.notify_bpmn_case_task_callback_if_needed(&case_instance)?;

        Ok(CmmnHumanTaskCompletionResult {
            task,
            case_instance,
        })
    }

    /// Java `TaskResource.updateTask` (TaskResource.java:76-99) →
    /// `populateTaskFromRequest` (TaskBaseResource.java:91-127) + `saveTask` +
    /// re-query. `CmmnHumanTaskUpdate` uses `Option<Option<T>>`: outer present
    /// in the request, inner applied (None clears the field).
    pub fn update_human_task(
        &self,
        task_id: &str,
        update: CmmnHumanTaskUpdate,
    ) -> Result<CmmnHumanTaskInstance, CmmnError> {
        let mut session = self.store.create_session()?;
        let mut task = load_human_task_session(&mut session, task_id)?.ok_or_else(|| {
            CmmnError::not_found(format!("CMMN human task '{task_id}' was not found"))
        })?;
        if let Some(name) = update.name {
            // `name` is a required String in the model (Java allows null);
            // explicit null clears to the empty string.
            task.name = name.unwrap_or_default();
        }
        if let Some(assignee) = update.assignee {
            task.assignee = assignee;
        }
        if let Some(owner) = update.owner {
            task.owner = owner;
        }
        if let Some(priority) = update.priority {
            task.priority = priority;
        }
        if let Some(due_date) = update.due_date {
            task.due_date = due_date;
        }
        if let Some(category) = update.category {
            task.category = category;
        }
        if let Some(delegation_state) = update.delegation_state {
            task.delegation_state = delegation_state;
        }
        persist_human_task_session(&mut session, &task)?;
        session.commit()?;
        Ok(task)
    }

    /// Java `ClaimTaskCmd` (ClaimTaskCmd.java:39-85): assign the task to a
    /// user. When the task is already claimed by a different user Java throws
    /// `FlowableTaskAlreadyClaimedException` (ClaimTaskCmd.java:51) → REST 409;
    /// re-claim by the same user is a no-op.
    pub fn claim_human_task(
        &self,
        task_id: &str,
        assignee: &str,
    ) -> Result<CmmnHumanTaskInstance, CmmnError> {
        let mut session = self.store.create_session()?;
        let mut task = self.claimable_human_task(&mut session, task_id)?;
        if let Some(current) = task.assignee.as_deref() {
            if current != assignee {
                return Err(CmmnError::conflict(format!(
                    "CMMN human task '{task_id}' is already claimed by '{current}'"
                )));
            }
        } else {
            task.assignee = Some(assignee.to_string());
        }
        persist_human_task_session(&mut session, &task)?;
        session.commit()?;
        Ok(task)
    }

    /// Java `DelegateTaskCmd` (DelegateTaskCmd.java:37-47): delegationState =
    /// PENDING, owner defaults to the current assignee, assignee becomes the
    /// delegatee.
    pub fn delegate_human_task(
        &self,
        task_id: &str,
        assignee: &str,
    ) -> Result<CmmnHumanTaskInstance, CmmnError> {
        let mut session = self.store.create_session()?;
        let mut task = self.claimable_human_task(&mut session, task_id)?;
        task.delegation_state = Some(CmmnDelegationState::Pending);
        if task.owner.is_none() {
            task.owner = task.assignee.clone();
        }
        task.assignee = Some(assignee.to_string());
        persist_human_task_session(&mut session, &task)?;
        session.commit()?;
        Ok(task)
    }

    /// Java `ResolveTaskCmd` (ResolveTaskCmd.java:55-57): delegationState =
    /// RESOLVED and the assignee returns to the owner.
    pub fn resolve_human_task(&self, task_id: &str) -> Result<CmmnHumanTaskInstance, CmmnError> {
        let mut session = self.store.create_session()?;
        let mut task = self.claimable_human_task(&mut session, task_id)?;
        task.delegation_state = Some(CmmnDelegationState::Resolved);
        task.assignee = task.owner.clone();
        persist_human_task_session(&mut session, &task)?;
        session.commit()?;
        Ok(task)
    }

    // ── Task-local variables (P115) ──────────────────────────────────────────
    // Java parity: TaskService#setVariableLocal (TaskServiceImpl.java:430-437)
    // → SetTaskVariablesCmd.java:42-47 → TaskEntity.setVariableLocal
    // (VariableScopeImpl.java:743-785) — writes land on the task's own scope
    // only, keyed by task id. A non-active task has no live TaskEntity in Java
    // (created on activation, deleted on complete/terminate —
    // HumanTaskActivityBehavior.java:107, CMMN TaskHelper.java:109-128), so
    // writes on them 404 like NeedsActiveTaskCmd.java:57.
    pub fn set_task_variable_local(
        &self,
        task_id: &str,
        variable_name: impl Into<String>,
        value: Value,
    ) -> Result<(), CmmnError> {
        self.set_task_variables_local(task_id, vec![(variable_name.into(), value)])
    }

    pub fn set_task_variables_local(
        &self,
        task_id: &str,
        variables: Vec<(String, Value)>,
    ) -> Result<(), CmmnError> {
        let mut session = self.store.create_session()?;
        let mut task = self.active_task_for_local_variables(&mut session, task_id)?;
        for (name, value) in variables {
            task.task_local_variables.insert(name, value);
        }
        persist_human_task_session(&mut session, &task)?;
        session.commit()?;
        Ok(())
    }

    // Java parity: TaskService#getVariableLocal (TaskServiceImpl.java:400-402)
    // → GetTaskVariableCmd.java:62-63 → task.getVariableLocal
    // (VariableScopeImpl.java:338-384) — the local scope only, no parent walk.
    pub fn get_task_variable_local(
        &self,
        task_id: &str,
        variable_name: &str,
    ) -> Result<Option<Value>, CmmnError> {
        let mut session = self.store.create_session()?;
        let task = self.readable_task(&mut session, task_id)?;
        Ok(task.task_local_variables.get(variable_name).cloned())
    }

    // Java parity: TaskService#getVariablesLocal (TaskServiceImpl.java:370-372)
    // → GetTaskVariablesCmd.java:62-63 → task.getVariablesLocal
    // (VariableScopeImpl.java:455-470).
    pub fn get_task_variables_local(&self, task_id: &str) -> Result<Map<String, Value>, CmmnError> {
        let mut session = self.store.create_session()?;
        let task = self.readable_task(&mut session, task_id)?;
        Ok(task.task_local_variables.clone())
    }

    // Java parity: TaskService#getVariable (TaskServiceImpl.java:385-387) →
    // GetTaskVariableCmd.java:62-66 → task.getVariable — the local scope first,
    // then the parent scope (the case instance;
    // DefaultCmmnTaskVariableScopeResolver.java:34-43). Local shadows case.
    pub fn get_task_variable(
        &self,
        task_id: &str,
        variable_name: &str,
    ) -> Result<Option<Value>, CmmnError> {
        let mut session = self.store.create_session()?;
        let task = self.readable_task(&mut session, task_id)?;
        if let Some(value) = task.task_local_variables.get(variable_name) {
            return Ok(Some(value.clone()));
        }
        let case_instance = load_case_instance_session(&mut session, &task.case_instance_id)?
            .ok_or_else(|| {
                CmmnError::storage(format!(
                    "CMMN case instance '{}' disappeared during task variable read",
                    task.case_instance_id
                ))
            })?;
        Ok(case_instance.variables.get(variable_name).cloned())
    }

    // Java parity: TaskService#getVariables (TaskServiceImpl.java:365-367) →
    // GetTaskVariablesCmd.java:62-66 → task.getVariables → collectVariables
    // (VariableScopeImpl.java:203-225) — parent (case) first, then local
    // overrides, so task-local shadows case on name conflicts.
    pub fn get_task_variables(&self, task_id: &str) -> Result<Map<String, Value>, CmmnError> {
        let mut session = self.store.create_session()?;
        let task = self.readable_task(&mut session, task_id)?;
        let case_instance = load_case_instance_session(&mut session, &task.case_instance_id)?
            .ok_or_else(|| {
                CmmnError::storage(format!(
                    "CMMN case instance '{}' disappeared during task variable read",
                    task.case_instance_id
                ))
            })?;
        let mut variables = case_instance.variables;
        for (name, value) in &task.task_local_variables {
            variables.insert(name.clone(), value.clone());
        }
        Ok(variables)
    }

    // Java parity: TaskService#hasVariableLocal (TaskServiceImpl.java:415-417)
    // → HasTaskVariableCmd.java:61-62 → task.hasVariableLocal
    // (VariableScopeImpl.java:425-431) — local scope only.
    pub fn has_task_variable_local(
        &self,
        task_id: &str,
        variable_name: &str,
    ) -> Result<bool, CmmnError> {
        let mut session = self.store.create_session()?;
        let task = self.readable_task(&mut session, task_id)?;
        Ok(task.task_local_variables.contains_key(variable_name))
    }

    // Java parity: TaskService#hasVariable (TaskServiceImpl.java:395-397) →
    // HasTaskVariableCmd.java:63-64 → task.hasVariable
    // (VariableScopeImpl.java:413-422) — local first, then parent.
    pub fn has_task_variable(&self, task_id: &str, variable_name: &str) -> Result<bool, CmmnError> {
        let mut session = self.store.create_session()?;
        let task = self.readable_task(&mut session, task_id)?;
        if task.task_local_variables.contains_key(variable_name) {
            return Ok(true);
        }
        let case_instance = load_case_instance_session(&mut session, &task.case_instance_id)?
            .ok_or_else(|| {
                CmmnError::storage(format!(
                    "CMMN case instance '{}' disappeared during task variable read",
                    task.case_instance_id
                ))
            })?;
        Ok(case_instance.variables.contains_key(variable_name))
    }

    // Java parity: TaskService#removeVariableLocal (TaskServiceImpl.java:457-461)
    // → RemoveTaskVariablesCmd.java:38-42 → task.removeVariablesLocal
    // (VariableScopeImpl.java:643-649 → 814-820).
    pub fn remove_task_variable_local(
        &self,
        task_id: &str,
        variable_name: &str,
    ) -> Result<(), CmmnError> {
        self.remove_task_variables_local(task_id, std::slice::from_ref(&variable_name))
    }

    pub fn remove_task_variables_local(
        &self,
        task_id: &str,
        variable_names: &[&str],
    ) -> Result<(), CmmnError> {
        let mut session = self.store.create_session()?;
        let mut task = self.active_task_for_local_variables(&mut session, task_id)?;
        for name in variable_names {
            task.task_local_variables.remove(*name);
        }
        persist_human_task_session(&mut session, &task)?;
        session.commit()?;
        Ok(())
    }

    fn readable_task(
        &self,
        session: &mut DbSession,
        task_id: &str,
    ) -> Result<CmmnHumanTaskInstance, CmmnError> {
        load_human_task_session(session, task_id)?.ok_or_else(|| {
            CmmnError::not_found(format!("CMMN human task '{task_id}' was not found"))
        })
    }

    /// Load a human task for a task-local variable write. Java has no live
    /// TaskEntity for non-active human tasks — the entity is created on
    /// activation and deleted on complete/terminate
    /// (HumanTaskActivityBehavior.java:107, TaskHelper.java:109-128) — so
    /// writes on them fail 404 like NeedsActiveTaskCmd.java:57.
    fn active_task_for_local_variables(
        &self,
        session: &mut DbSession,
        task_id: &str,
    ) -> Result<CmmnHumanTaskInstance, CmmnError> {
        let task = self.readable_task(session, task_id)?;
        if task.state != CmmnHumanTaskState::Active {
            return Err(CmmnError::not_found(format!(
                "CMMN human task '{task_id}' was not found"
            )));
        }
        Ok(task)
    }

    /// Load a human task and verify it is in a state that accepts the
    /// claim/delegate/resolve actions. Java `NeedsActiveTaskCmd` only guards a
    /// suspended task; the plan-item state model maps that to "active".
    fn claimable_human_task(
        &self,
        session: &mut DbSession,
        task_id: &str,
    ) -> Result<CmmnHumanTaskInstance, CmmnError> {
        let task = load_human_task_session(session, task_id)?.ok_or_else(|| {
            CmmnError::not_found(format!("CMMN human task '{task_id}' was not found"))
        })?;
        if task.state != CmmnHumanTaskState::Active {
            return Err(CmmnError::execution(format!(
                "CMMN human task '{task_id}' must be active before it can be claimed, delegated, or resolved"
            )));
        }
        Ok(task)
    }

    fn start_case_instance(
        &self,
        case_definition: CmmnCaseDefinition,
        request: CmmnCaseInstanceStartRequest,
    ) -> Result<CmmnCaseInstance, CmmnError> {
        // P126: install the engine's lifecycle listener registry for this call, so the
        // transition sites below (many of them free functions taking only a DbSession) can
        // resolve `class` / `delegateExpression` listeners. Java reaches the equivalent
        // registry through the CommandContext (CmmnListenerNotificationHelper.java:162-169).
        let _lifecycle_listener_guard =
            LifecycleListenerRegistryGuard::install(&self.lifecycle_listener_registry);
        let mut session = self.store.create_session()?;
        let case_instance = start_case_instance_session(
            &mut session,
            &case_definition,
            request,
            self.process_task_runner.as_ref(),
        )?;
        session.commit()?;
        // Do NOT notify BPMN here for EXECUTION_CHILD_CASE: the parent
        // CaseTaskActivityBehavior is still mid-command (execution not durable
        // yet). Sync auto-complete is handled by CaseTaskActivityBehavior after
        // start returns (CaseTaskActivityBehavior.java leaves only after execute
        // finishes / via later callback when the child was still open).
        // notify_bpmn_case_task_callback_if_needed is invoked from complete_human_task
        // and other post-start lifecycle paths instead.
        Ok(case_instance)
    }
}

fn start_case_instance_session(
    session: &mut DbSession,
    case_definition: &CmmnCaseDefinition,
    request: CmmnCaseInstanceStartRequest,
    process_task_runner: Option<&Arc<dyn CmmnProcessTaskRunner>>,
) -> Result<CmmnCaseInstance, CmmnError> {
    let variables = request.variables.as_object().cloned().ok_or_else(|| {
        CmmnError::execution("CMMN case instance variables must be a JSON object")
    })?;
    // Java CaseInstanceBuilder.transientVariables (CaseInstanceHelperImpl.java:275):
    // merged into the variable scope during start, never persisted.
    let transient_variables = request
        .transient_variables
        .as_object()
        .cloned()
        .unwrap_or_default();
    let mut activation_variables = variables.clone();
    for (name, value) in &transient_variables {
        activation_variables.insert(name.clone(), value.clone());
    }

    let case_instance = CmmnCaseInstance {
        id: request
            .predefined_case_instance_id
            .unwrap_or_else(|| format!("cmmn-case-instance:{}", Uuid::new_v4())),
        case_definition_id: case_definition.id.clone(),
        deployment_id: case_definition.deployment_id.clone(),
        case_definition_key: case_definition.key.clone(),
        case_definition_name: case_definition.name.clone(),
        case_definition_version: case_definition.version,
        business_key: request.business_key,
        name: request
            .name
            .unwrap_or_else(|| format!("{} instance", case_definition.name)),
        // Java overrideCaseDefinitionTenantId overrides the case instance tenant
        // (CaseInstanceHelperImpl.java:325-326); the definition lookup still uses
        // `tenant_id` (CaseInstanceHelperImpl.java:122-171).
        tenant_id: request
            .override_definition_tenant_id
            .or(request.tenant_id)
            .or_else(|| case_definition.tenant_id.clone()),
        started_by: request.started_by,
        // Java persists builder reference metadata on the case instance
        // (CaseInstanceBuilderImpl.java:45-46,191-198).
        reference_id: request.reference_id,
        reference_type: request.reference_type,
        started_at: Utc::now(),
        ended_at: None,
        state: CmmnCaseInstanceState::Active,
        business_status: None,
        variables,
        case_file_items: Vec::new(),
        // Java DefaultCaseInstanceService.java:74-75 — BPMN caseServiceTask parent link.
        callback_id: request.callback_id,
        callback_type: request.callback_type,
    };

    persist_case_instance_session(session, &case_instance)?;
    persist_historic_case_session(session, &CmmnHistoricCaseInstance::from(&case_instance))?;

    // Activation sees the merged scope (transient variables visible to
    // expression resolution) while the persisted instance keeps only the
    // real variables. `outcome` is accepted and dropped (no form engine).
    let activation_instance = CmmnCaseInstance {
        variables: activation_variables,
        ..case_instance.clone()
    };
    activate_container(
        session,
        case_definition,
        &activation_instance,
        ContainerView::from_case_plan_model(&case_definition.model.case_plan_model),
        None,
        process_task_runner,
    )?;

    maybe_complete_case(session, &case_instance.id)?;
    load_case_instance_session(session, &case_instance.id)?.ok_or_else(|| {
        CmmnError::storage(format!(
            "CMMN case instance '{}' disappeared during start",
            case_instance.id
        ))
    })
}

pub struct CmmnCaseInstanceQuery {
    store: CmmnStore,
    id: Option<String>,
    ids: Option<Vec<String>>,
    case_definition_id: Option<String>,
    case_definition_key: Option<String>,
    case_definition_key_like: Option<String>,
    case_definition_key_like_ignore_case: Option<String>,
    case_definition_keys: Option<Vec<String>>,
    exclude_case_definition_keys: Option<Vec<String>>,
    case_definition_name: Option<String>,
    case_definition_name_like: Option<String>,
    case_definition_name_like_ignore_case: Option<String>,
    name: Option<String>,
    name_like: Option<String>,
    name_like_ignore_case: Option<String>,
    business_key: Option<String>,
    business_key_like: Option<String>,
    business_key_like_ignore_case: Option<String>,
    business_status: Option<String>,
    business_status_like: Option<String>,
    business_status_like_ignore_case: Option<String>,
    started_by: Option<String>,
    /// Java `caseInstanceReferenceId/Type` exact predicates
    /// (`CaseInstanceQueryImpl.java:654-675`).
    reference_id: Option<String>,
    reference_type: Option<String>,
    started_before: Option<DateTime<Utc>>,
    started_after: Option<DateTime<Utc>>,
    callback_id: Option<String>,
    callback_ids: Option<Vec<String>>,
    callback_type: Option<String>,
    tenant_id: Option<String>,
    tenant_id_like: Option<String>,
    tenant_id_like_ignore_case: Option<String>,
    without_tenant_id: bool,
    state: Option<CmmnCaseInstanceState>,
    /// Java CaseInstanceQuery variable conditions (BaseCaseInstanceResource.java:204-206,
    /// :292-376). AND-ed; filtered in-memory via `variables_match_conditions`.
    variable_conditions: Vec<crate::QueryVariableCondition>,
    start: usize,
    size: Option<usize>,
}

impl CmmnCaseInstanceQuery {
    fn new(store: CmmnStore) -> Self {
        Self {
            store,
            id: None,
            ids: None,
            case_definition_id: None,
            case_definition_key: None,
            case_definition_key_like: None,
            case_definition_key_like_ignore_case: None,
            case_definition_keys: None,
            exclude_case_definition_keys: None,
            case_definition_name: None,
            case_definition_name_like: None,
            case_definition_name_like_ignore_case: None,
            name: None,
            name_like: None,
            name_like_ignore_case: None,
            business_key: None,
            business_key_like: None,
            business_key_like_ignore_case: None,
            business_status: None,
            business_status_like: None,
            business_status_like_ignore_case: None,
            started_by: None,
            reference_id: None,
            reference_type: None,
            started_before: None,
            started_after: None,
            callback_id: None,
            callback_ids: None,
            callback_type: None,
            tenant_id: None,
            tenant_id_like: None,
            tenant_id_like_ignore_case: None,
            without_tenant_id: false,
            state: None,
            variable_conditions: Vec::new(),
            start: 0,
            size: None,
        }
    }

    pub fn id(mut self, id: impl Into<String>) -> Self {
        self.id = Some(id.into());
        self
    }

    pub fn ids(mut self, ids: Vec<String>) -> Self {
        self.ids = Some(ids);
        self
    }

    pub fn case_definition_id(mut self, case_definition_id: impl Into<String>) -> Self {
        self.case_definition_id = Some(case_definition_id.into());
        self
    }

    pub fn case_definition_key(mut self, case_definition_key: impl Into<String>) -> Self {
        self.case_definition_key = Some(case_definition_key.into());
        self
    }

    pub fn case_definition_key_like(mut self, pattern: impl Into<String>) -> Self {
        self.case_definition_key_like = Some(pattern.into());
        self
    }

    pub fn case_definition_key_like_ignore_case(mut self, pattern: impl Into<String>) -> Self {
        self.case_definition_key_like_ignore_case = Some(pattern.into());
        self
    }

    pub fn case_definition_keys(mut self, keys: Vec<String>) -> Self {
        self.case_definition_keys = Some(keys);
        self
    }

    pub fn exclude_case_definition_keys(mut self, keys: Vec<String>) -> Self {
        self.exclude_case_definition_keys = Some(keys);
        self
    }

    pub fn case_definition_name(mut self, name: impl Into<String>) -> Self {
        self.case_definition_name = Some(name.into());
        self
    }

    pub fn case_definition_name_like(mut self, pattern: impl Into<String>) -> Self {
        self.case_definition_name_like = Some(pattern.into());
        self
    }

    pub fn case_definition_name_like_ignore_case(mut self, pattern: impl Into<String>) -> Self {
        self.case_definition_name_like_ignore_case = Some(pattern.into());
        self
    }

    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    pub fn name_like(mut self, pattern: impl Into<String>) -> Self {
        self.name_like = Some(pattern.into());
        self
    }

    pub fn name_like_ignore_case(mut self, pattern: impl Into<String>) -> Self {
        self.name_like_ignore_case = Some(pattern.into());
        self
    }

    pub fn business_key(mut self, business_key: impl Into<String>) -> Self {
        self.business_key = Some(business_key.into());
        self
    }

    pub fn business_key_like(mut self, pattern: impl Into<String>) -> Self {
        self.business_key_like = Some(pattern.into());
        self
    }

    pub fn business_key_like_ignore_case(mut self, pattern: impl Into<String>) -> Self {
        self.business_key_like_ignore_case = Some(pattern.into());
        self
    }

    pub fn business_status(mut self, business_status: impl Into<String>) -> Self {
        self.business_status = Some(business_status.into());
        self
    }

    pub fn business_status_like(mut self, pattern: impl Into<String>) -> Self {
        self.business_status_like = Some(pattern.into());
        self
    }

    pub fn business_status_like_ignore_case(mut self, pattern: impl Into<String>) -> Self {
        self.business_status_like_ignore_case = Some(pattern.into());
        self
    }

    pub fn started_by(mut self, started_by: impl Into<String>) -> Self {
        self.started_by = Some(started_by.into());
        self
    }

    /// Java `CaseInstanceQueryImpl.caseInstanceReferenceId`
    /// (`CaseInstanceQueryImpl.java:654-664`). Rust's typed argument cannot be null.
    pub fn reference_id(mut self, reference_id: impl Into<String>) -> Self {
        self.reference_id = Some(reference_id.into());
        self
    }

    /// Java `CaseInstanceQueryImpl.caseInstanceReferenceType`
    /// (`CaseInstanceQueryImpl.java:666-675`). Rust's typed argument cannot be null.
    pub fn reference_type(mut self, reference_type: impl Into<String>) -> Self {
        self.reference_type = Some(reference_type.into());
        self
    }

    pub fn started_before(mut self, started_before: DateTime<Utc>) -> Self {
        self.started_before = Some(started_before);
        self
    }

    pub fn started_after(mut self, started_after: DateTime<Utc>) -> Self {
        self.started_after = Some(started_after);
        self
    }

    pub fn callback_id(mut self, callback_id: impl Into<String>) -> Self {
        self.callback_id = Some(callback_id.into());
        self
    }

    pub fn callback_ids(mut self, callback_ids: Vec<String>) -> Self {
        self.callback_ids = Some(callback_ids);
        self
    }

    pub fn callback_type(mut self, callback_type: impl Into<String>) -> Self {
        self.callback_type = Some(callback_type.into());
        self
    }

    pub fn tenant_id(mut self, tenant_id: impl Into<String>) -> Self {
        self.tenant_id = Some(tenant_id.into());
        self
    }

    pub fn tenant_id_like(mut self, pattern: impl Into<String>) -> Self {
        self.tenant_id_like = Some(pattern.into());
        self
    }

    pub fn tenant_id_like_ignore_case(mut self, pattern: impl Into<String>) -> Self {
        self.tenant_id_like_ignore_case = Some(pattern.into());
        self
    }

    pub fn without_tenant_id(mut self) -> Self {
        self.without_tenant_id = true;
        self
    }

    pub fn state(mut self, state: CmmnCaseInstanceState) -> Self {
        self.state = Some(state);
        self
    }

    /// Java `variableValue*` family (BaseCaseInstanceResource.java:292-376).
    /// Multiple conditions are AND-ed.
    pub fn variable_conditions(mut self, conditions: Vec<crate::QueryVariableCondition>) -> Self {
        self.variable_conditions = conditions;
        self
    }

    pub fn page(mut self, start: usize, size: usize) -> Self {
        self.start = start;
        self.size = Some(size);
        self
    }

    pub fn list(&self) -> Result<Vec<CmmnCaseInstance>, CmmnError> {
        let mut session = self.store.create_session()?;
        let sql = String::from(
            "SELECT DATA_ FROM ACT_CMMN_CASE_INSTANCE ORDER BY STARTED_AT_ ASC, ID_ ASC",
        );
        let rendered = RenderedStatement::new(sql, DbParams::new());
        let rows = session.select_raw(rendered)?;
        let mut items = rows
            .into_iter()
            .map(|row| {
                let json = row.get_text("DATA_").ok_or_else(|| {
                    CmmnError::storage("Missing DATA_ in CMMN case instance query result")
                })?;
                serde_json::from_str::<CmmnCaseInstance>(&json).map_err(CmmnError::from)
            })
            .collect::<Result<Vec<_>, _>>()?;

        items.retain(|item| matches_optional(&self.id, &item.id));
        items.retain(|item| self.ids.as_ref().is_none_or(|ids| ids.contains(&item.id)));
        items.retain(|item| matches_optional(&self.case_definition_id, &item.case_definition_id));
        items.retain(|item| matches_optional(&self.case_definition_key, &item.case_definition_key));
        items.retain(|item| {
            like_optional(&self.case_definition_key_like, &item.case_definition_key)
        });
        items.retain(|item| {
            like_optional_ignore_case(
                &self.case_definition_key_like_ignore_case,
                &item.case_definition_key,
            )
        });
        items.retain(|item| {
            self.case_definition_keys
                .as_ref()
                .is_none_or(|keys| keys.contains(&item.case_definition_key))
        });
        items.retain(|item| {
            self.exclude_case_definition_keys
                .as_ref()
                .is_none_or(|keys| !keys.contains(&item.case_definition_key))
        });
        items.retain(|item| {
            matches_optional(&self.case_definition_name, &item.case_definition_name)
        });
        items.retain(|item| {
            like_optional(&self.case_definition_name_like, &item.case_definition_name)
        });
        items.retain(|item| {
            like_optional_ignore_case(
                &self.case_definition_name_like_ignore_case,
                &item.case_definition_name,
            )
        });
        items.retain(|item| matches_optional(&self.name, &item.name));
        items.retain(|item| like_optional(&self.name_like, &item.name));
        items.retain(|item| like_optional_ignore_case(&self.name_like_ignore_case, &item.name));
        items.retain(|item| {
            matches_optional_option(&self.business_key, item.business_key.as_deref())
        });
        items.retain(|item| {
            like_optional_option(&self.business_key_like, item.business_key.as_deref())
        });
        items.retain(|item| {
            like_optional_option_ignore_case(
                &self.business_key_like_ignore_case,
                item.business_key.as_deref(),
            )
        });
        items.retain(|item| {
            matches_optional_option(&self.business_status, item.business_status.as_deref())
        });
        items.retain(|item| {
            like_optional_option(&self.business_status_like, item.business_status.as_deref())
        });
        items.retain(|item| {
            like_optional_option_ignore_case(
                &self.business_status_like_ignore_case,
                item.business_status.as_deref(),
            )
        });
        items.retain(|item| matches_optional_option(&self.started_by, item.started_by.as_deref()));
        // Java runtime mapper predicates are exact equality for REFERENCE_ID_/TYPE_
        // (CaseInstanceQueryImpl.java:654-675).
        items.retain(|item| {
            matches_optional_option(&self.reference_id, item.reference_id.as_deref())
        });
        items.retain(|item| {
            matches_optional_option(&self.reference_type, item.reference_type.as_deref())
        });
        items.retain(|item| {
            self.started_before
                .is_none_or(|before| item.started_at < before)
        });
        items.retain(|item| {
            self.started_after
                .is_none_or(|after| item.started_at > after)
        });
        items
            .retain(|item| matches_optional_option(&self.callback_id, item.callback_id.as_deref()));
        items.retain(|item| {
            self.callback_ids.as_ref().is_none_or(|ids| {
                item.callback_id
                    .as_ref()
                    .is_some_and(|callback_id| ids.contains(callback_id))
            })
        });
        items.retain(|item| {
            matches_optional_option(&self.callback_type, item.callback_type.as_deref())
        });
        items.retain(|item| matches_optional_option(&self.tenant_id, item.tenant_id.as_deref()));
        items.retain(|item| like_optional_option(&self.tenant_id_like, item.tenant_id.as_deref()));
        items.retain(|item| {
            like_optional_option_ignore_case(
                &self.tenant_id_like_ignore_case,
                item.tenant_id.as_deref(),
            )
        });
        if self.without_tenant_id {
            items.retain(|item| item.tenant_id.is_none());
        }
        items.retain(|item| self.state.as_ref().is_none_or(|value| item.state == *value));
        // Java BaseCaseInstanceResource.java:204-206 + addVariables (:292-376).
        if !self.variable_conditions.is_empty() {
            items.retain(|item| {
                crate::variables_match_conditions(&item.variables, &self.variable_conditions)
            });
        }

        Ok(items)
    }

    pub fn single_result(&self) -> Result<Option<CmmnCaseInstance>, CmmnError> {
        Ok(self.list()?.into_iter().next())
    }

    pub fn list_page(&self) -> Result<PagedResult<CmmnCaseInstance>, CmmnError> {
        Ok(page_items(self.list()?, self.start, self.size))
    }

    /// Count matching case instances (Java `CaseInstanceQuery.count`).
    pub fn count(&self) -> Result<i64, CmmnError> {
        Ok(self.list()?.len() as i64)
    }
}

/// Resolves the group ids a user belongs to, for candidateUser /
/// candidateOrAssigned group expansion. Java: TaskQueryImpl.getGroupsForCandidateUser
/// (TaskQueryImpl.java:2021-2032) via IdmIdentityService GroupQuery.groupMember —
/// groups of the user are ACT_ID_GROUP rows with a membership in ACT_ID_MEMBERSHIP.
/// The CMMN engine has no identity store, so callers (the REST layer or tests)
/// supply the resolver; when absent, candidateUser matches direct user links only.
pub type CmmnUserGroupResolver = std::sync::Arc<dyn Fn(&str) -> Vec<String> + Send + Sync>;

pub struct CmmnHumanTaskQuery {
    store: CmmnStore,
    id: Option<String>,
    case_instance_id: Option<String>,
    case_definition_id: Option<String>,
    case_definition_key: Option<String>,
    case_definition_key_like: Option<String>,
    case_definition_key_like_ignore_case: Option<String>,
    stage_instance_id: Option<String>,
    state: Option<CmmnHumanTaskState>,
    // Java TaskQuery filters (TaskCollectionResource.java + TaskBaseResource.java)
    name: Option<String>,
    name_like: Option<String>,
    name_like_ignore_case: Option<String>,
    assignee: Option<String>,
    assignee_like: Option<String>,
    owner: Option<String>,
    owner_like: Option<String>,
    unassigned: bool,
    delegation_state: Option<CmmnDelegationState>,
    category: Option<String>,
    category_in: Option<Vec<String>>,
    category_not_in: Option<Vec<String>>,
    without_category: bool,
    task_definition_id: Option<String>,
    task_definition_id_like: Option<String>,
    // Java TaskEntity.priority is an int; Rust stores the resolved literal string,
    // so the numeric filters parse the stored value (see `priority_matches`).
    priority: Option<i64>,
    min_priority: Option<i64>,
    max_priority: Option<i64>,
    created_on: Option<DateTime<Utc>>,
    created_before: Option<DateTime<Utc>>,
    created_after: Option<DateTime<Utc>>,
    // Java stores dueDate as a Date; Rust keeps the resolved string, parsed here
    // for the date comparisons (see `parse_cmmn_datetime`).
    due_date: Option<DateTime<Utc>>,
    due_before: Option<DateTime<Utc>>,
    due_after: Option<DateTime<Utc>>,
    without_due_date: bool,
    // Java TaskQuery.active()/suspended() filters on the task suspension state.
    // The Rust CMMN engine never suspends cases/tasks, so Active retains all and
    // Suspended retains none (documented deviation — see P100 acceptance).
    suspension_state: Option<TaskSuspensionState>,
    // P101 plan-item query surface (PlanItemInstanceCollectionResource /
    // PlanItemInstanceBaseResource).
    case_instance_ids: Option<Vec<String>>,
    /// Java `elementId` (PlanItemInstanceEntityManagerImpl.java:92) is the plan
    /// item id; the Rust task entity's `plan_item_id` is that same id.
    element_id: Option<String>,
    /// Java `planItemDefinitionType`. Only human-task plan items enter the Rust
    /// task query, so any type other than `humantask` matches nothing.
    plan_item_definition_type: Option<String>,
    plan_item_definition_types: Option<Vec<String>>,
    // P114 candidate filters (Java TaskQueryImpl candidate setters + Task.xml
    // candidate blocks). The CMMN engine has no identity store, so the user→groups
    // expansion for candidateUser/candidateOrAssigned comes from
    // `user_group_resolver`; without one only direct user links match.
    candidate_user: Option<String>,
    candidate_group: Option<String>,
    candidate_group_in: Option<Vec<String>>,
    candidate_or_assigned: Option<String>,
    /// Java `ignoreAssigneeValue` (TaskQueryImpl.java:680-687): when true,
    /// candidate filters keep assigned tasks. Default false — candidate queries
    /// exclude already-assigned tasks (Task.xml:868-870).
    ignore_assignee: bool,
    user_group_resolver: Option<CmmnUserGroupResolver>,
    start: usize,
    size: Option<usize>,
}

/// Java `SuspensionState` (TaskQueryImpl.java:1942-1958): `active()` selects
/// non-suspended tasks, `suspended()` selects suspended ones.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskSuspensionState {
    Active,
    Suspended,
}

impl CmmnHumanTaskQuery {
    fn new(store: CmmnStore) -> Self {
        Self {
            store,
            id: None,
            case_instance_id: None,
            case_definition_id: None,
            case_definition_key: None,
            case_definition_key_like: None,
            case_definition_key_like_ignore_case: None,
            stage_instance_id: None,
            state: None,
            name: None,
            name_like: None,
            name_like_ignore_case: None,
            assignee: None,
            assignee_like: None,
            owner: None,
            owner_like: None,
            unassigned: false,
            delegation_state: None,
            category: None,
            category_in: None,
            category_not_in: None,
            without_category: false,
            task_definition_id: None,
            task_definition_id_like: None,
            priority: None,
            min_priority: None,
            max_priority: None,
            created_on: None,
            created_before: None,
            created_after: None,
            due_date: None,
            due_before: None,
            due_after: None,
            without_due_date: false,
            suspension_state: None,
            case_instance_ids: None,
            element_id: None,
            plan_item_definition_type: None,
            plan_item_definition_types: None,
            candidate_user: None,
            candidate_group: None,
            candidate_group_in: None,
            candidate_or_assigned: None,
            ignore_assignee: false,
            user_group_resolver: None,
            start: 0,
            size: None,
        }
    }

    pub fn id(mut self, id: impl Into<String>) -> Self {
        self.id = Some(id.into());
        self
    }

    pub fn case_instance_id(mut self, case_instance_id: impl Into<String>) -> Self {
        self.case_instance_id = Some(case_instance_id.into());
        self
    }

    pub fn case_definition_id(mut self, case_definition_id: impl Into<String>) -> Self {
        self.case_definition_id = Some(case_definition_id.into());
        self
    }

    pub fn case_definition_key(mut self, case_definition_key: impl Into<String>) -> Self {
        self.case_definition_key = Some(case_definition_key.into());
        self
    }

    pub fn case_definition_key_like(mut self, pattern: impl Into<String>) -> Self {
        self.case_definition_key_like = Some(pattern.into());
        self
    }

    pub fn case_definition_key_like_ignore_case(mut self, pattern: impl Into<String>) -> Self {
        self.case_definition_key_like_ignore_case = Some(pattern.into());
        self
    }

    pub fn stage_instance_id(mut self, stage_instance_id: impl Into<String>) -> Self {
        self.stage_instance_id = Some(stage_instance_id.into());
        self
    }

    pub fn state(mut self, state: CmmnHumanTaskState) -> Self {
        self.state = Some(state);
        self
    }

    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    pub fn name_like(mut self, pattern: impl Into<String>) -> Self {
        self.name_like = Some(pattern.into());
        self
    }

    pub fn name_like_ignore_case(mut self, pattern: impl Into<String>) -> Self {
        self.name_like_ignore_case = Some(pattern.into());
        self
    }

    pub fn assignee(mut self, assignee: impl Into<String>) -> Self {
        self.assignee = Some(assignee.into());
        self
    }

    pub fn assignee_like(mut self, pattern: impl Into<String>) -> Self {
        self.assignee_like = Some(pattern.into());
        self
    }

    pub fn owner(mut self, owner: impl Into<String>) -> Self {
        self.owner = Some(owner.into());
        self
    }

    pub fn owner_like(mut self, pattern: impl Into<String>) -> Self {
        self.owner_like = Some(pattern.into());
        self
    }

    /// Java `taskUnassigned()`: only tasks with no assignee. The REST layer passes
    /// the presence of the `unassigned` query param (Java applies the filter
    /// whenever the param is present, regardless of its boolean value —
    /// TaskBaseResource.java:182-184).
    pub fn unassigned(mut self) -> Self {
        self.unassigned = true;
        self
    }

    pub fn delegation_state(mut self, state: CmmnDelegationState) -> Self {
        self.delegation_state = Some(state);
        self
    }

    pub fn category(mut self, category: impl Into<String>) -> Self {
        self.category = Some(category.into());
        self
    }

    pub fn category_in(mut self, categories: Vec<String>) -> Self {
        self.category_in = Some(categories);
        self
    }

    pub fn category_not_in(mut self, categories: Vec<String>) -> Self {
        self.category_not_in = Some(categories);
        self
    }

    pub fn without_category(mut self) -> Self {
        self.without_category = true;
        self
    }

    /// Java `taskDefinitionKey` → the stored `task_definition_id`.
    pub fn task_definition_id(mut self, task_definition_id: impl Into<String>) -> Self {
        self.task_definition_id = Some(task_definition_id.into());
        self
    }

    pub fn task_definition_id_like(mut self, pattern: impl Into<String>) -> Self {
        self.task_definition_id_like = Some(pattern.into());
        self
    }

    pub fn priority(mut self, priority: i64) -> Self {
        self.priority = Some(priority);
        self
    }

    pub fn min_priority(mut self, minimum_priority: i64) -> Self {
        self.min_priority = Some(minimum_priority);
        self
    }

    pub fn max_priority(mut self, maximum_priority: i64) -> Self {
        self.max_priority = Some(maximum_priority);
        self
    }

    pub fn created_on(mut self, created_on: DateTime<Utc>) -> Self {
        self.created_on = Some(created_on);
        self
    }

    pub fn created_before(mut self, created_before: DateTime<Utc>) -> Self {
        self.created_before = Some(created_before);
        self
    }

    pub fn created_after(mut self, created_after: DateTime<Utc>) -> Self {
        self.created_after = Some(created_after);
        self
    }

    pub fn due_date(mut self, due_date: DateTime<Utc>) -> Self {
        self.due_date = Some(due_date);
        self
    }

    pub fn due_before(mut self, due_before: DateTime<Utc>) -> Self {
        self.due_before = Some(due_before);
        self
    }

    pub fn due_after(mut self, due_after: DateTime<Utc>) -> Self {
        self.due_after = Some(due_after);
        self
    }

    pub fn without_due_date(mut self) -> Self {
        self.without_due_date = true;
        self
    }

    /// Java `TaskQuery.active()`/`suspended()` (TaskQueryImpl.java:1942-1958).
    pub fn suspension_state(mut self, state: TaskSuspensionState) -> Self {
        self.suspension_state = Some(state);
        self
    }

    pub fn case_instance_ids(mut self, case_instance_ids: Vec<String>) -> Self {
        self.case_instance_ids = Some(case_instance_ids);
        self
    }

    /// Java `planItemInstanceElementId` (PlanItemInstanceBaseResource.java:91-93)
    /// — the plan item id, which is the task entity's `plan_item_id`.
    pub fn element_id(mut self, element_id: impl Into<String>) -> Self {
        self.element_id = Some(element_id.into());
        self
    }

    pub fn plan_item_definition_type(
        mut self,
        plan_item_definition_type: impl Into<String>,
    ) -> Self {
        self.plan_item_definition_type = Some(plan_item_definition_type.into());
        self
    }

    pub fn plan_item_definition_types(mut self, plan_item_definition_types: Vec<String>) -> Self {
        self.plan_item_definition_types = Some(plan_item_definition_types);
        self
    }

    /// Java `taskCandidateUser` (TaskQueryImpl.java:576-588): match tasks with a
    /// `candidate` identity link for this user or for any group the user belongs
    /// to (group expansion via `user_group_resolver`).
    pub fn candidate_user(mut self, candidate_user: impl Into<String>) -> Self {
        self.candidate_user = Some(candidate_user.into());
        self
    }

    /// Java `taskCandidateGroup` (TaskQueryImpl.java:620-635).
    pub fn candidate_group(mut self, candidate_group: impl Into<String>) -> Self {
        self.candidate_group = Some(candidate_group.into());
        self
    }

    /// Java `taskCandidateGroupIn` (TaskQueryImpl.java:658-677).
    pub fn candidate_group_in(mut self, candidate_group_in: Vec<String>) -> Self {
        self.candidate_group_in = Some(candidate_group_in);
        self
    }

    /// Java `taskCandidateOrAssigned` (TaskQueryImpl.java:638-655): task whose
    /// assignee is the user, or a candidate for the user (directly or via any of
    /// the user's groups).
    pub fn candidate_or_assigned(mut self, user_id: impl Into<String>) -> Self {
        self.candidate_or_assigned = Some(user_id.into());
        self
    }

    /// Java `ignoreAssigneeValue` (TaskQueryImpl.java:680-687): keep assigned
    /// tasks in candidate queries (drops the `ASSIGNEE_ is null` gate,
    /// Task.xml:868-870).
    pub fn ignore_assignee_value(mut self) -> Self {
        self.ignore_assignee = true;
        self
    }

    /// Supplies the user→groups expansion for candidateUser / candidateOrAssigned.
    pub fn user_group_resolver(mut self, resolver: CmmnUserGroupResolver) -> Self {
        self.user_group_resolver = Some(resolver);
        self
    }

    pub fn page(mut self, start: usize, size: usize) -> Self {
        self.start = start;
        self.size = Some(size);
        self
    }

    pub fn list(&self) -> Result<Vec<CmmnHumanTaskInstance>, CmmnError> {
        let mut session = self.store.create_session()?;
        let sql = String::from(
            "SELECT DATA_ FROM ACT_CMMN_HUMAN_TASK ORDER BY ACTIVATED_AT_ ASC, ID_ ASC",
        );
        let rendered = RenderedStatement::new(sql, DbParams::new());
        let rows = session.select_raw(rendered)?;
        let mut items = rows
            .into_iter()
            .map(|row| {
                let json = row.get_text("DATA_").ok_or_else(|| {
                    CmmnError::storage("Missing DATA_ in CMMN human task query result")
                })?;
                serde_json::from_str::<CmmnHumanTaskInstance>(&json).map_err(CmmnError::from)
            })
            .collect::<Result<Vec<_>, _>>()?;

        items.retain(|item| matches_optional(&self.id, &item.id));
        items.retain(|item| matches_optional(&self.case_instance_id, &item.case_instance_id));
        items.retain(|item| matches_optional(&self.case_definition_id, &item.case_definition_id));
        items.retain(|item| matches_optional(&self.case_definition_key, &item.case_definition_key));
        items.retain(|item| {
            like_optional(&self.case_definition_key_like, &item.case_definition_key)
        });
        items.retain(|item| {
            like_optional_ignore_case(
                &self.case_definition_key_like_ignore_case,
                &item.case_definition_key,
            )
        });
        items.retain(|item| {
            matches_optional_option(&self.stage_instance_id, item.stage_instance_id.as_deref())
        });
        items.retain(|item| self.state.as_ref().is_none_or(|value| item.state == *value));
        items.retain(|item| matches_optional(&self.name, &item.name));
        items.retain(|item| like_optional(&self.name_like, &item.name));
        items.retain(|item| like_optional_ignore_case(&self.name_like_ignore_case, &item.name));
        items.retain(|item| matches_optional_option(&self.assignee, item.assignee.as_deref()));
        items.retain(|item| like_optional_option(&self.assignee_like, item.assignee.as_deref()));
        items.retain(|item| matches_optional_option(&self.owner, item.owner.as_deref()));
        items.retain(|item| like_optional_option(&self.owner_like, item.owner.as_deref()));
        if self.unassigned {
            items.retain(|item| item.assignee.is_none());
        }
        items.retain(|item| {
            self.delegation_state
                .as_ref()
                .is_none_or(|state| item.delegation_state.as_ref() == Some(state))
        });
        items.retain(|item| matches_optional_option(&self.category, item.category.as_deref()));
        items.retain(|item| {
            self.category_in.as_ref().is_none_or(|categories| {
                item.category
                    .as_ref()
                    .is_some_and(|category| categories.contains(category))
            })
        });
        items.retain(|item| {
            self.category_not_in.as_ref().is_none_or(|categories| {
                item.category
                    .as_ref()
                    .is_none_or(|category| !categories.contains(category))
            })
        });
        if self.without_category {
            items.retain(|item| item.category.is_none());
        }
        items.retain(|item| matches_optional(&self.task_definition_id, &item.task_definition_id));
        items.retain(|item| like_optional(&self.task_definition_id_like, &item.task_definition_id));
        items.retain(|item| {
            self.priority.is_none_or(|priority| {
                parse_priority(&item.priority).is_some_and(|stored| stored == priority)
            })
        });
        items.retain(|item| {
            self.min_priority.is_none_or(|min| {
                parse_priority(&item.priority).is_some_and(|stored| stored >= min)
            })
        });
        items.retain(|item| {
            self.max_priority.is_none_or(|max| {
                parse_priority(&item.priority).is_some_and(|stored| stored <= max)
            })
        });
        items.retain(|item| self.created_on.is_none_or(|on| item.activated_at == on));
        items.retain(|item| {
            self.created_before
                .is_none_or(|before| item.activated_at < before)
        });
        items.retain(|item| {
            self.created_after
                .is_none_or(|after| item.activated_at > after)
        });
        items.retain(|item| {
            self.due_date.is_none_or(|due| {
                item.due_date
                    .as_deref()
                    .and_then(parse_cmmn_datetime)
                    .is_some_and(|stored| stored == due)
            })
        });
        items.retain(|item| {
            self.due_before.is_none_or(|before| {
                item.due_date
                    .as_deref()
                    .and_then(parse_cmmn_datetime)
                    .is_some_and(|stored| stored < before)
            })
        });
        items.retain(|item| {
            self.due_after.is_none_or(|after| {
                item.due_date
                    .as_deref()
                    .and_then(parse_cmmn_datetime)
                    .is_some_and(|stored| stored > after)
            })
        });
        if self.without_due_date {
            items.retain(|item| item.due_date.is_none());
        }
        items.retain(|item| {
            self.case_instance_ids
                .as_ref()
                .is_none_or(|ids| ids.contains(&item.case_instance_id))
        });
        items.retain(|item| matches_optional(&self.element_id, &item.plan_item_id));
        // Java `planItemDefinitionType(s)` (PlanItemInstanceBaseResource.java:82-87).
        // The Rust task query only holds human-task plan items, so a type other
        // than `humantask` matches nothing.
        items.retain(|_item| {
            let type_matches = |value: &str| value.eq_ignore_ascii_case("humantask");
            self.plan_item_definition_type
                .as_deref()
                .is_none_or(type_matches)
                && self
                    .plan_item_definition_types
                    .as_ref()
                    .is_none_or(|types| {
                        types
                            .iter()
                            .any(|value| value.eq_ignore_ascii_case("humantask"))
                    })
        });

        // P114 candidate filters. Java renders each candidate filter as one
        // correlated `exists` over ACT_RU_IDENTITYLINK plus a default
        // `ASSIGNEE_ is null` gate (Task.xml:867-896), and candidateOrAssigned
        // as `ASSIGNEE_ = user or ((unless ignoreAssignee) ASSIGNEE_ is null and
        // exists(...))` (Task.xml:1090-1131). The Rust CMMN human-task candidate
        // links live in ACT_CMMN_IDENTITY_LINK with scope humanTask (C10), so we
        // batch-load them once and evaluate the same semantics in memory. Group
        // expansion uses the injected `user_group_resolver`; without one,
        // candidateUser/candidateOrAssigned match direct user links only.
        let needs_candidate_links = self.candidate_user.is_some()
            || self.candidate_group.is_some()
            || self.candidate_group_in.is_some()
            || self.candidate_or_assigned.is_some();
        if needs_candidate_links {
            let link_rows = session.select_raw(RenderedStatement::new(
                "SELECT DATA_ FROM ACT_CMMN_IDENTITY_LINK".to_string(),
                DbParams::new(),
            ))?;
            let mut links_by_task: std::collections::HashMap<String, Vec<CmmnIdentityLink>> =
                std::collections::HashMap::new();
            for row in link_rows {
                let Some(json) = row.get_text("DATA_") else {
                    continue;
                };
                let Ok(link) = serde_json::from_str::<CmmnIdentityLink>(&json) else {
                    continue;
                };
                if link.scope_type == "humanTask" {
                    links_by_task
                        .entry(link.scope_id.clone())
                        .or_default()
                        .push(link);
                }
            }

            // Java default for the plain candidate block: `ASSIGNEE_ is null`
            // unless ignoreAssigneeValue (Task.xml:868-870). Applied once for the
            // whole block, not per condition.
            let has_plain_candidate = self.candidate_user.is_some()
                || self.candidate_group.is_some()
                || self.candidate_group_in.is_some();
            if has_plain_candidate && !self.ignore_assignee {
                items.retain(|task| task.assignee.is_none());
            }

            if let Some(candidate_user) = &self.candidate_user {
                // Java TaskQueryImpl.getGroupsForCandidateUser
                // (TaskQueryImpl.java:2021-2032): direct user link OR a candidate
                // link on any of the user's groups.
                let user_group_ids: std::collections::HashSet<String> = self
                    .user_group_resolver
                    .as_ref()
                    .map(|resolver| resolver(candidate_user).into_iter().collect())
                    .unwrap_or_default();
                items.retain(|task| {
                    links_by_task.get(task.id.as_str()).is_some_and(|links| {
                        links.iter().any(|link| {
                            link.link_type == "candidate"
                                && (link.user_id.as_deref() == Some(candidate_user.as_str())
                                    || link
                                        .group_id
                                        .as_ref()
                                        .is_some_and(|gid| user_group_ids.contains(gid)))
                        })
                    })
                });
            }

            if let Some(candidate_group) = &self.candidate_group {
                items.retain(|task| {
                    links_by_task.get(task.id.as_str()).is_some_and(|links| {
                        links.iter().any(|link| {
                            link.link_type == "candidate"
                                && link.group_id.as_deref() == Some(candidate_group.as_str())
                        })
                    })
                });
            }

            if let Some(candidate_group_in) = &self.candidate_group_in {
                let groups: std::collections::HashSet<&str> =
                    candidate_group_in.iter().map(String::as_str).collect();
                items.retain(|task| {
                    links_by_task.get(task.id.as_str()).is_some_and(|links| {
                        links.iter().any(|link| {
                            link.link_type == "candidate"
                                && link
                                    .group_id
                                    .as_deref()
                                    .is_some_and(|gid| groups.contains(gid))
                        })
                    })
                });
            }

            if let Some(candidate_or_assigned) = &self.candidate_or_assigned {
                // Java taskCandidateOrAssigned (Task.xml:1090-1131): assignee ==
                // the user, or (unassigned unless ignoreAssigneeValue) a candidate
                // link for the user or any of the user's groups.
                let user_group_ids: std::collections::HashSet<String> = self
                    .user_group_resolver
                    .as_ref()
                    .map(|resolver| resolver(candidate_or_assigned).into_iter().collect())
                    .unwrap_or_default();
                items.retain(|task| {
                    if task.assignee.as_deref() == Some(candidate_or_assigned.as_str()) {
                        return true;
                    }
                    if !self.ignore_assignee && task.assignee.is_some() {
                        return false;
                    }
                    links_by_task.get(task.id.as_str()).is_some_and(|links| {
                        links.iter().any(|link| {
                            link.link_type == "candidate"
                                && (link.user_id.as_deref() == Some(candidate_or_assigned.as_str())
                                    || link
                                        .group_id
                                        .as_ref()
                                        .is_some_and(|gid| user_group_ids.contains(gid)))
                        })
                    })
                });
            }
        }

        // Java `active()`/`suspended()` (TaskQueryImpl.java:1942-1958): the Rust
        // engine never suspends cases or tasks (see P100 acceptance), so Active is
        // always satisfied and Suspended never is.
        items.retain(|_item| match self.suspension_state {
            Some(TaskSuspensionState::Active) => true,
            Some(TaskSuspensionState::Suspended) => false,
            None => true,
        });

        Ok(items)
    }

    pub fn single_result(&self) -> Result<Option<CmmnHumanTaskInstance>, CmmnError> {
        Ok(self.list()?.into_iter().next())
    }

    pub fn list_page(&self) -> Result<PagedResult<CmmnHumanTaskInstance>, CmmnError> {
        Ok(page_items(self.list()?, self.start, self.size))
    }
}

/// P116: unified plan-item-instance query over the `ACT_CMMN_RU_PLAN_ITEM_INST`
/// mirror (stage / milestone / event listener). Human-task plan items stay backed
/// by ACT_CMMN_HUMAN_TASK (`CmmnHumanTaskQuery`) — the REST layer merges both
/// sources. Java reference: `PlanItemInstanceQueryImpl` (filter methods at
/// PlanItemInstanceQueryImpl.java:118-834) and `PlanItemInstanceBaseResource.java:59-139`.
pub struct CmmnPlanItemInstanceQuery {
    store: CmmnStore,
    id: Option<String>,
    case_instance_id: Option<String>,
    case_instance_ids: Option<Vec<String>>,
    case_definition_id: Option<String>,
    stage_instance_id: Option<String>,
    plan_item_definition_id: Option<String>,
    /// Java `planItemDefinitionType` — matched case-insensitively against the
    /// stored lowercase type (`stage` / `milestone` / `eventlistener`).
    plan_item_definition_type: Option<String>,
    plan_item_definition_types: Option<Vec<String>>,
    /// Java `elementId` (planItemInstanceElementId, PlanItemInstanceBaseResource.java:91-93).
    element_id: Option<String>,
    name: Option<String>,
    name_like: Option<String>,
    name_like_ignore_case: Option<String>,
    state: Option<String>,
    include_ended: bool,
    start: usize,
    size: Option<usize>,
}

impl CmmnPlanItemInstanceQuery {
    fn new(store: CmmnStore) -> Self {
        Self {
            store,
            id: None,
            case_instance_id: None,
            case_instance_ids: None,
            case_definition_id: None,
            stage_instance_id: None,
            plan_item_definition_id: None,
            plan_item_definition_type: None,
            plan_item_definition_types: None,
            element_id: None,
            name: None,
            name_like: None,
            name_like_ignore_case: None,
            state: None,
            include_ended: false,
            start: 0,
            size: None,
        }
    }

    pub fn id(mut self, id: impl Into<String>) -> Self {
        self.id = Some(id.into());
        self
    }

    pub fn case_instance_id(mut self, case_instance_id: impl Into<String>) -> Self {
        self.case_instance_id = Some(case_instance_id.into());
        self
    }

    pub fn case_instance_ids(mut self, case_instance_ids: Vec<String>) -> Self {
        self.case_instance_ids = Some(case_instance_ids);
        self
    }

    pub fn case_definition_id(mut self, case_definition_id: impl Into<String>) -> Self {
        self.case_definition_id = Some(case_definition_id.into());
        self
    }

    pub fn stage_instance_id(mut self, stage_instance_id: impl Into<String>) -> Self {
        self.stage_instance_id = Some(stage_instance_id.into());
        self
    }

    pub fn plan_item_definition_id(mut self, plan_item_definition_id: impl Into<String>) -> Self {
        self.plan_item_definition_id = Some(plan_item_definition_id.into());
        self
    }

    pub fn plan_item_definition_type(
        mut self,
        plan_item_definition_type: impl Into<String>,
    ) -> Self {
        self.plan_item_definition_type = Some(plan_item_definition_type.into());
        self
    }

    pub fn plan_item_definition_types(mut self, plan_item_definition_types: Vec<String>) -> Self {
        self.plan_item_definition_types = Some(plan_item_definition_types);
        self
    }

    pub fn element_id(mut self, element_id: impl Into<String>) -> Self {
        self.element_id = Some(element_id.into());
        self
    }

    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    pub fn name_like(mut self, name_like: impl Into<String>) -> Self {
        self.name_like = Some(name_like.into());
        self
    }

    pub fn name_like_ignore_case(mut self, name_like_ignore_case: impl Into<String>) -> Self {
        self.name_like_ignore_case = Some(name_like_ignore_case.into());
        self
    }

    pub fn state(mut self, state: impl Into<String>) -> Self {
        self.state = Some(state.into());
        self
    }

    /// Historic-query bridge for the lightweight mirror-backed history.
    /// Java runtime queries only see ACT_CMMN_RU_PLAN_ITEM_INST, whose terminal
    /// rows are deleted; Rust retains those rows in the mirror and hides them by
    /// default, exposing them only to the historic adapter.
    pub fn include_ended(mut self) -> Self {
        self.include_ended = true;
        self
    }

    pub fn page(mut self, start: usize, size: usize) -> Self {
        self.start = start;
        self.size = Some(size);
        self
    }

    pub fn list(&self) -> Result<Vec<CmmnPlanItemInstance>, CmmnError> {
        let mut session = self.store.create_session()?;
        let mut items = load_plan_item_instances_session(&mut session)?;

        if !self.include_ended {
            items.retain(|item| item.ended_at.is_none());
        }

        items.retain(|item| matches_optional(&self.id, &item.id));
        items.retain(|item| matches_optional(&self.case_instance_id, &item.case_instance_id));
        items.retain(|item| {
            self.case_instance_ids
                .as_ref()
                .is_none_or(|ids| ids.contains(&item.case_instance_id))
        });
        items.retain(|item| matches_optional(&self.case_definition_id, &item.case_definition_id));
        items.retain(|item| {
            matches_optional_option(&self.stage_instance_id, item.stage_instance_id.as_deref())
        });
        items.retain(|item| {
            matches_optional(&self.plan_item_definition_id, &item.plan_item_definition_id)
        });
        // Java `planItemDefinitionType(s)` (PlanItemInstanceBaseResource.java:82-87):
        // case-insensitive against the stored lowercase type.
        items.retain(|item| {
            self.plan_item_definition_type
                .as_deref()
                .is_none_or(|value| item.plan_item_definition_type.eq_ignore_ascii_case(value))
                && self
                    .plan_item_definition_types
                    .as_ref()
                    .is_none_or(|types| {
                        types
                            .iter()
                            .any(|value| item.plan_item_definition_type.eq_ignore_ascii_case(value))
                    })
        });
        items.retain(|item| matches_optional(&self.element_id, &item.plan_item_id));
        items.retain(|item| matches_optional(&self.name, &item.name));
        items.retain(|item| like_optional(&self.name_like, &item.name));
        items.retain(|item| like_optional_ignore_case(&self.name_like_ignore_case, &item.name));
        items.retain(|item| matches_optional(&self.state, &item.state));

        Ok(items)
    }

    pub fn single_result(&self) -> Result<Option<CmmnPlanItemInstance>, CmmnError> {
        Ok(self.list()?.into_iter().next())
    }

    pub fn list_page(&self) -> Result<PagedResult<CmmnPlanItemInstance>, CmmnError> {
        Ok(page_items(self.list()?, self.start, self.size))
    }
}

pub struct CmmnEventSubscriptionQuery {
    store: CmmnStore,
    id: Option<String>,
    event_type: Option<String>,
    event_name: Option<String>,
    activity_id: Option<String>,
    case_instance_id: Option<String>,
    case_definition_id: Option<String>,
    plan_item_instance_id: Option<String>,
    tenant_id: Option<String>,
    configuration: Option<String>,
    without_scope_id: bool,
    without_scope_definition_id: bool,
    without_tenant_id: bool,
    without_configuration: bool,
    /// P133: filter on CmmnEventSubscription.created_at
    created_after: Option<DateTime<Utc>>,
    created_before: Option<DateTime<Utc>>,
    start: usize,
    size: Option<usize>,
}

impl CmmnEventSubscriptionQuery {
    fn new(store: CmmnStore) -> Self {
        Self {
            store,
            id: None,
            event_type: None,
            event_name: None,
            activity_id: None,
            case_instance_id: None,
            case_definition_id: None,
            plan_item_instance_id: None,
            tenant_id: None,
            configuration: None,
            without_scope_id: false,
            without_scope_definition_id: false,
            without_tenant_id: false,
            without_configuration: false,
            created_after: None,
            created_before: None,
            start: 0,
            size: None,
        }
    }

    pub fn id(mut self, id: impl Into<String>) -> Self {
        self.id = Some(id.into());
        self
    }

    pub fn event_type(mut self, event_type: impl Into<String>) -> Self {
        self.event_type = Some(event_type.into());
        self
    }

    pub fn event_name(mut self, event_name: impl Into<String>) -> Self {
        self.event_name = Some(event_name.into());
        self
    }

    pub fn activity_id(mut self, activity_id: impl Into<String>) -> Self {
        self.activity_id = Some(activity_id.into());
        self
    }

    pub fn case_instance_id(mut self, case_instance_id: impl Into<String>) -> Self {
        self.case_instance_id = Some(case_instance_id.into());
        self
    }

    pub fn case_definition_id(mut self, case_definition_id: impl Into<String>) -> Self {
        self.case_definition_id = Some(case_definition_id.into());
        self
    }

    pub fn plan_item_instance_id(mut self, plan_item_instance_id: impl Into<String>) -> Self {
        self.plan_item_instance_id = Some(plan_item_instance_id.into());
        self
    }

    pub fn tenant_id(mut self, tenant_id: impl Into<String>) -> Self {
        self.tenant_id = Some(tenant_id.into());
        self
    }

    pub fn configuration(mut self, configuration: impl Into<String>) -> Self {
        self.configuration = Some(configuration.into());
        self
    }

    pub fn without_scope_id(mut self) -> Self {
        self.without_scope_id = true;
        self
    }

    pub fn without_scope_definition_id(mut self) -> Self {
        self.without_scope_definition_id = true;
        self
    }

    pub fn without_tenant_id(mut self) -> Self {
        self.without_tenant_id = true;
        self
    }

    pub fn without_configuration(mut self) -> Self {
        self.without_configuration = true;
        self
    }

    /// P133: only subscriptions with created_at after the given timestamp.
    pub fn created_after(mut self, created_after: DateTime<Utc>) -> Self {
        self.created_after = Some(created_after);
        self
    }

    /// P133: only subscriptions with created_at before the given timestamp.
    pub fn created_before(mut self, created_before: DateTime<Utc>) -> Self {
        self.created_before = Some(created_before);
        self
    }

    pub fn page(mut self, start: usize, size: usize) -> Self {
        self.start = start;
        self.size = Some(size);
        self
    }

    pub fn list(&self) -> Result<Vec<CmmnEventSubscription>, CmmnError> {
        let mut session = self.store.create_session()?;
        let sql = String::from(
            "SELECT DATA_ FROM ACT_CMMN_EVENT_SUBSCRIPTION ORDER BY CREATED_AT_ ASC, ID_ ASC",
        );
        let rendered = RenderedStatement::new(sql, DbParams::new());
        let rows = session.select_raw(rendered)?;
        let mut items = rows
            .into_iter()
            .map(|row| {
                let json = row.get_text("DATA_").ok_or_else(|| {
                    CmmnError::storage("Missing DATA_ in CMMN event subscription query result")
                })?;
                serde_json::from_str::<CmmnEventSubscription>(&json).map_err(CmmnError::from)
            })
            .collect::<Result<Vec<_>, _>>()?;

        items.retain(|item| matches_optional(&self.id, &item.id));
        items.retain(|item| matches_optional(&self.event_type, &item.event_type));
        items.retain(|item| matches_optional_option(&self.event_name, item.event_name.as_deref()));
        items
            .retain(|item| matches_optional_option(&self.activity_id, item.activity_id.as_deref()));
        items.retain(|item| {
            matches_optional_option(&self.case_instance_id, item.case_instance_id.as_deref())
        });
        items.retain(|item| {
            matches_optional_option(&self.case_definition_id, item.case_definition_id.as_deref())
        });
        items.retain(|item| {
            matches_optional_option(
                &self.plan_item_instance_id,
                item.plan_item_instance_id.as_deref(),
            )
        });
        items.retain(|item| matches_optional_option(&self.tenant_id, item.tenant_id.as_deref()));
        items.retain(|item| {
            matches_optional_option(&self.configuration, item.configuration.as_deref())
        });
        if self.without_scope_id {
            items.retain(|item| item.case_instance_id.is_none());
        }
        if self.without_scope_definition_id {
            items.retain(|item| item.case_definition_id.is_none());
        }
        if self.without_tenant_id {
            items.retain(|item| item.tenant_id.is_none());
        }
        if self.without_configuration {
            items.retain(|item| item.configuration.is_none());
        }
        // P133: created_at range filters
        if let Some(after) = self.created_after {
            items.retain(|item| item.created_at > after);
        }
        if let Some(before) = self.created_before {
            items.retain(|item| item.created_at < before);
        }

        Ok(items)
    }

    pub fn single_result(&self) -> Result<Option<CmmnEventSubscription>, CmmnError> {
        Ok(self.list()?.into_iter().next())
    }

    pub fn list_page(&self) -> Result<PagedResult<CmmnEventSubscription>, CmmnError> {
        Ok(page_items(self.list()?, self.start, self.size))
    }
}

pub struct CmmnTaskAssociationQuery {
    store: CmmnStore,
    id: Option<String>,
    case_instance_id: Option<String>,
    child_instance_id: Option<String>,
    kind: Option<CmmnTaskAssociationKind>,
    state: Option<CmmnTaskAssociationState>,
    start: usize,
    size: Option<usize>,
}

impl CmmnTaskAssociationQuery {
    fn new(store: CmmnStore) -> Self {
        Self {
            store,
            id: None,
            case_instance_id: None,
            child_instance_id: None,
            kind: None,
            state: None,
            start: 0,
            size: None,
        }
    }

    pub fn id(mut self, id: impl Into<String>) -> Self {
        self.id = Some(id.into());
        self
    }

    pub fn case_instance_id(mut self, case_instance_id: impl Into<String>) -> Self {
        self.case_instance_id = Some(case_instance_id.into());
        self
    }

    pub fn child_instance_id(mut self, child_instance_id: impl Into<String>) -> Self {
        self.child_instance_id = Some(child_instance_id.into());
        self
    }

    pub fn kind(mut self, kind: CmmnTaskAssociationKind) -> Self {
        self.kind = Some(kind);
        self
    }

    pub fn state(mut self, state: CmmnTaskAssociationState) -> Self {
        self.state = Some(state);
        self
    }

    pub fn page(mut self, start: usize, size: usize) -> Self {
        self.start = start;
        self.size = Some(size);
        self
    }

    pub fn list(&self) -> Result<Vec<CmmnTaskInstanceAssociation>, CmmnError> {
        let mut session = self.store.create_session()?;
        let sql = String::from(
            "SELECT DATA_ FROM ACT_CMMN_TASK_INSTANCE_ASSOCIATION ORDER BY CREATED_AT_ ASC, ID_ ASC",
        );
        let rendered = RenderedStatement::new(sql, DbParams::new());
        let rows = session.select_raw(rendered)?;
        let mut items = rows
            .into_iter()
            .map(|row| {
                let json = row.get_text("DATA_").ok_or_else(|| {
                    CmmnError::storage("Missing DATA_ in CMMN task association query result")
                })?;
                serde_json::from_str::<CmmnTaskInstanceAssociation>(&json).map_err(CmmnError::from)
            })
            .collect::<Result<Vec<_>, _>>()?;

        items.retain(|item| matches_optional(&self.id, &item.id));
        items.retain(|item| matches_optional(&self.case_instance_id, &item.case_instance_id));
        items.retain(|item| {
            matches_optional_option(&self.child_instance_id, item.child_instance_id.as_deref())
        });
        items.retain(|item| self.kind.as_ref().is_none_or(|kind| item.kind == *kind));
        items.retain(|item| self.state.as_ref().is_none_or(|state| item.state == *state));

        Ok(items)
    }

    pub fn single_result(&self) -> Result<Option<CmmnTaskInstanceAssociation>, CmmnError> {
        Ok(self.list()?.into_iter().next())
    }

    pub fn list_page(&self) -> Result<PagedResult<CmmnTaskInstanceAssociation>, CmmnError> {
        Ok(page_items(self.list()?, self.start, self.size))
    }
}

fn activate_container(
    session: &mut DbSession,
    case_definition: &CmmnCaseDefinition,
    case_instance: &CmmnCaseInstance,
    container: ContainerView<'_>,
    parent_stage_instance_id: Option<&str>,
    process_task_runner: Option<&Arc<dyn CmmnProcessTaskRunner>>,
) -> Result<(), CmmnError> {
    // Java creates every plan-item instance while activating its container
    // (`CmmnOperation.java:117-210`). Milestones therefore exist as AVAILABLE
    // before a sentry can make them occur; no-sentry milestones use the same row
    // and immediately transition it to COMPLETED below.
    materialize_available_milestone_instances(
        session,
        case_definition,
        case_instance,
        container,
        parent_stage_instance_id,
    )?;

    for plan_item in container.plan_items {
        if !plan_item.entry_criterion_ids.is_empty() {
            continue;
        }
        if let Some(stage) = container
            .stages
            .iter()
            .find(|candidate| candidate.id == plan_item.definition_ref)
        {
            activate_stage(
                session,
                case_definition,
                case_instance,
                plan_item,
                stage,
                parent_stage_instance_id,
            )?;
            continue;
        }
        if let Some(human_task) = container
            .human_tasks
            .iter()
            .find(|candidate| candidate.id == plan_item.definition_ref)
        {
            activate_human_task(
                session,
                case_definition,
                case_instance,
                plan_item,
                human_task,
                parent_stage_instance_id,
            )?;
            continue;
        }
        if let Some(decision_task) = container
            .decision_tasks
            .iter()
            .find(|candidate| candidate.id == plan_item.definition_ref)
        {
            if manual_activation_rule_matches(plan_item, case_instance)? {
                persist_enabled_plan_item_instance_session(
                    session,
                    case_definition,
                    case_instance,
                    plan_item,
                    parent_stage_instance_id,
                    "decisiontask",
                )?;
                continue;
            }
            complete_decision_task(
                session,
                case_definition,
                case_instance,
                plan_item,
                decision_task,
                parent_stage_instance_id,
            )?;
            continue;
        }
        if let Some(process_task) = container
            .process_tasks
            .iter()
            .find(|candidate| candidate.id == plan_item.definition_ref)
        {
            if manual_activation_rule_matches(plan_item, case_instance)? {
                persist_enabled_plan_item_instance_session(
                    session,
                    case_definition,
                    case_instance,
                    plan_item,
                    parent_stage_instance_id,
                    "processtask",
                )?;
                continue;
            }
            activate_process_task(
                session,
                case_definition,
                case_instance,
                plan_item,
                process_task,
                parent_stage_instance_id,
                process_task_runner,
            )?;
            continue;
        }
        if let Some(case_task) = container
            .case_tasks
            .iter()
            .find(|candidate| candidate.id == plan_item.definition_ref)
        {
            if manual_activation_rule_matches(plan_item, case_instance)? {
                persist_enabled_plan_item_instance_session(
                    session,
                    case_definition,
                    case_instance,
                    plan_item,
                    parent_stage_instance_id,
                    "casetask",
                )?;
                continue;
            }
            activate_case_task(
                session,
                case_definition,
                case_instance,
                plan_item,
                case_task,
                parent_stage_instance_id,
            )?;
            continue;
        }
        if let Some(milestone) = container
            .milestones
            .iter()
            .find(|candidate| candidate.id == plan_item.definition_ref)
        {
            if manual_activation_rule_matches(plan_item, case_instance)? {
                persist_enabled_plan_item_instance_session(
                    session,
                    case_definition,
                    case_instance,
                    plan_item,
                    parent_stage_instance_id,
                    "milestone",
                )?;
                continue;
            }
            reach_milestone(
                session,
                case_definition,
                case_instance,
                plan_item,
                milestone,
                parent_stage_instance_id,
            )?;
            continue;
        }
        if let Some(event_listener) = container
            .event_listeners
            .iter()
            .find(|candidate| candidate.id == plan_item.definition_ref)
        {
            if manual_activation_rule_matches(plan_item, case_instance)? {
                persist_enabled_plan_item_instance_session(
                    session,
                    case_definition,
                    case_instance,
                    plan_item,
                    parent_stage_instance_id,
                    PLAN_ITEM_DEFINITION_TYPE_EVENT_LISTENER,
                )?;
                continue;
            }
            activate_event_listener(
                session,
                case_definition,
                case_instance,
                plan_item,
                event_listener,
                parent_stage_instance_id,
            )?;
            continue;
        }

        return Err(CmmnError::storage(format!(
            "CMMN plan item '{}' references missing definition '{}'",
            plan_item.id, plan_item.definition_ref
        )));
    }

    Ok(())
}

fn materialize_available_milestone_instances(
    session: &mut DbSession,
    case_definition: &CmmnCaseDefinition,
    case_instance: &CmmnCaseInstance,
    container: ContainerView<'_>,
    parent_stage_instance_id: Option<&str>,
) -> Result<(), CmmnError> {
    let existing = load_plan_item_instances_session(session)?;
    for plan_item in container.plan_items {
        let Some(milestone) = container
            .milestones
            .iter()
            .find(|candidate| candidate.id == plan_item.definition_ref)
        else {
            continue;
        };
        if existing.iter().any(|instance| {
            instance.case_instance_id == case_instance.id
                && instance.plan_item_id == plan_item.id
                && instance.plan_item_definition_type == "milestone"
                && instance.stage_instance_id.as_deref() == parent_stage_instance_id
                && instance.ended_at.is_none()
        }) {
            continue;
        }
        let created_at = Utc::now();
        persist_plan_item_instance_session(
            session,
            &CmmnPlanItemInstance {
                id: format!("cmmn-plan-item-instance:{}", Uuid::new_v4()),
                case_instance_id: case_instance.id.clone(),
                case_definition_id: case_definition.id.clone(),
                stage_instance_id: parent_stage_instance_id.map(str::to_string),
                plan_item_id: plan_item.id.clone(),
                plan_item_definition_id: milestone.id.clone(),
                plan_item_definition_type: "milestone".to_string(),
                name: plan_item
                    .name
                    .clone()
                    .unwrap_or_else(|| milestone.name.clone()),
                state: "AVAILABLE".to_string(),
                created_at,
                last_enabled_at: None,
                ended_at: None,
                occurred_at: None,
                assignee: None,
                tenant_id: case_instance.tenant_id.clone(),
            },
        )?;
    }
    Ok(())
}

/// Command-scoped record of the plan-item lifecycle events propagated during
/// the current engine command (one `DbSession` == one transaction).
///
/// Mirrors the in-memory `satisfiedSentryPartInstances` collection Java keeps
/// on the plan item instance entity: for `onEvent` trigger-mode sentries an
/// onPart satisfaction is added to that collection but never inserted into the
/// database (AbstractEvaluationCriteriaOperation.java:707-713), so it lives
/// only for the duration of the command and must not accumulate across
/// commands the way the persisted ACT_CMMN_PLAN_ITEM_EVENT log does. `default`
/// trigger-mode sentries keep reading the persisted log instead.
///
/// Every event that can satisfy an onPart is, by construction, an event that
/// triggers a `handle_plan_item_standard_event` call for its `(source, event)`
/// pair, so recording at that entry captures exactly the onPart-relevant
/// events of the command.
#[derive(Default)]
struct CommandEventScope {
    events: HashSet<(String, String)>,
}

impl CommandEventScope {
    fn new() -> Self {
        Self::default()
    }

    fn record(&mut self, plan_item_id: &str, standard_event: &str) {
        self.events
            .insert((plan_item_id.to_string(), standard_event.to_string()));
    }

    fn contains(&self, plan_item_id: &str, standard_event: &str) -> bool {
        self.events
            .iter()
            .any(|(item, event)| item == plan_item_id && event == standard_event)
    }

    fn clear(&mut self) {
        self.events.clear();
    }
}

thread_local! {
    /// Ambient, command-scoped event set. A `DbSession` (one command, one
    /// transaction) runs synchronously on a single thread, so a thread-local
    /// mirrors Java's per-command in-memory sentry-part collection without
    /// threading a parameter through the whole evaluation cascade. It is reset
    /// at the start of every command in `CmmnStore::create_session`.
    static COMMAND_EVENT_SCOPE: RefCell<CommandEventScope> =
        RefCell::new(CommandEventScope::new());
}

/// Clears the command-scoped event set. Called when a new command opens its
/// `DbSession`, so `onEvent` onPart satisfaction never leaks across commands.
pub(crate) fn reset_command_event_scope() {
    COMMAND_EVENT_SCOPE.with(|scope| scope.borrow_mut().clear());
}

/// Records a propagated lifecycle event in the current command's scope.
fn record_command_event(plan_item_id: &str, standard_event: &str) {
    COMMAND_EVENT_SCOPE.with(|scope| scope.borrow_mut().record(plan_item_id, standard_event));
}

/// Whether `(plan_item_id, standard_event)` was propagated earlier in the
/// current command (the `onEvent` counterpart of the persisted-log lookup).
fn command_event_recorded(plan_item_id: &str, standard_event: &str) -> bool {
    COMMAND_EVENT_SCOPE.with(|scope| scope.borrow().contains(plan_item_id, standard_event))
}

fn handle_plan_item_standard_event(
    session: &mut DbSession,
    case_definition: &CmmnCaseDefinition,
    case_instance_id: &str,
    source_plan_item_id: &str,
    standard_event: &str,
    parent_stage_instance_id: Option<&str>,
) -> Result<(), CmmnError> {
    if find_container_with_direct_plan_item(
        ContainerView::from_case_plan_model(&case_definition.model.case_plan_model),
        source_plan_item_id,
    )
    .is_none()
        && find_discretionary_human_task_in_case_model(
            &case_definition.model.case_plan_model,
            source_plan_item_id,
        )
        .is_some()
    {
        return Ok(());
    }

    // The current lifecycle event belongs to this command; record it so
    // `onEvent` trigger-mode sentries observe their onParts satisfied within
    // the command without consulting the persisted log.
    record_command_event(source_plan_item_id, standard_event);

    // Each lifecycle event opens an evaluation cycle (Java: evaluateCriteria
    // runs for all available plan items on every cycle). In the default
    // trigger mode the ifPart of a multi-part sentry is evaluated each cycle
    // and persisted once satisfied, independently of its onParts
    // (AbstractEvaluationCriteriaOperation.java:550-566, :709-711).
    if let Some(case_instance) = load_case_instance_session(session, case_instance_id)? {
        record_satisfied_sentry_if_parts(session, case_definition, &case_instance)?;
    }

    let terminated_plan_items = terminate_exit_criterion_dependents(
        session,
        case_definition,
        case_instance_id,
        source_plan_item_id,
        standard_event,
    )?;

    for (plan_item_id, parent_stage_instance_id) in terminated_plan_items {
        handle_plan_item_standard_event(
            session,
            case_definition,
            case_instance_id,
            &plan_item_id,
            CmmnPlanItemOnPart::STANDARD_EVENT_TERMINATE,
            parent_stage_instance_id.as_deref(),
        )?;
    }

    activate_entry_criterion_dependents(
        session,
        case_definition,
        case_instance_id,
        source_plan_item_id,
        standard_event,
        parent_stage_instance_id,
    )?;

    // `exit` is a derived lifecycle event for human tasks and stages: fire once
    // when the source leaves the active lifecycle via complete or terminate.
    maybe_record_and_handle_exit_event(
        session,
        case_definition,
        case_instance_id,
        source_plan_item_id,
        standard_event,
        parent_stage_instance_id,
    )
}

/// Records and fans out the derived `exit` standard event for a human-task or
/// stage source that just completed or terminated. Idempotent: `exit` is
/// recorded at most once per plan item.
fn maybe_record_and_handle_exit_event(
    session: &mut DbSession,
    case_definition: &CmmnCaseDefinition,
    case_instance_id: &str,
    source_plan_item_id: &str,
    standard_event: &str,
    parent_stage_instance_id: Option<&str>,
) -> Result<(), CmmnError> {
    if !matches!(
        standard_event,
        CmmnPlanItemOnPart::STANDARD_EVENT_COMPLETE | CmmnPlanItemOnPart::STANDARD_EVENT_TERMINATE
    ) {
        return Ok(());
    }
    let is_human_task = find_human_task_plan_item_by_plan_item_id(
        &case_definition.model.case_plan_model,
        source_plan_item_id,
    )
    .is_some();
    let is_stage = find_stage_plan_item_by_plan_item_id(
        &case_definition.model.case_plan_model,
        source_plan_item_id,
    )
    .is_some();
    if !is_human_task && !is_stage {
        return Ok(());
    }

    // Ensure the primitive event is persisted (terminate paths often only
    // update historic task/stage state without writing ACT_CMMN_PLAN_ITEM_EVENT).
    if !plan_item_standard_event_occurred(
        session,
        case_instance_id,
        source_plan_item_id,
        standard_event,
    )? {
        record_plan_item_standard_event_session(
            session,
            case_instance_id,
            source_plan_item_id,
            standard_event,
        )?;
    }

    if plan_item_standard_event_occurred(
        session,
        case_instance_id,
        source_plan_item_id,
        CmmnPlanItemOnPart::STANDARD_EVENT_EXIT,
    )? {
        return Ok(());
    }

    record_plan_item_standard_event_session(
        session,
        case_instance_id,
        source_plan_item_id,
        CmmnPlanItemOnPart::STANDARD_EVENT_EXIT,
    )?;
    handle_plan_item_standard_event(
        session,
        case_definition,
        case_instance_id,
        source_plan_item_id,
        CmmnPlanItemOnPart::STANDARD_EVENT_EXIT,
        parent_stage_instance_id,
    )
}

fn terminate_exit_criterion_dependents(
    session: &mut DbSession,
    case_definition: &CmmnCaseDefinition,
    case_instance_id: &str,
    source_plan_item_id: &str,
    standard_event: &str,
) -> Result<Vec<(String, Option<String>)>, CmmnError> {
    let case_instance =
        load_case_instance_session(session, case_instance_id)?.ok_or_else(|| {
            CmmnError::storage(format!(
                "CMMN case instance '{case_instance_id}' disappeared during sentry evaluation"
            ))
        })?;
    let container = find_container_with_direct_plan_item(
        ContainerView::from_case_plan_model(&case_definition.model.case_plan_model),
        source_plan_item_id,
    )
    .ok_or_else(|| {
        CmmnError::storage(format!(
            "CMMN plan item '{source_plan_item_id}' was not found in case definition '{}'",
            case_definition.id
        ))
    })?;

    let mut matching_sentry_ids = Vec::new();
    for sentry in container.sentries {
        // Only sentries actually referenced as an exit criterion participate
        // here (Java evaluates criteria per plan item via
        // `planItem.getExitCriteria()`); otherwise this pass would consume
        // and delete the persisted ifPart marker of an entry-only sentry.
        if !container.plan_items.iter().any(|plan_item| {
            plan_item
                .exit_criterion_ids
                .iter()
                .any(|criterion_id| criterion_id == &sentry.id)
        }) {
            continue;
        }
        if sentry.plan_item_on_parts.iter().any(|on_part| {
            on_part.source_ref == source_plan_item_id && on_part.standard_event == standard_event
        }) && sentry_plan_item_on_parts_satisfied(session, case_instance_id, sentry)?
            && sentry_if_part_satisfied(session, sentry, &case_instance)?
        {
            matching_sentry_ids.push(sentry.id.as_str());
        }
    }
    if matching_sentry_ids.is_empty() {
        return Ok(Vec::new());
    }

    let terminated_plan_items = terminate_matching_exit_criterion_targets(
        session,
        case_definition,
        case_instance_id,
        container,
        &matching_sentry_ids,
    )?;

    // Java removes the sentry part instances together with the plan item
    // instance that leaves its waiting state once the criterion triggered
    // (PlanItemInstanceEntityManagerImpl.java:172-180).
    for sentry_id in &matching_sentry_ids {
        delete_sentry_if_part_satisfied(session, case_instance_id, sentry_id)?;
    }

    Ok(terminated_plan_items)
}

fn handle_if_part_only_exit_criteria(
    session: &mut DbSession,
    case_definition: &CmmnCaseDefinition,
    case_instance_id: &str,
) -> Result<(), CmmnError> {
    let terminated_plan_items = terminate_if_part_only_exit_criterion_dependents(
        session,
        case_definition,
        case_instance_id,
        ContainerView::from_case_plan_model(&case_definition.model.case_plan_model),
    )?;

    for (plan_item_id, parent_stage_instance_id) in terminated_plan_items {
        handle_plan_item_standard_event(
            session,
            case_definition,
            case_instance_id,
            &plan_item_id,
            CmmnPlanItemOnPart::STANDARD_EVENT_TERMINATE,
            parent_stage_instance_id.as_deref(),
        )?;
    }

    Ok(())
}

fn terminate_if_part_only_exit_criterion_dependents(
    session: &mut DbSession,
    case_definition: &CmmnCaseDefinition,
    case_instance_id: &str,
    container: ContainerView<'_>,
) -> Result<Vec<(String, Option<String>)>, CmmnError> {
    let case_instance =
        load_case_instance_session(session, case_instance_id)?.ok_or_else(|| {
            CmmnError::storage(format!(
                "CMMN case instance '{case_instance_id}' disappeared during sentry evaluation"
            ))
        })?;

    let matching_sentry_ids = container
        .sentries
        .iter()
        .filter(|sentry| sentry.plan_item_on_parts.is_empty() && sentry.if_part.is_some())
        .filter_map(
            |sentry| match sentry_if_part_matches(sentry, &case_instance) {
                Ok(true) => Some(Ok(sentry.id.as_str())),
                Ok(false) => None,
                Err(error) => Some(Err(error)),
            },
        )
        .collect::<Result<Vec<_>, _>>()?;

    let mut terminated_plan_items = terminate_matching_exit_criterion_targets(
        session,
        case_definition,
        case_instance_id,
        container,
        &matching_sentry_ids,
    )?;

    for stage in container.stages {
        terminated_plan_items.extend(terminate_if_part_only_exit_criterion_dependents(
            session,
            case_definition,
            case_instance_id,
            ContainerView::from_stage(stage),
        )?);
    }

    Ok(terminated_plan_items)
}

fn terminate_matching_exit_criterion_targets(
    session: &mut DbSession,
    case_definition: &CmmnCaseDefinition,
    case_instance_id: &str,
    container: ContainerView<'_>,
    matching_sentry_ids: &[&str],
) -> Result<Vec<(String, Option<String>)>, CmmnError> {
    if matching_sentry_ids.is_empty() {
        return Ok(Vec::new());
    }

    let mut terminated_plan_items = Vec::new();
    for plan_item in container.plan_items {
        if plan_item.exit_criterion_ids.is_empty()
            || !plan_item
                .exit_criterion_ids
                .iter()
                .any(|criterion_id| matching_sentry_ids.contains(&criterion_id.as_str()))
        {
            continue;
        }

        if container
            .human_tasks
            .iter()
            .any(|candidate| candidate.id == plan_item.definition_ref)
        {
            terminated_plan_items.extend(terminate_human_task_plan_item(
                session,
                case_instance_id,
                &plan_item.id,
            )?);
            continue;
        }

        if let Some(stage) = container
            .stages
            .iter()
            .find(|candidate| candidate.id == plan_item.definition_ref)
        {
            terminated_plan_items.extend(terminate_stage_plan_item(
                session,
                case_definition,
                case_instance_id,
                plan_item,
                stage,
            )?);
            continue;
        }

        if container
            .milestones
            .iter()
            .any(|candidate| candidate.id == plan_item.definition_ref)
        {
            terminated_plan_items.extend(terminate_occurred_milestone_plan_item(
                session,
                case_instance_id,
                &plan_item.id,
            )?);
            continue;
        }

        if container
            .event_listeners
            .iter()
            .any(|candidate| candidate.id == plan_item.definition_ref)
        {
            terminated_plan_items.extend(terminate_event_listener_plan_item(
                session,
                case_instance_id,
                &plan_item.id,
            )?);
            continue;
        }

        return Err(CmmnError::unsupported(
            "exit criterion target",
            format!(
                "case '{}' plan item '{}' has an exit criterion, but only human task, stage, occurred milestone, and event listener targets are supported",
                case_definition.key, plan_item.id
            ),
        ));
    }

    Ok(terminated_plan_items)
}

fn activate_entry_criterion_dependents(
    session: &mut DbSession,
    case_definition: &CmmnCaseDefinition,
    case_instance_id: &str,
    source_plan_item_id: &str,
    standard_event: &str,
    parent_stage_instance_id: Option<&str>,
) -> Result<(), CmmnError> {
    let case_instance =
        load_case_instance_session(session, case_instance_id)?.ok_or_else(|| {
            CmmnError::storage(format!(
                "CMMN case instance '{case_instance_id}' disappeared during sentry evaluation"
            ))
        })?;
    let container = find_container_with_direct_plan_item(
        ContainerView::from_case_plan_model(&case_definition.model.case_plan_model),
        source_plan_item_id,
    )
    .ok_or_else(|| {
        CmmnError::storage(format!(
            "CMMN plan item '{source_plan_item_id}' was not found in case definition '{}'",
            case_definition.id
        ))
    })?;

    let mut matching_sentry_ids = Vec::new();
    for sentry in container.sentries {
        // Only sentries actually referenced as an entry criterion participate
        // here (Java evaluates criteria per plan item via
        // `planItem.getEntryCriteria()`); otherwise this pass would consume
        // and delete the persisted ifPart marker of an exit-only sentry.
        if !container.plan_items.iter().any(|plan_item| {
            plan_item
                .entry_criterion_ids
                .iter()
                .any(|criterion_id| criterion_id == &sentry.id)
        }) {
            continue;
        }
        if sentry.plan_item_on_parts.iter().any(|on_part| {
            on_part.source_ref == source_plan_item_id && on_part.standard_event == standard_event
        }) && sentry_plan_item_on_parts_satisfied(session, case_instance_id, sentry)?
            && sentry_if_part_satisfied(session, sentry, &case_instance)?
        {
            matching_sentry_ids.push(sentry.id.as_str());
        }
    }
    if matching_sentry_ids.is_empty() {
        return Ok(());
    }

    if let Some(parent_stage_instance_id) = parent_stage_instance_id
        && !stage_instance_is_active(session, parent_stage_instance_id)?
    {
        return Ok(());
    }

    for plan_item in container.plan_items {
        if plan_item.entry_criterion_ids.is_empty()
            || !plan_item
                .entry_criterion_ids
                .iter()
                .any(|criterion_id| matching_sentry_ids.contains(&criterion_id.as_str()))
        {
            continue;
        }

        if let Some(human_task) = container
            .human_tasks
            .iter()
            .find(|candidate| candidate.id == plan_item.definition_ref)
        {
            if human_task_instance_exists(
                session,
                case_instance_id,
                &plan_item.id,
                parent_stage_instance_id,
            )? {
                continue;
            }
            activate_human_task(
                session,
                case_definition,
                &case_instance,
                plan_item,
                human_task,
                parent_stage_instance_id,
            )?;
            continue;
        }

        if let Some(stage) = container
            .stages
            .iter()
            .find(|candidate| candidate.id == plan_item.definition_ref)
        {
            if stage_instance_exists(session, case_instance_id, &plan_item.id)? {
                continue;
            }
            activate_stage(
                session,
                case_definition,
                &case_instance,
                plan_item,
                stage,
                parent_stage_instance_id,
            )?;
            continue;
        }

        if let Some(decision_task) = container
            .decision_tasks
            .iter()
            .find(|candidate| candidate.id == plan_item.definition_ref)
        {
            if plan_item_standard_event_occurred(
                session,
                case_instance_id,
                &plan_item.id,
                CmmnPlanItemOnPart::STANDARD_EVENT_COMPLETE,
            )? {
                continue;
            }
            if manual_activation_rule_matches(plan_item, &case_instance)? {
                persist_enabled_plan_item_instance_session(
                    session,
                    case_definition,
                    &case_instance,
                    plan_item,
                    parent_stage_instance_id,
                    "decisiontask",
                )?;
                continue;
            }
            complete_decision_task(
                session,
                case_definition,
                &case_instance,
                plan_item,
                decision_task,
                parent_stage_instance_id,
            )?;
            continue;
        }

        if let Some(process_task) = container
            .process_tasks
            .iter()
            .find(|candidate| candidate.id == plan_item.definition_ref)
        {
            if task_association_exists(
                session,
                case_instance_id,
                &plan_item.id,
                parent_stage_instance_id,
            )? {
                continue;
            }
            if manual_activation_rule_matches(plan_item, &case_instance)? {
                persist_enabled_plan_item_instance_session(
                    session,
                    case_definition,
                    &case_instance,
                    plan_item,
                    parent_stage_instance_id,
                    "processtask",
                )?;
                continue;
            }
            activate_process_task(
                session,
                case_definition,
                &case_instance,
                plan_item,
                process_task,
                parent_stage_instance_id,
                None,
            )?;
            continue;
        }

        if let Some(case_task) = container
            .case_tasks
            .iter()
            .find(|candidate| candidate.id == plan_item.definition_ref)
        {
            if task_association_exists(
                session,
                case_instance_id,
                &plan_item.id,
                parent_stage_instance_id,
            )? {
                continue;
            }
            if manual_activation_rule_matches(plan_item, &case_instance)? {
                persist_enabled_plan_item_instance_session(
                    session,
                    case_definition,
                    &case_instance,
                    plan_item,
                    parent_stage_instance_id,
                    "casetask",
                )?;
                continue;
            }
            activate_case_task(
                session,
                case_definition,
                &case_instance,
                plan_item,
                case_task,
                parent_stage_instance_id,
            )?;
            continue;
        }

        if let Some(milestone) = container
            .milestones
            .iter()
            .find(|candidate| candidate.id == plan_item.definition_ref)
        {
            if plan_item_standard_event_occurred(
                session,
                case_instance_id,
                &plan_item.id,
                CmmnPlanItemOnPart::STANDARD_EVENT_OCCUR,
            )? {
                continue;
            }
            if manual_activation_rule_matches(plan_item, &case_instance)? {
                persist_enabled_plan_item_instance_session(
                    session,
                    case_definition,
                    &case_instance,
                    plan_item,
                    parent_stage_instance_id,
                    "milestone",
                )?;
                continue;
            }

            reach_milestone(
                session,
                case_definition,
                &case_instance,
                plan_item,
                milestone,
                parent_stage_instance_id,
            )?;
            continue;
        }

        return Err(CmmnError::unsupported(
            "entry criterion target",
            format!(
                "case '{}' plan item '{}' has an entry criterion, but only human task, stage, decision task, process task, case task, and milestone targets are supported",
                case_definition.key, plan_item.id
            ),
        ));
    }

    // Java removes the sentry part instances together with the plan item
    // instance that leaves its waiting state once the criterion triggered
    // (PlanItemInstanceEntityManagerImpl.java:172-180).
    for sentry_id in &matching_sentry_ids {
        delete_sentry_if_part_satisfied(session, case_instance_id, sentry_id)?;
    }

    Ok(())
}

/// Whether every planItemOnPart of `sentry` is currently satisfied.
///
/// * `default` trigger mode — an onPart is satisfied once its source plan item
///   has ever reached the referenced standard event, read back from the
///   persisted ACT_CMMN_PLAN_ITEM_EVENT log (or the current human-task state),
///   so satisfaction accumulates across commands. This mirrors Java inserting a
///   `SentryPartInstanceEntity` per satisfied onPart
///   (AbstractEvaluationCriteriaOperation.java:709-711).
/// * `onEvent` trigger mode — the SentryPartInstance is never persisted and
///   only lives in memory for the current command (:707-713), so an onPart is
///   satisfied only when its event was propagated within this command. The
///   command-scoped `scope` stands in for that in-memory collection; nothing is
///   read from the persisted log, so satisfaction resets across commands.
fn sentry_plan_item_on_parts_satisfied(
    session: &mut DbSession,
    case_instance_id: &str,
    sentry: &CmmnSentry,
) -> Result<bool, CmmnError> {
    let on_event_trigger_mode = sentry.is_on_event_trigger_mode();
    for on_part in &sentry.plan_item_on_parts {
        if on_event_trigger_mode {
            if command_event_recorded(&on_part.source_ref, &on_part.standard_event) {
                continue;
            }
            return Ok(false);
        }

        if plan_item_standard_event_occurred(
            session,
            case_instance_id,
            &on_part.source_ref,
            &on_part.standard_event,
        )? {
            continue;
        }

        if !human_task_plan_item_reached_standard_event(
            session,
            case_instance_id,
            &on_part.source_ref,
            &on_part.standard_event,
        )? {
            return Ok(false);
        }
    }
    Ok(true)
}

fn sentry_if_part_matches(
    sentry: &CmmnSentry,
    case_instance: &CmmnCaseInstance,
) -> Result<bool, CmmnError> {
    let Some(expression) = sentry.if_part.as_ref() else {
        return Ok(true);
    };
    evaluate_if_part_condition(expression, case_instance)
}

/// Trigger-mode aware ifPart check, mirroring the multi-part branch of
/// `AbstractEvaluationCriteriaOperation.evaluateCriteria`
/// (AbstractEvaluationCriteriaOperation.java:506-577):
///
/// * default trigger mode — an ifPart satisfied in an earlier evaluation
///   cycle is read back from the persisted sentry part instances (:515-525)
///   and a newly satisfied ifPart is persisted (:558-566 via
///   `createSentryPartInstanceEntity`, inserted only in default mode
///   :709-711), so the satisfaction sticks even when the underlying
///   variables change afterwards;
/// * onEvent trigger mode — nothing is persisted and the ifPart must hold
///   at the moment all onParts are satisfied (:550-551).
///
/// Single-onPart-without-ifPart and ifPart-only sentries keep the plain
/// evaluation of the Java fast paths (:475-490, :492-504).
fn sentry_if_part_satisfied(
    session: &mut DbSession,
    sentry: &CmmnSentry,
    case_instance: &CmmnCaseInstance,
) -> Result<bool, CmmnError> {
    if sentry.if_part.is_none() {
        return Ok(true);
    }
    if sentry.is_multi_part() && sentry.is_default_trigger_mode() {
        if plan_item_standard_event_occurred(
            session,
            &case_instance.id,
            &sentry.id,
            SENTRY_IF_PART_SATISFIED_EVENT,
        )? {
            return Ok(true);
        }
        if sentry_if_part_matches(sentry, case_instance)? {
            record_sentry_if_part_satisfied(session, &case_instance.id, &sentry.id)?;
            return Ok(true);
        }
        return Ok(false);
    }
    sentry_if_part_matches(sentry, case_instance)
}

/// Evaluation-cycle sweep for default-trigger-mode multi-part sentries:
/// evaluates their ifParts against the current case variables and persists
/// a sentry part instance once satisfied, independently of the onPart
/// status (AbstractEvaluationCriteriaOperation.java:550-566 — in default
/// mode the ifPart branch runs on every cycle, not only when all onParts
/// are satisfied). Evaluation failures count as not-satisfied, matching
/// the `unwrap_or(false)` convention of `CmmnSentry::evaluate_for_event`.
fn record_satisfied_sentry_if_parts(
    session: &mut DbSession,
    case_definition: &CmmnCaseDefinition,
    case_instance: &CmmnCaseInstance,
) -> Result<(), CmmnError> {
    record_satisfied_sentry_if_parts_in_container(
        session,
        case_instance,
        ContainerView::from_case_plan_model(&case_definition.model.case_plan_model),
    )
}

fn record_satisfied_sentry_if_parts_in_container(
    session: &mut DbSession,
    case_instance: &CmmnCaseInstance,
    container: ContainerView<'_>,
) -> Result<(), CmmnError> {
    for sentry in container.sentries {
        if sentry.if_part.is_none() || !sentry.is_multi_part() || !sentry.is_default_trigger_mode()
        {
            continue;
        }
        if plan_item_standard_event_occurred(
            session,
            &case_instance.id,
            &sentry.id,
            SENTRY_IF_PART_SATISFIED_EVENT,
        )? {
            continue;
        }
        if matches!(sentry_if_part_matches(sentry, case_instance), Ok(true)) {
            record_sentry_if_part_satisfied(session, &case_instance.id, &sentry.id)?;
        }
    }

    for stage in container.stages {
        record_satisfied_sentry_if_parts_in_container(
            session,
            case_instance,
            ContainerView::from_stage(stage),
        )?;
    }

    Ok(())
}

/// Persists the ifPart satisfaction marker, the counterpart of
/// `createSentryPartInstanceEntity(..., null, sentry.getSentryIfPart())`
/// (AbstractEvaluationCriteriaOperation.java:679-715).
fn record_sentry_if_part_satisfied(
    session: &mut DbSession,
    case_instance_id: &str,
    sentry_id: &str,
) -> Result<(), CmmnError> {
    record_plan_item_standard_event_session(
        session,
        case_instance_id,
        sentry_id,
        SENTRY_IF_PART_SATISFIED_EVENT,
    )
}

/// Removes the persisted ifPart satisfaction marker of a sentry whose
/// criterion has triggered (Java deletes the sentry part instances with the
/// listening plan item instance, PlanItemInstanceEntityManagerImpl.java:172-180).
fn delete_sentry_if_part_satisfied(
    session: &mut DbSession,
    case_instance_id: &str,
    sentry_id: &str,
) -> Result<(), CmmnError> {
    let mut params = DbParams::new();
    params.push(case_instance_id);
    params.push(sentry_id);
    params.push(SENTRY_IF_PART_SATISFIED_EVENT);
    let rendered = RenderedStatement::new(
        "DELETE FROM ACT_CMMN_PLAN_ITEM_EVENT \
         WHERE CASE_INSTANCE_ID_ = ? AND PLAN_ITEM_ID_ = ? AND STANDARD_EVENT_ = ?"
            .to_string(),
        params,
    );
    session.execute_raw(rendered)?;
    Ok(())
}

fn evaluate_if_part_condition(
    expression: &CmmnSentryIfPartExpression,
    case_instance: &CmmnCaseInstance,
) -> Result<bool, CmmnError> {
    evaluate_to_bool(expression, &case_instance.variables, case_instance)
}

/// True when the raw expression uses the UEL `${…}` form (P69 SimpleExpression path).
fn is_uel_expression(expression: &str) -> bool {
    let trimmed = expression.trim();
    trimmed.starts_with("${") && trimmed.ends_with('}')
}

/// Case-variable scope for SimpleExpression evaluation (no `${execution}` root).
fn case_variable_scope(case_instance: &CmmnCaseInstance) -> MapVariableContainer {
    MapVariableContainer::from_json_map(&case_instance.variables)
        .with_tenant_id(case_instance.tenant_id.clone())
}

/// Evaluate a raw availableCondition string.
///
/// - `${…}` → SimpleExpression against case variables; only a JSON boolean
///   `true` counts as available (Java AbstractEvaluationCriteriaOperation
///   non-boolean / null / failed evaluation → unavailable).
/// - otherwise → existing CMMN if-part dialect (C7 parity).
fn evaluate_available_condition_expression(
    expression: &str,
    case_instance: &CmmnCaseInstance,
) -> bool {
    let trimmed = expression.trim();
    if is_uel_expression(trimmed) {
        let scope = case_variable_scope(case_instance);
        matches!(
            SimpleExpression::new(trimmed.to_string()).get_value(&scope),
            Some(Value::Bool(true))
        )
    } else {
        match CmmnSentryIfPartExpression::parse(trimmed) {
            Ok(parsed) => matches!(evaluate_if_part_condition(&parsed, case_instance), Ok(true)),
            Err(_) => false,
        }
    }
}

/// Resolve a human-task attribute: `${…}` via SimpleExpression, else literal.
/// SimpleExpression capability is the upper bound (P69); unresolved EL yields None.
fn resolve_el_or_literal_string(
    raw: Option<&str>,
    case_instance: &CmmnCaseInstance,
) -> Option<String> {
    let raw = raw?.trim();
    if raw.is_empty() {
        return None;
    }
    if !is_uel_expression(raw) {
        return Some(raw.to_string());
    }
    let scope = case_variable_scope(case_instance);
    match SimpleExpression::new(raw.to_string()).get_value(&scope) {
        Some(Value::String(s)) => Some(s),
        Some(Value::Number(n)) => Some(n.to_string()),
        Some(Value::Bool(b)) => Some(b.to_string()),
        Some(Value::Null) | None => None,
        Some(other) => Some(other.to_string()),
    }
}

/// Evaluate each candidate entry; if an entry is UEL, evaluate then comma-split
/// (Java handleCandidateUsers/Groups after expression resolution).
fn resolve_candidate_list(entries: &[String], case_instance: &CmmnCaseInstance) -> Vec<String> {
    let mut resolved = Vec::new();
    for entry in entries {
        let trimmed = entry.trim();
        if trimmed.is_empty() {
            continue;
        }
        if is_uel_expression(trimmed) {
            if let Some(value) = resolve_el_or_literal_string(Some(trimmed), case_instance) {
                for part in value.split(',') {
                    let part = part.trim();
                    if !part.is_empty() {
                        resolved.push(part.to_string());
                    }
                }
            }
        } else {
            resolved.push(trimmed.to_string());
        }
    }
    resolved
}

/// Maximum AST evaluation depth for CMMN ifPart expressions.
/// P142c: aligns with parser nesting caps so a deep AST cannot stack-overflow
/// the evaluator even if constructed by other means.
const MAX_IF_PART_EVAL_DEPTH: usize = 64;

fn evaluate_to_bool(
    expression: &CmmnSentryIfPartExpression,
    variables: &serde_json::Map<String, Value>,
    case_instance: &CmmnCaseInstance,
) -> Result<bool, CmmnError> {
    evaluate_to_bool_depth(expression, variables, case_instance, 0)
}

fn evaluate_to_bool_depth(
    expression: &CmmnSentryIfPartExpression,
    variables: &serde_json::Map<String, Value>,
    case_instance: &CmmnCaseInstance,
    depth: usize,
) -> Result<bool, CmmnError> {
    if depth >= MAX_IF_PART_EVAL_DEPTH {
        return Err(CmmnError::Execution {
            message: format!(
                "ifPart expression evaluation exceeds maximum depth of {MAX_IF_PART_EVAL_DEPTH}"
            ),
        });
    }
    let next = depth + 1;
    match expression {
        CmmnSentryIfPartExpression::Comparison(condition) => {
            evaluate_if_part_comparison(condition, case_instance)
        }
        CmmnSentryIfPartExpression::Logical { operator, operands } => match operator {
            CmmnSentryIfPartLogicalOperator::And => {
                for operand in operands {
                    if !evaluate_to_bool_depth(operand, variables, case_instance, next)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            CmmnSentryIfPartLogicalOperator::Or => {
                for operand in operands {
                    if evaluate_to_bool_depth(operand, variables, case_instance, next)? {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
        },
        CmmnSentryIfPartExpression::Not { operand } => Ok(!evaluate_to_bool_depth(
            operand,
            variables,
            case_instance,
            next,
        )?),
        CmmnSentryIfPartExpression::Empty { variable_name } => Ok(if_part_variable_is_empty(
            resolve_if_part_variable_path(variables, variable_name),
        )),
        CmmnSentryIfPartExpression::Contains {
            collection_variable_name,
            value,
            expected,
        } => {
            let collection = resolve_if_part_path_or_value_expression(
                variables,
                collection_variable_name,
                case_instance,
            );
            let value = resolve_if_part_literal_value(variables, value).or_else(|| {
                resolve_if_part_literal_value_expression(variables, value, case_instance)
            });
            Ok(if_part_contains(collection.as_ref(), value.as_ref()) == *expected)
        }
        CmmnSentryIfPartExpression::StartsWith {
            variable_name,
            prefix,
        } => {
            let value = resolve_if_part_variable_path(variables, variable_name);
            Ok(match value {
                Some(serde_json::Value::String(s)) => s.starts_with(prefix),
                _ => false,
            })
        }
        CmmnSentryIfPartExpression::EndsWith {
            variable_name,
            suffix,
        } => {
            let value = resolve_if_part_variable_path(variables, variable_name);
            Ok(match value {
                Some(serde_json::Value::String(s)) => s.ends_with(suffix),
                _ => false,
            })
        }
        CmmnSentryIfPartExpression::Matches {
            variable_name,
            regex,
        } => {
            let value = resolve_if_part_variable_path(variables, variable_name);
            Ok(match value {
                Some(serde_json::Value::String(s)) => regex::Regex::new(regex)
                    .map(|re| re.is_match(s))
                    .unwrap_or(false),
                _ => false,
            })
        }
        CmmnSentryIfPartExpression::Size {
            collection_variable_name,
            operator,
            literal,
        } => {
            let value = resolve_if_part_path_or_value_expression(
                variables,
                collection_variable_name,
                case_instance,
            );
            let size = value.as_ref().and_then(if_part_size).unwrap_or(0) as i64;
            let expected = resolve_if_part_literal_number(variables, literal);
            Ok(compare_numbers(size, *operator, expected))
        }
        CmmnSentryIfPartExpression::Length {
            variable_name,
            operator,
            literal,
        } => {
            let value =
                resolve_if_part_path_or_value_expression(variables, variable_name, case_instance);
            let length = value.as_ref().and_then(if_part_length).unwrap_or(0) as i64;
            let expected = resolve_if_part_literal_number(variables, literal);
            Ok(compare_numbers(length, *operator, expected))
        }
        other => {
            let val = evaluate_to_json_value_depth(other, variables, case_instance, next)?;
            Ok(is_truthy(&val))
        }
    }
}

fn evaluate_to_json_value(
    expression: &CmmnSentryIfPartExpression,
    variables: &serde_json::Map<String, Value>,
    case_instance: &CmmnCaseInstance,
) -> Result<Value, CmmnError> {
    evaluate_to_json_value_depth(expression, variables, case_instance, 0)
}

fn evaluate_to_json_value_depth(
    expression: &CmmnSentryIfPartExpression,
    variables: &serde_json::Map<String, Value>,
    case_instance: &CmmnCaseInstance,
    depth: usize,
) -> Result<Value, CmmnError> {
    if depth >= MAX_IF_PART_EVAL_DEPTH {
        return Err(CmmnError::Execution {
            message: format!(
                "ifPart expression evaluation exceeds maximum depth of {MAX_IF_PART_EVAL_DEPTH}"
            ),
        });
    }
    let next = depth + 1;
    match expression {
        CmmnSentryIfPartExpression::Comparison(_)
        | CmmnSentryIfPartExpression::Logical { .. }
        | CmmnSentryIfPartExpression::Not { .. }
        | CmmnSentryIfPartExpression::Empty { .. }
        | CmmnSentryIfPartExpression::Contains { .. }
        | CmmnSentryIfPartExpression::StartsWith { .. }
        | CmmnSentryIfPartExpression::EndsWith { .. }
        | CmmnSentryIfPartExpression::Matches { .. }
        | CmmnSentryIfPartExpression::Size { .. }
        | CmmnSentryIfPartExpression::Length { .. } => {
            let res = evaluate_to_bool_depth(expression, variables, case_instance, next)?;
            Ok(Value::Bool(res))
        }
        CmmnSentryIfPartExpression::Literal(lit) => {
            Ok(resolve_if_part_literal_value(variables, lit).unwrap_or(Value::Null))
        }
        CmmnSentryIfPartExpression::MethodCall {
            object,
            method,
            args: _,
        } => {
            if method == "size" || method == "length" {
                let size = object
                    .as_ref()
                    .and_then(|obj| {
                        resolve_if_part_path_or_value_expression(variables, obj, case_instance)
                    })
                    .as_ref()
                    .and_then(if_part_size)
                    .unwrap_or(0);
                return Ok(Value::Number(size.into()));
            }

            // Normal method call / property resolution:
            let path = if let Some(obj) = object {
                format!("{obj}.{method}")
            } else {
                method.clone()
            };
            if let Some(val) = resolve_if_part_variable_path(variables, &path) {
                return Ok(val.clone());
            }
            if let Some(obj) = object
                && let Some(val) = resolve_if_part_variable_path(variables, obj)
                && let Some(prop) = val.get(method)
            {
                return Ok(prop.clone());
            }
            Ok(Value::Null)
        }
        CmmnSentryIfPartExpression::Arithmetic {
            left,
            operator,
            right,
        } => {
            let l = evaluate_to_json_value_depth(left, variables, case_instance, next)?;
            let r = evaluate_to_json_value_depth(right, variables, case_instance, next)?;
            if operator == "+" && (matches!(l, Value::String(_)) || matches!(r, Value::String(_))) {
                return Ok(Value::String(format!(
                    "{}{}",
                    json_value_to_string_operand(&l),
                    json_value_to_string_operand(&r)
                )));
            }
            let l_num = l
                .as_f64()
                .or_else(|| l.as_i64().map(|n| n as f64))
                .unwrap_or(0.0);
            let r_num = r
                .as_f64()
                .or_else(|| r.as_i64().map(|n| n as f64))
                .unwrap_or(0.0);
            let result = match operator.as_str() {
                "+" => l_num + r_num,
                "-" => l_num - r_num,
                "*" => l_num * r_num,
                "/" if r_num != 0.0 => l_num / r_num,
                "%" if r_num != 0.0 => l_num % r_num,
                _ => 0.0,
            };
            Ok(Value::Number(
                serde_json::Number::from_f64(result).unwrap_or_else(|| serde_json::Number::from(0)),
            ))
        }
        CmmnSentryIfPartExpression::Ternary {
            condition,
            true_expr,
            false_expr,
        } => {
            let cond = evaluate_to_bool_depth(condition, variables, case_instance, next)?;
            if cond {
                evaluate_to_json_value_depth(true_expr, variables, case_instance, next)
            } else {
                evaluate_to_json_value_depth(false_expr, variables, case_instance, next)
            }
        }
        CmmnSentryIfPartExpression::PropertyAccess { object, property } => {
            let obj_val = evaluate_to_json_value_depth(object, variables, case_instance, next)?;
            let val = obj_val.get(property).cloned().unwrap_or(Value::Null);
            Ok(val)
        }
        CmmnSentryIfPartExpression::IndexAccess { object, index } => {
            let obj_val = evaluate_to_json_value_depth(object, variables, case_instance, next)?;
            let idx_val = evaluate_to_json_value_depth(index, variables, case_instance, next)?;
            let val = if let Some(idx) = idx_val.as_u64() {
                obj_val.get(idx as usize).cloned().unwrap_or(Value::Null)
            } else if let Some(idx) = idx_val.as_i64() {
                if idx >= 0 {
                    obj_val.get(idx as usize).cloned().unwrap_or(Value::Null)
                } else {
                    Value::Null
                }
            } else if let Some(idx) = idx_val.as_str() {
                obj_val.get(idx).cloned().unwrap_or(Value::Null)
            } else {
                Value::Null
            };
            Ok(val)
        }
    }
}

fn is_truthy(val: &Value) -> bool {
    match val {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => {
            n.as_f64().is_some_and(|f| f != 0.0) || n.as_i64().is_some_and(|i| i != 0)
        }
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

fn evaluate_if_part_comparison(
    condition: &crate::models::CmmnSentryIfPartCondition,
    case_instance: &CmmnCaseInstance,
) -> Result<bool, CmmnError> {
    let actual = resolve_if_part_comparison_operand(
        &case_instance.variables,
        &condition.variable_name,
        case_instance,
    );
    let expected =
        resolve_if_part_literal(&case_instance.variables, &condition.literal, case_instance);

    Ok(match condition.operator {
        CmmnSentryIfPartOperator::Equal => actual == expected,
        CmmnSentryIfPartOperator::NotEqual => actual != expected,
        CmmnSentryIfPartOperator::GreaterThan => {
            if_part_number_compare(actual.as_ref(), expected.as_ref())
                .is_some_and(|ordering| ordering > 0)
        }
        CmmnSentryIfPartOperator::GreaterThanOrEqual => {
            if_part_number_compare(actual.as_ref(), expected.as_ref())
                .is_some_and(|ordering| ordering >= 0)
        }
        CmmnSentryIfPartOperator::LessThan => {
            if_part_number_compare(actual.as_ref(), expected.as_ref())
                .is_some_and(|ordering| ordering < 0)
        }
        CmmnSentryIfPartOperator::LessThanOrEqual => {
            if_part_number_compare(actual.as_ref(), expected.as_ref())
                .is_some_and(|ordering| ordering <= 0)
        }
    })
}

#[derive(Debug, Clone, PartialEq)]
enum IfPartComparableValue {
    Null,
    Boolean(bool),
    String(String),
    Number(f64),
}

fn resolve_if_part_comparison_operand(
    variables: &serde_json::Map<String, Value>,
    operand: &str,
    case_instance: &CmmnCaseInstance,
) -> Option<IfPartComparableValue> {
    if let Some(variable_name) = operand
        .strip_prefix("size(")
        .and_then(|value| value.strip_suffix(')'))
    {
        return resolve_if_part_path_or_value_expression(variables, variable_name, case_instance)
            .as_ref()
            .and_then(if_part_size)
            .map(|size| IfPartComparableValue::Number(size as f64));
    }
    if let Some(variable_name) = operand
        .strip_prefix("length(")
        .and_then(|value| value.strip_suffix(')'))
    {
        return resolve_if_part_path_or_value_expression(variables, variable_name, case_instance)
            .as_ref()
            .and_then(if_part_length)
            .map(|size| IfPartComparableValue::Number(size as f64));
    }

    if let Some(value) = resolve_if_part_variable_path(variables, operand) {
        return json_value_to_if_part_comparable(Some(value));
    }

    resolve_if_part_value_expression(variables, operand, case_instance)
        .ok()
        .and_then(|value| json_value_to_if_part_comparable(Some(&value)))
}

fn resolve_if_part_literal(
    variables: &serde_json::Map<String, Value>,
    literal: &CmmnSentryIfPartLiteral,
    case_instance: &CmmnCaseInstance,
) -> Option<IfPartComparableValue> {
    match literal {
        CmmnSentryIfPartLiteral::Boolean(value) => Some(IfPartComparableValue::Boolean(*value)),
        CmmnSentryIfPartLiteral::String(value) => {
            Some(IfPartComparableValue::String(value.clone()))
        }
        CmmnSentryIfPartLiteral::Number(value) => {
            value.parse::<f64>().ok().map(IfPartComparableValue::Number)
        }
        CmmnSentryIfPartLiteral::Null => Some(IfPartComparableValue::Null),
        CmmnSentryIfPartLiteral::Variable(variable_name) => {
            if let Some(value) = resolve_if_part_variable_path(variables, variable_name) {
                return json_value_to_if_part_comparable(Some(value));
            }
            resolve_if_part_value_expression(variables, variable_name, case_instance)
                .ok()
                .and_then(|value| json_value_to_if_part_comparable(Some(&value)))
        }
    }
}

fn resolve_if_part_literal_value(
    variables: &serde_json::Map<String, Value>,
    literal: &CmmnSentryIfPartLiteral,
) -> Option<Value> {
    match literal {
        CmmnSentryIfPartLiteral::Boolean(value) => Some(Value::Bool(*value)),
        CmmnSentryIfPartLiteral::String(value) => Some(Value::String(value.clone())),
        CmmnSentryIfPartLiteral::Number(value) => {
            let number = value.parse::<serde_json::Number>().ok()?;
            Some(Value::Number(number))
        }
        CmmnSentryIfPartLiteral::Null => Some(Value::Null),
        CmmnSentryIfPartLiteral::Variable(variable_name) => {
            resolve_if_part_variable_path(variables, variable_name).cloned()
        }
    }
}

fn resolve_if_part_literal_value_expression(
    variables: &serde_json::Map<String, Value>,
    literal: &CmmnSentryIfPartLiteral,
    case_instance: &CmmnCaseInstance,
) -> Option<Value> {
    match literal {
        CmmnSentryIfPartLiteral::Variable(expression) => {
            resolve_if_part_value_expression(variables, expression, case_instance).ok()
        }
        _ => None,
    }
}

fn resolve_if_part_path_or_value_expression(
    variables: &serde_json::Map<String, Value>,
    expression: &str,
    case_instance: &CmmnCaseInstance,
) -> Option<Value> {
    resolve_if_part_variable_path(variables, expression)
        .cloned()
        .or_else(|| resolve_if_part_value_expression(variables, expression, case_instance).ok())
}

fn resolve_if_part_value_expression(
    variables: &serde_json::Map<String, Value>,
    expression: &str,
    case_instance: &CmmnCaseInstance,
) -> Result<Value, CmmnError> {
    let expression = flowable_cmmn_model::parse_sentry_value_expression(expression)
        .map_err(CmmnError::execution)?;
    evaluate_to_json_value(
        &CmmnSentryIfPartExpression::from(expression),
        variables,
        case_instance,
    )
}

fn json_value_to_string_operand(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        Value::String(value) => value.clone(),
        Value::Array(_) | Value::Object(_) => value.to_string(),
    }
}

fn json_value_to_if_part_comparable(value: Option<&Value>) -> Option<IfPartComparableValue> {
    match value {
        None | Some(Value::Null) => Some(IfPartComparableValue::Null),
        Some(Value::Bool(value)) => Some(IfPartComparableValue::Boolean(*value)),
        Some(Value::String(value)) => Some(IfPartComparableValue::String(value.clone())),
        Some(Value::Number(value)) => value.as_f64().map(IfPartComparableValue::Number),
        Some(Value::Array(_)) | Some(Value::Object(_)) => None,
    }
}

fn if_part_size(value: &Value) -> Option<usize> {
    match value {
        Value::String(value) => Some(value.chars().count()),
        Value::Array(value) => Some(value.len()),
        Value::Object(value) => Some(value.len()),
        Value::Null | Value::Bool(_) | Value::Number(_) => None,
    }
}

fn if_part_length(value: &Value) -> Option<usize> {
    if_part_size(value)
}

fn resolve_if_part_literal_number(
    variables: &serde_json::Map<String, Value>,
    literal: &CmmnSentryIfPartLiteral,
) -> i64 {
    match literal {
        CmmnSentryIfPartLiteral::Number(value) => value.parse::<i64>().unwrap_or(0),
        CmmnSentryIfPartLiteral::Variable(variable_name) => {
            resolve_if_part_variable_path(variables, variable_name)
                .and_then(|v| v.as_i64())
                .unwrap_or(0)
        }
        _ => 0,
    }
}

fn compare_numbers(actual: i64, operator: CmmnSentryIfPartOperator, expected: i64) -> bool {
    match operator {
        CmmnSentryIfPartOperator::Equal => actual == expected,
        CmmnSentryIfPartOperator::NotEqual => actual != expected,
        CmmnSentryIfPartOperator::GreaterThan => actual > expected,
        CmmnSentryIfPartOperator::GreaterThanOrEqual => actual >= expected,
        CmmnSentryIfPartOperator::LessThan => actual < expected,
        CmmnSentryIfPartOperator::LessThanOrEqual => actual <= expected,
    }
}

fn resolve_if_part_variable_path<'a>(
    variables: &'a serde_json::Map<String, Value>,
    variable_name: &str,
) -> Option<&'a Value> {
    let bytes = variable_name.as_bytes();
    let first = *bytes.first()?;
    if !is_if_part_identifier_start_byte(first) {
        return None;
    }

    let mut index = 1;
    while index < bytes.len() && is_if_part_identifier_byte(bytes[index]) {
        index += 1;
    }

    let mut current = variables.get(&variable_name[..index])?;
    while index < bytes.len() {
        match bytes[index] {
            b'.' => {
                index += 1;
                let property_start = index;
                if !bytes
                    .get(index)
                    .is_some_and(|candidate| is_if_part_identifier_start_byte(*candidate))
                {
                    return None;
                }
                index += 1;
                while index < bytes.len() && is_if_part_identifier_byte(bytes[index]) {
                    index += 1;
                }
                let Value::Object(object) = current else {
                    return None;
                };
                current = object.get(&variable_name[property_start..index])?;
            }
            b'[' => {
                index += 1;
                match bytes.get(index) {
                    Some(candidate) if candidate.is_ascii_digit() => {
                        let array_index_start = index;
                        while index < bytes.len() && bytes[index].is_ascii_digit() {
                            index += 1;
                        }
                        if array_index_start == index || bytes.get(index) != Some(&b']') {
                            return None;
                        }
                        let array_index = variable_name[array_index_start..index]
                            .parse::<usize>()
                            .ok()?;
                        index += 1;
                        let Value::Array(items) = current else {
                            return None;
                        };
                        current = items.get(array_index)?;
                    }
                    Some(quote @ (b'\'' | b'"')) => {
                        let quote = *quote;
                        index += 1;
                        let key_start = index;
                        while index < bytes.len() && bytes[index] != quote {
                            if bytes[index] == b'\\' {
                                return None;
                            }
                            index += 1;
                        }
                        if key_start == index
                            || bytes.get(index) != Some(&quote)
                            || bytes.get(index + 1) != Some(&b']')
                        {
                            return None;
                        }
                        let Value::Object(object) = current else {
                            return None;
                        };
                        current = object.get(&variable_name[key_start..index])?;
                        index += 2;
                    }
                    _ => return None,
                }
            }
            _ => return None,
        }
    }

    Some(current)
}

fn is_if_part_identifier_start_byte(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_'
}

fn is_if_part_identifier_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

fn if_part_variable_is_empty(actual: Option<&Value>) -> bool {
    match actual {
        None | Some(Value::Null) => true,
        Some(Value::String(value)) => value.is_empty(),
        Some(Value::Array(value)) => value.is_empty(),
        Some(Value::Object(value)) => value.is_empty(),
        Some(Value::Number(_)) | Some(Value::Bool(_)) => false,
    }
}

fn if_part_contains(collection: Option<&Value>, value: Option<&Value>) -> bool {
    match (collection, value) {
        (Some(Value::String(source)), Some(Value::String(needle))) => source.contains(needle),
        (Some(Value::Array(items)), Some(needle)) => items.iter().any(|item| item == needle),
        (Some(Value::Object(object)), Some(Value::String(key))) => object.contains_key(key),
        _ => false,
    }
}

fn if_part_number_compare(
    actual: Option<&IfPartComparableValue>,
    expected: Option<&IfPartComparableValue>,
) -> Option<i8> {
    let (
        Some(IfPartComparableValue::Number(actual)),
        Some(IfPartComparableValue::Number(expected)),
    ) = (actual, expected)
    else {
        return None;
    };
    if actual < expected {
        Some(-1)
    } else if actual > expected {
        Some(1)
    } else {
        Some(0)
    }
}

fn find_container_with_direct_plan_item<'a>(
    container: ContainerView<'a>,
    plan_item_id: &str,
) -> Option<ContainerView<'a>> {
    if container
        .plan_items
        .iter()
        .any(|plan_item| plan_item.id == plan_item_id)
    {
        return Some(container);
    }

    container.stages.iter().find_map(|stage| {
        find_container_with_direct_plan_item(ContainerView::from_stage(stage), plan_item_id)
    })
}

fn find_stage_by_definition_id<'a>(
    stages: &'a [CmmnStage],
    stage_id: &str,
) -> Option<&'a CmmnStage> {
    for stage in stages {
        if stage.id == stage_id {
            return Some(stage);
        }
        if let Some(nested_stage) = find_stage_by_definition_id(&stage.stages, stage_id) {
            return Some(nested_stage);
        }
    }
    None
}

fn find_activation_target_by_plan_item_id<'a>(
    case_plan_model: &'a CmmnCasePlanModel,
    plan_item_id: &str,
) -> Option<PlanItemActivationTarget<'a>> {
    find_activation_target_by_plan_item_id_in_container(
        case_plan_model.plan_items.as_slice(),
        case_plan_model.stages.as_slice(),
        case_plan_model.human_tasks.as_slice(),
        plan_item_id,
    )
}

fn find_activation_target_by_plan_item_id_in_container<'a>(
    plan_items: &'a [CmmnPlanItem],
    stages: &'a [CmmnStage],
    human_tasks: &'a [CmmnHumanTask],
    plan_item_id: &str,
) -> Option<PlanItemActivationTarget<'a>> {
    for plan_item in plan_items {
        if plan_item.id != plan_item_id {
            continue;
        }
        if let Some(human_task) = human_tasks
            .iter()
            .find(|human_task| human_task.id == plan_item.definition_ref)
        {
            return Some(PlanItemActivationTarget::HumanTask(plan_item, human_task));
        }
        if let Some(stage) = stages
            .iter()
            .find(|stage| stage.id == plan_item.definition_ref)
        {
            return Some(PlanItemActivationTarget::Stage(plan_item, stage));
        }
    }

    for stage in stages {
        if let Some(found) = find_activation_target_by_plan_item_id_in_container(
            stage.plan_items.as_slice(),
            stage.stages.as_slice(),
            stage.human_tasks.as_slice(),
            plan_item_id,
        ) {
            return Some(found);
        }
    }

    None
}

fn find_activation_target_by_definition_id<'a>(
    case_plan_model: &'a CmmnCasePlanModel,
    definition_id: &str,
) -> Option<PlanItemActivationTarget<'a>> {
    find_activation_target_by_definition_id_in_container(
        case_plan_model.plan_items.as_slice(),
        case_plan_model.stages.as_slice(),
        case_plan_model.human_tasks.as_slice(),
        definition_id,
    )
}

fn find_activation_target_by_definition_id_in_container<'a>(
    plan_items: &'a [CmmnPlanItem],
    stages: &'a [CmmnStage],
    human_tasks: &'a [CmmnHumanTask],
    definition_id: &str,
) -> Option<PlanItemActivationTarget<'a>> {
    for plan_item in plan_items {
        if plan_item.definition_ref != definition_id {
            continue;
        }
        if let Some(human_task) = human_tasks
            .iter()
            .find(|human_task| human_task.id == definition_id)
        {
            return Some(PlanItemActivationTarget::HumanTask(plan_item, human_task));
        }
        if let Some(stage) = stages.iter().find(|stage| stage.id == definition_id) {
            return Some(PlanItemActivationTarget::Stage(plan_item, stage));
        }
    }

    for stage in stages {
        if let Some(found) = find_activation_target_by_definition_id_in_container(
            stage.plan_items.as_slice(),
            stage.stages.as_slice(),
            stage.human_tasks.as_slice(),
            definition_id,
        ) {
            return Some(found);
        }
    }

    None
}

fn find_plan_item_definition_activation_target<'a>(
    case_plan_model: &'a CmmnCasePlanModel,
    definition_id: &str,
) -> Option<PlanItemDefinitionActivationTarget<'a>> {
    find_plan_item_definition_activation_target_in_container(
        ContainerView::from_case_plan_model(case_plan_model),
        definition_id,
    )
}

fn find_plan_item_definition_activation_target_in_container<'a>(
    container: ContainerView<'a>,
    definition_id: &str,
) -> Option<PlanItemDefinitionActivationTarget<'a>> {
    for plan_item in container.plan_items {
        if plan_item.definition_ref != definition_id {
            continue;
        }
        if let Some(human_task) = container
            .human_tasks
            .iter()
            .find(|human_task| human_task.id == definition_id)
        {
            return Some(PlanItemDefinitionActivationTarget::HumanTask(
                plan_item, human_task,
            ));
        }
        if let Some(stage) = container
            .stages
            .iter()
            .find(|stage| stage.id == definition_id)
        {
            return Some(PlanItemDefinitionActivationTarget::Stage(plan_item, stage));
        }
        if let Some(decision_task) = container
            .decision_tasks
            .iter()
            .find(|decision_task| decision_task.id == definition_id)
        {
            return Some(PlanItemDefinitionActivationTarget::DecisionTask(
                plan_item,
                decision_task,
            ));
        }
        if let Some(milestone) = container
            .milestones
            .iter()
            .find(|milestone| milestone.id == definition_id)
        {
            return Some(PlanItemDefinitionActivationTarget::Milestone(
                plan_item, milestone,
            ));
        }
        if let Some(event_listener) = container
            .event_listeners
            .iter()
            .find(|event_listener| event_listener.id == definition_id)
        {
            return Some(PlanItemDefinitionActivationTarget::EventListener(
                plan_item,
                event_listener,
            ));
        }
    }

    for stage in container.stages {
        if let Some(found) = find_plan_item_definition_activation_target_in_container(
            ContainerView::from_stage(stage),
            definition_id,
        ) {
            return Some(found);
        }
    }

    None
}

fn stage_instance_exists(
    session: &mut DbSession,
    case_instance_id: &str,
    plan_item_id: &str,
) -> Result<bool, CmmnError> {
    let mut params = DbParams::new();
    params.push(case_instance_id);
    let rendered = RenderedStatement::new(
        "SELECT DATA_ FROM ACT_CMMN_STAGE_INSTANCE WHERE CASE_INSTANCE_ID_ = ?".to_string(),
        params,
    );
    let rows = session.select_raw(rendered)?;
    let stages = rows
        .into_iter()
        .map(|row| {
            let json = row
                .get_text("DATA_")
                .ok_or_else(|| CmmnError::storage("Missing DATA_ in CMMN stage instance row"))?;
            serde_json::from_str::<CmmnStageInstance>(&json).map_err(CmmnError::from)
        })
        .collect::<Result<Vec<_>, _>>()?;

    Ok(stages
        .iter()
        .any(|stage| stage.plan_item_id == plan_item_id))
}

fn open_stage_instance_exists(
    session: &mut DbSession,
    case_instance_id: &str,
    plan_item_id: &str,
) -> Result<bool, CmmnError> {
    let mut params = DbParams::new();
    params.push(case_instance_id);
    let rendered = RenderedStatement::new(
        "SELECT DATA_ FROM ACT_CMMN_STAGE_INSTANCE \
         WHERE CASE_INSTANCE_ID_ = ? AND STATE_ <> 'COMPLETED' AND STATE_ <> 'TERMINATED'"
            .to_string(),
        params,
    );
    let rows = session.select_raw(rendered)?;
    let stages = rows
        .into_iter()
        .map(|row| {
            let json = row
                .get_text("DATA_")
                .ok_or_else(|| CmmnError::storage("Missing DATA_ in CMMN stage instance row"))?;
            serde_json::from_str::<CmmnStageInstance>(&json).map_err(CmmnError::from)
        })
        .collect::<Result<Vec<_>, _>>()?;

    Ok(stages
        .iter()
        .any(|stage| stage.plan_item_id == plan_item_id))
}

fn open_human_task_instance_exists(
    session: &mut DbSession,
    case_instance_id: &str,
    plan_item_id: &str,
) -> Result<bool, CmmnError> {
    let mut params = DbParams::new();
    params.push(case_instance_id);
    let rendered = RenderedStatement::new(
        "SELECT DATA_ FROM ACT_CMMN_HUMAN_TASK \
         WHERE CASE_INSTANCE_ID_ = ? AND STATE_ <> 'COMPLETED'"
            .to_string(),
        params,
    );
    let rows = session.select_raw(rendered)?;
    let tasks = rows
        .into_iter()
        .map(|row| {
            let json = row
                .get_text("DATA_")
                .ok_or_else(|| CmmnError::storage("Missing DATA_ in CMMN human task row"))?;
            serde_json::from_str::<CmmnHumanTaskInstance>(&json).map_err(CmmnError::from)
        })
        .collect::<Result<Vec<_>, _>>()?;

    Ok(tasks.iter().any(|task| task.plan_item_id == plan_item_id))
}

fn open_human_task_instance_exists_in_stage(
    session: &mut DbSession,
    case_instance_id: &str,
    stage_instance_id: &str,
    plan_item_id: &str,
) -> Result<bool, CmmnError> {
    let mut params = DbParams::new();
    params.push(case_instance_id);
    params.push(stage_instance_id);
    let rendered = RenderedStatement::new(
        "SELECT DATA_ FROM ACT_CMMN_HUMAN_TASK \
         WHERE CASE_INSTANCE_ID_ = ? AND STAGE_INSTANCE_ID_ = ? AND STATE_ <> 'COMPLETED'"
            .to_string(),
        params,
    );
    let rows = session.select_raw(rendered)?;
    let tasks = rows
        .into_iter()
        .map(|row| {
            let json = row
                .get_text("DATA_")
                .ok_or_else(|| CmmnError::storage("Missing DATA_ in CMMN human task row"))?;
            serde_json::from_str::<CmmnHumanTaskInstance>(&json).map_err(CmmnError::from)
        })
        .collect::<Result<Vec<_>, _>>()?;

    Ok(tasks.iter().any(|task| task.plan_item_id == plan_item_id))
}

fn human_task_plan_item_reached_standard_event(
    session: &mut DbSession,
    case_instance_id: &str,
    plan_item_id: &str,
    standard_event: &str,
) -> Result<bool, CmmnError> {
    let expected_state = match standard_event {
        CmmnPlanItemOnPart::STANDARD_EVENT_COMPLETE => CmmnHumanTaskState::Completed,
        CmmnPlanItemOnPart::STANDARD_EVENT_TERMINATE => CmmnHumanTaskState::Terminated,
        _ => return Ok(false),
    };

    let mut params = DbParams::new();
    params.push(case_instance_id);
    let rendered = RenderedStatement::new(
        "SELECT DATA_ FROM ACT_CMMN_HUMAN_TASK WHERE CASE_INSTANCE_ID_ = ?".to_string(),
        params,
    );
    let rows = session.select_raw(rendered)?;
    let tasks = rows
        .into_iter()
        .map(|row| {
            let json = row
                .get_text("DATA_")
                .ok_or_else(|| CmmnError::storage("Missing DATA_ in CMMN human task row"))?;
            serde_json::from_str::<CmmnHumanTaskInstance>(&json).map_err(CmmnError::from)
        })
        .collect::<Result<Vec<_>, _>>()?;

    if tasks
        .iter()
        .any(|task| task.plan_item_id == plan_item_id && task.state == expected_state)
    {
        return Ok(true);
    }

    let mut params = DbParams::new();
    params.push(case_instance_id);
    let rendered = RenderedStatement::new(
        "SELECT DATA_ FROM ACT_CMMN_HUMAN_TASK_HISTORY WHERE CASE_INSTANCE_ID_ = ?".to_string(),
        params,
    );
    let rows = session.select_raw(rendered)?;
    let historic_tasks = rows
        .into_iter()
        .map(|row| {
            let json = row.get_text("DATA_").ok_or_else(|| {
                CmmnError::storage("Missing DATA_ in CMMN historic human task row")
            })?;
            serde_json::from_str::<CmmnHistoricHumanTaskInstance>(&json).map_err(CmmnError::from)
        })
        .collect::<Result<Vec<_>, _>>()?;

    Ok(historic_tasks
        .iter()
        .any(|task| task.plan_item_id == plan_item_id && task.state == expected_state))
}

fn stage_instance_is_active(
    session: &mut DbSession,
    stage_instance_id: &str,
) -> Result<bool, CmmnError> {
    Ok(load_stage_instance_session(session, stage_instance_id)?
        .is_some_and(|stage_instance| stage_instance.state == CmmnStageInstanceState::Active))
}

fn plan_item_standard_event_occurred(
    session: &mut DbSession,
    case_instance_id: &str,
    plan_item_id: &str,
    standard_event: &str,
) -> Result<bool, CmmnError> {
    let mut params = DbParams::new();
    params.push(case_instance_id);
    params.push(plan_item_id);
    params.push(standard_event);
    let rendered = RenderedStatement::new(
        "SELECT COUNT(*) AS CNT FROM ACT_CMMN_PLAN_ITEM_EVENT \
         WHERE CASE_INSTANCE_ID_ = ? AND PLAN_ITEM_ID_ = ? AND STANDARD_EVENT_ = ?"
            .to_string(),
        params,
    );
    let row = session
        .select_one_raw(rendered)?
        .ok_or_else(|| CmmnError::storage("Missing COUNT row in plan item event query"))?;
    let count = row.get_integer("CNT").unwrap_or(0);
    Ok(count > 0)
}

/// Materialize a manual-activation plan item in its real ENABLED lifecycle state.
/// Java creates a plan-item instance first and then runs the enable operation
/// (`ActivatePlanItemInstanceOperation.java:48-55` and
/// `EnablePlanItemInstanceOperation.java:39-51`). Rust types without a dedicated
/// runtime table use the unified mirror as that durable instance.
fn persist_enabled_plan_item_instance_session(
    session: &mut DbSession,
    case_definition: &CmmnCaseDefinition,
    case_instance: &CmmnCaseInstance,
    plan_item: &CmmnPlanItem,
    parent_stage_instance_id: Option<&str>,
    plan_item_definition_type: &str,
) -> Result<(), CmmnError> {
    let enabled_at = Utc::now();
    if let Some(mut instance) = load_plan_item_instances_session(session)?
        .into_iter()
        .find(|instance| {
            instance.case_instance_id == case_instance.id
                && instance.plan_item_id == plan_item.id
                && instance.plan_item_definition_type == plan_item_definition_type
                && instance.stage_instance_id.as_deref() == parent_stage_instance_id
                && instance.ended_at.is_none()
        })
    {
        if instance.state != "ENABLED" {
            instance.state = "ENABLED".to_string();
            instance.last_enabled_at = Some(enabled_at);
            persist_plan_item_instance_session(session, &instance)?;
        }
        return Ok(());
    }
    persist_plan_item_instance_session(
        session,
        &CmmnPlanItemInstance {
            id: format!("cmmn-plan-item-instance:{}", Uuid::new_v4()),
            case_instance_id: case_instance.id.clone(),
            case_definition_id: case_definition.id.clone(),
            stage_instance_id: parent_stage_instance_id.map(str::to_string),
            plan_item_id: plan_item.id.clone(),
            plan_item_definition_id: plan_item.definition_ref.clone(),
            plan_item_definition_type: plan_item_definition_type.to_string(),
            name: plan_item
                .name
                .clone()
                .unwrap_or_else(|| plan_item.definition_ref.clone()),
            state: "ENABLED".to_string(),
            created_at: enabled_at,
            last_enabled_at: Some(enabled_at),
            ended_at: None,
            occurred_at: None,
            assignee: None,
            tenant_id: case_instance.tenant_id.clone(),
        },
    )
}

fn enabled_plan_item_instance_exists(
    session: &mut DbSession,
    case_instance_id: &str,
    plan_item_id: &str,
) -> Result<bool, CmmnError> {
    Ok(load_plan_item_instances_session(session)?
        .iter()
        .any(|instance| {
            instance.case_instance_id == case_instance_id
                && instance.plan_item_id == plan_item_id
                && instance.state == "ENABLED"
                && instance.ended_at.is_none()
        }))
}

/// Count mirror plan-item rows that block non-autocomplete container completion.
///
/// Java `PlanItemInstanceContainerUtil.java:143-146`: AVAILABLE and ENABLED children
/// only allow automatic completion when `containerIsAutocomplete` is true
/// (`shouldBeCompleted = shouldBeCompleted && containerIsAutocomplete`). Rust keeps
/// ACTIVE children in dedicated tables (human tasks, stages, task associations) and
/// AVAILABLE user-event subscriptions in `ACT_CMMN_EVENT_SUBSCRIPTION`. The unified
/// mirror still holds:
/// - ENABLED rows (manual-activation) for every definition type that lacks a dedicated
///   ENABLED table — always counted here.
/// - AVAILABLE timer/event-listener rows — counted here so a lone AVAILABLE
///   `timerEventListener` (which never writes an event-subscription row) keeps the
///   container open. Double-counting user event listeners that also own an
///   event-subscription row is harmless (any non-zero bucket blocks).
///
/// AVAILABLE milestones are intentionally not counted here: required ones already
/// block via `has_incomplete_required_plan_items` (Java :102-118), and counting
/// optional AVAILABLE milestones would change the pre-P139 c8 contract that only
/// required waiting milestones block. Stages themselves are excluded
/// (`plan_item_definition_type != "stage"`) because open stages are counted via
/// `ACT_CMMN_STAGE_INSTANCE`. Terminal rows are filtered by `ended_at.is_none()`.
fn count_blocking_mirror_plan_items(
    session: &mut DbSession,
    case_instance_id: &str,
    parent_stage_instance_id: Option<&str>,
) -> Result<i64, CmmnError> {
    Ok(load_plan_item_instances_session(session)?
        .into_iter()
        .filter(|instance| {
            instance.case_instance_id == case_instance_id
                && instance.plan_item_definition_type != "stage"
                && instance.ended_at.is_none()
                && mirror_state_blocks_non_autocomplete(&instance.state, &instance.plan_item_definition_type)
                && match parent_stage_instance_id {
                    Some(stage_id) => instance.stage_instance_id.as_deref() == Some(stage_id),
                    None => instance.stage_instance_id.is_none(),
                }
        })
        .count() as i64)
}

/// Whether a non-ended mirror row's state blocks a non-autocomplete container.
/// ENABLED always blocks (PlanItemInstanceContainerUtil.java:143-146). AVAILABLE
/// blocks only for event-listener definition types (timer has no other open bucket).
fn mirror_state_blocks_non_autocomplete(state: &str, plan_item_definition_type: &str) -> bool {
    match state {
        "ENABLED" => true,
        "AVAILABLE" => {
            plan_item_definition_type == PLAN_ITEM_DEFINITION_TYPE_TIMER_EVENT_LISTENER
                || plan_item_definition_type == PLAN_ITEM_DEFINITION_TYPE_EVENT_LISTENER
        }
        _ => false,
    }
}

fn delete_enabled_plan_item_instance_rows(
    session: &mut DbSession,
    case_instance_id: &str,
    plan_item_id: &str,
) -> Result<(), CmmnError> {
    let data_manager = CmmnPlanItemInstanceDataManager::new();
    for instance in load_plan_item_instances_session(session)? {
        if instance.case_instance_id == case_instance_id
            && instance.plan_item_id == plan_item_id
            && instance.state == "ENABLED"
            && instance.ended_at.is_none()
        {
            data_manager.delete(session, &plan_item_instance_entity_from_model(instance))?;
        }
    }
    Ok(())
}

fn list_stage_overview(
    store: &CmmnStore,
    case_instance_id: &str,
) -> Result<Vec<CmmnStageOverview>, CmmnError> {
    let mut session = store.create_session()?;
    let mut params = DbParams::new();
    params.push(case_instance_id);
    let rendered = RenderedStatement::new(
        "SELECT DATA_ FROM ACT_CMMN_STAGE_INSTANCE \
         WHERE CASE_INSTANCE_ID_ = ? \
         ORDER BY ACTIVATED_AT_ ASC, ID_ ASC"
            .to_string(),
        params,
    );
    let rows = session.select_raw(rendered)?;
    rows.into_iter()
        .map(|row| {
            let json = row
                .get_text("DATA_")
                .ok_or_else(|| CmmnError::storage("Missing DATA_ in CMMN stage instance row"))?;
            serde_json::from_str::<CmmnStageInstance>(&json)
                .map_err(CmmnError::from)
                .map(stage_overview_from_stage_instance)
        })
        .collect()
}

fn activate_process_task(
    session: &mut DbSession,
    case_definition: &CmmnCaseDefinition,
    case_instance: &CmmnCaseInstance,
    plan_item: &CmmnPlanItem,
    process_task: &CmmnProcessTask,
    parent_stage_instance_id: Option<&str>,
    process_task_runner: Option<&Arc<dyn CmmnProcessTaskRunner>>,
) -> Result<(), CmmnError> {
    if task_association_exists(
        session,
        &case_instance.id,
        &plan_item.id,
        parent_stage_instance_id,
    )? {
        return Ok(());
    }
    let process_ref = process_task.process_ref.as_deref().ok_or_else(|| {
        CmmnError::validation(format!(
            "CMMN process task '{}' must declare a process reference",
            process_task.id
        ))
    })?;
    let runner = process_task_runner.ok_or_else(|| {
        CmmnError::unsupported(
            "processTask runtime",
            format!(
                "case '{}' process task '{}' cannot start process '{}' without a configured process task runner",
                case_definition.key, process_task.id, process_ref
            ),
        )
    })?;

    let start_result = runner.start_process(CmmnProcessTaskStartRequest {
        process_definition_key: process_ref.to_string(),
        parent_case_instance_id: case_instance.id.clone(),
        parent_case_definition_id: case_definition.id.clone(),
        parent_case_definition_key: case_definition.key.clone(),
        parent_plan_item_id: plan_item.id.clone(),
        parent_task_definition_id: process_task.id.clone(),
        business_key: child_task_business_key(process_task.business_key.as_deref(), case_instance),
        tenant_id: case_instance.tenant_id.clone(),
        variables: child_task_in_variables(&process_task.in_parameters, case_instance),
    })?;

    let state = if process_task.is_blocking && !start_result.completed {
        CmmnTaskAssociationState::Active
    } else {
        CmmnTaskAssociationState::Completed
    };
    let mut association = CmmnTaskInstanceAssociation {
        id: format!("cmmn-task-association:{}", Uuid::new_v4()),
        kind: CmmnTaskAssociationKind::ProcessTask,
        state,
        case_instance_id: case_instance.id.clone(),
        case_definition_id: case_definition.id.clone(),
        case_definition_key: case_definition.key.clone(),
        stage_instance_id: parent_stage_instance_id.map(str::to_string),
        plan_item_id: plan_item.id.clone(),
        task_definition_id: process_task.id.clone(),
        child_definition_key: process_ref.to_string(),
        child_instance_id: Some(start_result.process_instance_id),
        created_at: Utc::now(),
        completed_at: None,
        failure_message: None,
    };
    if association.state == CmmnTaskAssociationState::Completed {
        association.completed_at = Some(Utc::now());
    }
    persist_task_association_session(session, &association)?;

    if association.state == CmmnTaskAssociationState::Completed {
        complete_task_plan_item_event_session(
            session,
            case_definition,
            &association,
            CmmnPlanItemOnPart::STANDARD_EVENT_COMPLETE,
        )?;
    }
    Ok(())
}

fn activate_case_task(
    session: &mut DbSession,
    case_definition: &CmmnCaseDefinition,
    case_instance: &CmmnCaseInstance,
    plan_item: &CmmnPlanItem,
    case_task: &CmmnCaseTask,
    parent_stage_instance_id: Option<&str>,
) -> Result<(), CmmnError> {
    if task_association_exists(
        session,
        &case_instance.id,
        &plan_item.id,
        parent_stage_instance_id,
    )? {
        return Ok(());
    }
    let case_ref = case_task.case_ref.as_deref().ok_or_else(|| {
        CmmnError::validation(format!(
            "CMMN case task '{}' must declare a case reference",
            case_task.id
        ))
    })?;
    let child_definition = latest_case_definition_by_key_session(
        session,
        case_ref,
        case_instance.tenant_id.as_deref(),
    )?;
    let mut child_start_request =
        CmmnCaseInstanceStartRequest::new().with_variables(Value::Object(child_task_in_variables(
            &case_task.in_parameters,
            case_instance,
        )));
    if let Some(business_key) =
        child_task_business_key(case_task.business_key.as_deref(), case_instance)
    {
        child_start_request = child_start_request.with_business_key(business_key);
    }
    if let Some(started_by) = case_instance.started_by.clone() {
        child_start_request = child_start_request.with_started_by(started_by);
    }
    if let Some(tenant_id) = case_instance.tenant_id.clone() {
        child_start_request = child_start_request.with_tenant_id(tenant_id);
    }
    let child_case_instance =
        start_case_instance_session(session, &child_definition, child_start_request, None)?;

    let state =
        if case_task.is_blocking && child_case_instance.state != CmmnCaseInstanceState::Completed {
            CmmnTaskAssociationState::Active
        } else {
            CmmnTaskAssociationState::Completed
        };
    let mut association = CmmnTaskInstanceAssociation {
        id: format!("cmmn-task-association:{}", Uuid::new_v4()),
        kind: CmmnTaskAssociationKind::CaseTask,
        state,
        case_instance_id: case_instance.id.clone(),
        case_definition_id: case_definition.id.clone(),
        case_definition_key: case_definition.key.clone(),
        stage_instance_id: parent_stage_instance_id.map(str::to_string),
        plan_item_id: plan_item.id.clone(),
        task_definition_id: case_task.id.clone(),
        child_definition_key: case_ref.to_string(),
        child_instance_id: Some(child_case_instance.id),
        created_at: Utc::now(),
        completed_at: None,
        failure_message: None,
    };
    if association.state == CmmnTaskAssociationState::Completed {
        association.completed_at = Some(Utc::now());
    }
    persist_task_association_session(session, &association)?;

    if association.state == CmmnTaskAssociationState::Completed {
        complete_task_plan_item_event_session(
            session,
            case_definition,
            &association,
            CmmnPlanItemOnPart::STANDARD_EVENT_COMPLETE,
        )?;
    }
    Ok(())
}

// Java parity: ChildTaskActivityBehavior.java:89-104 — an explicit business key on the child
// task wins over inheritance from the parent case. Deviation kept for baseline compatibility:
// without an explicit key the parent business key is always inherited, whereas Java only
// inherits when inheritBusinessKey=true.
fn child_task_business_key(
    explicit_business_key: Option<&str>,
    case_instance: &CmmnCaseInstance,
) -> Option<String> {
    explicit_business_key
        .map(str::to_string)
        .or_else(|| case_instance.business_key.clone())
}

// Java parity: CaseTaskActivityBehavior.java:97-98 / ProcessTaskActivityBehavior.java:87-88 —
// declared in-parameters build the child variable map from scratch (IOParameterUtil.java:56-92).
// Deviation kept for baseline compatibility: without declared in-parameters the full parent
// variable map keeps being passed (Java passes no variables at all in that case).
fn child_task_in_variables(
    in_parameters: &[CmmnIOParameter],
    case_instance: &CmmnCaseInstance,
) -> Map<String, Value> {
    if in_parameters.is_empty() {
        case_instance.variables.clone()
    } else {
        map_io_parameters(in_parameters, &case_instance.variables)
    }
}

// Java parity: IOParameterUtil.java:56-92 — copy each declared parameter from the source
// variables to the target name; a missing source variable still writes the target with a null
// value (IOParameterUtil.java:64-66 resolves it to null before assignment).
fn map_io_parameters(
    parameters: &[CmmnIOParameter],
    source_variables: &Map<String, Value>,
) -> Map<String, Value> {
    let mut mapped = Map::new();
    for parameter in parameters {
        let (Some(source), Some(target)) =
            (parameter.source.as_deref(), parameter.target.as_deref())
        else {
            continue;
        };
        let value = source_variables.get(source).cloned().unwrap_or(Value::Null);
        mapped.insert(target.to_string(), value);
    }
    mapped
}

fn complete_decision_task(
    session: &mut DbSession,
    case_definition: &CmmnCaseDefinition,
    case_instance: &CmmnCaseInstance,
    plan_item: &CmmnPlanItem,
    _decision_task: &CmmnDecisionTask,
    parent_stage_instance_id: Option<&str>,
) -> Result<(), CmmnError> {
    // Mirror human-task completion: record COMPLETE at most once, always fan out handlers,
    // then evaluate repetition so a matching rule re-queues the plan item as available.
    if !plan_item_standard_event_occurred(
        session,
        &case_instance.id,
        &plan_item.id,
        CmmnPlanItemOnPart::STANDARD_EVENT_COMPLETE,
    )? {
        record_plan_item_standard_event_session(
            session,
            &case_instance.id,
            &plan_item.id,
            CmmnPlanItemOnPart::STANDARD_EVENT_COMPLETE,
        )?;
    }
    handle_plan_item_standard_event(
        session,
        case_definition,
        &case_instance.id,
        &plan_item.id,
        CmmnPlanItemOnPart::STANDARD_EVENT_COMPLETE,
        parent_stage_instance_id,
    )?;
    repeat_decision_task_if_needed(
        session,
        case_definition,
        case_instance,
        plan_item,
        parent_stage_instance_id,
    )
}

fn repeat_decision_task_if_needed(
    session: &mut DbSession,
    case_definition: &CmmnCaseDefinition,
    case_instance: &CmmnCaseInstance,
    plan_item: &CmmnPlanItem,
    parent_stage_instance_id: Option<&str>,
) -> Result<(), CmmnError> {
    let case_instance =
        load_case_instance_session(session, &case_instance.id)?.ok_or_else(|| {
            CmmnError::storage(format!(
                "CMMN case instance '{}' disappeared during decision-task repetition evaluation",
                case_instance.id
            ))
        })?;
    if !repetition_rule_matches(plan_item, &case_instance)? {
        return Ok(());
    }
    if let Some(stage_instance_id) = parent_stage_instance_id
        && !stage_instance_is_active(session, stage_instance_id)?
    {
        return Ok(());
    }
    // Decision tasks have no dedicated runtime table. Java creates a new plan-item
    // instance for repetition; keep that instance in ENABLED until explicitly
    // started (ActivatePlanItemInstanceOperation.java:48-55).
    persist_enabled_plan_item_instance_session(
        session,
        case_definition,
        &case_instance,
        plan_item,
        parent_stage_instance_id,
        "decisiontask",
    )
}

fn reach_milestone(
    session: &mut DbSession,
    case_definition: &CmmnCaseDefinition,
    case_instance: &CmmnCaseInstance,
    plan_item: &CmmnPlanItem,
    milestone: &CmmnMilestone,
    parent_stage_instance_id: Option<&str>,
) -> Result<(), CmmnError> {
    if plan_item_standard_event_occurred(
        session,
        &case_instance.id,
        &plan_item.id,
        CmmnPlanItemOnPart::STANDARD_EVENT_OCCUR,
    )? {
        return Ok(());
    }

    // Java parity: MilestoneActivityBehavior.java:47-61 — on reach, a declared milestoneVariable
    // is set to true on the case instance (:51) and a declared businessStatus updates the case
    // business status (:59), both before the occur operation is planned (:64) so downstream
    // sentry ifParts observe the new values.
    if milestone.milestone_variable.is_some() || milestone.business_status.is_some() {
        let mut refreshed_case = load_case_instance_session(session, &case_instance.id)?
            .ok_or_else(|| {
                CmmnError::storage(format!(
                    "CMMN case instance '{}' disappeared while reaching milestone '{}'",
                    case_instance.id, milestone.id
                ))
            })?;
        if let Some(variable_name) = &milestone.milestone_variable {
            refreshed_case
                .variables
                .insert(variable_name.clone(), Value::Bool(true));
        }
        if let Some(business_status) = &milestone.business_status {
            refreshed_case.business_status = Some(business_status.clone());
        }
        persist_case_instance_session(session, &refreshed_case)?;
    }

    let historic_milestone = CmmnHistoricMilestoneInstance {
        id: format!("cmmn-historic-milestone:{}", Uuid::new_v4()),
        case_instance_id: case_instance.id.clone(),
        case_definition_id: case_definition.id.clone(),
        case_definition_key: case_definition.key.clone(),
        milestone_id: milestone.id.clone(),
        name: plan_item
            .name
            .clone()
            .unwrap_or_else(|| milestone.name.clone()),
        tenant_id: case_instance.tenant_id.clone(),
        time: Utc::now(),
    };
    persist_historic_milestone_session(session, &historic_milestone)?;

    // Java's milestone plan item instance reaches COMPLETED with occurredTime +
    // endedTime set on occur (OccurPlanItemInstanceOperation.java:34-63). Reuse a
    // materialized AVAILABLE (or manual-activation ENABLED) instance. The fallback
    // keeps older/internal callers idempotent when they did not activate a container first.
    let occurred_at = Utc::now();
    let mut milestone_instance = load_plan_item_instances_session(session)?
        .into_iter()
        .find(|instance| {
            instance.case_instance_id == case_instance.id
                && instance.plan_item_id == plan_item.id
                && instance.plan_item_definition_type == "milestone"
                && instance.stage_instance_id.as_deref() == parent_stage_instance_id
                && instance.ended_at.is_none()
        })
        .unwrap_or_else(|| CmmnPlanItemInstance {
            id: format!("cmmn-plan-item-instance:{}", Uuid::new_v4()),
            case_instance_id: case_instance.id.clone(),
            case_definition_id: case_definition.id.clone(),
            stage_instance_id: parent_stage_instance_id.map(str::to_string),
            plan_item_id: plan_item.id.clone(),
            plan_item_definition_id: milestone.id.clone(),
            plan_item_definition_type: "milestone".to_string(),
            name: plan_item
                .name
                .clone()
                .unwrap_or_else(|| milestone.name.clone()),
            state: "AVAILABLE".to_string(),
            created_at: occurred_at,
            last_enabled_at: None,
            ended_at: None,
            occurred_at: None,
            assignee: None,
            tenant_id: case_instance.tenant_id.clone(),
        });
    let source_state = milestone_instance.state.clone();

    // AbstractMovePlanItemInstanceToTerminalStateOperation.java:90-103 delegates
    // every terminal transition through the lifecycle notification helper.
    fire_plan_item_lifecycle_listeners_for_model(
        &case_definition.model,
        case_instance,
        Some(&milestone_instance.id),
        &milestone.id,
        Some("milestone"),
        &source_state,
        "completed",
    )?;
    milestone_instance.state = "COMPLETED".to_string();
    milestone_instance.ended_at = Some(occurred_at);
    milestone_instance.occurred_at = Some(occurred_at);
    persist_plan_item_instance_session(session, &milestone_instance)?;

    record_plan_item_standard_event_session(
        session,
        &case_instance.id,
        &plan_item.id,
        CmmnPlanItemOnPart::STANDARD_EVENT_OCCUR,
    )?;
    handle_plan_item_standard_event(
        session,
        case_definition,
        &case_instance.id,
        &plan_item.id,
        CmmnPlanItemOnPart::STANDARD_EVENT_OCCUR,
        parent_stage_instance_id,
    )
}

fn activate_stage(
    session: &mut DbSession,
    case_definition: &CmmnCaseDefinition,
    case_instance: &CmmnCaseInstance,
    plan_item: &CmmnPlanItem,
    stage: &CmmnStage,
    parent_stage_instance_id: Option<&str>,
) -> Result<(), CmmnError> {
    let state = if manual_activation_rule_matches(plan_item, case_instance)? {
        CmmnStageInstanceState::Enabled
    } else {
        CmmnStageInstanceState::Active
    };
    create_stage_instance(
        session,
        case_definition,
        case_instance,
        plan_item,
        stage,
        parent_stage_instance_id,
        state,
    )
}

fn create_stage_instance(
    session: &mut DbSession,
    case_definition: &CmmnCaseDefinition,
    case_instance: &CmmnCaseInstance,
    plan_item: &CmmnPlanItem,
    stage: &CmmnStage,
    parent_stage_instance_id: Option<&str>,
    state: CmmnStageInstanceState,
) -> Result<(), CmmnError> {
    let stage_instance = CmmnStageInstance {
        id: format!("cmmn-stage-instance:{}", Uuid::new_v4()),
        case_instance_id: case_instance.id.clone(),
        case_definition_id: case_definition.id.clone(),
        parent_stage_instance_id: parent_stage_instance_id.map(str::to_string),
        plan_item_id: plan_item.id.clone(),
        stage_definition_id: stage.id.clone(),
        name: plan_item.name.clone().unwrap_or_else(|| stage.name.clone()),
        activated_at: Utc::now(),
        ended_at: None,
        state,
    };

    persist_stage_instance_session(session, &stage_instance)?;
    if stage_instance.state != CmmnStageInstanceState::Active {
        return Ok(());
    }

    // `start` fires when a stage plan item enters Active (mirrors human-task start).
    if !plan_item_standard_event_occurred(
        session,
        &case_instance.id,
        &plan_item.id,
        CmmnPlanItemOnPart::STANDARD_EVENT_START,
    )? {
        record_plan_item_standard_event_session(
            session,
            &case_instance.id,
            &plan_item.id,
            CmmnPlanItemOnPart::STANDARD_EVENT_START,
        )?;
    }
    handle_plan_item_standard_event(
        session,
        case_definition,
        &case_instance.id,
        &plan_item.id,
        CmmnPlanItemOnPart::STANDARD_EVENT_START,
        parent_stage_instance_id,
    )?;

    start_stage_instance(
        session,
        case_definition,
        case_instance,
        stage_instance,
        stage,
    )
}

fn start_stage_instance(
    session: &mut DbSession,
    case_definition: &CmmnCaseDefinition,
    case_instance: &CmmnCaseInstance,
    mut stage_instance: CmmnStageInstance,
    stage: &CmmnStage,
) -> Result<(), CmmnError> {
    if stage_instance.state != CmmnStageInstanceState::Active {
        // P126: a stage is a plan item definition, so it gets `planItemLifecycleListener`
        // notifications like any other (StageActivityBehavior.java:99 →
        // CmmnListenerNotificationHelper.executeLifecycleListeners).
        fire_plan_item_lifecycle_listeners_session(
            session,
            &stage_instance.case_instance_id,
            Some(&stage_instance.id),
            &stage_instance.stage_definition_id,
            Some("stage"),
            stage_instance.state.as_str(),
            CmmnStageInstanceState::Active.as_str(),
        )?;
        stage_instance.state = CmmnStageInstanceState::Active;
        persist_stage_instance_session(session, &stage_instance)?;
    }
    activate_container(
        session,
        case_definition,
        case_instance,
        ContainerView::from_stage(stage),
        Some(stage_instance.id.as_str()),
        None,
    )?;
    maybe_complete_stage(session, case_definition, &stage_instance.id)?;
    Ok(())
}

fn activate_human_task(
    session: &mut DbSession,
    case_definition: &CmmnCaseDefinition,
    case_instance: &CmmnCaseInstance,
    plan_item: &CmmnPlanItem,
    human_task: &CmmnHumanTask,
    parent_stage_instance_id: Option<&str>,
) -> Result<(), CmmnError> {
    let state = if manual_activation_rule_matches(plan_item, case_instance)? {
        CmmnHumanTaskState::Enabled
    } else {
        CmmnHumanTaskState::Active
    };
    // Java parity: HumanTaskActivityBehavior.java:83,173-177 — a non-blocking human
    // task creates no task entry and its plan item completes immediately (manual
    // task semantics). Manual activation parks the plan item as ENABLED
    // (ActivatePlanItemInstanceOperation.java:48-55).
    if state == CmmnHumanTaskState::Active && !human_task.blocking {
        return complete_non_blocking_human_task(
            session,
            case_definition,
            case_instance,
            plan_item,
            parent_stage_instance_id,
        );
    }
    let is_active = state == CmmnHumanTaskState::Active;
    create_human_task_instance(
        session,
        case_definition,
        case_instance,
        plan_item,
        human_task,
        parent_stage_instance_id,
        state,
    )?;
    if is_active {
        if !plan_item_standard_event_occurred(
            session,
            &case_instance.id,
            &plan_item.id,
            CmmnPlanItemOnPart::STANDARD_EVENT_START,
        )? {
            record_plan_item_standard_event_session(
                session,
                &case_instance.id,
                &plan_item.id,
                CmmnPlanItemOnPart::STANDARD_EVENT_START,
            )?;
        }
        handle_plan_item_standard_event(
            session,
            case_definition,
            &case_instance.id,
            &plan_item.id,
            CmmnPlanItemOnPart::STANDARD_EVENT_START,
            parent_stage_instance_id,
        )?;
    }
    Ok(())
}

// Java parity: HumanTaskActivityBehavior.java:173-177 — the non-blocking branch
// plans an immediate plan-item completion without inserting a task. Mirrors the
// decision-task completion shape: record start/complete at most once, always fan
// out the standard-event handlers, and let the enclosing activation path run the
// stage/case completion evaluation.
fn complete_non_blocking_human_task(
    session: &mut DbSession,
    case_definition: &CmmnCaseDefinition,
    case_instance: &CmmnCaseInstance,
    plan_item: &CmmnPlanItem,
    parent_stage_instance_id: Option<&str>,
) -> Result<(), CmmnError> {
    for standard_event in [
        CmmnPlanItemOnPart::STANDARD_EVENT_START,
        CmmnPlanItemOnPart::STANDARD_EVENT_COMPLETE,
    ] {
        if !plan_item_standard_event_occurred(
            session,
            &case_instance.id,
            &plan_item.id,
            standard_event,
        )? {
            record_plan_item_standard_event_session(
                session,
                &case_instance.id,
                &plan_item.id,
                standard_event,
            )?;
        }
        handle_plan_item_standard_event(
            session,
            case_definition,
            &case_instance.id,
            &plan_item.id,
            standard_event,
            parent_stage_instance_id,
        )?;
    }
    Ok(())
}

fn create_human_task_instance(
    session: &mut DbSession,
    case_definition: &CmmnCaseDefinition,
    case_instance: &CmmnCaseInstance,
    plan_item: &CmmnPlanItem,
    human_task: &CmmnHumanTask,
    parent_stage_instance_id: Option<&str>,
    state: CmmnHumanTaskState,
) -> Result<(), CmmnError> {
    let task = CmmnHumanTaskInstance {
        id: format!("cmmn-human-task:{}", Uuid::new_v4()),
        case_instance_id: case_instance.id.clone(),
        case_definition_id: case_definition.id.clone(),
        case_definition_key: case_definition.key.clone(),
        stage_instance_id: parent_stage_instance_id.map(str::to_string),
        plan_item_id: plan_item.id.clone(),
        task_definition_id: human_task.id.clone(),
        name: plan_item
            .name
            .clone()
            .unwrap_or_else(|| human_task.name.clone()),
        activated_at: Utc::now(),
        last_enabled_at: (state == CmmnHumanTaskState::Enabled).then(Utc::now),
        completed_at: None,
        completed_by: None,
        state,
        // Java parity: HumanTaskActivityBehavior.java:107-147 — assignee/owner/
        // priority/dueDate/category are expression-resolved against case variables
        // when written as `${…}` (P69 SimpleExpression); non-expression literals
        // stay verbatim (C10 fallback).
        assignee: resolve_el_or_literal_string(human_task.assignee.as_deref(), case_instance),
        owner: resolve_el_or_literal_string(human_task.owner.as_deref(), case_instance),
        priority: resolve_el_or_literal_string(human_task.priority.as_deref(), case_instance),
        due_date: resolve_el_or_literal_string(human_task.due_date.as_deref(), case_instance),
        category: resolve_el_or_literal_string(human_task.category.as_deref(), case_instance),
        delegation_state: None,
        task_local_variables: Map::new(),
    };

    persist_human_task_session(session, &task)?;
    persist_historic_human_task_session(session, &CmmnHistoricHumanTaskInstance::from(&task))?;

    // Java parity: HumanTaskActivityBehavior.java:146-147 — candidate users and
    // groups are stored as identity links on the created task. Only the active
    // (blocking) branch creates a task entity in Java, so available plan items
    // have no candidate links yet. Candidate entries may themselves be `${…}`
    // expressions that expand to comma-delimited lists after evaluation (P69).
    if task.state == CmmnHumanTaskState::Active {
        create_human_task_candidate_identity_links(session, &task.id, human_task, case_instance)?;
    }

    // Java parity: HumanTaskActivityBehavior.java:148,456-464 — after the task is
    // inserted, a declared taskIdVariableName stores the task id in a variable.
    // Only the active branch runs the behavior in Java (available plan items have
    // no task entity yet).
    if task.state == CmmnHumanTaskState::Active
        && let Some(variable_name) = &human_task.task_id_variable_name
    {
        let mut refreshed_case = load_case_instance_session(session, &case_instance.id)?
            .ok_or_else(|| {
                CmmnError::storage(format!(
                    "CMMN case instance '{}' disappeared while storing the task id variable",
                    case_instance.id
                ))
            })?;
        refreshed_case
            .variables
            .insert(variable_name.clone(), Value::String(task.id.clone()));
        persist_case_instance_session(session, &refreshed_case)?;
    }
    Ok(())
}

/// Creates `candidate` identity links scoped to a human task for each declared
/// candidate user and group. Java parity: HumanTaskActivityBehavior.java:146-147
/// (`handleCandidateUsers`/`handleCandidateGroups`), where each entry becomes an
/// `IdentityLinkType.CANDIDATE` link on the task.
fn create_human_task_candidate_identity_links(
    session: &mut DbSession,
    task_id: &str,
    human_task: &CmmnHumanTask,
    case_instance: &CmmnCaseInstance,
) -> Result<(), CmmnError> {
    let link_manager = CmmnIdentityLinkDataManager::new();
    for user_id in resolve_candidate_list(&human_task.candidate_users, case_instance) {
        insert_human_task_candidate_link(session, &link_manager, task_id, Some(&user_id), None)?;
    }
    for group_id in resolve_candidate_list(&human_task.candidate_groups, case_instance) {
        insert_human_task_candidate_link(session, &link_manager, task_id, None, Some(&group_id))?;
    }
    Ok(())
}

fn insert_human_task_candidate_link(
    session: &mut DbSession,
    link_manager: &CmmnIdentityLinkDataManager,
    task_id: &str,
    user_id: Option<&str>,
    group_id: Option<&str>,
) -> Result<(), CmmnError> {
    let link = CmmnIdentityLink {
        id: format!("cmmn-task-candidate:{}", Uuid::new_v4()),
        scope_type: "humanTask".to_string(),
        scope_id: task_id.to_string(),
        link_type: "candidate".to_string(),
        user_id: user_id.map(str::to_string),
        group_id: group_id.map(str::to_string),
    };
    let mut entity = CmmnIdentityLinkEntity::new(
        link.id.clone(),
        link.scope_type.clone(),
        link.scope_id.clone(),
        link.link_type.clone(),
        serde_json::to_string(&link)?,
    );
    entity.set_user_id(link.user_id.clone());
    entity.set_group_id(link.group_id.clone());
    link_manager.insert(session, entity)?;
    Ok(())
}

fn repeat_human_task_if_needed(
    session: &mut DbSession,
    case_definition: &CmmnCaseDefinition,
    completed_task: &CmmnHumanTaskInstance,
) -> Result<(), CmmnError> {
    let case_instance = load_case_instance_session(session, &completed_task.case_instance_id)?
        .ok_or_else(|| {
            CmmnError::storage(format!(
                "CMMN case instance '{}' disappeared during repetition evaluation",
                completed_task.case_instance_id
            ))
        })?;
    let Some((plan_item, human_task)) = find_human_task_plan_item_by_plan_item_id(
        &case_definition.model.case_plan_model,
        &completed_task.plan_item_id,
    ) else {
        return Ok(());
    };
    if !repetition_rule_matches(plan_item, &case_instance)? {
        return Ok(());
    }
    if let Some(stage_instance_id) = completed_task.stage_instance_id.as_deref()
        && !stage_instance_is_active(session, stage_instance_id)?
    {
        return Ok(());
    }

    activate_human_task(
        session,
        case_definition,
        &case_instance,
        plan_item,
        human_task,
        completed_task.stage_instance_id.as_deref(),
    )
}

fn repeat_stage_if_needed(
    session: &mut DbSession,
    case_definition: &CmmnCaseDefinition,
    ended_stage_instance: &CmmnStageInstance,
) -> Result<(), CmmnError> {
    let case_instance =
        load_case_instance_session(session, &ended_stage_instance.case_instance_id)?.ok_or_else(
            || {
                CmmnError::storage(format!(
                    "CMMN case instance '{}' disappeared during stage repetition evaluation",
                    ended_stage_instance.case_instance_id
                ))
            },
        )?;
    let Some((plan_item, stage)) = find_stage_plan_item_by_plan_item_id(
        &case_definition.model.case_plan_model,
        &ended_stage_instance.plan_item_id,
    ) else {
        return Ok(());
    };
    if !repetition_rule_matches(plan_item, &case_instance)? {
        return Ok(());
    }
    if let Some(parent_stage_instance_id) = ended_stage_instance.parent_stage_instance_id.as_deref()
        && !stage_instance_is_active(session, parent_stage_instance_id)?
    {
        return Ok(());
    }
    if open_stage_instance_exists(session, &case_instance.id, &plan_item.id)? {
        return Ok(());
    }

    create_stage_instance(
        session,
        case_definition,
        &case_instance,
        plan_item,
        stage,
        ended_stage_instance.parent_stage_instance_id.as_deref(),
        CmmnStageInstanceState::Available,
    )
}

fn manual_activation_rule_matches(
    plan_item: &CmmnPlanItem,
    case_instance: &CmmnCaseInstance,
) -> Result<bool, CmmnError> {
    plan_item
        .manual_activation_rule
        .as_ref()
        .map(|rule| evaluate_if_part_condition(rule, case_instance))
        .unwrap_or(Ok(false))
}

fn repetition_rule_matches(
    plan_item: &CmmnPlanItem,
    case_instance: &CmmnCaseInstance,
) -> Result<bool, CmmnError> {
    plan_item
        .repetition_rule
        .as_ref()
        .map(|rule| evaluate_if_part_condition(rule, case_instance))
        .unwrap_or(Ok(false))
}

fn required_rule_matches(
    plan_item: &CmmnPlanItem,
    case_instance: &CmmnCaseInstance,
) -> Result<bool, CmmnError> {
    plan_item
        .required_rule
        .as_ref()
        .map(|rule| evaluate_if_part_condition(rule, case_instance))
        .unwrap_or(Ok(false))
}

#[allow(clippy::only_used_in_recursion)]
fn has_incomplete_required_plan_items(
    session: &mut DbSession,
    case_definition: &CmmnCaseDefinition,
    case_instance: &CmmnCaseInstance,
    container: ContainerView<'_>,
    ignored: &HashSet<String>,
) -> Result<bool, CmmnError> {
    for plan_item in container.plan_items {
        // Java: PlanItemInstanceContainerUtil.java:82-86 - a plan item ignored for parent
        // completion (e.g. parentCompletionRule=ignore) never blocks, including required checks.
        if ignored.contains(&plan_item.id) {
            continue;
        }
        if required_rule_matches(plan_item, case_instance)? {
            let completed_by_event = plan_item_standard_event_occurred(
                session,
                &case_instance.id,
                &plan_item.id,
                CmmnPlanItemOnPart::STANDARD_EVENT_COMPLETE,
            )?;
            // Java checks the materialized plan-item state directly
            // (`PlanItemInstanceContainerUtil.java:102-118`). Milestones emit OCCUR
            // while their instance becomes COMPLETED, so their real mirror state is
            // the completion evidence rather than a synthetic COMPLETE event.
            let completed_by_mirror = load_plan_item_instances_session(session)?
                .iter()
                .any(|instance| {
                    instance.case_instance_id == case_instance.id
                        && instance.plan_item_id == plan_item.id
                        && instance.state == "COMPLETED"
                });
            if !completed_by_event && !completed_by_mirror {
                return Ok(true);
            }
        }
    }
    for stage in container.stages {
        if has_incomplete_required_plan_items(
            session,
            case_definition,
            case_instance,
            ContainerView::from_stage(stage),
            ignored,
        )? {
            return Ok(true);
        }
    }
    Ok(false)
}

fn activate_event_listener(
    session: &mut DbSession,
    case_definition: &CmmnCaseDefinition,
    case_instance: &CmmnCaseInstance,
    plan_item: &CmmnPlanItem,
    event_listener: &CmmnEventListener,
    parent_stage_instance_id: Option<&str>,
) -> Result<(), CmmnError> {
    // Java: AbstractEvaluationCriteriaOperation.java:584-604 - a non-empty availableCondition
    // gates the listener: only a Boolean true result makes it available (and creates its
    // event subscription). A failing or non-boolean evaluation counts as false, so the
    // listener stays unavailable; it is re-evaluated when case variables change
    // (reevaluate_event_listener_available_conditions). `${…}` uses SimpleExpression (P69);
    // non-UEL text keeps the C7 if-part dialect.
    if let Some(condition) = &event_listener.available_condition
        && !evaluate_available_condition_expression(condition, case_instance)
    {
        return Ok(());
    }
    // Timer event listeners (Java `TimerEventListener extends EventListener`) schedule a
    // timer job on activate instead of an event subscription
    // (TimerEventListenerActivityBehaviour.java:66-78, handleCreateTransition).
    if event_listener.is_timer() {
        return schedule_timer_event_listener(
            session,
            case_definition,
            case_instance,
            plan_item,
            event_listener,
            parent_stage_instance_id,
        );
    }
    // Java: variable event listeners store the changeType in the subscription configuration
    // JSON (EvaluateVariableEventListenersOperation.java:80-91).
    // Event-registry listeners store the correlation key (or null for broadcast)
    // (EventRegistryEventListenerActivityBehaviour.createEventSubscription:139-153,
    // getCorrelationKey:156-188).
    let configuration = if event_listener.event_type == CmmnEventListener::EVENT_TYPE_VARIABLE {
        event_listener
            .variable_change_type
            .as_ref()
            .map(|change_type| serde_json::json!({ "changeType": change_type }).to_string())
    } else {
        correlation_configuration_for_listener(event_listener, case_instance)
    };
    let subscription = CmmnEventSubscription {
        id: format!("cmmn-event-subscription:{}", Uuid::new_v4()),
        event_type: event_listener.event_type.clone(),
        event_name: event_listener.event_name.clone(),
        activity_id: Some(event_listener.id.clone()),
        case_instance_id: Some(case_instance.id.clone()),
        case_definition_id: Some(case_definition.id.clone()),
        plan_item_instance_id: Some(plan_item.id.clone()),
        tenant_id: case_instance.tenant_id.clone(),
        configuration,
        created_at: Utc::now(),
    };
    persist_event_subscription_session(session, &subscription)?;

    // P116: mirror the activated event listener into the unified plan-item-instance
    // table. Java's event listener plan item instance becomes AVAILABLE on activate
    // (InitiatePlanItemInstanceOperation.java:33-52) and COMPLETED on occur
    // (OccurPlanItemInstanceOperation.java:34-61). A stale AVAILABLE row from an
    // earlier activate that was later made unavailable is dropped first.
    delete_plan_item_instance_rows(
        session,
        &case_instance.id,
        &plan_item.id,
        PLAN_ITEM_DEFINITION_TYPE_EVENT_LISTENER,
    )?;
    let available_at = Utc::now();
    let listener_instance = CmmnPlanItemInstance {
        id: format!("cmmn-plan-item-instance:{}", Uuid::new_v4()),
        case_instance_id: case_instance.id.clone(),
        case_definition_id: case_definition.id.clone(),
        stage_instance_id: parent_stage_instance_id.map(str::to_string),
        plan_item_id: plan_item.id.clone(),
        plan_item_definition_id: event_listener.id.clone(),
        plan_item_definition_type: PLAN_ITEM_DEFINITION_TYPE_EVENT_LISTENER.to_string(),
        name: plan_item
            .name
            .clone()
            .unwrap_or_else(|| event_listener.name.clone().unwrap_or_default()),
        state: "AVAILABLE".to_string(),
        created_at: available_at,
        last_enabled_at: None,
        ended_at: None,
        occurred_at: None,
        assignee: None,
        tenant_id: case_instance.tenant_id.clone(),
    };
    persist_plan_item_instance_session(session, &listener_instance)?;
    Ok(())
}

/// Java `TimerEventListenerActivityBehaviour.handleCreateTransition` (:90-155) +
/// `scheduleTimerJob` (:172-212): compute the due date from the resolved timer
/// expression and persist a `cmmn-trigger-timer` timer job for the listener.
fn schedule_timer_event_listener(
    session: &mut DbSession,
    case_definition: &CmmnCaseDefinition,
    case_instance: &CmmnCaseInstance,
    plan_item: &CmmnPlanItem,
    event_listener: &CmmnEventListener,
    parent_stage_instance_id: Option<&str>,
) -> Result<(), CmmnError> {
    // Java `timerJobForPlanItemInstanceExists` (:157-170): only one timer job can ever be
    // active for a plan item — re-activation (e.g. available condition toggling) must not
    // create a duplicate.
    if timer_job_for_plan_item_exists(session, &case_instance.id, &plan_item.id)? {
        return Ok(());
    }

    let expression = event_listener.timer_expression.as_deref().ok_or_else(|| {
        CmmnError::validation(format!(
            "timerEventListener '{}' has no timerExpression",
            event_listener.id
        ))
    })?;
    // Java `resolveTimerExpression` (:223-227): `${…}` is evaluated via the expression
    // manager; a literal ISO-8601 value is used as-is.
    let resolved =
        resolve_el_or_literal_string(Some(expression), case_instance).ok_or_else(|| {
            CmmnError::validation(format!(
                "Timer expression '{}' did not resolve for timerEventListener '{}'",
                expression, event_listener.id
            ))
        })?;
    let now = Utc::now();
    let due_date = resolve_timer_due(&resolved, now).ok_or_else(|| {
        CmmnError::validation(format!(
            "Timer expression '{}' did not resolve to a date/duration/repetition for timerEventListener '{}'",
            resolved, event_listener.id
        ))
    })?;

    // Java scheduleTimerJob (:206-208): repeating timers carry the prepared repeat string
    // (R<count>/<start>/<period>), which drives rescheduling after each fire.
    let is_repeating = resolved.trim_start().starts_with('R');
    let configuration = if is_repeating {
        Some(
            serde_json::json!({ TIMER_JOB_CONFIG_REPEAT_KEY: prepare_repeat(&resolved, now) })
                .to_string(),
        )
    } else {
        None
    };

    let mut job = CmmnJob::new(format!("cmmn-job:{}", Uuid::new_v4()), CmmnJobFamily::Timer)
        .with_handler(TYPE_TRIGGER_TIMER, configuration);
    job.scope_id = Some(case_instance.id.clone());
    job.sub_scope_id = Some(plan_item.id.clone());
    job.scope_definition_id = Some(case_definition.id.clone());
    job.element_id = Some(event_listener.id.clone());
    job.tenant_id = case_instance.tenant_id.clone();
    job.due_date = Some(due_date);
    job.retries = TIMER_JOB_RETRIES;
    insert_job_entity(session, &job)?;

    // P116: mirror the activated timer event listener into the unified plan-item-instance
    // table (Java TIMER_EVENT_LISTENER plan item instance becomes AVAILABLE on activate).
    delete_plan_item_instance_rows(
        session,
        &case_instance.id,
        &plan_item.id,
        PLAN_ITEM_DEFINITION_TYPE_TIMER_EVENT_LISTENER,
    )?;
    persist_timer_listener_available_instance(
        session,
        case_definition,
        case_instance,
        plan_item,
        event_listener,
        parent_stage_instance_id,
    )?;
    Ok(())
}

/// Persist a fresh AVAILABLE unified plan-item-instance row for a timer event listener
/// (Java InitiatePlanItemInstanceOperation.java:33-52 on activate; a new instance is also
/// created on each repeat, DefaultInternalCmmnJobManager.java:163-194).
/// `parent_stage_instance_id` scopes the mirror under its owning stage so
/// `count_blocking_mirror_plan_items` / `maybe_complete_stage` see stage-nested
/// AVAILABLE timers (PlanItemInstanceContainerUtil.java:143-146).
fn persist_timer_listener_available_instance(
    session: &mut DbSession,
    case_definition: &CmmnCaseDefinition,
    case_instance: &CmmnCaseInstance,
    plan_item: &CmmnPlanItem,
    event_listener: &CmmnEventListener,
    parent_stage_instance_id: Option<&str>,
) -> Result<(), CmmnError> {
    let available_at = Utc::now();
    let listener_instance = CmmnPlanItemInstance {
        id: format!("cmmn-plan-item-instance:{}", Uuid::new_v4()),
        case_instance_id: case_instance.id.clone(),
        case_definition_id: case_definition.id.clone(),
        stage_instance_id: parent_stage_instance_id.map(str::to_string),
        plan_item_id: plan_item.id.clone(),
        plan_item_definition_id: event_listener.id.clone(),
        plan_item_definition_type: PLAN_ITEM_DEFINITION_TYPE_TIMER_EVENT_LISTENER.to_string(),
        name: plan_item
            .name
            .clone()
            .unwrap_or_else(|| event_listener.name.clone().unwrap_or_default()),
        state: "AVAILABLE".to_string(),
        created_at: available_at,
        last_enabled_at: None,
        ended_at: None,
        occurred_at: None,
        assignee: None,
        tenant_id: case_instance.tenant_id.clone(),
    };
    persist_plan_item_instance_session(session, &listener_instance)?;
    Ok(())
}

/// Java `TimerEventListenerActivityBehaviour.timerJobForPlanItemInstanceExists`
/// (:157-170): a `cmmn-trigger-timer` job for (caseInstanceId, planItemId) is active.
fn timer_job_for_plan_item_exists(
    session: &mut DbSession,
    case_instance_id: &str,
    plan_item_id: &str,
) -> Result<bool, CmmnError> {
    let count = count_rows(
        session,
        "SELECT COUNT(*) AS CNT FROM ACT_CMMN_JOB WHERE FAMILY_ = 'timer' \
         AND SCOPE_ID_ = ? AND SUB_SCOPE_ID_ = ?",
        &[case_instance_id, plan_item_id],
    )?;
    Ok(count > 0)
}

/// Java `EventRegistryEventListenerActivityBehaviour.getCorrelationKey` (:156-188).
/// Empty correlation parameter list → `None` (broadcast match).
fn correlation_configuration_for_listener(
    event_listener: &CmmnEventListener,
    case_instance: &CmmnCaseInstance,
) -> Option<String> {
    if event_listener.event_correlation_parameters.is_empty() {
        return None;
    }
    let mut params = std::collections::BTreeMap::new();
    for param in &event_listener.event_correlation_parameters {
        let value = evaluate_correlation_value_expression(&param.value, case_instance);
        params.insert(param.name.clone(), value);
    }
    Some(generate_correlation_key(&params))
}

/// Evaluate a correlation value expression against case variables.
/// Java ExpressionManager on the plan-item scope (getCorrelationKey:172-174).
fn evaluate_correlation_value_expression(
    expression: &str,
    case_instance: &CmmnCaseInstance,
) -> Option<String> {
    let trimmed = expression.trim();
    if trimmed.is_empty() {
        return None;
    }
    if is_uel_expression(trimmed) {
        let scope = case_variable_scope(case_instance);
        match SimpleExpression::new(trimmed.to_string()).get_value(&scope) {
            Some(value) => Some(json_value_to_correlation_string(&value)),
            None => None,
        }
    } else {
        Some(trimmed.to_string())
    }
}

fn find_event_listener_in_definition<'a>(
    case_definition: &'a CmmnCaseDefinition,
    activity_id: &str,
) -> Option<&'a CmmnEventListener> {
    find_event_listener_in_plan_model(&case_definition.model.case_plan_model, activity_id)
}

fn find_event_listener_in_plan_model<'a>(
    plan_model: &'a CmmnCasePlanModel,
    activity_id: &str,
) -> Option<&'a CmmnEventListener> {
    if let Some(listener) = plan_model
        .event_listeners
        .iter()
        .find(|listener| listener.id == activity_id)
    {
        return Some(listener);
    }
    for stage in &plan_model.stages {
        if let Some(listener) = find_event_listener_in_stage(stage, activity_id) {
            return Some(listener);
        }
    }
    None
}

fn find_event_listener_in_stage<'a>(
    stage: &'a CmmnStage,
    activity_id: &str,
) -> Option<&'a CmmnEventListener> {
    if let Some(listener) = stage
        .event_listeners
        .iter()
        .find(|listener| listener.id == activity_id)
    {
        return Some(listener);
    }
    for nested in &stage.stages {
        if let Some(listener) = find_event_listener_in_stage(nested, activity_id) {
            return Some(listener);
        }
    }
    None
}

/// Apply non-transient eventOutParameter mappings from payload onto the case instance.
/// Java EventInstanceCmmnUtil.java:54-66.
fn apply_event_out_parameters_to_case(
    session: &mut DbSession,
    case_instance_id: &str,
    out_parameters: &[CmmnEventOutParameter],
    payload: &Value,
) -> Result<(), CmmnError> {
    if out_parameters.is_empty() {
        return Ok(());
    }
    let mut case_instance =
        load_case_instance_session(session, case_instance_id)?.ok_or_else(|| {
            CmmnError::not_found(format!(
                "CMMN case instance '{case_instance_id}' was not found during event payload mapping"
            ))
        })?;
    let payload_object = payload.as_object();
    for param in out_parameters {
        if param.is_transient {
            // Transient variables are not persisted in the Rust CMMN engine (no
            // transient scope on PlanItemInstance). Skip — same effect as a
            // Java transient that is never read after the trigger command.
            continue;
        }
        if param.target.is_empty() {
            continue;
        }
        let value = payload_object
            .and_then(|obj| obj.get(&param.source))
            .cloned()
            .unwrap_or(Value::Null);
        case_instance.variables.insert(param.target.clone(), value);
    }
    persist_case_instance_session(session, &case_instance)?;
    persist_historic_case_session(session, &CmmnHistoricCaseInstance::from(&case_instance))?;
    Ok(())
}

fn load_event_subscriptions_for_case_session(
    session: &mut DbSession,
    case_instance_id: &str,
) -> Result<Vec<CmmnEventSubscription>, CmmnError> {
    let mut params = DbParams::new();
    params.push(case_instance_id);
    let rendered = RenderedStatement::new(
        "SELECT DATA_ FROM ACT_CMMN_EVENT_SUBSCRIPTION WHERE CASE_INSTANCE_ID_ = ? \
         ORDER BY CREATED_AT_ ASC, ID_ ASC"
            .to_string(),
        params,
    );
    let rows = session.select_raw(rendered)?;
    rows.into_iter()
        .map(|row| {
            let json = row.get_text("DATA_").ok_or_else(|| {
                CmmnError::storage("Missing DATA_ in CMMN event subscription row")
            })?;
            serde_json::from_str::<CmmnEventSubscription>(&json).map_err(CmmnError::from)
        })
        .collect()
}

// Session-level mirror of occur_event_subscription (Java: GenericEventListenerActivityBehaviour
// trigger -> occur transition): deletes the subscription, records the occur transition and
// fans out the standard-event handling. Case completion is evaluated by the caller.
fn occur_event_subscription_in_session(
    session: &mut DbSession,
    case_definition: &CmmnCaseDefinition,
    subscription: &CmmnEventSubscription,
) -> Result<(), CmmnError> {
    let mut params = DbParams::new();
    params.push(subscription.id.as_str());
    session.execute_raw(RenderedStatement::new(
        "DELETE FROM ACT_CMMN_EVENT_SUBSCRIPTION WHERE ID_ = ?".to_string(),
        params,
    ))?;
    if let (Some(case_instance_id), Some(plan_item_id)) = (
        subscription.case_instance_id.as_deref(),
        subscription.plan_item_instance_id.as_deref(),
    ) {
        record_plan_item_standard_event_session(
            session,
            case_instance_id,
            plan_item_id,
            CmmnPlanItemOnPart::STANDARD_EVENT_OCCUR,
        )?;
        // P116: the event listener plan item instance reaches COMPLETED on occur.
        complete_plan_item_instance_rows(session, case_instance_id, plan_item_id, "eventlistener")?;
        handle_plan_item_standard_event(
            session,
            case_definition,
            case_instance_id,
            plan_item_id,
            CmmnPlanItemOnPart::STANDARD_EVENT_OCCUR,
            None,
        )?;
    }
    Ok(())
}

/// Load the case definition for a case instance (used by timer job firing).
fn load_case_definition_for_case_session(
    session: &mut DbSession,
    case_instance_id: &str,
) -> Result<CmmnCaseDefinition, CmmnError> {
    let case_instance =
        load_case_instance_session(session, case_instance_id)?.ok_or_else(|| {
            CmmnError::not_found(format!(
                "CMMN case instance '{case_instance_id}' was not found"
            ))
        })?;
    load_case_definition_session(session, &case_instance.case_definition_id)?.ok_or_else(|| {
        CmmnError::storage(format!(
            "CMMN case definition '{}' disappeared during timer trigger",
            case_instance.case_definition_id
        ))
    })
}

/// Fire a due timer event listener: the timer occurs the listener's plan item
/// (Java `TriggerTimerEventJobHandler.java:35-38` → `TriggerPlanItemInstanceOperation`
/// → `TimerEventListenerActivityBehaviour.trigger` → `OccurPlanItemInstanceOperation`),
/// recording the `occur` standard event, completing the P116 mirror row and fanning out
/// through the sentry machinery. Unlike `occur_event_subscription_in_session` there is
/// no subscription to delete — a timer listener owns a timer job instead.
fn occur_timer_event_listener_in_session(
    session: &mut DbSession,
    case_definition: &CmmnCaseDefinition,
    case_instance_id: &str,
    plan_item_id: &str,
) -> Result<(), CmmnError> {
    record_plan_item_standard_event_session(
        session,
        case_instance_id,
        plan_item_id,
        CmmnPlanItemOnPart::STANDARD_EVENT_OCCUR,
    )?;
    complete_plan_item_instance_rows(
        session,
        case_instance_id,
        plan_item_id,
        PLAN_ITEM_DEFINITION_TYPE_TIMER_EVENT_LISTENER,
    )?;
    handle_plan_item_standard_event(
        session,
        case_definition,
        case_instance_id,
        plan_item_id,
        CmmnPlanItemOnPart::STANDARD_EVENT_OCCUR,
        None,
    )?;
    maybe_complete_case(session, case_instance_id)?;
    Ok(())
}

/// Java `DefaultJobManager.executeTimerJob` (:535) →
/// `TimerJobSchedulerImpl.rescheduleTimerJobAfterExecution` (:40-52) →
/// `TimerJobEntityManagerImpl.createAndCalculateNextTimer` (:48-62): after a repeating
/// timer fires, decrement the repeat count and schedule the next fire. Returns early
/// when the cycle is exhausted or the job carries no repeat configuration.
///
/// Also mirrors Java `DefaultInternalCmmnJobManager.preRepeatedTimerScheduleInternal`
/// (:163-194): a fresh AVAILABLE plan item instance is created for the repeating timer,
/// since the original was completed on occur.
fn reschedule_timer_event_listener_job_in_session(
    session: &mut DbSession,
    case_definition: &CmmnCaseDefinition,
    fired_job: &CmmnJob,
) -> Result<(), CmmnError> {
    let Some(raw) = fired_job
        .configuration
        .as_deref()
        .filter(|s| !s.trim().is_empty())
    else {
        return Ok(());
    };
    let config: Value = serde_json::from_str(raw)
        .map_err(|e| CmmnError::execution(format!("Invalid CMMN job configuration JSON: {e}")))?;
    let Some(repeat) = config
        .get(TIMER_JOB_CONFIG_REPEAT_KEY)
        .and_then(|v| v.as_str())
    else {
        return Ok(());
    };
    let Some(next_repeat) = next_repeat_expression(repeat) else {
        return Ok(());
    };
    let now = Utc::now();
    let Some(next_due) = resolve_next_due(&next_repeat, now) else {
        return Ok(());
    };
    let mut next_job = fired_job.clone();
    next_job.id = format!("cmmn-job:{}", Uuid::new_v4());
    next_job.family = CmmnJobFamily::Timer;
    next_job.state = CmmnJobFamily::Timer.as_str().to_string();
    next_job.due_date = Some(next_due);
    next_job.configuration =
        Some(serde_json::json!({ TIMER_JOB_CONFIG_REPEAT_KEY: next_repeat }).to_string());
    next_job.lock_owner = None;
    next_job.exception_message = None;
    next_job.exception_stacktrace = None;
    insert_job_entity(session, &next_job)?;

    // A fresh AVAILABLE instance for the next cycle (the completed one was the fired
    // cycle's instance).
    if let (Some(case_instance_id), Some(plan_item_id)) = (
        fired_job.scope_id.as_deref(),
        fired_job.sub_scope_id.as_deref(),
    ) {
        let case_instance =
            load_case_instance_session(session, case_instance_id)?.ok_or_else(|| {
                CmmnError::storage(format!(
                    "CMMN case instance '{case_instance_id}' disappeared"
                ))
            })?;
        let plan_item = find_plan_item_in_case_definition(case_definition, plan_item_id)?;
        let listener = fired_job
            .element_id
            .as_deref()
            .and_then(|id| find_event_listener_in_definition(case_definition, id))
            .ok_or_else(|| {
                CmmnError::execution(format!(
                    "CMMN timer job '{}' references missing timerEventListener '{}'",
                    fired_job.id,
                    fired_job.element_id.as_deref().unwrap_or("")
                ))
            })?;
        // Java keeps the completed instance in the runtime query surface (includeEnded)
        // and adds a fresh AVAILABLE one for the next cycle; the completed row is not
        // dropped here (DefaultInternalCmmnJobManager.java:163-194). Preserve the
        // owning stage so stage-nested repeating timers still block their stage
        // (PlanItemInstanceContainerUtil.java:143-146).
        let parent_stage_instance_id = load_plan_item_instances_session(session)?
            .into_iter()
            .find(|instance| {
                instance.case_instance_id == case_instance.id
                    && instance.plan_item_id == plan_item.id
                    && instance.plan_item_definition_type
                        == PLAN_ITEM_DEFINITION_TYPE_TIMER_EVENT_LISTENER
            })
            .and_then(|instance| instance.stage_instance_id);
        persist_timer_listener_available_instance(
            session,
            case_definition,
            &case_instance,
            &plan_item,
            listener,
            parent_stage_instance_id.as_deref(),
        )?;
    }
    Ok(())
}

/// Java `TriggerPlanItemInstanceOperation.java:39-50`: an event listener may only be
/// triggered while its plan item is still AVAILABLE. The unified P116 mirror row is in
/// AVAILABLE state for a live (or freshly repeated) listener and dropped on terminate /
/// dismiss, so it is the source of truth for "still fireable".
fn timer_listener_still_available(
    session: &mut DbSession,
    case_instance_id: &str,
    plan_item_id: &str,
) -> Result<bool, CmmnError> {
    let instances = load_plan_item_instances_session(session)?;
    Ok(instances.iter().any(|instance| {
        instance.case_instance_id == case_instance_id
            && instance.plan_item_id == plan_item_id
            && instance.plan_item_definition_type == PLAN_ITEM_DEFINITION_TYPE_TIMER_EVENT_LISTENER
            && instance.state == "AVAILABLE"
    }))
}

/// Find a plan item in the case definition by its model id (recursing into stages).
fn find_plan_item_in_case_definition(
    case_definition: &CmmnCaseDefinition,
    plan_item_id: &str,
) -> Result<CmmnPlanItem, CmmnError> {
    find_plan_item_in_plan_model(&case_definition.model.case_plan_model, plan_item_id)
        .cloned()
        .ok_or_else(|| {
            CmmnError::execution(format!(
                "CMMN plan item '{plan_item_id}' was not found in case definition '{}'",
                case_definition.id
            ))
        })
}

/// Resolve the ACTIVE stage instance that owns `plan_item_id` when the plan item is
/// nested under a stage definition. Case-level plan items yield `None`. Used when the
/// activation path does not already know the parent stage instance id (variable-driven
/// re-availability, change-state activation).
fn resolve_parent_stage_instance_id_for_plan_item(
    session: &mut DbSession,
    case_definition: &CmmnCaseDefinition,
    case_instance_id: &str,
    plan_item_id: &str,
) -> Result<Option<String>, CmmnError> {
    let Some(stage_definition_id) = find_containing_stage_definition_id(
        ContainerView::from_case_plan_model(&case_definition.model.case_plan_model),
        plan_item_id,
    ) else {
        return Ok(None);
    };
    let mut params = DbParams::new();
    params.push(case_instance_id);
    let rendered = RenderedStatement::new(
        "SELECT DATA_ FROM ACT_CMMN_STAGE_INSTANCE WHERE CASE_INSTANCE_ID_ = ? AND STATE_ = 'ACTIVE'"
            .to_string(),
        params,
    );
    let rows = session.select_raw(rendered)?;
    for row in rows {
        let json = row
            .get_text("DATA_")
            .ok_or_else(|| CmmnError::storage("Missing DATA_ in CMMN stage instance row"))?;
        let stage_instance: CmmnStageInstance =
            serde_json::from_str(&json).map_err(CmmnError::from)?;
        if stage_instance.stage_definition_id == stage_definition_id {
            return Ok(Some(stage_instance.id));
        }
    }
    Ok(None)
}

/// Walk the case plan model and return the stage definition id that directly
/// (or transitively via nested stages) contains `plan_item_id`.
fn find_containing_stage_definition_id(
    container: ContainerView<'_>,
    plan_item_id: &str,
) -> Option<String> {
    for stage in container.stages {
        let stage_view = ContainerView::from_stage(stage);
        if stage_view
            .plan_items
            .iter()
            .any(|plan_item| plan_item.id == plan_item_id)
        {
            return Some(stage.id.clone());
        }
        if let Some(nested) = find_containing_stage_definition_id(stage_view, plan_item_id) {
            return Some(nested);
        }
    }
    None
}

fn find_plan_item_in_plan_model<'a>(
    plan_model: &'a CmmnCasePlanModel,
    plan_item_id: &str,
) -> Option<&'a CmmnPlanItem> {
    find_plan_item_in_container(
        ContainerView::from_case_plan_model(plan_model),
        plan_item_id,
    )
}

fn find_plan_item_in_container<'a>(
    container: ContainerView<'a>,
    plan_item_id: &str,
) -> Option<&'a CmmnPlanItem> {
    if let Some(plan_item) = container
        .plan_items
        .iter()
        .find(|plan_item| plan_item.id == plan_item_id)
    {
        return Some(plan_item);
    }
    for stage in container.stages {
        if let Some(found) =
            find_plan_item_in_container(ContainerView::from_stage(stage), plan_item_id)
        {
            return Some(found);
        }
    }
    None
}

fn case_still_active(session: &mut DbSession, case_instance_id: &str) -> Result<bool, CmmnError> {
    let case_instance = load_case_instance_session(session, case_instance_id)?;
    Ok(case_instance
        .is_some_and(|case_instance| case_instance.state == CmmnCaseInstanceState::Active))
}

fn job_exists(session: &mut DbSession, job_id: &str) -> Result<bool, CmmnError> {
    let mut params = DbParams::new();
    params.push(job_id);
    let count = count_rows(
        session,
        "SELECT COUNT(*) AS CNT FROM ACT_CMMN_JOB WHERE ID_ = ?",
        &[job_id],
    )?;
    Ok(count > 0)
}

/// Delete a job row tolerating a prior cascade delete (e.g. case completion removed
/// every job for the case in the same transaction).
fn delete_job_entity_if_exists(session: &mut DbSession, job_id: &str) -> Result<(), CmmnError> {
    if job_exists(session, job_id)? {
        crate::management::delete_job_entity(session, job_id)?;
    }
    Ok(())
}

// Java: EvaluateVariableEventListenersOperation.java:93-95 change type matching - a
// subscription matches the exact change type, "all", or "update-create" for both kinds.
fn variable_change_type_matches(subscription_change_type: &str, change_type: &str) -> bool {
    subscription_change_type == change_type
        || subscription_change_type == CmmnEventListener::CHANGE_TYPE_ALL
        || (subscription_change_type == CmmnEventListener::CHANGE_TYPE_UPDATE_CREATE
            && (change_type == CmmnEventListener::CHANGE_TYPE_CREATE
                || change_type == CmmnEventListener::CHANGE_TYPE_UPDATE))
}

// Java: EvaluateVariableEventListenersOperation.java:58-104 - after variables are written,
// every "variable" event subscription of the case whose eventName matches a written variable
// name and whose configured changeType matches the kind of write triggers the plan item
// instance (occur transition).
fn trigger_variable_event_listeners(
    session: &mut DbSession,
    case_definition: &CmmnCaseDefinition,
    case_instance: &CmmnCaseInstance,
    variable_changes: &[(String, &str)],
) -> Result<(), CmmnError> {
    if variable_changes.is_empty() {
        return Ok(());
    }
    let subscriptions = load_event_subscriptions_for_case_session(session, &case_instance.id)?;
    for subscription in subscriptions {
        if subscription.event_type != CmmnEventListener::EVENT_TYPE_VARIABLE {
            continue;
        }
        let Some(event_name) = subscription.event_name.as_deref() else {
            continue;
        };
        // Java :80-91 - the changeType lives in the subscription configuration JSON and
        // defaults to "all" when absent or unreadable.
        let subscription_change_type = subscription
            .configuration
            .as_deref()
            .and_then(|configuration| {
                serde_json::from_str::<Value>(configuration)
                    .ok()
                    .and_then(|node| {
                        node.get("changeType")
                            .and_then(|value| value.as_str().map(str::to_string))
                    })
            })
            .unwrap_or_else(|| CmmnEventListener::CHANGE_TYPE_ALL.to_string());
        let matches_change = variable_changes.iter().any(|(name, change_type)| {
            name == event_name
                && variable_change_type_matches(&subscription_change_type, change_type)
        });
        if !matches_change {
            continue;
        }
        occur_event_subscription_in_session(session, case_definition, &subscription)?;
    }
    Ok(())
}

fn active_stage_definition_ids(
    session: &mut DbSession,
    case_instance_id: &str,
) -> Result<Vec<String>, CmmnError> {
    let mut params = DbParams::new();
    params.push(case_instance_id);
    let rendered = RenderedStatement::new(
        "SELECT DATA_ FROM ACT_CMMN_STAGE_INSTANCE WHERE CASE_INSTANCE_ID_ = ? AND STATE_ = 'ACTIVE'"
            .to_string(),
        params,
    );
    let rows = session.select_raw(rendered)?;
    let stage_instances = rows
        .into_iter()
        .map(|row| {
            let json = row
                .get_text("DATA_")
                .ok_or_else(|| CmmnError::storage("Missing DATA_ in CMMN stage instance row"))?;
            serde_json::from_str::<CmmnStageInstance>(&json).map_err(CmmnError::from)
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(stage_instances
        .into_iter()
        .map(|stage_instance| stage_instance.stage_definition_id)
        .collect())
}

fn collect_reached_conditional_event_listeners<'a>(
    container: ContainerView<'a>,
    container_reached: bool,
    active_stage_definition_ids: &[String],
    targets: &mut Vec<(&'a CmmnPlanItem, &'a CmmnEventListener)>,
) {
    if container_reached {
        for plan_item in container.plan_items {
            if let Some(event_listener) = container
                .event_listeners
                .iter()
                .find(|candidate| candidate.id == plan_item.definition_ref)
                && event_listener.available_condition.is_some()
            {
                targets.push((plan_item, event_listener));
            }
        }
    }
    for stage in container.stages {
        let stage_reached = active_stage_definition_ids.iter().any(|id| id == &stage.id);
        collect_reached_conditional_event_listeners(
            ContainerView::from_stage(stage),
            stage_reached,
            active_stage_definition_ids,
            targets,
        );
    }
}

// Java: evaluateAvailableCondition is re-run on every evaluation cycle
// (AbstractEvaluationCriteriaOperation.java:584-604): an event listener whose condition
// turns true dispatches to available (its event subscription is created); one whose
// condition turns false moves back to unavailable (its subscription is removed again).
// An unavailable listener owns no subscription and therefore never blocks completion
// (PlanItemInstanceContainerUtil.java:143-146 only counts AVAILABLE/ENABLED plan items).
fn reevaluate_event_listener_available_conditions(
    session: &mut DbSession,
    case_definition: &CmmnCaseDefinition,
    case_instance: &CmmnCaseInstance,
) -> Result<(), CmmnError> {
    if case_instance.state != CmmnCaseInstanceState::Active {
        return Ok(());
    }
    let active_stage_ids = active_stage_definition_ids(session, &case_instance.id)?;
    let mut targets = Vec::new();
    collect_reached_conditional_event_listeners(
        ContainerView::from_case_plan_model(&case_definition.model.case_plan_model),
        true,
        &active_stage_ids,
        &mut targets,
    );
    if targets.is_empty() {
        return Ok(());
    }
    let subscriptions = load_event_subscriptions_for_case_session(session, &case_instance.id)?;
    for (plan_item, event_listener) in targets {
        // A listener that already occurred or left the runtime via terminate/exit stays in
        // its end state (Java: END_STATES check, PlanItemInstanceContainerUtil.java:86), and
        // a manual-activation listener waiting in enabled state is not re-dispatched either.
        if plan_item_standard_event_occurred(
            session,
            &case_instance.id,
            &plan_item.id,
            CmmnPlanItemOnPart::STANDARD_EVENT_OCCUR,
        )? || plan_item_standard_event_occurred(
            session,
            &case_instance.id,
            &plan_item.id,
            CmmnPlanItemOnPart::STANDARD_EVENT_TERMINATE,
        )? || plan_item_standard_event_occurred(
            session,
            &case_instance.id,
            &plan_item.id,
            CmmnPlanItemOnPart::STANDARD_EVENT_EXIT,
        )? || enabled_plan_item_instance_exists(session, &case_instance.id, &plan_item.id)?
        {
            continue;
        }
        let Some(condition) = event_listener.available_condition.as_ref() else {
            continue;
        };
        let satisfied = evaluate_available_condition_expression(condition, case_instance);
        let existing_subscription = subscriptions.iter().find(|subscription| {
            subscription.plan_item_instance_id.as_deref() == Some(plan_item.id.as_str())
        });
        if satisfied && existing_subscription.is_none() {
            // unavailable -> available: create the listener subscription now.
            // Resolve the owning ACTIVE stage when the plan item is nested so the
            // AVAILABLE mirror participates in stage completion
            // (PlanItemInstanceContainerUtil.java:143-146).
            let parent_stage_instance_id = resolve_parent_stage_instance_id_for_plan_item(
                session,
                case_definition,
                &case_instance.id,
                &plan_item.id,
            )?;
            activate_event_listener(
                session,
                case_definition,
                case_instance,
                plan_item,
                event_listener,
                parent_stage_instance_id.as_deref(),
            )?;
        } else if !satisfied {
            // available -> unavailable: drop the subscription (generic / variable
            // listeners) or the timer job (timer event listeners) — Java DISMISS
            // transition (TimerEventListenerActivityBehaviour.java:72-77).
            let had_timer_job = event_listener.is_timer()
                && timer_job_for_plan_item_exists(session, &case_instance.id, &plan_item.id)?;
            if existing_subscription.is_none() && !had_timer_job {
                continue;
            }
            if let Some(subscription) = existing_subscription {
                let mut params = DbParams::new();
                params.push(subscription.id.as_str());
                session.execute_raw(RenderedStatement::new(
                    "DELETE FROM ACT_CMMN_EVENT_SUBSCRIPTION WHERE ID_ = ?".to_string(),
                    params,
                ))?;
            }
            if had_timer_job {
                delete_timer_jobs_for_plan_item(session, &case_instance.id, &plan_item.id)?;
            }
            // P116: the listener leaves the available set — drop its unified
            // plan-item-instance mirror too (no AVAILABLE listener without a
            // subscription / timer job).
            delete_plan_item_instance_rows(
                session,
                &case_instance.id,
                &plan_item.id,
                "eventlistener",
            )?;
            delete_plan_item_instance_rows(
                session,
                &case_instance.id,
                &plan_item.id,
                PLAN_ITEM_DEFINITION_TYPE_TIMER_EVENT_LISTENER,
            )?;
        }
    }
    Ok(())
}

// Java parity: ExpressionUtil.java:260-265 evaluateAutoComplete - a non-empty
// autoCompleteCondition takes precedence over the static autoComplete flag. A condition
// that fails to evaluate counts as false (same lenient handling as if-part conditions).
fn evaluate_auto_complete(
    auto_complete: bool,
    condition: Option<&CmmnSentryIfPartExpression>,
    case_instance: &CmmnCaseInstance,
) -> bool {
    match condition {
        Some(expression) => matches!(
            evaluate_if_part_condition(expression, case_instance),
            Ok(true)
        ),
        None => auto_complete,
    }
}

fn maybe_complete_stage(
    session: &mut DbSession,
    case_definition: &CmmnCaseDefinition,
    stage_instance_id: &str,
) -> Result<(), CmmnError> {
    let mut stage_instance = match load_stage_instance_session(session, stage_instance_id)? {
        Some(stage_instance) => stage_instance,
        None => return Ok(()),
    };
    if stage_instance.state != CmmnStageInstanceState::Active {
        return Ok(());
    }

    let stage = find_stage_by_definition_id(
        &case_definition.model.case_plan_model.stages,
        &stage_instance.stage_definition_id,
    )
    .ok_or_else(|| {
        CmmnError::storage(format!(
            "CMMN stage definition '{}' was not found in case definition '{}'",
            stage_instance.stage_definition_id, case_definition.id
        ))
    })?;
    let case_instance = load_case_instance_session(session, &stage_instance.case_instance_id)?
        .ok_or_else(|| {
            CmmnError::storage(format!(
                "CMMN case instance '{}' was not found",
                stage_instance.case_instance_id
            ))
        })?;
    let auto_complete = evaluate_auto_complete(
        stage.auto_complete,
        stage.auto_complete_condition.as_ref(),
        &case_instance,
    );

    // Java: PlanItemInstanceContainerUtil.java:91-97 - ACTIVE plan items always block;
    // :143-146 - AVAILABLE/ENABLED plan items only block when the container is not autocomplete.
    let open_direct_tasks = if auto_complete {
        count_rows(
            session,
            "SELECT COUNT(*) AS CNT FROM ACT_CMMN_HUMAN_TASK \
             WHERE STAGE_INSTANCE_ID_ = ? AND STATE_ = 'ACTIVE'",
            &[stage_instance_id],
        )?
    } else {
        count_rows(
            session,
            "SELECT COUNT(*) AS CNT FROM ACT_CMMN_HUMAN_TASK \
             WHERE STAGE_INSTANCE_ID_ = ? AND STATE_ <> 'COMPLETED'",
            &[stage_instance_id],
        )?
    };
    let open_direct_stages = if auto_complete {
        count_rows(
            session,
            "SELECT COUNT(*) AS CNT FROM ACT_CMMN_STAGE_INSTANCE \
             WHERE PARENT_STAGE_INSTANCE_ID_ = ? AND STATE_ = 'ACTIVE'",
            &[stage_instance_id],
        )?
    } else {
        count_rows(
            session,
            "SELECT COUNT(*) AS CNT FROM ACT_CMMN_STAGE_INSTANCE \
             WHERE PARENT_STAGE_INSTANCE_ID_ = ? AND STATE_ <> 'COMPLETED'",
            &[stage_instance_id],
        )?
    };
    // Java: PlanItemInstanceContainerUtil.java:144-146 - available event listeners do not
    // block completion of an autocomplete container.
    let open_event_subscriptions = if auto_complete {
        0
    } else {
        open_event_subscription_count_for_container(
            session,
            &stage_instance.case_instance_id,
            ContainerView::from_stage(stage),
        )?
    };
    // Java: PlanItemInstanceContainerUtil.java:143-146 - AVAILABLE/ENABLED mirror rows
    // block only when the container is not autocomplete (autoComplete branch stays 0).
    let open_blocking_plan_items = if auto_complete {
        0
    } else {
        count_blocking_mirror_plan_items(
            session,
            &stage_instance.case_instance_id,
            Some(stage_instance_id),
        )?
    };

    // Java: PlanItemInstanceContainerUtil.java:73-190 - subtract plan items whose
    // parentCompletionRule / completionNeutralRule says to ignore them. No rule-bearing plan
    // items means the counts (and behavior) are untouched.
    let mut rule_items = Vec::new();
    collect_rule_bearing_plan_items(ContainerView::from_stage(stage), &mut rule_items);
    let mut ignored_ids: HashSet<String> = HashSet::new();
    let (open_direct_tasks, open_direct_stages, open_event_subscriptions, open_blocking_plan_items) =
        if rule_items.is_empty() {
            (
                open_direct_tasks,
                open_direct_stages,
                open_event_subscriptions,
                open_blocking_plan_items,
            )
        } else {
            let task_ignored = ignored_open_human_tasks(
                session,
                &case_instance,
                &rule_items,
                auto_complete,
                "STAGE_INSTANCE_ID_",
                stage_instance_id,
                &mut ignored_ids,
            )?;
            let stage_ignored = ignored_open_stage_instances(
                session,
                &case_instance,
                &rule_items,
                auto_complete,
                "PARENT_STAGE_INSTANCE_ID_",
                stage_instance_id,
                &mut ignored_ids,
            )?;
            let event_ignored = if auto_complete {
                0
            } else {
                ignored_open_event_subscriptions(
                    session,
                    &case_instance,
                    &rule_items,
                    &mut ignored_ids,
                )?
            };
            let blocking_ignored = if auto_complete {
                0
            } else {
                ignored_open_mirror_plan_items(
                    session,
                    &case_instance,
                    &rule_items,
                    Some(stage_instance_id),
                    &mut ignored_ids,
                )?
            };
            (
                open_direct_tasks.saturating_sub(task_ignored),
                open_direct_stages.saturating_sub(stage_ignored),
                open_event_subscriptions.saturating_sub(event_ignored as usize),
                open_blocking_plan_items.saturating_sub(blocking_ignored),
            )
        };

    // Java: PlanItemInstanceContainerUtil.java:102-118 - required plan items always block,
    // regardless of the autocomplete setting.
    let incomplete_required = has_incomplete_required_plan_items(
        session,
        case_definition,
        &case_instance,
        ContainerView::from_stage(stage),
        &ignored_ids,
    )?;

    if open_direct_tasks == 0
        && open_direct_stages == 0
        && open_event_subscriptions == 0
        && open_blocking_plan_items == 0
        && !incomplete_required
    {
        if auto_complete {
            terminate_residual_stage_children_for_auto_complete(
                session,
                case_definition,
                &stage_instance,
                stage,
            )?;
        }
        // P126: stage completion is a plan item state transition
        // (AbstractMovePlanItemInstanceToTerminalStateOperation.java:124).
        fire_plan_item_lifecycle_listeners_session(
            session,
            &stage_instance.case_instance_id,
            Some(&stage_instance.id),
            &stage_instance.stage_definition_id,
            Some("stage"),
            stage_instance.state.as_str(),
            CmmnStageInstanceState::Completed.as_str(),
        )?;
        stage_instance.state = CmmnStageInstanceState::Completed;
        stage_instance.ended_at = Some(Utc::now());
        persist_stage_instance_session(session, &stage_instance)?;
        repeat_stage_if_needed(session, case_definition, &stage_instance)?;

        if let Some(parent_stage_instance_id) = stage_instance.parent_stage_instance_id.as_deref() {
            maybe_complete_stage(session, case_definition, parent_stage_instance_id)?;
        }
    }

    Ok(())
}

// When an autocomplete stage completes, remaining non-active children (AVAILABLE/DISABLED
// tasks, available child stages, event subscriptions, manual-activation ENABLED rows) are exited,
// mirroring Java where completing the container moves skipped AVAILABLE/ENABLED children to a
// terminal state (PlanItemInstanceContainerUtil.java:143-146).
fn terminate_residual_stage_children_for_auto_complete(
    session: &mut DbSession,
    case_definition: &CmmnCaseDefinition,
    stage_instance: &CmmnStageInstance,
    stage: &CmmnStage,
) -> Result<(), CmmnError> {
    let mut params = DbParams::new();
    params.push(stage_instance.id.as_str());
    let rendered = RenderedStatement::new(
        "SELECT DATA_ FROM ACT_CMMN_STAGE_INSTANCE \
         WHERE PARENT_STAGE_INSTANCE_ID_ = ? AND STATE_ <> 'COMPLETED'"
            .to_string(),
        params,
    );
    let rows = session.select_raw(rendered)?;
    let child_stage_instances = rows
        .into_iter()
        .map(|row| {
            let json = row
                .get_text("DATA_")
                .ok_or_else(|| CmmnError::storage("Missing DATA_ in CMMN stage instance row"))?;
            serde_json::from_str::<CmmnStageInstance>(&json).map_err(CmmnError::from)
        })
        .collect::<Result<Vec<_>, _>>()?;
    for child_stage_instance in child_stage_instances {
        if load_stage_instance_session(session, &child_stage_instance.id)?.is_none() {
            continue;
        }
        let child_stage =
            find_stage_by_definition_id(&stage.stages, &child_stage_instance.stage_definition_id)
                .or_else(|| {
                    find_stage_by_definition_id(
                        &case_definition.model.case_plan_model.stages,
                        &child_stage_instance.stage_definition_id,
                    )
                })
                .ok_or_else(|| {
                    CmmnError::storage(format!(
                        "CMMN stage definition '{}' was not found in case definition '{}'",
                        child_stage_instance.stage_definition_id, case_definition.id
                    ))
                })?;
        terminate_stage_instance(session, case_definition, child_stage_instance, child_stage)?;
    }

    let mut params = DbParams::new();
    params.push(stage_instance.id.as_str());
    let rendered = RenderedStatement::new(
        "SELECT DATA_ FROM ACT_CMMN_HUMAN_TASK \
         WHERE STAGE_INSTANCE_ID_ = ? AND STATE_ <> 'COMPLETED'"
            .to_string(),
        params,
    );
    let rows = session.select_raw(rendered)?;
    let residual_tasks = rows
        .into_iter()
        .map(|row| {
            let json = row
                .get_text("DATA_")
                .ok_or_else(|| CmmnError::storage("Missing DATA_ in CMMN human task row"))?;
            serde_json::from_str::<CmmnHumanTaskInstance>(&json).map_err(CmmnError::from)
        })
        .collect::<Result<Vec<_>, _>>()?;
    for mut task in residual_tasks {
        task.state = CmmnHumanTaskState::Terminated;
        task.completed_at = Some(Utc::now());
        persist_historic_human_task_session(session, &CmmnHistoricHumanTaskInstance::from(&task))?;
        let mut params = DbParams::new();
        params.push(task.id.as_str());
        session.execute_raw(RenderedStatement::new(
            "DELETE FROM ACT_CMMN_HUMAN_TASK WHERE ID_ = ?".to_string(),
            params,
        ))?;
    }

    delete_event_subscriptions_for_container(
        session,
        &stage_instance.case_instance_id,
        ContainerView::from_stage(stage),
    )?;

    // Clear manual-activation ENABLED mirror rows within this stage scope so they no longer
    // block case-level completion counting.
    let mut plan_item_ids = Vec::new();
    collect_container_plan_item_ids(ContainerView::from_stage(stage), &mut plan_item_ids);
    for plan_item_id in plan_item_ids {
        delete_enabled_plan_item_instance_rows(
            session,
            &stage_instance.case_instance_id,
            plan_item_id,
        )?;
    }

    Ok(())
}

fn open_event_subscription_count_for_container(
    session: &mut DbSession,
    case_instance_id: &str,
    container: ContainerView<'_>,
) -> Result<usize, CmmnError> {
    let mut plan_item_ids = Vec::new();
    collect_container_plan_item_ids(container, &mut plan_item_ids);
    if plan_item_ids.is_empty() {
        return Ok(0);
    }

    let mut params = DbParams::new();
    params.push(case_instance_id);
    let rendered = RenderedStatement::new(
        "SELECT PLAN_ITEM_INSTANCE_ID_ FROM ACT_CMMN_EVENT_SUBSCRIPTION \
         WHERE CASE_INSTANCE_ID_ = ?"
            .to_string(),
        params,
    );
    let rows = session.select_raw(rendered)?;
    let subscription_plan_item_ids = rows
        .into_iter()
        .map(|row| row.get_text("PLAN_ITEM_INSTANCE_ID_"))
        .collect::<Vec<_>>();

    Ok(subscription_plan_item_ids
        .into_iter()
        .flatten()
        .filter(|plan_item_id| {
            plan_item_ids
                .iter()
                .any(|candidate| *candidate == plan_item_id)
        })
        .count())
}

fn collect_container_plan_item_ids<'a>(
    container: ContainerView<'a>,
    plan_item_ids: &mut Vec<&'a str>,
) {
    plan_item_ids.extend(
        container
            .plan_items
            .iter()
            .map(|plan_item| plan_item.id.as_str()),
    );
    for stage in container.stages {
        collect_container_plan_item_ids(ContainerView::from_stage(stage), plan_item_ids);
    }
}

// ----- C8: parentCompletionRule / completionNeutralRule completion semantics -----
// Java: PlanItemInstanceContainerUtil.java:73-190. A child plan item carrying a
// parentCompletionRule (or a satisfied completionNeutralRule) can be ignored when deciding
// whether its parent stage/case may complete. The Rust engine evaluates completion through
// COUNT-based queries, so we mirror the Java "ignore" semantics by subtracting the rule-ignored
// plan items from those counts. When no plan item in the container carries such a rule, the
// computed adjustments are all zero and completion behaves exactly as before this feature.
#[derive(Clone, Copy, PartialEq, Eq)]
enum PlanItemCompletionState {
    Active,
    Available,
    Enabled,
}

// Java: PlanItemInstanceContainerUtil.java:86-190 - decide whether a plan item should be ignored
// for parent completion, given its parentCompletionRule, completionNeutral flag, current
// lifecycle state, whether it already completed at least once and whether it is repeatable.
fn plan_item_ignored_for_completion(
    parent_completion_rule: Option<&str>,
    completion_neutral: bool,
    state: PlanItemCompletionState,
    already_completed: bool,
    repeatable: bool,
) -> bool {
    use PlanItemCompletionState::{Active, Available, Enabled};
    if let Some(rule) = parent_completion_rule {
        match rule {
            // line 86: IGNORE always skips, regardless of the current state.
            "ignore" => return true,
            // line 122-125: IGNORE_IF_AVAILABLE_OR_ENABLED skips AVAILABLE or ENABLED.
            "ignoreIfAvailableOrEnabled" if matches!(state, Available | Enabled) => return true,
            // line 128-131: IGNORE_IF_AVAILABLE skips AVAILABLE.
            "ignoreIfAvailable" if state == Available => return true,
            // line 181-184 (shouldIgnorePlanItemForCompletion): IGNORE_AFTER_FIRST_COMPLETION
            // returns alreadyCompleted, only reached for ACTIVE (line 94) or repeatable (line 138).
            "ignoreAfterFirstCompletion"
                if already_completed && (state == Active || repeatable) =>
            {
                return true;
            }
            // line 185-188: IGNORE_AFTER_FIRST_COMPLETION_IF_AVAILABLE_OR_ENABLED needs
            // AVAILABLE/ENABLED and is only reached for repeatable plan items (line 138).
            "ignoreAfterFirstCompletionIfAvailableOrEnabled"
                if already_completed && repeatable && matches!(state, Available | Enabled) =>
            {
                return true;
            }
            _ => {}
        }
    }
    // line 128-131: a completionNeutral plan item is ignored while AVAILABLE.
    completion_neutral && state == Available
}

// Only the "after first completion" variants consult the already-completed flag; for every other
// rule we skip the extra plan-item-event query.
fn rule_needs_already_completed(rule: Option<&str>) -> bool {
    matches!(
        rule,
        Some("ignoreAfterFirstCompletion") | Some("ignoreAfterFirstCompletionIfAvailableOrEnabled")
    )
}

// Recursively gathers the plan items in a container that carry a parentCompletionRule or a
// completionNeutralRule. An empty result means the completion evaluation is untouched.
fn collect_rule_bearing_plan_items<'a>(
    container: ContainerView<'a>,
    out: &mut Vec<&'a CmmnPlanItem>,
) {
    for plan_item in container.plan_items {
        if plan_item.parent_completion_rule.is_some() || plan_item.completion_neutral_rule.is_some()
        {
            out.push(plan_item);
        }
    }
    for stage in container.stages {
        collect_rule_bearing_plan_items(ContainerView::from_stage(stage), out);
    }
}

fn find_rule_plan_item<'a>(
    rule_items: &[&'a CmmnPlanItem],
    plan_item_id: &str,
) -> Option<&'a CmmnPlanItem> {
    rule_items
        .iter()
        .copied()
        .find(|plan_item| plan_item.id == plan_item_id)
}

// Evaluates the completionNeutral condition and repetition rule (ExpressionUtil.evaluate*),
// resolves the already-completed flag only when the rule requires it, then applies the Java
// ignore decision for the plan item in its current lifecycle state.
fn plan_item_state_is_ignored(
    session: &mut DbSession,
    case_instance: &CmmnCaseInstance,
    plan_item: &CmmnPlanItem,
    state: PlanItemCompletionState,
) -> Result<bool, CmmnError> {
    let completion_neutral = match plan_item.completion_neutral_rule.as_ref() {
        Some(rule) => evaluate_if_part_condition(rule, case_instance)?,
        None => false,
    };
    let repeatable = match plan_item.repetition_rule.as_ref() {
        Some(rule) => evaluate_if_part_condition(rule, case_instance)?,
        None => false,
    };
    let already_completed =
        if rule_needs_already_completed(plan_item.parent_completion_rule.as_deref()) {
            plan_item_standard_event_occurred(
                session,
                &case_instance.id,
                &plan_item.id,
                CmmnPlanItemOnPart::STANDARD_EVENT_COMPLETE,
            )?
        } else {
            false
        };
    Ok(plan_item_ignored_for_completion(
        plan_item.parent_completion_rule.as_deref(),
        completion_neutral,
        state,
        already_completed,
        repeatable,
    ))
}

// Reloads the same human-task rows a completion COUNT examined (identical scope + state filter)
// and returns how many carry a rule that ignores them for parent completion, collecting their
// plan item ids so the required-plan-item check can skip them too. Because it re-examines the
// counted rows, the returned value never exceeds the base count.
#[allow(clippy::too_many_arguments)]
fn ignored_open_human_tasks(
    session: &mut DbSession,
    case_instance: &CmmnCaseInstance,
    rule_items: &[&CmmnPlanItem],
    active_only: bool,
    scope_column: &str,
    scope_id: &str,
    ignored_ids: &mut HashSet<String>,
) -> Result<i64, CmmnError> {
    let state_filter = if active_only {
        "STATE_ = 'ACTIVE'"
    } else {
        "STATE_ <> 'COMPLETED'"
    };
    let mut params = DbParams::new();
    params.push(scope_id);
    let rendered = RenderedStatement::new(
        format!(
            "SELECT DATA_ FROM ACT_CMMN_HUMAN_TASK WHERE {scope_column} = ? AND {state_filter}"
        ),
        params,
    );
    let rows = session.select_raw(rendered)?;
    let mut ignored = 0;
    for row in rows {
        let json = row
            .get_text("DATA_")
            .ok_or_else(|| CmmnError::storage("Missing DATA_ in CMMN human task row"))?;
        let task: CmmnHumanTaskInstance = serde_json::from_str(&json)?;
        if let Some(plan_item) = find_rule_plan_item(rule_items, &task.plan_item_id) {
            let state = match task.state {
                CmmnHumanTaskState::Active => PlanItemCompletionState::Active,
                // Java PlanItemInstanceContainerUtil.java:122-145 keeps
                // ENABLED distinct from AVAILABLE for ignoreIfAvailable and
                // completion-neutral evaluation.
                CmmnHumanTaskState::Enabled => PlanItemCompletionState::Enabled,
                _ => PlanItemCompletionState::Available,
            };
            if plan_item_state_is_ignored(session, case_instance, plan_item, state)? {
                ignored += 1;
                ignored_ids.insert(task.plan_item_id.clone());
            }
        }
    }
    Ok(ignored)
}

// Same idea as `ignored_open_human_tasks`, for child stage instances.
#[allow(clippy::too_many_arguments)]
fn ignored_open_stage_instances(
    session: &mut DbSession,
    case_instance: &CmmnCaseInstance,
    rule_items: &[&CmmnPlanItem],
    active_only: bool,
    scope_column: &str,
    scope_id: &str,
    ignored_ids: &mut HashSet<String>,
) -> Result<i64, CmmnError> {
    let state_filter = if active_only {
        "STATE_ = 'ACTIVE'"
    } else {
        "STATE_ <> 'COMPLETED'"
    };
    let mut params = DbParams::new();
    params.push(scope_id);
    let rendered = RenderedStatement::new(
        format!(
            "SELECT DATA_ FROM ACT_CMMN_STAGE_INSTANCE WHERE {scope_column} = ? AND {state_filter}"
        ),
        params,
    );
    let rows = session.select_raw(rendered)?;
    let mut ignored = 0;
    for row in rows {
        let json = row
            .get_text("DATA_")
            .ok_or_else(|| CmmnError::storage("Missing DATA_ in CMMN stage instance row"))?;
        let stage_instance: CmmnStageInstance = serde_json::from_str(&json)?;
        if let Some(plan_item) = find_rule_plan_item(rule_items, &stage_instance.plan_item_id) {
            let state = match stage_instance.state {
                CmmnStageInstanceState::Active => PlanItemCompletionState::Active,
                // Java PlanItemInstanceContainerUtil.java:122-145 keeps
                // ENABLED distinct from AVAILABLE for ignoreIfAvailable and
                // completion-neutral evaluation.
                CmmnStageInstanceState::Enabled => PlanItemCompletionState::Enabled,
                _ => PlanItemCompletionState::Available,
            };
            if plan_item_state_is_ignored(session, case_instance, plan_item, state)? {
                ignored += 1;
                ignored_ids.insert(stage_instance.plan_item_id.clone());
            }
        }
    }
    Ok(ignored)
}

// Event subscriptions represent AVAILABLE event listeners. Restricting to `rule_items` (which for
// a stage scope only contains that stage's plan items) keeps this container-scoped.
fn ignored_open_event_subscriptions(
    session: &mut DbSession,
    case_instance: &CmmnCaseInstance,
    rule_items: &[&CmmnPlanItem],
    ignored_ids: &mut HashSet<String>,
) -> Result<i64, CmmnError> {
    let mut params = DbParams::new();
    params.push(case_instance.id.as_str());
    let rendered = RenderedStatement::new(
        "SELECT PLAN_ITEM_INSTANCE_ID_ FROM ACT_CMMN_EVENT_SUBSCRIPTION WHERE CASE_INSTANCE_ID_ = ?"
            .to_string(),
        params,
    );
    let rows = session.select_raw(rendered)?;
    let plan_item_ids = rows
        .into_iter()
        .filter_map(|row| row.get_text("PLAN_ITEM_INSTANCE_ID_"))
        .collect::<Vec<_>>();
    let mut ignored = 0;
    for plan_item_id in plan_item_ids {
        if let Some(plan_item) = find_rule_plan_item(rule_items, &plan_item_id) {
            if plan_item_state_is_ignored(
                session,
                case_instance,
                plan_item,
                PlanItemCompletionState::Available,
            )? {
                ignored += 1;
                ignored_ids.insert(plan_item_id);
            }
        }
    }
    Ok(ignored)
}

// Active process/case task associations are ACTIVE plan items in Java (always block unless a rule
// ignores them).
fn ignored_open_task_associations(
    session: &mut DbSession,
    case_instance: &CmmnCaseInstance,
    rule_items: &[&CmmnPlanItem],
    ignored_ids: &mut HashSet<String>,
) -> Result<i64, CmmnError> {
    let mut params = DbParams::new();
    params.push(case_instance.id.as_str());
    params.push(CmmnTaskAssociationState::Active.as_str());
    let rendered = RenderedStatement::new(
        "SELECT PLAN_ITEM_ID_ FROM ACT_CMMN_TASK_INSTANCE_ASSOCIATION \
         WHERE CASE_INSTANCE_ID_ = ? AND STATE_ = ?"
            .to_string(),
        params,
    );
    let rows = session.select_raw(rendered)?;
    let plan_item_ids = rows
        .into_iter()
        .filter_map(|row| row.get_text("PLAN_ITEM_ID_"))
        .collect::<Vec<_>>();
    let mut ignored = 0;
    for plan_item_id in plan_item_ids {
        if let Some(plan_item) = find_rule_plan_item(rule_items, &plan_item_id) {
            if plan_item_state_is_ignored(
                session,
                case_instance,
                plan_item,
                PlanItemCompletionState::Active,
            )? {
                ignored += 1;
                ignored_ids.insert(plan_item_id);
            }
        }
    }
    Ok(ignored)
}

// Unified mirror rows represent manual-activation ENABLED plan items and
// AVAILABLE timer/event-listener rows (see `count_blocking_mirror_plan_items`).
// Java applies parent-completion rules to both states
// (`PlanItemInstanceContainerUtil.java:122-146`); subtract rule-ignored rows from
// the blocking mirror count so IGNORE / IGNORE_IF_AVAILABLE stay consistent.
// AVAILABLE milestones are still considered for `ignored_ids` (required-item
// tracking) even though they do not contribute to the blocking count.
fn ignored_open_mirror_plan_items(
    session: &mut DbSession,
    case_instance: &CmmnCaseInstance,
    rule_items: &[&CmmnPlanItem],
    parent_stage_instance_id: Option<&str>,
    ignored_ids: &mut HashSet<String>,
) -> Result<i64, CmmnError> {
    let mut ignored = 0;
    for instance in load_plan_item_instances_session(session)? {
        if instance.case_instance_id != case_instance.id
            || !(instance.state == "ENABLED"
                || (instance.state == "AVAILABLE"
                    && (instance.plan_item_definition_type == "milestone"
                        || instance.plan_item_definition_type
                            == PLAN_ITEM_DEFINITION_TYPE_TIMER_EVENT_LISTENER
                        || instance.plan_item_definition_type
                            == PLAN_ITEM_DEFINITION_TYPE_EVENT_LISTENER)))
            || instance.ended_at.is_some()
            || parent_stage_instance_id
                .is_some_and(|stage_id| instance.stage_instance_id.as_deref() != Some(stage_id))
            || (parent_stage_instance_id.is_none() && instance.stage_instance_id.is_some())
        {
            continue;
        }
        if let Some(plan_item) = find_rule_plan_item(rule_items, &instance.plan_item_id) {
            let completion_state = if instance.state == "ENABLED" {
                PlanItemCompletionState::Enabled
            } else {
                PlanItemCompletionState::Available
            };
            if plan_item_state_is_ignored(
                session,
                case_instance,
                plan_item,
                completion_state,
            )? {
                // Only subtract rows that `count_blocking_mirror_plan_items` actually
                // counted (ENABLED always; AVAILABLE only for event-listener types).
                // AVAILABLE milestones still land in `ignored_ids` for required tracking.
                if mirror_state_blocks_non_autocomplete(
                    &instance.state,
                    &instance.plan_item_definition_type,
                ) {
                    ignored += 1;
                }
                ignored_ids.insert(instance.plan_item_id);
            }
        }
    }
    Ok(ignored)
}

/// Build the listener context for a case instance state transition and fire the matching
/// `flowable:caseLifecycleListener` entries declared on `model`
/// (CaseInstanceLifeCycleListenerUtil.java:41-48).
///
/// Free-function form so the session-only parts of the state machine (`maybe_complete_case` and
/// friends) can fire without a `&CmmnRuntimeService`; the registry is read from the ambient
/// thread-local installed by the service entry point.
fn fire_case_lifecycle_listeners_for_model(
    model: &CmmnCase,
    case_instance: &CmmnCaseInstance,
    old_state: &str,
    new_state: &str,
) -> Result<(), CmmnError> {
    let context = CmmnLifecycleListenerContext {
        scope: CmmnLifecycleScope::CaseInstance,
        case_instance_id: case_instance.id.clone(),
        case_definition_id: case_instance.case_definition_id.clone(),
        plan_item_instance_id: None,
        plan_item_definition_id: None,
        plan_item_definition_type: None,
        old_state: old_state.to_ascii_lowercase(),
        new_state: new_state.to_ascii_lowercase(),
        tenant_id: case_instance.tenant_id.clone(),
        variables: case_instance.variables.clone(),
    };
    fire_matching_lifecycle_listeners(&model.lifecycle_listeners, &context)
}

/// Session-scoped twin of [`fire_case_lifecycle_listeners_for_model`]: resolves the deployed
/// model through the caller's `session` rather than opening a second one.
///
/// Every case-level transition site already holds an open `DbSession`, and the store serialises
/// sessions, so going through `CmmnRepositoryService` here would block on the caller's own
/// session. Java has the same constraint expressed differently — the notification runs inside the
/// command context that already owns the transaction.
fn fire_case_lifecycle_listeners_session(
    session: &mut DbSession,
    case_instance: &CmmnCaseInstance,
    old_state: &str,
    new_state: &str,
) -> Result<(), CmmnError> {
    // CaseInstanceLifeCycleListenerUtil.java:36-38
    if old_state.eq_ignore_ascii_case(new_state) {
        return Ok(());
    }
    let case_definition =
        match load_case_definition_session(session, &case_instance.case_definition_id)? {
            Some(case_definition) => case_definition,
            // A deleted definition cannot declare listeners; nothing to fire.
            None => return Ok(()),
        };
    fire_case_lifecycle_listeners_for_model(
        &case_definition.model,
        case_instance,
        old_state,
        new_state,
    )
}

/// Plan-item twin of [`fire_case_lifecycle_listeners_for_model`]
/// (CmmnListenerNotificationHelper.java:111-115).
fn fire_plan_item_lifecycle_listeners_for_model(
    model: &CmmnCase,
    case_instance: &CmmnCaseInstance,
    plan_item_instance_id: Option<&str>,
    plan_item_definition_id: &str,
    plan_item_definition_type: Option<&str>,
    old_state: &str,
    new_state: &str,
) -> Result<(), CmmnError> {
    let listeners = model.plan_item_listeners(plan_item_definition_id);
    if listeners.is_empty() {
        return Ok(());
    }
    let context = CmmnLifecycleListenerContext {
        scope: CmmnLifecycleScope::PlanItem,
        case_instance_id: case_instance.id.clone(),
        case_definition_id: case_instance.case_definition_id.clone(),
        plan_item_instance_id: plan_item_instance_id.map(ToOwned::to_owned),
        plan_item_definition_id: Some(plan_item_definition_id.to_string()),
        plan_item_definition_type: plan_item_definition_type.map(ToOwned::to_owned),
        old_state: old_state.to_ascii_lowercase(),
        new_state: new_state.to_ascii_lowercase(),
        tenant_id: case_instance.tenant_id.clone(),
        variables: case_instance.variables.clone(),
    };
    fire_matching_lifecycle_listeners(listeners, &context)
}

/// Session-scoped convenience wrapper around [`fire_plan_item_lifecycle_listeners_for_model`]:
/// resolves the case instance and its deployed model from `session`, so a plan item transition
/// site only needs the ids and the two states.
///
/// Java gets the same data off the `PlanItemInstanceEntity` it is already holding
/// (CmmnListenerNotificationHelper.java:111). A missing case instance or definition means there
/// is nothing to fire.
fn fire_plan_item_lifecycle_listeners_session(
    session: &mut DbSession,
    case_instance_id: &str,
    plan_item_instance_id: Option<&str>,
    plan_item_definition_id: &str,
    plan_item_definition_type: Option<&str>,
    old_state: &str,
    new_state: &str,
) -> Result<(), CmmnError> {
    // CmmnListenerNotificationHelper.java:104-106
    if old_state.eq_ignore_ascii_case(new_state) {
        return Ok(());
    }
    let case_instance = match load_case_instance_session(session, case_instance_id)? {
        Some(case_instance) => case_instance,
        None => return Ok(()),
    };
    let case_definition =
        match load_case_definition_session(session, &case_instance.case_definition_id)? {
            Some(case_definition) => case_definition,
            None => return Ok(()),
        };
    fire_plan_item_lifecycle_listeners_for_model(
        &case_definition.model,
        &case_instance,
        plan_item_instance_id,
        plan_item_definition_id,
        plan_item_definition_type,
        old_state,
        new_state,
    )
}

fn maybe_complete_case(session: &mut DbSession, case_instance_id: &str) -> Result<(), CmmnError> {
    let mut case_instance = match load_case_instance_session(session, case_instance_id)? {
        Some(case_instance) => case_instance,
        None => return Ok(()),
    };
    if case_instance.state == CmmnCaseInstanceState::Completed {
        return Ok(());
    }
    if case_instance.state != CmmnCaseInstanceState::Active {
        return Ok(());
    }

    let case_definition = load_case_definition_session(session, &case_instance.case_definition_id)?
        .ok_or_else(|| {
            CmmnError::storage(format!(
                "CMMN case definition '{}' was not found",
                case_instance.case_definition_id
            ))
        })?;
    let case_plan_model = &case_definition.model.case_plan_model;
    let auto_complete = evaluate_auto_complete(
        case_plan_model.auto_complete,
        case_plan_model.auto_complete_condition.as_ref(),
        &case_instance,
    );

    // Java: PlanItemInstanceContainerUtil.java:91-97 - ACTIVE plan items always block;
    // :143-146 - AVAILABLE/ENABLED plan items only block when the container is not autocomplete.
    let open_tasks = if auto_complete {
        count_rows(
            session,
            "SELECT COUNT(*) AS CNT FROM ACT_CMMN_HUMAN_TASK \
             WHERE CASE_INSTANCE_ID_ = ? AND STATE_ = 'ACTIVE'",
            &[case_instance_id],
        )?
    } else {
        count_rows(
            session,
            "SELECT COUNT(*) AS CNT FROM ACT_CMMN_HUMAN_TASK \
             WHERE CASE_INSTANCE_ID_ = ? AND STATE_ <> 'COMPLETED'",
            &[case_instance_id],
        )?
    };
    let open_stages = if auto_complete {
        count_rows(
            session,
            "SELECT COUNT(*) AS CNT FROM ACT_CMMN_STAGE_INSTANCE \
             WHERE CASE_INSTANCE_ID_ = ? AND STATE_ = 'ACTIVE'",
            &[case_instance_id],
        )?
    } else {
        count_rows(
            session,
            "SELECT COUNT(*) AS CNT FROM ACT_CMMN_STAGE_INSTANCE \
             WHERE CASE_INSTANCE_ID_ = ? AND STATE_ <> 'COMPLETED'",
            &[case_instance_id],
        )?
    };
    // Java: PlanItemInstanceContainerUtil.java:144-146 - available event listeners and
    // enabled (manual-activation) plan items do not block an autocomplete container.
    let open_event_subscriptions = if auto_complete {
        0
    } else {
        count_rows(
            session,
            "SELECT COUNT(*) AS CNT FROM ACT_CMMN_EVENT_SUBSCRIPTION \
             WHERE CASE_INSTANCE_ID_ = ?",
            &[case_instance_id],
        )?
    };
    // Child process/case tasks are ACTIVE plan items in Java and always block (:91-97).
    let open_task_associations = count_rows_with_state(
        session,
        "SELECT COUNT(*) AS CNT FROM ACT_CMMN_TASK_INSTANCE_ASSOCIATION \
         WHERE CASE_INSTANCE_ID_ = ? AND STATE_ = ?",
        case_instance_id,
        CmmnTaskAssociationState::Active.as_str(),
    )?;
    // Java: PlanItemInstanceContainerUtil.java:143-146 - AVAILABLE/ENABLED mirror rows
    // block only when the container is not autocomplete (autoComplete branch stays 0).
    let blocking_plan_items = if auto_complete {
        0
    } else {
        count_blocking_mirror_plan_items(session, case_instance_id, None)?
    };
    let pending_case_discretionary_items = if auto_complete {
        0
    } else {
        pending_case_level_discretionary_item_count(session, &case_instance)?
    };

    // Java: PlanItemInstanceContainerUtil.java:73-190 - subtract plan items whose
    // parentCompletionRule / completionNeutralRule says to ignore them. No rule-bearing plan
    // items means the counts (and behavior) are untouched.
    let mut rule_items = Vec::new();
    collect_rule_bearing_plan_items(
        ContainerView::from_case_plan_model(case_plan_model),
        &mut rule_items,
    );
    let mut ignored_ids: HashSet<String> = HashSet::new();
    let (
        open_tasks,
        open_stages,
        open_event_subscriptions,
        open_task_associations,
        blocking_plan_items,
    ) = if rule_items.is_empty() {
        (
            open_tasks,
            open_stages,
            open_event_subscriptions,
            open_task_associations,
            blocking_plan_items,
        )
    } else {
        let task_ignored = ignored_open_human_tasks(
            session,
            &case_instance,
            &rule_items,
            auto_complete,
            "CASE_INSTANCE_ID_",
            case_instance_id,
            &mut ignored_ids,
        )?;
        let stage_ignored = ignored_open_stage_instances(
            session,
            &case_instance,
            &rule_items,
            auto_complete,
            "CASE_INSTANCE_ID_",
            case_instance_id,
            &mut ignored_ids,
        )?;
        let event_ignored = if auto_complete {
            0
        } else {
            ignored_open_event_subscriptions(
                session,
                &case_instance,
                &rule_items,
                &mut ignored_ids,
            )?
        };
        let association_ignored =
            ignored_open_task_associations(session, &case_instance, &rule_items, &mut ignored_ids)?;
        let blocking_ignored = if auto_complete {
            0
        } else {
            ignored_open_mirror_plan_items(
                session,
                &case_instance,
                &rule_items,
                None,
                &mut ignored_ids,
            )?
        };
        (
            open_tasks.saturating_sub(task_ignored),
            open_stages.saturating_sub(stage_ignored),
            open_event_subscriptions.saturating_sub(event_ignored),
            open_task_associations.saturating_sub(association_ignored),
            blocking_plan_items.saturating_sub(blocking_ignored),
        )
    };

    // Java: PlanItemInstanceContainerUtil.java:102-118 - required plan items always block.
    let incomplete_required = has_incomplete_required_plan_items(
        session,
        &case_definition,
        &case_instance,
        ContainerView::from_case_plan_model(case_plan_model),
        &ignored_ids,
    )?;

    if open_tasks == 0
        && open_stages == 0
        && open_event_subscriptions == 0
        && open_task_associations == 0
        && blocking_plan_items == 0
        && pending_case_discretionary_items == 0
        && !incomplete_required
    {
        if auto_complete {
            terminate_residual_case_children_for_auto_complete(
                session,
                &case_definition,
                case_instance_id,
            )?;
        }
        // Java fires the case lifecycle listeners before setState
        // (AbstractChangeCaseInstanceStateOperation.java:45,47), through
        // CaseInstanceLifeCycleListenerUtil.callLifecycleListeners
        // (CaseInstanceLifeCycleListenerUtil.java:35-85).
        fire_case_lifecycle_listeners_for_model(
            &case_definition.model,
            &case_instance,
            case_instance.state.as_str(),
            CmmnCaseInstanceState::Completed.as_str(),
        )?;
        case_instance.state = CmmnCaseInstanceState::Completed;
        let ended_at = Utc::now();
        case_instance.ended_at = Some(ended_at);
        // Lightweight-history choice: Java removes terminal runtime rows, while
        // Rust keeps them ended in the mirror for historic queries
        // (`AbstractMovePlanItemInstanceToTerminalStateOperation.java:74-143`).
        terminate_open_plan_item_instances_for_case_session(session, case_instance_id, ended_at)?;
        persist_case_instance_session(session, &case_instance)?;
        persist_historic_case_session(session, &CmmnHistoricCaseInstance::from(&case_instance))?;
        complete_parent_case_task_associations_for_child_case(session, case_instance_id)?;
    }

    Ok(())
}

// When an autocomplete case plan model completes, remaining non-active children
// (AVAILABLE/DISABLED tasks, available stages, event subscriptions, manual-activation
// markers) are exited, mirroring Java (PlanItemInstanceContainerUtil.java:143-146).
fn terminate_residual_case_children_for_auto_complete(
    session: &mut DbSession,
    case_definition: &CmmnCaseDefinition,
    case_instance_id: &str,
) -> Result<(), CmmnError> {
    let mut params = DbParams::new();
    params.push(case_instance_id);
    let rendered = RenderedStatement::new(
        "SELECT DATA_ FROM ACT_CMMN_STAGE_INSTANCE \
         WHERE CASE_INSTANCE_ID_ = ? AND STATE_ <> 'COMPLETED'"
            .to_string(),
        params,
    );
    let rows = session.select_raw(rendered)?;
    let residual_stage_instances = rows
        .into_iter()
        .map(|row| {
            let json = row
                .get_text("DATA_")
                .ok_or_else(|| CmmnError::storage("Missing DATA_ in CMMN stage instance row"))?;
            serde_json::from_str::<CmmnStageInstance>(&json).map_err(CmmnError::from)
        })
        .collect::<Result<Vec<_>, _>>()?;
    for residual_stage_instance in residual_stage_instances {
        // A nested residual stage may already be gone after its ancestor was terminated.
        if load_stage_instance_session(session, &residual_stage_instance.id)?.is_none() {
            continue;
        }
        let stage = find_stage_by_definition_id(
            &case_definition.model.case_plan_model.stages,
            &residual_stage_instance.stage_definition_id,
        )
        .ok_or_else(|| {
            CmmnError::storage(format!(
                "CMMN stage definition '{}' was not found in case definition '{}'",
                residual_stage_instance.stage_definition_id, case_definition.id
            ))
        })?;
        terminate_stage_instance(session, case_definition, residual_stage_instance, stage)?;
    }

    let mut params = DbParams::new();
    params.push(case_instance_id);
    let rendered = RenderedStatement::new(
        "SELECT DATA_ FROM ACT_CMMN_HUMAN_TASK \
         WHERE CASE_INSTANCE_ID_ = ? AND STATE_ <> 'COMPLETED'"
            .to_string(),
        params,
    );
    let rows = session.select_raw(rendered)?;
    let residual_tasks = rows
        .into_iter()
        .map(|row| {
            let json = row
                .get_text("DATA_")
                .ok_or_else(|| CmmnError::storage("Missing DATA_ in CMMN human task row"))?;
            serde_json::from_str::<CmmnHumanTaskInstance>(&json).map_err(CmmnError::from)
        })
        .collect::<Result<Vec<_>, _>>()?;
    for mut task in residual_tasks {
        task.state = CmmnHumanTaskState::Terminated;
        task.completed_at = Some(Utc::now());
        persist_historic_human_task_session(session, &CmmnHistoricHumanTaskInstance::from(&task))?;
        let mut params = DbParams::new();
        params.push(task.id.as_str());
        session.execute_raw(RenderedStatement::new(
            "DELETE FROM ACT_CMMN_HUMAN_TASK WHERE ID_ = ?".to_string(),
            params,
        ))?;
    }

    let mut params = DbParams::new();
    params.push(case_instance_id);
    session.execute_raw(RenderedStatement::new(
        "DELETE FROM ACT_CMMN_EVENT_SUBSCRIPTION WHERE CASE_INSTANCE_ID_ = ?".to_string(),
        params,
    ))?;

    Ok(())
}

fn pending_case_level_discretionary_item_count(
    session: &mut DbSession,
    case_instance: &CmmnCaseInstance,
) -> Result<usize, CmmnError> {
    let Some(case_definition) =
        load_case_definition_session(session, &case_instance.case_definition_id)?
    else {
        return Ok(0);
    };
    let mut pending = 0;
    for planning_table in &case_definition.model.case_plan_model.planning_tables {
        for discretionary_item in &planning_table.discretionary_items {
            if !human_task_instance_exists(
                session,
                &case_instance.id,
                &discretionary_item.id,
                None,
            )? {
                pending += 1;
            }
        }
    }
    Ok(pending)
}

fn human_task_instance_exists(
    session: &mut DbSession,
    case_instance_id: &str,
    plan_item_id: &str,
    stage_instance_id: Option<&str>,
) -> Result<bool, CmmnError> {
    let mut params = DbParams::new();
    params.push(case_instance_id);
    let rendered = RenderedStatement::new(
        "SELECT DATA_ FROM ACT_CMMN_HUMAN_TASK WHERE CASE_INSTANCE_ID_ = ?".to_string(),
        params,
    );
    let rows = session.select_raw(rendered)?;
    let tasks = rows
        .into_iter()
        .map(|row| {
            let json = row
                .get_text("DATA_")
                .ok_or_else(|| CmmnError::storage("Missing DATA_ in CMMN human task row"))?;
            serde_json::from_str::<CmmnHumanTaskInstance>(&json).map_err(CmmnError::from)
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(tasks.iter().any(|task| {
        task.plan_item_id == plan_item_id && task.stage_instance_id.as_deref() == stage_instance_id
    }))
}

fn validate_runtime_migration_state(
    store: &CmmnStore,
    case_instance: &CmmnCaseInstance,
    document: &CmmnMigrationDocument,
) -> Result<CmmnMigrationValidationResult, CmmnError> {
    if case_instance.case_definition_id == document.target_case_definition_id {
        return Ok(CmmnMigrationValidationResult::valid());
    }

    let mut session = store.create_session()?;
    let open_tasks = count_rows(
        &mut session,
        "SELECT COUNT(*) AS CNT FROM ACT_CMMN_HUMAN_TASK \
         WHERE CASE_INSTANCE_ID_ = ? AND STATE_ <> 'COMPLETED'",
        &[case_instance.id.as_str()],
    )?;
    let open_stages = count_rows(
        &mut session,
        "SELECT COUNT(*) AS CNT FROM ACT_CMMN_STAGE_INSTANCE \
         WHERE CASE_INSTANCE_ID_ = ? AND STATE_ <> 'COMPLETED'",
        &[case_instance.id.as_str()],
    )?;

    if open_tasks > 0 || open_stages > 0 {
        return Ok(CmmnMigrationValidationResult::invalid(format!(
            "CMMN case instance '{}' has active plan item instances and cannot be safely migrated to a different case definition",
            case_instance.id
        )));
    }

    Ok(CmmnMigrationValidationResult::valid())
}

fn count_rows(session: &mut DbSession, sql: &str, args: &[&str]) -> Result<i64, CmmnError> {
    let mut params = DbParams::new();
    for arg in args {
        params.push(*arg);
    }
    let rendered = RenderedStatement::new(sql.to_string(), params);
    let row = session
        .select_one_raw(rendered)?
        .ok_or_else(|| CmmnError::storage("Missing COUNT row"))?;
    Ok(row.get_integer("CNT").unwrap_or(0))
}

fn count_rows_with_state(
    session: &mut DbSession,
    sql: &str,
    case_instance_id: &str,
    state_value: &str,
) -> Result<i64, CmmnError> {
    let mut params = DbParams::new();
    params.push(case_instance_id);
    params.push(state_value);
    let rendered = RenderedStatement::new(sql.to_string(), params);
    let row = session
        .select_one_raw(rendered)?
        .ok_or_else(|| CmmnError::storage("Missing COUNT row"))?;
    Ok(row.get_integer("CNT").unwrap_or(0))
}

fn apply_case_definition_to_runtime_case(
    case_instance: &mut CmmnCaseInstance,
    target_definition: &CmmnCaseDefinition,
) {
    case_instance.case_definition_id = target_definition.id.clone();
    case_instance.deployment_id = target_definition.deployment_id.clone();
    case_instance.case_definition_key = target_definition.key.clone();
    case_instance.case_definition_name = target_definition.name.clone();
    case_instance.case_definition_version = target_definition.version;
    case_instance.tenant_id = case_instance
        .tenant_id
        .clone()
        .or_else(|| target_definition.tenant_id.clone());
}

enum ActivePlanItemInstance {
    HumanTask(CmmnHumanTaskInstance),
    Stage(CmmnStageInstance),
}

#[derive(Clone, Copy)]
enum PlanItemActivationTarget<'a> {
    HumanTask(&'a CmmnPlanItem, &'a CmmnHumanTask),
    Stage(&'a CmmnPlanItem, &'a CmmnStage),
}

#[derive(Clone, Copy)]
enum PlanItemDefinitionActivationTarget<'a> {
    HumanTask(&'a CmmnPlanItem, &'a CmmnHumanTask),
    Stage(&'a CmmnPlanItem, &'a CmmnStage),
    DecisionTask(&'a CmmnPlanItem, &'a CmmnDecisionTask),
    Milestone(&'a CmmnPlanItem, &'a CmmnMilestone),
    EventListener(&'a CmmnPlanItem, &'a CmmnEventListener),
}

impl ActivePlanItemInstance {
    fn plan_item_id(&self) -> &str {
        match self {
            Self::HumanTask(task) => &task.plan_item_id,
            Self::Stage(stage_instance) => &stage_instance.plan_item_id,
        }
    }

    fn parent_stage_instance_id(&self) -> Option<&str> {
        match self {
            Self::HumanTask(task) => task.stage_instance_id.as_deref(),
            Self::Stage(stage_instance) => stage_instance.parent_stage_instance_id.as_deref(),
        }
    }

    fn kind(&self) -> &'static str {
        match self {
            Self::HumanTask(_) => "humanTask",
            Self::Stage(_) => "stage",
        }
    }
}

fn change_plan_item_instances_by_target_plan_item_ids(
    session: &mut DbSession,
    case_definition: &CmmnCaseDefinition,
    case_instance: &CmmnCaseInstance,
    changes: &[(String, String)],
) -> Result<(), CmmnError> {
    for (plan_item_instance_id, target_plan_item_id) in changes {
        let target = find_activation_target_by_plan_item_id(
            &case_definition.model.case_plan_model,
            target_plan_item_id,
        )
        .ok_or_else(|| {
            CmmnError::not_found(format!(
                "CMMN target plan item '{target_plan_item_id}' was not found in case definition '{}'",
                case_definition.id
            ))
        })?;
        change_plan_item_instance_to_target(
            session,
            case_definition,
            case_instance,
            plan_item_instance_id,
            target,
        )?;
    }
    Ok(())
}

fn change_plan_item_instances_by_target_definition_ids(
    session: &mut DbSession,
    case_definition: &CmmnCaseDefinition,
    case_instance: &CmmnCaseInstance,
    changes: &[(String, String)],
) -> Result<(), CmmnError> {
    for (plan_item_instance_id, target_definition_id) in changes {
        let target = find_activation_target_by_definition_id(
            &case_definition.model.case_plan_model,
            target_definition_id,
        )
        .ok_or_else(|| {
            CmmnError::not_found(format!(
                "CMMN target plan item definition '{target_definition_id}' was not found in case definition '{}'",
                case_definition.id
            ))
        })?;
        change_plan_item_instance_to_target(
            session,
            case_definition,
            case_instance,
            plan_item_instance_id,
            target,
        )?;
    }
    Ok(())
}

fn change_plan_item_definitions_with_new_target_ids(
    session: &mut DbSession,
    case_definition: &CmmnCaseDefinition,
    case_instance: &CmmnCaseInstance,
    changes: &[CmmnPlanItemDefinitionWithTargetIds],
) -> Result<(), CmmnError> {
    for change in changes {
        let target = find_activation_target_by_plan_item_id(
            &case_definition.model.case_plan_model,
            &change.new_plan_item_id,
        )
        .ok_or_else(|| {
            CmmnError::not_found(format!(
                "CMMN target plan item '{}' was not found in case definition '{}'",
                change.new_plan_item_id, case_definition.id
            ))
        })?;
        ensure_target_definition_matches(
            &target,
            &change.new_plan_item_id,
            &change.new_plan_item_definition_id,
        )?;

        match target {
            PlanItemActivationTarget::HumanTask(_, _) => {
                let tasks = load_active_human_task_instances_by_definition_id(
                    session,
                    &case_instance.id,
                    &change.existing_plan_item_definition_id,
                )?;
                if tasks.is_empty() {
                    return Err(active_definition_not_found(
                        &case_instance.id,
                        &change.existing_plan_item_definition_id,
                    ));
                }
                for task in tasks {
                    change_loaded_plan_item_instance_to_target(
                        session,
                        case_definition,
                        case_instance,
                        ActivePlanItemInstance::HumanTask(task),
                        target,
                    )?;
                }
            }
            PlanItemActivationTarget::Stage(_, _) => {
                let stage_instances = load_active_stage_instances_by_definition_id(
                    session,
                    &case_instance.id,
                    &change.existing_plan_item_definition_id,
                )?;
                if stage_instances.is_empty() {
                    return Err(active_definition_not_found(
                        &case_instance.id,
                        &change.existing_plan_item_definition_id,
                    ));
                }
                for stage_instance in stage_instances {
                    change_loaded_plan_item_instance_to_target(
                        session,
                        case_definition,
                        case_instance,
                        ActivePlanItemInstance::Stage(stage_instance),
                        target,
                    )?;
                }
            }
        }
    }
    Ok(())
}

fn change_plan_item_instance_to_target(
    session: &mut DbSession,
    case_definition: &CmmnCaseDefinition,
    case_instance: &CmmnCaseInstance,
    plan_item_instance_id: &str,
    target: PlanItemActivationTarget<'_>,
) -> Result<(), CmmnError> {
    let source =
        load_active_plan_item_instance_by_id(session, &case_instance.id, plan_item_instance_id)?
            .ok_or_else(|| {
                CmmnError::not_found(format!(
                    "Active CMMN plan item instance '{plan_item_instance_id}' was not found in case instance '{}'",
                    case_instance.id
                ))
            })?;
    change_loaded_plan_item_instance_to_target(
        session,
        case_definition,
        case_instance,
        source,
        target,
    )
}

fn change_loaded_plan_item_instance_to_target(
    session: &mut DbSession,
    case_definition: &CmmnCaseDefinition,
    case_instance: &CmmnCaseInstance,
    source: ActivePlanItemInstance,
    target: PlanItemActivationTarget<'_>,
) -> Result<(), CmmnError> {
    ensure_supported_change_shape(&source, &target)?;
    let parent_stage_instance_id = source.parent_stage_instance_id().map(str::to_string);
    let terminated_plan_items =
        terminate_active_plan_item_instance(session, source, case_definition)?;
    for (plan_item_id, parent_stage_instance_id) in terminated_plan_items {
        handle_plan_item_standard_event(
            session,
            case_definition,
            &case_instance.id,
            &plan_item_id,
            CmmnPlanItemOnPart::STANDARD_EVENT_TERMINATE,
            parent_stage_instance_id.as_deref(),
        )?;
    }
    activate_plan_item_target(
        session,
        case_definition,
        case_instance,
        target,
        parent_stage_instance_id.as_deref(),
    )
}

fn ensure_supported_change_shape(
    source: &ActivePlanItemInstance,
    target: &PlanItemActivationTarget<'_>,
) -> Result<(), CmmnError> {
    match (source, target) {
        (ActivePlanItemInstance::HumanTask(_), PlanItemActivationTarget::HumanTask(_, _))
        | (ActivePlanItemInstance::Stage(_), PlanItemActivationTarget::Stage(_, _)) => Ok(()),
        _ => Err(CmmnError::unsupported(
            "change-state",
            format!(
                "id-based change from {} plan item '{}' to target plan item '{}' is outside the supported runtime subset",
                source.kind(),
                source.plan_item_id(),
                target.plan_item().id
            ),
        )),
    }
}

fn terminate_active_plan_item_instance(
    session: &mut DbSession,
    source: ActivePlanItemInstance,
    case_definition: &CmmnCaseDefinition,
) -> Result<Vec<(String, Option<String>)>, CmmnError> {
    match source {
        ActivePlanItemInstance::HumanTask(mut task) => {
            task.state = CmmnHumanTaskState::Terminated;
            task.completed_at = Some(Utc::now());
            persist_historic_human_task_session(
                session,
                &CmmnHistoricHumanTaskInstance::from(&task),
            )?;
            let mut params = DbParams::new();
            params.push(task.id.as_str());
            session.execute_raw(RenderedStatement::new(
                "DELETE FROM ACT_CMMN_HUMAN_TASK WHERE ID_ = ?".to_string(),
                params,
            ))?;
            Ok(vec![(task.plan_item_id, task.stage_instance_id)])
        }
        ActivePlanItemInstance::Stage(stage_instance) => {
            let stage = find_stage_by_definition_id(
                &case_definition.model.case_plan_model.stages,
                &stage_instance.stage_definition_id,
            )
            .ok_or_else(|| {
                CmmnError::storage(format!(
                    "CMMN stage definition '{}' was not found in case definition '{}'",
                    stage_instance.stage_definition_id, case_definition.id
                ))
            })?;
            terminate_stage_instance(session, case_definition, stage_instance, stage)
        }
    }
}

fn activate_plan_item_target(
    session: &mut DbSession,
    case_definition: &CmmnCaseDefinition,
    case_instance: &CmmnCaseInstance,
    target: PlanItemActivationTarget<'_>,
    parent_stage_instance_id: Option<&str>,
) -> Result<(), CmmnError> {
    match target {
        PlanItemActivationTarget::HumanTask(plan_item, human_task) => activate_human_task(
            session,
            case_definition,
            case_instance,
            plan_item,
            human_task,
            parent_stage_instance_id,
        ),
        PlanItemActivationTarget::Stage(plan_item, stage) => activate_stage(
            session,
            case_definition,
            case_instance,
            plan_item,
            stage,
            parent_stage_instance_id,
        ),
    }
}

impl<'a> PlanItemActivationTarget<'a> {
    fn plan_item(&self) -> &'a CmmnPlanItem {
        match self {
            Self::HumanTask(plan_item, _) | Self::Stage(plan_item, _) => plan_item,
        }
    }

    fn definition_id(&self) -> &'a str {
        match self {
            Self::HumanTask(_, human_task) => &human_task.id,
            Self::Stage(_, stage) => &stage.id,
        }
    }
}

fn ensure_target_definition_matches(
    target: &PlanItemActivationTarget<'_>,
    target_plan_item_id: &str,
    expected_definition_id: &str,
) -> Result<(), CmmnError> {
    if target.definition_id() == expected_definition_id {
        return Ok(());
    }
    Err(CmmnError::validation(format!(
        "CMMN target plan item '{target_plan_item_id}' references definition '{}', not '{expected_definition_id}'",
        target.definition_id()
    )))
}

fn load_active_plan_item_instance_by_id(
    session: &mut DbSession,
    case_instance_id: &str,
    plan_item_instance_id: &str,
) -> Result<Option<ActivePlanItemInstance>, CmmnError> {
    if let Some(task) =
        load_active_human_task_instance_by_id(session, case_instance_id, plan_item_instance_id)?
    {
        return Ok(Some(ActivePlanItemInstance::HumanTask(task)));
    }
    if let Some(stage_instance) =
        load_active_stage_instance_by_id(session, case_instance_id, plan_item_instance_id)?
    {
        return Ok(Some(ActivePlanItemInstance::Stage(stage_instance)));
    }
    Ok(None)
}

fn load_active_human_task_instance_by_id(
    session: &mut DbSession,
    case_instance_id: &str,
    task_id: &str,
) -> Result<Option<CmmnHumanTaskInstance>, CmmnError> {
    let mut params = DbParams::new();
    params.push(case_instance_id);
    params.push(task_id);
    let rendered = RenderedStatement::new(
        "SELECT DATA_ FROM ACT_CMMN_HUMAN_TASK WHERE CASE_INSTANCE_ID_ = ? AND ID_ = ? AND STATE_ <> 'COMPLETED'".to_string(),
        params,
    );
    let row = session.select_one_raw(rendered)?;
    row.map(|row| {
        let json = row
            .get_text("DATA_")
            .ok_or_else(|| CmmnError::storage("Missing DATA_ in CMMN human task row"))?;
        serde_json::from_str::<CmmnHumanTaskInstance>(&json).map_err(CmmnError::from)
    })
    .transpose()
}

fn load_active_stage_instance_by_id(
    session: &mut DbSession,
    case_instance_id: &str,
    stage_instance_id: &str,
) -> Result<Option<CmmnStageInstance>, CmmnError> {
    let mut params = DbParams::new();
    params.push(case_instance_id);
    params.push(stage_instance_id);
    let rendered = RenderedStatement::new(
        "SELECT DATA_ FROM ACT_CMMN_STAGE_INSTANCE WHERE CASE_INSTANCE_ID_ = ? AND ID_ = ? AND STATE_ <> 'COMPLETED'".to_string(),
        params,
    );
    let row = session.select_one_raw(rendered)?;
    row.map(|row| {
        let json = row
            .get_text("DATA_")
            .ok_or_else(|| CmmnError::storage("Missing DATA_ in CMMN stage instance row"))?;
        serde_json::from_str::<CmmnStageInstance>(&json).map_err(CmmnError::from)
    })
    .transpose()
}

fn load_active_human_task_instances_by_definition_id(
    session: &mut DbSession,
    case_instance_id: &str,
    definition_id: &str,
) -> Result<Vec<CmmnHumanTaskInstance>, CmmnError> {
    let mut params = DbParams::new();
    params.push(case_instance_id);
    let rendered = RenderedStatement::new(
        "SELECT DATA_ FROM ACT_CMMN_HUMAN_TASK WHERE CASE_INSTANCE_ID_ = ? AND STATE_ <> 'COMPLETED'".to_string(),
        params,
    );
    let rows = session.select_raw(rendered)?;
    let tasks = rows
        .into_iter()
        .map(|row| {
            let json = row
                .get_text("DATA_")
                .ok_or_else(|| CmmnError::storage("Missing DATA_ in CMMN human task row"))?;
            serde_json::from_str::<CmmnHumanTaskInstance>(&json).map_err(CmmnError::from)
        })
        .filter(|task| {
            task.as_ref().is_ok_and(|task| {
                task.task_definition_id == definition_id || task.plan_item_id == definition_id
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(tasks)
}

fn load_active_stage_instances_by_definition_id(
    session: &mut DbSession,
    case_instance_id: &str,
    definition_id: &str,
) -> Result<Vec<CmmnStageInstance>, CmmnError> {
    let mut params = DbParams::new();
    params.push(case_instance_id);
    let rendered = RenderedStatement::new(
        "SELECT DATA_ FROM ACT_CMMN_STAGE_INSTANCE WHERE CASE_INSTANCE_ID_ = ? AND STATE_ <> 'COMPLETED'".to_string(),
        params,
    );
    let rows = session.select_raw(rendered)?;
    let stage_instances = rows
        .into_iter()
        .map(|row| {
            let json = row
                .get_text("DATA_")
                .ok_or_else(|| CmmnError::storage("Missing DATA_ in CMMN stage instance row"))?;
            serde_json::from_str::<CmmnStageInstance>(&json).map_err(CmmnError::from)
        })
        .filter(|stage_instance| {
            stage_instance.as_ref().is_ok_and(|stage_instance| {
                stage_instance.stage_definition_id == definition_id
                    || stage_instance.plan_item_id == definition_id
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(stage_instances)
}

fn active_definition_not_found(case_instance_id: &str, definition_id: &str) -> CmmnError {
    CmmnError::not_found(format!(
        "Active CMMN plan item definition '{definition_id}' was not found in case instance '{case_instance_id}'"
    ))
}

fn terminate_human_task_plan_item(
    session: &mut DbSession,
    case_instance_id: &str,
    plan_item_id: &str,
) -> Result<Vec<(String, Option<String>)>, CmmnError> {
    let mut params = DbParams::new();
    params.push(case_instance_id);
    let rendered = RenderedStatement::new(
        "SELECT DATA_ FROM ACT_CMMN_HUMAN_TASK WHERE CASE_INSTANCE_ID_ = ? AND STATE_ <> 'COMPLETED'".to_string(),
        params,
    );
    let rows = session.select_raw(rendered)?;
    let tasks = rows
        .into_iter()
        .map(|row| {
            let json = row
                .get_text("DATA_")
                .ok_or_else(|| CmmnError::storage("Missing DATA_ in CMMN human task row"))?;
            serde_json::from_str::<CmmnHumanTaskInstance>(&json).map_err(CmmnError::from)
        })
        .filter(|task| {
            task.as_ref()
                .is_ok_and(|task| task.plan_item_id == plan_item_id)
        })
        .collect::<Result<Vec<_>, _>>()?;

    let mut terminated_plan_items = Vec::new();
    for mut task in tasks {
        // Java routes every terminal move through the notification helper before
        // assigning the state (`AbstractMovePlanItemInstanceToTerminalStateOperation.java:74-143`,
        // `CmmnListenerNotificationHelper.java:103-159`).
        fire_plan_item_lifecycle_listeners_session(
            session,
            &task.case_instance_id,
            Some(&task.id),
            &task.task_definition_id,
            Some("humantask"),
            task.state.as_str(),
            CmmnHumanTaskState::Terminated.as_str(),
        )?;
        task.state = CmmnHumanTaskState::Terminated;
        task.completed_at = Some(Utc::now());
        persist_historic_human_task_session(session, &CmmnHistoricHumanTaskInstance::from(&task))?;
        terminated_plan_items.push((task.plan_item_id.clone(), task.stage_instance_id.clone()));
        let mut params = DbParams::new();
        params.push(task.id.as_str());
        session.execute_raw(RenderedStatement::new(
            "DELETE FROM ACT_CMMN_HUMAN_TASK WHERE ID_ = ?".to_string(),
            params,
        ))?;
    }

    Ok(terminated_plan_items)
}

fn terminate_stage_plan_item(
    session: &mut DbSession,
    case_definition: &CmmnCaseDefinition,
    case_instance_id: &str,
    plan_item: &CmmnPlanItem,
    stage: &CmmnStage,
) -> Result<Vec<(String, Option<String>)>, CmmnError> {
    let mut params = DbParams::new();
    params.push(case_instance_id);
    let rendered = RenderedStatement::new(
        "SELECT DATA_ FROM ACT_CMMN_STAGE_INSTANCE WHERE CASE_INSTANCE_ID_ = ? AND STATE_ <> 'COMPLETED'".to_string(),
        params,
    );
    let rows = session.select_raw(rendered)?;
    let stage_instances = rows
        .into_iter()
        .map(|row| {
            let json = row
                .get_text("DATA_")
                .ok_or_else(|| CmmnError::storage("Missing DATA_ in CMMN stage instance row"))?;
            serde_json::from_str::<CmmnStageInstance>(&json).map_err(CmmnError::from)
        })
        .filter(|stage_instance| {
            stage_instance
                .as_ref()
                .is_ok_and(|stage_instance| stage_instance.plan_item_id == plan_item.id)
        })
        .collect::<Result<Vec<_>, _>>()?;

    let mut terminated_plan_items = Vec::new();
    for stage_instance in stage_instances {
        let parent_stage_instance_id = stage_instance.parent_stage_instance_id.clone();
        terminated_plan_items.extend(terminate_stage_instance(
            session,
            case_definition,
            stage_instance,
            stage,
        )?);
        let parent_is_active = match parent_stage_instance_id.as_deref() {
            Some(parent_stage_instance_id) => {
                stage_instance_is_active(session, parent_stage_instance_id)?
            }
            None => true,
        };
        let case_instance = load_case_instance_session(session, case_instance_id)?;
        let should_repeat = match case_instance.as_ref() {
            Some(case_instance) => repetition_rule_matches(plan_item, case_instance)?,
            None => false,
        };
        if should_repeat
            && parent_is_active
            && !open_stage_instance_exists(session, case_instance_id, &plan_item.id)?
            && let Some(case_instance) = case_instance
        {
            create_stage_instance(
                session,
                case_definition,
                &case_instance,
                plan_item,
                stage,
                parent_stage_instance_id.as_deref(),
                CmmnStageInstanceState::Available,
            )?;
        }
    }

    Ok(terminated_plan_items)
}

fn terminate_stage_instance(
    session: &mut DbSession,
    case_definition: &CmmnCaseDefinition,
    mut stage_instance: CmmnStageInstance,
    stage: &CmmnStage,
) -> Result<Vec<(String, Option<String>)>, CmmnError> {
    let mut terminated_plan_items = Vec::new();

    for child_stage in &stage.stages {
        let child_plan_items = stage
            .plan_items
            .iter()
            .filter(|plan_item| plan_item.definition_ref == child_stage.id)
            .collect::<Vec<_>>();
        for child_plan_item in child_plan_items {
            terminated_plan_items.extend(terminate_stage_plan_item(
                session,
                case_definition,
                &stage_instance.case_instance_id,
                child_plan_item,
                child_stage,
            )?);
        }
    }

    let mut params = DbParams::new();
    params.push(stage_instance.id.as_str());
    let rendered = RenderedStatement::new(
        "SELECT DATA_ FROM ACT_CMMN_HUMAN_TASK WHERE STAGE_INSTANCE_ID_ = ? AND STATE_ <> 'COMPLETED'".to_string(),
        params,
    );
    let rows = session.select_raw(rendered)?;
    let child_tasks = rows
        .into_iter()
        .map(|row| {
            let json = row
                .get_text("DATA_")
                .ok_or_else(|| CmmnError::storage("Missing DATA_ in CMMN human task row"))?;
            serde_json::from_str::<CmmnHumanTaskInstance>(&json).map_err(CmmnError::from)
        })
        .collect::<Result<Vec<_>, _>>()?;

    for mut task in child_tasks {
        // Cascaded child termination has the same Java listener path
        // (`AbstractMovePlanItemInstanceToTerminalStateOperation.java:74-143`,
        // `CmmnListenerNotificationHelper.java:103-159`).
        fire_plan_item_lifecycle_listeners_session(
            session,
            &task.case_instance_id,
            Some(&task.id),
            &task.task_definition_id,
            Some("humantask"),
            task.state.as_str(),
            CmmnHumanTaskState::Terminated.as_str(),
        )?;
        task.state = CmmnHumanTaskState::Terminated;
        task.completed_at = Some(Utc::now());
        persist_historic_human_task_session(session, &CmmnHistoricHumanTaskInstance::from(&task))?;
        terminated_plan_items.push((task.plan_item_id.clone(), task.stage_instance_id.clone()));
        let mut params = DbParams::new();
        params.push(task.id.as_str());
        session.execute_raw(RenderedStatement::new(
            "DELETE FROM ACT_CMMN_HUMAN_TASK WHERE ID_ = ?".to_string(),
            params,
        ))?;
    }

    delete_event_subscriptions_for_container(
        session,
        &stage_instance.case_instance_id,
        ContainerView::from_stage(stage),
    )?;

    // P126: terminating a stage is a plan item state transition
    // (AbstractMovePlanItemInstanceToTerminalStateOperation.java:124).
    fire_plan_item_lifecycle_listeners_session(
        session,
        &stage_instance.case_instance_id,
        Some(&stage_instance.id),
        &stage_instance.stage_definition_id,
        Some("stage"),
        stage_instance.state.as_str(),
        CmmnStageInstanceState::Terminated.as_str(),
    )?;
    stage_instance.state = CmmnStageInstanceState::Terminated;
    stage_instance.ended_at = Some(Utc::now());
    persist_historic_stage_instance_session(session, &stage_instance)?;
    let mut params = DbParams::new();
    params.push(stage_instance.id.as_str());
    session.execute_raw(RenderedStatement::new(
        "DELETE FROM ACT_CMMN_STAGE_INSTANCE WHERE ID_ = ?".to_string(),
        params,
    ))?;

    // Java's terminal operation preserves TERMINATED history after removing the
    // runtime row (`AbstractMovePlanItemInstanceToTerminalStateOperation.java:74-143`).
    terminate_plan_item_instance_rows(
        session,
        &stage_instance.case_instance_id,
        &stage_instance.plan_item_id,
        "stage",
    )?;

    terminated_plan_items.push((
        stage_instance.plan_item_id,
        stage_instance.parent_stage_instance_id,
    ));

    Ok(terminated_plan_items)
}

fn terminate_occurred_milestone_plan_item(
    session: &mut DbSession,
    case_instance_id: &str,
    plan_item_id: &str,
) -> Result<Vec<(String, Option<String>)>, CmmnError> {
    if !plan_item_standard_event_occurred(
        session,
        case_instance_id,
        plan_item_id,
        CmmnPlanItemOnPart::STANDARD_EVENT_OCCUR,
    )? || plan_item_standard_event_occurred(
        session,
        case_instance_id,
        plan_item_id,
        CmmnPlanItemOnPart::STANDARD_EVENT_TERMINATE,
    )? {
        return Ok(Vec::new());
    }

    // Occurred milestones still pass through the common terminal notification
    // path (`AbstractMovePlanItemInstanceToTerminalStateOperation.java:74-143`,
    // `CmmnListenerNotificationHelper.java:103-159`).
    if let Some(instance) = load_plan_item_instances_session(session)?
        .into_iter()
        .find(|instance| {
            instance.case_instance_id == case_instance_id
                && instance.plan_item_id == plan_item_id
                && instance.plan_item_definition_type == "milestone"
        })
    {
        fire_plan_item_lifecycle_listeners_session(
            session,
            case_instance_id,
            Some(&instance.id),
            &instance.plan_item_definition_id,
            Some("milestone"),
            &instance.state,
            "TERMINATED",
        )?;
    }

    record_plan_item_standard_event_session(
        session,
        case_instance_id,
        plan_item_id,
        CmmnPlanItemOnPart::STANDARD_EVENT_TERMINATE,
    )?;
    terminate_plan_item_instance_rows(session, case_instance_id, plan_item_id, "milestone")?;
    Ok(vec![(plan_item_id.to_string(), None)])
}

fn terminate_event_listener_plan_item(
    session: &mut DbSession,
    case_instance_id: &str,
    plan_item_id: &str,
) -> Result<Vec<(String, Option<String>)>, CmmnError> {
    // A listener is "active" when it owns an event subscription (generic / variable
    // listeners) or a timer job (timer event listeners). Capture both before deletion.
    let had_subscription = count_rows(
        session,
        "SELECT COUNT(*) AS CNT FROM ACT_CMMN_EVENT_SUBSCRIPTION \
         WHERE CASE_INSTANCE_ID_ = ? AND PLAN_ITEM_INSTANCE_ID_ = ?",
        &[case_instance_id, plan_item_id],
    )? > 0;
    let had_timer_job = timer_job_for_plan_item_exists(session, case_instance_id, plan_item_id)?;

    // Java TimerEventListenerActivityBehaviour.onStateTransition (:72-77): DISMISS /
    // TERMINATE / EXIT removes the timer job (removeTimerJob :214-221).
    let mut params = DbParams::new();
    params.push(case_instance_id);
    params.push(plan_item_id);
    session.execute_raw(RenderedStatement::new(
        "DELETE FROM ACT_CMMN_EVENT_SUBSCRIPTION WHERE CASE_INSTANCE_ID_ = ? AND PLAN_ITEM_INSTANCE_ID_ = ?".to_string(),
        params,
    ))?;
    delete_timer_jobs_for_plan_item(session, case_instance_id, plan_item_id)?;

    if !had_subscription && !had_timer_job
        || plan_item_standard_event_occurred(
            session,
            case_instance_id,
            plan_item_id,
            CmmnPlanItemOnPart::STANDARD_EVENT_TERMINATE,
        )?
    {
        return Ok(Vec::new());
    }

    for instance in load_plan_item_instances_session(session)?
        .into_iter()
        .filter(|instance| {
            instance.case_instance_id == case_instance_id
                && instance.plan_item_id == plan_item_id
                && matches!(
                    instance.plan_item_definition_type.as_str(),
                    "eventlistener" | PLAN_ITEM_DEFINITION_TYPE_TIMER_EVENT_LISTENER
                )
        })
    {
        // Use the persisted definition type so timer listeners notify as
        // `timereventlistener`, matching their Java definition class rather than
        // the generic type (`CmmnListenerNotificationHelper.java:111-115`).
        fire_plan_item_lifecycle_listeners_session(
            session,
            case_instance_id,
            Some(&instance.id),
            &instance.plan_item_definition_id,
            Some(&instance.plan_item_definition_type),
            &instance.state,
            "TERMINATED",
        )?;
    }

    record_plan_item_standard_event_session(
        session,
        case_instance_id,
        plan_item_id,
        CmmnPlanItemOnPart::STANDARD_EVENT_TERMINATE,
    )?;
    // `AbstractMovePlanItemInstanceToTerminalStateOperation.java:74-143`
    // produces TERMINATED history for both generic and timer listeners.
    terminate_plan_item_instance_rows(session, case_instance_id, plan_item_id, "eventlistener")?;
    terminate_plan_item_instance_rows(
        session,
        case_instance_id,
        plan_item_id,
        PLAN_ITEM_DEFINITION_TYPE_TIMER_EVENT_LISTENER,
    )?;
    Ok(vec![(plan_item_id.to_string(), None)])
}

/// Java `TimerEventListenerActivityBehaviour.removeTimerJob` (:214-221): delete every
/// timer job for a plan item (keyed by case instance + plan item).
fn delete_timer_jobs_for_plan_item(
    session: &mut DbSession,
    case_instance_id: &str,
    plan_item_id: &str,
) -> Result<(), CmmnError> {
    let mut params = DbParams::new();
    params.push(case_instance_id);
    params.push(plan_item_id);
    session.execute_raw(RenderedStatement::new(
        "DELETE FROM ACT_CMMN_JOB WHERE FAMILY_ = 'timer' AND SCOPE_ID_ = ? AND SUB_SCOPE_ID_ = ?"
            .to_string(),
        params,
    ))?;
    Ok(())
}

fn delete_event_subscriptions_for_container(
    session: &mut DbSession,
    case_instance_id: &str,
    container: ContainerView<'_>,
) -> Result<(), CmmnError> {
    for plan_item in container.plan_items {
        let mut params = DbParams::new();
        params.push(case_instance_id);
        params.push(plan_item.id.as_str());
        session.execute_raw(RenderedStatement::new(
            "DELETE FROM ACT_CMMN_EVENT_SUBSCRIPTION WHERE CASE_INSTANCE_ID_ = ? AND PLAN_ITEM_INSTANCE_ID_ = ?".to_string(),
            params,
        ))?;
        // Timer event listeners own timer jobs rather than subscriptions; remove them
        // with the container (Java removeTimerJob on TERMINATE/EXIT/DISMISS).
        delete_timer_jobs_for_plan_item(session, case_instance_id, &plan_item.id)?;
    }
    for stage in container.stages {
        delete_event_subscriptions_for_container(
            session,
            case_instance_id,
            ContainerView::from_stage(stage),
        )?;
    }
    Ok(())
}

fn terminate_human_tasks_by_definition_ids(
    session: &mut DbSession,
    case_instance_id: &str,
    task_definition_ids: &[String],
) -> Result<Vec<(String, Option<String>)>, CmmnError> {
    remove_human_tasks_by_definition_ids(
        session,
        case_instance_id,
        task_definition_ids,
        CmmnHumanTaskState::Terminated,
    )
}

fn move_human_tasks_to_available_by_definition_ids(
    session: &mut DbSession,
    case_instance_id: &str,
    task_definition_ids: &[String],
) -> Result<(), CmmnError> {
    remove_human_tasks_by_definition_ids(
        session,
        case_instance_id,
        task_definition_ids,
        CmmnHumanTaskState::Available,
    )?;
    Ok(())
}

fn add_waiting_for_repetition_human_tasks_by_definition_ids(
    session: &mut DbSession,
    case_definition: &CmmnCaseDefinition,
    case_instance: &CmmnCaseInstance,
    task_definition_ids: &[String],
) -> Result<(), CmmnError> {
    for task_definition_id in task_definition_ids {
        let (plan_item, human_task) = find_human_task_plan_item(
            &case_definition.model.case_plan_model,
            task_definition_id.as_str(),
        )
        .ok_or_else(|| {
            CmmnError::not_found(format!(
                "CMMN plan item definition '{task_definition_id}' was not found in case definition '{}'",
                case_definition.id
            ))
        })?;
        if plan_item.repetition_rule.is_none() {
            return Err(CmmnError::execution(format!(
                "CMMN plan item definition '{task_definition_id}' does not define a repetitionRule"
            )));
        }
        if open_human_task_instance_exists(session, &case_instance.id, &plan_item.id)? {
            continue;
        }

        create_human_task_instance(
            session,
            case_definition,
            case_instance,
            plan_item,
            human_task,
            None,
            CmmnHumanTaskState::Available,
        )?;
    }

    Ok(())
}

fn remove_waiting_for_repetition_human_tasks_by_definition_ids(
    session: &mut DbSession,
    case_instance_id: &str,
    task_definition_ids: &[String],
) -> Result<(), CmmnError> {
    for task_definition_id in task_definition_ids {
        let mut params = DbParams::new();
        params.push(case_instance_id);
        let rendered = RenderedStatement::new(
            "SELECT DATA_ FROM ACT_CMMN_HUMAN_TASK WHERE CASE_INSTANCE_ID_ = ? AND STATE_ = 'AVAILABLE'".to_string(),
            params,
        );
        let rows = session.select_raw(rendered)?;
        let tasks = rows
            .into_iter()
            .map(|row| {
                let json = row
                    .get_text("DATA_")
                    .ok_or_else(|| CmmnError::storage("Missing DATA_ in CMMN human task row"))?;
                serde_json::from_str::<CmmnHumanTaskInstance>(&json).map_err(CmmnError::from)
            })
            .collect::<Result<Vec<_>, _>>()?;

        for task in tasks.into_iter().filter(|task| {
            task.task_definition_id == *task_definition_id
                || task.plan_item_id == *task_definition_id
        }) {
            let mut params = DbParams::new();
            params.push(task.id.as_str());
            session.execute_raw(RenderedStatement::new(
                "DELETE FROM ACT_CMMN_HUMAN_TASK WHERE ID_ = ?".to_string(),
                params,
            ))?;
        }
    }

    Ok(())
}

fn remove_human_tasks_by_definition_ids(
    session: &mut DbSession,
    case_instance_id: &str,
    task_definition_ids: &[String],
    target_state: CmmnHumanTaskState,
) -> Result<Vec<(String, Option<String>)>, CmmnError> {
    let mut changed_plan_items = Vec::new();
    for task_definition_id in task_definition_ids {
        let mut params = DbParams::new();
        params.push(case_instance_id);
        let rendered = RenderedStatement::new(
            "SELECT DATA_ FROM ACT_CMMN_HUMAN_TASK WHERE CASE_INSTANCE_ID_ = ? AND STATE_ <> 'COMPLETED'".to_string(),
            params,
        );
        let rows = session.select_raw(rendered)?;
        let tasks = rows
            .into_iter()
            .map(|row| {
                let json = row
                    .get_text("DATA_")
                    .ok_or_else(|| CmmnError::storage("Missing DATA_ in CMMN human task row"))?;
                serde_json::from_str::<CmmnHumanTaskInstance>(&json).map_err(CmmnError::from)
            })
            .collect::<Result<Vec<_>, _>>()?;

        let matching_tasks = tasks
            .into_iter()
            .filter(|task| {
                task.task_definition_id == *task_definition_id
                    || task.plan_item_id == *task_definition_id
            })
            .collect::<Vec<_>>();
        if matching_tasks.is_empty() {
            return Err(CmmnError::not_found(format!(
                "Active CMMN plan item definition '{task_definition_id}' was not found in case instance '{case_instance_id}'"
            )));
        }

        for mut task in matching_tasks {
            task.state = target_state.clone();
            task.completed_at = Some(Utc::now());
            persist_historic_human_task_session(
                session,
                &CmmnHistoricHumanTaskInstance::from(&task),
            )?;
            changed_plan_items.push((task.plan_item_id.clone(), task.stage_instance_id.clone()));
            let mut params = DbParams::new();
            params.push(task.id.as_str());
            session.execute_raw(RenderedStatement::new(
                "DELETE FROM ACT_CMMN_HUMAN_TASK WHERE ID_ = ?".to_string(),
                params,
            ))?;
        }
    }
    Ok(changed_plan_items)
}

fn activate_plan_items_by_definition_ids(
    session: &mut DbSession,
    case_definition: &CmmnCaseDefinition,
    case_instance: &CmmnCaseInstance,
    plan_item_definition_ids: &[String],
) -> Result<(), CmmnError> {
    let activated_enabled_task_definition_ids = activate_enabled_human_tasks_by_definition_ids(
        session,
        case_instance,
        plan_item_definition_ids,
    )?;
    let activated_enabled_stage_definition_ids = activate_enabled_stages_by_definition_ids(
        session,
        case_definition,
        case_instance,
        plan_item_definition_ids,
    )?;
    let activated_discretionary_definition_ids = activate_discretionary_items_by_definition_ids(
        session,
        case_definition,
        case_instance,
        plan_item_definition_ids,
    )?;
    let plan_item_definition_ids = plan_item_definition_ids
        .iter()
        .filter(|task_definition_id| {
            !activated_enabled_task_definition_ids.contains(*task_definition_id)
                && !activated_enabled_stage_definition_ids.contains(*task_definition_id)
                && !activated_discretionary_definition_ids.contains(*task_definition_id)
        })
        .collect::<Vec<_>>();
    if plan_item_definition_ids.is_empty() {
        return Ok(());
    }

    let active_count = count_rows(
        session,
        "SELECT COUNT(*) AS CNT FROM ACT_CMMN_HUMAN_TASK WHERE CASE_INSTANCE_ID_ = ? AND STATE_ <> 'COMPLETED'",
        &[case_instance.id.as_str()],
    )?;
    let active_stage_count = count_rows(
        session,
        "SELECT COUNT(*) AS CNT FROM ACT_CMMN_STAGE_INSTANCE WHERE CASE_INSTANCE_ID_ = ? AND STATE_ = 'ACTIVE'",
        &[case_instance.id.as_str()],
    )?;
    if active_count > 0 || active_stage_count > 0 {
        return Err(CmmnError::conflict(format!(
            "CMMN case instance '{}' already has active plan item instances",
            case_instance.id
        )));
    }

    for plan_item_definition_id in plan_item_definition_ids {
        let target = find_plan_item_definition_activation_target(
            &case_definition.model.case_plan_model,
            plan_item_definition_id.as_str(),
        )
        .ok_or_else(|| {
            CmmnError::not_found(format!(
                "CMMN plan item definition '{plan_item_definition_id}' was not found in case definition '{}'",
                case_definition.id
            ))
        })?;
        match target {
            PlanItemDefinitionActivationTarget::HumanTask(plan_item, human_task) => {
                create_human_task_instance(
                    session,
                    case_definition,
                    case_instance,
                    plan_item,
                    human_task,
                    None,
                    CmmnHumanTaskState::Active,
                )?;
            }
            PlanItemDefinitionActivationTarget::Stage(plan_item, stage) => {
                create_stage_instance(
                    session,
                    case_definition,
                    case_instance,
                    plan_item,
                    stage,
                    None,
                    CmmnStageInstanceState::Active,
                )?;
            }
            PlanItemDefinitionActivationTarget::DecisionTask(plan_item, decision_task) => {
                delete_enabled_plan_item_instance_rows(session, &case_instance.id, &plan_item.id)?;
                complete_decision_task(
                    session,
                    case_definition,
                    case_instance,
                    plan_item,
                    decision_task,
                    None,
                )?;
            }
            PlanItemDefinitionActivationTarget::Milestone(plan_item, milestone) => {
                reach_milestone(
                    session,
                    case_definition,
                    case_instance,
                    plan_item,
                    milestone,
                    None,
                )?;
            }
            PlanItemDefinitionActivationTarget::EventListener(plan_item, event_listener) => {
                delete_enabled_plan_item_instance_rows(session, &case_instance.id, &plan_item.id)?;
                let parent_stage_instance_id = resolve_parent_stage_instance_id_for_plan_item(
                    session,
                    case_definition,
                    &case_instance.id,
                    &plan_item.id,
                )?;
                activate_event_listener(
                    session,
                    case_definition,
                    case_instance,
                    plan_item,
                    event_listener,
                    parent_stage_instance_id.as_deref(),
                )?;
            }
        }
    }
    Ok(())
}

fn activate_enabled_human_tasks_by_definition_ids(
    session: &mut DbSession,
    case_instance: &CmmnCaseInstance,
    task_definition_ids: &[String],
) -> Result<Vec<String>, CmmnError> {
    let mut activated_task_definition_ids = Vec::new();
    for task_definition_id in task_definition_ids {
        let mut params = DbParams::new();
        params.push(case_instance.id.as_str());
        let rendered = RenderedStatement::new(
            // Java StartPlanItemInstanceCmd.java:54-58 only starts ENABLED.
            "SELECT DATA_ FROM ACT_CMMN_HUMAN_TASK WHERE CASE_INSTANCE_ID_ = ? AND STATE_ = 'ENABLED'".to_string(),
            params,
        );
        let rows = session.select_raw(rendered)?;
        let tasks = rows
            .into_iter()
            .map(|row| {
                let json = row
                    .get_text("DATA_")
                    .ok_or_else(|| CmmnError::storage("Missing DATA_ in CMMN human task row"))?;
                serde_json::from_str::<CmmnHumanTaskInstance>(&json).map_err(CmmnError::from)
            })
            .collect::<Result<Vec<_>, _>>()?;

        let mut activated_any = false;
        for mut task in tasks.into_iter().filter(|task| {
            task.task_definition_id == *task_definition_id
                || task.plan_item_id == *task_definition_id
        }) {
            task.state = CmmnHumanTaskState::Active;
            persist_human_task_session(session, &task)?;
            persist_historic_human_task_session(
                session,
                &CmmnHistoricHumanTaskInstance::from(&task),
            )?;
            activated_any = true;
        }
        if activated_any {
            activated_task_definition_ids.push(task_definition_id.clone());
        }
    }
    Ok(activated_task_definition_ids)
}

fn activate_enabled_stages_by_definition_ids(
    session: &mut DbSession,
    case_definition: &CmmnCaseDefinition,
    case_instance: &CmmnCaseInstance,
    stage_definition_ids: &[String],
) -> Result<Vec<String>, CmmnError> {
    let mut activated_stage_definition_ids = Vec::new();
    for stage_definition_id in stage_definition_ids {
        let mut params = DbParams::new();
        params.push(case_instance.id.as_str());
        let rendered = RenderedStatement::new(
            // Java manual activation parks stages in ENABLED too
            // (ActivatePlanItemInstanceOperation.java:48-55).
            "SELECT DATA_ FROM ACT_CMMN_STAGE_INSTANCE WHERE CASE_INSTANCE_ID_ = ? AND STATE_ = 'ENABLED'".to_string(),
            params,
        );
        let rows = session.select_raw(rendered)?;
        let stage_instances = rows
            .into_iter()
            .map(|row| {
                let json = row.get_text("DATA_").ok_or_else(|| {
                    CmmnError::storage("Missing DATA_ in CMMN stage instance row")
                })?;
                serde_json::from_str::<CmmnStageInstance>(&json).map_err(CmmnError::from)
            })
            .collect::<Result<Vec<_>, _>>()?;

        let mut activated_any = false;
        for stage_instance in stage_instances.into_iter().filter(|stage_instance| {
            stage_instance.stage_definition_id == *stage_definition_id
                || stage_instance.plan_item_id == *stage_definition_id
        }) {
            let stage = find_stage_by_definition_id(
                &case_definition.model.case_plan_model.stages,
                &stage_instance.stage_definition_id,
            )
            .ok_or_else(|| {
                CmmnError::storage(format!(
                    "CMMN stage definition '{}' was not found in case definition '{}'",
                    stage_instance.stage_definition_id, case_definition.id
                ))
            })?;
            start_stage_instance(
                session,
                case_definition,
                case_instance,
                stage_instance,
                stage,
            )?;
            activated_any = true;
        }
        if activated_any {
            activated_stage_definition_ids.push(stage_definition_id.clone());
        }
    }
    Ok(activated_stage_definition_ids)
}

fn activate_discretionary_items_by_definition_ids(
    session: &mut DbSession,
    case_definition: &CmmnCaseDefinition,
    case_instance: &CmmnCaseInstance,
    plan_item_definition_ids: &[String],
) -> Result<Vec<String>, CmmnError> {
    let mut params = DbParams::new();
    params.push(case_instance.id.as_str());
    let rendered = RenderedStatement::new(
        "SELECT DATA_ FROM ACT_CMMN_STAGE_INSTANCE WHERE CASE_INSTANCE_ID_ = ? AND STATE_ = 'ACTIVE'".to_string(),
        params,
    );
    let rows = session.select_raw(rendered)?;
    let active_stage_instances = rows
        .into_iter()
        .map(|row| {
            let json = row
                .get_text("DATA_")
                .ok_or_else(|| CmmnError::storage("Missing DATA_ in CMMN stage instance row"))?;
            serde_json::from_str::<CmmnStageInstance>(&json).map_err(CmmnError::from)
        })
        .collect::<Result<Vec<_>, _>>()?;

    let mut activated_definition_ids = Vec::new();
    for definition_id in plan_item_definition_ids {
        let mut activated_any = false;
        if let Some((discretionary_item, human_task)) = find_discretionary_human_task(
            case_definition
                .model
                .case_plan_model
                .planning_tables
                .as_slice(),
            case_definition.model.case_plan_model.human_tasks.as_slice(),
            definition_id,
        ) && !open_human_task_instance_exists(
            session,
            &case_instance.id,
            &discretionary_item.id,
        )? {
            let plan_item = discretionary_item_plan_item(discretionary_item);
            create_human_task_instance(
                session,
                case_definition,
                case_instance,
                &plan_item,
                human_task,
                None,
                CmmnHumanTaskState::Active,
            )?;
            activated_any = true;
        }

        for stage_instance in &active_stage_instances {
            let stage = find_stage_by_definition_id(
                &case_definition.model.case_plan_model.stages,
                &stage_instance.stage_definition_id,
            )
            .ok_or_else(|| {
                CmmnError::storage(format!(
                    "CMMN stage definition '{}' was not found in case definition '{}'",
                    stage_instance.stage_definition_id, case_definition.id
                ))
            })?;
            let Some((discretionary_item, human_task)) =
                find_discretionary_human_task_in_stage(stage, definition_id)
            else {
                continue;
            };
            if open_human_task_instance_exists_in_stage(
                session,
                &case_instance.id,
                &stage_instance.id,
                &discretionary_item.id,
            )? {
                continue;
            }

            let plan_item = discretionary_item_plan_item(discretionary_item);
            create_human_task_instance(
                session,
                case_definition,
                case_instance,
                &plan_item,
                human_task,
                Some(stage_instance.id.as_str()),
                CmmnHumanTaskState::Active,
            )?;
            activated_any = true;
        }
        if activated_any {
            activated_definition_ids.push(definition_id.clone());
        }
    }

    Ok(activated_definition_ids)
}

fn discretionary_item_plan_item(discretionary_item: &CmmnDiscretionaryItem) -> CmmnPlanItem {
    CmmnPlanItem::new(
        discretionary_item.id.clone(),
        discretionary_item.definition_ref.clone(),
    )
    .with_name(discretionary_item.name.clone())
}

fn find_discretionary_human_task_in_stage<'a>(
    stage: &'a CmmnStage,
    definition_id: &str,
) -> Option<(&'a CmmnDiscretionaryItem, &'a CmmnHumanTask)> {
    find_discretionary_human_task(
        stage.planning_tables.as_slice(),
        stage.human_tasks.as_slice(),
        definition_id,
    )
}

fn find_discretionary_human_task_in_case_model<'a>(
    case_plan_model: &'a CmmnCasePlanModel,
    definition_id: &str,
) -> Option<(&'a CmmnDiscretionaryItem, &'a CmmnHumanTask)> {
    find_discretionary_human_task(
        case_plan_model.planning_tables.as_slice(),
        case_plan_model.human_tasks.as_slice(),
        definition_id,
    )
    .or_else(|| find_discretionary_human_task_in_stages(&case_plan_model.stages, definition_id))
}

fn find_discretionary_human_task_in_stages<'a>(
    stages: &'a [CmmnStage],
    definition_id: &str,
) -> Option<(&'a CmmnDiscretionaryItem, &'a CmmnHumanTask)> {
    for stage in stages {
        if let Some(found) = find_discretionary_human_task_in_stage(stage, definition_id) {
            return Some(found);
        }
        if let Some(found) = find_discretionary_human_task_in_stages(&stage.stages, definition_id) {
            return Some(found);
        }
    }
    None
}

fn find_discretionary_human_task<'a>(
    planning_tables: &'a [CmmnPlanningTable],
    human_tasks: &'a [CmmnHumanTask],
    definition_id: &str,
) -> Option<(&'a CmmnDiscretionaryItem, &'a CmmnHumanTask)> {
    for planning_table in planning_tables {
        for discretionary_item in &planning_table.discretionary_items {
            if discretionary_item.definition_ref != definition_id
                && discretionary_item.id != definition_id
            {
                continue;
            }
            if let Some(human_task) = human_tasks
                .iter()
                .find(|human_task| human_task.id == discretionary_item.definition_ref)
            {
                return Some((discretionary_item, human_task));
            }
        }
    }

    None
}

fn find_human_task_plan_item<'a>(
    case_plan_model: &'a CmmnCasePlanModel,
    task_definition_id: &str,
) -> Option<(&'a CmmnPlanItem, &'a CmmnHumanTask)> {
    find_human_task_plan_item_in_container(
        case_plan_model.plan_items.as_slice(),
        case_plan_model.stages.as_slice(),
        case_plan_model.human_tasks.as_slice(),
        task_definition_id,
    )
}

fn find_human_task_plan_item_by_plan_item_id<'a>(
    case_plan_model: &'a CmmnCasePlanModel,
    plan_item_id: &str,
) -> Option<(&'a CmmnPlanItem, &'a CmmnHumanTask)> {
    find_human_task_plan_item_by_plan_item_id_in_container(
        case_plan_model.plan_items.as_slice(),
        case_plan_model.stages.as_slice(),
        case_plan_model.human_tasks.as_slice(),
        plan_item_id,
    )
}

fn find_human_task_plan_item_by_plan_item_id_in_container<'a>(
    plan_items: &'a [CmmnPlanItem],
    stages: &'a [CmmnStage],
    human_tasks: &'a [CmmnHumanTask],
    plan_item_id: &str,
) -> Option<(&'a CmmnPlanItem, &'a CmmnHumanTask)> {
    for plan_item in plan_items {
        if plan_item.id == plan_item_id
            && let Some(human_task) = human_tasks
                .iter()
                .find(|human_task| human_task.id == plan_item.definition_ref)
        {
            return Some((plan_item, human_task));
        }
    }

    for stage in stages {
        if let Some(found) = find_human_task_plan_item_by_plan_item_id_in_container(
            stage.plan_items.as_slice(),
            stage.stages.as_slice(),
            stage.human_tasks.as_slice(),
            plan_item_id,
        ) {
            return Some(found);
        }
    }

    None
}

fn find_stage_plan_item_by_plan_item_id<'a>(
    case_plan_model: &'a CmmnCasePlanModel,
    plan_item_id: &str,
) -> Option<(&'a CmmnPlanItem, &'a CmmnStage)> {
    find_stage_plan_item_by_plan_item_id_in_container(
        case_plan_model.plan_items.as_slice(),
        case_plan_model.stages.as_slice(),
        plan_item_id,
    )
}

fn find_stage_plan_item_by_plan_item_id_in_container<'a>(
    plan_items: &'a [CmmnPlanItem],
    stages: &'a [CmmnStage],
    plan_item_id: &str,
) -> Option<(&'a CmmnPlanItem, &'a CmmnStage)> {
    for plan_item in plan_items {
        if plan_item.id == plan_item_id
            && let Some(stage) = stages
                .iter()
                .find(|stage| stage.id == plan_item.definition_ref)
        {
            return Some((plan_item, stage));
        }
    }

    for stage in stages {
        if let Some(found) = find_stage_plan_item_by_plan_item_id_in_container(
            stage.plan_items.as_slice(),
            stage.stages.as_slice(),
            plan_item_id,
        ) {
            return Some(found);
        }
    }

    None
}

fn find_human_task_plan_item_in_container<'a>(
    plan_items: &'a [CmmnPlanItem],
    stages: &'a [CmmnStage],
    human_tasks: &'a [CmmnHumanTask],
    task_definition_id: &str,
) -> Option<(&'a CmmnPlanItem, &'a CmmnHumanTask)> {
    for plan_item in plan_items {
        if plan_item.definition_ref == task_definition_id
            && let Some(human_task) = human_tasks
                .iter()
                .find(|human_task| human_task.id == task_definition_id)
        {
            return Some((plan_item, human_task));
        }
    }

    for stage in stages {
        if let Some(found) = find_human_task_plan_item_in_container(
            stage.plan_items.as_slice(),
            stage.stages.as_slice(),
            stage.human_tasks.as_slice(),
            task_definition_id,
        ) {
            return Some(found);
        }
    }

    None
}

fn persist_case_instance_session(
    session: &mut DbSession,
    case_instance: &CmmnCaseInstance,
) -> Result<(), CmmnError> {
    let mut entity = CmmnCaseInstanceEntity::new(
        case_instance.id.clone(),
        case_instance.case_definition_id.clone(),
        case_instance.case_definition_key.clone(),
        case_instance.state.as_str().to_string(),
        case_instance.started_at.to_rfc3339(),
        serde_json::to_string(case_instance)?,
    );
    entity.set_tenant_id(case_instance.tenant_id.clone());
    entity.set_business_key(case_instance.business_key.clone());
    CmmnCaseInstanceDataManager::new().insert(session, entity)?;
    Ok(())
}

fn persist_stage_instance_session(
    session: &mut DbSession,
    stage_instance: &CmmnStageInstance,
) -> Result<(), CmmnError> {
    let mut entity = CmmnStageInstanceEntity::new(
        stage_instance.id.clone(),
        stage_instance.case_instance_id.clone(),
        stage_instance.stage_definition_id.clone(),
        stage_instance.state.as_str().to_string(),
        stage_instance.activated_at.to_rfc3339(),
        serde_json::to_string(stage_instance)?,
    );
    entity.set_parent_stage_instance_id(stage_instance.parent_stage_instance_id.clone());
    CmmnStageInstanceDataManager::new().insert(session, entity)?;
    persist_historic_stage_instance_session(session, stage_instance)?;

    // P116: mirror the stage into the unified plan-item-instance table. Java has no
    // separate stage table — a stage IS a PlanItemInstanceEntity (isStage=true,
    // PlanItemInstanceEntityManagerImpl.java:94-99), so the plan item instance id is
    // the stage instance id.
    let plan_item_instance = CmmnPlanItemInstance {
        id: stage_instance.id.clone(),
        case_instance_id: stage_instance.case_instance_id.clone(),
        case_definition_id: stage_instance.case_definition_id.clone(),
        stage_instance_id: None,
        plan_item_id: stage_instance.plan_item_id.clone(),
        plan_item_definition_id: stage_instance.stage_definition_id.clone(),
        plan_item_definition_type: "stage".to_string(),
        name: stage_instance.name.clone(),
        state: stage_instance.state.as_str().to_string(),
        created_at: stage_instance.activated_at,
        last_enabled_at: (stage_instance.state == CmmnStageInstanceState::Enabled)
            .then_some(stage_instance.activated_at),
        ended_at: stage_instance.ended_at,
        occurred_at: None,
        assignee: None,
        tenant_id: None,
    };
    persist_plan_item_instance_session(session, &plan_item_instance)?;
    Ok(())
}

fn persist_historic_stage_instance_session(
    session: &mut DbSession,
    stage_instance: &CmmnStageInstance,
) -> Result<(), CmmnError> {
    let mut entity = CmmnStageHistoryEntity::new(
        stage_instance.id.clone(),
        stage_instance.case_instance_id.clone(),
        stage_instance.case_definition_id.clone(),
        stage_instance.stage_definition_id.clone(),
        stage_instance.state.as_str().to_string(),
        stage_instance.activated_at.to_rfc3339(),
        serde_json::to_string(stage_instance)?,
    );
    entity.set_parent_stage_instance_id(stage_instance.parent_stage_instance_id.clone());
    entity.set_ended_at(stage_instance.ended_at.map(|value| value.to_rfc3339()));
    CmmnStageHistoryDataManager::new().insert(session, entity)?;
    Ok(())
}

fn persist_human_task_session(
    session: &mut DbSession,
    human_task: &CmmnHumanTaskInstance,
) -> Result<(), CmmnError> {
    let mut entity = CmmnHumanTaskEntity::new(
        human_task.id.clone(),
        human_task.case_instance_id.clone(),
        human_task.case_definition_id.clone(),
        human_task.case_definition_key.clone(),
        human_task.state.as_str().to_string(),
        human_task.activated_at.to_rfc3339(),
        serde_json::to_string(human_task)?,
    );
    entity.set_stage_instance_id(human_task.stage_instance_id.clone());
    CmmnHumanTaskDataManager::new().insert(session, entity)?;
    Ok(())
}

// P116: unified plan-item-instance mirror. Java keeps every plan item instance in
// one runtime table (ACT_CMMN_RU_PLAN_ITEM_INST); the Rust engine keeps type-specific
// source tables (stage / human task) and mirrors stage / milestone / event listener
// instances here so the unified query surface reads one table. Human-task rows are
// NOT mirrored — `CmmnHumanTaskQuery` stays backed by ACT_CMMN_HUMAN_TASK.
fn persist_plan_item_instance_session(
    session: &mut DbSession,
    instance: &CmmnPlanItemInstance,
) -> Result<(), CmmnError> {
    let mut entity = CmmnPlanItemInstanceEntity::new(
        instance.id.clone(),
        instance.case_definition_id.clone(),
        instance.case_instance_id.clone(),
        instance.plan_item_id.clone(),
        instance.plan_item_definition_id.clone(),
        instance.plan_item_definition_type.clone(),
        instance.name.clone(),
        instance.state.clone(),
        instance.created_at.to_rfc3339(),
        serde_json::to_string(instance)?,
    );
    entity.set_stage_instance_id(instance.stage_instance_id.clone());
    entity.set_ended_time(instance.ended_at.map(|value| value.to_rfc3339()));
    entity.set_occurred_time(instance.occurred_at.map(|value| value.to_rfc3339()));
    entity.set_assignee(instance.assignee.clone());
    entity.set_tenant_id(instance.tenant_id.clone());
    CmmnPlanItemInstanceDataManager::new().insert(session, entity)?;
    Ok(())
}

/// Load all plan-item-instance mirror rows, including lightweight historic rows.
fn load_plan_item_instances_session(
    session: &mut DbSession,
) -> Result<Vec<CmmnPlanItemInstance>, CmmnError> {
    let rows = session.select_list(
        flowable_persistence::statement::StatementId::SelectAllCmmnPlanItemInstances,
        DbParams::new(),
    )?;
    rows.iter()
        .map(|row| {
            let json = row.get_text("DATA_").ok_or_else(|| {
                CmmnError::storage("Missing DATA_ in CMMN plan item instance row")
            })?;
            serde_json::from_str::<CmmnPlanItemInstance>(&json).map_err(CmmnError::from)
        })
        .collect()
}

/// Delete superseded transient mirror rows during activation/replacement. Terminal
/// transitions use `terminate_plan_item_instance_rows` so the state move from
/// `AbstractMovePlanItemInstanceToTerminalStateOperation.java:74-143` remains historic.
fn delete_plan_item_instance_rows(
    session: &mut DbSession,
    case_instance_id: &str,
    element_id: &str,
    plan_item_definition_type: &str,
) -> Result<(), CmmnError> {
    let instances = load_plan_item_instances_session(session)?;
    let data_manager = CmmnPlanItemInstanceDataManager::new();
    for instance in instances {
        if instance.case_instance_id == case_instance_id
            && instance.plan_item_id == element_id
            && instance.plan_item_definition_type == plan_item_definition_type
        {
            data_manager.delete(session, &plan_item_instance_entity_from_model(instance))?;
        }
    }
    Ok(())
}

fn terminate_plan_item_instance_rows(
    session: &mut DbSession,
    case_instance_id: &str,
    element_id: &str,
    plan_item_definition_type: &str,
) -> Result<(), CmmnError> {
    let ended_at = Utc::now();
    for mut instance in load_plan_item_instances_session(session)? {
        if instance.case_instance_id == case_instance_id
            && instance.plan_item_id == element_id
            && instance.plan_item_definition_type == plan_item_definition_type
        {
            // Java removes the runtime row but preserves TERMINATED + endedTime in
            // ACT_CMMN_HI_PLAN_ITEM_INST. This lightweight model retains that
            // historic representation in the unified mirror.
            instance.state = "TERMINATED".to_string();
            instance.ended_at = Some(ended_at);
            persist_plan_item_instance_session(session, &instance)?;
        }
    }
    Ok(())
}

fn terminate_open_plan_item_instances_for_case_session(
    session: &mut DbSession,
    case_instance_id: &str,
    ended_at: DateTime<Utc>,
) -> Result<(), CmmnError> {
    for mut instance in load_plan_item_instances_session(session)? {
        if instance.case_instance_id == case_instance_id && instance.ended_at.is_none() {
            instance.state = "TERMINATED".to_string();
            instance.ended_at = Some(ended_at);
            persist_plan_item_instance_session(session, &instance)?;
        }
    }
    Ok(())
}

fn plan_item_instance_entity_from_model(
    instance: CmmnPlanItemInstance,
) -> CmmnPlanItemInstanceEntity {
    CmmnPlanItemInstanceEntity {
        id: instance.id,
        case_definition_id: instance.case_definition_id,
        case_instance_id: instance.case_instance_id,
        stage_instance_id: instance.stage_instance_id,
        element_id: instance.plan_item_id,
        item_definition_id: instance.plan_item_definition_id,
        item_definition_type: instance.plan_item_definition_type,
        name: instance.name,
        state: instance.state,
        create_time: instance.created_at.to_rfc3339(),
        ended_time: instance.ended_at.map(|value| value.to_rfc3339()),
        occurred_time: instance.occurred_at.map(|value| value.to_rfc3339()),
        assignee: instance.assignee,
        tenant_id: instance.tenant_id,
        data: String::new(),
    }
}

/// Mark the mirror row(s) for an occurred event listener COMPLETED. Java sets
/// occurredTime + endedTime on occur (OccurPlanItemInstanceOperation.java:61-62).
fn complete_plan_item_instance_rows(
    session: &mut DbSession,
    case_instance_id: &str,
    element_id: &str,
    plan_item_definition_type: &str,
) -> Result<(), CmmnError> {
    let instances = load_plan_item_instances_session(session)?;
    for mut instance in instances {
        if instance.case_instance_id == case_instance_id
            && instance.plan_item_id == element_id
            && instance.plan_item_definition_type == plan_item_definition_type
        {
            let occurred_at = Utc::now();
            // P126: occur is a plan item state transition too — Java notifies from
            // OccurPlanItemInstanceOperation's base class
            // (AbstractMovePlanItemInstanceToTerminalStateOperation.java:124).
            fire_plan_item_lifecycle_listeners_session(
                session,
                case_instance_id,
                Some(&instance.id),
                &instance.plan_item_definition_id,
                Some(plan_item_definition_type),
                &instance.state,
                "COMPLETED",
            )?;
            instance.state = "COMPLETED".to_string();
            instance.ended_at = Some(occurred_at);
            instance.occurred_at = Some(occurred_at);
            persist_plan_item_instance_session(session, &instance)?;
        }
    }
    Ok(())
}

fn persist_task_association_session(
    session: &mut DbSession,
    association: &CmmnTaskInstanceAssociation,
) -> Result<(), CmmnError> {
    let mut entity = CmmnTaskInstanceAssociationEntity::new(
        association.id.clone(),
        association.kind.as_str().to_string(),
        association.state.as_str().to_string(),
        association.case_instance_id.clone(),
        association.case_definition_id.clone(),
        association.case_definition_key.clone(),
        association.plan_item_id.clone(),
        association.task_definition_id.clone(),
        association.child_definition_key.clone(),
        association.child_instance_id.clone().unwrap_or_default(),
        association.created_at.timestamp_millis(),
        serde_json::to_string(association)?,
    );
    entity.set_stage_instance_id(association.stage_instance_id.clone());
    entity.set_completed_at(association.completed_at.map(|value| value.to_rfc3339()));
    entity.set_failure_message(association.failure_message.clone());
    CmmnTaskInstanceAssociationDataManager::new().insert(session, entity)?;
    Ok(())
}

fn task_association_exists(
    session: &mut DbSession,
    case_instance_id: &str,
    plan_item_id: &str,
    stage_instance_id: Option<&str>,
) -> Result<bool, CmmnError> {
    let mut params = DbParams::new();
    params.push(case_instance_id);
    params.push(plan_item_id);
    params.push(stage_instance_id);
    params.push(stage_instance_id);
    let rendered = RenderedStatement::new(
        "SELECT COUNT(*) AS CNT FROM ACT_CMMN_TASK_INSTANCE_ASSOCIATION WHERE CASE_INSTANCE_ID_ = ? AND PLAN_ITEM_ID_ = ? AND ((STAGE_INSTANCE_ID_ IS NULL AND ? IS NULL) OR STAGE_INSTANCE_ID_ = ?)".to_string(),
        params,
    );
    let row = session
        .select_one_raw(rendered)?
        .ok_or_else(|| CmmnError::storage("Missing COUNT row"))?;
    Ok(row.get_integer("CNT").unwrap_or(0) > 0)
}

fn load_active_task_association_by_child_instance_session(
    session: &mut DbSession,
    kind: &CmmnTaskAssociationKind,
    child_instance_id: &str,
) -> Result<Option<CmmnTaskInstanceAssociation>, CmmnError> {
    let mut params = DbParams::new();
    params.push(kind.as_str());
    params.push(child_instance_id);
    params.push(CmmnTaskAssociationState::Active.as_str());
    let rendered = RenderedStatement::new(
        "SELECT DATA_ FROM ACT_CMMN_TASK_INSTANCE_ASSOCIATION WHERE KIND_ = ? AND CHILD_INSTANCE_ID_ = ? AND STATE_ = ? ORDER BY CREATED_AT_ ASC, ID_ ASC LIMIT 1".to_string(),
        params,
    );
    let row = session.select_one_raw(rendered)?;
    row.map(|row| {
        let json = row.get_text("DATA_").ok_or_else(|| {
            CmmnError::storage("Missing DATA_ in CMMN task instance association row")
        })?;
        serde_json::from_str::<CmmnTaskInstanceAssociation>(&json).map_err(CmmnError::from)
    })
    .transpose()
}

fn load_active_case_task_associations_by_child_case_session(
    session: &mut DbSession,
    child_case_instance_id: &str,
) -> Result<Vec<CmmnTaskInstanceAssociation>, CmmnError> {
    let mut params = DbParams::new();
    params.push(CmmnTaskAssociationKind::CaseTask.as_str());
    params.push(child_case_instance_id);
    params.push(CmmnTaskAssociationState::Active.as_str());
    let rendered = RenderedStatement::new(
        "SELECT DATA_ FROM ACT_CMMN_TASK_INSTANCE_ASSOCIATION WHERE KIND_ = ? AND CHILD_INSTANCE_ID_ = ? AND STATE_ = ? ORDER BY CREATED_AT_ ASC, ID_ ASC".to_string(),
        params,
    );
    let rows = session.select_raw(rendered)?;
    rows.into_iter()
        .map(|row| {
            let json = row.get_text("DATA_").ok_or_else(|| {
                CmmnError::storage("Missing DATA_ in CMMN task instance association row")
            })?;
            serde_json::from_str::<CmmnTaskInstanceAssociation>(&json).map_err(CmmnError::from)
        })
        .collect()
}

fn persist_historic_case_session(
    session: &mut DbSession,
    historic_case: &CmmnHistoricCaseInstance,
) -> Result<(), CmmnError> {
    let mut params = DbParams::new();
    params.push(historic_case.case_instance_id.as_str());
    params.push(historic_case.case_definition_id.as_str());
    params.push(historic_case.case_definition_key.as_str());
    params.push(historic_case.tenant_id.clone());
    params.push(historic_case.business_key.clone());
    params.push(historic_case.state.as_str());
    params.push(historic_case.started_at.to_rfc3339());
    params.push(historic_case.completed_at.map(|value| value.to_rfc3339()));
    params.push(serde_json::to_string(historic_case)?);
    session.upsert_raw(
        "ACT_CMMN_CASE_HISTORY",
        "CASE_INSTANCE_ID_",
        &[
            "CASE_INSTANCE_ID_",
            "CASE_DEFINITION_ID_",
            "CASE_KEY_",
            "TENANT_ID_",
            "BUSINESS_KEY_",
            "STATE_",
            "STARTED_AT_",
            "COMPLETED_AT_",
            "DATA_",
        ],
        params,
    )?;
    Ok(())
}

fn persist_historic_human_task_session(
    session: &mut DbSession,
    historic_task: &CmmnHistoricHumanTaskInstance,
) -> Result<(), CmmnError> {
    let mut params = DbParams::new();
    params.push(historic_task.task_id.as_str());
    params.push(historic_task.case_instance_id.as_str());
    params.push(historic_task.case_definition_id.as_str());
    params.push(historic_task.case_definition_key.as_str());
    params.push(historic_task.stage_instance_id.clone());
    params.push(historic_task.state.as_str());
    params.push(historic_task.activated_at.to_rfc3339());
    params.push(historic_task.completed_at.map(|value| value.to_rfc3339()));
    params.push(serde_json::to_string(historic_task)?);
    session.upsert_raw(
        "ACT_CMMN_HUMAN_TASK_HISTORY",
        "TASK_ID_",
        &[
            "TASK_ID_",
            "CASE_INSTANCE_ID_",
            "CASE_DEFINITION_ID_",
            "CASE_KEY_",
            "STAGE_INSTANCE_ID_",
            "STATE_",
            "ACTIVATED_AT_",
            "COMPLETED_AT_",
            "DATA_",
        ],
        params,
    )?;
    Ok(())
}

fn load_historic_human_task_session(
    session: &mut DbSession,
    task_id: &str,
) -> Result<Option<CmmnHistoricHumanTaskInstance>, CmmnError> {
    let mut params = DbParams::new();
    params.push(task_id);
    let rendered = RenderedStatement::new(
        "SELECT DATA_ FROM ACT_CMMN_HUMAN_TASK_HISTORY WHERE TASK_ID_ = ?".to_string(),
        params,
    );
    let row = session.select_one_raw(rendered)?;
    row.map(|row| {
        let json = row
            .get_text("DATA_")
            .ok_or_else(|| CmmnError::storage("Missing DATA_ in CMMN historic human task row"))?;
        serde_json::from_str::<CmmnHistoricHumanTaskInstance>(&json).map_err(CmmnError::from)
    })
    .transpose()
}

fn find_human_task_by_definition_id<'a>(
    human_tasks: &'a [CmmnHumanTask],
    definition_id: &str,
) -> Option<&'a CmmnHumanTask> {
    human_tasks.iter().find(|t| t.id == definition_id)
}

fn find_plan_item_by_id<'a>(
    plan_items: &'a [CmmnPlanItem],
    plan_item_id: &str,
) -> Option<&'a CmmnPlanItem> {
    plan_items.iter().find(|p| p.id == plan_item_id)
}

fn persist_historic_milestone_session(
    session: &mut DbSession,
    historic_milestone: &CmmnHistoricMilestoneInstance,
) -> Result<(), CmmnError> {
    let mut params = DbParams::new();
    params.push(historic_milestone.id.as_str());
    params.push(historic_milestone.case_instance_id.as_str());
    params.push(historic_milestone.case_definition_id.as_str());
    params.push(historic_milestone.case_definition_key.as_str());
    params.push(historic_milestone.milestone_id.as_str());
    params.push(historic_milestone.time.to_rfc3339());
    params.push(serde_json::to_string(historic_milestone)?);
    session.upsert_raw(
        "ACT_CMMN_MILESTONE_HISTORY",
        "ID_",
        &[
            "ID_",
            "CASE_INSTANCE_ID_",
            "CASE_DEFINITION_ID_",
            "CASE_KEY_",
            "MILESTONE_ID_",
            "TIME_",
            "DATA_",
        ],
        params,
    )?;
    Ok(())
}

fn persist_event_subscription_session(
    session: &mut DbSession,
    subscription: &CmmnEventSubscription,
) -> Result<(), CmmnError> {
    let mut params = DbParams::new();
    params.push(subscription.id.as_str());
    params.push(subscription.event_type.as_str());
    params.push(subscription.event_name.clone());
    params.push(subscription.activity_id.clone());
    params.push(subscription.case_instance_id.clone());
    params.push(subscription.case_definition_id.clone());
    params.push(subscription.plan_item_instance_id.clone());
    params.push(subscription.tenant_id.clone());
    params.push(subscription.configuration.clone());
    params.push(subscription.created_at.to_rfc3339());
    params.push(serde_json::to_string(subscription)?);
    session.upsert_raw(
        "ACT_CMMN_EVENT_SUBSCRIPTION",
        "ID_",
        &[
            "ID_",
            "EVENT_TYPE_",
            "EVENT_NAME_",
            "ACTIVITY_ID_",
            "CASE_INSTANCE_ID_",
            "CASE_DEFINITION_ID_",
            "PLAN_ITEM_INSTANCE_ID_",
            "TENANT_ID_",
            "CONFIGURATION_",
            "CREATED_AT_",
            "DATA_",
        ],
        params,
    )?;
    Ok(())
}

fn record_plan_item_standard_event_session(
    session: &mut DbSession,
    case_instance_id: &str,
    plan_item_id: &str,
    standard_event: &str,
) -> Result<(), CmmnError> {
    let mut params = DbParams::new();
    params.push(format!("cmmn-plan-item-event:{}", Uuid::new_v4()));
    params.push(case_instance_id);
    params.push(plan_item_id);
    params.push(standard_event);
    params.push(Utc::now().to_rfc3339());
    session.execute_raw(RenderedStatement::new(
        "INSERT INTO ACT_CMMN_PLAN_ITEM_EVENT (ID_, CASE_INSTANCE_ID_, PLAN_ITEM_ID_, STANDARD_EVENT_, OCCURRED_AT_) VALUES (?, ?, ?, ?, ?)".to_string(),
        params,
    ))?;
    Ok(())
}

fn complete_task_association_session(
    session: &mut DbSession,
    association: &mut CmmnTaskInstanceAssociation,
    target_state: CmmnTaskAssociationState,
    failure_message: Option<String>,
    child_variables: Option<&Map<String, Value>>,
) -> Result<(), CmmnError> {
    if association.state != CmmnTaskAssociationState::Active {
        return Ok(());
    }
    let standard_event = match target_state {
        CmmnTaskAssociationState::Completed => CmmnPlanItemOnPart::STANDARD_EVENT_COMPLETE,
        CmmnTaskAssociationState::Failed => CmmnPlanItemOnPart::STANDARD_EVENT_TERMINATE,
        CmmnTaskAssociationState::Active => return Ok(()),
    };

    association.state = target_state;
    association.completed_at = Some(Utc::now());
    association.failure_message = failure_message;
    persist_task_association_session(session, association)?;

    let case_definition = load_case_definition_session(session, &association.case_definition_id)?
        .ok_or_else(|| {
        CmmnError::storage(format!(
            "CMMN case definition '{}' disappeared during task association completion",
            association.case_definition_id
        ))
    })?;
    // Java parity: CaseTaskActivityBehavior.java:177 / ProcessTaskActivityBehavior.java:156 —
    // out-parameters are applied on child completion before the parent plan item completes;
    // a terminated child skips them (CaseTaskActivityBehavior.java:195-211 only maps on EXIT).
    if association.state == CmmnTaskAssociationState::Completed {
        apply_child_task_out_parameters(session, &case_definition, association, child_variables)?;
    }
    complete_task_plan_item_event_session(session, &case_definition, association, standard_event)?;
    maybe_complete_case(session, &association.case_instance_id)
}

// Java parity: IOParameterUtil.java:56-92 applied with the child instance as source and the
// parent case as target (CaseTaskActivityBehavior.java:244-253).
fn apply_child_task_out_parameters(
    session: &mut DbSession,
    case_definition: &CmmnCaseDefinition,
    association: &CmmnTaskInstanceAssociation,
    child_variables: Option<&Map<String, Value>>,
) -> Result<(), CmmnError> {
    let out_parameters = match find_child_task_out_parameters(
        ContainerView::from_case_plan_model(&case_definition.model.case_plan_model),
        &association.kind,
        &association.task_definition_id,
    ) {
        Some(out_parameters) if !out_parameters.is_empty() => out_parameters,
        _ => return Ok(()),
    };

    let loaded_child_variables;
    let source_variables = match child_variables {
        Some(child_variables) => child_variables,
        None if association.kind == CmmnTaskAssociationKind::CaseTask => {
            let Some(child_instance_id) = association.child_instance_id.as_deref() else {
                return Ok(());
            };
            match load_case_instance_session(session, child_instance_id)? {
                Some(child_case) => {
                    loaded_child_variables = child_case.variables;
                    &loaded_child_variables
                }
                None => return Ok(()),
            }
        }
        // Process task completion without a variable payload: the declared targets are still
        // written, resolving every source to null (IOParameterUtil.java:64-66 sets null when the
        // source variable is missing on the child).
        None => {
            loaded_child_variables = Map::new();
            &loaded_child_variables
        }
    };

    let mapped = map_io_parameters(out_parameters, source_variables);
    if mapped.is_empty() {
        return Ok(());
    }
    let mut parent_case = load_case_instance_session(session, &association.case_instance_id)?
        .ok_or_else(|| {
            CmmnError::storage(format!(
                "CMMN case instance '{}' disappeared during out-parameter mapping",
                association.case_instance_id
            ))
        })?;
    for (name, value) in mapped {
        parent_case.variables.insert(name, value);
    }
    persist_case_instance_session(session, &parent_case)
}

fn find_child_task_out_parameters<'a>(
    container: ContainerView<'a>,
    kind: &CmmnTaskAssociationKind,
    task_definition_id: &str,
) -> Option<&'a [CmmnIOParameter]> {
    let direct = match kind {
        CmmnTaskAssociationKind::CaseTask => container
            .case_tasks
            .iter()
            .find(|task| task.id == task_definition_id)
            .map(|task| task.out_parameters.as_slice()),
        CmmnTaskAssociationKind::ProcessTask => container
            .process_tasks
            .iter()
            .find(|task| task.id == task_definition_id)
            .map(|task| task.out_parameters.as_slice()),
    };
    if direct.is_some() {
        return direct;
    }
    container.stages.iter().find_map(|stage| {
        find_child_task_out_parameters(ContainerView::from_stage(stage), kind, task_definition_id)
    })
}

fn complete_task_plan_item_event_session(
    session: &mut DbSession,
    case_definition: &CmmnCaseDefinition,
    association: &CmmnTaskInstanceAssociation,
    standard_event: &str,
) -> Result<(), CmmnError> {
    if !plan_item_standard_event_occurred(
        session,
        &association.case_instance_id,
        &association.plan_item_id,
        standard_event,
    )? {
        record_plan_item_standard_event_session(
            session,
            &association.case_instance_id,
            &association.plan_item_id,
            standard_event,
        )?;
    }
    handle_plan_item_standard_event(
        session,
        case_definition,
        &association.case_instance_id,
        &association.plan_item_id,
        standard_event,
        association.stage_instance_id.as_deref(),
    )
}

fn complete_parent_case_task_associations_for_child_case(
    session: &mut DbSession,
    child_case_instance_id: &str,
) -> Result<(), CmmnError> {
    let child_case = match load_case_instance_session(session, child_case_instance_id)? {
        Some(child_case) if child_case.state == CmmnCaseInstanceState::Completed => child_case,
        _ => return Ok(()),
    };
    for mut association in
        load_active_case_task_associations_by_child_case_session(session, &child_case.id)?
    {
        complete_task_association_session(
            session,
            &mut association,
            CmmnTaskAssociationState::Completed,
            None,
            Some(&child_case.variables),
        )?;
    }
    Ok(())
}

fn terminate_parent_case_task_associations_for_child_case(
    session: &mut DbSession,
    child_case_instance_id: &str,
) -> Result<(), CmmnError> {
    for mut association in
        load_active_case_task_associations_by_child_case_session(session, child_case_instance_id)?
    {
        complete_task_association_session(
            session,
            &mut association,
            CmmnTaskAssociationState::Failed,
            Some(format!(
                "Child CMMN case instance '{}' was terminated",
                child_case_instance_id
            )),
            None,
        )?;
    }
    Ok(())
}

fn delete_runtime_case_instance_session(
    session: &mut DbSession,
    case_instance_id: &str,
) -> Result<(), CmmnError> {
    let mut params = DbParams::new();
    params.push(case_instance_id);
    let rendered = RenderedStatement::new(
        "SELECT ID_ FROM ACT_CMMN_HUMAN_TASK WHERE CASE_INSTANCE_ID_ = ?".to_string(),
        params,
    );
    let rows = session.select_raw(rendered)?;
    let task_ids = rows
        .into_iter()
        .map(|row| {
            row.get_text("ID_")
                .ok_or_else(|| CmmnError::storage("Missing ID_ in CMMN human task row"))
        })
        .collect::<Result<Vec<_>, _>>()?;

    let mut params = DbParams::new();
    params.push(case_instance_id);
    session.execute_raw(RenderedStatement::new(
        "DELETE FROM ACT_CMMN_IDENTITY_LINK WHERE SCOPE_TYPE_ = 'caseInstance' AND SCOPE_ID_ = ?"
            .to_string(),
        params,
    ))?;
    for task_id in task_ids {
        let mut params = DbParams::new();
        params.push(task_id.as_str());
        session.execute_raw(RenderedStatement::new(
            "DELETE FROM ACT_CMMN_IDENTITY_LINK WHERE SCOPE_TYPE_ = 'humanTask' AND SCOPE_ID_ = ?"
                .to_string(),
            params,
        ))?;
    }
    let mut params = DbParams::new();
    params.push(case_instance_id);
    params.push(case_instance_id);
    session.execute_raw(RenderedStatement::new(
        "DELETE FROM ACT_CMMN_JOB WHERE SCOPE_ID_ = ? OR SUB_SCOPE_ID_ = ?".to_string(),
        params,
    ))?;
    let mut params = DbParams::new();
    params.push(case_instance_id);
    session.execute_raw(RenderedStatement::new(
        "DELETE FROM ACT_CMMN_EVENT_SUBSCRIPTION WHERE CASE_INSTANCE_ID_ = ?".to_string(),
        params,
    ))?;
    let mut params = DbParams::new();
    params.push(case_instance_id);
    session.execute_raw(RenderedStatement::new(
        "DELETE FROM ACT_CMMN_PLAN_ITEM_EVENT WHERE CASE_INSTANCE_ID_ = ?".to_string(),
        params,
    ))?;
    let mut params = DbParams::new();
    params.push(case_instance_id);
    session.execute_raw(RenderedStatement::new(
        "DELETE FROM ACT_CMMN_TASK_INSTANCE_ASSOCIATION WHERE CASE_INSTANCE_ID_ = ?".to_string(),
        params,
    ))?;
    let mut params = DbParams::new();
    params.push(case_instance_id);
    session.execute_raw(RenderedStatement::new(
        "DELETE FROM ACT_CMMN_HUMAN_TASK WHERE CASE_INSTANCE_ID_ = ?".to_string(),
        params,
    ))?;
    let mut params = DbParams::new();
    params.push(case_instance_id);
    session.execute_raw(RenderedStatement::new(
        "DELETE FROM ACT_CMMN_STAGE_INSTANCE WHERE CASE_INSTANCE_ID_ = ?".to_string(),
        params,
    ))?;
    let mut params = DbParams::new();
    params.push(case_instance_id);
    session.execute_raw(RenderedStatement::new(
        "DELETE FROM ACT_CMMN_CASE_INSTANCE WHERE ID_ = ?".to_string(),
        params,
    ))?;
    Ok(())
}

fn persist_ended_stage_history_for_case_session(
    session: &mut DbSession,
    case_instance_id: &str,
    ended_at: chrono::DateTime<Utc>,
) -> Result<(), CmmnError> {
    let mut params = DbParams::new();
    params.push(case_instance_id);
    let rendered = RenderedStatement::new(
        "SELECT DATA_ FROM ACT_CMMN_STAGE_INSTANCE WHERE CASE_INSTANCE_ID_ = ? ORDER BY ACTIVATED_AT_ ASC, ID_ ASC".to_string(),
        params,
    );
    let rows = session.select_raw(rendered)?;
    let stages = rows
        .into_iter()
        .map(|row| {
            let json = row
                .get_text("DATA_")
                .ok_or_else(|| CmmnError::storage("Missing DATA_ in CMMN stage instance row"))?;
            serde_json::from_str::<CmmnStageInstance>(&json).map_err(CmmnError::from)
        })
        .collect::<Result<Vec<_>, _>>()?;

    for mut stage in stages {
        if stage.ended_at.is_none() {
            stage.ended_at = Some(ended_at);
        }
        if stage.state == CmmnStageInstanceState::Active {
            stage.state = CmmnStageInstanceState::Terminated;
        }
        persist_historic_stage_instance_session(session, &stage)?;
    }

    Ok(())
}

fn stage_overview_from_stage_instance(stage: CmmnStageInstance) -> CmmnStageOverview {
    let current = stage.state == CmmnStageInstanceState::Active && stage.ended_at.is_none();
    let ended = stage.ended_at.is_some()
        || matches!(
            stage.state,
            CmmnStageInstanceState::Completed | CmmnStageInstanceState::Terminated
        );

    CmmnStageOverview {
        id: stage.stage_definition_id,
        name: stage.name,
        current,
        ended,
        end_time: stage.ended_at,
    }
}

pub(crate) fn load_case_instance_session(
    session: &mut DbSession,
    case_instance_id: &str,
) -> Result<Option<CmmnCaseInstance>, CmmnError> {
    let mut params = DbParams::new();
    params.push(case_instance_id);
    let rendered = RenderedStatement::new(
        "SELECT DATA_ FROM ACT_CMMN_CASE_INSTANCE WHERE ID_ = ?".to_string(),
        params,
    );
    let row = session.select_one_raw(rendered)?;
    row.map(|row| {
        let json = row
            .get_text("DATA_")
            .ok_or_else(|| CmmnError::storage("Missing DATA_ in CMMN case instance row"))?;
        serde_json::from_str::<CmmnCaseInstance>(&json).map_err(CmmnError::from)
    })
    .transpose()
}

fn load_case_definition_session(
    session: &mut DbSession,
    case_definition_id: &str,
) -> Result<Option<CmmnCaseDefinition>, CmmnError> {
    let mut params = DbParams::new();
    params.push(case_definition_id);
    let rendered = RenderedStatement::new(
        "SELECT DATA_ FROM ACT_CMMN_CASE_DEFINITION WHERE ID_ = ?".to_string(),
        params,
    );
    let row = session.select_one_raw(rendered)?;
    row.map(|row| {
        let json = row
            .get_text("DATA_")
            .ok_or_else(|| CmmnError::storage("Missing DATA_ in CMMN case definition row"))?;
        serde_json::from_str::<CmmnCaseDefinition>(&json).map_err(CmmnError::from)
    })
    .transpose()
}

fn latest_case_definition_by_key_session(
    session: &mut DbSession,
    case_definition_key: &str,
    tenant_id: Option<&str>,
) -> Result<CmmnCaseDefinition, CmmnError> {
    let mut params = DbParams::new();
    params.push(case_definition_key);
    params.push(tenant_id);
    params.push(tenant_id);
    let rendered = RenderedStatement::new(
        "SELECT DATA_ FROM ACT_CMMN_CASE_DEFINITION \
         WHERE CASE_KEY_ = ? \
           AND ((TENANT_ID_ IS NULL AND ? IS NULL) OR TENANT_ID_ = ?) \
           ORDER BY VERSION_ DESC, ID_ ASC \
           LIMIT 1"
            .to_string(),
        params,
    );
    let row = session.select_one_raw(rendered)?;
    row.map(|row| {
        let json = row
            .get_text("DATA_")
            .ok_or_else(|| CmmnError::storage("Missing DATA_ in CMMN case definition row"))?;
        serde_json::from_str::<CmmnCaseDefinition>(&json).map_err(CmmnError::from)
    })
    .transpose()?
    .ok_or_else(|| {
        CmmnError::not_found(format!(
            "CMMN case definition '{case_definition_key}' was not found"
        ))
    })
}

fn load_event_subscription_session(
    session: &mut DbSession,
    event_subscription_id: &str,
) -> Result<Option<CmmnEventSubscription>, CmmnError> {
    let mut params = DbParams::new();
    params.push(event_subscription_id);
    let rendered = RenderedStatement::new(
        "SELECT DATA_ FROM ACT_CMMN_EVENT_SUBSCRIPTION WHERE ID_ = ?".to_string(),
        params,
    );
    let row = session.select_one_raw(rendered)?;
    row.map(|row| {
        let json = row
            .get_text("DATA_")
            .ok_or_else(|| CmmnError::storage("Missing DATA_ in CMMN event subscription row"))?;
        serde_json::from_str::<CmmnEventSubscription>(&json).map_err(CmmnError::from)
    })
    .transpose()
}

fn load_stage_instance_session(
    session: &mut DbSession,
    stage_instance_id: &str,
) -> Result<Option<CmmnStageInstance>, CmmnError> {
    let mut params = DbParams::new();
    params.push(stage_instance_id);
    let rendered = RenderedStatement::new(
        "SELECT DATA_ FROM ACT_CMMN_STAGE_INSTANCE WHERE ID_ = ?".to_string(),
        params,
    );
    let row = session.select_one_raw(rendered)?;
    row.map(|row| {
        let json = row
            .get_text("DATA_")
            .ok_or_else(|| CmmnError::storage("Missing DATA_ in CMMN stage instance row"))?;
        serde_json::from_str::<CmmnStageInstance>(&json).map_err(CmmnError::from)
    })
    .transpose()
}

pub(crate) fn load_human_task_session(
    session: &mut DbSession,
    task_id: &str,
) -> Result<Option<CmmnHumanTaskInstance>, CmmnError> {
    let mut params = DbParams::new();
    params.push(task_id);
    let rendered = RenderedStatement::new(
        "SELECT DATA_ FROM ACT_CMMN_HUMAN_TASK WHERE ID_ = ?".to_string(),
        params,
    );
    let row = session.select_one_raw(rendered)?;
    row.map(|row| {
        let json = row
            .get_text("DATA_")
            .ok_or_else(|| CmmnError::storage("Missing DATA_ in CMMN human task row"))?;
        serde_json::from_str::<CmmnHumanTaskInstance>(&json).map_err(CmmnError::from)
    })
    .transpose()
}

fn matches_optional(filter: &Option<String>, actual: &str) -> bool {
    filter.as_ref().is_none_or(|value| value == actual)
}

fn matches_optional_option(filter: &Option<String>, actual: Option<&str>) -> bool {
    filter
        .as_ref()
        .is_none_or(|value| actual == Some(value.as_str()))
}

/// SQL `LIKE` matching (`%` any sequence, `_` single char) for a required field,
/// mirroring Java's `LIKE` semantics used by `taskNameLike` and friends.
fn like_optional(pattern: &Option<String>, actual: &str) -> bool {
    pattern
        .as_ref()
        .is_none_or(|pattern| like_match(pattern, actual))
}

/// `LIKE` with the pattern and value lower-cased, mirroring Java's
/// `taskNameLikeIgnoreCase` (TaskQueryImpl) which lower-cases both sides.
fn like_optional_ignore_case(pattern: &Option<String>, actual: &str) -> bool {
    pattern
        .as_ref()
        .is_none_or(|pattern| like_match(&pattern.to_lowercase(), &actual.to_lowercase()))
}

fn like_optional_option(pattern: &Option<String>, actual: Option<&str>) -> bool {
    pattern
        .as_ref()
        .is_none_or(|pattern| actual.is_some_and(|value| like_match(pattern, value)))
}

fn like_optional_option_ignore_case(pattern: &Option<String>, actual: Option<&str>) -> bool {
    pattern.as_ref().is_none_or(|pattern| {
        actual.is_some_and(|value| like_match(&pattern.to_lowercase(), &value.to_lowercase()))
    })
}

fn like_match(pattern: &str, haystack: &str) -> bool {
    // Delegates to flowable_engine_common::like::sql_like_matches (P143 unified LIKE, O(m)+512 cap).
    flowable_engine_common::like::sql_like_matches(pattern, haystack)
}

/// Java `TaskEntity.priority` is an int (HumanTaskActivityBehavior.java:278-288);
/// Rust stores the resolved literal/expression string, so numeric query filters
/// parse it. Values Java could never have stored (non-numeric) fail to parse and
/// therefore never match a numeric filter.
fn parse_priority(value: &Option<String>) -> Option<i64> {
    value.as_deref().and_then(|value| value.parse::<i64>().ok())
}

/// Parse a stored due-date string to a `DateTime<Utc>` for the Java `taskDueDate`
/// family comparisons. Java stores a resolved `Date` (HumanTaskActivityBehavior.java:330-347)
/// while Rust keeps the raw resolved string; the lenient formats mirror
/// Java's `RequestUtil.parseLongDate` (RequestUtil.java:40-58).
fn parse_cmmn_datetime(value: &str) -> Option<DateTime<Utc>> {
    if let Ok(date) = DateTime::parse_from_rfc3339(value) {
        return Some(date.with_timezone(&Utc));
    }
    for format in [
        "%Y-%m-%dT%H:%M:%S%.3f",
        "%Y-%m-%dT%H:%M:%S",
        "%Y-%m-%dT%H:%M",
    ] {
        if let Ok(date) = NaiveDateTime::parse_from_str(value, format) {
            return Some(date.and_utc());
        }
    }
    NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .ok()
        .and_then(|date| date.and_hms_opt(0, 0, 0))
        .map(|date| date.and_utc())
}

fn page_items<T>(items: Vec<T>, start: usize, size: Option<usize>) -> PagedResult<T> {
    let total = items.len();
    let start = start.min(total);
    let page_size = size.unwrap_or(total.saturating_sub(start));
    let data = items
        .into_iter()
        .skip(start)
        .take(page_size)
        .collect::<Vec<_>>();

    PagedResult {
        start,
        size: data.len(),
        total,
        data,
    }
}

pub struct CmmnCaseFileItemService {
    store: CmmnStore,
}

impl CmmnCaseFileItemService {
    pub(crate) fn new(store: CmmnStore) -> Self {
        Self { store }
    }

    pub fn get_case_file_item(
        &self,
        case_instance_id: &str,
        item_id: &str,
    ) -> Result<CmmnCaseFileItem, CmmnError> {
        let mut session = self.store.create_session()?;
        let case_instance = load_case_instance_session(&mut session, case_instance_id)?
            .ok_or_else(|| {
                CmmnError::not_found(format!(
                    "CMMN case instance '{case_instance_id}' was not found"
                ))
            })?;

        case_instance
            .case_file_items
            .into_iter()
            .find(|item| item.id == item_id)
            .ok_or_else(|| {
                CmmnError::not_found(format!(
                    "CMMN case file item '{item_id}' was not found in case instance '{case_instance_id}'"
                ))
            })
    }

    pub fn create_case_file_item(
        &self,
        case_instance_id: &str,
        item: CmmnCaseFileItem,
    ) -> Result<(), CmmnError> {
        let mut session = self.store.create_session()?;

        let mut case_instance = load_case_instance_session(&mut session, case_instance_id)?
            .ok_or_else(|| {
                CmmnError::not_found(format!(
                    "CMMN case instance '{case_instance_id}' was not found"
                ))
            })?;

        let item_id = item.id.clone();
        let definition_refs = {
            let mut graph = CaseFileGraph::new(&mut case_instance.case_file_items)?;
            graph.insert(item)?;
            graph.ancestry_definition_refs(&item_id)
        };
        persist_case_instance_session(&mut session, &case_instance)?;
        persist_historic_case_session(
            &mut session,
            &CmmnHistoricCaseInstance::from(&case_instance),
        )?;

        let case_definition =
            load_case_definition_session(&mut session, &case_instance.case_definition_id)?
                .ok_or_else(|| {
                    CmmnError::storage(format!(
                        "CMMN case definition '{}' disappeared during case file item creation",
                        case_instance.case_definition_id
                    ))
                })?;

        for definition_ref in definition_refs {
            handle_case_file_item_on_part(
                &mut session,
                &case_definition,
                case_instance_id,
                &definition_ref,
                CmmnCaseFileItemOnPart::STANDARD_EVENT_CREATE,
            )?;
        }

        session.commit()?;
        Ok(())
    }

    pub fn create_child_case_file_item(
        &self,
        case_instance_id: &str,
        parent_item_id: &str,
        item: CmmnCaseFileItem,
    ) -> Result<(), CmmnError> {
        self.create_case_file_item(case_instance_id, item.with_parent(parent_item_id))
    }

    pub fn get_case_file_item_children(
        &self,
        case_instance_id: &str,
        parent_item_id: &str,
    ) -> Result<Vec<CmmnCaseFileItem>, CmmnError> {
        let mut session = self.store.create_session()?;
        let mut case_instance = load_case_instance_session(&mut session, case_instance_id)?
            .ok_or_else(|| {
                CmmnError::not_found(format!(
                    "CMMN case instance '{case_instance_id}' was not found"
                ))
            })?;
        let graph = CaseFileGraph::new(&mut case_instance.case_file_items)?;
        Ok(graph.children(parent_item_id))
    }

    pub fn get_case_file_item_descendants(
        &self,
        case_instance_id: &str,
        parent_item_id: &str,
    ) -> Result<Vec<CmmnCaseFileItem>, CmmnError> {
        let mut session = self.store.create_session()?;
        let mut case_instance = load_case_instance_session(&mut session, case_instance_id)?
            .ok_or_else(|| {
                CmmnError::not_found(format!(
                    "CMMN case instance '{case_instance_id}' was not found"
                ))
            })?;
        let graph = CaseFileGraph::new(&mut case_instance.case_file_items)?;
        Ok(graph.descendants(parent_item_id))
    }

    pub fn update_case_file_item(
        &self,
        case_instance_id: &str,
        item_id: &str,
        value: serde_json::Value,
    ) -> Result<(), CmmnError> {
        let mut session = self.store.create_session()?;

        let mut case_instance = load_case_instance_session(&mut session, case_instance_id)?
            .ok_or_else(|| {
                CmmnError::not_found(format!(
                    "CMMN case instance '{case_instance_id}' was not found"
                ))
            })?;

        let definition_refs = {
            let mut graph = CaseFileGraph::new(&mut case_instance.case_file_items)?;
            let item = graph.get_mut(item_id).ok_or_else(|| CmmnError::not_found(format!("CMMN case file item '{item_id}' was not found in case instance '{case_instance_id}'")))?;
            item.value = Some(value);
            item.version = item.version.saturating_add(1);
            graph.ancestry_definition_refs(item_id)
        };
        persist_case_instance_session(&mut session, &case_instance)?;
        persist_historic_case_session(
            &mut session,
            &CmmnHistoricCaseInstance::from(&case_instance),
        )?;

        let case_definition =
            load_case_definition_session(&mut session, &case_instance.case_definition_id)?
                .ok_or_else(|| {
                    CmmnError::storage(format!(
                        "CMMN case definition '{}' disappeared during case file item update",
                        case_instance.case_definition_id
                    ))
                })?;

        for definition_ref in definition_refs {
            handle_case_file_item_on_part(
                &mut session,
                &case_definition,
                case_instance_id,
                &definition_ref,
                CmmnCaseFileItemOnPart::STANDARD_EVENT_UPDATE,
            )?;
        }

        session.commit()?;
        Ok(())
    }

    pub fn delete_case_file_item(
        &self,
        case_instance_id: &str,
        item_id: &str,
    ) -> Result<(), CmmnError> {
        let mut session = self.store.create_session()?;

        let mut case_instance = load_case_instance_session(&mut session, case_instance_id)?
            .ok_or_else(|| {
                CmmnError::not_found(format!(
                    "CMMN case instance '{case_instance_id}' was not found"
                ))
            })?;

        let definition_refs = {
            let mut graph = CaseFileGraph::new(&mut case_instance.case_file_items)?;
            let refs = graph.ancestry_definition_refs(item_id);
            graph.remove_subtree(item_id)?;
            refs
        };
        persist_case_instance_session(&mut session, &case_instance)?;
        persist_historic_case_session(
            &mut session,
            &CmmnHistoricCaseInstance::from(&case_instance),
        )?;

        let case_definition =
            load_case_definition_session(&mut session, &case_instance.case_definition_id)?
                .ok_or_else(|| {
                    CmmnError::storage(format!(
                        "CMMN case definition '{}' disappeared during case file item deletion",
                        case_instance.case_definition_id
                    ))
                })?;

        for definition_ref in definition_refs {
            handle_case_file_item_on_part(
                &mut session,
                &case_definition,
                case_instance_id,
                &definition_ref,
                CmmnCaseFileItemOnPart::STANDARD_EVENT_DELETE,
            )?;
        }

        session.commit()?;
        Ok(())
    }

    pub fn complete_case_file_item(
        &self,
        case_instance_id: &str,
        item_id: &str,
    ) -> Result<(), CmmnError> {
        let mut session = self.store.create_session()?;

        let case_instance = load_case_instance_session(&mut session, case_instance_id)?
            .ok_or_else(|| {
                CmmnError::not_found(format!(
                    "CMMN case instance '{case_instance_id}' was not found"
                ))
            })?;

        let item = case_instance
            .case_file_items
            .iter()
            .find(|item| item.id == item_id)
            .ok_or_else(|| {
                CmmnError::not_found(format!(
                    "CMMN case file item '{item_id}' was not found in case instance '{case_instance_id}'"
                ))
            })?;

        if item.state == CmmnCaseFileItemState::Removed {
            return Err(CmmnError::conflict(format!(
                "CMMN case file item '{item_id}' is removed and cannot be completed"
            )));
        }

        let case_definition =
            load_case_definition_session(&mut session, &case_instance.case_definition_id)?
                .ok_or_else(|| {
                    CmmnError::storage(format!(
                        "CMMN case definition '{}' disappeared during case file item completion",
                        case_instance.case_definition_id
                    ))
                })?;

        handle_case_file_item_on_part(
            &mut session,
            &case_definition,
            case_instance_id,
            item_id,
            CmmnCaseFileItemOnPart::STANDARD_EVENT_COMPLETE,
        )?;

        session.commit()?;
        Ok(())
    }
}

fn handle_case_file_item_on_part(
    session: &mut DbSession,
    case_definition: &CmmnCaseDefinition,
    case_instance_id: &str,
    case_file_item_ref: &str,
    standard_event: &str,
) -> Result<(), CmmnError> {
    let case_instance = load_case_instance_session(session, case_instance_id)?.ok_or_else(|| {
        CmmnError::storage(format!(
            "CMMN case instance '{case_instance_id}' disappeared during case file item onPart evaluation"
        ))
    })?;

    let container = ContainerView::from_case_plan_model(&case_definition.model.case_plan_model);

    evaluate_case_file_item_on_parts_in_container(
        session,
        case_definition,
        &case_instance,
        container,
        case_file_item_ref,
        standard_event,
    )?;

    Ok(())
}

fn evaluate_case_file_item_on_parts_in_container(
    session: &mut DbSession,
    case_definition: &CmmnCaseDefinition,
    case_instance: &CmmnCaseInstance,
    container: ContainerView<'_>,
    case_file_item_ref: &str,
    standard_event: &str,
) -> Result<(), CmmnError> {
    for sentry in container.sentries {
        if sentry.case_file_item_on_parts.iter().any(|on_part| {
            on_part.case_file_item_ref == case_file_item_ref
                && on_part.standard_event == standard_event
        }) && sentry_if_part_satisfied(session, sentry, case_instance)?
        {
            activate_plan_items_by_entry_criterion(
                session,
                case_definition,
                case_instance,
                container,
                &sentry.id,
                None,
            )?;
            // Java removes the sentry part instances together with the plan
            // item instance that leaves its waiting state once the criterion
            // triggered (PlanItemInstanceEntityManagerImpl.java:172-180).
            delete_sentry_if_part_satisfied(session, &case_instance.id, &sentry.id)?;
        }
    }

    for stage in container.stages {
        evaluate_case_file_item_on_parts_in_container(
            session,
            case_definition,
            case_instance,
            ContainerView::from_stage(stage),
            case_file_item_ref,
            standard_event,
        )?;
    }

    Ok(())
}

fn activate_plan_items_by_entry_criterion(
    session: &mut DbSession,
    case_definition: &CmmnCaseDefinition,
    case_instance: &CmmnCaseInstance,
    container: ContainerView<'_>,
    sentry_id: &str,
    parent_stage_instance_id: Option<&str>,
) -> Result<(), CmmnError> {
    for plan_item in container.plan_items {
        if plan_item.entry_criterion_ids.is_empty()
            || !plan_item
                .entry_criterion_ids
                .iter()
                .any(|criterion_id| criterion_id == sentry_id)
        {
            continue;
        }

        if let Some(human_task) = container
            .human_tasks
            .iter()
            .find(|candidate| candidate.id == plan_item.definition_ref)
        {
            if human_task_instance_exists(
                session,
                &case_instance.id,
                &plan_item.id,
                parent_stage_instance_id,
            )? {
                continue;
            }
            activate_human_task(
                session,
                case_definition,
                case_instance,
                plan_item,
                human_task,
                parent_stage_instance_id,
            )?;
            continue;
        }

        if let Some(stage) = container
            .stages
            .iter()
            .find(|candidate| candidate.id == plan_item.definition_ref)
        {
            if stage_instance_exists(session, &case_instance.id, &plan_item.id)? {
                continue;
            }
            activate_stage(
                session,
                case_definition,
                case_instance,
                plan_item,
                stage,
                parent_stage_instance_id,
            )?;
            continue;
        }
    }

    Ok(())
}

impl CmmnDiscretionaryItem {
    pub fn activate(
        &self,
        _case_instance_id: &str,
        _context: &mut CmmnCaseInstance,
    ) -> Result<(), CmmnError> {
        if self.required && self.manual_activation {
            return Err(CmmnError::validation(format!(
                "CMMN discretionary item '{}' cannot be both required and manual activation",
                self.id
            )));
        }

        let _plan_item = CmmnPlanItem::new(format!("plan-item-{}", self.id), &self.definition_ref);

        Ok(())
    }

    pub fn complete(
        &self,
        _case_instance_id: &str,
        _context: &mut CmmnCaseInstance,
    ) -> Result<(), CmmnError> {
        Ok(())
    }
}

impl CmmnPlanFragment {
    pub fn execute(
        &self,
        case_definition: &CmmnCaseDefinition,
        case_instance: &CmmnCaseInstance,
        session: &mut DbSession,
    ) -> Result<(), CmmnError> {
        for plan_item in &self.plan_items {
            if plan_item.entry_criterion_ids.is_empty()
                && let Some(human_task) = self
                    .human_tasks
                    .iter()
                    .find(|ht| ht.id == plan_item.definition_ref)
            {
                activate_human_task(
                    session,
                    case_definition,
                    case_instance,
                    plan_item,
                    human_task,
                    None,
                )?;
            }
        }

        Ok(())
    }

    pub fn find_sentry(&self, sentry_id: &str) -> Option<&CmmnSentry> {
        self.sentries.iter().find(|s| s.id == sentry_id)
    }

    pub fn find_plan_item(&self, plan_item_id: &str) -> Option<&CmmnPlanItem> {
        self.plan_items.iter().find(|pi| pi.id == plan_item_id)
    }
}

#[cfg(test)]
mod completion_rule_tests {
    use super::{PlanItemCompletionState, plan_item_ignored_for_completion};

    // Java: PlanItemInstanceContainerUtil.java:86 - IGNORE always skips regardless of state.
    #[test]
    fn ignore_rule_always_ignores() {
        for state in [
            PlanItemCompletionState::Active,
            PlanItemCompletionState::Available,
            PlanItemCompletionState::Enabled,
        ] {
            assert!(plan_item_ignored_for_completion(
                Some("ignore"),
                false,
                state,
                false,
                false
            ));
        }
    }

    // Java: PlanItemInstanceContainerUtil.java:128-131 - IGNORE_IF_AVAILABLE only skips AVAILABLE.
    #[test]
    fn ignore_if_available_only_ignores_available() {
        assert!(plan_item_ignored_for_completion(
            Some("ignoreIfAvailable"),
            false,
            PlanItemCompletionState::Available,
            false,
            false
        ));
        assert!(!plan_item_ignored_for_completion(
            Some("ignoreIfAvailable"),
            false,
            PlanItemCompletionState::Enabled,
            false,
            false
        ));
        assert!(!plan_item_ignored_for_completion(
            Some("ignoreIfAvailable"),
            false,
            PlanItemCompletionState::Active,
            false,
            false
        ));
    }

    // Java: PlanItemInstanceContainerUtil.java:122-125 - IGNORE_IF_AVAILABLE_OR_ENABLED skips
    // AVAILABLE and ENABLED but never ACTIVE.
    #[test]
    fn ignore_if_available_or_enabled_covers_enabled() {
        assert!(plan_item_ignored_for_completion(
            Some("ignoreIfAvailableOrEnabled"),
            false,
            PlanItemCompletionState::Enabled,
            false,
            false
        ));
        assert!(plan_item_ignored_for_completion(
            Some("ignoreIfAvailableOrEnabled"),
            false,
            PlanItemCompletionState::Available,
            false,
            false
        ));
        assert!(!plan_item_ignored_for_completion(
            Some("ignoreIfAvailableOrEnabled"),
            false,
            PlanItemCompletionState::Active,
            false,
            false
        ));
    }

    // Java: PlanItemInstanceContainerUtil.java:94 + 181-184 - IGNORE_AFTER_FIRST_COMPLETION is
    // consulted for an ACTIVE plan item and returns the alreadyCompleted flag.
    #[test]
    fn ignore_after_first_completion_active_requires_already_completed() {
        assert!(plan_item_ignored_for_completion(
            Some("ignoreAfterFirstCompletion"),
            false,
            PlanItemCompletionState::Active,
            true,
            false
        ));
        assert!(!plan_item_ignored_for_completion(
            Some("ignoreAfterFirstCompletion"),
            false,
            PlanItemCompletionState::Active,
            false,
            false
        ));
    }

    // Java: PlanItemInstanceContainerUtil.java:134-140 - IGNORE_AFTER_FIRST_COMPLETION is also
    // consulted for repeatable plan items in non-active states, still gated on alreadyCompleted.
    #[test]
    fn ignore_after_first_completion_needs_active_or_repeatable() {
        // AVAILABLE + already completed but not repeatable -> not ignored (never reached in Java).
        assert!(!plan_item_ignored_for_completion(
            Some("ignoreAfterFirstCompletion"),
            false,
            PlanItemCompletionState::Available,
            true,
            false
        ));
        // AVAILABLE + already completed + repeatable -> ignored.
        assert!(plan_item_ignored_for_completion(
            Some("ignoreAfterFirstCompletion"),
            false,
            PlanItemCompletionState::Available,
            true,
            true
        ));
    }

    // Java: PlanItemInstanceContainerUtil.java:185-188 - the AVAILABLE/ENABLED variant only
    // applies to repeatable plan items in AVAILABLE/ENABLED state that already completed.
    #[test]
    fn ignore_after_first_completion_if_available_or_enabled() {
        assert!(plan_item_ignored_for_completion(
            Some("ignoreAfterFirstCompletionIfAvailableOrEnabled"),
            false,
            PlanItemCompletionState::Enabled,
            true,
            true
        ));
        // Not repeatable -> not ignored.
        assert!(!plan_item_ignored_for_completion(
            Some("ignoreAfterFirstCompletionIfAvailableOrEnabled"),
            false,
            PlanItemCompletionState::Enabled,
            true,
            false
        ));
        // ACTIVE state -> not covered by this rule.
        assert!(!plan_item_ignored_for_completion(
            Some("ignoreAfterFirstCompletionIfAvailableOrEnabled"),
            false,
            PlanItemCompletionState::Active,
            true,
            true
        ));
    }

    // Java: PlanItemInstanceContainerUtil.java:128-131 - a completionNeutral plan item is ignored
    // while AVAILABLE, independent of any parentCompletionRule.
    #[test]
    fn completion_neutral_ignores_available_only() {
        assert!(plan_item_ignored_for_completion(
            None,
            true,
            PlanItemCompletionState::Available,
            false,
            false
        ));
        assert!(!plan_item_ignored_for_completion(
            None,
            true,
            PlanItemCompletionState::Enabled,
            false,
            false
        ));
        assert!(!plan_item_ignored_for_completion(
            None,
            true,
            PlanItemCompletionState::Active,
            false,
            false
        ));
    }

    // No rule and not completion-neutral: never ignored (byte-identical to legacy behavior).
    #[test]
    fn no_rule_never_ignores() {
        for state in [
            PlanItemCompletionState::Active,
            PlanItemCompletionState::Available,
            PlanItemCompletionState::Enabled,
        ] {
            assert!(!plan_item_ignored_for_completion(
                None, false, state, true, true
            ));
        }
        // An unknown / "default" rule string is inert as well.
        assert!(!plan_item_ignored_for_completion(
            Some("default"),
            false,
            PlanItemCompletionState::Available,
            true,
            true
        ));
    }
}
