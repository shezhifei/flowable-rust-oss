use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_rest::run_server;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::net::TcpListener;

async fn spawn_server(test_name: &str) -> (Arc<ProcessEngine>, String, reqwest::Client) {
    let engine = Arc::new(ProcessEngine::new(test_name.to_string()));

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
            { "name": "orderId", "type": "string" }
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
            "name": "Event Registry Runtime Deployment",
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

    assert!(response.status().is_success());
}

async fn deploy_tenant_event_registry_definitions(client: &reqwest::Client, base_url: &str) {
    for (deployment_name, tenant_id, resource_prefix, required_field) in [
        (
            "Global Tenant Runtime v1",
            None,
            "global-tenant-runtime-v1",
            "globalOnly",
        ),
        (
            "Global Tenant Runtime v2",
            None,
            "global-tenant-runtime-v2",
            "globalOnly",
        ),
        (
            "Tenant A Runtime",
            Some("tenant-a"),
            "tenant-a-runtime",
            "tenantOnly",
        ),
    ] {
        let inbound_channel = json!({
            "key": "tenantAwareRestOrdersInbound",
            "name": format!("{deployment_name} inbound"),
            "channelType": "inbound",
            "resourceName": format!("{resource_prefix}-inbound.channel"),
            "type": "in-memory",
            "destination": format!("{resource_prefix}-inbound"),
            "deserializerType": "json"
        });

        let outbound_channel = json!({
            "key": "tenantAwareRestOrdersOutbound",
            "name": format!("{deployment_name} outbound"),
            "channelType": "outbound",
            "resourceName": format!("{resource_prefix}-outbound.channel"),
            "type": "in-memory",
            "destination": format!("{resource_prefix}-outbound"),
            "serializerType": "json"
        });

        let inbound_event = json!({
            "key": "tenantAwareRestOrderReceived",
            "name": format!("{deployment_name} received"),
            "eventType": "tenant.aware.rest.order.received",
            "channelKey": "tenantAwareRestOrdersInbound",
            "resourceName": format!("{resource_prefix}-received.event"),
            "payload": [
                { "name": required_field, "type": "string", "required": true }
            ]
        });

        let outbound_event = json!({
            "key": "tenantAwareRestOrderPublished",
            "name": format!("{deployment_name} published"),
            "eventType": "tenant.aware.rest.order.published",
            "channelKey": "tenantAwareRestOrdersOutbound",
            "resourceName": format!("{resource_prefix}-published.event"),
            "payload": [
                { "name": required_field, "type": "string", "required": true }
            ]
        });

        let mut deployment = json!({
            "name": deployment_name,
            "resources": [
                {
                    "resourceName": format!("{resource_prefix}-inbound.channel"),
                    "resource": inbound_channel.to_string()
                },
                {
                    "resourceName": format!("{resource_prefix}-outbound.channel"),
                    "resource": outbound_channel.to_string()
                },
                {
                    "resourceName": format!("{resource_prefix}-received.event"),
                    "resource": inbound_event.to_string()
                },
                {
                    "resourceName": format!("{resource_prefix}-published.event"),
                    "resource": outbound_event.to_string()
                }
            ]
        });
        if let Some(tenant_id) = tenant_id {
            deployment["tenantId"] = json!(tenant_id);
        }

        let response = client
            .post(format!(
                "{}/event-registry-repository/deployments",
                base_url
            ))
            .basic_auth("admin", Some("test"))
            .json(&deployment)
            .send()
            .await
            .unwrap();

        assert!(response.status().is_success());
    }
}

#[tokio::test]
async fn event_registry_runtime_event_instances_accepts_inbound_contract() {
    let (_engine, base_url, client) = spawn_server("rest-event-registry-runtime-contract").await;
    deploy_event_registry_definitions(&client, &base_url).await;

    let event_instance_response = client
        .post(format!(
            "{}/event-registry-runtime/event-instances",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "eventDefinitionKey": "orderReceived",
            "channelDefinitionKey": "ordersInbound",
            "eventPayload": {
                "orderId": "A-200"
            }
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(
        event_instance_response.status(),
        reqwest::StatusCode::NO_CONTENT
    );
    assert_eq!(event_instance_response.text().await.unwrap(), "");

    let deliveries_response = client
        .get(format!(
            "{}/event-registry-management/event-instance-deliveries?direction=inbound&status=PROCESSED&eventType=order.received&channelKey=ordersInbound",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(deliveries_response.status(), reqwest::StatusCode::OK);
    let deliveries_body: Value = deliveries_response.json().await.unwrap();
    assert_eq!(deliveries_body["total"], 1);
    assert_eq!(
        deliveries_body["data"][0]["eventDefinitionKey"],
        "orderReceived"
    );
    assert_eq!(deliveries_body["data"][0]["direction"], "inbound");
    assert_eq!(deliveries_body["data"][0]["status"], "PROCESSED");
}

#[tokio::test]
async fn event_registry_runtime_and_management_routes_cover_inbound_and_outbound_processing() {
    let (_engine, base_url, client) = spawn_server("rest-event-registry-runtime").await;
    deploy_event_registry_definitions(&client, &base_url).await;

    let inbound_response = client
        .post(format!(
            "{}/event-registry-runtime/inbound-event-instances",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "eventType": "order.received",
            "eventPayload": {
                "orderId": "A-100"
            }
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(inbound_response.status(), reqwest::StatusCode::CREATED);
    let inbound_body: Value = inbound_response.json().await.unwrap();
    assert_eq!(inbound_body["direction"], "inbound");
    assert_eq!(inbound_body["status"], "PROCESSED");
    assert_eq!(inbound_body["eventType"], "order.received");
    assert_eq!(inbound_body["channelKey"], "ordersInbound");

    let outbound_response = client
        .post(format!(
            "{}/event-registry-runtime/event-instances",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "eventDefinitionKey": "orderPublished",
            "eventPayload": {
                "orderId": "A-100"
            }
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(outbound_response.status(), reqwest::StatusCode::CREATED);
    let outbound_body: Value = outbound_response.json().await.unwrap();
    assert_eq!(outbound_body["direction"], "outbound");
    assert_eq!(outbound_body["status"], "PUBLISHED");
    assert_eq!(outbound_body["eventType"], "order.published");
    assert_eq!(outbound_body["channelKey"], "ordersOutbound");

    let deliveries_response = client
        .get(format!(
            "{}/event-registry-management/event-instance-deliveries?start=0&size=10",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert!(deliveries_response.status().is_success());
    let deliveries_body: Value = deliveries_response.json().await.unwrap();
    assert_eq!(deliveries_body["start"], 0);
    assert_eq!(deliveries_body["size"], 2);
    assert_eq!(deliveries_body["total"], 2);

    let outbound_delivery_id = outbound_body["id"].as_str().unwrap();
    let outbound_delivery = client
        .get(format!(
            "{}/event-registry-management/event-instance-deliveries/{}",
            base_url, outbound_delivery_id
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert!(outbound_delivery.status().is_success());
    let outbound_delivery_body: Value = outbound_delivery.json().await.unwrap();
    assert_eq!(outbound_delivery_body["id"], outbound_body["id"]);
    assert_eq!(outbound_delivery_body["status"], "PUBLISHED");
    assert_eq!(
        outbound_delivery_body["statusHistory"],
        json!(["CREATED", "PUBLISHED"])
    );

    let filtered_deliveries = client
        .get(format!(
            "{}/event-registry-management/event-instance-deliveries?direction=outbound&status=PUBLISHED&start=0&size=10",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert!(filtered_deliveries.status().is_success());
    let filtered_deliveries_body: Value = filtered_deliveries.json().await.unwrap();
    assert_eq!(filtered_deliveries_body["total"], 1);
    assert_eq!(
        filtered_deliveries_body["data"][0]["id"],
        outbound_body["id"]
    );

    let case_filtered_deliveries = client
        .get(format!(
            "{}/event-registry-management/event-instance-deliveries?direction=OUTBOUND&status=published&start=0&size=10",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert!(case_filtered_deliveries.status().is_success());
    let case_filtered_deliveries_body: Value = case_filtered_deliveries.json().await.unwrap();
    assert_eq!(case_filtered_deliveries_body["total"], 1);
    assert_eq!(
        case_filtered_deliveries_body["data"][0]["id"],
        outbound_body["id"]
    );
}

#[tokio::test]
async fn event_registry_runtime_routes_resolve_tenant_specific_inbound_and_outbound_definitions() {
    let (_engine, base_url, client) =
        spawn_server("rest-event-registry-runtime-tenant-resolution").await;
    deploy_tenant_event_registry_definitions(&client, &base_url).await;

    let inbound_response = client
        .post(format!(
            "{}/event-registry-runtime/inbound-event-instances",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "eventType": "tenant.aware.rest.order.received",
            "eventPayload": {
                "tenantOnly": "T-500"
            },
            "tenantId": "tenant-a"
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(inbound_response.status(), reqwest::StatusCode::CREATED);
    let inbound_body: Value = inbound_response.json().await.unwrap();
    assert_eq!(inbound_body["direction"], "inbound");
    assert_eq!(inbound_body["status"], "PROCESSED");
    assert_eq!(
        inbound_body["statusHistory"],
        json!(["RECEIVED", "PROCESSED"])
    );
    assert_eq!(
        inbound_body["eventDefinitionKey"],
        "tenantAwareRestOrderReceived"
    );

    let outbound_response = client
        .post(format!(
            "{}/event-registry-runtime/event-instances",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "eventDefinitionKey": "tenantAwareRestOrderPublished",
            "eventPayload": {
                "tenantOnly": "T-600"
            },
            "tenantId": "tenant-a"
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(outbound_response.status(), reqwest::StatusCode::CREATED);
    let outbound_body: Value = outbound_response.json().await.unwrap();
    assert_eq!(outbound_body["direction"], "outbound");
    assert_eq!(outbound_body["status"], "PUBLISHED");
    assert_eq!(
        outbound_body["statusHistory"],
        json!(["CREATED", "PUBLISHED"])
    );
    assert_eq!(
        outbound_body["eventDefinitionKey"],
        "tenantAwareRestOrderPublished"
    );

    let deliveries_response = client
        .get(format!(
            "{}/event-registry-management/event-instance-deliveries?start=0&size=10",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(deliveries_response.status(), reqwest::StatusCode::OK);
    let deliveries_body: Value = deliveries_response.json().await.unwrap();
    assert_eq!(deliveries_body["total"], 2);
}

#[tokio::test]
async fn event_registry_runtime_routes_return_structured_errors_without_bpmn_fallbacks() {
    let (_engine, base_url, client) = spawn_server("rest-event-registry-runtime-errors").await;
    deploy_event_registry_definitions(&client, &base_url).await;

    let missing_inbound_definition = client
        .post(format!(
            "{}/event-registry-runtime/inbound-event-instances",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "eventType": "does.not.exist",
            "eventPayload": {
                "orderId": "missing"
            }
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(
        missing_inbound_definition.status(),
        reqwest::StatusCode::NOT_FOUND
    );
    let missing_inbound_definition_body: Value = missing_inbound_definition.json().await.unwrap();
    assert_eq!(missing_inbound_definition_body["code"], "NOT_FOUND");

    let missing_outbound_definition = client
        .post(format!(
            "{}/event-registry-runtime/event-instances",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "eventDefinitionKey": "missingDefinition",
            "eventPayload": {
                "orderId": "missing"
            }
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(
        missing_outbound_definition.status(),
        reqwest::StatusCode::NOT_FOUND
    );
    let missing_outbound_definition_body: Value = missing_outbound_definition.json().await.unwrap();
    assert_eq!(missing_outbound_definition_body["code"], "NOT_FOUND");

    let wrong_channel_type = client
        .post(format!(
            "{}/event-registry-runtime/inbound-event-instances",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "eventType": "order.published",
            "eventPayload": {
                "orderId": "wrong-channel"
            }
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(
        wrong_channel_type.status(),
        reqwest::StatusCode::BAD_REQUEST
    );
    let wrong_channel_type_body: Value = wrong_channel_type.json().await.unwrap();
    assert_eq!(wrong_channel_type_body["code"], "BAD_REQUEST");
    assert!(
        wrong_channel_type_body["details"]
            .as_str()
            .unwrap()
            .contains("inbound")
    );

    let unsupported_management_filter = client
        .get(format!(
            "{}/event-registry-management/event-instance-deliveries?eventDefinitionKey=orderPublished",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(
        unsupported_management_filter.status(),
        reqwest::StatusCode::BAD_REQUEST
    );
    let unsupported_management_filter_body: Value =
        unsupported_management_filter.json().await.unwrap();
    assert_eq!(unsupported_management_filter_body["code"], "BAD_REQUEST");
    assert!(
        unsupported_management_filter_body["details"]
            .as_str()
            .unwrap()
            .contains("eventDefinitionKey")
    );

    let missing_delivery = client
        .get(format!(
            "{}/event-registry-management/event-instance-deliveries/does-not-exist",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(missing_delivery.status(), reqwest::StatusCode::NOT_FOUND);
    let missing_delivery_body: Value = missing_delivery.json().await.unwrap();
    assert_eq!(missing_delivery_body["code"], "NOT_FOUND");
}

async fn deploy_pipeline_channel(
    client: &reqwest::Client,
    base_url: &str,
    channel_extra: Value,
) {
    let mut channel = json!({
        "key": "pipelineInbound",
        "name": "Pipeline inbound",
        "channelType": "inbound",
        "resourceName": "pipeline-inbound.channel",
        "type": "in-memory",
        "destination": "pipeline-inbound",
        "deserializerType": "json"
    });
    if let (Some(object), Some(extra)) = (channel.as_object_mut(), channel_extra.as_object()) {
        for (key, value) in extra {
            object.insert(key.clone(), value.clone());
        }
    }
    let event = json!({
        "key": "pipelineEvent",
        "name": "Pipeline event",
        "eventType": "pipeline.event",
        "channelKey": "pipelineInbound",
        "resourceName": "pipeline-event.event",
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
            "name": "pipeline deployment",
            "resources": [
                {
                    "resourceName": "pipeline-inbound.channel",
                    "resource": channel.to_string()
                },
                {
                    "resourceName": "pipeline-event.event",
                    "resource": event.to_string()
                }
            ]
        }))
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success());
}

#[tokio::test]
async fn rest_inbound_channel_pipeline_filter_failure_and_delivery_status() {
    let (_engine, base_url, client) =
        spawn_server("rest-event-registry-pipeline-filter").await;
    deploy_pipeline_channel(&client, &base_url, json!({ "rejectAll": true })).await;

    let response = client
        .post(format!(
            "{}/event-registry-runtime/inbound-event-instances",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "channelKey": "pipelineInbound",
            "eventPayload": { "orderId": "X", "eventKey": "pipelineEvent" },
            "headers": { "eventKey": "pipelineEvent" }
        }))
        .send()
        .await
        .unwrap();

    assert!(
        response.status().is_client_error(),
        "filter rejection should surface as client error"
    );
    let body: Value = response.json().await.unwrap();
    let details = body["details"].as_str().unwrap_or_default().to_lowercase();
    assert!(details.contains("filter"), "details={details}");

    let deliveries = client
        .get(format!(
            "{}/event-registry-management/event-instance-deliveries?start=0&size=10",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(deliveries["total"], 0);
}

#[tokio::test]
async fn rest_inbound_channel_pipeline_transform_failure() {
    let (_engine, base_url, client) =
        spawn_server("rest-event-registry-pipeline-transform").await;
    deploy_pipeline_channel(&client, &base_url, json!({ "failTransform": true })).await;

    let response = client
        .post(format!(
            "{}/event-registry-runtime/inbound-event-instances",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "channelKey": "pipelineInbound",
            "eventPayload": { "orderId": "X" },
            "headers": { "eventKey": "pipelineEvent" }
        }))
        .send()
        .await
        .unwrap();

    assert!(response.status().is_client_error() || response.status().is_server_error());
    let body: Value = response.json().await.unwrap();
    let details = body["details"].as_str().unwrap_or_default();
    if body["code"] == "INTERNAL_SERVER_ERROR" {
        // 5xx details are generic (no pipeline/transform internals).
        assert_eq!(details, "Internal server error");
    } else {
        assert!(
            details.to_lowercase().contains("transform"),
            "details={details}"
        );
    }
}

#[tokio::test]
async fn rest_inbound_channel_pipeline_key_detection_failure() {
    let (_engine, base_url, client) =
        spawn_server("rest-event-registry-pipeline-key").await;
    deploy_pipeline_channel(&client, &base_url, json!({})).await;

    let response = client
        .post(format!(
            "{}/event-registry-runtime/inbound-event-instances",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "channelKey": "pipelineInbound",
            "eventPayload": { "orderId": "X" }
        }))
        .send()
        .await
        .unwrap();

    assert!(response.status().is_client_error() || response.status().is_server_error());
    let body: Value = response.json().await.unwrap();
    let details = body["details"].as_str().unwrap_or_default();
    if body["code"] == "INTERNAL_SERVER_ERROR" {
        // 5xx details are generic (no key-detection internals).
        assert_eq!(details, "Internal server error");
    } else {
        assert!(
            details.to_lowercase().contains("event key") || details.to_lowercase().contains("key"),
            "details={details}"
        );
    }
}

#[tokio::test]
async fn rest_outbound_publish_exposes_dispatch_token_and_delivery_status() {
    let (_engine, base_url, client) =
        spawn_server("rest-event-registry-outbound-dispatch-token").await;
    deploy_event_registry_definitions(&client, &base_url).await;

    let outbound_response = client
        .post(format!(
            "{}/event-registry-runtime/event-instances",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "eventDefinitionKey": "orderPublished",
            "eventPayload": { "orderId": "token-1" }
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(outbound_response.status(), reqwest::StatusCode::CREATED);
    let body: Value = outbound_response.json().await.unwrap();
    assert_eq!(body["status"], "PUBLISHED");
    assert!(
        body["dispatchToken"].as_str().is_some_and(|token| token.starts_with("dispatch:")),
        "dispatchToken should be present: {body}"
    );

    let delivery_id = body["id"].as_str().unwrap();
    let get_delivery = client
        .get(format!(
            "{}/event-registry-management/event-instance-deliveries/{}",
            base_url, delivery_id
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(get_delivery.status().is_success());
    let delivery: Value = get_delivery.json().await.unwrap();
    assert_eq!(delivery["status"], "PUBLISHED");
    assert_eq!(delivery["dispatchToken"], body["dispatchToken"]);
}

#[tokio::test]
async fn rest_inbound_channel_pipeline_success_and_status_retrieval() {
    let (_engine, base_url, client) =
        spawn_server("rest-event-registry-pipeline-success").await;
    deploy_pipeline_channel(&client, &base_url, json!({})).await;

    let response = client
        .post(format!(
            "{}/event-registry-runtime/inbound-event-instances",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "channelKey": "pipelineInbound",
            "eventPayload": { "orderId": "ok" },
            "headers": { "eventKey": "pipelineEvent" }
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), reqwest::StatusCode::CREATED);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["status"], "PROCESSED");
    assert_eq!(body["channelKey"], "pipelineInbound");
    assert_eq!(
        body["statusHistory"],
        json!(["RECEIVED", "PROCESSED"])
    );

    let delivery_id = body["id"].as_str().unwrap();
    let get_delivery = client
        .get(format!(
            "{}/event-registry-management/event-instance-deliveries/{}",
            base_url, delivery_id
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(get_delivery.status().is_success());
    let delivery: Value = get_delivery.json().await.unwrap();
    assert_eq!(delivery["status"], "PROCESSED");
    assert_eq!(delivery["eventDefinitionKey"], "pipelineEvent");
}
