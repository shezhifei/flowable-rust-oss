//! Contract tests for Java multi-instance variable semantics parity:
//!   - `nrOfInstances` / `nrOfActiveInstances` / `nrOfCompletedInstances` are written
//!     with `setVariableLocal` on the multi-instance root execution
//!     (`ParallelMultiInstanceBehavior#createInstances` /
//!     `SequentialMultiInstanceBehavior#createInstances` via
//!     `MultiInstanceActivityBehavior#setLoopVariable` → `setVariableLocal`);
//!   - `loopCounter` / element variable / element index variable are execution-local
//!     on the child instance execution
//!     (`ContinueMultiInstanceOperation#run` → `setVariableLocal(elementIndexVariable, …)`);
//!   - MI child executions are created empty (`createChildExecution`): they do NOT
//!     snapshot the parent's variables, so `getVariable(childId, "nrOf…")` resolves the
//!     LIVE value from the MI root through the parent scope chain;
//!   - collection resolution (`MultiInstanceActivityBehavior#resolveAndValidateCollection`
//!     → `execution.getVariable`) and `completionCondition` evaluation
//!     (`expressionManager.createExpression(…).getValue(execution)`) resolve variables
//!     through the `VariableScope` parent chain, not just the MI root row.

use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::runtime::process_instance::ProcessInstance;
use serde_json::json;
use std::collections::HashMap;

const PARALLEL_MI_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
    <process id="miParityParallel" isExecutable="true">
        <startEvent id="start" />
        <sequenceFlow id="f1" sourceRef="start" targetRef="miTask" />
        <userTask id="miTask" name="MI Task">
            <multiInstanceLoopCharacteristics isSequential="false">
                <loopCardinality>3</loopCardinality>
            </multiInstanceLoopCharacteristics>
        </userTask>
        <sequenceFlow id="f2" sourceRef="miTask" targetRef="end" />
        <endEvent id="end" />
    </process>
</definitions>"#;

const SEQUENTIAL_MI_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
    <process id="miParitySequential" isExecutable="true">
        <startEvent id="start" />
        <sequenceFlow id="f1" sourceRef="start" targetRef="miTask" />
        <userTask id="miTask" name="MI Task">
            <multiInstanceLoopCharacteristics isSequential="true">
                <loopCardinality>3</loopCardinality>
            </multiInstanceLoopCharacteristics>
        </userTask>
        <sequenceFlow id="f2" sourceRef="miTask" targetRef="end" />
        <endEvent id="end" />
    </process>
</definitions>"#;

const PARALLEL_MI_COLLECTION_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
    <process id="miParityCollection" isExecutable="true">
        <startEvent id="start" />
        <sequenceFlow id="f1" sourceRef="start" targetRef="miTask" />
        <userTask id="miTask" name="MI Task">
            <multiInstanceLoopCharacteristics isSequential="false"
                                              flowable:collection="approvers"
                                              flowable:elementVariable="approver"
                                              flowable:elementIndexVariable="approverIndex" />
        </userTask>
        <sequenceFlow id="f2" sourceRef="miTask" targetRef="end" />
        <endEvent id="end" />
    </process>
</definitions>"#;

/// Parallel gateway fork in front of the MI activity: the MI root execution is a
/// fork branch child with empty variable maps (P4-7b), while process variables
/// (`approvers`, `threshold`) live on the process-instance scope row upstream.
const FORK_MI_COLLECTION_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
    <process id="miParityForkCollection" isExecutable="true">
        <startEvent id="start" />
        <sequenceFlow id="f1" sourceRef="start" targetRef="fork" />
        <parallelGateway id="fork" />
        <sequenceFlow id="f2" sourceRef="fork" targetRef="miTask" />
        <userTask id="miTask" name="MI Task">
            <multiInstanceLoopCharacteristics isSequential="false"
                                              flowable:collection="approvers"
                                              flowable:elementVariable="approver" />
        </userTask>
        <sequenceFlow id="f3" sourceRef="miTask" targetRef="join" />
        <sequenceFlow id="f4" sourceRef="fork" targetRef="taskB" />
        <userTask id="taskB" name="Task B" />
        <sequenceFlow id="f5" sourceRef="taskB" targetRef="join" />
        <parallelGateway id="join" />
        <sequenceFlow id="f6" sourceRef="join" targetRef="afterJoin" />
        <userTask id="afterJoin" name="After Join" />
        <sequenceFlow id="f7" sourceRef="afterJoin" targetRef="end" />
        <endEvent id="end" />
    </process>
</definitions>"#;

const FORK_MI_COMPLETION_CONDITION_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
    <process id="miParityForkCompletion" isExecutable="true">
        <startEvent id="start" />
        <sequenceFlow id="f1" sourceRef="start" targetRef="fork" />
        <parallelGateway id="fork" />
        <sequenceFlow id="f2" sourceRef="fork" targetRef="miTask" />
        <userTask id="miTask" name="MI Task">
            <multiInstanceLoopCharacteristics isSequential="false">
                <loopCardinality>3</loopCardinality>
                <completionCondition>${nrOfCompletedInstances >= threshold}</completionCondition>
            </multiInstanceLoopCharacteristics>
        </userTask>
        <sequenceFlow id="f3" sourceRef="miTask" targetRef="join" />
        <sequenceFlow id="f4" sourceRef="fork" targetRef="taskB" />
        <userTask id="taskB" name="Task B" />
        <sequenceFlow id="f5" sourceRef="taskB" targetRef="join" />
        <parallelGateway id="join" />
        <sequenceFlow id="f6" sourceRef="join" targetRef="afterJoin" />
        <userTask id="afterJoin" name="After Join" />
        <sequenceFlow id="f7" sourceRef="afterJoin" targetRef="end" />
        <endEvent id="end" />
    </process>
</definitions>"#;

fn deploy(engine: &ProcessEngine, resource: &str, xml: &str) -> String {
    let repo = engine.get_repository_service();
    repo.deploy(
        repo.create_deployment()
            .add_string(resource.to_string(), xml.to_string()),
    )
    .unwrap();
    engine
        .get_repository_service()
        .get_process_definition_ids()
        .unwrap()[0]
        .clone()
}

fn start(
    engine: &ProcessEngine,
    definition_id: String,
    variables: HashMap<String, serde_json::Value>,
) -> ProcessInstance {
    let runtime = engine.get_runtime_service();
    let mut builder = runtime
        .create_process_instance_builder()
        .process_definition_id(definition_id);
    for (name, value) in variables {
        builder = builder.variable(name, value);
    }
    runtime.start_process_instance(builder).unwrap()
}

fn mi_root_id(engine: &ProcessEngine, child_execution_id: &str) -> String {
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    store
        .find_execution(child_execution_id, &mut session)
        .expect("child execution")
        .parent_id
        .expect("MI child must have a parent (the MI root)")
}

/// Difference #1 (frozen snapshot): a parallel MI child must read the LIVE
/// `nrOfCompletedInstances` / `nrOfActiveInstances` from the MI root through the
/// parent scope chain, not a snapshot cloned at child-creation time.
/// Java: children are `createChildExecution` (empty); `getVariable` walks the chain.
#[test]
fn parallel_mi_child_reads_live_nr_of_variables_from_mi_root() {
    let engine = ProcessEngine::new("mi-parity-live-nr-of".to_string());
    let definition_id = deploy(&engine, "mi_parallel.bpmn20.xml", PARALLEL_MI_XML);
    let process_instance = start(&engine, definition_id, HashMap::new());
    let runtime = engine.get_runtime_service();
    let task_service = engine.get_task_service();

    let tasks = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 3);

    task_service
        .complete_task_by_id(tasks[0].id.clone())
        .unwrap();

    let remaining_child = tasks[1].execution_id.clone();
    assert_eq!(
        runtime
            .get_variable(
                remaining_child.clone(),
                "nrOfCompletedInstances".to_string()
            )
            .unwrap(),
        Some(json!(1)),
        "child must resolve the live nrOfCompletedInstances from the MI root"
    );
    assert_eq!(
        runtime
            .get_variable(remaining_child.clone(), "nrOfActiveInstances".to_string())
            .unwrap(),
        Some(json!(2)),
        "child must resolve the live nrOfActiveInstances from the MI root"
    );
    // The child holds no own copy: the narrow local view stays empty.
    assert_eq!(
        runtime
            .get_variable_local(remaining_child, "nrOfCompletedInstances".to_string())
            .unwrap(),
        None,
        "nrOf* variables are local to the MI root, never copied onto children"
    );
}

/// Regression guard (green before and after): the merged read from a child sees
/// `nrOfInstances`, and the MI root sees it too.
#[test]
fn nr_of_instances_visible_from_child_and_root_via_merged_read() {
    let engine = ProcessEngine::new("mi-parity-guard-merged".to_string());
    let definition_id = deploy(&engine, "mi_parallel.bpmn20.xml", PARALLEL_MI_XML);
    let process_instance = start(&engine, definition_id, HashMap::new());
    let runtime = engine.get_runtime_service();
    let task_service = engine.get_task_service();

    let tasks = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 3);

    let child = tasks[0].execution_id.clone();
    assert_eq!(
        runtime
            .get_variable(child.clone(), "nrOfInstances".to_string())
            .unwrap(),
        Some(json!(3))
    );
    let root = mi_root_id(&engine, &child);
    assert_eq!(
        runtime
            .get_variables(root)
            .unwrap()
            .get("nrOfInstances")
            .cloned(),
        Some(json!(3))
    );
}

/// Difference #2 (loop variables local): `loopCounter`, the element variable and
/// the element index variable are execution-LOCAL on the child instance
/// (`ContinueMultiInstanceOperation` → `setVariableLocal`), so the narrow
/// `getVariableLocal` view must expose them.
#[test]
fn mi_instance_variables_are_execution_local_on_children() {
    let engine = ProcessEngine::new("mi-parity-local-instance-vars".to_string());
    let definition_id = deploy(
        &engine,
        "mi_collection.bpmn20.xml",
        PARALLEL_MI_COLLECTION_XML,
    );
    let mut variables = HashMap::new();
    variables.insert("approvers".to_string(), json!(["amy", "ben", "cy"]));
    let process_instance = start(&engine, definition_id, variables);
    let runtime = engine.get_runtime_service();
    let task_service = engine.get_task_service();

    let tasks = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 3);

    let child = tasks[0].execution_id.clone();
    for name in ["loopCounter", "approver", "approverIndex"] {
        assert!(
            runtime
                .get_variable_local(child.clone(), name.to_string())
                .unwrap()
                .is_some(),
            "{name} must be execution-local on the MI child instance"
        );
    }
    // Guard (green before and after): the merged read resolves the element variable.
    assert!(
        runtime
            .get_variable(child, "approver".to_string())
            .unwrap()
            .is_some()
    );
}

/// Difference #2 on the MI root: Java writes `nrOf*` with `setVariableLocal` on
/// the MI root execution, so they belong to its local scope.
#[test]
fn nr_of_variables_are_local_on_mi_root() {
    let engine = ProcessEngine::new("mi-parity-nr-of-local".to_string());
    let definition_id = deploy(&engine, "mi_parallel.bpmn20.xml", PARALLEL_MI_XML);
    let process_instance = start(&engine, definition_id, HashMap::new());
    let runtime = engine.get_runtime_service();
    let task_service = engine.get_task_service();

    let tasks = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    let root = mi_root_id(&engine, &tasks[0].execution_id);

    let locals = runtime.get_variables_local(root).unwrap();
    assert_eq!(locals.get("nrOfInstances"), Some(&json!(3)));
    assert_eq!(locals.get("nrOfActiveInstances"), Some(&json!(3)));
    assert_eq!(locals.get("nrOfCompletedInstances"), Some(&json!(0)));
}

/// Difference #2, sequential side: `loopCounter` lives only on the child instance
/// (Java never stores it on the MI root), and it is execution-local there.
#[test]
fn sequential_mi_loop_counter_is_local_on_child_and_absent_from_root() {
    let engine = ProcessEngine::new("mi-parity-seq-loop-counter".to_string());
    let definition_id = deploy(&engine, "mi_sequential.bpmn20.xml", SEQUENTIAL_MI_XML);
    let process_instance = start(&engine, definition_id, HashMap::new());
    let runtime = engine.get_runtime_service();
    let task_service = engine.get_task_service();

    let tasks = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 1);

    let child = tasks[0].execution_id.clone();
    assert_eq!(
        runtime
            .get_variable_local(child.clone(), "loopCounter".to_string())
            .unwrap(),
        Some(json!(0)),
        "loopCounter must be execution-local on the sequential MI child"
    );

    let root = mi_root_id(&engine, &child);
    assert_eq!(
        runtime
            .get_variable(root, "loopCounter".to_string())
            .unwrap(),
        None,
        "Java keeps no loopCounter on the MI root execution"
    );
}

/// Difference #3: collection resolution walks the parent VariableScope chain
/// (`resolveAndValidateCollection` → `execution.getVariable`). With a fork in
/// front of the MI activity the collection lives on the process-instance scope
/// row, not on the (empty) MI root branch execution.
#[test]
fn mi_collection_resolves_through_parent_scope_chain() {
    let engine = ProcessEngine::new("mi-parity-fork-collection".to_string());
    let definition_id = deploy(
        &engine,
        "mi_fork_collection.bpmn20.xml",
        FORK_MI_COLLECTION_XML,
    );
    let mut variables = HashMap::new();
    variables.insert("approvers".to_string(), json!(["amy", "ben", "cy"]));
    let process_instance = start(&engine, definition_id, variables);
    let runtime = engine.get_runtime_service();
    let task_service = engine.get_task_service();

    let tasks = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(
        tasks.len(),
        4,
        "3 MI instances + taskB should be active after the fork"
    );

    let mut approvers = tasks
        .iter()
        .filter(|task| task.task_definition_key == "miTask")
        .map(|task| {
            runtime
                .get_variable(task.execution_id.clone(), "approver".to_string())
                .unwrap()
                .and_then(|value| value.as_str().map(ToOwned::to_owned))
                .expect("element variable on MI child")
        })
        .collect::<Vec<_>>();
    approvers.sort();
    assert_eq!(approvers, vec!["amy", "ben", "cy"]);
}

/// Difference #4: `completionCondition` is evaluated against the MI root with
/// variable resolution through the parent chain (`expressionManager…getValue(execution)`),
/// so a process-level `threshold` must be visible.
#[test]
fn completion_condition_resolves_process_variables_through_parent_chain() {
    let engine = ProcessEngine::new("mi-parity-fork-completion".to_string());
    let definition_id = deploy(
        &engine,
        "mi_fork_completion.bpmn20.xml",
        FORK_MI_COMPLETION_CONDITION_XML,
    );
    let mut variables = HashMap::new();
    variables.insert("threshold".to_string(), json!(2));
    let process_instance = start(&engine, definition_id, variables);
    let task_service = engine.get_task_service();

    let tasks = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 4, "3 MI instances + taskB");

    let mi_tasks = tasks
        .iter()
        .filter(|task| task.task_definition_key == "miTask")
        .collect::<Vec<_>>();
    task_service
        .complete_task_by_id(mi_tasks[0].id.clone())
        .unwrap();
    task_service
        .complete_task_by_id(mi_tasks[1].id.clone())
        .unwrap();

    let remaining = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(
        remaining.len(),
        1,
        "completion condition must cancel the third MI instance once threshold is reached"
    );
    assert_eq!(remaining[0].task_definition_key, "taskB");
}
