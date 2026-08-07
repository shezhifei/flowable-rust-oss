use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_rest::run_server;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::net::TcpListener;

async fn spawn_server(test_name: &str) -> (Arc<ProcessEngine>, String, reqwest::Client) {
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
    let base_url = format!("http://{}", listener.local_addr().unwrap());

    let engine_clone = Arc::clone(&engine);
    tokio::spawn(async move {
        run_server(engine_clone, listener).await.unwrap();
    });

    (engine, base_url, reqwest::Client::new())
}

fn one_task_process_xml(process_id: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
        <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
            <process id="{process_id}" name="{process_id}" isExecutable="true">
                <startEvent id="start" />
                <sequenceFlow id="flow1" sourceRef="start" targetRef="task1" />
                <userTask id="task1" name="Task 1" />
                <sequenceFlow id="flow2" sourceRef="task1" targetRef="end" />
                <endEvent id="end" />
            </process>
        </definitions>"#
    )
}

fn auto_complete_process_xml(process_id: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
        <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
            <process id="{process_id}" name="{process_id}" isExecutable="true">
                <startEvent id="start" />
                <sequenceFlow id="flow1" sourceRef="start" targetRef="end" />
                <endEvent id="end" />
            </process>
        </definitions>"#
    )
}

async fn deploy_and_start(
    engine: &ProcessEngine,
    client: &reqwest::Client,
    base_url: &str,
    process_id: &str,
    process_xml: String,
) -> String {
    let deploy_response = client
        .post(format!("{base_url}/repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": format!("{process_id} deployment"),
            "resourceName": format!("{process_id}.bpmn20.xml"),
            "resource": process_xml
        }))
        .send()
        .await
        .unwrap();
    assert!(deploy_response.status().is_success());

    let process_definition_id = engine
        .get_repository_service()
        .get_process_definition_ids()
        .unwrap()[0]
        .clone();

    let start_response = client
        .post(format!("{base_url}/runtime/process-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "processDefinitionId": process_definition_id }))
        .send()
        .await
        .unwrap();
    assert!(start_response.status().is_success());
    let started: Value = start_response.json().await.unwrap();
    started["id"].as_str().unwrap().to_string()
}

/// Java `IdentityLinkEntityManagerImpl` performs no dedup on the create path:
/// each POST appends a fresh row, so duplicate user+type POSTs yield duplicate
/// collection entries (and single GET/DELETE resolve the first match).
#[tokio::test]
async fn duplicate_post_appends_identity_links() {
    let (engine, base_url, client) = spawn_server("pi-identity-link-duplicate-post").await;
    let process_instance_id = deploy_and_start(
        &engine,
        &client,
        &base_url,
        "identityLinkDuplicateProcess",
        one_task_process_xml("identityLinkDuplicateProcess"),
    )
    .await;

    for _ in 0..2 {
        let response = client
            .post(format!(
                "{base_url}/runtime/process-instances/{process_instance_id}/identitylinks"
            ))
            .basic_auth("admin", Some("test"))
            .json(&json!({ "user": "gonzo", "type": "participant" }))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::CREATED);
    }

    let list_response = client
        .get(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}/identitylinks"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(list_response.status(), reqwest::StatusCode::OK);
    let listed: Value = list_response.json().await.unwrap();
    let listed = listed.as_array().unwrap();
    assert_eq!(listed.len(), 3);
    assert_eq!(
        listed
            .iter()
            .filter(|link| link["user"] == "gonzo" && link["type"] == "participant")
            .count(),
        2
    );
    assert!(
        listed
            .iter()
            .any(|link| link["user"] == "admin" && link["type"] == "starter")
    );

    let get_response = client
        .get(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}/identitylinks/users/gonzo/participant"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(get_response.status(), reqwest::StatusCode::OK);

    let delete_response = client
        .delete(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}/identitylinks/users/gonzo/participant"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(delete_response.status(), reqwest::StatusCode::NO_CONTENT);

    let after_delete_response = client
        .get(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}/identitylinks"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    let after_delete: Value = after_delete_response.json().await.unwrap();
    let after_delete = after_delete.as_array().unwrap();
    assert_eq!(after_delete.len(), 2);
    assert!(
        after_delete
            .iter()
            .any(|link| link["user"] == "admin" && link["type"] == "starter")
    );
}

/// Java `BaseProcessInstanceResource.getProcessInstanceFromRequest` queries the
/// runtime instance only, so the whole runtime identity-link family answers
/// 404 once the instance has ended; the history endpoints remain reachable.
#[tokio::test]
async fn completed_instance_runtime_identity_links_return_404() {
    let (engine, base_url, client) = spawn_server("pi-identity-link-completed-404").await;
    let process_instance_id = deploy_and_start(
        &engine,
        &client,
        &base_url,
        "identityLinkCompletedProcess",
        auto_complete_process_xml("identityLinkCompletedProcess"),
    )
    .await;

    let historic_response = client
        .get(format!(
            "{base_url}/history/historic-process-instances/{process_instance_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(historic_response.status(), reqwest::StatusCode::OK);
    let historic: Value = historic_response.json().await.unwrap();
    assert!(
        !historic["endTime"].is_null(),
        "instance should be completed: {historic}"
    );

    let collection_get = client
        .get(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}/identitylinks"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(collection_get.status(), reqwest::StatusCode::NOT_FOUND);

    let create_response = client
        .post(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}/identitylinks"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "user": "gonzo", "type": "participant" }))
        .send()
        .await
        .unwrap();
    assert_eq!(create_response.status(), reqwest::StatusCode::NOT_FOUND);

    let single_get = client
        .get(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}/identitylinks/users/gonzo/participant"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(single_get.status(), reqwest::StatusCode::NOT_FOUND);

    let delete_response = client
        .delete(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}/identitylinks/users/gonzo/participant"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(delete_response.status(), reqwest::StatusCode::NOT_FOUND);

    let history_links = client
        .get(format!(
            "{base_url}/history/historic-process-instances/{process_instance_id}/identitylinks"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(history_links.status(), reqwest::StatusCode::OK);
}

/// Java writes a TYPE_EVENT comment (`AddUserLink`/`DeleteUserLink`) on the
/// process instance for identity-link changes; these surface through the
/// historic process-instance comments endpoint with the request principal as
/// author.
#[tokio::test]
async fn identity_link_changes_appear_in_historic_process_instance_comments() {
    let (engine, base_url, client) = spawn_server("pi-identity-link-comments").await;
    let process_instance_id = deploy_and_start(
        &engine,
        &client,
        &base_url,
        "identityLinkCommentProcess",
        one_task_process_xml("identityLinkCommentProcess"),
    )
    .await;

    let create_response = client
        .post(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}/identitylinks"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "user": "fozzie", "type": "participant" }))
        .send()
        .await
        .unwrap();
    assert_eq!(create_response.status(), reqwest::StatusCode::CREATED);

    let comments_response = client
        .get(format!(
            "{base_url}/history/historic-process-instances/{process_instance_id}/comments"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(comments_response.status(), reqwest::StatusCode::OK);
    let comments: Value = comments_response.json().await.unwrap();
    let comments = comments.as_array().unwrap();
    assert_eq!(comments.len(), 1);
    let message = comments[0]["message"].as_str().unwrap();
    assert!(message.contains("fozzie"), "message: {message}");
    assert!(message.contains("participant"), "message: {message}");
    assert_eq!(comments[0]["author"], "admin");

    let delete_response = client
        .delete(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}/identitylinks/users/fozzie/participant"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(delete_response.status(), reqwest::StatusCode::NO_CONTENT);

    let after_delete_comments = client
        .get(format!(
            "{base_url}/history/historic-process-instances/{process_instance_id}/comments"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    let after_delete: Value = after_delete_comments.json().await.unwrap();
    assert_eq!(after_delete.as_array().unwrap().len(), 2);
}
