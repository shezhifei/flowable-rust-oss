use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::runtime::execution::Execution;
use flowable_rest::run_server;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::net::TcpListener;

async fn start_test_server(test_name: &str) -> (reqwest::Client, String, Arc<ProcessEngine>) {
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
    let addr = listener.local_addr().unwrap();
    let base_url = format!("http://{}", addr);
    let engine_clone = Arc::clone(&engine);
    tokio::spawn(async move {
        run_server(engine_clone, listener).await.unwrap();
    });

    (reqwest::Client::new(), base_url, engine)
}

async fn deploy_and_start_user_task_process(
    client: &reqwest::Client,
    base_url: &str,
    engine: &ProcessEngine,
) -> (String, String) {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="phase3VariablesProcess" name="Phase 3 Variables Process" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="flow1" sourceRef="start" targetRef="reviewTask" />
            <userTask id="reviewTask" name="Review JSON Variables" />
            <sequenceFlow id="flow2" sourceRef="reviewTask" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#;

    let deploy_response = client
        .post(format!("{base_url}/repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "Phase 3 Variables Deployment",
            "resourceName": "phase3_variables.bpmn20.xml",
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
            "businessKey": "phase-3-json-variables"
        }))
        .send()
        .await
        .unwrap();
    assert!(start_response.status().is_success());
    let start_body: Value = start_response.json().await.unwrap();
    let process_instance_id = start_body["id"].as_str().unwrap().to_string();

    let task_response = client
        .get(format!(
            "{base_url}/runtime/tasks?processInstanceId={process_instance_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(task_response.status().is_success());
    let task_body: Value = task_response.json().await.unwrap();
    let execution_id = task_body["data"][0]["executionId"]
        .as_str()
        .unwrap()
        .to_string();

    (process_instance_id, execution_id)
}

#[tokio::test]
async fn bpmn_runtime_and_history_variable_endpoints_are_real_backed_for_json_values() {
    let (client, base_url, engine) = start_test_server("rest-bpmn-phase3-variables").await;
    let (process_instance_id, execution_id) =
        deploy_and_start_user_task_process(&client, &base_url, &engine).await;

    let create_response = client
        .post(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}/variables"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!([{
            "name": "approvalPayload",
            "type": "json",
            "value": {
                "approved": true,
                "score": 42
            }
        }]))
        .send()
        .await
        .unwrap();
    assert_eq!(create_response.status(), reqwest::StatusCode::CREATED);
    let created_body: Value = create_response.json().await.unwrap();
    assert_eq!(created_body[0]["name"], "approvalPayload");
    assert_eq!(created_body[0]["type"], "json");
    assert_eq!(created_body[0]["value"]["approved"], true);

    let process_variables_response = client
        .get(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}/variables"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(process_variables_response.status().is_success());
    let process_variables_body: Value = process_variables_response.json().await.unwrap();
    assert_eq!(process_variables_body.as_array().unwrap().len(), 1);
    assert_eq!(process_variables_body[0]["name"], "approvalPayload");

    let task_response = client
        .get(format!(
            "{base_url}/runtime/tasks?processInstanceId={process_instance_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(task_response.status().is_success());
    let task_body: Value = task_response.json().await.unwrap();
    let task_id = task_body["data"][0]["id"].as_str().unwrap();

    let task_variables_response = client
        .get(format!("{base_url}/runtime/tasks/{task_id}/variables"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(task_variables_response.status().is_success());
    let task_variables_body: Value = task_variables_response.json().await.unwrap();
    assert_eq!(task_variables_body.as_array().unwrap().len(), 1);
    assert_eq!(task_variables_body[0]["name"], "approvalPayload");

    let task_variable_response = client
        .get(format!(
            "{base_url}/runtime/tasks/{task_id}/variables/approvalPayload"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(task_variable_response.status().is_success());
    let task_variable_body: Value = task_variable_response.json().await.unwrap();
    assert_eq!(task_variable_body["value"]["score"], 42);

    let task_variable_data_response = client
        .get(format!(
            "{base_url}/runtime/tasks/{task_id}/variables/approvalPayload/data"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(task_variable_data_response.status().is_success());
    let task_variable_data_body: Value = task_variable_data_response.json().await.unwrap();
    assert_eq!(task_variable_data_body["approved"], true);

    let execution_variable_response = client
        .get(format!(
            "{base_url}/runtime/executions/{execution_id}/variables/approvalPayload"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(execution_variable_response.status().is_success());
    let execution_variable_body: Value = execution_variable_response.json().await.unwrap();
    assert_eq!(execution_variable_body["value"]["score"], 42);

    let process_variable_data_response = client
        .get(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}/variables/approvalPayload/data"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(process_variable_data_response.status().is_success());
    let process_variable_data_body: Value = process_variable_data_response.json().await.unwrap();
    assert_eq!(process_variable_data_body["score"], 42);

    let execution_variable_data_response = client
        .get(format!(
            "{base_url}/runtime/executions/{execution_id}/variables/approvalPayload/data"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(execution_variable_data_response.status().is_success());
    let execution_variable_data_body: Value =
        execution_variable_data_response.json().await.unwrap();
    assert_eq!(execution_variable_data_body["approved"], true);

    let update_response = client
        .put(format!(
            "{base_url}/runtime/executions/{execution_id}/variables/approvalPayload"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "approvalPayload",
            "type": "json",
            "value": {
                "approved": false,
                "score": 7
            }
        }))
        .send()
        .await
        .unwrap();
    assert!(update_response.status().is_success());

    let variable_instances_response = client
        .get(format!(
            "{base_url}/runtime/variable-instances?processInstanceId={process_instance_id}&variableName=approvalPayload"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(variable_instances_response.status().is_success());
    let variable_instances_body: Value = variable_instances_response.json().await.unwrap();
    assert_eq!(variable_instances_body["total"], 1);
    assert_eq!(variable_instances_body["data"][0]["type"], "json");
    assert_eq!(variable_instances_body["data"][0]["value"]["score"], 7);
    let variable_instance_id = variable_instances_body["data"][0]["id"]
        .as_str()
        .unwrap()
        .to_string();

    let runtime_data_response = client
        .get(format!(
            "{base_url}/runtime/variable-instances/{variable_instance_id}/data"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(runtime_data_response.status().is_success());
    let runtime_data_body: Value = runtime_data_response.json().await.unwrap();
    assert_eq!(runtime_data_body["score"], 7);

    let historic_variables_response = client
        .get(format!(
            "{base_url}/history/historic-variable-instances?processInstanceId={process_instance_id}&variableName=approvalPayload"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(historic_variables_response.status().is_success());
    let historic_variables_body: Value = historic_variables_response.json().await.unwrap();
    assert_eq!(historic_variables_body["total"], 1);
    assert_eq!(historic_variables_body["data"][0]["variableType"], "json");
    assert_eq!(historic_variables_body["data"][0]["value"]["score"], 7);
    let historic_variable_instance_id = historic_variables_body["data"][0]["id"]
        .as_str()
        .unwrap()
        .to_string();

    let historic_data_response = client
        .get(format!(
            "{base_url}/history/historic-variable-instances/{historic_variable_instance_id}/data"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(historic_data_response.status().is_success());
    let historic_data_body: Value = historic_data_response.json().await.unwrap();
    assert_eq!(historic_data_body["approved"], false);

    let historic_process_variable_data_response = client
        .get(format!(
            "{base_url}/history/historic-process-instances/{process_instance_id}/variables/approvalPayload/data"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(
        historic_process_variable_data_response
            .status()
            .is_success()
    );
    let historic_process_variable_data_body: Value = historic_process_variable_data_response
        .json()
        .await
        .unwrap();
    assert_eq!(historic_process_variable_data_body["score"], 7);

    let historic_task_variable_data_response = client
        .get(format!(
            "{base_url}/history/historic-task-instances/{task_id}/variables/approvalPayload/data"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(historic_task_variable_data_response.status().is_success());
    let historic_task_variable_data_body: Value =
        historic_task_variable_data_response.json().await.unwrap();
    assert_eq!(historic_task_variable_data_body["score"], 7);
}

#[tokio::test]
async fn process_instance_binary_variable_metadata_and_data_round_trip_raw_bytes() {
    let (client, base_url, engine) = start_test_server("rest-bpmn-binary-variable-data").await;
    let (process_instance_id, _execution_id) =
        deploy_and_start_user_task_process(&client, &base_url, &engine).await;

    let create_response = client
        .post(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}/variables"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "documentBytes",
            "type": "binary",
            "value": null
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(create_response.status(), reqwest::StatusCode::CREATED);
    let created_body: Value = create_response.json().await.unwrap();
    assert_eq!(created_body[0]["name"], "documentBytes");
    assert_eq!(created_body[0]["type"], "binary");
    assert!(created_body[0]["value"].is_null());

    let metadata_response = client
        .get(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}/variables/documentBytes"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(metadata_response.status(), reqwest::StatusCode::OK);
    let metadata_body: Value = metadata_response.json().await.unwrap();
    assert_eq!(metadata_body["name"], "documentBytes");
    assert_eq!(metadata_body["type"], "binary");
    assert!(metadata_body["value"].is_null());

    let original_bytes = vec![0, 1, 2, 3, 254, 255, b'f', b'l', b'o', b'w'];
    let update_data_response = client
        .put(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}/variables/documentBytes/data"
        ))
        .basic_auth("admin", Some("test"))
        .header(reqwest::header::CONTENT_TYPE, "application/octet-stream")
        .body(original_bytes.clone())
        .send()
        .await
        .unwrap();
    assert_eq!(
        update_data_response.status(),
        reqwest::StatusCode::NO_CONTENT
    );

    let data_response = client
        .get(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}/variables/documentBytes/data"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(data_response.status(), reqwest::StatusCode::OK);
    assert_eq!(
        data_response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .unwrap(),
        "application/octet-stream"
    );
    let returned_bytes = data_response.bytes().await.unwrap();
    assert_eq!(returned_bytes.as_ref(), original_bytes.as_slice());
}

#[tokio::test]
async fn process_instance_serializable_variable_metadata_and_data_round_trip_json_bytes() {
    let (client, base_url, engine) =
        start_test_server("rest-bpmn-serializable-variable-data").await;
    let (process_instance_id, _execution_id) =
        deploy_and_start_user_task_process(&client, &base_url, &engine).await;

    let create_response = client
        .post(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}/variables"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "serializableObject",
            "type": "serializable",
            "value": null
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(create_response.status(), reqwest::StatusCode::CREATED);
    let created_body: Value = create_response.json().await.unwrap();
    assert_eq!(created_body[0]["name"], "serializableObject");
    assert_eq!(created_body[0]["type"], "serializable");
    assert!(created_body[0]["value"].is_null());

    let metadata_response = client
        .get(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}/variables/serializableObject"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(metadata_response.status(), reqwest::StatusCode::OK);
    let metadata_body: Value = metadata_response.json().await.unwrap();
    assert_eq!(metadata_body["name"], "serializableObject");
    assert_eq!(metadata_body["type"], "serializable");
    assert!(metadata_body["value"].is_null());

    let object_data = json!({
        "className": "com.example.OrderPayload",
        "fields": {
            "approved": true,
            "score": 9
        }
    })
    .to_string()
    .into_bytes();
    let update_data_response = client
        .put(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}/variables/serializableObject/data"
        ))
        .basic_auth("admin", Some("test"))
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(object_data.clone())
        .send()
        .await
        .unwrap();
    assert_eq!(
        update_data_response.status(),
        reqwest::StatusCode::NO_CONTENT
    );

    let data_response = client
        .get(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}/variables/serializableObject/data"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(data_response.status(), reqwest::StatusCode::OK);
    assert_eq!(
        data_response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .unwrap(),
        "application/x-java-serialized-object"
    );
    let returned_bytes = data_response.bytes().await.unwrap();
    assert_eq!(returned_bytes.as_ref(), object_data.as_slice());
    let returned_json: Value = serde_json::from_slice(&returned_bytes).unwrap();
    assert_eq!(returned_json["fields"]["score"], 9);
}

#[tokio::test]
async fn serializable_variable_metadata_rejects_inline_value_shape() {
    let (client, base_url, engine) =
        start_test_server("rest-bpmn-serializable-variable-invalid-shape").await;
    let (process_instance_id, _execution_id) =
        deploy_and_start_user_task_process(&client, &base_url, &engine).await;

    let create_response = client
        .post(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}/variables"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "serializableObject",
            "type": "serializable",
            "value": {
                "inline": "not accepted"
            }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(create_response.status(), reqwest::StatusCode::BAD_REQUEST);
    let error_body = create_response.text().await.unwrap();
    assert!(error_body.contains("serializable"));
    assert!(error_body.contains("metadata must use null value"));
}

#[tokio::test]
async fn task_variable_write_endpoints_update_backing_execution_variables() {
    let (client, base_url, engine) = start_test_server("rest-bpmn-task-variable-write").await;
    let (process_instance_id, execution_id) =
        deploy_and_start_user_task_process(&client, &base_url, &engine).await;

    let task_response = client
        .get(format!(
            "{base_url}/runtime/tasks?processInstanceId={process_instance_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(task_response.status().is_success());
    let task_body: Value = task_response.json().await.unwrap();
    let task_id = task_body["data"][0]["id"].as_str().unwrap();

    let create_response = client
        .post(format!(
            "{base_url}/runtime/tasks/{task_id}/variables?scope=global"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!([{
            "name": "taskGlobal",
            "type": "string",
            "value": "created"
        }]))
        .send()
        .await
        .unwrap();
    assert_eq!(create_response.status(), reqwest::StatusCode::CREATED);
    let created_body: Value = create_response.json().await.unwrap();
    assert_eq!(created_body[0]["name"], "taskGlobal");
    assert_eq!(created_body[0]["value"], "created");

    let update_collection_response = client
        .put(format!("{base_url}/runtime/tasks/{task_id}/variables"))
        .basic_auth("admin", Some("test"))
        .json(&json!([{
            "name": "taskGlobal",
            "type": "string",
            "value": "updated-by-collection"
        }]))
        .send()
        .await
        .unwrap();
    assert!(update_collection_response.status().is_success());
    let updated_collection_body: Value = update_collection_response.json().await.unwrap();
    assert_eq!(updated_collection_body[0]["value"], "updated-by-collection");

    let update_single_response = client
        .put(format!(
            "{base_url}/runtime/tasks/{task_id}/variables/taskGlobal?scope=global"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "taskGlobal",
            "type": "string",
            "value": "updated-single"
        }))
        .send()
        .await
        .unwrap();
    assert!(update_single_response.status().is_success());
    let updated_single_body: Value = update_single_response.json().await.unwrap();
    assert_eq!(updated_single_body["value"], "updated-single");

    let backing_variable = engine
        .get_variable_service()
        .get_variable(execution_id.clone(), "taskGlobal".to_string())
        .unwrap();
    assert_eq!(backing_variable, Some(json!("updated-single")));

    let task_local_create_response = client
        .post(format!("{base_url}/runtime/tasks/{task_id}/variables"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "localOnly",
            "type": "string",
            "value": "task-local"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        task_local_create_response.status(),
        reqwest::StatusCode::CREATED
    );
    let task_local_create_body: Value = task_local_create_response.json().await.unwrap();
    assert_eq!(task_local_create_body[0]["name"], "localOnly");
    assert_eq!(task_local_create_body[0]["scope"], "local");
    assert_eq!(task_local_create_body[0]["value"], "task-local");

    let task_local_read_response = client
        .get(format!(
            "{base_url}/runtime/tasks/{task_id}/variables/localOnly?scope=local"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(task_local_read_response.status(), reqwest::StatusCode::OK);
    let task_local_read_body: Value = task_local_read_response.json().await.unwrap();
    assert_eq!(task_local_read_body["scope"], "local");
    assert_eq!(task_local_read_body["value"], "task-local");

    let task_global_local_only_response = client
        .get(format!(
            "{base_url}/runtime/tasks/{task_id}/variables/localOnly?scope=global"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(
        task_global_local_only_response.status(),
        reqwest::StatusCode::NOT_FOUND
    );
    assert_eq!(
        engine
            .get_variable_service()
            .get_variable(execution_id.clone(), "localOnly".to_string())
            .unwrap(),
        None
    );

    let delete_response = client
        .delete(format!(
            "{base_url}/runtime/tasks/{task_id}/variables/taskGlobal?scope=global"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(delete_response.status(), reqwest::StatusCode::NO_CONTENT);

    let get_deleted_response = client
        .get(format!(
            "{base_url}/runtime/tasks/{task_id}/variables/taskGlobal"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(
        get_deleted_response.status(),
        reqwest::StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn process_instance_variables_use_root_execution_when_multiple_executions_are_active() {
    let (client, base_url, engine) =
        start_test_server("rest-bpmn-process-instance-variables-root-execution").await;
    let (process_instance_id, _execution_id) =
        deploy_and_start_user_task_process(&client, &base_url, &engine).await;

    let sibling_execution = Execution {
        id: format!("{process_instance_id}:parallel"),
        process_instance_id: Some(process_instance_id.clone()),
        root_process_instance_id: Some(process_instance_id.clone()),
        parent_id: Some(process_instance_id.clone()),
        is_active: true,
        ..Default::default()
    };
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    store.insert_execution(&sibling_execution, &mut session);
    session.flush_and_commit().unwrap();

    let create_response = client
        .post(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}/variables"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "processScopeFlag",
            "type": "boolean",
            "value": true
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(create_response.status(), reqwest::StatusCode::CREATED);

    let get_response = client
        .get(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}/variables/processScopeFlag"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(get_response.status().is_success());
    let get_body: Value = get_response.json().await.unwrap();
    assert_eq!(get_body["value"], true);

    let root_variable = engine
        .get_variable_service()
        .get_variable(process_instance_id, "processScopeFlag".to_string())
        .unwrap();
    assert_eq!(root_variable, Some(json!(true)));
}

#[tokio::test]
async fn runtime_execution_and_process_instance_variable_delete_removes_single_variable() {
    let (client, base_url, engine) = start_test_server("rest-bpmn-runtime-variable-delete").await;
    let (process_instance_id, execution_id) =
        deploy_and_start_user_task_process(&client, &base_url, &engine).await;

    let create_execution_variable_response = client
        .post(format!(
            "{base_url}/runtime/executions/{execution_id}/variables"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "deleteMe",
            "type": "string",
            "value": "present"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        create_execution_variable_response.status(),
        reqwest::StatusCode::CREATED
    );

    let delete_execution_variable_response = client
        .delete(format!(
            "{base_url}/runtime/executions/{execution_id}/variables/deleteMe"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(
        delete_execution_variable_response.status(),
        reqwest::StatusCode::NO_CONTENT
    );

    let get_deleted_execution_variable_response = client
        .get(format!(
            "{base_url}/runtime/executions/{execution_id}/variables/deleteMe"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(
        get_deleted_execution_variable_response.status(),
        reqwest::StatusCode::NOT_FOUND
    );

    let create_process_variable_response = client
        .post(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}/variables"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "deleteFromProcess",
            "type": "integer",
            "value": 5
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        create_process_variable_response.status(),
        reqwest::StatusCode::CREATED
    );

    let delete_process_variable_response = client
        .delete(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}/variables/deleteFromProcess"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(
        delete_process_variable_response.status(),
        reqwest::StatusCode::NO_CONTENT
    );

    let delete_missing_variable_response = client
        .delete(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}/variables/deleteFromProcess"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(
        delete_missing_variable_response.status(),
        reqwest::StatusCode::NOT_FOUND
    );
}

// --- Execution variable suspension guard (Java parity) ----------------------
// Java split across two layers:
// * `BaseExecutionVariableResource.setVariable:217-224` runs the
//   create-conflict / update-miss `hasVariableOnScope` checks in the request
//   thread BEFORE dispatching into the engine cmd.
// * `SetExecutionVariablesCmd` / `RemoveExecutionVariablesCmd` /
//   `SetAsyncExecutionVariablesCmd` extend `NeedsActiveExecutionCmd` with
//   prefixes "Cannot set variables to" / "Cannot remove variables from"
//   (`RuntimeServiceImpl.java:366-393` for the set*Async entry points;
//   `SetExecutionVariablesCmd:79-80`, `RemoveExecutionVariablesCmd:57-58`).
// So an update-only PUT or a require-exists DELETE of a missing name is 404
// even on a suspended execution — the suspended guard is unreachable. A
// create-only POST that passes the mode check reaches the guard and is 500.
// Read commands do NOT extend NeedsActiveExecutionCmd and are allowed.

#[tokio::test]
async fn execution_variable_write_endpoints_reject_suspended_execution_with_500() {
    let (client, base_url, engine) = start_test_server("rest-bpmn-exec-var-suspend-reject").await;
    let (process_instance_id, execution_id) =
        deploy_and_start_user_task_process(&client, &base_url, &engine).await;

    let suspend = client
        .put(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "action": "suspend" }))
        .send()
        .await
        .unwrap();
    assert!(suspend.status().is_success());

    // POST /runtime/executions/{id}/variables: create-only, name is free →
    // mode check passes, then NeedsActiveExecutionCmd → 500.
    let post = client
        .post(format!(
            "{base_url}/runtime/executions/{execution_id}/variables"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!([{ "name": "execVar", "type": "string", "value": "v" }]))
        .send()
        .await
        .unwrap();
    assert_eq!(post.status(), reqwest::StatusCode::INTERNAL_SERVER_ERROR);
    let post_body: Value = post.json().await.unwrap();
    // 5xx: raw engine messages are logged server-side only; public details
    // is a fixed string (no suspended-execution / path echo).
    assert_eq!(post_body["details"], "Internal server error");

    // PUT /runtime/executions/{id}/variables/{name}: update-only. Java
    // `BaseExecutionVariableResource.setVariable:222-224` 404s on a missing
    // name before the cmd-level suspended guard is reached.
    let put = client
        .put(format!(
            "{base_url}/runtime/executions/{execution_id}/variables/execVar"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "name": "execVar", "type": "string", "value": "v" }))
        .send()
        .await
        .unwrap();
    assert_eq!(put.status(), reqwest::StatusCode::NOT_FOUND);
    let put_body: Value = put.json().await.unwrap();
    assert_eq!(
        put_body["details"],
        format!("Execution '{execution_id}' does not have a variable with name: 'execVar'.")
    );

    // DELETE /runtime/executions/{id}/variables/{name}: Java
    // `ExecutionVariableResource.deleteVariable:197-199` runs
    // `hasVariableOnScope` first → 404 for a missing name on a suspended
    // execution (suspended guard unreachable).
    let delete = client
        .delete(format!(
            "{base_url}/runtime/executions/{execution_id}/variables/execVar"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(delete.status(), reqwest::StatusCode::NOT_FOUND);

    // POST /runtime/executions/{id}/variables-async  (SetAsyncExecutionVariablesCmd)
    let post_async = client
        .post(format!(
            "{base_url}/runtime/executions/{execution_id}/variables-async"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!([{ "name": "execAsyncVar", "type": "string", "value": "v" }]))
        .send()
        .await
        .unwrap();
    assert_eq!(
        post_async.status(),
        reqwest::StatusCode::INTERNAL_SERVER_ERROR
    );

    // PUT /runtime/executions/{id}/variables-async  (SetAsyncExecutionVariablesCmd)
    let put_async = client
        .put(format!(
            "{base_url}/runtime/executions/{execution_id}/variables-async"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!([{ "name": "execAsyncVar", "type": "string", "value": "v" }]))
        .send()
        .await
        .unwrap();
    assert_eq!(
        put_async.status(),
        reqwest::StatusCode::INTERNAL_SERVER_ERROR
    );

    // PUT /runtime/executions/{id}/variables-async/{name}: Java
    // `BaseExecutionVariableResource.setVariable` runs the update-only
    // `hasVariableOnScope` check in the request thread, BEFORE the async
    // command's suspended-execution guard is reached — the absent variable is
    // a 404 here even on a suspended execution.
    let put_async_single = client
        .put(format!(
            "{base_url}/runtime/executions/{execution_id}/variables-async/execAsyncVar"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "name": "execAsyncVar", "type": "string", "value": "v" }))
        .send()
        .await
        .unwrap();
    assert_eq!(put_async_single.status(), reqwest::StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn execution_variable_read_endpoints_succeed_on_suspended_execution() {
    let (client, base_url, engine) = start_test_server("rest-bpmn-exec-var-suspend-read").await;
    let (process_instance_id, execution_id) =
        deploy_and_start_user_task_process(&client, &base_url, &engine).await;

    // Set a variable while active so reads have something to return.
    let create = client
        .post(format!(
            "{base_url}/runtime/executions/{execution_id}/variables"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!([{ "name": "execReadVar", "type": "string", "value": "readval" }]))
        .send()
        .await
        .unwrap();
    assert_eq!(create.status(), reqwest::StatusCode::CREATED);

    let suspend = client
        .put(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "action": "suspend" }))
        .send()
        .await
        .unwrap();
    assert!(suspend.status().is_success());

    let get_all = client
        .get(format!(
            "{base_url}/runtime/executions/{execution_id}/variables"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(get_all.status(), reqwest::StatusCode::OK);

    let get_one = client
        .get(format!(
            "{base_url}/runtime/executions/{execution_id}/variables/execReadVar"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(get_one.status(), reqwest::StatusCode::OK);
}

#[tokio::test]
async fn execution_variable_write_succeeds_after_activate() {
    let (client, base_url, engine) =
        start_test_server("rest-bpmn-exec-var-suspend-reactivate").await;
    let (process_instance_id, execution_id) =
        deploy_and_start_user_task_process(&client, &base_url, &engine).await;

    let suspend = client
        .put(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "action": "suspend" }))
        .send()
        .await
        .unwrap();
    assert!(suspend.status().is_success());

    let activate = client
        .put(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "action": "activate" }))
        .send()
        .await
        .unwrap();
    assert!(activate.status().is_success());

    let post = client
        .post(format!(
            "{base_url}/runtime/executions/{execution_id}/variables"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!([{ "name": "execVar", "type": "string", "value": "after-activate" }]))
        .send()
        .await
        .unwrap();
    assert_eq!(post.status(), reqwest::StatusCode::CREATED);
}
