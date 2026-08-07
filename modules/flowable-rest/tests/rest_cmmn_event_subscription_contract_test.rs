use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_rest::run_server;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::net::TcpListener;

const EVENT_LISTENER_CMMN: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/CMMN/20151109/MODEL"
             targetNamespace="http://flowable.org/cmmn">
  <case id="eventSubscriptionCase" name="Event Subscription Case">
    <casePlanModel id="planModel" name="Plan Model" autoComplete="false">
      <planItem id="planItemEvent" definitionRef="waitForApproval" />
      <eventListener id="waitForApproval"
                     name="Wait for approval"
                     eventType="message"
                     eventName="approvalReceived" />
    </casePlanModel>
  </case>
</definitions>"#;

async fn spawn_server(test_name: &str) -> (String, reqwest::Client) {
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

    tokio::spawn(async move {
        run_server(engine, listener).await.unwrap();
    });

    (base_url, reqwest::Client::new())
}

async fn deploy_and_start_event_case(base_url: &str, client: &reqwest::Client) -> String {
    let deploy_response = client
        .post(format!("{base_url}/cmmn-repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "CMMN Event Subscription Deployment",
            "resourceName": "event-subscription.cmmn",
            "resource": EVENT_LISTENER_CMMN
        }))
        .send()
        .await
        .unwrap();
    assert!(deploy_response.status().is_success());

    let start_response = client
        .post(format!("{base_url}/cmmn-runtime/case-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "caseDefinitionKey": "eventSubscriptionCase",
            "businessKey": "event-subscription-bk"
        }))
        .send()
        .await
        .unwrap();
    assert!(start_response.status().is_success());

    start_response.json::<Value>().await.unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string()
}

#[tokio::test]
async fn cmmn_event_subscription_paths_return_real_runtime_subscriptions() {
    let (base_url, client) = spawn_server("rest-cmmn-event-subscription-contract").await;
    let case_instance_id = deploy_and_start_event_case(&base_url, &client).await;

    let list_response = client
        .get(format!(
            "{base_url}/cmmn-runtime/event-subscriptions?caseInstanceId={case_instance_id}&eventType=message&eventName=approvalReceived&start=0&size=10"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(list_response.status().is_success());

    let list_body: Value = list_response.json().await.unwrap();
    assert_eq!(list_body["start"], 0);
    assert_eq!(list_body["size"], 1);
    assert_eq!(list_body["total"], 1);

    let subscription = &list_body["data"][0];
    assert!(
        subscription["id"]
            .as_str()
            .unwrap()
            .starts_with("cmmn-event-subscription:")
    );
    assert_eq!(subscription["eventType"], "message");
    assert_eq!(subscription["eventName"], "approvalReceived");
    assert_eq!(subscription["activityId"], "waitForApproval");
    assert_eq!(subscription["caseInstanceId"], case_instance_id);
    assert!(
        subscription["caseDefinitionId"]
            .as_str()
            .unwrap()
            .contains("eventSubscriptionCase")
    );
    assert_eq!(subscription["planItemInstanceId"], "planItemEvent");
    assert!(subscription["tenantId"].is_null());
    assert!(subscription["configuration"].is_null());
    assert!(subscription["created"].as_str().unwrap().contains('T'));

    let subscription_id = subscription["id"].as_str().unwrap();
    let get_response = client
        .get(format!(
            "{base_url}/cmmn-runtime/event-subscriptions/{subscription_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(get_response.status().is_success());
    let get_body: Value = get_response.json().await.unwrap();
    assert_eq!(get_body["id"], subscription_id);
    assert_eq!(get_body["eventType"], "message");
    assert_eq!(get_body["eventName"], "approvalReceived");
    assert_eq!(get_body["caseInstanceId"], case_instance_id);
}
