use flowable_platform_bootstrap::{
    DirectoryConfiguration, DirectoryProviderKind, FlowablePlatform, PlatformBootstrapError,
    PlatformConfiguration,
};

pub fn create_platform_with_ldap(
    directory_config: DirectoryConfiguration,
) -> Result<FlowablePlatform, PlatformBootstrapError> {
    let config = PlatformConfiguration {
        directory: directory_config,
        ..PlatformConfiguration::default()
    };
    FlowablePlatform::bootstrap(config)
}

pub fn resolve_provider_kind(provider: &str) -> Result<DirectoryProviderKind, String> {
    match provider.to_lowercase().as_str() {
        "internal" => Ok(DirectoryProviderKind::Internal),
        "ldap-mirror" => Ok(DirectoryProviderKind::LdapMirror),
        "ldap-live" => Ok(DirectoryProviderKind::LdapLive),
        other => Err(format!("Unknown directory provider: '{}'", other)),
    }
}

pub fn validate_ldap_config(config: &DirectoryConfiguration) -> Result<(), String> {
    let kind = resolve_provider_kind(&config.provider)?;
    match kind {
        DirectoryProviderKind::Internal => Ok(()),
        DirectoryProviderKind::LdapMirror => {
            if config.bundle_path.is_none() {
                return Err("ldap-mirror requires bundle_path".to_string());
            }
            Ok(())
        }
        DirectoryProviderKind::LdapLive => {
            if config.bundle_path.is_none() {
                return Err("ldap-live requires bundle_path".to_string());
            }
            if config.sync_on_bootstrap {
                return Err("ldap-live does not support sync_on_bootstrap".to_string());
            }
            Ok(())
        }
    }
}

pub fn default_directory_config() -> DirectoryConfiguration {
    DirectoryConfiguration {
        provider: "internal".to_string(),
        sync_on_bootstrap: false,
        bundle_path: None,
        transport: "ldaps".to_string(),
        auth_mode: "service-account-bind".to_string(),
        deployment_mode: "sidecar-session".to_string(),
        conflict_policy: "live-wins".to_string(),
        filter_breadth: "identity-surface-full".to_string(),
    }
}
