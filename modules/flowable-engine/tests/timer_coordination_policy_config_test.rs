use flowable_engine::service::config::{AuthProviderKind, ServicePolicyConfig};

#[test]
fn test_timer_coordination_policy_config() {
    let json = r#"{
        "bind_addr": "127.0.0.1:9090",
        "max_request_size": 2048,
        "auth_provider": "local-static",
        "policy_version": "v2",
        "auth_keys": {
            "my-admin-token": {
                "actor_id": "operator-bob",
                "subject": "bob-subject",
                "issuer": "issuer-bob",
                "role": "admin",
                "tenant_id": "tenant-a"
            },
            "my-read-token": {
                "actor_id": "dashboard",
                "subject": "dashboard-subject",
                "role": "read"
            }
        }
    }"#;

    let file_path = "test_policy_config.json";
    std::fs::write(file_path, json).unwrap();

    let config = ServicePolicyConfig::load_from_file(file_path).unwrap();
    std::fs::remove_file(file_path).unwrap();

    assert_eq!(config.bind_addr, "127.0.0.1:9090");
    assert_eq!(config.max_request_size, 2048);
    assert_eq!(config.auth_provider, AuthProviderKind::LocalStatic);
    assert_eq!(config.policy_version, "v2");
    assert_eq!(config.auth_keys.len(), 2);

    let db = flowable_engine::persistence::db_store::DbStore::new_in_memory().unwrap();
    let runtime_store =
        flowable_engine::persistence::runtime_store::RuntimeStore::new_with_memory_backend_for_test(
            std::sync::Arc::new(db),
        );
    let auth_provider = config.to_auth_provider(runtime_store);

    let admin_principal = auth_provider.authenticate(Some("my-admin-token")).unwrap();
    assert!(admin_principal.has_role("admin"));
    assert_eq!(admin_principal.subject, "bob-subject");
    assert_eq!(admin_principal.issuer, "issuer-bob");
    assert_eq!(admin_principal.tenant_id.as_deref(), Some("tenant-a"));

    let read_principal = auth_provider.authenticate(Some("my-read-token")).unwrap();
    assert!(read_principal.has_role("read"));
    assert!(!read_principal.has_role("admin"));
    assert_eq!(read_principal.subject, "dashboard-subject");
    assert_eq!(read_principal.issuer, "local-static");
}

#[test]
fn test_external_policy_config_parses_external_provider_block() {
    let json = r#"{
        "bind_addr": "127.0.0.1:9091",
        "max_request_size": 2048,
        "auth_provider": "external",
        "external_provider": {
            "issuer": "issuer-a",
            "audience": "flowable-timer"
        },
        "auth_keys": {}
    }"#;

    let file_path = "test_external_policy_config.json";
    std::fs::write(file_path, json).unwrap();

    let config = ServicePolicyConfig::load_from_file(file_path).unwrap();
    std::fs::remove_file(file_path).unwrap();

    let external_cfg = config
        .external_provider
        .expect("external provider config should be present");
    assert_eq!(external_cfg.issuer, "issuer-a");
    assert_eq!(external_cfg.audience, "flowable-timer");
}

#[test]
fn test_external_policy_config_parses_jwks_refresh_policy() {
    let json = r#"{
        "bind_addr": "127.0.0.1:9092",
        "max_request_size": 4096,
        "auth_provider": "external",
        "external_provider": {
            "trusted_profiles": [
                {
                    "id": "issuer-a",
                    "issuer": "https://issuer-a.example.com",
                    "audience": "flowable-timer",
                    "jwks_uri": "https://issuer-a.example.com/.well-known/jwks.json",
                    "jwks_cache_ttl_seconds": 120,
                    "jwks_refresh_policy": {
                        "min_refresh_interval_seconds": 15,
                        "backoff_multiplier": 3.0,
                        "max_retry_delay_seconds": 45,
                        "allow_stale_on_failure": true,
                        "stale_tolerance_seconds": 20,
                        "negative_cache_seconds": 10
                    }
                }
            ]
        },
        "auth_keys": {}
    }"#;

    let file_path = "test_external_jwks_refresh_policy_config.json";
    std::fs::write(file_path, json).unwrap();

    let config = ServicePolicyConfig::load_from_file(file_path).unwrap();
    std::fs::remove_file(file_path).unwrap();

    let external_cfg = config
        .external_provider
        .expect("external provider config should be present");
    assert_eq!(external_cfg.trusted_profiles.len(), 1);

    let profile = external_cfg.trusted_profiles.first().unwrap();
    assert_eq!(profile.jwks_refresh_policy.min_refresh_interval_seconds, 15);
    assert_eq!(profile.jwks_refresh_policy.backoff_multiplier, 3.0);
    assert_eq!(profile.jwks_refresh_policy.max_retry_delay_seconds, 45);
    assert!(profile.jwks_refresh_policy.allow_stale_on_failure);
    assert_eq!(profile.jwks_refresh_policy.stale_tolerance_seconds, 20);
    assert_eq!(profile.jwks_refresh_policy.negative_cache_seconds, 10);

    let issuer_profile = profile.to_issuer_profile();
    assert_eq!(
        issuer_profile
            .jwks_refresh_policy
            .min_refresh_interval_seconds,
        15
    );
    assert_eq!(issuer_profile.jwks_refresh_policy.backoff_multiplier, 3.0);
    assert_eq!(
        issuer_profile.jwks_refresh_policy.max_retry_delay_seconds,
        45
    );
    assert!(issuer_profile.jwks_refresh_policy.allow_stale_on_failure);
    assert_eq!(
        issuer_profile.jwks_refresh_policy.stale_tolerance_seconds,
        20
    );
    assert_eq!(
        issuer_profile.jwks_refresh_policy.negative_cache_seconds,
        10
    );
}
