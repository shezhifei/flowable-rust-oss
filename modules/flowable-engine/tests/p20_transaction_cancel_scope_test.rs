//! P20-A/B contract tests: a cancel end event compensates and destroys ONLY
//! its enclosing transaction scope, never the whole process instance.
//!
//! Java evidence:
//! - `CancelEndEventActivityBehavior#execute` (43-135): walks up to the
//!   enclosing sub-process scope execution, creates the compensation copy for
//!   THAT scope only, deletes all executions inside the scope
//!   (TRANSACTION_CANCELED) and moves the execution to the cancel boundary.
//! - `BoundaryCancelEventActivityBehavior#trigger` (41-86): compensation is
//!   thrown from the subscriptions of the transaction scope execution only.
//! - `TransactionSubProcessTest.testNestedCancelInner` (:175): inner cancel
//!   compensates inner activities only; the outer transaction's task and
//!   subscriptions are untouched.
//! - `TransactionSubProcessTest.testNestedCancelOuter` (:229): outer cancel
//!   destroys the still-active inner transaction (its subscriptions are
//!   removed WITHOUT running the handlers) while completed outer activities
//!   are compensated.
//! - `TransactionSubProcessTest.testCancelEndConcurrent` (:131): all
//!   concurrent executions inside the transaction are destroyed and every
//!   completed activity of the transaction is compensated.
//! - `TransactionSubProcessTest.testSimpleCaseTxSuccessful` (:59-70): after a
//!   transaction completes successfully its compensation subscriptions are
//!   RETAINED (Java: moved to an event-scope execution) so a later
//!   compensation throw can still compensate the transaction's activities.

use flowable_engine::engine::process_engine::ProcessEngine;

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

fn task_keys(engine: &ProcessEngine, process_instance_id: &str) -> Vec<String> {
    let mut keys = engine
        .get_task_service()
        .get_tasks_by_process_instance_id(process_instance_id.to_string())
        .unwrap()
        .into_iter()
        .map(|task| task.task_definition_key)
        .collect::<Vec<_>>();
    keys.sort();
    keys
}

fn complete_task_by_key(engine: &ProcessEngine, process_instance_id: &str, key: &str) {
    let task_service = engine.get_task_service();
    let task = task_service
        .get_tasks_by_process_instance_id(process_instance_id.to_string())
        .unwrap()
        .into_iter()
        .find(|task| task.task_definition_key == key)
        .unwrap_or_else(|| panic!("expected an open task '{key}'"));
    task_service.complete_task_by_id(task.id).unwrap();
}

fn subscription_activity_ids(engine: &ProcessEngine, process_instance_id: &str) -> Vec<String> {
    let runtime_store = engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let mut activity_ids = runtime_store
        .find_compensation_subscriptions_by_process_instance_id(process_instance_id, &mut session)
        .into_iter()
        .map(|subscription| subscription.activity_id)
        .collect::<Vec<_>>();
    let _ = session.rollback();
    activity_ids.sort();
    activity_ids
}

/// A cancel end event must not consume compensation subscriptions registered
/// OUTSIDE its transaction (process-level `pBook` stays untouched).
const SIMPLE_SCOPE_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
    <process id="p20TxScopeSimple" isExecutable="true">
        <startEvent id="start" />
        <sequenceFlow id="f1" sourceRef="start" targetRef="pBook" />
        <userTask id="pBook" />
        <boundaryEvent id="pBookComp" attachedToRef="pBook">
            <compensateEventDefinition />
        </boundaryEvent>
        <userTask id="undoPBook" isForCompensation="true" />
        <sequenceFlow id="f2" sourceRef="pBook" targetRef="tx" />
        <transaction id="tx">
            <startEvent id="txStart" />
            <sequenceFlow id="tf1" sourceRef="txStart" targetRef="txBook" />
            <userTask id="txBook" />
            <boundaryEvent id="txBookComp" attachedToRef="txBook">
                <compensateEventDefinition />
            </boundaryEvent>
            <userTask id="undoTxBook" isForCompensation="true" />
            <sequenceFlow id="tf2" sourceRef="txBook" targetRef="txAsk" />
            <userTask id="txAsk" />
            <sequenceFlow id="tf3" sourceRef="txAsk" targetRef="txCancelEnd" />
            <endEvent id="txCancelEnd">
                <cancelEventDefinition />
            </endEvent>
        </transaction>
        <boundaryEvent id="catchCancel" attachedToRef="tx">
            <cancelEventDefinition />
        </boundaryEvent>
        <sequenceFlow id="f3" sourceRef="catchCancel" targetRef="afterCancellation" />
        <userTask id="afterCancellation" />
        <sequenceFlow id="f4" sourceRef="tx" targetRef="afterSuccess" />
        <userTask id="afterSuccess" />
        <sequenceFlow id="f5" sourceRef="afterCancellation" targetRef="end" />
        <sequenceFlow id="f6" sourceRef="afterSuccess" targetRef="end" />
        <endEvent id="end" />
    </process>
</definitions>"#;

#[test]
fn cancel_end_event_only_compensates_own_transaction_scope() {
    let engine = ProcessEngine::new("p20-tx-scope-simple".to_string());
    deploy(&engine, "p20_tx_scope_simple", SIMPLE_SCOPE_XML);
    let pi = start_by_key(&engine, "p20TxScopeSimple");

    complete_task_by_key(&engine, &pi, "pBook");
    complete_task_by_key(&engine, &pi, "txBook");
    assert_eq!(
        subscription_activity_ids(&engine, &pi),
        vec!["pBook".to_string(), "txBook".to_string()]
    );

    // cancel end event fires
    complete_task_by_key(&engine, &pi, "txAsk");

    assert_eq!(
        task_keys(&engine, &pi),
        vec!["undoTxBook".to_string()],
        "only the transaction's own completed activity may be compensated; \
         the process-level handler undoPBook must NOT spawn"
    );
    assert_eq!(
        subscription_activity_ids(&engine, &pi),
        vec!["pBook".to_string()],
        "the process-level subscription must survive the transaction cancel"
    );

    // compensation done -> cancel boundary path activates
    complete_task_by_key(&engine, &pi, "undoTxBook");
    assert_eq!(
        task_keys(&engine, &pi),
        vec!["afterCancellation".to_string()]
    );
    assert_eq!(
        subscription_activity_ids(&engine, &pi),
        vec!["pBook".to_string()]
    );
}

/// Java `TransactionSubProcessTest.testNestedCancelInner`: the inner cancel
/// end event only compensates the INNER transaction's completed activities;
/// the outer transaction's task and subscriptions stay untouched, and the
/// process continues via the inner cancel boundary.
const NESTED_INNER_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
    <process id="p20NestedInner" isExecutable="true">
        <startEvent id="start" />
        <sequenceFlow id="f1" sourceRef="start" targetRef="outerTx" />
        <transaction id="outerTx">
            <startEvent id="oStart" />
            <sequenceFlow id="of1" sourceRef="oStart" targetRef="outerBook" />
            <userTask id="outerBook" />
            <boundaryEvent id="outerBookComp" attachedToRef="outerBook">
                <compensateEventDefinition />
            </boundaryEvent>
            <userTask id="undoOuterBook" isForCompensation="true" />
            <sequenceFlow id="of2" sourceRef="outerBook" targetRef="oFork" />
            <parallelGateway id="oFork" />
            <sequenceFlow id="of3" sourceRef="oFork" targetRef="bookFlight" />
            <userTask id="bookFlight" />
            <sequenceFlow id="of4" sourceRef="bookFlight" targetRef="bEnd" />
            <endEvent id="bEnd" />
            <sequenceFlow id="of5" sourceRef="oFork" targetRef="innerTx" />
            <transaction id="innerTx">
                <startEvent id="iStart" />
                <sequenceFlow id="if1" sourceRef="iStart" targetRef="innerBook" />
                <userTask id="innerBook" />
                <boundaryEvent id="innerBookComp" attachedToRef="innerBook">
                    <compensateEventDefinition />
                </boundaryEvent>
                <userTask id="undoInnerBook" isForCompensation="true" />
                <sequenceFlow id="if2" sourceRef="innerBook" targetRef="innerAsk" />
                <userTask id="innerAsk" />
                <sequenceFlow id="if3" sourceRef="innerAsk" targetRef="innerCancelEnd" />
                <endEvent id="innerCancelEnd">
                    <cancelEventDefinition />
                </endEvent>
            </transaction>
            <boundaryEvent id="innerCatchCancel" attachedToRef="innerTx">
                <cancelEventDefinition />
            </boundaryEvent>
            <sequenceFlow id="of6" sourceRef="innerCatchCancel" targetRef="afterInnerCancellation" />
            <userTask id="afterInnerCancellation" />
            <sequenceFlow id="of7" sourceRef="afterInnerCancellation" targetRef="aEnd" />
            <endEvent id="aEnd" />
        </transaction>
        <sequenceFlow id="f2" sourceRef="outerTx" targetRef="afterOuter" />
        <userTask id="afterOuter" />
        <sequenceFlow id="f3" sourceRef="afterOuter" targetRef="end" />
        <endEvent id="end" />
    </process>
</definitions>"#;

#[test]
fn nested_cancel_inner_only_compensates_inner_transaction() {
    let engine = ProcessEngine::new("p20-nested-inner".to_string());
    deploy(&engine, "p20_nested_inner", NESTED_INNER_XML);
    let pi = start_by_key(&engine, "p20NestedInner");

    complete_task_by_key(&engine, &pi, "outerBook");
    // Nested transaction must actually start (recursive Transaction lookup).
    assert_eq!(
        task_keys(&engine, &pi),
        vec!["bookFlight".to_string(), "innerBook".to_string()],
        "the nested transaction must start its own start event"
    );

    complete_task_by_key(&engine, &pi, "innerBook");
    assert_eq!(
        subscription_activity_ids(&engine, &pi),
        vec!["innerBook".to_string(), "outerBook".to_string()]
    );

    // inner cancel end event fires
    complete_task_by_key(&engine, &pi, "innerAsk");

    assert_eq!(
        task_keys(&engine, &pi),
        vec!["bookFlight".to_string(), "undoInnerBook".to_string()],
        "inner cancel must only compensate the inner transaction's activities; \
         the outer task must survive and undoOuterBook must NOT spawn"
    );
    assert_eq!(
        subscription_activity_ids(&engine, &pi),
        vec!["outerBook".to_string()],
        "the outer transaction's subscription must survive the inner cancel"
    );

    // inner compensation done -> inner cancel boundary path activates
    complete_task_by_key(&engine, &pi, "undoInnerBook");
    assert_eq!(
        task_keys(&engine, &pi),
        vec![
            "afterInnerCancellation".to_string(),
            "bookFlight".to_string()
        ]
    );
}

/// Java `TransactionSubProcessTest.testNestedCancelOuter`: the outer cancel
/// destroys the still-active inner transaction — its subscriptions are
/// removed WITHOUT invoking the handlers — while the completed outer activity
/// is compensated.
const NESTED_OUTER_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
    <process id="p20NestedOuter" isExecutable="true">
        <startEvent id="start" />
        <sequenceFlow id="f1" sourceRef="start" targetRef="outerTx" />
        <transaction id="outerTx">
            <startEvent id="oStart" />
            <sequenceFlow id="of1" sourceRef="oStart" targetRef="oFork" />
            <parallelGateway id="oFork" />
            <sequenceFlow id="of2" sourceRef="oFork" targetRef="bookFlight" />
            <userTask id="bookFlight" />
            <boundaryEvent id="bookFlightComp" attachedToRef="bookFlight">
                <compensateEventDefinition />
            </boundaryEvent>
            <userTask id="undoBookFlight" isForCompensation="true" />
            <sequenceFlow id="of3" sourceRef="bookFlight" targetRef="outerCancelEnd" />
            <endEvent id="outerCancelEnd">
                <cancelEventDefinition />
            </endEvent>
            <sequenceFlow id="of4" sourceRef="oFork" targetRef="innerTx" />
            <transaction id="innerTx">
                <startEvent id="iStart" />
                <sequenceFlow id="if1" sourceRef="iStart" targetRef="innerBook" />
                <userTask id="innerBook" />
                <boundaryEvent id="innerBookComp" attachedToRef="innerBook">
                    <compensateEventDefinition />
                </boundaryEvent>
                <userTask id="undoInnerBook" isForCompensation="true" />
                <sequenceFlow id="if2" sourceRef="innerBook" targetRef="innerAsk" />
                <userTask id="innerAsk" />
                <sequenceFlow id="if3" sourceRef="innerAsk" targetRef="iEnd" />
                <endEvent id="iEnd" />
            </transaction>
        </transaction>
        <boundaryEvent id="outerCatchCancel" attachedToRef="outerTx">
            <cancelEventDefinition />
        </boundaryEvent>
        <sequenceFlow id="f2" sourceRef="outerCatchCancel" targetRef="afterOuterCancellation" />
        <userTask id="afterOuterCancellation" />
        <sequenceFlow id="f3" sourceRef="outerTx" targetRef="afterSuccess" />
        <userTask id="afterSuccess" />
        <sequenceFlow id="f4" sourceRef="afterOuterCancellation" targetRef="end" />
        <sequenceFlow id="f5" sourceRef="afterSuccess" targetRef="end" />
        <endEvent id="end" />
    </process>
</definitions>"#;

#[test]
fn nested_cancel_outer_destroys_active_inner_transaction_without_compensation() {
    let engine = ProcessEngine::new("p20-nested-outer".to_string());
    deploy(&engine, "p20_nested_outer", NESTED_OUTER_XML);
    let pi = start_by_key(&engine, "p20NestedOuter");

    assert_eq!(
        task_keys(&engine, &pi),
        vec!["bookFlight".to_string(), "innerBook".to_string()]
    );

    // completed activity INSIDE the still-active inner transaction
    complete_task_by_key(&engine, &pi, "innerBook");
    assert_eq!(
        subscription_activity_ids(&engine, &pi),
        vec!["innerBook".to_string()]
    );

    // outer cancel end event fires (bookFlight registers its subscription on
    // completion, then the walker reaches outerCancelEnd)
    complete_task_by_key(&engine, &pi, "bookFlight");

    assert_eq!(
        task_keys(&engine, &pi),
        vec!["undoBookFlight".to_string()],
        "innerAsk must be destroyed with the inner transaction and \
         undoInnerBook must NOT spawn (inner tx still active => destroyed, \
         not compensated; Java testNestedCancelOuter)"
    );
    assert_eq!(
        subscription_activity_ids(&engine, &pi),
        Vec::<String>::new(),
        "all subscriptions of the cancelled outer scope must be removed"
    );

    // outer compensation done -> outer cancel boundary path activates
    complete_task_by_key(&engine, &pi, "undoBookFlight");
    assert_eq!(
        task_keys(&engine, &pi),
        vec!["afterOuterCancellation".to_string()]
    );
}

/// Java `TransactionSubProcessTest.testCancelEndConcurrent`: on cancel, ALL
/// concurrent executions inside the transaction are destroyed and every
/// completed activity of the transaction is compensated.
const CONCURRENT_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
    <process id="p20ConcurrentCancel" isExecutable="true">
        <startEvent id="start" />
        <sequenceFlow id="f1" sourceRef="start" targetRef="tx" />
        <transaction id="tx">
            <startEvent id="tStart" />
            <sequenceFlow id="tf1" sourceRef="tStart" targetRef="fork" />
            <parallelGateway id="fork" />
            <sequenceFlow id="tf2" sourceRef="fork" targetRef="bookHotel" />
            <userTask id="bookHotel" />
            <boundaryEvent id="bookHotelComp" attachedToRef="bookHotel">
                <compensateEventDefinition />
            </boundaryEvent>
            <userTask id="undoBookHotel" isForCompensation="true" />
            <sequenceFlow id="tf3" sourceRef="bookHotel" targetRef="waitHotel" />
            <userTask id="waitHotel" />
            <sequenceFlow id="tf4" sourceRef="waitHotel" targetRef="wEnd" />
            <endEvent id="wEnd" />
            <sequenceFlow id="tf5" sourceRef="fork" targetRef="askCustomer" />
            <userTask id="askCustomer" />
            <sequenceFlow id="tf6" sourceRef="askCustomer" targetRef="cancelEnd" />
            <endEvent id="cancelEnd">
                <cancelEventDefinition />
            </endEvent>
        </transaction>
        <boundaryEvent id="catchCancel" attachedToRef="tx">
            <cancelEventDefinition />
        </boundaryEvent>
        <sequenceFlow id="f2" sourceRef="catchCancel" targetRef="afterCancellation" />
        <userTask id="afterCancellation" />
        <sequenceFlow id="f3" sourceRef="tx" targetRef="afterSuccess" />
        <userTask id="afterSuccess" />
        <sequenceFlow id="f4" sourceRef="afterCancellation" targetRef="end" />
        <sequenceFlow id="f5" sourceRef="afterSuccess" targetRef="end" />
        <endEvent id="end" />
    </process>
</definitions>"#;

#[test]
fn cancel_end_concurrent_destroys_all_executions_and_compensates_completed() {
    let engine = ProcessEngine::new("p20-concurrent-cancel".to_string());
    deploy(&engine, "p20_concurrent_cancel", CONCURRENT_XML);
    let pi = start_by_key(&engine, "p20ConcurrentCancel");

    assert_eq!(
        task_keys(&engine, &pi),
        vec!["askCustomer".to_string(), "bookHotel".to_string()]
    );

    complete_task_by_key(&engine, &pi, "bookHotel");
    assert_eq!(
        task_keys(&engine, &pi),
        vec!["askCustomer".to_string(), "waitHotel".to_string()]
    );

    // cancel end event fires on the other concurrent branch
    complete_task_by_key(&engine, &pi, "askCustomer");

    assert_eq!(
        task_keys(&engine, &pi),
        vec!["undoBookHotel".to_string()],
        "the concurrent waitHotel execution must be destroyed and the \
         completed bookHotel compensated"
    );
    assert_eq!(
        subscription_activity_ids(&engine, &pi),
        Vec::<String>::new()
    );

    complete_task_by_key(&engine, &pi, "undoBookHotel");
    assert_eq!(
        task_keys(&engine, &pi),
        vec!["afterCancellation".to_string()]
    );
}

/// P20-B probe — Java `TransactionSubProcessTest.testSimpleCaseTxSuccessful`:
/// after the transaction completes successfully its compensation
/// subscriptions are retained (Java parks them on an event-scope execution),
/// so a later process-level compensation throw still reaches them.
///
/// Rust has no event-scope execution rows; subscriptions simply stay in the
/// flat store, which yields the same observable behavior. (Not reachable in
/// Rust: Java's additional transaction-level subscription on the process
/// instance execution and the event-scope execution row itself.)
const TX_SUCCESS_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
    <process id="p20TxSuccess" isExecutable="true">
        <startEvent id="start" />
        <sequenceFlow id="f1" sourceRef="start" targetRef="tx" />
        <transaction id="tx">
            <startEvent id="tStart" />
            <sequenceFlow id="tf1" sourceRef="tStart" targetRef="bookHotel" />
            <userTask id="bookHotel" />
            <boundaryEvent id="bookHotelComp" attachedToRef="bookHotel">
                <compensateEventDefinition />
            </boundaryEvent>
            <userTask id="undoBookHotel" isForCompensation="true" />
            <sequenceFlow id="tf2" sourceRef="bookHotel" targetRef="tEnd" />
            <endEvent id="tEnd" />
        </transaction>
        <sequenceFlow id="f2" sourceRef="tx" targetRef="afterTx" />
        <userTask id="afterTx" />
        <sequenceFlow id="f3" sourceRef="afterTx" targetRef="throwComp" />
        <intermediateThrowEvent id="throwComp">
            <compensateEventDefinition />
        </intermediateThrowEvent>
        <sequenceFlow id="f4" sourceRef="throwComp" targetRef="afterThrow" />
        <userTask id="afterThrow" />
        <sequenceFlow id="f5" sourceRef="afterThrow" targetRef="end" />
        <endEvent id="end" />
    </process>
</definitions>"#;

#[test]
fn tx_success_retains_subscriptions_for_later_compensation_throw() {
    let engine = ProcessEngine::new("p20-tx-success".to_string());
    deploy(&engine, "p20_tx_success", TX_SUCCESS_XML);
    let pi = start_by_key(&engine, "p20TxSuccess");

    // transaction completes successfully
    complete_task_by_key(&engine, &pi, "bookHotel");
    assert_eq!(task_keys(&engine, &pi), vec!["afterTx".to_string()]);
    assert_eq!(
        subscription_activity_ids(&engine, &pi),
        vec!["bookHotel".to_string()],
        "a successful transaction must RETAIN the compensation subscriptions \
         of its completed activities (Java testSimpleCaseTxSuccessful :59-70)"
    );

    // a later process-level compensation throw still compensates the
    // transaction's activity
    complete_task_by_key(&engine, &pi, "afterTx");
    assert_eq!(
        task_keys(&engine, &pi),
        vec!["afterThrow".to_string(), "undoBookHotel".to_string()],
        "an outer compensation throw must still reach the completed \
         transaction's activities"
    );
    assert_eq!(
        subscription_activity_ids(&engine, &pi),
        Vec::<String>::new()
    );
}
