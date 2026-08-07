use crate::cmd::job_suspension::{
    activate_suspended_jobs_for_process_instance, suspend_jobs_for_process_instance,
};
use crate::engine::event_dispatcher::{EngineEvent, EngineEventType, EntityEventData, EntityKind};
use crate::error::FlowableError;
use crate::interceptor::command_context::CommandContext;
use crate::runtime::process_instance::ProcessInstance;

/// Suspends or activates a process instance along with its child executions,
/// tasks, and jobs. Dispatches typed entity events in the Java-compatible order:
///   1. root process instance / execution
///   2. child executions
///   3. tasks
pub(crate) fn set_process_instance_suspension_state(
    command_context: &mut CommandContext,
    mut process_instance: ProcessInstance,
    suspended: bool,
) -> Result<ProcessInstance, FlowableError> {
    // Java parity (`SuspensionStateUtil.setSuspensionState`): reject if the
    // instance is already in the target state. Definition-level suspension
    // filters instances to the opposite state before invoking this helper, so
    // the duplicate check primarily governs direct instance operations.
    if process_instance.is_suspended == suspended {
        let state = if suspended { "suspended" } else { "active" };
        return Err(FlowableError::ExecutionError(format!(
            "Cannot set suspension state '{}' for process instance '{}': already in state '{}'.",
            state, process_instance.id, state
        )));
    }

    let store = command_context.runtime_store_handle();
    let event_type = if suspended {
        EngineEventType::EntitySuspended
    } else {
        EngineEventType::EntityActivated
    };

    // 1. Root process instance / execution
    process_instance.is_suspended = suspended;
    store.update_process_instance(&process_instance, &mut command_context.session);

    if let Some(mut root_execution) =
        store.find_execution(&process_instance.id, &mut command_context.session)
    {
        root_execution.is_suspended = suspended;
        store.update_execution(&root_execution, &mut command_context.session);
    }

    // Dispatch root entity event (executionId == processInstanceId for root)
    command_context.add_post_agenda_event(EngineEvent::Entity {
        event_type,
        data: EntityEventData {
            entity_kind: EntityKind::Execution,
            entity_id: process_instance.id.clone(),
            process_instance_id: Some(process_instance.id.clone()),
            execution_id: Some(process_instance.id.clone()),
            process_definition_id: Some(process_instance.process_definition_id.clone()),
            scope_type: None,
            scope_id: None,
            sub_scope_id: None,
        },
    });

    // 2. Child executions
    let child_executions: Vec<_> = store
        .snapshot_executions(&mut command_context.session)
        .into_values()
        .filter(|execution| {
            execution.process_instance_id.as_deref() == Some(process_instance.id.as_str())
                && execution.id != process_instance.id
        })
        .collect();

    for mut execution in child_executions {
        execution.is_suspended = suspended;
        store.update_execution(&execution, &mut command_context.session);
        command_context.add_post_agenda_event(EngineEvent::Entity {
            event_type,
            data: EntityEventData {
                entity_kind: EntityKind::Execution,
                entity_id: execution.id.clone(),
                process_instance_id: Some(process_instance.id.clone()),
                execution_id: Some(execution.id.clone()),
                process_definition_id: Some(process_instance.process_definition_id.clone()),
                scope_type: None,
                scope_id: None,
                sub_scope_id: None,
            },
        });
    }

    // 3. Tasks
    let tasks =
        store.find_tasks_by_process_instance_id(&process_instance.id, &mut command_context.session);
    for mut task in tasks {
        let previous_state = task.suspension_state;
        task.set_suspension_state(suspended);
        store.update_task(&task, &mut command_context.session);
        command_context
            .history_manager
            .record_task_suspension_state_change(
                &task.id,
                previous_state,
                task.suspension_state,
                &task,
                &mut command_context.session,
            );
        command_context.add_post_agenda_event(EngineEvent::Entity {
            event_type,
            data: EntityEventData {
                entity_kind: EntityKind::Task,
                entity_id: task.id.clone(),
                process_instance_id: Some(process_instance.id.clone()),
                execution_id: Some(task.execution_id.clone()),
                process_definition_id: Some(process_instance.process_definition_id.clone()),
                scope_type: None,
                scope_id: None,
                sub_scope_id: None,
            },
        });
    }

    // 4. Jobs
    if suspended {
        suspend_jobs_for_process_instance(command_context, &process_instance.id)?;
    } else {
        activate_suspended_jobs_for_process_instance(command_context, &process_instance.id)?;
    }
    Ok(process_instance)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::process_engine::ProcessEngine;
    use crate::interceptor::command::Command;
    use crate::interceptor::command_executor::CommandExecutor;
    use crate::persistence::runtime_store::RuntimeTimerJobState;

    struct SuspendThenFailCmd;

    impl Command<()> for SuspendThenFailCmd {
        fn execute(&self, command_context: &mut CommandContext) -> Result<(), FlowableError> {
            let store = command_context.runtime_store_handle();
            let process_instance = store
                .find_process_instance("process-1", &mut command_context.session)
                .expect("seeded process instance");
            set_process_instance_suspension_state(command_context, process_instance, true)?;
            Err(FlowableError::ExecutionError(
                "forced rollback after process suspension".to_string(),
            ))
        }
    }

    #[test]
    fn process_and_job_suspension_roll_back_together() {
        let engine = ProcessEngine::new("process-suspension-rollback".to_string());
        let store = engine.get_runtime_store();
        let mut session = store.create_session().unwrap();
        store.insert_process_instance(&process_instance(), &mut session);
        store.insert_timer_job_state(&timer_job(), &mut session);
        session.flush_and_commit().unwrap();

        let error = engine
            .get_command_executor()
            .execute(&SuspendThenFailCmd)
            .expect_err("outer command should force rollback");
        assert_eq!(
            error.to_string(),
            "Execution error: forced rollback after process suspension"
        );

        let mut session = store.create_session().unwrap();
        assert!(
            !store
                .find_process_instance("process-1", &mut session)
                .expect("process instance should remain")
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
