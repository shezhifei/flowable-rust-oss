//! Task UI contract tests (stream B).

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::identity::entities::User;
use flowable_ui_rest::task::{
    create_rest_variable, rest_variable_value, router_with_engine, RestVariable, RestVariableScope,
};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use std::sync::Arc;
use tower::ServiceExt;

async fn body_json(res: axum::response::Response) -> Value {
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap_or(Value::Null)
}

fn test_engine() -> Arc<ProcessEngine> {
    let engine = Arc::new(ProcessEngine::new("ui-task-test".into()));
    engine.get_identity_service().save_user(User {
        id: "admin".into(),
        first_name: Some("Test".into()),
        last_name: Some("Admin".into()),
        email: Some("admin@example.com".into()),
        password: Some("test".into()),
        tenant_id: None,
    });
    engine
}

#[tokio::test]
async fn task_health_probe() {
    let app = router_with_engine(test_engine());
    let res = app
        .oneshot(
            Request::builder()
                .uri("/app/rest/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let v = body_json(res).await;
    assert_eq!(v["app"], "task");
    assert_eq!(v["engine"], true);
}

#[test]
fn rest_variable_types_cover_converters() {
    let cases = vec![
        ("s", json!("hi"), "string"),
        ("i", json!(7), "integer"),
        ("b", json!(false), "boolean"),
        ("d", json!(1.5), "double"),
    ];
    for (name, val, ty) in cases {
        let rv = create_rest_variable(name, Some(val.clone()), Some(RestVariableScope::Global), true);
        assert_eq!(rv.r#type.as_deref(), Some(ty), "name={name}");
        assert_eq!(rv.scope.as_deref(), Some("global"));
        let back = rest_variable_value(&rv).unwrap().unwrap();
        match ty {
            "double" => assert!((back.as_f64().unwrap() - 1.5).abs() < f64::EPSILON),
            _ => assert_eq!(back, val),
        }
    }
}

#[test]
fn rest_variable_serde_matches_java_field_names() {
    let rv = RestVariable {
        name: "amount".into(),
        r#type: Some("integer".into()),
        value: Some(json!(10)),
        scope: Some("local".into()),
        value_url: None,
    };
    let s = serde_json::to_value(&rv).unwrap();
    assert_eq!(s["name"], "amount");
    assert_eq!(s["type"], "integer");
    assert_eq!(s["value"], 10);
    assert_eq!(s["scope"], "local");
    assert!(s.get("valueUrl").is_none());
}

#[tokio::test]
async fn create_list_claim_complete_task_flow() {
    let engine = test_engine();
    let app = router_with_engine(Arc::clone(&engine));

    // Create standalone task
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/app/rest/tasks")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&json!({
                        "name": "Review invoice",
                        "description": "check totals"
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let created = body_json(res).await;
    let task_id = created["id"].as_str().unwrap().to_string();
    assert_eq!(created["name"], "Review invoice");
    assert_eq!(created["assignee"]["id"], "admin");

    // Query open tasks
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/app/rest/query/tasks")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&json!({ "assignment": "assignee", "size": 25 })).unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let list = body_json(res).await;
    assert!(list["total"].as_i64().unwrap() >= 1);
    assert!(list["data"].as_array().unwrap().iter().any(|t| t["id"] == task_id));

    // Comment before complete
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/app/rest/tasks/{task_id}/comments"))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&json!({ "message": "looks good" })).unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    // Claim (already assignee, should still succeed)
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(format!("/app/rest/tasks/{task_id}/action/claim"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    // Complete
    let res = app
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(format!("/app/rest/tasks/{task_id}/action/complete"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
}

#[tokio::test]
async fn workflow_users_lists_identity() {
    let app = router_with_engine(test_engine());
    let res = app
        .oneshot(
            Request::builder()
                .uri("/app/rest/workflow-users")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let v = body_json(res).await;
    assert!(v["data"].as_array().unwrap().iter().any(|u| u["id"] == "admin"));
}

#[tokio::test]
async fn content_create_and_list_for_task() {
    let app = router_with_engine(test_engine());
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/app/rest/tasks")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&json!({ "name": "with content" })).unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let task_id = body_json(res).await["id"].as_str().unwrap().to_string();

    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/app/rest/tasks/{task_id}/content"))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&json!({
                        "name": "note.txt",
                        "content": "hello",
                        "mimeType": "text/plain"
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let created = body_json(res).await;
    assert_eq!(created["name"], "note.txt");

    let res = app
        .oneshot(
            Request::builder()
                .uri(format!("/app/rest/tasks/{task_id}/content"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let list = body_json(res).await;
    assert!(list["data"].as_array().unwrap().iter().any(|c| c["name"] == "note.txt"));
}

#[tokio::test]
async fn debugger_gate_and_allowed_flag() {
    let app = router_with_engine(test_engine());
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/app/rest/debugger")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    // default false
    let v = body_json(res).await;
    assert_eq!(v, json!(false));

    let res = app
        .oneshot(
            Request::builder()
                .uri("/app/rest/debugger/breakpoints")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn assign_task_returns_representation() {
    let app = router_with_engine(test_engine());
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/app/rest/tasks")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&json!({ "name": "Assign me" })).unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let task_id = body_json(res).await["id"].as_str().unwrap().to_string();

    let res = app
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(format!("/app/rest/tasks/{task_id}/action/assign"))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&json!({ "assignee": "admin" })).unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let v = body_json(res).await;
    assert_eq!(v["assignee"]["id"], "admin");
}
