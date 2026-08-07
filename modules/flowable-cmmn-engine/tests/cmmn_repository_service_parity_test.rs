use flowable_cmmn_engine::{
    CaseDefinitionSortField, CmmnCase, CmmnCasePlanModel, CmmnDecisionResolver, CmmnDecisionTask,
    CmmnDeploymentRequest, CmmnEngine, CmmnFormResolver, CmmnHumanTask, CmmnModel, CmmnPlanItem,
    DeploymentSortField, ReferencedDecision, ReferencedFormDefinition, SortDirection,
};
use std::io::Write;
use uuid::Uuid;

fn model() -> CmmnModel {
    CmmnModel::new(vec![CmmnCase::new(
        "repository-parity-case",
        "repository-parity-key",
        "Repository parity case",
        CmmnCasePlanModel::new("repository-parity-plan", "Repository parity plan"),
    )])
}

#[test]
fn model_request_defaults_match_repository_contract() {
    let request = CmmnDeploymentRequest::new("deployment");

    assert_eq!(request.name.as_deref(), Some("deployment"));
    assert_eq!(request.category, None);
    assert_eq!(request.key, None);
    assert_eq!(request.tenant_id, None);
    assert_eq!(request.parent_deployment_id, None);
    assert!(!request.enable_duplicate_filtering);
    assert!(request.validate_schema);
}

#[test]
fn metadata_round_trips_through_deployment_and_definition_models() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let deployment = engine
        .deploy(
            CmmnDeploymentRequest::new("repository metadata")
                .with_category("category-a")
                .with_key("deployment-key-a")
                .with_tenant_id("tenant-a")
                .with_parent_deployment_id("parent-a")
                .enable_duplicate_filtering()
                .disable_schema_validation()
                .with_resource("case.cmmn", model()),
        )
        .expect("deployment should succeed");

    assert_eq!(deployment.name.as_deref(), Some("repository metadata"));
    assert_eq!(deployment.category.as_deref(), Some("category-a"));
    assert_eq!(deployment.key.as_deref(), Some("deployment-key-a"));
    assert_eq!(deployment.tenant_id.as_deref(), Some("tenant-a"));
    assert_eq!(deployment.parent_deployment_id.as_deref(), Some("parent-a"));

    let definition = engine
        .repository_service()
        .create_case_definition_query()
        .key("repository-parity-key")
        .single_result()
        .expect("definition query should succeed")
        .expect("definition should exist");
    assert_eq!(definition.category.as_deref(), Some("category-a"));
    assert_eq!(definition.tenant_id.as_deref(), Some("tenant-a"));
    assert_eq!(definition.diagram_resource_name, None);
}

#[test]
fn metadata_round_trips_after_sqlite_restart() {
    let path =
        std::env::temp_dir().join(format!("flowable-cmmn-metadata-{}.sqlite", Uuid::new_v4()));
    let deployment_id;
    let definition_id;

    {
        let engine = CmmnEngine::new_sqlite(&path).expect("engine");
        let deployment = engine
            .deploy(
                CmmnDeploymentRequest::new("repository metadata")
                    .with_category("category-a")
                    .with_key("deployment-key-a")
                    .with_tenant_id("tenant-a")
                    .with_parent_deployment_id("parent-a")
                    .with_resource("case.cmmn", model()),
            )
            .expect("deployment should succeed");
        deployment_id = deployment.id;
        definition_id = engine
            .repository_service()
            .create_case_definition_query()
            .key("repository-parity-key")
            .single_result()
            .expect("definition query should succeed")
            .expect("definition should exist")
            .id;
    }

    let engine = CmmnEngine::new_sqlite(&path).expect("reopened engine");
    let deployment = engine
        .repository_service()
        .get_deployment(&deployment_id)
        .expect("deployment should exist after restart");
    let definition = engine
        .repository_service()
        .get_case_definition(&definition_id)
        .expect("definition should exist after restart");

    assert_eq!(deployment.category.as_deref(), Some("category-a"));
    assert_eq!(deployment.key.as_deref(), Some("deployment-key-a"));
    assert_eq!(deployment.tenant_id.as_deref(), Some("tenant-a"));
    assert_eq!(deployment.parent_deployment_id.as_deref(), Some("parent-a"));
    assert_eq!(definition.category.as_deref(), Some("category-a"));
    assert_eq!(definition.tenant_id.as_deref(), Some("tenant-a"));

    drop(engine);
    std::fs::remove_file(path).expect("temporary database should be removable");
}

#[test]
fn deployment_builder_accepts_raw_cmmn_and_attachments() {
    let xml = r#"<definitions xmlns="http://www.omg.org/spec/CMMN/20151109/MODEL"><case id="case" name="Case"><casePlanModel id="plan" name="Plan"/></case></definitions>"#;
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let deployment = engine
        .repository_service()
        .new_deployment()
        .category("category")
        .add_string("case.CMMN.XML", xml)
        .expect("CMMN resource")
        .add_bytes("diagram.png", [1, 2, 3])
        .expect("attachment")
        .deploy()
        .expect("deployment");

    assert_eq!(deployment.name, None);
    assert_eq!(
        engine
            .repository_service()
            .get_deployment_resource_bytes(&deployment.id, "diagram.png")
            .expect("attachment"),
        vec![1, 2, 3]
    );
}

#[test]
fn deployment_builder_extracts_zip_resources() {
    let xml = r#"<definitions xmlns="http://www.omg.org/spec/CMMN/20151109/MODEL"><case id="zip-case" name="Zip"><casePlanModel id="zip-plan" name="Plan"/></case></definitions>"#;
    let cursor = std::io::Cursor::new(Vec::new());
    let mut zip = zip::ZipWriter::new(cursor);
    zip.start_file("case.cmmn", zip::write::SimpleFileOptions::default())
        .expect("CMMN zip entry");
    zip.write_all(xml.as_bytes()).expect("CMMN bytes");
    zip.start_file("note.txt", zip::write::SimpleFileOptions::default())
        .expect("attachment zip entry");
    zip.write_all(b"attachment").expect("attachment bytes");
    let bytes = zip.finish().expect("zip bytes").into_inner();

    let engine = CmmnEngine::new_in_memory().expect("engine");
    let deployment = engine
        .repository_service()
        .new_deployment()
        .add_zip(bytes)
        .expect("ZIP resources")
        .deploy()
        .expect("deployment");
    assert_eq!(
        engine
            .repository_service()
            .get_deployment_resource_bytes(&deployment.id, "note.txt")
            .expect("attachment"),
        b"attachment"
    );
}

#[test]
fn duplicate_filtering_is_resource_and_tenant_aware() {
    let xml = r#"<definitions xmlns="http://www.omg.org/spec/CMMN/20151109/MODEL"><case id="duplicate-case" name="Duplicate"><casePlanModel id="duplicate-plan" name="Plan"/></case></definitions>"#;
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let first = engine
        .repository_service()
        .new_deployment()
        .tenant_id("tenant-a")
        .add_string("case.cmmn", xml)
        .expect("resource")
        .add_bytes("note.txt", b"one")
        .expect("attachment")
        .deploy()
        .expect("first deployment");
    let duplicate = engine
        .repository_service()
        .new_deployment()
        .tenant_id("tenant-a")
        .enable_duplicate_filtering()
        .add_string("case.cmmn", xml)
        .expect("resource")
        .add_bytes("note.txt", b"one")
        .expect("attachment")
        .deploy()
        .expect("duplicate deployment");
    let changed_attachment = engine
        .repository_service()
        .new_deployment()
        .tenant_id("tenant-a")
        .enable_duplicate_filtering()
        .add_string("case.cmmn", xml)
        .expect("resource")
        .add_bytes("note.txt", b"two")
        .expect("attachment")
        .deploy()
        .expect("changed attachment deployment");
    let different_tenant = engine
        .repository_service()
        .new_deployment()
        .tenant_id("tenant-b")
        .enable_duplicate_filtering()
        .add_string("case.cmmn", xml)
        .expect("resource")
        .add_bytes("note.txt", b"one")
        .expect("attachment")
        .deploy()
        .expect("different tenant deployment");

    assert_eq!(duplicate.id, first.id);
    assert_ne!(changed_attachment.id, first.id);
    assert_ne!(different_tenant.id, first.id);
}

#[test]
fn diagram_lookup_uses_java_resource_name_convention() {
    let xml = r#"<definitions xmlns="http://www.omg.org/spec/CMMN/20151109/MODEL"><case id="diagram-case" name="Diagram"><casePlanModel id="diagram-plan" name="Plan"/></case></definitions>"#;
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let deployment = engine
        .repository_service()
        .new_deployment()
        .add_string("case.cmmn", xml)
        .expect("resource")
        .add_bytes("casediagram-case.png", [4, 5, 6])
        .expect("diagram")
        .deploy()
        .expect("deployment");
    let definition = engine
        .repository_service()
        .create_case_definition_query()
        .deployment_id(&deployment.id)
        .single_result()
        .expect("query")
        .expect("definition");

    assert_eq!(
        definition.diagram_resource_name.as_deref(),
        Some("casediagram-case.png")
    );
    assert_eq!(
        engine
            .repository_service()
            .get_case_diagram(&definition.id)
            .expect("diagram lookup")
            .expect("diagram")
            .bytes,
        vec![4, 5, 6]
    );
}

// ---------------------------------------------------------------------------
// Task 11: SQL-backed query contract tests
// ---------------------------------------------------------------------------

fn case_model(case_id: &str, key: &str, name: &str) -> CmmnModel {
    CmmnModel::new(vec![CmmnCase::new(
        case_id,
        key,
        name,
        CmmnCasePlanModel::new(format!("{case_id}-plan"), format!("{name} plan")),
    )])
}

#[allow(clippy::too_many_arguments)]
fn deploy(
    engine: &CmmnEngine,
    name: &str,
    category: Option<&str>,
    key: Option<&str>,
    tenant_id: Option<&str>,
    parent_deployment_id: Option<&str>,
    case_key: &str,
    case_name: &str,
) -> flowable_cmmn_engine::CmmnDeployment {
    let mut req = CmmnDeploymentRequest::new(name)
        .with_resource("case.cmmn", case_model("case-id", case_key, case_name));
    if let Some(cat) = category {
        req = req.with_category(cat);
    }
    if let Some(k) = key {
        req = req.with_key(k);
    }
    if let Some(t) = tenant_id {
        req = req.with_tenant_id(t);
    }
    if let Some(p) = parent_deployment_id {
        req = req.with_parent_deployment_id(p);
    }
    engine.deploy(req).expect("deployment should succeed")
}

#[test]
fn deployment_query_by_name() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    deploy(&engine, "alpha", None, None, None, None, "k1", "Case One");
    deploy(&engine, "beta", None, None, None, None, "k2", "Case Two");

    let result = engine
        .repository_service()
        .create_deployment_query()
        .name("alpha")
        .list()
        .expect("query");
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].name.as_deref(), Some("alpha"));
}

#[test]
fn deployment_query_by_name_like() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    deploy(&engine, "alpha-dep", None, None, None, None, "k1", "C1");
    deploy(&engine, "alpha-other", None, None, None, None, "k2", "C2");
    deploy(&engine, "beta", None, None, None, None, "k3", "C3");

    let result = engine
        .repository_service()
        .create_deployment_query()
        .name_like("alpha%")
        .list()
        .expect("query");
    assert_eq!(result.len(), 2);
}

#[test]
fn deployment_query_by_category_and_category_not_equals() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    deploy(&engine, "a", Some("cat-a"), None, None, None, "k1", "C1");
    deploy(&engine, "b", Some("cat-b"), None, None, None, "k2", "C2");
    deploy(&engine, "c", Some("cat-a"), None, None, None, "k3", "C3");

    let cat_a = engine
        .repository_service()
        .create_deployment_query()
        .category("cat-a")
        .list()
        .expect("query");
    assert_eq!(cat_a.len(), 2);

    let not_cat_a = engine
        .repository_service()
        .create_deployment_query()
        .category_not_equals("cat-a")
        .list()
        .expect("query");
    assert_eq!(not_cat_a.len(), 1);
    assert_eq!(not_cat_a[0].name.as_deref(), Some("b"));
}

#[test]
fn deployment_query_by_key() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    deploy(
        &engine,
        "a",
        None,
        Some("dep-key-1"),
        None,
        None,
        "k1",
        "C1",
    );
    deploy(
        &engine,
        "b",
        None,
        Some("dep-key-2"),
        None,
        None,
        "k2",
        "C2",
    );

    let result = engine
        .repository_service()
        .create_deployment_query()
        .key("dep-key-1")
        .list()
        .expect("query");
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].key.as_deref(), Some("dep-key-1"));
}

#[test]
fn deployment_query_by_tenant_id_and_like() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    deploy(&engine, "a", None, None, Some("tenant-a"), None, "k1", "C1");
    deploy(&engine, "b", None, None, Some("tenant-b"), None, "k2", "C2");
    deploy(&engine, "c", None, None, None, None, "k3", "C3");

    let by_tenant = engine
        .repository_service()
        .create_deployment_query()
        .tenant_id("tenant-a")
        .list()
        .expect("query");
    assert_eq!(by_tenant.len(), 1);

    let like_tenant = engine
        .repository_service()
        .create_deployment_query()
        .tenant_id_like("tenant%")
        .list()
        .expect("query");
    assert_eq!(like_tenant.len(), 2);

    let no_tenant = engine
        .repository_service()
        .create_deployment_query()
        .without_tenant_id()
        .list()
        .expect("query");
    assert_eq!(no_tenant.len(), 1);
    assert_eq!(no_tenant[0].name.as_deref(), Some("c"));
}

#[test]
fn deployment_query_by_parent_deployment_id() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let parent = deploy(&engine, "parent", None, None, None, None, "k1", "C1");
    deploy(
        &engine,
        "child",
        None,
        None,
        None,
        Some(&parent.id),
        "k2",
        "C2",
    );
    deploy(&engine, "orphan", None, None, None, None, "k3", "C3");

    let children = engine
        .repository_service()
        .create_deployment_query()
        .parent_deployment_id(parent.id.clone())
        .list()
        .expect("query");
    assert_eq!(children.len(), 1);
    assert_eq!(children[0].name.as_deref(), Some("child"));
}

#[test]
fn deployment_query_latest_requires_key() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    // latest() without key should error
    let err = engine
        .repository_service()
        .create_deployment_query()
        .latest()
        .list();
    assert!(err.is_err());
}

#[test]
fn deployment_query_latest_returns_most_recent_for_key() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    deploy(&engine, "first", None, Some("dk"), None, None, "k1", "C1");
    // Small delay to ensure different deployed_at timestamps
    std::thread::sleep(std::time::Duration::from_millis(10));
    deploy(&engine, "second", None, Some("dk"), None, None, "k1", "C1");

    let latest = engine
        .repository_service()
        .create_deployment_query()
        .key("dk")
        .latest()
        .single_result()
        .expect("query")
        .expect("deployment");
    assert_eq!(latest.name.as_deref(), Some("second"));
}

#[test]
fn deployment_query_count() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    deploy(&engine, "a", Some("cat-a"), None, None, None, "k1", "C1");
    deploy(&engine, "b", Some("cat-a"), None, None, None, "k2", "C2");
    deploy(&engine, "c", Some("cat-b"), None, None, None, "k3", "C3");

    let count = engine
        .repository_service()
        .create_deployment_query()
        .category("cat-a")
        .count()
        .expect("count");
    assert_eq!(count, 2);
}

#[test]
fn deployment_query_order_by_and_page() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    deploy(&engine, "charlie", None, None, None, None, "k1", "C1");
    deploy(&engine, "alpha", None, None, None, None, "k2", "C2");
    deploy(&engine, "bravo", None, None, None, None, "k3", "C3");

    let asc = engine
        .repository_service()
        .create_deployment_query()
        .order_by(DeploymentSortField::Name, SortDirection::Asc)
        .list()
        .expect("query");
    let names: Vec<_> = asc.iter().filter_map(|d| d.name.as_deref()).collect();
    assert_eq!(names, vec!["alpha", "bravo", "charlie"]);

    let desc = engine
        .repository_service()
        .create_deployment_query()
        .order_by(DeploymentSortField::Name, SortDirection::Desc)
        .list()
        .expect("query");
    let names: Vec<_> = desc.iter().filter_map(|d| d.name.as_deref()).collect();
    assert_eq!(names, vec!["charlie", "bravo", "alpha"]);

    let paged = engine
        .repository_service()
        .create_deployment_query()
        .order_by(DeploymentSortField::Name, SortDirection::Asc)
        .page(1, 1)
        .list_page()
        .expect("page");
    assert_eq!(paged.data.len(), 1);
    assert_eq!(paged.data[0].name.as_deref(), Some("bravo"));
}

// ---- Case definition query tests ----

#[test]
fn definition_query_by_key() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    deploy(&engine, "a", None, None, None, None, "case-key-a", "Name A");
    deploy(&engine, "b", None, None, None, None, "case-key-b", "Name B");

    let result = engine
        .repository_service()
        .create_case_definition_query()
        .key("case-key-a")
        .list()
        .expect("query");
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].key, "case-key-a");
}

#[test]
fn definition_query_by_key_like() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    deploy(&engine, "a", None, None, None, None, "alpha-1", "A1");
    deploy(&engine, "b", None, None, None, None, "alpha-2", "A2");
    deploy(&engine, "c", None, None, None, None, "beta-1", "B1");

    let result = engine
        .repository_service()
        .create_case_definition_query()
        .key_like("alpha%")
        .list()
        .expect("query");
    assert_eq!(result.len(), 2);
}

#[test]
fn definition_query_by_category_and_like() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    deploy(&engine, "a", Some("finance"), None, None, None, "k1", "C1");
    deploy(
        &engine,
        "b",
        Some("finance-reports"),
        None,
        None,
        None,
        "k2",
        "C2",
    );
    deploy(&engine, "c", Some("hr"), None, None, None, "k3", "C3");

    let exact = engine
        .repository_service()
        .create_case_definition_query()
        .category("finance")
        .list()
        .expect("query");
    assert_eq!(exact.len(), 1);

    let like = engine
        .repository_service()
        .create_case_definition_query()
        .category_like("finance%")
        .list()
        .expect("query");
    assert_eq!(like.len(), 2);

    let not_eq = engine
        .repository_service()
        .create_case_definition_query()
        .category_not_equals("finance")
        .list()
        .expect("query");
    assert_eq!(not_eq.len(), 2);
}

#[test]
fn definition_query_by_name_and_name_like() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    deploy(
        &engine,
        "a",
        None,
        None,
        None,
        None,
        "k1",
        "Invoice Approval",
    );
    deploy(&engine, "b", None, None, None, None, "k2", "Invoice Review");
    deploy(&engine, "c", None, None, None, None, "k3", "Expense Report");

    let exact = engine
        .repository_service()
        .create_case_definition_query()
        .name("Invoice Approval")
        .list()
        .expect("query");
    assert_eq!(exact.len(), 1);

    let like = engine
        .repository_service()
        .create_case_definition_query()
        .name_like("Invoice%")
        .list()
        .expect("query");
    assert_eq!(like.len(), 2);

    let icase = engine
        .repository_service()
        .create_case_definition_query()
        .name_like_ignore_case("invoice%")
        .list()
        .expect("query");
    assert_eq!(icase.len(), 2);
}

#[test]
fn definition_query_by_deployment_id_and_ids() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let d1 = deploy(&engine, "a", None, None, None, None, "k1", "C1");
    let d2 = deploy(&engine, "b", None, None, None, None, "k2", "C2");
    deploy(&engine, "c", None, None, None, None, "k3", "C3");

    let by_dep = engine
        .repository_service()
        .create_case_definition_query()
        .deployment_id(d1.id.clone())
        .list()
        .expect("query");
    assert_eq!(by_dep.len(), 1);

    let by_deps = engine
        .repository_service()
        .create_case_definition_query()
        .deployment_ids(vec![d1.id.clone(), d2.id.clone()])
        .list()
        .expect("query");
    assert_eq!(by_deps.len(), 2);
}

#[test]
fn definition_query_by_parent_deployment_id() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let parent = deploy(&engine, "parent", None, None, None, None, "k1", "C1");
    deploy(
        &engine,
        "child",
        None,
        None,
        None,
        Some(&parent.id),
        "k2",
        "C2",
    );
    deploy(&engine, "orphan", None, None, None, None, "k3", "C3");

    let children = engine
        .repository_service()
        .create_case_definition_query()
        .parent_deployment_id(parent.id)
        .list()
        .expect("query");
    assert_eq!(children.len(), 1);
    assert_eq!(children[0].key, "k2");
}

#[test]
fn definition_query_by_tenant_id_and_without() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    deploy(&engine, "a", None, None, Some("t1"), None, "k1", "C1");
    deploy(&engine, "b", None, None, Some("t2"), None, "k2", "C2");
    deploy(&engine, "c", None, None, None, None, "k3", "C3");

    let by_tenant = engine
        .repository_service()
        .create_case_definition_query()
        .tenant_id("t1")
        .list()
        .expect("query");
    assert_eq!(by_tenant.len(), 1);

    let like_tenant = engine
        .repository_service()
        .create_case_definition_query()
        .tenant_id_like("t%")
        .list()
        .expect("query");
    assert_eq!(like_tenant.len(), 2);

    let no_tenant = engine
        .repository_service()
        .create_case_definition_query()
        .without_tenant_id()
        .list()
        .expect("query");
    assert_eq!(no_tenant.len(), 1);
}

#[test]
fn definition_query_by_resource_name_and_like() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    deploy(&engine, "a", None, None, None, None, "k1", "C1");
    deploy(&engine, "b", None, None, None, None, "k2", "C2");

    let exact = engine
        .repository_service()
        .create_case_definition_query()
        .resource_name("case.cmmn")
        .list()
        .expect("query");
    assert_eq!(exact.len(), 2);

    let like = engine
        .repository_service()
        .create_case_definition_query()
        .resource_name_like("case%")
        .list()
        .expect("query");
    assert_eq!(like.len(), 2);
}

#[test]
fn definition_query_version_filters() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    // Deploy the same case key 3 times to create versions 1, 2, 3
    deploy(
        &engine,
        "v1",
        None,
        None,
        None,
        None,
        "shared-key",
        "Shared",
    );
    deploy(
        &engine,
        "v2",
        None,
        None,
        None,
        None,
        "shared-key",
        "Shared",
    );
    deploy(
        &engine,
        "v3",
        None,
        None,
        None,
        None,
        "shared-key",
        "Shared",
    );

    let v2 = engine
        .repository_service()
        .create_case_definition_query()
        .key("shared-key")
        .version(2)
        .single_result()
        .expect("query")
        .expect("definition");
    assert_eq!(v2.version, 2);

    let gt1 = engine
        .repository_service()
        .create_case_definition_query()
        .key("shared-key")
        .version_gt(1)
        .list()
        .expect("query");
    assert_eq!(gt1.len(), 2);

    let gte2 = engine
        .repository_service()
        .create_case_definition_query()
        .key("shared-key")
        .version_gte(2)
        .list()
        .expect("query");
    assert_eq!(gte2.len(), 2);

    let lt3 = engine
        .repository_service()
        .create_case_definition_query()
        .key("shared-key")
        .version_lt(3)
        .list()
        .expect("query");
    assert_eq!(lt3.len(), 2);

    let lte2 = engine
        .repository_service()
        .create_case_definition_query()
        .key("shared-key")
        .version_lte(2)
        .list()
        .expect("query");
    assert_eq!(lte2.len(), 2);
}

#[test]
fn definition_query_latest_version() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    // Deploy same key 3 times -> versions 1, 2, 3
    deploy(&engine, "v1", None, None, None, None, "lk", "Latest");
    deploy(&engine, "v2", None, None, None, None, "lk", "Latest");
    deploy(&engine, "v3", None, None, None, None, "lk", "Latest");
    // Another key with 1 version
    deploy(
        &engine,
        "other",
        None,
        None,
        None,
        None,
        "other-key",
        "Other",
    );

    let latest = engine
        .repository_service()
        .create_case_definition_query()
        .latest_version()
        .list()
        .expect("query");
    assert_eq!(latest.len(), 2);
    let lk = latest
        .iter()
        .find(|d| d.key == "lk")
        .expect("lk definition");
    assert_eq!(lk.version, 3);
}

#[test]
fn definition_query_count() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    deploy(&engine, "a", Some("cat-a"), None, None, None, "k1", "C1");
    deploy(&engine, "b", Some("cat-a"), None, None, None, "k2", "C2");
    deploy(&engine, "c", Some("cat-b"), None, None, None, "k3", "C3");

    let count = engine
        .repository_service()
        .create_case_definition_query()
        .category("cat-a")
        .count()
        .expect("count");
    assert_eq!(count, 2);

    // Count with post-filter (name) should also work
    let name_count = engine
        .repository_service()
        .create_case_definition_query()
        .name("C1")
        .count()
        .expect("count");
    assert_eq!(name_count, 1);
}

#[test]
fn definition_query_order_by_and_page() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    deploy(&engine, "a", None, None, None, None, "key-c", "C");
    deploy(&engine, "b", None, None, None, None, "key-a", "A");
    deploy(&engine, "c", None, None, None, None, "key-b", "B");

    let asc = engine
        .repository_service()
        .create_case_definition_query()
        .order_by(CaseDefinitionSortField::Key, SortDirection::Asc)
        .list()
        .expect("query");
    let keys: Vec<_> = asc.iter().map(|d| d.key.as_str()).collect();
    assert_eq!(keys, vec!["key-a", "key-b", "key-c"]);

    let paged = engine
        .repository_service()
        .create_case_definition_query()
        .order_by(CaseDefinitionSortField::Key, SortDirection::Asc)
        .page(1, 1)
        .list_page()
        .expect("page");
    assert_eq!(paged.data.len(), 1);
    assert_eq!(paged.data[0].key, "key-b");
}

#[test]
fn definition_query_by_id_and_ids() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    deploy(&engine, "a", None, None, None, None, "k1", "C1");
    deploy(&engine, "b", None, None, None, None, "k2", "C2");
    deploy(&engine, "c", None, None, None, None, "k3", "C3");

    // Get a definition id
    let def2 = engine
        .repository_service()
        .create_case_definition_query()
        .key("k2")
        .single_result()
        .expect("query")
        .expect("definition");

    let by_id = engine
        .repository_service()
        .create_case_definition_query()
        .id(def2.id.clone())
        .single_result()
        .expect("query")
        .expect("definition");
    assert_eq!(by_id.key, "k2");

    let all = engine
        .repository_service()
        .create_case_definition_query()
        .list()
        .expect("query");
    let all_ids: Vec<String> = all.iter().map(|d| d.id.clone()).collect();

    let by_ids = engine
        .repository_service()
        .create_case_definition_query()
        .ids(all_ids)
        .list()
        .expect("query");
    assert_eq!(by_ids.len(), 3);
}

// ---------------------------------------------------------------------------
// Task 12: Strict single_result tests
// ---------------------------------------------------------------------------

#[test]
fn single_result_deployment_zero_matches_returns_none() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let result = engine
        .repository_service()
        .create_deployment_query()
        .name("nonexistent")
        .single_result()
        .expect("query should succeed");
    assert!(result.is_none());
}

#[test]
fn single_result_deployment_one_match_returns_value() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    deploy(&engine, "only-one", None, None, None, None, "k1", "C1");

    let result = engine
        .repository_service()
        .create_deployment_query()
        .name("only-one")
        .single_result()
        .expect("query should succeed")
        .expect("deployment should exist");
    assert_eq!(result.name.as_deref(), Some("only-one"));
}

#[test]
fn single_result_deployment_multiple_matches_errors() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    deploy(&engine, "dup", None, None, Some("t1"), None, "k1", "C1");
    deploy(&engine, "dup", None, None, Some("t2"), None, "k2", "C2");

    let result = engine
        .repository_service()
        .create_deployment_query()
        .name("dup")
        .single_result();
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("non-unique") || err.contains("NonUnique") || err.contains("multiple"),
        "expected non-unique error, got: {err}"
    );
}

#[test]
fn single_result_definition_zero_matches_returns_none() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let result = engine
        .repository_service()
        .create_case_definition_query()
        .key("nonexistent")
        .single_result()
        .expect("query should succeed");
    assert!(result.is_none());
}

#[test]
fn single_result_definition_one_match_returns_value() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    deploy(&engine, "a", None, None, None, None, "unique-key", "Unique");

    let result = engine
        .repository_service()
        .create_case_definition_query()
        .key("unique-key")
        .single_result()
        .expect("query should succeed")
        .expect("definition should exist");
    assert_eq!(result.key, "unique-key");
}

#[test]
fn single_result_definition_multiple_matches_errors() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    // Deploy same key twice -> two versions
    deploy(&engine, "v1", None, None, None, None, "multi-key", "Multi");
    deploy(&engine, "v2", None, None, None, None, "multi-key", "Multi");

    let result = engine
        .repository_service()
        .create_case_definition_query()
        .key("multi-key")
        .single_result();
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("non-unique") || err.contains("NonUnique") || err.contains("multiple"),
        "expected non-unique error, got: {err}"
    );
}

// ---------------------------------------------------------------------------
// Task 13: Category and parent-deployment mutation tests
// ---------------------------------------------------------------------------

#[test]
fn category_update_persists_and_is_queryable() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let deployment = deploy(
        &engine, "cat-dep", None, None, None, None, "cat-key", "Cat Case",
    );
    let definition = engine
        .repository_service()
        .create_case_definition_query()
        .deployment_id(&deployment.id)
        .single_result()
        .expect("query")
        .expect("definition");
    assert_eq!(definition.category, None);

    engine
        .repository_service()
        .set_case_definition_category(&definition.id, Some("updated-category"))
        .expect("update category");

    let reloaded = engine
        .repository_service()
        .get_case_definition(&definition.id)
        .expect("reload");
    assert_eq!(reloaded.category.as_deref(), Some("updated-category"));

    let queried = engine
        .repository_service()
        .create_case_definition_query()
        .category("updated-category")
        .single_result()
        .expect("query by category")
        .expect("definition should be findable by new category");
    assert_eq!(queried.id, definition.id);
}

#[test]
fn category_clear_to_none_persists() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let deployment = deploy(
        &engine,
        "cat-clear",
        Some("initial-category"),
        None,
        None,
        None,
        "clear-key",
        "Clear Case",
    );
    let definition = engine
        .repository_service()
        .create_case_definition_query()
        .deployment_id(&deployment.id)
        .single_result()
        .expect("query")
        .expect("definition");
    assert_eq!(definition.category.as_deref(), Some("initial-category"));

    engine
        .repository_service()
        .set_case_definition_category(&definition.id, None)
        .expect("clear category");

    let reloaded = engine
        .repository_service()
        .get_case_definition(&definition.id)
        .expect("reload");
    assert_eq!(reloaded.category, None);
}

#[test]
fn category_update_unknown_definition_returns_not_found() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let result = engine
        .repository_service()
        .set_case_definition_category("cmmn-case-definition:nonexistent", Some("cat"));
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("not found") || err.contains("NotFound"),
        "expected not-found error, got: {err}"
    );
}

#[test]
fn category_update_is_tenant_isolated() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let dep_a = deploy(
        &engine,
        "tenant-a-dep",
        None,
        None,
        Some("tenant-a"),
        None,
        "iso-key",
        "Iso A",
    );
    let dep_b = deploy(
        &engine,
        "tenant-b-dep",
        None,
        None,
        Some("tenant-b"),
        None,
        "iso-key",
        "Iso B",
    );
    let def_a = engine
        .repository_service()
        .create_case_definition_query()
        .deployment_id(&dep_a.id)
        .single_result()
        .expect("query a")
        .expect("def a");
    let def_b = engine
        .repository_service()
        .create_case_definition_query()
        .deployment_id(&dep_b.id)
        .single_result()
        .expect("query b")
        .expect("def b");

    engine
        .repository_service()
        .set_case_definition_category(&def_a.id, Some("cat-a"))
        .expect("update a");
    engine
        .repository_service()
        .set_case_definition_category(&def_b.id, Some("cat-b"))
        .expect("update b");

    let reloaded_a = engine
        .repository_service()
        .get_case_definition(&def_a.id)
        .expect("reload a");
    let reloaded_b = engine
        .repository_service()
        .get_case_definition(&def_b.id)
        .expect("reload b");
    assert_eq!(reloaded_a.category.as_deref(), Some("cat-a"));
    assert_eq!(reloaded_b.category.as_deref(), Some("cat-b"));
    assert_ne!(reloaded_a.category, reloaded_b.category);
}

#[test]
fn parent_deployment_set_replace_and_clear() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let deployment = deploy(
        &engine,
        "parent-dep",
        None,
        None,
        None,
        None,
        "p-key",
        "P Case",
    );

    // Set
    engine
        .repository_service()
        .set_deployment_parent_id(&deployment.id, Some("parent-1"))
        .expect("set parent");
    let reloaded = engine
        .repository_service()
        .get_deployment(&deployment.id)
        .expect("reload");
    assert_eq!(reloaded.parent_deployment_id.as_deref(), Some("parent-1"));

    // Replace
    engine
        .repository_service()
        .set_deployment_parent_id(&deployment.id, Some("parent-2"))
        .expect("replace parent");
    let reloaded = engine
        .repository_service()
        .get_deployment(&deployment.id)
        .expect("reload after replace");
    assert_eq!(reloaded.parent_deployment_id.as_deref(), Some("parent-2"));

    // Clear
    engine
        .repository_service()
        .set_deployment_parent_id(&deployment.id, None)
        .expect("clear parent");
    let reloaded = engine
        .repository_service()
        .get_deployment(&deployment.id)
        .expect("reload after clear");
    assert_eq!(reloaded.parent_deployment_id, None);
}

#[test]
fn parent_deployment_update_unknown_deployment_returns_not_found() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let result = engine
        .repository_service()
        .set_deployment_parent_id("cmmn-deployment:nonexistent", Some("parent-x"));
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("not found") || err.contains("NotFound"),
        "expected not-found error, got: {err}"
    );
}

#[test]
fn parent_deployment_query_reflects_update() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let deployment = deploy(
        &engine,
        "query-parent",
        None,
        None,
        None,
        None,
        "q-key",
        "Q Case",
    );

    engine
        .repository_service()
        .set_deployment_parent_id(&deployment.id, Some("parent-query"))
        .expect("set parent");

    let queried = engine
        .repository_service()
        .create_deployment_query()
        .parent_deployment_id("parent-query")
        .single_result()
        .expect("query")
        .expect("deployment");
    assert_eq!(queried.id, deployment.id);
}

// ---------------------------------------------------------------------------
// Task 14: Candidate starter identity-link tests
// ---------------------------------------------------------------------------

#[test]
fn starter_add_and_list_user_and_group() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let deployment = deploy(
        &engine,
        "starter-dep",
        None,
        None,
        None,
        None,
        "starter-key",
        "Starter",
    );
    let definition = engine
        .repository_service()
        .create_case_definition_query()
        .deployment_id(&deployment.id)
        .single_result()
        .expect("query")
        .expect("definition");

    // Initially no links
    let links = engine
        .repository_service()
        .get_identity_links_for_case_definition(&definition.id)
        .expect("list links");
    assert!(links.is_empty());

    // Add candidate starter user
    engine
        .repository_service()
        .add_candidate_starter_user(&definition.id, "user1")
        .expect("add user");
    // Add candidate starter group
    engine
        .repository_service()
        .add_candidate_starter_group(&definition.id, "group1")
        .expect("add group");

    let links = engine
        .repository_service()
        .get_identity_links_for_case_definition(&definition.id)
        .expect("list links after add");
    assert_eq!(links.len(), 2);
    let user_ids: Vec<_> = links.iter().filter_map(|l| l.user_id.as_deref()).collect();
    let group_ids: Vec<_> = links.iter().filter_map(|l| l.group_id.as_deref()).collect();
    assert!(user_ids.contains(&"user1"));
    assert!(group_ids.contains(&"group1"));
}

#[test]
fn starter_delete_user_and_group() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let deployment = deploy(
        &engine,
        "del-starter",
        None,
        None,
        None,
        None,
        "del-key",
        "Del",
    );
    let definition = engine
        .repository_service()
        .create_case_definition_query()
        .deployment_id(&deployment.id)
        .single_result()
        .expect("query")
        .expect("definition");

    engine
        .repository_service()
        .add_candidate_starter_user(&definition.id, "user1")
        .expect("add user");
    engine
        .repository_service()
        .add_candidate_starter_group(&definition.id, "group1")
        .expect("add group");

    // Delete non-existent is a no-op (Java behavior)
    engine
        .repository_service()
        .delete_candidate_starter_user(&definition.id, "nonexistent")
        .expect("delete nonexistent user");

    // Delete user
    engine
        .repository_service()
        .delete_candidate_starter_user(&definition.id, "user1")
        .expect("delete user");
    let links = engine
        .repository_service()
        .get_identity_links_for_case_definition(&definition.id)
        .expect("list after user delete");
    assert_eq!(links.len(), 1);
    assert!(links[0].group_id.as_deref() == Some("group1"));

    // Delete group
    engine
        .repository_service()
        .delete_candidate_starter_group(&definition.id, "group1")
        .expect("delete group");
    let links = engine
        .repository_service()
        .get_identity_links_for_case_definition(&definition.id)
        .expect("list after group delete");
    assert!(links.is_empty());
}

#[test]
fn starter_add_duplicate_is_idempotent() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let deployment = deploy(
        &engine,
        "dup-starter",
        None,
        None,
        None,
        None,
        "dup-key",
        "Dup",
    );
    let definition = engine
        .repository_service()
        .create_case_definition_query()
        .deployment_id(&deployment.id)
        .single_result()
        .expect("query")
        .expect("definition");

    engine
        .repository_service()
        .add_candidate_starter_user(&definition.id, "user1")
        .expect("first add");
    engine
        .repository_service()
        .add_candidate_starter_user(&definition.id, "user1")
        .expect("second add");

    let links = engine
        .repository_service()
        .get_identity_links_for_case_definition(&definition.id)
        .expect("list");
    assert_eq!(
        links.len(),
        1,
        "duplicate add must not create a second link"
    );
}

#[test]
fn starter_unknown_definition_returns_not_found() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let result = engine
        .repository_service()
        .add_candidate_starter_user("cmmn-case-definition:nonexistent", "user1");
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("not found") || err.contains("NotFound"),
        "expected not-found, got: {err}"
    );
}

#[test]
fn starter_query_startable_by_user() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let dep_a = deploy(&engine, "a", None, None, None, None, "key-a", "A");
    let dep_b = deploy(&engine, "b", None, None, None, None, "key-b", "B");
    let def_a = engine
        .repository_service()
        .create_case_definition_query()
        .deployment_id(&dep_a.id)
        .single_result()
        .expect("query a")
        .expect("def a");
    let def_b = engine
        .repository_service()
        .create_case_definition_query()
        .deployment_id(&dep_b.id)
        .single_result()
        .expect("query b")
        .expect("def b");

    engine
        .repository_service()
        .add_candidate_starter_user(&def_a.id, "user1")
        .expect("add");
    engine
        .repository_service()
        .add_candidate_starter_user(&def_b.id, "user2")
        .expect("add");

    let startable_by_user1 = engine
        .repository_service()
        .create_case_definition_query()
        .startable_by_user("user1")
        .list()
        .expect("query");
    assert_eq!(startable_by_user1.len(), 1);
    assert_eq!(startable_by_user1[0].key, "key-a");

    let startable_by_unknown = engine
        .repository_service()
        .create_case_definition_query()
        .startable_by_user("unknown-user")
        .list()
        .expect("query");
    assert!(startable_by_unknown.is_empty());
}

#[test]
fn starter_query_startable_by_user_or_groups() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let dep_a = deploy(&engine, "a", None, None, None, None, "gkey-a", "A");
    let dep_b = deploy(&engine, "b", None, None, None, None, "gkey-b", "B");
    let dep_c = deploy(&engine, "c", None, None, None, None, "gkey-c", "C");
    let def_a = engine
        .repository_service()
        .create_case_definition_query()
        .deployment_id(&dep_a.id)
        .single_result()
        .expect("query a")
        .expect("def a");
    let def_b = engine
        .repository_service()
        .create_case_definition_query()
        .deployment_id(&dep_b.id)
        .single_result()
        .expect("query b")
        .expect("def b");
    let def_c = engine
        .repository_service()
        .create_case_definition_query()
        .deployment_id(&dep_c.id)
        .single_result()
        .expect("query c")
        .expect("def c");

    // def_a: user1
    engine
        .repository_service()
        .add_candidate_starter_user(&def_a.id, "user1")
        .expect("add");
    // def_b: group1
    engine
        .repository_service()
        .add_candidate_starter_group(&def_b.id, "group1")
        .expect("add");
    // def_c: group2
    engine
        .repository_service()
        .add_candidate_starter_group(&def_c.id, "group2")
        .expect("add");

    // user1 can start def_a only
    let by_user = engine
        .repository_service()
        .create_case_definition_query()
        .startable_by_user_or_groups(Some("user1"), &[])
        .list()
        .expect("query");
    assert_eq!(by_user.len(), 1);
    assert_eq!(by_user[0].key, "gkey-a");

    // group1 can start def_b only
    let by_group = engine
        .repository_service()
        .create_case_definition_query()
        .startable_by_user_or_groups(None, &["group1"])
        .list()
        .expect("query");
    assert_eq!(by_group.len(), 1);
    assert_eq!(by_group[0].key, "gkey-b");

    // user1 OR group1 can start def_a and def_b
    let by_user_or_group = engine
        .repository_service()
        .create_case_definition_query()
        .startable_by_user_or_groups(Some("user1"), &["group1"])
        .list()
        .expect("query");
    assert_eq!(by_user_or_group.len(), 2);

    // user1 OR group2 can start def_a and def_c
    let by_user_or_g2 = engine
        .repository_service()
        .create_case_definition_query()
        .startable_by_user_or_groups(Some("user1"), &["group2"])
        .list()
        .expect("query");
    assert_eq!(by_user_or_g2.len(), 2);

    // Empty user and empty groups -> no results
    let empty = engine
        .repository_service()
        .create_case_definition_query()
        .startable_by_user_or_groups(None, &[])
        .list()
        .expect("query");
    assert!(empty.is_empty());
}

#[test]
fn starter_query_does_not_duplicate_when_user_and_group_both_match() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let deployment = deploy(&engine, "dup-q", None, None, None, None, "dq-key", "DQ");
    let definition = engine
        .repository_service()
        .create_case_definition_query()
        .deployment_id(&deployment.id)
        .single_result()
        .expect("query")
        .expect("definition");

    // Add both user and group to same definition
    engine
        .repository_service()
        .add_candidate_starter_user(&definition.id, "user1")
        .expect("add user");
    engine
        .repository_service()
        .add_candidate_starter_group(&definition.id, "group1")
        .expect("add group");

    let results = engine
        .repository_service()
        .create_case_definition_query()
        .startable_by_user_or_groups(Some("user1"), &["group1"])
        .list()
        .expect("query");
    assert_eq!(
        results.len(),
        1,
        "definition must appear only once even if both user and group match"
    );
}

// ── Task 15: Referenced DMN decisions and form definitions ──

/// A mock decision resolver that holds a map of key -> ReferencedDecision.
#[allow(clippy::type_complexity)]
struct MockDecisionResolver {
    decisions: std::collections::HashMap<String, ReferencedDecision>,
    /// Records the arguments of each `resolve_decision` call for assertion.
    calls: std::sync::Mutex<Vec<(String, Option<String>, Option<String>)>>,
}

impl MockDecisionResolver {
    fn new() -> Self {
        Self {
            decisions: std::collections::HashMap::new(),
            calls: std::sync::Mutex::new(Vec::new()),
        }
    }

    fn with_decision(mut self, key: &str, decision: ReferencedDecision) -> Self {
        self.decisions.insert(key.to_string(), decision);
        self
    }

    fn calls(&self) -> Vec<(String, Option<String>, Option<String>)> {
        self.calls.lock().unwrap().clone()
    }
}

impl CmmnDecisionResolver for MockDecisionResolver {
    fn resolve_decision(
        &self,
        key: &str,
        tenant_id: Option<&str>,
        parent_deployment_id: Option<&str>,
    ) -> Result<Option<ReferencedDecision>, flowable_cmmn_engine::CmmnError> {
        self.calls.lock().unwrap().push((
            key.to_string(),
            tenant_id.map(str::to_string),
            parent_deployment_id.map(str::to_string),
        ));
        Ok(self.decisions.get(key).cloned())
    }
}

/// A mock form resolver that holds a map of key -> ReferencedFormDefinition.
#[allow(clippy::type_complexity)]
struct MockFormResolver {
    forms: std::collections::HashMap<String, ReferencedFormDefinition>,
    calls: std::sync::Mutex<Vec<(String, Option<String>, Option<String>)>>,
}

impl MockFormResolver {
    fn new() -> Self {
        Self {
            forms: std::collections::HashMap::new(),
            calls: std::sync::Mutex::new(Vec::new()),
        }
    }

    fn with_form(mut self, key: &str, form: ReferencedFormDefinition) -> Self {
        self.forms.insert(key.to_string(), form);
        self
    }

    fn calls(&self) -> Vec<(String, Option<String>, Option<String>)> {
        self.calls.lock().unwrap().clone()
    }
}

impl CmmnFormResolver for MockFormResolver {
    fn resolve_form(
        &self,
        key: &str,
        tenant_id: Option<&str>,
        parent_deployment_id: Option<&str>,
    ) -> Result<Option<ReferencedFormDefinition>, flowable_cmmn_engine::CmmnError> {
        self.calls.lock().unwrap().push((
            key.to_string(),
            tenant_id.map(str::to_string),
            parent_deployment_id.map(str::to_string),
        ));
        Ok(self.forms.get(key).cloned())
    }
}

/// Build a CMMN model that references decision keys and form keys.
fn model_with_references(
    case_key: &str,
    decision_keys: &[&str],
    form_keys: &[&str],
    start_form_key: Option<&str>,
) -> CmmnModel {
    let mut plan_model =
        CmmnCasePlanModel::new(format!("{case_key}-plan"), format!("{case_key} plan"));
    if let Some(sf) = start_form_key {
        plan_model = plan_model.with_start_form_key(sf);
    }
    for dk in decision_keys {
        let task_id = format!("decision-task-{dk}");
        plan_model = plan_model.with_decision_task(
            CmmnDecisionTask::new(task_id.clone(), format!("Decision {dk}")).with_decision_ref(*dk),
        );
        plan_model =
            plan_model.with_plan_item(CmmnPlanItem::new(format!("plan-item-{dk}"), task_id));
    }
    for fk in form_keys {
        let task_id = format!("human-task-{fk}");
        plan_model = plan_model.with_human_task(
            CmmnHumanTask::new(task_id.clone(), format!("Task {fk}")).with_form_key(*fk),
        );
        plan_model =
            plan_model.with_plan_item(CmmnPlanItem::new(format!("plan-item-{fk}"), task_id));
    }
    CmmnModel::new(vec![CmmnCase::new(
        format!("{case_key}-case"),
        case_key,
        format!("{case_key} case"),
        plan_model,
    )])
}

fn deploy_with_references(
    engine: &CmmnEngine,
    case_key: &str,
    decision_keys: &[&str],
    form_keys: &[&str],
    start_form_key: Option<&str>,
    parent_deployment_id: Option<&str>,
    tenant_id: Option<&str>,
) -> String {
    let model = model_with_references(case_key, decision_keys, form_keys, start_form_key);
    let mut req = CmmnDeploymentRequest::new(format!("deploy-{case_key}"))
        .with_resource(format!("{case_key}.cmmn"), model);
    if let Some(p) = parent_deployment_id {
        req = req.with_parent_deployment_id(p);
    }
    if let Some(t) = tenant_id {
        req = req.with_tenant_id(t);
    }
    let _deployment = engine.deploy(req).expect("deployment should succeed");
    engine
        .repository_service()
        .create_case_definition_query()
        .key(case_key)
        .single_result()
        .expect("definition query should succeed")
        .expect("definition should exist")
        .id
}

#[test]
fn referenced_decisions_resolves_all_deployed_decision_keys() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let def_id = deploy_with_references(
        &engine,
        "ref-decisions-case",
        &["decision-a", "decision-b"],
        &[],
        None,
        None,
        None,
    );

    let resolver = MockDecisionResolver::new()
        .with_decision(
            "decision-a",
            ReferencedDecision {
                id: "dmn-1".to_string(),
                key: "decision-a".to_string(),
                name: "Decision A".to_string(),
                version: 1,
                deployment_id: "dmn-deploy-1".to_string(),
                tenant_id: None,
                resource_name: "a.dmn".to_string(),
            },
        )
        .with_decision(
            "decision-b",
            ReferencedDecision {
                id: "dmn-2".to_string(),
                key: "decision-b".to_string(),
                name: "Decision B".to_string(),
                version: 2,
                deployment_id: "dmn-deploy-2".to_string(),
                tenant_id: None,
                resource_name: "b.dmn".to_string(),
            },
        );

    let results = engine
        .repository_service()
        .list_referenced_decisions(&def_id, &resolver)
        .expect("resolve decisions");

    assert_eq!(results.len(), 2);
    assert_eq!(results[0].key, "decision-a");
    assert_eq!(results[0].version, 1);
    assert_eq!(results[1].key, "decision-b");
    assert_eq!(results[1].version, 2);
}

#[test]
fn referenced_decisions_silently_omits_missing_references() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let def_id = deploy_with_references(
        &engine,
        "ref-missing-decisions-case",
        &["deployed-decision", "missing-decision"],
        &[],
        None,
        None,
        None,
    );

    // Only "deployed-decision" is resolvable; "missing-decision" is not.
    let resolver = MockDecisionResolver::new().with_decision(
        "deployed-decision",
        ReferencedDecision {
            id: "dmn-1".to_string(),
            key: "deployed-decision".to_string(),
            name: "Deployed".to_string(),
            version: 1,
            deployment_id: "dmn-deploy-1".to_string(),
            tenant_id: None,
            resource_name: "deployed.dmn".to_string(),
        },
    );

    let results = engine
        .repository_service()
        .list_referenced_decisions(&def_id, &resolver)
        .expect("resolve decisions should not error on missing");

    assert_eq!(
        results.len(),
        1,
        "missing decision must be silently omitted"
    );
    assert_eq!(results[0].key, "deployed-decision");
}

#[test]
fn referenced_decisions_unknown_definition_returns_not_found() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let resolver = MockDecisionResolver::new();
    let err = engine
        .repository_service()
        .list_referenced_decisions("nonexistent-definition", &resolver)
        .expect_err("must fail for unknown definition");
    assert!(err.to_string().contains("not found"));
}

#[test]
fn referenced_decisions_passes_parent_deployment_id_to_resolver() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let def_id = deploy_with_references(
        &engine,
        "ref-parent-decisions-case",
        &["decision-scoped"],
        &[],
        None,
        Some("parent-deploy-123"),
        None,
    );

    let resolver = MockDecisionResolver::new().with_decision(
        "decision-scoped",
        ReferencedDecision {
            id: "dmn-1".to_string(),
            key: "decision-scoped".to_string(),
            name: "Scoped".to_string(),
            version: 1,
            deployment_id: "dmn-deploy-1".to_string(),
            tenant_id: None,
            resource_name: "scoped.dmn".to_string(),
        },
    );

    let results = engine
        .repository_service()
        .list_referenced_decisions(&def_id, &resolver)
        .expect("resolve");

    assert_eq!(results.len(), 1);
    let calls = resolver.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, "decision-scoped");
    assert_eq!(calls[0].2.as_deref(), Some("parent-deploy-123"));
}

#[test]
fn referenced_decisions_passes_tenant_id_to_resolver() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let def_id = deploy_with_references(
        &engine,
        "ref-tenant-decisions-case",
        &["decision-tenant"],
        &[],
        None,
        None,
        Some("tenant-x"),
    );

    let resolver = MockDecisionResolver::new().with_decision(
        "decision-tenant",
        ReferencedDecision {
            id: "dmn-1".to_string(),
            key: "decision-tenant".to_string(),
            name: "Tenant".to_string(),
            version: 1,
            deployment_id: "dmn-deploy-1".to_string(),
            tenant_id: Some("tenant-x".to_string()),
            resource_name: "tenant.dmn".to_string(),
        },
    );

    let results = engine
        .repository_service()
        .list_referenced_decisions(&def_id, &resolver)
        .expect("resolve");

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].tenant_id.as_deref(), Some("tenant-x"));
    let calls = resolver.calls();
    assert_eq!(calls[0].1.as_deref(), Some("tenant-x"));
}

#[test]
fn referenced_decisions_sorted_by_key_then_version() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    // Deploy keys in non-alphabetical order
    let def_id = deploy_with_references(
        &engine,
        "ref-sorted-decisions-case",
        &["zebra", "alpha"],
        &[],
        None,
        None,
        None,
    );

    let resolver = MockDecisionResolver::new()
        .with_decision(
            "zebra",
            ReferencedDecision {
                id: "dmn-z".to_string(),
                key: "zebra".to_string(),
                name: "Z".to_string(),
                version: 1,
                deployment_id: "d1".to_string(),
                tenant_id: None,
                resource_name: "z.dmn".to_string(),
            },
        )
        .with_decision(
            "alpha",
            ReferencedDecision {
                id: "dmn-a".to_string(),
                key: "alpha".to_string(),
                name: "A".to_string(),
                version: 1,
                deployment_id: "d2".to_string(),
                tenant_id: None,
                resource_name: "a.dmn".to_string(),
            },
        );

    let results = engine
        .repository_service()
        .list_referenced_decisions(&def_id, &resolver)
        .expect("resolve");

    assert_eq!(results.len(), 2);
    assert_eq!(results[0].key, "alpha", "must be sorted alphabetically");
    assert_eq!(results[1].key, "zebra");
}

#[test]
fn referenced_forms_resolves_all_deployed_form_keys() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let def_id = deploy_with_references(
        &engine,
        "ref-forms-case",
        &[],
        &["form-a", "form-b"],
        None,
        None,
        None,
    );

    let resolver = MockFormResolver::new()
        .with_form(
            "form-a",
            ReferencedFormDefinition {
                id: "form-1".to_string(),
                key: "form-a".to_string(),
                name: "Form A".to_string(),
                version: 1,
                deployment_id: "form-deploy-1".to_string(),
                resource_name: "a.form".to_string(),
            },
        )
        .with_form(
            "form-b",
            ReferencedFormDefinition {
                id: "form-2".to_string(),
                key: "form-b".to_string(),
                name: "Form B".to_string(),
                version: 2,
                deployment_id: "form-deploy-2".to_string(),
                resource_name: "b.form".to_string(),
            },
        );

    let results = engine
        .repository_service()
        .list_referenced_form_definitions(&def_id, &resolver)
        .expect("resolve forms");

    assert_eq!(results.len(), 2);
    assert_eq!(results[0].key, "form-a");
    assert_eq!(results[1].key, "form-b");
}

#[test]
fn referenced_forms_silently_omits_missing_references() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let def_id = deploy_with_references(
        &engine,
        "ref-missing-forms-case",
        &[],
        &["deployed-form", "missing-form"],
        None,
        None,
        None,
    );

    let resolver = MockFormResolver::new().with_form(
        "deployed-form",
        ReferencedFormDefinition {
            id: "form-1".to_string(),
            key: "deployed-form".to_string(),
            name: "Deployed".to_string(),
            version: 1,
            deployment_id: "form-deploy-1".to_string(),
            resource_name: "deployed.form".to_string(),
        },
    );

    let results = engine
        .repository_service()
        .list_referenced_form_definitions(&def_id, &resolver)
        .expect("resolve forms should not error on missing");

    assert_eq!(results.len(), 1, "missing form must be silently omitted");
    assert_eq!(results[0].key, "deployed-form");
}

#[test]
fn referenced_forms_unknown_definition_returns_not_found() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let resolver = MockFormResolver::new();
    let err = engine
        .repository_service()
        .list_referenced_form_definitions("nonexistent-definition", &resolver)
        .expect_err("must fail for unknown definition");
    assert!(err.to_string().contains("not found"));
}

#[test]
fn referenced_forms_includes_start_form_key() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let def_id = deploy_with_references(
        &engine,
        "ref-start-form-case",
        &[],
        &["task-form"],
        Some("start-form-key"),
        None,
        None,
    );

    let resolver = MockFormResolver::new()
        .with_form(
            "start-form-key",
            ReferencedFormDefinition {
                id: "form-start".to_string(),
                key: "start-form-key".to_string(),
                name: "Start".to_string(),
                version: 1,
                deployment_id: "form-deploy-1".to_string(),
                resource_name: "start.form".to_string(),
            },
        )
        .with_form(
            "task-form",
            ReferencedFormDefinition {
                id: "form-task".to_string(),
                key: "task-form".to_string(),
                name: "Task".to_string(),
                version: 1,
                deployment_id: "form-deploy-1".to_string(),
                resource_name: "task.form".to_string(),
            },
        );

    let results = engine
        .repository_service()
        .list_referenced_form_definitions(&def_id, &resolver)
        .expect("resolve forms");

    let keys: Vec<&str> = results.iter().map(|f| f.key.as_str()).collect();
    assert!(
        keys.contains(&"start-form-key"),
        "start form key must be included in referenced forms: {keys:?}"
    );
    assert!(
        keys.contains(&"task-form"),
        "task form key must be included: {keys:?}"
    );
}

#[test]
fn referenced_forms_passes_parent_deployment_id_to_resolver() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let def_id = deploy_with_references(
        &engine,
        "ref-parent-forms-case",
        &[],
        &["form-scoped"],
        None,
        Some("parent-deploy-456"),
        None,
    );

    let resolver = MockFormResolver::new().with_form(
        "form-scoped",
        ReferencedFormDefinition {
            id: "form-1".to_string(),
            key: "form-scoped".to_string(),
            name: "Scoped".to_string(),
            version: 1,
            deployment_id: "form-deploy-1".to_string(),
            resource_name: "scoped.form".to_string(),
        },
    );

    let results = engine
        .repository_service()
        .list_referenced_form_definitions(&def_id, &resolver)
        .expect("resolve");

    assert_eq!(results.len(), 1);
    let calls = resolver.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, "form-scoped");
    assert_eq!(calls[0].2.as_deref(), Some("parent-deploy-456"));
}

#[test]
fn referenced_decisions_no_keys_returns_empty() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let def_id =
        deploy_with_references(&engine, "ref-no-decisions-case", &[], &[], None, None, None);

    let resolver = MockDecisionResolver::new();
    let results = engine
        .repository_service()
        .list_referenced_decisions(&def_id, &resolver)
        .expect("resolve");
    assert!(results.is_empty());
    assert!(
        resolver.calls().is_empty(),
        "resolver must not be called when there are no keys"
    );
}

#[test]
fn referenced_forms_no_keys_returns_empty() {
    let engine = CmmnEngine::new_in_memory().expect("engine");
    let def_id = deploy_with_references(&engine, "ref-no-forms-case", &[], &[], None, None, None);

    let resolver = MockFormResolver::new();
    let results = engine
        .repository_service()
        .list_referenced_form_definitions(&def_id, &resolver)
        .expect("resolve");
    assert!(results.is_empty());
    assert!(
        resolver.calls().is_empty(),
        "resolver must not be called when there are no keys"
    );
}

#[derive(serde::Deserialize)]
struct ParityMatrix {
    capabilities: Vec<ParityCapability>,
    deployment_query_filters: Vec<ParityFilter>,
    case_definition_query_filters: Vec<ParityFilter>,
    rest_endpoints: Vec<ParityRestEndpoint>,
}

#[derive(serde::Deserialize)]
#[allow(dead_code)]
struct ParityCapability {
    java_api: String,
    #[serde(default)]
    rust_api: Option<String>,
    category: String,
    status: String,
    #[serde(default)]
    tests: Vec<String>,
    #[serde(default)]
    rationale: Option<String>,
}

#[derive(serde::Deserialize)]
#[allow(dead_code)]
struct ParityFilter {
    java: String,
    rust: String,
    #[serde(default)]
    tests: Vec<String>,
    status: String,
}

#[derive(serde::Deserialize)]
struct ParityRestEndpoint {
    method: String,
    path: String,
    #[serde(default)]
    tests: Vec<String>,
    status: String,
}

#[test]
fn parity_matrix_has_no_unowned_gaps() {
    let matrix_json = include_str!("fixtures/repository_parity_cases.json");
    let matrix: ParityMatrix =
        serde_json::from_str(matrix_json).expect("repository_parity_cases.json must be valid JSON");

    let valid_statuses = ["aligned", "intentionally_different", "unsupported"];

    for cap in &matrix.capabilities {
        assert!(
            valid_statuses.contains(&cap.status.as_str()),
            "capability '{}' has invalid status '{}'; must be one of {:?}",
            cap.java_api,
            cap.status,
            valid_statuses
        );

        if cap.status == "aligned" || cap.status == "intentionally_different" {
            assert!(
                cap.rust_api.is_some(),
                "capability '{}' is {:?} but has no rust_api mapping",
                cap.java_api,
                cap.status
            );
        }

        if cap.status == "intentionally_different" || cap.status == "unsupported" {
            assert!(
                cap.rationale.is_some() && !cap.rationale.as_ref().unwrap().is_empty(),
                "capability '{}' is {:?} but has no rationale explaining the difference",
                cap.java_api,
                cap.status
            );
        }

        if cap.status == "aligned" {
            assert!(
                !cap.tests.is_empty(),
                "capability '{}' is aligned but has no tests referenced",
                cap.java_api
            );
        }

        for test_name in &cap.tests {
            let is_rest_test = test_name.starts_with("rest_");
            let is_cascade_test = test_name.starts_with("deployment_cascade")
                || test_name.starts_with("deployment_non_cascade");
            let is_parity_test = !is_rest_test && !is_cascade_test;
            assert!(
                is_rest_test || is_cascade_test || is_parity_test,
                "capability '{}' references test '{}' that cannot be attributed to a known test file",
                cap.java_api,
                test_name
            );
        }
    }

    for filt in matrix
        .deployment_query_filters
        .iter()
        .chain(matrix.case_definition_query_filters.iter())
    {
        assert!(
            valid_statuses.contains(&filt.status.as_str()),
            "filter '{}' has invalid status '{}'",
            filt.java,
            filt.status
        );
    }

    for ep in &matrix.rest_endpoints {
        assert!(
            valid_statuses.contains(&ep.status.as_str()),
            "REST endpoint {} {} has invalid status '{}'",
            ep.method,
            ep.path,
            ep.status
        );
        assert!(
            ep.tests.iter().all(|t| t.starts_with("rest_")),
            "REST endpoint {} {} references non-rest test",
            ep.method,
            ep.path
        );
    }

    let total_caps = matrix.capabilities.len();
    let aligned_caps = matrix
        .capabilities
        .iter()
        .filter(|c| c.status == "aligned")
        .count();
    let intentional_caps = matrix
        .capabilities
        .iter()
        .filter(|c| c.status == "intentionally_different")
        .count();
    let unsupported_caps = matrix
        .capabilities
        .iter()
        .filter(|c| c.status == "unsupported")
        .count();
    eprintln!(
        "Parity matrix: {} capabilities total, {} aligned, {} intentionally different, {} unsupported",
        total_caps, aligned_caps, intentional_caps, unsupported_caps
    );
    assert!(
        aligned_caps > 0,
        "must have at least one aligned capability"
    );
    assert!(
        total_caps >= 15,
        "expected at least 15 capabilities to cover Java interface"
    );
}
