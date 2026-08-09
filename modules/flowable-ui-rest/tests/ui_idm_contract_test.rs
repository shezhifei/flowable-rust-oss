//! Contract tests for `/idm-app/rest/**`, asserting response shapes field for
//! field against the Java representations.
//!
//! These run with `AuthMode::Disabled` so the dev identity is injected and every
//! test is about the endpoint rather than the cookie. The auth path itself is
//! covered by `ui_auth_contract_test`.

use std::sync::Arc;

use axum::extract::Extension;
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::identity::entities::{Group, Privilege, User};
use flowable_ui_rest::auth::{AuthMode, UiAuthConfig};
use serde_json::{Value, json};
use tokio::net::TcpListener;

const REST: &str = "/idm-app/rest";

async fn spawn(test_name: &str) -> (Arc<ProcessEngine>, String, reqwest::Client) {
    let engine = Arc::new(ProcessEngine::new(test_name.to_string()));
    save_user(&engine, "admin", Some("Ad"), Some("Min"), Some("admin@example.com"));

    let config = UiAuthConfig {
        mode: AuthMode::Disabled,
        ..UiAuthConfig::default()
    };
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let app = flowable_ui_rest::ui_router_with_config(Arc::new(config))
        .layer(Extension(Arc::clone(&engine)));

    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    (engine, base_url, reqwest::Client::new())
}

fn save_user(
    engine: &Arc<ProcessEngine>,
    id: &str,
    first: Option<&str>,
    last: Option<&str>,
    email: Option<&str>,
) {
    engine.get_identity_service().save_user(User {
        id: id.to_string(),
        first_name: first.map(str::to_string),
        last_name: last.map(str::to_string),
        email: email.map(str::to_string),
        password: Some("test".to_string()),
        tenant_id: None,
    });
}

fn save_group(engine: &Arc<ProcessEngine>, id: &str, name: &str) {
    engine.get_identity_service().save_group(Group {
        id: id.to_string(),
        name: name.to_string(),
        group_type: Some("assignment".to_string()),
    });
}

fn save_privilege(engine: &Arc<ProcessEngine>, id: &str, name: &str) {
    engine.get_identity_service().save_privilege(Privilege {
        id: id.to_string(),
        name: name.to_string(),
    });
}

// ── Account ──

#[tokio::test]
async fn account_carries_every_java_field_including_nulls() {
    let (engine, base_url, client) = spawn("idm_account_shape").await;
    save_group(&engine, "sales", "Sales");
    engine
        .get_identity_service()
        .create_membership("admin".to_string(), "sales".to_string());
    save_privilege(&engine, "priv-idm", "access-idm");
    engine
        .get_identity_service()
        .add_user_privilege_mapping("priv-idm".to_string(), "admin".to_string());

    let body: Value = client
        .get(format!("{base_url}{REST}/account"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert_eq!(body["id"], "admin");
    assert_eq!(body["firstName"], "Ad");
    assert_eq!(body["lastName"], "Min");
    assert_eq!(body["email"], "admin@example.com");
    assert_eq!(body["fullName"], "Ad Min");
    // Jackson's Include.ALWAYS: a null tenant is emitted, not omitted.
    assert!(body.get("tenantId").is_some(), "tenantId must be present");
    assert!(body["tenantId"].is_null());

    // Privileges are the *names*, not the ids.
    assert_eq!(body["privileges"], json!(["access-idm"]));
    assert_eq!(body["groups"][0]["id"], "sales");
    assert_eq!(body["groups"][0]["name"], "Sales");
    assert_eq!(body["groups"][0]["type"], "assignment");
}

#[tokio::test]
async fn full_name_of_a_user_with_no_names_is_a_single_space() {
    let (engine, base_url, client) = spawn("idm_account_blank_name").await;
    save_user(&engine, "admin", None, None, None);

    let body: Value = client
        .get(format!("{base_url}{REST}/account"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    // Java concatenates first + " " + last with nulls coerced to "", so the
    // result is one space rather than an empty string.
    assert_eq!(body["fullName"], " ");
    assert!(body["firstName"].is_null());
}

#[tokio::test]
async fn authenticate_returns_only_the_login() {
    let (_engine, base_url, client) = spawn("idm_authenticate_shape").await;

    let body: Value = client
        .get(format!("{base_url}{REST}/authenticate"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert_eq!(body, json!({ "login": "admin" }));
}

// ── Users ──

#[tokio::test]
async fn user_list_shape_and_total() {
    let (engine, base_url, client) = spawn("idm_users_list").await;
    save_user(&engine, "bob", Some("Bob"), Some("Baker"), Some("bob@x.com"));

    let body: Value = client
        .get(format!("{base_url}{REST}/admin/users"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert_eq!(body["total"], 2);
    assert_eq!(body["size"], 2);
    assert_eq!(body["start"], 0);
    assert_eq!(body["data"].as_array().unwrap().len(), 2);
    // List entries use the plain constructor: groups and privileges stay empty.
    assert_eq!(body["data"][0]["groups"], json!([]));
    assert_eq!(body["data"][0]["privileges"], json!([]));
}

#[tokio::test]
async fn user_filter_matches_full_name_case_insensitively() {
    let (engine, base_url, client) = spawn("idm_users_filter").await;
    save_user(&engine, "bob", Some("Bob"), Some("Baker"), None);
    save_user(&engine, "carol", Some("Carol"), Some("Smith"), None);

    // Matches across the first/last boundary, which is what the SQL
    // concat-then-LIKE does.
    let body: Value = client
        .get(format!("{base_url}{REST}/admin/users?filter=ob%20bak"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert_eq!(body["total"], 1);
    assert_eq!(body["data"][0]["id"], "bob");

    // Case folding.
    let body: Value = client
        .get(format!("{base_url}{REST}/admin/users?filter=CAROL"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(body["total"], 1);
    assert_eq!(body["data"][0]["id"], "carol");
}

#[tokio::test]
async fn a_user_with_no_surname_is_still_findable_by_first_name() {
    let (engine, base_url, client) = spawn("idm_users_filter_null_half").await;
    save_user(&engine, "dave", Some("Dave"), None, None);

    let body: Value = client
        .get(format!("{base_url}{REST}/admin/users?filter=dave"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    // Deliberate: a null name half is treated as empty. Java's answer here
    // depends on the database — MySQL's CONCAT yields null and drops the row,
    // PostgreSQL's skips the null and keeps it. This follows PostgreSQL.
    assert_eq!(body["total"], 1);
    assert_eq!(body["data"][0]["id"], "dave");
}

#[tokio::test]
async fn user_sort_and_start_parameters() {
    let (engine, base_url, client) = spawn("idm_users_sort").await;
    save_user(&engine, "bob", Some("Bob"), None, Some("b@x.com"));
    save_user(&engine, "carol", Some("Carol"), None, Some("a@x.com"));

    let ids = |body: &Value| -> Vec<String> {
        body["data"]
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| entry["id"].as_str().unwrap().to_string())
            .collect()
    };

    let body: Value = client
        .get(format!("{base_url}{REST}/admin/users?sort=idDesc"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(ids(&body), vec!["carol", "bob", "admin"]);

    // a@x.com (carol) < admin@example.com (admin) < b@x.com (bob).
    let body: Value = client
        .get(format!("{base_url}{REST}/admin/users?sort=emailAsc"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(ids(&body), vec!["carol", "admin", "bob"]);

    // `start` skips rows but `total` stays the unpaged count.
    let body: Value = client
        .get(format!("{base_url}{REST}/admin/users?sort=idAsc&start=2"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(body["total"], 3);
    assert_eq!(body["start"], 2);
    assert_eq!(body["size"], 1);
    assert_eq!(ids(&body), vec!["carol"]);
}

#[tokio::test]
async fn create_user_requires_id_password_and_first_name() {
    let (_engine, base_url, client) = spawn("idm_users_create_validation").await;

    for payload in [
        json!({ "firstName": "No", "password": "pw" }),               // no id
        json!({ "id": "x", "firstName": "No" }),                      // no password
        json!({ "id": "x", "password": "pw" }),                       // no first name
        json!({ "id": "   ", "firstName": "No", "password": "pw" }),  // blank id
    ] {
        let response = client
            .post(format!("{base_url}{REST}/admin/users"))
            .json(&payload)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 400, "payload {payload} should be rejected");
        let body: Value = response.json().await.unwrap();
        assert_eq!(body["message"], "Id, password and first name are required");
        assert_eq!(body["messageKey"], "GENERAL.ERROR.BAD-REQUEST");
    }
}

#[tokio::test]
async fn create_user_returns_the_entity_without_the_password() {
    let (engine, base_url, client) = spawn("idm_users_create").await;

    let response = client
        .post(format!("{base_url}{REST}/admin/users"))
        .json(&json!({
            "id": "dave",
            "firstName": "Dave",
            "lastName": "Jones",
            "email": "dave@x.com",
            "password": "secret",
            "tenantId": "acme"
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["id"], "dave");
    assert_eq!(body["fullName"], "Dave Jones");
    assert_eq!(body["tenantId"], "acme");
    assert!(
        body.get("password").is_none(),
        "the representation must not expose a password"
    );

    // The stored password is hashed, and verifies against the plaintext.
    let stored = engine.get_identity_service().find_user_by_id("dave").unwrap();
    assert_ne!(stored.password.as_deref(), Some("secret"));
    assert!(
        engine
            .get_identity_service()
            .check_password("dave", "secret")
    );
}

#[tokio::test]
async fn duplicate_id_and_email_both_conflict_with_the_signup_message_key() {
    let (engine, base_url, client) = spawn("idm_users_conflict").await;
    save_user(&engine, "dave", Some("Dave"), None, Some("dave@x.com"));

    // Same id.
    let response = client
        .post(format!("{base_url}{REST}/admin/users"))
        .json(&json!({ "id": "dave", "firstName": "D", "password": "pw" }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 409);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["message"], "User already registered");
    assert_eq!(body["messageKey"], "ACCOUNT.SIGNUP.ERROR.ALREADY-REGISTERED");

    // Same email, different id.
    let response = client
        .post(format!("{base_url}{REST}/admin/users"))
        .json(&json!({
            "id": "other", "firstName": "O", "password": "pw", "email": "dave@x.com"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 409);
}

#[tokio::test]
async fn update_user_overwrites_fields_including_with_null() {
    let (engine, base_url, client) = spawn("idm_users_update").await;
    save_user(&engine, "bob", Some("Bob"), Some("Baker"), Some("bob@x.com"));

    let response = client
        .put(format!("{base_url}{REST}/admin/users/bob"))
        .json(&json!({ "firstName": "Robert", "lastName": null, "email": "r@x.com" }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);

    let stored = engine.get_identity_service().find_user_by_id("bob").unwrap();
    assert_eq!(stored.first_name.as_deref(), Some("Robert"));
    // Java calls setLastName(null) unconditionally, so an omitted field clears.
    assert_eq!(stored.last_name, None);
    assert_eq!(stored.email.as_deref(), Some("r@x.com"));
    // The password survives the update and still verifies — the loaded hash must
    // not be re-hashed on save.
    assert!(engine.get_identity_service().check_password("bob", "test"));
}

#[tokio::test]
async fn update_of_a_missing_user_is_silently_accepted() {
    let (_engine, base_url, client) = spawn("idm_users_update_missing").await;

    // Java's service guards with `if (user != null)` and returns 200 regardless.
    let response = client
        .put(format!("{base_url}{REST}/admin/users/nosuchuser"))
        .json(&json!({ "firstName": "X" }))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
}

#[tokio::test]
async fn bulk_password_update_skips_unknown_ids() {
    let (engine, base_url, client) = spawn("idm_users_bulk").await;
    save_user(&engine, "bob", Some("Bob"), None, None);

    let response = client
        .put(format!("{base_url}{REST}/admin/users"))
        .json(&json!({ "users": ["bob", "ghost"], "password": "newpw" }))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let identity = engine.get_identity_service();
    assert!(identity.check_password("bob", "newpw"));
    assert!(!identity.check_password("bob", "test"));
    assert!(identity.find_user_by_id("ghost").is_none());
}

#[tokio::test]
async fn delete_user_cascades_memberships_and_direct_privileges_only() {
    let (engine, base_url, client) = spawn("idm_users_delete_cascade").await;
    let identity = engine.get_identity_service();

    save_user(&engine, "bob", Some("Bob"), None, None);
    save_group(&engine, "sales", "Sales");
    identity.create_membership("bob".to_string(), "sales".to_string());

    save_privilege(&engine, "priv-direct", "direct");
    identity.add_user_privilege_mapping("priv-direct".to_string(), "bob".to_string());
    save_privilege(&engine, "priv-group", "viaGroup");
    identity.add_group_privilege_mapping("priv-group".to_string(), "sales".to_string());

    let response = client
        .delete(format!("{base_url}{REST}/admin/users/bob"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);

    assert!(identity.find_user_by_id("bob").is_none());
    assert!(!identity.membership_exists("bob", "sales"));
    assert!(
        identity.get_privilege_mapping_ids("priv-direct").0.is_empty(),
        "the user's own grant must be revoked"
    );
    // The group's grant belongs to the group and must survive.
    assert_eq!(
        identity.get_privilege_mapping_ids("priv-group").1,
        vec!["sales".to_string()],
        "deleting a member must not revoke the group's privilege"
    );
}

// ── Groups ──

#[tokio::test]
async fn group_list_is_a_bare_array_sorted_by_name() {
    let (engine, base_url, client) = spawn("idm_groups_list").await;
    save_group(&engine, "z", "Zebra");
    save_group(&engine, "a", "Antelope");

    let body: Value = client
        .get(format!("{base_url}{REST}/admin/groups"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    // Not a ResultListDataRepresentation — this endpoint has no paging.
    let entries = body.as_array().expect("groups must serialise as an array");
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0]["name"], "Antelope");
    assert_eq!(entries[1]["name"], "Zebra");
}

#[tokio::test]
async fn group_filter_is_case_insensitive() {
    let (engine, base_url, client) = spawn("idm_groups_filter").await;
    save_group(&engine, "sales", "Sales");
    save_group(&engine, "eng", "Engineering");

    let body: Value = client
        .get(format!("{base_url}{REST}/admin/groups?filter=SALE"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert_eq!(body.as_array().unwrap().len(), 1);
    assert_eq!(body[0]["id"], "sales");
}

#[tokio::test]
async fn create_group_defaults_the_type_and_requires_a_name() {
    let (_engine, base_url, client) = spawn("idm_groups_create").await;

    let body: Value = client
        .post(format!("{base_url}{REST}/admin/groups"))
        .json(&json!({ "id": "ops", "name": "Ops" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(body["type"], "assignment");

    let response = client
        .post(format!("{base_url}{REST}/admin/groups"))
        .json(&json!({ "id": "noname" }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 400);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["message"], "Group name required");
}

#[tokio::test]
async fn get_and_update_and_delete_of_a_missing_group_are_404() {
    let (_engine, base_url, client) = spawn("idm_groups_missing").await;

    let response = client
        .get(format!("{base_url}{REST}/admin/groups/ghost"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 404);
    let body: Value = response.json().await.unwrap();
    // `new NotFoundException()` leaves the message null, and Jackson emits it.
    assert!(body.get("message").is_some());
    assert!(body["message"].is_null());
    assert_eq!(body["messageKey"], "GENERAL.ERROR.NOT-FOUND");

    let response = client
        .put(format!("{base_url}{REST}/admin/groups/ghost"))
        .json(&json!({ "name": "New" }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 404);

    let response = client
        .delete(format!("{base_url}{REST}/admin/groups/ghost"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 404);
}

#[tokio::test]
async fn update_group_changes_only_the_name() {
    let (engine, base_url, client) = spawn("idm_groups_update").await;
    save_group(&engine, "sales", "Sales");

    let body: Value = client
        .put(format!("{base_url}{REST}/admin/groups/sales"))
        .json(&json!({ "name": "Sales EMEA", "type": "security-role" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert_eq!(body["name"], "Sales EMEA");
    // The type is not reachable through this endpoint.
    assert_eq!(body["type"], "assignment");
}

#[tokio::test]
async fn group_users_defaults_page_and_page_size() {
    let (engine, base_url, client) = spawn("idm_group_users_defaults").await;
    let identity = engine.get_identity_service();
    save_group(&engine, "sales", "Sales");
    for index in 0..3 {
        let id = format!("user{index}");
        save_user(&engine, &id, Some("User"), Some(&index.to_string()), None);
        identity.create_membership(id, "sales".to_string());
    }

    // Both parameters omitted. Java would throw NullPointerException computing
    // `page * pageSize`; this port defaults to 0/50.
    let response = client
        .get(format!("{base_url}{REST}/admin/groups/sales/users"))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["total"], 3);
    assert_eq!(body["size"], 3);
    assert_eq!(body["start"], 0);
}

#[tokio::test]
async fn group_users_paging_and_filter() {
    let (engine, base_url, client) = spawn("idm_group_users_paging").await;
    let identity = engine.get_identity_service();
    save_group(&engine, "sales", "Sales");
    for name in ["Anna", "Brian", "Clara"] {
        let id = name.to_lowercase();
        save_user(&engine, &id, Some(name), Some("Stone"), None);
        identity.create_membership(id, "sales".to_string());
    }

    let body: Value = client
        .get(format!("{base_url}{REST}/admin/groups/sales/users?page=1&pageSize=2"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert_eq!(body["total"], 3);
    assert_eq!(body["start"], 2, "start is page * pageSize");
    assert_eq!(body["size"], 1);

    let body: Value = client
        .get(format!("{base_url}{REST}/admin/groups/sales/users?filter=bri"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(body["total"], 1);
    assert_eq!(body["data"][0]["id"], "brian");
}

#[tokio::test]
async fn group_membership_add_and_delete_require_both_sides() {
    let (engine, base_url, client) = spawn("idm_group_members").await;
    save_group(&engine, "sales", "Sales");
    save_user(&engine, "bob", Some("Bob"), None, None);

    let response = client
        .post(format!("{base_url}{REST}/admin/groups/sales/members/bob"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert!(engine.get_identity_service().membership_exists("bob", "sales"));

    for path in [
        "/admin/groups/ghost/members/bob",
        "/admin/groups/sales/members/ghost",
    ] {
        let response = client
            .post(format!("{base_url}{REST}{path}"))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 404, "{path} should 404");
    }

    let response = client
        .delete(format!("{base_url}{REST}/admin/groups/sales/members/bob"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert!(!engine.get_identity_service().membership_exists("bob", "sales"));
}

// ── Privileges ──

#[tokio::test]
async fn privilege_list_leaves_users_and_groups_null() {
    let (engine, base_url, client) = spawn("idm_privileges_list").await;
    save_privilege(&engine, "priv-idm", "access-idm");
    engine
        .get_identity_service()
        .add_user_privilege_mapping("priv-idm".to_string(), "admin".to_string());

    let body: Value = client
        .get(format!("{base_url}{REST}/admin/privileges"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert_eq!(body[0]["id"], "priv-idm");
    assert_eq!(body[0]["name"], "access-idm");
    // Java's two-argument constructor leaves both collections unset, and
    // Include.ALWAYS emits them as null — not as empty arrays.
    assert!(body[0]["users"].is_null(), "users must be null on the list");
    assert!(body[0]["groups"].is_null(), "groups must be null on the list");
}

#[tokio::test]
async fn single_privilege_populates_users_and_groups() {
    let (engine, base_url, client) = spawn("idm_privileges_detail").await;
    let identity = engine.get_identity_service();
    save_privilege(&engine, "priv-idm", "access-idm");
    save_group(&engine, "sales", "Sales");
    identity.add_user_privilege_mapping("priv-idm".to_string(), "admin".to_string());
    identity.add_group_privilege_mapping("priv-idm".to_string(), "sales".to_string());

    let body: Value = client
        .get(format!("{base_url}{REST}/admin/privileges/priv-idm"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert_eq!(body["users"].as_array().unwrap().len(), 1);
    assert_eq!(body["users"][0]["id"], "admin");
    assert_eq!(body["groups"].as_array().unwrap().len(), 1);
    assert_eq!(body["groups"][0]["id"], "sales");

    // The two collection endpoints return exactly those sub-documents.
    let users: Value = client
        .get(format!("{base_url}{REST}/admin/privileges/priv-idm/users"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(users, body["users"]);

    let groups: Value = client
        .get(format!("{base_url}{REST}/admin/privileges/priv-idm/groups"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(groups, body["groups"]);
}

#[tokio::test]
async fn missing_privilege_is_404_on_all_three_read_endpoints() {
    let (_engine, base_url, client) = spawn("idm_privileges_missing").await;

    for path in [
        "/admin/privileges/ghost",
        "/admin/privileges/ghost/users",
        "/admin/privileges/ghost/groups",
    ] {
        let response = client
            .get(format!("{base_url}{REST}{path}"))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 404, "{path} should 404");
    }
}

#[tokio::test]
async fn granting_a_privilege_is_idempotent_and_validates_the_subject() {
    let (engine, base_url, client) = spawn("idm_privileges_grant").await;
    save_privilege(&engine, "priv-idm", "access-idm");
    save_group(&engine, "sales", "Sales");

    // Twice, to prove the second is a no-op rather than a duplicate row.
    for _ in 0..2 {
        let response = client
            .post(format!("{base_url}{REST}/admin/privileges/priv-idm/users"))
            .json(&json!({ "userId": "admin" }))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
    }
    let (users, _) = engine
        .get_identity_service()
        .get_privilege_mapping_ids("priv-idm");
    assert_eq!(users, vec!["admin".to_string()]);

    for _ in 0..2 {
        let response = client
            .post(format!("{base_url}{REST}/admin/privileges/priv-idm/groups"))
            .json(&json!({ "groupId": "sales" }))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
    }
    let (_, groups) = engine
        .get_identity_service()
        .get_privilege_mapping_ids("priv-idm");
    assert_eq!(groups, vec!["sales".to_string()]);

    // An unknown subject is the caller's fault: 400, where Java lets an
    // IllegalArgumentException escape as a 500.
    let response = client
        .post(format!("{base_url}{REST}/admin/privileges/priv-idm/users"))
        .json(&json!({ "userId": "ghost" }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 400);
    assert_eq!(
        response.json::<Value>().await.unwrap()["message"],
        "Invalid user id"
    );

    let response = client
        .post(format!("{base_url}{REST}/admin/privileges/priv-idm/groups"))
        .json(&json!({ "groupId": "ghost" }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 400);
}

#[tokio::test]
async fn revoking_a_privilege_removes_only_the_named_mapping() {
    let (engine, base_url, client) = spawn("idm_privileges_revoke").await;
    let identity = engine.get_identity_service();
    save_privilege(&engine, "priv-idm", "access-idm");
    save_user(&engine, "bob", Some("Bob"), None, None);
    save_group(&engine, "sales", "Sales");
    identity.add_user_privilege_mapping("priv-idm".to_string(), "admin".to_string());
    identity.add_user_privilege_mapping("priv-idm".to_string(), "bob".to_string());
    identity.add_group_privilege_mapping("priv-idm".to_string(), "sales".to_string());

    let response = client
        .delete(format!("{base_url}{REST}/admin/privileges/priv-idm/users/bob"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);

    let (users, groups) = identity.get_privilege_mapping_ids("priv-idm");
    assert_eq!(users, vec!["admin".to_string()]);
    assert_eq!(groups, vec!["sales".to_string()]);

    let response = client
        .delete(format!("{base_url}{REST}/admin/privileges/priv-idm/groups/sales"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert!(identity.get_privilege_mapping_ids("priv-idm").1.is_empty());
}

// ── Profile ──

#[tokio::test]
async fn profile_matches_the_account_body() {
    let (engine, base_url, client) = spawn("idm_profile_get").await;
    save_group(&engine, "sales", "Sales");
    engine
        .get_identity_service()
        .create_membership("admin".to_string(), "sales".to_string());

    let profile: Value = client
        .get(format!("{base_url}{REST}/admin/profile"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let account: Value = client
        .get(format!("{base_url}{REST}/account"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert_eq!(profile, account);
}

#[tokio::test]
async fn update_profile_rejects_an_empty_email_and_ignores_the_body_id() {
    let (engine, base_url, client) = spawn("idm_profile_update").await;
    save_user(&engine, "bob", Some("Bob"), None, Some("bob@x.com"));

    let response = client
        .post(format!("{base_url}{REST}/admin/profile"))
        .json(&json!({ "firstName": "A", "email": "" }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 400);
    assert_eq!(
        response.json::<Value>().await.unwrap()["message"],
        "Empty email is not allowed"
    );

    // A body naming another user still updates only the caller.
    let body: Value = client
        .post(format!("{base_url}{REST}/admin/profile"))
        .json(&json!({
            "id": "bob", "firstName": "Adam", "lastName": "Ministrator",
            "email": "adam@x.com", "tenantId": "hijack"
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert_eq!(body["id"], "admin");
    assert_eq!(body["fullName"], "Adam Ministrator");
    // tenantId is not updatable here.
    assert!(body["tenantId"].is_null());

    let bob = engine.get_identity_service().find_user_by_id("bob").unwrap();
    assert_eq!(bob.first_name.as_deref(), Some("Bob"), "bob must be untouched");
}

#[tokio::test]
async fn change_password_is_404_on_a_wrong_current_password() {
    let (engine, base_url, client) = spawn("idm_profile_password").await;

    let response = client
        .post(format!("{base_url}{REST}/admin/profile-password"))
        .json(&json!({ "originalPassword": "wrong", "newPassword": "next" }))
        .send()
        .await
        .unwrap();
    // Java throws NotFoundException here, so it is a 404 and not a 401 or 403.
    assert_eq!(response.status(), 404);
    assert!(
        engine.get_identity_service().check_password("admin", "test"),
        "a failed attempt must not change the password"
    );

    let response = client
        .post(format!("{base_url}{REST}/admin/profile-password"))
        .json(&json!({ "originalPassword": "test", "newPassword": "next" }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);

    let identity = engine.get_identity_service();
    assert!(identity.check_password("admin", "next"));
    assert!(!identity.check_password("admin", "test"));
}

#[tokio::test]
async fn profile_picture_round_trip_and_404_when_absent() {
    let (_engine, base_url, client) = spawn("idm_profile_picture").await;

    let response = client
        .get(format!("{base_url}{REST}/admin/profile-picture"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 404);

    let png = vec![0x89u8, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
    let form = reqwest::multipart::Form::new().part(
        "file",
        reqwest::multipart::Part::bytes(png.clone())
            .file_name("me.png")
            .mime_str("image/png")
            .unwrap(),
    );
    let response = client
        .post(format!("{base_url}{REST}/admin/profile-picture"))
        .multipart(form)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);

    let response = client
        .get(format!("{base_url}{REST}/admin/profile-picture"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(
        response.headers().get("content-type").unwrap(),
        "image/png",
        "the stored mime type must come back verbatim"
    );
    assert_eq!(response.bytes().await.unwrap().to_vec(), png);
}

#[tokio::test]
async fn profile_picture_upload_without_a_file_part_is_400() {
    let (_engine, base_url, client) = spawn("idm_profile_picture_nofile").await;

    let form = reqwest::multipart::Form::new().text("notfile", "x");
    let response = client
        .post(format!("{base_url}{REST}/admin/profile-picture"))
        .multipart(form)
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 400);
}
