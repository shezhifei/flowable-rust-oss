//! ServerConfig store aligned with Java admin domain + representation.

use super::crypto::PasswordCipher;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::RwLock;
use uuid::Uuid;

/// Endpoint type codes matching Java `EndpointType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(i32)]
pub enum EndpointType {
    Process = 1,
    Dmn = 2,
    Form = 3,
    Content = 4,
    Cmmn = 5,
    App = 6,
}

impl EndpointType {
    pub fn from_code(code: i32) -> Option<Self> {
        match code {
            1 => Some(Self::Process),
            2 => Some(Self::Dmn),
            3 => Some(Self::Form),
            4 => Some(Self::Content),
            5 => Some(Self::Cmmn),
            6 => Some(Self::App),
            _ => None,
        }
    }

    pub fn code(self) -> i32 {
        self as i32
    }

    pub fn all() -> [Self; 6] {
        [
            Self::Process,
            Self::Cmmn,
            Self::App,
            Self::Dmn,
            Self::Form,
            Self::Content,
        ]
    }
}

/// Internal stored config (password always encrypted at rest).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerConfig {
    pub id: String,
    pub name: String,
    pub description: String,
    pub server_address: String,
    pub port: i32,
    pub context_root: String,
    pub rest_root: String,
    pub user_name: String,
    /// AES/CBC encrypted password (base64), matching Java storage.
    pub password: String,
    pub endpoint_type: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tenant_id: Option<String>,
}

/// API representation — password omitted on read (Java `@JsonInclude(NON_NULL)`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerConfigRepresentation {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub name: String,
    pub description: String,
    pub server_address: String,
    pub server_port: i32,
    pub context_root: String,
    pub rest_root: String,
    pub user_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
    pub endpoint_type: i32,
}

impl From<&ServerConfig> for ServerConfigRepresentation {
    fn from(c: &ServerConfig) -> Self {
        Self {
            id: Some(c.id.clone()),
            name: c.name.clone(),
            description: c.description.clone(),
            server_address: c.server_address.clone(),
            server_port: c.port,
            context_root: c.context_root.clone(),
            rest_root: c.rest_root.clone(),
            user_name: c.user_name.clone(),
            password: None,
            endpoint_type: c.endpoint_type,
        }
    }
}

/// In-memory store seeded with defaults pointing at this process's engine REST.
pub struct ServerConfigStore {
    configs: RwLock<HashMap<String, ServerConfig>>,
    cipher: PasswordCipher,
}

impl ServerConfigStore {
    pub fn with_defaults() -> Self {
        let store = Self {
            configs: RwLock::new(HashMap::new()),
            cipher: PasswordCipher::from_env(),
        };
        store.seed_defaults();
        store
    }

    pub fn empty_for_tests(cipher: PasswordCipher) -> Self {
        Self {
            configs: RwLock::new(HashMap::new()),
            cipher,
        }
    }

    pub fn cipher(&self) -> &PasswordCipher {
        &self.cipher
    }

    fn seed_defaults(&self) {
        let port = std::env::var("FLOWABLE_UI_ENGINE_PORT")
            .ok()
            .and_then(|p| p.parse().ok())
            .unwrap_or(8080);
        let address = std::env::var("FLOWABLE_UI_ENGINE_HOST")
            .unwrap_or_else(|_| "http://127.0.0.1".into());
        let user = std::env::var("FLOWABLE_UI_ENGINE_USER").unwrap_or_else(|_| "admin".into());
        let password =
            std::env::var("FLOWABLE_UI_ENGINE_PASSWORD").unwrap_or_else(|_| "test".into());

        // Rust engine REST is flat (no process-api / cmmn-api prefixes).
        // Empty context/rest roots → URL is host:port/<engine-path>.
        for endpoint in EndpointType::all() {
            let (name, description) = default_meta(endpoint);
            let mut cfg = ServerConfig {
                id: Uuid::new_v4().to_string(),
                name: name.into(),
                description: description.into(),
                server_address: address.clone(),
                port,
                context_root: String::new(),
                rest_root: String::new(),
                user_name: user.clone(),
                password: password.clone(),
                endpoint_type: endpoint.code(),
                tenant_id: None,
            };
            cfg.password = self
                .cipher
                .encrypt(&cfg.password)
                .expect("default password encrypt");
            self.configs
                .write()
                .expect("server config lock")
                .insert(cfg.id.clone(), cfg);
        }
    }

    pub fn list_representations(&self) -> Vec<ServerConfigRepresentation> {
        let guard = self.configs.read().expect("server config lock");
        let mut list: Vec<_> = guard.values().map(ServerConfigRepresentation::from).collect();
        list.sort_by_key(|c| c.endpoint_type);
        list
    }

    pub fn get(&self, id: &str) -> Option<ServerConfig> {
        self.configs.read().expect("server config lock").get(id).cloned()
    }

    pub fn get_by_endpoint(&self, endpoint: EndpointType) -> Result<ServerConfig, String> {
        let guard = self.configs.read().expect("server config lock");
        let matches: Vec<_> = guard
            .values()
            .filter(|c| c.endpoint_type == endpoint.code())
            .cloned()
            .collect();
        match matches.len() {
            0 => Err("No server config found".into()),
            1 => Ok(matches.into_iter().next().unwrap()),
            _ => Err("Only one server config per endpoint type allowed".into()),
        }
    }

    pub fn decrypt_password(&self, config: &ServerConfig) -> Result<String, String> {
        self.cipher.decrypt(&config.password)
    }

    pub fn update(
        &self,
        server_id: &str,
        rep: ServerConfigRepresentation,
    ) -> Result<(), String> {
        let mut guard = self.configs.write().expect("server config lock");
        let config = guard
            .get_mut(server_id)
            .ok_or_else(|| format!("Server with id '{server_id}' does not exist"))?;

        if let Some(plain) = rep.password.filter(|p| !p.is_empty()) {
            config.password = self.cipher.encrypt(&plain)?;
        }
        config.context_root = rep.context_root;
        config.description = rep.description;
        config.name = rep.name;
        config.port = rep.server_port;
        config.rest_root = rep.rest_root;
        config.server_address = rep.server_address;
        config.user_name = rep.user_name;
        Ok(())
    }

    pub fn save_new(&self, mut config: ServerConfig, encrypt_password: bool) -> Result<(), String> {
        if encrypt_password {
            config.password = self.cipher.encrypt(&config.password)?;
        }
        if config.id.is_empty() {
            config.id = Uuid::new_v4().to_string();
        }
        self.configs
            .write()
            .expect("server config lock")
            .insert(config.id.clone(), config);
        Ok(())
    }

    pub fn default_representation(endpoint: EndpointType) -> ServerConfigRepresentation {
        let port = std::env::var("FLOWABLE_UI_ENGINE_PORT")
            .ok()
            .and_then(|p| p.parse().ok())
            .unwrap_or(8080);
        let address = std::env::var("FLOWABLE_UI_ENGINE_HOST")
            .unwrap_or_else(|_| "http://127.0.0.1".into());
        let (name, description) = default_meta(endpoint);
        ServerConfigRepresentation {
            id: None,
            name: name.into(),
            description: description.into(),
            server_address: address,
            server_port: port,
            context_root: String::new(),
            rest_root: String::new(),
            user_name: "admin".into(),
            password: Some("test".into()),
            endpoint_type: endpoint.code(),
        }
    }
}

fn default_meta(endpoint: EndpointType) -> (&'static str, &'static str) {
    match endpoint {
        EndpointType::Process => ("Flowable Process app", "Flowable Process REST config"),
        EndpointType::Cmmn => ("Flowable CMMN app", "Flowable CMMN REST config"),
        EndpointType::App => ("Flowable App app", "Flowable App REST config"),
        EndpointType::Dmn => ("Flowable DMN app", "Flowable DMN REST config"),
        EndpointType::Form => ("Flowable Form app", "Flowable Form REST config"),
        EndpointType::Content => ("Flowable Content app", "Flowable Content REST config"),
    }
}
