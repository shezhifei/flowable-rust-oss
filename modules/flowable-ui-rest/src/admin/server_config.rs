// Pre-existing `unwrap()` call(s), grandfathered by the workspace clippy ratchet
// (`[workspace.lints.clippy] unwrap_used = "warn"` in the root Cargo.toml). These
// sites predate the ratchet and were NOT individually audited against Java. The
// exemption is scoped with `cfg_attr(test, ...)`, so it covers only this file's
// `#[cfg(test)]` code; a NEW unwrap() in production code is still surfaced.
// Do not add more without an audit note.
#![cfg_attr(test, allow(clippy::unwrap_used))]

//! ServerConfig store aligned with Java admin domain + representation.
//! Durable via JSON file (path from `FLOWABLE_UI_SERVER_CONFIG_PATH` or
//! `./data/ui-admin-server-configs.json`).

use super::crypto::PasswordCipher;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::RwLock;
use uuid::Uuid;

/// Failure kind of a server-config store operation.
///
/// Java parity:
/// - `Missing` is exactly the case the resource turns into
///   `BadRequestException` — `ServerConfigsResource.updateServer` throws it when
///   `serverConfigService.findOne(serverId)` returns `null`
///   (`flowable-engine-6.8.0/.../ui/admin/rest/ServerConfigsResource.java:67-71`),
///   which `RestExceptionHandlerAdvice.handleBadRequest` answers with HTTP 400
///   (`.../ui/common/rest/exception/RestExceptionHandlerAdvice.java:58-63`).
/// - `Storage` covers encryption / persistence failures. Java lets those
///   `RuntimeException`s escape `AbstractEncryptingService.encrypt/decrypt`
///   (`.../admin/service/engine/AbstractEncryptingService.java:48-69`, both
///   `throw new RuntimeException(nsae)`) and the MyBatis
///   `ServerConfigRepositoryImpl.save` call
///   (`.../admin/repository/ServerConfigRepositoryImpl.java:52-60`), so Spring's
///   default handler answers HTTP 500 — `RestExceptionHandlerAdvice` declares no
///   handler for `RuntimeException`, and the same surface uses
///   `InternalServerErrorException` for persistence failures elsewhere, e.g.
///   `RelatedContentResource.java:80` "ContentItem on task could not be saved".
///   Reporting them as 400 would blame the caller for a server-side failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerConfigStoreError {
    /// No config with that id — HTTP 400 in Java (`BadRequestException`).
    Missing(String),
    /// Encryption or persistence failure — HTTP 500 in Java.
    Storage(String),
}

impl std::fmt::Display for ServerConfigStoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Missing(message) | Self::Storage(message) => f.write_str(message),
        }
    }
}

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

fn default_store_path() -> PathBuf {
    std::env::var("FLOWABLE_UI_SERVER_CONFIG_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("./data/ui-admin-server-configs.json"))
}

/// In-memory store with optional JSON file durability.
pub struct ServerConfigStore {
    configs: RwLock<HashMap<String, ServerConfig>>,
    cipher: PasswordCipher,
    path: PathBuf,
}

impl ServerConfigStore {
    /// Builds the production store from env and loads/seeds the JSON file.
    ///
    /// Fail-closed (P1-D): an invalid `FLOWABLE_ADMIN_CREDENTIALS_IV` /
    /// `..._SECRET` length is returned as an `Err` at startup instead of
    /// silently reverting to the public default cipher. Java aborts bean
    /// construction for the same misconfiguration
    /// (`AbstractEncryptingService.java:39-46`), so callers propagate this to
    /// the process boundary rather than encrypting credentials with a known key.
    pub fn with_defaults() -> Result<Self, String> {
        let path = default_store_path();
        let store = Self {
            configs: RwLock::new(HashMap::new()),
            cipher: PasswordCipher::from_env()?,
            path,
        };
        store.load_or_seed();
        Ok(store)
    }

    /// Loads persisted configs, seeding the built-in defaults on first run.
    ///
    /// The seeded configs are inserted into memory first and therefore stay
    /// usable even when persisting them fails: a disk/permission problem at
    /// startup must not block the in-memory admin surface. The persistence
    /// failure still has to be loud, because without a log the seeded defaults
    /// silently vanish on the next restart (P3 observability gap).
    fn load_or_seed(&self) {
        if self.load_from_disk() {
            return;
        }
        self.seed_defaults();
        if let Err(error) = self.persist() {
            tracing::error!(
                path = %self.path.display(),
                "failed to persist seeded default admin server configs: {error}; \
                 the in-memory configs stay usable for this process, but the seeded \
                 defaults may be lost on restart"
            );
        }
    }

    pub fn empty_for_tests(cipher: PasswordCipher) -> Self {
        Self {
            configs: RwLock::new(HashMap::new()),
            cipher,
            path: std::env::temp_dir().join(format!("flowable-ui-sc-test-{}.json", Uuid::new_v4())),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn cipher(&self) -> &PasswordCipher {
        &self.cipher
    }

    fn load_from_disk(&self) -> bool {
        let Ok(bytes) = std::fs::read(&self.path) else {
            return false;
        };
        let Ok(list) = serde_json::from_slice::<Vec<ServerConfig>>(&bytes) else {
            return false;
        };
        if list.is_empty() {
            return false;
        }
        let mut guard = self.configs.write().unwrap_or_else(|e| e.into_inner());
        guard.clear();
        for cfg in list {
            guard.insert(cfg.id.clone(), cfg);
        }
        true
    }

    fn persist(&self) -> Result<(), String> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let guard = self.configs.read().unwrap_or_else(|e| e.into_inner());
        let mut list: Vec<_> = guard.values().cloned().collect();
        list.sort_by_key(|c| c.endpoint_type);
        let bytes = serde_json::to_vec_pretty(&list).map_err(|e| e.to_string())?;
        std::fs::write(&self.path, bytes).map_err(|e| e.to_string())
    }

    /// True when a server config row for `server_id` exists.
    ///
    /// Used by the REST layer to separate the one case Java reports as a client
    /// error from the cases it reports as a server error.
    pub fn contains(&self, server_id: &str) -> bool {
        self.configs
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .contains_key(server_id)
    }

    /// Updates an existing server config, reporting the failure kind.
    ///
    /// Java parity (see [`ServerConfigStoreError`]): the missing-row case is a
    /// `BadRequestException` (400) while an encrypt/persist failure is a
    /// `RuntimeException` (500), so the two must not collapse into one status.
    pub fn update_or_missing(
        &self,
        server_id: &str,
        rep: ServerConfigRepresentation,
    ) -> Result<(), ServerConfigStoreError> {
        if !self.contains(server_id) {
            return Err(ServerConfigStoreError::Missing(format!(
                "Server with id '{server_id}' does not exist"
            )));
        }
        self.update(server_id, rep)
            .map_err(ServerConfigStoreError::Storage)
    }

    fn seed_defaults(&self) {
        let port = std::env::var("FLOWABLE_UI_ENGINE_PORT")
            .ok()
            .and_then(|p| p.parse().ok())
            .unwrap_or(8080);
        let address =
            std::env::var("FLOWABLE_UI_ENGINE_HOST").unwrap_or_else(|_| "http://127.0.0.1".into());
        let user = std::env::var("FLOWABLE_UI_ENGINE_USER").unwrap_or_else(|_| "admin".into());
        let password =
            std::env::var("FLOWABLE_UI_ENGINE_PASSWORD").unwrap_or_else(|_| "test".into());

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
            // Never persist a cleartext secret. `ServerConfig` documents its `password` as
            // "always encrypted at rest" (doc comment above this struct), and the previous
            // `.unwrap_or_else(|_| cfg.password.clone())` wrote the cleartext password to disk
            // whenever the cipher failed — a silent downgrade that turned a crypto fault into a
            // plaintext-on-disk vulnerability. Java has no such fallback: the admin
            // `EncryptingService` throws and the failure propagates instead of degrading, so a
            // startup that cannot encrypt is not trustworthy and must not proceed.
            cfg.password = match self.cipher.encrypt(&cfg.password) {
                Ok(encrypted) => encrypted,
                Err(error) => panic!(
                    "refusing to seed the default server configs: password encryption failed ({error}). \
                     Persisting the cleartext password would break the encrypted-at-rest contract on \
                     `ServerConfig`, so startup is aborted instead."
                ),
            };
            self.configs
                .write()
                .unwrap_or_else(|e| e.into_inner())
                .insert(cfg.id.clone(), cfg);
        }
    }

    pub fn list_representations(&self) -> Vec<ServerConfigRepresentation> {
        let guard = self.configs.read().unwrap_or_else(|e| e.into_inner());
        let mut list: Vec<_> = guard
            .values()
            .map(ServerConfigRepresentation::from)
            .collect();
        list.sort_by_key(|c| c.endpoint_type);
        list
    }

    pub fn get(&self, id: &str) -> Option<ServerConfig> {
        // Java parity: lock poisoning recovers, never aborts (ConcurrentHashMap has no poison).
        self.configs
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(id)
            .cloned()
    }

    pub fn get_by_endpoint(&self, endpoint: EndpointType) -> Result<ServerConfig, String> {
        let guard = self.configs.read().unwrap_or_else(|e| e.into_inner());
        let matches: Vec<_> = guard
            .values()
            .filter(|c| c.endpoint_type == endpoint.code())
            .cloned()
            .collect();
        match matches.len() {
            0 => Err("No server config found".into()),
            // len==1 guarantees Some; ok_or avoids panic, maps to 404 parity.
            1 => matches
                .into_iter()
                .next()
                .ok_or("No server config found".into()),
            _ => Err("Only one server config per endpoint type allowed".into()),
        }
    }

    pub fn decrypt_password(&self, config: &ServerConfig) -> Result<String, String> {
        self.cipher.decrypt(&config.password)
    }

    pub fn update(&self, server_id: &str, rep: ServerConfigRepresentation) -> Result<(), String> {
        {
            let mut guard = self.configs.write().unwrap_or_else(|e| e.into_inner());
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
        }
        self.persist()
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
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(config.id.clone(), config);
        self.persist()
    }

    pub fn default_representation(endpoint: EndpointType) -> ServerConfigRepresentation {
        let port = std::env::var("FLOWABLE_UI_ENGINE_PORT")
            .ok()
            .and_then(|p| p.parse().ok())
            .unwrap_or(8080);
        let address =
            std::env::var("FLOWABLE_UI_ENGINE_HOST").unwrap_or_else(|_| "http://127.0.0.1".into());
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

#[cfg(test)]
mod tests {
    use super::*;

    fn store_at(path: PathBuf) -> ServerConfigStore {
        ServerConfigStore {
            configs: RwLock::new(HashMap::new()),
            cipher: PasswordCipher::default(),
            path,
        }
    }

    /// P3: when the first-run seed cannot be persisted, startup must not panic
    /// and the seeded configs must remain usable in memory for this process.
    #[test]
    fn seed_persist_failure_keeps_in_memory_configs() {
        let dir = std::env::temp_dir().join(format!("flowable-ui-sc-seed-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let blocker = dir.join("not-a-directory");
        std::fs::write(&blocker, b"x").expect("blocker file");
        // The store path's parent is a regular file, so `create_dir_all` inside
        // `persist` fails deterministically on both Windows and Unix.
        let store = store_at(blocker.join("server-configs.json"));

        store.load_or_seed();

        assert_eq!(
            store.list_representations().len(),
            EndpointType::all().len(),
            "seeded configs must be available in memory despite persist failure"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The happy counterpart: a writable fresh path is seeded, persisted and
    /// reloaded with the same config set.
    #[test]
    fn seed_persist_success_round_trips() {
        let path =
            std::env::temp_dir().join(format!("flowable-ui-sc-seed-ok-{}.json", Uuid::new_v4()));
        let store = store_at(path.clone());

        store.load_or_seed();

        assert!(path.is_file(), "seed configs were persisted");
        let reloaded = store_at(path.clone());
        assert!(reloaded.load_from_disk(), "persisted seed loads back");
        assert_eq!(
            reloaded.list_representations().len(),
            EndpointType::all().len()
        );
        let _ = std::fs::remove_file(&path);
    }
}
