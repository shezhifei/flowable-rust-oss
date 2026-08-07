use flowable_engine::persistence::db_store::DbStore;
use flowable_engine::persistence::entity_manager::EntityManager;
use flowable_engine::persistence::execution_entity_manager::{
    DefaultExecutionEntityManager, ExecutionEntityManager,
};
use flowable_engine::persistence::runtime_store::RuntimeStore;
use flowable_engine::runtime::execution::Execution;
use flowable_engine::runtime::process_instance::ProcessInstance;
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;

#[test]
fn test_execution_scope_tree() {
    let db_store = Arc::new(DbStore::new_in_memory().unwrap());
    let runtime_store = RuntimeStore::new_with_memory_backend_for_test(db_store);
    let mut session = runtime_store.create_session().unwrap();

    // 1. Create process instance
    let pi = ProcessInstance {
        id: "pi-1".to_string(),
        name: None,
        process_definition_id: "pd-1".to_string(),
        process_definition_key: "my-process".to_string(),
        process_definition_name: None,
        process_definition_version: 1,
        business_key: None,
        business_status: None,
        is_suspended: false,
        tenant_id: None,
        start_time: None,
        start_user_id: None,
        callback_id: None,
        callback_type: None,
        reference_id: None,
        reference_type: None,
        is_ended: false,
        super_execution_id: None,
        root_process_instance_id: Some("pi-1".to_string()),
    };
    runtime_store.insert_process_instance(&pi, &mut session);

    let mut em = DefaultExecutionEntityManager::new(runtime_store.clone());

    // 2. Create parent execution
    let mut parent_exec = Execution {
        id: "exec-parent".to_string(),
        parent_id: None,
        super_execution_id: None,
        root_process_instance_id: Some("pi-1".to_string()),
        process_instance_id: Some("pi-1".to_string()),
        process_definition_id: Some("pd-1".to_string()),
        process_definition_key: Some("my-process".to_string()),
        process_definition_name: None,
        process_definition_version: Some(1),
        activity_id: Some("subProcess1".to_string()),
        activity_name: None,
        name: None,
        description: None,
        is_suspended: false,
        is_ended: false,
        is_active: true,
        is_concurrent: false,
        is_scope: true,
        is_multi_instance_root: false,
        tenant_id: None,
        ..Default::default()
    };

    let mut parent_vars = HashMap::new();
    parent_vars.insert("globalVar".to_string(), json!("hello"));
    parent_exec.set_process_variables(parent_vars);

    em.insert(&parent_exec, &mut session);

    // 3. Create child execution 1
    let mut child_exec1 = Execution {
        id: "exec-child-1".to_string(),
        parent_id: Some("exec-parent".to_string()),
        super_execution_id: None,
        root_process_instance_id: Some("pi-1".to_string()),
        process_instance_id: Some("pi-1".to_string()),
        process_definition_id: Some("pd-1".to_string()),
        process_definition_key: Some("my-process".to_string()),
        process_definition_name: None,
        process_definition_version: Some(1),
        activity_id: Some("task1".to_string()),
        activity_name: None,
        name: None,
        description: None,
        is_suspended: false,
        is_ended: false,
        is_active: true,
        is_concurrent: true,
        is_scope: false,
        is_multi_instance_root: false,
        tenant_id: None,
        ..Default::default()
    };
    let mut child1_vars = HashMap::new();
    child1_vars.insert("localVar1".to_string(), json!("val1"));
    child_exec1.set_process_variables(child1_vars);
    em.insert(&child_exec1, &mut session);

    // 4. Create child execution 2
    let mut child_exec2 = Execution {
        id: "exec-child-2".to_string(),
        parent_id: Some("exec-parent".to_string()),
        super_execution_id: None,
        root_process_instance_id: Some("pi-1".to_string()),
        process_instance_id: Some("pi-1".to_string()),
        process_definition_id: Some("pd-1".to_string()),
        process_definition_key: Some("my-process".to_string()),
        process_definition_name: None,
        process_definition_version: Some(1),
        activity_id: Some("task2".to_string()),
        activity_name: None,
        name: None,
        description: None,
        is_suspended: false,
        is_ended: false,
        is_active: true,
        is_concurrent: true,
        is_scope: false,
        is_multi_instance_root: false,
        tenant_id: None,
        ..Default::default()
    };
    let mut child2_vars = HashMap::new();
    child2_vars.insert("localVar2".to_string(), json!("val2"));
    child_exec2.set_process_variables(child2_vars);
    em.insert(&child_exec2, &mut session);

    // Assertions
    // Preserving parent_id, process_instance_id, root_process_instance_id
    let retrieved_child = em.find_by_id("exec-child-1", &mut session).unwrap();
    assert_eq!(retrieved_child.parent_id.as_deref(), Some("exec-parent"));
    assert_eq!(retrieved_child.process_instance_id.as_deref(), Some("pi-1"));
    assert_eq!(
        retrieved_child.root_process_instance_id.as_deref(),
        Some("pi-1")
    );

    // Recovering parent/child execution relationships from persisted runtime state
    let children = em.find_child_executions_by_parent_execution_id("exec-parent", &mut session);
    assert_eq!(children.len(), 2);
    let child_ids: Vec<&str> = children.iter().map(|e| e.id.as_str()).collect();
    assert!(child_ids.contains(&"exec-child-1"));
    assert!(child_ids.contains(&"exec-child-2"));

    // Scoped variables not leaking automatically across sibling executions
    let c1_var = em
        .find_by_id("exec-child-1", &mut session)
        .unwrap()
        .process_variable("localVar1");
    assert_eq!(c1_var, Some(json!("val1")));

    let c1_var2 = em
        .find_by_id("exec-child-1", &mut session)
        .unwrap()
        .process_variable("localVar2");
    assert_eq!(c1_var2, None); // Should not leak from child 2

    let c2_var1 = em
        .find_by_id("exec-child-2", &mut session)
        .unwrap()
        .process_variable("localVar1");
    assert_eq!(c2_var1, None); // Should not leak from child 1
}
