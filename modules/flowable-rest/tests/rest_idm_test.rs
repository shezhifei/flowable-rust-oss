use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::engine::time_source::SystemTimeSource;
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
        .save_user(flowable_engine::identity::entities::User {
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
async fn idm_paths_cover_users_groups_privileges_and_engine_info() {
    let engine = build_engine("rest-idm-native-test");
    let (base_url, client) = spawn_server(Arc::clone(&engine)).await;

    let create_user = client
        .post(format!("{}/users", base_url))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "id": "kermit",
            "firstName": "Kermit",
            "lastName": "The Frog",
            "email": "kermit@example.test",
            "password": "secret"
        }))
        .send()
        .await
        .unwrap();
    assert!(create_user.status().is_success());
    let create_user_body: Value = create_user.json().await.unwrap();
    assert_eq!(create_user_body["id"], "kermit");
    assert_eq!(create_user_body["firstName"], "Kermit");
    assert_eq!(create_user_body["displayName"], "Kermit The Frog");
    // Security deviation from Java: the create response never echoes the password.
    assert!(create_user_body.get("password").is_none());
    assert!(create_user_body["tenantId"].is_null());
    assert!(create_user_body["pictureUrl"].is_null());
    assert!(
        create_user_body["url"]
            .as_str()
            .unwrap()
            .ends_with("/identity/users/kermit")
    );

    let update_user = client
        .put(format!("{}/users/kermit", base_url))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "firstName": "Kermit",
            "lastName": "Updated",
            "email": "updated@example.test"
        }))
        .send()
        .await
        .unwrap();
    assert!(update_user.status().is_success());
    let update_user_body = update_user.json::<Value>().await.unwrap();
    assert_eq!(update_user_body["lastName"], "Updated");
    assert!(update_user_body["password"].is_null());

    let create_group = client
        .post(format!("{}/groups", base_url))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "id": "managers",
            "name": "Managers",
            "type": "security-role"
        }))
        .send()
        .await
        .unwrap();
    assert!(create_group.status().is_success());
    let create_group_body: Value = create_group.json().await.unwrap();
    assert_eq!(create_group_body["id"], "managers");
    assert_eq!(create_group_body["type"], "security-role");
    assert!(
        create_group_body["url"]
            .as_str()
            .unwrap()
            .ends_with("/identity/groups/managers")
    );

    let update_identity_user_alias = client
        .put(format!("{}/identity/users/kermit", base_url))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "firstName": "Kermit",
            "lastName": "Identity Alias",
            "email": "identity-alias@example.test"
        }))
        .send()
        .await
        .unwrap();
    assert!(update_identity_user_alias.status().is_success());
    assert_eq!(
        update_identity_user_alias.json::<Value>().await.unwrap()["lastName"],
        "Identity Alias"
    );

    let update_identity_group_alias = client
        .put(format!("{}/identity/groups/managers", base_url))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "Identity Managers",
            "type": "assignment"
        }))
        .send()
        .await
        .unwrap();
    assert!(update_identity_group_alias.status().is_success());
    assert_eq!(
        update_identity_group_alias.json::<Value>().await.unwrap()["type"],
        "assignment"
    );

    let create_membership = client
        .post(format!("{}/groups/managers/members", base_url))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "userId": "kermit"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(create_membership.status(), reqwest::StatusCode::CREATED);

    let create_privilege = client
        .post(format!("{}/identity/privileges", base_url))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "id": "access-idm",
            "name": "Access IDM"
        }))
        .send()
        .await
        .unwrap();
    assert!(create_privilege.status().is_success());

    let add_user_privilege = client
        .post(format!("{}/privileges/access-idm/users", base_url))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "userId": "kermit"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(add_user_privilege.status(), reqwest::StatusCode::OK);

    let add_group_privilege = client
        .post(format!("{}/privileges/access-idm/groups", base_url))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "groupId": "managers"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(add_group_privilege.status(), reqwest::StatusCode::OK);

    let list_privileges = client
        .get(format!("{}/privileges?userId=kermit", base_url))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(list_privileges.status().is_success());
    let list_privileges_body: Value = list_privileges.json().await.unwrap();
    assert_eq!(
        list_privileges_body["data"].as_array().unwrap()[0]["id"],
        "access-idm"
    );
    assert!(list_privileges_body["data"][0]["users"].is_null());
    assert!(list_privileges_body["data"][0]["groups"].is_null());

    let privilege = client
        .get(format!("{}/privileges/access-idm", base_url))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(privilege.status().is_success());
    let privilege_body: Value = privilege.json().await.unwrap();
    assert_eq!(privilege_body["id"], "access-idm");
    assert_eq!(privilege_body["users"][0]["id"], "kermit");
    assert_eq!(
        privilege_body["users"][0]["displayName"],
        "Kermit Identity Alias"
    );
    assert_eq!(privilege_body["groups"][0]["id"], "managers");
    assert_eq!(
        privilege_body["groups"][0]["url"],
        "http://localhost/identity/groups/managers"
    );

    let privilege_users = client
        .get(format!("{}/privileges/access-idm/users", base_url))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(privilege_users.status().is_success());
    let privilege_users_body: Value = privilege_users.json().await.unwrap();
    assert_eq!(privilege_users_body.as_array().unwrap()[0]["id"], "kermit");

    let privilege_groups = client
        .get(format!("{}/privileges/access-idm/groups", base_url))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(privilege_groups.status().is_success());
    let privilege_groups_body: Value = privilege_groups.json().await.unwrap();
    assert_eq!(
        privilege_groups_body.as_array().unwrap()[0]["id"],
        "managers"
    );

    let engine_info = client
        .get(format!("{}/idm-management/engine", base_url))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(engine_info.status().is_success());
    let engine_info_body: Value = engine_info.json().await.unwrap();
    assert_eq!(engine_info_body["name"], "rest-idm-native-test");
    assert!(engine_info_body["version"].is_string());
}

#[tokio::test]
async fn idm_user_shape_supports_null_updates_without_leaking_to_identity_response() {
    let engine = build_engine("rest-idm-user-shape");
    let (base_url, client) = spawn_server(Arc::clone(&engine)).await;

    let create_user = client
        .post(format!("{}/users", base_url))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "id": "rowlf",
            "firstName": "Rowlf",
            "lastName": "Dog",
            "email": "rowlf@example.test",
            "password": "piano"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(create_user.status(), reqwest::StatusCode::CREATED);
    let create_user_body: Value = create_user.json().await.unwrap();
    // Security deviation from Java: the create response never echoes the password.
    assert!(create_user_body.get("password").is_none());
    assert!(create_user_body["tenantId"].is_null());
    assert!(create_user_body["pictureUrl"].is_null());

    let get_rest_user = client
        .get(format!("{}/users/rowlf", base_url))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(get_rest_user.status().is_success());
    let get_rest_user_body: Value = get_rest_user.json().await.unwrap();
    assert!(get_rest_user_body["password"].is_null());
    assert!(get_rest_user_body["tenantId"].is_null());
    assert!(get_rest_user_body["pictureUrl"].is_null());

    let clear_rest_user_fields = client
        .put(format!("{}/users/rowlf", base_url))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "firstName": null,
            "email": null,
            "password": null
        }))
        .send()
        .await
        .unwrap();
    assert!(clear_rest_user_fields.status().is_success());
    let clear_rest_user_fields_body: Value = clear_rest_user_fields.json().await.unwrap();
    assert!(clear_rest_user_fields_body["firstName"].is_null());
    assert_eq!(clear_rest_user_fields_body["displayName"], "Dog");
    assert!(clear_rest_user_fields_body["email"].is_null());
    assert!(clear_rest_user_fields_body["password"].is_null());

    let identity_user = client
        .get(format!("{}/identity/users/rowlf", base_url))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(identity_user.status().is_success());
    let identity_user_body: Value = identity_user.json().await.unwrap();
    assert!(identity_user_body["firstName"].is_null());
    assert_eq!(identity_user_body["displayName"], "Dog");
    assert!(identity_user_body.get("password").is_none());
    assert!(identity_user_body.get("tenantId").is_none());
    assert!(identity_user_body.get("pictureUrl").is_none());
}

#[tokio::test]
async fn idm_identity_group_member_paths_match_membership_aliases() {
    let engine = build_engine("rest-idm-identity-group-members");
    let (base_url, client) = spawn_server(Arc::clone(&engine)).await;

    let create_user = client
        .post(format!("{}/identity/users", base_url))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "id": "fozzie",
            "firstName": "Fozzie",
            "lastName": "Bear",
            "email": "fozzie@example.test",
            "password": "wakka"
        }))
        .send()
        .await
        .unwrap();
    assert!(create_user.status().is_success());

    let create_group = client
        .post(format!("{}/identity/groups", base_url))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "id": "performers",
            "name": "Performers",
            "type": "security-role"
        }))
        .send()
        .await
        .unwrap();
    assert!(create_group.status().is_success());

    let create_membership = client
        .post(format!("{}/identity/groups/performers/members", base_url))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "userId": "fozzie"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(create_membership.status(), reqwest::StatusCode::CREATED);

    let user_memberships = client
        .get(format!("{}/identity/users/fozzie/memberships", base_url))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(user_memberships.status().is_success());
    let user_memberships_body: Value = user_memberships.json().await.unwrap();
    assert_eq!(
        user_memberships_body.as_array().unwrap()[0]["groupId"],
        "performers"
    );
    assert_eq!(
        user_memberships_body.as_array().unwrap()[0]["userId"],
        "fozzie"
    );
    assert!(
        user_memberships_body.as_array().unwrap()[0]["url"]
            .as_str()
            .unwrap()
            .ends_with("/identity/groups/performers/members/fozzie")
    );

    let delete_membership = client
        .delete(format!(
            "{}/identity/groups/performers/members/fozzie",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(delete_membership.status(), reqwest::StatusCode::NO_CONTENT);

    let user_memberships_after_delete = client
        .get(format!("{}/identity/users/fozzie/memberships", base_url))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(user_memberships_after_delete.status().is_success());
    let user_memberships_after_delete_body: Value =
        user_memberships_after_delete.json().await.unwrap();
    assert!(
        user_memberships_after_delete_body
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn idm_list_queries_use_camel_case_filters_sort_and_paging_envelope() {
    let engine = build_engine("rest-idm-list-query");
    let (base_url, client) = spawn_server(Arc::clone(&engine)).await;

    for (id, first_name, last_name, email) in [
        ("kermit", "Kermit", "Frog", "kermit@example.test"),
        ("fozzie", "Fozzie", "Bear", "fozzie@example.test"),
        ("gonzo", "Gonzo", "Great", "gonzo@example.test"),
    ] {
        let create_user = client
            .post(format!("{}/users", base_url))
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
        assert_eq!(create_user.status(), reqwest::StatusCode::CREATED);
    }

    for (id, name, group_type) in [
        ("performers", "Performers", "security-role"),
        ("operators", "Operators", "assignment"),
    ] {
        let create_group = client
            .post(format!("{}/groups", base_url))
            .basic_auth("admin", Some("test"))
            .json(&json!({
                "id": id,
                "name": name,
                "type": group_type
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(create_group.status(), reqwest::StatusCode::CREATED);
    }

    let create_membership = client
        .post(format!("{}/groups/performers/members", base_url))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "userId": "kermit" }))
        .send()
        .await
        .unwrap();
    assert_eq!(create_membership.status(), reqwest::StatusCode::CREATED);
    let membership_body: Value = create_membership.json().await.unwrap();
    assert_eq!(membership_body["userId"], "kermit");
    assert_eq!(membership_body["groupId"], "performers");

    let duplicate_membership = client
        .post(format!("{}/groups/performers/members", base_url))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "userId": "kermit" }))
        .send()
        .await
        .unwrap();
    assert_eq!(duplicate_membership.status(), reqwest::StatusCode::CONFLICT);

    let users = client
        .get(format!(
            "{}/users?firstNameLike=o&sort=lastName&order=desc&start=0&size=2",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(users.status().is_success());
    let users_body: Value = users.json().await.unwrap();
    assert_eq!(users_body["total"], 2);
    assert_eq!(users_body["start"], 0);
    assert_eq!(users_body["size"], 2);
    assert_eq!(users_body["sort"], "lastName");
    assert_eq!(users_body["order"], "desc");
    assert_eq!(users_body["data"][0]["id"], "gonzo");
    assert_eq!(users_body["data"][1]["id"], "fozzie");

    let users_by_group = client
        .get(format!("{}/users?memberOfGroup=performers", base_url))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(users_by_group.status().is_success());
    let users_by_group_body: Value = users_by_group.json().await.unwrap();
    assert_eq!(users_by_group_body["data"].as_array().unwrap().len(), 1);
    assert_eq!(users_by_group_body["data"][0]["id"], "kermit");

    let groups = client
        .get(format!(
            "{}/groups?nameLike=per&type=security-role&member=kermit&sort=name&order=asc",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(groups.status().is_success());
    let groups_body: Value = groups.json().await.unwrap();
    assert_eq!(groups_body["total"], 1);
    assert_eq!(groups_body["data"][0]["id"], "performers");
    assert_eq!(groups_body["data"][0]["type"], "security-role");
}

#[tokio::test]
async fn idm_user_info_entries_are_persisted_and_follow_canonical_contract() {
    let engine = build_engine("rest-idm-user-info");
    let (base_url, client) = spawn_server(Arc::clone(&engine)).await;

    let create = client
        .post(format!("{}/identity/users/admin/info", base_url))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "key": "department",
            "value": "support"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(create.status(), reqwest::StatusCode::CREATED);
    let create_body: Value = create.json().await.unwrap();
    assert_eq!(create_body["key"], "department");
    assert_eq!(create_body["value"], "support");
    assert!(
        create_body["url"]
            .as_str()
            .unwrap()
            .ends_with("/identity/users/admin/info/department")
    );

    let duplicate = client
        .post(format!("{}/identity/users/admin/info", base_url))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "key": "department",
            "value": "sales"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(duplicate.status(), reqwest::StatusCode::CONFLICT);

    let list = client
        .get(format!("{}/identity/users/admin/info", base_url))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(list.status().is_success());
    let list_body: Value = list.json().await.unwrap();
    assert_eq!(list_body.as_array().unwrap().len(), 1);
    assert_eq!(list_body[0]["key"], "department");
    assert!(list_body[0]["value"].is_null());

    let get = client
        .get(format!("{}/identity/users/admin/info/department", base_url))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(get.status().is_success());
    assert_eq!(get.json::<Value>().await.unwrap()["value"], "support");

    let update = client
        .put(format!("{}/identity/users/admin/info/department", base_url))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "value": "engineering"
        }))
        .send()
        .await
        .unwrap();
    assert!(update.status().is_success());
    assert_eq!(
        update.json::<Value>().await.unwrap()["value"],
        "engineering"
    );

    let delete = client
        .delete(format!("{}/identity/users/admin/info/department", base_url))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(delete.status(), reqwest::StatusCode::NO_CONTENT);

    let get_deleted = client
        .get(format!("{}/identity/users/admin/info/department", base_url))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(get_deleted.status(), reqwest::StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn idm_user_picture_is_persisted_as_binary_data_with_mime_type() {
    let engine = build_engine("rest-idm-user-picture");
    let (base_url, client) = spawn_server(Arc::clone(&engine)).await;

    let boundary = "flowable-rust-picture-boundary";
    let png_bytes = b"\x89PNG\r\n\x1a\nflowable-picture";
    let mut upload_body = Vec::new();
    upload_body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
    upload_body.extend_from_slice(
        b"Content-Disposition: form-data; name=\"file\"; filename=\"avatar.png\"\r\n",
    );
    upload_body.extend_from_slice(b"Content-Type: image/png\r\n\r\n");
    upload_body.extend_from_slice(png_bytes);
    upload_body.extend_from_slice(format!("\r\n--{boundary}\r\n").as_bytes());
    upload_body.extend_from_slice(b"Content-Disposition: form-data; name=\"mimeType\"\r\n\r\n");
    upload_body.extend_from_slice(b"image/png");
    upload_body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());

    let put = client
        .put(format!("{}/identity/users/admin/picture", base_url))
        .basic_auth("admin", Some("test"))
        .header(
            reqwest::header::CONTENT_TYPE,
            format!("multipart/form-data; boundary={boundary}"),
        )
        .body(upload_body)
        .send()
        .await
        .unwrap();
    assert_eq!(put.status(), reqwest::StatusCode::NO_CONTENT);

    let get = client
        .get(format!("{}/identity/users/admin/picture", base_url))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(get.status().is_success());
    assert_eq!(
        get.headers()
            .get(reqwest::header::CONTENT_TYPE)
            .unwrap()
            .to_str()
            .unwrap(),
        "image/png"
    );
    assert_eq!(get.bytes().await.unwrap().as_ref(), png_bytes);

    let rest_user_with_picture = client
        .get(format!("{}/users/admin", base_url))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(rest_user_with_picture.status().is_success());
    let rest_user_with_picture_body: Value = rest_user_with_picture.json().await.unwrap();
    assert_eq!(
        rest_user_with_picture_body["pictureUrl"],
        "http://localhost/identity/users/admin/picture"
    );
    assert!(rest_user_with_picture_body["password"].is_null());

    let replacement_bytes = b"replacement-picture";
    let mut replacement_body = Vec::new();
    replacement_body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
    replacement_body.extend_from_slice(
        b"Content-Disposition: form-data; name=\"file\"; filename=\"avatar.jpg\"\r\n\r\n",
    );
    replacement_body.extend_from_slice(replacement_bytes);
    replacement_body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());

    let replace = client
        .put(format!("{}/identity/users/admin/picture", base_url))
        .basic_auth("admin", Some("test"))
        .header(
            reqwest::header::CONTENT_TYPE,
            format!("multipart/form-data; boundary={boundary}"),
        )
        .body(replacement_body)
        .send()
        .await
        .unwrap();
    assert_eq!(replace.status(), reqwest::StatusCode::NO_CONTENT);

    let get_replacement = client
        .get(format!("{}/identity/users/admin/picture", base_url))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(
        get_replacement
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .unwrap()
            .to_str()
            .unwrap(),
        "image/jpeg"
    );
    assert_eq!(
        get_replacement.bytes().await.unwrap().as_ref(),
        replacement_bytes
    );
}

#[tokio::test]
async fn idm_user_picture_post_creates_and_delete_removes() {
    let engine = build_engine("rest-idm-user-picture-post-delete");
    let (base_url, client) = spawn_server(Arc::clone(&engine)).await;

    let boundary = "flowable-rust-picture-post-delete";
    let png_bytes = b"\x89PNG\r\n\x1a\nflowable-picture-post";
    let mut upload_body = Vec::new();
    upload_body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
    upload_body.extend_from_slice(
        b"Content-Disposition: form-data; name=\"file\"; filename=\"avatar.png\"\r\n",
    );
    upload_body.extend_from_slice(b"Content-Type: image/png\r\n\r\n");
    upload_body.extend_from_slice(png_bytes);
    upload_body.extend_from_slice(format!("\r\n--{boundary}\r\n").as_bytes());
    upload_body.extend_from_slice(b"Content-Disposition: form-data; name=\"mimeType\"\r\n\r\n");
    upload_body.extend_from_slice(b"image/png");
    upload_body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());

    let post = client
        .post(format!("{}/identity/users/admin/picture", base_url))
        .basic_auth("admin", Some("test"))
        .header(
            reqwest::header::CONTENT_TYPE,
            format!("multipart/form-data; boundary={boundary}"),
        )
        .body(upload_body)
        .send()
        .await
        .unwrap();
    assert_eq!(post.status(), reqwest::StatusCode::CREATED);

    let get = client
        .get(format!("{}/identity/users/admin/picture", base_url))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(get.status().is_success());
    assert_eq!(
        get.headers()
            .get(reqwest::header::CONTENT_TYPE)
            .unwrap()
            .to_str()
            .unwrap(),
        "image/png"
    );
    assert_eq!(get.bytes().await.unwrap().as_ref(), png_bytes);

    let delete = client
        .delete(format!("{}/identity/users/admin/picture", base_url))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(delete.status(), reqwest::StatusCode::NO_CONTENT);

    let get_after_delete = client
        .get(format!("{}/identity/users/admin/picture", base_url))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(get_after_delete.status(), reqwest::StatusCode::NOT_FOUND);

    let delete_again = client
        .delete(format!("{}/identity/users/admin/picture", base_url))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(delete_again.status(), reqwest::StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn idm_user_info_has_created_at_and_updated_at_timestamps() {
    let engine = build_engine("rest-idm-user-info-timestamps");
    let (base_url, client) = spawn_server(Arc::clone(&engine)).await;

    let create = client
        .post(format!("{}/identity/users/admin/info", base_url))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "key": "department",
            "value": "support"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(create.status(), reqwest::StatusCode::CREATED);

    let info = engine
        .get_identity_service()
        .get_user_info("admin", "department")
        .unwrap();
    assert!(info.created_at.is_some());
    assert!(info.updated_at.is_some());
    assert!(info.created_at.unwrap() > 0);
    assert!(info.updated_at.unwrap() > 0);

    let update = client
        .put(format!("{}/identity/users/admin/info/department", base_url))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "value": "engineering"
        }))
        .send()
        .await
        .unwrap();
    assert!(update.status().is_success());

    let updated_info = engine
        .get_identity_service()
        .get_user_info("admin", "department")
        .unwrap();
    assert_eq!(updated_info.created_at, info.created_at);
    assert!(updated_info.updated_at.unwrap() >= info.updated_at.unwrap());
}
