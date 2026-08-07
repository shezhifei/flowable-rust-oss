//! P14 contract: error boundary events are always interrupting at runtime,
//! regardless of the model-level `cancelActivity` flag.
//!
//! Java splits the semantics:
//! - Model side: `BoundaryEventXMLConverter.java:86-93` forces
//!   `cancelActivity=false` when the boundary event has exactly one
//!   `ErrorEventDefinition` (size()==1 semantics).
//! - Runtime side: `ErrorEventDefinitionParseHandler.java:34` creates
//!   `BoundaryEventActivityBehavior(boundaryEvent, true)` — interrupting is
//!   hardcoded and the model flag is never read.
//!
//! Net effect in Java: when an error boundary catches, the host activity /
//! execution is always destroyed and only the boundary outgoing path remains.

use flowable_bpmn_converter::BpmnXMLConverter;
use flowable_bpmn_model::model::{EventDefinitionEnum, FlowElementEnum};
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::persistence::runtime_store::EventSubscriptionKind;

/// XML with explicit `cancelActivity="false"` on an error boundary attached to
/// a subprocess whose parallel branch keeps a live user task. After the error
/// throws, Java destroys the host subprocess (interrupting): the parallel
/// waiting task inside the subprocess must be gone, only the boundary path
/// task remains.
#[test]
fn p14_error_boundary_with_model_cancel_activity_false_still_interrupts_host() {
    let engine = ProcessEngine::new("p14-error-interrupting".to_string());
    let repository_service = engine.get_repository_service();
    let runtime_service = engine.get_runtime_service();
    let task_service = engine.get_task_service();

    // Subprocess forks: one branch waits on `waitTask`, the other throws err1.
    // If the boundary were honored as non-interrupting (model flag), waitTask
    // would survive the catch — Java kills it.
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <error id="err1" errorCode="E1" />
        <process id="p14ErrInterrupt" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="f1" sourceRef="start" targetRef="sub" />
            <subProcess id="sub">
                <startEvent id="subStart" />
                <sequenceFlow id="sf1" sourceRef="subStart" targetRef="fork" />
                <parallelGateway id="fork" />
                <sequenceFlow id="sf2" sourceRef="fork" targetRef="waitTask" />
                <sequenceFlow id="sf3" sourceRef="fork" targetRef="throwTask" />
                <userTask id="waitTask" />
                <userTask id="throwTask" />
                <sequenceFlow id="sf4" sourceRef="throwTask" targetRef="throwErr" />
                <sequenceFlow id="sf5" sourceRef="waitTask" targetRef="subEnd" />
                <endEvent id="throwErr">
                    <errorEventDefinition errorRef="err1" />
                </endEvent>
                <endEvent id="subEnd" />
            </subProcess>
            <boundaryEvent id="catchErr" attachedToRef="sub" cancelActivity="false">
                <errorEventDefinition errorRef="err1" />
            </boundaryEvent>
            <sequenceFlow id="f2" sourceRef="catchErr" targetRef="errTask" />
            <userTask id="errTask" />
            <sequenceFlow id="f3" sourceRef="errTask" targetRef="end" />
            <sequenceFlow id="f4" sourceRef="sub" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#;

    repository_service
        .deploy(
            repository_service
                .create_deployment()
                .name("p14-error-interrupting".to_string())
                .add_string(
                    "p14_error_interrupting.bpmn20.xml".to_string(),
                    xml.to_string(),
                ),
        )
        .unwrap();

    let def_id = repository_service.get_process_definition_ids().unwrap()[0].clone();
    let pi = runtime_service
        .start_process_instance(
            runtime_service
                .create_process_instance_builder()
                .process_definition_id(def_id),
        )
        .unwrap();

    // Runtime registration: interrupting despite model cancelActivity=false.
    let runtime_store = engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let states =
        runtime_store.find_boundary_event_states_by_process_instance_id(&pi.id, &mut session);
    assert_eq!(states.len(), 1);
    assert_eq!(
        states[0].event_subscription.kind,
        EventSubscriptionKind::Error
    );
    assert!(
        states[0].cancel_activity,
        "error boundary runtime state must be interrupting \
         (ErrorEventDefinitionParseHandler.java:34)"
    );
    drop(session);

    // Both parallel branches waiting.
    let tasks = task_service
        .get_tasks_by_process_instance_id(pi.id.clone())
        .unwrap();
    let mut keys: Vec<_> = tasks
        .iter()
        .map(|t| t.task_definition_key.clone())
        .collect();
    keys.sort();
    assert_eq!(keys, vec!["throwTask", "waitTask"]);

    // Throw the BPMN error from one branch.
    let throw_task = tasks
        .iter()
        .find(|t| t.task_definition_key == "throwTask")
        .unwrap();
    task_service
        .complete_task_by_id(throw_task.id.clone())
        .unwrap();

    // Java interrupting semantics: host subprocess destroyed → waitTask gone,
    // only the boundary path task remains.
    let tasks_after = task_service
        .get_tasks_by_process_instance_id(pi.id.clone())
        .unwrap();
    assert_eq!(
        tasks_after.len(),
        1,
        "host subprocess must be destroyed on error catch (interrupting), \
         got tasks: {:?}",
        tasks_after
            .iter()
            .map(|t| t.task_definition_key.clone())
            .collect::<Vec<_>>()
    );
    assert_eq!(tasks_after[0].task_definition_key, "errTask");

    // Host subprocess scope execution must be gone.
    let mut session = runtime_store.create_session().unwrap();
    let executions = runtime_store.snapshot_executions(&mut session);
    assert!(
        !executions.values().any(|e| {
            e.process_instance_id.as_deref() == Some(pi.id.as_str())
                && e.activity_id.as_deref() == Some("sub")
        }),
        "host subprocess scope execution must be deleted after interrupting catch"
    );
    assert!(
        !executions.values().any(|e| {
            e.process_instance_id.as_deref() == Some(pi.id.as_str())
                && e.activity_id.as_deref() == Some("waitTask")
        }),
        "waitTask execution inside the host scope must be deleted"
    );
    drop(session);

    // Boundary path completes the process.
    task_service
        .complete_task_by_id(tasks_after[0].id.clone())
        .unwrap();
    let mut session = runtime_store.create_session().unwrap();
    let pi_row = runtime_store
        .find_process_instance(&pi.id, &mut session)
        .unwrap();
    assert!(pi_row.is_ended);
}

// ─── Converter contract: model-side cancelActivity forcing ──────────────────

fn parse_boundary(xml: &str, boundary_id: &str) -> flowable_bpmn_model::model::BoundaryEvent {
    let converter = BpmnXMLConverter::new();
    let model = converter.try_convert_to_bpmn_model(xml).unwrap();
    let process = model.main_process.as_ref().unwrap();
    match process.flow_element_map.get(boundary_id) {
        Some(FlowElementEnum::BoundaryEvent(be)) => be.clone(),
        other => panic!("boundary event '{boundary_id}' not found: {other:?}"),
    }
}

fn error_boundary_xml(cancel_activity_attr: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <error id="err1" errorCode="E1" />
        <process id="p14Conv" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="f1" sourceRef="start" targetRef="hostTask" />
            <userTask id="hostTask" />
            <boundaryEvent id="catchErr" attachedToRef="hostTask"{cancel_activity_attr}>
                <errorEventDefinition errorRef="err1" />
            </boundaryEvent>
            <sequenceFlow id="f2" sourceRef="catchErr" targetRef="end" />
            <sequenceFlow id="f3" sourceRef="hostTask" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#
    )
}

/// Java `BoundaryEventXMLConverter.java:86-93`: model cancelActivity is forced
/// to false for a single-ErrorEventDefinition boundary regardless of the XML
/// attribute (true / false / absent).
#[test]
fn p14_converter_forces_model_cancel_activity_false_for_single_error_definition() {
    for attr in [
        "",
        r#" cancelActivity="true""#,
        r#" cancelActivity="false""#,
    ] {
        let boundary = parse_boundary(&error_boundary_xml(attr), "catchErr");
        assert!(
            matches!(
                boundary.event.event_definitions.as_slice(),
                [EventDefinitionEnum::ErrorEventDefinition(_)]
            ),
            "expected a single error event definition"
        );
        assert!(
            !boundary.cancel_activity,
            "model cancel_activity must be forced false for a single error \
             definition (XML attr: '{attr}')"
        );
    }
}

/// Negative case for the size()==1 semantics: with more than one event
/// definition Java does NOT force cancelActivity, the XML value is kept.
#[test]
fn p14_converter_keeps_cancel_activity_when_error_definition_is_not_alone() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <error id="err1" errorCode="E1" />
        <signal id="sig1" name="SIG" />
        <process id="p14ConvMulti" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="f1" sourceRef="start" targetRef="hostTask" />
            <userTask id="hostTask" />
            <boundaryEvent id="catchMulti" attachedToRef="hostTask" cancelActivity="true">
                <errorEventDefinition errorRef="err1" />
                <signalEventDefinition signalRef="sig1" />
            </boundaryEvent>
            <sequenceFlow id="f2" sourceRef="catchMulti" targetRef="end" />
            <sequenceFlow id="f3" sourceRef="hostTask" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#;

    let boundary = parse_boundary(xml, "catchMulti");
    assert_eq!(
        boundary.event.event_definitions.len(),
        2,
        "fixture must carry two event definitions"
    );
    assert!(
        boundary.cancel_activity,
        "with more than one event definition the model flag must not be forced \
         (Java size()==1 semantics)"
    );
}
