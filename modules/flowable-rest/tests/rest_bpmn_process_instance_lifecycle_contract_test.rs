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

fn two_task_process_xml(process_id: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
        <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
            <process id="{process_id}" name="{process_id}" isExecutable="true">
                <startEvent id="start" />
                <sequenceFlow id="flow1" sourceRef="start" targetRef="reviewA" />
                <userTask id="reviewA" name="Review A" />
                <sequenceFlow id="flow2" sourceRef="reviewA" targetRef="reviewB" />
                <userTask id="reviewB" name="Review B" />
                <sequenceFlow id="flow3" sourceRef="reviewB" targetRef="end" />
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
) -> Value {
    let deploy_response = client
        .post(format!("{base_url}/repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": format!("{process_id} deployment"),
            "resourceName": format!("{process_id}.bpmn20.xml"),
            "resource": one_task_process_xml(process_id)
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
        .json(&json!({
            "processDefinitionId": process_definition_id,
            "businessKey": "initial-business-key"
        }))
        .send()
        .await
        .unwrap();
    assert!(start_response.status().is_success());
    start_response.json().await.unwrap()
}

#[tokio::test]
async fn delete_process_instance_terminates_single_runtime_instance() {
    let (engine, base_url, client) = spawn_server("rest-bpmn-pi-delete").await;
    let started = deploy_and_start(&engine, &client, &base_url, "deleteLifecycleProcess").await;
    let process_instance_id = started["id"].as_str().unwrap();

    let delete_response = client
        .delete(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "deleteReason": "rest lifecycle test" }))
        .send()
        .await
        .unwrap();

    assert_eq!(delete_response.status(), reqwest::StatusCode::NO_CONTENT);
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    assert!(
        store
            .find_process_instance(process_instance_id, &mut session)
            .is_none()
    );
    assert!(
        store
            .snapshot_executions(&mut session)
            .values()
            .all(|execution| execution.process_instance_id.as_deref() != Some(process_instance_id))
    );
    let _ = session.rollback();
}

#[tokio::test]
async fn delete_process_instance_accepts_delete_reason_query_parameter() {
    let (engine, base_url, client) = spawn_server("rest-bpmn-pi-delete-query").await;
    let started =
        deploy_and_start(&engine, &client, &base_url, "deleteQueryLifecycleProcess").await;
    let process_instance_id = started["id"].as_str().unwrap();

    let delete_response = client
        .delete(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}?deleteReason=query-reason"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(delete_response.status(), reqwest::StatusCode::NO_CONTENT);

    let historic_response = client
        .get(format!(
            "{base_url}/history/historic-process-instances/{process_instance_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(historic_response.status().is_success());
    let historic_body: Value = historic_response.json().await.unwrap();
    assert_eq!(historic_body["deleteReason"], "query-reason");
}

#[tokio::test]
async fn delete_process_instance_returns_not_found_for_missing_instance() {
    let (_engine, base_url, client) = spawn_server("rest-bpmn-pi-delete-missing").await;

    let response = client
        .delete(format!(
            "{base_url}/runtime/process-instances/missing-instance"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), reqwest::StatusCode::NOT_FOUND);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["code"], "NOT_FOUND");
    assert!(
        body["details"]
            .as_str()
            .unwrap()
            .contains("missing-instance")
    );
}

#[tokio::test]
async fn delete_process_instance_after_delete_returns_not_found() {
    // Mirrors Flowable Java's DeleteProcessInstanceCmd: the `isDeleted()` guard
    // is a transient in-transaction flag. Externally, once the first delete
    // commits the execution row is physically removed, so a second delete must
    // surface as 404 (FlowableObjectNotFoundException), not a silent success.
    let (engine, base_url, client) = spawn_server("rest-bpmn-pi-delete-twice").await;
    let started = deploy_and_start(&engine, &client, &base_url, "deleteTwiceProcess").await;
    let process_instance_id = started["id"].as_str().unwrap();

    let first = client
        .delete(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "deleteReason": "first" }))
        .send()
        .await
        .unwrap();
    assert_eq!(first.status(), reqwest::StatusCode::NO_CONTENT);

    let second = client
        .delete(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "deleteReason": "second" }))
        .send()
        .await
        .unwrap();
    assert_eq!(second.status(), reqwest::StatusCode::NOT_FOUND);
    let body: Value = second.json().await.unwrap();
    assert_eq!(body["code"], "NOT_FOUND");
}

#[tokio::test]
async fn put_process_instance_updates_name_and_business_key() {
    let (engine, base_url, client) = spawn_server("rest-bpmn-pi-update").await;
    let started = deploy_and_start(&engine, &client, &base_url, "updateLifecycleProcess").await;
    let process_instance_id = started["id"].as_str().unwrap();

    let update_response = client
        .put(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "action": "update",
            "name": "Updated instance name",
            "businessKey": "updated-business-key",
            "businessStatus": "customer-waiting",
            "callbackId": "callback-42",
            "callbackType": "order-callback",
            "referenceId": "reference-99",
            "referenceType": "order"
        }))
        .send()
        .await
        .unwrap();

    assert!(update_response.status().is_success());
    let update_body: Value = update_response.json().await.unwrap();
    assert_eq!(update_body["name"], "Updated instance name");
    assert_eq!(update_body["businessKey"], "updated-business-key");
    assert_eq!(update_body["businessStatus"], "customer-waiting");
    assert_eq!(update_body["callbackId"], "callback-42");
    assert_eq!(update_body["callbackType"], "order-callback");
    assert_eq!(update_body["referenceId"], "reference-99");
    assert_eq!(update_body["referenceType"], "order");
    assert_eq!(update_body["isSuspended"], false);

    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let stored = store
        .find_process_instance(process_instance_id, &mut session)
        .unwrap();
    assert_eq!(stored.name.as_deref(), Some("Updated instance name"));
    assert_eq!(stored.business_key.as_deref(), Some("updated-business-key"));
    assert_eq!(stored.business_status.as_deref(), Some("customer-waiting"));
    assert_eq!(stored.callback_id.as_deref(), Some("callback-42"));
    assert_eq!(stored.callback_type.as_deref(), Some("order-callback"));
    assert_eq!(stored.reference_id.as_deref(), Some("reference-99"));
    assert_eq!(stored.reference_type.as_deref(), Some("order"));
    let _ = session.rollback();

    let get_response = client
        .get(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(get_response.status().is_success());
    let get_body: Value = get_response.json().await.unwrap();
    assert_eq!(get_body["businessStatus"], "customer-waiting");
    assert_eq!(get_body["callbackId"], "callback-42");
    assert_eq!(get_body["callbackType"], "order-callback");
    assert_eq!(get_body["referenceId"], "reference-99");
    assert_eq!(get_body["referenceType"], "order");
}

#[tokio::test]
async fn put_process_instance_can_clear_name_and_business_key() {
    let (engine, base_url, client) = spawn_server("rest-bpmn-pi-clear-fields").await;
    let started = deploy_and_start(&engine, &client, &base_url, "clearLifecycleProcess").await;
    let process_instance_id = started["id"].as_str().unwrap();

    let set_response = client
        .put(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "Temporary name",
            "businessKey": "temporary-business-key",
            "businessStatus": "temporary-status",
            "callbackId": "temporary-callback",
            "callbackType": "temporary-callback-type",
            "referenceId": "temporary-reference",
            "referenceType": "temporary-reference-type"
        }))
        .send()
        .await
        .unwrap();
    assert!(set_response.status().is_success());

    let clear_response = client
        .put(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "action": "update",
            "name": null,
            "businessKey": null,
            "businessStatus": null,
            "callbackId": null,
            "callbackType": null,
            "referenceId": null,
            "referenceType": null
        }))
        .send()
        .await
        .unwrap();

    assert!(clear_response.status().is_success());
    let clear_body: Value = clear_response.json().await.unwrap();
    assert!(clear_body["name"].is_null());
    assert!(clear_body["businessKey"].is_null());
    assert!(clear_body.get("businessStatus").is_none());
    assert!(clear_body.get("callbackId").is_none());
    assert!(clear_body.get("callbackType").is_none());
    assert!(clear_body.get("referenceId").is_none());
    assert!(clear_body.get("referenceType").is_none());

    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let stored = store
        .find_process_instance(process_instance_id, &mut session)
        .unwrap();
    assert!(stored.name.is_none());
    assert!(stored.business_key.is_none());
    assert!(stored.business_status.is_none());
    assert!(stored.callback_id.is_none());
    assert!(stored.callback_type.is_none());
    assert!(stored.reference_id.is_none());
    assert!(stored.reference_type.is_none());
    let _ = session.rollback();
}

#[tokio::test]
async fn change_state_accepts_move_activity_id_to_shape() {
    let (engine, base_url, client) = spawn_server("rest-bpmn-pi-change-state").await;
    let process_id = "moveActivityLifecycleProcess";
    let deploy_response = client
        .post(format!("{base_url}/repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": format!("{process_id} deployment"),
            "resourceName": format!("{process_id}.bpmn20.xml"),
            "resource": two_task_process_xml(process_id)
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
        .json(&json!({
            "processDefinitionId": process_definition_id,
            "businessKey": "change-state-instance"
        }))
        .send()
        .await
        .unwrap();
    assert!(start_response.status().is_success());
    let start_body: Value = start_response.json().await.unwrap();
    let process_instance_id = start_body["id"].as_str().unwrap();

    let change_response = client
        .post(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}/change-state"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "moveActivityIdTo": {
                "reviewA": "reviewB"
            }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(change_response.status(), reqwest::StatusCode::OK);

    let tasks_response = client
        .post(format!("{base_url}/query/tasks"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "processInstanceId": process_instance_id
        }))
        .send()
        .await
        .unwrap();
    assert!(tasks_response.status().is_success());
    let tasks_body: Value = tasks_response.json().await.unwrap();
    assert_eq!(tasks_body["total"], 1);
    assert_eq!(tasks_body["data"][0]["name"], "Review B");
    assert_eq!(tasks_body["data"][0]["taskDefinitionKey"], "reviewB");
}

#[tokio::test]
async fn put_process_instance_suspends_and_activates_instance_and_executions() {
    let (engine, base_url, client) = spawn_server("rest-bpmn-pi-suspend-activate").await;
    let started = deploy_and_start(&engine, &client, &base_url, "suspendLifecycleProcess").await;
    let process_instance_id = started["id"].as_str().unwrap();

    let suspend_response = client
        .put(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "action": "suspend" }))
        .send()
        .await
        .unwrap();

    assert!(suspend_response.status().is_success());
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    assert!(
        store
            .find_process_instance(process_instance_id, &mut session)
            .unwrap()
            .is_suspended
    );
    assert!(
        store
            .snapshot_executions(&mut session)
            .values()
            .filter(|execution| {
                execution.process_instance_id.as_deref() == Some(process_instance_id)
            })
            .all(|execution| execution.is_suspended)
    );
    let _ = session.rollback();

    let activate_response = client
        .put(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "action": "activate" }))
        .send()
        .await
        .unwrap();

    assert!(activate_response.status().is_success());
    let mut session = store.create_session().unwrap();
    assert!(
        !store
            .find_process_instance(process_instance_id, &mut session)
            .unwrap()
            .is_suspended
    );
    assert!(
        store
            .snapshot_executions(&mut session)
            .values()
            .filter(|execution| {
                execution.process_instance_id.as_deref() == Some(process_instance_id)
            })
            .all(|execution| !execution.is_suspended)
    );
    let _ = session.rollback();
}

#[tokio::test]
async fn put_process_instance_rejects_activate_when_already_active() {
    let (engine, base_url, client) = spawn_server("rest-bpmn-pi-activate-active").await;
    let started = deploy_and_start(
        &engine,
        &client,
        &base_url,
        "activateActiveLifecycleProcess",
    )
    .await;
    let process_instance_id = started["id"].as_str().unwrap();

    let response = client
        .put(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "action": "activate" }))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), reqwest::StatusCode::CONFLICT);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["code"], "CONFLICT");
    assert!(
        body["details"]
            .as_str()
            .unwrap()
            .contains("is already active")
    );
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    assert!(
        !store
            .find_process_instance(process_instance_id, &mut session)
            .unwrap()
            .is_suspended
    );
    let _ = session.rollback();
}

#[tokio::test]
async fn put_process_instance_rejects_suspend_when_already_suspended() {
    let (engine, base_url, client) = spawn_server("rest-bpmn-pi-suspend-suspended").await;
    let started = deploy_and_start(
        &engine,
        &client,
        &base_url,
        "suspendSuspendedLifecycleProcess",
    )
    .await;
    let process_instance_id = started["id"].as_str().unwrap();

    let first_suspend_response = client
        .put(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "action": "suspend" }))
        .send()
        .await
        .unwrap();
    assert!(first_suspend_response.status().is_success());

    let duplicate_suspend_response = client
        .put(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "action": "suspend" }))
        .send()
        .await
        .unwrap();

    assert_eq!(
        duplicate_suspend_response.status(),
        reqwest::StatusCode::CONFLICT
    );
    let body: Value = duplicate_suspend_response.json().await.unwrap();
    assert_eq!(body["code"], "CONFLICT");
    assert!(
        body["details"]
            .as_str()
            .unwrap()
            .contains("is already suspended")
    );
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    assert!(
        store
            .find_process_instance(process_instance_id, &mut session)
            .unwrap()
            .is_suspended
    );
    assert!(
        store
            .snapshot_executions(&mut session)
            .values()
            .filter(|execution| {
                execution.process_instance_id.as_deref() == Some(process_instance_id)
            })
            .all(|execution| execution.is_suspended)
    );
    let _ = session.rollback();
}

#[tokio::test]
async fn put_process_instance_rejects_unknown_fields() {
    let (engine, base_url, client) = spawn_server("rest-bpmn-pi-unknown-field").await;
    let started =
        deploy_and_start(&engine, &client, &base_url, "unknownFieldLifecycleProcess").await;
    let process_instance_id = started["id"].as_str().unwrap();

    let response = client
        .put(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "action": "update",
            "name": "Updated",
            "unexpectedField": true
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["code"], "BAD_REQUEST");
    assert!(
        body["details"]
            .as_str()
            .unwrap()
            .contains("unexpectedField")
    );
}

#[tokio::test]
async fn put_process_instance_rejects_unknown_action() {
    let (engine, base_url, client) = spawn_server("rest-bpmn-pi-unknown-action").await;
    let started =
        deploy_and_start(&engine, &client, &base_url, "unknownActionLifecycleProcess").await;
    let process_instance_id = started["id"].as_str().unwrap();

    let response = client
        .put(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "action": "restart" }))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["code"], "BAD_REQUEST");
    assert!(
        body["details"]
            .as_str()
            .unwrap()
            .contains("Illegal action: 'restart'")
    );
}

#[tokio::test]
async fn put_process_instance_rejects_ended_instance() {
    let (engine, base_url, client) = spawn_server("rest-bpmn-pi-ended").await;
    let started = deploy_and_start(&engine, &client, &base_url, "endedLifecycleProcess").await;
    let process_instance_id = started["id"].as_str().unwrap();
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let mut stored = store
        .find_process_instance(process_instance_id, &mut session)
        .unwrap();
    stored.is_ended = true;
    store.update_process_instance(&stored, &mut session);
    session.flush_and_commit().unwrap();

    let response = client
        .put(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "action": "update",
            "name": "Cannot update ended"
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["code"], "BAD_REQUEST");
    assert!(
        body["details"]
            .as_str()
            .unwrap()
            .contains("Cannot update ended process instance")
    );
}
