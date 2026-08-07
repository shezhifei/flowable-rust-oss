use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::identity::entities::User;
use flowable_rest::run_server;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::net::TcpListener;

const PROCESS_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
    <process id="definitionActionProcess" name="Definition Action Process" isExecutable="true">
        <startEvent id="start" />
        <sequenceFlow id="flow1" sourceRef="start" targetRef="end" />
        <endEvent id="end" />
    </process>
</definitions>"#;

const USER_TASK_PROCESS_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
    <process id="definitionActionUserTaskProcess" name="Definition Action User Task Process" isExecutable="true">
        <startEvent id="start" />
        <sequenceFlow id="flow1" sourceRef="start" targetRef="task1" />
        <userTask id="task1" name="Task 1" />
        <sequenceFlow id="flow2" sourceRef="task1" targetRef="end" />
        <endEvent id="end" />
    </process>
</definitions>"#;

async fn spawn_server() -> (Arc<ProcessEngine>, String, reqwest::Client) {
    let engine = Arc::new(ProcessEngine::new(
        "rest-bpmn-process-definition-actions".to_string(),
    ));
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
    let engine_for_server = Arc::clone(&engine);
    tokio::spawn(async move {
        run_server(engine_for_server, listener).await.unwrap();
    });

    (engine, base_url, reqwest::Client::new())
}

async fn deploy_process(
    engine: &ProcessEngine,
    base_url: &str,
    client: &reqwest::Client,
) -> String {
    let deploy_response = client
        .post(format!("{base_url}/repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "Definition actions",
            "resourceName": "definition-actions.bpmn20.xml",
            "resource": PROCESS_XML
        }))
        .send()
        .await
        .unwrap();
    assert!(deploy_response.status().is_success());

    engine
        .get_repository_service()
        .latest_process_definition_by_key("definitionActionProcess", None)
        .unwrap()
        .unwrap()
        .id
}

async fn deploy_user_task_process(
    engine: &ProcessEngine,
    base_url: &str,
    client: &reqwest::Client,
) -> String {
    let deploy_response = client
        .post(format!("{base_url}/repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "Definition actions user task",
            "resourceName": "definition-actions-user-task.bpmn20.xml",
            "resource": USER_TASK_PROCESS_XML
        }))
        .send()
        .await
        .unwrap();
    assert!(deploy_response.status().is_success());

    engine
        .get_repository_service()
        .latest_process_definition_by_key("definitionActionUserTaskProcess", None)
        .unwrap()
        .unwrap()
        .id
}

async fn start_process_instance(
    base_url: &str,
    client: &reqwest::Client,
    process_definition_id: &str,
) -> String {
    let start_response = client
        .post(format!("{base_url}/runtime/process-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "processDefinitionId": process_definition_id }))
        .send()
        .await
        .unwrap();
    assert!(start_response.status().is_success());
    let body: Value = start_response.json().await.unwrap();
    body["id"].as_str().unwrap().to_string()
}

async fn put_definition_action(
    base_url: &str,
    client: &reqwest::Client,
    process_definition_id: &str,
    payload: Value,
) -> reqwest::Response {
    client
        .put(format!(
            "{base_url}/repository/process-definitions/{process_definition_id}"
        ))
        .basic_auth("admin", Some("test"))
        .json(&payload)
        .send()
        .await
        .unwrap()
}

async fn process_instance_is_suspended(
    base_url: &str,
    client: &reqwest::Client,
    process_instance_id: &str,
) -> bool {
    let response = client
        .get(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let body: Value = response.json().await.unwrap();
    body["isSuspended"].as_bool().unwrap()
}

#[tokio::test]
async fn process_definition_put_updates_category_and_suspension_state() {
    let (engine, base_url, client) = spawn_server().await;
    let process_definition_id = deploy_process(&engine, &base_url, &client).await;

    let category_response = client
        .put(format!(
            "{base_url}/repository/process-definitions/{process_definition_id}"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "category": "http://example.com/category"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(category_response.status(), reqwest::StatusCode::OK);
    let category_body: Value = category_response.json().await.unwrap();
    assert_eq!(category_body["category"], "http://example.com/category");
    assert_eq!(category_body["suspended"], false);

    let suspend_response = client
        .put(format!(
            "{base_url}/repository/process-definitions/{process_definition_id}"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "action": "suspend",
            "includeProcessInstances": true
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(suspend_response.status(), reqwest::StatusCode::OK);
    let suspend_body: Value = suspend_response.json().await.unwrap();
    assert_eq!(suspend_body["suspended"], true);

    let activate_response = client
        .put(format!(
            "{base_url}/repository/process-definitions/{process_definition_id}"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "action": "activate"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(activate_response.status(), reqwest::StatusCode::OK);
    let activate_body: Value = activate_response.json().await.unwrap();
    assert_eq!(activate_body["suspended"], false);

    let scheduled_response = client
        .put(format!(
            "{base_url}/repository/process-definitions/{process_definition_id}"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "action": "suspend",
            "date": "2030-01-01T00:00:00Z"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(scheduled_response.status(), reqwest::StatusCode::OK);
    let scheduled_body: Value = scheduled_response.json().await.unwrap();
    assert_eq!(scheduled_body["suspended"], true);

    let timer_jobs_response = client
        .get(format!("{base_url}/management/timer-jobs"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(timer_jobs_response.status(), reqwest::StatusCode::OK);
    let timer_jobs_body: Value = timer_jobs_response.json().await.unwrap();
    assert_eq!(timer_jobs_body["total"], 1);
    let timer_job_id = timer_jobs_body["data"][0]["id"].as_str().unwrap();
    assert_eq!(
        timer_jobs_body["data"][0]["elementId"],
        "process-definition-suspend"
    );

    let execute_response = client
        .post(format!("{base_url}/management/timer-jobs/{timer_job_id}"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "action": "execute"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(execute_response.status(), reqwest::StatusCode::NO_CONTENT);

    let definition_response = client
        .get(format!(
            "{base_url}/repository/process-definitions/{process_definition_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(definition_response.status(), reqwest::StatusCode::OK);
    let definition_body: Value = definition_response.json().await.unwrap();
    assert_eq!(definition_body["suspended"], true);
}

#[tokio::test]
async fn process_definition_scheduled_action_with_past_date_executes_immediately() {
    let (engine, base_url, client) = spawn_server().await;
    let process_definition_id = deploy_process(&engine, &base_url, &client).await;

    let response = client
        .put(format!(
            "{base_url}/repository/process-definitions/{process_definition_id}"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "action": "suspend",
            "date": "2000-01-01T00:00:00Z"
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["suspended"], true);

    let timer_jobs_response = client
        .get(format!("{base_url}/management/timer-jobs"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(timer_jobs_response.status(), reqwest::StatusCode::OK);
    let timer_jobs_body: Value = timer_jobs_response.json().await.unwrap();
    assert_eq!(timer_jobs_body["total"], 0);
}

#[tokio::test]
async fn process_definition_scheduled_action_rejects_invalid_date() {
    let (engine, base_url, client) = spawn_server().await;
    let process_definition_id = deploy_process(&engine, &base_url, &client).await;
    engine
        .get_repository_service()
        .set_process_definition_suspended(&process_definition_id, true)
        .unwrap();

    let response = client
        .put(format!(
            "{base_url}/repository/process-definitions/{process_definition_id}"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "action": "activate",
            "date": "not-a-date"
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
            .contains("Invalid process definition action date")
    );
}

#[tokio::test]
async fn process_definition_action_rejects_missing_or_unknown_action_with_canonical_message() {
    let (engine, base_url, client) = spawn_server().await;
    let process_definition_id = deploy_process(&engine, &base_url, &client).await;

    let missing_action_response = client
        .put(format!(
            "{base_url}/repository/process-definitions/{process_definition_id}"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({}))
        .send()
        .await
        .unwrap();

    assert_eq!(
        missing_action_response.status(),
        reqwest::StatusCode::BAD_REQUEST
    );
    let missing_action_body: Value = missing_action_response.json().await.unwrap();
    assert_eq!(missing_action_body["code"], "BAD_REQUEST");
    assert_eq!(missing_action_body["message"], "Bad Request");
    assert_eq!(missing_action_body["details"], "Invalid action: 'null'.");

    let unknown_action_response = client
        .put(format!(
            "{base_url}/repository/process-definitions/{process_definition_id}"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "action": "unexistingaction"
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(
        unknown_action_response.status(),
        reqwest::StatusCode::BAD_REQUEST
    );
    let unknown_action_body: Value = unknown_action_response.json().await.unwrap();
    assert_eq!(unknown_action_body["code"], "BAD_REQUEST");
    assert_eq!(
        unknown_action_body["details"],
        "Invalid action: 'unexistingaction'."
    );
}

#[tokio::test]
async fn process_definition_put_returns_404_for_nonexistent_id() {
    let (_engine, base_url, client) = spawn_server().await;

    let response = client
        .put(format!(
            "{base_url}/repository/process-definitions/nonexistent"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "category": "test-category"
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), reqwest::StatusCode::NOT_FOUND);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["code"], "NOT_FOUND");
}

#[tokio::test]
async fn process_definition_category_update_persists_on_followup_get() {
    let (engine, base_url, client) = spawn_server().await;
    let process_definition_id = deploy_process(&engine, &base_url, &client).await;

    let put_response = client
        .put(format!(
            "{base_url}/repository/process-definitions/{process_definition_id}"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "category": "persisted-category"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(put_response.status(), reqwest::StatusCode::OK);

    let get_response = client
        .get(format!(
            "{base_url}/repository/process-definitions/{process_definition_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(get_response.status(), reqwest::StatusCode::OK);
    let get_body: Value = get_response.json().await.unwrap();
    assert_eq!(get_body["category"], "persisted-category");
}

#[tokio::test]
async fn process_definition_identity_links_can_be_created_and_fetched_by_family() {
    let (engine, base_url, client) = spawn_server().await;
    let process_definition_id = deploy_process(&engine, &base_url, &client).await;

    let create_user_response = client
        .post(format!(
            "{base_url}/repository/process-definitions/{process_definition_id}/identitylinks"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "user": "kermit",
            "type": "candidate"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(create_user_response.status(), reqwest::StatusCode::CREATED);
    let create_user_body: Value = create_user_response.json().await.unwrap();
    assert_eq!(create_user_body["user"], "kermit");
    assert!(create_user_body["group"].is_null());
    assert_eq!(create_user_body["type"], "candidate");

    let create_group_response = client
        .post(format!(
            "{base_url}/repository/process-definitions/{process_definition_id}/identitylinks"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "group": "management",
            "type": "candidate"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(create_group_response.status(), reqwest::StatusCode::CREATED);

    let get_user_response = client
        .get(format!(
            "{base_url}/repository/process-definitions/{process_definition_id}/identitylinks/users/kermit"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(get_user_response.status(), reqwest::StatusCode::OK);
    let get_user_body: Value = get_user_response.json().await.unwrap();
    assert_eq!(get_user_body["user"], "kermit");
    assert_eq!(get_user_body["type"], "candidate");

    let get_group_response = client
        .get(format!(
            "{base_url}/repository/process-definitions/{process_definition_id}/identitylinks/groups/management"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(get_group_response.status(), reqwest::StatusCode::OK);
    let get_group_body: Value = get_group_response.json().await.unwrap();
    assert_eq!(get_group_body["group"], "management");

    let bad_payload_response = client
        .post(format!(
            "{base_url}/repository/process-definitions/{process_definition_id}/identitylinks"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "user": "kermit",
            "group": "management",
            "type": "candidate"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        bad_payload_response.status(),
        reqwest::StatusCode::BAD_REQUEST
    );
}

/// Java parity: `ProcessDefinitionResource.suspendProcessDefinition` /
/// `activateProcessDefinition` reject an action targeting the current state
/// with `FlowableConflictException` (HTTP 409).
#[tokio::test]
async fn process_definition_suspend_and_activate_conflict_when_already_in_state() {
    let (engine, base_url, client) = spawn_server().await;
    let process_definition_id = deploy_process(&engine, &base_url, &client).await;

    // Activate an already-active definition -> 409.
    let activate_conflict = put_definition_action(
        &base_url,
        &client,
        &process_definition_id,
        json!({ "action": "activate" }),
    )
    .await;
    assert_eq!(activate_conflict.status(), reqwest::StatusCode::CONFLICT);
    let activate_body: Value = activate_conflict.json().await.unwrap();
    assert_eq!(activate_body["code"], "CONFLICT");
    assert!(
        activate_body["details"]
            .as_str()
            .unwrap()
            .contains("is already active")
    );

    let suspend_response = put_definition_action(
        &base_url,
        &client,
        &process_definition_id,
        json!({ "action": "suspend" }),
    )
    .await;
    assert_eq!(suspend_response.status(), reqwest::StatusCode::OK);

    // Suspend an already-suspended definition -> 409.
    let suspend_conflict = put_definition_action(
        &base_url,
        &client,
        &process_definition_id,
        json!({ "action": "suspend" }),
    )
    .await;
    assert_eq!(suspend_conflict.status(), reqwest::StatusCode::CONFLICT);
    let suspend_body: Value = suspend_conflict.json().await.unwrap();
    assert_eq!(suspend_body["code"], "CONFLICT");
    assert!(
        suspend_body["details"]
            .as_str()
            .unwrap()
            .contains("is already suspended")
    );

    // The definition state must be unchanged by the conflicting request.
    assert!(
        engine
            .get_repository_service()
            .get_process_definition(&process_definition_id)
            .unwrap()
            .is_suspended
    );
}

/// Java parity: `AbstractSetProcessDefinitionStateCmd.changeProcessDefinitionState`
/// with `includeProcessInstances=true` cascades the immediate (non-scheduled)
/// action to process instances, their executions, tasks and jobs.
#[tokio::test]
async fn immediate_suspend_with_include_process_instances_cascades_to_running_instances() {
    let (engine, base_url, client) = spawn_server().await;
    let process_definition_id = deploy_user_task_process(&engine, &base_url, &client).await;
    let process_instance_id =
        start_process_instance(&base_url, &client, &process_definition_id).await;

    let suspend_response = put_definition_action(
        &base_url,
        &client,
        &process_definition_id,
        json!({ "action": "suspend", "includeProcessInstances": true }),
    )
    .await;
    assert_eq!(suspend_response.status(), reqwest::StatusCode::OK);
    let suspend_body: Value = suspend_response.json().await.unwrap();
    assert_eq!(suspend_body["suspended"], true);

    assert!(process_instance_is_suspended(&base_url, &client, &process_instance_id).await);

    // Tasks of the cascaded instance are suspended too
    // (Java `AbstractSetProcessInstanceStateCmd` task migration).
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let tasks = store.find_tasks_by_process_instance_id(&process_instance_id, &mut session);
    assert!(!tasks.is_empty());
    assert!(tasks.iter().all(|task| task.suspension_state == 1));
    session.rollback().unwrap();

    // Activating with includeProcessInstances=true migrates the suspended
    // instance back to active.
    let activate_response = put_definition_action(
        &base_url,
        &client,
        &process_definition_id,
        json!({ "action": "activate", "includeProcessInstances": true }),
    )
    .await;
    assert_eq!(activate_response.status(), reqwest::StatusCode::OK);

    assert!(!process_instance_is_suspended(&base_url, &client, &process_instance_id).await);
    let mut session = store.create_session().unwrap();
    let tasks = store.find_tasks_by_process_instance_id(&process_instance_id, &mut session);
    assert!(tasks.iter().all(|task| task.suspension_state == 0));
    session.rollback().unwrap();
}

/// Java parity: without `includeProcessInstances`, only the definition state
/// changes; running instances stay active.
#[tokio::test]
async fn immediate_suspend_without_include_process_instances_leaves_instances_active() {
    let (engine, base_url, client) = spawn_server().await;
    let process_definition_id = deploy_user_task_process(&engine, &base_url, &client).await;
    let process_instance_id =
        start_process_instance(&base_url, &client, &process_definition_id).await;

    let suspend_response = put_definition_action(
        &base_url,
        &client,
        &process_definition_id,
        json!({ "action": "suspend" }),
    )
    .await;
    assert_eq!(suspend_response.status(), reqwest::StatusCode::OK);

    assert!(
        engine
            .get_repository_service()
            .get_process_definition(&process_definition_id)
            .unwrap()
            .is_suspended
    );
    assert!(!process_instance_is_suspended(&base_url, &client, &process_instance_id).await);

    // Starting a new instance of a suspended definition fails like Java
    // `ProcessInstanceHelper` (FlowableException -> 500).
    let start_response = client
        .post(format!("{base_url}/runtime/process-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "processDefinitionId": process_definition_id }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        start_response.status(),
        reqwest::StatusCode::INTERNAL_SERVER_ERROR
    );
}
