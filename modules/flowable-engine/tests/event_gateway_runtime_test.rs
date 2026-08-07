use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::engine::query::Query;
use flowable_engine::persistence::runtime_store::EventSubscriptionKind;

const EVENT_GATEWAY_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
    <process id="eventGatewayProcess" isExecutable="true">
        <startEvent id="start" />
        <sequenceFlow id="flow1" sourceRef="start" targetRef="gw" />
        <eventBasedGateway id="gw" />
        <sequenceFlow id="flow2" sourceRef="gw" targetRef="catchMessage" />
        <sequenceFlow id="flow3" sourceRef="gw" targetRef="catchSignal" />
        <intermediateCatchEvent id="catchMessage">
            <messageEventDefinition messageRef="msg1" />
        </intermediateCatchEvent>
        <intermediateCatchEvent id="catchSignal">
            <signalEventDefinition signalRef="sig1" />
        </intermediateCatchEvent>
        <sequenceFlow id="flow4" sourceRef="catchMessage" targetRef="taskAfterMsg" />
        <sequenceFlow id="flow5" sourceRef="catchSignal" targetRef="taskAfterSig" />
        <userTask id="taskAfterMsg" name="Task After Message" />
        <userTask id="taskAfterSig" name="Task After Signal" />
        <endEvent id="end" />
    </process>
</definitions>"#;

/// P52 probe (Java parity: message + timer behind event-based gateway).
/// After the message path fires, the sibling timer subscription/job must be
/// cleaned up and must not be able to take the process down the timer branch.
const EVENT_GATEWAY_MESSAGE_TIMER_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
    <process id="eventGatewayMessageTimerProcess" isExecutable="true">
        <startEvent id="start" />
        <sequenceFlow id="flow1" sourceRef="start" targetRef="gw" />
        <eventBasedGateway id="gw" />
        <sequenceFlow id="flow2" sourceRef="gw" targetRef="catchMessage" />
        <sequenceFlow id="flow3" sourceRef="gw" targetRef="catchTimer" />
        <intermediateCatchEvent id="catchMessage">
            <messageEventDefinition messageRef="msg1" />
        </intermediateCatchEvent>
        <intermediateCatchEvent id="catchTimer">
            <timerEventDefinition>
                <timeDuration>PT1S</timeDuration>
            </timerEventDefinition>
        </intermediateCatchEvent>
        <sequenceFlow id="flow4" sourceRef="catchMessage" targetRef="taskAfterMsg" />
        <sequenceFlow id="flow5" sourceRef="catchTimer" targetRef="taskAfterTimer" />
        <userTask id="taskAfterMsg" name="Task After Message" />
        <userTask id="taskAfterTimer" name="Task After Timer" />
        <endEvent id="end" />
    </process>
</definitions>"#;

#[test]
fn test_event_gateway_first_trigger_wins() {
    let process_engine = ProcessEngine::new("default".to_string());
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();
    let task_service = process_engine.get_task_service();

    let deployment_builder = repository_service.create_deployment().add_string(
        "event_gateway.bpmn20.xml".to_string(),
        EVENT_GATEWAY_XML.to_string(),
    );
    repository_service.deploy(deployment_builder).unwrap();

    let process_def_id = repository_service.get_process_definition_ids().unwrap()[0].clone();
    let process_instance = runtime_service
        .start_process_instance_by_id(process_def_id, None)
        .unwrap();

    // 1. Should have 2 event wait states
    let wait_states =
        task_service.get_event_wait_states_by_process_instance_id(process_instance.id.clone());
    assert_eq!(wait_states.len(), 2);

    let msg_wait_state = wait_states
        .iter()
        .find(|ws| ws.event_ref.as_deref() == Some("msg1"))
        .unwrap();

    // 2. Trigger message
    runtime_service.trigger_event_intermediate_catch(
        EventSubscriptionKind::Message,
        "msg1".to_string(),
        msg_wait_state.execution_id.clone(),
    );

    // 3. Signal wait state should be gone, and we should be at Task After Message
    let wait_states =
        task_service.get_event_wait_states_by_process_instance_id(process_instance.id.clone());
    assert_eq!(wait_states.len(), 0);

    let tasks = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].name, "Task After Message");
}

/// P52: event-based gateway sibling cleanup — message wins over timer.
///
/// Java semantic (EventBasedGatewayTest.testCatchSignalCancelsTimer style):
/// when one outgoing catch of an event-based gateway is triggered, remaining
/// event subscriptions and timers under that gateway are cancelled so they
/// can no longer fire.
#[test]
fn test_event_gateway_message_cancels_sibling_timer() {
    let process_engine = ProcessEngine::new("p52-event-gw-msg-timer".to_string());
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();
    let task_service = process_engine.get_task_service();
    let runtime_store = process_engine.get_runtime_store();

    repository_service
        .deploy(
            repository_service.create_deployment().add_string(
                "event_gateway_message_timer.bpmn20.xml".to_string(),
                EVENT_GATEWAY_MESSAGE_TIMER_XML.to_string(),
            ),
        )
        .unwrap();

    let process_def_id = repository_service.get_process_definition_ids().unwrap()[0].clone();
    let process_instance = runtime_service
        .start_process_instance_by_id(process_def_id, None)
        .unwrap();

    // Before trigger: message wait-state + intermediate timer job both present.
    let wait_states =
        task_service.get_event_wait_states_by_process_instance_id(process_instance.id.clone());
    assert_eq!(wait_states.len(), 1, "expected exactly one message wait state");
    let msg_wait = wait_states
        .iter()
        .find(|ws| ws.event_ref.as_deref() == Some("msg1"))
        .expect("message wait state for msg1");

    let timer_execution_id = {
        let mut session = runtime_store.create_session().unwrap();
        let timer_exec = runtime_store
            .snapshot_executions(&mut session)
            .into_values()
            .find(|e| {
                e.process_instance_id.as_deref() == Some(process_instance.id.as_str())
                    && e.activity_id.as_deref() == Some("catchTimer")
                    && !e.is_ended
            })
            .expect("timer catch execution should exist behind the event gateway");
        let timer_states = runtime_store
            .find_timer_job_states_by_process_instance_id(&process_instance.id, &mut session);
        assert_eq!(timer_states.len(), 1, "expected one intermediate timer job");
        assert_eq!(timer_states[0].activity_id, "catchTimer");
        assert!(!timer_states[0].is_boundary);
        assert_eq!(timer_states[0].time_duration.as_deref(), Some("PT1S"));
        timer_exec.id
    };

    // Message path wins.
    runtime_service.trigger_event_intermediate_catch(
        EventSubscriptionKind::Message,
        "msg1".to_string(),
        msg_wait.execution_id.clone(),
    );

    // Sibling timer subscription/job must be gone; only message branch remains.
    {
        let mut session = runtime_store.create_session().unwrap();
        let timer_states = runtime_store
            .find_timer_job_states_by_process_instance_id(&process_instance.id, &mut session);
        assert!(
            timer_states.is_empty(),
            "sibling timer job must be deleted after message triggers event gateway branch"
        );

        let timer_exec_gone = runtime_store
            .find_execution(&timer_execution_id, &mut session)
            .is_none()
            || runtime_store
                .find_execution(&timer_execution_id, &mut session)
                .map(|e| e.is_ended)
                .unwrap_or(true);
        assert!(
            timer_exec_gone,
            "sibling timer execution must be deleted/ended after gateway cancel"
        );
    }

    let wait_states =
        task_service.get_event_wait_states_by_process_instance_id(process_instance.id.clone());
    assert_eq!(wait_states.len(), 0);

    let tasks = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].name, "Task After Message");

    // Timer must no longer be triggerable (due-time fire becomes a no-op).
    let trigger_result =
        runtime_service.trigger_timer_intermediate_catch_event(timer_execution_id.clone());
    assert!(
        trigger_result.is_ok(),
        "missing sibling timer should soft-no-op, not hard-fail: {trigger_result:?}"
    );

    let tasks_after = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(tasks_after.len(), 1);
    assert_eq!(
        tasks_after[0].name, "Task After Message",
        "timer must not open the sibling branch after message already won"
    );
    assert!(
        tasks_after.iter().all(|t| t.name != "Task After Timer"),
        "Task After Timer must never appear once message cancelled the sibling timer"
    );
}

/// P71: event-based gateway cancel records Java
/// `DeleteReason.EVENT_BASED_GATEWAY_CANCEL` on the cancelled sibling's
/// historic activity; the winning branch and normally-completed nodes keep
/// `delete_reason == None`.
#[test]
fn test_event_gateway_cancel_sets_historic_activity_delete_reason() {
    let process_engine = ProcessEngine::new("p71-event-gw-delete-reason".to_string());
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();
    let task_service = process_engine.get_task_service();
    let history_service = process_engine.get_history_service();

    repository_service
        .deploy(
            repository_service.create_deployment().add_string(
                "event_gateway_message_timer.bpmn20.xml".to_string(),
                EVENT_GATEWAY_MESSAGE_TIMER_XML.to_string(),
            ),
        )
        .unwrap();

    let process_def_id = repository_service.get_process_definition_ids().unwrap()[0].clone();
    let process_instance = runtime_service
        .start_process_instance_by_id(process_def_id, None)
        .unwrap();

    // Pre-trigger: open historic activities for both catches have no deleteReason.
    let pre = history_service
        .create_historic_activity_instance_query()
        .process_instance_id(process_instance.id.clone())
        .list()
        .unwrap();
    let pre_timer = pre
        .iter()
        .find(|a| a.activity_id == "catchTimer" && a.end_time.is_none())
        .expect("open historic activity for catchTimer before trigger");
    assert!(
        pre_timer.delete_reason.is_none(),
        "untriggered sibling must not have a deleteReason yet"
    );
    let pre_msg = pre
        .iter()
        .find(|a| a.activity_id == "catchMessage" && a.end_time.is_none())
        .expect("open historic activity for catchMessage before trigger");
    assert!(
        pre_msg.delete_reason.is_none(),
        "winning branch must not have a deleteReason before trigger"
    );

    let wait_states =
        task_service.get_event_wait_states_by_process_instance_id(process_instance.id.clone());
    let msg_wait = wait_states
        .iter()
        .find(|ws| ws.event_ref.as_deref() == Some("msg1"))
        .expect("message wait state for msg1");

    runtime_service.trigger_event_intermediate_catch(
        EventSubscriptionKind::Message,
        "msg1".to_string(),
        msg_wait.execution_id.clone(),
    );

    let post = history_service
        .create_historic_activity_instance_query()
        .process_instance_id(process_instance.id.clone())
        .list()
        .unwrap();

    let cancelled_timer = post
        .iter()
        .find(|a| a.activity_id == "catchTimer")
        .expect("historic activity for cancelled catchTimer");
    assert!(
        cancelled_timer.end_time.is_some(),
        "cancelled sibling historic activity must be ended"
    );
    assert_eq!(
        cancelled_timer.delete_reason.as_deref(),
        Some(flowable_engine::history::delete_reason::EVENT_BASED_GATEWAY_CANCEL),
        "cancelled sibling must carry event based gateway cancel"
    );

    // Winning catch left normally via take-outgoing — no deleteReason.
    let msg_catch = post
        .iter()
        .find(|a| a.activity_id == "catchMessage" && a.end_time.is_some())
        .expect("ended historic activity for winning catchMessage");
    assert!(
        msg_catch.delete_reason.is_none(),
        "normally completed activity must keep deleteReason null"
    );

    // Gateway itself completed normally.
    let gw = post
        .iter()
        .find(|a| a.activity_id == "gw" && a.end_time.is_some())
        .expect("ended historic activity for event-based gateway");
    assert!(
        gw.delete_reason.is_none(),
        "gateway leave is a normal complete, not a cancel"
    );
}
