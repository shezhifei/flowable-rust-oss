use flowable_platform_bootstrap::{
    FlowablePlatform, OperationsExposureKind, OperationsObjectFamilyBreadth, PlatformConfiguration,
};
use std::path::Path;

fn write_platform_config(config_path: &Path, database_path: &Path, exposure: &str) {
    let escaped_database_path = database_path.display().to_string().replace('\\', "\\\\");
    let config = format!(
        r#"
[server]
bind_address = "127.0.0.1:0"

[process]
engine_name = "m36-operations-jmx-contract"
database_path = "{database_path}"

[security]
auth_mode = "disabled"

[bootstrap]
create_default_admin = false
admin_user_id = "admin"
admin_password = "ignored"

[operations]
exposure = "{exposure}"
management_api_enabled = true
"#,
        database_path = escaped_database_path,
        exposure = exposure,
    );
    std::fs::write(config_path, config).expect("config");
}

#[test]
fn bootstrap_resolves_native_compatible_jmx_contract_from_platform_config() {
    let tempdir = tempfile::tempdir().expect("temp dir");
    let config_path = tempdir.path().join("flowable-platform.toml");
    write_platform_config(
        &config_path,
        &tempdir.path().join("process-engine.sqlite"),
        "jmx-native-compatible",
    );

    let platform = FlowablePlatform::bootstrap_from_sources(Some(config_path)).expect("platform");
    let contract = platform.operations_support_contract();

    assert_eq!(
        contract.exposure,
        OperationsExposureKind::JmxNativeCompatible
    );
    assert!(contract.management_api_enabled);
    assert!(contract.runtime_ledger_enabled);
    assert!(contract.timer_ledger_enabled);
    assert!(contract.topology_ledger_enabled);
    assert!(contract.native_compatible_connector_enabled);
    assert!(contract.mbean_registry_enabled);
    assert!(contract.operations_bus_enabled);
    assert_eq!(
        contract.object_family_breadth,
        OperationsObjectFamilyBreadth::CoreRuntimeAndPlatformLedgers
    );
    assert!(
        contract.support_statement.contains("native-compatible"),
        "support statement should explicitly describe native-compatible JMX closure"
    );
    assert!(
        contract
            .support_statement
            .contains("does not emulate an external RMI connector server"),
        "contract must explicitly disclaim an external RMI connector server"
    );
}

#[test]
fn bootstrap_keeps_bounded_jmx_bridge_contract_compatible() {
    let mut configuration = PlatformConfiguration::default();
    configuration.process.engine_name = "m36-jmx-bridge-contract".to_string();
    configuration.process.database_path = ":memory:".to_string();
    configuration.bootstrap.create_default_admin = false;
    configuration.security.auth_mode = "disabled".to_string();
    configuration.operations.exposure = "jmx-bridge".to_string();
    configuration.operations.management_api_enabled = true;

    let platform = FlowablePlatform::bootstrap(configuration).expect("platform");
    let contract = platform.operations_support_contract();

    assert_eq!(contract.exposure, OperationsExposureKind::JmxBridge);
    assert!(contract.management_api_enabled);
    assert!(contract.runtime_ledger_enabled);
    assert!(contract.timer_ledger_enabled);
    assert!(contract.topology_ledger_enabled);
    assert!(!contract.native_compatible_connector_enabled);
    assert!(!contract.mbean_registry_enabled);
    assert!(!contract.operations_bus_enabled);
    assert_eq!(
        contract.object_family_breadth,
        OperationsObjectFamilyBreadth::LedgersOnly
    );
    assert!(
        contract
            .support_statement
            .contains("bounded management API bridge"),
        "management bridge contract should stay explicit about its bounded transport"
    );
}
