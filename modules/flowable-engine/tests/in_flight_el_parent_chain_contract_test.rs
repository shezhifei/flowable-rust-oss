//! Contract tests for in-flight EL evaluation walking the parent VariableScope
//! chain (P4-7).
//!
//! Java reference: sequence-flow conditions are evaluated on a `DelegateExecution`
//! whose `getVariable` walks `VariableScopeImpl` parent chain. Rust historically
//! evaluated conditions via `Execution::process_variable`, which only reads the
//! current row's three maps. Forked concurrent children therefore only saw process
//! variables when the fork cloned a snapshot of the parent's maps — and a variable
//! written on the process-instance scope *after* the fork was invisible to branch
//! gateway conditions.
//!
//! Topology used by the gap tests (top-level parallel, so branch rows are real
//! concurrent children under the preserved PI scope row):
//!
//! ```text
//! start → fork ──► holdTask
//!              └──► stageTask → decision ──► approvedTask
//!                                        └──► rejectedTask
//! ```

use flowable_engine::engine::process_engine::ProcessEngine;
use serde_json::json;

const FORK_EXCLUSIVE_GATEWAY_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
  <process id="forkExclusiveEl" name="Fork Exclusive EL" isExecutable="true">
    <startEvent id="start" />
    <sequenceFlow id="toFork" sourceRef="start" targetRef="fork" />
    <parallelGateway id="fork" />

    <sequenceFlow id="toHold" sourceRef="fork" targetRef="holdTask" />
    <userTask id="holdTask" name="Hold" />

    <sequenceFlow id="toStage" sourceRef="fork" targetRef="stageTask" />
    <userTask id="stageTask" name="Stage" />
    <sequenceFlow id="toDecision" sourceRef="stageTask" targetRef="decision" />
    <exclusiveGateway id="decision" />
    <sequenceFlow id="toApproved" sourceRef="decision" targetRef="approvedTask">
      <conditionExpression><![CDATA[${approved}]]></conditionExpression>
    </sequenceFlow>
    <sequenceFlow id="toRejected" sourceRef="decision" targetRef="rejectedTask">
      <conditionExpression><![CDATA[${!approved}]]></conditionExpression>
    </sequenceFlow>
    <userTask id="approvedTask" name="Approved" />
    <userTask id="rejectedTask" name="Rejected" />
  </process>
</definitions>"#;

const PLAIN_PARALLEL_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
  <process id="plainParallel" name="Plain Parallel" isExecutable="true">
    <startEvent id="start" />
    <sequenceFlow id="toFork" sourceRef="start" targetRef="fork" />
    <parallelGateway id="fork" />
    <sequenceFlow id="toA" sourceRef="fork" targetRef="taskA" />
    <sequenceFlow id="toB" sourceRef="fork" targetRef="taskB" />
    <userTask id="taskA" name="Task A" />
    <userTask id="taskB" name="Task B" />
  </process>
</definitions>"#;

fn deploy_and_start(
    engine: &ProcessEngine,
    xml: &str,
    resource: &str,
    variables: Vec<(String, serde_json::Value)>,
) -> String {
    let repository_service = engine.get_repository_service();
    let builder = repository_service
        .create_deployment()
        .name(resource.to_string())
        .add_string(format!("{resource}.bpmn20.xml"), xml.to_string());
    repository_service.deploy(builder).unwrap();
    let process_definition_id = repository_service.get_process_definition_ids().unwrap()[0].clone();
    let mut start = engine
        .get_runtime_service()
        .create_process_instance_builder()
        .process_definition_id(process_definition_id);
    for (name, value) in variables {
        start = start.variable(name, value);
    }
    engine
        .get_runtime_service()
        .start_process_instance(start)
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
    let task = engine
        .get_task_service()
        .get_tasks_by_process_instance_id(process_instance_id.to_string())
        .unwrap()
        .into_iter()
        .find(|task| task.task_definition_key == key)
        .unwrap_or_else(|| panic!("expected task with key {key}"));
    engine
        .get_task_service()
        .complete_task_by_id(task.id)
        .unwrap();
}

/// Gap test (red before P4-7a): a process variable written on the PI scope
/// *after* the top-level fork must still drive a branch exclusive-gateway
/// condition. The fork snapshot does not contain that write, so evaluation
/// that only reads the child row maps evaluates both conditions to false and
/// the token is silently deleted.
#[test]
fn branch_exclusive_gateway_resolves_process_variable_written_after_fork() {
    let engine = ProcessEngine::new("p4-7a-post-fork-write".to_string());
    let process_instance_id = deploy_and_start(
        &engine,
        FORK_EXCLUSIVE_GATEWAY_XML,
        "fork_exclusive_post_fork",
        vec![],
    );

    assert_eq!(
        task_keys(&engine, &process_instance_id),
        vec!["holdTask".to_string(), "stageTask".to_string()]
    );

    engine
        .get_runtime_service()
        .set_variable(
            process_instance_id.clone(),
            "approved".to_string(),
            json!(true),
        )
        .unwrap();

    complete_task_by_key(&engine, &process_instance_id, "stageTask");

    let keys = task_keys(&engine, &process_instance_id);
    assert!(
        keys.contains(&"approvedTask".to_string()),
        "branch exclusive gateway must see process variable written after fork; got {keys:?}"
    );
    assert!(
        !keys.contains(&"rejectedTask".to_string()),
        "rejected branch must not be taken when approved=true; got {keys:?}"
    );
    assert!(
        keys.contains(&"holdTask".to_string()),
        "sibling parallel branch must remain; got {keys:?}"
    );
}

/// P4-6C probe turned into a contract: start variable `approved=true`, top-level
/// fork, then branch exclusive gateway `${approved}` / `${!approved}` must take
/// `approvedTask`. With fork snapshots still present this already works; after
/// P4-7b snapshot removal it continues to work only because evaluation walks
/// the parent chain.
#[test]
fn branch_exclusive_gateway_resolves_start_variable_after_fork() {
    let engine = ProcessEngine::new("p4-7a-start-var".to_string());
    let process_instance_id = deploy_and_start(
        &engine,
        FORK_EXCLUSIVE_GATEWAY_XML,
        "fork_exclusive_start_var",
        vec![("approved".to_string(), json!(true))],
    );

    assert_eq!(
        task_keys(&engine, &process_instance_id),
        vec!["holdTask".to_string(), "stageTask".to_string()]
    );

    complete_task_by_key(&engine, &process_instance_id, "stageTask");

    let keys = task_keys(&engine, &process_instance_id);
    assert!(
        keys.contains(&"approvedTask".to_string()),
        "start variable approved=true must route to approvedTask; got {keys:?}"
    );
    assert!(
        !keys.contains(&"rejectedTask".to_string()),
        "rejected branch must not be taken; got {keys:?}"
    );
}

/// Regression guard: a plain parallel fork without branch-internal EL keeps
/// both user tasks. Must stay green before and after the evaluation fix.
#[test]
fn plain_parallel_fork_without_branch_el_still_reaches_both_tasks() {
    let engine = ProcessEngine::new("p4-7a-plain-parallel".to_string());
    let process_instance_id =
        deploy_and_start(&engine, PLAIN_PARALLEL_XML, "plain_parallel_guard", vec![]);

    assert_eq!(
        task_keys(&engine, &process_instance_id),
        vec!["taskA".to_string(), "taskB".to_string()],
        "plain parallel topology must be unchanged by EL parent-chain work"
    );
}

const INCLUSIVE_GATEWAY_EXPECTED_TOKEN_COUNT_VARIABLE: &str =
    "__inclusive_gateway_expected_token_count";

const INCLUSIVE_SPLIT_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
  <process id="inclusiveSplitLock" name="Inclusive Split Lock" isExecutable="true">
    <startEvent id="start" />
    <sequenceFlow id="toSplit" sourceRef="start" targetRef="split" />
    <inclusiveGateway id="split" />
    <sequenceFlow id="toA" sourceRef="split" targetRef="taskA">
      <conditionExpression><![CDATA[${takeA == true}]]></conditionExpression>
    </sequenceFlow>
    <sequenceFlow id="toB" sourceRef="split" targetRef="taskB">
      <conditionExpression><![CDATA[${takeB == true}]]></conditionExpression>
    </sequenceFlow>
    <userTask id="taskA" name="A" />
    <userTask id="taskB" name="B" />
    <sequenceFlow id="aToJoin" sourceRef="taskA" targetRef="join" />
    <sequenceFlow id="bToJoin" sourceRef="taskB" targetRef="join" />
    <inclusiveGateway id="join" />
    <sequenceFlow id="toEnd" sourceRef="join" targetRef="end" />
    <endEvent id="end" />
  </process>
</definitions>"#;

/// P4-7b lock: after fork/split, concurrent children must not carry a cloned
/// process-variable snapshot. Inclusive children still own their join-count
/// bookkeeping variable.
#[test]
fn concurrent_children_start_without_variable_snapshot() {
    let engine = ProcessEngine::new("p4-7b-no-snapshot".to_string());
    let process_instance_id = deploy_and_start(
        &engine,
        PLAIN_PARALLEL_XML,
        "plain_parallel_no_snapshot",
        vec![("orderId".to_string(), json!("ORD-1"))],
    );

    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let children: Vec<_> = store
        .snapshot_executions(&mut session)
        .into_values()
        .filter(|execution| {
            execution.process_instance_id.as_deref() == Some(process_instance_id.as_str())
                && execution.parent_id.is_some()
                && !execution.is_ended
        })
        .collect();
    assert_eq!(
        children.len(),
        2,
        "expected two concurrent branch executions"
    );
    for child in &children {
        assert!(
            child.variables.is_empty(),
            "parallel child {} must not snapshot process variables; got {:?}",
            child.id,
            child.variables
        );
        assert!(child.local_variables.is_empty());
        assert!(child.transient_variables.is_empty());
    }

    // Start variable still lives on the PI scope row and resolves via parent chain.
    let root = store
        .find_execution(&process_instance_id, &mut session)
        .expect("PI scope row");
    assert_eq!(root.variables.get("orderId"), Some(&json!("ORD-1")));
}

/// Inclusive split no longer writes `__inclusive_gateway_expected_token_count`
/// onto each child — P50 replaced the token-count join with
/// `ExecutionGraphUtil::is_reachable` (Java `ExecutionGraphUtil.isReachable`),
/// and P54 dropped the dead write path. The child must therefore start with
/// an empty owned-variable map (consistent with parallel children) and the
/// join still completes via reachability analysis.
#[test]
fn inclusive_split_does_not_write_legacy_token_count_variable() {
    let engine = ProcessEngine::new("p4-7b-inclusive-count".to_string());
    let process_instance_id = deploy_and_start(
        &engine,
        INCLUSIVE_SPLIT_XML,
        "inclusive_split_count_lock",
        vec![
            ("takeA".to_string(), json!(true)),
            ("takeB".to_string(), json!(true)),
        ],
    );

    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let children: Vec<_> = store
        .snapshot_executions(&mut session)
        .into_values()
        .filter(|execution| {
            execution.process_instance_id.as_deref() == Some(process_instance_id.as_str())
                && execution.parent_id.is_some()
                && !execution.is_ended
        })
        .collect();
    assert_eq!(children.len(), 2);

    for child in &children {
        // Java parity: token-count variable is gone. Join now relies on
        // ExecutionGraphUtil.isReachable (P50), not a child-owned counter.
        assert!(
            child
                .variables
                .get(INCLUSIVE_GATEWAY_EXPECTED_TOKEN_COUNT_VARIABLE)
                .is_none(),
            "child {} must not own the removed inclusive expected-token-count variable; got {:?}",
            child.id,
            child.variables
        );
        // No process variables should have been snapshotted from the parent.
        assert_eq!(
            child.variables.len(),
            0,
            "inclusive child must start with no owned variables; got {:?}",
            child.variables
        );
        assert!(
            child.variables.get("takeA").is_none() && child.variables.get("takeB").is_none(),
            "condition variables must not be snapshotted onto the child"
        );
    }
}

/// Regression guard for inclusive join after snapshot cleanup + P54 dead-code
/// removal: both matching branches complete and the process ends (join now uses
/// `ExecutionGraphUtil::is_reachable` from P50, not the dropped child-owned
/// counter).
#[test]
fn inclusive_join_still_waits_for_expected_token_count_after_snapshot_cleanup() {
    let engine = ProcessEngine::new("p4-7b-inclusive-join".to_string());
    let process_instance_id = deploy_and_start(
        &engine,
        INCLUSIVE_SPLIT_XML,
        "inclusive_join_after_cleanup",
        vec![
            ("takeA".to_string(), json!(true)),
            ("takeB".to_string(), json!(true)),
        ],
    );
    let task_service = engine.get_task_service();

    let tasks = task_service
        .get_tasks_by_process_instance_id(process_instance_id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 2);

    task_service
        .complete_task_by_id(tasks[0].id.clone())
        .unwrap();

    // One branch still open — process must not have ended.
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let pi = store
        .find_process_instance(&process_instance_id, &mut session)
        .unwrap();
    assert!(
        !pi.is_ended,
        "inclusive join must wait for the second branch"
    );
    drop(session);

    let remaining = task_service
        .get_tasks_by_process_instance_id(process_instance_id.clone())
        .unwrap();
    assert_eq!(remaining.len(), 1);
    task_service
        .complete_task_by_id(remaining[0].id.clone())
        .unwrap();

    let mut session = store.create_session().unwrap();
    let pi = store
        .find_process_instance(&process_instance_id, &mut session)
        .unwrap();
    assert!(
        pi.is_ended,
        "inclusive join must complete after both branches"
    );
}
