use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_rest::run_server;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::net::TcpListener;

#[tokio::test]
async fn runtime_process_instance_inject_creates_dynamic_user_task() {
    let engine = Arc::new(ProcessEngine::new("rest-bpmn-inject".to_string()));

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
        <process id="injectProcess" name="Inject Process" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="f1" sourceRef="start" targetRef="userTask" />
            <userTask id="userTask" name="Original Review" />
            <sequenceFlow id="f2" sourceRef="userTask" targetRef="end" />
            <subProcess id="modeledSubprocess" name="Modeled Subprocess">
                <startEvent id="subStart" />
                <sequenceFlow id="subFlow1" sourceRef="subStart" targetRef="subReview" />
                <userTask id="subReview" name="Injected Subprocess Review" />
                <sequenceFlow id="subFlow2" sourceRef="subReview" targetRef="subEnd" />
                <endEvent id="subEnd" />
            </subProcess>
            <endEvent id="end" />
        </process>
    </definitions>"#;

    let deploy_response = client
        .post(format!("{base_url}/repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "Inject Deployment",
            "resourceName": "inject_process.bpmn20.xml",
            "resource": xml
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
            "businessKey": "Inject Instance"
        }))
        .send()
        .await
        .unwrap();
    assert!(start_response.status().is_success());
    let start_body: Value = start_response.json().await.unwrap();
    let process_instance_id = start_body["id"].as_str().unwrap().to_string();

    let unsupported_response = client
        .post(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}/inject"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "injectionType": "subprocess",
            "id": "dynamicSubprocess"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(unsupported_response.status(), 400);
    let unsupported_body: Value = unsupported_response.json().await.unwrap();
    assert_eq!(unsupported_body["code"], "BAD_REQUEST");
    assert!(
        unsupported_body["details"]
            .as_str()
            .unwrap()
            .contains("was not found")
    );

    let subprocess_inject_response = client
        .post(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}/inject"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "injectionType": "subprocess",
            "id": "modeledSubprocess"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(subprocess_inject_response.status(), 200);

    let subprocess_tasks_response = client
        .post(format!("{base_url}/query/tasks"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "processInstanceId": process_instance_id
        }))
        .send()
        .await
        .unwrap();
    assert!(subprocess_tasks_response.status().is_success());
    let subprocess_tasks_body: Value = subprocess_tasks_response.json().await.unwrap();
    assert_eq!(subprocess_tasks_body["total"], 2);
    assert!(
        subprocess_tasks_body["data"]
            .as_array()
            .unwrap()
            .iter()
            .any(|task| task["name"] == "Injected Subprocess Review")
    );

    let inject_response = client
        .post(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}/inject"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "injectionType": "task",
            "id": "dynamic-review",
            "name": "Dynamic Review",
            "assignee": "kermit"
        }))
        .send()
        .await
        .unwrap();
    assert!(inject_response.status().is_success());

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
    assert_eq!(tasks_body["total"], 3);
    assert!(
        tasks_body["data"]
            .as_array()
            .unwrap()
            .iter()
            .any(|task| task["id"] == "dynamic-review" && task["name"] == "Dynamic Review")
    );

    let historic_tasks_response = client
        .post(format!("{base_url}/query/historic-task-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "processInstanceId": process_instance_id
        }))
        .send()
        .await
        .unwrap();
    assert!(historic_tasks_response.status().is_success());
    let historic_tasks_body: Value = historic_tasks_response.json().await.unwrap();
    assert!(
        historic_tasks_body["data"]
            .as_array()
            .unwrap()
            .iter()
            .any(|task| task["id"] == "dynamic-review" && task["name"] == "Dynamic Review")
    );

    let user_task_alias_response = client
        .post(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}/inject"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "injectionType": "userTask",
            "taskId": "user-task-alias",
            "name": "User Task Alias",
            "assignee": "fozzie"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(user_task_alias_response.status(), 200);

    let alias_tasks_response = client
        .post(format!("{base_url}/query/tasks"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "processInstanceId": process_instance_id
        }))
        .send()
        .await
        .unwrap();
    assert!(alias_tasks_response.status().is_success());
    let alias_tasks_body: Value = alias_tasks_response.json().await.unwrap();
    assert_eq!(alias_tasks_body["total"], 4);
    assert!(
        alias_tasks_body["data"]
            .as_array()
            .unwrap()
            .iter()
            .any(|task| task["id"] == "user-task-alias" && task["name"] == "User Task Alias")
    );

    let activity_alias_response = client
        .post(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}/inject"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "injectionType": "TASK",
            "activityId": "activity-id-alias",
            "activityName": "Activity Name Alias",
            "assignee": "gonzo",
            "variables": [
                {
                    "name": "injectedFlag",
                    "type": "boolean",
                    "value": true
                }
            ]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(activity_alias_response.status(), 200);

    let activity_alias_tasks_response = client
        .post(format!("{base_url}/query/tasks"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "processInstanceId": process_instance_id
        }))
        .send()
        .await
        .unwrap();
    assert!(activity_alias_tasks_response.status().is_success());
    let activity_alias_tasks_body: Value = activity_alias_tasks_response.json().await.unwrap();
    assert_eq!(activity_alias_tasks_body["total"], 5);
    assert!(
        activity_alias_tasks_body["data"]
            .as_array()
            .unwrap()
            .iter()
            .any(|task| task["id"] == "activity-id-alias" && task["name"] == "Activity Name Alias")
    );
    let injected_task = engine
        .get_task_service()
        .get_tasks_by_process_instance_id(process_instance_id.clone())
        .unwrap()
        .into_iter()
        .find(|task| task.id == "activity-id-alias")
        .unwrap();
    assert_eq!(injected_task.assignee.as_deref(), Some("gonzo"));

    let variables_response = client
        .get(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}/variables"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(variables_response.status().is_success());
    let variables_body: Value = variables_response.json().await.unwrap();
    assert!(
        variables_body
            .as_array()
            .unwrap()
            .iter()
            .any(|variable| variable["name"] == "injectedFlag" && variable["value"] == true)
    );

    let missing_response = client
        .post(format!(
            "{base_url}/runtime/process-instances/missing-process/inject"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "injectionType": "task",
            "id": "missing-dynamic-task"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(missing_response.status(), 404);
}

#[tokio::test]
async fn runtime_process_instance_inject_start_before_moves_current_wait_state() {
    let engine = Arc::new(ProcessEngine::new(
        "rest-bpmn-inject-start-before".to_string(),
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
        <process id="injectStartBeforeProcess" name="Inject Start Before Process" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="f1" sourceRef="start" targetRef="reviewA" />
            <userTask id="reviewA" name="Review A" />
            <sequenceFlow id="f2" sourceRef="reviewA" targetRef="reviewB" />
            <userTask id="reviewB" name="Review B" />
            <sequenceFlow id="f3" sourceRef="reviewB" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#;

    let deploy_response = client
        .post(format!("{base_url}/repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "Inject Start Before Deployment",
            "resourceName": "inject_start_before_process.bpmn20.xml",
            "resource": xml
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
            "businessKey": "Inject Start Before Instance"
        }))
        .send()
        .await
        .unwrap();
    assert!(start_response.status().is_success());
    let start_body: Value = start_response.json().await.unwrap();
    let process_instance_id = start_body["id"].as_str().unwrap().to_string();

    let start_before_response = client
        .post(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}/inject"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "injectionType": "startBefore",
            "id": "reviewB"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(start_before_response.status(), 200);

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

    let start_after_response = client
        .post(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}/inject"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "injectionType": "startAfter",
            "id": "reviewB"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(start_after_response.status(), 200);

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
    assert_eq!(tasks_body["total"], 0);

    let process_instance_response = client
        .get(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(process_instance_response.status().is_success());
    let process_instance_body: Value = process_instance_response.json().await.unwrap();
    assert_eq!(process_instance_body["isEnded"], true);
}

#[tokio::test]
async fn runtime_process_instance_inject_start_after_moves_to_single_successor_user_task() {
    let engine = Arc::new(ProcessEngine::new(
        "rest-bpmn-inject-start-after".to_string(),
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
        <process id="injectStartAfterProcess" name="Inject Start After Process" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="f1" sourceRef="start" targetRef="reviewA" />
            <userTask id="reviewA" name="Review A" />
            <sequenceFlow id="f2" sourceRef="reviewA" targetRef="reviewB" />
            <userTask id="reviewB" name="Review B" />
            <sequenceFlow id="f3" sourceRef="reviewB" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#;

    let deploy_response = client
        .post(format!("{base_url}/repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "Inject Start After Deployment",
            "resourceName": "inject_start_after_process.bpmn20.xml",
            "resource": xml
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
            "businessKey": "Inject Start After Instance"
        }))
        .send()
        .await
        .unwrap();
    assert!(start_response.status().is_success());
    let start_body: Value = start_response.json().await.unwrap();
    let process_instance_id = start_body["id"].as_str().unwrap().to_string();

    let start_after_response = client
        .post(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}/inject"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "injectionType": "startAfter",
            "id": "reviewA"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(start_after_response.status(), 200);

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
}

#[tokio::test]
async fn runtime_process_instance_inject_start_after_rejects_multiple_successors() {
    let engine = Arc::new(ProcessEngine::new(
        "rest-bpmn-inject-start-after-branch".to_string(),
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
        <process id="injectStartAfterBranchProcess" name="Inject Start After Branch Process" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="f1" sourceRef="start" targetRef="reviewA" />
            <userTask id="reviewA" name="Review A" />
            <sequenceFlow id="f2" sourceRef="reviewA" targetRef="reviewB" />
            <sequenceFlow id="f3" sourceRef="reviewA" targetRef="reviewC" />
            <userTask id="reviewB" name="Review B" />
            <userTask id="reviewC" name="Review C" />
        </process>
    </definitions>"#;

    let deploy_response = client
        .post(format!("{base_url}/repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "Inject Start After Branch Deployment",
            "resourceName": "inject_start_after_branch_process.bpmn20.xml",
            "resource": xml
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
            "businessKey": "Inject Start After Branch Instance"
        }))
        .send()
        .await
        .unwrap();
    assert!(start_response.status().is_success());
    let start_body: Value = start_response.json().await.unwrap();
    let process_instance_id = start_body["id"].as_str().unwrap().to_string();

    let start_after_response = client
        .post(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}/inject"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "injectionType": "startAfter",
            "id": "reviewA"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(start_after_response.status(), 400);
    let start_after_body: Value = start_after_response.json().await.unwrap();
    assert_eq!(start_after_body["code"], "BAD_REQUEST");
    assert!(
        start_after_body["details"]
            .as_str()
            .unwrap()
            .contains("multiple outgoing sequence flows")
    );
}
