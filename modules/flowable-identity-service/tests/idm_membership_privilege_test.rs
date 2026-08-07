use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::identity::entities::{Group, Privilege, User};
use flowable_identity_service::FlowableIdentityService;
use std::sync::Arc;

fn setup() -> (Arc<ProcessEngine>, FlowableIdentityService) {
    let engine = Arc::new(ProcessEngine::new("idm-membership-privilege".to_string()));
    let facade = FlowableIdentityService::new(Arc::clone(&engine));
    (engine, facade)
}

#[test]
fn membership_create_delete_and_query_lifecycle() {
    let (_, facade) = setup();

    facade.save_user(User {
        id: "kermit".to_string(),
        first_name: Some("Kermit".to_string()),
        last_name: None,
        email: None,
        password: None,
        tenant_id: None,
    });
    facade.save_user(User {
        id: "fozzie".to_string(),
        first_name: Some("Fozzie".to_string()),
        last_name: None,
        email: None,
        password: None,
        tenant_id: None,
    });
    facade.save_group(Group {
        id: "muppets".to_string(),
        name: "Muppets".to_string(),
        group_type: None,
    });

    facade.create_membership("kermit".to_string(), "muppets".to_string());
    facade.create_membership("fozzie".to_string(), "muppets".to_string());

    assert!(facade.membership_exists("kermit", "muppets"));
    assert!(facade.membership_exists("fozzie", "muppets"));

    let groups_for_kermit = facade.get_groups_by_user("kermit");
    assert_eq!(groups_for_kermit.len(), 1);
    assert_eq!(groups_for_kermit[0].id, "muppets");

    let users_in_muppets = facade.get_users_by_group("muppets");
    assert_eq!(users_in_muppets.len(), 2);

    facade.delete_membership("kermit", "muppets");
    assert!(!facade.membership_exists("kermit", "muppets"));
    assert!(facade.membership_exists("fozzie", "muppets"));

    let users_after_delete = facade.get_users_by_group("muppets");
    assert_eq!(users_after_delete.len(), 1);
    assert_eq!(users_after_delete[0].id, "fozzie");
}

#[test]
fn privilege_crud_and_user_group_mappings() {
    let (_, facade) = setup();

    let admin_priv = Privilege {
        id: "admin-priv".to_string(),
        name: "Administrator".to_string(),
    };
    let read_priv = Privilege {
        id: "read-priv".to_string(),
        name: "Read Access".to_string(),
    };
    facade.save_privilege(admin_priv);
    facade.save_privilege(read_priv);

    assert!(facade.find_privilege_by_id("admin-priv").is_some());
    assert_eq!(
        facade.find_privilege_by_id("admin-priv").unwrap().name,
        "Administrator"
    );
    assert!(facade.find_privilege_by_id("read-priv").is_some());

    facade.add_user_privilege_mapping("admin-priv".to_string(), "kermit".to_string());
    facade.add_group_privilege_mapping("read-priv".to_string(), "muppets".to_string());

    let kermit_privs = facade.get_privileges_for_user("kermit");
    assert_eq!(kermit_privs.len(), 1);
    assert_eq!(kermit_privs[0].id, "admin-priv");

    let group_privs = facade.get_privileges_for_group("muppets");
    assert_eq!(group_privs.len(), 1);
    assert_eq!(group_privs[0].id, "read-priv");

    facade.delete_user_privilege_mapping("admin-priv", "kermit");
    let kermit_after = facade.get_privileges_for_user("kermit");
    assert!(kermit_after.is_empty());

    facade.delete_group_privilege_mapping("read-priv", "muppets");
    let group_after = facade.get_privileges_for_group("muppets");
    assert!(group_after.is_empty());

    facade.delete_privilege("admin-priv");
    assert!(facade.find_privilege_by_id("admin-priv").is_none());
}

#[test]
fn user_privileges_include_inherited_group_privileges() {
    let (_, facade) = setup();

    facade.save_user(User {
        id: "kermit".to_string(),
        first_name: None,
        last_name: None,
        email: None,
        password: None,
        tenant_id: None,
    });
    facade.save_group(Group {
        id: "admins".to_string(),
        name: "Admins".to_string(),
        group_type: None,
    });
    facade.create_membership("kermit".to_string(), "admins".to_string());

    facade.save_privilege(Privilege {
        id: "direct-priv".to_string(),
        name: "Direct".to_string(),
    });
    facade.save_privilege(Privilege {
        id: "group-priv".to_string(),
        name: "Group".to_string(),
    });

    facade.add_user_privilege_mapping("direct-priv".to_string(), "kermit".to_string());
    facade.add_group_privilege_mapping("group-priv".to_string(), "admins".to_string());

    let all_privs = facade.get_privileges_for_user("kermit");
    assert_eq!(all_privs.len(), 2);
    let priv_ids: Vec<&str> = all_privs.iter().map(|p| p.id.as_str()).collect();
    assert!(priv_ids.contains(&"direct-priv"));
    assert!(priv_ids.contains(&"group-priv"));
}
