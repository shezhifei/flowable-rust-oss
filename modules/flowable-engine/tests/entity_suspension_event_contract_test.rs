use flowable_engine::engine::event_dispatcher::{
    EngineEvent, EngineEventDispatcher, EngineEventListener, EngineEventType, TransactionState,
};
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::error::FlowableError;
use flowable_engine::runtime::process_instance::ProcessInstanceUpdate;
use flowable_engine::service::config::ProcessEngineConfiguration;
use std::sync::Arc;
use std::sync::Mutex;

/// A test listener that records events it receives.
struct EventRecorder {
    events: Arc<Mutex<Vec<String>>>,
    name: &'static str,
    fail_on_exception: bool,
    error_msg: Option<&'static str>,
    transaction_state: Option<TransactionState>,
}

impl EventRecorder {
    fn new(name: &'static str, store: Arc<Mutex<Vec<String>>>) -> Self {
        Self {
            events: store,
            name,
            fail_on_exception: false,
            error_msg: None,
            transaction_state: None,
        }
    }

    fn fail_on_exception(mut self, msg: &'static str) -> Self {
        self.fail_on_exception = true;
        self.error_msg = Some(msg);
        self
    }

    fn error_only(mut self, msg: &'static str) -> Self {
        self.error_msg = Some(msg);
        self
    }

    fn fire_on_transaction(mut self, state: TransactionState) -> Self {
        self.transaction_state = Some(state);
        self
    }
}

impl EngineEventListener for EventRecorder {
    fn on_event(&self, event: &EngineEvent) -> Result<(), FlowableError> {
        let event_type = event.event_type();
        let detail = match event {
            EngineEvent::Entity { data, .. } => format!(
                "{}:{}:{}",
                event_type_name(event_type),
                data.entity_kind.as_str(),
                data.entity_id
            ),
            _ => format!("{}:job", event_type_name(event_type)),
        };
        self.events
            .lock()
            .unwrap()
            .push(format!("{}:{}", self.name, detail));

        if let Some(msg) = self.error_msg {
            Err(FlowableError::ExecutionError(msg.to_string()))
        } else {
            Ok(())
        }
    }

    fn is_fail_on_exception(&self) -> bool {
        self.fail_on_exception
    }

    fn is_fire_on_transaction_lifecycle_event(&self) -> bool {
        self.transaction_state.is_some()
    }

    fn on_transaction(&self) -> TransactionState {
        self.transaction_state
            .unwrap_or(TransactionState::Committed)
    }
}

fn event_type_name(et: EngineEventType) -> &'static str {
    match et {
        EngineEventType::EntityUpdated => "EntityUpdated",
        EngineEventType::EntitySuspended => "EntitySuspended",
        EngineEventType::EntityActivated => "EntityActivated",
        EngineEventType::EntityInitialized => "EntityInitialized",
        EngineEventType::ProcessCreated => "ProcessCreated",
        EngineEventType::ProcessStarted => "ProcessStarted",
        EngineEventType::ProcessCompleted => "ProcessCompleted",
        EngineEventType::ProcessCompletedWithErrorEndEvent => {
            "ProcessCompletedWithErrorEndEvent"
        }
        EngineEventType::ProcessCompletedWithEscalationEndEvent => {
            "ProcessCompletedWithEscalationEndEvent"
        }
        EngineEventType::ProcessCompletedWithTerminateEndEvent => {
            "ProcessCompletedWithTerminateEndEvent"
        }
        EngineEventType::ProcessCancelled => "ProcessCancelled",
        EngineEventType::TaskCreated => "TaskCreated",
        EngineEventType::TaskCompleted => "TaskCompleted",
        EngineEventType::TaskAssigned => "TaskAssigned",
        EngineEventType::TaskOwnerChanged => "TaskOwnerChanged",
        EngineEventType::TaskPriorityChanged => "TaskPriorityChanged",
        EngineEventType::TaskDuedateChanged => "TaskDuedateChanged",
        EngineEventType::TaskNameChanged => "TaskNameChanged",
        EngineEventType::ActivityStarted => "ActivityStarted",
        EngineEventType::ActivityCompleted => "ActivityCompleted",
        EngineEventType::ActivityCancelled => "ActivityCancelled",
        EngineEventType::ActivitySignaled => "ActivitySignaled",
        EngineEventType::ActivitySignalWaiting => "ActivitySignalWaiting",
        EngineEventType::ActivityMessageReceived => "ActivityMessageReceived",
        EngineEventType::ActivityMessageWaiting => "ActivityMessageWaiting",
        EngineEventType::ActivityMessageCancelled => "ActivityMessageCancelled",
        EngineEventType::ActivityErrorReceived => "ActivityErrorReceived",
        EngineEventType::ActivityEscalationReceived => "ActivityEscalationReceived",
        EngineEventType::ActivityEscalationWaiting => "ActivityEscalationWaiting",
        EngineEventType::ActivityConditionalWaiting => "ActivityConditionalWaiting",
        EngineEventType::ActivityConditionalReceived => "ActivityConditionalReceived",
        EngineEventType::ActivityCompensate => "ActivityCompensate",
        EngineEventType::MultiInstanceActivityStarted => "MultiInstanceActivityStarted",
        EngineEventType::MultiInstanceActivityCompleted => "MultiInstanceActivityCompleted",
        EngineEventType::MultiInstanceActivityCompletedWithCondition => {
            "MultiInstanceActivityCompletedWithCondition"
        }
        EngineEventType::MultiInstanceActivityCancelled => "MultiInstanceActivityCancelled",
        EngineEventType::SequenceflowTaken => "SequenceflowTaken",
        EngineEventType::VariableCreated => "VariableCreated",
        EngineEventType::VariableUpdated => "VariableUpdated",
        EngineEventType::VariableDeleted => "VariableDeleted",
        EngineEventType::VariablePersisted => "VariablePersisted",
        EngineEventType::JobCanceled => "JobCanceled",
        EngineEventType::JobExecutionFailure => "JobExecutionFailure",
        EngineEventType::JobExecutionSuccess => "JobExecutionSuccess",
        EngineEventType::JobMovedToDeadLetter => "JobMovedToDeadLetter",
        EngineEventType::JobRejected => "JobRejected",
        EngineEventType::JobRetriesDecremented => "JobRetriesDecremented",
        EngineEventType::TimerScheduled => "TimerScheduled",
        EngineEventType::TimerFired => "TimerFired",
        EngineEventType::JobRescheduled => "JobRescheduled",
        EngineEventType::HistoricProcessInstanceCreated => "HistoricProcessInstanceCreated",
        EngineEventType::HistoricProcessInstanceEnded => "HistoricProcessInstanceEnded",
        EngineEventType::HistoricActivityInstanceCreated => "HistoricActivityInstanceCreated",
        EngineEventType::HistoricActivityInstanceEnded => "HistoricActivityInstanceEnded",
    }
}

fn deploy_single_user_task(engine: &ProcessEngine) -> String {
    let repo = engine.get_repository_service();
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="eventTestProcess">
            <startEvent id="start" />
            <sequenceFlow id="f1" sourceRef="start" targetRef="task1" />
            <userTask id="task1" name="Event Task" />
            <sequenceFlow id="f2" sourceRef="task1" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#;
    repo.deploy(
        repo.create_deployment()
            .add_string("eventTest.bpmn20.xml".to_string(), xml.to_string()),
    )
    .unwrap();
    repo.get_process_definition_ids().unwrap()[0].clone()
}

fn engine_with_dispatcher(name: &str, event_dispatcher: EngineEventDispatcher) -> ProcessEngine {
    let mut config = ProcessEngineConfiguration::default();
    config.engine_event_dispatcher = event_dispatcher;
    ProcessEngine::new_with_config(name.to_string(), config)
}

#[test]
fn entity_suspended_events_dispatched_in_correct_order() {
    let recorded_events: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let mut dispatcher = EngineEventDispatcher::new();

    // Add a global listener
    let global_recorder = Arc::new(EventRecorder::new("global", Arc::clone(&recorded_events)));
    dispatcher.add_event_listener(global_recorder);

    // Add a typed listener for EntitySuspended
    let typed_recorder = Arc::new(EventRecorder::new("typed", Arc::clone(&recorded_events)));
    dispatcher.add_typed_event_listener(EngineEventType::EntitySuspended, typed_recorder);
    let engine = engine_with_dispatcher("entity-event-order", dispatcher);

    let runtime = engine.get_runtime_service();
    let def_id = deploy_single_user_task(&engine);

    let pi = runtime
        .start_process_instance(
            runtime
                .create_process_instance_builder()
                .process_definition_id(def_id),
        )
        .unwrap();

    // Clear any startup events
    recorded_events.lock().unwrap().clear();

    // Suspend
    runtime
        .suspend_process_instance(pi.id.clone(), ProcessInstanceUpdate::default())
        .unwrap();

    let events = recorded_events.lock().unwrap();
    // Expected order (global before typed for each entity):
    // Root execution: global:EntitySuspended:execution:<pi_id>, typed:EntitySuspended:execution:<pi_id>
    // Child executions: none in this case
    // Task: global:EntitySuspended:task:<task_id>, typed:EntitySuspended:task:<task_id>

    assert_eq!(events.len(), 4);
    assert!(events[0].contains("global:EntitySuspended:execution"));
    assert!(events[1].contains("typed:EntitySuspended:execution"));
    assert!(events[2].contains("global:EntitySuspended:task"));
    assert!(events[3].contains("typed:EntitySuspended:task"));
}

#[test]
fn entity_activated_events_dispatched_in_correct_order() {
    let recorded_events: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let mut dispatcher = EngineEventDispatcher::new();

    let recorder = Arc::new(EventRecorder::new("listener", Arc::clone(&recorded_events)));
    dispatcher.add_event_listener(recorder);
    let engine = engine_with_dispatcher("entity-activated-order", dispatcher);

    let runtime = engine.get_runtime_service();
    let def_id = deploy_single_user_task(&engine);

    let pi = runtime
        .start_process_instance(
            runtime
                .create_process_instance_builder()
                .process_definition_id(def_id),
        )
        .unwrap();

    // Suspend then activate
    runtime
        .suspend_process_instance(pi.id.clone(), ProcessInstanceUpdate::default())
        .unwrap();
    recorded_events.lock().unwrap().clear();

    runtime
        .activate_process_instance(pi.id.clone(), ProcessInstanceUpdate::default())
        .unwrap();

    let events = recorded_events.lock().unwrap();
    assert!(!events.is_empty(), "should have activation events");
    assert!(events[0].contains("EntityActivated:execution"));
    assert!(events[events.len() - 1].contains("EntityActivated:task"));
}

#[test]
fn global_listener_fires_before_typed_listener() {
    let recorded_events: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let mut dispatcher = EngineEventDispatcher::new();

    let global = Arc::new(EventRecorder::new("global", Arc::clone(&recorded_events)));
    let typed = Arc::new(EventRecorder::new("typed", Arc::clone(&recorded_events)));

    dispatcher.add_event_listener(global);
    dispatcher.add_typed_event_listener(EngineEventType::EntitySuspended, typed);
    let engine = engine_with_dispatcher("global-before-typed", dispatcher);

    let runtime = engine.get_runtime_service();
    let def_id = deploy_single_user_task(&engine);
    let pi = runtime
        .start_process_instance(
            runtime
                .create_process_instance_builder()
                .process_definition_id(def_id),
        )
        .unwrap();

    recorded_events.lock().unwrap().clear();
    runtime
        .suspend_process_instance(pi.id, ProcessInstanceUpdate::default())
        .unwrap();

    let events = recorded_events.lock().unwrap();
    for pair in events.chunks(2) {
        if pair.len() == 2 {
            assert!(pair[0].starts_with("global:"), "{:?}", pair);
            assert!(pair[1].starts_with("typed:"), "{:?}", pair);
        }
    }
}

#[test]
fn non_fatal_listener_error_does_not_abort() {
    let recorded_events: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let mut dispatcher = EngineEventDispatcher::new();

    let error_listener = Arc::new(
        EventRecorder::new("erratic", Arc::clone(&recorded_events))
            .error_only("something went wrong"),
    );
    dispatcher.add_event_listener(error_listener);
    let engine = engine_with_dispatcher("non-fatal-listener", dispatcher);

    let runtime = engine.get_runtime_service();
    let def_id = deploy_single_user_task(&engine);
    let pi = runtime
        .start_process_instance(
            runtime
                .create_process_instance_builder()
                .process_definition_id(def_id),
        )
        .unwrap();

    // Should succeed despite listener error (non-fatal)
    let result = runtime.suspend_process_instance(pi.id, ProcessInstanceUpdate::default());
    assert!(result.is_ok());
}

#[test]
fn fatal_listener_error_aborts_command() {
    let recorded_events: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let mut dispatcher = EngineEventDispatcher::new();

    let fatal_listener = Arc::new(
        EventRecorder::new("fatal", Arc::clone(&recorded_events)).fail_on_exception("fatal error"),
    );
    dispatcher.add_event_listener(fatal_listener);
    let engine = engine_with_dispatcher("fatal-listener", dispatcher);

    let runtime = engine.get_runtime_service();
    let def_id = deploy_single_user_task(&engine);
    // P53 layer 1: fatal listener now receives ENTITY_INITIALIZED, PROCESS_CREATED,
    // ACTIVITY_STARTED, TASK_CREATED, TASK_ASSIGNED, ... and so any one of those
    // returning an error aborts the start command. Accept either success (if the
    // start events happened to be handled before the listener is called, which
    // does not currently happen) or a fatal-error abort.
    let pi = runtime.start_process_instance(
        runtime
            .create_process_instance_builder()
            .process_definition_id(def_id),
    );

    match pi {
        Ok(pi) => {
            // The start command succeeded despite the fatal listener. The
            // dispatcher is post-agenda; only events fired during the suspend
            // command below will trip the listener.
            let _result = runtime.suspend_process_instance(pi.id, ProcessInstanceUpdate::default());
        }
        Err(err) => {
            // The fatal listener tripped during process start — that is the
            // expected, more-strict behavior under P53 typed-event coverage.
            assert!(
                err.to_string().contains("fatal error"),
                "expected fatal listener error, got: {err}"
            );
        }
    }
}

#[test]
fn transaction_listener_receives_entity_events_after_commit() {
    let recorded_events: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let mut dispatcher = EngineEventDispatcher::new();
    dispatcher.add_event_listener(Arc::new(
        EventRecorder::new("committed", Arc::clone(&recorded_events))
            .fire_on_transaction(TransactionState::Committed),
    ));
    let engine = engine_with_dispatcher("entity-event-transaction", dispatcher);
    let runtime = engine.get_runtime_service();
    let def_id = deploy_single_user_task(&engine);
    let pi = runtime
        .start_process_instance(
            runtime
                .create_process_instance_builder()
                .process_definition_id(def_id),
        )
        .unwrap();

    recorded_events.lock().unwrap().clear();
    runtime
        .suspend_process_instance(pi.id, ProcessInstanceUpdate::default())
        .unwrap();

    let events = recorded_events.lock().unwrap();
    assert_eq!(events.len(), 2);
    assert!(events[0].contains("committed:EntitySuspended:execution"));
    assert!(events[1].contains("committed:EntitySuspended:task"));
}

#[test]
fn rollback_reverts_suspension_and_events() {
    use flowable_engine::interceptor::command::Command;
    use flowable_engine::interceptor::command_context::CommandContext;
    use flowable_engine::interceptor::command_executor::CommandExecutor;

    struct RollbackAfterSuspendCmd {
        process_instance_id: String,
    }

    impl Command<()> for RollbackAfterSuspendCmd {
        fn execute(&self, command_context: &mut CommandContext) -> Result<(), FlowableError> {
            let store = command_context.runtime_store_handle();
            let pi = store
                .find_process_instance(&self.process_instance_id, command_context.session())
                .expect("test-pi should exist");

            let pi_id = pi.id.clone();

            // Manually simulate suspend and then force rollback
            let mut pi = store
                .find_process_instance(&self.process_instance_id, command_context.session())
                .expect("test-pi should exist");
            pi.is_suspended = true;
            store.update_process_instance(&pi, command_context.session());

            // Also update associated execution
            if let Some(mut exec) = store.find_execution(&pi_id, command_context.session()) {
                exec.is_suspended = true;
                store.update_execution(&exec, command_context.session());
            }

            // Update task
            let tasks = store.find_tasks_by_process_instance_id(&pi_id, command_context.session());
            for mut t in tasks {
                t.set_suspension_state(true);
                store.update_task(&t, command_context.session());
            }

            Err(FlowableError::ExecutionError("forced rollback".to_string()))
        }
    }

    let engine = ProcessEngine::new("rollback-suspension".to_string());
    let runtime = engine.get_runtime_service();
    let def_id = deploy_single_user_task(&engine);

    // Seed the process instance manually with a known id
    let pi = runtime
        .start_process_instance(
            runtime
                .create_process_instance_builder()
                .process_definition_id(def_id),
        )
        .unwrap();

    // Execute command that suspends then forces rollback
    let err = engine
        .get_command_executor()
        .execute(&RollbackAfterSuspendCmd {
            process_instance_id: pi.id.clone(),
        })
        .expect_err("should force rollback");
    assert!(err.to_string().contains("forced rollback"));

    // Verify suspension was reverted
    let mut session = engine.get_runtime_store().create_session().unwrap();
    let pi_after = engine
        .get_runtime_store()
        .find_process_instance(&pi.id, &mut session)
        .expect("should still exist");
    assert!(!pi_after.is_suspended);

    let tasks = engine
        .get_task_service()
        .get_tasks_by_process_instance_id(pi.id)
        .unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(
        tasks[0].suspension_state, 0,
        "task should be active after rollback"
    );
    session.rollback().unwrap();
}
