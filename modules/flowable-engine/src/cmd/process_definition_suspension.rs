use crate::cmd::process_instance_suspension::set_process_instance_suspension_state;
use crate::engine::event_dispatcher::{EngineEvent, EngineEventType, EntityEventData, EntityKind};
use crate::engine::repository_service::{
    PROCESS_DEFINITION_ACTIVATE_TIMER_ACTIVITY_ID, PROCESS_DEFINITION_SUSPEND_TIMER_ACTIVITY_ID,
    PROCESS_DEFINITION_TIMER_INCLUDE_INSTANCES,
};
use crate::error::FlowableError;
use crate::interceptor::command::Command;
use crate::interceptor::command_context::CommandContext;
use crate::persistence::runtime_store::RuntimeTimerJobState;
use crate::repository::process_definition::ProcessDefinition;

/// Classifies a scheduled timer job as a process-definition suspend/activate
/// action. Returns the target suspension state, or `None` when the job is not
/// a scheduled process-definition action.
///
/// Mirrors Java `TimerSuspendProcessDefinitionHandler` /
/// `TimerActivateProcessDefinitionHandler` dispatch by activity id.
pub(crate) fn scheduled_process_definition_suspended(job: &RuntimeTimerJobState) -> Option<bool> {
    match job.activity_id.as_str() {
        PROCESS_DEFINITION_SUSPEND_TIMER_ACTIVITY_ID => Some(true),
        PROCESS_DEFINITION_ACTIVATE_TIMER_ACTIVITY_ID => Some(false),
        _ => None,
    }
}

/// Shared command executing a scheduled process-definition suspend/activate
/// timer inside a single [`CommandContext`]. Both the manual
/// `RuntimeService::execute_timer_job_by_id` path and the real timer worker
/// (`ExecuteTimerWorkCmd`) dispatch through this command so there is exactly
/// one implementation of the transactional semantics:
///
///   1. update the definition suspension state;
///   2. when `include-process-instances` is set, migrate only the instances
///      (and their executions/tasks/jobs) currently in the opposite state;
///   3. on success, delete the scheduling timer.
///
/// Any failure rolls the whole unit back — definition, instances, executions,
/// tasks, jobs and the scheduling timer are restored together — because every
/// mutation happens on the same command session.
///
/// Mirrors Java `AbstractSetProcessDefinitionStateCmd` delayed-action handling
/// and `TimerSuspendProcessDefinitionHandler`.
pub(crate) struct ExecuteScheduledProcessDefinitionActionCmd {
    timer_job: RuntimeTimerJobState,
    suspended: bool,
}

impl ExecuteScheduledProcessDefinitionActionCmd {
    pub(crate) fn new(timer_job: RuntimeTimerJobState, suspended: bool) -> Self {
        Self {
            timer_job,
            suspended,
        }
    }
}

impl Command<()> for ExecuteScheduledProcessDefinitionActionCmd {
    fn execute(&self, command_context: &mut CommandContext) -> Result<(), FlowableError> {
        // For scheduled process-definition timers the definition id is carried
        // in `execution_id` (see `RepositoryService::schedule_process_definition_suspended`).
        let process_definition_id = self.timer_job.execution_id.clone();
        let include_process_instances = self.timer_job.attached_activity_id.as_deref()
            == Some(PROCESS_DEFINITION_TIMER_INCLUDE_INSTANCES);

        set_process_definition_suspension_state(
            command_context,
            &process_definition_id,
            self.suspended,
            include_process_instances,
        )?;

        let (store, session) = command_context.store_and_session();
        store.delete_timer_job_state(&self.timer_job.timer_job_id, session);
        Ok(())
    }
}

pub(crate) fn set_process_definition_suspension_state(
    command_context: &mut CommandContext,
    process_definition_id: &str,
    suspended: bool,
    include_process_instances: bool,
) -> Result<ProcessDefinition, FlowableError> {
    let deployment_manager = command_context.deployment_manager_handle();
    let mut definition = deployment_manager
        .get_process_definitions(&mut command_context.session)
        .remove(process_definition_id)
        .ok_or_else(|| {
            FlowableError::NotFound(format!(
                "Process definition '{}' was not found",
                process_definition_id
            ))
        })?;
    if definition.is_suspended == suspended {
        let state = if suspended { "suspended" } else { "active" };
        return Err(FlowableError::ExecutionError(format!(
            "Cannot set suspension state '{}' for process definition '{}': already in state '{}'.",
            state, process_definition_id, state
        )));
    }

    definition.is_suspended = suspended;
    deployment_manager
        .update_process_definition(definition.clone(), &mut command_context.session)
        .ok_or_else(|| {
            FlowableError::NotFound(format!(
                "Process definition '{}' was not found",
                process_definition_id
            ))
        })?;

    let event_type = if suspended {
        EngineEventType::EntitySuspended
    } else {
        EngineEventType::EntityActivated
    };
    command_context.add_post_agenda_event(EngineEvent::Entity {
        event_type,
        data: EntityEventData {
            entity_kind: EntityKind::ProcessDefinition,
            entity_id: process_definition_id.to_string(),
            process_instance_id: None,
            execution_id: None,
            process_definition_id: Some(process_definition_id.to_string()),
            scope_type: None,
            scope_id: None,
            sub_scope_id: None,
        },
    });

    if include_process_instances {
        let store = command_context.runtime_store_handle();
        // Java `AbstractSetProcessDefinitionStateCmd.fetchProcessInstancesPage`
        // selects only instances in the opposite state: active instances when
        // suspending and suspended instances when activating. Instances that
        // already match the target state are intentionally left untouched.
        let process_instances = store
            .snapshot_process_instances(&mut command_context.session)
            .into_values()
            .filter(|instance| {
                instance.process_definition_id == process_definition_id
                    && instance.is_suspended != suspended
            })
            .collect::<Vec<_>>();
        for process_instance in process_instances {
            set_process_instance_suspension_state(command_context, process_instance, suspended)?;
        }
    }
    Ok(definition)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::process_engine::ProcessEngine;
    use crate::interceptor::command::Command;
    use crate::interceptor::command_executor::CommandExecutor;
    use crate::persistence::runtime_store::RuntimeTimerJobState;
    use crate::runtime::process_instance::ProcessInstance;

    struct SuspendDefinitionThenFailCmd;

    impl Command<()> for SuspendDefinitionThenFailCmd {
        fn execute(&self, command_context: &mut CommandContext) -> Result<(), FlowableError> {
            set_process_definition_suspension_state(command_context, "definition-1", true, true)?;
            Err(FlowableError::ExecutionError(
                "forced rollback after definition suspension".to_string(),
            ))
        }
    }

    #[test]
    fn definition_instance_and_job_suspension_roll_back_together() {
        let engine = ProcessEngine::new("definition-suspension-rollback".to_string());
        let executor = engine.get_command_executor();
        let deployment_manager = executor.deployment_manager();
        let mut session = deployment_manager.create_session().unwrap();
        deployment_manager.insert_process_definition(definition(), &mut session);
        session.flush_and_commit().unwrap();

        let store = engine.get_runtime_store();
        let mut session = store.create_session().unwrap();
        store.insert_process_instance(&process_instance(), &mut session);
        store.insert_timer_job_state(&timer_job(), &mut session);
        session.flush_and_commit().unwrap();

        let error = executor
            .execute(&SuspendDefinitionThenFailCmd)
            .expect_err("outer command should force rollback");
        assert_eq!(
            error.to_string(),
            "Execution error: forced rollback after definition suspension"
        );

        assert!(
            !engine
                .get_repository_service()
                .get_process_definition("definition-1")
                .expect("definition should remain")
                .is_suspended
        );
        let mut session = store.create_session().unwrap();
        assert!(
            !store
                .find_process_instance("process-1", &mut session)
                .expect("process should remain")
                .is_suspended
        );
        let job = store
            .find_timer_job_state("timer-1", &mut session)
            .expect("timer should remain");
        assert_eq!(job.job_state.as_deref(), Some("timer"));
        assert_eq!(job.lock_owner.as_deref(), Some("old-owner"));
        assert!(store.find_timer_job_type("timer-1", &mut session).is_none());
        session.rollback().unwrap();
    }

    fn definition() -> ProcessDefinition {
        ProcessDefinition {
            id: "definition-1".to_string(),
            category: None,
            name: Some("Definition".to_string()),
            key: "definition".to_string(),
            description: None,
            version: 1,
            resource_name: None,
            deployment_id: None,
            diagram_resource_name: None,
            has_start_form_key: false,
            has_graphical_notation: false,
            is_suspended: false,
            tenant_id: None,
            engine_version: None,
            app_version: None,
        history_level: None,
        }
    }

    fn process_instance() -> ProcessInstance {
        ProcessInstance {
            id: "process-1".to_string(),
            name: None,
            process_definition_id: "definition-1".to_string(),
            process_definition_key: "definition".to_string(),
            process_definition_name: None,
            process_definition_version: 1,
            business_key: None,
            business_status: None,
            is_suspended: false,
            tenant_id: None,
            start_time: None,
            start_user_id: None,
            callback_id: None,
            callback_type: None,
            reference_id: None,
            reference_type: None,
            is_ended: false,
            super_execution_id: None,
            root_process_instance_id: Some("process-1".to_string()),
        }
    }

    fn timer_job() -> RuntimeTimerJobState {
        RuntimeTimerJobState {
            timer_job_id: "timer-1".to_string(),
            process_instance_id: "process-1".to_string(),
            execution_id: "execution-1".to_string(),
            activity_id: "timer".to_string(),
            job_state: Some("timer".to_string()),
            is_boundary: false,
            attached_activity_id: None,
            cancel_activity: false,
            time_duration: None,
            time_date: None,
            time_cycle: None,
            end_date: None,
            due_time: Some(1),
            lock_owner: Some("old-owner".to_string()),
            lock_time: Some(1),
            lock_expiration_time: Some(2),
            retries: Some(1),
            error_message: None,
            error_details: None,
            category: None,
            ..Default::default()
        }
    }
}
