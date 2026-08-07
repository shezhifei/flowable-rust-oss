use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_rest::run_server;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::net::TcpListener;

/// Verifies that the cross-process signal broadcast at `POST /runtime/signals`
/// scopes its delivery to the requested tenant.
///
/// Two process instances on two different tenants are parked on a
/// signal-intermediate-catch event with the same signal name. Firing the
/// signal with `tenantId = "acme"` must advance the acme instance while
/// leaving the other tenant's instance waiting.
#[tokio::test]
async fn runtime_signals_broadcast_scopes_delivery_by_tenant_id() {
    let engine = Arc::new(ProcessEngine::new(
        "rest-bpmn-runtime-signal-tenant".to_string(),
    ));

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
    let addr = listener.local_addr().unwrap();
    let base_url = format!("http://{}", addr);

    let engine_clone = Arc::clone(&engine);
    tokio::spawn(async move {
        run_server(engine_clone, listener).await.unwrap();
    });

    let client = reqwest::Client::new();
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <signal id="tenantAlert" name="Tenant Alert" />
        <process id="tenantSignalProcess" name="Tenant Signal Process" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="f1" sourceRef="start" targetRef="catchAlert" />
            <intermediateCatchEvent id="catchAlert" name="Catch Alert">
                <signalEventDefinition signalRef="tenantAlert" />
            </intermediateCatchEvent>
            <sequenceFlow id="f2" sourceRef="catchAlert" targetRef="afterCatch" />
            <userTask id="afterCatch" name="After Catch" />
            <sequenceFlow id="f3" sourceRef="afterCatch" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#;

    let repository_service = engine.get_repository_service();

    // Deploy the process definition twice — once per tenant.
    repository_service
        .deploy(
            repository_service
                .create_deployment()
                .name("acme deployment".to_string())
                .tenant_id("acme".to_string())
                .add_string("acme_signal.bpmn20.xml".to_string(), xml.to_string()),
        )
        .unwrap();
    repository_service
        .deploy(
            repository_service
                .create_deployment()
                .name("globex deployment".to_string())
                .tenant_id("globex".to_string())
                .add_string("globex_signal.bpmn20.xml".to_string(), xml.to_string()),
        )
        .unwrap();

    let acme_definition = repository_service
        .latest_process_definition_by_key("tenantSignalProcess", Some("acme"))
        .unwrap()
        .expect("acme process definition should exist")
        .id;
    let globex_definition = repository_service
        .latest_process_definition_by_key("tenantSignalProcess", Some("globex"))
        .unwrap()
        .expect("globex process definition should exist")
        .id;

    // Start one process instance on each tenant.
    let acme_instance = start_instance(&client, &base_url, &acme_definition, "acme-1").await;
    let globex_instance = start_instance(&client, &base_url, &globex_definition, "globex-1").await;

    // Both instances should be parked on the intermediate catch event.
    assert_eq!(
        count_tasks_for_instance(&client, &base_url, &acme_instance).await,
        0,
        "acme instance should be waiting on the signal"
    );
    assert_eq!(
        count_tasks_for_instance(&client, &base_url, &globex_instance).await,
        0,
        "globex instance should be waiting on the signal"
    );

    // Fire the signal with the acme tenant scope — only the acme instance
    // should advance; the globex instance must remain waiting.
    let scoped_response = client
        .post(format!("{base_url}/runtime/signals"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "signalName": "Tenant Alert",
            "tenantId": "acme"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(scoped_response.status(), 204);

    assert_eq!(
        count_tasks_for_instance(&client, &base_url, &acme_instance).await,
        1,
        "scoped signal must advance the acme instance to the user task"
    );
    assert_eq!(
        count_tasks_for_instance(&client, &base_url, &globex_instance).await,
        0,
        "scoped signal must NOT advance a different tenant's instance"
    );

    // Now fire the signal without a tenant — both instances should progress.
    let unscoped_response = client
        .post(format!("{base_url}/runtime/signals"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "signalName": "Tenant Alert"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(unscoped_response.status(), 204);

    assert_eq!(
        count_tasks_for_instance(&client, &base_url, &globex_instance).await,
        1,
        "unscoped signal must advance the still-waiting globex instance"
    );

    // Firing the signal with a non-existent tenant is a no-op for both
    // already-completed instances (we only assert that the call succeeds).
    let unknown_tenant_response = client
        .post(format!("{base_url}/runtime/signals"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "signalName": "Tenant Alert",
            "tenantId": "unknown-tenant"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(unknown_tenant_response.status(), 204);
}

async fn start_instance(
    client: &reqwest::Client,
    base_url: &str,
    process_definition_id: &str,
    business_key: &str,
) -> String {
    let response = client
        .post(format!("{base_url}/runtime/process-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "processDefinitionId": process_definition_id,
            "businessKey": business_key,
        }))
        .send()
        .await
        .unwrap();
    assert!(
        response.status().is_success(),
        "start process instance failed: {}",
        response.status()
    );
    let body: Value = response.json().await.unwrap();
    body["id"].as_str().unwrap().to_string()
}

async fn count_tasks_for_instance(
    client: &reqwest::Client,
    base_url: &str,
    process_instance_id: &str,
) -> u64 {
    let response = client
        .post(format!("{base_url}/query/tasks"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "processInstanceId": process_instance_id
        }))
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success());
    let body: Value = response.json().await.unwrap();
    body["total"].as_u64().unwrap_or(0)
}
