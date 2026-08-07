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
async fn entity_link_rest_endpoints_cover_create_query_and_delete() {
    let (_engine, base_url, client) = spawn_server("rest-entity-link-contract").await;

    let create = client
        .post(format!("{}/runtime/entity-links", base_url))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "id": "entity-link-1",
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
    assert_eq!(created["id"], "entity-link-1");
    assert_eq!(created["linkType"], "reference");
    assert_eq!(created["scopeId"], "scope-1");
    assert_eq!(created["referenceScopeId"], "ref-1");
    assert_eq!(created["hierarchyType"], "child");

    let list_by_scope = client
        .get(format!(
            "{}/runtime/entity-links?scopeId=scope-1&scopeType=processInstance",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert!(list_by_scope.status().is_success());
    let list_by_scope_body: Value = list_by_scope.json().await.unwrap();
    assert_eq!(list_by_scope_body.as_array().unwrap().len(), 1);
    assert_eq!(list_by_scope_body[0]["id"], "entity-link-1");

    let list_by_reference = client
        .get(format!(
            "{}/runtime/entity-links?referenceScopeId=ref-1&referenceScopeType=task",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert!(list_by_reference.status().is_success());
    let list_by_reference_body: Value = list_by_reference.json().await.unwrap();
    assert_eq!(list_by_reference_body.as_array().unwrap().len(), 1);
    assert_eq!(list_by_reference_body[0]["linkType"], "reference");

    let list_by_deprecated_query_alias = client
        .get(format!(
            "{}/runtime/entity-links?scope_id=scope-1&scope_type=processInstance&link_type=reference",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(
        list_by_deprecated_query_alias.status(),
        reqwest::StatusCode::BAD_REQUEST
    );
    let deprecated_query_body: Value = list_by_deprecated_query_alias.json().await.unwrap();
    assert_eq!(deprecated_query_body["code"], "BAD_REQUEST");
    assert!(
        deprecated_query_body["details"]
            .as_str()
            .unwrap()
            .contains("scope_id")
            || deprecated_query_body["details"]
                .as_str()
                .unwrap()
                .contains("scope_type")
            || deprecated_query_body["details"]
                .as_str()
                .unwrap()
                .contains("link_type")
            || deprecated_query_body["details"]
                .as_str()
                .unwrap()
                .contains("unknown field")
            || deprecated_query_body["details"]
                .as_str()
                .unwrap()
                .contains("unknown variant")
            || deprecated_query_body["details"]
                .as_str()
                .unwrap()
                .contains("Invalid query parameters"),
        "details: {}",
        deprecated_query_body["details"]
    );

    let delete = client
        .delete(format!("{}/runtime/entity-links/entity-link-1", base_url))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(delete.status(), reqwest::StatusCode::NO_CONTENT);

    let list_after_delete = client
        .get(format!("{}/runtime/entity-links?scopeId=scope-1", base_url))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(list_after_delete.status().is_success());
    let list_after_delete_body: Value = list_after_delete.json().await.unwrap();
    assert!(list_after_delete_body.as_array().unwrap().is_empty());
}
