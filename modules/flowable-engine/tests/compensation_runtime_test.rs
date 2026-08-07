use flowable_engine::bpmn::behavior::cancel_end_event_activity_behavior::CancelEndEventActivityBehavior;
use flowable_engine::delegate::activity_behavior::ActivityBehavior;
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::persistence::runtime_store::HttpTaskRecord;
use flowable_engine::runtime::compensation::CompensationSubscription;
use flowable_engine::runtime::execution::Execution;
use flowable_http_service::DeterministicHttpRuntime;
use std::sync::Arc;

#[test]
fn test_throw_compensation_without_activity_ref_schedules_completed_handlers_lifo() {
    let process_engine = ProcessEngine::new("throw-compensation-lifo-order-test".to_string());
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();
    let task_service = process_engine.get_task_service();
    let runtime_store = process_engine.get_runtime_store();

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
    <process id="throwCompensationLifoOrderProcess" isExecutable="true">
        <startEvent id="start" />
        <sequenceFlow id="flow1" sourceRef="start" targetRef="firstScope" />
        <subProcess id="firstScope">
            <startEvent id="firstStart" />
            <sequenceFlow id="firstFlow1" sourceRef="firstStart" targetRef="bookFirst" />
            <userTask id="bookFirst" name="Book First" />
            <boundaryEvent id="bookFirstCompensation" attachedToRef="bookFirst">
                <compensateEventDefinition />
            </boundaryEvent>
            <serviceTask id="undoFirstBooking"
                         name="Undo First Booking"
                         isForCompensation="true"
                         flowable:type="http">
                <extensionElements>
                    <flowable:requestMethod>POST</flowable:requestMethod>
                    <flowable:requestUrl>https://example.flowable.local/compensation/first</flowable:requestUrl>
                </extensionElements>
            </serviceTask>
            <sequenceFlow id="firstFlow2" sourceRef="bookFirst" targetRef="firstEnd" />
            <endEvent id="firstEnd" />
        </subProcess>
        <sequenceFlow id="flow2" sourceRef="firstScope" targetRef="secondScope" />
        <subProcess id="secondScope">
            <startEvent id="secondStart" />
            <sequenceFlow id="secondFlow1" sourceRef="secondStart" targetRef="bookSecond" />
            <userTask id="bookSecond" name="Book Second" />
            <boundaryEvent id="bookSecondCompensation" attachedToRef="bookSecond">
                <compensateEventDefinition />
            </boundaryEvent>
            <serviceTask id="undoSecondBooking"
                         name="Undo Second Booking"
                         isForCompensation="true"
                         flowable:type="http">
                <extensionElements>
                    <flowable:requestMethod>POST</flowable:requestMethod>
                    <flowable:requestUrl>https://example.flowable.local/compensation/second</flowable:requestUrl>
                </extensionElements>
            </serviceTask>
            <sequenceFlow id="secondFlow2" sourceRef="bookSecond" targetRef="secondEnd" />
            <endEvent id="secondEnd" />
        </subProcess>
        <sequenceFlow id="flow3" sourceRef="secondScope" targetRef="throwAllCompensation" />
        <intermediateThrowEvent id="throwAllCompensation">
            <compensateEventDefinition />
        </intermediateThrowEvent>
        <sequenceFlow id="flow4" sourceRef="throwAllCompensation" targetRef="afterCompensation" />
        <userTask id="afterCompensation" name="After Compensation" />
        <sequenceFlow id="flow5" sourceRef="afterCompensation" targetRef="end" />
        <endEvent id="end" />
    </process>
</definitions>"#;

    repository_service
        .deploy(
            repository_service
                .create_deployment()
                .name("Throw Compensation LIFO Order Deployment".to_string())
                .add_string(
                    "throw_compensation_lifo_order.bpmn20.xml".to_string(),
                    xml.to_string(),
                ),
        )
        .unwrap();

    let process_definition_id = repository_service.get_process_definition_ids().unwrap()[0].clone();
    let process_instance = runtime_service
        .start_process_instance(
            runtime_service
                .create_process_instance_builder()
                .process_definition_id(process_definition_id),
        )
        .unwrap();

    let tasks = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].task_definition_key, "bookFirst");
    task_service
        .complete_task_by_id(tasks[0].id.clone())
        .unwrap();

    let tasks = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].task_definition_key, "bookSecond");
    task_service
        .complete_task_by_id(tasks[0].id.clone())
        .unwrap();

    let handler_activity_ids = {
        let mut session = runtime_store.create_session().unwrap();
        let mut params = flowable_engine::persistence::DbParams::new();
        params.push(process_instance.id.as_str());
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
    };

    assert_eq!(
        handler_activity_ids,
        vec![
            "undoSecondBooking".to_string(),
            "undoFirstBooking".to_string(),
        ],
        "unscoped compensation must dispatch completed handlers in reverse completion order"
    );

    let task_keys = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap()
        .into_iter()
        .map(|task| task.task_definition_key)
        .collect::<Vec<_>>();
    assert_eq!(task_keys, vec!["afterCompensation".to_string()]);
}

#[test]
fn test_end_compensation_without_activity_ref_schedules_completed_handlers_lifo() {
    let process_engine = ProcessEngine::new("end-compensation-lifo-order-test".to_string());
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();
    let task_service = process_engine.get_task_service();
    let runtime_store = process_engine.get_runtime_store();

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
    <process id="endCompensationLifoOrderProcess" isExecutable="true">
        <startEvent id="start" />
        <sequenceFlow id="flow1" sourceRef="start" targetRef="firstScope" />
        <subProcess id="firstScope">
            <startEvent id="firstStart" />
            <sequenceFlow id="firstFlow1" sourceRef="firstStart" targetRef="reserveFirst" />
            <userTask id="reserveFirst" name="Reserve First" />
            <boundaryEvent id="reserveFirstCompensation" attachedToRef="reserveFirst">
                <compensateEventDefinition />
            </boundaryEvent>
            <serviceTask id="undoFirstReservation"
                         name="Undo First Reservation"
                         isForCompensation="true"
                         flowable:type="http">
                <extensionElements>
                    <flowable:requestMethod>POST</flowable:requestMethod>
                    <flowable:requestUrl>https://example.flowable.local/compensation/end/first</flowable:requestUrl>
                </extensionElements>
            </serviceTask>
            <sequenceFlow id="firstFlow2" sourceRef="reserveFirst" targetRef="firstEnd" />
            <endEvent id="firstEnd" />
        </subProcess>
        <sequenceFlow id="flow2" sourceRef="firstScope" targetRef="secondScope" />
        <subProcess id="secondScope">
            <startEvent id="secondStart" />
            <sequenceFlow id="secondFlow1" sourceRef="secondStart" targetRef="reserveSecond" />
            <userTask id="reserveSecond" name="Reserve Second" />
            <boundaryEvent id="reserveSecondCompensation" attachedToRef="reserveSecond">
                <compensateEventDefinition />
            </boundaryEvent>
            <serviceTask id="undoSecondReservation"
                         name="Undo Second Reservation"
                         isForCompensation="true"
                         flowable:type="http">
                <extensionElements>
                    <flowable:requestMethod>POST</flowable:requestMethod>
                    <flowable:requestUrl>https://example.flowable.local/compensation/end/second</flowable:requestUrl>
                </extensionElements>
            </serviceTask>
            <sequenceFlow id="secondFlow2" sourceRef="reserveSecond" targetRef="secondEnd" />
            <endEvent id="secondEnd" />
        </subProcess>
        <sequenceFlow id="flow3" sourceRef="secondScope" targetRef="compensateEnd" />
        <endEvent id="compensateEnd">
            <compensateEventDefinition />
        </endEvent>
    </process>
</definitions>"#;

    repository_service
        .deploy(
            repository_service
                .create_deployment()
                .name("End Compensation LIFO Order Deployment".to_string())
                .add_string(
                    "end_compensation_lifo_order.bpmn20.xml".to_string(),
                    xml.to_string(),
                ),
        )
        .unwrap();

    let process_definition_id = repository_service.get_process_definition_ids().unwrap()[0].clone();
    let process_instance = runtime_service
        .start_process_instance(
            runtime_service
                .create_process_instance_builder()
                .process_definition_id(process_definition_id),
        )
        .unwrap();

    let tasks = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].task_definition_key, "reserveFirst");
    task_service
        .complete_task_by_id(tasks[0].id.clone())
        .unwrap();

    let tasks = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].task_definition_key, "reserveSecond");
    task_service
        .complete_task_by_id(tasks[0].id.clone())
        .unwrap();

    let handler_activity_ids = {
        let mut session = runtime_store.create_session().unwrap();
        let mut params = flowable_engine::persistence::DbParams::new();
        params.push(process_instance.id.as_str());
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
    };

    assert_eq!(
        handler_activity_ids,
        vec![
            "undoSecondReservation".to_string(),
            "undoFirstReservation".to_string(),
        ],
        "unscoped compensation end events must dispatch completed handlers in reverse completion order"
    );
}

#[test]
fn test_throw_compensation_with_activity_ref_consumes_only_matching_subscription() {
    let process_engine = ProcessEngine::new("throw-compensation-activity-ref-test".to_string());
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();
    let task_service = process_engine.get_task_service();
    let runtime_store = process_engine.get_runtime_store();

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
    <process id="throwCompensationActivityRefProcess" isExecutable="true">
        <startEvent id="start" />
        <sequenceFlow id="flow1" sourceRef="start" targetRef="hotelScope" />
        <subProcess id="hotelScope">
            <startEvent id="hotelStart" />
            <sequenceFlow id="hotelFlow1" sourceRef="hotelStart" targetRef="bookHotel" />
            <userTask id="bookHotel" name="Book Hotel" />
            <boundaryEvent id="bookHotelCompensation" attachedToRef="bookHotel">
                <compensateEventDefinition />
            </boundaryEvent>
            <userTask id="undoBookHotel" name="Undo Book Hotel" isForCompensation="true" />
            <sequenceFlow id="hotelFlow2" sourceRef="bookHotel" targetRef="hotelEnd" />
            <endEvent id="hotelEnd" />
        </subProcess>
        <sequenceFlow id="flow2" sourceRef="hotelScope" targetRef="flightScope" />
        <subProcess id="flightScope">
            <startEvent id="flightStart" />
            <sequenceFlow id="flightFlow1" sourceRef="flightStart" targetRef="bookFlight" />
            <userTask id="bookFlight" name="Book Flight" />
            <boundaryEvent id="bookFlightCompensation" attachedToRef="bookFlight">
                <compensateEventDefinition />
            </boundaryEvent>
            <userTask id="undoBookFlight" name="Undo Book Flight" isForCompensation="true" />
            <sequenceFlow id="flightFlow2" sourceRef="bookFlight" targetRef="flightEnd" />
            <endEvent id="flightEnd" />
        </subProcess>
        <sequenceFlow id="flow3" sourceRef="flightScope" targetRef="throwHotelCompensation" />
        <intermediateThrowEvent id="throwHotelCompensation">
            <compensateEventDefinition activityRef="bookHotel" />
        </intermediateThrowEvent>
        <sequenceFlow id="flow4" sourceRef="throwHotelCompensation" targetRef="afterCompensation" />
        <userTask id="afterCompensation" name="After Compensation" />
        <sequenceFlow id="flow5" sourceRef="afterCompensation" targetRef="end" />
        <endEvent id="end" />
    </process>
</definitions>"#;

    repository_service
        .deploy(
            repository_service
                .create_deployment()
                .name("Throw Compensation ActivityRef Deployment".to_string())
                .add_string(
                    "throw_compensation_activity_ref.bpmn20.xml".to_string(),
                    xml.to_string(),
                ),
        )
        .unwrap();

    let process_definition_id = repository_service.get_process_definition_ids().unwrap()[0].clone();
    let process_instance = runtime_service
        .start_process_instance(
            runtime_service
                .create_process_instance_builder()
                .process_definition_id(process_definition_id),
        )
        .unwrap();

    let tasks = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].task_definition_key, "bookHotel");
    task_service
        .complete_task_by_id(tasks[0].id.clone())
        .unwrap();

    let mut hotel_session = runtime_store.create_session().unwrap();
    let hotel_subscriptions = runtime_store.find_compensation_subscriptions_by_process_instance_id(
        &process_instance.id,
        &mut hotel_session,
    );
    assert_eq!(hotel_subscriptions.len(), 1);
    assert_eq!(hotel_subscriptions[0].activity_id, "bookHotel");
    drop(hotel_session);

    let tasks = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].task_definition_key, "bookFlight");
    task_service
        .complete_task_by_id(tasks[0].id.clone())
        .unwrap();

    let mut task_keys = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap()
        .into_iter()
        .map(|task| task.task_definition_key)
        .collect::<Vec<_>>();
    task_keys.sort();

    assert_eq!(
        task_keys,
        vec!["afterCompensation".to_string(), "undoBookHotel".to_string()],
        "activityRef should trigger only the matching completed activity handler"
    );
    assert!(
        !task_keys.iter().any(|key| key == "undoBookFlight"),
        "activityRef=bookHotel must not trigger the bookFlight compensation handler"
    );

    let mut sub_session = runtime_store.create_session().unwrap();
    let remaining_subscriptions = runtime_store
        .find_compensation_subscriptions_by_process_instance_id(
            &process_instance.id,
            &mut sub_session,
        );
    assert_eq!(remaining_subscriptions.len(), 1);
    assert_eq!(
        remaining_subscriptions[0].activity_id, "bookFlight",
        "unmatched completed activity subscriptions must remain registered"
    );
}

#[test]
fn test_end_compensation_with_activity_ref_consumes_only_matching_subscription() {
    let process_engine = ProcessEngine::new("end-compensation-activity-ref-test".to_string());
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();
    let task_service = process_engine.get_task_service();
    let runtime_store = process_engine.get_runtime_store();

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
    <process id="endCompensationActivityRefProcess" isExecutable="true">
        <startEvent id="start" />
        <sequenceFlow id="flow1" sourceRef="start" targetRef="hotelScope" />
        <subProcess id="hotelScope">
            <startEvent id="hotelStart" />
            <sequenceFlow id="hotelFlow1" sourceRef="hotelStart" targetRef="bookHotel" />
            <userTask id="bookHotel" name="Book Hotel" />
            <boundaryEvent id="bookHotelCompensation" attachedToRef="bookHotel">
                <compensateEventDefinition />
            </boundaryEvent>
            <userTask id="undoBookHotel" name="Undo Book Hotel" isForCompensation="true" />
            <sequenceFlow id="hotelFlow2" sourceRef="bookHotel" targetRef="hotelEnd" />
            <endEvent id="hotelEnd" />
        </subProcess>
        <sequenceFlow id="flow2" sourceRef="hotelScope" targetRef="flightScope" />
        <subProcess id="flightScope">
            <startEvent id="flightStart" />
            <sequenceFlow id="flightFlow1" sourceRef="flightStart" targetRef="bookFlight" />
            <userTask id="bookFlight" name="Book Flight" />
            <boundaryEvent id="bookFlightCompensation" attachedToRef="bookFlight">
                <compensateEventDefinition />
            </boundaryEvent>
            <userTask id="undoBookFlight" name="Undo Book Flight" isForCompensation="true" />
            <sequenceFlow id="flightFlow2" sourceRef="bookFlight" targetRef="flightEnd" />
            <endEvent id="flightEnd" />
        </subProcess>
        <sequenceFlow id="flow3" sourceRef="flightScope" targetRef="compensateHotelEnd" />
        <endEvent id="compensateHotelEnd">
            <compensateEventDefinition activityRef="bookHotel" />
        </endEvent>
    </process>
</definitions>"#;

    repository_service
        .deploy(
            repository_service
                .create_deployment()
                .name("End Compensation ActivityRef Deployment".to_string())
                .add_string(
                    "end_compensation_activity_ref.bpmn20.xml".to_string(),
                    xml.to_string(),
                ),
        )
        .unwrap();

    let process_definition_id = repository_service.get_process_definition_ids().unwrap()[0].clone();
    let process_instance = runtime_service
        .start_process_instance(
            runtime_service
                .create_process_instance_builder()
                .process_definition_id(process_definition_id),
        )
        .unwrap();

    let tasks = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].task_definition_key, "bookHotel");
    task_service
        .complete_task_by_id(tasks[0].id.clone())
        .unwrap();

    let tasks = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].task_definition_key, "bookFlight");
    task_service
        .complete_task_by_id(tasks[0].id.clone())
        .unwrap();

    let task_keys = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap()
        .into_iter()
        .map(|task| task.task_definition_key)
        .collect::<Vec<_>>();

    assert_eq!(
        task_keys,
        vec!["undoBookHotel".to_string()],
        "compensation end event should trigger only the activityRef handler"
    );

    let mut sub_session = runtime_store.create_session().unwrap();
    let remaining_subscriptions = runtime_store
        .find_compensation_subscriptions_by_process_instance_id(
            &process_instance.id,
            &mut sub_session,
        );
    assert_eq!(remaining_subscriptions.len(), 1);
    assert_eq!(
        remaining_subscriptions[0].activity_id, "bookFlight",
        "compensation end event must not delete unmatched subscriptions"
    );
}

#[test]
fn test_throw_compensation_runs_only_registered_completed_activity_handlers() {
    let process_engine = ProcessEngine::new("throw-compensation-completed-only-test".to_string());
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();
    let task_service = process_engine.get_task_service();
    let runtime_store = process_engine.get_runtime_store();

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
    <process id="throwCompensationCompletedOnlyProcess" isExecutable="true">
        <startEvent id="start" />
        <sequenceFlow id="flow1" sourceRef="start" targetRef="fork" />
        <parallelGateway id="fork" />
        <sequenceFlow id="flow2" sourceRef="fork" targetRef="completedScope" />
        <sequenceFlow id="flow3" sourceRef="fork" targetRef="waitingScope" />

        <subProcess id="completedScope">
            <startEvent id="completedScopeStart" />
            <sequenceFlow id="completedScopeFlow1" sourceRef="completedScopeStart" targetRef="doWork" />
            <userTask id="doWork" name="Do Work" />
            <boundaryEvent id="doWorkCompensation" attachedToRef="doWork">
                <compensateEventDefinition />
            </boundaryEvent>
            <userTask id="undoDoWork" name="Undo Do Work" isForCompensation="true" />
            <sequenceFlow id="completedScopeFlow2" sourceRef="doWork" targetRef="throwCompensation" />
            <intermediateThrowEvent id="throwCompensation">
                <compensateEventDefinition />
            </intermediateThrowEvent>
            <sequenceFlow id="completedScopeFlow3" sourceRef="throwCompensation" targetRef="completedScopeEnd" />
            <endEvent id="completedScopeEnd" />
        </subProcess>

        <subProcess id="waitingScope">
            <startEvent id="waitingScopeStart" />
            <sequenceFlow id="waitingScopeFlow1" sourceRef="waitingScopeStart" targetRef="waitWork" />
            <userTask id="waitWork" name="Wait Work" />
            <boundaryEvent id="waitWorkCompensation" attachedToRef="waitWork">
                <compensateEventDefinition />
            </boundaryEvent>
            <userTask id="undoWaitWork" name="Undo Wait Work" isForCompensation="true" />
            <sequenceFlow id="waitingScopeFlow2" sourceRef="waitWork" targetRef="waitingScopeEnd" />
            <endEvent id="waitingScopeEnd" />
        </subProcess>
    </process>
</definitions>"#;

    repository_service
        .deploy(
            repository_service
                .create_deployment()
                .name("Throw Compensation Completed Only Deployment".to_string())
                .add_string(
                    "throw_compensation_completed_only.bpmn20.xml".to_string(),
                    xml.to_string(),
                ),
        )
        .unwrap();

    let process_definition_id = repository_service.get_process_definition_ids().unwrap()[0].clone();
    let process_instance = runtime_service
        .start_process_instance(
            runtime_service
                .create_process_instance_builder()
                .process_definition_id(process_definition_id),
        )
        .unwrap();

    let tasks = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 2);

    let completed_task = tasks
        .iter()
        .find(|task| task.task_definition_key == "doWork")
        .expect("doWork should be active before it is completed");
    task_service
        .complete_task_by_id(completed_task.id.clone())
        .unwrap();

    let mut task_keys = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap()
        .into_iter()
        .map(|task| task.task_definition_key)
        .collect::<Vec<_>>();
    task_keys.sort();

    assert_eq!(
        task_keys,
        vec!["undoDoWork".to_string(), "waitWork".to_string()],
        "throw compensation should run only the handler registered after doWork completed"
    );
    assert!(
        !task_keys.iter().any(|key| key == "undoWaitWork"),
        "the compensation handler for the still-active waitWork task must not run"
    );

    let mut sub_session = runtime_store.create_session().unwrap();
    let remaining_subscriptions = runtime_store
        .find_compensation_subscriptions_by_process_instance_id(
            &process_instance.id,
            &mut sub_session,
        );
    assert!(
        remaining_subscriptions.is_empty(),
        "throw compensation should consume registered compensation subscriptions"
    );
}

#[test]
fn test_compensation_newest_first_uses_explicit_subscription_order_not_sqlite_rowid() {
    let engine = Arc::new(ProcessEngine::new(
        "compensation-explicit-order-store-test".to_string(),
    ));
    let store = engine.get_runtime_store();
    let pi_id = "explicit_order_pi";

    let mut insert_session = store.create_session().unwrap();
    for (id, activity_id, compensation_activity_id) in [
        ("sub_first", "bookFirst", "undoBookFirst"),
        ("sub_second", "bookSecond", "undoBookSecond"),
        ("sub_third", "bookThird", "undoBookThird"),
    ] {
        store.insert_compensation_subscription(
            CompensationSubscription {
                id: id.to_string(),
                process_instance_id: pi_id.to_string(),
                execution_id: format!("exec_{activity_id}"),
                activity_id: activity_id.to_string(),
                compensation_activity_id: compensation_activity_id.to_string(),
                subscription_order: 0,
                variables_snapshot: Default::default(),
            },
            &mut insert_session,
        );
    }
    insert_session.flush_and_commit().unwrap();

    {
        let mut update_session = store.create_session().unwrap();
        let mut p1 = flowable_engine::persistence::DbParams::new();
        p1.push(1000i64);
        p1.push("sub_first");
        update_session
            .execute_raw(
                "UPDATE compensation_subscriptions SET rowid = ?1 WHERE id = ?2",
                p1,
            )
            .unwrap();
        let mut p2 = flowable_engine::persistence::DbParams::new();
        p2.push(999i64);
        p2.push("sub_second");
        update_session
            .execute_raw(
                "UPDATE compensation_subscriptions SET rowid = ?1 WHERE id = ?2",
                p2,
            )
            .unwrap();
        let mut p3 = flowable_engine::persistence::DbParams::new();
        p3.push(998i64);
        p3.push("sub_third");
        update_session
            .execute_raw(
                "UPDATE compensation_subscriptions SET rowid = ?1 WHERE id = ?2",
                p3,
            )
            .unwrap();
        update_session.flush_and_commit().unwrap();
    }

    let mut newest_session = store.create_session().unwrap();
    let activity_ids = store
        .find_compensation_subscriptions_by_process_instance_id_newest_first(
            pi_id,
            &mut newest_session,
        )
        .into_iter()
        .map(|subscription| subscription.activity_id)
        .collect::<Vec<_>>();

    assert_eq!(
        activity_ids,
        vec![
            "bookThird".to_string(),
            "bookSecond".to_string(),
            "bookFirst".to_string(),
        ],
        "newest-first compensation ordering must follow the explicit subscription order, not SQLite rowid"
    );
}

#[test]
fn test_minimal_compensation_registration_and_cancel() {
    let engine = Arc::new(ProcessEngine::new("comp-test".to_string()));
    let store = engine.get_runtime_store();

    let pi_id = "test_pi";

    // 1. Manually register a compensation subscription
    let sub = CompensationSubscription {
        id: "sub1".to_string(),
        process_instance_id: pi_id.to_string(),
        execution_id: "exec1".to_string(),
        activity_id: "serviceTask1".to_string(),
        compensation_activity_id: "undoServiceTask1".to_string(),
        subscription_order: 0,
        variables_snapshot: Default::default(),
    };
    let mut insert_session = store.create_session().unwrap();
    store.insert_compensation_subscription(sub, &mut insert_session);
    insert_session.flush_and_commit().unwrap();

    // 2. Setup a manual Execution
    let mut execution = Execution {
        id: "exec_test".to_string(),
        process_instance_id: Some(pi_id.to_string()),
        ..Default::default()
    };

    // 3. Trigger CancelEndEventBehavior
    let behavior = CancelEndEventActivityBehavior::new();
    let mut ctx = flowable_engine::interceptor::command_context::CommandContext::new(
        engine.get_command_executor().deployment_manager().clone(),
        store.clone(),
        store.create_session().unwrap(),
        engine.get_config(),
        Arc::new(DeterministicHttpRuntime::default()),
    );

    behavior.execute(&mut execution, &mut ctx).unwrap();
    ctx.session().flush_and_commit().unwrap();

    // Verification: Success if no panic and println shows the trigger
    assert!(execution.is_ended);

    // Check that subscriptions are cleaned up
    let mut check_session = store.create_session().unwrap();
    let remaining_subs =
        store.find_compensation_subscriptions_by_process_instance_id(pi_id, &mut check_session);
    assert!(
        remaining_subs.is_empty(),
        "Compensation subscriptions should be cleaned up"
    );
    check_session.rollback().unwrap();

    // We can't easily check agenda operations directly as it's an internal queue, but
    // we can check the execution_entity_manager to see if it has the new execution.
    // Wait, the context is dropped here, so we'd have to check store for new executions.
    // However, context execution_entity_manager is backed by store.
    let execs: Vec<Execution> = store
        .db_store()
        .find_all_by("executions", "process_instance_id", pi_id)
        .unwrap();
    assert!(
        !execs.is_empty(),
        "Should have created a new execution for compensation"
    );
}

/// P44 regression: pre-6.4.0 models may reference the compensation handler
/// activity directly in `activityRef` instead of the compensated activity.
/// Java `IntermediateThrowCompensationEventActivityBehavior` (78-108) falls
/// back to scanning the model for an `isForCompensation` activity and
/// resolving it back to the compensated activity. The Rust engine matches
/// `activityRef` against both the subscription's `activity_id` and its
/// `compensation_activity_id`, so a handler ref triggers the right handler.
#[test]
fn test_throw_compensation_activity_ref_referencing_handler_resolves_via_reverse_lookup() {
    let process_engine =
        ProcessEngine::new("throw-compensation-activity-ref-reverse-lookup-test".to_string());
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();
    let task_service = process_engine.get_task_service();
    let runtime_store = process_engine.get_runtime_store();

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
    <process id="throwCompensationActivityRefReverseLookupProcess" isExecutable="true">
        <startEvent id="start" />
        <sequenceFlow id="flow1" sourceRef="start" targetRef="hotelScope" />
        <subProcess id="hotelScope">
            <startEvent id="hotelStart" />
            <sequenceFlow id="hotelFlow1" sourceRef="hotelStart" targetRef="bookHotel" />
            <userTask id="bookHotel" name="Book Hotel" />
            <boundaryEvent id="bookHotelCompensation" attachedToRef="bookHotel">
                <compensateEventDefinition />
            </boundaryEvent>
            <userTask id="undoBookHotel" name="Undo Book Hotel" isForCompensation="true" />
            <sequenceFlow id="hotelFlow2" sourceRef="bookHotel" targetRef="hotelEnd" />
            <endEvent id="hotelEnd" />
        </subProcess>
        <sequenceFlow id="flow2" sourceRef="hotelScope" targetRef="flightScope" />
        <subProcess id="flightScope">
            <startEvent id="flightStart" />
            <sequenceFlow id="flightFlow1" sourceRef="flightStart" targetRef="bookFlight" />
            <userTask id="bookFlight" name="Book Flight" />
            <boundaryEvent id="bookFlightCompensation" attachedToRef="bookFlight">
                <compensateEventDefinition />
            </boundaryEvent>
            <userTask id="undoBookFlight" name="Undo Book Flight" isForCompensation="true" />
            <sequenceFlow id="flightFlow2" sourceRef="bookFlight" targetRef="flightEnd" />
            <endEvent id="flightEnd" />
        </subProcess>
        <sequenceFlow id="flow3" sourceRef="flightScope" targetRef="throwHotelCompensation" />
        <intermediateThrowEvent id="throwHotelCompensation">
            <compensateEventDefinition activityRef="undoBookHotel" />
        </intermediateThrowEvent>
        <sequenceFlow id="flow4" sourceRef="throwHotelCompensation" targetRef="afterCompensation" />
        <userTask id="afterCompensation" name="After Compensation" />
        <sequenceFlow id="flow5" sourceRef="afterCompensation" targetRef="end" />
        <endEvent id="end" />
    </process>
</definitions>"#;

    repository_service
        .deploy(
            repository_service
                .create_deployment()
                .name("Throw Compensation ActivityRef Reverse Lookup Deployment".to_string())
                .add_string(
                    "throw_compensation_activity_ref_reverse_lookup.bpmn20.xml".to_string(),
                    xml.to_string(),
                ),
        )
        .unwrap();

    let process_definition_id = repository_service.get_process_definition_ids().unwrap()[0].clone();
    let process_instance = runtime_service
        .start_process_instance(
            runtime_service
                .create_process_instance_builder()
                .process_definition_id(process_definition_id),
        )
        .unwrap();

    let tasks = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].task_definition_key, "bookHotel");
    task_service
        .complete_task_by_id(tasks[0].id.clone())
        .unwrap();

    let tasks = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].task_definition_key, "bookFlight");
    task_service
        .complete_task_by_id(tasks[0].id.clone())
        .unwrap();

    // activityRef points at the compensation HANDLER (undoBookHotel), not the
    // compensated activity (bookHotel). The reverse lookup must resolve it.
    let mut task_keys = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap()
        .into_iter()
        .map(|task| task.task_definition_key)
        .collect::<Vec<_>>();
    task_keys.sort();

    assert_eq!(
        task_keys,
        vec!["afterCompensation".to_string(), "undoBookHotel".to_string()],
        "activityRef referencing the handler activity must trigger the matching \
         compensation handler via reverse lookup"
    );
    assert!(
        !task_keys.iter().any(|key| key == "undoBookFlight"),
        "activityRef=undoBookHotel must not trigger the bookFlight compensation handler"
    );

    let mut sub_session = runtime_store.create_session().unwrap();
    let remaining_subscriptions = runtime_store
        .find_compensation_subscriptions_by_process_instance_id(
            &process_instance.id,
            &mut sub_session,
        );
    assert_eq!(remaining_subscriptions.len(), 1);
    assert_eq!(
        remaining_subscriptions[0].activity_id, "bookFlight",
        "only the unmatched bookFlight subscription should remain registered"
    );
}
