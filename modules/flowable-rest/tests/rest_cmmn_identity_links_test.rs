use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::identity::entities::User;
use flowable_rest::run_server;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::net::TcpListener;

const SIMPLE_CMMN: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/CMMN/20151109/MODEL"
             targetNamespace="http://flowable.org/cmmn">
  <case id="identityCase" name="Identity Case">
    <casePlanModel id="identityPlan" name="Identity Plan" autoComplete="false">
      <planItem id="planItemReview" name="Review" definitionRef="reviewTask" />
      <humanTask id="reviewTask" name="Review" isBlocking="true" />
    </casePlanModel>
  </case>
</definitions>"#;

async fn spawn_server() -> (String, reqwest::Client) {
    let engine = Arc::new(ProcessEngine::new("rest-cmmn-identity-links".to_string()));
    engine.get_identity_service().save_user(User {
        id: "admin".to_string(),
        first_name: None,
        last_name: None,
        email: None,
        password: Some("test".to_string()),
        tenant_id: None,
    });

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        run_server(engine, listener).await.unwrap();
    });

    (base_url, reqwest::Client::new())
}

async fn deploy_and_start_case(
    base_url: &str,
    client: &reqwest::Client,
) -> (String, String, String) {
    let deploy_response = client
        .post(format!("{base_url}/cmmn-repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "CMMN identity link deployment",
            "resourceName": "identity.cmmn",
            "resource": SIMPLE_CMMN
        }))
        .send()
        .await
        .unwrap();
    assert!(deploy_response.status().is_success());

    let definitions_response = client
        .get(format!(
            "{base_url}/cmmn-repository/case-definitions?key=identityCase"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(definitions_response.status().is_success());
    let definitions_body: Value = definitions_response.json().await.unwrap();
    let case_definition_id = definitions_body["data"][0]["id"]
        .as_str()
        .unwrap()
        .to_string();

    let start_response = client
        .post(format!("{base_url}/cmmn-runtime/case-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "caseDefinitionKey": "identityCase",
            "businessKey": "identity-bk"
        }))
        .send()
        .await
        .unwrap();
    assert!(start_response.status().is_success());
    let case_instance_id = start_response.json::<Value>().await.unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();

    let tasks_response = client
        .get(format!(
            "{base_url}/cmmn-runtime/tasks?caseInstanceId={case_instance_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(tasks_response.status().is_success());
    let tasks_body: Value = tasks_response.json().await.unwrap();
    let task_id = tasks_body["data"][0]["id"].as_str().unwrap().to_string();

    (case_definition_id, case_instance_id, task_id)
}

#[tokio::test]
async fn cmmn_identity_metadata_links_are_persisted_listed_read_and_deleted() {
    let (base_url, client) = spawn_server().await;
    let (case_definition_id, case_instance_id, task_id) =
        deploy_and_start_case(&base_url, &client).await;

    let create_definition_link = client
        .post(format!(
            "{base_url}/cmmn-repository/case-definitions/{case_definition_id}/identitylinks"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "group": "case-managers",
            "type": "candidate"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        create_definition_link.status(),
        reqwest::StatusCode::CREATED
    );

    let definition_links = client
        .get(format!(
            "{base_url}/cmmn-repository/case-definitions/{case_definition_id}/identitylinks"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(definition_links.status(), reqwest::StatusCode::OK);
    let definition_links_body: Value = definition_links.json().await.unwrap();
    assert_eq!(definition_links_body.as_array().unwrap().len(), 1);
    assert_eq!(definition_links_body[0]["group"], "case-managers");
    assert_eq!(definition_links_body[0]["type"], "candidate");

    let create_case_link = client
        .post(format!(
            "{base_url}/cmmn-runtime/case-instances/{case_instance_id}/identitylinks"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "user": "kermit",
            "type": "participant"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(create_case_link.status(), reqwest::StatusCode::CREATED);

    let case_link = client
        .get(format!(
            "{base_url}/cmmn-runtime/case-instances/{case_instance_id}/identitylinks/users/kermit/participant"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(case_link.status(), reqwest::StatusCode::OK);
    let case_link_body: Value = case_link.json().await.unwrap();
    assert_eq!(case_link_body["user"], "kermit");
    assert_eq!(case_link_body["type"], "participant");

    let historic_case_links = client
        .get(format!(
            "{base_url}/cmmn-history/historic-case-instances/{case_instance_id}/identitylinks"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(historic_case_links.status(), reqwest::StatusCode::OK);
    let historic_case_links_body: Value = historic_case_links.json().await.unwrap();
    assert!(
        historic_case_links_body
            .as_array()
            .unwrap()
            .iter()
            .any(|link| { link["user"] == "kermit" && link["type"] == "participant" })
    );

    let create_task_link = client
        .post(format!(
            "{base_url}/cmmn-runtime/tasks/{task_id}/identitylinks"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "user": "gonzo",
            "type": "candidate"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(create_task_link.status(), reqwest::StatusCode::CREATED);

    let task_user_links = client
        .get(format!(
            "{base_url}/cmmn-runtime/tasks/{task_id}/identitylinks/USERS"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(task_user_links.status(), reqwest::StatusCode::OK);
    let task_user_links_body: Value = task_user_links.json().await.unwrap();
    assert_eq!(task_user_links_body.as_array().unwrap().len(), 1);
    assert_eq!(task_user_links_body[0]["user"], "gonzo");

    let task_link = client
        .get(format!(
            "{base_url}/cmmn-runtime/tasks/{task_id}/identitylinks/Users/gonzo/candidate"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(task_link.status(), reqwest::StatusCode::OK);

    let historic_task_links = client
        .get(format!(
            "{base_url}/cmmn-history/historic-task-instances/{task_id}/identitylinks"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(historic_task_links.status(), reqwest::StatusCode::OK);
    let historic_task_links_body: Value = historic_task_links.json().await.unwrap();
    assert!(
        historic_task_links_body
            .as_array()
            .unwrap()
            .iter()
            .any(|link| { link["user"] == "gonzo" && link["type"] == "candidate" })
    );

    let delete_task_link = client
        .delete(format!(
            "{base_url}/cmmn-runtime/tasks/{task_id}/identitylinks/USERS/gonzo/candidate"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(delete_task_link.status(), reqwest::StatusCode::NO_CONTENT);

    let task_links_after_delete = client
        .get(format!(
            "{base_url}/cmmn-runtime/tasks/{task_id}/identitylinks"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(task_links_after_delete.status(), reqwest::StatusCode::OK);
    let task_links_after_delete_body: Value = task_links_after_delete.json().await.unwrap();
    assert!(task_links_after_delete_body.as_array().unwrap().is_empty());

    let delete_definition_link = client
        .delete(format!(
            "{base_url}/cmmn-repository/case-definitions/{case_definition_id}/identitylinks/GROUPS/case-managers"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(
        delete_definition_link.status(),
        reqwest::StatusCode::NO_CONTENT
    );
}
