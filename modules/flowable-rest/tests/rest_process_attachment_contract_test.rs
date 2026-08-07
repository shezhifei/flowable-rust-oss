//! REST contract for process-instance attachments (P65 Rust extension).
//!
//! Java exposes processInstanceId attachment operations via TaskService only;
//! there is no BPMN REST collection equivalent. These tests cover the owned
//! Rust routes under `/runtime/process-instances/:id/attachments`.

use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_rest::run_server;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::net::TcpListener;

const ATTACHMENT_PROCESS_BPMN: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             targetNamespace="Examples">
    <process id="procAttachmentProcess" name="Process Attachment Process" isExecutable="true">
        <startEvent id="startEvent" />
        <sequenceFlow id="flow1" sourceRef="startEvent" targetRef="reviewTask" />
        <userTask id="reviewTask" name="Review attachment" />
        <sequenceFlow id="flow2" sourceRef="reviewTask" targetRef="endEvent" />
        <endEvent id="endEvent" />
    </process>
</definitions>"#;

async fn spawn_server() -> (Arc<ProcessEngine>, String, reqwest::Client) {
    let engine = Arc::new(ProcessEngine::new("rest-process-attachments".to_string()));
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
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let engine_for_server = Arc::clone(&engine);
    tokio::spawn(async move {
        run_server(engine_for_server, listener).await.unwrap();
    });

    (engine, base_url, reqwest::Client::new())
}

async fn deploy_process(client: &reqwest::Client, base_url: &str) {
    let response = client
        .post(format!("{base_url}/repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "Process attachment process",
            "resourceName": "proc-attachment-process.bpmn20.xml",
            "resource": ATTACHMENT_PROCESS_BPMN
        }))
        .send()
        .await
        .unwrap();
    // P109: deploy (both JSON superset and multipart paths) returns 201.
    assert_eq!(response.status(), reqwest::StatusCode::CREATED);
}

async fn start_process(client: &reqwest::Client, base_url: &str, process_definition_id: &str) -> String {
    let response = client
        .post(format!("{base_url}/runtime/process-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "processDefinitionId": process_definition_id
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let body = response.json::<Value>().await.unwrap();
    body["id"].as_str().unwrap().to_string()
}

#[tokio::test]
async fn process_attachment_routes_create_list_get_content_delete_and_isolate() {
    let (engine, base_url, client) = spawn_server().await;
    deploy_process(&client, &base_url).await;
    let process_definition_id = engine
        .get_repository_service()
        .latest_process_definition_by_key("procAttachmentProcess", None)
        .unwrap()
        .unwrap()
        .id;
    let process_instance_id_a = start_process(&client, &base_url, &process_definition_id).await;
    let process_instance_id_b = start_process(&client, &base_url, &process_definition_id).await;

    let create = client
        .post(format!(
            "{base_url}/runtime/process-instances/{process_instance_id_a}/attachments"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "proc-note.txt",
            "description": "Process note",
            "type": "text/plain",
            "content": "process payload"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(create.status(), reqwest::StatusCode::CREATED);
    let create_body = create.json::<Value>().await.unwrap();
    let attachment_id = create_body["id"].as_str().unwrap().to_string();
    assert_eq!(create_body["name"], "proc-note.txt");
    assert_eq!(create_body["description"], "Process note");
    assert_eq!(create_body["type"], "text/plain");
    assert!(
        create_body["url"]
            .as_str()
            .unwrap()
            .contains(&format!(
                "/runtime/process-instances/{process_instance_id_a}/attachments/"
            ))
    );
    assert!(
        create_body["contentUrl"]
            .as_str()
            .unwrap()
            .ends_with("/content")
    );
    assert!(
        create_body["processInstanceUrl"]
            .as_str()
            .unwrap()
            .ends_with(&process_instance_id_a)
    );
    // Pure process attachment: no task URL.
    assert!(create_body["taskUrl"].is_null());

    let list = client
        .get(format!(
            "{base_url}/runtime/process-instances/{process_instance_id_a}/attachments"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(list.status(), reqwest::StatusCode::OK);
    let list_body = list.json::<Value>().await.unwrap();
    assert_eq!(list_body.as_array().unwrap().len(), 1);
    assert_eq!(list_body[0]["id"], attachment_id);

    let get = client
        .get(format!(
            "{base_url}/runtime/process-instances/{process_instance_id_a}/attachments/{attachment_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(get.status(), reqwest::StatusCode::OK);
    let get_body = get.json::<Value>().await.unwrap();
    assert_eq!(get_body["id"], attachment_id);

    let content = client
        .get(format!(
            "{base_url}/runtime/process-instances/{process_instance_id_a}/attachments/{attachment_id}/content"
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
    assert_eq!(content.bytes().await.unwrap().as_ref(), b"process payload");

    // Cross-process isolation: lookup under B must not leak A's attachment.
    let leak = client
        .get(format!(
            "{base_url}/runtime/process-instances/{process_instance_id_b}/attachments/{attachment_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(leak.status(), reqwest::StatusCode::NOT_FOUND);

    let list_b = client
        .get(format!(
            "{base_url}/runtime/process-instances/{process_instance_id_b}/attachments"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(list_b.status(), reqwest::StatusCode::OK);
    let list_b_body = list_b.json::<Value>().await.unwrap();
    assert!(list_b_body.as_array().unwrap().is_empty());

    let delete = client
        .delete(format!(
            "{base_url}/runtime/process-instances/{process_instance_id_a}/attachments/{attachment_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(delete.status(), reqwest::StatusCode::NO_CONTENT);

    let after_delete = client
        .get(format!(
            "{base_url}/runtime/process-instances/{process_instance_id_a}/attachments/{attachment_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(after_delete.status(), reqwest::StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn process_attachment_missing_process_is_not_found() {
    let (_engine, base_url, client) = spawn_server().await;

    let create = client
        .post(format!(
            "{base_url}/runtime/process-instances/missing-pi/attachments"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "x",
            "externalUrl": "http://example.com"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(create.status(), reqwest::StatusCode::NOT_FOUND);

    let list = client
        .get(format!(
            "{base_url}/runtime/process-instances/missing-pi/attachments"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(list.status(), reqwest::StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn task_attachment_is_visible_under_process_instance_collection() {
    let (engine, base_url, client) = spawn_server().await;
    deploy_process(&client, &base_url).await;
    let process_definition_id = engine
        .get_repository_service()
        .latest_process_definition_by_key("procAttachmentProcess", None)
        .unwrap()
        .unwrap()
        .id;
    let process_instance_id = start_process(&client, &base_url, &process_definition_id).await;

    let tasks = client
        .get(format!("{base_url}/runtime/tasks"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(tasks.status(), reqwest::StatusCode::OK);
    let tasks_body = tasks.json::<Value>().await.unwrap();
    let task_id = tasks_body["data"][0]["id"].as_str().unwrap().to_string();

    let create = client
        .post(format!("{base_url}/runtime/tasks/{task_id}/attachments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "from-task.txt",
            "type": "text/plain",
            "content": "via task"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(create.status(), reqwest::StatusCode::CREATED);
    let create_body = create.json::<Value>().await.unwrap();
    let attachment_id = create_body["id"].as_str().unwrap();

    let list = client
        .get(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}/attachments"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(list.status(), reqwest::StatusCode::OK);
    let list_body = list.json::<Value>().await.unwrap();
    assert!(
        list_body
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["id"] == attachment_id)
    );
}
