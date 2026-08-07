use flowable_engine::cmd::trigger_start_event_subscription_cmd::TriggerEventSubprocessByEventCmd;
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::engine::query::Query;
use flowable_engine::interceptor::command_executor::CommandExecutor;
use flowable_engine::persistence::runtime_store::EventSubscriptionKind;

const PROCESS_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
    <process id="subprocessProcess" isExecutable="true">
        <startEvent id="startEvent" />
        <sequenceFlow id="flow1" sourceRef="startEvent" targetRef="subProcess" />
        <subProcess id="subProcess">
            <startEvent id="innerStart" />
            <sequenceFlow id="innerFlow1" sourceRef="innerStart" targetRef="innerTask" />
            <userTask id="innerTask" name="Inner Task" />
            <sequenceFlow id="innerFlow2" sourceRef="innerTask" targetRef="innerEnd" />
            <endEvent id="innerEnd" />
        </subProcess>
        <sequenceFlow id="flow2" sourceRef="subProcess" targetRef="outerTask" />
        <userTask id="outerTask" name="Outer Task" />
        <sequenceFlow id="flow3" sourceRef="outerTask" targetRef="outerEnd" />
        <endEvent id="outerEnd" />
    </process>
</definitions>"#;

#[test]
fn test_subprocess_runtime_semantics() {
    let process_engine = ProcessEngine::new("default".to_string());
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();
    let task_service = process_engine.get_task_service();

    let deployment_builder = repository_service
        .create_deployment()
        .name("Subprocess Deployment".to_string())
        .add_string(
            "subprocess_process.bpmn20.xml".to_string(),
            PROCESS_XML.to_string(),
        );
    repository_service.deploy(deployment_builder).unwrap();

    let process_def_id = repository_service.get_process_definition_ids().unwrap()[0].clone();

    let builder = runtime_service
        .create_process_instance_builder()
        .process_definition_id(process_def_id);
    let process_instance = runtime_service.start_process_instance(builder).unwrap();

    // 1. Process starts, enters SubProcess, and creates Inner Task
    let tasks = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 1, "Should be at inner task");
    assert_eq!(tasks[0].task_definition_key, "innerTask");

    // 2. Complete Inner Task, subprocess should complete and resume outer flow
    task_service
        .complete_task_by_id(tasks[0].id.clone())
        .unwrap();

    // 3. Outer flow resumes, creating Outer Task
    let tasks = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 1, "Should be at outer task");
    assert_eq!(tasks[0].task_definition_key, "outerTask");

    // 4. Complete Outer Task, process instance should end
    task_service
        .complete_task_by_id(tasks[0].id.clone())
        .unwrap();

    let runtime_store = process_engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let pi = runtime_store
        .find_process_instance(&process_instance.id, &mut session)
        .unwrap();
    session.rollback().unwrap();
    assert!(pi.is_ended, "Process instance should be ended");
}

const EVENT_SUBPROCESS_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
    <process id="eventSubprocessProcess" isExecutable="true">
        <startEvent id="startEvent" />
        <sequenceFlow id="flow1" sourceRef="startEvent" targetRef="mainTask" />
        <userTask id="mainTask" name="Main Task" />
        <sequenceFlow id="flow2" sourceRef="mainTask" targetRef="outerEnd" />
        <endEvent id="outerEnd" />

        <subProcess id="eventSubProcess" triggeredByEvent="true">
            <startEvent id="eventSubStart" isInterrupting="true">
                <messageEventDefinition messageRef="cancelMessage" />
            </startEvent>
            <sequenceFlow id="esFlow1" sourceRef="eventSubStart" targetRef="esTask" />
            <userTask id="esTask" name="Event Sub Task" />
            <sequenceFlow id="esFlow2" sourceRef="esTask" targetRef="esEnd" />
            <endEvent id="esEnd" />
        </subProcess>
    </process>
</definitions>"#;

#[test]
fn test_event_subprocess_interrupting() {
    let process_engine = ProcessEngine::new("default".to_string());
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();
    let task_service = process_engine.get_task_service();

    let deployment_builder = repository_service
        .create_deployment()
        .name("Event Subprocess Deployment".to_string())
        .add_string(
            "event_subprocess.bpmn20.xml".to_string(),
            EVENT_SUBPROCESS_XML.to_string(),
        );
    repository_service.deploy(deployment_builder).unwrap();

    let process_def_id = repository_service.get_process_definition_ids().unwrap()[0].clone();

    let builder = runtime_service
        .create_process_instance_builder()
        .process_definition_id(process_def_id);
    let process_instance = runtime_service.start_process_instance(builder).unwrap();

    // 1. Process starts, at mainTask
    let tasks = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 1, "Should be at main task");
    assert_eq!(tasks[0].task_definition_key, "mainTask");

    // 2. Trigger Event Subprocess (Interrupting)
    let _ = runtime_service.trigger_event_subprocess_by_message(
        "cancelMessage".to_string(),
        process_instance.id.clone(),
    );

    // 3. Main Task should be cancelled, Event Sub Task should be active
    let tasks = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 1, "Should be at event sub task");
    assert_eq!(tasks[0].task_definition_key, "esTask");

    // 4. Complete Event Sub Task, process instance should end (since it's interrupting, no more active scopes)
    task_service
        .complete_task_by_id(tasks[0].id.clone())
        .unwrap();

    let runtime_store = process_engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let pi = runtime_store
        .find_process_instance(&process_instance.id, &mut session)
        .unwrap();
    session.rollback().unwrap();
    assert!(
        pi.is_ended,
        "Process instance should be ended after event sub completion"
    );
}

const EMBEDDED_ESCALATION_EVENT_SUBPROCESS_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
    <escalation id="approvalEscalation" escalationCode="APPROVAL_TIMEOUT" />
    <process id="embeddedEscalationEventSubprocessProcess" isExecutable="true">
        <startEvent id="startEvent" />
        <sequenceFlow id="flow1" sourceRef="startEvent" targetRef="forkScopes" />
        <parallelGateway id="forkScopes" />
        <sequenceFlow id="flow2" sourceRef="forkScopes" targetRef="leftSubProcess" />
        <sequenceFlow id="flow3" sourceRef="forkScopes" targetRef="rightSubProcess" />

        <subProcess id="leftSubProcess">
            <startEvent id="leftStart" />
            <sequenceFlow id="leftFlow1" sourceRef="leftStart" targetRef="leftHostTask" />
            <userTask id="leftHostTask" name="Left Host Task" />
            <sequenceFlow id="leftFlow2" sourceRef="leftHostTask" targetRef="leftEnd" />
            <endEvent id="leftEnd" />

            <subProcess id="leftEscalationEventSubProcess" triggeredByEvent="true">
                <startEvent id="leftEscalationStart" isInterrupting="true">
                    <escalationEventDefinition escalationRef="approvalEscalation" />
                </startEvent>
                <sequenceFlow id="leftEscalationFlow1" sourceRef="leftEscalationStart" targetRef="leftEscalatedTask" />
                <userTask id="leftEscalatedTask" name="Left Escalated Task" />
                <sequenceFlow id="leftEscalationFlow2" sourceRef="leftEscalatedTask" targetRef="leftEscalationEnd" />
                <endEvent id="leftEscalationEnd" />
            </subProcess>
        </subProcess>

        <subProcess id="rightSubProcess">
            <startEvent id="rightStart" />
            <sequenceFlow id="rightFlow1" sourceRef="rightStart" targetRef="rightFork" />
            <parallelGateway id="rightFork" />
            <sequenceFlow id="rightFlow2" sourceRef="rightFork" targetRef="rightHostTask" />
            <sequenceFlow id="rightFlow3" sourceRef="rightFork" targetRef="throwEscalation" />
            <userTask id="rightHostTask" name="Right Host Task" />
            <sequenceFlow id="rightFlow4" sourceRef="rightHostTask" targetRef="rightEnd" />
            <intermediateThrowEvent id="throwEscalation">
                <escalationEventDefinition escalationCode="APPROVAL_TIMEOUT" />
            </intermediateThrowEvent>
            <sequenceFlow id="rightFlow5" sourceRef="throwEscalation" targetRef="throwEnd" />
            <endEvent id="throwEnd" />
            <endEvent id="rightEnd" />

            <subProcess id="rightEscalationEventSubProcess" triggeredByEvent="true">
                <startEvent id="rightEscalationStart" isInterrupting="true">
                    <escalationEventDefinition escalationRef="approvalEscalation" />
                </startEvent>
                <sequenceFlow id="rightEscalationFlow1" sourceRef="rightEscalationStart" targetRef="rightEscalatedTask" />
                <userTask id="rightEscalatedTask" name="Right Escalated Task" />
                <sequenceFlow id="rightEscalationFlow2" sourceRef="rightEscalatedTask" targetRef="rightEscalationEnd" />
                <endEvent id="rightEscalationEnd" />
            </subProcess>
        </subProcess>
    </process>
</definitions>"#;

#[test]
fn test_embedded_escalation_event_subprocess_prefers_throwing_scope() {
    let process_engine =
        ProcessEngine::new("embedded-escalation-event-subprocess-test".to_string());
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();
    let task_service = process_engine.get_task_service();

    let deployment_builder = repository_service
        .create_deployment()
        .name("Embedded Escalation Event Subprocess Deployment".to_string())
        .add_string(
            "embedded_escalation_event_subprocess.bpmn20.xml".to_string(),
            EMBEDDED_ESCALATION_EVENT_SUBPROCESS_XML.to_string(),
        );
    repository_service.deploy(deployment_builder).unwrap();

    let process_def_id = repository_service.get_process_definition_ids().unwrap()[0].clone();

    let builder = runtime_service
        .create_process_instance_builder()
        .process_definition_id(process_def_id);
    let process_instance = runtime_service.start_process_instance(builder).unwrap();

    let mut task_keys = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap()
        .into_iter()
        .map(|task| task.task_definition_key)
        .collect::<Vec<_>>();
    task_keys.sort();

    assert_eq!(
        task_keys,
        vec!["leftHostTask".to_string(), "rightEscalatedTask".to_string()],
        "the escalation should interrupt only the throwing scope and must not trigger the sibling scope event subprocess"
    );
}

const EMBEDDED_END_ESCALATION_EVENT_SUBPROCESS_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
    <escalation id="approvalEscalation" escalationCode="APPROVAL_TIMEOUT" />
    <process id="embeddedEndEscalationEventSubprocessProcess" isExecutable="true">
        <startEvent id="startEvent" />
        <sequenceFlow id="flow1" sourceRef="startEvent" targetRef="reviewSubProcess" />

        <subProcess id="reviewSubProcess">
            <startEvent id="reviewStart" />
            <sequenceFlow id="reviewFlow1" sourceRef="reviewStart" targetRef="reviewFork" />
            <parallelGateway id="reviewFork" />
            <sequenceFlow id="reviewFlow2" sourceRef="reviewFork" targetRef="reviewTask" />
            <sequenceFlow id="reviewFlow3" sourceRef="reviewFork" targetRef="throwingEnd" />
            <userTask id="reviewTask" name="Review Task" />
            <sequenceFlow id="reviewFlow4" sourceRef="reviewTask" targetRef="reviewEnd" />
            <endEvent id="reviewEnd" />
            <endEvent id="throwingEnd">
                <escalationEventDefinition escalationCode="APPROVAL_TIMEOUT" />
            </endEvent>

            <subProcess id="reviewEscalationEventSubProcess" triggeredByEvent="true">
                <startEvent id="reviewEscalationStart" isInterrupting="true">
                    <escalationEventDefinition escalationRef="approvalEscalation" />
                </startEvent>
                <sequenceFlow id="reviewEscalationFlow1" sourceRef="reviewEscalationStart" targetRef="reviewEscalatedTask" />
                <userTask id="reviewEscalatedTask" name="Review Escalated Task" />
                <sequenceFlow id="reviewEscalationFlow2" sourceRef="reviewEscalatedTask" targetRef="reviewEscalationEnd" />
                <endEvent id="reviewEscalationEnd" />
            </subProcess>
        </subProcess>
    </process>
</definitions>"#;

const EMBEDDED_END_ERROR_EVENT_SUBPROCESS_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
    <process id="embeddedEndErrorEventSubprocessProcess" isExecutable="true">
        <startEvent id="startEvent" />
        <sequenceFlow id="flow1" sourceRef="startEvent" targetRef="reviewSubProcess" />

        <subProcess id="reviewSubProcess">
            <startEvent id="reviewStart" />
            <sequenceFlow id="reviewFlow1" sourceRef="reviewStart" targetRef="reviewFork" />
            <parallelGateway id="reviewFork" />
            <sequenceFlow id="reviewFlow2" sourceRef="reviewFork" targetRef="reviewTask" />
            <sequenceFlow id="reviewFlow3" sourceRef="reviewFork" targetRef="throwingEnd" />
            <userTask id="reviewTask" name="Review Task" />
            <sequenceFlow id="reviewFlow4" sourceRef="reviewTask" targetRef="reviewEnd" />
            <endEvent id="reviewEnd" />
            <endEvent id="throwingEnd">
                <errorEventDefinition errorCode="BUSINESS_ERROR" />
            </endEvent>

            <subProcess id="reviewErrorEventSubProcess" triggeredByEvent="true">
                <startEvent id="reviewErrorStart" isInterrupting="true">
                    <errorEventDefinition errorCode="BUSINESS_ERROR" />
                </startEvent>
                <sequenceFlow id="reviewErrorFlow1" sourceRef="reviewErrorStart" targetRef="reviewErrorTask" />
                <userTask id="reviewErrorTask" name="Review Error Task" />
                <sequenceFlow id="reviewErrorFlow2" sourceRef="reviewErrorTask" targetRef="reviewErrorEnd" />
                <endEvent id="reviewErrorEnd" />
            </subProcess>
        </subProcess>
    </process>
</definitions>"#;

#[test]
fn test_embedded_escalation_event_subprocess_starts_from_end_escalation() {
    let process_engine =
        ProcessEngine::new("embedded-end-escalation-event-subprocess-test".to_string());
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();
    let task_service = process_engine.get_task_service();

    let deployment_builder = repository_service
        .create_deployment()
        .name("Embedded End Escalation Event Subprocess Deployment".to_string())
        .add_string(
            "embedded_end_escalation_event_subprocess.bpmn20.xml".to_string(),
            EMBEDDED_END_ESCALATION_EVENT_SUBPROCESS_XML.to_string(),
        );
    repository_service.deploy(deployment_builder).unwrap();

    let process_def_id = repository_service.get_process_definition_ids().unwrap()[0].clone();

    let builder = runtime_service
        .create_process_instance_builder()
        .process_definition_id(process_def_id);
    let process_instance = runtime_service.start_process_instance(builder).unwrap();

    let task_keys = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap()
        .into_iter()
        .map(|task| task.task_definition_key)
        .collect::<Vec<_>>();

    assert_eq!(
        task_keys,
        vec!["reviewEscalatedTask".to_string()],
        "an interrupting escalation event subprocess should cancel the original scope task when started from an escalation end event"
    );
}

#[test]
fn test_embedded_error_event_subprocess_starts_from_error_end_and_cancels_scope() {
    let process_engine = ProcessEngine::new("embedded-end-error-event-subprocess-test".to_string());
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();
    let task_service = process_engine.get_task_service();

    let deployment_builder = repository_service
        .create_deployment()
        .name("Embedded End Error Event Subprocess Deployment".to_string())
        .add_string(
            "embedded_end_error_event_subprocess.bpmn20.xml".to_string(),
            EMBEDDED_END_ERROR_EVENT_SUBPROCESS_XML.to_string(),
        );
    repository_service.deploy(deployment_builder).unwrap();

    let process_def_id = repository_service.get_process_definition_ids().unwrap()[0].clone();

    let builder = runtime_service
        .create_process_instance_builder()
        .process_definition_id(process_def_id);
    let process_instance = runtime_service.start_process_instance(builder).unwrap();

    let task_keys = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap()
        .into_iter()
        .map(|task| task.task_definition_key)
        .collect::<Vec<_>>();

    assert_eq!(
        task_keys,
        vec!["reviewErrorTask".to_string()],
        "an interrupting error event subprocess should catch the error end event and cancel the original scope task"
    );

    let __runtime_store = process_engine.get_runtime_store();
    let mut __runtime_session = __runtime_store.create_session().unwrap();
    let executions = __runtime_store.snapshot_executions(&mut __runtime_session);
    __runtime_session.rollback().unwrap();
    assert!(
        executions
            .values()
            .all(|execution| execution.activity_id.as_deref() != Some("reviewTask")),
        "the interrupted scope should not leave the original review task execution active"
    );
}

#[test]
fn test_non_interrupting_embedded_escalation_event_subprocess_preserves_host_task() {
    let process_engine = ProcessEngine::new(
        "non-interrupting-embedded-escalation-event-subprocess-test".to_string(),
    );
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();
    let task_service = process_engine.get_task_service();
    let xml = EMBEDDED_END_ESCALATION_EVENT_SUBPROCESS_XML
        .replace(r#"isInterrupting="true""#, r#"isInterrupting="false""#);

    let deployment_builder = repository_service
        .create_deployment()
        .name("Non Interrupting Embedded Escalation Event Subprocess Deployment".to_string())
        .add_string(
            "non_interrupting_embedded_escalation_event_subprocess.bpmn20.xml".to_string(),
            xml,
        );
    repository_service.deploy(deployment_builder).unwrap();

    let process_def_id = repository_service.get_process_definition_ids().unwrap()[0].clone();

    let builder = runtime_service
        .create_process_instance_builder()
        .process_definition_id(process_def_id);
    let process_instance = runtime_service.start_process_instance(builder).unwrap();

    let mut task_keys = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap()
        .into_iter()
        .map(|task| task.task_definition_key)
        .collect::<Vec<_>>();
    task_keys.sort();

    assert_eq!(
        task_keys,
        vec!["reviewEscalatedTask".to_string(), "reviewTask".to_string()],
        "a non-interrupting escalation event subprocess should start its path and preserve the host task in the same scope"
    );
}

#[test]
fn test_non_interrupting_embedded_escalation_event_subprocess_stays_in_throwing_scope() {
    let process_engine = ProcessEngine::new(
        "non-interrupting-embedded-escalation-event-subprocess-scope-test".to_string(),
    );
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();
    let task_service = process_engine.get_task_service();
    let xml = EMBEDDED_ESCALATION_EVENT_SUBPROCESS_XML
        .replace(r#"isInterrupting="true""#, r#"isInterrupting="false""#);

    let deployment_builder = repository_service
        .create_deployment()
        .name("Non Interrupting Scoped Escalation Event Subprocess Deployment".to_string())
        .add_string(
            "non_interrupting_scoped_escalation_event_subprocess.bpmn20.xml".to_string(),
            xml,
        );
    repository_service.deploy(deployment_builder).unwrap();

    let process_def_id = repository_service.get_process_definition_ids().unwrap()[0].clone();

    let builder = runtime_service
        .create_process_instance_builder()
        .process_definition_id(process_def_id);
    let process_instance = runtime_service.start_process_instance(builder).unwrap();

    let mut task_keys = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap()
        .into_iter()
        .map(|task| task.task_definition_key)
        .collect::<Vec<_>>();
    task_keys.sort();

    assert_eq!(
        task_keys,
        vec![
            "leftHostTask".to_string(),
            "rightEscalatedTask".to_string(),
            "rightHostTask".to_string()
        ],
        "the non-interrupting escalation event subprocess should preserve the throwing scope host task and must not trigger the sibling scope"
    );
}

const REPEATABLE_ESCALATION_EVENT_SUBPROCESS_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
    <escalation id="approvalEscalation" escalationCode="APPROVAL_TIMEOUT" />
    <process id="repeatableEscalationEventSubprocessProcess" isExecutable="true">
        <startEvent id="startEvent" />
        <sequenceFlow id="flow1" sourceRef="startEvent" targetRef="reviewSubProcess" />

        <subProcess id="reviewSubProcess">
            <startEvent id="reviewStart" />
            <sequenceFlow id="reviewFlow1" sourceRef="reviewStart" targetRef="reviewTask" />
            <userTask id="reviewTask" name="Review Task" />
            <sequenceFlow id="reviewFlow2" sourceRef="reviewTask" targetRef="reviewEnd" />
            <endEvent id="reviewEnd" />

            <subProcess id="reviewEscalationEventSubProcess" triggeredByEvent="true">
                <startEvent id="reviewEscalationStart" isInterrupting="false">
                    <escalationEventDefinition escalationRef="approvalEscalation" />
                </startEvent>
                <sequenceFlow id="reviewEscalationFlow1" sourceRef="reviewEscalationStart" targetRef="reviewEscalatedTask" />
                <userTask id="reviewEscalatedTask" name="Review Escalated Task" />
                <sequenceFlow id="reviewEscalationFlow2" sourceRef="reviewEscalatedTask" targetRef="reviewEscalationEnd" />
                <endEvent id="reviewEscalationEnd" />
            </subProcess>
        </subProcess>

        <sequenceFlow id="flow2" sourceRef="reviewSubProcess" targetRef="afterReviewTask" />
        <userTask id="afterReviewTask" name="After Review Task" />
        <sequenceFlow id="flow3" sourceRef="afterReviewTask" targetRef="endEvent" />
        <endEvent id="endEvent" />
    </process>
</definitions>"#;

#[test]
fn test_non_interrupting_escalation_event_subprocess_subscription_repeats_until_host_scope_ends() {
    let process_engine =
        ProcessEngine::new("repeatable-escalation-event-subprocess-test".to_string());
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();
    let task_service = process_engine.get_task_service();
    let history_service = process_engine.get_history_service();
    let command_executor = process_engine.get_command_executor();

    let deployment_builder = repository_service
        .create_deployment()
        .name("Repeatable Escalation Event Subprocess Deployment".to_string())
        .add_string(
            "repeatable_escalation_event_subprocess.bpmn20.xml".to_string(),
            REPEATABLE_ESCALATION_EVENT_SUBPROCESS_XML.to_string(),
        );
    repository_service.deploy(deployment_builder).unwrap();

    let process_def_id = repository_service.get_process_definition_ids().unwrap()[0].clone();

    let builder = runtime_service
        .create_process_instance_builder()
        .process_definition_id(process_def_id);
    let process_instance = runtime_service.start_process_instance(builder).unwrap();

    let task_keys = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap()
        .into_iter()
        .map(|task| task.task_definition_key)
        .collect::<Vec<_>>();
    assert_eq!(task_keys, vec!["reviewTask".to_string()]);

    let first_trigger = TriggerEventSubprocessByEventCmd::new(
        EventSubscriptionKind::Escalation,
        "APPROVAL_TIMEOUT".to_string(),
        process_instance.id.clone(),
    );
    let first_triggered = command_executor.execute(&first_trigger).unwrap();
    assert_eq!(
        first_triggered.len(),
        1,
        "first escalation should activate the event subprocess"
    );

    let runtime_store = process_engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let subscriptions_after_first = runtime_store
        .find_event_subprocess_event_subscriptions_by_process_instance_id(
            &process_instance.id,
            &mut session,
        )
        .into_iter()
        .filter(|subscription| subscription.event_kind == EventSubscriptionKind::Escalation)
        .collect::<Vec<_>>();
    session.rollback().unwrap();
    assert_eq!(
        subscriptions_after_first.len(),
        1,
        "non-interrupting escalation event subprocess subscription must remain while the host scope is active"
    );

    let second_trigger = TriggerEventSubprocessByEventCmd::new(
        EventSubscriptionKind::Escalation,
        "APPROVAL_TIMEOUT".to_string(),
        process_instance.id.clone(),
    );
    let second_triggered = command_executor.execute(&second_trigger).unwrap();
    assert_eq!(
        second_triggered.len(),
        1,
        "second escalation in the same host scope should activate the same event subprocess again"
    );

    let mut task_keys = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap()
        .into_iter()
        .map(|task| task.task_definition_key)
        .collect::<Vec<_>>();
    task_keys.sort();

    assert_eq!(
        task_keys,
        vec![
            "reviewEscalatedTask".to_string(),
            "reviewEscalatedTask".to_string(),
            "reviewTask".to_string()
        ],
        "the host task should remain and two event subprocess paths should be active"
    );

    let escalated_history_count = history_service
        .create_historic_activity_instance_query()
        .process_instance_id(process_instance.id.clone())
        .list()
        .unwrap()
        .into_iter()
        .filter(|activity| activity.activity_id == "reviewEscalatedTask")
        .count();
    assert_eq!(
        escalated_history_count, 2,
        "history should contain one activity instance for each event subprocess path"
    );

    let active_tasks = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    for task in active_tasks
        .iter()
        .filter(|task| task.task_definition_key == "reviewEscalatedTask")
    {
        task_service.complete_task_by_id(task.id.clone()).unwrap();
    }

    let tasks_after_event_paths_end = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(
        tasks_after_event_paths_end.len(),
        1,
        "ending non-interrupting event subprocess paths must not advance the host subprocess"
    );
    assert_eq!(
        tasks_after_event_paths_end[0].task_definition_key, "reviewTask",
        "host subprocess task should keep waiting after the event subprocess path ends"
    );

    let runtime_store = process_engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let process_instance_after_event_paths_end = runtime_store
        .find_process_instance(&process_instance.id, &mut session)
        .expect("process instance should still exist");
    session.rollback().unwrap();
    assert!(
        !process_instance_after_event_paths_end.is_ended,
        "process must not end while the host subprocess task is still waiting"
    );

    let review_task = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap()
        .into_iter()
        .find(|task| task.task_definition_key == "reviewTask")
        .expect("review task should still be active until host scope completes");
    task_service.complete_task_by_id(review_task.id).unwrap();

    let runtime_store = process_engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let subscriptions_after_host_scope = runtime_store
        .find_event_subprocess_event_subscriptions_by_process_instance_id(
            &process_instance.id,
            &mut session,
        )
        .into_iter()
        .filter(|subscription| subscription.event_kind == EventSubscriptionKind::Escalation)
        .collect::<Vec<_>>();
    session.rollback().unwrap();
    assert!(
        subscriptions_after_host_scope.is_empty(),
        "event subprocess subscription should be removed when the host scope completes"
    );

    let third_trigger = TriggerEventSubprocessByEventCmd::new(
        EventSubscriptionKind::Escalation,
        "APPROVAL_TIMEOUT".to_string(),
        process_instance.id.clone(),
    );
    let third_triggered = command_executor.execute(&third_trigger).unwrap();
    assert!(
        third_triggered.is_empty(),
        "completed host scope should not receive later escalations"
    );

    let task_keys = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap()
        .into_iter()
        .map(|task| task.task_definition_key)
        .collect::<Vec<_>>();
    assert_eq!(
        task_keys,
        vec!["afterReviewTask".to_string()],
        "process should continue outside the completed host scope without a third event subprocess task"
    );
}

#[test]
fn test_interrupting_escalation_event_subprocess_subscription_is_consumed() {
    let process_engine =
        ProcessEngine::new("interrupting-escalation-event-subprocess-repeat-test".to_string());
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();
    let task_service = process_engine.get_task_service();
    let command_executor = process_engine.get_command_executor();
    let xml = REPEATABLE_ESCALATION_EVENT_SUBPROCESS_XML
        .replace(r#"isInterrupting="false""#, r#"isInterrupting="true""#);

    let deployment_builder = repository_service
        .create_deployment()
        .name("Interrupting Escalation Event Subprocess Repeat Deployment".to_string())
        .add_string(
            "interrupting_escalation_event_subprocess_repeat.bpmn20.xml".to_string(),
            xml,
        );
    repository_service.deploy(deployment_builder).unwrap();

    let process_def_id = repository_service.get_process_definition_ids().unwrap()[0].clone();

    let builder = runtime_service
        .create_process_instance_builder()
        .process_definition_id(process_def_id);
    let process_instance = runtime_service.start_process_instance(builder).unwrap();

    let first_trigger = TriggerEventSubprocessByEventCmd::new(
        EventSubscriptionKind::Escalation,
        "APPROVAL_TIMEOUT".to_string(),
        process_instance.id.clone(),
    );
    let first_triggered = command_executor.execute(&first_trigger).unwrap();
    assert_eq!(
        first_triggered.len(),
        1,
        "first escalation should activate the interrupting event subprocess"
    );

    let mut task_keys = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap()
        .into_iter()
        .map(|task| task.task_definition_key)
        .collect::<Vec<_>>();
    task_keys.sort();
    assert_eq!(
        task_keys,
        vec!["reviewEscalatedTask".to_string()],
        "interrupting escalation event subprocess should cancel the host task"
    );

    let second_trigger = TriggerEventSubprocessByEventCmd::new(
        EventSubscriptionKind::Escalation,
        "APPROVAL_TIMEOUT".to_string(),
        process_instance.id.clone(),
    );
    let second_triggered = command_executor.execute(&second_trigger).unwrap();
    assert!(
        second_triggered.is_empty(),
        "interrupting escalation event subprocess subscription should be consumed after triggering"
    );
}

#[test]
fn test_no_code_escalation_event_subprocess_catches_any_escalation() {
    let process_engine = ProcessEngine::new("no-code-escalation-event-subprocess-test".to_string());
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();
    let task_service = process_engine.get_task_service();
    let command_executor = process_engine.get_command_executor();

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
    <escalation id="approvalEscalation" escalationCode="APPROVAL_TIMEOUT" />
    <process id="noCodeEscalationEventSubprocessProcess" isExecutable="true">
        <startEvent id="startEvent" />
        <sequenceFlow id="flow1" sourceRef="startEvent" targetRef="reviewSubProcess" />

        <subProcess id="reviewSubProcess">
            <startEvent id="reviewStart" />
            <sequenceFlow id="reviewFlow1" sourceRef="reviewStart" targetRef="reviewTask" />
            <userTask id="reviewTask" name="Review Task" />
            <sequenceFlow id="reviewFlow2" sourceRef="reviewTask" targetRef="reviewEnd" />
            <endEvent id="reviewEnd" />

            <subProcess id="catchAllEscalationEventSubProcess" triggeredByEvent="true">
                <startEvent id="catchAllEscalationStart" isInterrupting="false">
                    <escalationEventDefinition />
                </startEvent>
                <sequenceFlow id="catchAllFlow1" sourceRef="catchAllEscalationStart" targetRef="catchAllEscalatedTask" />
                <userTask id="catchAllEscalatedTask" name="Catch All Escalated Task" />
                <sequenceFlow id="catchAllFlow2" sourceRef="catchAllEscalatedTask" targetRef="catchAllEnd" />
                <endEvent id="catchAllEnd" />
            </subProcess>

            <subProcess id="otherEscalationEventSubProcess" triggeredByEvent="true">
                <startEvent id="otherEscalationStart" isInterrupting="false">
                    <escalationEventDefinition escalationCode="OTHER_TIMEOUT" />
                </startEvent>
                <sequenceFlow id="otherFlow1" sourceRef="otherEscalationStart" targetRef="otherEscalatedTask" />
                <userTask id="otherEscalatedTask" name="Other Escalated Task" />
                <sequenceFlow id="otherFlow2" sourceRef="otherEscalatedTask" targetRef="otherEnd" />
                <endEvent id="otherEnd" />
            </subProcess>
        </subProcess>
    </process>
</definitions>"#;

    let deployment_builder = repository_service
        .create_deployment()
        .name("No Code Escalation Event Subprocess Deployment".to_string())
        .add_string(
            "no_code_escalation_event_subprocess.bpmn20.xml".to_string(),
            xml.to_string(),
        );
    repository_service.deploy(deployment_builder).unwrap();

    let process_def_id = repository_service.get_process_definition_ids().unwrap()[0].clone();
    let process_instance = runtime_service
        .start_process_instance(
            runtime_service
                .create_process_instance_builder()
                .process_definition_id(process_def_id),
        )
        .unwrap();

    let trigger = TriggerEventSubprocessByEventCmd::new(
        EventSubscriptionKind::Escalation,
        "APPROVAL_TIMEOUT".to_string(),
        process_instance.id.clone(),
    );
    let triggered = command_executor.execute(&trigger).unwrap();
    assert_eq!(
        triggered.len(),
        1,
        "a no-code escalation start event should catch the thrown escalation while the different coded catch should not"
    );

    let mut task_keys = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap()
        .into_iter()
        .map(|task| task.task_definition_key)
        .collect::<Vec<_>>();
    task_keys.sort();
    assert_eq!(
        task_keys,
        vec![
            "catchAllEscalatedTask".to_string(),
            "reviewTask".to_string()
        ]
    );
}

#[test]
fn test_escalation_event_subprocess_prefers_exact_code_over_no_code_in_same_scope() {
    let process_engine = ProcessEngine::new(
        "exact-code-before-catch-all-escalation-event-subprocess-test".to_string(),
    );
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();
    let task_service = process_engine.get_task_service();
    let command_executor = process_engine.get_command_executor();

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
    <escalation id="approvalEscalation" escalationCode="APPROVAL_TIMEOUT" />
    <process id="exactEscalationEventSubprocessProcess" isExecutable="true">
        <startEvent id="startEvent" />
        <sequenceFlow id="flow1" sourceRef="startEvent" targetRef="reviewSubProcess" />

        <subProcess id="reviewSubProcess">
            <startEvent id="reviewStart" />
            <sequenceFlow id="reviewFlow1" sourceRef="reviewStart" targetRef="reviewTask" />
            <userTask id="reviewTask" name="Review Task" />
            <sequenceFlow id="reviewFlow2" sourceRef="reviewTask" targetRef="reviewEnd" />
            <endEvent id="reviewEnd" />

            <subProcess id="catchAllEscalationEventSubProcess" triggeredByEvent="true">
                <startEvent id="catchAllEscalationStart" isInterrupting="false">
                    <escalationEventDefinition />
                </startEvent>
                <sequenceFlow id="catchAllFlow1" sourceRef="catchAllEscalationStart" targetRef="catchAllEscalatedTask" />
                <userTask id="catchAllEscalatedTask" name="Catch All Escalated Task" />
                <sequenceFlow id="catchAllFlow2" sourceRef="catchAllEscalatedTask" targetRef="catchAllEnd" />
                <endEvent id="catchAllEnd" />
            </subProcess>

            <subProcess id="exactEscalationEventSubProcess" triggeredByEvent="true">
                <startEvent id="exactEscalationStart" isInterrupting="false">
                    <escalationEventDefinition escalationRef="approvalEscalation" />
                </startEvent>
                <sequenceFlow id="exactFlow1" sourceRef="exactEscalationStart" targetRef="exactEscalatedTask" />
                <userTask id="exactEscalatedTask" name="Exact Escalated Task" />
                <sequenceFlow id="exactFlow2" sourceRef="exactEscalatedTask" targetRef="exactEnd" />
                <endEvent id="exactEnd" />
            </subProcess>
        </subProcess>
    </process>
</definitions>"#;

    repository_service
        .deploy(
            repository_service
                .create_deployment()
                .name("Exact Escalation Event Subprocess Deployment".to_string())
                .add_string(
                    "exact_escalation_event_subprocess.bpmn20.xml".to_string(),
                    xml.to_string(),
                ),
        )
        .unwrap();

    let process_def_id = repository_service.get_process_definition_ids().unwrap()[0].clone();
    let process_instance = runtime_service
        .start_process_instance(
            runtime_service
                .create_process_instance_builder()
                .process_definition_id(process_def_id),
        )
        .unwrap();

    let trigger = TriggerEventSubprocessByEventCmd::new(
        EventSubscriptionKind::Escalation,
        "APPROVAL_TIMEOUT".to_string(),
        process_instance.id.clone(),
    );
    let triggered = command_executor.execute(&trigger).unwrap();
    assert_eq!(
        triggered.len(),
        1,
        "the coded escalation should activate exactly one event subprocess"
    );

    let mut task_keys = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap()
        .into_iter()
        .map(|task| task.task_definition_key)
        .collect::<Vec<_>>();
    task_keys.sort();
    assert_eq!(
        task_keys,
        vec!["exactEscalatedTask".to_string(), "reviewTask".to_string()],
        "same-scope exact escalation catch must win over the no-code catch-all"
    );
}

const PROCESS_SCOPE_ESCALATION_EVENT_SUBPROCESS_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
    <escalation id="approvalEscalation" escalationCode="APPROVAL_TIMEOUT" />
    <process id="processScopeEscalationEventSubprocessProcess" isExecutable="true">
        <startEvent id="startEvent" />
        <sequenceFlow id="flow1" sourceRef="startEvent" targetRef="mainTask" />
        <userTask id="mainTask" name="Main Task" />
        <sequenceFlow id="flow2" sourceRef="mainTask" targetRef="endEvent" />
        <endEvent id="endEvent" />

        <subProcess id="processEscalationEventSubProcess" triggeredByEvent="true">
            <startEvent id="processEscalationStart" isInterrupting="false">
                <escalationEventDefinition escalationRef="approvalEscalation" />
            </startEvent>
            <sequenceFlow id="processEscalationFlow1" sourceRef="processEscalationStart" targetRef="processEscalatedTask" />
            <userTask id="processEscalatedTask" name="Process Escalated Task" />
            <sequenceFlow id="processEscalationFlow2" sourceRef="processEscalatedTask" targetRef="processEscalationEnd" />
            <endEvent id="processEscalationEnd" />
        </subProcess>
    </process>
</definitions>"#;

#[test]
fn test_process_scope_non_interrupting_escalation_event_subprocess_repeats_while_process_active() {
    let process_engine =
        ProcessEngine::new("process-scope-repeatable-escalation-event-subprocess-test".to_string());
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();
    let task_service = process_engine.get_task_service();
    let command_executor = process_engine.get_command_executor();

    repository_service
        .deploy(
            repository_service
                .create_deployment()
                .name("Process Scope Escalation Event Subprocess Deployment".to_string())
                .add_string(
                    "process_scope_escalation_event_subprocess.bpmn20.xml".to_string(),
                    PROCESS_SCOPE_ESCALATION_EVENT_SUBPROCESS_XML.to_string(),
                ),
        )
        .unwrap();

    let process_def_id = repository_service.get_process_definition_ids().unwrap()[0].clone();
    let process_instance = runtime_service
        .start_process_instance(
            runtime_service
                .create_process_instance_builder()
                .process_definition_id(process_def_id),
        )
        .unwrap();

    let runtime_store = process_engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let subscriptions = runtime_store
        .find_event_subprocess_event_subscriptions_by_process_instance_id(
            &process_instance.id,
            &mut session,
        )
        .into_iter()
        .filter(|subscription| subscription.event_kind == EventSubscriptionKind::Escalation)
        .collect::<Vec<_>>();
    session.rollback().unwrap();
    assert_eq!(
        subscriptions.len(),
        1,
        "the process-scope non-interrupting escalation event subprocess should be subscribed while the process is active"
    );

    for _ in 0..2 {
        let trigger = TriggerEventSubprocessByEventCmd::new(
            EventSubscriptionKind::Escalation,
            "APPROVAL_TIMEOUT".to_string(),
            process_instance.id.clone(),
        );
        let triggered = command_executor.execute(&trigger).unwrap();
        assert_eq!(
            triggered.len(),
            1,
            "each escalation should activate the process-scope non-interrupting event subprocess while the process is active"
        );
    }

    let mut task_keys = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap()
        .into_iter()
        .map(|task| task.task_definition_key)
        .collect::<Vec<_>>();
    task_keys.sort();
    assert_eq!(
        task_keys,
        vec![
            "mainTask".to_string(),
            "processEscalatedTask".to_string(),
            "processEscalatedTask".to_string()
        ],
        "the host task should remain and both event subprocess paths should be active"
    );

    let active_tasks = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    for task in active_tasks
        .iter()
        .filter(|task| task.task_definition_key == "processEscalatedTask")
    {
        task_service.complete_task_by_id(task.id.clone()).unwrap();
    }

    let tasks_after_event_paths_end = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(
        tasks_after_event_paths_end.len(),
        1,
        "ending process-scope non-interrupting event subprocess paths must not advance the host process"
    );
    assert_eq!(
        tasks_after_event_paths_end[0].task_definition_key, "mainTask",
        "host process task should keep waiting after the event subprocess path ends"
    );

    let runtime_store = process_engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let process_instance_after_event_paths_end = runtime_store
        .find_process_instance(&process_instance.id, &mut session)
        .expect("process instance should still exist");
    session.rollback().unwrap();
    assert!(
        !process_instance_after_event_paths_end.is_ended,
        "process must not end while the host process task is still waiting"
    );
}

#[test]
fn test_process_scope_non_interrupting_escalation_event_subprocess_subscription_clears_when_process_ends()
 {
    let process_engine =
        ProcessEngine::new("process-scope-ended-escalation-event-subprocess-test".to_string());
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();
    let task_service = process_engine.get_task_service();
    let command_executor = process_engine.get_command_executor();

    repository_service
        .deploy(
            repository_service
                .create_deployment()
                .name("Process Scope Escalation Event Subprocess Cleanup Deployment".to_string())
                .add_string(
                    "process_scope_escalation_event_subprocess_cleanup.bpmn20.xml".to_string(),
                    PROCESS_SCOPE_ESCALATION_EVENT_SUBPROCESS_XML.to_string(),
                ),
        )
        .unwrap();

    let process_def_id = repository_service.get_process_definition_ids().unwrap()[0].clone();
    let process_instance = runtime_service
        .start_process_instance(
            runtime_service
                .create_process_instance_builder()
                .process_definition_id(process_def_id),
        )
        .unwrap();

    let runtime_store = process_engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let subscriptions_before_end = runtime_store
        .find_event_subprocess_event_subscriptions_by_process_instance_id(
            &process_instance.id,
            &mut session,
        )
        .into_iter()
        .filter(|subscription| subscription.event_kind == EventSubscriptionKind::Escalation)
        .collect::<Vec<_>>();
    session.rollback().unwrap();
    assert_eq!(
        subscriptions_before_end.len(),
        1,
        "the process-scope escalation event subprocess should be subscribed before the process ends"
    );

    let main_task = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap()
        .into_iter()
        .find(|task| task.task_definition_key == "mainTask")
        .expect("main task should still be active until the process completes");
    task_service.complete_task_by_id(main_task.id).unwrap();

    let runtime_store = process_engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let process_instance_after = runtime_store
        .find_process_instance(&process_instance.id, &mut session)
        .expect("process instance should still be queryable");
    session.rollback().unwrap();
    assert!(
        process_instance_after.is_ended,
        "the process instance should be ended after its main path completes"
    );

    let runtime_store = process_engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let subscriptions_after_end = runtime_store
        .find_event_subprocess_event_subscriptions_by_process_instance_id(
            &process_instance.id,
            &mut session,
        )
        .into_iter()
        .filter(|subscription| subscription.event_kind == EventSubscriptionKind::Escalation)
        .collect::<Vec<_>>();
    session.rollback().unwrap();
    assert!(
        subscriptions_after_end.is_empty(),
        "process-scope escalation event subprocess subscriptions should be removed when the process ends"
    );

    let trigger_after_end = TriggerEventSubprocessByEventCmd::new(
        EventSubscriptionKind::Escalation,
        "APPROVAL_TIMEOUT".to_string(),
        process_instance.id.clone(),
    );
    let triggered_after_end = command_executor.execute(&trigger_after_end).unwrap();
    assert!(
        triggered_after_end.is_empty(),
        "an ended process must not receive later escalation event subprocess triggers"
    );
}

/// P44 regression: Java `StartEventParseHandler` (66-67) forces error start
/// events to interrupting=true regardless of the model's `isInterrupting`
/// flag, and `ErrorPropagation#executeCatch` (263-275) always destroys the
/// source scope. Even when the model declares `isInterrupting="false"` on an
/// error start event, the throwing scope must be cancelled — only the error
/// handler task should remain.
#[test]
fn test_error_start_event_forces_interrupting_even_when_model_says_non_interrupting() {
    let process_engine = ProcessEngine::new("error-start-forced-interrupting-test".to_string());
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();
    let task_service = process_engine.get_task_service();

    // Same XML as EMBEDDED_END_ERROR_EVENT_SUBPROCESS_XML but with the error
    // start event explicitly marked non-interrupting.
    let xml = EMBEDDED_END_ERROR_EVENT_SUBPROCESS_XML
        .replace(r#"isInterrupting="true""#, r#"isInterrupting="false""#);

    repository_service
        .deploy(
            repository_service
                .create_deployment()
                .name("Error Start Forced Interrupting Deployment".to_string())
                .add_string(
                    "error_start_forced_interrupting.bpmn20.xml".to_string(),
                    xml,
                ),
        )
        .unwrap();

    let process_def_id = repository_service.get_process_definition_ids().unwrap()[0].clone();

    let builder = runtime_service
        .create_process_instance_builder()
        .process_definition_id(process_def_id);
    let process_instance = runtime_service.start_process_instance(builder).unwrap();

    let task_keys = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap()
        .into_iter()
        .map(|task| task.task_definition_key)
        .collect::<Vec<_>>();

    assert_eq!(
        task_keys,
        vec!["reviewErrorTask".to_string()],
        "an error event subprocess must catch the error and cancel the original \
         scope task even when the model declares isInterrupting=\"false\""
    );

    let runtime_store = process_engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let executions = runtime_store.snapshot_executions(&mut session);
    session.rollback().unwrap();
    assert!(
        executions
            .values()
            .all(|execution| execution.activity_id.as_deref() != Some("reviewTask")),
        "the source scope must be destroyed even with isInterrupting=\"false\" \
         on an error start event"
    );
}
