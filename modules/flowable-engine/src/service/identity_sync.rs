use crate::persistence::runtime_store::RuntimeStore;
use crate::service::jwks::JwksCache;
use crate::service::revocation::TokenRevocationRegistry;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// Background poller that synchronizes identity events across the cluster.
///
/// It watches the timer_admin_audit_logs table to detect when other nodes
/// have modified issuer profiles or revoked tokens, ensuring local caches
/// are invalidated promptly.
pub struct IdentitySyncPoller {
    runtime_store: RuntimeStore,
    jwks_cache: Arc<JwksCache>,
    revocation_registry: Arc<TokenRevocationRegistry>,
    poll_interval: Duration,
}

impl IdentitySyncPoller {
    pub fn new(
        runtime_store: RuntimeStore,
        jwks_cache: Arc<JwksCache>,
        revocation_registry: Arc<TokenRevocationRegistry>,
    ) -> Self {
        Self {
            runtime_store,
            jwks_cache,
            revocation_registry,
            poll_interval: Duration::from_secs(1),
        }
    }

    pub fn start(&self, stop_signal: Arc<AtomicBool>) -> std::thread::JoinHandle<()> {
        let rs = self.runtime_store.clone();
        let jc = self.jwks_cache.clone();
        let _rr = self.revocation_registry.clone();
        let interval = self.poll_interval;

        std::thread::spawn(move || {
            // Start polling from the current time to avoid processing historical logs
            // unless we want full replay on startup. For cache invalidation, starting
            // from "now" is usually safer and more efficient.
            let mut last_processed_ts = rs.time_source().now().timestamp_millis();

            while !stop_signal.load(Ordering::SeqCst) {
                let start_poll = Instant::now();

                let mut session = rs.create_session().unwrap();
                let records =
                    rs.find_timer_admin_audit_records_since(last_processed_ts, &mut session);

                for record in records {
                    // Only process successful administrative actions that affect identity state.
                    if record.outcome != "success" {
                        continue;
                    }

                    match record.action.as_str() {
                        "issuer-profile-create"
                        | "issuer-profile-update"
                        | "issuer-profile-delete" => {
                            // Target is usually "profile:ID" or "issuer:URL".
                            // For profiles, we need to extract the issuer URL if possible,
                            // or invalidate all if we can't be sure.
                            // In timer_coordination_service, we audit profile mutations with target "profile:id".
                            // We might need to lookup the issuer by profile_id if it's not in the record.
                            if let Some(ref profile_id) = record.profile_id {
                                if let Some(profile) =
                                    rs.find_issuer_profile(profile_id, &mut session)
                                {
                                    jc.invalidate_issuer(&profile.issuer);
                                }
                            } else if record.target.starts_with("issuer:") {
                                let issuer = &record.target[7..];
                                jc.invalidate_issuer(issuer);
                            }
                        }
                        "revoke" | "unrevoke" => {
                            // For revocations, the registry handles local consistency within the node,
                            // but other nodes need to know their local JWKS/Policy view might be stale
                            // if revocation affects rollout or validation logic.
                            // However, the RevocationRegistry is DB-backed and doesn't usually
                            // need explicit invalidation signal for individual JTIs (they are checked per auth).
                            // But if we ever add an in-memory "negative cache" for revocations,
                            // this is where we'd invalidate it.
                        }
                        _ => {}
                    }

                    if record.timestamp > last_processed_ts {
                        last_processed_ts = record.timestamp;
                    }
                }
                session.rollback().unwrap();

                // Sleep until next interval, accounting for time spent polling
                let elapsed = start_poll.elapsed();
                if elapsed < interval {
                    std::thread::sleep(interval - elapsed);
                }
            }
        })
    }
}
