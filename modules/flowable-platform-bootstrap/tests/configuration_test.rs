use flowable_platform_bootstrap::PlatformConfiguration;
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

    /// Removes `key` for the duration of the guard so a stray value in the
    /// developer environment cannot mask an "absent variable" assertion.
    fn unset(key: &'static str) -> Self {
        let original_value = std::env::var(key).ok();
        unsafe {
            std::env::remove_var(key);
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

#[test]
fn loads_configuration_file_and_env_overrides() {
    let _environment_lock = environment_lock();
    let tempdir = tempfile::tempdir().expect("temp dir");
    let config_path = tempdir.path().join("flowable-platform.toml");
    fs::write(
        &config_path,
        r#"
[server]
bind_address = "127.0.0.1:9010"

[process]
engine_name = "file-engine"
database_path = "file-process.db"

[dmn]
database_path = "file-dmn.db"

[bootstrap]
create_default_admin = false
admin_user_id = "file-admin"
admin_password = "file-secret"
"#,
    )
    .expect("config file");

    let _server_bind = EnvVarGuard::set("FLOWABLE_SERVER_BIND_ADDRESS", "127.0.0.1:9020");
    let _engine_name = EnvVarGuard::set("FLOWABLE_PROCESS_ENGINE_NAME", "env-engine");
    let _process_db = EnvVarGuard::set("FLOWABLE_PROCESS_DATABASE_PATH", "env-process.db");
    let _dmn_db = EnvVarGuard::set("FLOWABLE_DMN_DATABASE_PATH", "env-dmn.db");
    let _create_admin = EnvVarGuard::set("FLOWABLE_BOOTSTRAP_CREATE_DEFAULT_ADMIN", "true");
    let _admin_password = EnvVarGuard::set("FLOWABLE_BOOTSTRAP_ADMIN_PASSWORD", "env-secret");

    let configuration =
        PlatformConfiguration::load_from_sources(Some(config_path)).expect("configuration");

    assert_eq!(configuration.server.bind_address, "127.0.0.1:9020");
    assert_eq!(configuration.process.engine_name, "env-engine");
    assert_eq!(configuration.process.database_path, "env-process.db");
    assert_eq!(
        configuration.dmn.database_path.as_deref(),
        Some("env-dmn.db")
    );
    assert!(configuration.bootstrap.create_default_admin);
    assert_eq!(configuration.bootstrap.admin_user_id, "file-admin");
    assert_eq!(configuration.bootstrap.admin_password, "env-secret");
}

#[test]
fn loads_canonical_style_properties_file_and_applies_env_overrides() {
    let _environment_lock = environment_lock();
    let tempdir = tempfile::tempdir().expect("temp dir");
    let config_path = tempdir.path().join("application.properties");
    fs::write(
        &config_path,
        r#"
server.address=0.0.0.0
server.port=9011
flowable.process.engine-name=properties-engine
spring.datasource.url=jdbc:sqlite:properties-process.db
flowable.dmn.datasource.url=jdbc:sqlite:properties-dmn.db
flowable.cmmn.datasource.url=jdbc:sqlite:properties-cmmn.db
flowable.app.datasource.url=jdbc:sqlite:properties-app.db
flowable.security.auth-mode=disabled
flowable.bootstrap.admin.enabled=false
flowable.bootstrap.admin.user-id=properties-admin
flowable.bootstrap.admin.password=properties-secret
"#,
    )
    .expect("config file");

    let _server_bind = EnvVarGuard::set("FLOWABLE_SERVER_BIND_ADDRESS", "127.0.0.1:9021");
    let _process_db = EnvVarGuard::set("FLOWABLE_PROCESS_DATABASE_PATH", "env-override-process.db");
    let _admin_password =
        EnvVarGuard::set("FLOWABLE_BOOTSTRAP_ADMIN_PASSWORD", "env-override-secret");

    let configuration =
        PlatformConfiguration::load_from_sources(Some(config_path)).expect("configuration");

    assert_eq!(configuration.server.bind_address, "127.0.0.1:9021");
    assert_eq!(configuration.process.engine_name, "properties-engine");
    assert_eq!(
        configuration.process.database_path,
        "env-override-process.db"
    );
    assert_eq!(
        configuration.dmn.database_path.as_deref(),
        Some("properties-dmn.db")
    );
    assert_eq!(
        configuration.cmmn.database_path.as_deref(),
        Some("properties-cmmn.db")
    );
    assert_eq!(
        configuration.app.database_path.as_deref(),
        Some("properties-app.db")
    );
    assert_eq!(configuration.security.auth_mode, "disabled");
    assert!(!configuration.bootstrap.create_default_admin);
    assert_eq!(configuration.bootstrap.admin_user_id, "properties-admin");
    assert_eq!(
        configuration.bootstrap.admin_password,
        "env-override-secret"
    );
}

#[test]
fn database_url_mysql_scheme_selects_mysql_kind() {
    let _environment_lock = environment_lock();
    let _url = EnvVarGuard::set(
        "FLOWABLE_DATABASE_URL",
        "mysql://flowable:flowable@127.0.0.1:3306/flowable",
    );
    let configuration = PlatformConfiguration::load_from_sources(None).unwrap();
    assert_eq!(
        configuration.process.database_kind.as_deref(),
        Some("mysql")
    );
    assert_eq!(
        configuration.process.database_url.as_deref(),
        Some("mysql://flowable:flowable@127.0.0.1:3306/flowable")
    );
}

#[test]
fn database_url_postgres_scheme_selects_postgres_kind() {
    let _environment_lock = environment_lock();
    let _url = EnvVarGuard::set(
        "FLOWABLE_DATABASE_URL",
        "postgres://postgres:postgres@127.0.0.1:5432/flowable",
    );
    let configuration = PlatformConfiguration::load_from_sources(None).unwrap();
    assert_eq!(
        configuration.process.database_kind.as_deref(),
        Some("postgres")
    );
}

#[test]
fn database_url_shares_one_backend_across_modules() {
    let _environment_lock = environment_lock();
    let _url = EnvVarGuard::set(
        "FLOWABLE_DATABASE_URL",
        "mysql://flowable:flowable@127.0.0.1:3306/flowable",
    );
    let configuration = PlatformConfiguration::load_from_sources(None).unwrap();
    for module in [
        &configuration.dmn,
        &configuration.cmmn,
        &configuration.app,
    ] {
        assert_eq!(module.database_kind.as_deref(), Some("mysql"));
        assert_eq!(
            module.database_url.as_deref(),
            Some("mysql://flowable:flowable@127.0.0.1:3306/flowable")
        );
    }
}

#[test]
fn absent_database_url_keeps_sqlite_path() {
    let _environment_lock = environment_lock();
    let _url = EnvVarGuard::unset("FLOWABLE_DATABASE_URL");
    let configuration = PlatformConfiguration::load_from_sources(None).unwrap();
    assert!(configuration.process.database_url.is_none());
    assert!(configuration.process.database_kind.is_none());
    assert!(!configuration.process.database_path.is_empty());
}
