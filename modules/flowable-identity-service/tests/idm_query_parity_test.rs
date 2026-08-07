use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::engine::query::Query;
use flowable_engine::identity::entities::{Group, User};
use flowable_identity_service::FlowableIdentityService;
use std::sync::Arc;

fn setup() -> (Arc<ProcessEngine>, FlowableIdentityService) {
    let engine = Arc::new(ProcessEngine::new("idm-query-parity".to_string()));
    let facade = FlowableIdentityService::new(Arc::clone(&engine));
    (engine, facade)
}

#[test]
fn user_query_supports_first_name_last_name_email_and_member_of_group_filters() {
    let (_, facade) = setup();

    facade.save_user(User {
        id: "kermit".to_string(),
        first_name: Some("Kermit".to_string()),
        last_name: Some("Frog".to_string()),
        email: Some("kermit@muppets.test".to_string()),
        password: Some("pass".to_string()),
        tenant_id: None,
    });
    facade.save_user(User {
        id: "fozzie".to_string(),
        first_name: Some("Fozzie".to_string()),
        last_name: Some("Bear".to_string()),
        email: Some("fozzie@muppets.test".to_string()),
        password: Some("pass".to_string()),
        tenant_id: None,
    });
    facade.save_user(User {
        id: "gonzo".to_string(),
        first_name: Some("Gonzo".to_string()),
        last_name: Some("Great".to_string()),
        email: Some("gonzo@muppets.test".to_string()),
        password: Some("pass".to_string()),
        tenant_id: None,
    });

    facade.save_group(Group {
        id: "performers".to_string(),
        name: "Performers".to_string(),
        group_type: Some("assignment".to_string()),
    });
    facade.create_membership("kermit".to_string(), "performers".to_string());
    facade.create_membership("fozzie".to_string(), "performers".to_string());

    let all_users = facade.create_user_query().list().unwrap();
    assert_eq!(all_users.len(), 3);

    let by_first_name = facade
        .create_user_query()
        .first_name("Kermit".to_string())
        .list()
        .unwrap();
    assert_eq!(by_first_name.len(), 1);
    assert_eq!(by_first_name[0].id, "kermit");

    let by_last_name = facade
        .create_user_query()
        .last_name("Bear".to_string())
        .list()
        .unwrap();
    assert_eq!(by_last_name.len(), 1);
    assert_eq!(by_last_name[0].id, "fozzie");

    let by_email = facade
        .create_user_query()
        .email("gonzo@muppets.test".to_string())
        .list()
        .unwrap();
    assert_eq!(by_email.len(), 1);
    assert_eq!(by_email[0].id, "gonzo");

    let by_group = facade
        .create_user_query()
        .member_of_group_id("performers".to_string())
        .list()
        .unwrap();
    assert_eq!(by_group.len(), 2);
    let found_ids: Vec<&str> = by_group.iter().map(|u| u.id.as_str()).collect();
    assert!(found_ids.contains(&"kermit"));
    assert!(found_ids.contains(&"fozzie"));
}

#[test]
fn user_query_supports_ordering_by_first_name_and_last_name() {
    let (_, facade) = setup();

    for (id, first, last) in [
        ("c", "Charlie", "Zebra"),
        ("a", "Alice", "Moose"),
        ("b", "Bob", "Ant"),
    ] {
        facade.save_user(User {
            id: id.to_string(),
            first_name: Some(first.to_string()),
            last_name: Some(last.to_string()),
            email: None,
            password: None,
            tenant_id: None,
        });
    }

    let asc_first = facade
        .create_user_query()
        .order_by_first_name()
        .asc()
        .list()
        .unwrap();
    assert_eq!(asc_first[0].first_name.as_deref(), Some("Alice"));
    assert_eq!(asc_first[2].first_name.as_deref(), Some("Charlie"));

    let desc_last = facade
        .create_user_query()
        .order_by_last_name()
        .desc()
        .list()
        .unwrap();
    assert_eq!(desc_last[0].last_name.as_deref(), Some("Zebra"));
    assert_eq!(desc_last[2].last_name.as_deref(), Some("Ant"));
}

#[test]
fn group_query_supports_name_type_and_member_user_id_filters() {
    let (_, facade) = setup();

    facade.save_group(Group {
        id: "admin".to_string(),
        name: "Admin".to_string(),
        group_type: Some("security-role".to_string()),
    });
    facade.save_group(Group {
        id: "users".to_string(),
        name: "Users".to_string(),
        group_type: Some("assignment".to_string()),
    });
    facade.save_user(User {
        id: "kermit".to_string(),
        first_name: None,
        last_name: None,
        email: None,
        password: None,
        tenant_id: None,
    });
    facade.create_membership("kermit".to_string(), "admin".to_string());

    let by_name = facade
        .create_group_query()
        .name("Admin".to_string())
        .list()
        .unwrap();
    assert_eq!(by_name.len(), 1);
    assert_eq!(by_name[0].id, "admin");

    let by_type = facade
        .create_group_query()
        .group_type("assignment".to_string())
        .list()
        .unwrap();
    assert_eq!(by_type.len(), 1);
    assert_eq!(by_type[0].id, "users");

    let by_member = facade
        .create_group_query()
        .member_user_id("kermit".to_string())
        .list()
        .unwrap();
    assert_eq!(by_member.len(), 1);
    assert_eq!(by_member[0].id, "admin");
}

#[test]
fn group_query_supports_ordering_by_name() {
    let (_, facade) = setup();

    for id in ["gamma", "alpha", "beta"] {
        facade.save_group(Group {
            id: id.to_string(),
            name: id.to_string(),
            group_type: None,
        });
    }

    let asc = facade
        .create_group_query()
        .order_by_name()
        .asc()
        .list()
        .unwrap();
    assert_eq!(asc[0].name, "alpha");
    assert_eq!(asc[2].name, "gamma");

    let desc = facade
        .create_group_query()
        .order_by_name()
        .desc()
        .list()
        .unwrap();
    assert_eq!(desc[0].name, "gamma");
    assert_eq!(desc[2].name, "alpha");
}

#[test]
fn user_query_count_returns_correct_number() {
    let (_, facade) = setup();

    for i in 0..5 {
        facade.save_user(User {
            id: format!("user-{}", i),
            first_name: Some(format!("User{}", i)),
            last_name: None,
            email: None,
            password: None,
            tenant_id: None,
        });
    }

    let count = facade.create_user_query().count().unwrap();
    assert_eq!(count, 5);
}

#[test]
fn group_query_count_returns_correct_number() {
    let (_, facade) = setup();

    for i in 0..3 {
        facade.save_group(Group {
            id: format!("group-{}", i),
            name: format!("Group{}", i),
            group_type: None,
        });
    }

    let count = facade.create_group_query().count().unwrap();
    assert_eq!(count, 3);
}
