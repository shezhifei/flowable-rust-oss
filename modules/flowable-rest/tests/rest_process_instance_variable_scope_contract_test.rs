//! Contract tests for the `scope` semantics of the process-instance-variable
//! REST endpoints.
//!
//! Java reference: `ProcessInstanceVariableResource` /
//! `ProcessInstanceVariableCollectionResource` /
//! `ProcessInstanceVariableDataResource`, which share
//! `BaseExecutionVariableResource` / `BaseVariableCollectionResource` with the
//! execution-variable resources. The process-instance-specific rules under
//! test:
//!
//! * the endpoints always resolve to the process instance's own execution row
//!   (a root, `parentId == null`), so there is no parent (global) scope:
//!   a GLOBAL collection read is an EMPTY list, a GLOBAL collection write is a
//!   400, and GLOBAL single-variable reads/updates/deletes are 404s;
//! * single-variable responses carry `scope = null`
//!   (`ProcessInstanceVariableResource.constructRestVariable` overrides the
//!   scope away), while collection responses label every variable `local`
//!   (`ProcessInstanceVariableCollectionResource.addLocalVariables`);
//! * writes default to the LOCAL scope; create conflicts (409) and update
//!   misses (404) are decided on the requested scope;
//! * the Java collection PUT (upsert) and DELETE (clear local scope) endpoints
//!   exist and behave like their execution-endpoint counterparts;
//! * an unknown scope string is a 400 ("Invalid variable scope: '<scope>'").

use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::persistence::runtime_store::job_handler_types;
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

const SIMPLE_TASK_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
  <process id="piVarScopeProcess" name="Process Instance Variable Scope" isExecutable="true">
    <startEvent id="start" />
    <sequenceFlow id="flow1" sourceRef="start" targetRef="reviewTask" />
    <userTask id="reviewTask" name="Review" />
    <sequenceFlow id="flow2" sourceRef="reviewTask" targetRef="end" />
    <endEvent id="end" />
  </process>
</definitions>"#;

async fn deploy(
    client: &reqwest::Client,
    base_url: &str,
    engine: &ProcessEngine,
    resource_name: &str,
) -> String {
    let deploy = client
        .post(format!("{base_url}/repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": resource_name,
            "resourceName": format!("{resource_name}.bpmn20.xml"),
            "resource": SIMPLE_TASK_XML
        }))
        .send()
        .await
        .unwrap();
    assert!(deploy.status().is_success(), "deploy failed");
    engine
        .get_repository_service()
        .get_process_definition_ids()
        .unwrap()[0]
        .clone()
}

async fn start_process_instance(
    client: &reqwest::Client,
    base_url: &str,
    process_definition_id: &str,
    variables: Value,
) -> String {
    let start = client
        .post(format!("{base_url}/runtime/process-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "processDefinitionId": process_definition_id,
            "variables": variables
        }))
        .send()
        .await
        .unwrap();
    assert!(start.status().is_success(), "start failed");
    let body: Value = start.json().await.unwrap();
    body["id"].as_str().unwrap().to_string()
}

async fn deploy_and_start(
    client: &reqwest::Client,
    base_url: &str,
    engine: &ProcessEngine,
    resource_name: &str,
) -> String {
    let definition_id = deploy(client, base_url, engine, resource_name).await;
    start_process_instance(client, base_url, &definition_id, json!([])).await
}

async fn create_variable(
    client: &reqwest::Client,
    base_url: &str,
    process_instance_id: &str,
    variables: Value,
) -> reqwest::Response {
    client
        .post(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}/variables"
        ))
        .basic_auth("admin", Some("test"))
        .json(&variables)
        .send()
        .await
        .unwrap()
}

/// Java `setSimpleVariable`: a write without a scope defaults to LOCAL. On the
/// process instance row that is the execution's own scope, so the variable is
/// visible through the (narrow) local-variable API as well as the merged one.
#[tokio::test]
async fn create_without_scope_writes_to_the_local_scope() {
    let (client, base_url, engine) = start_test_server("rest-pi-var-scope-default-local").await;
    let process_instance_id =
        deploy_and_start(&client, &base_url, &engine, "pi_scope_default_local").await;

    let create = create_variable(
        &client,
        &base_url,
        &process_instance_id,
        json!([{ "name": "note", "type": "string", "value": "local-value" }]),
    )
    .await;
    assert_eq!(create.status(), reqwest::StatusCode::CREATED);
    let body: Value = create.json().await.unwrap();
    assert_eq!(body[0]["scope"], "local");

    assert_eq!(
        engine
            .get_runtime_service()
            .get_variable_local(process_instance_id.clone(), "note".to_string())
            .unwrap(),
        Some(json!("local-value")),
        "a scope-less write must land on the execution's own (local) scope"
    );
    // Regression guard: the merged read must keep seeing the variable.
    assert_eq!(
        engine
            .get_runtime_service()
            .get_variable(process_instance_id, "note".to_string())
            .unwrap(),
        Some(json!("local-value"))
    );
}

/// Java `ProcessInstanceVariableCollectionResource.addLocalVariables` labels
/// every variable LOCAL, and its `addGlobalVariables` override is a no-op, so
/// `?scope=global` yields an empty list (not an error).
#[tokio::test]
async fn collection_read_labels_local_and_global_scope_is_empty() {
    let (client, base_url, engine) = start_test_server("rest-pi-var-scope-collection-read").await;
    let process_instance_id =
        deploy_and_start(&client, &base_url, &engine, "pi_scope_collection_read").await;

    let create = create_variable(
        &client,
        &base_url,
        &process_instance_id,
        json!([{ "name": "listed", "type": "string", "value": "v" }]),
    )
    .await;
    assert_eq!(create.status(), reqwest::StatusCode::CREATED);

    for url in [
        format!("{base_url}/runtime/process-instances/{process_instance_id}/variables"),
        format!("{base_url}/runtime/process-instances/{process_instance_id}/variables?scope=local"),
    ] {
        let read = client
            .get(&url)
            .basic_auth("admin", Some("test"))
            .send()
            .await
            .unwrap();
        assert_eq!(read.status(), reqwest::StatusCode::OK, "reading {url}");
        let body: Value = read.json().await.unwrap();
        let entries = body.as_array().unwrap();
        assert_eq!(entries.len(), 1, "expected exactly one variable from {url}");
        assert_eq!(entries[0]["name"], "listed");
        assert_eq!(entries[0]["scope"], "local");
    }

    let global = client
        .get(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}/variables?scope=global"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(global.status(), reqwest::StatusCode::OK);
    let body: Value = global.json().await.unwrap();
    assert!(
        body.as_array().unwrap().is_empty(),
        "a process instance has no global scope, got {body}"
    );
}

/// Java `BaseVariableCollectionResource.createExecutionVariable`: a GLOBAL
/// collection write needs a parent execution; the process instance row has
/// none, so the write is a 400.
#[tokio::test]
async fn create_with_global_scope_is_rejected() {
    let (client, base_url, engine) = start_test_server("rest-pi-var-scope-global-create").await;
    let process_instance_id =
        deploy_and_start(&client, &base_url, &engine, "pi_scope_global_create").await;

    let create = create_variable(
        &client,
        &base_url,
        &process_instance_id,
        json!([{ "name": "g", "type": "string", "value": "v", "scope": "global" }]),
    )
    .await;
    assert_eq!(create.status(), reqwest::StatusCode::BAD_REQUEST);
    let body: Value = create.json().await.unwrap();
    assert_eq!(
        body["details"],
        format!(
            "Cannot set global variables on execution '{process_instance_id}', task is not part of process."
        )
    );
}

/// Java: a POST on an existing name is a 409 decided on the requested scope.
#[tokio::test]
async fn create_conflict_on_the_same_name() {
    let (client, base_url, engine) = start_test_server("rest-pi-var-scope-conflict").await;
    let process_instance_id =
        deploy_and_start(&client, &base_url, &engine, "pi_scope_conflict").await;

    let first = create_variable(
        &client,
        &base_url,
        &process_instance_id,
        json!([{ "name": "dup", "type": "string", "value": "one" }]),
    )
    .await;
    assert_eq!(first.status(), reqwest::StatusCode::CREATED);

    let conflict = create_variable(
        &client,
        &base_url,
        &process_instance_id,
        json!([{ "name": "dup", "type": "string", "value": "two" }]),
    )
    .await;
    assert_eq!(conflict.status(), reqwest::StatusCode::CONFLICT);
    let body: Value = conflict.json().await.unwrap();
    assert_eq!(
        body["details"],
        format!("Variable 'dup' is already present on execution '{process_instance_id}'.")
    );

    // The failed create must not have overwritten the original value.
    assert_eq!(
        engine
            .get_runtime_service()
            .get_variable(process_instance_id, "dup".to_string())
            .unwrap(),
        Some(json!("one"))
    );
}

/// Java: all variables of one request must resolve to the same scope.
#[tokio::test]
async fn mixed_scopes_in_one_request_are_rejected() {
    let (client, base_url, engine) = start_test_server("rest-pi-var-scope-mixed").await;
    let process_instance_id = deploy_and_start(&client, &base_url, &engine, "pi_scope_mixed").await;

    let mixed = create_variable(
        &client,
        &base_url,
        &process_instance_id,
        json!([
            { "name": "a", "type": "string", "value": "1", "scope": "local" },
            { "name": "b", "type": "string", "value": "2", "scope": "global" }
        ]),
    )
    .await;
    assert_eq!(mixed.status(), reqwest::StatusCode::BAD_REQUEST);
    let body: Value = mixed.json().await.unwrap();
    assert_eq!(
        body["details"],
        "Only allowed to update multiple variables in the same scope."
    );

    assert_eq!(
        engine
            .get_runtime_service()
            .get_variable(process_instance_id, "a".to_string())
            .unwrap(),
        None,
        "the batch is validated before any write"
    );
}

/// Java `setSimpleVariable(isNew = false)`: a single-variable PUT is
/// update-only on the resolved scope and 404s when the variable is absent.
#[tokio::test]
async fn update_requires_the_variable_to_exist() {
    let (client, base_url, engine) = start_test_server("rest-pi-var-scope-update").await;
    let process_instance_id =
        deploy_and_start(&client, &base_url, &engine, "pi_scope_update").await;

    let missing = client
        .put(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}/variables/missing"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "name": "missing", "type": "string", "value": "v" }))
        .send()
        .await
        .unwrap();
    assert_eq!(missing.status(), reqwest::StatusCode::NOT_FOUND);
    let body: Value = missing.json().await.unwrap();
    assert_eq!(
        body["details"],
        format!("Execution '{process_instance_id}' does not have a variable with name: 'missing'.")
    );

    let create = create_variable(
        &client,
        &base_url,
        &process_instance_id,
        json!([{ "name": "existing", "type": "string", "value": "v1" }]),
    )
    .await;
    assert_eq!(create.status(), reqwest::StatusCode::CREATED);

    let updated = client
        .put(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}/variables/existing"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "name": "existing", "type": "string", "value": "v2" }))
        .send()
        .await
        .unwrap();
    assert_eq!(updated.status(), reqwest::StatusCode::OK);
    let body: Value = updated.json().await.unwrap();
    assert_eq!(body["value"], "v2");
    assert_eq!(
        engine
            .get_runtime_service()
            .get_variable(process_instance_id, "existing".to_string())
            .unwrap(),
        Some(json!("v2"))
    );
}

/// Java `ProcessInstanceVariableResource.constructRestVariable` overrides the
/// scope to null: single-variable responses carry no scope label, while the
/// collection endpoints label variables `local`.
#[tokio::test]
async fn single_variable_responses_have_no_scope_label() {
    let (client, base_url, engine) = start_test_server("rest-pi-var-scope-null-label").await;
    let process_instance_id =
        deploy_and_start(&client, &base_url, &engine, "pi_scope_null_label").await;

    let create = create_variable(
        &client,
        &base_url,
        &process_instance_id,
        json!([{ "name": "unscoped", "type": "string", "value": "v1" }]),
    )
    .await;
    assert_eq!(create.status(), reqwest::StatusCode::CREATED);
    let body: Value = create.json().await.unwrap();
    assert_eq!(
        body[0]["scope"], "local",
        "collection responses label variables local"
    );

    let read = client
        .get(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}/variables/unscoped"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(read.status(), reqwest::StatusCode::OK);
    let body: Value = read.json().await.unwrap();
    assert_eq!(body["value"], "v1");
    assert!(
        body["scope"].is_null(),
        "single-variable GET responses carry scope = null, got {body}"
    );

    let updated = client
        .put(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}/variables/unscoped"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "name": "unscoped", "type": "string", "value": "v2" }))
        .send()
        .await
        .unwrap();
    assert_eq!(updated.status(), reqwest::StatusCode::OK);
    let body: Value = updated.json().await.unwrap();
    assert!(
        body["scope"].is_null(),
        "single-variable PUT responses carry scope = null, got {body}"
    );
}

/// Java: on the process instance row `hasVariableOnScope(GLOBAL)` is always
/// false (`parentId == null`), so every explicit-GLOBAL single-variable
/// operation is a 404 — the `setVariable` parent fallback is unreachable.
#[tokio::test]
async fn single_variable_global_scope_is_not_found() {
    let (client, base_url, engine) = start_test_server("rest-pi-var-scope-global-404").await;
    let process_instance_id =
        deploy_and_start(&client, &base_url, &engine, "pi_scope_global_404").await;

    let create = create_variable(
        &client,
        &base_url,
        &process_instance_id,
        json!([{ "name": "onlyLocal", "type": "string", "value": "v" }]),
    )
    .await;
    assert_eq!(create.status(), reqwest::StatusCode::CREATED);

    let read = client
        .get(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}/variables/onlyLocal?scope=global"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(read.status(), reqwest::StatusCode::NOT_FOUND);
    let body: Value = read.json().await.unwrap();
    assert_eq!(
        body["details"],
        format!(
            "Execution '{process_instance_id}' does not have a variable with name: 'onlyLocal'."
        )
    );

    let update = client
        .put(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}/variables/onlyLocal"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "onlyLocal", "type": "string", "value": "v2", "scope": "global"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(update.status(), reqwest::StatusCode::NOT_FOUND);
    let body: Value = update.json().await.unwrap();
    assert_eq!(
        body["details"],
        format!(
            "Execution '{process_instance_id}' does not have a variable with name: 'onlyLocal'."
        )
    );

    let delete = client
        .delete(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}/variables/onlyLocal?scope=global"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(delete.status(), reqwest::StatusCode::NOT_FOUND);
    let body: Value = delete.json().await.unwrap();
    assert_eq!(
        body["details"],
        format!(
            "Execution '{process_instance_id}' does not have a variable 'onlyLocal' in scope global"
        )
    );

    // The variable itself survived every failed global operation.
    assert_eq!(
        engine
            .get_runtime_service()
            .get_variable(process_instance_id, "onlyLocal".to_string())
            .unwrap(),
        Some(json!("v"))
    );
}

/// Java `deleteVariable`: defaults to LOCAL, removes the variable, and 404s
/// with the scope label in the message when the variable is absent.
#[tokio::test]
async fn delete_defaults_to_the_local_scope() {
    let (client, base_url, engine) = start_test_server("rest-pi-var-scope-delete").await;
    let process_instance_id =
        deploy_and_start(&client, &base_url, &engine, "pi_scope_delete").await;

    let create = create_variable(
        &client,
        &base_url,
        &process_instance_id,
        json!([{ "name": "removeMe", "type": "string", "value": "v" }]),
    )
    .await;
    assert_eq!(create.status(), reqwest::StatusCode::CREATED);

    let delete = client
        .delete(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}/variables/removeMe"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(delete.status(), reqwest::StatusCode::NO_CONTENT);
    assert_eq!(
        engine
            .get_runtime_service()
            .get_variable(process_instance_id.clone(), "removeMe".to_string())
            .unwrap(),
        None,
        "the variable must be gone after the delete"
    );

    let missing = client
        .delete(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}/variables/removeMe"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(missing.status(), reqwest::StatusCode::NOT_FOUND);
    let body: Value = missing.json().await.unwrap();
    assert_eq!(
        body["details"],
        format!(
            "Execution '{process_instance_id}' does not have a variable 'removeMe' in scope local"
        )
    );
}

/// Java `RestVariable.getScopeFromString`: an unknown scope is a 400, from the
/// body and from the query string alike.
#[tokio::test]
async fn invalid_scope_is_a_bad_request() {
    let (client, base_url, engine) = start_test_server("rest-pi-var-scope-invalid").await;
    let process_instance_id =
        deploy_and_start(&client, &base_url, &engine, "pi_scope_invalid").await;

    let body_scope = create_variable(
        &client,
        &base_url,
        &process_instance_id,
        json!([{ "name": "v", "type": "string", "value": "x", "scope": "sideways" }]),
    )
    .await;
    assert_eq!(body_scope.status(), reqwest::StatusCode::BAD_REQUEST);
    let body: Value = body_scope.json().await.unwrap();
    assert_eq!(body["details"], "Invalid variable scope: 'sideways'");

    let query_scope = client
        .get(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}/variables?scope=sideways"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(query_scope.status(), reqwest::StatusCode::BAD_REQUEST);
}

/// Java `createOrUpdateProcessVariable` (collection PUT) upserts without
/// conflict; `deleteLocalProcessVariable` (collection DELETE) clears the whole
/// local scope of the process instance.
#[tokio::test]
async fn collection_put_upserts_and_collection_delete_clears_all_variables() {
    let (client, base_url, engine) = start_test_server("rest-pi-var-scope-collection-rw").await;
    let process_instance_id =
        deploy_and_start(&client, &base_url, &engine, "pi_scope_collection_rw").await;

    let create = create_variable(
        &client,
        &base_url,
        &process_instance_id,
        json!([{ "name": "kept", "type": "string", "value": "v1" }]),
    )
    .await;
    assert_eq!(create.status(), reqwest::StatusCode::CREATED);

    let upsert = client
        .put(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}/variables"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!([
            { "name": "kept", "type": "string", "value": "v2" },
            { "name": "added", "type": "string", "value": "new" }
        ]))
        .send()
        .await
        .unwrap();
    assert_eq!(upsert.status(), reqwest::StatusCode::CREATED);
    let body: Value = upsert.json().await.unwrap();
    assert_eq!(body.as_array().unwrap().len(), 2);

    // A GLOBAL upsert on the process instance is rejected like the POST.
    let global_upsert = client
        .put(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}/variables"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!([{ "name": "g", "type": "string", "value": "v", "scope": "global" }]))
        .send()
        .await
        .unwrap();
    assert_eq!(global_upsert.status(), reqwest::StatusCode::BAD_REQUEST);

    let delete_all = client
        .delete(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}/variables"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(delete_all.status(), reqwest::StatusCode::NO_CONTENT);

    let list = client
        .get(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}/variables"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    let body: Value = list.json().await.unwrap();
    assert!(
        body.as_array().unwrap().is_empty(),
        "the collection DELETE must clear every variable, got {body}"
    );
}

/// Java: the async variants run the same synchronous scope validation; only the
/// write itself is deferred to a `set-async-variables` job.
#[tokio::test]
async fn async_endpoints_apply_the_same_scope_rules() {
    let (client, base_url, engine) = start_test_server("rest-pi-var-scope-async").await;
    let process_instance_id = deploy_and_start(&client, &base_url, &engine, "pi_scope_async").await;

    let global_create = client
        .post(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}/variables-async"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!([{ "name": "g", "type": "string", "value": "v", "scope": "global" }]))
        .send()
        .await
        .unwrap();
    assert_eq!(global_create.status(), reqwest::StatusCode::BAD_REQUEST);

    let missing_update = client
        .put(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}/variables-async/missing"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "name": "missing", "type": "string", "value": "v" }))
        .send()
        .await
        .unwrap();
    assert_eq!(missing_update.status(), reqwest::StatusCode::NOT_FOUND);

    let create = client
        .post(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}/variables-async"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!([{ "name": "asyncVar", "type": "string", "value": "v" }]))
        .send()
        .await
        .unwrap();
    // Java parity: the collection async POST has no `@ResponseStatus`
    // override, so `BaseVariableCollectionResource.createExecutionVariable`'s
    // unconditional `setStatus(201)` applies.
    assert_eq!(create.status(), reqwest::StatusCode::CREATED);

    // The 201 only means the job was scheduled: the value is pending until the
    // async executor runs it.
    assert_eq!(
        engine
            .get_runtime_service()
            .get_variable_local(process_instance_id.clone(), "asyncVar".to_string())
            .unwrap(),
        None,
        "the async write must not be visible before the job runs"
    );
    let jobs: Vec<String> = engine
        .get_management_service()
        .list_executable_jobs()
        .into_iter()
        .filter(|job| job.handler_type.as_deref() == Some(job_handler_types::SET_ASYNC_VARIABLES))
        .map(|job| job.timer_job_id)
        .collect();
    assert_eq!(
        jobs.len(),
        1,
        "the async write must schedule exactly one set-async-variables job"
    );
    engine
        .get_management_service()
        .execute_job(&jobs[0])
        .expect("the set-async-variables job should execute successfully");

    assert_eq!(
        engine
            .get_runtime_service()
            .get_variable_local(process_instance_id, "asyncVar".to_string())
            .unwrap(),
        Some(json!("v")),
        "a scope-less async write must land on the local scope"
    );
}

/// Java `getVariableDataByteArray` resolves the variable through
/// `getVariableFromRequest`, so the data endpoint applies the same scope rules.
#[tokio::test]
async fn variable_data_get_applies_scope() {
    let (client, base_url, engine) = start_test_server("rest-pi-var-scope-data").await;
    let process_instance_id = deploy_and_start(&client, &base_url, &engine, "pi_scope_data").await;

    let create = create_variable(
        &client,
        &base_url,
        &process_instance_id,
        json!([{ "name": "plain", "type": "string", "value": "v" }]),
    )
    .await;
    assert_eq!(create.status(), reqwest::StatusCode::CREATED);

    let read = client
        .get(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}/variables/plain/data?scope=local"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(read.status(), reqwest::StatusCode::OK);

    let global = client
        .get(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}/variables/plain/data?scope=global"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(global.status(), reqwest::StatusCode::NOT_FOUND);
}

/// Parity pin for the shared mutation commands: on a ROOT execution the Java
/// single-variable endpoints run `hasVariableOnScope` before the write, so an
/// explicit-GLOBAL update or delete is a 404 (not the collection write's 400).
#[tokio::test]
async fn execution_root_single_global_update_and_delete_are_not_found() {
    let (client, base_url, engine) = start_test_server("rest-pi-var-scope-exec-root").await;
    let process_instance_id =
        deploy_and_start(&client, &base_url, &engine, "pi_scope_exec_root").await;
    // Simple topology: the root execution row id equals the process instance id.
    let root_id = process_instance_id.clone();

    let create = client
        .post(format!("{base_url}/runtime/executions/{root_id}/variables"))
        .basic_auth("admin", Some("test"))
        .json(&json!([{ "name": "v", "type": "string", "value": "x" }]))
        .send()
        .await
        .unwrap();
    assert_eq!(create.status(), reqwest::StatusCode::CREATED);

    let update = client
        .put(format!(
            "{base_url}/runtime/executions/{root_id}/variables/v"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "name": "v", "type": "string", "value": "y", "scope": "global" }))
        .send()
        .await
        .unwrap();
    assert_eq!(update.status(), reqwest::StatusCode::NOT_FOUND);
    let body: Value = update.json().await.unwrap();
    assert_eq!(
        body["details"],
        format!("Execution '{root_id}' does not have a variable with name: 'v'.")
    );

    let delete = client
        .delete(format!(
            "{base_url}/runtime/executions/{root_id}/variables/v?scope=global"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(delete.status(), reqwest::StatusCode::NOT_FOUND);
    let body: Value = delete.json().await.unwrap();
    assert_eq!(
        body["details"],
        format!("Execution '{root_id}' does not have a variable 'v' in scope global")
    );
}

/// Regression guard (green before the fix): variables passed at process start
/// are held by the process instance row itself, so they are part of its LOCAL
/// scope and stay readable through every scoped read.
#[tokio::test]
async fn process_start_variables_are_part_of_the_local_scope() {
    let (client, base_url, engine) = start_test_server("rest-pi-var-scope-start-vars").await;
    let definition_id = deploy(&client, &base_url, &engine, "pi_scope_start_vars").await;
    let process_instance_id = start_process_instance(
        &client,
        &base_url,
        &definition_id,
        json!([{ "name": "startVar", "type": "string", "value": "sv" }]),
    )
    .await;

    let locals = client
        .get(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}/variables?scope=local"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(locals.status(), reqwest::StatusCode::OK);
    let body: Value = locals.json().await.unwrap();
    assert!(
        body.as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["name"] == "startVar"),
        "start variables must be part of the local scope, got {body}"
    );

    for url in [
        format!("{base_url}/runtime/process-instances/{process_instance_id}/variables/startVar"),
        format!(
            "{base_url}/runtime/process-instances/{process_instance_id}/variables/startVar?scope=local"
        ),
    ] {
        let read = client
            .get(&url)
            .basic_auth("admin", Some("test"))
            .send()
            .await
            .unwrap();
        assert_eq!(read.status(), reqwest::StatusCode::OK, "reading {url}");
        let body: Value = read.json().await.unwrap();
        assert_eq!(body["value"], "sv");
    }
}

/// Same order as the execution endpoints: PI variable handlers share
/// `mutate_execution_variables`, so a suspended process instance + missing
/// update-only PUT is 404 (Java `BaseExecutionVariableResource.setVariable:222-224`),
/// while create-only still reaches the suspended guard
/// (`SetExecutionVariablesCmd.getSuspendedExceptionMessagePrefix` =
/// "Cannot set variables to").
#[tokio::test]
async fn suspended_process_instance_mode_checks_precede_the_suspend_guard() {
    let (client, base_url, engine) = start_test_server("rest-pi-var-scope-suspend-order").await;
    let process_instance_id =
        deploy_and_start(&client, &base_url, &engine, "pi_scope_suspend_order").await;

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

    let post = client
        .post(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}/variables"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!([{ "name": "newOnSuspended", "type": "string", "value": "v" }]))
        .send()
        .await
        .unwrap();
    assert_eq!(post.status(), reqwest::StatusCode::INTERNAL_SERVER_ERROR);
    let body: Value = post.json().await.unwrap();
    // 5xx: raw engine messages are logged server-side only; public details
    // is a fixed string (no suspended-execution id echo).
    assert_eq!(body["details"], "Internal server error");

    let put = client
        .put(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}/variables/missingOnSuspended"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "name": "missingOnSuspended", "type": "string", "value": "v" }))
        .send()
        .await
        .unwrap();
    assert_eq!(put.status(), reqwest::StatusCode::NOT_FOUND);
    let body: Value = put.json().await.unwrap();
    assert_eq!(
        body["details"],
        format!(
            "Execution '{process_instance_id}' does not have a variable with name: 'missingOnSuspended'."
        )
    );

    let delete = client
        .delete(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}/variables/missingOnSuspended"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(delete.status(), reqwest::StatusCode::NOT_FOUND);
}
