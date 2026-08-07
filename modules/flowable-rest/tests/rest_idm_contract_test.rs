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
async fn group_membership_lifecycle_lists_group_members_with_filters_sorting_and_paging() {
    let (_engine, base_url, client) = spawn_server("rest-idm-membership-contract").await;

    for (id, first_name, last_name, email) in [
        ("kermit", "Kermit", "Frog", "kermit@muppets.test"),
        ("fozzie", "Fozzie", "Bear", "fozzie@muppets.test"),
        ("gonzo", "Gonzo", "Great", "gonzo@muppets.test"),
    ] {
        let response = client
            .post(format!("{}/identity/users", base_url))
            .basic_auth("admin", Some("test"))
            .json(&json!({
                "id": id,
                "firstName": first_name,
                "lastName": last_name,
                "email": email,
                "password": "secret"
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::CREATED);
    }

    for (id, name) in [("performers", "Performers"), ("auditors", "Auditors")] {
        let response = client
            .post(format!("{}/identity/groups", base_url))
            .basic_auth("admin", Some("test"))
            .json(&json!({
                "id": id,
                "name": name,
                "type": "security-role"
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::CREATED);
    }

    for (user_id, group_id) in [
        ("kermit", "performers"),
        ("fozzie", "performers"),
        ("gonzo", "auditors"),
    ] {
        let response = client
            .post(format!("{}/identity/memberships", base_url))
            .basic_auth("admin", Some("test"))
            .json(&json!({
                "userId": user_id,
                "groupId": group_id
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::CREATED);
    }

    let duplicate = client
        .post(format!("{}/identity/memberships", base_url))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "userId": "kermit",
            "groupId": "performers"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(duplicate.status(), reqwest::StatusCode::CONFLICT);

    let paged_members = client
        .get(format!(
            "{}/identity/groups/performers/members?sort=lastName&order=asc&start=0&size=1",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(paged_members.status(), reqwest::StatusCode::OK);
    let paged_members_body: Value = paged_members.json().await.unwrap();
    assert_eq!(paged_members_body["total"], 2);
    assert_eq!(paged_members_body["start"], 0);
    assert_eq!(paged_members_body["size"], 1);
    assert_eq!(paged_members_body["sort"], "lastName");
    assert_eq!(paged_members_body["order"], "asc");
    assert_eq!(paged_members_body["data"][0]["id"], "fozzie");
    assert_eq!(paged_members_body["data"][0]["displayName"], "Fozzie Bear");

    let canonical_filtered_members = client
        .get(format!(
            "{}/groups/performers/members?firstNameLike=kerm",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(canonical_filtered_members.status(), reqwest::StatusCode::OK);
    let canonical_filtered_members_body: Value = canonical_filtered_members.json().await.unwrap();
    assert_eq!(canonical_filtered_members_body["total"], 1);
    assert_eq!(canonical_filtered_members_body["data"][0]["id"], "kermit");

    let missing_group = client
        .get(format!("{}/identity/groups/missing/members", base_url))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(missing_group.status(), reqwest::StatusCode::NOT_FOUND);

    let conflicting_group_filter = client
        .get(format!(
            "{}/identity/groups/performers/members?memberOfGroup=auditors",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(
        conflicting_group_filter.status(),
        reqwest::StatusCode::BAD_REQUEST
    );

    let missing_user_membership = client
        .post(format!("{}/identity/memberships", base_url))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "userId": "missing",
            "groupId": "performers"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        missing_user_membership.status(),
        reqwest::StatusCode::NOT_FOUND
    );
}
