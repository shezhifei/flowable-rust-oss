use flowable_engine::engine::process_engine::ProcessEngine;

fn process_xml(process_name: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
  <process id="p59DuplicateProcess" name="{process_name}">
    <startEvent id="start" />
    <sequenceFlow id="toEnd" sourceRef="start" targetRef="end" />
    <endEvent id="end" />
  </process>
</definitions>"#
    )
}

fn deploy(
    engine: &ProcessEngine,
    xml: &str,
    tenant_id: Option<&str>,
    duplicate_filtering: bool,
) -> flowable_engine::repository::deployment::Deployment {
    let repository = engine.get_repository_service();
    let mut builder = repository
        .create_deployment()
        .name("P59 duplicate deployment".to_string())
        .add_string("p59-duplicate.bpmn20.xml".to_string(), xml.to_string());
    if let Some(tenant_id) = tenant_id {
        builder = builder.tenant_id(tenant_id.to_string());
    }
    if duplicate_filtering {
        builder = builder.enable_duplicate_filtering();
    }
    repository.deploy(builder).unwrap()
}

#[test]
fn identical_latest_deployment_is_reused_without_bumping_version() {
    let engine = ProcessEngine::new("default".to_string());
    let xml = process_xml("first");

    let first = deploy(&engine, &xml, None, true);
    let duplicate = deploy(&engine, &xml, None, true);

    assert_eq!(duplicate.id, first.id);
    assert!(!duplicate.is_new);
    let deployments = engine.get_repository_service().get_deployments().unwrap();
    assert_eq!(deployments.len(), 1);
    let definitions = engine
        .get_repository_service()
        .get_process_definitions()
        .unwrap();
    assert_eq!(definitions.len(), 1);
    assert_eq!(definitions[0].version, 1);
}

#[test]
fn changed_latest_resource_creates_new_version_and_then_becomes_reusable() {
    let engine = ProcessEngine::new("default".to_string());
    let first = deploy(&engine, &process_xml("first"), None, true);
    let changed_xml = process_xml("changed");

    let changed = deploy(&engine, &changed_xml, None, true);
    let duplicate_changed = deploy(&engine, &changed_xml, None, true);

    assert_ne!(changed.id, first.id);
    assert_eq!(duplicate_changed.id, changed.id);
    let mut versions = engine
        .get_repository_service()
        .get_process_definitions()
        .unwrap()
        .into_iter()
        .map(|definition| definition.version)
        .collect::<Vec<_>>();
    versions.sort_unstable();
    assert_eq!(versions, vec![1, 2]);
}

#[test]
fn duplicate_filtering_is_tenant_aware_and_opt_in() {
    let engine = ProcessEngine::new("default".to_string());
    let xml = process_xml("tenant-aware");

    let tenant_a = deploy(&engine, &xml, Some("tenant-a"), true);
    let tenant_b = deploy(&engine, &xml, Some("tenant-b"), true);
    let unfiltered_first = deploy(&engine, &xml, None, false);
    let unfiltered_second = deploy(&engine, &xml, None, false);

    assert_ne!(tenant_a.id, tenant_b.id);
    assert_ne!(unfiltered_first.id, unfiltered_second.id);
    assert_eq!(
        engine
            .get_repository_service()
            .get_deployments()
            .unwrap()
            .len(),
        4
    );
}
