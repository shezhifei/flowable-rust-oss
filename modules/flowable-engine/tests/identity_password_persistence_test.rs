//! Password handling across the save path of `IdentityService`.
//!
//! Passwords are argon2id-hashed on write (a deviation from Java, which stores
//! plaintext), which makes the save path subtle in two directions: a plaintext
//! value must be hashed exactly once, and an already-hashed value must survive a
//! re-save untouched.

use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::identity::entities::User;

fn user(id: &str, password: Option<&str>) -> User {
    User {
        id: id.to_string(),
        first_name: Some("First".to_string()),
        last_name: None,
        email: None,
        password: password.map(str::to_string),
        tenant_id: None,
    }
}

#[test]
fn plaintext_password_is_hashed_on_save() {
    let engine = ProcessEngine::new("password_hashed_on_save".to_string());
    let identity = engine.get_identity_service();

    identity.save_user(user("bob", Some("hunter2")));

    let stored = identity.find_user_by_id("bob").unwrap();
    assert_ne!(
        stored.password.as_deref(),
        Some("hunter2"),
        "the plaintext must not reach the store"
    );
    assert!(stored.password.unwrap().starts_with("$argon2id$"));
    assert!(identity.check_password("bob", "hunter2"));
}

/// Regression: `save_user_in_session` used to `take()` the password before
/// testing whether it needed hashing, so the already-hashed branch left the
/// field empty. Every update that re-saved a loaded user — changing a display
/// name, an email, a tenant — wiped that user's password and locked them out.
#[test]
fn resaving_a_loaded_user_preserves_the_password() {
    let engine = ProcessEngine::new("password_survives_resave".to_string());
    let identity = engine.get_identity_service();

    identity.save_user(user("bob", Some("hunter2")));
    let loaded = identity.find_user_by_id("bob").unwrap();
    let hash_before = loaded.password.clone().expect("password was stored");

    // An unrelated field changes; the password rides along as its stored hash.
    identity.save_user(User {
        first_name: Some("Robert".to_string()),
        ..loaded
    });

    let after = identity.find_user_by_id("bob").unwrap();
    assert_eq!(after.first_name.as_deref(), Some("Robert"));
    assert_eq!(
        after.password.as_deref(),
        Some(hash_before.as_str()),
        "the stored hash must be written back byte for byte"
    );
    assert!(
        identity.check_password("bob", "hunter2"),
        "the user must still be able to log in after an unrelated update"
    );
}

#[test]
fn a_hash_is_not_rehashed_on_resave() {
    let engine = ProcessEngine::new("password_no_double_hash".to_string());
    let identity = engine.get_identity_service();

    identity.save_user(user("bob", Some("hunter2")));
    let first = identity.find_user_by_id("bob").unwrap().password.unwrap();

    // Re-save the loaded entity three times over.
    for _ in 0..3 {
        let loaded = identity.find_user_by_id("bob").unwrap();
        identity.save_user(loaded);
    }

    let last = identity.find_user_by_id("bob").unwrap().password.unwrap();
    assert_eq!(first, last, "re-hashing would change the digest each time");
    assert!(identity.check_password("bob", "hunter2"));
}

#[test]
fn setting_a_new_plaintext_password_replaces_the_hash() {
    let engine = ProcessEngine::new("password_replace".to_string());
    let identity = engine.get_identity_service();

    identity.save_user(user("bob", Some("hunter2")));
    let loaded = identity.find_user_by_id("bob").unwrap();

    identity.save_user(User {
        password: Some("newsecret".to_string()),
        ..loaded
    });

    assert!(identity.check_password("bob", "newsecret"));
    assert!(!identity.check_password("bob", "hunter2"));
}

/// A chosen password that merely looks like a PHC string must still be hashed,
/// or it would land in the store as plaintext and never verify again.
#[test]
fn a_password_shaped_like_a_hash_is_still_hashed() {
    let engine = ProcessEngine::new("password_hash_shaped".to_string());
    let identity = engine.get_identity_service();

    let chosen = "$argon2id$hunter2";
    identity.save_user(user("bob", Some(chosen)));

    let stored = identity.find_user_by_id("bob").unwrap().password.unwrap();
    assert_ne!(stored, chosen, "a malformed lookalike must not be stored as-is");
    assert!(identity.check_password("bob", chosen));
}

#[test]
fn a_user_without_a_password_stays_without_one() {
    let engine = ProcessEngine::new("password_absent".to_string());
    let identity = engine.get_identity_service();

    identity.save_user(user("bob", None));

    let stored = identity.find_user_by_id("bob").unwrap();
    assert_eq!(stored.password, None);
    // No password means no successful authentication, including with an empty one.
    assert!(!identity.check_password("bob", ""));
}
