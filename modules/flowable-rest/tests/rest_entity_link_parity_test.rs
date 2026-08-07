use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::identity::entities::User;
use flowable_rest::run_server;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::net::TcpListener;

async fn spawn_server(test_name: &str) -> (Arc<ProcessEngine>, String, reqwest::Client) {
    let engine = Arc::new(ProcessEngine::new(test_name.to_string()));
    engine.get_identity_service().save_user(User {
        id: "admin".to_string(),
        first_name: None,
        last_name: None,
        email: None,
        password: Some("test".to_string()),
        tenant_id: None,
    });

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let base_url = format!("http://{}", addr);

    let engine_clone = Arc::clone(&engine);
    tokio::spawn(async move {
        run_server(engine_clone, listener).await.unwrap();
    });

    (engine, base_url, reqwest::Client::new())
}

#[tokio::test]
async fn rest_entity_link_create_query_delete_parity() {
    let (_engine, base_url, client) = spawn_server("rest-entity-link-parity").await;

    let create = client
        .post(format!("{}/runtime/entity-links", base_url))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "id": "el-1",
            "linkType": "reference",
            "scopeId": "scope-1",
            "scopeType": "processInstance",
            "referenceScopeId": "ref-1",
            "referenceScopeType": "task",
            "hierarchyType": "child"
        }))
        .send()
        .await
        .unwrap();
    assert!(create.status().is_success());
    let created: Value = create.json().await.unwrap();
    assert_eq!(created["id"], "el-1");

    let create2 = client
        .post(format!("{}/runtime/entity-links", base_url))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "id": "el-2",
            "linkType": "dependency",
            "scopeId": "scope-1",
            "scopeType": "processInstance",
            "referenceScopeId": "ref-2",
            "referenceScopeType": "subProcess"
        }))
        .send()
        .await
        .unwrap();
    assert!(create2.status().is_success());

    let list = client
        .get(format!("{}/runtime/entity-links?scopeId=scope-1", base_url))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(list.status(), reqwest::StatusCode::OK);
    let body: Value = list.json().await.unwrap();
    assert_eq!(body.as_array().unwrap().len(), 2);

    let by_type = client
        .get(format!(
            "{}/runtime/entity-links?scopeId=scope-1&linkType=reference",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    let by_type_body: Value = by_type.json().await.unwrap();
    assert_eq!(by_type_body.as_array().unwrap().len(), 1);
    assert_eq!(by_type_body[0]["id"], "el-1");

    let by_ref = client
        .get(format!(
            "{}/runtime/entity-links?referenceScopeId=ref-1",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    let by_ref_body: Value = by_ref.json().await.unwrap();
    assert_eq!(by_ref_body.as_array().unwrap().len(), 1);

    let delete = client
        .delete(format!("{}/runtime/entity-links/el-1", base_url))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(delete.status(), reqwest::StatusCode::NO_CONTENT);

    let after = client
        .get(format!("{}/runtime/entity-links?scopeId=scope-1", base_url))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    let after_body: Value = after.json().await.unwrap();
    assert_eq!(after_body.as_array().unwrap().len(), 1);
    assert_eq!(after_body[0]["id"], "el-2");
}
