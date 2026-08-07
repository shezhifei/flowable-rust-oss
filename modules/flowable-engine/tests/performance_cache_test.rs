use flowable_engine::engine::bpmn_model_cache::BpmnModelCache;
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::engine::time_source::SystemTimeSource;
use flowable_engine::persistence::db_store::DbStore;
use std::sync::Arc;

const SIMPLE_BPMN: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
  <process id="cacheProcess" name="Cache Process" isExecutable="true">
    <startEvent id="start"/>
    <sequenceFlow id="flow1" sourceRef="start" targetRef="task1"/>
    <userTask id="task1" name="Task"/>
    <sequenceFlow id="flow2" sourceRef="task1" targetRef="end"/>
    <endEvent id="end"/>
  </process>
</definitions>"#;

#[test]
fn bpmn_model_cache_returns_same_arc_on_second_lookup() {
    let cache = BpmnModelCache::new();
    let bytes = SIMPLE_BPMN.as_bytes();

    assert!(cache.is_empty());
    let first = cache
        .get_or_parse("dep-1", "process.bpmn20.xml", bytes)
        .expect("first parse");
    assert_eq!(cache.len(), 1);
    assert!(cache.contains_key("dep-1", "process.bpmn20.xml"));

    let second = cache
        .get_or_parse("dep-1", "process.bpmn20.xml", bytes)
        .expect("cache hit");
    assert_eq!(cache.len(), 1);
    assert!(Arc::ptr_eq(&first, &second));
}

#[test]
fn bpmn_model_cache_invalidate_removes_deployment_entries() {
    let cache = BpmnModelCache::new();
    let bytes = SIMPLE_BPMN.as_bytes();
    cache
        .get_or_parse("dep-a", "a.bpmn20.xml", bytes)
        .expect("parse a");
    cache
        .get_or_parse("dep-b", "b.bpmn20.xml", bytes)
        .expect("parse b");
    assert_eq!(cache.len(), 2);

    cache.invalidate("dep-a");
    assert!(!cache.contains_key("dep-a", "a.bpmn20.xml"));
    assert!(cache.contains_key("dep-b", "b.bpmn20.xml"));
    assert_eq!(cache.len(), 1);
}

#[test]
fn deployment_manager_caches_bpmn_model_across_queries() {
    let engine = ProcessEngine::new("perf-cache-deploy".to_string());
    let repo = engine.get_repository_service();
    let builder = repo
        .create_deployment()
        .name("cache-deploy".to_string())
        .add_string("process.bpmn20.xml".to_string(), SIMPLE_BPMN.to_string());
    repo.deploy(builder).expect("deploy");

    let pd_ids = repo.get_process_definition_ids().expect("pd ids");
    assert_eq!(pd_ids.len(), 1);
    let pd_id = &pd_ids[0];

    let executor = engine.get_command_executor();
    let dm = executor.deployment_manager();
    assert!(
        dm.contains_bpmn_model(pd_id),
        "process definition model should be cached after deploy"
    );
    let first = dm.get_bpmn_model(pd_id).expect("first model");
    let second = dm.get_bpmn_model(pd_id).expect("second model");
    assert!(Arc::ptr_eq(&first, &second));
}

#[test]
fn deployment_resource_bytes_are_cached_across_reads() {
    let engine = ProcessEngine::new("perf-cache-resource".to_string());
    let repo = engine.get_repository_service();
    let builder = repo
        .create_deployment()
        .name("resource-cache".to_string())
        .add_string("process.bpmn20.xml".to_string(), SIMPLE_BPMN.to_string());
    let deployment = repo.deploy(builder).expect("deploy");

    let executor = engine.get_command_executor();
    let dm = executor.deployment_manager();
    let mut session = dm.create_session().expect("session");
    let first = dm
        .get_deployment_resource_bytes(&deployment.id, "process.bpmn20.xml", &mut session)
        .expect("first bytes");
    let second = dm
        .get_deployment_resource_bytes(&deployment.id, "process.bpmn20.xml", &mut session)
        .expect("cached bytes");
    session.rollback().ok();
    assert_eq!(first, second);
    assert_eq!(first, SIMPLE_BPMN.as_bytes());
}

#[test]
fn process_engine_rebuilds_model_cache_after_deploy() {
    let db = Arc::new(DbStore::new_in_memory().expect("db"));
    let engine = ProcessEngine::build(
        "perf-cache-rebuild".to_string(),
        Arc::new(SystemTimeSource),
        db,
    );
    let repo = engine.get_repository_service();
    let builder = repo
        .create_deployment()
        .name("rebuild".to_string())
        .add_string("process.bpmn20.xml".to_string(), SIMPLE_BPMN.to_string());
    repo.deploy(builder).expect("deploy");

    let pd_id = &repo.get_process_definition_ids().expect("ids")[0];
    let executor = engine.get_command_executor();
    let model = executor
        .deployment_manager()
        .get_bpmn_model(pd_id)
        .expect("cached model after deploy");

    let has_process = model
        .processes
        .iter()
        .any(|p| p.base_element.id.as_deref() == Some("cacheProcess"))
        || model
            .main_process
            .as_ref()
            .is_some_and(|p| p.base_element.id.as_deref() == Some("cacheProcess"));
    assert!(has_process, "cached model should contain cacheProcess");
}
