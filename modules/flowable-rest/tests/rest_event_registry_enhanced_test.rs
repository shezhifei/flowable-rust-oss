use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_rest::run_server;
use serde_json::json;
use std::sync::Arc;
use tokio::net::TcpListener;

async fn spawn_server(test_name: &str) -> (Arc<ProcessEngine>, String, reqwest::Client) {
    // The mock outbound HTTP servers in this file bind 127.0.0.1, which the P142b
    // SSRF guard denies by default. These tests predate the guard and intentionally
    // exercise loopback delivery, so opt in via the documented engine-level escape
    // hatch (mirrored into the event-registry outbound guard by
    // `FlowableEventRegistryService::new`).
    let mut engine_config =
        flowable_engine::service::config::ProcessEngineConfiguration::default();
    engine_config.http_service.real_client.allow_private_networks = true;
    let engine = Arc::new(
        ProcessEngine::try_new_with_config(test_name.to_string(), engine_config)
            .expect("process engine"),
    );

    let user = flowable_engine::identity::entities::User {
        id: "admin".to_string(),
        first_name: None,
        last_name: None,
        email: None,
        password: Some("test".to_string()),
        tenant_id: None,
    };
    engine.get_identity_service().save_user(user);

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let base_url = format!("http://{}", addr);

    let engine_clone = Arc::clone(&engine);
    tokio::spawn(async move {
        run_server(engine_clone, listener).await.unwrap();
    });

    (engine, base_url, reqwest::Client::new())
}

async fn deploy_event_registry_definitions(client: &reqwest::Client, base_url: &str) {
    let inbound_channel = json!({
        "key": "ordersInbound",
        "name": "Orders inbound",
        "description": "Inbound orders channel",
        "channelType": "inbound",
        "resourceName": "orders-inbound.channel",
        "type": "in-memory",
        "destination": "orders-inbound",
        "deserializerType": "json"
    });

    let outbound_channel = json!({
        "key": "ordersOutbound",
        "name": "Orders outbound",
        "description": "Outbound orders channel",
        "channelType": "outbound",
        "resourceName": "orders-outbound.channel",
        "type": "in-memory",
        "destination": "orders-outbound",
        "serializerType": "json"
    });

    let inbound_event = json!({
        "key": "orderReceived",
        "name": "Order received",
        "description": "Inbound order event",
        "eventType": "order.received",
        "channelKey": "ordersInbound",
        "resourceName": "order-received.event",
        "payload": [
            { "name": "orderId", "type": "string", "required": true },
            { "name": "amount", "type": "double", "required": true },
            { "name": "customerId", "type": "string", "required": false }
        ]
    });

    let outbound_event = json!({
        "key": "orderPublished",
        "name": "Order published",
        "description": "Outbound order event",
        "eventType": "order.published",
        "channelKey": "ordersOutbound",
        "resourceName": "order-published.event",
        "payload": [
            { "name": "orderId", "type": "string" }
        ]
    });

    let response = client
        .post(format!(
            "{}/event-registry-repository/deployments",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "Event Registry Enhanced Deployment",
            "resources": [
                {
                    "resourceName": "orders-inbound.channel",
                    "resource": inbound_channel.to_string()
                },
                {
                    "resourceName": "orders-outbound.channel",
                    "resource": outbound_channel.to_string()
                },
                {
                    "resourceName": "order-received.event",
                    "resource": inbound_event.to_string()
                },
                {
                    "resourceName": "order-published.event",
                    "resource": outbound_event.to_string()
                }
            ]
        }))
        .send()
        .await
        .unwrap();

    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    assert!(
        status.is_success(),
        "Failed to deploy event registry definitions: {}, Body: {}",
        status,
        text
    );
}

async fn deploy_failing_rest_outbound_definition(
    client: &reqwest::Client,
    base_url: &str,
    destination: &str,
) {
    let outbound_channel = json!({
        "key": "failingRestOutbound",
        "name": "Failing REST outbound",
        "channelType": "outbound",
        "resourceName": "failing-rest-outbound.channel",
        "type": "rest",
        "destination": destination,
        "serializerType": "json"
    });

    let outbound_event = json!({
        "key": "failingRestEvent",
        "name": "Failing REST event",
        "eventType": "rest.failed",
        "channelKey": "failingRestOutbound",
        "resourceName": "failing-rest.event",
        "payload": [
            { "name": "orderId", "type": "string", "required": true }
        ]
    });

    let response = client
        .post(format!(
            "{}/event-registry-repository/deployments",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "Failing REST Outbound Deployment",
            "resources": [
                {
                    "resourceName": "failing-rest-outbound.channel",
                    "resource": outbound_channel.to_string()
                },
                {
                    "resourceName": "failing-rest.event",
                    "resource": outbound_event.to_string()
                }
            ]
        }))
        .send()
        .await
        .unwrap();

    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    assert!(
        status.is_success(),
        "Failed to deploy failing REST outbound definitions: {}, Body: {}",
        status,
        text
    );
}

fn start_rest_status_server(status_code: u16) -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let destination = format!("http://{}/events", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            use std::io::{Read, Write};
            let mut buffer = [0_u8; 1024];
            let _ = stream.read(&mut buffer);
            let body = if (200..300).contains(&status_code) {
                b"{}".as_slice()
            } else {
                b"{\"error\":\"boom\"}".as_slice()
            };
            let reason = if (200..300).contains(&status_code) {
                "OK"
            } else {
                "Internal Server Error"
            };
            let response = format!(
                "HTTP/1.1 {status_code} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                String::from_utf8_lossy(body)
            );
            let _ = stream.write_all(response.as_bytes());
        }
    });
    destination
}

#[tokio::test]
async fn rest_channel_accepts_inbound_events() {
    let (_engine, base_url, client) = spawn_server("rest_channel_accepts_inbound_events").await;
    deploy_event_registry_definitions(&client, &base_url).await;

    let response = client
        .post(format!(
            "{}/event-registry-runtime/inbound-event-instances",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "eventType": "order.received",
            "eventPayload": {
                "orderId": "ORD-001",
                "amount": 99.99,
                "customerId": "CUST-001"
            }
        }))
        .send()
        .await
        .unwrap();

    assert!(
        response.status().is_success(),
        "Inbound event should be accepted: {}",
        response.status()
    );
}

#[tokio::test]
async fn failed_rest_outbound_delivery_is_queryable_with_retry_metadata() {
    let (_engine, base_url, client) =
        spawn_server("failed_rest_outbound_delivery_is_queryable_with_retry_metadata").await;
    let destination = start_rest_status_server(500);

    deploy_failing_rest_outbound_definition(&client, &base_url, &destination).await;

    let publish_response = client
        .post(format!(
            "{}/event-registry-runtime/event-instances",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "eventDefinitionKey": "failingRestEvent",
            "eventPayload": {
                "orderId": "ORD-FAILED"
            }
        }))
        .send()
        .await
        .unwrap();
    assert!(
        publish_response.status().is_server_error(),
        "REST dispatch failure should surface as server error, got {}",
        publish_response.status()
    );

    let failed_response = client
        .get(format!(
            "{}/event-registry-management/event-instance-deliveries?status=FAILED",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    let failed_status = failed_response.status();
    let failed_text = failed_response.text().await.unwrap_or_default();
    assert!(
        failed_status.is_success(),
        "FAILED delivery query should succeed: {failed_status}, body: {failed_text}"
    );
    let failed_body: serde_json::Value = serde_json::from_str(&failed_text).unwrap();
    assert_eq!(failed_body["total"], 1);
    let delivery = &failed_body["data"][0];
    assert_eq!(delivery["status"], "FAILED");
    let last_error = delivery["lastError"].as_str().unwrap();
    assert!(
        last_error.contains("status 500"),
        "unexpected lastError: {last_error}"
    );
    assert_eq!(delivery["retryCount"], 0);
    assert!(delivery["lastFailureAt"].is_number());
    assert!(delivery["nextRetryAt"].is_number());
    assert!(delivery["lastRetryAt"].is_null());

    let delivery_id = delivery["id"].as_str().unwrap();
    let get_response = client
        .get(format!(
            "{}/event-registry-management/event-instance-deliveries/{}",
            base_url, delivery_id
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    let get_status = get_response.status();
    let get_text = get_response.text().await.unwrap_or_default();
    assert!(
        get_status.is_success(),
        "FAILED delivery get should succeed: {get_status}, body: {get_text}"
    );
    let get_body: serde_json::Value = serde_json::from_str(&get_text).unwrap();
    assert_eq!(get_body["id"], delivery_id);
    assert_eq!(get_body["lastError"], delivery["lastError"]);
    assert_eq!(get_body["retryCount"], delivery["retryCount"]);
    assert_eq!(get_body["lastFailureAt"], delivery["lastFailureAt"]);
    assert_eq!(get_body["nextRetryAt"], delivery["nextRetryAt"]);
    assert_eq!(get_body["lastRetryAt"], delivery["lastRetryAt"]);
}

#[tokio::test]
async fn rest_channel_sends_outbound_events() {
    let (_engine, base_url, client) = spawn_server("rest_channel_sends_outbound_events").await;
    deploy_event_registry_definitions(&client, &base_url).await;

    let response = client
        .post(format!(
            "{}/event-registry-runtime/event-instances",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "eventDefinitionKey": "orderPublished",
            "eventPayload": {
                "orderId": "ORD-002"
            }
        }))
        .send()
        .await
        .unwrap();

    assert!(
        response.status().is_success(),
        "Outbound event should be sent: {}",
        response.status()
    );
}

#[tokio::test]
async fn retry_failed_rest_outbound_delivery_returns_retry_metadata() {
    let (_engine, base_url, client) =
        spawn_server("retry_failed_rest_outbound_delivery_returns_retry_metadata").await;
    let failing_destination = start_rest_status_server(500);
    deploy_failing_rest_outbound_definition(&client, &base_url, &failing_destination).await;

    let publish_response = client
        .post(format!(
            "{}/event-registry-runtime/event-instances",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "eventDefinitionKey": "failingRestEvent",
            "eventPayload": {
                "orderId": "ORD-RETRY-METADATA"
            }
        }))
        .send()
        .await
        .unwrap();
    assert!(
        publish_response.status().is_server_error(),
        "initial REST dispatch failure should surface as server error"
    );

    let failed_response = client
        .get(format!(
            "{}/event-registry-management/event-instance-deliveries?status=FAILED",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(failed_response.status().is_success());
    let failed_body: serde_json::Value = failed_response.json().await.unwrap();
    let delivery_id = failed_body["data"][0]["id"].as_str().unwrap();

    let channels_response = client
        .get(format!(
            "{}/event-registry-repository/channel-definitions",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(channels_response.status().is_success());
    let channels: serde_json::Value = channels_response.json().await.unwrap();
    let channel_id = channels["data"]
        .as_array()
        .unwrap()
        .iter()
        .find(|channel| channel["key"] == "failingRestOutbound")
        .and_then(|channel| channel["id"].as_str())
        .unwrap();

    let retry_destination = start_rest_status_server(204);
    let update_response = client
        .put(format!(
            "{}/event-registry-repository/channel-definitions/{}",
            base_url, channel_id
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "configuration": {
                "type": "rest",
                "destination": retry_destination,
                "serializerType": "json"
            }
        }))
        .send()
        .await
        .unwrap();
    assert!(
        update_response.status().is_success(),
        "channel update should succeed before retry"
    );

    let retry_response = client
        .post(format!(
            "{}/event-registry-management/event-deliveries/{}/retry",
            base_url, delivery_id
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    let retry_status = retry_response.status();
    let retry_text = retry_response.text().await.unwrap_or_default();
    assert!(
        retry_status.is_success(),
        "retry should succeed: {retry_status}, body: {retry_text}"
    );
    let retry_body: serde_json::Value = serde_json::from_str(&retry_text).unwrap();
    assert_eq!(retry_body["id"], delivery_id);
    assert_eq!(retry_body["status"], "PUBLISHED");
    assert_eq!(retry_body["retryCount"], 1);
    assert!(retry_body["lastRetryAt"].is_number());
    assert!(retry_body["lastError"].is_null());
    assert!(retry_body["lastFailureAt"].is_null());
    assert!(retry_body["nextRetryAt"].is_null());
}

#[tokio::test]
async fn event_payload_validation_rejects_invalid() {
    let (_engine, base_url, client) =
        spawn_server("event_payload_validation_rejects_invalid").await;
    deploy_event_registry_definitions(&client, &base_url).await;

    let response = client
        .post(format!(
            "{}/event-registry-runtime/inbound-event-instances",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "eventType": "order.received",
            "eventPayload": {
                "customerId": "CUST-001"
            }
        }))
        .send()
        .await
        .unwrap();

    assert!(
        response.status().is_server_error(),
        "Missing required fields should cause server error: {}",
        response.status()
    );
}

#[tokio::test]
async fn event_delivery_retry_on_failure() {
    let (_engine, base_url, client) = spawn_server("event_delivery_retry_on_failure").await;
    deploy_event_registry_definitions(&client, &base_url).await;

    let response = client
        .post(format!(
            "{}/event-registry-runtime/inbound-event-instances",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "eventType": "order.received",
            "eventPayload": {
                "orderId": "ORD-003",
                "amount": 150.00
            }
        }))
        .send()
        .await
        .unwrap();

    assert!(response.status().is_success());

    let delivery: serde_json::Value = response.json().await.unwrap();
    let delivery_id = delivery["id"].as_str().unwrap().to_string();
    assert_eq!(delivery["status"], "PROCESSED");

    let retry_response = client
        .post(format!(
            "{}/event-registry-management/event-deliveries/{}/retry",
            base_url, delivery_id
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    // Successfully processed inbound deliveries are not retryable: a replay
    // would re-run the consumer and duplicate side effects.
    assert_eq!(
        retry_response.status(),
        reqwest::StatusCode::CONFLICT,
        "Retrying a PROCESSED delivery must be rejected with 409"
    );
    let error_body: serde_json::Value = retry_response.json().await.unwrap();
    assert!(
        error_body["details"]
            .as_str()
            .unwrap_or_default()
            .contains("not retryable"),
        "error details should explain the delivery is not retryable: {error_body}"
    );
}

#[tokio::test]
async fn manual_retry_delivery() {
    let (_engine, base_url, client) = spawn_server("manual_retry_delivery").await;
    deploy_event_registry_definitions(&client, &base_url).await;

    let response = client
        .post(format!(
            "{}/event-registry-runtime/inbound-event-instances",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "eventType": "order.received",
            "eventPayload": {
                "orderId": "ORD-004",
                "amount": 200.00
            }
        }))
        .send()
        .await
        .unwrap();

    assert!(response.status().is_success());
    let delivery: serde_json::Value = response.json().await.unwrap();
    let delivery_id = delivery["id"].as_str().unwrap().to_string();
    assert_eq!(delivery["status"], "PROCESSED");

    let retry_response = client
        .post(format!(
            "{}/event-registry-management/event-deliveries/{}/retry",
            base_url, delivery_id
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    // Manual retry of a PROCESSED inbound delivery is rejected: only FAILED
    // inbound deliveries may be replayed through the pipeline.
    assert_eq!(retry_response.status(), reqwest::StatusCode::CONFLICT);

    let get_response = client
        .get(format!(
            "{}/event-registry-management/event-instance-deliveries/{}",
            base_url, delivery_id
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(get_response.status().is_success());
    let after: serde_json::Value = get_response.json().await.unwrap();
    assert_eq!(after["id"], delivery_id);
    assert_eq!(after["status"], "PROCESSED");
    assert_eq!(after["retryCount"], 0);
}

#[tokio::test]
async fn update_channel_definition() {
    let (_engine, base_url, client) = spawn_server("update_channel_definition").await;
    deploy_event_registry_definitions(&client, &base_url).await;

    let list_response = client
        .get(format!(
            "{}/event-registry-repository/channel-definitions",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert!(list_response.status().is_success());
    let channels: serde_json::Value = list_response.json().await.unwrap();
    let channel_id = channels["data"][0]["id"].as_str().unwrap().to_string();

    let update_response = client
        .put(format!(
            "{}/event-registry-repository/channel-definitions/{}",
            base_url, channel_id
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "Updated Channel Name"
        }))
        .send()
        .await
        .unwrap();

    assert!(
        update_response.status().is_success(),
        "Channel update should succeed: {}",
        update_response.status()
    );

    let updated: serde_json::Value = update_response.json().await.unwrap();
    assert_eq!(updated["name"], "Updated Channel Name");
}

#[tokio::test]
async fn update_event_definition() {
    let (_engine, base_url, client) = spawn_server("update_event_definition").await;
    deploy_event_registry_definitions(&client, &base_url).await;

    let list_response = client
        .get(format!(
            "{}/event-registry-repository/event-definitions",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert!(list_response.status().is_success());
    let events: serde_json::Value = list_response.json().await.unwrap();
    let event_id = events["data"][0]["id"].as_str().unwrap().to_string();

    let update_response = client
        .put(format!(
            "{}/event-registry-repository/event-definitions/{}",
            base_url, event_id
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "Updated Event Name"
        }))
        .send()
        .await
        .unwrap();

    assert!(
        update_response.status().is_success(),
        "Event definition update should succeed: {}",
        update_response.status()
    );

    let updated: serde_json::Value = update_response.json().await.unwrap();
    assert_eq!(updated["name"], "Updated Event Name");
}

#[tokio::test]
async fn delete_event_delivery() {
    let (_engine, base_url, client) = spawn_server("delete_event_delivery").await;
    deploy_event_registry_definitions(&client, &base_url).await;

    let response = client
        .post(format!(
            "{}/event-registry-runtime/inbound-event-instances",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "eventType": "order.received",
            "eventPayload": {
                "orderId": "ORD-005",
                "amount": 75.50
            }
        }))
        .send()
        .await
        .unwrap();

    assert!(response.status().is_success());
    let delivery: serde_json::Value = response.json().await.unwrap();
    let delivery_id = delivery["id"].as_str().unwrap().to_string();

    let delete_response = client
        .delete(format!(
            "{}/event-registry-management/event-deliveries/{}",
            base_url, delivery_id
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert!(
        delete_response.status().is_success(),
        "Delete delivery should succeed: {}",
        delete_response.status()
    );

    let get_response = client
        .get(format!(
            "{}/event-registry-management/event-instance-deliveries/{}",
            base_url, delivery_id
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert!(
        get_response.status().is_client_error(),
        "Deleted delivery should not be found: {}",
        get_response.status()
    );
}
