use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_rest::run_server;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::net::TcpListener;

#[tokio::test]
async fn runtime_process_instances_bulk_delete_removes_runtime_state_and_records_history() {
    let engine = Arc::new(ProcessEngine::new(
        "rest-bpmn-bulk-process-delete".to_string(),
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
        <process id="bulkDeleteProcess" name="Bulk Delete Process" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="f1" sourceRef="start" targetRef="userTask" />
            <userTask id="userTask" name="Review" />
            <sequenceFlow id="f2" sourceRef="userTask" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#;

    let deploy_response = client
        .post(format!("{base_url}/repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "Bulk Delete Deployment",
            "resourceName": "bulk_delete_process.bpmn20.xml",
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

    let first_id = start_instance(&client, &base_url, &process_definition_id, "first").await;
    let second_id = start_instance(&client, &base_url, &process_definition_id, "second").await;

    let illegal_action_response = client
        .post(format!("{base_url}/runtime/process-instances/delete"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "action": "terminate",
            "instanceIds": [first_id]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(illegal_action_response.status(), 400);

    let delete_response = client
        .post(format!("{base_url}/runtime/process-instances/delete"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "action": "delete",
            "instanceIds": [first_id, second_id],
            "deleteReason": "cleanup requested"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(delete_response.status(), 204);

    for process_instance_id in [first_id.as_str(), second_id.as_str()] {
        let get_runtime_response = client
            .get(format!(
                "{base_url}/runtime/process-instances/{process_instance_id}"
            ))
            .basic_auth("admin", Some("test"))
            .send()
            .await
            .unwrap();
        assert_eq!(get_runtime_response.status(), 404);

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
        assert_eq!(historic_body["id"], process_instance_id);
        assert!(historic_body["endTime"].is_string());
        assert_eq!(historic_body["deleteReason"], "cleanup requested");
    }

    let illegal_history_delete_response = client
        .post(format!(
            "{base_url}/history/historic-process-instances/delete"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "action": "terminate",
            "instanceIds": [first_id]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(illegal_history_delete_response.status(), 400);

    let history_delete_response = client
        .post(format!(
            "{base_url}/history/historic-process-instances/delete"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "action": "delete",
            "instanceIds": [first_id, second_id]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(history_delete_response.status(), 204);

    for process_instance_id in [first_id.as_str(), second_id.as_str()] {
        let historic_response = client
            .get(format!(
                "{base_url}/history/historic-process-instances/{process_instance_id}"
            ))
            .basic_auth("admin", Some("test"))
            .send()
            .await
            .unwrap();
        assert_eq!(historic_response.status(), 404);
    }

    let missing_response = client
        .post(format!("{base_url}/runtime/process-instances/delete"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "action": "delete",
            "instanceIds": ["missing-process-instance"]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(missing_response.status(), 404);

    let missing_historic_response = client
        .post(format!(
            "{base_url}/history/historic-process-instances/delete"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "action": "delete",
            "instanceIds": ["missing-historic-process-instance"]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(missing_historic_response.status(), 404);
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
            "businessKey": business_key
        }))
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success());
    let body: Value = response.json().await.unwrap();
    body["id"].as_str().unwrap().to_string()
}
