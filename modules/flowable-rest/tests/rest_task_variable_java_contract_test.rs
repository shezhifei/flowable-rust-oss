//! Java-parity contract tests for the task-variable REST endpoints
//! (`/runtime/tasks/{taskId}/variables[...]`).
//!
//! Case map (Java contract cases from the P2-TVAR batch 3 spec):
//!   1-2   collection_and_single_get_without_scope_merge_and_prefer_local
//!   3-4   post_scope_resolution_places_variables_on_local_or_execution
//!   5     post_rejects_mixed_scopes_and_writes_nothing
//!   6-7   post_rejects_empty_array_and_conflicting_names_atomically
//!   8-9   put_single_is_update_only_and_validates_body_name
//!   10-11 delete_single_defaults_to_local_and_collection_delete_keeps_globals
//!   12    invalid_scope_in_query_or_body_is_rejected
//!   13    multipart_post_without_file_is_rejected
//!   14    multipart_binary_variable_lifecycle
//!   15    multipart_serializable_variable_round_trips_opaque_bytes
//!   16    single_object_post_extension_is_accepted
//!   17    put_collection_extension_upserts_with_default_global_scope
//!   18    put_variable_data_extension_round_trips_raw_bytes
//!   19-20 query_scope_fallback_and_body_scope_precedence

use flowable_engine::engine::process_engine::ProcessEngine;
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

/// Deploys a single-user-task process, starts an instance and returns
/// (process_instance_id, execution_id, task_id).
async fn deploy_and_start_user_task_process(
    client: &reqwest::Client,
    base_url: &str,
    engine: &ProcessEngine,
) -> (String, String, String) {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="taskVariableContractProcess" name="Task Variable Contract Process" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="flow1" sourceRef="start" targetRef="reviewTask" />
            <userTask id="reviewTask" name="Review Task Variables" />
            <sequenceFlow id="flow2" sourceRef="reviewTask" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#;

    let deploy_response = client
        .post(format!("{base_url}/repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "Task Variable Contract Deployment",
            "resourceName": "task_variable_contract.bpmn20.xml",
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
            "businessKey": "task-variable-contract"
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
    let task_id = task_body["data"][0]["id"].as_str().unwrap().to_string();
    let execution_id = task_body["data"][0]["executionId"]
        .as_str()
        .unwrap()
        .to_string();

    (process_instance_id, execution_id, task_id)
}

async fn create_task_variables(
    client: &reqwest::Client,
    base_url: &str,
    task_id: &str,
    url_suffix: &str,
    body: Value,
) -> reqwest::Response {
    client
        .post(format!(
            "{base_url}/runtime/tasks/{task_id}/variables{url_suffix}"
        ))
        .basic_auth("admin", Some("test"))
        .json(&body)
        .send()
        .await
        .unwrap()
}

async fn get_task_variables(
    client: &reqwest::Client,
    base_url: &str,
    task_id: &str,
    url_suffix: &str,
) -> Vec<Value> {
    let response = client
        .get(format!(
            "{base_url}/runtime/tasks/{task_id}/variables{url_suffix}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    response.json::<Value>().await.unwrap().as_array().unwrap().clone()
}

fn variable_named<'a>(variables: &'a [Value], name: &str) -> Option<&'a Value> {
    variables.iter().find(|variable| variable["name"] == name)
}

fn execution_variable(engine: &ProcessEngine, execution_id: &str, name: &str) -> Option<Value> {
    engine
        .get_variable_service()
        .get_variable(execution_id.to_string(), name.to_string())
        .unwrap()
}

fn task_local_variable(engine: &ProcessEngine, task_id: &str, name: &str) -> Option<Value> {
    engine
        .get_task_service()
        .get_task_local_variable(task_id.to_string(), name.to_string())
        .unwrap()
}

fn multipart_text_field(body: &mut Vec<u8>, boundary: &str, name: &str, value: &str) {
    body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
    body.extend_from_slice(
        format!("Content-Disposition: form-data; name=\"{name}\"\r\n\r\n").as_bytes(),
    );
    body.extend_from_slice(value.as_bytes());
    body.extend_from_slice(b"\r\n");
}

fn multipart_file_field(body: &mut Vec<u8>, boundary: &str, name: &str, filename: &str, bytes: &[u8]) {
    body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
    body.extend_from_slice(
        format!("Content-Disposition: form-data; name=\"{name}\"; filename=\"{filename}\"\r\n")
            .as_bytes(),
    );
    body.extend_from_slice(b"Content-Type: application/octet-stream\r\n\r\n");
    body.extend_from_slice(bytes);
    body.extend_from_slice(b"\r\n");
}

fn multipart_close(body: &mut Vec<u8>, boundary: &str) {
    body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
}

async fn send_multipart(
    client: &reqwest::Client,
    method: reqwest::Method,
    url: String,
    boundary: &str,
    body: Vec<u8>,
) -> reqwest::Response {
    client
        .request(method, url)
        .basic_auth("admin", Some("test"))
        .header(
            reqwest::header::CONTENT_TYPE,
            format!("multipart/form-data; boundary={boundary}"),
        )
        .body(body)
        .send()
        .await
        .unwrap()
}

#[tokio::test]
async fn collection_and_single_get_without_scope_merge_and_prefer_local() {
    let (client, base_url, engine) = start_test_server("rest-tvar-merged-read").await;
    let (_process_instance_id, _execution_id, task_id) =
        deploy_and_start_user_task_process(&client, &base_url, &engine).await;

    // Seed a global "shared" + "globalOnly" and a local "shared" + "localOnly".
    let global_create = create_task_variables(
        &client,
        &base_url,
        &task_id,
        "?scope=global",
        json!([
            { "name": "shared", "type": "string", "value": "global-value" },
            { "name": "globalOnly", "type": "string", "value": "global-only" }
        ]),
    )
    .await;
    assert_eq!(global_create.status(), reqwest::StatusCode::CREATED);
    let local_create = create_task_variables(
        &client,
        &base_url,
        &task_id,
        "",
        json!([
            { "name": "shared", "type": "string", "value": "local-value" },
            { "name": "localOnly", "type": "string", "value": "local-only" }
        ]),
    )
    .await;
    assert_eq!(local_create.status(), reqwest::StatusCode::CREATED);

    // Case 1: GET collection without scope merges both scopes; the local
    // "shared" shadows the global one and scope labels reflect the origin.
    let merged = get_task_variables(&client, &base_url, &task_id, "").await;
    let shared_entries: Vec<&Value> = merged
        .iter()
        .filter(|variable| variable["name"] == "shared")
        .collect();
    assert_eq!(shared_entries.len(), 1, "local shadows global by name");
    assert_eq!(shared_entries[0]["value"], "local-value");
    assert_eq!(shared_entries[0]["scope"], "local");
    assert_eq!(
        variable_named(&merged, "globalOnly").unwrap()["scope"],
        "global"
    );
    assert_eq!(
        variable_named(&merged, "globalOnly").unwrap()["value"],
        "global-only"
    );
    assert_eq!(
        variable_named(&merged, "localOnly").unwrap()["scope"],
        "local"
    );

    // Case 2: GET single without scope resolves local-first.
    let single = client
        .get(format!(
            "{base_url}/runtime/tasks/{task_id}/variables/shared"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(single.status(), reqwest::StatusCode::OK);
    let single_body: Value = single.json().await.unwrap();
    assert_eq!(single_body["value"], "local-value");
    assert_eq!(single_body["scope"], "local");
}

#[tokio::test]
async fn post_scope_resolution_places_variables_on_local_or_execution() {
    let (client, base_url, engine) = start_test_server("rest-tvar-post-scope").await;
    let (_process_instance_id, execution_id, task_id) =
        deploy_and_start_user_task_process(&client, &base_url, &engine).await;

    // Case 3: POST without any scope creates task-local variables.
    let local_create = create_task_variables(
        &client,
        &base_url,
        &task_id,
        "",
        json!([
            { "name": "defaultLocal", "type": "string", "value": "one" },
            { "name": "defaultLocalTwo", "type": "integer", "value": 2 }
        ]),
    )
    .await;
    assert_eq!(local_create.status(), reqwest::StatusCode::CREATED);
    let local_body: Value = local_create.json().await.unwrap();
    assert_eq!(local_body.as_array().unwrap().len(), 2);
    assert_eq!(local_body[0]["scope"], "local");
    assert_eq!(local_body[1]["scope"], "local");
    assert_eq!(
        task_local_variable(&engine, &task_id, "defaultLocal"),
        Some(json!("one"))
    );
    assert_eq!(
        task_local_variable(&engine, &task_id, "defaultLocalTwo"),
        Some(json!(2))
    );
    assert_eq!(
        execution_variable(&engine, &execution_id, "defaultLocal"),
        None,
        "default POST scope must not touch the backing execution"
    );
    assert_eq!(
        execution_variable(&engine, &execution_id, "defaultLocalTwo"),
        None
    );

    // Case 4: POST with a per-variable body scope of "global" lands on the
    // backing execution.
    let global_create = create_task_variables(
        &client,
        &base_url,
        &task_id,
        "",
        json!([
            { "name": "bodyGlobal", "type": "string", "value": "g1", "scope": "global" },
            { "name": "bodyGlobalTwo", "type": "boolean", "value": true, "scope": "global" }
        ]),
    )
    .await;
    assert_eq!(global_create.status(), reqwest::StatusCode::CREATED);
    let global_body: Value = global_create.json().await.unwrap();
    assert_eq!(global_body[0]["scope"], "global");
    assert_eq!(global_body[1]["scope"], "global");
    assert_eq!(
        execution_variable(&engine, &execution_id, "bodyGlobal"),
        Some(json!("g1"))
    );
    assert_eq!(
        execution_variable(&engine, &execution_id, "bodyGlobalTwo"),
        Some(json!(true))
    );
    assert_eq!(task_local_variable(&engine, &task_id, "bodyGlobal"), None);
}

#[tokio::test]
async fn post_rejects_mixed_scopes_and_writes_nothing() {
    let (client, base_url, engine) = start_test_server("rest-tvar-mixed-scopes").await;
    let (_process_instance_id, _execution_id, task_id) =
        deploy_and_start_user_task_process(&client, &base_url, &engine).await;

    // Case 5: a batch whose variables resolve to different scopes is a 400.
    let mixed = create_task_variables(
        &client,
        &base_url,
        &task_id,
        "",
        json!([
            { "name": "mixedLocal", "value": 1, "scope": "local" },
            { "name": "mixedGlobal", "value": 2, "scope": "global" }
        ]),
    )
    .await;
    assert_eq!(mixed.status(), reqwest::StatusCode::BAD_REQUEST);
    let mixed_body: Value = mixed.json().await.unwrap();
    assert_eq!(
        mixed_body["details"].as_str().unwrap(),
        "Only allowed to update multiple variables in the same scope."
    );

    let variables = get_task_variables(&client, &base_url, &task_id, "").await;
    assert!(
        variable_named(&variables, "mixedLocal").is_none(),
        "rejected batch must not write the local variable"
    );
    assert!(
        variable_named(&variables, "mixedGlobal").is_none(),
        "rejected batch must not write the global variable"
    );
}

#[tokio::test]
async fn post_rejects_empty_array_and_conflicting_names_atomically() {
    let (client, base_url, engine) = start_test_server("rest-tvar-post-validation").await;
    let (_process_instance_id, _execution_id, task_id) =
        deploy_and_start_user_task_process(&client, &base_url, &engine).await;

    // Case 6: an empty array is a 400 with the Java message.
    let empty = create_task_variables(&client, &base_url, &task_id, "", json!([])).await;
    assert_eq!(empty.status(), reqwest::StatusCode::BAD_REQUEST);
    let empty_body: Value = empty.json().await.unwrap();
    assert_eq!(
        empty_body["details"].as_str().unwrap(),
        "Request did not contain a list of variables to create."
    );

    // Case 7: re-creating an existing variable on the same scope is a 409,
    // and a batch with one duplicate + one new variable writes nothing.
    let initial = create_task_variables(
        &client,
        &base_url,
        &task_id,
        "",
        json!([{ "name": "dup", "type": "string", "value": "original" }]),
    )
    .await;
    assert_eq!(initial.status(), reqwest::StatusCode::CREATED);

    let conflict = create_task_variables(
        &client,
        &base_url,
        &task_id,
        "",
        json!([
            { "name": "dup", "type": "string", "value": "changed" },
            { "name": "brandNew", "type": "string", "value": "new" }
        ]),
    )
    .await;
    assert_eq!(conflict.status(), reqwest::StatusCode::CONFLICT);
    let conflict_body: Value = conflict.json().await.unwrap();
    assert!(
        conflict_body["details"]
            .as_str()
            .unwrap()
            .contains("Variable 'dup' is already present on task"),
        "details: {}",
        conflict_body["details"]
    );

    let variables = get_task_variables(&client, &base_url, &task_id, "").await;
    assert_eq!(
        variable_named(&variables, "dup").unwrap()["value"],
        "original",
        "conflicting batch must not overwrite the existing variable"
    );
    assert!(
        variable_named(&variables, "brandNew").is_none(),
        "conflicting batch must be atomic: the new variable was not written"
    );
}

#[tokio::test]
async fn put_single_is_update_only_and_validates_body_name() {
    let (client, base_url, engine) = start_test_server("rest-tvar-put-single").await;
    let (_process_instance_id, _execution_id, task_id) =
        deploy_and_start_user_task_process(&client, &base_url, &engine).await;

    // Case 8: PUT on a variable absent from the resolved scope is a 404 with
    // the engine/Java message.
    let missing = client
        .put(format!(
            "{base_url}/runtime/tasks/{task_id}/variables/absent"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "name": "absent", "type": "integer", "value": 1 }))
        .send()
        .await
        .unwrap();
    assert_eq!(missing.status(), reqwest::StatusCode::NOT_FOUND);
    let missing_body: Value = missing.json().await.unwrap();
    assert_eq!(
        missing_body["details"].as_str().unwrap(),
        format!("Task '{task_id}' does not have a variable with name: 'absent'.")
    );

    // Case 9: a body name that differs from the path name is a 400.
    let create = create_task_variables(
        &client,
        &base_url,
        &task_id,
        "",
        json!([{ "name": "realName", "type": "string", "value": "v1" }]),
    )
    .await;
    assert_eq!(create.status(), reqwest::StatusCode::CREATED);

    let mismatch = client
        .put(format!(
            "{base_url}/runtime/tasks/{task_id}/variables/realName"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "name": "otherName", "type": "string", "value": "v2" }))
        .send()
        .await
        .unwrap();
    assert_eq!(mismatch.status(), reqwest::StatusCode::BAD_REQUEST);
    let mismatch_body: Value = mismatch.json().await.unwrap();
    assert_eq!(
        mismatch_body["details"].as_str().unwrap(),
        "Variable name in the body should be equal to the name used in the requested URL."
    );

    // A matching update succeeds on the local scope by default.
    let update = client
        .put(format!(
            "{base_url}/runtime/tasks/{task_id}/variables/realName"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "name": "realName", "type": "string", "value": "v2" }))
        .send()
        .await
        .unwrap();
    assert_eq!(update.status(), reqwest::StatusCode::OK);
    let update_body: Value = update.json().await.unwrap();
    assert_eq!(update_body["value"], "v2");
    assert_eq!(update_body["scope"], "local");
}

#[tokio::test]
async fn delete_single_defaults_to_local_and_collection_delete_keeps_globals() {
    let (client, base_url, engine) = start_test_server("rest-tvar-delete").await;
    let (_process_instance_id, _execution_id, task_id) =
        deploy_and_start_user_task_process(&client, &base_url, &engine).await;

    let global_create = create_task_variables(
        &client,
        &base_url,
        &task_id,
        "?scope=global",
        json!([
            { "name": "shadowed", "type": "string", "value": "global-value" },
            { "name": "globalSurvivor", "type": "string", "value": "global" }
        ]),
    )
    .await;
    assert_eq!(global_create.status(), reqwest::StatusCode::CREATED);
    let local_create = create_task_variables(
        &client,
        &base_url,
        &task_id,
        "",
        json!([
            { "name": "shadowed", "type": "string", "value": "local-value" },
            { "name": "localOnly", "type": "string", "value": "local" }
        ]),
    )
    .await;
    assert_eq!(local_create.status(), reqwest::StatusCode::CREATED);

    // Case 10: DELETE single without a scope removes the task-local variable;
    // the same-named global survives and GET single falls back to it.
    let delete_local = client
        .delete(format!(
            "{base_url}/runtime/tasks/{task_id}/variables/shadowed"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(delete_local.status(), reqwest::StatusCode::NO_CONTENT);
    assert_eq!(task_local_variable(&engine, &task_id, "shadowed"), None);

    let fallback = client
        .get(format!(
            "{base_url}/runtime/tasks/{task_id}/variables/shadowed"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(fallback.status(), reqwest::StatusCode::OK);
    let fallback_body: Value = fallback.json().await.unwrap();
    assert_eq!(fallback_body["scope"], "global");
    assert_eq!(fallback_body["value"], "global-value");

    // Case 11: DELETE on the collection removes only the remaining locals.
    let delete_all = client
        .delete(format!("{base_url}/runtime/tasks/{task_id}/variables"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(delete_all.status(), reqwest::StatusCode::NO_CONTENT);

    let remaining = get_task_variables(&client, &base_url, &task_id, "").await;
    assert!(variable_named(&remaining, "localOnly").is_none());
    assert!(
        remaining
            .iter()
            .all(|variable| variable["scope"] == "global"),
        "only globals survive a collection delete: {remaining:?}"
    );
    assert!(variable_named(&remaining, "globalSurvivor").is_some());
    assert!(variable_named(&remaining, "shadowed").is_some());
}

#[tokio::test]
async fn invalid_scope_in_query_or_body_is_rejected() {
    let (client, base_url, engine) = start_test_server("rest-tvar-invalid-scope").await;
    let (_process_instance_id, _execution_id, task_id) =
        deploy_and_start_user_task_process(&client, &base_url, &engine).await;

    // Case 12a: an invalid ?scope= query keeps the Rust extension message.
    let bad_query = client
        .get(format!(
            "{base_url}/runtime/tasks/{task_id}/variables?scope=bogus"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(bad_query.status(), reqwest::StatusCode::BAD_REQUEST);
    let bad_query_body: Value = bad_query.json().await.unwrap();
    assert_eq!(
        bad_query_body["details"].as_str().unwrap(),
        "Unsupported task variable scope 'bogus'"
    );

    // Case 12b: an invalid per-variable body scope uses the Java message.
    let bad_body = create_task_variables(
        &client,
        &base_url,
        &task_id,
        "",
        json!([{ "name": "scoped", "value": 1, "scope": "bogus" }]),
    )
    .await;
    assert_eq!(bad_body.status(), reqwest::StatusCode::BAD_REQUEST);
    let bad_body_body: Value = bad_body.json().await.unwrap();
    assert_eq!(
        bad_body_body["details"].as_str().unwrap(),
        "Invalid variable scope: 'bogus'"
    );
}

#[tokio::test]
async fn multipart_post_without_file_is_rejected() {
    let (client, base_url, engine) = start_test_server("rest-tvar-multipart-no-file").await;
    let (_process_instance_id, _execution_id, task_id) =
        deploy_and_start_user_task_process(&client, &base_url, &engine).await;

    // Case 13: multipart without any file part is a 400 with the Java message.
    let boundary = "flowable-rust-tvar-boundary";
    let mut body = Vec::new();
    multipart_text_field(&mut body, boundary, "name", "noFileVar");
    multipart_text_field(&mut body, boundary, "type", "binary");
    multipart_close(&mut body, boundary);

    let response = send_multipart(
        &client,
        reqwest::Method::POST,
        format!("{base_url}/runtime/tasks/{task_id}/variables"),
        boundary,
        body,
    )
    .await;
    assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
    let response_body: Value = response.json().await.unwrap();
    assert_eq!(
        response_body["details"].as_str().unwrap(),
        "No file content was found in request body."
    );
}

#[tokio::test]
async fn multipart_binary_variable_lifecycle() {
    let (client, base_url, engine) = start_test_server("rest-tvar-multipart-binary").await;
    let (_process_instance_id, _execution_id, task_id) =
        deploy_and_start_user_task_process(&client, &base_url, &engine).await;

    // Case 14: multipart POST creates (single-object response, default local
    // scope), multipart PUT updates, GET .../data streams the updated bytes.
    let boundary = "flowable-rust-tvar-binary";
    let original_bytes = b"\x00\x01\x02original-binary".to_vec();
    let mut post_body = Vec::new();
    multipart_text_field(&mut post_body, boundary, "name", "uploadDoc");
    multipart_text_field(&mut post_body, boundary, "type", "binary");
    multipart_file_field(&mut post_body, boundary, "file", "doc.bin", &original_bytes);
    multipart_close(&mut post_body, boundary);

    let create = send_multipart(
        &client,
        reqwest::Method::POST,
        format!("{base_url}/runtime/tasks/{task_id}/variables"),
        boundary,
        post_body,
    )
    .await;
    assert_eq!(create.status(), reqwest::StatusCode::CREATED);
    let create_body: Value = create.json().await.unwrap();
    assert!(
        create_body.is_object(),
        "multipart create returns a single variable object: {create_body}"
    );
    assert_eq!(create_body["name"], "uploadDoc");
    assert_eq!(create_body["type"], "binary");
    assert_eq!(create_body["scope"], "local");

    let updated_bytes = b"updated-binary-bytes".to_vec();
    let mut put_body = Vec::new();
    multipart_text_field(&mut put_body, boundary, "name", "uploadDoc");
    multipart_text_field(&mut put_body, boundary, "type", "binary");
    multipart_file_field(&mut put_body, boundary, "file", "doc.bin", &updated_bytes);
    multipart_close(&mut put_body, boundary);

    let update = send_multipart(
        &client,
        reqwest::Method::PUT,
        format!("{base_url}/runtime/tasks/{task_id}/variables/uploadDoc"),
        boundary,
        put_body,
    )
    .await;
    assert_eq!(update.status(), reqwest::StatusCode::OK);
    let update_body: Value = update.json().await.unwrap();
    assert_eq!(update_body["name"], "uploadDoc");
    assert_eq!(update_body["scope"], "local");

    let data = client
        .get(format!(
            "{base_url}/runtime/tasks/{task_id}/variables/uploadDoc/data"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(data.status(), reqwest::StatusCode::OK);
    assert_eq!(
        data.headers()
            .get(reqwest::header::CONTENT_TYPE)
            .unwrap()
            .to_str()
            .unwrap(),
        "application/octet-stream"
    );
    assert_eq!(data.bytes().await.unwrap().as_ref(), updated_bytes.as_slice());
}

#[tokio::test]
async fn multipart_serializable_variable_round_trips_opaque_bytes() {
    let (client, base_url, engine) = start_test_server("rest-tvar-multipart-serializable").await;
    let (_process_instance_id, _execution_id, task_id) =
        deploy_and_start_user_task_process(&client, &base_url, &engine).await;

    // Case 15: serializable bytes are stored and streamed back opaquely with
    // the Java serialized-object content type (0xAC 0xED = Java stream magic).
    let boundary = "flowable-rust-tvar-serializable";
    let opaque_bytes: Vec<u8> = vec![
        0xAC, 0xED, 0x00, 0x05, 0x74, 0x00, 0x0B, b'f', b'l', b'o', b'w', b'a', b'b', b'l', b'e',
        0x00, 0x01,
    ];
    let mut post_body = Vec::new();
    multipart_text_field(&mut post_body, boundary, "name", "serializedObject");
    multipart_text_field(&mut post_body, boundary, "type", "serializable");
    multipart_file_field(&mut post_body, boundary, "file", "object.ser", &opaque_bytes);
    multipart_close(&mut post_body, boundary);

    let create = send_multipart(
        &client,
        reqwest::Method::POST,
        format!("{base_url}/runtime/tasks/{task_id}/variables"),
        boundary,
        post_body,
    )
    .await;
    assert_eq!(create.status(), reqwest::StatusCode::CREATED);
    let create_body: Value = create.json().await.unwrap();
    assert_eq!(create_body["name"], "serializedObject");
    assert_eq!(create_body["type"], "serializable");
    assert_eq!(create_body["scope"], "local");

    let data = client
        .get(format!(
            "{base_url}/runtime/tasks/{task_id}/variables/serializedObject/data"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(data.status(), reqwest::StatusCode::OK);
    assert_eq!(
        data.headers()
            .get(reqwest::header::CONTENT_TYPE)
            .unwrap()
            .to_str()
            .unwrap(),
        "application/x-java-serialized-object"
    );
    assert_eq!(
        data.bytes().await.unwrap().as_ref(),
        opaque_bytes.as_slice(),
        "serializable bytes round-trip byte-identical and are never deserialized"
    );
}

#[tokio::test]
async fn single_object_post_extension_is_accepted() {
    let (client, base_url, engine) = start_test_server("rest-tvar-single-object").await;
    let (_process_instance_id, _execution_id, task_id) =
        deploy_and_start_user_task_process(&client, &base_url, &engine).await;

    // Case 16 (Rust extension): a single JSON object body is accepted like a
    // one-element array.
    let create = create_task_variables(
        &client,
        &base_url,
        &task_id,
        "",
        json!({ "name": "singleObject", "type": "string", "value": "extension" }),
    )
    .await;
    assert_eq!(create.status(), reqwest::StatusCode::CREATED);
    let create_body: Value = create.json().await.unwrap();
    assert_eq!(create_body[0]["name"], "singleObject");
    assert_eq!(create_body[0]["scope"], "local");
    assert_eq!(
        task_local_variable(&engine, &task_id, "singleObject"),
        Some(json!("extension"))
    );
}

#[tokio::test]
async fn put_collection_extension_upserts_with_default_global_scope() {
    let (client, base_url, engine) = start_test_server("rest-tvar-put-collection").await;
    let (_process_instance_id, execution_id, task_id) =
        deploy_and_start_user_task_process(&client, &base_url, &engine).await;

    // Case 17 (Rust extension, no Java counterpart): PUT on the collection is
    // an upsert and keeps its pre-parity default-global scope.
    let create = client
        .put(format!("{base_url}/runtime/tasks/{task_id}/variables"))
        .basic_auth("admin", Some("test"))
        .json(&json!([{ "name": "upserted", "type": "string", "value": "v1" }]))
        .send()
        .await
        .unwrap();
    assert_eq!(create.status(), reqwest::StatusCode::OK);
    let create_body: Value = create.json().await.unwrap();
    assert_eq!(create_body[0]["scope"], "global");
    assert_eq!(
        execution_variable(&engine, &execution_id, "upserted"),
        Some(json!("v1"))
    );

    let update = client
        .put(format!("{base_url}/runtime/tasks/{task_id}/variables"))
        .basic_auth("admin", Some("test"))
        .json(&json!([{ "name": "upserted", "type": "string", "value": "v2" }]))
        .send()
        .await
        .unwrap();
    assert_eq!(
        update.status(),
        reqwest::StatusCode::OK,
        "collection PUT is an upsert: re-PUTting an existing name succeeds"
    );
    assert_eq!(
        execution_variable(&engine, &execution_id, "upserted"),
        Some(json!("v2"))
    );
}

#[tokio::test]
async fn put_variable_data_extension_round_trips_raw_bytes() {
    let (client, base_url, engine) = start_test_server("rest-tvar-put-data").await;
    let (_process_instance_id, _execution_id, task_id) =
        deploy_and_start_user_task_process(&client, &base_url, &engine).await;

    // Case 18 (Rust extension): raw bytes PUT on .../variables/:name/data.
    // Without ?scope=local the extension writes the execution variable.
    let create = create_task_variables(
        &client,
        &base_url,
        &task_id,
        "?scope=global",
        json!([{ "name": "rawDoc", "type": "binary", "value": null }]),
    )
    .await;
    assert_eq!(create.status(), reqwest::StatusCode::CREATED);

    let raw_bytes = b"\xde\xad\xbe\xefraw-extension-bytes".to_vec();
    let update = client
        .put(format!(
            "{base_url}/runtime/tasks/{task_id}/variables/rawDoc/data"
        ))
        .basic_auth("admin", Some("test"))
        .header(reqwest::header::CONTENT_TYPE, "application/octet-stream")
        .body(raw_bytes.clone())
        .send()
        .await
        .unwrap();
    assert_eq!(update.status(), reqwest::StatusCode::NO_CONTENT);

    let data = client
        .get(format!(
            "{base_url}/runtime/tasks/{task_id}/variables/rawDoc/data"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(data.status(), reqwest::StatusCode::OK);
    assert_eq!(
        data.headers()
            .get(reqwest::header::CONTENT_TYPE)
            .unwrap()
            .to_str()
            .unwrap(),
        "application/octet-stream"
    );
    assert_eq!(data.bytes().await.unwrap().as_ref(), raw_bytes.as_slice());
}

#[tokio::test]
async fn query_scope_fallback_and_body_scope_precedence() {
    let (client, base_url, engine) = start_test_server("rest-tvar-scope-precedence").await;
    let (_process_instance_id, execution_id, task_id) =
        deploy_and_start_user_task_process(&client, &base_url, &engine).await;

    // Case 19 (Rust extension): ?scope= is the fallback when the body carries
    // no scope.
    let query_global = create_task_variables(
        &client,
        &base_url,
        &task_id,
        "?scope=global",
        json!([{ "name": "queryGlobal", "type": "string", "value": "via-query" }]),
    )
    .await;
    assert_eq!(query_global.status(), reqwest::StatusCode::CREATED);
    let query_global_body: Value = query_global.json().await.unwrap();
    assert_eq!(query_global_body[0]["scope"], "global");
    assert_eq!(
        execution_variable(&engine, &execution_id, "queryGlobal"),
        Some(json!("via-query"))
    );
    assert_eq!(task_local_variable(&engine, &task_id, "queryGlobal"), None);

    // Case 20: a per-variable body scope wins over ?scope= (Java-standard
    // bodies must not be rewritten by the query extension).
    let body_wins = create_task_variables(
        &client,
        &base_url,
        &task_id,
        "?scope=global",
        json!([{ "name": "bodyWins", "type": "string", "value": "local", "scope": "local" }]),
    )
    .await;
    assert_eq!(body_wins.status(), reqwest::StatusCode::CREATED);
    let body_wins_body: Value = body_wins.json().await.unwrap();
    assert_eq!(body_wins_body[0]["scope"], "local");
    assert_eq!(
        task_local_variable(&engine, &task_id, "bodyWins"),
        Some(json!("local"))
    );
    assert_eq!(
        execution_variable(&engine, &execution_id, "bodyWins"),
        None,
        "body scope must beat the query scope"
    );
}
