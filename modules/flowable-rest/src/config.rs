use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::identity::entities::User;
use flowable_platform_bootstrap::PlatformConfiguration;
use serde::{Deserialize, Serialize};
use std::env;

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
        Self {
            enabled: false,
            user_id: "admin".to_string(),
            password: "admin".to_string(),
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
        Self {
            bind_address: config.server.bind_address.clone(),
            database_path: config.process.database_path.clone(),
            engine_name: config.process.engine_name.clone(),
            security: RestSecurityConfig {
                auth: RestAuthConfig {
                    mode: RestAuthMode::from_platform_auth_mode(&config.security.auth_mode),
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

        config
    }

    pub fn without_identity_seed(mut self) -> Self {
        self.security.admin_seed.enabled = false;
        self
    }

    pub fn apply_identity_seed(&self, engine: &ProcessEngine) {
        if !self.security.admin_seed.enabled {
            return;
        }

        engine.get_identity_service().save_user(User {
            id: self.security.admin_seed.user_id.clone(),
            first_name: self.security.admin_seed.first_name.clone(),
            last_name: self.security.admin_seed.last_name.clone(),
            email: self.security.admin_seed.email.clone(),
            password: Some(self.security.admin_seed.password.clone()),
            tenant_id: None,
        });
    }
}

fn parse_bool(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "true" | "1" | "yes" | "on"
    )
}
