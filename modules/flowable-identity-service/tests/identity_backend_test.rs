// Tests opt out of the workspace `clippy::unwrap_used` ratchet on purpose: here
// `unwrap()` is the correct tool, because a failing assertion or a missing fixture
// should abort loudly rather than be papered over. Production code under `src/` is
// held to the lint; see the root Cargo.toml `[workspace.lints]` table.
#![allow(clippy::unwrap_used)]

use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::engine::query::Query;
use flowable_engine::identity::entities::{Group, User};
use flowable_identity_service::FlowableIdentityService;
use std::sync::Arc;

#[test]
fn test_identity_persistence_and_auth() {
    let engine = Arc::new(ProcessEngine::new("identity-test".to_string()).unwrap());
    let identity_facade = FlowableIdentityService::new(Arc::clone(&engine));
    let engine_identity = engine.get_identity_service();

    // 1. Create User
    let user = User {
        id: "kermit".to_string(),
        first_name: Some("Kermit".to_string()),
        last_name: Some("The Frog".to_string()),
        email: Some("kermit@muppets.com".to_string()),
        password: Some("thegreen".to_string()),
        tenant_id: None,
    };
    identity_facade.save_user(user).unwrap();

    // 2. Test Auth
    assert!(
        identity_facade
            .authenticate_password("kermit", "thegreen")
            .unwrap()
    );
    assert!(
        !identity_facade
            .authenticate_password("kermit", "wrong")
            .unwrap()
    );
    assert!(
        !identity_facade
            .authenticate_password("nonexistent", "any")
            .unwrap()
    );

    // 3. Create Group and Membership
    let group = Group {
        id: "muppets".to_string(),
        name: "The Muppets".to_string(),
        group_type: Some("assignment".to_string()),
    };
    identity_facade.save_group(group).unwrap();
    identity_facade
        .create_membership("kermit".to_string(), "muppets".to_string())
        .unwrap();

    // 4. Verify Membership
    let groups = engine_identity.get_groups_by_user("kermit").unwrap();
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].id, "muppets");

    let users = identity_facade.get_users_by_group("muppets").unwrap();
    assert_eq!(users.len(), 1);
    assert_eq!(users[0].id, "kermit");
    assert!(
        identity_facade
            .membership_exists("kermit", "muppets")
            .unwrap()
    );

    let queried_users = identity_facade
        .create_user_query()
        .member_of_group_id("muppets".to_string())
        .list()
        .unwrap();
    assert_eq!(queried_users.len(), 1);
    assert_eq!(queried_users[0].id, "kermit");

    identity_facade
        .delete_membership("kermit", "muppets")
        .unwrap();
    assert!(
        !identity_facade
            .membership_exists("kermit", "muppets")
            .unwrap()
    );
    assert!(
        identity_facade
            .get_users_by_group("muppets")
            .unwrap()
            .is_empty()
    );
}
