use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_rest::run_server;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::net::TcpListener;

const COMMENT_PROCESS_BPMN: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             targetNamespace="Examples">
    <process id="commentProcess" name="Comment Process" isExecutable="true">
        <startEvent id="startEvent" />
        <sequenceFlow id="flow1" sourceRef="startEvent" targetRef="reviewTask" />
        <userTask id="reviewTask" name="Review comment" />
        <sequenceFlow id="flow2" sourceRef="reviewTask" targetRef="endEvent" />
        <endEvent id="endEvent" />
    </process>
</definitions>"#;

async fn spawn_server() -> (Arc<ProcessEngine>, String, reqwest::Client) {
    let engine = Arc::new(ProcessEngine::new("rest-task-comments-events".to_string()));
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
            "name": "Comment process",
            "resourceName": "comment-process.bpmn20.xml",
            "resource": COMMENT_PROCESS_BPMN
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

async fn active_task(client: &reqwest::Client, base_url: &str) -> Value {
    let response = client
        .get(format!("{base_url}/runtime/tasks"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    response.json::<Value>().await.unwrap()["data"][0].clone()
}

#[tokio::test]
async fn task_comments_are_persisted_for_runtime_history_and_task_events() {
    let (engine, base_url, client) = spawn_server().await;
    deploy_process(&client, &base_url).await;
    let process_definition_id = engine
        .get_repository_service()
        .latest_process_definition_by_key("commentProcess", None)
        .unwrap()
        .unwrap()
        .id;
    start_process(&client, &base_url, &process_definition_id).await;
    let task = active_task(&client, &base_url).await;
    let task_id = task["id"].as_str().unwrap();
    let process_instance_id = task["processInstanceId"].as_str().unwrap();

    let create = client
        .post(format!("{base_url}/runtime/tasks/{task_id}/comments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "message": "Please verify the invoice total",
            "saveProcessInstanceId": true
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(create.status(), reqwest::StatusCode::CREATED);
    let create_body = create.json::<Value>().await.unwrap();
    let comment_id = create_body["id"].as_str().unwrap();
    assert_eq!(create_body["message"], "Please verify the invoice total");
    assert_eq!(create_body["taskId"], task_id);
    assert_eq!(create_body["processInstanceId"], process_instance_id);
    assert!(create_body["taskUrl"].as_str().unwrap().contains(task_id));
    assert!(
        create_body["processInstanceUrl"]
            .as_str()
            .unwrap()
            .contains(process_instance_id)
    );

    let task_comments = client
        .get(format!("{base_url}/runtime/tasks/{task_id}/comments"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(task_comments.status(), reqwest::StatusCode::OK);
    let task_comments_body = task_comments.json::<Value>().await.unwrap();
    assert_eq!(task_comments_body.as_array().unwrap().len(), 1);
    assert_eq!(task_comments_body[0]["id"], comment_id);

    let task_comment = client
        .get(format!(
            "{base_url}/runtime/tasks/{task_id}/comments/{comment_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(task_comment.status(), reqwest::StatusCode::OK);
    assert_eq!(
        task_comment.json::<Value>().await.unwrap()["id"],
        comment_id
    );

    let history_comments = client
        .get(format!(
            "{base_url}/history/historic-process-instances/{process_instance_id}/comments"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(history_comments.status(), reqwest::StatusCode::OK);
    let history_comments_body = history_comments.json::<Value>().await.unwrap();
    assert_eq!(history_comments_body.as_array().unwrap().len(), 1);
    assert_eq!(history_comments_body[0]["id"], comment_id);

    let history_comment = client
        .get(format!(
            "{base_url}/history/historic-process-instances/{process_instance_id}/comments/{comment_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(history_comment.status(), reqwest::StatusCode::OK);
    assert_eq!(
        history_comment.json::<Value>().await.unwrap()["id"],
        comment_id
    );

    let task_events = client
        .get(format!("{base_url}/runtime/tasks/{task_id}/events"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(task_events.status(), reqwest::StatusCode::OK);
    let task_events_body = task_events.json::<Value>().await.unwrap();
    assert_eq!(task_events_body.as_array().unwrap().len(), 2);
    // Java Comment.xml selectEventsByTaskId: order by TIME_ desc (newest first).
    assert_eq!(task_events_body[0]["action"], "AddComment");
    let event_id = task_events_body[0]["id"].as_str().unwrap();
    assert_eq!(
        task_events_body[0]["message"][0],
        "Please verify the invoice total"
    );
    assert_eq!(task_events_body[1]["action"], "userTaskCreated");

    let task_event = client
        .get(format!(
            "{base_url}/runtime/tasks/{task_id}/events/{event_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(task_event.status(), reqwest::StatusCode::OK);
    assert_eq!(task_event.json::<Value>().await.unwrap()["id"], event_id);

    let delete = client
        .delete(format!(
            "{base_url}/runtime/tasks/{task_id}/comments/{comment_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(delete.status(), reqwest::StatusCode::NO_CONTENT);

    let after_delete = client
        .get(format!(
            "{base_url}/runtime/tasks/{task_id}/comments/{comment_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(after_delete.status(), reqwest::StatusCode::NOT_FOUND);
}
