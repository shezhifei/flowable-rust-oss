use flowable_engine::engine::process_engine::ProcessEngine;

#[test]
fn deployment_resources_are_queryable_after_deploy() {
    let process_engine = ProcessEngine::new("default".to_string());
    let repository_service = process_engine.get_repository_service();

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="resourceProcess" name="Resource Process">
            <startEvent id="startEvent1" />
            <endEvent id="endEvent1" />
        </process>
    </definitions>"#;

    let deployment = repository_service
        .create_deployment()
        .name("Resource Deployment".to_string())
        .add_string("resource-process.bpmn20.xml".to_string(), xml.to_string());

    let deployment = repository_service.deploy(deployment).unwrap();
    let resource_names = repository_service.get_deployment_resource_names(&deployment.id);

    assert_eq!(
        resource_names.unwrap(),
        vec!["resource-process.bpmn20.xml".to_string()]
    );
}

#[test]
fn deleting_deployment_removes_process_definitions() {
    let process_engine = ProcessEngine::new("default".to_string());
    let repository_service = process_engine.get_repository_service();

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="deletableProcess" name="Deletable Process">
            <startEvent id="startEvent1" />
            <endEvent id="endEvent1" />
        </process>
    </definitions>"#;

    let deployment = repository_service
        .create_deployment()
        .name("Deletable Deployment".to_string())
        .add_string("deletable-process.bpmn20.xml".to_string(), xml.to_string());

    let deployment = repository_service.deploy(deployment).unwrap();
    let process_definition_ids = repository_service.get_process_definition_ids().unwrap();

    assert_eq!(process_definition_ids.len(), 1);
    assert_eq!(
        repository_service
            .get_deployment_resource_names(&deployment.id)
            .unwrap(),
        vec!["deletable-process.bpmn20.xml".to_string()]
    );

    repository_service
        .delete_deployment(&deployment.id)
        .unwrap();

    assert!(
        repository_service
            .get_process_definition_ids()
            .unwrap()
            .is_empty()
    );
    assert!(
        repository_service
            .get_deployment_resource_names(&deployment.id)
            .unwrap()
            .is_empty()
    );
}
