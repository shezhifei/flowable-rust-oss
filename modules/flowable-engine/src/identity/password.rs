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

/// True when `stored` *claims* to be an argon2id hash, by prefix alone.
///
/// Deliberately a prefix test, not a parse: this drives the read path in
/// `verify_password`, where claiming-but-malformed must route to argon2
/// verification (and fail) rather than to the plaintext comparison. Treating
/// `$argon2id$<garbage>` as plaintext would let anyone who knows that literal
/// string authenticate as the user.
pub fn is_hash(stored: &str) -> bool {
    stored.starts_with(HASH_PREFIX)
}

/// True when `stored` is a *well-formed* argon2id hash — prefix plus a
/// successful PHC parse.
///
/// This drives the write path, where the question is the opposite one: "may I
/// skip hashing this value?". Skipping on prefix alone is fail-open — a user
/// whose chosen password happens to start with `$argon2id$` would be written to
/// the database verbatim, i.e. stored in plaintext *and* unverifiable
/// afterwards (`verify_password` sees the prefix, fails to parse, and rejects
/// every attempt), locking that user out permanently.
/// A bare `PasswordHash::new` is not enough: PHC parsing accepts a string with
/// no digest at all (`$argon2id$whatever` parses fine, with `hash: None`), and
/// such a value can never verify. Require both a salt and a digest, which is
/// exactly what `verify_password` needs to succeed later.
pub fn is_valid_hash(stored: &str) -> bool {
    is_hash(stored)
        && PasswordHash::new(stored)
            .is_ok_and(|parsed| parsed.hash.is_some() && parsed.salt.is_some())
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
    fn is_valid_hash_accepts_only_parseable_hashes() {
        assert!(is_valid_hash(&hash_password("x")));
        // Claims the prefix but is not a PHC string: must not be mistaken for
        // an already-hashed value on the write path.
        assert!(!is_valid_hash("$argon2id$not-a-valid-hash"));
        assert!(!is_valid_hash("$argon2id$"));
        assert!(!is_valid_hash("plaintext"));
    }

    #[test]
    fn a_password_shaped_like_a_hash_is_still_hashed() {
        // Regression: with a prefix-only write guard this value was stored
        // verbatim — plaintext in the database, and unverifiable afterwards,
        // locking the user out. It must be treated as a plaintext password.
        let chosen = "$argon2id$hunter2";
        assert!(is_hash(chosen), "it does claim the prefix");
        assert!(!is_valid_hash(chosen), "but it is not a real hash");

        // What the write path does with it, and that login still works.
        let stored = hash_password(chosen);
        assert!(is_valid_hash(&stored));
        assert!(verify_password(chosen, &stored));
        assert!(!verify_password("hunter2", &stored));
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
