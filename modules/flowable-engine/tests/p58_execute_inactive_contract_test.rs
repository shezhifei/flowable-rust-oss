//! P58: ExecuteInactiveBehaviors re-evaluation contract.
//!
//! Java runs `ExecuteInactiveBehaviorsOperation` after the agenda drains at
//! the end of every command (CommandInvoker.java:82-88), re-running the join
//! logic of inactive inclusive-gateway tokens
//! (ExecuteInactiveBehaviorsOperation.java:49-101,
//! InclusiveGatewayActivityBehavior.java:58-61 `executeInactive`).
//!
//! Contract under test: the ordering «token parks at the inclusive join
//! FIRST → sibling branch is destroyed LATER» must release the join in the
//! same command that destroyed the sibling. The reverse ordering is already
//! covered by P50 (`inclusive_gateway_semantics_test.rs`).

use flowable_engine::engine::process_engine::ProcessEngine;

/// Spec scenario 1: branch A parks at the inclusive join, branch B's user
/// task is destroyed by an interrupting boundary event afterwards. The join
/// must be re-evaluated at the end of the boundary-trigger command and flow
/// on to the successor task.
///
/// Java: CommandInvoker.java:82-88 + InclusiveGatewayActivityBehavior.java:95-115.
#[test]
fn parked_inclusive_join_releases_when_interrupting_boundary_destroys_sibling() {
    let process_engine = ProcessEngine::new("p58-boundary-after-park".to_string());
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();
    let task_service = process_engine.get_task_service();
    let runtime_store = process_engine.get_runtime_store();

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="p58BoundaryJoin" name="P58 Boundary Join" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="f1" sourceRef="start" targetRef="split" />

            <inclusiveGateway id="split" />
            <sequenceFlow id="f2" sourceRef="split" targetRef="taskA">
                <conditionExpression><![CDATA[${takeA == true}]]></conditionExpression>
            </sequenceFlow>
            <sequenceFlow id="f3" sourceRef="split" targetRef="taskB">
                <conditionExpression><![CDATA[${takeB == true}]]></conditionExpression>
            </sequenceFlow>

            <userTask id="taskA" name="Branch A" />
            <sequenceFlow id="f4" sourceRef="taskA" targetRef="join" />

            <userTask id="taskB" name="Branch B" />
            <boundaryEvent id="cancelB" attachedToRef="taskB" cancelActivity="true">
                <messageEventDefinition messageRef="cancelMsg" />
            </boundaryEvent>
            <sequenceFlow id="f_cancel" sourceRef="cancelB" targetRef="cancelEnd" />
            <endEvent id="cancelEnd" />
            <sequenceFlow id="f5" sourceRef="taskB" targetRef="join" />

            <inclusiveGateway id="join" />
            <sequenceFlow id="f6" sourceRef="join" targetRef="afterJoin" />
            <userTask id="afterJoin" name="After Join" />
            <sequenceFlow id="f7" sourceRef="afterJoin" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#;

    repository_service
        .deploy(
            repository_service
                .create_deployment()
                .name("p58-boundary-join".to_string())
                .add_string("p58BoundaryJoin.bpmn20.xml".to_string(), xml.to_string()),
        )
        .unwrap();

    let def_id = repository_service.get_process_definition_ids().unwrap()[0].clone();
    let pi = runtime_service
        .start_process_instance(
            runtime_service
                .create_process_instance_builder()
                .process_definition_id(def_id)
                .variable("takeA".to_string(), serde_json::Value::Bool(true))
                .variable("takeB".to_string(), serde_json::Value::Bool(true)),
        )
        .unwrap();

    let tasks = task_service
        .get_tasks_by_process_instance_id(pi.id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 2, "both inclusive branches should start");

    // Branch A completes FIRST and parks at the join (branch B can still
    // reach it, so the token is inactivated and waits).
    let task_a = tasks
        .iter()
        .find(|t| t.task_definition_key == "taskA")
        .cloned()
        .expect("branch A task");
    task_service.complete_task_by_id(task_a.id.clone()).unwrap();

    let tasks_parked = task_service
        .get_tasks_by_process_instance_id(pi.id.clone())
        .unwrap();
    assert_eq!(tasks_parked.len(), 1, "join must wait while B is reachable");
    assert_eq!(tasks_parked[0].task_definition_key, "taskB");

    // Now destroy branch B via the interrupting boundary. The parked join
    // token must be re-evaluated at the end of THIS command and released.
    runtime_service
        .trigger_boundary_event("cancelB".to_string(), pi.id.clone())
        .unwrap();

    let after = task_service
        .get_tasks_by_process_instance_id(pi.id.clone())
        .unwrap();
    assert_eq!(
        after.len(),
        1,
        "parked inclusive join must release in the boundary command; got {:?}",
        after
            .iter()
            .map(|t| t.task_definition_key.clone())
            .collect::<Vec<_>>()
    );
    assert_eq!(after[0].task_definition_key, "afterJoin");

    task_service.complete_task_by_id(after[0].id.clone()).unwrap();

    let mut session = runtime_store.create_session().unwrap();
    let stored_pi = runtime_store
        .find_process_instance(&pi.id, &mut session)
        .expect("process instance row");
    assert!(stored_pi.is_ended, "process should end after afterJoin");
}

/// Spec scenario 2 (terminate variant): branch B parks at the join first,
/// then the sibling subprocess branch is destroyed by a terminate end event.
/// The join must release in the terminate command.
///
/// Java: ExecuteInactiveBehaviorsOperation.java:79-88 (inactive scan) +
/// InclusiveGatewayActivityBehavior.java:59-61.
#[test]
fn parked_inclusive_join_releases_when_terminate_end_destroys_sibling() {
    let process_engine = ProcessEngine::new("p58-terminate-after-park".to_string());
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();
    let task_service = process_engine.get_task_service();
    let runtime_store = process_engine.get_runtime_store();

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="p58TerminateJoin" name="P58 Terminate Join" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="f1" sourceRef="start" targetRef="split" />

            <inclusiveGateway id="split" />
            <sequenceFlow id="f2" sourceRef="split" targetRef="sub">
                <conditionExpression><![CDATA[${takeSub == true}]]></conditionExpression>
            </sequenceFlow>
            <sequenceFlow id="f3" sourceRef="split" targetRef="taskB">
                <conditionExpression><![CDATA[${takeB == true}]]></conditionExpression>
            </sequenceFlow>

            <subProcess id="sub">
                <startEvent id="subStart" />
                <sequenceFlow id="sf1" sourceRef="subStart" targetRef="preTerminate" />
                <userTask id="preTerminate" name="Pre Terminate" />
                <sequenceFlow id="sf2" sourceRef="preTerminate" targetRef="termEnd" />
                <endEvent id="termEnd">
                    <terminateEventDefinition />
                </endEvent>
            </subProcess>
            <sequenceFlow id="f4" sourceRef="sub" targetRef="join" />

            <userTask id="taskB" name="Branch B" />
            <sequenceFlow id="f5" sourceRef="taskB" targetRef="join" />

            <inclusiveGateway id="join" />
            <sequenceFlow id="f6" sourceRef="join" targetRef="afterJoin" />
            <userTask id="afterJoin" name="After Join" />
            <sequenceFlow id="f7" sourceRef="afterJoin" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#;

    repository_service
        .deploy(
            repository_service
                .create_deployment()
                .name("p58-terminate-join".to_string())
                .add_string("p58TerminateJoin.bpmn20.xml".to_string(), xml.to_string()),
        )
        .unwrap();

    let def_id = repository_service.get_process_definition_ids().unwrap()[0].clone();
    let pi = runtime_service
        .start_process_instance(
            runtime_service
                .create_process_instance_builder()
                .process_definition_id(def_id)
                .variable("takeSub".to_string(), serde_json::Value::Bool(true))
                .variable("takeB".to_string(), serde_json::Value::Bool(true)),
        )
        .unwrap();

    let tasks = task_service
        .get_tasks_by_process_instance_id(pi.id.clone())
        .unwrap();
    let mut keys: Vec<_> = tasks
        .iter()
        .map(|t| t.task_definition_key.clone())
        .collect();
    keys.sort();
    assert_eq!(keys, vec!["preTerminate", "taskB"]);

    // Branch B completes FIRST and parks at the join (subprocess branch is
    // still reachable via f4).
    let task_b = tasks
        .iter()
        .find(|t| t.task_definition_key == "taskB")
        .cloned()
        .expect("branch B task");
    task_service.complete_task_by_id(task_b.id.clone()).unwrap();

    let tasks_parked = task_service
        .get_tasks_by_process_instance_id(pi.id.clone())
        .unwrap();
    assert_eq!(
        tasks_parked.len(),
        1,
        "join must wait while subprocess branch is reachable"
    );
    assert_eq!(tasks_parked[0].task_definition_key, "preTerminate");

    // Terminate end destroys the subprocess branch. The parked join token
    // must be re-evaluated at the end of THIS command.
    task_service
        .complete_task_by_id(tasks_parked[0].id.clone())
        .unwrap();

    let after = task_service
        .get_tasks_by_process_instance_id(pi.id.clone())
        .unwrap();
    assert_eq!(
        after.len(),
        1,
        "parked inclusive join must release in the terminate command; got {:?}",
        after
            .iter()
            .map(|t| t.task_definition_key.clone())
            .collect::<Vec<_>>()
    );
    assert_eq!(after[0].task_definition_key, "afterJoin");

    task_service.complete_task_by_id(after[0].id.clone()).unwrap();

    let mut session = runtime_store.create_session().unwrap();
    let stored_pi = runtime_store
        .find_process_instance(&pi.id, &mut session)
        .expect("process instance row");
    assert!(stored_pi.is_ended);
}

/// Fixpoint / cascade: activating one parked join plans operations whose
/// tokens flow into a SECOND join further downstream, which must also merge
/// within the same command. Mirrors Java's agenda re-drain after
/// `planExecuteInactiveBehaviorsOperation` (CommandInvoker.java:85-87) —
/// activation is looped, not a one-shot scan.
#[test]
fn inactive_join_reevaluation_cascades_through_downstream_join_in_same_command() {
    let process_engine = ProcessEngine::new("p58-cascade-joins".to_string());
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();
    let task_service = process_engine.get_task_service();
    let runtime_store = process_engine.get_runtime_store();

    // split fans out to A/B (→ join1) and C (→ join2); join1 feeds join2.
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="p58CascadeJoin" name="P58 Cascade Join" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="f1" sourceRef="start" targetRef="split" />

            <inclusiveGateway id="split" />
            <sequenceFlow id="f2" sourceRef="split" targetRef="taskA" />
            <sequenceFlow id="f3" sourceRef="split" targetRef="taskB" />
            <sequenceFlow id="f4" sourceRef="split" targetRef="taskC" />

            <userTask id="taskA" name="Branch A" />
            <sequenceFlow id="f5" sourceRef="taskA" targetRef="join1" />

            <userTask id="taskB" name="Branch B" />
            <boundaryEvent id="cancelB" attachedToRef="taskB" cancelActivity="true">
                <messageEventDefinition messageRef="cancelMsg" />
            </boundaryEvent>
            <sequenceFlow id="f_cancel" sourceRef="cancelB" targetRef="cancelEnd" />
            <endEvent id="cancelEnd" />
            <sequenceFlow id="f6" sourceRef="taskB" targetRef="join1" />

            <inclusiveGateway id="join1" />
            <sequenceFlow id="f7" sourceRef="join1" targetRef="join2" />

            <userTask id="taskC" name="Branch C" />
            <sequenceFlow id="f8" sourceRef="taskC" targetRef="join2" />

            <inclusiveGateway id="join2" />
            <sequenceFlow id="f9" sourceRef="join2" targetRef="afterJoin" />
            <userTask id="afterJoin" name="After Join" />
            <sequenceFlow id="f10" sourceRef="afterJoin" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#;

    repository_service
        .deploy(
            repository_service
                .create_deployment()
                .name("p58-cascade-join".to_string())
                .add_string("p58CascadeJoin.bpmn20.xml".to_string(), xml.to_string()),
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

    let tasks = task_service
        .get_tasks_by_process_instance_id(pi.id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 3, "all three branches should start");

    // Park a token at join1 (A) and a token at join2 (C); B stays active.
    let task_a = tasks
        .iter()
        .find(|t| t.task_definition_key == "taskA")
        .cloned()
        .expect("branch A task");
    task_service.complete_task_by_id(task_a.id.clone()).unwrap();

    let task_c = task_service
        .get_tasks_by_process_instance_id(pi.id.clone())
        .unwrap()
        .iter()
        .find(|t| t.task_definition_key == "taskC")
        .cloned()
        .expect("branch C task");
    task_service.complete_task_by_id(task_c.id.clone()).unwrap();

    let tasks_parked = task_service
        .get_tasks_by_process_instance_id(pi.id.clone())
        .unwrap();
    assert_eq!(tasks_parked.len(), 1, "join1 and join2 must both wait on B");
    assert_eq!(tasks_parked[0].task_definition_key, "taskB");

    // Destroy branch B. In one command: join1 re-evaluates and releases, its
    // token flows join1 → join2, and join2 merges with parked C.
    runtime_service
        .trigger_boundary_event("cancelB".to_string(), pi.id.clone())
        .unwrap();

    let after = task_service
        .get_tasks_by_process_instance_id(pi.id.clone())
        .unwrap();
    assert_eq!(
        after.len(),
        1,
        "cascade must resolve both joins in the boundary command; got {:?}",
        after
            .iter()
            .map(|t| t.task_definition_key.clone())
            .collect::<Vec<_>>()
    );
    assert_eq!(after[0].task_definition_key, "afterJoin");

    task_service.complete_task_by_id(after[0].id.clone()).unwrap();

    let mut session = runtime_store.create_session().unwrap();
    let stored_pi = runtime_store
        .find_process_instance(&pi.id, &mut session)
        .expect("process instance row");
    assert!(stored_pi.is_ended);
}
