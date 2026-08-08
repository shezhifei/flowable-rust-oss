//! Password hashing (argon2id) with constant-time verification.
//!
//! Security deviation from Java: Flowable Java stores user passwords in
//! plaintext; this engine stores an argon2id PHC-string hash instead.
//! Legacy plaintext rows (created before this change, or by Java-compatible
//! tooling) are still verifiable via a constant-time comparison and are
//! upgraded to hashes on the next save.

use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};

/// Argon2id parameters per OWASP recommendations: m=19MiB, t=2, p=1.
const M_COST: u32 = 19_456;
const T_COST: u32 = 2;
const P_COST: u32 = 1;

/// PHC prefix emitted by `hash_password`; used to distinguish hashes from
/// legacy plaintext values on read.
pub const HASH_PREFIX: &str = "$argon2id$";

/// True when `stored` is an argon2id hash rather than a legacy plaintext row.
pub fn is_hash(stored: &str) -> bool {
    stored.starts_with(HASH_PREFIX)
}

/// Hash a plaintext password into an argon2id PHC string. The salt comes from
/// `uuid` v4's getrandom-backed RNG (no additional rand dependency). Never
/// returns an error: the parameter set is compile-time constant and valid.
pub fn hash_password(plain: &str) -> String {
    let salt_bytes = *uuid::Uuid::new_v4().as_bytes();
    let salt = SaltString::encode_b64(&salt_bytes).expect("16 random bytes are a valid salt");
    let params = argon2::Params::new(M_COST, T_COST, P_COST, None)
        .expect("argon2 parameter set is valid");
    argon2::Argon2::new(
        argon2::Algorithm::Argon2id,
        argon2::Version::V0x13,
        params,
    )
    .hash_password(plain.as_bytes(), &salt)
    .expect("argon2 hashing with valid params cannot fail")
    .to_string()
}

/// Verify `plain` against a stored value.
///
/// - argon2id hash → argon2 verification;
/// - anything else → legacy plaintext comparison in constant time (bounded by
///   the longer input), so the pre-hash migration path never reintroduces a
///   timing side channel.
pub fn verify_password(plain: &str, stored: &str) -> bool {
    if is_hash(stored) {
        match PasswordHash::new(stored) {
            Ok(parsed) => argon2::Argon2::default()
                .verify_password(plain.as_bytes(), &parsed)
                .is_ok(),
            Err(_) => false,
        }
    } else {
        constant_time_eq(plain.as_bytes(), stored.as_bytes())
    }
}

/// Byte-wise comparison that runs in time proportional to the longer input,
/// independent of where the first difference occurs.
fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    let mut diff = (left.len() as u64) ^ (right.len() as u64);
    let max_len = left.len().max(right.len());
    for i in 0..max_len {
        let a = left.get(i).copied().unwrap_or(0);
        let b = right.get(i).copied().unwrap_or(0);
        diff |= u64::from(a) ^ u64::from(b);
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_round_trip_verifies() {
        let hash = hash_password("correct horse battery staple");
        assert!(is_hash(&hash));
        assert!(verify_password("correct horse battery staple", &hash));
        assert!(!verify_password("wrong", &hash));
    }

    #[test]
    fn hashes_are_salted_and_unique() {
        let a = hash_password("same-password");
        let b = hash_password("same-password");
        assert_ne!(a, b);
        assert!(verify_password("same-password", &a));
        assert!(verify_password("same-password", &b));
    }

    #[test]
    fn verify_legacy_plaintext_still_works() {
        assert!(verify_password("secret", "secret"));
        assert!(!verify_password("wrong", "secret"));
    }

    #[test]
    fn constant_time_eq_handles_unequal_lengths() {
        assert!(!constant_time_eq(b"a", b"ab"));
        assert!(constant_time_eq(b"", b""));
        assert!(!constant_time_eq(b"a", b"b"));
    }

    #[test]
    fn malformed_hash_is_rejected() {
        assert!(!verify_password("x", "$argon2id$not-a-valid-hash"));
    }
}
