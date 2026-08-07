use crate::engine::runtime_service::RuntimeService;
use crate::persistence::runtime_store::{CoordinatorLeadershipStatus, NodeStatus};
use crate::service::config::{IdentityRuntimeComponents, ServicePolicyConfig};
use crate::service::issuer_health::IssuerHealthCollector;
use crate::service::issuer_profile::IssuerProfile;
use crate::service::jwks::JwksCache;
use crate::service::policy::{
    AuthorizationRequest, PolicyEngine, ResourceAction, ResourceType, TenantAwarePolicyEngine,
};
use crate::service::principal::{AuthProvider, Principal};
use crate::service::revocation::TokenRevocationRegistry;
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Serialize, Deserialize, Debug)]
pub struct TimerCoordinatorStatusDto {
    pub leader_node_id: String,
    pub fencing_token: i64,
    pub lease_expiry_time: i64,
    pub status: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct TimerNodeStatusDto {
    pub node_id: String,
    pub last_heartbeat: i64,
    pub worker_type: String,
    pub status: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ReleaseRequest {
    pub fencing_token: i64,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct DeregisterRequest {
    pub node_id: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct RevokeRequest {
    pub jti: String,
    pub issuer: String,
    pub reason: String,
    #[serde(default = "default_revoke_ttl_seconds")]
    pub ttl_seconds: u64,
}

fn default_revoke_ttl_seconds() -> u64 {
    3600
}

fn build_audit_input(
    request_id: String,
    principal: &Principal,
    action: &str,
    target: String,
    outcome: String,
) -> crate::service::audit::TimerAdminAuditInput {
    crate::service::audit::TimerAdminAuditInput {
        request_id,
        tenant_id: principal.tenant_id.clone(),
        issuer: principal.issuer.clone(),
        subject: principal.subject.clone(),
        actor: principal.actor_id.clone(),
        action: action.to_string(),
        target,
        outcome,
        profile_id: principal.profile_id.clone(),
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub struct UnrevokeRequest {
    pub jti: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct RevocationStatusRequest {
    pub jti: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Envelope<T> {
    pub data: Option<T>,
    pub error: Option<ErrorDto>,
}

impl<T: Serialize> Envelope<T> {
    pub fn ok(data: T) -> Self {
        Self {
            data: Some(data),
            error: None,
        }
    }

    pub fn err(code: &str, message: &str) -> Self {
        Self {
            data: None,
            error: Some(ErrorDto {
                code: code.to_string(),
                message: message.to_string(),
            }),
        }
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ErrorDto {
    pub code: String,
    pub message: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct SuccessResponse {
    pub success: bool,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct StepDownResponse {
    pub success: bool,
    pub new_fencing_token: i64,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct CleanupResponse {
    pub cleaned_count: usize,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct UnrevokeResponse {
    pub success: bool,
}

pub struct TimerCoordinationService {
    runtime_service: Arc<RuntimeService>,
    config: ServicePolicyConfig,
    auth_provider: Arc<dyn AuthProvider>,
    health_collector: Arc<IssuerHealthCollector>,
    revocation_registry: Arc<TokenRevocationRegistry>,
    jwks_cache: Arc<JwksCache>,
    identity_sync_poller: Arc<crate::service::identity_sync::IdentitySyncPoller>,
}

impl TimerCoordinationService {
    pub fn new(runtime_service: Arc<RuntimeService>, config: ServicePolicyConfig) -> Self {
        let identity = runtime_service.build_identity_runtime(&config);
        Self::from_identity_runtime(runtime_service, config, identity)
    }

    pub fn with_identity_components(
        runtime_service: Arc<RuntimeService>,
        config: ServicePolicyConfig,
        profiles: Vec<IssuerProfile>,
        jwks_cache: Arc<JwksCache>,
        revocation_registry: Arc<TokenRevocationRegistry>,
    ) -> Self {
        let identity = runtime_service.build_identity_runtime_with_components(
            &config,
            profiles,
            jwks_cache,
            revocation_registry,
        );
        Self::from_identity_runtime(runtime_service, config, identity)
    }

    fn from_identity_runtime(
        runtime_service: Arc<RuntimeService>,
        config: ServicePolicyConfig,
        identity: IdentityRuntimeComponents,
    ) -> Self {
        let health_collector = Arc::new(IssuerHealthCollector::new(
            identity.runtime_store.clone(),
            identity.jwks_cache.clone(),
            Arc::clone(&identity.revocation_registry),
        ));

        let identity_sync_poller =
            Arc::new(crate::service::identity_sync::IdentitySyncPoller::new(
                identity.runtime_store.clone(),
                identity.jwks_cache.clone(),
                identity.revocation_registry.clone(),
            ));

        Self {
            runtime_service,
            config,
            auth_provider: identity.auth_provider,
            health_collector,
            revocation_registry: identity.revocation_registry,
            jwks_cache: identity.jwks_cache,
            identity_sync_poller,
        }
    }

    pub fn start(&self, stop_signal: Arc<AtomicBool>) -> std::thread::JoinHandle<()> {
        // Start the identity sync poller background task
        let _poller_handle = self.identity_sync_poller.start(Arc::clone(&stop_signal));

        let listener = match TcpListener::bind(&self.config.bind_addr) {
            Ok(listener) => listener,
            Err(error) => {
                tracing::error!(
                    error = %error,
                    addr = %self.config.bind_addr,
                    "failed to bind timer coordination listener"
                );
                return std::thread::spawn(|| {});
            }
        };
        if let Err(error) = listener.set_nonblocking(true) {
            tracing::error!(error = %error, "failed to set nonblocking on timer coordination listener");
            return std::thread::spawn(|| {});
        }

        let runtime_service = Arc::clone(&self.runtime_service);
        let auth_provider = Arc::clone(&self.auth_provider);
        let max_request_size = self.config.max_request_size;
        let health_collector = Arc::clone(&self.health_collector);
        let revocation_registry = Arc::clone(&self.revocation_registry);
        let jwks_cache = Arc::clone(&self.jwks_cache);

        std::thread::spawn(move || {
            while !stop_signal.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((stream, _addr)) => {
                        let rs = Arc::clone(&runtime_service);
                        let ap = Arc::clone(&auth_provider);
                        let hc = Arc::clone(&health_collector);
                        let rr = Arc::clone(&revocation_registry);
                        let jc = Arc::clone(&jwks_cache);
                        std::thread::spawn(move || {
                            handle_client(stream, rs, ap, max_request_size, hc, rr, jc);
                        });
                    }
                    Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(std::time::Duration::from_millis(50));
                    }
                    Err(e) => {
                        tracing::warn!("Accept error: {:?}", e);
                    }
                }
            }
        })
    }
}

fn classify_path(path: &str, method: &str) -> (ResourceType, ResourceAction) {
    if path == "/issuer-health" || path.starts_with("/issuer-health/") {
        (ResourceType::IssuerHealth, ResourceAction::Read)
    } else if path == "/issuer-profiles" || path.starts_with("/issuer-profiles/") {
        if method == "GET" {
            (ResourceType::IssuerAdmin, ResourceAction::Read)
        } else {
            (ResourceType::IssuerAdmin, ResourceAction::IdentityAdmin)
        }
    } else if path == "/revocation/stats" {
        (ResourceType::RevocationAdmin, ResourceAction::Read)
    } else if path.starts_with("/revocation/") {
        (ResourceType::RevocationAdmin, ResourceAction::IdentityAdmin)
    } else if path == "/status" || path == "/nodes" || path == "/health" {
        match path {
            "/status" | "/nodes" => (ResourceType::TimerCoordinator, ResourceAction::Read),
            "/health" => (ResourceType::TimerCoordinator, ResourceAction::Read),
            _ => (ResourceType::TimerCoordinator, ResourceAction::Read),
        }
    } else if path == "/release" || path == "/step-down" {
        (
            ResourceType::TimerCoordinator,
            ResourceAction::AdminDestructive,
        )
    } else if path == "/deregister" {
        (ResourceType::TimerNode, ResourceAction::AdminDestructive)
    } else if path == "/cleanup" {
        (ResourceType::ClusterNodes, ResourceAction::AdminDestructive)
    } else {
        (
            ResourceType::TimerCoordinator,
            ResourceAction::AdminDestructive,
        )
    }
}

fn handle_client(
    mut stream: TcpStream,
    runtime_service: Arc<RuntimeService>,
    auth_provider: Arc<dyn AuthProvider>,
    max_request_size: usize,
    health_collector: Arc<IssuerHealthCollector>,
    revocation_registry: Arc<TokenRevocationRegistry>,
    jwks_cache: Arc<JwksCache>,
) {
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut request_line = String::new();
    if reader.read_line(&mut request_line).is_err() || request_line.is_empty() {
        return;
    }

    let parts: Vec<&str> = request_line.split_whitespace().collect();
    if parts.len() < 2 {
        let res = Envelope::<()>::err("BAD_REQUEST", "Malformed request line");
        send_response(&mut stream, "400 Bad Request", &res);
        return;
    }

    let method = parts[0];
    let path = parts[1].to_string();

    let mut content_length = 0;
    let mut auth_token = None;
    let mut request_id = uuid::Uuid::new_v4().to_string();
    let mut tenant_id = None;

    loop {
        let mut header_line = String::new();
        if reader.read_line(&mut header_line).is_err() {
            return;
        }
        let header_line = header_line.trim();
        if header_line.is_empty() {
            break;
        }
        let lower_header = header_line.to_lowercase();
        if lower_header.starts_with("content-length:") {
            let val = header_line[15..].trim();
            if let Ok(len) = val.parse::<usize>() {
                content_length = len;
            }
        } else if lower_header.starts_with("authorization: bearer ") {
            auth_token = Some(header_line[22..].trim().to_string());
        } else if lower_header.starts_with("x-request-id:") {
            request_id = header_line[13..].trim().to_string();
        } else if lower_header.starts_with("x-tenant-id:") {
            tenant_id = Some(header_line[12..].trim().to_string());
        }
    }

    if content_length > max_request_size {
        let res = Envelope::<()>::err("PAYLOAD_TOO_LARGE", "Request body too large");
        send_response(&mut stream, "413 Payload Too Large", &res);
        return;
    }

    let mut body = vec![0; content_length];
    if content_length > 0 && reader.read_exact(&mut body).is_err() {
        return;
    }

    let principal_opt = auth_provider.authenticate(auth_token.as_deref());
    let policy_engine = TenantAwarePolicyEngine::new();

    let (resource_type, required_action) = classify_path(&path, method);

    let is_health = path == "/health";

    if !is_health {
        let authorized = if let Some(ref principal) = principal_opt {
            policy_engine.authorize(AuthorizationRequest {
                principal,
                action: required_action,
                resource: resource_type,
                tenant_id: tenant_id.as_deref(),
            })
        } else {
            false
        };

        if !authorized {
            let res = Envelope::<()>::err(
                "UNAUTHORIZED",
                "Invalid or missing credentials, or insufficient permissions",
            );
            send_response(&mut stream, "401 Unauthorized", &res);
            return;
        }
    }

    let principal = principal_opt.unwrap_or_else(|| {
        crate::service::principal::Principal::new("anonymous", "anonymous", "none", None)
    });

    match (method, path.as_str()) {
        ("GET", "/health") => {
            let res = Envelope::ok(SuccessResponse { success: true });
            send_response(&mut stream, "200 OK", &res);
        }
        ("GET", "/status") => {
            let internal_status = runtime_service.get_timer_coordinator_status();
            let dto = TimerCoordinatorStatusDto {
                leader_node_id: internal_status.leader_node_id,
                fencing_token: internal_status.fencing_token,
                lease_expiry_time: internal_status.lease_expiry_time,
                status: match internal_status.status {
                    CoordinatorLeadershipStatus::NoLeader => "NoLeader".to_string(),
                    CoordinatorLeadershipStatus::Active => "Active".to_string(),
                    CoordinatorLeadershipStatus::Expired => "Expired".to_string(),
                },
            };
            send_response(&mut stream, "200 OK", &Envelope::ok(dto));
        }
        ("GET", "/nodes") => {
            let internal_nodes = runtime_service.list_timer_nodes().unwrap();
            let nodes: Vec<TimerNodeStatusDto> = internal_nodes
                .into_iter()
                .map(|n| TimerNodeStatusDto {
                    node_id: n.node_id,
                    last_heartbeat: n.last_heartbeat,
                    worker_type: n.worker_type,
                    status: match n.status {
                        NodeStatus::Active => "Active".to_string(),
                        NodeStatus::Expired => "Expired".to_string(),
                    },
                })
                .collect();
            send_response(&mut stream, "200 OK", &Envelope::ok(nodes));
        }
        ("POST", "/release") => {
            if let Ok(req) = serde_json::from_slice::<ReleaseRequest>(&body) {
                let success = runtime_service
                    .release_leadership(req.fencing_token)
                    .unwrap();
                let outcome = if success { "success" } else { "failure" };
                runtime_service
                    .audit_admin_action(build_audit_input(
                        request_id,
                        &principal,
                        "release",
                        "timer-coordinator".to_string(),
                        outcome.to_string(),
                    ))
                    .unwrap();
                let res = SuccessResponse { success };
                send_response(&mut stream, "200 OK", &Envelope::ok(res));
            } else {
                let res = Envelope::<()>::err("BAD_REQUEST", "Invalid body");
                send_response(&mut stream, "400 Bad Request", &res);
            }
        }
        ("POST", "/step-down") => {
            let (success, new_fencing_token) = runtime_service.admin_step_down().unwrap();
            let outcome = if success { "success" } else { "failure" };
            runtime_service
                .audit_admin_action(build_audit_input(
                    request_id,
                    &principal,
                    "step-down",
                    "timer-coordinator".to_string(),
                    outcome.to_string(),
                ))
                .unwrap();
            let res = StepDownResponse {
                success,
                new_fencing_token,
            };
            send_response(&mut stream, "200 OK", &Envelope::ok(res));
        }
        ("POST", "/deregister") => {
            if let Ok(req) = serde_json::from_slice::<DeregisterRequest>(&body) {
                let success = runtime_service.deregister_timer_node(&req.node_id).unwrap();
                let outcome = if success { "success" } else { "failure" };
                runtime_service
                    .audit_admin_action(build_audit_input(
                        request_id,
                        &principal,
                        "deregister",
                        req.node_id,
                        outcome.to_string(),
                    ))
                    .unwrap();
                let res = SuccessResponse { success };
                send_response(&mut stream, "200 OK", &Envelope::ok(res));
            } else {
                let res = Envelope::<()>::err("BAD_REQUEST", "Invalid body");
                send_response(&mut stream, "400 Bad Request", &res);
            }
        }
        ("POST", "/cleanup") => {
            let cleaned_count = runtime_service.cleanup_expired_timer_nodes().unwrap();
            runtime_service
                .audit_admin_action(build_audit_input(
                    request_id,
                    &principal,
                    "cleanup",
                    "cluster-nodes".to_string(),
                    format!("success: {}", cleaned_count),
                ))
                .unwrap();
            let res = CleanupResponse { cleaned_count };
            send_response(&mut stream, "200 OK", &Envelope::ok(res));
        }

        // ── Issuer Health Control Plane ──
        ("GET", "/issuer-health") => {
            let snapshots = health_collector.collect_all();
            let _ = runtime_service.audit_admin_action(build_audit_input(
                request_id,
                &principal,
                "issuer-health-read",
                "all-issuers".to_string(),
                "success".to_string(),
            ));
            send_response(&mut stream, "200 OK", &Envelope::ok(snapshots));
        }
        (method, path_str) if method == "GET" && path_str.starts_with("/issuer-health/") => {
            let issuer = &path_str["/issuer-health/".len()..];
            let issuer_decoded = url_decode(issuer);
            match health_collector.collect_for_issuer(&issuer_decoded) {
                Some(snapshot) => {
                    let _ = runtime_service.audit_admin_action(build_audit_input(
                        request_id,
                        &principal,
                        "issuer-health-read",
                        format!("issuer:{}", issuer_decoded),
                        "success".to_string(),
                    ));
                    send_response(&mut stream, "200 OK", &Envelope::ok(snapshot));
                }
                None => {
                    let res = Envelope::<()>::err("NOT_FOUND", "Issuer not found or not active");
                    send_response(&mut stream, "404 Not Found", &res);
                }
            }
        }

        // ── Issuer Profile Admin Control Plane ──
        ("GET", "/issuer-profiles") => {
            let profiles = runtime_service.list_issuer_profiles();
            let _ = runtime_service.audit_admin_action(build_audit_input(
                request_id,
                &principal,
                "issuer-profile-list",
                "all-profiles".to_string(),
                "success".to_string(),
            ));
            send_response(&mut stream, "200 OK", &Envelope::ok(profiles));
        }
        (method, path_str) if method == "GET" && path_str.starts_with("/issuer-profiles/") => {
            let profile_id = &path_str["/issuer-profiles/".len()..];
            let profile_id_decoded = url_decode(profile_id);
            let found = runtime_service.find_issuer_profile(&profile_id_decoded);
            match found {
                Some(profile) => {
                    let _ = runtime_service.audit_admin_action(build_audit_input(
                        request_id,
                        &principal,
                        "issuer-profile-read",
                        format!("profile:{}", profile_id_decoded),
                        "success".to_string(),
                    ));
                    send_response(&mut stream, "200 OK", &Envelope::ok(profile));
                }
                None => {
                    let res = Envelope::<()>::err("NOT_FOUND", "Issuer profile not found");
                    send_response(&mut stream, "404 Not Found", &res);
                }
            }
        }
        ("POST", "/issuer-profiles") => {
            if let Ok(profile) =
                serde_json::from_slice::<crate::service::issuer_profile::IssuerProfile>(&body)
            {
                let profile = runtime_service.insert_issuer_profile(profile);
                jwks_cache.invalidate_issuer(&profile.issuer);
                let _ = runtime_service.audit_admin_action(build_audit_input(
                    request_id,
                    &principal,
                    "issuer-profile-create",
                    format!("profile:{}", profile.id),
                    "success".to_string(),
                ));
                send_response(&mut stream, "200 OK", &Envelope::ok(profile));
            } else {
                let res = Envelope::<()>::err("BAD_REQUEST", "Invalid profile body");
                send_response(&mut stream, "400 Bad Request", &res);
            }
        }
        (method, path_str) if method == "PUT" && path_str.starts_with("/issuer-profiles/") => {
            let profile_id = &path_str["/issuer-profiles/".len()..];
            let profile_id_decoded = url_decode(profile_id);
            if let Ok(profile) =
                serde_json::from_slice::<crate::service::issuer_profile::IssuerProfile>(&body)
            {
                if profile.id != profile_id_decoded {
                    let res = Envelope::<()>::err("BAD_REQUEST", "Profile ID mismatch");
                    send_response(&mut stream, "400 Bad Request", &res);
                } else {
                    let expected_version = profile.version;
                    match runtime_service.update_issuer_profile(profile, expected_version) {
                        Ok(result) => {
                            jwks_cache.invalidate_issuer(&result.new_profile.issuer);
                            if result.old_profile.issuer != result.new_profile.issuer {
                                jwks_cache.invalidate_issuer(&result.old_profile.issuer);
                            }
                            let _ = runtime_service.audit_admin_action(build_audit_input(
                                request_id,
                                &principal,
                                "issuer-profile-update",
                                format!("profile:{}", profile_id_decoded),
                                "success".to_string(),
                            ));
                            send_response(&mut stream, "200 OK", &Envelope::ok(result.new_profile));
                        }
                        Err(crate::persistence::StorageError::OptimisticLockConflict) => {
                            let res = Envelope::<()>::err(
                                "CONFLICT",
                                "Optimistic lock conflict: profile version mismatch",
                            );
                            send_response(&mut stream, "409 Conflict", &res);
                        }
                        Err(crate::persistence::StorageError::Sql(msg))
                            if msg == "Issuer profile not found" =>
                        {
                            let res = Envelope::<()>::err("NOT_FOUND", "Issuer profile not found");
                            send_response(&mut stream, "404 Not Found", &res);
                        }
                        Err(e) => {
                            let res = Envelope::<()>::err("INTERNAL_ERROR", e.to_string().as_str());
                            send_response(&mut stream, "500 Internal Server Error", &res);
                        }
                    }
                }
            } else {
                let res = Envelope::<()>::err("BAD_REQUEST", "Invalid profile body");
                send_response(&mut stream, "400 Bad Request", &res);
            }
        }
        (method, path_str) if method == "DELETE" && path_str.starts_with("/issuer-profiles/") => {
            let profile_id = &path_str["/issuer-profiles/".len()..];
            let profile_id_decoded = url_decode(profile_id);
            let found_profile = runtime_service.delete_issuer_profile(&profile_id_decoded);
            if let Some(ref profile) = found_profile {
                jwks_cache.invalidate_issuer(&profile.issuer);
            }
            let outcome = if found_profile.is_some() {
                "success"
            } else {
                "not-found"
            };
            let _ = runtime_service.audit_admin_action(build_audit_input(
                request_id,
                &principal,
                "issuer-profile-delete",
                format!("profile:{}", profile_id_decoded),
                outcome.to_string(),
            ));
            send_response(
                &mut stream,
                "200 OK",
                &Envelope::ok(SuccessResponse {
                    success: found_profile.is_some(),
                }),
            );
        }

        // ── Revocation Admin Control Plane ──
        ("POST", "/revocation/revoke") => {
            if let Ok(req) = serde_json::from_slice::<RevokeRequest>(&body) {
                let ttl = std::time::Duration::from_secs(req.ttl_seconds);
                revocation_registry.admin_revoke_with_ttl(&req.jti, &req.issuer, &req.reason, ttl);
                let _ = runtime_service.audit_admin_action(build_audit_input(
                    request_id,
                    &principal,
                    "revoke",
                    format!("jti:{}", req.jti),
                    "success".to_string(),
                ));
                let res = SuccessResponse { success: true };
                send_response(&mut stream, "200 OK", &Envelope::ok(res));
            } else {
                let res = Envelope::<()>::err("BAD_REQUEST", "Invalid revoke request body");
                send_response(&mut stream, "400 Bad Request", &res);
            }
        }
        ("POST", "/revocation/unrevoke") => {
            if let Ok(req) = serde_json::from_slice::<UnrevokeRequest>(&body) {
                let success = revocation_registry.admin_unrevoke(&req.jti);
                let outcome = if success { "success" } else { "not-found" };
                let _ = runtime_service.audit_admin_action(build_audit_input(
                    request_id,
                    &principal,
                    "unrevoke",
                    format!("jti:{}", req.jti),
                    outcome.to_string(),
                ));
                let res = UnrevokeResponse { success };
                send_response(&mut stream, "200 OK", &Envelope::ok(res));
            } else {
                let res = Envelope::<()>::err("BAD_REQUEST", "Invalid unrevoke request body");
                send_response(&mut stream, "400 Bad Request", &res);
            }
        }
        (method, path_str) if method == "GET" && path_str.starts_with("/revocation/status/") => {
            let jti = &path_str["/revocation/status/".len()..];
            let jti_decoded = url_decode(jti);
            let status = health_collector.revocation_status(&jti_decoded);
            let _ = runtime_service.audit_admin_action(build_audit_input(
                request_id,
                &principal,
                "revocation-status-read",
                format!("jti:{}", jti_decoded),
                "success".to_string(),
            ));
            send_response(&mut stream, "200 OK", &Envelope::ok(status));
        }
        ("GET", "/revocation/stats") => {
            let snapshot = health_collector.revocation_snapshot();
            let _ = runtime_service.audit_admin_action(build_audit_input(
                request_id,
                &principal,
                "revocation-stats-read",
                "revocation-registry".to_string(),
                "success".to_string(),
            ));
            send_response(&mut stream, "200 OK", &Envelope::ok(snapshot));
        }

        _ => {
            let res = Envelope::<()>::err("NOT_FOUND", "Not Found");
            send_response(&mut stream, "404 Not Found", &res);
        }
    }
}

fn url_decode(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let mut chars = input.bytes();
    while let Some(b) = chars.next() {
        if b == b'%' {
            let hi = chars.next();
            let lo = chars.next();
            if let (Some(h), Some(l)) = (hi, lo)
                && let (Some(hv), Some(lv)) = (hex_val(h), hex_val(l))
            {
                result.push(char::from(hv << 4 | lv));
                continue;
            }
            result.push('%');
            result.push(char::from(hi.unwrap_or(b'?')));
            result.push(char::from(lo.unwrap_or(b'?')));
        } else if b == b'+' {
            result.push(' ');
        } else {
            result.push(char::from(b));
        }
    }
    result
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

fn send_response<T: Serialize>(stream: &mut TcpStream, status_line: &str, envelope: &Envelope<T>) {
    let response_body = serde_json::to_string(envelope).unwrap();
    let response = format!(
        "HTTP/1.1 {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
        status_line,
        response_body.len(),
        response_body
    );
    let _ = stream.write_all(response.as_bytes());
}
