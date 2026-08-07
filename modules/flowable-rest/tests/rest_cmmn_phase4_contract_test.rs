use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_rest::run_server;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::net::TcpListener;

const SIMPLE_CMMN: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/CMMN/20151109/MODEL"
             targetNamespace="http://flowable.org/cmmn">
  <case id="phase4Case" name="Phase 4 Case">
    <casePlanModel id="phase4Plan" name="Phase 4 Plan" autoComplete="false">
      <planItem id="planItemReview" name="Review" definitionRef="reviewTask" />
      <humanTask id="reviewTask" name="Review" isBlocking="true" />
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

async fn deploy_and_start_case(base_url: &str, client: &reqwest::Client) -> String {
    let deploy_response = client
        .post(format!("{base_url}/cmmn-repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "CMMN Phase 4 Deployment",
            "resourceName": "phase4.cmmn",
            "resource": SIMPLE_CMMN
        }))
        .send()
        .await
        .unwrap();
    assert!(deploy_response.status().is_success());

    let start_response = client
        .post(format!("{base_url}/cmmn-runtime/case-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "caseDefinitionKey": "phase4Case",
            "businessKey": "phase4-bk",
            "variables": {
                "customer": "acme",
                "amount": 42,
                "approved": false
            }
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
async fn cmmn_management_engine_exposes_stable_engine_info() {
    let (base_url, client) = spawn_server("rest-cmmn-phase4-management").await;

    let response = client
        .get(format!("{base_url}/cmmn-management/engine"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert!(response.status().is_success());
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["name"], "cmmn-engine");
    assert!(body["version"].is_string());
    assert!(body["resourceUrl"].is_null());
    assert!(body["exception"].is_null());
}

#[tokio::test]
async fn cmmn_variable_instance_routes_read_real_case_variables() {
    let (base_url, client) = spawn_server("rest-cmmn-phase4-variables").await;
    let case_instance_id = deploy_and_start_case(&base_url, &client).await;

    let list_response = client
        .get(format!(
            "{base_url}/cmmn-runtime/variable-instances?caseInstanceId={case_instance_id}&start=0&size=10"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(list_response.status().is_success());
    let list_body: Value = list_response.json().await.unwrap();
    assert_eq!(list_body["total"], 3);
    assert_eq!(list_body["start"], 0);
    assert_eq!(list_body["size"], 3);

    let customer_variable = list_body["data"]
        .as_array()
        .unwrap()
        .iter()
        .find(|variable| variable["name"] == "customer")
        .expect("customer variable");
    assert_eq!(customer_variable["caseInstanceId"], case_instance_id);
    assert_eq!(customer_variable["scopeId"], case_instance_id);
    assert_eq!(customer_variable["scopeType"], "cmmn");
    assert_eq!(customer_variable["type"], "string");
    assert_eq!(customer_variable["value"], "acme");
    let customer_variable_id = customer_variable["id"].as_str().unwrap();

    let get_response = client
        .get(format!(
            "{base_url}/cmmn-runtime/variable-instances/{customer_variable_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(get_response.status().is_success());
    let get_body: Value = get_response.json().await.unwrap();
    assert_eq!(get_body["id"], customer_variable_id);
    assert_eq!(get_body["name"], "customer");
    assert_eq!(get_body["value"], "acme");

    let data_response = client
        .get(format!(
            "{base_url}/cmmn-runtime/variable-instances/{customer_variable_id}/data"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(data_response.status().is_success());
    let data_body: Value = data_response.json().await.unwrap();
    assert_eq!(data_body, "acme");

    let query_response = client
        .post(format!("{base_url}/cmmn-query/variable-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "caseInstanceId": case_instance_id,
            "variableName": "amount",
            "start": 0,
            "size": 10
        }))
        .send()
        .await
        .unwrap();
    assert!(query_response.status().is_success());
    let query_body: Value = query_response.json().await.unwrap();
    assert_eq!(query_body["total"], 1);
    assert_eq!(query_body["data"][0]["name"], "amount");
    assert_eq!(query_body["data"][0]["type"], "integer");
    assert_eq!(query_body["data"][0]["value"], 42);
}

#[tokio::test]
async fn cmmn_variable_instance_queries_accept_task_and_plan_item_aliases() {
    let (base_url, client) = spawn_server("rest-cmmn-phase4-variable-query-aliases").await;
    let case_instance_id = deploy_and_start_case(&base_url, &client).await;

    let tasks_response = client
        .get(format!(
            "{base_url}/cmmn-runtime/tasks?caseInstanceId={case_instance_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(tasks_response.status().is_success());
    let tasks_body: Value = tasks_response.json().await.unwrap();
    let task_id = tasks_body["data"][0]["id"].as_str().unwrap();

    let plan_item_alias_response = client
        .get(format!(
            "{base_url}/cmmn-runtime/variable-instances?planItemInstanceId={task_id}&start=0&size=10"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(plan_item_alias_response.status().is_success());
    let plan_item_alias_body: Value = plan_item_alias_response.json().await.unwrap();
    assert_eq!(plan_item_alias_body["total"], 3);
    assert!(
        plan_item_alias_body["data"]
            .as_array()
            .unwrap()
            .iter()
            .any(|variable| variable["name"] == "customer")
    );

    let task_alias_response = client
        .post(format!("{base_url}/cmmn-query/variable-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "taskId": task_id,
            "variableName": "amount",
            "start": 0,
            "size": 10
        }))
        .send()
        .await
        .unwrap();
    assert!(task_alias_response.status().is_success());
    let task_alias_body: Value = task_alias_response.json().await.unwrap();
    assert_eq!(task_alias_body["total"], 1);
    assert_eq!(task_alias_body["data"][0]["name"], "amount");
    assert_eq!(
        task_alias_body["data"][0]["caseInstanceId"],
        case_instance_id
    );

    let conflicting_alias_response = client
        .post(format!("{base_url}/cmmn-query/variable-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "taskId": task_id,
            "planItemInstanceId": "different-plan-item"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        conflicting_alias_response.status(),
        reqwest::StatusCode::BAD_REQUEST
    );
    let conflicting_alias_body: Value = conflicting_alias_response.json().await.unwrap();
    assert_eq!(conflicting_alias_body["code"], "BAD_REQUEST");
    assert!(
        conflicting_alias_body["details"]
            .as_str()
            .unwrap()
            .contains("Only one of taskId or planItemInstanceId")
    );
}

#[tokio::test]
async fn cmmn_case_instance_variable_paths_read_real_case_variables() {
    let (base_url, client) = spawn_server("rest-cmmn-phase4-case-variable-paths").await;
    let case_instance_id = deploy_and_start_case(&base_url, &client).await;

    let list_response = client
        .get(format!(
            "{base_url}/cmmn-runtime/case-instances/{case_instance_id}/variables"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(list_response.status().is_success());
    let list_body: Value = list_response.json().await.unwrap();
    let variables = list_body.as_array().unwrap();
    assert_eq!(variables.len(), 3);
    assert!(
        variables
            .iter()
            .any(|variable| variable["name"] == "customer")
    );

    let get_response = client
        .get(format!(
            "{base_url}/cmmn-runtime/case-instances/{case_instance_id}/variables/customer"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(get_response.status().is_success());
    let get_body: Value = get_response.json().await.unwrap();
    assert_eq!(get_body["name"], "customer");
    assert_eq!(get_body["value"], "acme");
    assert_eq!(get_body["caseInstanceId"], case_instance_id);

    let data_response = client
        .get(format!(
            "{base_url}/cmmn-runtime/case-instances/{case_instance_id}/variables/customer/data"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(data_response.status().is_success());
    let data_body: Value = data_response.json().await.unwrap();
    assert_eq!(data_body, "acme");
}

#[tokio::test]
async fn cmmn_case_instance_binary_variable_metadata_and_data_round_trip_raw_bytes() {
    let (base_url, client) = spawn_server("rest-cmmn-phase4-case-binary-data").await;
    let case_instance_id = deploy_and_start_case(&base_url, &client).await;

    let create_response = client
        .post(format!(
            "{base_url}/cmmn-runtime/case-instances/{case_instance_id}/variables-async"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!([{
            "name": "caseDocumentBytes",
            "type": "binary",
            "value": null
        }]))
        .send()
        .await
        .unwrap();
    assert_eq!(create_response.status(), reqwest::StatusCode::NO_CONTENT);

    let metadata_response = client
        .get(format!(
            "{base_url}/cmmn-runtime/case-instances/{case_instance_id}/variables/caseDocumentBytes"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(metadata_response.status(), reqwest::StatusCode::OK);
    let metadata_body: Value = metadata_response.json().await.unwrap();
    assert_eq!(metadata_body["name"], "caseDocumentBytes");
    assert_eq!(metadata_body["type"], "binary");
    assert!(metadata_body["value"].is_null());

    let original_bytes = vec![0, 1, 2, 3, 254, 255, b'c', b'm', b'm', b'n'];
    let update_data_response = client
        .put(format!(
            "{base_url}/cmmn-runtime/case-instances/{case_instance_id}/variables/caseDocumentBytes/data"
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
            "{base_url}/cmmn-runtime/case-instances/{case_instance_id}/variables/caseDocumentBytes/data"
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
async fn cmmn_plan_item_bytes_variable_metadata_and_data_round_trip_raw_bytes() {
    let (base_url, client) = spawn_server("rest-cmmn-phase4-plan-item-bytes-data").await;
    let case_instance_id = deploy_and_start_case(&base_url, &client).await;

    let plan_items_response = client
        .get(format!(
            "{base_url}/cmmn-runtime/plan-item-instances?caseInstanceId={case_instance_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(plan_items_response.status().is_success());
    let plan_items_body: Value = plan_items_response.json().await.unwrap();
    let plan_item_instance_id = plan_items_body["data"][0]["id"].as_str().unwrap();

    let create_response = client
        .post(format!(
            "{base_url}/cmmn-runtime/plan-item-instances/{plan_item_instance_id}/variables-async"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!([{
            "name": "planItemPayloadBytes",
            "type": "bytes",
            "value": null
        }]))
        .send()
        .await
        .unwrap();
    assert_eq!(create_response.status(), reqwest::StatusCode::NO_CONTENT);

    let metadata_response = client
        .get(format!(
            "{base_url}/cmmn-runtime/plan-item-instances/{plan_item_instance_id}/variables/planItemPayloadBytes"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(metadata_response.status(), reqwest::StatusCode::OK);
    let metadata_body: Value = metadata_response.json().await.unwrap();
    assert_eq!(metadata_body["name"], "planItemPayloadBytes");
    assert_eq!(metadata_body["type"], "bytes");
    assert!(metadata_body["value"].is_null());
    assert_eq!(metadata_body["scopeId"], case_instance_id);

    let original_bytes = vec![0, 10, 20, 30, 240, 250, b'p', b'l', b'a', b'n'];
    let update_data_response = client
        .put(format!(
            "{base_url}/cmmn-runtime/plan-item-instances/{plan_item_instance_id}/variables/planItemPayloadBytes/data"
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
            "{base_url}/cmmn-runtime/plan-item-instances/{plan_item_instance_id}/variables/planItemPayloadBytes/data"
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
async fn cmmn_case_serializable_variable_metadata_and_data_round_trip_json_bytes() {
    let (base_url, client) = spawn_server("rest-cmmn-phase4-case-serializable-data").await;
    let case_instance_id = deploy_and_start_case(&base_url, &client).await;

    let create_response = client
        .post(format!(
            "{base_url}/cmmn-runtime/case-instances/{case_instance_id}/variables-async"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!([{
            "name": "serializableObject",
            "type": "serializable",
            "value": null
        }]))
        .send()
        .await
        .unwrap();
    assert_eq!(create_response.status(), reqwest::StatusCode::NO_CONTENT);

    let metadata_response = client
        .get(format!(
            "{base_url}/cmmn-runtime/case-instances/{case_instance_id}/variables/serializableObject"
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
    assert_eq!(metadata_body["caseInstanceId"], case_instance_id);

    let object_data = json!({
        "className": "com.example.CasePayload",
        "fields": {
            "customer": "acme",
            "amount": 42
        }
    })
    .to_string()
    .into_bytes();
    let update_data_response = client
        .put(format!(
            "{base_url}/cmmn-runtime/case-instances/{case_instance_id}/variables/serializableObject/data"
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
            "{base_url}/cmmn-runtime/case-instances/{case_instance_id}/variables/serializableObject/data"
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
    assert_eq!(returned_json["fields"]["amount"], 42);
}

#[tokio::test]
async fn cmmn_plan_item_serializable_variable_metadata_and_data_round_trip_json_bytes() {
    let (base_url, client) = spawn_server("rest-cmmn-phase4-plan-serializable-data").await;
    let case_instance_id = deploy_and_start_case(&base_url, &client).await;

    let plan_items_response = client
        .get(format!(
            "{base_url}/cmmn-runtime/plan-item-instances?caseInstanceId={case_instance_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(plan_items_response.status().is_success());
    let plan_items_body: Value = plan_items_response.json().await.unwrap();
    let plan_item_instance_id = plan_items_body["data"][0]["id"].as_str().unwrap();

    let create_response = client
        .post(format!(
            "{base_url}/cmmn-runtime/plan-item-instances/{plan_item_instance_id}/variables-async"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!([{
            "name": "planItemObject",
            "type": "serializable",
            "value": null
        }]))
        .send()
        .await
        .unwrap();
    assert_eq!(create_response.status(), reqwest::StatusCode::NO_CONTENT);

    let metadata_response = client
        .get(format!(
            "{base_url}/cmmn-runtime/plan-item-instances/{plan_item_instance_id}/variables/planItemObject"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(metadata_response.status(), reqwest::StatusCode::OK);
    let metadata_body: Value = metadata_response.json().await.unwrap();
    assert_eq!(metadata_body["name"], "planItemObject");
    assert_eq!(metadata_body["type"], "serializable");
    assert!(metadata_body["value"].is_null());
    assert_eq!(metadata_body["scopeId"], case_instance_id);

    let object_data = json!({
        "className": "com.example.PlanItemPayload",
        "fields": {
            "reviewed": true
        }
    })
    .to_string()
    .into_bytes();
    let update_data_response = client
        .put(format!(
            "{base_url}/cmmn-runtime/plan-item-instances/{plan_item_instance_id}/variables/planItemObject/data"
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
            "{base_url}/cmmn-runtime/plan-item-instances/{plan_item_instance_id}/variables/planItemObject/data"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(data_response.status(), reqwest::StatusCode::OK);
    let returned_bytes = data_response.bytes().await.unwrap();
    assert_eq!(returned_bytes.as_ref(), object_data.as_slice());
    let returned_json: Value = serde_json::from_slice(&returned_bytes).unwrap();
    assert_eq!(returned_json["fields"]["reviewed"], true);
}

#[tokio::test]
async fn cmmn_serializable_variable_metadata_rejects_inline_value_shape() {
    let (base_url, client) =
        spawn_server("rest-cmmn-phase4-serializable-variable-invalid-shape").await;
    let case_instance_id = deploy_and_start_case(&base_url, &client).await;

    let create_response = client
        .post(format!(
            "{base_url}/cmmn-runtime/case-instances/{case_instance_id}/variables-async"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!([{
            "name": "serializableObject",
            "type": "serializable",
            "value": {
                "inline": "not accepted"
            }
        }]))
        .send()
        .await
        .unwrap();
    assert_eq!(create_response.status(), reqwest::StatusCode::BAD_REQUEST);
    let error_body = create_response.text().await.unwrap();
    assert!(error_body.contains("serializable"));
    assert!(error_body.contains("metadata must use null value"));
}

#[tokio::test]
async fn cmmn_task_and_plan_item_variable_paths_read_case_scope_variables() {
    let (base_url, client) = spawn_server("rest-cmmn-phase4-task-variable-paths").await;
    let case_instance_id = deploy_and_start_case(&base_url, &client).await;

    let tasks_response = client
        .get(format!(
            "{base_url}/cmmn-runtime/tasks?caseInstanceId={case_instance_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(tasks_response.status().is_success());
    let tasks_body: Value = tasks_response.json().await.unwrap();
    let task_id = tasks_body["data"][0]["id"].as_str().unwrap();

    for prefix in [
        format!("{base_url}/cmmn-runtime/tasks/{task_id}/variables"),
        format!("{base_url}/cmmn-runtime/plan-item-instances/{task_id}/variables"),
    ] {
        let list_response = client
            .get(format!("{prefix}?scope=GlObAl"))
            .basic_auth("admin", Some("test"))
            .send()
            .await
            .unwrap();
        assert!(list_response.status().is_success());
        let list_body: Value = list_response.json().await.unwrap();
        let variables = list_body.as_array().unwrap();
        assert_eq!(variables.len(), 3);

        let get_response = client
            .get(format!("{prefix}/amount?scope=GlObAl"))
            .basic_auth("admin", Some("test"))
            .send()
            .await
            .unwrap();
        assert!(get_response.status().is_success());
        let get_body: Value = get_response.json().await.unwrap();
        assert_eq!(get_body["name"], "amount");
        assert_eq!(get_body["value"], 42);
        assert_eq!(get_body["scopeId"], case_instance_id);

        let data_response = client
            .get(format!("{prefix}/amount/data?scope=GlObAl"))
            .basic_auth("admin", Some("test"))
            .send()
            .await
            .unwrap();
        assert!(data_response.status().is_success());
        let data_body: Value = data_response.json().await.unwrap();
        assert_eq!(data_body, 42);

        let local_list_response = client
            .get(format!("{prefix}?scope=LoCaL"))
            .basic_auth("admin", Some("test"))
            .send()
            .await
            .unwrap();
        assert_eq!(local_list_response.status(), reqwest::StatusCode::OK);
        let local_list_body: Value = local_list_response.json().await.unwrap();
        assert_eq!(local_list_body.as_array().unwrap().len(), 0);

        let unknown_scope_response = client
            .get(format!("{prefix}?scope=unknown"))
            .basic_auth("admin", Some("test"))
            .send()
            .await
            .unwrap();
        assert_eq!(
            unknown_scope_response.status(),
            reqwest::StatusCode::BAD_REQUEST
        );
        let unknown_scope_body: Value = unknown_scope_response.json().await.unwrap();
        assert_eq!(unknown_scope_body["code"], "BAD_REQUEST");
        assert!(
            unknown_scope_body["details"]
                .as_str()
                .unwrap()
                .contains("Unsupported CMMN variable scope 'unknown'")
        );
    }
}

#[tokio::test]
async fn cmmn_task_service_surface_exposes_subtasks_and_form_contracts() {
    let (base_url, client) = spawn_server("rest-cmmn-phase4-task-service-surface").await;
    let case_instance_id = deploy_and_start_case(&base_url, &client).await;

    let tasks_response = client
        .get(format!(
            "{base_url}/cmmn-runtime/tasks?caseInstanceId={case_instance_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(tasks_response.status().is_success());
    let tasks_body: Value = tasks_response.json().await.unwrap();
    let task_id = tasks_body["data"][0].as_object().unwrap()["id"]
        .as_str()
        .unwrap();

    let subtasks_response = client
        .get(format!("{base_url}/cmmn-runtime/tasks/{task_id}/subtasks"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(subtasks_response.status().is_success());
    let subtasks_body: Value = subtasks_response.json().await.unwrap();
    assert_eq!(subtasks_body, json!([]));

    let form_response = client
        .get(format!("{base_url}/cmmn-runtime/tasks/{task_id}/form"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(form_response.status(), reqwest::StatusCode::NOT_FOUND);
    let form_body: Value = form_response.json().await.unwrap();
    assert_eq!(form_body["code"], "NOT_FOUND");
    assert!(
        form_body["details"]
            .as_str()
            .unwrap()
            .contains("form was not found")
    );

    let historic_tasks_response = client
        .get(format!(
            "{base_url}/cmmn-history/historic-task-instances?caseInstanceId={case_instance_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(historic_tasks_response.status().is_success());
    let historic_tasks_body: Value = historic_tasks_response.json().await.unwrap();
    let historic_task_id = historic_tasks_body["data"][0].as_object().unwrap()["id"]
        .as_str()
        .unwrap();

    let historic_form_response = client
        .get(format!(
            "{base_url}/cmmn-history/historic-task-instances/{historic_task_id}/form"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(
        historic_form_response.status(),
        reqwest::StatusCode::NOT_FOUND
    );
    let historic_form_body: Value = historic_form_response.json().await.unwrap();
    assert_eq!(historic_form_body["code"], "NOT_FOUND");
    assert!(
        historic_form_body["details"]
            .as_str()
            .unwrap()
            .contains("form was not found")
    );
}

#[tokio::test]
async fn cmmn_variable_async_paths_persist_case_scope_variables_and_return_no_content() {
    let (base_url, client) = spawn_server("rest-cmmn-phase4-variable-async").await;
    let case_instance_id = deploy_and_start_case(&base_url, &client).await;

    let case_create_response = client
        .post(format!(
            "{base_url}/cmmn-runtime/case-instances/{case_instance_id}/variables-async"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!([{
            "name": "caseAsyncCreated",
            "type": "json",
            "value": {
                "source": "case-post",
                "version": 1
            }
        }]))
        .send()
        .await
        .unwrap();
    assert_eq!(
        case_create_response.status(),
        reqwest::StatusCode::NO_CONTENT
    );

    let case_get_response = client
        .get(format!(
            "{base_url}/cmmn-runtime/case-instances/{case_instance_id}/variables/caseAsyncCreated"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(case_get_response.status().is_success());
    let case_get_body: Value = case_get_response.json().await.unwrap();
    assert_eq!(case_get_body["value"]["source"], "case-post");
    assert_eq!(case_get_body["value"]["version"], 1);

    let case_update_response = client
        .put(format!(
            "{base_url}/cmmn-runtime/case-instances/{case_instance_id}/variables-async/caseAsyncCreated"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "caseAsyncCreated",
            "type": "json",
            "value": {
                "source": "case-put",
                "version": 2
            }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        case_update_response.status(),
        reqwest::StatusCode::NO_CONTENT
    );

    let case_data_response = client
        .get(format!(
            "{base_url}/cmmn-runtime/case-instances/{case_instance_id}/variables/caseAsyncCreated/data"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(case_data_response.status().is_success());
    let case_data_body: Value = case_data_response.json().await.unwrap();
    assert_eq!(case_data_body["source"], "case-put");
    assert_eq!(case_data_body["version"], 2);

    let plan_items_response = client
        .get(format!(
            "{base_url}/cmmn-runtime/plan-item-instances?caseInstanceId={case_instance_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(plan_items_response.status().is_success());
    let plan_items_body: Value = plan_items_response.json().await.unwrap();
    let plan_item_instance_id = plan_items_body["data"][0]["id"].as_str().unwrap();

    let plan_item_create_response = client
        .put(format!(
            "{base_url}/cmmn-runtime/plan-item-instances/{plan_item_instance_id}/variables-async"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!([{
            "name": "planItemAsyncCreated",
            "type": "integer",
            "value": 10
        }]))
        .send()
        .await
        .unwrap();
    assert_eq!(
        plan_item_create_response.status(),
        reqwest::StatusCode::NO_CONTENT
    );

    let plan_item_update_response = client
        .put(format!(
            "{base_url}/cmmn-runtime/plan-item-instances/{plan_item_instance_id}/variables-async/planItemAsyncCreated"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "planItemAsyncCreated",
            "type": "integer",
            "value": 11
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        plan_item_update_response.status(),
        reqwest::StatusCode::NO_CONTENT
    );

    let plan_item_get_response = client
        .get(format!(
            "{base_url}/cmmn-runtime/plan-item-instances/{plan_item_instance_id}/variables/planItemAsyncCreated"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(plan_item_get_response.status().is_success());
    let plan_item_get_body: Value = plan_item_get_response.json().await.unwrap();
    assert_eq!(plan_item_get_body["value"], 11);
    assert_eq!(plan_item_get_body["scopeId"], case_instance_id);
}

#[tokio::test]
async fn cmmn_historic_variable_paths_read_historic_case_variables() {
    let (base_url, client) = spawn_server("rest-cmmn-phase4-historic-variable-paths").await;
    let case_instance_id = deploy_and_start_case(&base_url, &client).await;

    let list_response = client
        .get(format!(
            "{base_url}/cmmn-history/historic-variable-instances?caseInstanceId={case_instance_id}&start=0&size=10"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(list_response.status().is_success());
    let list_body: Value = list_response.json().await.unwrap();
    assert_eq!(list_body["total"], 3);
    let customer_variable = list_body["data"]
        .as_array()
        .unwrap()
        .iter()
        .find(|variable| variable["name"] == "customer")
        .expect("customer historic variable");
    let variable_id = customer_variable["id"].as_str().unwrap();

    let data_response = client
        .get(format!(
            "{base_url}/cmmn-history/historic-variable-instances/{variable_id}/data"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(data_response.status().is_success());
    let data_body: Value = data_response.json().await.unwrap();
    assert_eq!(data_body, "acme");

    let query_response = client
        .post(format!("{base_url}/cmmn-query/historic-variable-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "caseInstanceId": case_instance_id,
            "variableName": "approved",
            "start": 0,
            "size": 10
        }))
        .send()
        .await
        .unwrap();
    assert!(query_response.status().is_success());
    let query_body: Value = query_response.json().await.unwrap();
    assert_eq!(query_body["total"], 1);
    assert_eq!(query_body["data"][0]["name"], "approved");
    assert_eq!(query_body["data"][0]["value"], false);

    let case_data_response = client
        .get(format!(
            "{base_url}/cmmn-history/historic-case-instances/{case_instance_id}/variables/customer/data"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(case_data_response.status().is_success());
    let case_data_body: Value = case_data_response.json().await.unwrap();
    assert_eq!(case_data_body, "acme");

    let tasks_response = client
        .get(format!(
            "{base_url}/cmmn-history/historic-task-instances?caseInstanceId={case_instance_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(tasks_response.status().is_success());
    let tasks_body: Value = tasks_response.json().await.unwrap();
    let task_id = tasks_body["data"][0]["id"].as_str().unwrap();

    let task_data_response = client
        .get(format!(
            "{base_url}/cmmn-history/historic-task-instances/{task_id}/variables/amount/data"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(task_data_response.status().is_success());
    let task_data_body: Value = task_data_response.json().await.unwrap();
    assert_eq!(task_data_body, 42);
}

#[tokio::test]
async fn cmmn_historic_queries_accept_task_and_plan_item_aliases() {
    let (base_url, client) = spawn_server("rest-cmmn-phase4-historic-query-aliases").await;
    let case_instance_id = deploy_and_start_case(&base_url, &client).await;

    let tasks_response = client
        .get(format!(
            "{base_url}/cmmn-history/historic-task-instances?caseInstanceId={case_instance_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(tasks_response.status().is_success());
    let tasks_body: Value = tasks_response.json().await.unwrap();
    let task_id = tasks_body["data"][0]["id"].as_str().unwrap();

    let historic_plan_item_alias_response = client
        .get(format!(
            "{base_url}/cmmn-history/historic-planitem-instances?planItemInstanceId={task_id}&start=0&size=10"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(historic_plan_item_alias_response.status().is_success());
    let historic_plan_item_alias_body: Value =
        historic_plan_item_alias_response.json().await.unwrap();
    assert_eq!(historic_plan_item_alias_body["total"], 1);
    assert_eq!(historic_plan_item_alias_body["data"][0]["id"], task_id);

    let plan_item_variable_alias_response = client
        .get(format!(
            "{base_url}/cmmn-history/historic-variable-instances?planItemInstanceId={task_id}&variableName=customer&start=0&size=10"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(plan_item_variable_alias_response.status().is_success());
    let plan_item_variable_alias_body: Value =
        plan_item_variable_alias_response.json().await.unwrap();
    assert_eq!(plan_item_variable_alias_body["total"], 1);
    assert_eq!(plan_item_variable_alias_body["data"][0]["name"], "customer");
    assert_eq!(
        plan_item_variable_alias_body["data"][0]["caseInstanceId"],
        case_instance_id
    );

    let task_variable_alias_response = client
        .post(format!("{base_url}/cmmn-query/historic-variable-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "taskId": task_id,
            "name": "approved",
            "start": 0,
            "size": 10
        }))
        .send()
        .await
        .unwrap();
    assert!(task_variable_alias_response.status().is_success());
    let task_variable_alias_body: Value = task_variable_alias_response.json().await.unwrap();
    assert_eq!(task_variable_alias_body["total"], 1);
    assert_eq!(task_variable_alias_body["data"][0]["name"], "approved");
    assert_eq!(task_variable_alias_body["data"][0]["value"], false);
}
