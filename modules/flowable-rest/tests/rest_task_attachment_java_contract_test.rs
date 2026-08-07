//! Java-parity REST contract tests for task attachments (P2-ATTACHMENT).
//!
//! Case map:
//!   1 multipart create → 201, fields, event, content download + Content-Type
//!   2 JSON url create → fields + event; missing name → 400
//!   3 mid-create failure → no orphan content / event
//!   4 delete clears attachment + event; missing → 404
//!   5 after complete: GET list/get/content 200; POST/DELETE 404
//!   6 suspended create rejected without side effects

use flowable_content_service::FORCE_FAIL_ATTACHMENT_TYPE;
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::task::Task;
use flowable_rest::run_server;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::net::TcpListener;

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
        <process id="{process_key}" name="Attachment Contract Process" isExecutable="true">
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
            "name": "Attachment Contract Deployment",
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
async fn multipart_create_returns_fields_event_and_downloadable_content() {
    let (client, base_url, engine) = start_test_server("rest-attach-multipart").await;
    let task_id =
        deploy_and_start_user_task(&client, &base_url, &engine, "restAttachMultipart").await;

    let boundary = "----FlowableAttachmentBoundary";
    let body = format!(
        "--{boundary}\r\n\
Content-Disposition: form-data; name=\"name\"\r\n\r\n\
review-note.txt\r\n\
--{boundary}\r\n\
Content-Disposition: form-data; name=\"description\"\r\n\r\n\
Review note\r\n\
--{boundary}\r\n\
Content-Disposition: form-data; name=\"type\"\r\n\r\n\
text/plain\r\n\
--{boundary}\r\n\
Content-Disposition: form-data; name=\"file\"; filename=\"review-note.txt\"\r\n\
Content-Type: text/plain\r\n\r\n\
approved payload\r\n\
--{boundary}--\r\n"
    );

    let create = client
        .post(format!("{base_url}/runtime/tasks/{task_id}/attachments"))
        .basic_auth("admin", Some("test"))
        .header(
            "Content-Type",
            format!("multipart/form-data; boundary={boundary}"),
        )
        .body(body)
        .send()
        .await
        .unwrap();
    assert_eq!(create.status(), reqwest::StatusCode::CREATED);
    let body: Value = create.json().await.unwrap();
    let attachment_id = body["id"].as_str().unwrap();
    assert_eq!(body["name"], "review-note.txt");
    assert_eq!(body["description"], "Review note");
    assert_eq!(body["type"], "text/plain");
    assert!(body["taskUrl"].as_str().unwrap().ends_with(&task_id));
    assert!(
        body["contentUrl"]
            .as_str()
            .unwrap()
            .ends_with("/content")
    );
    assert!(body["externalUrl"].is_null());
    assert_eq!(body["userId"], "admin");
    assert!(body["time"].as_str().is_some());

    let content = client
        .get(format!(
            "{base_url}/runtime/tasks/{task_id}/attachments/{attachment_id}/content"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(content.status(), reqwest::StatusCode::OK);
    assert_eq!(
        content
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .unwrap(),
        "text/plain"
    );
    assert_eq!(content.bytes().await.unwrap().as_ref(), b"approved payload");

    let events = client
        .get(format!("{base_url}/runtime/tasks/{task_id}/events"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    let events_body: Value = events.json().await.unwrap();
    assert_eq!(events_body[0]["action"], "AddAttachment");
    assert_eq!(events_body[0]["message"][0], "review-note.txt");
}

#[tokio::test]
async fn json_url_create_and_missing_name_classification() {
    let (client, base_url, engine) = start_test_server("rest-attach-url").await;
    let task_id = deploy_and_start_user_task(&client, &base_url, &engine, "restAttachUrl").await;

    let create = client
        .post(format!("{base_url}/runtime/tasks/{task_id}/attachments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "Simple attachment",
            "description": "Simple attachment description",
            "type": "simpleType",
            "externalUrl": "http://flowable.org"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(create.status(), reqwest::StatusCode::CREATED);
    let body: Value = create.json().await.unwrap();
    assert_eq!(body["name"], "Simple attachment");
    assert_eq!(body["type"], "simpleType");
    assert_eq!(body["externalUrl"], "http://flowable.org");
    assert!(body["contentUrl"].is_null());

    let events = client
        .get(format!("{base_url}/runtime/tasks/{task_id}/events"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    let events_body: Value = events.json().await.unwrap();
    assert_eq!(events_body[0]["action"], "AddAttachment");
    assert_eq!(events_body[0]["message"][0], "Simple attachment");

    let missing_name = client
        .post(format!("{base_url}/runtime/tasks/{task_id}/attachments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "description": "no name",
            "externalUrl": "http://x"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(missing_name.status(), reqwest::StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn create_mid_failure_leaves_no_orphan_content_or_event() {
    let (client, base_url, engine) = start_test_server("rest-attach-fail").await;
    let task_id = deploy_and_start_user_task(&client, &base_url, &engine, "restAttachFail").await;

    let fail = client
        .post(format!("{base_url}/runtime/tasks/{task_id}/attachments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "will-fail",
            "type": FORCE_FAIL_ATTACHMENT_TYPE,
            "content": "should-not-persist"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(fail.status(), reqwest::StatusCode::BAD_REQUEST);

    let list = client
        .get(format!("{base_url}/runtime/tasks/{task_id}/attachments"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    let list_body: Value = list.json().await.unwrap();
    assert_eq!(list_body.as_array().unwrap().len(), 0);

    let events = client
        .get(format!("{base_url}/runtime/tasks/{task_id}/events"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    let events_body: Value = events.json().await.unwrap();
    assert!(
        !events_body
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["action"] == "AddAttachment")
    );
}

#[tokio::test]
async fn delete_clears_attachment_and_missing_is_404() {
    let (client, base_url, engine) = start_test_server("rest-attach-delete").await;
    let task_id =
        deploy_and_start_user_task(&client, &base_url, &engine, "restAttachDelete").await;

    let create = client
        .post(format!("{base_url}/runtime/tasks/{task_id}/attachments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "to-delete.txt",
            "type": "text/plain",
            "content": "x"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(create.status(), reqwest::StatusCode::CREATED);
    let attachment_id = create.json::<Value>().await.unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();

    let delete = client
        .delete(format!(
            "{base_url}/runtime/tasks/{task_id}/attachments/{attachment_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(delete.status(), reqwest::StatusCode::NO_CONTENT);

    let get = client
        .get(format!(
            "{base_url}/runtime/tasks/{task_id}/attachments/{attachment_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(get.status(), reqwest::StatusCode::NOT_FOUND);

    let events = client
        .get(format!("{base_url}/runtime/tasks/{task_id}/events"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    let events_body: Value = events.json().await.unwrap();
    assert_eq!(events_body[0]["action"], "DeleteAttachment");
    assert_eq!(events_body[0]["message"][0], "to-delete.txt");

    let missing = client
        .delete(format!(
            "{base_url}/runtime/tasks/{task_id}/attachments/does-not-exist"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(missing.status(), reqwest::StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn after_completion_get_ok_post_delete_not_found() {
    let (client, base_url, engine) = start_test_server("rest-attach-complete").await;
    let task_id =
        deploy_and_start_user_task(&client, &base_url, &engine, "restAttachComplete").await;

    let create = client
        .post(format!("{base_url}/runtime/tasks/{task_id}/attachments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "survives.txt",
            "type": "text/plain",
            "content": "keep-me"
        }))
        .send()
        .await
        .unwrap();
    let attachment_id = create.json::<Value>().await.unwrap()["id"]
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
        complete.status().is_success(),
        "complete status {}",
        complete.status()
    );

    let list = client
        .get(format!("{base_url}/runtime/tasks/{task_id}/attachments"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(list.status(), reqwest::StatusCode::OK);
    assert_eq!(list.json::<Value>().await.unwrap().as_array().unwrap().len(), 1);

    let get = client
        .get(format!(
            "{base_url}/runtime/tasks/{task_id}/attachments/{attachment_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(get.status(), reqwest::StatusCode::OK);

    let content = client
        .get(format!(
            "{base_url}/runtime/tasks/{task_id}/attachments/{attachment_id}/content"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(content.status(), reqwest::StatusCode::OK);
    assert_eq!(content.bytes().await.unwrap().as_ref(), b"keep-me");

    let post = client
        .post(format!("{base_url}/runtime/tasks/{task_id}/attachments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "nope",
            "externalUrl": "http://x"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(post.status(), reqwest::StatusCode::NOT_FOUND);

    let delete = client
        .delete(format!(
            "{base_url}/runtime/tasks/{task_id}/attachments/{attachment_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(delete.status(), reqwest::StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn suspended_task_create_rejected_without_side_effects() {
    let (client, base_url, engine) = start_test_server("rest-attach-suspended").await;

    // Insert a suspended standalone runtime task directly.
    {
        let store = engine.get_runtime_store();
        let mut session = store.create_session().unwrap();
        let mut task = Task::new(
            "suspended-task-1".into(),
            String::new(),
            String::new(),
            "standalone".into(),
            "Suspended".into(),
        );
        task.set_suspension_state(true);
        store.insert_task(&task, &mut session);
        // Historic row so list would resolve if create incorrectly succeeded.
        store.insert_historic_task_instance(
            flowable_engine::history::historic_entities::HistoricTaskInstance {
                id: "suspended-task-1".into(),
                process_instance_id: String::new(),
                process_definition_id: Some("standalone".into()),
                execution_id: String::new(),
                task_definition_key: Some("standalone".into()),
                name: Some("Suspended".into()),
                description: None,
                assignee: None,
                owner: None,
                claim_time: None,
                tenant_id: None,
                category: None,
                form_key: None,
                parent_task_id: None,
                priority: None,
                due_date: None,
                start_time: chrono::Utc::now(),
                end_time: None,
                duration_ms: None,
                delete_reason: None,
            },
            &mut session,
        );
        session.flush_and_commit().unwrap();
    }

    let create = client
        .post(format!(
            "{base_url}/runtime/tasks/suspended-task-1/attachments"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "nope",
            "type": "text/plain",
            "content": "x"
        }))
        .send()
        .await
        .unwrap();
    // Suspended → ExecutionError maps to 500 Internal in current ApiError mapping
    // (Java FlowableException is also non-400). Assert not 2xx and no side effects.
    assert!(
        !create.status().is_success(),
        "expected create on suspended task to fail, got {}",
        create.status()
    );

    let list = client
        .get(format!(
            "{base_url}/runtime/tasks/suspended-task-1/attachments"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(list.status(), reqwest::StatusCode::OK);
    assert_eq!(list.json::<Value>().await.unwrap().as_array().unwrap().len(), 0);
}
