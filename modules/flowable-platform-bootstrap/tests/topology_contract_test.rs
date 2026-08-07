use flowable_platform_bootstrap::{
    CertifiedTopologyProfile, FlowablePlatform, PlatformConfiguration,
};

fn base_configuration() -> PlatformConfiguration {
    let mut configuration = PlatformConfiguration::default();
    configuration.process.engine_name = "m33-topology-contract".to_string();
    configuration.process.database_path = ":memory:".to_string();
    configuration.bootstrap.create_default_admin = false;
    configuration.operations.exposure = "jmx-bridge".to_string();
    configuration.operations.management_api_enabled = true;
    configuration
}

#[test]
fn bootstraps_reverse_proxy_terminated_topology_contract() {
    let mut configuration = base_configuration();
    configuration.security.auth_mode = "basic".to_string();
    configuration.embedding.mode = "standalone".to_string();
    configuration.embedding.profile = "standalone-service".to_string();
    configuration.topology.profile = "reverse-proxy-terminated".to_string();

    let platform = FlowablePlatform::bootstrap(configuration).expect("platform");
    let contract = platform.topology_certification_contract();

    assert_eq!(
        contract.profile,
        CertifiedTopologyProfile::ReverseProxyTerminated
    );
    assert!(contract.startup_certified);
    assert!(contract.auth_certified);
    assert!(contract.cutover_certified);
    assert!(contract.rollback_certified);
    assert!(contract.recovery_certified);
    assert!(
        contract
            .supported_historical_ingress
            .contains(&"raw-mysql-dump".to_string())
    );
}

#[test]
fn reverse_proxy_terminated_topology_fails_fast_when_auth_mode_is_not_basic() {
    let mut configuration = base_configuration();
    configuration.security.auth_mode = "disabled".to_string();
    configuration.embedding.mode = "standalone".to_string();
    configuration.embedding.profile = "standalone-service".to_string();
    configuration.topology.profile = "reverse-proxy-terminated".to_string();

    let error = FlowablePlatform::bootstrap(configuration)
        .err()
        .expect("expected failure");

    assert_eq!(
        error.to_string(),
        "Topology profile 'reverse-proxy-terminated' requires basic auth mode"
    );
}

#[test]
fn bootstraps_cdi_sidecar_and_osgi_operations_topology_contracts() {
    let mut cdi_configuration = base_configuration();
    cdi_configuration.security.auth_mode = "disabled".to_string();
    cdi_configuration.embedding.mode = "embedded".to_string();
    cdi_configuration.embedding.profile = "cdi-compatible".to_string();
    cdi_configuration.enterprise.adapters = vec!["camel".to_string(), "cdi".to_string()];
    cdi_configuration.topology.profile = "cdi-sidecar".to_string();

    let cdi_platform = FlowablePlatform::bootstrap(cdi_configuration).expect("cdi platform");
    let cdi_contract = cdi_platform.topology_certification_contract();
    assert_eq!(cdi_contract.profile, CertifiedTopologyProfile::CdiSidecar);
    assert!(cdi_contract.startup_certified);
    assert!(cdi_contract.auth_certified);
    assert!(cdi_contract.recovery_certified);
    assert!(!cdi_contract.cutover_certified);

    let mut osgi_configuration = base_configuration();
    osgi_configuration.security.auth_mode = "disabled".to_string();
    osgi_configuration.embedding.mode = "embedded".to_string();
    osgi_configuration.embedding.profile = "osgi-managed".to_string();
    osgi_configuration.enterprise.adapters =
        vec!["camel".to_string(), "cxf".to_string(), "osgi".to_string()];
    osgi_configuration.topology.profile = "osgi-operations-node".to_string();

    let osgi_platform = FlowablePlatform::bootstrap(osgi_configuration).expect("osgi platform");
    let osgi_contract = osgi_platform.topology_certification_contract();
    assert_eq!(
        osgi_contract.profile,
        CertifiedTopologyProfile::OsgiOperationsNode
    );
    assert!(osgi_contract.startup_certified);
    assert!(osgi_contract.auth_certified);
    assert!(osgi_contract.recovery_certified);
    assert!(!osgi_contract.cutover_certified);
}

#[test]
fn bootstraps_declared_external_service_mesh_topology_contract() {
    let mut configuration = base_configuration();
    configuration.security.auth_mode = "basic".to_string();
    configuration.embedding.mode = "standalone".to_string();
    configuration.embedding.profile = "standalone-service".to_string();
    configuration.topology.profile = "declared-external".to_string();
    configuration.topology.ingress = "service-mesh-terminated".to_string();
    configuration.topology.packaging = "standalone-service".to_string();

    let platform = FlowablePlatform::bootstrap(configuration).expect("platform");
    let contract = platform.topology_certification_contract();

    assert_eq!(contract.profile, CertifiedTopologyProfile::DeclaredExternal);
    assert_eq!(contract.ingress, "service-mesh-terminated");
    assert_eq!(contract.packaging, "standalone-service");
    assert!(contract.startup_certified);
    assert!(contract.auth_certified);
    assert!(contract.cutover_certified);
    assert!(contract.rollback_certified);
    assert!(contract.recovery_certified);
    assert!(
        contract
            .supported_historical_ingress
            .contains(&"source-manifest".to_string())
    );
}
