use crate::service::issuer_profile::JwksRefreshPolicy;
use jsonwebtoken::DecodingKey;
use jsonwebtoken::jwk::{Jwk, JwkSet};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

/// Structured failure reasons for JWKS key resolution.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JwksKeyError {
    /// The kid was fetched but not found in the issuer's JWKS.
    UnknownKid { issuer: String, kid: String },
    /// A refresh was attempted but failed (network / parse / HTTP error).
    FetchFailed { issuer: String, detail: String },
    /// The kid is in the negative cache and we are not retrying yet.
    NegativelyCached { issuer: String, kid: String },
    /// Decoding key construction failed from an otherwise valid JWK.
    KeyConstructionFailed { detail: String },
    /// The key was found but is stale and stale-reuse is not allowed.
    StaleKeyRejected { issuer: String, kid: String },
}

impl std::fmt::Display for JwksKeyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownKid { issuer, kid } => {
                write!(f, "Key id {} not found in jwks for issuer {}", kid, issuer)
            }
            Self::FetchFailed { issuer, detail } => {
                write!(f, "Failed to fetch jwks for issuer {}: {}", issuer, detail)
            }
            Self::NegativelyCached { issuer, kid } => write!(
                f,
                "Key id {} is negatively cached for issuer {}",
                kid, issuer
            ),
            Self::KeyConstructionFailed { detail } => {
                write!(f, "Failed to construct decoding key: {}", detail)
            }
            Self::StaleKeyRejected { issuer, kid } => write!(
                f,
                "Stale key {} rejected for issuer {} (tolerance exceeded or not allowed)",
                kid, issuer
            ),
        }
    }
}

#[derive(Clone)]
pub struct CachedKey {
    pub key: Jwk,
    pub expires_at: Instant,
    pub jwks_uri: String,
}

/// Tracks per-issuer refresh timing for backoff and rate-limiting.
#[derive(Clone)]
struct IssuerRefreshState {
    last_attempt: Instant,
    consecutive_failures: u32,
    jwks_uri: String,
}

/// Entry in the negative cache for unknown kids.
#[derive(Clone)]
struct NegativeCacheEntry {
    expires_at: Instant,
    jwks_uri: String,
}

#[derive(Clone)]
pub struct JwksCache {
    keys: Arc<RwLock<HashMap<(String, String), CachedKey>>>,
    negative_cache: Arc<RwLock<HashMap<(String, String), NegativeCacheEntry>>>,
    refresh_state: Arc<RwLock<HashMap<String, IssuerRefreshState>>>,
    client: reqwest::blocking::Client,
}

impl JwksCache {
    pub fn new() -> Self {
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap_or_default();

        Self {
            keys: Arc::new(RwLock::new(HashMap::new())),
            negative_cache: Arc::new(RwLock::new(HashMap::new())),
            refresh_state: Arc::new(RwLock::new(HashMap::new())),
            client,
        }
    }

    /// Resolve a decoding key for the given issuer/kid, applying the
    /// configured refresh policy for backoff, negative cache, and stale
    /// key tolerance.
    pub fn get_key(
        &self,
        issuer: &str,
        kid: &str,
        jwks_uri: &str,
        cache_ttl: Duration,
        policy: &JwksRefreshPolicy,
    ) -> Result<DecodingKey, JwksKeyError> {
        let cache_key = (issuer.to_string(), kid.to_string());

        // 1. Check negative cache first to avoid hammering.
        {
            let neg = self.negative_cache.read().unwrap();
            if let Some(entry) = neg.get(&cache_key)
                && entry.expires_at > Instant::now()
                && entry.jwks_uri == jwks_uri
            {
                return Err(JwksKeyError::NegativelyCached {
                    issuer: issuer.to_string(),
                    kid: kid.to_string(),
                });
            }
        }

        // 2. Check fresh cache hit.
        {
            let lock = self.keys.read().unwrap();
            if let Some(cached) = lock.get(&cache_key)
                && cached.expires_at > Instant::now()
                && cached.jwks_uri == jwks_uri
            {
                return DecodingKey::from_jwk(&cached.key).map_err(|e| {
                    JwksKeyError::KeyConstructionFailed {
                        detail: e.to_string(),
                    }
                });
            }
        }

        // 3. Rate-limit refresh attempts per issuer.
        if !self.should_attempt_refresh(issuer, jwks_uri, policy) {
            // Check stale key tolerance.
            return self.try_stale_key(issuer, kid, jwks_uri, policy);
        }

        // 4. Attempt refresh.
        match self.refresh_keys(issuer, jwks_uri, cache_ttl) {
            Ok(()) => {
                self.record_refresh_success(issuer, jwks_uri);
            }
            Err(detail) => {
                self.record_refresh_failure(issuer, jwks_uri);
                // On failure, try stale key if policy allows.
                if policy.allow_stale_on_failure
                    && let Ok(key) = self.try_stale_key(issuer, kid, jwks_uri, policy)
                {
                    return Ok(key);
                }
                return Err(JwksKeyError::FetchFailed {
                    issuer: issuer.to_string(),
                    detail,
                });
            }
        }

        // 5. After successful refresh, look up the kid.
        let lock = self.keys.read().unwrap();
        if let Some(cached) = lock.get(&cache_key)
            && cached.jwks_uri == jwks_uri
        {
            return DecodingKey::from_jwk(&cached.key).map_err(|e| {
                JwksKeyError::KeyConstructionFailed {
                    detail: e.to_string(),
                }
            });
        }

        // Kid genuinely not in the JWKS — add to negative cache.
        drop(lock);
        self.add_negative_cache(issuer, kid, jwks_uri, policy.negative_cache_duration());
        Err(JwksKeyError::UnknownKid {
            issuer: issuer.to_string(),
            kid: kid.to_string(),
        })
    }

    /// Older API surface without an explicit policy.
    pub fn get_key_simple(
        &self,
        issuer: &str,
        kid: &str,
        jwks_uri: &str,
        cache_ttl: Duration,
    ) -> Result<DecodingKey, String> {
        let default_policy = JwksRefreshPolicy::default();
        self.get_key(issuer, kid, jwks_uri, cache_ttl, &default_policy)
            .map_err(|e| e.to_string())
    }

    /// Try to return a stale cached key within tolerance bounds.
    fn try_stale_key(
        &self,
        issuer: &str,
        kid: &str,
        jwks_uri: &str,
        policy: &JwksRefreshPolicy,
    ) -> Result<DecodingKey, JwksKeyError> {
        if !policy.allow_stale_on_failure {
            return Err(JwksKeyError::StaleKeyRejected {
                issuer: issuer.to_string(),
                kid: kid.to_string(),
            });
        }

        let lock = self.keys.read().unwrap();
        let cache_key = (issuer.to_string(), kid.to_string());
        if let Some(cached) = lock.get(&cache_key)
            && cached.jwks_uri == jwks_uri
        {
            let stale_deadline = cached.expires_at + policy.stale_tolerance();
            if Instant::now() <= stale_deadline {
                return DecodingKey::from_jwk(&cached.key).map_err(|e| {
                    JwksKeyError::KeyConstructionFailed {
                        detail: e.to_string(),
                    }
                });
            }
        }

        Err(JwksKeyError::StaleKeyRejected {
            issuer: issuer.to_string(),
            kid: kid.to_string(),
        })
    }

    fn should_attempt_refresh(
        &self,
        issuer: &str,
        jwks_uri: &str,
        policy: &JwksRefreshPolicy,
    ) -> bool {
        let lock = self.refresh_state.read().unwrap();
        if let Some(state) = lock.get(issuer) {
            if state.jwks_uri != jwks_uri {
                return true;
            }
            let backoff_delay = self.compute_backoff_delay(state.consecutive_failures, policy);
            let min_interval = policy.min_refresh_interval().max(backoff_delay);
            state.last_attempt.elapsed() >= min_interval
        } else {
            true
        }
    }

    fn compute_backoff_delay(
        &self,
        consecutive_failures: u32,
        policy: &JwksRefreshPolicy,
    ) -> Duration {
        if consecutive_failures == 0 {
            return Duration::from_secs(0);
        }
        let base = policy.min_refresh_interval_seconds as f64;
        let delay = base * policy.backoff_multiplier.powi(consecutive_failures as i32);
        let capped = delay.min(policy.max_retry_delay_seconds as f64);
        Duration::from_secs(capped as u64)
    }

    fn record_refresh_success(&self, issuer: &str, jwks_uri: &str) {
        let mut lock = self.refresh_state.write().unwrap();
        lock.insert(
            issuer.to_string(),
            IssuerRefreshState {
                last_attempt: Instant::now(),
                consecutive_failures: 0,
                jwks_uri: jwks_uri.to_string(),
            },
        );
    }

    fn record_refresh_failure(&self, issuer: &str, jwks_uri: &str) {
        let mut lock = self.refresh_state.write().unwrap();
        let entry = lock
            .entry(issuer.to_string())
            .or_insert(IssuerRefreshState {
                last_attempt: Instant::now(),
                consecutive_failures: 0,
                jwks_uri: jwks_uri.to_string(),
            });
        entry.last_attempt = Instant::now();
        entry.consecutive_failures = entry.consecutive_failures.saturating_add(1);
        entry.jwks_uri = jwks_uri.to_string();
    }

    fn add_negative_cache(&self, issuer: &str, kid: &str, jwks_uri: &str, ttl: Duration) {
        let mut lock = self.negative_cache.write().unwrap();
        lock.insert(
            (issuer.to_string(), kid.to_string()),
            NegativeCacheEntry {
                expires_at: Instant::now() + ttl,
                jwks_uri: jwks_uri.to_string(),
            },
        );
    }

    fn refresh_keys(
        &self,
        issuer: &str,
        jwks_uri: &str,
        cache_ttl: Duration,
    ) -> Result<(), String> {
        if jwks_uri == "test-local" {
            let jwk_set: JwkSet = serde_json::from_str(r#"{"keys":[{"kty":"RSA","kid":"test-kid","n":"sZheYveJ-RGFFYQ5l5skvpvkBlCmm0vrfkH1yjZMaH2kAAbMlf4d5h-a1DUNw3Rniq7zXdCYz_fsr-MR9hiHowJeE46ApTxFrORAds1Wz6_7RSgFQYZJ-rAeEUx_xR35IGl6jID0ibHyupbpKpcGsZqS-geHapRqgLv2dDvD0YcyqO5Ncmy0bBXYvi66WsC73YV3KR26iD3qi4KEGDxg_cL22fwRk2E2l8ZCf_5ZtlED7xmJsSoeOAR-bQLwaLcdmkwC9DiYCKXU8E2dzZq1mmu6Xf54o0ymk5JC9OsHgkJghS3jPnskGJBHGGyFpGE9Xyq97R8W1_S7u5H8axVbvw","e":"AQAB"}]}"#).unwrap();
            let mut lock = self.keys.write().unwrap();
            let expires_at = Instant::now() + cache_ttl;
            for jwk in jwk_set.keys {
                if let Some(kid) = &jwk.common.key_id {
                    lock.insert(
                        (issuer.to_string(), kid.clone()),
                        CachedKey {
                            key: jwk.clone(),
                            expires_at,
                            jwks_uri: jwks_uri.to_string(),
                        },
                    );
                }
            }
            return Ok(());
        }

        let res = self
            .client
            .get(jwks_uri)
            .send()
            .map_err(|e| format!("Failed to fetch jwks: {}", e))?;

        if !res.status().is_success() {
            return Err(format!("Failed to fetch jwks: HTTP {}", res.status()));
        }

        let jwk_set: JwkSet = res
            .json()
            .map_err(|e| format!("Failed to parse jwks: {}", e))?;

        let mut lock = self.keys.write().unwrap();
        let expires_at = Instant::now() + cache_ttl;

        for jwk in jwk_set.keys {
            if let Some(kid) = &jwk.common.key_id {
                lock.insert(
                    (issuer.to_string(), kid.clone()),
                    CachedKey {
                        key: jwk.clone(),
                        expires_at,
                        jwks_uri: jwks_uri.to_string(),
                    },
                );
            }
        }

        Ok(())
    }

    pub fn inject_key(&self, issuer: &str, kid: &str, key: Jwk) {
        let mut lock = self.keys.write().unwrap();
        lock.insert(
            (issuer.to_string(), kid.to_string()),
            CachedKey {
                key,
                expires_at: Instant::now() + Duration::from_secs(3600),
                jwks_uri: "test-local".to_string(),
            },
        );
    }

    /// Inject a key with a custom TTL (for testing stale-key scenarios).
    pub fn inject_key_with_ttl(&self, issuer: &str, kid: &str, key: Jwk, ttl: Duration) {
        let mut lock = self.keys.write().unwrap();
        lock.insert(
            (issuer.to_string(), kid.to_string()),
            CachedKey {
                key,
                expires_at: Instant::now() + ttl,
                jwks_uri: "test-local".to_string(),
            },
        );
    }

    /// Inject a pre-expired key (for testing stale-key tolerance).
    pub fn inject_expired_key(
        &self,
        issuer: &str,
        kid: &str,
        jwks_uri: &str,
        key: Jwk,
        expired_ago: Duration,
    ) {
        let mut lock = self.keys.write().unwrap();
        lock.insert(
            (issuer.to_string(), kid.to_string()),
            CachedKey {
                key,
                expires_at: Instant::now() - expired_ago,
                jwks_uri: jwks_uri.to_string(),
            },
        );
    }

    /// Clear negative cache entries for testing.
    pub fn clear_negative_cache(&self) {
        let mut lock = self.negative_cache.write().unwrap();
        lock.clear();
    }

    /// Clear refresh state for testing.
    pub fn clear_refresh_state(&self) {
        let mut lock = self.refresh_state.write().unwrap();
        lock.clear();
    }

    pub fn count_keys_for_issuer(&self, issuer: &str) -> usize {
        let lock = self.keys.read().unwrap();
        let now = Instant::now();
        lock.iter()
            .filter(|((iss, _), cached)| iss == issuer && cached.expires_at > now)
            .count()
    }

    pub fn count_negative_cache_for_issuer(&self, issuer: &str) -> usize {
        let lock = self.negative_cache.read().unwrap();
        let now = Instant::now();
        lock.iter()
            .filter(|((iss, _), entry)| iss == issuer && entry.expires_at > now)
            .count()
    }

    pub fn refresh_state_for_issuer(&self, issuer: &str) -> (u32, Option<u64>) {
        let lock = self.refresh_state.read().unwrap();
        match lock.get(issuer) {
            Some(state) => {
                let ago = state.last_attempt.elapsed().as_secs();
                (state.consecutive_failures, Some(ago))
            }
            None => (0, None),
        }
    }

    /// Evicts all cached keys, negative cache entries, and refresh state
    /// associated with the given issuer. This ensures that any subsequent
    /// request for this issuer forces a fresh JWKS resolution.
    pub fn invalidate_issuer(&self, issuer: &str) {
        {
            let mut keys_lock = self.keys.write().unwrap();
            keys_lock.retain(|(iss, _), _| iss != issuer);
        }
        {
            let mut neg_cache_lock = self.negative_cache.write().unwrap();
            neg_cache_lock.retain(|(iss, _), _| iss != issuer);
        }
        {
            let mut refresh_lock = self.refresh_state.write().unwrap();
            refresh_lock.remove(issuer);
        }
    }
}

impl Default for JwksCache {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_jwk() -> Jwk {
        serde_json::from_str(r#"{
            "kty": "RSA",
            "kid": "test-kid",
            "n": "sZheYveJ-RGFFYQ5l5skvpvkBlCmm0vrfkH1yjZMaH2kAAbMlf4d5h-a1DUNw3Rniq7zXdCYz_fsr-MR9hiHowJeE46ApTxFrORAds1Wz6_7RSgFQYZJ-rAeEUx_xR35IGl6jID0ibHyupbpKpcGsZqS-geHapRqgLv2dDvD0YcyqO5Ncmy0bBXYvi66WsC73YV3KR26iD3qi4KEGDxg_cL22fwRk2E2l8ZCf_5ZtlED7xmJsSoeOAR-bQLwaLcdmkwC9DiYCKXU8E2dzZq1mmu6Xf54o0ymk5JC9OsHgkJghS3jPnskGJBHGGyFpGE9Xyq97R8W1_S7u5H8axVbvw",
            "e": "AQAB"
        }"#).unwrap()
    }

    fn default_policy() -> JwksRefreshPolicy {
        JwksRefreshPolicy::default()
    }

    #[test]
    fn test_get_key_refreshes_from_test_local() {
        let cache = JwksCache::new();
        let key = cache.get_key(
            "issuer-a",
            "test-kid",
            "test-local",
            Duration::from_secs(60),
            &default_policy(),
        );

        assert!(key.is_ok(), "test-local JWKS should provide a decoding key");

        let lock = cache.keys.read().unwrap();
        assert!(lock.contains_key(&("issuer-a".to_string(), "test-kid".to_string())));
    }

    #[test]
    fn test_expired_cached_key_refreshes_with_new_ttl() {
        let cache = JwksCache::new();
        {
            let mut lock = cache.keys.write().unwrap();
            lock.insert(
                ("issuer-b".to_string(), "test-kid".to_string()),
                CachedKey {
                    key: test_jwk(),
                    expires_at: Instant::now() - Duration::from_secs(1),
                    jwks_uri: "test-local".to_string(),
                },
            );
        }

        let ttl = Duration::from_secs(120);
        let key = cache.get_key("issuer-b", "test-kid", "test-local", ttl, &default_policy());
        assert!(key.is_ok(), "expired key should refresh from JWKS source");

        let lock = cache.keys.read().unwrap();
        let refreshed = lock
            .get(&("issuer-b".to_string(), "test-kid".to_string()))
            .expect("refreshed key should be present");
        assert!(refreshed.expires_at > Instant::now() + Duration::from_secs(100));
    }

    #[test]
    fn test_unknown_kid_enters_negative_cache() {
        let cache = JwksCache::new();
        let policy = JwksRefreshPolicy {
            negative_cache_seconds: 60,
            ..Default::default()
        };

        // First call fetches from test-local but kid "unknown-kid" is not there.
        let result = cache.get_key(
            "issuer-c",
            "unknown-kid",
            "test-local",
            Duration::from_secs(60),
            &policy,
        );
        assert!(matches!(result, Err(JwksKeyError::UnknownKid { .. })));

        // Second call should hit negative cache.
        let result2 = cache.get_key(
            "issuer-c",
            "unknown-kid",
            "test-local",
            Duration::from_secs(60),
            &policy,
        );
        assert!(matches!(
            result2,
            Err(JwksKeyError::NegativelyCached { .. })
        ));
    }

    #[test]
    fn test_stale_key_tolerance_allows_use() {
        let cache = JwksCache::new();
        let policy = JwksRefreshPolicy {
            allow_stale_on_failure: true,
            stale_tolerance_seconds: 120,
            min_refresh_interval_seconds: 0,
            ..Default::default()
        };

        // Insert a key that expired 10 seconds ago.
        cache.inject_expired_key(
            "issuer-d",
            "test-kid",
            "http://127.0.0.1:1/nonexistent",
            test_jwk(),
            Duration::from_secs(10),
        );

        // Using a bad URI that will fail to fetch.
        let result = cache.get_key(
            "issuer-d",
            "test-kid",
            "http://127.0.0.1:1/nonexistent",
            Duration::from_secs(60),
            &policy,
        );
        assert!(
            result.is_ok(),
            "Stale key within tolerance should be accepted on fetch failure"
        );
    }

    #[test]
    fn test_stale_key_rejected_when_not_allowed() {
        let cache = JwksCache::new();
        let policy = JwksRefreshPolicy {
            allow_stale_on_failure: false,
            min_refresh_interval_seconds: 0,
            ..Default::default()
        };

        cache.inject_expired_key(
            "issuer-e",
            "test-kid",
            "http://127.0.0.1:1/nonexistent",
            test_jwk(),
            Duration::from_secs(10),
        );

        let result = cache.get_key(
            "issuer-e",
            "test-kid",
            "http://127.0.0.1:1/nonexistent",
            Duration::from_secs(60),
            &policy,
        );
        assert!(
            matches!(result, Err(JwksKeyError::FetchFailed { .. })),
            "Stale key should be rejected when allow_stale_on_failure=false"
        );
    }

    #[test]
    fn test_stale_key_rejected_past_tolerance() {
        let cache = JwksCache::new();
        let policy = JwksRefreshPolicy {
            allow_stale_on_failure: true,
            stale_tolerance_seconds: 5, // only 5 seconds of stale tolerance
            min_refresh_interval_seconds: 0,
            ..Default::default()
        };

        // Expired 30 seconds ago, tolerance is 5 seconds → too stale.
        cache.inject_expired_key(
            "issuer-f",
            "test-kid",
            "http://127.0.0.1:1/nonexistent",
            test_jwk(),
            Duration::from_secs(30),
        );

        let result = cache.get_key(
            "issuer-f",
            "test-kid",
            "http://127.0.0.1:1/nonexistent",
            Duration::from_secs(60),
            &policy,
        );
        assert!(
            result.is_err(),
            "Key stale beyond tolerance should be rejected"
        );
    }

    #[test]
    fn test_refresh_rate_limiting() {
        let cache = JwksCache::new();
        let policy = JwksRefreshPolicy {
            min_refresh_interval_seconds: 3600, // very long to force rate limiting
            ..Default::default()
        };

        // First call succeeds and records state.
        let _ = cache.get_key(
            "issuer-g",
            "test-kid",
            "test-local",
            Duration::from_secs(1),
            &policy,
        );

        // Expire the cached key immediately.
        {
            let mut lock = cache.keys.write().unwrap();
            if let Some(entry) = lock.get_mut(&("issuer-g".to_string(), "test-kid".to_string())) {
                entry.expires_at = Instant::now() - Duration::from_secs(1);
            }
        }

        // Second call is rate-limited — no stale tolerance allowed, so it fails.
        let result = cache.get_key(
            "issuer-g",
            "test-kid",
            "test-local",
            Duration::from_secs(1),
            &policy,
        );
        assert!(
            result.is_err(),
            "Should be rate-limited and reject stale key without allow_stale_on_failure"
        );
    }
}
