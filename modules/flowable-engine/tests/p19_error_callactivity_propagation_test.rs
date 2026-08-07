//! P19: BPMN error propagation across call activities.
//!
//! Java reference:
//! - `ErrorPropagation.java` walks parent/superExecution and collects catch events
//!   from parent process definitions; uncaught child PI errors delete the child
//!   and let the parent call activity error boundary take over.
//! - `ErrorPropagationTest` (two-level call: catchError4 → catchError3 → throwError)
//!   expects the middle process boundary to land on `MyErrorTaskNested`.
//! - `BoundaryErrorEventTest.testCatchErrorEndEventOnCallActivity` (single-level
//!   call with error end, boundary on wrapping subprocess).

use flowable_engine::engine::process_engine::ProcessEngine;

const THROW_ERROR_XML: &str = r#"
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             targetNamespace="http://flowable.org/test">
  <error id="myError" errorCode="MY_ERROR" />
  <process id="throwError" isExecutable="true">
    <startEvent id="startThrow" />
    <sequenceFlow id="fThrow" sourceRef="startThrow" targetRef="errorEnd" />
    <endEvent id="errorEnd">
      <errorEventDefinition errorRef="myError" />
    </endEvent>
  </process>
</definitions>
"#;

const THROW_OTHER_ERROR_XML: &str = r#"
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             targetNamespace="http://flowable.org/test">
  <error id="otherError" errorCode="OTHER_ERROR" />
  <process id="throwOtherError" isExecutable="true">
    <startEvent id="startThrow" />
    <sequenceFlow id="fThrow" sourceRef="startThrow" targetRef="errorEnd" />
    <endEvent id="errorEnd">
      <errorEventDefinition errorRef="otherError" />
    </endEvent>
  </process>
</definitions>
"#;

/// Single-level: parent wraps call activity in a subprocess with a matching
/// error boundary (Java `BoundaryErrorEventTest.callActivityWithErrorEndEventCatch`).
const SINGLE_LEVEL_PARENT_XML: &str = r#"
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             targetNamespace="http://flowable.org/test">
  <error id="myError" errorCode="MY_ERROR" />
  <process id="singleLevelCatch" isExecutable="true">
    <startEvent id="start" />
    <sequenceFlow id="f1" sourceRef="start" targetRef="sub" />
    <subProcess id="sub">
      <startEvent id="subStart" />
      <sequenceFlow id="sf1" sourceRef="subStart" targetRef="callChild" />
      <callActivity id="callChild" calledElement="throwError" />
      <sequenceFlow id="sf2" sourceRef="callChild" targetRef="subEnd" />
      <endEvent id="subEnd" />
    </subProcess>
    <boundaryEvent id="catchMyError" attachedToRef="sub">
      <errorEventDefinition errorRef="myError" />
    </boundaryEvent>
    <boundaryEvent id="catchAny" attachedToRef="sub">
      <errorEventDefinition />
    </boundaryEvent>
    <sequenceFlow id="toSpecific" sourceRef="catchMyError" targetRef="specificTask" />
    <sequenceFlow id="toAny" sourceRef="catchAny" targetRef="anyTask" />
    <userTask id="specificTask" name="SpecificErrorTask" />
    <userTask id="anyTask" name="AnyErrorTask" />
    <sequenceFlow id="endSpecific" sourceRef="specificTask" targetRef="end" />
    <sequenceFlow id="endAny" sourceRef="anyTask" targetRef="end" />
    <sequenceFlow id="normal" sourceRef="sub" targetRef="end" />
    <endEvent id="end" />
  </process>
</definitions>
"#;

/// Boundary attached directly to the call activity (not the wrapping subprocess).
const CALL_ACTIVITY_BOUNDARY_PARENT_XML: &str = r#"
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             targetNamespace="http://flowable.org/test">
  <error id="myError" errorCode="MY_ERROR" />
  <process id="callActivityBoundaryCatch" isExecutable="true">
    <startEvent id="start" />
    <sequenceFlow id="f1" sourceRef="start" targetRef="callChild" />
    <callActivity id="callChild" calledElement="throwError" />
    <boundaryEvent id="catchMyError" attachedToRef="callChild">
      <errorEventDefinition errorRef="myError" />
    </boundaryEvent>
    <sequenceFlow id="toErrorTask" sourceRef="catchMyError" targetRef="errorTask" />
    <userTask id="errorTask" name="ErrorTask" />
    <sequenceFlow id="endError" sourceRef="errorTask" targetRef="end" />
    <sequenceFlow id="normal" sourceRef="callChild" targetRef="end" />
    <endEvent id="end" />
  </process>
</definitions>
"#;

/// No-code (catch-all) boundary on call activity; child throws coded error.
const NO_CODE_BOUNDARY_PARENT_XML: &str = r#"
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             targetNamespace="http://flowable.org/test">
  <error id="myError" errorCode="MY_ERROR" />
  <process id="noCodeBoundaryCatch" isExecutable="true">
    <startEvent id="start" />
    <sequenceFlow id="f1" sourceRef="start" targetRef="callChild" />
    <callActivity id="callChild" calledElement="throwError" />
    <boundaryEvent id="catchAny" attachedToRef="callChild">
      <errorEventDefinition />
    </boundaryEvent>
    <sequenceFlow id="toAnyTask" sourceRef="catchAny" targetRef="anyTask" />
    <userTask id="anyTask" name="AnyErrorTask" />
    <sequenceFlow id="endAny" sourceRef="anyTask" targetRef="end" />
    <sequenceFlow id="normal" sourceRef="callChild" targetRef="end" />
    <endEvent id="end" />
  </process>
</definitions>
"#;

/// Parent has no error boundary — child uncaught error should Failed-end the child.
const NO_BOUNDARY_PARENT_XML: &str = r#"
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             targetNamespace="http://flowable.org/test">
  <process id="noBoundaryParent" isExecutable="true">
    <startEvent id="start" />
    <sequenceFlow id="f1" sourceRef="start" targetRef="callChild" />
    <callActivity id="callChild" calledElement="throwError" />
    <sequenceFlow id="f2" sourceRef="callChild" targetRef="afterCall" />
    <userTask id="afterCall" name="AfterCall" />
    <sequenceFlow id="f3" sourceRef="afterCall" targetRef="end" />
    <endEvent id="end" />
  </process>
</definitions>
"#;

/// Middle process: call throwError inside subprocess with myError boundary
/// (Java catchError3 / MyErrorTaskNested).
const MIDDLE_CATCH_XML: &str = r#"
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             targetNamespace="http://flowable.org/test">
  <error id="myError" errorCode="MY_ERROR" />
  <process id="catchError3" isExecutable="true">
    <startEvent id="startCatchErrorNested" />
    <sequenceFlow id="flow1Nested" sourceRef="startCatchErrorNested" targetRef="subprocessCatchError" />
    <subProcess id="subprocessCatchError" name="Sub process nested">
      <startEvent id="startSubprocessNested" />
      <sequenceFlow id="flow2Nested" sourceRef="startSubprocessNested" targetRef="callActivitySubprocessNested" />
      <callActivity id="callActivitySubprocessNested" calledElement="throwError" />
      <sequenceFlow id="flow3Nested" sourceRef="callActivitySubprocessNested" targetRef="endSubprocessNested" />
      <endEvent id="endSubprocessNested" />
    </subProcess>
    <boundaryEvent id="catchOtherErrorsNested" attachedToRef="subprocessCatchError">
      <errorEventDefinition />
    </boundaryEvent>
    <boundaryEvent id="boundaryCatchMyErrorNested" attachedToRef="subprocessCatchError">
      <errorEventDefinition errorRef="myError" />
    </boundaryEvent>
    <userTask id="otherErrorsTaskNested" name="OtherErrorsTaskNested" />
    <userTask id="myErrorTaskNested" name="MyErrorTaskNested" />
    <sequenceFlow id="flow5Nested" sourceRef="boundaryCatchMyErrorNested" targetRef="myErrorTaskNested" />
    <sequenceFlow id="flow6Nested" sourceRef="myErrorTaskNested" targetRef="endCatchErrorNested" />
    <sequenceFlow id="flow7Nested" sourceRef="catchOtherErrorsNested" targetRef="otherErrorsTaskNested" />
    <sequenceFlow id="flow8Nested" sourceRef="otherErrorsTaskNested" targetRef="endCatchErrorNested" />
    <sequenceFlow id="flow4Nested" sourceRef="subprocessCatchError" targetRef="endCatchErrorNested" />
    <endEvent id="endCatchErrorNested" />
  </process>
</definitions>
"#;

/// Outer process: call catchError3 inside subprocess with myError boundary
/// (Java catchError4). Middle should catch first → MyErrorTaskNested.
const OUTER_CATCH_XML: &str = r#"
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             targetNamespace="http://flowable.org/test">
  <error id="myError" errorCode="MY_ERROR" />
  <process id="catchError4" isExecutable="true">
    <startEvent id="startCatchError" />
    <sequenceFlow id="flow1" sourceRef="startCatchError" targetRef="subprocessCatchError" />
    <subProcess id="subprocessCatchError" name="Sub Process">
      <startEvent id="startSubprocess" />
      <sequenceFlow id="flow2" sourceRef="startSubprocess" targetRef="callActivitySubprocess" />
      <callActivity id="callActivitySubprocess" calledElement="catchError3" />
      <sequenceFlow id="flow3" sourceRef="callActivitySubprocess" targetRef="endSubprocess" />
      <endEvent id="endSubprocess" />
    </subProcess>
    <boundaryEvent id="catchOtherErrors" attachedToRef="subprocessCatchError">
      <errorEventDefinition />
    </boundaryEvent>
    <boundaryEvent id="boundaryCatchMyError" attachedToRef="subprocessCatchError">
      <errorEventDefinition errorRef="myError" />
    </boundaryEvent>
    <userTask id="otherErrorsTask" name="OtherErrorsTask" />
    <userTask id="myErrorTask" name="MyErrorTask" />
    <sequenceFlow id="flow5" sourceRef="boundaryCatchMyError" targetRef="myErrorTask" />
    <sequenceFlow id="flow6" sourceRef="myErrorTask" targetRef="endCatchError" />
    <sequenceFlow id="flow7" sourceRef="catchOtherErrors" targetRef="otherErrorsTask" />
    <sequenceFlow id="flow8" sourceRef="otherErrorsTask" targetRef="endCatchError" />
    <sequenceFlow id="flow4" sourceRef="subprocessCatchError" targetRef="endCatchError" />
    <endEvent id="endCatchError" />
  </process>
</definitions>
"#;

/// Two-level: middle has no boundary; outer must catch (proves multi-hop super_execution walk).
const MIDDLE_PASSTHROUGH_XML: &str = r#"
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             targetNamespace="http://flowable.org/test">
  <process id="middlePassthrough" isExecutable="true">
    <startEvent id="start" />
    <sequenceFlow id="f1" sourceRef="start" targetRef="callThrow" />
    <callActivity id="callThrow" calledElement="throwError" />
    <sequenceFlow id="f2" sourceRef="callThrow" targetRef="end" />
    <endEvent id="end" />
  </process>
</definitions>
"#;

const OUTER_CATCH_VIA_MIDDLE_XML: &str = r#"
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             targetNamespace="http://flowable.org/test">
  <error id="myError" errorCode="MY_ERROR" />
  <process id="outerCatchViaMiddle" isExecutable="true">
    <startEvent id="start" />
    <sequenceFlow id="f1" sourceRef="start" targetRef="callMiddle" />
    <callActivity id="callMiddle" calledElement="middlePassthrough" />
    <boundaryEvent id="catchMyError" attachedToRef="callMiddle">
      <errorEventDefinition errorRef="myError" />
    </boundaryEvent>
    <sequenceFlow id="toError" sourceRef="catchMyError" targetRef="outerErrorTask" />
    <userTask id="outerErrorTask" name="OuterErrorTask" />
    <sequenceFlow id="endError" sourceRef="outerErrorTask" targetRef="end" />
    <sequenceFlow id="normal" sourceRef="callMiddle" targetRef="end" />
    <endEvent id="end" />
  </process>
</definitions>
"#;

fn deploy(engine: &ProcessEngine, name: &str, resources: &[(&str, &str)]) {
    let mut builder = engine
        .get_repository_service()
        .create_deployment()
        .name(name.to_string());
    for (file_name, xml) in resources {
        builder = builder.add_string(file_name.to_string(), xml.to_string());
    }
    engine
        .get_repository_service()
        .deploy(builder)
        .expect("deploy");
}

fn start_by_key(engine: &ProcessEngine, key: &str) -> String {
    let runtime = engine.get_runtime_service();
    let defs = engine
        .get_repository_service()
        .get_process_definition_ids()
        .unwrap();
    let def_id = defs
        .into_iter()
        .find(|id| id.starts_with(key))
        .unwrap_or_else(|| panic!("process definition for key '{key}'"));
    let builder = runtime
        .create_process_instance_builder()
        .process_definition_id(def_id);
    runtime.start_process_instance(builder).unwrap().id
}

fn tasks_for(engine: &ProcessEngine, pi_id: &str) -> Vec<(String, String)> {
    engine
        .get_task_service()
        .get_tasks_by_process_instance_id(pi_id.to_string())
        .unwrap()
        .into_iter()
        .map(|t| (t.task_definition_key.clone(), t.name.clone()))
        .collect()
}

fn all_process_instances(engine: &ProcessEngine) -> Vec<(String, bool, Option<String>)> {
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    store
        .snapshot_process_instances(&mut session)
        .into_values()
        .map(|pi| (pi.id, pi.is_ended, pi.super_execution_id))
        .collect()
}

/// Java `BoundaryErrorEventTest.testCatchErrorEndEventOnCallActivity`:
/// child error end → parent subprocess error boundary (exact code).
#[test]
fn p19_single_level_call_activity_error_caught_by_subprocess_boundary() {
    let engine = ProcessEngine::new("p19-single-level-sub".to_string());
    deploy(
        &engine,
        "p19-single",
        &[
            ("throwError.bpmn20.xml", THROW_ERROR_XML),
            ("singleLevelCatch.bpmn20.xml", SINGLE_LEVEL_PARENT_XML),
        ],
    );

    let parent_id = start_by_key(&engine, "singleLevelCatch");
    let tasks = tasks_for(&engine, &parent_id);
    assert_eq!(
        tasks.len(),
        1,
        "parent should land on error boundary path, tasks={tasks:?}"
    );
    assert_eq!(tasks[0].0, "specificTask");
    assert_eq!(tasks[0].1, "SpecificErrorTask");

    let instances = all_process_instances(&engine);
    let child = instances
        .iter()
        .find(|(_, _, super_id)| super_id.is_some())
        .expect("child process instance must exist");
    assert!(
        child.1,
        "child process instance must be ended after error propagation"
    );
}

/// Boundary attached directly on the call activity element.
#[test]
fn p19_single_level_error_boundary_on_call_activity() {
    let engine = ProcessEngine::new("p19-single-level-ca".to_string());
    deploy(
        &engine,
        "p19-ca-boundary",
        &[
            ("throwError.bpmn20.xml", THROW_ERROR_XML),
            (
                "callActivityBoundaryCatch.bpmn20.xml",
                CALL_ACTIVITY_BOUNDARY_PARENT_XML,
            ),
        ],
    );

    let parent_id = start_by_key(&engine, "callActivityBoundaryCatch");
    let tasks = tasks_for(&engine, &parent_id);
    assert_eq!(tasks.len(), 1, "tasks={tasks:?}");
    assert_eq!(tasks[0].0, "errorTask");
    assert_eq!(tasks[0].1, "ErrorTask");
}

/// No-code error boundary on call activity catches any thrown code.
#[test]
fn p19_no_code_boundary_on_call_activity_catches_coded_error() {
    let engine = ProcessEngine::new("p19-no-code".to_string());
    deploy(
        &engine,
        "p19-no-code",
        &[
            ("throwError.bpmn20.xml", THROW_ERROR_XML),
            ("noCodeBoundaryCatch.bpmn20.xml", NO_CODE_BOUNDARY_PARENT_XML),
        ],
    );

    let parent_id = start_by_key(&engine, "noCodeBoundaryCatch");
    let tasks = tasks_for(&engine, &parent_id);
    assert_eq!(tasks.len(), 1, "tasks={tasks:?}");
    assert_eq!(tasks[0].0, "anyTask");
    assert_eq!(tasks[0].1, "AnyErrorTask");
}

/// Exact errorCode on subprocess boundary does not match; empty catch-all does.
#[test]
fn p19_no_code_boundary_fallback_when_code_does_not_match() {
    let engine = ProcessEngine::new("p19-no-code-fallback".to_string());
    let parent_xml = SINGLE_LEVEL_PARENT_XML
        .replace("throwError", "throwOtherError")
        .replace("id=\"singleLevelCatch\"", "id=\"singleLevelCatchOther\"");
    deploy(
        &engine,
        "p19-fallback",
        &[
            ("throwOtherError.bpmn20.xml", THROW_OTHER_ERROR_XML),
            ("singleLevelCatchOther.bpmn20.xml", &parent_xml),
        ],
    );

    let parent_id = start_by_key(&engine, "singleLevelCatchOther");
    let tasks = tasks_for(&engine, &parent_id);
    assert_eq!(tasks.len(), 1, "tasks={tasks:?}");
    assert_eq!(
        tasks[0].0, "anyTask",
        "no-code boundary must catch non-matching error code"
    );
}

/// Uncaught across call activity: child ends with Failed outcome (existing
/// semantics). Parent leave via call-activity out/outgoing is intentionally
/// preserved by `end_process_instance_with_callback_outcome` — not part of P19.
#[test]
fn p19_uncaught_error_on_call_activity_fails_child() {
    let engine = ProcessEngine::new("p19-uncaught".to_string());
    deploy(
        &engine,
        "p19-uncaught",
        &[
            ("throwError.bpmn20.xml", THROW_ERROR_XML),
            ("noBoundaryParent.bpmn20.xml", NO_BOUNDARY_PARENT_XML),
        ],
    );

    let parent_id = start_by_key(&engine, "noBoundaryParent");
    let instances = all_process_instances(&engine);
    let child = instances
        .iter()
        .find(|(id, _, super_id)| *id != parent_id && super_id.is_some())
        .expect("child process instance");
    assert!(child.1, "child must be ended (Failed) when error is uncaught");
}

/// Java `ErrorPropagationTest`: two-level call, middle boundary catches → MyErrorTaskNested.
#[test]
fn p19_two_level_call_activity_middle_boundary_catches() {
    let engine = ProcessEngine::new("p19-two-level-middle".to_string());
    deploy(
        &engine,
        "p19-two-level",
        &[
            ("throwError.bpmn20.xml", THROW_ERROR_XML),
            ("catchError3.bpmn20.xml", MIDDLE_CATCH_XML),
            ("catchError4.bpmn20.xml", OUTER_CATCH_XML),
        ],
    );

    let outer_id = start_by_key(&engine, "catchError4");
    // Middle process is a separate PI; task lives on the middle instance.
    let instances = all_process_instances(&engine);
    let middle = instances
        .iter()
        .find(|(id, _, super_id)| *id != outer_id && super_id.is_some() && !id.starts_with("throw"))
        .or_else(|| {
            // Find the PI that is child of outer (has super) and is not ended with no tasks.
            instances
                .iter()
                .find(|(id, ended, super_id)| *id != outer_id && super_id.is_some() && !ended)
        });

    // Collect tasks across all non-ended instances (middle should hold MyErrorTaskNested).
    let mut found = None;
    for (pi_id, is_ended, _) in &instances {
        if *is_ended {
            continue;
        }
        let tasks = tasks_for(&engine, pi_id);
        if let Some(task) = tasks.iter().find(|(_, name)| name == "MyErrorTaskNested") {
            found = Some((pi_id.clone(), task.clone()));
            break;
        }
        if let Some(task) = tasks.iter().find(|(key, _)| key == "myErrorTaskNested") {
            found = Some((pi_id.clone(), task.clone()));
            break;
        }
    }

    assert!(
        found.is_some(),
        "expected MyErrorTaskNested on middle process; instances={instances:?}, outer_tasks={:?}",
        tasks_for(&engine, &outer_id)
    );
    let (_pi, (key, name)) = found.unwrap();
    assert_eq!(key, "myErrorTaskNested");
    assert_eq!(name, "MyErrorTaskNested");

    // Outer should not have caught (middle did).
    let outer_tasks = tasks_for(&engine, &outer_id);
    assert!(
        outer_tasks.is_empty()
            || outer_tasks
                .iter()
                .all(|(k, _)| k != "myErrorTask" && k != "otherErrorsTask"),
        "outer must not catch when middle already did, outer_tasks={outer_tasks:?}"
    );
    let _ = middle;
}

/// Two-level hop: middle has no catch; outer call activity boundary must catch.
#[test]
fn p19_two_level_call_activity_outer_boundary_catches_through_middle() {
    let engine = ProcessEngine::new("p19-two-level-outer".to_string());
    deploy(
        &engine,
        "p19-two-level-outer",
        &[
            ("throwError.bpmn20.xml", THROW_ERROR_XML),
            ("middlePassthrough.bpmn20.xml", MIDDLE_PASSTHROUGH_XML),
            ("outerCatchViaMiddle.bpmn20.xml", OUTER_CATCH_VIA_MIDDLE_XML),
        ],
    );

    let outer_id = start_by_key(&engine, "outerCatchViaMiddle");
    let tasks = tasks_for(&engine, &outer_id);
    assert_eq!(tasks.len(), 1, "tasks={tasks:?}");
    assert_eq!(tasks[0].0, "outerErrorTask");
    assert_eq!(tasks[0].1, "OuterErrorTask");

    // Intermediate middle + throw child PIs must be ended.
    let instances = all_process_instances(&engine);
    for (id, is_ended, super_id) in &instances {
        if id != &outer_id && super_id.is_some() {
            assert!(
                *is_ended,
                "intermediate/child PI {id} must be ended after outer catch"
            );
        }
    }
}

/// Probe: after entering call activity, error boundary state must be registered
/// on the call activity host execution.
#[test]
fn p19_call_activity_registers_error_boundary_state() {
    let engine = ProcessEngine::new("p19-register".to_string());
    // Child that waits so we can inspect boundary registration mid-flight.
    let waiting_child = r#"
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
                 targetNamespace="http://flowable.org/test">
      <process id="waitingChild" isExecutable="true">
        <startEvent id="start" />
        <sequenceFlow id="f1" sourceRef="start" targetRef="wait" />
        <userTask id="wait" name="Wait" />
        <sequenceFlow id="f2" sourceRef="wait" targetRef="end" />
        <endEvent id="end" />
      </process>
    </definitions>
    "#;
    let parent = CALL_ACTIVITY_BOUNDARY_PARENT_XML.replace("throwError", "waitingChild");
    deploy(
        &engine,
        "p19-register",
        &[
            ("waitingChild.bpmn20.xml", waiting_child),
            ("parent.bpmn20.xml", &parent),
        ],
    );

    let parent_id = start_by_key(&engine, "callActivityBoundaryCatch");
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let boundaries = store.find_boundary_event_states_by_process_instance_id(&parent_id, &mut session);
    assert!(
        boundaries
            .iter()
            .any(|b| b.boundary_event_id == "catchMyError"
                && b.attached_activity_id == "callChild"
                && matches!(
                    b.event_subscription.kind,
                    flowable_engine::persistence::runtime_store::EventSubscriptionKind::Error
                )),
        "call activity must register its error boundary state, found={boundaries:?}"
    );
}
