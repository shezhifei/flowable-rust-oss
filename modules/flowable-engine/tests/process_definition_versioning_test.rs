use flowable_engine::engine::process_engine::ProcessEngine;

fn extract_process_definition_version(process_definition_id: &str) -> i32 {
    process_definition_id
        .split(':')
        .nth(1)
        .expect("process definition id should contain a version segment")
        .parse()
        .expect("version segment should be an integer")
}

#[test]
fn repeated_deployment_increments_process_definition_version() {
    let process_engine = ProcessEngine::new("default".to_string());
    let repository_service = process_engine.get_repository_service();

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="versionedProcess" name="Versioned Process">
            <startEvent id="startEvent1" />
            <endEvent id="endEvent1" />
        </process>
    </definitions>"#;

    let deployment_builder = repository_service
        .create_deployment()
        .name("Versioned Deployment".to_string())
        .add_string("versioned-process.bpmn20.xml".to_string(), xml.to_string());

    repository_service
        .deploy(deployment_builder.clone())
        .unwrap();
    repository_service.deploy(deployment_builder).unwrap();

    let mut versions: Vec<i32> = repository_service
        .get_process_definition_ids()
        .unwrap()
        .iter()
        .map(|id| extract_process_definition_version(id.as_str()))
        .collect();
    versions.sort_unstable();

    assert_eq!(versions, vec![1, 2]);
}
