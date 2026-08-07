use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::engine::query::Query;
use flowable_engine::identity::entities::IdentityLink;

fn deploy_simple_process(engine: &ProcessEngine) -> String {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="starterLinkProcess" name="Starter Link Process">
            <startEvent id="startEvent1" />
            <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="userTask1" />
            <userTask id="userTask1" name="User Task" />
            <sequenceFlow id="flow2" sourceRef="userTask1" targetRef="endEvent1" />
            <endEvent id="endEvent1" />
        </process>
    </definitions>"#;

    let repository_service = engine.get_repository_service();
    repository_service
        .deploy(
            repository_service
                .create_deployment()
                .name("starter-link-deploy".to_string())
                .add_string("process.bpmn20.xml".to_string(), xml.to_string()),
        )
        .unwrap();
    repository_service.get_process_definition_ids().unwrap()[0].clone()
}

#[test]
fn start_with_user_creates_starter_identity_link_and_records_start_user() {
    let engine = ProcessEngine::new("starter-link-with-user".to_string());
    let process_definition_id = deploy_simple_process(&engine);

    let instance = engine
        .get_runtime_service()
        .start_process_instance(
            engine
                .get_runtime_service()
                .create_process_instance_builder()
                .process_definition_id(process_definition_id)
                .start_user_id("admin".to_string()),
        )
        .unwrap();

    assert_eq!(instance.start_user_id.as_deref(), Some("admin"));

    let links = engine
        .get_identity_link_service()
        .create_identity_link_query()
        .process_instance_id(instance.id.clone())
        .list()
        .unwrap();
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].link_type, "starter");
    assert_eq!(links[0].user_id.as_deref(), Some("admin"));
    assert_eq!(links[0].group_id, None);
    assert_eq!(links[0].task_id, None);

    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let historic = store
        .get_historic_process_instance(&instance.id, &mut session)
        .expect("historic process instance should exist");
    assert_eq!(historic.start_user_id.as_deref(), Some("admin"));
    assert_eq!(
        store.find_process_instance_ids_by_involved_user("admin", &mut session),
        vec![instance.id]
    );
    session.rollback().unwrap();
}

#[test]
fn start_without_user_creates_no_starter_identity_link() {
    let engine = ProcessEngine::new("starter-link-without-user".to_string());
    let process_definition_id = deploy_simple_process(&engine);

    let instance = engine
        .get_runtime_service()
        .start_process_instance(
            engine
                .get_runtime_service()
                .create_process_instance_builder()
                .process_definition_id(process_definition_id),
        )
        .unwrap();

    assert_eq!(instance.start_user_id, None);
    assert!(
        engine
            .get_identity_link_service()
            .create_identity_link_query()
            .process_instance_id(instance.id.clone())
            .list()
            .unwrap()
            .is_empty()
    );

    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let historic = store
        .get_historic_process_instance(&instance.id, &mut session)
        .expect("historic process instance should exist");
    assert_eq!(historic.start_user_id, None);
    assert!(
        store
            .find_process_instance_ids_by_involved_user("admin", &mut session)
            .is_empty()
    );
    session.rollback().unwrap();
}

#[test]
fn async_start_with_user_creates_starter_identity_link() {
    let engine = ProcessEngine::new("starter-link-async-start".to_string());
    let process_definition_id = deploy_simple_process(&engine);

    let instance = engine
        .get_runtime_service()
        .start_process_instance_async(
            engine
                .get_runtime_service()
                .create_process_instance_builder()
                .process_definition_id(process_definition_id)
                .start_user_id("admin".to_string()),
        )
        .unwrap();

    assert_eq!(instance.start_user_id.as_deref(), Some("admin"));
    let links = engine
        .get_identity_link_service()
        .create_identity_link_query()
        .process_instance_id(instance.id.clone())
        .list()
        .unwrap();
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].link_type, "starter");
    assert_eq!(links[0].user_id.as_deref(), Some("admin"));

    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let historic = store
        .get_historic_process_instance(&instance.id, &mut session)
        .expect("historic process instance should exist");
    assert_eq!(historic.start_user_id.as_deref(), Some("admin"));
    session.rollback().unwrap();
}

#[test]
fn involved_user_matches_any_process_instance_link_type_and_deduplicates() {
    let engine = ProcessEngine::new("involved-user-link-matching".to_string());
    let identity_link_service = engine.get_identity_link_service();

    for (id, link_type, user_id, process_instance_id, task_id) in [
        ("participant", "participant", "kermit", Some("proc-1"), None),
        ("starter", "starter", "kermit", Some("proc-2"), None),
        ("duplicate", "custom", "kermit", Some("proc-2"), None),
        ("task", "candidate", "kermit", None, Some("task-1")),
        ("other", "participant", "fozzie", Some("proc-3"), None),
    ] {
        identity_link_service.add_identity_link(IdentityLink {
            id: id.to_string(),
            link_type: link_type.to_string(),
            user_id: Some(user_id.to_string()),
            group_id: None,
            task_id: task_id.map(str::to_string),
            process_instance_id: process_instance_id.map(str::to_string),
            process_definition_id: None,
        });
    }

    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    assert_eq!(
        store.find_process_instance_ids_by_involved_user("kermit", &mut session),
        vec!["proc-1".to_string(), "proc-2".to_string()]
    );
    assert_eq!(
        store.find_process_instance_ids_by_involved_user("fozzie", &mut session),
        vec!["proc-3".to_string()]
    );
    assert!(
        store
            .find_process_instance_ids_by_involved_user("nobody", &mut session)
            .is_empty()
    );
    session.rollback().unwrap();
}

#[test]
fn historic_process_instance_query_filters_by_involved_user() {
    let engine = ProcessEngine::new("historic-involved-user".to_string());
    let process_definition_id = deploy_simple_process(&engine);

    let with_user = engine
        .get_runtime_service()
        .start_process_instance(
            engine
                .get_runtime_service()
                .create_process_instance_builder()
                .process_definition_id(process_definition_id.clone())
                .start_user_id("admin".to_string()),
        )
        .unwrap();
    let without_user = engine
        .get_runtime_service()
        .start_process_instance(
            engine
                .get_runtime_service()
                .create_process_instance_builder()
                .process_definition_id(process_definition_id),
        )
        .unwrap();

    let history_service = engine.get_history_service();
    let involved: Vec<String> = history_service
        .create_historic_process_instance_query()
        .involved_user("admin".to_string())
        .list()
        .unwrap()
        .into_iter()
        .map(|instance| instance.id)
        .collect();
    assert_eq!(involved, vec![with_user.id.clone()]);
    assert!(
        history_service
            .create_historic_process_instance_query()
            .involved_user("someone-else".to_string())
            .list()
            .unwrap()
            .is_empty()
    );

    let all_ids: Vec<String> = history_service
        .create_historic_process_instance_query()
        .list()
        .unwrap()
        .into_iter()
        .map(|instance| instance.id)
        .collect();
    assert!(all_ids.contains(&with_user.id));
    assert!(all_ids.contains(&without_user.id));
}
