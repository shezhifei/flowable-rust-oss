use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_rest::run_server;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::net::TcpListener;

const ATTACHMENT_PROCESS_BPMN: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             targetNamespace="Examples">
    <process id="attachmentProcess" name="Attachment Process" isExecutable="true">
        <startEvent id="startEvent" />
        <sequenceFlow id="flow1" sourceRef="startEvent" targetRef="reviewTask" />
        <userTask id="reviewTask" name="Review attachment" />
        <sequenceFlow id="flow2" sourceRef="reviewTask" targetRef="endEvent" />
        <endEvent id="endEvent" />
    </process>
</definitions>"#;

async fn spawn_server() -> (Arc<ProcessEngine>, String, reqwest::Client) {
    let engine = Arc::new(ProcessEngine::new("rest-task-attachments".to_string()));
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
            "name": "Attachment process",
            "resourceName": "attachment-process.bpmn20.xml",
            "resource": ATTACHMENT_PROCESS_BPMN
        }))
        .send()
        .await
        .unwrap();
    // P109: deploy (both JSON superset and multipart paths) returns 201.
    assert_eq!(response.status(), reqwest::StatusCode::CREATED);
}

async fn start_process(client: &reqwest::Client, base_url: &str, process_definition_id: &str) {
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
}

async fn active_task_id(client: &reqwest::Client, base_url: &str) -> String {
    let response = client
        .get(format!("{base_url}/runtime/tasks"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let body = response.json::<Value>().await.unwrap();
    body["data"][0]["id"].as_str().unwrap().to_string()
}

#[tokio::test]
async fn task_attachment_routes_are_backed_by_content_service_and_task_scope() {
    let (engine, base_url, client) = spawn_server().await;
    deploy_process(&client, &base_url).await;
    let process_definition_id = engine
        .get_repository_service()
        .latest_process_definition_by_key("attachmentProcess", None)
        .unwrap()
        .unwrap()
        .id;
    start_process(&client, &base_url, &process_definition_id).await;
    let task_id = active_task_id(&client, &base_url).await;

    let create = client
        .post(format!("{base_url}/runtime/tasks/{task_id}/attachments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "review-note.txt",
            "description": "Review note",
            "type": "text/plain",
            "content": "approved payload"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(create.status(), reqwest::StatusCode::CREATED);
    let create_body = create.json::<Value>().await.unwrap();
    let attachment_id = create_body["id"].as_str().unwrap();
    assert_eq!(create_body["name"], "review-note.txt");
    assert_eq!(create_body["description"], "Review note");
    assert_eq!(create_body["type"], "text/plain");
    assert!(create_body["taskUrl"].as_str().unwrap().ends_with(&task_id));
    assert!(
        create_body["contentUrl"]
            .as_str()
            .unwrap()
            .ends_with("/content")
    );

    let list = client
        .get(format!("{base_url}/runtime/tasks/{task_id}/attachments"))
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
            "{base_url}/runtime/tasks/{task_id}/attachments/{attachment_id}"
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

    let events_after_create = client
        .get(format!("{base_url}/runtime/tasks/{task_id}/events"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(events_after_create.status(), reqwest::StatusCode::OK);
    let events_after_create_body: Value = events_after_create.json().await.unwrap();
    assert_eq!(events_after_create_body.as_array().unwrap().len(), 2);
    // Java Comment.xml selectEventsByTaskId: order by TIME_ desc (newest first).
    assert_eq!(events_after_create_body[0]["action"], "AddAttachment");
    assert_eq!(events_after_create_body[0]["message"][0], "review-note.txt");
    assert_eq!(events_after_create_body[1]["action"], "userTaskCreated");

    let delete = client
        .delete(format!(
            "{base_url}/runtime/tasks/{task_id}/attachments/{attachment_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(delete.status(), reqwest::StatusCode::NO_CONTENT);

    let events_after_delete = client
        .get(format!("{base_url}/runtime/tasks/{task_id}/events"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(events_after_delete.status(), reqwest::StatusCode::OK);
    let events_after_delete_body: Value = events_after_delete.json().await.unwrap();
    assert_eq!(events_after_delete_body.as_array().unwrap().len(), 3);
    assert_eq!(events_after_delete_body[0]["action"], "DeleteAttachment");
    assert_eq!(events_after_delete_body[0]["message"][0], "review-note.txt");

    let after_delete = client
        .get(format!(
            "{base_url}/runtime/tasks/{task_id}/attachments/{attachment_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(after_delete.status(), reqwest::StatusCode::NOT_FOUND);
}
