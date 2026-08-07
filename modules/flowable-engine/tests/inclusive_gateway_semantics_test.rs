use flowable_engine::engine::process_engine::ProcessEngine;

fn deploy_inclusive_gateway_process(
    repository_service: &flowable_engine::engine::repository_service::RepositoryService,
) -> String {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="inclusiveGatewayProcess" name="Inclusive Gateway Process">
            <startEvent id="startEvent1" />
            <sequenceFlow id="flow_start_split" sourceRef="startEvent1" targetRef="inclusiveGatewaySplit" />

            <inclusiveGateway id="inclusiveGatewaySplit" default="flow_default" />
            <sequenceFlow id="flow_to_task1" sourceRef="inclusiveGatewaySplit" targetRef="userTask1">
                <conditionExpression><![CDATA[${approved == true}]]></conditionExpression>
            </sequenceFlow>
            <sequenceFlow id="flow_to_task2" sourceRef="inclusiveGatewaySplit" targetRef="userTask2">
                <conditionExpression><![CDATA[${secondary == true}]]></conditionExpression>
            </sequenceFlow>
            <sequenceFlow id="flow_default" sourceRef="inclusiveGatewaySplit" targetRef="userTaskDefault" />

            <userTask id="userTask1" name="First Branch" />
            <sequenceFlow id="flow_task1_join" sourceRef="userTask1" targetRef="inclusiveGatewayJoin" />

            <userTask id="userTask2" name="Second Branch" />
            <sequenceFlow id="flow_task2_join" sourceRef="userTask2" targetRef="inclusiveGatewayJoin" />

            <userTask id="userTaskDefault" name="Default Branch" />
            <sequenceFlow id="flow_default_join" sourceRef="userTaskDefault" targetRef="inclusiveGatewayJoin" />

            <inclusiveGateway id="inclusiveGatewayJoin" />
            <sequenceFlow id="flow_join_end" sourceRef="inclusiveGatewayJoin" targetRef="endEvent1" />

            <endEvent id="endEvent1" />
        </process>
    </definitions>"#;

    let deployment = repository_service
        .create_deployment()
        .name("Inclusive Gateway Deployment".to_string())
        .add_string(
            "inclusiveGatewayProcess.bpmn20.xml".to_string(),
            xml.to_string(),
        );

    repository_service.deploy(deployment).unwrap();
    repository_service.get_process_definition_ids().unwrap()[0].clone()
}

fn start_process_instance(
    runtime_service: &flowable_engine::engine::runtime_service::RuntimeService,
    process_definition_id: String,
    approved: bool,
    secondary: bool,
) -> flowable_engine::runtime::process_instance::ProcessInstance {
    runtime_service
        .start_process_instance(
            runtime_service
                .create_process_instance_builder()
                .process_definition_id(process_definition_id)
                .name("Inclusive Gateway Instance".to_string())
                .variable("approved".to_string(), serde_json::Value::Bool(approved))
                .variable("secondary".to_string(), serde_json::Value::Bool(secondary)),
        )
        .unwrap()
}

#[test]
fn inclusive_gateway_takes_all_matching_outgoing_flows_and_joins_after_both_complete() {
    let process_engine = ProcessEngine::new("default".to_string());
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();
    let task_service = process_engine.get_task_service();
    let runtime_store = process_engine.get_runtime_store();

    let process_definition_id = deploy_inclusive_gateway_process(&repository_service);
    let process_instance =
        start_process_instance(&runtime_service, process_definition_id, true, true);

    let initial_tasks = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(initial_tasks.len(), 2);

    let first_task = initial_tasks
        .iter()
        .find(|task| task.task_definition_key == "userTask1")
        .cloned()
        .expect("expected first inclusive branch task");
    let second_task = initial_tasks
        .iter()
        .find(|task| task.task_definition_key == "userTask2")
        .cloned()
        .expect("expected second inclusive branch task");

    task_service
        .complete_task_by_id(first_task.id.clone())
        .unwrap();

    let tasks_after_first_complete = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(tasks_after_first_complete.len(), 1);
    assert_eq!(
        tasks_after_first_complete[0].task_definition_key,
        "userTask2"
    );

    let mut session = runtime_store.create_session().unwrap();
    let snapshot_after_first = runtime_store.snapshot_executions(&mut session);
    assert!(
        !snapshot_after_first
            .values()
            .any(|execution| execution.activity_id.as_deref() == Some("endEvent1")),
        "inclusive join should not reach end event after only one branch completes"
    );
    drop(session);

    task_service
        .complete_task_by_id(second_task.id.clone())
        .unwrap();

    let remaining_tasks = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert!(remaining_tasks.is_empty());

    let mut session = runtime_store.create_session().unwrap();
    let stored_pi = runtime_store
        .find_process_instance(&process_instance.id, &mut session)
        .expect("Process instance should be in runtime store");
    assert!(stored_pi.is_ended);
}

#[test]
fn inclusive_gateway_uses_default_flow_when_no_condition_matches_and_joins_single_branch() {
    let process_engine = ProcessEngine::new("default".to_string());
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();
    let task_service = process_engine.get_task_service();
    let runtime_store = process_engine.get_runtime_store();

    let process_definition_id = deploy_inclusive_gateway_process(&repository_service);
    let process_instance =
        start_process_instance(&runtime_service, process_definition_id, false, false);

    let tasks = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].task_definition_key, "userTaskDefault");

    task_service
        .complete_task_by_id(tasks[0].id.clone())
        .unwrap();

    let remaining_tasks = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert!(remaining_tasks.is_empty());

    let mut session = runtime_store.create_session().unwrap();
    let stored_pi = runtime_store
        .find_process_instance(&process_instance.id, &mut session)
        .expect("Process instance should be in runtime store");
    assert!(stored_pi.is_ended);
}

/// P50 / G1-1: when one inclusive branch is destroyed by an interrupting
/// boundary (without arriving at the join), the join must still activate once
/// the remaining branch arrives.
///
/// Java: InclusiveGatewayActivityBehavior + ExecutionGraphUtil.isReachable —
/// token-count join would wait forever for the destroyed branch.
#[test]
fn inclusive_join_activates_after_interrupting_boundary_destroys_sibling_branch() {
    let process_engine = ProcessEngine::new("inclusive-join-boundary-destroy".to_string());
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();
    let task_service = process_engine.get_task_service();
    let runtime_store = process_engine.get_runtime_store();

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="inclusiveBoundaryJoin" name="Inclusive Boundary Join" isExecutable="true">
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
            <boundaryEvent id="cancelA" attachedToRef="taskA" cancelActivity="true">
                <messageEventDefinition messageRef="cancelMsg" />
            </boundaryEvent>
            <sequenceFlow id="f_cancel" sourceRef="cancelA" targetRef="cancelEnd" />
            <endEvent id="cancelEnd" />
            <sequenceFlow id="f4" sourceRef="taskA" targetRef="join" />

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
                .name("inclusive-boundary-join".to_string())
                .add_string(
                    "inclusiveBoundaryJoin.bpmn20.xml".to_string(),
                    xml.to_string(),
                ),
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

    // Destroy branch A via interrupting boundary before it reaches the join.
    runtime_service
        .trigger_boundary_event("cancelA".to_string(), pi.id.clone())
        .unwrap();

    let tasks_after_boundary = task_service
        .get_tasks_by_process_instance_id(pi.id.clone())
        .unwrap();
    assert_eq!(
        tasks_after_boundary.len(),
        1,
        "only branch B should remain after interrupting boundary"
    );
    assert_eq!(tasks_after_boundary[0].task_definition_key, "taskB");

    // Complete the surviving branch — join must activate (isReachable finds
    // no other path that can still arrive).
    task_service
        .complete_task_by_id(tasks_after_boundary[0].id.clone())
        .unwrap();

    let after_join_tasks = task_service
        .get_tasks_by_process_instance_id(pi.id.clone())
        .unwrap();
    assert_eq!(
        after_join_tasks.len(),
        1,
        "inclusive join must pass after sibling branch was destroyed by boundary; got {:?}",
        after_join_tasks
            .iter()
            .map(|t| t.task_definition_key.clone())
            .collect::<Vec<_>>()
    );
    assert_eq!(after_join_tasks[0].task_definition_key, "afterJoin");

    task_service
        .complete_task_by_id(after_join_tasks[0].id.clone())
        .unwrap();

    let mut session = runtime_store.create_session().unwrap();
    let stored_pi = runtime_store
        .find_process_instance(&pi.id, &mut session)
        .expect("process instance row");
    assert!(
        stored_pi.is_ended,
        "process should end after afterJoin completes"
    );
}

/// P50 / G1-1: one inclusive branch ends on a dead path (exclusive → end,
/// never reaches join). Completing the surviving branch must activate the join.
///
/// Mirrors Java InclusiveGatewayTest#testMergeWithEndedExecution ordering
/// where the dead path completes first (no ExecuteInactiveBehaviors needed).
#[test]
fn inclusive_join_activates_when_sibling_branch_ends_without_reaching_join() {
    let process_engine = ProcessEngine::new("inclusive-join-dead-path".to_string());
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();
    let task_service = process_engine.get_task_service();
    let runtime_store = process_engine.get_runtime_store();

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="inclusiveDeadPathJoin" name="Inclusive Dead Path Join" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="f1" sourceRef="start" targetRef="split" />

            <inclusiveGateway id="split" />
            <sequenceFlow id="f2" sourceRef="split" targetRef="taskStay">
                <conditionExpression><![CDATA[${takeStay == true}]]></conditionExpression>
            </sequenceFlow>
            <sequenceFlow id="f3" sourceRef="split" targetRef="taskExit">
                <conditionExpression><![CDATA[${takeExit == true}]]></conditionExpression>
            </sequenceFlow>

            <userTask id="taskStay" name="Surviving Branch" />
            <sequenceFlow id="f4" sourceRef="taskStay" targetRef="join" />

            <userTask id="taskExit" name="Dead Path Branch" />
            <sequenceFlow id="f5" sourceRef="taskExit" targetRef="exclusive" />
            <exclusiveGateway id="exclusive" />
            <sequenceFlow id="f_to_join" sourceRef="exclusive" targetRef="join">
                <conditionExpression><![CDATA[${goToJoin == true}]]></conditionExpression>
            </sequenceFlow>
            <sequenceFlow id="f_to_end" sourceRef="exclusive" targetRef="deadEnd">
                <conditionExpression><![CDATA[${goToJoin == false}]]></conditionExpression>
            </sequenceFlow>
            <endEvent id="deadEnd" />

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
                .name("inclusive-dead-path-join".to_string())
                .add_string(
                    "inclusiveDeadPathJoin.bpmn20.xml".to_string(),
                    xml.to_string(),
                ),
        )
        .unwrap();

    let def_id = repository_service.get_process_definition_ids().unwrap()[0].clone();
    let pi = runtime_service
        .start_process_instance(
            runtime_service
                .create_process_instance_builder()
                .process_definition_id(def_id)
                .variable("takeStay".to_string(), serde_json::Value::Bool(true))
                .variable("takeExit".to_string(), serde_json::Value::Bool(true))
                .variable("goToJoin".to_string(), serde_json::Value::Bool(false)),
        )
        .unwrap();

    let tasks = task_service
        .get_tasks_by_process_instance_id(pi.id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 2);

    let exit_task = tasks
        .iter()
        .find(|t| t.task_definition_key == "taskExit")
        .cloned()
        .expect("dead path task");
    let stay_task = tasks
        .iter()
        .find(|t| t.task_definition_key == "taskStay")
        .cloned()
        .expect("surviving task");

    // Dead path completes first and ends without reaching the join.
    task_service
        .complete_task_by_id(exit_task.id.clone())
        .unwrap();

    // Surviving branch arrives — join must activate (no other reachable path).
    task_service
        .complete_task_by_id(stay_task.id.clone())
        .unwrap();

    let after = task_service
        .get_tasks_by_process_instance_id(pi.id.clone())
        .unwrap();
    assert_eq!(
        after.len(),
        1,
        "inclusive join must pass after sibling dead-path end; got {:?}",
        after
            .iter()
            .map(|t| t.task_definition_key.clone())
            .collect::<Vec<_>>()
    );
    assert_eq!(after[0].task_definition_key, "afterJoin");

    task_service
        .complete_task_by_id(after[0].id.clone())
        .unwrap();

    let mut session = runtime_store.create_session().unwrap();
    let stored_pi = runtime_store
        .find_process_instance(&pi.id, &mut session)
        .expect("process instance row");
    assert!(stored_pi.is_ended);
}

/// P50 / G1-1: one inclusive branch is terminated by a terminate end event
/// inside an embedded subprocess (default terminate kills only the subprocess
/// scope). The outer inclusive join must still activate for the surviving
/// branch.
#[test]
fn inclusive_join_activates_after_terminate_end_destroys_subprocess_branch() {
    let process_engine = ProcessEngine::new("inclusive-join-terminate-sub".to_string());
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();
    let task_service = process_engine.get_task_service();
    let runtime_store = process_engine.get_runtime_store();

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="inclusiveTerminateJoin" name="Inclusive Terminate Join" isExecutable="true">
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
                .name("inclusive-terminate-join".to_string())
                .add_string(
                    "inclusiveTerminateJoin.bpmn20.xml".to_string(),
                    xml.to_string(),
                ),
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

    // Terminate destroys the subprocess branch (default terminate = scope).
    let pre_term = tasks
        .iter()
        .find(|t| t.task_definition_key == "preTerminate")
        .unwrap();
    task_service
        .complete_task_by_id(pre_term.id.clone())
        .unwrap();

    let tasks_after_term = task_service
        .get_tasks_by_process_instance_id(pi.id.clone())
        .unwrap();
    assert_eq!(
        tasks_after_term.len(),
        1,
        "only branch B should remain after subprocess terminate; got {:?}",
        tasks_after_term
            .iter()
            .map(|t| t.task_definition_key.clone())
            .collect::<Vec<_>>()
    );
    assert_eq!(tasks_after_term[0].task_definition_key, "taskB");

    task_service
        .complete_task_by_id(tasks_after_term[0].id.clone())
        .unwrap();

    let after = task_service
        .get_tasks_by_process_instance_id(pi.id.clone())
        .unwrap();
    assert_eq!(
        after.len(),
        1,
        "inclusive join must pass after terminate destroyed the subprocess branch; got {:?}",
        after
            .iter()
            .map(|t| t.task_definition_key.clone())
            .collect::<Vec<_>>()
    );
    assert_eq!(after[0].task_definition_key, "afterJoin");

    task_service
        .complete_task_by_id(after[0].id.clone())
        .unwrap();

    let mut session = runtime_store.create_session().unwrap();
    let stored_pi = runtime_store
        .find_process_instance(&pi.id, &mut session)
        .expect("process instance row");
    assert!(stored_pi.is_ended);
}
