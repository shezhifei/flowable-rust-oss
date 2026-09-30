use crate::agenda::FlowableEngineAgenda;
use crate::bpmn::behavior::boundary_event_activity_behavior::{
    resolve_boundary_event_subscription, runtime_cancel_activity,
};
use crate::bpmn::behavior::error_event_support::resolve_error_event_ref;
use crate::bpmn::behavior::escalation_event_support::resolve_escalation_event_ref;
use crate::bpmn::behavior::event_registry_event_support::resolve_event_type_extension;
use crate::bpmn::job_category::resolve_job_category;
use crate::delegate::activity_behavior::ActivityBehavior;
use crate::identity::entities::IdentityLink;
use crate::interceptor::command_context::CommandContext;
use crate::persistence::runtime_store::{
    EventSubprocessTimerSubscription, EventSubscriptionKind, RuntimeBoundaryEventState,
    RuntimeTimerJobState,
};
use crate::runtime::execution::Execution;
use crate::task::Task;
use flowable_bpmn_model::model::{EventDefinitionEnum, FlowElementEnum};
use uuid::Uuid;

use crate::agenda::continue_process_operation::find_flow_element;

pub struct UserTaskActivityBehavior;

impl Default for UserTaskActivityBehavior {
    fn default() -> Self {
        Self::new()
    }
}

impl UserTaskActivityBehavior {
    pub fn new() -> Self {
        Self
    }
}

impl ActivityBehavior for UserTaskActivityBehavior {
    fn execute(
        &self,
        execution: &mut Execution,
        command_context: &mut CommandContext,
    ) -> Result<(), crate::error::FlowableError> {
        let activity_id = match execution.activity_id.clone() {
            Some(activity_id) => activity_id,
            None => {
                return Ok(());
            }
        };

        let process_definition_id = match execution.process_definition_id.clone() {
            Some(process_definition_id) => process_definition_id,
            None => {
                return Ok(());
            }
        };

        let (
            task_name,
            boundary_events,
            data_input_associations,
            candidate_users,
            candidate_groups,
            skip_expression,
            task_listeners,
            model_assignee,
            model_owner,
            model_category,
            model_form_key,
            model_due_date,
            model_priority,
        ) = {
            let model_arc = command_context
                .deployment_manager
                .get_bpmn_model(&process_definition_id);
            let maybe_flow_element = model_arc
                .as_ref()
                .and_then(|model| model.main_process.as_ref())
                .and_then(|process| find_flow_element(process, &activity_id));

            match maybe_flow_element {
                Some(FlowElementEnum::UserTask(user_task)) => {
                    let name = user_task
                        .task
                        .activity
                        .flow_node
                        .flow_element
                        .name
                        .clone()
                        .unwrap_or_else(|| activity_id.clone());
                    let boundary_events = user_task.task.activity.boundary_events.clone();
                    let data_input_associations =
                        user_task.task.activity.data_input_associations.clone();
                    (
                        name,
                        boundary_events,
                        data_input_associations,
                        user_task.candidate_users.clone(),
                        user_task.candidate_groups.clone(),
                        user_task.skip_expression.clone(),
                        user_task.task_listeners.clone(),
                        user_task.assignee.clone(),
                        // P86a: Java `UserTaskActivityBehavior.handleAssignments:363-371`
                        // also applies the model owner via `TaskHelper.changeTaskOwner`
                        // after insert — previously only assignee was resolved here.
                        user_task.owner.clone(),
                        user_task.category.clone(),
                        user_task.form_key.clone(),
                        user_task.due_date.clone(),
                        user_task.priority.clone(),
                    )
                }
                Some(_) => (
                    activity_id.clone(),
                    Vec::new(),
                    Vec::new(),
                    Vec::new(),
                    Vec::new(),
                    None,
                    Vec::new(),
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                ),
                None => (
                    activity_id.clone(),
                    Vec::new(),
                    Vec::new(),
                    Vec::new(),
                    Vec::new(),
                    None,
                    Vec::new(),
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                ),
            }
        };

        let process_instance_id = execution
            .process_instance_id
            .clone()
            .unwrap_or_else(|| execution.id.clone());

        let evaluation_execution =
            crate::engine::variable_service::evaluation_execution(command_context, execution);
        if crate::bpmn::skip_expression::should_skip_flow_element(
            skip_expression.as_deref(),
            "UserTask",
            evaluation_execution.activity_id.as_deref(),
            &evaluation_execution,
        )? {
            command_context
                .agenda
                .plan_take_outgoing_sequence_flows_operation(execution.clone());
            return Ok(());
        }

        // Apply Data Input Associations
        if !data_input_associations.is_empty() {
            let process_vars = execution.process_variables();
            match crate::engine::data_routing::DataRoutingService::apply_data_input_associations(
                &data_input_associations,
                &process_vars,
            ) {
                Ok(local_vars) => {
                    execution.set_process_variables(local_vars);
                }
                Err(err) => return Err(err),
            }
        }

        // W3 / A.2 #16b group F: name/category/formKey follow Java
        // handleName/handleCategory/handleFormKey — eval failure falls back to
        // the model text + warn; parse/compile failure propagates (N4-1).
        //
        // C4: a successful evaluation to null must NOT be replaced by
        // activity_id (Java `task.setName(null)`). `Task.name` is a plain
        // `String`, so Java null is represented as the empty string here —
        // distinct from the model-default fallback to activity_id which only
        // applies when the model carries no name attribute at all.
        let model_had_name = !task_name.is_empty() && task_name != activity_id;
        let resolved_name =
            resolve_user_task_f_field("name", Some(task_name.as_str()), &evaluation_execution)?;
        let task_name_for_new = match resolved_name {
            Some(name) => name,
            None if model_had_name => {
                // Legal null EL result → Java setName(null) → empty in Rust.
                String::new()
            }
            None => activity_id.clone(),
        };
        let mut task = Task::new(
            Uuid::new_v4().to_string(),
            process_instance_id.clone(),
            execution.id.clone(),
            activity_id.clone(),
            task_name_for_new,
        );
        task.tenant_id = execution.tenant_id.clone();
        task.category = resolve_user_task_f_field(
            "category",
            model_category.as_deref(),
            &evaluation_execution,
        )?;
        task.form_key = resolve_user_task_f_field(
            "formKey",
            model_form_key.as_deref(),
            &evaluation_execution,
        )?;
        if let Some(assignee) = resolve_user_task_assignment_expression(
            model_assignee.as_deref(),
            &evaluation_execution,
        )? {
            task.assignee = Some(assignee);
        }
        // P86a: Java `UserTaskActivityBehavior.handleAssignments:363-371` sets
        // owner the same way as assignee (expression-evaluated, then
        // `TaskHelper.changeTaskOwner`). Without this, `record_task_created`
        // would resolve owner from BPMN props into the historic row only, and
        // the subsequent `record_task_updated` would see None on the runtime
        // task and append a spurious null-owner historic identity link.
        if let Some(owner) =
            resolve_user_task_assignment_expression(model_owner.as_deref(), &evaluation_execution)?
        {
            task.owner = Some(owner);
        }
        task.due_date = crate::persistence::runtime_store::evaluate_user_task_due_date(
            model_due_date.as_deref(),
            &evaluation_execution,
            command_context.runtime_store.time_source().now(),
        )?;
        // C6 / A.2 #16c group S: priority follows Java handlePriority:274-291 —
        // getValue has no catch, evaluation errors propagate; non-numeric result
        // is FlowableIllegalArgumentException.
        if task.priority.is_none() {
            task.priority =
                resolve_user_task_priority(model_priority.as_deref(), &evaluation_execution)?;
        }

        command_context
            .history_manager
            .record_task_created(&task, &mut command_context.session)?;

        let task_id = task.id.clone();
        command_context
            .task_entity_manager
            .insert(&task, &mut command_context.session)?;
        insert_candidate_identity_links(
            command_context,
            &task_id,
            &process_instance_id,
            &process_definition_id,
            &candidate_users,
            &candidate_groups,
        )?;

        // Task listeners: create (always), assignment (when assignee is set).
        crate::bpmn::listener::notify_task_listeners(
            &mut task,
            execution,
            command_context,
            &task_listeners,
            "create",
            &evaluation_execution,
        )?;
        // P53 layer 1: dispatch `TASK_CREATED` after the create task listener
        // has run (Java `TaskHelper.completeTaskCreate` flow). Listeners
        // can rely on TASK_CREATED firing once per persisted task row.
        crate::engine::event_dispatcher::dispatch_task_created(
            command_context,
            &task.id,
            Some(&task.process_instance_id),
            Some(&task.execution_id),
            None,
        );
        if task.assignee.is_some() {
            crate::bpmn::listener::notify_task_listeners(
                &mut task,
                execution,
                command_context,
                &task_listeners,
                "assignment",
                &evaluation_execution,
            )?;
            // P53: emit `TASK_ASSIGNED` whenever the initial creation
            // already carries an assignee (Java `TaskHelper` parity).
            crate::engine::event_dispatcher::dispatch_task_assigned(
                command_context,
                &task.id,
                Some(&task.process_instance_id),
                Some(&task.execution_id),
            );
        }
        // Persist listener side-effects on the task and execution.
        // record_task_updated must run BEFORE the store update: it diffs
        // assignee/owner against the historic row to emit identity links
        // (Java HistoricTaskServiceImpl.recordTaskInfoChange:142-152).
        command_context
            .history_manager
            .record_task_updated(&task, &mut command_context.session)?;
        command_context
            .task_entity_manager
            .update(&task, &mut command_context.session)?;
        command_context
            .execution_entity_manager
            .update(execution, &mut command_context.session)?;

        let bpmn_model = command_context
            .deployment_manager
            .get_bpmn_model(&process_definition_id);

        // Register boundary events for this user task.
        //
        // Java parity (`ContinueProcessOperation#executeMultiInstanceSynchronous`
        // 221–233): for a multi-instance activity the boundary events attach
        // once, on the MI root execution — never per instance child
        // (`ContinueMultiInstanceOperation` creates no boundary events).
        // Resolve the host before the loop; re-registration by sibling
        // instances is deduplicated below.
        let boundary_host_id =
            crate::bpmn::behavior::multi_instance_support::boundary_host_execution_id(
                command_context,
                execution,
            );
        for boundary_event in boundary_events {
            if let Some(ref boundary_event_id) =
                boundary_event.event.flow_node.flow_element.base_element.id
            {
                if let [EventDefinitionEnum::TimerEventDefinition(timer_def)] =
                    boundary_event.event.event_definitions.as_slice()
                {
                    // Dedup: every MI instance child reaches this loop, but
                    // Java schedules exactly one timer job per boundary event
                    // for the whole MI activity
                    // (`BoundaryTimerEventActivityBehavior#execute` runs once
                    // on the MI-root boundary execution).
                    let already_registered = command_context
                        .runtime_store
                        .find_timer_job_states_by_process_instance_id(
                            &process_instance_id,
                            &mut command_context.session,
                        )
                        .iter()
                        .any(|state| {
                            state.is_boundary
                                && state.activity_id == *boundary_event_id
                                && state.execution_id == boundary_host_id
                        });
                    if already_registered {
                        continue;
                    }
                    // P17: EL-evaluate timeDate/timeDuration/timeCycle/endDate
                    // before P16 prepare_repeat (Java TimerUtil.createTimerEntity).
                    let now = command_context.runtime_store.time_source().now();
                    let schedule = crate::bpmn::timer_util::resolve_timer_schedule(
                        timer_def.time_date.as_ref(),
                        timer_def.time_duration.as_ref(),
                        timer_def.time_cycle.as_ref(),
                        timer_def.end_date.as_ref(),
                        timer_def.calendar_name.as_ref(),
                        &evaluation_execution,
                        &command_context.config.business_calendar_registry,
                        now,
                    )?;
                    let timer_job = RuntimeTimerJobState {
                        timer_job_id: Uuid::new_v4().to_string(),
                        process_instance_id: process_instance_id.clone(),
                        execution_id: boundary_host_id.clone(),
                        activity_id: boundary_event_id.clone(),
                        job_state: Some("timer".to_string()),
                        is_boundary: true,
                        attached_activity_id: Some(activity_id.clone()),
                        cancel_activity: boundary_event.cancel_activity,
                        time_duration: schedule.time_duration,
                        time_date: schedule.time_date,
                        time_cycle: schedule.time_cycle,
                        end_date: schedule.end_date,
                        calendar_name: schedule.calendar_name,
                        due_time: schedule.due_time,
                        lock_owner: None,
                        lock_time: None,
                        lock_expiration_time: None,
                        retries: crate::bpmn::timer_util::default_timer_retries(command_context),
                        error_message: None,
                        error_details: None,
                        category: resolve_job_category(
                            &boundary_event.event.flow_node.flow_element.base_element,
                            &evaluation_execution,
                        )?,
                        ..Default::default()
                    };
                    command_context
                        .runtime_store
                        .insert_timer_job_state(&timer_job, &mut command_context.session)?;
                    // P119: TIMER_SCHEDULED — Java TimerJobSchedulerImpl.java:69-73.
                    crate::engine::event_dispatcher::dispatch_timer_scheduled(
                        command_context,
                        &timer_job,
                    );
                    continue;
                }

                let event_sub = match resolve_boundary_event_subscription(
                    &boundary_event,
                    bpmn_model.as_deref(),
                ) {
                    Some(sub) => sub,
                    None => {
                        return Err(crate::error::FlowableError::UnsupportedElement {
                            element_type: "BoundaryEvent".to_string(),
                            activity_id: boundary_event_id.clone(),
                        });
                    }
                };

                let configuration =
                    crate::bpmn::behavior::boundary_event_activity_behavior::resolve_boundary_configuration(
                        &boundary_event,
                        Some(execution),
                    )?;
                let state = RuntimeBoundaryEventState {
                    boundary_event_id: boundary_event_id.clone(),
                    attached_activity_id: activity_id.clone(),
                    process_instance_id: process_instance_id.clone(),
                    // MI parity: keyed by (process_instance_id, boundary_event_id),
                    // so sibling instances idempotently re-write the same row
                    // hosted on the MI root.
                    host_execution_id: boundary_host_id.clone(),
                    cancel_activity: runtime_cancel_activity(&boundary_event, &event_sub),
                    event_subscription: event_sub,
                    // BoundaryEventRegistryEventActivityBehavior.java:68
                    configuration,
                };
                crate::bpmn::behavior::boundary_event_activity_behavior::insert_boundary_event_state_with_waiting(
                    command_context,
                    state,
                    execution.process_definition_id.as_deref(),
                );
            }
        }

        execution.is_active = false;
        command_context
            .execution_entity_manager
            .update(execution, &mut command_context.session)?;

        register_event_subprocess_timer_subscriptions(
            command_context,
            &process_definition_id,
            &process_instance_id,
            execution,
        )?;

        Ok(())
    }
}

/// Evaluates a user-task assignee/owner model expression (literal or `${...}`).
/// Shared by assignee and owner — Java `handleAssignments` uses the same
/// expression-manager path for both (`UserTaskActivityBehavior.java:346-371`).
///
/// A.2 #16a group S: assignee/owner `getValue` has no catch — evaluation
/// errors propagate. (name/description/category/formKey are group F and must
/// NOT use this helper.)
fn resolve_user_task_assignment_expression(
    model_value: Option<&str>,
    execution: &Execution,
) -> Result<Option<String>, crate::error::FlowableError> {
    let Some(raw) = model_value.map(str::trim).filter(|v| !v.is_empty()) else {
        return Ok(None);
    };
    if raw.starts_with("${") && raw.ends_with('}') {
        use crate::el::expression::SimpleExpression;
        let value = SimpleExpression::new(raw.to_string())
            .get_value_strict(execution)
            .map_err(|error| {
                crate::error::FlowableError::ExecutionError(format!(
                    "user task assignment expression '{raw}' failed: {error}"
                ))
            })?;
        Ok(value.and_then(|v| match v {
            serde_json::Value::String(s) => Some(s),
            serde_json::Value::Number(n) => Some(n.to_string()),
            serde_json::Value::Bool(b) => Some(b.to_string()),
            _ => None,
        }))
    } else {
        Ok(Some(raw.to_string()))
    }
}

/// F-group (research §2.2 / W3): user-task name / category / formKey follow
/// Java `UserTaskActivityBehavior.handleName:208-221` (and handleCategory /
/// handleFormKey): evaluate via `createExpression(…).getValue`, and on
/// `FlowableException` fall back to the **model text** + `warn` — never empty
/// string, never propagate an eval error.
///
/// N4-1 typing: a parse/compile failure (`ExpressionEvalError::CompileFailed`,
/// Java `ELException` from `createExpression`) **propagates**; only eval-time
/// failures are caught and turned into the fallback.
///
/// A successful evaluation to null clears the field (Java leaves the local
/// `String name = null`) — it does not substitute empty text.
fn resolve_user_task_f_field(
    field_label: &str,
    model_value: Option<&str>,
    execution: &Execution,
) -> Result<Option<String>, crate::error::FlowableError> {
    let Some(raw) = model_value.map(str::trim).filter(|v| !v.is_empty()) else {
        return Ok(None);
    };

    // Literal text: Java createExpression("My Task").getValue() returns the
    // literal itself (no EL), so keep the model text unchanged.
    if !(raw.starts_with("${") && raw.ends_with('}')) {
        return Ok(Some(raw.to_string()));
    }

    use crate::el::expression::SimpleExpression;
    match SimpleExpression::new(raw.to_string()).get_value_strict(execution) {
        Ok(Some(serde_json::Value::Null)) | Ok(None) => {
            // Legal null: clear the field (do NOT substitute empty string).
            Ok(None)
        }
        Ok(Some(value)) => Ok(Some(value_to_field_string(&value))),
        Err(error) if error.is_compile_failure() => {
            // N4-1: parse/compile failure is createExpression ELException —
            // not a catchable FlowableException. Propagate.
            Err(crate::error::FlowableError::ExecutionError(format!(
                "user task {field_label} expression '{raw}' failed to compile: {error}"
            )))
        }
        Err(error) => {
            // Eval-time FlowableException → Java catch → fallback + warn.
            tracing::warn!(
                "property not found in task {field_label} expression '{raw}': {error}"
            );
            Ok(Some(raw.to_string()))
        }
    }
}

/// Java `Object.toString()` mapping for F-group field values.
fn value_to_field_string(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Bool(b) => b.to_string(),
        other => other.to_string(),
    }
}

/// C6 / A.2 #16c group S: user-task priority follows Java
/// `handlePriority:274-291` — `getValue` has no catch (evaluation errors
/// propagate); a non-numeric result is `FlowableIllegalArgumentException`.
fn resolve_user_task_priority(
    model_value: Option<&str>,
    execution: &Execution,
) -> Result<Option<i32>, crate::error::FlowableError> {
    let Some(raw) = model_value.map(str::trim).filter(|v| !v.is_empty()) else {
        return Ok(None);
    };

    // Literal numeric text parses directly (Java Integer.valueOf).
    if !raw.starts_with("${") && !raw.starts_with("#{") {
        return raw
            .parse::<i32>()
            .map(Some)
            .map_err(|_| crate::error::FlowableError::ExecutionError(format!(
                "Priority does not resolve to a number: {raw}"
            )));
    }

    use crate::el::expression::SimpleExpression;
    match SimpleExpression::new(raw.to_string()).get_value_strict(execution) {
        Ok(Some(serde_json::Value::Number(n))) => n
            .as_i64()
            .map(|v| Some(v as i32))
            .ok_or_else(|| crate::error::FlowableError::ExecutionError(format!(
                "Priority does not resolve to a number: {n}"
            ))),
        Ok(Some(serde_json::Value::String(s))) => s
            .trim()
            .parse::<i32>()
            .map(Some)
            .map_err(|_| crate::error::FlowableError::ExecutionError(format!(
                "Priority does not resolve to a number: {s}"
            ))),
        Ok(Some(serde_json::Value::Null)) | Ok(None) => Ok(None),
        Ok(Some(other)) => Err(crate::error::FlowableError::ExecutionError(format!(
            "Priority expression does not resolve to a number: {other}"
        ))),
        Err(error) => Err(crate::error::FlowableError::ExecutionError(format!(
            "Priority expression '{raw}' failed: {error}"
        ))),
    }
}

fn insert_candidate_identity_links(
    command_context: &mut CommandContext,
    task_id: &str,
    process_instance_id: &str,
    process_definition_id: &str,
    candidate_users: &[String],
    candidate_groups: &[String],
) -> Result<(), crate::error::FlowableError> {
    for user_id in candidate_users {
        let link = IdentityLink {
            id: format!("task:{task_id}:users:{user_id}:type:candidate"),
            link_type: "candidate".to_string(),
            user_id: Some(user_id.clone()),
            group_id: None,
            task_id: Some(task_id.to_string()),
            process_instance_id: Some(process_instance_id.to_string()),
            process_definition_id: Some(process_definition_id.to_string()),
        };
        // P77: Java IdentityLinkUtil.handleTaskIdentityLinkAddition → historic IL.
        command_context
            .history_manager
            .record_identity_link_created(&link, &mut command_context.session)?;
        command_context
            .runtime_store
            .insert_identity_link(link, &mut command_context.session);
    }

    for group_id in candidate_groups {
        let link = IdentityLink {
            id: format!("task:{task_id}:groups:{group_id}:type:candidate"),
            link_type: "candidate".to_string(),
            user_id: None,
            group_id: Some(group_id.clone()),
            task_id: Some(task_id.to_string()),
            process_instance_id: Some(process_instance_id.to_string()),
            process_definition_id: Some(process_definition_id.to_string()),
        };
        command_context
            .history_manager
            .record_identity_link_created(&link, &mut command_context.session)?;
        command_context
            .runtime_store
            .insert_identity_link(link, &mut command_context.session);
    }
    Ok(())
}

fn register_event_subprocess_timer_subscriptions(
    command_context: &mut CommandContext,
    process_definition_id: &str,
    process_instance_id: &str,
    execution: &Execution,
) -> Result<(), crate::error::FlowableError> {
    register_event_subprocess_subscriptions(
        command_context,
        process_definition_id,
        process_instance_id,
        execution,
    )
}

/// Unified event subprocess subscription registration.
/// Scans all SubProcess and EventSubProcess elements in the process, registering
/// timer, message, and signal start event subscriptions for the given process instance.
fn register_event_subprocess_subscriptions(
    command_context: &mut CommandContext,
    process_definition_id: &str,
    process_instance_id: &str,
    execution: &Execution,
) -> Result<(), crate::error::FlowableError> {
    let bpmn_model = match command_context
        .deployment_manager
        .get_bpmn_model(process_definition_id)
    {
        Some(m) => m,
        None => return Ok(()),
    };
    let main_process = match bpmn_model.main_process.as_ref() {
        Some(p) => p,
        None => return Ok(()),
    };

    let now = command_context.runtime_store.time_source().now();

    // P6-B: job_category expression must walk the parent scope chain (forked
    // child maps may be empty after P4-7b).
    let evaluation_execution =
        crate::engine::variable_service::evaluation_execution(command_context, execution);

    // Get existing subscriptions to avoid duplicates
    let existing_timer_subs = command_context
        .runtime_store
        .find_event_subprocess_timer_subscriptions_by_process_instance_id(
            process_instance_id,
            &mut command_context.session,
        );

    let existing_event_subs = command_context
        .runtime_store
        .find_event_subprocess_event_subscriptions_by_process_instance_id(
            process_instance_id,
            &mut command_context.session,
        );

    // Collect all event-triggered subprocesses from both SubProcess and EventSubProcess variants
    let mut event_subprocesses: Vec<(&flowable_bpmn_model::model::SubProcess, String)> = Vec::new();
    for flow_element in &main_process.flow_elements {
        match flow_element {
            FlowElementEnum::EventSubProcess(esp) => {
                let esp_id = esp
                    .sub_process
                    .activity
                    .flow_node
                    .flow_element
                    .base_element
                    .id
                    .clone()
                    .unwrap_or_default();
                event_subprocesses.push((&esp.sub_process, esp_id));
            }
            FlowElementEnum::SubProcess(sub) if sub.triggered_by_event => {
                let sub_id = sub
                    .activity
                    .flow_node
                    .flow_element
                    .base_element
                    .id
                    .clone()
                    .unwrap_or_default();
                event_subprocesses.push((sub, sub_id));
            }
            _ => continue,
        }
    }

    for (sub_process, event_sub_id) in event_subprocesses {
        for inner_element in &sub_process.flow_elements {
            let start_event = match inner_element {
                FlowElementEnum::StartEvent(se) => se,
                _ => continue,
            };

            let start_event_id = start_event
                .event
                .flow_node
                .flow_element
                .base_element
                .id
                .clone()
                .unwrap_or_default();

            // Java ProcessInstanceHelper.java:371-398 — empty defs + eventType.
            if start_event.event.event_definitions.is_empty() {
                if let Some(event_type) = resolve_event_type_extension(
                    &start_event.event.flow_node.flow_element.base_element,
                ) {
                    if existing_event_subs.iter().any(|s| {
                        s.event_subprocess_id == event_sub_id
                            && s.event_kind == EventSubscriptionKind::EventRegistry
                            && s.event_ref == event_type
                    }) {
                        continue;
                    }
                    let sub_id = Uuid::new_v4().to_string();
                    // P134/P125: Java ProcessInstanceHelper.java:343-358 —
                    // ACTIVITY_MESSAGE/SIGNAL_WAITING on message/signal register.
                    crate::engine::event_dispatcher::insert_event_subprocess_subscription_with_waiting(
                        command_context,
                            crate::persistence::runtime_store::EventSubprocessEventSubscription {
                                subscription_id: sub_id,
                                process_instance_id: process_instance_id.to_string(),
                                scope_execution_id: Some(process_instance_id.to_string()),
                                scope_activity_id: None,
                                event_subprocess_id: event_sub_id.clone(),
                                start_event_id: start_event_id.clone(),
                                interrupting: start_event.interrupting,
                                event_kind: EventSubscriptionKind::EventRegistry,
                                event_ref: event_type,
                                // Event-subprocess event-registry correlation is
                                // not computed at runtime yet (P93 scope note).
                                configuration: None,
                            },
                        Some(process_definition_id),
                    );
                }
                continue;
            }

            for event_def in &start_event.event.event_definitions {
                match event_def {
                    EventDefinitionEnum::TimerEventDefinition(timer_def) => {
                        // Skip if already registered
                        if existing_timer_subs
                            .iter()
                            .any(|s| s.event_subprocess_id == event_sub_id)
                        {
                            continue;
                        }

                        let sub_id = Uuid::new_v4().to_string();
                        // P6-B: resolve against parent-chain merged variables for
                        // expression categories (forked child maps may be empty).
                        let category = resolve_job_category(
                            &start_event.event.flow_node.flow_element.base_element,
                            &evaluation_execution,
                        )?;
                        let schedule = crate::bpmn::timer_util::resolve_timer_schedule(
                            timer_def.time_date.as_ref(),
                            timer_def.time_duration.as_ref(),
                            timer_def.time_cycle.as_ref(),
                            timer_def.end_date.as_ref(),
                            timer_def.calendar_name.as_ref(),
                            &evaluation_execution,
                            &command_context.config.business_calendar_registry,
                            now,
                        )?;
                        command_context
                            .runtime_store
                            .insert_event_subprocess_timer_subscription(
                                EventSubprocessTimerSubscription {
                                    subscription_id: sub_id,
                                    process_instance_id: process_instance_id.to_string(),
                                    event_subprocess_id: event_sub_id.clone(),
                                    start_event_id: start_event_id.clone(),
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
                    EventDefinitionEnum::MessageEventDefinition(msg_def) => {
                        if let Some(ref msg_ref) = msg_def.message_ref {
                            // Skip if already registered
                            if existing_event_subs.iter().any(|s| {
                                s.event_subprocess_id == event_sub_id
                                    && s.event_kind == EventSubscriptionKind::Message
                                    && s.event_ref == *msg_ref
                            }) {
                                continue;
                            }

                            let sub_id = Uuid::new_v4().to_string();
                            // P134/P125: Java ProcessInstanceHelper.java:343-358 —
                            // ACTIVITY_MESSAGE/SIGNAL_WAITING on message/signal register.
                            crate::engine::event_dispatcher::insert_event_subprocess_subscription_with_waiting(
                                command_context,
                                    crate::persistence::runtime_store::EventSubprocessEventSubscription {
                                        subscription_id: sub_id,
                                        process_instance_id: process_instance_id.to_string(),
                                        scope_execution_id: Some(process_instance_id.to_string()),
                                        scope_activity_id: None,
                                        event_subprocess_id: event_sub_id.clone(),
                                        start_event_id: start_event_id.clone(),
                                        interrupting: start_event.interrupting,
                                        event_kind: EventSubscriptionKind::Message,
                                        event_ref: msg_ref.clone(),
                                        configuration: None,
                                    },
                                Some(process_definition_id),
                            );
                        }
                    }
                    EventDefinitionEnum::SignalEventDefinition(sig_def) => {
                        if let Some(ref sig_ref) = sig_def.signal_ref {
                            // Skip if already registered
                            if existing_event_subs.iter().any(|s| {
                                s.event_subprocess_id == event_sub_id
                                    && s.event_kind == EventSubscriptionKind::Signal
                                    && s.event_ref == *sig_ref
                            }) {
                                continue;
                            }

                            let sub_id = Uuid::new_v4().to_string();
                            // P134/P125: Java ProcessInstanceHelper.java:343-358 —
                            // ACTIVITY_MESSAGE/SIGNAL_WAITING on message/signal register.
                            crate::engine::event_dispatcher::insert_event_subprocess_subscription_with_waiting(
                                command_context,
                                    crate::persistence::runtime_store::EventSubprocessEventSubscription {
                                        subscription_id: sub_id,
                                        process_instance_id: process_instance_id.to_string(),
                                        scope_execution_id: Some(process_instance_id.to_string()),
                                        scope_activity_id: None,
                                        event_subprocess_id: event_sub_id.clone(),
                                        start_event_id: start_event_id.clone(),
                                        interrupting: start_event.interrupting,
                                        event_kind: EventSubscriptionKind::Signal,
                                        event_ref: sig_ref.clone(),
                                        configuration: None,
                                    },
                                Some(process_definition_id),
                            );
                        }
                    }
                    EventDefinitionEnum::EscalationEventDefinition(escalation_def) => {
                        let escalation_ref =
                            resolve_escalation_event_ref(escalation_def, Some(bpmn_model.as_ref()));

                        if existing_event_subs.iter().any(|s| {
                            s.event_subprocess_id == event_sub_id
                                && s.event_kind == EventSubscriptionKind::Escalation
                                && s.event_ref == escalation_ref
                        }) {
                            continue;
                        }

                        let sub_id = Uuid::new_v4().to_string();
                        // P134/P125: Java ProcessInstanceHelper.java:343-358 —
                        // ACTIVITY_MESSAGE/SIGNAL_WAITING on message/signal register.
                        crate::engine::event_dispatcher::insert_event_subprocess_subscription_with_waiting(
                            command_context,
                            crate::persistence::runtime_store::EventSubprocessEventSubscription {
                                subscription_id: sub_id,
                                process_instance_id: process_instance_id.to_string(),
                                scope_execution_id: Some(process_instance_id.to_string()),
                                scope_activity_id: None,
                                event_subprocess_id: event_sub_id.clone(),
                                start_event_id: start_event_id.clone(),
                                interrupting: start_event.interrupting,
                                event_kind: EventSubscriptionKind::Escalation,
                                event_ref: escalation_ref.clone(),
                                configuration: None,
                            },
                            Some(process_definition_id),
                        );
                    }
                    EventDefinitionEnum::ErrorEventDefinition(error_def) => {
                        let error_ref =
                            resolve_error_event_ref(error_def, Some(bpmn_model.as_ref()));

                        if existing_event_subs.iter().any(|s| {
                            s.event_subprocess_id == event_sub_id
                                && s.event_kind == EventSubscriptionKind::Error
                                && s.event_ref == error_ref
                        }) {
                            continue;
                        }

                        let sub_id = Uuid::new_v4().to_string();
                        let subscription =
                            crate::persistence::runtime_store::EventSubprocessEventSubscription {
                                subscription_id: sub_id,
                                process_instance_id: process_instance_id.to_string(),
                                scope_execution_id: Some(process_instance_id.to_string()),
                                scope_activity_id: None,
                                event_subprocess_id: event_sub_id.clone(),
                                start_event_id: start_event_id.clone(),
                                interrupting: start_event.interrupting,
                                event_kind: EventSubscriptionKind::Error,
                                event_ref: error_ref.clone(),
                                configuration: None,
                            };
                        // P134/P125: Java ProcessInstanceHelper.java:343-358 —
                        // ACTIVITY_MESSAGE/SIGNAL_WAITING on message/signal register.
                        crate::engine::event_dispatcher::insert_event_subprocess_subscription_with_waiting(
                            command_context,
                                subscription,
                            Some(process_definition_id),
                        );
                    }
                    _ => {}
                }
            }
        }
    }
    Ok(())
}
