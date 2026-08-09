use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::engine::query::Query;
use flowable_engine::identity::entities::{Token, User};
use flowable_identity_service::FlowableIdentityService;
use std::sync::Arc;

#[test]
fn token_query_filters_by_user_and_token_value_and_supports_delete() {
    let engine = Arc::new(ProcessEngine::new("idm-token-query".to_string()));
    let identity_facade = FlowableIdentityService::new(Arc::clone(&engine));

    identity_facade.save_user(User {
        id: "kermit".to_string(),
        first_name: Some("Kermit".to_string()),
        last_name: None,
        email: Some("kermit@muppets.test".to_string()),
        password: Some("thegreen".to_string()),
        tenant_id: None,
    });
    identity_facade.save_user(User {
        id: "gonzo".to_string(),
        first_name: Some("Gonzo".to_string()),
        last_name: None,
        email: Some("gonzo@muppets.test".to_string()),
        password: Some("whatever".to_string()),
        tenant_id: None,
    });

    identity_facade.save_token(Token {
        id: "token-1".to_string(),
        token_value: "alpha-token".to_string(),
        user_id: Some("kermit".to_string()),
        token_date: None,
        ip_address: None,
        user_agent: None,
    });
    identity_facade.save_token(Token {
        id: "token-2".to_string(),
        token_value: "beta-token".to_string(),
        user_id: Some("gonzo".to_string()),
        token_date: None,
        ip_address: None,
        user_agent: None,
    });

    let kermit_tokens = identity_facade
        .create_token_query()
        .user_id("kermit".to_string())
        .list()
        .unwrap();
    assert_eq!(kermit_tokens.len(), 1);
    assert_eq!(kermit_tokens[0].id, "token-1");
    assert_eq!(kermit_tokens[0].token_value, "alpha-token");

    let token_by_value = identity_facade
        .create_token_query()
        .token_value("beta-token".to_string())
        .single_result()
        .unwrap()
        .unwrap();
    assert_eq!(token_by_value.id, "token-2");
    assert_eq!(token_by_value.user_id.as_deref(), Some("gonzo"));

    identity_facade.delete_token("token-1");
    assert!(identity_facade.find_token_by_id("token-1").is_none());

    let remaining = identity_facade.create_token_query().list().unwrap();
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].id, "token-2");
}
