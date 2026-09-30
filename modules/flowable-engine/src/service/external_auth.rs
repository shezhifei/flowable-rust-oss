use crate::persistence::runtime_store::RuntimeStore;
use crate::service::jwks::JwksCache;
use crate::service::principal::{AuthProvider, Principal};
use crate::service::revocation::{RevocationStatus, TokenRevocationRegistry};
use base64::Engine;
use std::sync::Arc;

#[derive(Debug)]
pub enum AuthFailure {
    InvalidToken,
    Revoked,
}

pub struct ExternalAuthProvider {
    runtime_store: RuntimeStore,
    jwks_cache: Arc<JwksCache>,
    revocation_registry: Arc<TokenRevocationRegistry>,
    rate_limiter: Option<Arc<crate::service::rate_limit::RateLimiter>>,
}

impl ExternalAuthProvider {
    /// The revocation registry is a required dependency: defaulting it to a
    /// private in-memory store would open a throwaway SQLite pool on the
    /// production startup path (and panic if that failed) only for the caller
    /// to replace it, and a forgotten override would silently ignore every
    /// persisted revocation.
    pub fn new(
        runtime_store: RuntimeStore,
        revocation_registry: Arc<TokenRevocationRegistry>,
    ) -> Self {
        Self {
            runtime_store,
            jwks_cache: Arc::new(JwksCache::new()),
            revocation_registry,
            rate_limiter: None,
        }
    }

    pub fn with_jwks_cache(mut self, cache: Arc<JwksCache>) -> Self {
        self.jwks_cache = cache;
        self
    }

    pub fn with_rate_limiter(
        mut self,
        limiter: Arc<crate::service::rate_limit::RateLimiter>,
    ) -> Self {
        self.rate_limiter = Some(limiter);
        self
    }

    pub fn revocation_registry(&self) -> &Arc<TokenRevocationRegistry> {
        &self.revocation_registry
    }
}

impl AuthProvider for ExternalAuthProvider {
    fn authenticate(&self, token: Option<&str>) -> Option<Principal> {
        let token_str = token?;

        let mut limiter_key = "global".to_string();
        if let Some(ref limiter) = self.rate_limiter
            && let Err(backoff) = limiter.check(&limiter_key)
        {
            tracing::warn!("Rate limited: backoff {:?}", backoff);
            return None;
        }

        let header = match jsonwebtoken::decode_header(token_str) {
            Ok(h) => h,
            Err(e) => {
                tracing::warn!("Decode header failed: {:?}", e);
                if let Some(ref limiter) = self.rate_limiter {
                    limiter.record_failure(&limiter_key);
                }
                return None;
            }
        };
        let kid = header.kid.as_deref().or_else(|| {
            if let Some(ref limiter) = self.rate_limiter {
                limiter.record_failure(&limiter_key);
            }
            None
        })?;

        // Extract unverified claims to route to the correct profile
        let unverified_claims: std::collections::HashMap<String, serde_json::Value> = {
            let parts: Vec<&str> = token_str.split('.').collect();
            if parts.len() != 3 {
                if let Some(ref limiter) = self.rate_limiter {
                    limiter.record_failure(&limiter_key);
                }
                return None;
            }
            let payload = match base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(parts[1]) {
                Ok(p) => p,
                Err(e) => {
                    tracing::warn!("Base64 fail: {:?}", e);
                    if let Some(ref limiter) = self.rate_limiter {
                        limiter.record_failure(&limiter_key);
                    }
                    return None;
                }
            };
            match serde_json::from_slice(&payload) {
                Ok(c) => c,
                Err(e) => {
                    tracing::warn!("JSON fail: {:?}", e);
                    if let Some(ref limiter) = self.rate_limiter {
                        limiter.record_failure(&limiter_key);
                    }
                    return None;
                }
            }
        };

        let issuer = unverified_claims
            .get("iss")
            .and_then(|v| v.as_str())
            .or_else(|| {
                if let Some(ref limiter) = self.rate_limiter {
                    limiter.record_failure(&limiter_key);
                }
                None
            })?;

        // Update limiter key to be issuer-specific
        limiter_key = issuer.to_string();
        if let Some(ref limiter) = self.rate_limiter
            && let Err(backoff) = limiter.check(&limiter_key)
        {
            tracing::warn!("Rate limited for issuer {}: backoff {:?}", issuer, backoff);
            return None;
        }

        let audience = unverified_claims
            .get("aud")
            .and_then(|v| v.as_str())
            .or_else(|| {
                if let Some(ref limiter) = self.rate_limiter {
                    limiter.record_failure(&limiter_key);
                }
                None
            })?;

        let mut matched_profile = None;
        // Fail-closed: unreadable profiles -> auth denied. Java's BasicAuthenticationProvider
        // has no try/catch around checkPassword, so a store error surfaces as a 500 there.
        let mut session = match self.runtime_store.create_session() {
            Ok(session) => session,
            Err(error) => {
                tracing::error!(
                    error = %error,
                    "external auth: storage session unavailable; denying authentication"
                );
                return None;
            }
        };
        // Java parity: a profile-query storage failure throws inside the command; this
        // path always denies auth, but it must not do so silently.
        let profiles = match self.runtime_store.list_issuer_profiles(&mut session) {
            Ok(profiles) => profiles,
            Err(error) => {
                tracing::error!(
                    error = %error,
                    "external auth: issuer profile read failed; denying authentication"
                );
                let _ = session.rollback();
                return None;
            }
        };
        for profile in &profiles {
            if profile.issuer == issuer && profile.audience == audience {
                if profile.rollout_state == crate::service::issuer_profile::RolloutState::Deprecated
                {
                    continue;
                }

                let tenant_id_claim = profile
                    .mapping
                    .tenant_id_claim
                    .as_deref()
                    .unwrap_or("tenant");
                if profile.required_tenant && !unverified_claims.contains_key(tenant_id_claim) {
                    continue;
                }

                matched_profile = Some(profile.clone());
                break;
            }
        }
        let _ = session.rollback();
        let profile = matched_profile.or_else(|| {
            tracing::warn!("No profile for iss {} aud {}", issuer, audience);
            if let Some(ref limiter) = self.rate_limiter {
                limiter.record_failure(&limiter_key);
            }
            None
        })?;

        // NOW: Fetch the key from the JWKS cache and actually verify the signature
        let jwks_uri = profile.jwks_uri.as_deref().unwrap_or("");
        if jwks_uri.is_empty() {
            tracing::warn!("jwks_uri is empty for iss {} aud {}", issuer, audience);
            if let Some(ref limiter) = self.rate_limiter {
                limiter.record_failure(&limiter_key);
            }
            return None;
        }

        let decoding_key = match self.jwks_cache.get_key(
            issuer,
            kid,
            jwks_uri,
            std::time::Duration::from_secs(profile.jwks_cache_ttl_seconds),
            &profile.jwks_refresh_policy,
        ) {
            Ok(key) => key,
            Err(e) => {
                tracing::warn!("get_key failed: {}", e);
                if let Some(ref limiter) = self.rate_limiter {
                    limiter.record_failure(&limiter_key);
                }
                return None;
            }
        };

        let mut validation = jsonwebtoken::Validation::new(jsonwebtoken::Algorithm::RS256);
        validation.set_issuer(&[issuer]);
        validation.set_audience(&[audience]);

        let verified_token_res: Result<
            jsonwebtoken::TokenData<std::collections::HashMap<String, serde_json::Value>>,
            _,
        > = jsonwebtoken::decode(token_str, &decoding_key, &validation);

        let verified_token = match verified_token_res {
            Ok(token) => token,
            Err(e) => {
                tracing::warn!("verify failed: {:?}", e);
                if let Some(ref limiter) = self.rate_limiter {
                    limiter.record_failure(&limiter_key);
                }
                return None;
            }
        };

        let verified_claims = verified_token.claims;

        if let Some(jti) = verified_claims.get("jti").and_then(|v| v.as_str())
            && let RevocationStatus::Revoked { .. } = self.revocation_registry.check(jti)
        {
            if let Some(ref limiter) = self.rate_limiter {
                limiter.record_failure(&limiter_key);
            }
            return None;
        }

        let subject_claim = if profile.mapping.subject_claim.is_empty() {
            "sub"
        } else {
            &profile.mapping.subject_claim
        };
        let subject = verified_claims
            .get(subject_claim)
            .and_then(|v| v.as_str())
            .unwrap_or("sub");

        let tenant_id_claim = profile
            .mapping
            .tenant_id_claim
            .as_deref()
            .unwrap_or("tenant");
        let tenant_id = verified_claims
            .get(tenant_id_claim)
            .and_then(|v| v.as_str().map(|s| s.to_string()));

        let mut principal = Principal::new(subject, subject, issuer, tenant_id);
        principal = principal.with_profile_id(&profile.id);

        let role_claim = if profile.mapping.role_claim.is_empty() {
            "role"
        } else {
            &profile.mapping.role_claim
        };
        if let Some(roles) = verified_claims.get(role_claim) {
            let role_strs: Vec<&str> = if let Some(arr) = roles.as_array() {
                arr.iter().filter_map(|v| v.as_str()).collect()
            } else if let Some(s) = roles.as_str() {
                vec![s]
            } else {
                vec![]
            };

            for role in role_strs {
                let mapped_role = profile
                    .role_mappings
                    .iter()
                    .find(|r| r.external_role == role)
                    .map(|r| r.internal_role.as_str())
                    .unwrap_or(role);
                principal = principal.with_role(mapped_role);
            }
        }

        tracing::warn!("Returning Some(principal): id={}", principal.actor_id);

        // Reset failures on successful auth
        if let Some(ref limiter) = self.rate_limiter {
            limiter.reset(&limiter_key);
        }

        Some(principal)
    }
}
