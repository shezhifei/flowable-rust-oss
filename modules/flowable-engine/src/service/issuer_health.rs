use crate::persistence::runtime_store::RuntimeStore;
use crate::service::issuer_profile::{IssuerProfile, JwksRefreshPolicy, RolloutState};
use crate::service::jwks::JwksCache;
use crate::service::revocation::TokenRevocationRegistry;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IssuerHealthSnapshot {
    pub profile_id: String,
    pub issuer: String,
    pub rollout_state: String,
    pub allowed_algorithms: Vec<String>,
    pub jwks_uri_present: bool,
    pub jwks_cache_ttl_seconds: u64,
    pub refresh_policy: JwksRefreshPolicySummary,
    pub known_key_count: usize,
    pub negative_cache_count: usize,
    pub refresh_health: RefreshHealthSummary,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JwksRefreshPolicySummary {
    pub min_refresh_interval_seconds: u64,
    pub backoff_multiplier: f64,
    pub max_retry_delay_seconds: u64,
    pub allow_stale_on_failure: bool,
    pub stale_tolerance_seconds: u64,
    pub negative_cache_seconds: u64,
}

impl From<&JwksRefreshPolicy> for JwksRefreshPolicySummary {
    fn from(p: &JwksRefreshPolicy) -> Self {
        Self {
            min_refresh_interval_seconds: p.min_refresh_interval_seconds,
            backoff_multiplier: p.backoff_multiplier,
            max_retry_delay_seconds: p.max_retry_delay_seconds,
            allow_stale_on_failure: p.allow_stale_on_failure,
            stale_tolerance_seconds: p.stale_tolerance_seconds,
            negative_cache_seconds: p.negative_cache_seconds,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RefreshHealthSummary {
    pub consecutive_failures: u32,
    pub last_refresh_ago_seconds: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RevocationRegistrySnapshot {
    pub active_count: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RevocationStatusDto {
    pub jti: String,
    pub is_revoked: bool,
    pub issuer: Option<String>,
    pub reason: Option<String>,
}

pub struct IssuerHealthCollector {
    runtime_store: RuntimeStore,
    jwks_cache: Arc<JwksCache>,
    revocation_registry: Arc<TokenRevocationRegistry>,
}

impl IssuerHealthCollector {
    pub fn new(
        runtime_store: RuntimeStore,
        jwks_cache: Arc<JwksCache>,
        revocation_registry: Arc<TokenRevocationRegistry>,
    ) -> Self {
        Self {
            runtime_store,
            jwks_cache,
            revocation_registry,
        }
    }

    pub fn collect_all(&self) -> Vec<IssuerHealthSnapshot> {
        let mut session = self.runtime_store.create_session().unwrap();
        let profiles = self.runtime_store.list_issuer_profiles(&mut session);
        let snapshots = profiles
            .iter()
            .map(|p| self.collect_for_profile(p))
            .collect();
        session.rollback().unwrap();
        snapshots
    }

    pub fn collect_for_issuer(&self, issuer: &str) -> Option<IssuerHealthSnapshot> {
        let mut session = self.runtime_store.create_session().unwrap();
        let profile = self
            .runtime_store
            .list_issuer_profiles(&mut session)
            .into_iter()
            .find(|p| p.issuer == issuer && p.is_active());
        session.rollback().unwrap();
        profile.map(|p| self.collect_for_profile(&p))
    }

    fn collect_for_profile(&self, profile: &IssuerProfile) -> IssuerHealthSnapshot {
        let known_key_count = self.jwks_cache.count_keys_for_issuer(&profile.issuer);
        let negative_cache_count = self
            .jwks_cache
            .count_negative_cache_for_issuer(&profile.issuer);
        let (consecutive_failures, last_refresh_ago) =
            self.jwks_cache.refresh_state_for_issuer(&profile.issuer);

        IssuerHealthSnapshot {
            profile_id: profile.id.clone(),
            issuer: profile.issuer.clone(),
            rollout_state: match profile.rollout_state {
                RolloutState::Active => "Active".to_string(),
                RolloutState::Deprecated => "Deprecated".to_string(),
            },
            allowed_algorithms: profile.allowed_algorithms.clone(),
            jwks_uri_present: profile.jwks_uri.is_some(),
            jwks_cache_ttl_seconds: profile.jwks_cache_ttl_seconds,
            refresh_policy: JwksRefreshPolicySummary::from(&profile.jwks_refresh_policy),
            known_key_count,
            negative_cache_count,
            refresh_health: RefreshHealthSummary {
                consecutive_failures,
                last_refresh_ago_seconds: last_refresh_ago,
            },
        }
    }

    pub fn revocation_snapshot(&self) -> RevocationRegistrySnapshot {
        RevocationRegistrySnapshot {
            active_count: self.revocation_registry.active_count(),
        }
    }

    pub fn revocation_status(&self, jti: &str) -> RevocationStatusDto {
        match self.revocation_registry.admin_check(jti) {
            crate::service::revocation::RevocationStatus::Revoked {
                jti,
                issuer,
                reason,
            } => RevocationStatusDto {
                jti,
                is_revoked: true,
                issuer: Some(issuer),
                reason: Some(reason),
            },
            crate::service::revocation::RevocationStatus::NotRevoked => RevocationStatusDto {
                jti: jti.to_string(),
                is_revoked: false,
                issuer: None,
                reason: None,
            },
        }
    }
}
