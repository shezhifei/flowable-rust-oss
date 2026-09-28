use crate::error::FlowableError;
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
        // In-memory backend creation is infallible in practice; fail fast with
        // context instead of a bare unwrap.
        let db_store = std::sync::Arc::new(
            crate::persistence::db_store::DbStore::new_in_memory()
                .expect("in-memory revocation store must initialize"),
        );
        let runtime_store = RuntimeStore::new_with_memory_backend_for_test(db_store);
        Self::new(runtime_store)
    }

    /// Revoke a token by its `jti`.
    ///
    /// `ttl` controls how long the revocation entry persists. It should
    /// typically match the token's remaining lifetime so that after the
    /// token would have expired naturally, the entry is evicted.
    ///
    /// Returns the storage failure instead of swallowing it. The previous body logged a
    /// warning and returned normally, so `POST /revocation/revoke` answered
    /// `{"success": true}` for a revocation that was never persisted — the token stayed
    /// live while the operator was told it had been revoked.
    pub fn revoke(
        &self,
        jti: &str,
        issuer: &str,
        reason: &str,
        ttl: Duration,
    ) -> Result<(), FlowableError> {
        let now = self.runtime_store.time_source().now().timestamp_millis();
        let expires_at = now + ttl.as_millis() as i64;
        let mut session = self.runtime_store.create_session()?;
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
        // Surfaces both a failed insert (recorded via `note_write_error`) and a failed
        // commit, so neither can masquerade as a persisted revocation.
        session.flush_and_commit()?;
        Ok(())
    }

    /// Check whether a given `jti` is revoked.
    pub fn check(&self, jti: &str) -> RevocationStatus {
        let now = self.runtime_store.time_source().now().timestamp_millis();
        // Fail-closed: DB unavailable -> treat as revoked (deny), never panic.
        // Stricter than Java 8, which has no token revocation store at all.
        let Ok(mut session) = self.runtime_store.create_session() else {
            return unavailable(jti);
        };

        let found = self.runtime_store.find_token_revocation(jti, &mut session);
        // `find_token_revocation` reports a storage failure as `None` plus a sticky
        // session error, so the `None` arm below would otherwise answer "not revoked"
        // during an outage and accept a token that may well be revoked. Probe the slot
        // before trusting the lookup result.
        if session.has_pending_error() {
            let _ = session.rollback();
            return RevocationStatus::Revoked {
                jti: jti.to_string(),
                issuer: String::new(),
                reason: "revocation store lookup failed".to_string(),
            };
        }

        match found {
            Some(entry) if entry.expires_at > now => {
                let _ = session.rollback();
                RevocationStatus::Revoked {
                    jti: entry.jti,
                    issuer: entry.issuer,
                    reason: entry.reason,
                }
            }
            Some(_expired) => {
                self.runtime_store
                    .delete_token_revocation(jti, &mut session);
                let _ = session.flush_and_commit();
                RevocationStatus::NotRevoked
            }
            None => {
                let _ = session.rollback();
                RevocationStatus::NotRevoked
            }
        }
    }

    /// Remove expired entries. Call periodically to bound memory/db size.
    pub fn evict_expired(&self) -> Result<(), FlowableError> {
        let mut session = self.runtime_store.create_session()?;
        self.runtime_store
            .cleanup_expired_token_revocations(&mut session);
        session.flush_and_commit()?;
        Ok(())
    }

    /// Number of active (non-expired) revocation entries.
    pub fn active_count(&self) -> Result<usize, FlowableError> {
        let mut session = self.runtime_store.create_session()?;
        let count = self
            .runtime_store
            .count_active_token_revocations(&mut session);
        // A failed count used to answer `0`, which reads as "nothing is revoked" during
        // an outage — exactly the wrong direction for a security control.
        session.rollback_read()?;
        Ok(count)
    }

    /// Remove a specific revocation entry (un-revoke).
    pub fn remove(&self, jti: &str) -> Result<bool, FlowableError> {
        let mut session = self.runtime_store.create_session()?;
        let removed = self
            .runtime_store
            .delete_token_revocation(jti, &mut session);
        session.flush_and_commit()?;
        Ok(removed)
    }

    /// Admin read: check whether a jti is revoked, returning details if so.
    /// Unlike `check`, this does not perform lazy eviction.
    pub fn admin_check(&self, jti: &str) -> RevocationStatus {
        let now = self.runtime_store.time_source().now().timestamp_millis();
        let Ok(mut session) = self.runtime_store.create_session() else {
            return unavailable(jti);
        };
        let found = self.runtime_store.find_token_revocation(jti, &mut session);
        // Same fail-closed rule as `check`: a storage error must not read as "not revoked".
        if session.has_pending_error() {
            let _ = session.rollback();
            return RevocationStatus::Revoked {
                jti: jti.to_string(),
                issuer: String::new(),
                reason: "revocation store lookup failed".to_string(),
            };
        }
        let result = if let Some(entry) = found
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
        let _ = session.rollback();
        result
    }

    /// Admin revoke: revoke a token by jti with a default TTL of 1 hour.
    pub fn admin_revoke(
        &self,
        jti: &str,
        issuer: &str,
        reason: &str,
    ) -> Result<(), FlowableError> {
        self.revoke(jti, issuer, reason, Duration::from_secs(3600))
    }

    /// Admin revoke with explicit TTL.
    pub fn admin_revoke_with_ttl(
        &self,
        jti: &str,
        issuer: &str,
        reason: &str,
        ttl: Duration,
    ) -> Result<(), FlowableError> {
        self.revoke(jti, issuer, reason, ttl)
    }

    /// Admin un-revoke: remove a revocation entry.
    pub fn admin_unrevoke(&self, jti: &str) -> Result<bool, FlowableError> {
        self.remove(jti)
    }
}

/// Fail-closed status used when the registry cannot even open a session.
fn unavailable(jti: &str) -> RevocationStatus {
    RevocationStatus::Revoked {
        jti: jti.to_string(),
        issuer: String::new(),
        reason: "revocation store unavailable".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Makes every later read of `token_revocations` fail at the driver level, the way
    /// an unreachable database or a dropped table fails in production.
    fn drop_revocation_table(registry: &TokenRevocationRegistry) {
        let mut session = registry
            .runtime_store
            .create_session()
            .expect("session for schema change");
        session
            .execute_raw_sql("DROP TABLE token_revocations")
            .expect("drop token_revocations");
        session.flush_and_commit().expect("commit drop");
    }

    #[test]
    fn test_revoke_and_check() {
        let registry = TokenRevocationRegistry::new_in_memory();

        assert_eq!(registry.check("jti-1"), RevocationStatus::NotRevoked);

        registry
            .revoke(
                "jti-1",
                "issuer-a",
                "compromised",
                Duration::from_secs(3600),
            )
            .expect("in-memory revoke must succeed");

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
        registry
            .revoke("jti-2", "issuer-a", "test", Duration::from_secs(0))
            .expect("in-memory revoke must succeed");

        // Small sleep to ensure time passes.
        std::thread::sleep(Duration::from_millis(10));

        assert_eq!(registry.check("jti-2"), RevocationStatus::NotRevoked);
        assert_eq!(
            registry.active_count().expect("in-memory count must succeed"),
            0,
            "Expired entry should be lazily evicted on check"
        );
    }

    #[test]
    fn test_evict_expired() {
        let registry = TokenRevocationRegistry::new_in_memory();

        registry
            .revoke("jti-3", "issuer-a", "test", Duration::from_secs(0))
            .expect("in-memory revoke must succeed");
        registry
            .revoke("jti-4", "issuer-a", "test", Duration::from_secs(3600))
            .expect("in-memory revoke must succeed");

        std::thread::sleep(Duration::from_millis(10));

        registry.evict_expired().expect("in-memory evict must succeed");
        assert_eq!(
            registry.active_count().expect("in-memory count must succeed"),
            1
        );
    }

    #[test]
    fn test_remove() {
        let registry = TokenRevocationRegistry::new_in_memory();

        registry
            .revoke("jti-5", "issuer-a", "test", Duration::from_secs(3600))
            .expect("in-memory revoke must succeed");
        assert!(matches!(
            registry.check("jti-5"),
            RevocationStatus::Revoked { .. }
        ));

        assert!(registry.remove("jti-5").expect("in-memory remove must succeed"));
        assert_eq!(registry.check("jti-5"), RevocationStatus::NotRevoked);

        assert!(
            !registry
                .remove("nonexistent")
                .expect("in-memory remove must succeed"),
            "a no-row delete is not a storage failure"
        );
    }

    #[test]
    fn test_active_count() {
        let registry = TokenRevocationRegistry::new_in_memory();
        assert_eq!(registry.active_count().expect("in-memory count"), 0);

        registry
            .revoke("a", "i", "r", Duration::from_secs(3600))
            .expect("in-memory revoke must succeed");
        registry
            .revoke("b", "i", "r", Duration::from_secs(3600))
            .expect("in-memory revoke must succeed");
        assert_eq!(registry.active_count().expect("in-memory count"), 2);
    }

    /// The core regression this registry must never ship again: a store outage used to
    /// answer "not revoked", because `find_token_revocation` reports a read failure as
    /// `None`. That is fail-OPEN — a revoked token is accepted exactly when nobody can
    /// verify the revocation list.
    #[test]
    fn storage_failure_during_check_is_fail_closed() {
        let registry = TokenRevocationRegistry::new_in_memory();
        drop_revocation_table(&registry);

        assert!(
            matches!(registry.check("jti-x"), RevocationStatus::Revoked { .. }),
            "a failed lookup must deny, not report 'not revoked'"
        );
        assert!(
            matches!(
                registry.admin_check("jti-x"),
                RevocationStatus::Revoked { .. }
            ),
            "admin_check must deny on a failed lookup too"
        );
    }

    /// The admin control plane must report the outage, not answer `{"success": true}`.
    #[test]
    fn storage_failure_during_admin_write_is_reported() {
        let registry = TokenRevocationRegistry::new_in_memory();
        drop_revocation_table(&registry);

        assert!(
            registry
                .admin_revoke("jti-y", "issuer-a", "compromised")
                .is_err(),
            "a revoke that could not be persisted must not report success"
        );
        assert!(
            registry.active_count().is_err(),
            "a count that could not be read must not report 0"
        );
        assert!(
            registry.admin_unrevoke("jti-y").is_err(),
            "an un-revoke that could not be performed must not report 'not found'"
        );
    }
}
