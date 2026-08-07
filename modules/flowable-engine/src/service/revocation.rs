use crate::persistence::runtime_store::{RuntimeStore, RuntimeTokenRevocation};
use std::time::Duration;

/// A database-backed token revocation registry keyed by `jti` (JWT ID).
///
/// Design decisions:
/// - This is a database-backed, cluster-coherent revocation source of truth.
/// - Revocation entries carry a TTL so the registry does not grow unbounded.
///   Entries older than their TTL are lazily ignored on check and explicitly evicted.
/// - The registry is fail-closed: if a token has a `jti` and that `jti` is
///   revoked, the token is rejected before it reaches authorization/audit.
/// - Tokens without a `jti` cannot be individually revoked; they rely on
///   their natural expiry or key rotation.
#[derive(Clone)]
pub struct TokenRevocationRegistry {
    runtime_store: RuntimeStore,
}

/// Result of checking a token against the revocation registry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RevocationStatus {
    /// The token is not revoked (or has no jti).
    NotRevoked,
    /// The token is revoked.
    Revoked {
        jti: String,
        issuer: String,
        reason: String,
    },
}

impl TokenRevocationRegistry {
    pub fn new(runtime_store: RuntimeStore) -> Self {
        Self { runtime_store }
    }

    /// Creates an in-memory revocation registry, useful for tests.
    pub fn new_in_memory() -> Self {
        let db_store =
            std::sync::Arc::new(crate::persistence::db_store::DbStore::new_in_memory().unwrap());
        let runtime_store = RuntimeStore::new_with_memory_backend_for_test(db_store);
        Self::new(runtime_store)
    }

    /// Revoke a token by its `jti`.
    ///
    /// `ttl` controls how long the revocation entry persists. It should
    /// typically match the token's remaining lifetime so that after the
    /// token would have expired naturally, the entry is evicted.
    pub fn revoke(&self, jti: &str, issuer: &str, reason: &str, ttl: Duration) {
        let now = self.runtime_store.time_source().now().timestamp_millis();
        let expires_at = now + ttl.as_millis() as i64;
        let mut session = self.runtime_store.create_session().unwrap();
        self.runtime_store.insert_token_revocation(
            RuntimeTokenRevocation {
                jti: jti.to_string(),
                issuer: issuer.to_string(),
                reason: reason.to_string(),
                expires_at,
                created_at: now,
            },
            &mut session,
        );
        session.flush_and_commit().unwrap();
    }

    /// Check whether a given `jti` is revoked.
    pub fn check(&self, jti: &str) -> RevocationStatus {
        let now = self.runtime_store.time_source().now().timestamp_millis();
        let mut session = self.runtime_store.create_session().unwrap();

        match self.runtime_store.find_token_revocation(jti, &mut session) {
            Some(entry) if entry.expires_at > now => {
                session.rollback().unwrap();
                RevocationStatus::Revoked {
                    jti: entry.jti,
                    issuer: entry.issuer,
                    reason: entry.reason,
                }
            }
            Some(_expired) => {
                self.runtime_store
                    .delete_token_revocation(jti, &mut session);
                session.flush_and_commit().unwrap();
                RevocationStatus::NotRevoked
            }
            None => {
                session.rollback().unwrap();
                RevocationStatus::NotRevoked
            }
        }
    }

    /// Remove expired entries. Call periodically to bound memory/db size.
    pub fn evict_expired(&self) {
        let mut session = self.runtime_store.create_session().unwrap();
        self.runtime_store
            .cleanup_expired_token_revocations(&mut session);
        let _ = session.flush_and_commit();
    }

    /// Number of active (non-expired) revocation entries.
    pub fn active_count(&self) -> usize {
        let mut session = self.runtime_store.create_session().unwrap();
        let count = self
            .runtime_store
            .count_active_token_revocations(&mut session);
        session.rollback().unwrap();
        count
    }

    /// Remove a specific revocation entry (un-revoke).
    pub fn remove(&self, jti: &str) -> bool {
        let mut session = self.runtime_store.create_session().unwrap();
        let removed = self
            .runtime_store
            .delete_token_revocation(jti, &mut session);
        let _ = session.flush_and_commit();
        removed
    }

    /// Admin read: check whether a jti is revoked, returning details if so.
    /// Unlike `check`, this does not perform lazy eviction.
    pub fn admin_check(&self, jti: &str) -> RevocationStatus {
        let now = self.runtime_store.time_source().now().timestamp_millis();
        let mut session = self.runtime_store.create_session().unwrap();
        let result = if let Some(entry) =
            self.runtime_store.find_token_revocation(jti, &mut session)
            && entry.expires_at > now
        {
            RevocationStatus::Revoked {
                jti: entry.jti,
                issuer: entry.issuer,
                reason: entry.reason,
            }
        } else {
            RevocationStatus::NotRevoked
        };
        session.rollback().unwrap();
        result
    }

    /// Admin revoke: revoke a token by jti with a default TTL of 1 hour.
    pub fn admin_revoke(&self, jti: &str, issuer: &str, reason: &str) {
        self.revoke(jti, issuer, reason, Duration::from_secs(3600));
    }

    /// Admin revoke with explicit TTL.
    pub fn admin_revoke_with_ttl(&self, jti: &str, issuer: &str, reason: &str, ttl: Duration) {
        self.revoke(jti, issuer, reason, ttl);
    }

    /// Admin un-revoke: remove a revocation entry.
    pub fn admin_unrevoke(&self, jti: &str) -> bool {
        self.remove(jti)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_revoke_and_check() {
        let registry = TokenRevocationRegistry::new_in_memory();

        assert_eq!(registry.check("jti-1"), RevocationStatus::NotRevoked);

        registry.revoke(
            "jti-1",
            "issuer-a",
            "compromised",
            Duration::from_secs(3600),
        );

        let status = registry.check("jti-1");
        assert!(
            matches!(status, RevocationStatus::Revoked { .. }),
            "Expected revoked"
        );
        if let RevocationStatus::Revoked {
            jti,
            issuer,
            reason,
        } = status
        {
            assert_eq!(jti, "jti-1");
            assert_eq!(issuer, "issuer-a");
            assert_eq!(reason, "compromised");
        }
    }

    #[test]
    fn test_expired_revocation_is_not_revoked() {
        let registry = TokenRevocationRegistry::new_in_memory();

        // Revoke with zero TTL → immediately expired.
        registry.revoke("jti-2", "issuer-a", "test", Duration::from_secs(0));

        // Small sleep to ensure time passes.
        std::thread::sleep(Duration::from_millis(10));

        assert_eq!(registry.check("jti-2"), RevocationStatus::NotRevoked);
        assert_eq!(
            registry.active_count(),
            0,
            "Expired entry should be lazily evicted on check"
        );
    }

    #[test]
    fn test_evict_expired() {
        let registry = TokenRevocationRegistry::new_in_memory();

        registry.revoke("jti-3", "issuer-a", "test", Duration::from_secs(0));
        registry.revoke("jti-4", "issuer-a", "test", Duration::from_secs(3600));

        std::thread::sleep(Duration::from_millis(10));

        registry.evict_expired();
        assert_eq!(registry.active_count(), 1);
    }

    #[test]
    fn test_remove() {
        let registry = TokenRevocationRegistry::new_in_memory();

        registry.revoke("jti-5", "issuer-a", "test", Duration::from_secs(3600));
        assert!(matches!(
            registry.check("jti-5"),
            RevocationStatus::Revoked { .. }
        ));

        assert!(registry.remove("jti-5"));
        assert_eq!(registry.check("jti-5"), RevocationStatus::NotRevoked);

        assert!(!registry.remove("nonexistent"));
    }

    #[test]
    fn test_active_count() {
        let registry = TokenRevocationRegistry::new_in_memory();
        assert_eq!(registry.active_count(), 0);

        registry.revoke("a", "i", "r", Duration::from_secs(3600));
        registry.revoke("b", "i", "r", Duration::from_secs(3600));
        assert_eq!(registry.active_count(), 2);
    }
}
