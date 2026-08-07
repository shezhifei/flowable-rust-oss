//! P18-B contract tests: an unscoped compensation throw only compensates
//! activities inside the throwing event's OWN scope container.
//!
//! Java evidence: `IntermediateThrowCompensationEventActivityBehavior#execute`
//! (lines 112-131) — when no `activityRef` is given, the flow-elements
//! container of the throw event (its sub-process, or the process itself) is
//! used to collect compensation subscriptions; activities outside that
//! container are NOT compensated. Handlers run in reverse completion order
//! (`ScopeUtil.throwCompensationEvent` sorts by created desc), and
//! compensating a sub-process cascades to its completed children
//! (`CompensationEventHandler`).

use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::persistence::runtime_store::HttpTaskRecord;

/// `bookOuter` lives in the main process; the throw (no activityRef) is inside
/// `innerScope`, so only `stepA`/`stepB` (in nested scopes of `innerScope`)
/// may be compensated.
const INNER_THROW_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
    <process id="compensationInnerThrowP18" isExecutable="true">
        <startEvent id="start" />
        <sequenceFlow id="flow1" sourceRef="start" targetRef="bookOuter" />
        <userTask id="bookOuter" name="Book Outer" />
        <boundaryEvent id="bookOuterCompensation" attachedToRef="bookOuter">
            <compensateEventDefinition />
        </boundaryEvent>
        <serviceTask id="undoOuter"
                     name="Undo Outer"
                     isForCompensation="true"
                     flowable:type="http">
            <extensionElements>
                <flowable:requestMethod>POST</flowable:requestMethod>
                <flowable:requestUrl>https://example.flowable.local/p18/undo-outer</flowable:requestUrl>
            </extensionElements>
        </serviceTask>
        <sequenceFlow id="flow2" sourceRef="bookOuter" targetRef="innerScope" />
        <subProcess id="innerScope">
            <startEvent id="innerStart" />
            <sequenceFlow id="innerFlow1" sourceRef="innerStart" targetRef="scopeA" />
            <subProcess id="scopeA">
                <startEvent id="aStart" />
                <sequenceFlow id="aFlow1" sourceRef="aStart" targetRef="stepA" />
                <userTask id="stepA" name="Step A" />
                <boundaryEvent id="stepACompensation" attachedToRef="stepA">
                    <compensateEventDefinition />
                </boundaryEvent>
                <serviceTask id="undoA"
                             name="Undo A"
                             isForCompensation="true"
                             flowable:type="http">
                    <extensionElements>
                        <flowable:requestMethod>POST</flowable:requestMethod>
                        <flowable:requestUrl>https://example.flowable.local/p18/undo-a</flowable:requestUrl>
                    </extensionElements>
                </serviceTask>
                <sequenceFlow id="aFlow2" sourceRef="stepA" targetRef="aEnd" />
                <endEvent id="aEnd" />
            </subProcess>
            <sequenceFlow id="innerFlow2" sourceRef="scopeA" targetRef="scopeB" />
            <subProcess id="scopeB">
                <startEvent id="bStart" />
                <sequenceFlow id="bFlow1" sourceRef="bStart" targetRef="stepB" />
                <userTask id="stepB" name="Step B" />
                <boundaryEvent id="stepBCompensation" attachedToRef="stepB">
                    <compensateEventDefinition />
                </boundaryEvent>
                <serviceTask id="undoB"
                             name="Undo B"
                             isForCompensation="true"
                             flowable:type="http">
                    <extensionElements>
                        <flowable:requestMethod>POST</flowable:requestMethod>
                        <flowable:requestUrl>https://example.flowable.local/p18/undo-b</flowable:requestUrl>
                    </extensionElements>
                </serviceTask>
                <sequenceFlow id="bFlow2" sourceRef="stepB" targetRef="bEnd" />
                <endEvent id="bEnd" />
            </subProcess>
            <sequenceFlow id="innerFlow3" sourceRef="scopeB" targetRef="throwInner" />
            <intermediateThrowEvent id="throwInner">
                <compensateEventDefinition />
            </intermediateThrowEvent>
            <sequenceFlow id="innerFlow4" sourceRef="throwInner" targetRef="innerEnd" />
            <endEvent id="innerEnd" />
        </subProcess>
        <sequenceFlow id="flow3" sourceRef="innerScope" targetRef="afterScope" />
        <userTask id="afterScope" name="After Scope" />
        <sequenceFlow id="flow4" sourceRef="afterScope" targetRef="end" />
        <endEvent id="end" />
    </process>
</definitions>"#;

/// Same topology but the unscoped throw sits in the MAIN process after the
/// sub-process — everything completed (including nested children) compensates.
const TOP_THROW_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
    <process id="compensationTopThrowP18" isExecutable="true">
        <startEvent id="start" />
        <sequenceFlow id="flow1" sourceRef="start" targetRef="bookOuter" />
        <userTask id="bookOuter" name="Book Outer" />
        <boundaryEvent id="bookOuterCompensation" attachedToRef="bookOuter">
            <compensateEventDefinition />
        </boundaryEvent>
        <serviceTask id="undoOuter"
                     name="Undo Outer"
                     isForCompensation="true"
                     flowable:type="http">
            <extensionElements>
                <flowable:requestMethod>POST</flowable:requestMethod>
                <flowable:requestUrl>https://example.flowable.local/p18/top/undo-outer</flowable:requestUrl>
            </extensionElements>
        </serviceTask>
        <sequenceFlow id="flow2" sourceRef="bookOuter" targetRef="innerScope" />
        <subProcess id="innerScope">
            <startEvent id="innerStart" />
            <sequenceFlow id="innerFlow1" sourceRef="innerStart" targetRef="scopeA" />
            <subProcess id="scopeA">
                <startEvent id="aStart" />
                <sequenceFlow id="aFlow1" sourceRef="aStart" targetRef="stepA" />
                <userTask id="stepA" name="Step A" />
                <boundaryEvent id="stepACompensation" attachedToRef="stepA">
                    <compensateEventDefinition />
                </boundaryEvent>
                <serviceTask id="undoA"
                             name="Undo A"
                             isForCompensation="true"
                             flowable:type="http">
                    <extensionElements>
                        <flowable:requestMethod>POST</flowable:requestMethod>
                        <flowable:requestUrl>https://example.flowable.local/p18/top/undo-a</flowable:requestUrl>
                    </extensionElements>
                </serviceTask>
                <sequenceFlow id="aFlow2" sourceRef="stepA" targetRef="aEnd" />
                <endEvent id="aEnd" />
            </subProcess>
            <sequenceFlow id="innerFlow2" sourceRef="scopeA" targetRef="scopeB" />
            <subProcess id="scopeB">
                <startEvent id="bStart" />
                <sequenceFlow id="bFlow1" sourceRef="bStart" targetRef="stepB" />
                <userTask id="stepB" name="Step B" />
                <boundaryEvent id="stepBCompensation" attachedToRef="stepB">
                    <compensateEventDefinition />
                </boundaryEvent>
                <serviceTask id="undoB"
                             name="Undo B"
                             isForCompensation="true"
                             flowable:type="http">
                    <extensionElements>
                        <flowable:requestMethod>POST</flowable:requestMethod>
                        <flowable:requestUrl>https://example.flowable.local/p18/top/undo-b</flowable:requestUrl>
                    </extensionElements>
                </serviceTask>
                <sequenceFlow id="bFlow2" sourceRef="stepB" targetRef="bEnd" />
                <endEvent id="bEnd" />
            </subProcess>
            <sequenceFlow id="innerFlow3" sourceRef="scopeB" targetRef="innerEnd" />
            <endEvent id="innerEnd" />
        </subProcess>
        <sequenceFlow id="flow3" sourceRef="innerScope" targetRef="throwTop" />
        <intermediateThrowEvent id="throwTop">
            <compensateEventDefinition />
        </intermediateThrowEvent>
        <sequenceFlow id="flow4" sourceRef="throwTop" targetRef="afterThrow" />
        <userTask id="afterThrow" name="After Throw" />
        <sequenceFlow id="flow5" sourceRef="afterThrow" targetRef="end" />
        <endEvent id="end" />
    </process>
</definitions>"#;

fn deploy(engine: &ProcessEngine, resource: &str, xml: &str) {
    let repository_service = engine.get_repository_service();
    repository_service
        .deploy(
            repository_service
                .create_deployment()
                .name(format!("{resource} deployment"))
                .add_string(format!("{resource}.bpmn20.xml"), xml.to_string()),
        )
        .unwrap();
}

fn start_by_key(engine: &ProcessEngine, key: &str) -> String {
    let runtime_service = engine.get_runtime_service();
    runtime_service
        .start_process_instance(
            runtime_service
                .create_process_instance_builder()
                .process_definition_key(key.to_string()),
        )
        .unwrap()
        .id
}

fn complete_single_task(engine: &ProcessEngine, process_instance_id: &str, expected_key: &str) {
    let task_service = engine.get_task_service();
    let tasks = task_service
        .get_tasks_by_process_instance_id(process_instance_id.to_string())
        .unwrap();
    assert_eq!(tasks.len(), 1, "expected exactly one open task");
    assert_eq!(tasks[0].task_definition_key, expected_key);
    task_service
        .complete_task_by_id(tasks[0].id.clone())
        .unwrap();
}

fn http_handler_activity_ids(engine: &ProcessEngine, process_instance_id: &str) -> Vec<String> {
    let runtime_store = engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let mut params = flowable_engine::persistence::DbParams::new();
    params.push(process_instance_id);
    let rows = session
        .raw_query(
            "SELECT data FROM http_task_records \
             WHERE process_instance_id = ?1 \
             ORDER BY rowid ASC",
            params,
        )
        .unwrap();
    let mut activity_ids = Vec::new();
    for row in rows {
        if let Some(data) = row.get_text("data") {
            let record: HttpTaskRecord = serde_json::from_str(&data).unwrap();
            activity_ids.push(record.activity_id);
        }
    }
    let _ = session.rollback();
    activity_ids
}

/// Java `CompensateEventTest.testCompensateScope` semantics: an unscoped
/// throw inside a sub-process compensates only completed activities of that
/// sub-process, in reverse completion order. Outer activities stay untouched.
#[test]
fn unscoped_throw_inside_subprocess_only_compensates_that_scope() {
    let engine = ProcessEngine::new("p18-compensation-inner-scope".to_string());
    deploy(&engine, "compensation_inner_throw_p18", INNER_THROW_XML);

    let process_instance_id = start_by_key(&engine, "compensationInnerThrowP18");
    complete_single_task(&engine, &process_instance_id, "bookOuter");
    complete_single_task(&engine, &process_instance_id, "stepA");
    complete_single_task(&engine, &process_instance_id, "stepB");

    assert_eq!(
        http_handler_activity_ids(&engine, &process_instance_id),
        vec!["undoB".to_string(), "undoA".to_string()],
        "unscoped throw inside innerScope must compensate only innerScope's \
         completed activities, in reverse completion order"
    );

    let task_keys = engine
        .get_task_service()
        .get_tasks_by_process_instance_id(process_instance_id.clone())
        .unwrap()
        .into_iter()
        .map(|task| task.task_definition_key)
        .collect::<Vec<_>>();
    assert_eq!(task_keys, vec!["afterScope".to_string()]);

    let runtime_store = engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let remaining = runtime_store
        .find_compensation_subscriptions_by_process_instance_id(&process_instance_id, &mut session)
        .into_iter()
        .map(|subscription| subscription.activity_id)
        .collect::<Vec<_>>();
    let _ = session.rollback();
    assert_eq!(
        remaining,
        vec!["bookOuter".to_string()],
        "the outer activity's compensation subscription must survive an inner-scope throw"
    );
}

/// Guard: an unscoped throw at the MAIN-process level still cascades into
/// completed nested sub-process children (Java `testCompensateNestedSubprocess`
/// / `CompensationEventHandler` cascade), reverse completion order preserved.
#[test]
fn unscoped_throw_at_process_level_compensates_nested_children_in_reverse_order() {
    let engine = ProcessEngine::new("p18-compensation-top-scope".to_string());
    deploy(&engine, "compensation_top_throw_p18", TOP_THROW_XML);

    let process_instance_id = start_by_key(&engine, "compensationTopThrowP18");
    complete_single_task(&engine, &process_instance_id, "bookOuter");
    complete_single_task(&engine, &process_instance_id, "stepA");
    complete_single_task(&engine, &process_instance_id, "stepB");

    assert_eq!(
        http_handler_activity_ids(&engine, &process_instance_id),
        vec![
            "undoB".to_string(),
            "undoA".to_string(),
            "undoOuter".to_string(),
        ],
        "a process-level unscoped throw must compensate everything completed, \
         nested children included, in reverse completion order"
    );

    let task_keys = engine
        .get_task_service()
        .get_tasks_by_process_instance_id(process_instance_id.clone())
        .unwrap()
        .into_iter()
        .map(|task| task.task_definition_key)
        .collect::<Vec<_>>();
    assert_eq!(task_keys, vec!["afterThrow".to_string()]);
}
