//! Task UI contract tests (stream B) — RestVariable + health for B0/B2 start.

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use flowable_ui_rest::task::{
    create_rest_variable, rest_variable_value, router, RestVariable, RestVariableScope,
};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tower::ServiceExt;

async fn body_json(res: axum::response::Response) -> Value {
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap_or(Value::Null)
}

#[tokio::test]
async fn task_health_probe() {
    let app = router();
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
