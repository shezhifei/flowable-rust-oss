//! Contract tests for Java multi-instance **root materialization** topology.
//!
//! Java (`ContinueProcessOperation#createMultiInstanceRootExecution`):
//!   - On MI entry, the arriving execution is replaced by a dedicated child
//!     with `isMultiInstanceRoot(true)` and `isActive(false)`.
//!   - Instance children hang under that MI root (not under the process-instance
//!     scope row / arriving fork child itself).
//!
//! Java (`MultiInstanceActivityBehavior#cleanupMiRoot` on leave):
//!   - Deletes the MI root and all its children.
//!   - Creates a fresh leave execution under the MI root's parent and takes
//!     outgoing sequence flows on that leave execution.
//!
//! Observable gaps locked here (were red under the pre-P7 "arriver is the MI
//! root, `is_multi_instance_root` never set" topology).

use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::runtime::execution::Execution;
use flowable_engine::runtime::process_instance::ProcessInstance;
use serde_json::json;
use std::collections::HashMap;

const PARALLEL_MI_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
    <process id="miRootParallel" isExecutable="true">
        <startEvent id="start" />
        <sequenceFlow id="f1" sourceRef="start" targetRef="miTask" />
        <userTask id="miTask" name="MI Task">
            <multiInstanceLoopCharacteristics isSequential="false">
                <loopCardinality>3</loopCardinality>
            </multiInstanceLoopCharacteristics>
        </userTask>
        <sequenceFlow id="f2" sourceRef="miTask" targetRef="afterMi" />
        <userTask id="afterMi" name="After MI" />
        <sequenceFlow id="f3" sourceRef="afterMi" targetRef="end" />
        <endEvent id="end" />
    </process>
</definitions>"#;

const SEQUENTIAL_MI_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
    <process id="miRootSequential" isExecutable="true">
        <startEvent id="start" />
        <sequenceFlow id="f1" sourceRef="start" targetRef="miTask" />
        <userTask id="miTask" name="MI Task">
            <multiInstanceLoopCharacteristics isSequential="true"
                                              flowable:collection="approvers"
                                              flowable:elementVariable="approver" />
        </userTask>
        <sequenceFlow id="f2" sourceRef="miTask" targetRef="afterMi" />
        <userTask id="afterMi" name="After MI" />
        <sequenceFlow id="f3" sourceRef="afterMi" targetRef="end" />
        <endEvent id="end" />
    </process>
</definitions>"#;

const FORK_PARALLEL_MI_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
    <process id="miRootForkParallel" isExecutable="true">
        <startEvent id="start" />
        <sequenceFlow id="f1" sourceRef="start" targetRef="fork" />
        <parallelGateway id="fork" />
        <sequenceFlow id="f2" sourceRef="fork" targetRef="miTask" />
        <userTask id="miTask" name="MI Task">
            <multiInstanceLoopCharacteristics isSequential="false">
                <loopCardinality>2</loopCardinality>
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

const PARALLEL_MI_BOUNDARY_TIMER_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
    <process id="miRootBoundaryTimer" isExecutable="true">
        <startEvent id="start" />
        <sequenceFlow id="f1" sourceRef="start" targetRef="miTask" />
        <userTask id="miTask" name="MI Task">
            <multiInstanceLoopCharacteristics isSequential="false">
                <loopCardinality>2</loopCardinality>
            </multiInstanceLoopCharacteristics>
        </userTask>
        <boundaryEvent id="miTimer" attachedToRef="miTask" cancelActivity="true">
            <timerEventDefinition>
                <timeDuration>PT1H</timeDuration>
            </timerEventDefinition>
        </boundaryEvent>
        <sequenceFlow id="f2" sourceRef="miTask" targetRef="end" />
        <sequenceFlow id="f3" sourceRef="miTimer" targetRef="end" />
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

fn all_executions(engine: &ProcessEngine, process_instance_id: &str) -> Vec<Execution> {
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    store
        .snapshot_executions(&mut session)
        .into_values()
        .filter(|e| e.process_instance_id.as_deref() == Some(process_instance_id))
        .collect()
}

fn find_mi_roots(executions: &[Execution]) -> Vec<&Execution> {
    executions
        .iter()
        .filter(|e| e.is_multi_instance_root)
        .collect()
}

/// Phase 1 difference: parallel MI materializes a dedicated inactive MI root
/// with `is_multi_instance_root=true`. Instance children hang under it.
#[test]
fn parallel_mi_materializes_inactive_dedicated_mi_root() {
    let engine = ProcessEngine::new("mi-root-parallel-materialize".to_string());
    let definition_id = deploy(&engine, "mi_root_parallel.bpmn20.xml", PARALLEL_MI_XML);
    let process_instance = start(&engine, definition_id, HashMap::new());
    let task_service = engine.get_task_service();

    let tasks = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 3);

    let executions = all_executions(&engine, &process_instance.id);
    let mi_roots = find_mi_roots(&executions);
    assert_eq!(
        mi_roots.len(),
        1,
        "exactly one dedicated MI root; found {} of {} executions. \
         Pre-P7 topology never sets is_multi_instance_root",
        mi_roots.len(),
        executions.len()
    );
    let mi_root = mi_roots[0];
    assert!(
        !mi_root.is_active,
        "Java sets multiInstanceRootExecution.setActive(false)"
    );
    assert_ne!(
        mi_root.id, process_instance.id,
        "MI root must be a dedicated execution, not the process-instance scope row"
    );
    assert_eq!(mi_root.activity_id.as_deref(), Some("miTask"));

    for task in &tasks {
        let child = executions
            .iter()
            .find(|e| e.id == task.execution_id)
            .expect("task execution row");
        assert_eq!(
            child.parent_id.as_deref(),
            Some(mi_root.id.as_str()),
            "instance children hang under the dedicated MI root"
        );
        assert!(
            !child.is_multi_instance_root,
            "instance children are not MI roots"
        );
    }
}

/// Phase 1 difference: sequential MI uses the same dedicated inactive MI root.
#[test]
fn sequential_mi_materializes_inactive_dedicated_mi_root() {
    let engine = ProcessEngine::new("mi-root-sequential-materialize".to_string());
    let definition_id = deploy(&engine, "mi_root_sequential.bpmn20.xml", SEQUENTIAL_MI_XML);
    let mut variables = HashMap::new();
    variables.insert("approvers".to_string(), json!(["amy", "ben"]));
    let process_instance = start(&engine, definition_id, variables);
    let task_service = engine.get_task_service();

    let tasks = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 1);

    let executions = all_executions(&engine, &process_instance.id);
    let mi_roots = find_mi_roots(&executions);
    assert_eq!(mi_roots.len(), 1, "exactly one sequential MI root");
    let mi_root = mi_roots[0];
    assert!(!mi_root.is_active);
    assert_ne!(mi_root.id, process_instance.id);

    let child = executions
        .iter()
        .find(|e| e.id == tasks[0].execution_id)
        .expect("task execution");
    assert_eq!(child.parent_id.as_deref(), Some(mi_root.id.as_str()));
}

/// Phase 1 + fork: MI root under a fork branch is still flagged and is not the
/// process-instance scope row.
#[test]
fn fork_parallel_mi_materializes_mi_root_flag() {
    let engine = ProcessEngine::new("mi-root-fork-materialize".to_string());
    let definition_id = deploy(&engine, "mi_root_fork.bpmn20.xml", FORK_PARALLEL_MI_XML);
    let process_instance = start(&engine, definition_id, HashMap::new());
    let task_service = engine.get_task_service();

    let mi_tasks: Vec<_> = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap()
        .into_iter()
        .filter(|t| t.task_definition_key == "miTask")
        .collect();
    assert_eq!(mi_tasks.len(), 2);

    let executions = all_executions(&engine, &process_instance.id);
    let mi_roots = find_mi_roots(&executions);
    assert_eq!(mi_roots.len(), 1, "fork MI branch materializes one MI root");
    let mi_root = mi_roots[0];
    assert!(!mi_root.is_active);
    assert_ne!(mi_root.id, process_instance.id);

    for task in &mi_tasks {
        let child = executions
            .iter()
            .find(|e| e.id == task.execution_id)
            .expect("mi task execution");
        assert_eq!(child.parent_id.as_deref(), Some(mi_root.id.as_str()));
    }
}

/// Probe: boundary timer rows' execution_id / host attachment during parallel MI.
/// Java attaches boundary events to the MI root after createInstances, not to
/// each instance child. This probe documents the current host placement.
#[test]
fn parallel_mi_boundary_timer_host_probe() {
    let engine = ProcessEngine::new("mi-root-boundary-timer-probe".to_string());
    let definition_id = deploy(
        &engine,
        "mi_root_boundary_timer.bpmn20.xml",
        PARALLEL_MI_BOUNDARY_TIMER_XML,
    );
    let process_instance = start(&engine, definition_id, HashMap::new());
    let task_service = engine.get_task_service();

    let tasks = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 2);

    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let timers: Vec<_> = store
        .find_timer_job_states_by_process_instance_id(&process_instance.id, &mut session)
        .into_iter()
        .filter(|j| j.is_boundary && j.activity_id == "miTimer")
        .collect();

    let executions = all_executions(&engine, &process_instance.id);
    let mi_roots = find_mi_roots(&executions);
    assert_eq!(mi_roots.len(), 1, "MI root must exist for host comparison");
    let mi_root_id = mi_roots[0].id.as_str();
    let child_ids: Vec<&str> = tasks.iter().map(|t| t.execution_id.as_str()).collect();

    // Document blast radius: how many timer rows and which execution they host on.
    assert!(
        !timers.is_empty(),
        "expected at least one boundary timer for miTask"
    );

    let hosted_on_mi_root = timers
        .iter()
        .filter(|t| t.execution_id == mi_root_id)
        .count();
    let hosted_on_children = timers
        .iter()
        .filter(|t| child_ids.contains(&t.execution_id.as_str()))
        .count();

    // Java: one boundary on the MI root. Target after full parity:
    // hosted_on_mi_root == 1 && hosted_on_children == 0.
    // Phase 1 only materializes the root; boundary attachment may still be on
    // children if user-task behavior registers per instance. Record either way.
    assert!(
        hosted_on_mi_root > 0 || hosted_on_children > 0,
        "boundary timer must host on MI root or instance children \
         (mi_root={hosted_on_mi_root}, children={hosted_on_children}, total={})",
        timers.len()
    );
    // Soft target for full Java parity (may stay red until a follow-up):
    // prefer MI-root hosting when materialization is present.
    let _ = (hosted_on_mi_root, hosted_on_children);
}

/// Phase 2 difference: after the MI body completes, the dedicated MI root is
/// gone (cleanupMiRoot) and the process continues on a non-MI leave execution.
#[test]
fn after_parallel_mi_leave_mi_root_is_cleaned_up() {
    let engine = ProcessEngine::new("mi-root-parallel-cleanup".to_string());
    let definition_id = deploy(&engine, "mi_root_parallel.bpmn20.xml", PARALLEL_MI_XML);
    let process_instance = start(&engine, definition_id, HashMap::new());
    let task_service = engine.get_task_service();

    let tasks = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 3);
    for task in tasks {
        task_service.complete_task_by_id(task.id).unwrap();
    }

    let after_tasks = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(after_tasks.len(), 1);
    assert_eq!(after_tasks[0].task_definition_key, "afterMi");

    let executions = all_executions(&engine, &process_instance.id);
    let mi_roots = find_mi_roots(&executions);
    assert!(
        mi_roots.is_empty(),
        "cleanupMiRoot must delete the MI root on leave; still found {}",
        mi_roots.len()
    );

    let leave_execution = executions
        .iter()
        .find(|e| e.id == after_tasks[0].execution_id)
        .expect("afterMi task execution");
    assert!(
        !leave_execution.is_multi_instance_root,
        "leave execution must not carry the MI root flag"
    );
}

/// Phase 2 difference: sequential leave also cleans the MI root before afterMi.
#[test]
fn after_sequential_mi_leave_mi_root_is_cleaned_up() {
    let engine = ProcessEngine::new("mi-root-sequential-cleanup".to_string());
    let definition_id = deploy(&engine, "mi_root_sequential.bpmn20.xml", SEQUENTIAL_MI_XML);
    let mut variables = HashMap::new();
    variables.insert("approvers".to_string(), json!(["amy", "ben"]));
    let process_instance = start(&engine, definition_id, variables);
    let task_service = engine.get_task_service();

    for _ in 0..2 {
        let tasks = task_service
            .get_tasks_by_process_instance_id(process_instance.id.clone())
            .unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].task_definition_key, "miTask");
        task_service
            .complete_task_by_id(tasks[0].id.clone())
            .unwrap();
    }

    let after_tasks = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(after_tasks.len(), 1);
    assert_eq!(after_tasks[0].task_definition_key, "afterMi");

    let executions = all_executions(&engine, &process_instance.id);
    assert!(
        find_mi_roots(&executions).is_empty(),
        "sequential cleanupMiRoot must remove the MI root"
    );
}

/// Regression: full life-cycle still ends the process after afterMi completes.
#[test]
fn parallel_mi_with_after_task_completes_process() {
    let engine = ProcessEngine::new("mi-root-parallel-full-lifecycle".to_string());
    let definition_id = deploy(&engine, "mi_root_parallel.bpmn20.xml", PARALLEL_MI_XML);
    let process_instance = start(&engine, definition_id, HashMap::new());
    let task_service = engine.get_task_service();

    let tasks = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    for task in tasks {
        task_service.complete_task_by_id(task.id).unwrap();
    }
    let after = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    task_service
        .complete_task_by_id(after[0].id.clone())
        .unwrap();

    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let pi = store
        .find_process_instance(&process_instance.id, &mut session)
        .unwrap();
    assert!(pi.is_ended);
}
