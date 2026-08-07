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

async fn deploy_event_registry_definitions(client: &reqwest::Client, base_url: &str) -> Value {
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
            "name": "Event Registry Deployment",
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
    response.json().await.unwrap()
}

#[tokio::test]
async fn event_registry_repository_routes_follow_common_rest_contract() {
    let (engine, base_url, client) = spawn_server("rest-event-registry-contract").await;
    let deployment = deploy_event_registry_definitions(&client, &base_url).await;

    let deployments_response = client
        .get(format!(
            "{}/event-registry-repository/deployments?nameLike=%25Registry%25&sort=deployTime&order=desc&start=0&size=1",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(deployments_response.status(), reqwest::StatusCode::OK);
    let deployments_body: Value = deployments_response.json().await.unwrap();
    assert_eq!(deployments_body["start"], 0);
    assert_eq!(deployments_body["size"], 1);
    assert_eq!(deployments_body["total"], 1);
    assert_eq!(deployments_body["data"][0]["id"], deployment["id"]);
    assert_eq!(
        deployments_body["data"][0]["name"],
        "Event Registry Deployment"
    );

    let channel_response = client
        .get(format!(
            "{}/event-registry-repository/channel-definitions?start=0&size=10",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert!(channel_response.status().is_success());
    let channel_body: Value = channel_response.json().await.unwrap();
    assert_eq!(channel_body["start"], 0);
    assert_eq!(channel_body["size"], 2);
    assert_eq!(channel_body["total"], 2);
    let channels = channel_body["data"].as_array().unwrap();
    let inbound_channel = channels
        .iter()
        .find(|item| item["key"] == "ordersInbound")
        .unwrap();
    assert_eq!(inbound_channel["channelType"], "inbound");
    assert_eq!(inbound_channel["type"], "inbound");
    assert_eq!(
        inbound_channel["url"],
        format!(
            "/event-registry-repository/channel-definitions/{}",
            inbound_channel["id"].as_str().unwrap()
        )
    );
    assert_eq!(
        inbound_channel["deploymentUrl"],
        format!(
            "/event-registry-repository/deployments/{}",
            deployment["id"].as_str().unwrap()
        )
    );
    assert_eq!(
        inbound_channel["resource"],
        format!(
            "/event-registry-repository/deployments/{}/resources/orders-inbound.channel",
            deployment["id"].as_str().unwrap()
        )
    );
    assert_eq!(inbound_channel["resourceName"], "orders-inbound.channel");
    assert_eq!(inbound_channel["deploymentId"], deployment["id"]);

    let channel_query_response = client
        .get(format!(
            "{}/event-registry-repository/channel-definitions?deploymentId={}&resourceName=orders-inbound.channel&keyLike=orders%&nameLike=%inbound&onlyInbound=true&sort=deploymentId&order=desc",
            base_url,
            deployment["id"].as_str().unwrap()
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(channel_query_response.status(), reqwest::StatusCode::OK);
    let channel_query_body: Value = channel_query_response.json().await.unwrap();
    assert_eq!(channel_query_body["total"], 1);
    assert_eq!(channel_query_body["data"][0]["key"], "ordersInbound");

    let channel_ignore_case_response = client
        .get(format!(
            "{}/event-registry-repository/channel-definitions?keyLikeIgnoreCase=ORDERS%&nameLikeIgnoreCase=%INBOUND",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(
        channel_ignore_case_response.status(),
        reqwest::StatusCode::OK
    );
    let channel_ignore_case_body: Value = channel_ignore_case_response.json().await.unwrap();
    assert_eq!(channel_ignore_case_body["total"], 1);
    assert_eq!(channel_ignore_case_body["data"][0]["key"], "ordersInbound");

    let inbound_channel_id = inbound_channel["id"].as_str().unwrap().to_string();
    let channel_get_response = client
        .get(format!(
            "{}/event-registry-repository/channel-definitions/{}",
            base_url, inbound_channel_id
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert!(channel_get_response.status().is_success());
    let channel_get_body: Value = channel_get_response.json().await.unwrap();
    assert_eq!(channel_get_body["id"], inbound_channel_id);
    assert_eq!(channel_get_body["key"], "ordersInbound");

    let event_response = client
        .get(format!(
            "{}/event-registry-repository/event-definitions?channelKey=ordersInbound&start=0&size=10",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert!(event_response.status().is_success());
    let event_body: Value = event_response.json().await.unwrap();
    assert_eq!(event_body["start"], 0);
    assert_eq!(event_body["size"], 1);
    assert_eq!(event_body["total"], 1);
    let inbound_event = &event_body["data"][0];
    assert_eq!(inbound_event["eventType"], "order.received");
    assert_eq!(inbound_event["channelKey"], "ordersInbound");
    assert_eq!(inbound_event["deploymentId"], deployment["id"]);
    assert_eq!(
        inbound_event["url"],
        format!(
            "/event-registry-repository/event-definitions/{}",
            inbound_event["id"].as_str().unwrap()
        )
    );
    assert_eq!(
        inbound_event["deploymentUrl"],
        format!(
            "/event-registry-repository/deployments/{}",
            deployment["id"].as_str().unwrap()
        )
    );
    assert_eq!(
        inbound_event["resource"],
        format!(
            "/event-registry-repository/deployments/{}/resources/order-received.event",
            deployment["id"].as_str().unwrap()
        )
    );

    let event_query_response = client
        .get(format!(
            "{}/event-registry-repository/event-definitions?deploymentId={}&resourceName=order-received.event&keyLike=order%&nameLike=%received&sort=deploymentId&order=desc",
            base_url,
            deployment["id"].as_str().unwrap()
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(event_query_response.status(), reqwest::StatusCode::OK);
    let event_query_body: Value = event_query_response.json().await.unwrap();
    assert_eq!(event_query_body["total"], 1);
    assert_eq!(event_query_body["data"][0]["key"], "orderReceived");

    let event_ignore_case_response = client
        .get(format!(
            "{}/event-registry-repository/event-definitions?keyLikeIgnoreCase=ORDER%&nameLikeIgnoreCase=%RECEIVED",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(event_ignore_case_response.status(), reqwest::StatusCode::OK);
    let event_ignore_case_body: Value = event_ignore_case_response.json().await.unwrap();
    assert_eq!(event_ignore_case_body["total"], 1);
    assert_eq!(event_ignore_case_body["data"][0]["key"], "orderReceived");

    let inbound_event_id = inbound_event["id"].as_str().unwrap().to_string();
    let event_get_response = client
        .get(format!(
            "{}/event-registry-repository/event-definitions/{}",
            base_url, inbound_event_id
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert!(event_get_response.status().is_success());
    let event_get_body: Value = event_get_response.json().await.unwrap();
    assert_eq!(event_get_body["id"], inbound_event_id);
    assert_eq!(event_get_body["key"], "orderReceived");

    let channel_model_response = client
        .get(format!(
            "{}/event-registry-repository/channel-definitions/{}/model",
            base_url, inbound_channel_id
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert!(channel_model_response.status().is_success());
    let channel_model_body: Value = channel_model_response.json().await.unwrap();
    assert_eq!(channel_model_body["key"], "ordersInbound");
    assert_eq!(channel_model_body["channelType"], "inbound");
    assert_eq!(channel_model_body["resourceName"], "orders-inbound.channel");

    let event_model_response = client
        .get(format!(
            "{}/event-registry-repository/event-definitions/{}/model",
            base_url, inbound_event_id
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert!(event_model_response.status().is_success());
    let event_model_body: Value = event_model_response.json().await.unwrap();
    assert_eq!(event_model_body["key"], "orderReceived");
    assert_eq!(event_model_body["eventType"], "order.received");
    assert_eq!(event_model_body["channelKey"], "ordersInbound");

    let engine_response = client
        .get(format!("{}/event-registry-management/engine", base_url))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert!(engine_response.status().is_success());
    let engine_body: Value = engine_response.json().await.unwrap();
    assert_eq!(engine_body["name"], engine.get_name());
    assert!(engine_body["version"].is_string());
}

#[tokio::test]
async fn event_registry_deployment_resource_endpoints_return_stored_bytes() {
    let (_engine, base_url, client) = spawn_server("rest-event-registry-resource-data").await;
    let deployment = deploy_event_registry_definitions(&client, &base_url).await;
    let deployment_id = deployment["id"].as_str().unwrap();

    let deployment_response = client
        .get(format!(
            "{}/event-registry-repository/deployments/{}",
            base_url, deployment_id
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(deployment_response.status(), reqwest::StatusCode::OK);
    let deployment_body: Value = deployment_response.json().await.unwrap();
    assert_eq!(deployment_body["id"], deployment_id);
    assert_eq!(deployment_body["resourceNames"][0], "order-published.event");

    let resources = client
        .get(format!(
            "{}/event-registry-repository/deployments/{}/resources",
            base_url, deployment_id
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(resources.status(), reqwest::StatusCode::OK);
    let resources_body: Value = resources.json().await.unwrap();
    assert_eq!(resources_body.as_array().unwrap().len(), 4);
    let inbound_channel = resources_body
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["id"] == "orders-inbound.channel")
        .unwrap();
    assert_eq!(inbound_channel["mediaType"], "application/json");
    assert_eq!(
        inbound_channel["url"],
        format!(
            "/event-registry-repository/deployments/{}/resources/orders-inbound.channel",
            deployment_id
        )
    );
    assert_eq!(
        inbound_channel["contentUrl"],
        format!(
            "/event-registry-repository/deployments/{}/resourcedata/orders-inbound.channel",
            deployment_id
        )
    );

    let resource_data = client
        .get(format!(
            "{}/event-registry-repository/deployments/{}/resourcedata/orders-inbound.channel",
            base_url, deployment_id
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(resource_data.status(), reqwest::StatusCode::OK);
    assert_eq!(
        resource_data
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .unwrap()
            .to_str()
            .unwrap(),
        "application/json"
    );
    let resource_body = resource_data.text().await.unwrap();
    assert!(resource_body.contains("\"key\":\"ordersInbound\""));
    assert!(resource_body.contains("\"destination\":\"orders-inbound\""));

    let resource_metadata = client
        .get(format!(
            "{}/event-registry-repository/deployments/{}/resources/orders-inbound.channel",
            base_url, deployment_id
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(resource_metadata.status(), reqwest::StatusCode::OK);
    let metadata_body: Value = resource_metadata.json().await.unwrap();
    assert_eq!(metadata_body["id"], "orders-inbound.channel");

    let channel_response = client
        .get(format!(
            "{}/event-registry-repository/channel-definitions?key=ordersInbound",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    let channel_body: Value = channel_response.json().await.unwrap();
    let channel_id = channel_body["data"][0]["id"].as_str().unwrap();
    let channel_data = client
        .get(format!(
            "{}/event-registry-repository/channel-definitions/{}/resourcedata",
            base_url, channel_id
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(channel_data.status(), reqwest::StatusCode::OK);
    assert_eq!(channel_data.text().await.unwrap(), resource_body);

    let event_response = client
        .get(format!(
            "{}/event-registry-repository/event-definitions?key=orderReceived",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    let event_body: Value = event_response.json().await.unwrap();
    let event_id = event_body["data"][0]["id"].as_str().unwrap();
    let event_data = client
        .get(format!(
            "{}/event-registry-repository/event-definitions/{}/resourcedata",
            base_url, event_id
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(event_data.status(), reqwest::StatusCode::OK);
    let event_resource = event_data.text().await.unwrap();
    assert!(event_resource.contains("\"key\":\"orderReceived\""));
    assert!(event_resource.contains("\"eventType\":\"order.received\""));
}

#[tokio::test]
async fn event_registry_deployment_delete_matches_repository_contract() {
    let (_engine, base_url, client) = spawn_server("rest-event-registry-delete").await;
    let deployment = deploy_event_registry_definitions(&client, &base_url).await;
    let deployment_id = deployment["id"].as_str().unwrap();

    let delete_response = client
        .delete(format!(
            "{}/event-registry-repository/deployments/{}",
            base_url, deployment_id
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(delete_response.status(), reqwest::StatusCode::NO_CONTENT);
    assert_eq!(delete_response.text().await.unwrap(), "");

    let missing_deployment_response = client
        .get(format!(
            "{}/event-registry-repository/deployments/{}",
            base_url, deployment_id
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(
        missing_deployment_response.status(),
        reqwest::StatusCode::NOT_FOUND
    );

    let removed_event_definitions = client
        .get(format!(
            "{}/event-registry-repository/event-definitions?deploymentId={}",
            base_url, deployment_id
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(removed_event_definitions.status(), reqwest::StatusCode::OK);
    let removed_event_definitions_body: Value = removed_event_definitions.json().await.unwrap();
    assert_eq!(removed_event_definitions_body["total"], 0);

    let removed_channel_definitions = client
        .get(format!(
            "{}/event-registry-repository/channel-definitions?deploymentId={}",
            base_url, deployment_id
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(
        removed_channel_definitions.status(),
        reqwest::StatusCode::OK
    );
    let removed_channel_definitions_body: Value = removed_channel_definitions.json().await.unwrap();
    assert_eq!(removed_channel_definitions_body["total"], 0);

    let removed_resource_response = client
        .get(format!(
            "{}/event-registry-repository/deployments/{}/resources/orders-inbound.channel",
            base_url, deployment_id
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(
        removed_resource_response.status(),
        reqwest::StatusCode::NOT_FOUND
    );

    let missing_delete_response = client
        .delete(format!(
            "{}/event-registry-repository/deployments/does-not-exist",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(
        missing_delete_response.status(),
        reqwest::StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn event_registry_repository_queries_support_metadata_version_and_latest() {
    let (_engine, base_url, client) = spawn_server("rest-event-registry-query-metadata").await;
    deploy_event_registry_definitions(&client, &base_url).await;

    let deploy_v2 = client
        .post(format!(
            "{}/event-registry-repository/deployments",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "Tenant Event Registry Deployment",
            "category": "orders-v2",
            "parentDeploymentId": "parent-orders",
            "tenantId": "tenant-a",
            "resources": [
                {
                    "resourceName": "orders-inbound-v2.channel",
                    "resource": json!({
                        "key": "ordersInbound",
                        "name": "Orders inbound v2",
                        "description": "Inbound orders channel v2",
                        "channelType": "inbound",
                        "resourceName": "orders-inbound-v2.channel",
                        "type": "in-memory",
                        "destination": "orders-inbound-v2",
                        "deserializerType": "json"
                    }).to_string()
                },
                {
                    "resourceName": "order-received-v2.event",
                    "resource": json!({
                        "key": "orderReceived",
                        "name": "Order received v2",
                        "description": "Inbound order event v2",
                        "eventType": "order.received",
                        "channelKey": "ordersInbound",
                        "resourceName": "order-received-v2.event",
                        "payload": [
                            { "name": "orderId", "type": "string" },
                            { "name": "priority", "type": "string" }
                        ]
                    }).to_string()
                }
            ]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(deploy_v2.status(), reqwest::StatusCode::OK);
    let deploy_v2_body: Value = deploy_v2.json().await.unwrap();
    assert_eq!(deploy_v2_body["category"], "orders-v2");
    assert_eq!(deploy_v2_body["tenantId"], "tenant-a");
    assert_eq!(deploy_v2_body["parentDeploymentId"], "parent-orders");

    let deploy_v3 = client
        .post(format!(
            "{}/event-registry-repository/deployments",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "Tenant Event Registry Deployment v3",
            "category": "orders-v3",
            "parentDeploymentId": "parent-orders",
            "tenantId": "tenant-a",
            "resources": [
                {
                    "resourceName": "orders-inbound-v3.channel",
                    "resource": json!({
                        "key": "ordersInbound",
                        "name": "Orders inbound v3",
                        "description": "Inbound orders channel v3",
                        "channelType": "inbound",
                        "resourceName": "orders-inbound-v3.channel",
                        "type": "in-memory",
                        "destination": "orders-inbound-v3",
                        "deserializerType": "json"
                    }).to_string()
                },
                {
                    "resourceName": "order-received-v3.event",
                    "resource": json!({
                        "key": "orderReceived",
                        "name": "Order received v3",
                        "description": "Inbound order event v3",
                        "eventType": "order.received",
                        "channelKey": "ordersInbound",
                        "resourceName": "order-received-v3.event",
                        "payload": [
                            { "name": "orderId", "type": "string" },
                            { "name": "priority", "type": "string" },
                            { "name": "region", "type": "string" }
                        ]
                    }).to_string()
                }
            ]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(deploy_v3.status(), reqwest::StatusCode::OK);

    let tenant_deployments = client
        .get(format!(
            "{}/event-registry-repository/deployments?categoryNotEquals=previous&parentDeploymentIdLike=parent-%25&tenantIdLike=tenant-%25&sort=tenantId&order=asc",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(tenant_deployments.status(), reqwest::StatusCode::OK);
    let tenant_deployments_body: Value = tenant_deployments.json().await.unwrap();
    assert_eq!(tenant_deployments_body["total"], 2);
    assert!(
        tenant_deployments_body["data"]
            .as_array()
            .unwrap()
            .iter()
            .all(|deployment| deployment["tenantId"] == "tenant-a")
    );

    let without_tenant = client
        .get(format!(
            "{}/event-registry-repository/deployments?withoutTenantId=true",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(without_tenant.status(), reqwest::StatusCode::OK);
    let without_tenant_body: Value = without_tenant.json().await.unwrap();
    assert_eq!(without_tenant_body["total"], 1);

    let version_filter = client
        .get(format!(
            "{}/event-registry-repository/event-definitions?key=orderReceived&tenantId=tenant-a&version=1",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(version_filter.status(), reqwest::StatusCode::OK);
    let version_body: Value = version_filter.json().await.unwrap();
    assert_eq!(version_body["total"], 1);
    assert_eq!(version_body["data"][0]["version"], 1);
    assert_eq!(version_body["data"][0]["category"], "orders-v2");

    let latest = client
        .get(format!(
            "{}/event-registry-repository/event-definitions?key=orderReceived&latest=true&sort=version&order=desc",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(latest.status(), reqwest::StatusCode::OK);
    let latest_body: Value = latest.json().await.unwrap();
    assert_eq!(latest_body["total"], 2);
    let latest_definitions = latest_body["data"].as_array().unwrap();
    let tenant_latest = latest_definitions
        .iter()
        .find(|definition| definition["tenantId"] == "tenant-a")
        .unwrap();
    assert_eq!(tenant_latest["version"], 2);
    assert_eq!(tenant_latest["name"], "Order received v3");
    let global_latest = latest_definitions
        .iter()
        .find(|definition| definition["tenantId"].is_null())
        .unwrap();
    assert_eq!(global_latest["version"], 1);

    let metadata_filter = client
        .get(format!(
            "{}/event-registry-repository/event-definitions?categoryLike=orders-%25&categoryNotEquals=orders-v2&parentDeploymentId=parent-orders&tenantIdLike=tenant-%25&sort=category&order=asc",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(metadata_filter.status(), reqwest::StatusCode::OK);
    let metadata_filter_body: Value = metadata_filter.json().await.unwrap();
    assert_eq!(metadata_filter_body["total"], 1);
    assert_eq!(metadata_filter_body["data"][0]["category"], "orders-v3");

    let channel_version_filter = client
        .get(format!(
            "{}/event-registry-repository/channel-definitions?key=ordersInbound&tenantId=tenant-a&version=1",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(channel_version_filter.status(), reqwest::StatusCode::OK);
    let channel_version_body: Value = channel_version_filter.json().await.unwrap();
    assert_eq!(channel_version_body["total"], 1);
    assert_eq!(channel_version_body["data"][0]["version"], 1);
    assert_eq!(channel_version_body["data"][0]["category"], "orders-v2");

    let channel_latest = client
        .get(format!(
            "{}/event-registry-repository/channel-definitions?key=ordersInbound&latest=true&sort=version&order=desc",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(channel_latest.status(), reqwest::StatusCode::OK);
    let channel_latest_body: Value = channel_latest.json().await.unwrap();
    assert_eq!(channel_latest_body["total"], 2);
    let latest_channels = channel_latest_body["data"].as_array().unwrap();
    let tenant_channel_latest = latest_channels
        .iter()
        .find(|definition| definition["tenantId"] == "tenant-a")
        .unwrap();
    assert_eq!(tenant_channel_latest["version"], 2);
    assert_eq!(tenant_channel_latest["name"], "Orders inbound v3");
    let global_channel_latest = latest_channels
        .iter()
        .find(|definition| definition["tenantId"].is_null())
        .unwrap();
    assert_eq!(global_channel_latest["version"], 1);

    let channel_metadata_filter = client
        .get(format!(
            "{}/event-registry-repository/channel-definitions?categoryLike=orders-%25&categoryNotEquals=orders-v2&parentDeploymentId=parent-orders&tenantIdLike=tenant-%25&sort=category&order=asc",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(channel_metadata_filter.status(), reqwest::StatusCode::OK);
    let channel_metadata_filter_body: Value = channel_metadata_filter.json().await.unwrap();
    assert_eq!(channel_metadata_filter_body["total"], 1);
    assert_eq!(
        channel_metadata_filter_body["data"][0]["category"],
        "orders-v3"
    );
}

#[tokio::test]
async fn event_registry_repository_routes_reject_unsupported_filters_and_missing_resources() {
    let (_engine, base_url, client) = spawn_server("rest-event-registry-errors").await;
    deploy_event_registry_definitions(&client, &base_url).await;

    let channel_bad_query = client
        .get(format!(
            "{}/event-registry-repository/channel-definitions?sort=unsupportedSort",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(channel_bad_query.status(), reqwest::StatusCode::BAD_REQUEST);
    let channel_bad_query_body: Value = channel_bad_query.json().await.unwrap();
    assert_eq!(channel_bad_query_body["code"], "BAD_REQUEST");
    assert!(
        channel_bad_query_body["details"]
            .as_str()
            .unwrap()
            .contains("unsupportedSort")
    );

    let channel_bad_sort = client
        .get(format!(
            "{}/event-registry-repository/channel-definitions?onlyInbound=true&onlyOutbound=true",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(channel_bad_sort.status(), reqwest::StatusCode::BAD_REQUEST);
    let channel_bad_sort_body: Value = channel_bad_sort.json().await.unwrap();
    assert_eq!(channel_bad_sort_body["code"], "BAD_REQUEST");
    assert!(
        channel_bad_sort_body["details"]
            .as_str()
            .unwrap()
            .contains("onlyInbound")
    );

    let event_filtered_by_deployment = client
        .get(format!(
            "{}/event-registry-repository/event-definitions?deploymentId=unexpected",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(
        event_filtered_by_deployment.status(),
        reqwest::StatusCode::OK
    );
    let event_filtered_by_deployment_body: Value =
        event_filtered_by_deployment.json().await.unwrap();
    assert_eq!(event_filtered_by_deployment_body["total"], 0);

    let missing_channel = client
        .get(format!(
            "{}/event-registry-repository/channel-definitions/does-not-exist",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(missing_channel.status(), reqwest::StatusCode::NOT_FOUND);
    let missing_channel_body: Value = missing_channel.json().await.unwrap();
    assert_eq!(missing_channel_body["code"], "NOT_FOUND");

    let missing_event = client
        .get(format!(
            "{}/event-registry-repository/event-definitions/does-not-exist",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(missing_event.status(), reqwest::StatusCode::NOT_FOUND);
    let missing_event_body: Value = missing_event.json().await.unwrap();
    assert_eq!(missing_event_body["code"], "NOT_FOUND");

    let bad_deployment_sort = client
        .get(format!(
            "{}/event-registry-repository/deployments?sort=unsupportedEventDeploymentSort",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(
        bad_deployment_sort.status(),
        reqwest::StatusCode::BAD_REQUEST
    );
    let bad_deployment_sort_body: Value = bad_deployment_sort.json().await.unwrap();
    assert_eq!(bad_deployment_sort_body["code"], "BAD_REQUEST");
    assert!(
        bad_deployment_sort_body["details"]
            .as_str()
            .unwrap()
            .contains("unsupportedEventDeploymentSort")
    );
}
