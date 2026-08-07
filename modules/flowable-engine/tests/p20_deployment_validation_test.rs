//! P20-C contract tests: deployment-time validation for cancel / compensate
//! event constraints, aligned with the Java process validators.
//!
//! Java evidence (flowable-process-validation):
//! - `BoundaryEventValidator` (:80): "boundary event with cancelEventDefinition
//!   only supported on transaction subprocesses"
//! - `BoundaryEventValidator` (:137): "multiple boundary events with
//!   cancelEventDefinition not supported on same transaction subprocess."
//! - `BoundaryEventValidator` (:143): "Multiple boundary events of type
//!   'compensate' is invalid"
//! - `EndEventValidator` (:46): "end event with cancelEventDefinition only
//!   supported inside transaction subprocess"
//! - `EventValidator` (:102): "Invalid attribute value for 'activityRef':
//!   no activity with the given id"

use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::service::config::ProcessEngineConfiguration;

fn deploy_xml(
    xml: &str,
    config: ProcessEngineConfiguration,
) -> Result<(), flowable_engine::error::FlowableError> {
    let process_engine = ProcessEngine::new_with_config("default".to_string(), config);
    let repository_service = process_engine.get_repository_service();

    let builder = repository_service
        .create_deployment()
        .name("P20 Validation Test Deployment".to_string())
        .add_string("test_process.bpmn20.xml".to_string(), xml.to_string());

    repository_service.deploy(builder).map(|_| ())
}

#[test]
fn test_multiple_cancel_boundary_events_on_same_transaction_fail() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="p20MultiCancelBoundary" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="f1" sourceRef="start" targetRef="tx" />
            <transaction id="tx">
                <startEvent id="txStart" />
                <sequenceFlow id="tf1" sourceRef="txStart" targetRef="txEnd" />
                <endEvent id="txEnd">
                    <cancelEventDefinition />
                </endEvent>
            </transaction>
            <boundaryEvent id="catchCancel1" attachedToRef="tx">
                <cancelEventDefinition />
            </boundaryEvent>
            <boundaryEvent id="catchCancel2" attachedToRef="tx">
                <cancelEventDefinition />
            </boundaryEvent>
            <sequenceFlow id="f2" sourceRef="tx" targetRef="end" />
            <sequenceFlow id="f3" sourceRef="catchCancel1" targetRef="end" />
            <sequenceFlow id="f4" sourceRef="catchCancel2" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#;

    let result = deploy_xml(xml, ProcessEngineConfiguration::default());
    assert!(result.is_err());
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains(
            "multiple boundary events with cancelEventDefinition not supported on same transaction subprocess."
        ),
        "unexpected error message: {err_msg}"
    );
}

#[test]
fn test_cancel_boundary_event_on_non_transaction_fails() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="p20CancelBoundaryOnTask" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="f1" sourceRef="start" targetRef="task1" />
            <userTask id="task1" />
            <boundaryEvent id="catchCancel" attachedToRef="task1">
                <cancelEventDefinition />
            </boundaryEvent>
            <sequenceFlow id="f2" sourceRef="task1" targetRef="end" />
            <sequenceFlow id="f3" sourceRef="catchCancel" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#;

    let result = deploy_xml(xml, ProcessEngineConfiguration::default());
    assert!(result.is_err());
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains(
            "boundary event with cancelEventDefinition only supported on transaction subprocesses"
        ),
        "unexpected error message: {err_msg}"
    );
}

#[test]
fn test_cancel_end_event_outside_transaction_fails() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="p20CancelEndOutsideTx" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="f1" sourceRef="start" targetRef="cancelEnd" />
            <endEvent id="cancelEnd">
                <cancelEventDefinition />
            </endEvent>
        </process>
    </definitions>"#;

    let result = deploy_xml(xml, ProcessEngineConfiguration::default());
    assert!(result.is_err());
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains(
            "end event with cancelEventDefinition only supported inside transaction subprocess"
        ),
        "unexpected error message: {err_msg}"
    );
}

#[test]
fn test_cancel_end_event_in_plain_subprocess_fails() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="p20CancelEndInSubProcess" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="f1" sourceRef="start" targetRef="sub" />
            <subProcess id="sub">
                <startEvent id="subStart" />
                <sequenceFlow id="sf1" sourceRef="subStart" targetRef="subCancelEnd" />
                <endEvent id="subCancelEnd">
                    <cancelEventDefinition />
                </endEvent>
            </subProcess>
            <sequenceFlow id="f2" sourceRef="sub" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#;

    let result = deploy_xml(xml, ProcessEngineConfiguration::default());
    assert!(result.is_err());
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains(
            "end event with cancelEventDefinition only supported inside transaction subprocess"
        ),
        "unexpected error message: {err_msg}"
    );
}

#[test]
fn test_multiple_compensate_boundary_events_on_same_activity_fail() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="p20MultiCompensateBoundary" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="f1" sourceRef="start" targetRef="bookHotel" />
            <userTask id="bookHotel" />
            <boundaryEvent id="comp1" attachedToRef="bookHotel">
                <compensateEventDefinition />
            </boundaryEvent>
            <boundaryEvent id="comp2" attachedToRef="bookHotel">
                <compensateEventDefinition />
            </boundaryEvent>
            <userTask id="undoBookHotel" isForCompensation="true" />
            <sequenceFlow id="f2" sourceRef="bookHotel" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#;

    let result = deploy_xml(xml, ProcessEngineConfiguration::default());
    assert!(result.is_err());
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("Multiple boundary events of type 'compensate' is invalid"),
        "unexpected error message: {err_msg}"
    );
}

#[test]
fn test_compensate_throw_with_invalid_activity_ref_fails() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="p20InvalidActivityRef" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="f1" sourceRef="start" targetRef="bookHotel" />
            <userTask id="bookHotel" />
            <boundaryEvent id="comp1" attachedToRef="bookHotel">
                <compensateEventDefinition />
            </boundaryEvent>
            <userTask id="undoBookHotel" isForCompensation="true" />
            <sequenceFlow id="f2" sourceRef="bookHotel" targetRef="throwComp" />
            <intermediateThrowEvent id="throwComp">
                <compensateEventDefinition activityRef="doesNotExist" />
            </intermediateThrowEvent>
            <sequenceFlow id="f3" sourceRef="throwComp" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#;

    let result = deploy_xml(xml, ProcessEngineConfiguration::default());
    assert!(result.is_err());
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg
            .contains("Invalid attribute value for 'activityRef': no activity with the given id"),
        "unexpected error message: {err_msg}"
    );
}

/// Positive guard: a legal transaction model with a cancel boundary, a cancel
/// end event, compensation boundaries and a compensate throw referencing a
/// NESTED activity must still deploy.
#[test]
fn test_valid_cancel_and_compensate_model_deploys() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="p20ValidModel" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="f1" sourceRef="start" targetRef="tx" />
            <transaction id="tx">
                <startEvent id="txStart" />
                <sequenceFlow id="tf1" sourceRef="txStart" targetRef="bookHotel" />
                <userTask id="bookHotel" />
                <boundaryEvent id="bookHotelComp" attachedToRef="bookHotel">
                    <compensateEventDefinition />
                </boundaryEvent>
                <userTask id="undoBookHotel" isForCompensation="true" />
                <sequenceFlow id="tf2" sourceRef="bookHotel" targetRef="txCancelEnd" />
                <endEvent id="txCancelEnd">
                    <cancelEventDefinition />
                </endEvent>
            </transaction>
            <boundaryEvent id="catchCancel" attachedToRef="tx">
                <cancelEventDefinition />
            </boundaryEvent>
            <sequenceFlow id="f2" sourceRef="catchCancel" targetRef="throwComp" />
            <intermediateThrowEvent id="throwComp">
                <compensateEventDefinition activityRef="bookHotel" />
            </intermediateThrowEvent>
            <sequenceFlow id="f3" sourceRef="throwComp" targetRef="end" />
            <sequenceFlow id="f4" sourceRef="tx" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#;

    let result = deploy_xml(xml, ProcessEngineConfiguration::default());
    assert!(
        result.is_ok(),
        "legal cancel/compensate transaction model must deploy: {:?}",
        result.err()
    );
}
