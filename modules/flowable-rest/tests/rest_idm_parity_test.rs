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
async fn rest_idm_user_crud_and_query_parity() {
    let (_engine, base_url, client) = spawn_server("rest-idm-parity").await;

    for (id, first, last, email) in [
        ("kermit", "Kermit", "Frog", "kermit@muppets.test"),
        ("fozzie", "Fozzie", "Bear", "fozzie@muppets.test"),
    ] {
        let resp = client
            .post(format!("{}/identity/users", base_url))
            .basic_auth("admin", Some("test"))
            .json(&json!({
                "id": id,
                "firstName": first,
                "lastName": last,
                "email": email,
                "password": "secret"
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), reqwest::StatusCode::CREATED);
    }

    let list = client
        .get(format!("{}/identity/users", base_url))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(list.status(), reqwest::StatusCode::OK);
    let body: Value = list.json().await.unwrap();
    assert_eq!(body["data"].as_array().unwrap().len(), 3);

    let get = client
        .get(format!("{}/identity/users/kermit", base_url))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(get.status(), reqwest::StatusCode::OK);
    let user: Value = get.json().await.unwrap();
    assert_eq!(user["firstName"], "Kermit");

    let delete = client
        .delete(format!("{}/identity/users/fozzie", base_url))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(delete.status(), reqwest::StatusCode::NO_CONTENT);

    let missing = client
        .get(format!("{}/identity/users/fozzie", base_url))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(missing.status(), reqwest::StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn rest_idm_group_crud_and_membership_parity() {
    let (_engine, base_url, client) = spawn_server("rest-idm-group-parity").await;

    client
        .post(format!("{}/identity/users", base_url))
        .basic_auth("admin", Some("test"))
        .json(&json!({"id": "kermit", "password": "secret"}))
        .send()
        .await
        .unwrap();

    let resp = client
        .post(format!("{}/identity/groups", base_url))
        .basic_auth("admin", Some("test"))
        .json(&json!({"id": "muppets", "name": "Muppets", "type": "assignment"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::CREATED);

    let membership = client
        .post(format!("{}/identity/memberships", base_url))
        .basic_auth("admin", Some("test"))
        .json(&json!({"userId": "kermit", "groupId": "muppets"}))
        .send()
        .await
        .unwrap();
    assert_eq!(membership.status(), reqwest::StatusCode::CREATED);

    let members = client
        .get(format!("{}/identity/groups/muppets/members", base_url))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(members.status(), reqwest::StatusCode::OK);
    let body: Value = members.json().await.unwrap();
    assert_eq!(body["data"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn rest_idm_privilege_crud_parity() {
    let (_engine, base_url, client) = spawn_server("rest-idm-privilege-parity").await;

    let resp = client
        .post(format!("{}/identity/privileges", base_url))
        .basic_auth("admin", Some("test"))
        .json(&json!({"id": "admin-priv", "name": "Administrator"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::OK);

    let list = client
        .get(format!("{}/identity/privileges", base_url))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(list.status(), reqwest::StatusCode::OK);
    let body: Value = list.json().await.unwrap();
    assert!(!body["data"].as_array().unwrap().is_empty());

    let get = client
        .get(format!("{}/identity/privileges/admin-priv", base_url))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(get.status(), reqwest::StatusCode::OK);
    let priv_data: Value = get.json().await.unwrap();
    assert_eq!(priv_data["name"], "Administrator");
}
