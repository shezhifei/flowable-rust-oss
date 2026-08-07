use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::identity::entities::User;
use flowable_platform_bootstrap::PlatformConfiguration;
use serde::{Deserialize, Serialize};
use std::env;

/// Well-known default password rejected when admin seeding is enabled.
/// Security deviation from Java weak default admin/admin.
pub const DEFAULT_ADMIN_PASSWORD: &str = "admin";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum RestAuthMode {
    #[default]
    Basic,
    Disabled,
}

impl RestAuthMode {
    pub fn is_enforced(&self) -> bool {
        matches!(self, Self::Basic)
    }

    pub fn from_platform_auth_mode(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "disabled" => Self::Disabled,
            _ => Self::Basic,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RestAuthConfig {
    #[serde(default)]
    pub mode: RestAuthMode,
    /// User ids treated as admin for privileged write paths (deployments, IDM writes,
    /// management writes). Least-invasive admin concept: config list rather than an
    /// identity-service schema change (User has no admin flag).
    #[serde(default)]
    pub admin_users: Vec<String>,
}

impl RestAuthConfig {
    pub fn is_admin_user(&self, user_id: &str) -> bool {
        self.admin_users.iter().any(|u| u == user_id)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RestAdminSeedConfig {
    pub enabled: bool,
    pub user_id: String,
    pub password: String,
    #[serde(default)]
    pub first_name: Option<String>,
    #[serde(default)]
    pub last_name: Option<String>,
    #[serde(default)]
    pub email: Option<String>,
}

impl Default for RestAdminSeedConfig {
    fn default() -> Self {
        // enabled: false — no default admin. password field retains "admin" only as a
        // placeholder that is rejected when seeding is enabled (security deviation from Java).
        Self {
            enabled: false,
            user_id: "admin".to_string(),
            password: DEFAULT_ADMIN_PASSWORD.to_string(),
            first_name: None,
            last_name: None,
            email: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RestSecurityConfig {
    #[serde(default)]
    pub auth: RestAuthConfig,
    #[serde(default)]
    pub admin_seed: RestAdminSeedConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RestConfig {
    pub bind_address: String,
    pub database_path: String,
    pub engine_name: String,
    #[serde(default)]
    pub security: RestSecurityConfig,
}

impl Default for RestConfig {
    fn default() -> Self {
        Self {
            bind_address: "127.0.0.1:8080".to_string(),
            database_path: "flowable-rest.db".to_string(),
            engine_name: "flowable-rest-engine".to_string(),
            security: RestSecurityConfig::default(),
        }
    }
}

impl RestConfig {
    pub fn from_platform_configuration(config: &PlatformConfiguration) -> Self {
        // When platform bootstrap creates the admin user, mark that user as REST admin.
        let mut admin_users = Vec::new();
        if config.bootstrap.create_default_admin {
            admin_users.push(config.bootstrap.admin_user_id.clone());
        }
        Self {
            bind_address: config.server.bind_address.clone(),
            database_path: config.process.database_path.clone(),
            engine_name: config.process.engine_name.clone(),
            security: RestSecurityConfig {
                auth: RestAuthConfig {
                    mode: RestAuthMode::from_platform_auth_mode(&config.security.auth_mode),
                    admin_users,
                },
                admin_seed: RestAdminSeedConfig {
                    enabled: false,
                    user_id: config.bootstrap.admin_user_id.clone(),
                    password: config.bootstrap.admin_password.clone(),
                    ..RestAdminSeedConfig::default()
                },
            },
        }
    }

    pub fn from_env() -> Self {
        let mut config = Self::default();

        if let Ok(value) = env::var("FLOWABLE_REST_BIND_ADDRESS") {
            config.bind_address = value;
        }
        if let Ok(value) = env::var("FLOWABLE_REST_DB_PATH") {
            config.database_path = value;
        }
        if let Ok(value) = env::var("FLOWABLE_REST_ENGINE_NAME") {
            config.engine_name = value;
        }
        if let Ok(value) = env::var("FLOWABLE_REST_AUTH_MODE") {
            config.security.auth.mode = RestAuthMode::from_platform_auth_mode(&value);
        }
        if let Ok(value) = env::var("FLOWABLE_REST_ADMIN_USERS") {
            config.security.auth.admin_users = parse_admin_users(&value);
        }
        if let Ok(value) = env::var("FLOWABLE_REST_ADMIN_SEED_ENABLED") {
            config.security.admin_seed.enabled = parse_bool(&value);
        }
        if let Ok(value) = env::var("FLOWABLE_REST_ADMIN_USER_ID") {
            config.security.admin_seed.user_id = value;
        }
        if let Ok(value) = env::var("FLOWABLE_REST_ADMIN_PASSWORD") {
            config.security.admin_seed.password = value;
        }
        if let Ok(value) = env::var("FLOWABLE_REST_ADMIN_FIRST_NAME") {
            config.security.admin_seed.first_name = Some(value);
        }
        if let Ok(value) = env::var("FLOWABLE_REST_ADMIN_LAST_NAME") {
            config.security.admin_seed.last_name = Some(value);
        }
        if let Ok(value) = env::var("FLOWABLE_REST_ADMIN_EMAIL") {
            config.security.admin_seed.email = Some(value);
        }

        // When seed is enabled, ensure the seed user is an admin unless an explicit list was set.
        if config.security.admin_seed.enabled
            && !config
                .security
                .auth
                .admin_users
                .iter()
                .any(|u| u == &config.security.admin_seed.user_id)
        {
            config
                .security
                .auth
                .admin_users
                .push(config.security.admin_seed.user_id.clone());
        }

        config
    }

    pub fn without_identity_seed(mut self) -> Self {
        self.security.admin_seed.enabled = false;
        self
    }

    /// Convenience used by the test-oriented `run_server` helper: treat user id `admin`
    /// as a privileged admin for write paths without seeding credentials.
    pub fn with_test_admin_user(mut self) -> Self {
        if !self.security.auth.admin_users.iter().any(|u| u == "admin") {
            self.security.auth.admin_users.push("admin".to_string());
        }
        self
    }

    /// Validate security settings before serving. Returns an error when startup must fail.
    pub fn validate_for_startup(&self) -> Result<(), String> {
        if self.security.admin_seed.enabled
            && self.security.admin_seed.password == DEFAULT_ADMIN_PASSWORD
        {
            return Err(
                "Refusing to seed REST admin with password \"admin\". \
                 Set security.admin_seed.password (or FLOWABLE_REST_ADMIN_PASSWORD) \
                 to a non-default value when admin_seed.enabled is true \
                 (security deviation from Java weak default admin/admin)."
                    .to_string(),
            );
        }

        if !self.security.auth.mode.is_enforced() {
            tracing::warn!(
                "FLOWABLE_REST_AUTH_MODE=disabled (or auth.mode=disabled): REST authentication \
                 is OFF. All API endpoints are unauthenticated. Do not expose this bind address \
                 beyond loopback."
            );
            if !is_loopback_bind_address(&self.bind_address) {
                return Err(format!(
                    "Refusing to start with auth disabled on non-loopback bind address '{}'. \
                     Use a loopback address (127.0.0.1 / ::1 / localhost) or enable \
                     FLOWABLE_REST_AUTH_MODE=basic.",
                    self.bind_address
                ));
            }
        }

        Ok(())
    }

    pub fn apply_identity_seed(&self, engine: &ProcessEngine) -> Result<(), String> {
        if !self.security.admin_seed.enabled {
            return Ok(());
        }

        if self.security.admin_seed.password == DEFAULT_ADMIN_PASSWORD {
            return Err(
                "Refusing to seed REST admin with password \"admin\". \
                 Set security.admin_seed.password (or FLOWABLE_REST_ADMIN_PASSWORD) \
                 to a non-default value when admin_seed.enabled is true \
                 (security deviation from Java weak default admin/admin)."
                    .to_string(),
            );
        }

        engine.get_identity_service().save_user(User {
            id: self.security.admin_seed.user_id.clone(),
            first_name: self.security.admin_seed.first_name.clone(),
            last_name: self.security.admin_seed.last_name.clone(),
            email: self.security.admin_seed.email.clone(),
            password: Some(self.security.admin_seed.password.clone()),
            tenant_id: None,
        });
        Ok(())
    }
}

fn parse_bool(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "true" | "1" | "yes" | "on"
    )
}

fn parse_admin_users(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

/// True when the bind host is loopback (127.0.0.1, ::1, localhost) or unspecified port-only.
pub fn is_loopback_bind_address(bind_address: &str) -> bool {
    let host = bind_address
        .rsplit_once(':')
        .map(|(h, _)| h)
        .unwrap_or(bind_address)
        .trim_matches(|c| c == '[' || c == ']');
    matches!(host, "127.0.0.1" | "::1" | "localhost" | "0:0:0:0:0:0:0:1")
        || host.eq_ignore_ascii_case("localhost")
}
