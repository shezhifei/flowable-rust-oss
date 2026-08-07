// P110: identity query parameter completion.
//
// Java references:
// - UserCollectionResource.java:111,123,132 — displayName, displayNameLike,
//   tenantId.
// - GroupCollectionResource.java:80 documents `potentialStarter`; engine
//   semantics `IdentityService.java:102` + `GetPotentialStarterGroupsCmd.java`
//   resolve the process definition's group identity links (candidate starters).
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::engine::time_source::SystemTimeSource;
use flowable_engine::identity::entities::{IdentityLink, User};
use flowable_rest::run_server;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::net::TcpListener;

fn build_engine(test_name: &str) -> Arc<ProcessEngine> {
    let db_store =
        Arc::new(flowable_engine::persistence::db_store::DbStore::new_in_memory().unwrap());
    let engine = Arc::new(ProcessEngine::build(
        test_name.to_string(),
        Arc::new(SystemTimeSource) as Arc<_>,
        db_store,
    ));

    engine
        .get_identity_service()
        .save_user(User {
            id: "admin".to_string(),
            first_name: None,
            last_name: None,
            email: None,
            password: Some("test".to_string()),
            tenant_id: None,
        });

    engine
}

async fn spawn_server(engine: Arc<ProcessEngine>) -> (String, reqwest::Client) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let base_url = format!("http://{}", addr);

    let engine_clone = Arc::clone(&engine);
    tokio::spawn(async move {
        run_server(engine_clone, listener).await.unwrap();
    });

    (base_url, reqwest::Client::new())
}

#[tokio::test]
async fn user_query_params_filter_by_display_name_and_tenant() {
    let engine = build_engine("rest-idm-user-query-params");
    let (base_url, client) = spawn_server(Arc::clone(&engine)).await;

    let create = client
        .post(format!("{base_url}/identity/users"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "id": "kermit",
            "firstName": "Kermit",
            "lastName": "The Frog",
            "email": "kermit@example.test",
            "tenantId": "tenant-a"
        }))
        .send()
        .await
        .unwrap();
    assert!(create.status().is_success());

    // displayName exact (Java UserCollectionResource.java:111).
    let resp = client
        .get(format!(
            "{base_url}/identity/users?displayName=Kermit The Frog"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["total"], 1);
    assert_eq!(body["data"][0]["id"], "kermit");

    // displayName no-match.
    let resp = client
        .get(format!(
            "{base_url}/identity/users?displayName=Kermit Frog"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["total"], 0);

    // displayNameLike (Java :123) — follows the identity route's existing
    // case-insensitive contains convention (firstNameLike etc.).
    let resp = client
        .get(format!(
            "{base_url}/identity/users?displayNameLike=the frog"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["total"], 1);
    assert_eq!(body["data"][0]["id"], "kermit");

    // tenantId exact (Java :132).
    let resp = client
        .get(format!(
            "{base_url}/identity/users?tenantId=tenant-a"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["total"], 1);
    assert_eq!(body["data"][0]["id"], "kermit");

    // tenantId no-match.
    let resp = client
        .get(format!("{base_url}/identity/users?tenantId=tenant-b"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["total"], 0);
}

#[tokio::test]
async fn group_query_potential_starter_filters_by_process_definition_links() {
    let engine = build_engine("rest-idm-group-potential-starter");
    let (base_url, client) = spawn_server(Arc::clone(&engine)).await;

    let create = client
        .post(format!("{base_url}/identity/groups"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "id": "managers",
            "name": "Managers",
            "type": "security-role"
        }))
        .send()
        .await
        .unwrap();
    assert!(create.status().is_success());

    // Deploy a minimal process definition so the identity link has a real
    // target (Java GetPotentialStarterGroupsCmd resolves the definition first).
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="starterProcess" name="Starter Process" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="f1" sourceRef="start" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#;
    engine
        .get_repository_service()
        .deploy(
            engine
                .get_repository_service()
                .create_deployment()
                .name("starter".to_string())
                .add_string("starter.bpmn20.xml".to_string(), xml.to_string()),
        )
        .unwrap();
    let process_definition_id = engine
        .get_repository_service()
        .get_process_definition_ids()
        .unwrap()[0]
        .clone();

    // A candidate-starter group identity link on the process definition. The
    // BPMN engine does not populate definition-level candidate links yet, so
    // the test seeds the row the same way GetPotentialStarterGroupsCmd reads
    // it (process definition identity links).
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    store.insert_identity_link(
        IdentityLink {
            id: "link-1".to_string(),
            link_type: "candidate".to_string(),
            user_id: None,
            group_id: Some("managers".to_string()),
            task_id: None,
            process_instance_id: None,
            process_definition_id: Some(process_definition_id.clone()),
        },
        &mut session,
    );
    session.flush_and_commit().unwrap();

    // potentialStarter=<process definition id> → the linked group.
    let resp = client
        .get(format!(
            "{base_url}/identity/groups?potentialStarter={process_definition_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(resp.status().is_success());
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["total"], 1);
    assert_eq!(body["data"][0]["id"], "managers");

    // Unknown process definition id → no group links → empty.
    let resp = client
        .get(format!(
            "{base_url}/identity/groups?potentialStarter=unknown-process-definition"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["total"], 0);
}
