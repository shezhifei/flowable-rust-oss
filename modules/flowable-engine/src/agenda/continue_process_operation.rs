use crate::agenda::{AgendaOperation, FlowableEngineAgenda};
use crate::bpmn::job_category::{flow_element_base_element, resolve_job_category};
use crate::interceptor::command_context::CommandContext;
use crate::persistence::runtime_store::{
    RuntimeTimerJobState, job_handler_types, stamp_new_job_metadata,
};
use crate::runtime::execution::Execution;
use flowable_bpmn_model::model::{FlowElementEnum, Process};
use uuid::Uuid;

pub(crate) const ASYNC_CONTINUATION_JOB_STATE: &str = "async";
pub(crate) const ASYNC_CONTINUATION_JOB_TYPE_MARKER: &str = "__flowable_async_continuation";
pub(crate) const ASYNC_CONTINUATION_RESUME_FLAG: &str = "__flowable_async_continuation_resume";
pub(crate) const ASYNC_AFTER_JOB_STATE: &str = "async-after";
pub(crate) const ASYNC_AFTER_JOB_TYPE_MARKER: &str = "__flowable_async_after";
pub(crate) const ASYNC_AFTER_RESUME_FLAG: &str = "__flowable_async_after_resume";

pub struct ContinueProcessOperation {
    execution: Execution,
}

impl ContinueProcessOperation {
    pub fn new(execution: Execution) -> Self {
        Self { execution }
    }
}

pub(crate) fn flow_element_id(flow_element: &FlowElementEnum) -> Option<&str> {
    match flow_element {
        FlowElementEnum::SequenceFlow(flow_flow) => {
            flow_flow.flow_element.base_element.id.as_deref()
        }
        FlowElementEnum::Task(task) => task
            .activity
            .flow_node
            .flow_element
            .base_element
            .id
            .as_deref(),
        FlowElementEnum::UserTask(task) => task
            .task
            .activity
            .flow_node
            .flow_element
            .base_element
            .id
            .as_deref(),
        FlowElementEnum::ServiceTask(task) => task
            .task
            .activity
            .flow_node
            .flow_element
            .base_element
            .id
            .as_deref(),
        FlowElementEnum::CaseServiceTask(task) => task
            .service_task
            .task
            .activity
            .flow_node
            .flow_element
            .base_element
            .id
            .as_deref(),
        FlowElementEnum::SendTask(task) => task
            .service_task
            .task
            .activity
            .flow_node
            .flow_element
            .base_element
            .id
            .as_deref(),
        FlowElementEnum::ScriptTask(task) => task
            .task
            .activity
            .flow_node
            .flow_element
            .base_element
            .id
            .as_deref(),
        FlowElementEnum::ManualTask(task) => task
            .task
            .activity
            .flow_node
            .flow_element
            .base_element
            .id
            .as_deref(),
        FlowElementEnum::ReceiveTask(task) => task
            .task
            .activity
            .flow_node
            .flow_element
            .base_element
            .id
            .as_deref(),
        FlowElementEnum::BusinessRuleTask(task) => task
            .task
            .activity
            .flow_node
            .flow_element
            .base_element
            .id
            .as_deref(),
        FlowElementEnum::StartEvent(event) => event
            .event
            .flow_node
            .flow_element
            .base_element
            .id
            .as_deref(),
        FlowElementEnum::EndEvent(event) => event
            .event
            .flow_node
            .flow_element
            .base_element
            .id
            .as_deref(),
        FlowElementEnum::ExclusiveGateway(gateway) => gateway
            .gateway
            .flow_node
            .flow_element
            .base_element
            .id
            .as_deref(),
        FlowElementEnum::ParallelGateway(gateway) => gateway
            .gateway
            .flow_node
            .flow_element
            .base_element
            .id
            .as_deref(),
        FlowElementEnum::InclusiveGateway(gateway) => gateway
            .gateway
            .flow_node
            .flow_element
            .base_element
            .id
            .as_deref(),
        FlowElementEnum::EventBasedGateway(gateway) => gateway
            .gateway
            .flow_node
            .flow_element
            .base_element
            .id
            .as_deref(),
        FlowElementEnum::ComplexGateway(gateway) => gateway
            .gateway
            .flow_node
            .flow_element
            .base_element
            .id
            .as_deref(),
        FlowElementEnum::IntermediateCatchEvent(event) => event
            .event
            .flow_node
            .flow_element
            .base_element
            .id
            .as_deref(),
        FlowElementEnum::IntermediateThrowEvent(event) => event
            .event
            .flow_node
            .flow_element
            .base_element
            .id
            .as_deref(),
        FlowElementEnum::SubProcess(sub_process) => sub_process
            .activity
            .flow_node
            .flow_element
            .base_element
            .id
            .as_deref(),
        FlowElementEnum::Transaction(transaction) => transaction
            .sub_process
            .activity
            .flow_node
            .flow_element
            .base_element
            .id
            .as_deref(),
        FlowElementEnum::EventSubProcess(event_sub_process) => event_sub_process
            .sub_process
            .activity
            .flow_node
            .flow_element
            .base_element
            .id
            .as_deref(),
        FlowElementEnum::AdhocSubProcess(sub_process) => sub_process
            .sub_process
            .activity
            .flow_node
            .flow_element
            .base_element
            .id
            .as_deref(),
        FlowElementEnum::CallActivity(call_activity) => call_activity
            .activity
            .flow_node
            .flow_element
            .base_element
            .id
            .as_deref(),
        FlowElementEnum::BoundaryEvent(boundary_event) => boundary_event
            .event
            .flow_node
            .flow_element
            .base_element
            .id
            .as_deref(),
        FlowElementEnum::ValuedDataObject(data_object) => data_object.base_element.id.as_deref(),
    }
}

pub(crate) fn flow_element_type(flow_element: &FlowElementEnum) -> &'static str {
    match flow_element {
        FlowElementEnum::SequenceFlow(_) => "SequenceFlow",
        FlowElementEnum::Task(_) => "Task",
        FlowElementEnum::UserTask(_) => "UserTask",
        FlowElementEnum::ServiceTask(_) => "ServiceTask",
        FlowElementEnum::CaseServiceTask(_) => "CaseServiceTask",
        FlowElementEnum::SendTask(_) => "SendTask",
        FlowElementEnum::ScriptTask(_) => "ScriptTask",
        FlowElementEnum::ManualTask(_) => "ManualTask",
        FlowElementEnum::ReceiveTask(_) => "ReceiveTask",
        FlowElementEnum::BusinessRuleTask(_) => "BusinessRuleTask",
        FlowElementEnum::StartEvent(_) => "StartEvent",
        FlowElementEnum::EndEvent(_) => "EndEvent",
        FlowElementEnum::ExclusiveGateway(_) => "ExclusiveGateway",
        FlowElementEnum::ParallelGateway(_) => "ParallelGateway",
        FlowElementEnum::InclusiveGateway(_) => "InclusiveGateway",
        FlowElementEnum::EventBasedGateway(_) => "EventBasedGateway",
        FlowElementEnum::ComplexGateway(_) => "ComplexGateway",
        FlowElementEnum::IntermediateCatchEvent(_) => "IntermediateCatchEvent",
        FlowElementEnum::IntermediateThrowEvent(_) => "IntermediateThrowEvent",
        FlowElementEnum::SubProcess(_) => "SubProcess",
        FlowElementEnum::Transaction(_) => "Transaction",
        FlowElementEnum::EventSubProcess(_) => "EventSubProcess",
        FlowElementEnum::AdhocSubProcess(_) => "AdhocSubProcess",
        FlowElementEnum::CallActivity(_) => "CallActivity",
        FlowElementEnum::BoundaryEvent(_) => "BoundaryEvent",
        FlowElementEnum::ValuedDataObject(_) => "ValuedDataObject",
    }
}

pub fn find_flow_element<'a>(
    process: &'a Process,
    activity_id: &str,
) -> Option<&'a FlowElementEnum> {
    process.flow_element_map.get(activity_id).or_else(|| {
        process
            .flow_elements
            .iter()
            .find(|flow_element| flow_element_id(flow_element) == Some(activity_id))
    })
}

/// Java `ContinueProcessOperation.java:106-108`: a start event counts as the
/// process's initial flow element only when it is a *direct* child of the main
/// process (0 incoming flows by BPMN definition, `subProcess == null`). Start
/// events nested inside (event) subprocesses must not fire process-level start
/// listeners.
fn is_process_level_start_event(process: &Process, activity_id: &str) -> bool {
    process
        .flow_elements
        .iter()
        .any(|element| matches!(element, FlowElementEnum::StartEvent(_))
            && flow_element_id(element) == Some(activity_id))
}

fn is_async_before(flow_element: &FlowElementEnum) -> bool {
    match flow_element {
        FlowElementEnum::Task(task) => task.activity.flow_node.asynchronous,
        FlowElementEnum::UserTask(task) => task.task.activity.flow_node.asynchronous,
        FlowElementEnum::ServiceTask(task) => task.task.activity.flow_node.asynchronous,
        FlowElementEnum::CaseServiceTask(task) => task.service_task.task.activity.flow_node.asynchronous,
        FlowElementEnum::ScriptTask(task) => task.task.activity.flow_node.asynchronous,
        FlowElementEnum::ManualTask(task) => task.task.activity.flow_node.asynchronous,
        FlowElementEnum::ReceiveTask(task) => task.task.activity.flow_node.asynchronous,
        FlowElementEnum::BusinessRuleTask(task) => task.task.activity.flow_node.asynchronous,
        FlowElementEnum::CallActivity(activity) => activity.activity.flow_node.asynchronous,
        FlowElementEnum::SubProcess(sub_process) => sub_process.activity.flow_node.asynchronous,
        FlowElementEnum::Transaction(transaction) => {
            transaction.sub_process.activity.flow_node.asynchronous
        }
        FlowElementEnum::EventSubProcess(sub_process) => {
            sub_process.sub_process.activity.flow_node.asynchronous
        }
        FlowElementEnum::AdhocSubProcess(sub_process) => {
            sub_process.sub_process.activity.flow_node.asynchronous
        }
        _ => false,
    }
}

fn is_async_after(flow_element: &FlowElementEnum) -> bool {
    match flow_element {
        FlowElementEnum::Task(task) => task.activity.flow_node.asynchronous_leave,
        FlowElementEnum::UserTask(task) => task.task.activity.flow_node.asynchronous_leave,
        FlowElementEnum::ServiceTask(task) => task.task.activity.flow_node.asynchronous_leave,
        FlowElementEnum::CaseServiceTask(task) => task.service_task.task.activity.flow_node.asynchronous_leave,
        FlowElementEnum::ScriptTask(task) => task.task.activity.flow_node.asynchronous_leave,
        FlowElementEnum::ManualTask(task) => task.task.activity.flow_node.asynchronous_leave,
        FlowElementEnum::ReceiveTask(task) => task.task.activity.flow_node.asynchronous_leave,
        FlowElementEnum::BusinessRuleTask(task) => task.task.activity.flow_node.asynchronous_leave,
        FlowElementEnum::CallActivity(activity) => activity.activity.flow_node.asynchronous_leave,
        FlowElementEnum::SubProcess(sub_process) => {
            sub_process.activity.flow_node.asynchronous_leave
        }
        FlowElementEnum::Transaction(transaction) => {
            transaction
                .sub_process
                .activity
                .flow_node
                .asynchronous_leave
        }
        FlowElementEnum::EventSubProcess(sub_process) => {
            sub_process
                .sub_process
                .activity
                .flow_node
                .asynchronous_leave
        }
        FlowElementEnum::AdhocSubProcess(sub_process) => {
            sub_process
                .sub_process
                .activity
                .flow_node
                .asynchronous_leave
        }
        _ => false,
    }
}

/// Java `ContinueProcessOperation.java:190`: `createAsyncJob(job, flowNode.isExclusive())`.
/// Mirrors the converter default of `true` for elements without an explicit
/// `flowable:exclusive="false"` attribute (AbstractJobEntityImpl.DEFAULT_EXCLUSIVE).
fn is_exclusive(flow_element: &FlowElementEnum) -> bool {
    match flow_element {
        FlowElementEnum::Task(task) => task.activity.flow_node.exclusive,
        FlowElementEnum::UserTask(task) => task.task.activity.flow_node.exclusive,
        FlowElementEnum::ServiceTask(task) => task.task.activity.flow_node.exclusive,
        FlowElementEnum::CaseServiceTask(task) => task.service_task.task.activity.flow_node.exclusive,
        FlowElementEnum::ScriptTask(task) => task.task.activity.flow_node.exclusive,
        FlowElementEnum::ManualTask(task) => task.task.activity.flow_node.exclusive,
        FlowElementEnum::ReceiveTask(task) => task.task.activity.flow_node.exclusive,
        FlowElementEnum::BusinessRuleTask(task) => task.task.activity.flow_node.exclusive,
        FlowElementEnum::CallActivity(activity) => activity.activity.flow_node.exclusive,
        FlowElementEnum::SubProcess(sub_process) => sub_process.activity.flow_node.exclusive,
        FlowElementEnum::Transaction(transaction) => {
            transaction.sub_process.activity.flow_node.exclusive
        }
        FlowElementEnum::EventSubProcess(sub_process) => {
            sub_process.sub_process.activity.flow_node.exclusive
        }
        FlowElementEnum::AdhocSubProcess(sub_process) => {
            sub_process.sub_process.activity.flow_node.exclusive
        }
        _ => true,
    }
}

/// Java `TakeOutgoingSequenceFlowsOperation.java:143`:
/// `createAsyncJob(job, flowNode.isAsynchronousLeaveExclusive())`.
fn is_async_leave_exclusive(flow_element: &FlowElementEnum) -> bool {
    match flow_element {
        FlowElementEnum::Task(task) => task.activity.flow_node.asynchronous_leave_exclusive,
        FlowElementEnum::UserTask(task) => {
            task.task.activity.flow_node.asynchronous_leave_exclusive
        }
        FlowElementEnum::ServiceTask(task) => {
            task.task.activity.flow_node.asynchronous_leave_exclusive
        }
        FlowElementEnum::CaseServiceTask(task) => {
            task.service_task.task.activity.flow_node.asynchronous_leave_exclusive
        }
        FlowElementEnum::ScriptTask(task) => {
            task.task.activity.flow_node.asynchronous_leave_exclusive
        }
        FlowElementEnum::ManualTask(task) => {
            task.task.activity.flow_node.asynchronous_leave_exclusive
        }
        FlowElementEnum::ReceiveTask(task) => {
            task.task.activity.flow_node.asynchronous_leave_exclusive
        }
        FlowElementEnum::BusinessRuleTask(task) => {
            task.task.activity.flow_node.asynchronous_leave_exclusive
        }
        FlowElementEnum::CallActivity(activity) => {
            activity.activity.flow_node.asynchronous_leave_exclusive
        }
        FlowElementEnum::SubProcess(sub_process) => {
            sub_process.activity.flow_node.asynchronous_leave_exclusive
        }
        FlowElementEnum::Transaction(transaction) => {
            transaction
                .sub_process
                .activity
                .flow_node
                .asynchronous_leave_exclusive
        }
        FlowElementEnum::EventSubProcess(sub_process) => {
            sub_process
                .sub_process
                .activity
                .flow_node
                .asynchronous_leave_exclusive
        }
        FlowElementEnum::AdhocSubProcess(sub_process) => {
            sub_process
                .sub_process
                .activity
                .flow_node
                .asynchronous_leave_exclusive
        }
        _ => true,
    }
}

fn failed_job_retry_time_cycle_value(flow_element: &FlowElementEnum) -> Option<&str> {
    match flow_element {
        FlowElementEnum::Task(task) => task.activity.failed_job_retry_time_cycle_value.as_deref(),
        FlowElementEnum::UserTask(task) => task
            .task
            .activity
            .failed_job_retry_time_cycle_value
            .as_deref(),
        FlowElementEnum::ServiceTask(task) => task
            .task
            .activity
            .failed_job_retry_time_cycle_value
            .as_deref(),
        FlowElementEnum::CaseServiceTask(task) => task
            .service_task
            .task
            .activity
            .failed_job_retry_time_cycle_value
            .as_deref(),
        FlowElementEnum::ScriptTask(task) => task
            .task
            .activity
            .failed_job_retry_time_cycle_value
            .as_deref(),
        FlowElementEnum::ManualTask(task) => task
            .task
            .activity
            .failed_job_retry_time_cycle_value
            .as_deref(),
        FlowElementEnum::ReceiveTask(task) => task
            .task
            .activity
            .failed_job_retry_time_cycle_value
            .as_deref(),
        FlowElementEnum::BusinessRuleTask(task) => task
            .task
            .activity
            .failed_job_retry_time_cycle_value
            .as_deref(),
        FlowElementEnum::CallActivity(activity) => activity
            .activity
            .failed_job_retry_time_cycle_value
            .as_deref(),
        FlowElementEnum::SubProcess(sub_process) => sub_process
            .activity
            .failed_job_retry_time_cycle_value
            .as_deref(),
        FlowElementEnum::Transaction(transaction) => transaction
            .sub_process
            .activity
            .failed_job_retry_time_cycle_value
            .as_deref(),
        FlowElementEnum::EventSubProcess(sub_process) => sub_process
            .sub_process
            .activity
            .failed_job_retry_time_cycle_value
            .as_deref(),
        FlowElementEnum::AdhocSubProcess(sub_process) => sub_process
            .sub_process
            .activity
            .failed_job_retry_time_cycle_value
            .as_deref(),
        _ => None,
    }
}

fn is_resuming_async_continuation(execution: &Execution) -> bool {
    execution
        .transient_variables
        .get(ASYNC_CONTINUATION_RESUME_FLAG)
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
}

fn is_resuming_async_after(execution: &Execution) -> bool {
    execution
        .transient_variables
        .get(ASYNC_AFTER_RESUME_FLAG)
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
}

fn schedule_async_continuation_job(
    execution: &mut Execution,
    flow_element: &FlowElementEnum,
    command_context: &mut CommandContext,
) {
    execution.is_active = false;
    execution.is_ended = false;
    command_context
        .execution_entity_manager
        .update(execution, &mut command_context.session);

    // P6-B: job_category expression must walk the parent scope chain. The
    // forked child execution's variable maps are empty (P4-7b), so we
    // evaluate against `evaluation_execution` which merges the parent chain
    // and the PI scope row.
    let evaluation_execution =
        crate::engine::variable_service::evaluation_execution(command_context, execution);
    let process_instance_id = execution
        .process_instance_id
        .clone()
        .unwrap_or_else(|| execution.id.clone());
    let store = command_context.runtime_store_handle();
    let due_time = store.time_source().now().timestamp_millis();
    let mut job = RuntimeTimerJobState {
        timer_job_id: Uuid::new_v4().to_string(),
        process_instance_id,
        execution_id: execution.id.clone(),
        activity_id: execution.activity_id.clone().unwrap_or_default(),
        job_state: Some(ASYNC_CONTINUATION_JOB_STATE.to_string()),
        is_boundary: false,
        attached_activity_id: None,
        cancel_activity: true,
        time_duration: Some(ASYNC_CONTINUATION_JOB_TYPE_MARKER.to_string()),
        time_date: None,
        time_cycle: failed_job_retry_time_cycle_value(flow_element).map(str::to_string),
        end_date: None,
        due_time: Some(due_time),
        lock_owner: None,
        lock_time: None,
        lock_expiration_time: None,
        retries: Some(
            command_context
                .config
                .async_executor
                .number_of_retries
                .max(0),
        ),
        error_message: None,
        error_details: None,
        category: resolve_job_category(
            flow_element_base_element(flow_element),
            &evaluation_execution,
        ),
        // Java ContinueProcessOperation.java:190: createAsyncJob(job, flowNode.isExclusive()).
        exclusive: is_exclusive(flow_element),
        ..Default::default()
    };
    stamp_new_job_metadata(
        &mut job,
        due_time,
        job_handler_types::ASYNC_CONTINUATION,
        execution.tenant_id.clone(),
        execution.process_definition_id.clone(),
        execution.activity_name.clone(),
    );
    store.insert_timer_job_state(&job, &mut command_context.session);
}

fn schedule_async_after_job(
    execution: &mut Execution,
    flow_element: &FlowElementEnum,
    command_context: &mut CommandContext,
) {
    execution.is_active = false;
    execution.is_ended = false;
    command_context
        .execution_entity_manager
        .update(execution, &mut command_context.session);

    // P6-B: job_category expression must walk the parent scope chain (see
    // schedule_async_continuation_job for rationale).
    let evaluation_execution =
        crate::engine::variable_service::evaluation_execution(command_context, execution);
    let process_instance_id = execution
        .process_instance_id
        .clone()
        .unwrap_or_else(|| execution.id.clone());
    let store = command_context.runtime_store_handle();
    let due_time = store.time_source().now().timestamp_millis();
    let mut job = RuntimeTimerJobState {
        timer_job_id: Uuid::new_v4().to_string(),
        process_instance_id,
        execution_id: execution.id.clone(),
        activity_id: execution.activity_id.clone().unwrap_or_default(),
        job_state: Some(ASYNC_AFTER_JOB_STATE.to_string()),
        is_boundary: false,
        attached_activity_id: None,
        cancel_activity: true,
        time_duration: Some(ASYNC_AFTER_JOB_TYPE_MARKER.to_string()),
        time_date: None,
        time_cycle: failed_job_retry_time_cycle_value(flow_element).map(str::to_string),
        end_date: None,
        due_time: Some(due_time),
        lock_owner: None,
        lock_time: None,
        lock_expiration_time: None,
        retries: Some(
            command_context
                .config
                .async_executor
                .number_of_retries
                .max(0),
        ),
        error_message: None,
        error_details: None,
        category: resolve_job_category(
            flow_element_base_element(flow_element),
            &evaluation_execution,
        ),
        // Java TakeOutgoingSequenceFlowsOperation.java:143:
        // createAsyncJob(job, flowNode.isAsynchronousLeaveExclusive()).
        exclusive: is_async_leave_exclusive(flow_element),
        ..Default::default()
    };
    stamp_new_job_metadata(
        &mut job,
        due_time,
        job_handler_types::ASYNC_AFTER,
        execution.tenant_id.clone(),
        execution.process_definition_id.clone(),
        execution.activity_name.clone(),
    );
    store.insert_timer_job_state(&job, &mut command_context.session);
}

impl AgendaOperation for ContinueProcessOperation {
    fn run(&self, command_context: &mut CommandContext) -> Result<(), crate::error::FlowableError> {
        // Skip if the execution was deleted while this op was queued (e.g. ad-hoc
        // cancelRemainingInstances ended the scope after leave already planned
        // continue — Java EndExecutionOperation deletes children before those
        // continues can create new work). Using only the in-memory clone would
        // resurrect cancelled siblings.
        if command_context
            .runtime_store
            .find_execution(&self.execution.id, &mut command_context.session)
            .is_none()
        {
            return Ok(());
        }

        let mut execution = self.execution.clone();
        execution.is_ended = false;
        execution.is_active = true;

        if let Some(ref activity_id) = execution.activity_id {
            let activity_id_owned = activity_id.clone();
            let process_definition_id = match execution.process_definition_id.as_deref() {
                Some(id) => id,
                None => {
                    return Ok(());
                }
            };

            let dm = command_context.deployment_manager_handle();
            let bpmn_model = match dm.get_bpmn_model(process_definition_id) {
                Some(model) => model,
                None => {
                    return Ok(());
                }
            };

            let main_process = match bpmn_model.main_process.as_ref() {
                Some(process) => process,
                None => {
                    return Ok(());
                }
            };

            let flow_element = match find_flow_element(main_process, activity_id) {
                Some(flow_element) => flow_element,
                None => {
                    return Ok(());
                }
            };

            if is_async_before(flow_element) && !is_resuming_async_continuation(&execution) {
                schedule_async_continuation_job(&mut execution, flow_element, command_context);
                return Ok(());
            }

            if let Some(behavior) = command_context
                .behavior_factory
                .create_behavior(flow_element)
            {
                let activity_id_str = flow_element_id(flow_element).unwrap_or("<unknown>");
                let activity_type = flow_element_type(flow_element);

                // P53 layer 1 + 2: dispatch PROCESS_STARTED when execution
                // first lands on a StartEvent (Java
                // `ProcessInstanceHelper.java:302-317`). This is the
                // transition from "process instance created" to
                // "process instance started".
                if matches!(flow_element, FlowElementEnum::StartEvent(_)) {
                    if let (Some(pi_id), Some(pd_id)) = (
                        execution.process_instance_id.as_deref(),
                        execution.process_definition_id.as_deref(),
                    ) {
                        crate::engine::event_dispatcher::dispatch_process_instance_started(
                            command_context,
                            pi_id,
                            pd_id,
                        );
                    }

                    // Java `ContinueProcessOperation.java:105-111 + 96-98`:
                    // when the initial flow element of the process is entered
                    // (no incoming flows, not nested in a subprocess), the
                    // process-level execution listeners fire for `start`,
                    // before the start event's own start listeners below.
                    if is_process_level_start_event(main_process, activity_id) {
                        let process_start_listeners: Vec<_> =
                            main_process.execution_listeners.clone();
                        let process_start_evaluation_execution =
                            crate::engine::variable_service::evaluation_execution(
                                command_context,
                                &execution,
                            );
                        crate::bpmn::listener::execute_execution_listeners(
                            &mut execution,
                            command_context,
                            &process_start_listeners,
                            "start",
                            &process_start_evaluation_execution,
                        )?;
                        // Persist any process variables written by the process
                        // start listener.
                        command_context
                            .execution_entity_manager
                            .update(&execution, &mut command_context.session);
                    }
                }
                // P53 layer 2 / P119: ACTIVITY_STARTED for ordinary flow nodes;
                // MULTI_INSTANCE_ACTIVITY_STARTED when the node has multi-instance
                // loop characteristics (Java `ContinueProcessOperation.java:274-285`
                // — exclusive: MI path does not also emit ACTIVITY_STARTED).
                if crate::bpmn::execution_graph_util::has_loop_characteristics(flow_element) {
                    crate::engine::event_dispatcher::dispatch_multi_instance_activity_started(
                        command_context,
                        activity_id_str,
                        activity_type,
                        execution.process_instance_id.as_deref(),
                        Some(&execution.id),
                        execution.process_definition_id.as_deref(),
                    );
                } else {
                    crate::engine::event_dispatcher::dispatch_activity_started(
                        command_context,
                        activity_id_str,
                        activity_type,
                        execution.process_instance_id.as_deref(),
                        Some(&execution.id),
                        execution.process_definition_id.as_deref(),
                    );
                }
                command_context.history_manager.record_activity_start(
                    activity_id_str,
                    None,
                    activity_type,
                    execution.process_instance_id.as_deref().unwrap_or_default(),
                    &execution.id,
                    &mut command_context.session,
                );

                // Clone listeners before mutable borrow of command_context for execute.
                let start_listeners: Vec<_> =
                    crate::bpmn::listener::flow_element_execution_listeners(flow_element).to_vec();
                let start_evaluation_execution =
                    crate::engine::variable_service::evaluation_execution(
                        command_context,
                        &execution,
                    );
                crate::bpmn::listener::execute_execution_listeners(
                    &mut execution,
                    command_context,
                    &start_listeners,
                    "start",
                    &start_evaluation_execution,
                )?;
                // Persist any process variables written by start listeners.
                command_context
                    .execution_entity_manager
                    .update(&execution, &mut command_context.session);

                let pre_execute_exec_id = execution.id.clone();
                let was_multi_instance_root = execution.is_multi_instance_root;

                behavior.execute(&mut execution, command_context)?;

                // Java parity: multi-instance root executions are containers only; they
                // must not produce historic activity instance records. The activity
                // history is recorded per MI child instance. Delete the spurious start
                // record inserted above for both first-arrival (execution id changes via
                // materialize_multi_instance_root) and continuation (execution is already
                // MI root) cases.
                if execution.id != pre_execute_exec_id
                    || execution.is_multi_instance_root
                    || was_multi_instance_root
                {
                    command_context
                        .runtime_store
                        .delete_open_historic_activity_instance(
                            &pre_execute_exec_id,
                            &activity_id_owned,
                            &mut command_context.session,
                        );
                }

                if is_async_after(flow_element)
                    && execution.is_active
                    && !execution.is_ended
                    && !is_resuming_async_after(&execution)
                {
                    command_context.agenda.clear();
                    schedule_async_after_job(&mut execution, flow_element, command_context);
                    return Ok(());
                }
            } else {
                let activity_id = flow_element_id(flow_element).unwrap_or("<unknown>");
                return Err(crate::error::FlowableError::UnsupportedElement {
                    element_type: flow_element_type(flow_element).to_string(),
                    activity_id: activity_id.to_string(),
                });
            }
        }
        Ok(())
    }
}
