use flowable_platform_bootstrap::{
    EnterpriseAdapterFamily, EnterpriseSupportKind, FlowablePlatform, PlatformConfiguration,
    RuntimeEmbeddingMode, RuntimeEmbeddingProfile,
};
use std::fs;
use std::sync::{Mutex, MutexGuard, OnceLock};

fn environment_lock() -> MutexGuard<'static, ()> {
    static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    ENV_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .expect("environment lock")
}

struct EnvVarGuard {
    key: &'static str,
    original_value: Option<String>,
}

impl EnvVarGuard {
    fn set(key: &'static str, value: &str) -> Self {
        let original_value = std::env::var(key).ok();
        unsafe {
            std::env::set_var(key, value);
        }
        Self {
            key,
            original_value,
        }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        unsafe {
            if let Some(value) = &self.original_value {
                std::env::set_var(self.key, value);
            } else {
                std::env::remove_var(self.key);
            }
        }
    }
}

fn embedded_platform_configuration(profile: &str, adapters: &[&str]) -> PlatformConfiguration {
    let mut configuration = PlatformConfiguration::default();
    configuration.process.engine_name = "m26-embedding-contract".to_string();
    configuration.process.database_path = ":memory:".to_string();
    configuration.embedding.mode = "embedded".to_string();
    configuration.embedding.profile = profile.to_string();
    configuration.enterprise.adapters = adapters.iter().map(|item| (*item).to_string()).collect();
    configuration.bootstrap.create_default_admin = false;
    configuration
}

#[test]
fn loads_embedding_contract_configuration_from_toml_sources() {
    let _environment_lock = environment_lock();
    let tempdir = tempfile::tempdir().expect("temp dir");
    let config_path = tempdir.path().join("flowable-platform.toml");
    fs::write(
        &config_path,
        r#"
[process]
engine_name = "m26-toml"
database_path = ":memory:"

[embedding]
mode = "embedded"
profile = "osgi-managed"

[enterprise]
adapters = ["camel", "osgi"]
"#,
    )
    .expect("config file");

    let configuration =
        PlatformConfiguration::load_from_sources(Some(config_path)).expect("configuration");

    assert_eq!(configuration.embedding.mode, "embedded");
    assert_eq!(configuration.embedding.profile, "osgi-managed");
    assert_eq!(
        configuration.enterprise.adapters,
        vec!["camel".to_string(), "osgi".to_string()]
    );
}

#[test]
fn loads_embedding_contract_configuration_from_properties_and_env_overrides() {
    let _environment_lock = environment_lock();
    let tempdir = tempfile::tempdir().expect("temp dir");
    let config_path = tempdir.path().join("application.properties");
    fs::write(
        &config_path,
        r#"
flowable.embedding.mode=standalone
flowable.embedding.profile=standalone-service
flowable.enterprise.adapters=camel,cxf
"#,
    )
    .expect("config file");

    let _embedding_mode = EnvVarGuard::set("FLOWABLE_EMBEDDING_MODE", "embedded");
    let _embedding_profile = EnvVarGuard::set("FLOWABLE_EMBEDDING_PROFILE", "cdi-compatible");
    let _enterprise_adapters = EnvVarGuard::set("FLOWABLE_ENTERPRISE_ADAPTERS", "camel,cdi");

    let configuration =
        PlatformConfiguration::load_from_sources(Some(config_path)).expect("configuration");

    assert_eq!(configuration.embedding.mode, "embedded");
    assert_eq!(configuration.embedding.profile, "cdi-compatible");
    assert_eq!(
        configuration.enterprise.adapters,
        vec!["camel".to_string(), "cdi".to_string()]
    );
}

#[test]
fn bootstrap_fails_fast_for_incompatible_embedding_mode_and_profile() {
    let mut configuration = PlatformConfiguration::default();
    configuration.process.database_path = ":memory:".to_string();
    configuration.embedding.mode = "standalone".to_string();
    configuration.embedding.profile = "cdi-compatible".to_string();
    configuration.bootstrap.create_default_admin = false;

    let error = FlowablePlatform::bootstrap(configuration)
        .err()
        .expect("expected failure");

    assert_eq!(
        error.to_string(),
        "Embedding mode 'standalone' is incompatible with profile 'cdi-compatible'"
    );
}

#[test]
fn bootstrap_fails_fast_for_unsupported_enterprise_adapter_profile_combination() {
    let configuration = embedded_platform_configuration("osgi-managed", &["camel", "cdi"]);

    let error = FlowablePlatform::bootstrap(configuration)
        .err()
        .expect("expected failure");

    assert_eq!(
        error.to_string(),
        "Enterprise adapter 'Cdi' is not supported for embedding profile 'OsgiManaged'"
    );
}

#[test]
fn bootstraps_embedded_profile_and_exposes_enterprise_support_contracts() {
    let configuration = embedded_platform_configuration("cdi-compatible", &["camel", "cdi"]);

    let platform = FlowablePlatform::bootstrap(configuration).expect("platform");
    let embedding_contract = platform.runtime_embedding_contract();

    assert_eq!(embedding_contract.mode, RuntimeEmbeddingMode::Embedded);
    assert_eq!(
        embedding_contract.profile,
        RuntimeEmbeddingProfile::CdiCompatible
    );
    assert_eq!(
        embedding_contract.adapters,
        vec![EnterpriseAdapterFamily::Camel, EnterpriseAdapterFamily::Cdi]
    );

    let support_contracts = platform.enterprise_adapter_support_contracts();
    assert_eq!(support_contracts.len(), 2);

    assert_eq!(support_contracts[0].family, EnterpriseAdapterFamily::Camel);
    assert_eq!(
        support_contracts[0].support_kind,
        EnterpriseSupportKind::ReplacementArchitecture
    );
    assert_eq!(
        support_contracts[0].supported_profiles,
        vec![
            RuntimeEmbeddingProfile::StandaloneService,
            RuntimeEmbeddingProfile::CdiCompatible,
            RuntimeEmbeddingProfile::OsgiManaged,
        ]
    );
    assert_eq!(
        support_contracts[0].external_source_anchor,
        "org.flowable.camel.*"
    );
    assert!(
        support_contracts[0]
            .support_statement
            .contains("shared runtime embedding contract")
    );

    assert_eq!(support_contracts[1].family, EnterpriseAdapterFamily::Cdi);
    assert_eq!(
        support_contracts[1].support_kind,
        EnterpriseSupportKind::CompatibilityLayer
    );
    assert_eq!(
        support_contracts[1].supported_profiles,
        vec![RuntimeEmbeddingProfile::CdiCompatible]
    );
    assert_eq!(
        support_contracts[1].external_source_anchor,
        "org.flowable.cdi.*"
    );
    assert!(
        support_contracts[1]
            .support_statement
            .contains("bounded embedding adapter")
    );

    assert_eq!(
        platform.enterprise_support_statement(),
        "bounded enterprise runtime embedding contract: mode=Embedded, profile=CdiCompatible, adapters=camel, cdi"
    );
}
