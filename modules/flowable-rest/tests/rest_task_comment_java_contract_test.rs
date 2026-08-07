//! Java-parity REST contract tests for task comments/events (P2-COMMENT).
//!
//! Case map:
//!   1 empty / whitespace message accepted (201)
//!   2 missing / null message rejected (400)
//!   3 full comment message retained; event message normalized ≤163
//!   4 comments/events readable after task completion (historic task)
//!   5 list order newest → oldest
//!   6 author is basic-auth user
//!   7 missing task/comment 404 classification

use flowable_engine::cmd::create_task_comment_cmd::normalize_comment_event_message;
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_rest::run_server;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::time::{Duration, sleep};

async fn start_test_server(test_name: &str) -> (reqwest::Client, String, Arc<ProcessEngine>) {
    let engine = Arc::new(ProcessEngine::new(test_name.to_string()));
    engine
        .get_identity_service()
        .save_user(flowable_engine::identity::entities::User {
            id: "admin".to_string(),
            first_name: None,
            last_name: None,
            email: None,
            password: Some("test".to_string()),
            tenant_id: None,
        });

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let base_url = format!("http://{}", addr);
    let engine_clone = Arc::clone(&engine);
    tokio::spawn(async move {
        run_server(engine_clone, listener).await.unwrap();
    });

    (reqwest::Client::new(), base_url, engine)
}

async fn deploy_and_start_user_task(
    client: &reqwest::Client,
    base_url: &str,
    engine: &ProcessEngine,
    process_key: &str,
) -> String {
    let xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="{process_key}" name="Comment Contract Process" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="flow1" sourceRef="start" targetRef="reviewTask" />
            <userTask id="reviewTask" name="Review" />
            <sequenceFlow id="flow2" sourceRef="reviewTask" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#
    );

    let deploy_response = client
        .post(format!("{base_url}/repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "Comment Contract Deployment",
            "resourceName": format!("{process_key}.bpmn20.xml"),
            "resource": xml
        }))
        .send()
        .await
        .unwrap();
    assert!(
        deploy_response.status().is_success(),
        "deploy failed: {}",
        deploy_response.status()
    );

    let process_definition_id = engine
        .get_repository_service()
        .latest_process_definition_by_key(process_key, None)
        .unwrap()
        .unwrap()
        .id;

    let start_response = client
        .post(format!("{base_url}/runtime/process-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "processDefinitionId": process_definition_id }))
        .send()
        .await
        .unwrap();
    assert!(start_response.status().is_success());

    let task_response = client
        .get(format!("{base_url}/runtime/tasks"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(task_response.status().is_success());
    let task_body: Value = task_response.json().await.unwrap();
    task_body["data"][0]["id"].as_str().unwrap().to_string()
}

#[tokio::test]
async fn empty_and_whitespace_messages_are_accepted() {
    let (client, base_url, engine) = start_test_server("rest-comment-empty-ws").await;
    let task_id =
        deploy_and_start_user_task(&client, &base_url, &engine, "restCommentEmptyWs").await;

    let empty = client
        .post(format!("{base_url}/runtime/tasks/{task_id}/comments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "message": "" }))
        .send()
        .await
        .unwrap();
    assert_eq!(empty.status(), reqwest::StatusCode::CREATED);
    assert_eq!(empty.json::<Value>().await.unwrap()["message"], "");

    let whitespace = client
        .post(format!("{base_url}/runtime/tasks/{task_id}/comments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "message": "   \t  " }))
        .send()
        .await
        .unwrap();
    assert_eq!(whitespace.status(), reqwest::StatusCode::CREATED);
    assert_eq!(
        whitespace.json::<Value>().await.unwrap()["message"],
        "   \t  "
    );
}

#[tokio::test]
async fn missing_and_null_message_are_rejected() {
    let (client, base_url, engine) = start_test_server("rest-comment-null-msg").await;
    let task_id =
        deploy_and_start_user_task(&client, &base_url, &engine, "restCommentNullMsg").await;

    let missing = client
        .post(format!("{base_url}/runtime/tasks/{task_id}/comments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(missing.status(), reqwest::StatusCode::BAD_REQUEST);

    let null_msg = client
        .post(format!("{base_url}/runtime/tasks/{task_id}/comments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "message": null }))
        .send()
        .await
        .unwrap();
    assert_eq!(null_msg.status(), reqwest::StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn comment_keeps_full_message_event_is_normalized() {
    let (client, base_url, engine) = start_test_server("rest-comment-normalize").await;
    let task_id =
        deploy_and_start_user_task(&client, &base_url, &engine, "restCommentNormalize").await;

    let raw = "Please   review\nthis   carefully";
    let create = client
        .post(format!("{base_url}/runtime/tasks/{task_id}/comments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "message": raw }))
        .send()
        .await
        .unwrap();
    assert_eq!(create.status(), reqwest::StatusCode::CREATED);
    let create_body = create.json::<Value>().await.unwrap();
    assert_eq!(create_body["message"], raw);
    assert_eq!(create_body["author"], "admin");

    let comments = client
        .get(format!("{base_url}/runtime/tasks/{task_id}/comments"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(comments[0]["message"], raw);

    let events = client
        .get(format!("{base_url}/runtime/tasks/{task_id}/events"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    let add_comment = events
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["action"] == "AddComment")
        .expect("AddComment event");
    assert_eq!(
        add_comment["message"][0],
        normalize_comment_event_message(raw)
    );
    assert_eq!(add_comment["userId"], "admin");

    // Long message truncation on event only.
    let long: String = "q".repeat(200);
    let create_long = client
        .post(format!("{base_url}/runtime/tasks/{task_id}/comments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "message": long }))
        .send()
        .await
        .unwrap();
    assert_eq!(create_long.status(), reqwest::StatusCode::CREATED);
    assert_eq!(create_long.json::<Value>().await.unwrap()["message"], long);

    let events_after = client
        .get(format!("{base_url}/runtime/tasks/{task_id}/events"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    let long_event = events_after
        .as_array()
        .unwrap()
        .iter()
        .find(|e| {
            e["action"] == "AddComment"
                && e["message"][0]
                    .as_str()
                    .map(|m| m.ends_with("..."))
                    .unwrap_or(false)
        })
        .expect("truncated AddComment event");
    assert_eq!(
        long_event["message"][0].as_str().unwrap().len(),
        163
    );
}

#[tokio::test]
async fn comments_and_events_readable_after_task_completion() {
    let (client, base_url, engine) = start_test_server("rest-comment-after-complete").await;
    let task_id =
        deploy_and_start_user_task(&client, &base_url, &engine, "restCommentAfterComplete").await;

    let create = client
        .post(format!("{base_url}/runtime/tasks/{task_id}/comments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "message": "keep after complete" }))
        .send()
        .await
        .unwrap();
    assert_eq!(create.status(), reqwest::StatusCode::CREATED);
    let comment_id = create.json::<Value>().await.unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();

    let complete = client
        .post(format!("{base_url}/runtime/tasks/{task_id}"))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "action": "complete" }))
        .send()
        .await
        .unwrap();
    assert!(
        complete.status().is_success() || complete.status() == reqwest::StatusCode::OK,
        "complete status {}",
        complete.status()
    );

    // Runtime task gone.
    let runtime = client
        .get(format!("{base_url}/runtime/tasks/{task_id}"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(runtime.status(), reqwest::StatusCode::NOT_FOUND);

    // Comments/events still readable via historic task.
    let comments = client
        .get(format!("{base_url}/runtime/tasks/{task_id}/comments"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(comments.status(), reqwest::StatusCode::OK);
    let comments_body = comments.json::<Value>().await.unwrap();
    assert_eq!(comments_body.as_array().unwrap().len(), 1);
    assert_eq!(comments_body[0]["id"], comment_id);
    assert_eq!(comments_body[0]["message"], "keep after complete");
    assert_eq!(comments_body[0]["author"], "admin");

    let single = client
        .get(format!(
            "{base_url}/runtime/tasks/{task_id}/comments/{comment_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(single.status(), reqwest::StatusCode::OK);

    let events = client
        .get(format!("{base_url}/runtime/tasks/{task_id}/events"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(events.status(), reqwest::StatusCode::OK);
    let events_body = events.json::<Value>().await.unwrap();
    assert!(
        events_body
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["action"] == "AddComment")
    );
}

#[tokio::test]
async fn comments_list_is_newest_first() {
    let (client, base_url, engine) = start_test_server("rest-comment-order").await;
    let task_id =
        deploy_and_start_user_task(&client, &base_url, &engine, "restCommentOrder").await;

    for msg in ["first", "second", "third"] {
        let resp = client
            .post(format!("{base_url}/runtime/tasks/{task_id}/comments"))
            .basic_auth("admin", Some("test"))
            .json(&json!({ "message": msg }))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), reqwest::StatusCode::CREATED);
        sleep(Duration::from_millis(5)).await;
    }

    let comments = client
        .get(format!("{base_url}/runtime/tasks/{task_id}/comments"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    let messages: Vec<&str> = comments
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["message"].as_str().unwrap())
        .collect();
    assert_eq!(messages, vec!["third", "second", "first"]);

    let events = client
        .get(format!("{base_url}/runtime/tasks/{task_id}/events"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    let add_messages: Vec<&str> = events
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["action"] == "AddComment")
        .map(|e| e["message"][0].as_str().unwrap())
        .collect();
    assert_eq!(add_messages, vec!["third", "second", "first"]);
}

#[tokio::test]
async fn missing_task_and_comment_return_not_found() {
    let (client, base_url, engine) = start_test_server("rest-comment-404").await;
    let task_id = deploy_and_start_user_task(&client, &base_url, &engine, "restComment404").await;

    let missing_task = client
        .get(format!("{base_url}/runtime/tasks/no-such-task/comments"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(missing_task.status(), reqwest::StatusCode::NOT_FOUND);

    let missing_comment = client
        .get(format!(
            "{base_url}/runtime/tasks/{task_id}/comments/no-such-comment"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(missing_comment.status(), reqwest::StatusCode::NOT_FOUND);

    let create_missing_task = client
        .post(format!("{base_url}/runtime/tasks/no-such-task/comments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "message": "x" }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        create_missing_task.status(),
        reqwest::StatusCode::NOT_FOUND
    );
}

// ── P65-comment-type ────────────────────────────────────────────────────────

#[tokio::test]
async fn rest_create_defaults_type_to_comment() {
    let (client, base_url, engine) = start_test_server("rest-comment-default-type").await;
    let task_id =
        deploy_and_start_user_task(&client, &base_url, &engine, "restCommentDefaultType").await;

    // Java REST ignores body `type` and always creates TYPE_COMMENT.
    let create = client
        .post(format!("{base_url}/runtime/tasks/{task_id}/comments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "message": "hello", "type": "ignored-by-java-rest" }))
        .send()
        .await
        .unwrap();
    assert_eq!(create.status(), reqwest::StatusCode::CREATED);
    let body = create.json::<Value>().await.unwrap();
    assert_eq!(body["message"], "hello");
    assert_eq!(body["type"], "comment");

    let list = client
        .get(format!("{base_url}/runtime/tasks/{task_id}/comments"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(list.as_array().unwrap().len(), 1);
    assert_eq!(list[0]["type"], "comment");
}

#[tokio::test]
async fn rest_surfaces_engine_typed_comments_without_conflating_events() {
    let (client, base_url, engine) = start_test_server("rest-comment-engine-type").await;
    let task_id =
        deploy_and_start_user_task(&client, &base_url, &engine, "restCommentEngineType").await;

    // Typed creation is an engine-service contract (Java REST create does not
    // pass type through). Default REST list only returns TYPE_COMMENT.
    engine
        .get_history_service()
        .create_task_comment_with_type(&task_id, None, "audit", "typed via engine", Some("admin"))
        .unwrap();
    engine
        .get_history_service()
        .create_task_comment(&task_id, None, "plain via engine", Some("admin"))
        .unwrap();

    let list = client
        .get(format!("{base_url}/runtime/tasks/{task_id}/comments"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    let items = list.as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["message"], "plain via engine");
    assert_eq!(items[0]["type"], "comment");

    // Events endpoint remains independent of comment type.
    let events = client
        .get(format!("{base_url}/runtime/tasks/{task_id}/events"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    let add_count = events
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["action"] == "AddComment")
        .count();
    assert_eq!(add_count, 2);
}
