use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::identity::entities::User;
use flowable_rest::run_server;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::net::TcpListener;

const SIMPLE_CMMN: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/CMMN/20151109/MODEL"
             targetNamespace="http://flowable.org/cmmn">
  <case id="deleteCase" name="Delete Case">
    <casePlanModel id="deletePlan" name="Delete Plan" autoComplete="false">
      <planItem id="planItemReview" name="Review" definitionRef="reviewTask" />
      <humanTask id="reviewTask" name="Review" isBlocking="true" />
    </casePlanModel>
  </case>
</definitions>"#;

async fn spawn_server(test_name: &str) -> (String, reqwest::Client) {
    let engine = Arc::new(ProcessEngine::new(test_name.to_string()));
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

async fn deploy_case(base_url: &str, client: &reqwest::Client) {
    let response = client
        .post(format!("{base_url}/cmmn-repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "CMMN delete deployment",
            "resourceName": "delete.cmmn",
            "resource": SIMPLE_CMMN
        }))
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success());
}

async fn start_case(base_url: &str, client: &reqwest::Client) -> String {
    let response = client
        .post(format!("{base_url}/cmmn-runtime/case-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "caseDefinitionKey": "deleteCase" }))
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success());
    response.json::<Value>().await.unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string()
}

#[tokio::test]
async fn cmmn_case_delete_endpoints_remove_runtime_and_keep_terminated_history() {
    let (base_url, client) = spawn_server("rest-cmmn-case-delete-single").await;
    deploy_case(&base_url, &client).await;
    let case_instance_id = start_case(&base_url, &client).await;

    let delete_response = client
        .delete(format!(
            "{base_url}/cmmn-runtime/case-instances/{case_instance_id}/delete"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(delete_response.status(), reqwest::StatusCode::NO_CONTENT);

    let runtime_response = client
        .get(format!(
            "{base_url}/cmmn-runtime/case-instances/{case_instance_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(runtime_response.status(), reqwest::StatusCode::NOT_FOUND);

    let historic_response = client
        .get(format!(
            "{base_url}/cmmn-history/historic-case-instances/{case_instance_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(historic_response.status(), reqwest::StatusCode::OK);
    let historic_body: Value = historic_response.json().await.unwrap();
    assert_eq!(historic_body["id"], case_instance_id);
    assert_eq!(historic_body["state"], "TERMINATED");
    assert!(historic_body["endedAt"].as_str().is_some());
}

#[tokio::test]
async fn cmmn_bulk_case_delete_is_atomic_and_supports_historic_bulk_delete() {
    let (base_url, client) = spawn_server("rest-cmmn-case-delete-bulk").await;
    deploy_case(&base_url, &client).await;
    let case_one = start_case(&base_url, &client).await;
    let case_two = start_case(&base_url, &client).await;
    let case_three = start_case(&base_url, &client).await;

    let failed_bulk = client
        .post(format!("{base_url}/cmmn-runtime/case-instances/delete"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "action": "delete",
            "instanceIds": [case_one, case_two, "missing-case"]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(failed_bulk.status(), reqwest::StatusCode::NOT_FOUND);

    for case_instance_id in [&case_one, &case_two, &case_three] {
        let response = client
            .get(format!(
                "{base_url}/cmmn-runtime/case-instances/{case_instance_id}"
            ))
            .basic_auth("admin", Some("test"))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::OK);
    }

    let successful_bulk = client
        .post(format!("{base_url}/cmmn-runtime/case-instances/delete"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "action": "delete",
            "instanceIds": [case_one, case_two]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(successful_bulk.status(), reqwest::StatusCode::NO_CONTENT);

    let remaining_response = client
        .get(format!(
            "{base_url}/cmmn-runtime/case-instances/{case_three}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(remaining_response.status(), reqwest::StatusCode::OK);

    let history_delete = client
        .post(format!(
            "{base_url}/cmmn-history/historic-case-instances/delete"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "action": "delete",
            "instanceIds": [case_one, case_two]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(history_delete.status(), reqwest::StatusCode::NO_CONTENT);

    for case_instance_id in [&case_one, &case_two] {
        let response = client
            .get(format!(
                "{base_url}/cmmn-history/historic-case-instances/{case_instance_id}"
            ))
            .basic_auth("admin", Some("test"))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::NOT_FOUND);
    }
}
