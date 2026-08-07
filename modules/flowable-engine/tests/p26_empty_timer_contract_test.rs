//! P26 contract tests: empty `<timerEventDefinition />` (no timeDate /
//! timeCycle / timeDuration) must hard-fail instead of silently inserting a
//! never-firing timer.
//!
//! Java evidence:
//! - `EventValidator.java:89-93` (flowable-process-validation): deployment
//!   validation adds `EVENT_TIMER_MISSING_CONFIGURATION` — "Timer needs
//!   configuration (either timeDate, timeCycle or timeDuration is needed)".
//! - `TimerUtil.java:152-155` (flowable-engine): runtime safety net throws
//!   `FlowableException("Timer needs configuration (either timeDate, timeCycle
//!   or timeDuration is needed) (...)")`, rolling back the command.

use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::service::config::ProcessEngineConfiguration;

const EXPECTED_MESSAGE: &str =
    "Timer needs configuration (either timeDate, timeCycle or timeDuration is needed)";

fn deploy_xml(xml: &str) -> Result<(), flowable_engine::error::FlowableError> {
    let process_engine = ProcessEngine::new_with_config(
        "default".to_string(),
        ProcessEngineConfiguration::default(),
    );
    let repository_service = process_engine.get_repository_service();

    let builder = repository_service
        .create_deployment()
        .name("P26 Empty Timer Test Deployment".to_string())
        .add_string("test_process.bpmn20.xml".to_string(), xml.to_string());

    repository_service.deploy(builder).map(|_| ())
}

fn assert_empty_timer_deploy_error(xml: &str) {
    let result = deploy_xml(xml);
    assert!(result.is_err(), "deployment should fail for empty timer");
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains(EXPECTED_MESSAGE),
        "unexpected error message: {err_msg}"
    );
}

/// Java EventValidator.java:89-93 — empty timer on a boundary event fails
/// deployment.
#[test]
fn test_empty_timer_boundary_event_fails_deployment() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="p26EmptyTimerBoundary" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="f1" sourceRef="start" targetRef="task1" />
            <userTask id="task1" />
            <boundaryEvent id="timerBoundary" attachedToRef="task1" cancelActivity="true">
                <timerEventDefinition />
            </boundaryEvent>
            <sequenceFlow id="f2" sourceRef="task1" targetRef="end" />
            <sequenceFlow id="f3" sourceRef="timerBoundary" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#;

    assert_empty_timer_deploy_error(xml);
}

/// Java EventValidator.java:89-93 — empty timer on a start event fails
/// deployment.
#[test]
fn test_empty_timer_start_event_fails_deployment() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="p26EmptyTimerStart" isExecutable="true">
            <startEvent id="timerStart">
                <timerEventDefinition />
            </startEvent>
            <sequenceFlow id="f1" sourceRef="timerStart" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#;

    assert_empty_timer_deploy_error(xml);
}

/// Java EventValidator.java:89-93 — empty timer on an intermediate catch
/// event fails deployment.
#[test]
fn test_empty_timer_intermediate_catch_event_fails_deployment() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="p26EmptyTimerCatch" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="f1" sourceRef="start" targetRef="timerCatch" />
            <intermediateCatchEvent id="timerCatch">
                <timerEventDefinition />
            </intermediateCatchEvent>
            <sequenceFlow id="f2" sourceRef="timerCatch" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#;

    assert_empty_timer_deploy_error(xml);
}

/// Java EventValidator.java:89-93 — empty timer nested inside an event
/// subprocess start event fails deployment.
#[test]
fn test_empty_timer_event_subprocess_start_fails_deployment() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="p26EmptyTimerEventSub" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="f1" sourceRef="start" targetRef="task1" />
            <userTask id="task1" />
            <sequenceFlow id="f2" sourceRef="task1" targetRef="end" />
            <endEvent id="end" />
            <subProcess id="eventSub" triggeredByEvent="true">
                <startEvent id="timerSubStart">
                    <timerEventDefinition />
                </startEvent>
                <sequenceFlow id="sf1" sourceRef="timerSubStart" targetRef="subEnd" />
                <endEvent id="subEnd" />
            </subProcess>
        </process>
    </definitions>"#;

    assert_empty_timer_deploy_error(xml);
}

/// A properly configured timer still deploys (guard against over-rejection).
#[test]
fn test_configured_timer_boundary_event_deploys() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="p26ConfiguredTimerBoundary" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="f1" sourceRef="start" targetRef="task1" />
            <userTask id="task1" />
            <boundaryEvent id="timerBoundary" attachedToRef="task1" cancelActivity="true">
                <timerEventDefinition><timeDuration>PT10H</timeDuration></timerEventDefinition>
            </boundaryEvent>
            <sequenceFlow id="f2" sourceRef="task1" targetRef="end" />
            <sequenceFlow id="f3" sourceRef="timerBoundary" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#;

    assert!(
        deploy_xml(xml).is_ok(),
        "configured timer boundary should deploy"
    );
}
