use flowable_identity_service::ldap;
use flowable_platform_bootstrap::DirectoryConfiguration;

fn internal_config() -> DirectoryConfiguration {
    ldap::default_directory_config()
}

#[test]
fn internal_provider_resolves_correctly() {
    let config = internal_config();
    let kind = ldap::resolve_provider_kind(&config.provider).unwrap();
    assert!(matches!(
        kind,
        flowable_platform_bootstrap::DirectoryProviderKind::Internal
    ));
}

#[test]
fn ldap_mirror_requires_bundle_path() {
    let config = DirectoryConfiguration {
        provider: "ldap-mirror".to_string(),
        sync_on_bootstrap: true,
        bundle_path: None,
        ..internal_config()
    };
    let result = ldap::validate_ldap_config(&config);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("bundle_path"));
}

#[test]
fn ldap_live_requires_bundle_path() {
    let config = DirectoryConfiguration {
        provider: "ldap-live".to_string(),
        bundle_path: None,
        ..internal_config()
    };
    let result = ldap::validate_ldap_config(&config);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("bundle_path"));
}

#[test]
fn ldap_live_rejects_sync_on_bootstrap() {
    let config = DirectoryConfiguration {
        provider: "ldap-live".to_string(),
        sync_on_bootstrap: true,
        bundle_path: Some("/tmp/test-bundle.toml".to_string()),
        ..internal_config()
    };
    let result = ldap::validate_ldap_config(&config);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("sync_on_bootstrap"));
}

#[test]
fn unknown_provider_returns_error() {
    let result = ldap::resolve_provider_kind("unknown-provider");
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("Unknown directory provider"));
}

#[test]
fn internal_config_validates_successfully() {
    let config = internal_config();
    assert!(ldap::validate_ldap_config(&config).is_ok());
}

#[test]
fn ldap_mirror_with_bundle_path_validates_successfully() {
    let config = DirectoryConfiguration {
        provider: "ldap-mirror".to_string(),
        sync_on_bootstrap: false,
        bundle_path: Some("/tmp/bundle.toml".to_string()),
        ..internal_config()
    };
    assert!(ldap::validate_ldap_config(&config).is_ok());
}

#[test]
fn ldap_live_with_bundle_path_validates_successfully() {
    let config = DirectoryConfiguration {
        provider: "ldap-live".to_string(),
        sync_on_bootstrap: false,
        bundle_path: Some("/tmp/bundle.toml".to_string()),
        ..internal_config()
    };
    assert!(ldap::validate_ldap_config(&config).is_ok());
}
