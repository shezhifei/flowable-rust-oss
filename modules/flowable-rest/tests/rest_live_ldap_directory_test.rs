use flowable_platform_bootstrap::FlowablePlatform;
use flowable_rest::run_platform_server;
use serde_json::Value;
use std::path::Path;
use tokio::net::TcpListener;

fn write_live_directory_bundle(
    path: &Path,
    user_id: &str,
    first_name: &str,
    email: &str,
    group_id: &str,
    group_name: &str,
) {
    std::fs::write(
        path,
        format!(
            r#"
[[users]]
id = "{user_id}"
first_name = "{first_name}"
last_name = "Directory"
email = "{email}"

[[groups]]
id = "{group_id}"
name = "{group_name}"
group_type = "security-role"

[[memberships]]
user_id = "{user_id}"
group_id = "{group_id}"
"#
        ),
    )
    .expect("live directory bundle");
}

fn write_platform_config(config_path: &Path, database_path: &Path, bundle_path: &Path) {
    let escaped_database_path = database_path.display().to_string().replace('\\', "\\\\");
    let escaped_bundle_path = bundle_path.display().to_string().replace('\\', "\\\\");
    let config = format!(
        r#"
[server]
bind_address = "127.0.0.1:0"

[process]
engine_name = "m30-rest-live-ldap"
database_path = "{database_path}"

[security]
auth_mode = "disabled"

[bootstrap]
create_default_admin = false
admin_user_id = "admin"
admin_password = "ignored"

[directory]
provider = "ldap-live"
sync_on_bootstrap = false
bundle_path = "{bundle_path}"

[operations]
exposure = "jmx-bridge"
management_api_enabled = true
"#,
        database_path = escaped_database_path,
        bundle_path = escaped_bundle_path,
    );
    std::fs::write(config_path, config).expect("config");
}

async fn spawn_platform_server(platform: FlowablePlatform) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let address = listener.local_addr().expect("local addr");
    let base_url = format!("http://{address}");

    tokio::spawn(async move {
        run_platform_server(platform, listener)
            .await
            .expect("server should start");
    });

    base_url
}

fn identity_collection_data(body: &Value) -> &[Value] {
    body.get("data")
        .and_then(Value::as_array)
        .or_else(|| body.as_array())
        .expect("identity collection payload")
}

#[tokio::test]
async fn ldap_live_provider_exposes_runtime_directory_reads_without_bootstrap_import() {
    let tempdir = tempfile::tempdir().expect("temp dir");
    let bundle_path = tempdir.path().join("ldap-live-directory.toml");
    let config_path = tempdir.path().join("flowable-platform.toml");
    write_live_directory_bundle(
        &bundle_path,
        "live-alice",
        "Alice",
        "alice@example.test",
        "live-admins",
        "Live Admins",
    );
    write_platform_config(
        &config_path,
        &tempdir.path().join("process-engine.sqlite"),
        &bundle_path,
    );

    let platform = FlowablePlatform::bootstrap_from_sources(Some(config_path)).expect("platform");
    assert!(
        platform
            .process_engine()
            .get_identity_service()
            .find_user_by_id("live-alice")
            .is_none(),
        "ldap-live should remain outside bootstrap mirror import"
    );

    let base_url = spawn_platform_server(platform).await;
    let client = reqwest::Client::new();

    let directory_support: Value = client
        .get(format!("{base_url}/management/directory/support"))
        .send()
        .await
        .expect("directory support request")
        .json()
        .await
        .expect("directory support payload");
    assert_eq!(directory_support["provider"], "ldap-live");
    assert_eq!(directory_support["syncOnBootstrap"], false);
    assert_eq!(directory_support["importedUserCount"], 0);
    assert_eq!(directory_support["runtimeUserReadEnabled"], true);
    assert_eq!(directory_support["runtimeGroupReadEnabled"], true);
    assert_eq!(directory_support["runtimeMembershipReadEnabled"], true);
    assert_eq!(directory_support["runtimeUserWriteEnabled"], true);
    assert_eq!(directory_support["runtimeGroupWriteEnabled"], true);
    assert_eq!(directory_support["runtimeMembershipWriteEnabled"], true);
    assert_eq!(directory_support["transport"], "ldaps");
    assert_eq!(directory_support["authMode"], "service-account-bind");
    assert_eq!(directory_support["deploymentMode"], "sidecar-session");
    assert_eq!(directory_support["conflictPolicy"], "live-wins");
    assert_eq!(directory_support["filterBreadth"], "identity-surface-full");
    assert_eq!(directory_support["runtimeBidirectionalSyncEnabled"], true);

    let first_user = client
        .get(format!("{base_url}/identity/users/live-alice"))
        .send()
        .await
        .expect("initial live user request");
    assert_eq!(first_user.status(), reqwest::StatusCode::OK);
    let first_user_payload: Value = first_user.json().await.expect("initial live user payload");
    assert_eq!(first_user_payload["email"], "alice@example.test");

    let first_group = client
        .get(format!("{base_url}/identity/groups/live-admins"))
        .send()
        .await
        .expect("initial live group request");
    assert_eq!(first_group.status(), reqwest::StatusCode::OK);

    let first_memberships: Value = client
        .get(format!("{base_url}/identity/users/live-alice/memberships"))
        .send()
        .await
        .expect("initial live memberships request")
        .json()
        .await
        .expect("initial memberships payload");
    assert_eq!(
        first_memberships
            .as_array()
            .expect("memberships array")
            .len(),
        1
    );
    assert_eq!(first_memberships[0]["groupId"], "live-admins");

    write_live_directory_bundle(
        &bundle_path,
        "live-bob",
        "Bob",
        "bob@example.test",
        "live-auditors",
        "Live Auditors",
    );

    let old_user = client
        .get(format!("{base_url}/identity/users/live-alice"))
        .send()
        .await
        .expect("old live user request");
    assert_eq!(old_user.status(), reqwest::StatusCode::NOT_FOUND);

    let updated_users: Value = client
        .get(format!("{base_url}/identity/users"))
        .send()
        .await
        .expect("updated user list request")
        .json()
        .await
        .expect("updated user list payload");
    let updated_users = identity_collection_data(&updated_users);
    assert_eq!(updated_users.len(), 1);
    assert_eq!(updated_users[0]["id"], "live-bob");

    let updated_group = client
        .get(format!("{base_url}/identity/groups/live-auditors"))
        .send()
        .await
        .expect("updated live group request");
    assert_eq!(updated_group.status(), reqwest::StatusCode::OK);

    let updated_memberships: Value = client
        .get(format!("{base_url}/identity/users/live-bob/memberships"))
        .send()
        .await
        .expect("updated memberships request")
        .json()
        .await
        .expect("updated memberships payload");
    assert_eq!(
        updated_memberships
            .as_array()
            .expect("memberships array")
            .len(),
        1
    );
    assert_eq!(updated_memberships[0]["groupId"], "live-auditors");
}

#[tokio::test]
async fn ldap_live_provider_persists_mutations_through_identity_routes() {
    let tempdir = tempfile::tempdir().expect("temp dir");
    let bundle_path = tempdir.path().join("ldap-live-directory.toml");
    let config_path = tempdir.path().join("flowable-platform.toml");
    write_live_directory_bundle(
        &bundle_path,
        "live-alice",
        "Alice",
        "alice@example.test",
        "live-admins",
        "Live Admins",
    );
    write_platform_config(
        &config_path,
        &tempdir.path().join("process-engine.sqlite"),
        &bundle_path,
    );

    let platform = FlowablePlatform::bootstrap_from_sources(Some(config_path)).expect("platform");
    let engine = platform.process_engine();
    let base_url = spawn_platform_server(platform).await;
    let client = reqwest::Client::new();

    let created_user = client
        .post(format!("{base_url}/identity/users"))
        .json(&serde_json::json!({
            "id": "live-bob",
            "first_name": "Bob",
            "last_name": "Writer",
            "email": "bob@example.test"
        }))
        .send()
        .await
        .expect("create live user");
    assert_eq!(created_user.status(), reqwest::StatusCode::CREATED);

    let created_group = client
        .post(format!("{base_url}/identity/groups"))
        .json(&serde_json::json!({
            "id": "live-auditors",
            "name": "Live Auditors",
            "group_type": "security-role"
        }))
        .send()
        .await
        .expect("create live group");
    assert_eq!(created_group.status(), reqwest::StatusCode::CREATED);

    let created_membership = client
        .post(format!("{base_url}/identity/memberships"))
        .json(&serde_json::json!({
            "user_id": "live-bob",
            "group_id": "live-auditors"
        }))
        .send()
        .await
        .expect("create live membership");
    assert_eq!(created_membership.status(), reqwest::StatusCode::CREATED);

    let filtered_users: Value = client
        .get(format!("{base_url}/identity/users"))
        .query(&[("email", "bob@example.test")])
        .send()
        .await
        .expect("filtered user query")
        .json()
        .await
        .expect("filtered user payload");
    let filtered_users = identity_collection_data(&filtered_users);
    assert_eq!(filtered_users.len(), 1);
    assert_eq!(filtered_users[0]["id"], "live-bob");

    let contains_filtered_users: Value = client
        .get(format!("{base_url}/identity/users"))
        .query(&[
            ("email_contains", "example.test"),
            ("member_of_group_id", "live-auditors"),
        ])
        .send()
        .await
        .expect("contains user query")
        .json()
        .await
        .expect("contains user payload");
    let contains_filtered_users = identity_collection_data(&contains_filtered_users);
    assert_eq!(contains_filtered_users.len(), 1);
    assert_eq!(contains_filtered_users[0]["id"], "live-bob");

    let filtered_groups: Value = client
        .get(format!("{base_url}/identity/groups"))
        .query(&[("name_contains", "Audit"), ("member_user_id", "live-bob")])
        .send()
        .await
        .expect("filtered group query")
        .json()
        .await
        .expect("filtered group payload");
    let filtered_groups = identity_collection_data(&filtered_groups);
    assert_eq!(filtered_groups.len(), 1);
    assert_eq!(filtered_groups[0]["id"], "live-auditors");

    let memberships: Value = client
        .get(format!("{base_url}/identity/users/live-bob/memberships"))
        .send()
        .await
        .expect("live bob memberships")
        .json()
        .await
        .expect("live bob memberships payload");
    assert_eq!(memberships.as_array().expect("memberships array").len(), 1);
    assert_eq!(memberships[0]["groupId"], "live-auditors");

    assert!(
        engine
            .get_identity_service()
            .find_user_by_id("live-bob")
            .is_none(),
        "live ldap mutation must not create an owned-store user"
    );
    assert!(
        engine
            .get_identity_service()
            .find_group_by_id("live-auditors")
            .is_none(),
        "live ldap mutation must not create an owned-store group"
    );

    let bundle_contents = std::fs::read_to_string(&bundle_path).expect("bundle contents");
    assert!(bundle_contents.contains("live-bob"));
    assert!(bundle_contents.contains("live-auditors"));

    let deleted_membership = client
        .delete(format!(
            "{base_url}/identity/memberships/live-bob/live-auditors"
        ))
        .send()
        .await
        .expect("delete live membership");
    assert_eq!(deleted_membership.status(), reqwest::StatusCode::NO_CONTENT);

    let deleted_group = client
        .delete(format!("{base_url}/identity/groups/live-auditors"))
        .send()
        .await
        .expect("delete live group");
    assert_eq!(deleted_group.status(), reqwest::StatusCode::NO_CONTENT);

    let deleted_user = client
        .delete(format!("{base_url}/identity/users/live-bob"))
        .send()
        .await
        .expect("delete live user");
    assert_eq!(deleted_user.status(), reqwest::StatusCode::NO_CONTENT);

    let final_users: Value = client
        .get(format!("{base_url}/identity/users"))
        .query(&[("email", "bob@example.test")])
        .send()
        .await
        .expect("final filtered user query")
        .json()
        .await
        .expect("final filtered user payload");
    assert!(identity_collection_data(&final_users).is_empty());

    let final_bundle_contents =
        std::fs::read_to_string(&bundle_path).expect("final bundle contents");
    assert!(!final_bundle_contents.contains("live-bob"));
    assert!(!final_bundle_contents.contains("live-auditors"));
}

#[tokio::test]
async fn ldap_live_reconcile_reports_and_repairs_shadowed_owned_identity_state() {
    let tempdir = tempfile::tempdir().expect("temp dir");
    let bundle_path = tempdir.path().join("ldap-live-directory.toml");
    let config_path = tempdir.path().join("flowable-platform.toml");
    write_live_directory_bundle(
        &bundle_path,
        "live-alice",
        "Alice",
        "alice@example.test",
        "live-admins",
        "Live Admins",
    );
    write_platform_config(
        &config_path,
        &tempdir.path().join("process-engine.sqlite"),
        &bundle_path,
    );

    let platform = FlowablePlatform::bootstrap_from_sources(Some(config_path)).expect("platform");
    let engine = platform.process_engine();
    let identity_service = engine.get_identity_service();
    identity_service.save_user(flowable_engine::identity::entities::User {
        id: "live-alice".to_string(),
        first_name: Some("Shadow".to_string()),
        last_name: Some("User".to_string()),
        email: Some("shadow@example.test".to_string()),
        password: None,
        tenant_id: None,
    });
    identity_service.save_group(flowable_engine::identity::entities::Group {
        id: "live-admins".to_string(),
        name: "Shadow Admins".to_string(),
        group_type: Some("security-role".to_string()),
    });
    identity_service.create_membership("live-alice".to_string(), "live-admins".to_string());

    let base_url = spawn_platform_server(platform).await;
    let client = reqwest::Client::new();

    let report: Value = client
        .get(format!("{base_url}/management/directory/reconcile"))
        .send()
        .await
        .expect("reconcile report request")
        .json()
        .await
        .expect("reconcile report payload");
    assert_eq!(report["provider"], "ldap-live");
    assert_eq!(report["supported"], true);
    assert_eq!(report["applied"], false);
    assert_eq!(report["shadowedUserIds"][0], "live-alice");
    assert_eq!(report["shadowedGroupIds"][0], "live-admins");
    assert_eq!(report["shadowedMemberships"][0]["userId"], "live-alice");
    assert_eq!(report["shadowedMemberships"][0]["groupId"], "live-admins");

    let applied: Value = client
        .post(format!("{base_url}/management/directory/reconcile"))
        .send()
        .await
        .expect("reconcile apply request")
        .json()
        .await
        .expect("reconcile apply payload");
    assert_eq!(applied["applied"], true);
    assert_eq!(applied["removedUsers"], 1);
    assert_eq!(applied["removedGroups"], 1);
    assert_eq!(applied["removedMemberships"], 1);

    assert!(
        engine
            .get_identity_service()
            .find_user_by_id("live-alice")
            .is_none(),
        "reconcile should remove the shadowed owned-store user"
    );
    assert!(
        engine
            .get_identity_service()
            .find_group_by_id("live-admins")
            .is_none(),
        "reconcile should remove the shadowed owned-store group"
    );

    let live_user = client
        .get(format!("{base_url}/identity/users/live-alice"))
        .send()
        .await
        .expect("live user request after reconcile");
    assert_eq!(live_user.status(), reqwest::StatusCode::OK);
}

#[tokio::test]
async fn ldap_live_reconcile_can_promote_owned_only_identity_state_into_live_directory() {
    let tempdir = tempfile::tempdir().expect("temp dir");
    let bundle_path = tempdir.path().join("ldap-live-directory.toml");
    let config_path = tempdir.path().join("flowable-platform.toml");
    write_live_directory_bundle(
        &bundle_path,
        "live-alice",
        "Alice",
        "alice@example.test",
        "live-admins",
        "Live Admins",
    );
    write_platform_config(
        &config_path,
        &tempdir.path().join("process-engine.sqlite"),
        &bundle_path,
    );

    let platform = FlowablePlatform::bootstrap_from_sources(Some(config_path)).expect("platform");
    let engine = platform.process_engine();
    let identity_service = engine.get_identity_service();
    identity_service.save_user(flowable_engine::identity::entities::User {
        id: "owned-bob".to_string(),
        first_name: Some("Bob".to_string()),
        last_name: Some("Owned".to_string()),
        email: Some("owned-bob@example.test".to_string()),
        password: None,
        tenant_id: None,
    });
    identity_service.save_group(flowable_engine::identity::entities::Group {
        id: "owned-auditors".to_string(),
        name: "Owned Auditors".to_string(),
        group_type: Some("security-role".to_string()),
    });
    identity_service.create_membership("owned-bob".to_string(), "owned-auditors".to_string());

    let base_url = spawn_platform_server(platform).await;
    let client = reqwest::Client::new();

    let report: Value = client
        .get(format!(
            "{base_url}/management/directory/reconcile?mode=owned-to-live"
        ))
        .send()
        .await
        .expect("owned-to-live reconcile report request")
        .json()
        .await
        .expect("owned-to-live reconcile report payload");
    assert_eq!(report["provider"], "ldap-live");
    assert_eq!(report["mode"], "owned-to-live");
    assert_eq!(report["supported"], true);
    assert_eq!(report["applied"], false);
    assert_eq!(report["ownedOnlyUserIds"][0], "owned-bob");
    assert_eq!(report["ownedOnlyGroupIds"][0], "owned-auditors");
    assert_eq!(report["ownedOnlyMemberships"][0]["userId"], "owned-bob");
    assert_eq!(
        report["ownedOnlyMemberships"][0]["groupId"],
        "owned-auditors"
    );

    let applied: Value = client
        .post(format!(
            "{base_url}/management/directory/reconcile?mode=owned-to-live"
        ))
        .send()
        .await
        .expect("owned-to-live reconcile apply request")
        .json()
        .await
        .expect("owned-to-live reconcile apply payload");
    assert_eq!(applied["mode"], "owned-to-live");
    assert_eq!(applied["applied"], true);
    assert_eq!(applied["addedUsers"], 1);
    assert_eq!(applied["addedGroups"], 1);
    assert_eq!(applied["addedMemberships"], 1);
    assert_eq!(applied["removedUsers"], 1);
    assert_eq!(applied["removedGroups"], 1);
    assert_eq!(applied["removedMemberships"], 1);

    let promoted_user = client
        .get(format!("{base_url}/identity/users/owned-bob"))
        .send()
        .await
        .expect("promoted user request");
    assert_eq!(promoted_user.status(), reqwest::StatusCode::OK);
    let promoted_user_payload: Value = promoted_user.json().await.expect("promoted user payload");
    assert_eq!(promoted_user_payload["email"], "owned-bob@example.test");

    let promoted_group = client
        .get(format!("{base_url}/identity/groups/owned-auditors"))
        .send()
        .await
        .expect("promoted group request");
    assert_eq!(promoted_group.status(), reqwest::StatusCode::OK);

    let promoted_memberships: Value = client
        .get(format!("{base_url}/identity/users/owned-bob/memberships"))
        .send()
        .await
        .expect("promoted memberships request")
        .json()
        .await
        .expect("promoted memberships payload");
    assert_eq!(
        promoted_memberships
            .as_array()
            .expect("promoted memberships array")
            .len(),
        1
    );
    assert_eq!(promoted_memberships[0]["groupId"], "owned-auditors");

    assert!(
        engine
            .get_identity_service()
            .find_user_by_id("owned-bob")
            .is_none(),
        "owned-to-live reconcile should remove the owned-store user after promotion"
    );
    assert!(
        engine
            .get_identity_service()
            .find_group_by_id("owned-auditors")
            .is_none(),
        "owned-to-live reconcile should remove the owned-store group after promotion"
    );
}
