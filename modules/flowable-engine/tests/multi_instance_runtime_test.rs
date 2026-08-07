use flowable_engine::engine::process_engine::ProcessEngine;
use serde_json::json;
use std::collections::HashMap;

const SEQUENTIAL_MI_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
    <process id="sequentialMIProcess" isExecutable="true">
        <startEvent id="startEvent" />
        <sequenceFlow id="flow1" sourceRef="startEvent" targetRef="miTask" />
        <userTask id="miTask" name="MI Task">
            <multiInstanceLoopCharacteristics isSequential="true">
                <loopCardinality>3</loopCardinality>
            </multiInstanceLoopCharacteristics>
        </userTask>
        <sequenceFlow id="flow2" sourceRef="miTask" targetRef="endEvent" />
        <endEvent id="endEvent" />
    </process>
</definitions>"#;

#[test]
fn test_sequential_multi_instance_user_task() {
    let process_engine = ProcessEngine::new("default".to_string());
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();
    let task_service = process_engine.get_task_service();

    let deployment_builder = repository_service
        .create_deployment()
        .name("Sequential MI Deployment".to_string())
        .add_string(
            "sequential_mi.bpmn20.xml".to_string(),
            SEQUENTIAL_MI_XML.to_string(),
        );
    repository_service.deploy(deployment_builder).unwrap();

    let process_def_id = repository_service.get_process_definition_ids().unwrap()[0].clone();

    let builder = runtime_service
        .create_process_instance_builder()
        .process_definition_id(process_def_id);
    let process_instance = runtime_service.start_process_instance(builder).unwrap();

    // 1. Should have 1 task (first instance)
    let tasks = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 1, "Should have 1 task (instance 1)");
    assert_eq!(tasks[0].task_definition_key, "miTask");

    // 2. Complete first task, should have second task
    task_service
        .complete_task_by_id(tasks[0].id.clone())
        .unwrap();
    let tasks = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 1, "Should have 1 task (instance 2)");

    // 3. Complete second task, should have third task
    task_service
        .complete_task_by_id(tasks[0].id.clone())
        .unwrap();
    let tasks = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 1, "Should have 1 task (instance 3)");

    // 4. Complete third task, process should end
    task_service
        .complete_task_by_id(tasks[0].id.clone())
        .unwrap();
    let runtime_store = process_engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let pi = runtime_store
        .find_process_instance(&process_instance.id, &mut session)
        .unwrap();
    assert!(pi.is_ended, "Process instance should be ended");
}

const PARALLEL_MI_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
    <process id="parallelMIProcess" isExecutable="true">
        <startEvent id="startEvent" />
        <sequenceFlow id="flow1" sourceRef="startEvent" targetRef="miTask" />
        <userTask id="miTask" name="MI Task">
            <multiInstanceLoopCharacteristics isSequential="false">
                <loopCardinality>3</loopCardinality>
            </multiInstanceLoopCharacteristics>
        </userTask>
        <sequenceFlow id="flow2" sourceRef="miTask" targetRef="endEvent" />
        <endEvent id="endEvent" />
    </process>
</definitions>"#;

#[test]
fn test_parallel_multi_instance_user_task() {
    let process_engine = ProcessEngine::new("default".to_string());
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();
    let task_service = process_engine.get_task_service();

    let deployment_builder = repository_service
        .create_deployment()
        .name("Parallel MI Deployment".to_string())
        .add_string(
            "parallel_mi.bpmn20.xml".to_string(),
            PARALLEL_MI_XML.to_string(),
        );
    repository_service.deploy(deployment_builder).unwrap();

    let process_def_id = repository_service.get_process_definition_ids().unwrap()[0].clone();

    let builder = runtime_service
        .create_process_instance_builder()
        .process_definition_id(process_def_id);
    let process_instance = runtime_service.start_process_instance(builder).unwrap();

    // 1. Should have 3 tasks (all instances in parallel)
    let tasks = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 3, "Should have 3 tasks in parallel");

    // 2. Complete tasks one by one
    task_service
        .complete_task_by_id(tasks[0].id.clone())
        .unwrap();
    let tasks_remaining = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(tasks_remaining.len(), 2);

    task_service
        .complete_task_by_id(tasks[1].id.clone())
        .unwrap();
    let tasks_remaining = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(tasks_remaining.len(), 1);

    task_service
        .complete_task_by_id(tasks[2].id.clone())
        .unwrap();

    // 3. Process should end
    let runtime_store = process_engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let pi = runtime_store
        .find_process_instance(&process_instance.id, &mut session)
        .unwrap();
    assert!(pi.is_ended, "Process instance should be ended");
}

const PARALLEL_MI_COMPLETION_CONDITION_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
    <process id="parallelMICompletionConditionProcess" isExecutable="true">
        <startEvent id="startEvent" />
        <sequenceFlow id="flow1" sourceRef="startEvent" targetRef="miTask" />
        <userTask id="miTask" name="MI Task">
            <multiInstanceLoopCharacteristics isSequential="false">
                <loopCardinality>3</loopCardinality>
                <completionCondition>${nrOfCompletedInstances == 2}</completionCondition>
            </multiInstanceLoopCharacteristics>
        </userTask>
        <sequenceFlow id="flow2" sourceRef="miTask" targetRef="afterMiTask" />
        <userTask id="afterMiTask" name="After MI Task" />
        <sequenceFlow id="flow3" sourceRef="afterMiTask" targetRef="endEvent" />
        <endEvent id="endEvent" />
    </process>
</definitions>"#;

#[test]
fn parallel_multi_instance_completion_condition_leaves_and_cancels_remaining_tasks() {
    let process_engine = ProcessEngine::new("default".to_string());
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();
    let task_service = process_engine.get_task_service();

    let deployment_builder = repository_service
        .create_deployment()
        .name("Parallel MI Completion Condition Deployment".to_string())
        .add_string(
            "parallel_mi_completion_condition.bpmn20.xml".to_string(),
            PARALLEL_MI_COMPLETION_CONDITION_XML.to_string(),
        );
    repository_service.deploy(deployment_builder).unwrap();

    let process_def_id = repository_service.get_process_definition_ids().unwrap()[0].clone();

    let builder = runtime_service
        .create_process_instance_builder()
        .process_definition_id(process_def_id);
    let process_instance = runtime_service.start_process_instance(builder).unwrap();

    let tasks = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 3, "all parallel instances should start");

    task_service
        .complete_task_by_id(tasks[0].id.clone())
        .unwrap();
    let tasks_after_first = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(
        tasks_after_first.len(),
        2,
        "completion condition should not fire after one completion"
    );

    task_service
        .complete_task_by_id(tasks[1].id.clone())
        .unwrap();
    let tasks_after_condition = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(
        tasks_after_condition.len(),
        1,
        "completion condition should cancel remaining MI work and continue once"
    );
    assert_eq!(tasks_after_condition[0].task_definition_key, "afterMiTask");
}

const PARALLEL_MI_COLLECTION_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
    <process id="parallelMICollectionProcess" isExecutable="true">
        <startEvent id="startEvent" />
        <sequenceFlow id="flow1" sourceRef="startEvent" targetRef="miTask" />
        <userTask id="miTask" name="MI Task">
            <multiInstanceLoopCharacteristics isSequential="false"
                                              flowable:collection="approvers"
                                              flowable:elementVariable="approver" />
        </userTask>
        <sequenceFlow id="flow2" sourceRef="miTask" targetRef="endEvent" />
        <endEvent id="endEvent" />
    </process>
</definitions>"#;

const PARALLEL_MI_COLLECTION_EXPRESSION_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
    <process id="parallelMICollectionExpressionProcess" isExecutable="true">
        <startEvent id="startEvent" />
        <sequenceFlow id="flow1" sourceRef="startEvent" targetRef="miTask" />
        <userTask id="miTask" name="MI Task">
            <multiInstanceLoopCharacteristics isSequential="false"
                                              flowable:collection="${approvers}"
                                              flowable:elementVariable="approver" />
        </userTask>
        <sequenceFlow id="flow2" sourceRef="miTask" targetRef="endEvent" />
        <endEvent id="endEvent" />
    </process>
</definitions>"#;

const PARALLEL_MI_EMPTY_COLLECTION_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
    <process id="parallelMIEmptyCollectionProcess" isExecutable="true">
        <startEvent id="startEvent" />
        <sequenceFlow id="flow1" sourceRef="startEvent" targetRef="miTask" />
        <userTask id="miTask" name="MI Task">
            <multiInstanceLoopCharacteristics isSequential="false"
                                              flowable:collection="approvers"
                                              flowable:elementVariable="approver" />
        </userTask>
        <sequenceFlow id="flow2" sourceRef="miTask" targetRef="afterMiTask" />
        <userTask id="afterMiTask" name="After MI Task" />
        <sequenceFlow id="flow3" sourceRef="afterMiTask" targetRef="endEvent" />
        <endEvent id="endEvent" />
    </process>
</definitions>"#;

#[test]
fn parallel_multi_instance_collection_sets_element_variable_per_instance() {
    let process_engine = ProcessEngine::new("default".to_string());
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();
    let task_service = process_engine.get_task_service();

    let deployment_builder = repository_service
        .create_deployment()
        .name("Parallel MI Collection Deployment".to_string())
        .add_string(
            "parallel_mi_collection.bpmn20.xml".to_string(),
            PARALLEL_MI_COLLECTION_XML.to_string(),
        );
    repository_service.deploy(deployment_builder).unwrap();

    let process_def_id = repository_service.get_process_definition_ids().unwrap()[0].clone();

    let builder = runtime_service
        .create_process_instance_builder()
        .process_definition_id(process_def_id)
        .variable("approvers".to_string(), json!(["amy", "ben", "cy"]));
    let process_instance = runtime_service.start_process_instance(builder).unwrap();

    let tasks = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 3, "collection should drive MI cardinality");

    let runtime_store = process_engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let mut approvers = tasks
        .iter()
        .map(|task| {
            runtime_store
                .find_execution(&task.execution_id, &mut session)
                .expect("task execution")
                .process_variable("approver")
                .and_then(|value| value.as_str().map(ToOwned::to_owned))
                .expect("element variable")
        })
        .collect::<Vec<_>>();
    approvers.sort();
    assert_eq!(approvers, vec!["amy", "ben", "cy"]);
}

#[test]
fn parallel_multi_instance_collection_accepts_simple_expression_reference() {
    let process_engine = ProcessEngine::new("default".to_string());
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();
    let task_service = process_engine.get_task_service();

    let deployment_builder = repository_service
        .create_deployment()
        .name("Parallel MI Collection Expression Deployment".to_string())
        .add_string(
            "parallel_mi_collection_expression.bpmn20.xml".to_string(),
            PARALLEL_MI_COLLECTION_EXPRESSION_XML.to_string(),
        );
    repository_service.deploy(deployment_builder).unwrap();

    let process_def_id = repository_service.get_process_definition_ids().unwrap()[0].clone();

    let builder = runtime_service
        .create_process_instance_builder()
        .process_definition_id(process_def_id)
        .variable("approvers".to_string(), json!(["amy", "ben"]));
    let process_instance = runtime_service.start_process_instance(builder).unwrap();

    let tasks = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(
        tasks.len(),
        2,
        "${{approvers}} should resolve to the approvers collection"
    );
}

#[test]
fn parallel_multi_instance_empty_collection_continues_without_creating_instances() {
    let process_engine = ProcessEngine::new("default".to_string());
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();
    let task_service = process_engine.get_task_service();

    let deployment_builder = repository_service
        .create_deployment()
        .name("Parallel MI Empty Collection Deployment".to_string())
        .add_string(
            "parallel_mi_empty_collection.bpmn20.xml".to_string(),
            PARALLEL_MI_EMPTY_COLLECTION_XML.to_string(),
        );
    repository_service.deploy(deployment_builder).unwrap();

    let process_def_id = repository_service.get_process_definition_ids().unwrap()[0].clone();

    let builder = runtime_service
        .create_process_instance_builder()
        .process_definition_id(process_def_id)
        .variable("approvers".to_string(), json!([]));
    let process_instance = runtime_service.start_process_instance(builder).unwrap();

    let tasks = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(
        tasks.len(),
        1,
        "empty MI collection should continue to the single outgoing task"
    );
    assert_eq!(tasks[0].task_definition_key, "afterMiTask");
}

#[test]
fn parallel_multi_instance_missing_collection_variable_fails_instead_of_creating_single_task() {
    let process_engine = ProcessEngine::new("default".to_string());
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();

    let deployment_builder = repository_service
        .create_deployment()
        .name("Parallel MI Missing Collection Deployment".to_string())
        .add_string(
            "parallel_mi_missing_collection.bpmn20.xml".to_string(),
            PARALLEL_MI_COLLECTION_XML.to_string(),
        );
    repository_service.deploy(deployment_builder).unwrap();

    let process_def_id = repository_service.get_process_definition_ids().unwrap()[0].clone();

    let builder = runtime_service
        .create_process_instance_builder()
        .process_definition_id(process_def_id);
    let error = runtime_service
        .start_process_instance(builder)
        .expect_err("missing MI collection variable should fail");
    let message = format!("{error:?}");
    // Java MultiInstanceActivityBehavior.java:500-501 / unresolved collection message
    assert!(
        message.contains("approvers")
            && (message.contains("was not found") || message.contains("Couldn't resolve")),
        "unexpected error: {message}"
    );
}

const SEQUENTIAL_MI_COLLECTION_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
    <process id="sequentialMICollectionProcess" isExecutable="true">
        <startEvent id="startEvent" />
        <sequenceFlow id="flow1" sourceRef="startEvent" targetRef="miTask" />
        <userTask id="miTask" name="MI Task">
            <multiInstanceLoopCharacteristics isSequential="true"
                                              flowable:collection="approvers"
                                              flowable:elementVariable="approver"
                                              flowable:elementIndexVariable="approverIndex" />
        </userTask>
        <sequenceFlow id="flow2" sourceRef="miTask" targetRef="endEvent" />
        <endEvent id="endEvent" />
    </process>
</definitions>"#;

#[test]
fn sequential_multi_instance_collection_advances_element_and_index_variables() {
    let process_engine = ProcessEngine::new("default".to_string());
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();
    let task_service = process_engine.get_task_service();

    let deployment_builder = repository_service
        .create_deployment()
        .name("Sequential MI Collection Deployment".to_string())
        .add_string(
            "sequential_mi_collection.bpmn20.xml".to_string(),
            SEQUENTIAL_MI_COLLECTION_XML.to_string(),
        );
    repository_service.deploy(deployment_builder).unwrap();

    let process_def_id = repository_service.get_process_definition_ids().unwrap()[0].clone();

    let builder = runtime_service
        .create_process_instance_builder()
        .process_definition_id(process_def_id)
        .variable("approvers".to_string(), json!(["amy", "ben", "cy"]));
    let process_instance = runtime_service.start_process_instance(builder).unwrap();

    for (expected_index, expected_approver) in ["amy", "ben", "cy"].into_iter().enumerate() {
        let tasks = task_service
            .get_tasks_by_process_instance_id(process_instance.id.clone())
            .unwrap();
        assert_eq!(
            tasks.len(),
            1,
            "sequential MI should expose one active task"
        );

        let runtime_store = process_engine.get_runtime_store();
        let mut session = runtime_store.create_session().unwrap();
        let execution = runtime_store
            .find_execution(&tasks[0].execution_id, &mut session)
            .expect("task execution");
        assert_eq!(
            execution.process_variable("approver").as_ref(),
            Some(&json!(expected_approver))
        );
        assert_eq!(
            execution.process_variable("approverIndex").as_ref(),
            Some(&json!(expected_index))
        );
        drop(session);

        task_service
            .complete_task_by_id(tasks[0].id.clone())
            .unwrap();
    }

    let runtime_store = process_engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let process_instance = runtime_store
        .find_process_instance(&process_instance.id, &mut session)
        .unwrap();
    assert!(process_instance.is_ended);
}

const PARALLEL_MI_SERVICE_TASK_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
    <process id="parallelMIServiceTaskProcess" isExecutable="true">
        <startEvent id="startEvent" />
        <sequenceFlow id="flow1" sourceRef="startEvent" targetRef="miServiceTask" />
        <serviceTask id="miServiceTask" name="MI Service Task">
            <multiInstanceLoopCharacteristics isSequential="false">
                <loopCardinality>3</loopCardinality>
            </multiInstanceLoopCharacteristics>
        </serviceTask>
        <sequenceFlow id="flow2" sourceRef="miServiceTask" targetRef="afterMiTask" />
        <userTask id="afterMiTask" name="After MI Task" />
        <sequenceFlow id="flow3" sourceRef="afterMiTask" targetRef="endEvent" />
        <endEvent id="endEvent" />
    </process>
</definitions>"#;

#[test]
fn parallel_multi_instance_service_task_continues_once_after_all_instances_complete() {
    let process_engine = ProcessEngine::new("default".to_string());
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();
    let task_service = process_engine.get_task_service();

    let deployment_builder = repository_service
        .create_deployment()
        .name("Parallel MI Service Task Deployment".to_string())
        .add_string(
            "parallel_mi_service_task.bpmn20.xml".to_string(),
            PARALLEL_MI_SERVICE_TASK_XML.to_string(),
        );
    repository_service.deploy(deployment_builder).unwrap();

    let process_def_id = repository_service.get_process_definition_ids().unwrap()[0].clone();
    let process_instance = runtime_service
        .start_process_instance(
            runtime_service
                .create_process_instance_builder()
                .process_definition_id(process_def_id),
        )
        .unwrap();

    let tasks = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(
        tasks.len(),
        1,
        "parallel service task MI should leave the MI activity once, not once per child"
    );
    assert_eq!(tasks[0].task_definition_key, "afterMiTask");
}

const PARALLEL_MI_VARIABLE_AGGREGATION_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
    <process id="parallelMIVariableAggregationProcess" isExecutable="true">
        <startEvent id="startEvent" />
        <sequenceFlow id="flow1" sourceRef="startEvent" targetRef="miTask" />
        <userTask id="miTask" name="MI Task">
            <multiInstanceLoopCharacteristics isSequential="false">
                <extensionElements>
                    <flowable:variableAggregation target="reviews">
                        <variable source="approver" target="userId" />
                        <variable source="approved" />
                    </flowable:variableAggregation>
                </extensionElements>
                <loopCardinality>2</loopCardinality>
            </multiInstanceLoopCharacteristics>
        </userTask>
        <sequenceFlow id="flow2" sourceRef="miTask" targetRef="afterMiTask" />
        <userTask id="afterMiTask" name="After MI Task" />
        <sequenceFlow id="flow3" sourceRef="afterMiTask" targetRef="endEvent" />
        <endEvent id="endEvent" />
    </process>
</definitions>"#;

#[test]
fn multi_instance_variable_aggregation_collects_task_local_completion_variables() {
    let process_engine = ProcessEngine::new("default".to_string());
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();
    let task_service = process_engine.get_task_service();

    let deployment_builder = repository_service
        .create_deployment()
        .name("Parallel MI Variable Aggregation Deployment".to_string())
        .add_string(
            "parallel_mi_variable_aggregation.bpmn20.xml".to_string(),
            PARALLEL_MI_VARIABLE_AGGREGATION_XML.to_string(),
        );
    repository_service.deploy(deployment_builder).unwrap();

    let process_def_id = repository_service.get_process_definition_ids().unwrap()[0].clone();
    let process_instance = runtime_service
        .start_process_instance(
            runtime_service
                .create_process_instance_builder()
                .process_definition_id(process_def_id),
        )
        .unwrap();

    let mut tasks = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    tasks.sort_by(|left, right| left.id.cmp(&right.id));
    assert_eq!(tasks.len(), 2, "parallel MI should create two tasks");

    for (task, approver, approved) in [(&tasks[0], "amy", true), (&tasks[1], "ben", false)] {
        let mut variables = HashMap::new();
        variables.insert("approver".to_string(), json!(approver));
        variables.insert("approved".to_string(), json!(approved));
        task_service
            .complete_task_by_id_with_local_variables(task.id.clone(), variables)
            .unwrap();
    }

    let runtime_store = process_engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let root_execution = runtime_store
        .find_execution(&process_instance.id, &mut session)
        .expect("root execution");
    let mut reviews = root_execution
        .process_variable("reviews")
        .and_then(|value| value.as_array().cloned())
        .expect("reviews aggregation");
    reviews.sort_by(|left, right| left["userId"].as_str().cmp(&right["userId"].as_str()));

    assert_eq!(
        reviews,
        vec![
            json!({"userId": "amy", "approved": true}),
            json!({"userId": "ben", "approved": false}),
        ]
    );
}

// ---------------------------------------------------------------------------
// P82a — loopCardinality / collection EL evaluation
// ---------------------------------------------------------------------------

/// Parallel MI with loopCardinality as EL variable reference `${nrOfLoops}`.
const PARALLEL_MI_LOOP_CARDINALITY_EL_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
    <process id="parallelMILoopCardElProcess" isExecutable="true">
        <startEvent id="startEvent" />
        <sequenceFlow id="flow1" sourceRef="startEvent" targetRef="miTask" />
        <userTask id="miTask" name="MI Task">
            <multiInstanceLoopCharacteristics isSequential="false">
                <loopCardinality>${nrOfLoops}</loopCardinality>
            </multiInstanceLoopCharacteristics>
        </userTask>
        <sequenceFlow id="flow2" sourceRef="miTask" targetRef="endEvent" />
        <endEvent id="endEvent" />
    </process>
</definitions>"#;

/// loopCardinality EL + collection both present; cardinality must win.
const PARALLEL_MI_CARDINALITY_AND_COLLECTION_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
    <process id="parallelMICardAndCollProcess" isExecutable="true">
        <startEvent id="startEvent" />
        <sequenceFlow id="flow1" sourceRef="startEvent" targetRef="miTask" />
        <userTask id="miTask" name="MI Task">
            <multiInstanceLoopCharacteristics isSequential="false"
                                              flowable:collection="approvers"
                                              flowable:elementVariable="approver">
                <loopCardinality>${nrOfLoops}</loopCardinality>
            </multiInstanceLoopCharacteristics>
        </userTask>
        <sequenceFlow id="flow2" sourceRef="miTask" targetRef="endEvent" />
        <endEvent id="endEvent" />
    </process>
</definitions>"#;

fn deploy_and_start_mi(
    engine_name: &str,
    xml: &str,
    resource: &str,
    variables: HashMap<String, serde_json::Value>,
) -> Result<(ProcessEngine, String), flowable_engine::error::FlowableError> {
    let process_engine = ProcessEngine::new(engine_name.to_string());
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();

    let deployment_builder = repository_service
        .create_deployment()
        .name(format!("{engine_name} deployment"))
        .add_string(resource.to_string(), xml.to_string());
    repository_service.deploy(deployment_builder).unwrap();

    let process_def_id = repository_service.get_process_definition_ids().unwrap()[0].clone();
    let mut builder = runtime_service
        .create_process_instance_builder()
        .process_definition_id(process_def_id);
    for (k, v) in variables {
        builder = builder.variable(k, v);
    }
    let pi = runtime_service.start_process_instance(builder)?;
    Ok((process_engine, pi.id))
}

/// P82a: `${nrOfLoops}` evaluates to a number → N parallel tasks.
#[test]
fn p82a_loop_cardinality_el_number_variable() {
    let mut vars = HashMap::new();
    vars.insert("nrOfLoops".to_string(), json!(3));
    let (engine, pi_id) = deploy_and_start_mi(
        "p82a-card-number",
        PARALLEL_MI_LOOP_CARDINALITY_EL_XML,
        "p82a_card_number.bpmn20.xml",
        vars,
    )
    .expect("start should succeed");
    let tasks = engine
        .get_task_service()
        .get_tasks_by_process_instance_id(pi_id)
        .unwrap();
    assert_eq!(tasks.len(), 3, "nrOfLoops=3 (number) should create 3 instances");
}

/// P82a: `${nrOfLoops}` evaluates to a numeric string → Integer.valueOf path.
#[test]
fn p82a_loop_cardinality_el_number_string_variable() {
    let mut vars = HashMap::new();
    vars.insert("nrOfLoops".to_string(), json!("2"));
    let (engine, pi_id) = deploy_and_start_mi(
        "p82a-card-numstr",
        PARALLEL_MI_LOOP_CARDINALITY_EL_XML,
        "p82a_card_numstr.bpmn20.xml",
        vars,
    )
    .expect("start should succeed");
    let tasks = engine
        .get_task_service()
        .get_tasks_by_process_instance_id(pi_id)
        .unwrap();
    assert_eq!(
        tasks.len(),
        2,
        "nrOfLoops=\"2\" (number String) should create 2 instances"
    );
}

/// P82a: `${nrOfLoops}` is boolean → error (not a number nor number String).
#[test]
fn p82a_loop_cardinality_el_boolean_rejected() {
    let mut vars = HashMap::new();
    vars.insert("nrOfLoops".to_string(), json!(true));
    let result = deploy_and_start_mi(
        "p82a-card-bool",
        PARALLEL_MI_LOOP_CARDINALITY_EL_XML,
        "p82a_card_bool.bpmn20.xml",
        vars,
    );
    let err = match result {
        Err(e) => e,
        Ok(_) => panic!("boolean loopCardinality must fail"),
    };
    let message = format!("{err:?}");
    assert!(
        message.contains("Could not resolve loopCardinality expression")
            && message.contains("not a number nor number String"),
        "unexpected error: {message}"
    );
}

/// P82a: `${nrOfLoops}` variable missing → error (same Java message).
#[test]
fn p82a_loop_cardinality_el_missing_variable_rejected() {
    let result = deploy_and_start_mi(
        "p82a-card-missing",
        PARALLEL_MI_LOOP_CARDINALITY_EL_XML,
        "p82a_card_missing.bpmn20.xml",
        HashMap::new(),
    );
    let err = match result {
        Err(e) => e,
        Ok(_) => panic!("missing loopCardinality variable must fail"),
    };
    let message = format!("{err:?}");
    assert!(
        message.contains("Could not resolve loopCardinality expression")
            && message.contains("not a number nor number String"),
        "unexpected error: {message}"
    );
}

/// P82a regression: literal loopCardinality `5` (no EL braces) still works.
#[test]
fn p82a_loop_cardinality_literal_five_regression() {
    const XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
    <process id="parallelMILiteral5" isExecutable="true">
        <startEvent id="startEvent" />
        <sequenceFlow id="flow1" sourceRef="startEvent" targetRef="miTask" />
        <userTask id="miTask" name="MI Task">
            <multiInstanceLoopCharacteristics isSequential="false">
                <loopCardinality>5</loopCardinality>
            </multiInstanceLoopCharacteristics>
        </userTask>
        <sequenceFlow id="flow2" sourceRef="miTask" targetRef="endEvent" />
        <endEvent id="endEvent" />
    </process>
</definitions>"#;
    let (engine, pi_id) = deploy_and_start_mi(
        "p82a-card-literal5",
        XML,
        "p82a_card_literal5.bpmn20.xml",
        HashMap::new(),
    )
    .expect("literal 5 should succeed");
    let tasks = engine
        .get_task_service()
        .get_tasks_by_process_instance_id(pi_id)
        .unwrap();
    assert_eq!(tasks.len(), 5, "literal loopCardinality 5 should create 5 instances");
}

/// P82a: collection array variable still drives cardinality when no loopCardinality.
#[test]
fn p82a_collection_array_variable_unchanged() {
    let mut vars = HashMap::new();
    vars.insert("approvers".to_string(), json!(["a", "b", "c", "d"]));
    let (engine, pi_id) = deploy_and_start_mi(
        "p82a-coll-array",
        PARALLEL_MI_COLLECTION_XML,
        "p82a_coll_array.bpmn20.xml",
        vars,
    )
    .expect("collection start should succeed");
    let tasks = engine
        .get_task_service()
        .get_tasks_by_process_instance_id(pi_id)
        .unwrap();
    assert_eq!(tasks.len(), 4, "collection of 4 should create 4 instances");
}

/// P82a: loopCardinality takes precedence over collection size when both present.
/// Java MultiInstanceActivityBehavior#resolveNrOfInstances lines 450-461.
#[test]
fn p82a_loop_cardinality_preferred_over_collection() {
    let mut vars = HashMap::new();
    // Collection has 4 items; loopCardinality says 2 → only 2 instances.
    vars.insert("approvers".to_string(), json!(["a", "b", "c", "d"]));
    vars.insert("nrOfLoops".to_string(), json!(2));
    let (engine, pi_id) = deploy_and_start_mi(
        "p82a-card-over-coll",
        PARALLEL_MI_CARDINALITY_AND_COLLECTION_XML,
        "p82a_card_over_coll.bpmn20.xml",
        vars,
    )
    .expect("start should succeed");
    let tasks = engine
        .get_task_service()
        .get_tasks_by_process_instance_id(pi_id)
        .unwrap();
    assert_eq!(
        tasks.len(),
        2,
        "loopCardinality=2 must win over collection size 4"
    );
}
