use flowable_engine::identity::entities::{Group, User};
use flowable_platform_bootstrap::{
    DirectoryProviderKind, FlowablePlatform, OperationsExposureKind, PlatformConfiguration,
};
use std::path::Path;

fn write_directory_bundle(path: &Path) {
    std::fs::write(
        path,
        r#"
[[users]]
id = "ldap-alice"
first_name = "Alice"
last_name = "Directory"
email = "alice@example.test"
password = "ignored"

[[groups]]
id = "platform-admins"
name = "Platform Admins"
group_type = "security-role"

[[memberships]]
user_id = "ldap-alice"
group_id = "platform-admins"
"#,
    )
    .expect("directory bundle");
}

fn write_platform_config(config_path: &Path, database_path: &Path, bundle_path: &Path) {
    let escaped_database_path = database_path.display().to_string().replace('\\', "\\\\");
    let escaped_bundle_path = bundle_path.display().to_string().replace('\\', "\\\\");
    let config = format!(
        r#"
[server]
bind_address = "127.0.0.1:0"

[process]
engine_name = "m27-directory-ops"
database_path = "{database_path}"

[security]
auth_mode = "disabled"

[directory]
provider = "ldap-mirror"
sync_on_bootstrap = true
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

#[test]
fn bootstrap_loads_directory_and_operations_contracts_and_imports_directory_bundle() {
    let tempdir = tempfile::tempdir().expect("temp dir");
    let bundle_path = tempdir.path().join("ldap-directory.toml");
    let config_path = tempdir.path().join("flowable-platform.toml");
    write_directory_bundle(&bundle_path);
    write_platform_config(
        &config_path,
        &tempdir.path().join("process-engine.sqlite"),
        &bundle_path,
    );

    let platform = FlowablePlatform::bootstrap_from_sources(Some(config_path)).expect("platform");

    assert_eq!(
        platform.directory_support_contract().provider,
        DirectoryProviderKind::LdapMirror
    );
    assert!(platform.directory_support_contract().sync_on_bootstrap);
    assert_eq!(
        platform.operations_support_contract().exposure,
        OperationsExposureKind::JmxBridge
    );
    assert!(
        platform
            .operations_support_contract()
            .management_api_enabled
    );
    assert!(
        platform
            .operations_support_contract()
            .runtime_ledger_enabled
    );
    assert!(platform.operations_support_contract().timer_ledger_enabled);
    assert!(
        platform
            .operations_support_contract()
            .topology_ledger_enabled
    );

    let identity_service = platform.process_engine().get_identity_service();
    let user = identity_service
        .find_user_by_id("ldap-alice")
        .expect("directory user should be imported");
    assert_eq!(user.email.as_deref(), Some("alice@example.test"));

    let group = identity_service
        .find_group_by_id("platform-admins")
        .expect("directory group should be imported");
    assert_eq!(group.name, "Platform Admins");

    let memberships = identity_service.get_groups_by_user("ldap-alice");
    assert_eq!(memberships.len(), 1);
    assert_eq!(memberships[0].id, "platform-admins");
}

#[test]
fn bootstrap_fails_fast_when_ldap_mirror_provider_has_no_bundle_path() {
    let mut configuration = PlatformConfiguration::default();
    configuration.process.database_path = ":memory:".to_string();
    configuration.bootstrap.create_default_admin = false;
    configuration.directory.provider = "ldap-mirror".to_string();
    configuration.directory.sync_on_bootstrap = true;
    configuration.operations.exposure = "jmx-bridge".to_string();
    configuration.operations.management_api_enabled = true;

    let error = FlowablePlatform::bootstrap(configuration)
        .err()
        .expect("expected failure");

    assert_eq!(
        error.to_string(),
        "Directory provider 'ldap-mirror' requires a bundle path when sync_on_bootstrap is enabled"
    );
}

#[test]
fn bootstrap_accepts_bounded_ldap_live_provider_without_importing_into_engine_store() {
    let tempdir = tempfile::tempdir().expect("temp dir");
    let bundle_path = tempdir.path().join("ldap-live-directory.toml");
    let config_path = tempdir.path().join("flowable-platform.toml");
    write_directory_bundle(&bundle_path);

    let escaped_database_path = tempdir
        .path()
        .join("process-engine.sqlite")
        .display()
        .to_string()
        .replace('\\', "\\\\");
    let escaped_bundle_path = bundle_path.display().to_string().replace('\\', "\\\\");
    let config = format!(
        r#"
[server]
bind_address = "127.0.0.1:0"

[process]
engine_name = "m30-directory-live"
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
"#,
        database_path = escaped_database_path,
        bundle_path = escaped_bundle_path,
    );
    std::fs::write(&config_path, config).expect("config");

    let platform = FlowablePlatform::bootstrap_from_sources(Some(config_path)).expect("platform");
    let contract = platform.directory_support_contract();

    assert_eq!(format!("{:?}", contract.provider), "LdapLive");
    assert!(!contract.sync_on_bootstrap);
    assert_eq!(contract.imported_user_count, 0);
    assert_eq!(contract.imported_group_count, 0);
    assert_eq!(contract.imported_membership_count, 0);
    assert!(
        contract.support_statement.contains("runtime")
            || contract.support_statement.contains("write-through"),
        "ldap-live contract should be explicit about bounded runtime directory operations"
    );
    assert!(contract.runtime_user_read_enabled);
    assert!(contract.runtime_group_read_enabled);
    assert!(contract.runtime_membership_read_enabled);
    assert!(contract.runtime_user_write_enabled);
    assert!(contract.runtime_group_write_enabled);
    assert!(contract.runtime_membership_write_enabled);
    assert_eq!(contract.transport, "ldaps");
    assert_eq!(contract.auth_mode, "service-account-bind");
    assert_eq!(contract.deployment_mode, "sidecar-session");
    assert_eq!(contract.conflict_policy, "live-wins");
    assert_eq!(contract.filter_breadth, "identity-surface-full");
    assert!(contract.runtime_bidirectional_sync_enabled);

    let identity_service = platform.process_engine().get_identity_service();
    assert!(
        identity_service.find_user_by_id("ldap-alice").is_none(),
        "ldap-live must not mirror-import into the owned engine store"
    );
}

#[test]
fn bounded_ldap_live_provider_persists_bundle_mutations_without_engine_import() {
    let tempdir = tempfile::tempdir().expect("temp dir");
    let bundle_path = tempdir.path().join("ldap-live-directory.toml");
    let config_path = tempdir.path().join("flowable-platform.toml");
    write_directory_bundle(&bundle_path);

    let escaped_database_path = tempdir
        .path()
        .join("process-engine.sqlite")
        .display()
        .to_string()
        .replace('\\', "\\\\");
    let escaped_bundle_path = bundle_path.display().to_string().replace('\\', "\\\\");
    let config = format!(
        r#"
[server]
bind_address = "127.0.0.1:0"

[process]
engine_name = "m34-directory-live"
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
"#,
        database_path = escaped_database_path,
        bundle_path = escaped_bundle_path,
    );
    std::fs::write(&config_path, config).expect("config");

    let platform = FlowablePlatform::bootstrap_from_sources(Some(config_path)).expect("platform");
    let provider = platform
        .live_directory_provider()
        .expect("bounded ldap-live provider");

    provider
        .save_user(User {
            id: "ldap-bob".to_string(),
            first_name: Some("Bob".to_string()),
            last_name: Some("Writer".to_string()),
            email: Some("bob@example.test".to_string()),
            password: None,
            tenant_id: None,
        })
        .expect("save live user");
    provider
        .save_group(Group {
            id: "audit-team".to_string(),
            name: "Audit Team".to_string(),
            group_type: Some("security-role".to_string()),
        })
        .expect("save live group");
    provider
        .create_membership("ldap-bob", "audit-team")
        .expect("create live membership");
    provider
        .delete_membership("ldap-alice", "platform-admins")
        .expect("delete original membership");
    provider
        .delete_group("platform-admins")
        .expect("delete original group");
    provider
        .delete_user("ldap-alice")
        .expect("delete original user");

    let snapshot = provider.load_snapshot().expect("live snapshot");
    assert_eq!(snapshot.users.len(), 1);
    assert_eq!(snapshot.users[0].id, "ldap-bob");
    assert_eq!(snapshot.groups.len(), 1);
    assert_eq!(snapshot.groups[0].id, "audit-team");
    assert_eq!(snapshot.memberships.len(), 1);
    assert_eq!(snapshot.memberships[0].user_id, "ldap-bob");
    assert_eq!(snapshot.memberships[0].group_id, "audit-team");

    let identity_service = platform.process_engine().get_identity_service();
    assert!(
        identity_service.find_user_by_id("ldap-bob").is_none(),
        "live provider mutations must stay outside the owned engine store"
    );
    assert!(
        identity_service.find_group_by_id("audit-team").is_none(),
        "live provider group mutations must stay outside the owned engine store"
    );
}
