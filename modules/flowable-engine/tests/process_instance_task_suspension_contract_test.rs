use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::engine::query::Query;
use flowable_engine::engine::task_service::TaskUpdate;
use flowable_engine::runtime::process_instance::ProcessInstanceUpdate;
use serde_json::json;

fn deploy_and_start(engine: &ProcessEngine, id_suffix: &str) -> (String, String) {
    let repo = engine.get_repository_service();
    let runtime = engine.get_runtime_service();

    let xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="process{id_suffix}">
            <startEvent id="start" />
            <sequenceFlow id="f1" sourceRef="start" targetRef="task1" />
            <userTask id="task1" name="Task 1" />
            <sequenceFlow id="f2" sourceRef="task1" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#
    );

    repo.deploy(
        repo.create_deployment()
            .add_string(format!("process{id_suffix}.bpmn20.xml"), xml),
    )
    .unwrap();

    let def_id = repo.get_process_definition_ids().unwrap()[0].clone();
    let instance = runtime
        .start_process_instance(
            runtime
                .create_process_instance_builder()
                .process_definition_id(def_id.clone()),
        )
        .unwrap();

    (instance.id, def_id)
}

#[test]
fn suspend_cascades_to_tasks_and_query_reflects_state() {
    let engine = ProcessEngine::new("suspend-cascade-tasks".to_string());
    let runtime = engine.get_runtime_service();
    let task_service = engine.get_task_service();

    // Start two process instances so we can verify isolation
    let (pi_id_1, _) = deploy_and_start(&engine, "1");
    let (pi_id_2, _) = deploy_and_start(&engine, "2");

    // Both instances have one active task
    let tasks_1_before = task_service
        .get_tasks_by_process_instance_id(pi_id_1.clone())
        .unwrap();
    assert_eq!(tasks_1_before.len(), 1);
    assert_eq!(tasks_1_before[0].suspension_state, 0);
    assert!(!tasks_1_before[0].is_suspended());
    assert!(tasks_1_before[0].is_active());

    // Suspend process instance 1
    let suspended = runtime
        .suspend_process_instance(pi_id_1.clone(), ProcessInstanceUpdate::default())
        .unwrap();
    assert!(suspended.is_suspended);

    // Task from pi_1 should now be suspended
    let tasks_1_after = task_service
        .get_tasks_by_process_instance_id(pi_id_1.clone())
        .unwrap();
    assert_eq!(tasks_1_after.len(), 1);
    assert_eq!(tasks_1_after[0].suspension_state, 1);
    assert!(tasks_1_after[0].is_suspended());
    assert!(!tasks_1_after[0].is_active());

    // Task from pi_2 must remain active
    let tasks_2 = task_service
        .get_tasks_by_process_instance_id(pi_id_2.clone())
        .unwrap();
    assert_eq!(tasks_2.len(), 1);
    assert_eq!(tasks_2[0].suspension_state, 0);
    assert!(!tasks_2[0].is_suspended());

    // Activate process instance 1
    let activated = runtime
        .activate_process_instance(pi_id_1.clone(), ProcessInstanceUpdate::default())
        .unwrap();
    assert!(!activated.is_suspended);

    let tasks_1_final = task_service
        .get_tasks_by_process_instance_id(pi_id_1.clone())
        .unwrap();
    assert_eq!(tasks_1_final[0].suspension_state, 0);
    assert!(!tasks_1_final[0].is_suspended());
    assert!(tasks_1_final[0].is_active());
}

#[test]
fn task_query_suspended_and_active_filters() {
    let engine = ProcessEngine::new("task-query-suspension".to_string());
    let runtime = engine.get_runtime_service();
    let task_service = engine.get_task_service();

    let (pi_id_1, _) = deploy_and_start(&engine, "1");
    let (pi_id_2, _) = deploy_and_start(&engine, "2");

    // Suspend pi_1, keep pi_2 active
    runtime
        .suspend_process_instance(pi_id_1.clone(), ProcessInstanceUpdate::default())
        .unwrap();

    // Query only suspended tasks
    let suspended_tasks = task_service.create_task_query().suspended().list().unwrap();
    assert_eq!(suspended_tasks.len(), 1);
    assert_eq!(suspended_tasks[0].process_instance_id, pi_id_1);

    // Query only active tasks
    let active_tasks = task_service.create_task_query().active().list().unwrap();
    assert_eq!(active_tasks.len(), 1);
    assert_eq!(active_tasks[0].process_instance_id, pi_id_2);

    // Count works
    assert_eq!(
        task_service
            .create_task_query()
            .suspended()
            .count()
            .unwrap(),
        1
    );
    assert_eq!(
        task_service.create_task_query().active().count().unwrap(),
        1
    );

    // Combined with process_instance_id filter
    let pi_1_suspended = task_service
        .create_task_query()
        .process_instance_id(pi_id_1.clone())
        .suspended()
        .list()
        .unwrap();
    assert_eq!(pi_1_suspended.len(), 1);

    let pi_1_active = task_service
        .create_task_query()
        .process_instance_id(pi_id_1.clone())
        .active()
        .list()
        .unwrap();
    assert_eq!(pi_1_active.len(), 0);
}

#[test]
fn complete_suspended_task_rejected() {
    let engine = ProcessEngine::new("complete-suspended-rejected".to_string());
    let runtime = engine.get_runtime_service();
    let task_service = engine.get_task_service();

    let (pi_id, _) = deploy_and_start(&engine, "1");
    runtime
        .suspend_process_instance(pi_id.clone(), ProcessInstanceUpdate::default())
        .unwrap();

    let tasks = task_service
        .get_tasks_by_process_instance_id(pi_id)
        .unwrap();
    assert_eq!(tasks.len(), 1);

    let err = task_service
        .complete_task_by_id(tasks[0].id.clone())
        .expect_err("should reject completing a suspended task");
    assert!(err.to_string().contains("suspended task"));
}

#[test]
fn claim_suspended_task_rejected() {
    let engine = ProcessEngine::new("claim-suspended-rejected".to_string());
    let runtime = engine.get_runtime_service();
    let task_service = engine.get_task_service();

    let (pi_id, _) = deploy_and_start(&engine, "1");
    runtime
        .suspend_process_instance(pi_id.clone(), ProcessInstanceUpdate::default())
        .unwrap();

    let tasks = task_service
        .get_tasks_by_process_instance_id(pi_id)
        .unwrap();

    let err = task_service
        .claim_task_by_id(tasks[0].id.clone(), "user1".to_string())
        .expect_err("should reject claiming a suspended task");
    assert!(err.to_string().contains("suspended task"));
}

#[test]
fn unclaim_suspended_task_rejected() {
    let engine = ProcessEngine::new("unclaim-suspended-rejected".to_string());
    let runtime = engine.get_runtime_service();
    let task_service = engine.get_task_service();

    let (pi_id, _) = deploy_and_start(&engine, "1");
    runtime
        .suspend_process_instance(pi_id.clone(), ProcessInstanceUpdate::default())
        .unwrap();

    let tasks = task_service
        .get_tasks_by_process_instance_id(pi_id)
        .unwrap();

    let err = task_service
        .unclaim_task_by_id(tasks[0].id.clone())
        .expect_err("should reject unclaiming a suspended task");
    assert!(err.to_string().contains("suspended task"));
}

#[test]
fn delegate_suspended_task_rejected() {
    let engine = ProcessEngine::new("delegate-suspended-rejected".to_string());
    let runtime = engine.get_runtime_service();
    let task_service = engine.get_task_service();

    let (pi_id, _) = deploy_and_start(&engine, "1");
    runtime
        .suspend_process_instance(pi_id.clone(), ProcessInstanceUpdate::default())
        .unwrap();

    let tasks = task_service
        .get_tasks_by_process_instance_id(pi_id)
        .unwrap();

    let err = task_service
        .delegate_task_by_id(tasks[0].id.clone(), "user2".to_string())
        .expect_err("should reject delegating a suspended task");
    assert!(err.to_string().contains("suspended task"));
}

#[test]
fn update_suspended_task_allowed() {
    let engine = ProcessEngine::new("update-suspended-allowed".to_string());
    let runtime = engine.get_runtime_service();
    let task_service = engine.get_task_service();

    let (pi_id, _) = deploy_and_start(&engine, "1");
    runtime
        .suspend_process_instance(pi_id.clone(), ProcessInstanceUpdate::default())
        .unwrap();

    let tasks = task_service
        .get_tasks_by_process_instance_id(pi_id)
        .unwrap();

    // Java parity: REST PUT /runtime/tasks/{id} uses `taskService.saveTask()`
    // which does NOT check suspension. Updating a suspended task succeeds.
    let updated = task_service
        .update_task_by_id(
            tasks[0].id.clone(),
            TaskUpdate {
                name: Some("New Name".to_string()),
                ..TaskUpdate::default()
            },
        )
        .expect("Java saveTask allows updating a suspended task");
    assert_eq!(updated.name, "New Name");
}

#[test]
fn set_variable_on_suspended_task_rejected() {
    let engine = ProcessEngine::new("set-var-suspended-rejected".to_string());
    let runtime = engine.get_runtime_service();
    let task_service = engine.get_task_service();

    let (pi_id, _) = deploy_and_start(&engine, "1");
    runtime
        .suspend_process_instance(pi_id.clone(), ProcessInstanceUpdate::default())
        .unwrap();

    let tasks = task_service
        .get_tasks_by_process_instance_id(pi_id)
        .unwrap();

    let err = task_service
        .set_task_local_variable(tasks[0].id.clone(), "myvar".to_string(), json!("value"))
        .expect_err("should reject setting variable on suspended task");
    assert!(err.to_string().contains("suspended task"));
}

#[test]
fn resolve_suspended_task_rejected() {
    let engine = ProcessEngine::new("resolve-suspended-rejected".to_string());
    let runtime = engine.get_runtime_service();
    let task_service = engine.get_task_service();

    let (pi_id, _) = deploy_and_start(&engine, "1");
    // First claim to set up delegation
    let tasks = task_service
        .get_tasks_by_process_instance_id(pi_id.clone())
        .unwrap();
    task_service
        .claim_task_by_id(tasks[0].id.clone(), "owner1".to_string())
        .unwrap();
    task_service
        .delegate_task_by_id(tasks[0].id.clone(), "delegate1".to_string())
        .unwrap();

    // Now suspend
    runtime
        .suspend_process_instance(pi_id.clone(), ProcessInstanceUpdate::default())
        .unwrap();

    let err = task_service
        .resolve_task_by_id(tasks[0].id.clone())
        .expect_err("should reject resolving a suspended task");
    assert!(err.to_string().contains("suspended task"));
}

#[test]
fn activate_preserves_task_data() {
    let engine = ProcessEngine::new("activate-preserves-data".to_string());
    let runtime = engine.get_runtime_service();
    let task_service = engine.get_task_service();

    // Deploy a process with a user task that has an assignee
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
                 xmlns:flowable="http://flowable.org/bpmn"
                 targetNamespace="Examples">
        <process id="preserveDataProcess">
            <startEvent id="start" />
            <sequenceFlow id="f1" sourceRef="start" targetRef="task1" />
            <userTask id="task1" name="Preserve Me" flowable:assignee="originalUser" />
            <sequenceFlow id="f2" sourceRef="task1" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#;

    let repo = engine.get_repository_service();
    repo.deploy(
        repo.create_deployment()
            .add_string("preserveData.bpmn20.xml".to_string(), xml.to_string()),
    )
    .unwrap();

    let def_id = repo.get_process_definition_ids().unwrap()[0].clone();
    let pi = runtime
        .start_process_instance(
            runtime
                .create_process_instance_builder()
                .process_definition_id(def_id),
        )
        .unwrap();

    // Set a local variable
    let tasks_before = task_service
        .get_tasks_by_process_instance_id(pi.id.clone())
        .unwrap();
    let task_id = tasks_before[0].id.clone();
    task_service
        .set_task_local_variable(task_id.clone(), "key1".to_string(), json!("val1"))
        .unwrap();

    // Suspend
    runtime
        .suspend_process_instance(pi.id.clone(), ProcessInstanceUpdate::default())
        .unwrap();

    // Activate
    runtime
        .activate_process_instance(pi.id.clone(), ProcessInstanceUpdate::default())
        .unwrap();

    // Verify task data is preserved
    let tasks_after = task_service
        .get_tasks_by_process_instance_id(pi.id.clone())
        .unwrap();
    assert_eq!(tasks_after.len(), 1);
    let task = &tasks_after[0];

    // Fields preserved
    assert_eq!(task.name, "Preserve Me");
    assert_eq!(task.assignee.as_deref(), Some("originalUser"));
    assert_eq!(task.suspension_state, 0); // active
    assert!(!task.is_suspended());

    // Verify local variable preserved
    let var = task_service
        .get_task_local_variable(task_id.clone(), "key1".to_string())
        .unwrap();
    assert_eq!(var, Some(json!("val1")));
}

#[test]
fn add_candidate_user_to_suspended_task_rejected() {
    let engine = ProcessEngine::new("candidate-user-suspended-rejected".to_string());
    let runtime = engine.get_runtime_service();
    let task_service = engine.get_task_service();

    let (pi_id, _) = deploy_and_start(&engine, "1");
    runtime
        .suspend_process_instance(pi_id.clone(), ProcessInstanceUpdate::default())
        .unwrap();

    let tasks = task_service
        .get_tasks_by_process_instance_id(pi_id)
        .unwrap();

    let err = task_service
        .add_candidate_user(tasks[0].id.clone(), "candidate1".to_string())
        .expect_err("should reject adding candidate user to suspended task");
    assert!(err.to_string().contains("suspended task"));
}

#[test]
fn add_candidate_group_to_suspended_task_rejected() {
    let engine = ProcessEngine::new("candidate-group-suspended-rejected".to_string());
    let runtime = engine.get_runtime_service();
    let task_service = engine.get_task_service();

    let (pi_id, _) = deploy_and_start(&engine, "1");
    runtime
        .suspend_process_instance(pi_id.clone(), ProcessInstanceUpdate::default())
        .unwrap();

    let tasks = task_service
        .get_tasks_by_process_instance_id(pi_id)
        .unwrap();

    let err = task_service
        .add_candidate_group(tasks[0].id.clone(), "group1".to_string())
        .expect_err("should reject adding candidate group to suspended task");
    assert!(err.to_string().contains("suspended task"));
}

#[test]
fn delete_candidate_user_from_suspended_task_rejected() {
    let engine = ProcessEngine::new("delete-candidate-user-suspended-rejected".to_string());
    let runtime = engine.get_runtime_service();
    let task_service = engine.get_task_service();

    let (pi_id, _) = deploy_and_start(&engine, "1");
    let task = task_service
        .get_tasks_by_process_instance_id(pi_id.clone())
        .unwrap()
        .remove(0);
    task_service
        .add_candidate_user(task.id.clone(), "candidate1".to_string())
        .unwrap();
    runtime
        .suspend_process_instance(pi_id, ProcessInstanceUpdate::default())
        .unwrap();

    let error = task_service
        .delete_candidate_user(task.id, "candidate1".to_string())
        .expect_err("Java DeleteIdentityLinkCmd extends NeedsActiveTaskCmd");
    assert!(error.to_string().contains("suspended task"));
}
