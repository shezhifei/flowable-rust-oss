//! Contract tests for the `scope` semantics of the execution-variable REST
//! endpoints.
//!
//! Java reference: `BaseExecutionVariableResource` /
//! `BaseVariableCollectionResource` / `ExecutionVariableResource` /
//! `ExecutionVariableCollectionResource`. The scope rules under test:
//!
//! * writes default to the LOCAL scope when no scope is supplied;
//! * a GLOBAL write targets the PARENT execution when one exists;
//! * a GLOBAL write on a root execution is a 400 on the collection endpoint
//!   ("Cannot set global variables on execution '<id>', task is not part of
//!   process.");
//! * `hasVariableOnScope` is scope-strict, so create conflicts (409) and
//!   update misses (404) are decided per scope;
//! * reads without a scope prefer the local value and fall back to the parent
//!   (global) scope; the collection read merges both with local precedence;
//! * an unknown scope string is a 400 ("Invalid variable scope: '<scope>'").

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

/// An embedded subprocess containing a parallel fork. This is the only topology
/// that materializes a real ancestor execution row in the Rust engine: the
/// subprocess scope execution (`parentId == null`, id == process instance id)
/// plus one child execution per parallel branch.
const SUBPROCESS_FORK_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
  <process id="execScopeProcess" name="Execution Variable Scope" isExecutable="true">
    <startEvent id="start" />
    <sequenceFlow id="toSub" sourceRef="start" targetRef="outerSub" />
    <subProcess id="outerSub" name="Outer Subprocess">
      <startEvent id="subStart" />
      <sequenceFlow id="toFork" sourceRef="subStart" targetRef="fork" />
      <parallelGateway id="fork" />
      <sequenceFlow id="toA" sourceRef="fork" targetRef="taskA" />
      <sequenceFlow id="toB" sourceRef="fork" targetRef="taskB" />
      <userTask id="taskA" name="Task A" />
      <userTask id="taskB" name="Task B" />
      <sequenceFlow id="fromA" sourceRef="taskA" targetRef="join" />
      <sequenceFlow id="fromB" sourceRef="taskB" targetRef="join" />
      <parallelGateway id="join" />
      <sequenceFlow id="toSubEnd" sourceRef="join" targetRef="subEnd" />
      <endEvent id="subEnd" />
    </subProcess>
    <sequenceFlow id="toEnd" sourceRef="outerSub" targetRef="end" />
    <endEvent id="end" />
  </process>
</definitions>"#;

const SIMPLE_TASK_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
  <process id="execScopeRootProcess" name="Execution Variable Scope Root" isExecutable="true">
    <startEvent id="start" />
    <sequenceFlow id="flow1" sourceRef="start" targetRef="reviewTask" />
    <userTask id="reviewTask" name="Review" />
    <sequenceFlow id="flow2" sourceRef="reviewTask" targetRef="end" />
    <endEvent id="end" />
  </process>
</definitions>"#;

async fn deploy_and_start(
    client: &reqwest::Client,
    base_url: &str,
    engine: &ProcessEngine,
    xml: &str,
    resource_name: &str,
) -> String {
    let deploy = client
        .post(format!("{base_url}/repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": resource_name,
            "resourceName": format!("{resource_name}.bpmn20.xml"),
            "resource": xml
        }))
        .send()
        .await
        .unwrap();
    assert!(deploy.status().is_success(), "deploy failed");

    let process_definition_id = engine
        .get_repository_service()
        .get_process_definition_ids()
        .unwrap()[0]
        .clone();

    let start = client
        .post(format!("{base_url}/runtime/process-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "processDefinitionId": process_definition_id }))
        .send()
        .await
        .unwrap();
    assert!(start.status().is_success(), "start failed");
    let body: Value = start.json().await.unwrap();
    body["id"].as_str().unwrap().to_string()
}

/// The execution row of a parallel branch: it has a parent, so it has a real
/// global (parent) scope.
fn child_execution_id(engine: &ProcessEngine, activity_id: &str) -> String {
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    store
        .snapshot_executions(&mut session)
        .into_values()
        .find(|execution| {
            execution.activity_id.as_deref() == Some(activity_id)
                && execution.parent_id.is_some()
                && !execution.is_ended
        })
        .map(|execution| execution.id)
        .unwrap_or_else(|| panic!("no child execution at '{activity_id}'"))
}

/// The subprocess scope execution row (`parentId == null`).
fn scope_execution_id(engine: &ProcessEngine) -> String {
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    store
        .snapshot_executions(&mut session)
        .into_values()
        .find(|execution| execution.parent_id.is_none() && !execution.is_ended)
        .map(|execution| execution.id)
        .expect("no scope execution")
}

/// Java `setVariable` with no scope: LOCAL. The variable lands on the child
/// execution itself and is invisible from the parent scope.
#[tokio::test]
async fn create_without_scope_writes_to_the_local_scope() {
    let (client, base_url, engine) = start_test_server("rest-exec-var-scope-default-local").await;
    deploy_and_start(
        &client,
        &base_url,
        &engine,
        SUBPROCESS_FORK_XML,
        "exec_scope_default_local",
    )
    .await;
    let child_id = child_execution_id(&engine, "taskA");
    let scope_id = scope_execution_id(&engine);

    let create = client
        .post(format!(
            "{base_url}/runtime/executions/{child_id}/variables"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!([{ "name": "branchNote", "type": "string", "value": "local-value" }]))
        .send()
        .await
        .unwrap();
    assert_eq!(create.status(), reqwest::StatusCode::CREATED);
    let body: Value = create.json().await.unwrap();
    assert_eq!(body[0]["scope"], "local");

    assert_eq!(
        engine
            .get_runtime_service()
            .get_variable_local(child_id.clone(), "branchNote".to_string())
            .unwrap(),
        Some(json!("local-value")),
        "default scope must be local"
    );
    assert_eq!(
        engine
            .get_runtime_service()
            .has_variable_local(scope_id, "branchNote".to_string())
            .unwrap(),
        false,
        "a local write must not touch the parent scope"
    );
}

/// Java `setVariable` with GLOBAL scope writes to `execution.getParentId()`.
#[tokio::test]
async fn create_with_global_scope_writes_to_the_parent_execution() {
    let (client, base_url, engine) = start_test_server("rest-exec-var-scope-global-parent").await;
    deploy_and_start(
        &client,
        &base_url,
        &engine,
        SUBPROCESS_FORK_XML,
        "exec_scope_global_parent",
    )
    .await;
    let child_id = child_execution_id(&engine, "taskA");
    let scope_id = scope_execution_id(&engine);

    let create = client
        .post(format!(
            "{base_url}/runtime/executions/{child_id}/variables"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!([{
            "name": "sharedNote",
            "type": "string",
            "value": "global-value",
            "scope": "global"
        }]))
        .send()
        .await
        .unwrap();
    assert_eq!(create.status(), reqwest::StatusCode::CREATED);
    let body: Value = create.json().await.unwrap();
    assert_eq!(body[0]["scope"], "global");

    assert_eq!(
        engine
            .get_runtime_service()
            .get_variable(scope_id, "sharedNote".to_string())
            .unwrap(),
        Some(json!("global-value")),
        "a global write must land on the parent execution"
    );
    assert_eq!(
        engine
            .get_runtime_service()
            .has_variable_local(child_id, "sharedNote".to_string())
            .unwrap(),
        false,
        "a global write must not create a local copy on the child"
    );
}

/// Java `BaseVariableCollectionResource.createExecutionVariable`: a GLOBAL
/// write on a root execution has no parent to target and is a 400.
#[tokio::test]
async fn create_with_global_scope_on_a_root_execution_is_rejected() {
    let (client, base_url, engine) = start_test_server("rest-exec-var-scope-global-root").await;
    deploy_and_start(
        &client,
        &base_url,
        &engine,
        SIMPLE_TASK_XML,
        "exec_scope_global_root",
    )
    .await;
    let root_id = scope_execution_id(&engine);

    let create = client
        .post(format!("{base_url}/runtime/executions/{root_id}/variables"))
        .basic_auth("admin", Some("test"))
        .json(&json!([{
            "name": "rootGlobal",
            "type": "string",
            "value": "v",
            "scope": "global"
        }]))
        .send()
        .await
        .unwrap();
    assert_eq!(create.status(), reqwest::StatusCode::BAD_REQUEST);
    let body: Value = create.json().await.unwrap();
    assert_eq!(
        body["details"],
        format!(
            "Cannot set global variables on execution '{root_id}', task is not part of process."
        )
    );
}

/// Java `hasVariableOnScope` is scope-strict: a POST conflict is decided on the
/// requested scope only, so the same name can exist locally and globally.
#[tokio::test]
async fn create_conflict_is_decided_per_scope() {
    let (client, base_url, engine) = start_test_server("rest-exec-var-scope-conflict").await;
    deploy_and_start(
        &client,
        &base_url,
        &engine,
        SUBPROCESS_FORK_XML,
        "exec_scope_conflict",
    )
    .await;
    let child_id = child_execution_id(&engine, "taskA");

    let local_create = client
        .post(format!(
            "{base_url}/runtime/executions/{child_id}/variables"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!([{ "name": "dual", "type": "string", "value": "local" }]))
        .send()
        .await
        .unwrap();
    assert_eq!(local_create.status(), reqwest::StatusCode::CREATED);

    // Same name on the GLOBAL scope: no conflict, the scopes are independent.
    let global_create = client
        .post(format!(
            "{base_url}/runtime/executions/{child_id}/variables"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!([{
            "name": "dual", "type": "string", "value": "global", "scope": "global"
        }]))
        .send()
        .await
        .unwrap();
    assert_eq!(global_create.status(), reqwest::StatusCode::CREATED);

    // Re-creating on the local scope now conflicts.
    let conflict = client
        .post(format!(
            "{base_url}/runtime/executions/{child_id}/variables"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!([{ "name": "dual", "type": "string", "value": "again" }]))
        .send()
        .await
        .unwrap();
    assert_eq!(conflict.status(), reqwest::StatusCode::CONFLICT);
    let body: Value = conflict.json().await.unwrap();
    assert_eq!(
        body["details"],
        format!("Variable 'dual' is already present on execution '{child_id}'.")
    );

    // The local write from the first request is untouched by the conflict.
    assert_eq!(
        engine
            .get_runtime_service()
            .get_variable_local(child_id, "dual".to_string())
            .unwrap(),
        Some(json!("local"))
    );
}

/// Java single-variable PUT: update-only on the resolved scope, 404 when the
/// variable is absent there even if it exists on the other scope.
#[tokio::test]
async fn update_requires_the_variable_on_the_requested_scope() {
    let (client, base_url, engine) = start_test_server("rest-exec-var-scope-update-404").await;
    deploy_and_start(
        &client,
        &base_url,
        &engine,
        SUBPROCESS_FORK_XML,
        "exec_scope_update_404",
    )
    .await;
    let child_id = child_execution_id(&engine, "taskA");

    let create = client
        .post(format!(
            "{base_url}/runtime/executions/{child_id}/variables"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!([{ "name": "onlyLocal", "type": "string", "value": "v1" }]))
        .send()
        .await
        .unwrap();
    assert_eq!(create.status(), reqwest::StatusCode::CREATED);

    // Global scope does not have it.
    let missing = client
        .put(format!(
            "{base_url}/runtime/executions/{child_id}/variables/onlyLocal"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "onlyLocal", "type": "string", "value": "v2", "scope": "global"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(missing.status(), reqwest::StatusCode::NOT_FOUND);
    let body: Value = missing.json().await.unwrap();
    assert_eq!(
        body["details"],
        format!("Execution '{child_id}' does not have a variable with name: 'onlyLocal'.")
    );

    // Default scope (local) does have it.
    let updated = client
        .put(format!(
            "{base_url}/runtime/executions/{child_id}/variables/onlyLocal"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "name": "onlyLocal", "type": "string", "value": "v2" }))
        .send()
        .await
        .unwrap();
    assert_eq!(updated.status(), reqwest::StatusCode::OK);
    let body: Value = updated.json().await.unwrap();
    assert_eq!(body["scope"], "local");
    assert_eq!(body["value"], "v2");
}

/// Java `getVariableFromRequestWithoutAccessCheck`: no scope means local first,
/// then the parent (global) scope.
#[tokio::test]
async fn read_without_scope_prefers_the_local_value() {
    let (client, base_url, engine) = start_test_server("rest-exec-var-scope-read-order").await;
    deploy_and_start(
        &client,
        &base_url,
        &engine,
        SUBPROCESS_FORK_XML,
        "exec_scope_read_order",
    )
    .await;
    let child_id = child_execution_id(&engine, "taskA");

    for (value, scope) in [("global-value", "global"), ("local-value", "local")] {
        let create = client
            .post(format!(
                "{base_url}/runtime/executions/{child_id}/variables"
            ))
            .basic_auth("admin", Some("test"))
            .json(&json!([{
                "name": "shadowed", "type": "string", "value": value, "scope": scope
            }]))
            .send()
            .await
            .unwrap();
        assert_eq!(create.status(), reqwest::StatusCode::CREATED);
    }

    let implicit = client
        .get(format!(
            "{base_url}/runtime/executions/{child_id}/variables/shadowed"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(implicit.status(), reqwest::StatusCode::OK);
    let body: Value = implicit.json().await.unwrap();
    assert_eq!(body["value"], "local-value");
    assert_eq!(body["scope"], "local");

    let explicit_global = client
        .get(format!(
            "{base_url}/runtime/executions/{child_id}/variables/shadowed?scope=global"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(explicit_global.status(), reqwest::StatusCode::OK);
    let body: Value = explicit_global.json().await.unwrap();
    assert_eq!(body["value"], "global-value");
    assert_eq!(body["scope"], "global");

    let explicit_local = client
        .get(format!(
            "{base_url}/runtime/executions/{child_id}/variables/shadowed?scope=local"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(explicit_local.status(), reqwest::StatusCode::OK);
    let body: Value = explicit_local.json().await.unwrap();
    assert_eq!(body["value"], "local-value");
}

/// Java `processVariables`: no scope merges both with local precedence; an
/// explicit scope returns only that scope.
#[tokio::test]
async fn collection_read_merges_scopes_with_local_precedence() {
    let (client, base_url, engine) = start_test_server("rest-exec-var-scope-collection").await;
    deploy_and_start(
        &client,
        &base_url,
        &engine,
        SUBPROCESS_FORK_XML,
        "exec_scope_collection",
    )
    .await;
    let child_id = child_execution_id(&engine, "taskA");

    let create_global = client
        .post(format!(
            "{base_url}/runtime/executions/{child_id}/variables"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!([
            { "name": "shared", "type": "string", "value": "from-global", "scope": "global" },
            { "name": "globalOnly", "type": "string", "value": "g", "scope": "global" }
        ]))
        .send()
        .await
        .unwrap();
    assert_eq!(create_global.status(), reqwest::StatusCode::CREATED);

    let create_local = client
        .post(format!(
            "{base_url}/runtime/executions/{child_id}/variables"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!([
            { "name": "shared", "type": "string", "value": "from-local" },
            { "name": "localOnly", "type": "string", "value": "l" }
        ]))
        .send()
        .await
        .unwrap();
    assert_eq!(create_local.status(), reqwest::StatusCode::CREATED);

    let merged = client
        .get(format!(
            "{base_url}/runtime/executions/{child_id}/variables"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(merged.status(), reqwest::StatusCode::OK);
    let body: Value = merged.json().await.unwrap();
    let entries = body.as_array().unwrap();
    let find = |name: &str| {
        entries
            .iter()
            .find(|entry| entry["name"] == name)
            .unwrap_or_else(|| panic!("missing variable '{name}'"))
            .clone()
    };
    assert_eq!(find("shared")["value"], "from-local");
    assert_eq!(find("shared")["scope"], "local");
    assert_eq!(find("localOnly")["scope"], "local");
    assert_eq!(find("globalOnly")["scope"], "global");
    assert_eq!(
        entries
            .iter()
            .filter(|entry| entry["name"] == "shared")
            .count(),
        1,
        "the merged list must not contain duplicates"
    );

    let locals = client
        .get(format!(
            "{base_url}/runtime/executions/{child_id}/variables?scope=local"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(locals.status(), reqwest::StatusCode::OK);
    let body: Value = locals.json().await.unwrap();
    let names = body
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["name"].as_str().unwrap().to_string())
        .collect::<Vec<_>>();
    assert!(names.contains(&"localOnly".to_string()));
    assert!(
        !names.contains(&"globalOnly".to_string()),
        "scope=local must not return parent variables"
    );
}

/// Java DELETE: defaults to the LOCAL scope and 404s when the variable is not
/// present there; the ancestor copy survives.
#[tokio::test]
async fn delete_defaults_to_the_local_scope() {
    let (client, base_url, engine) = start_test_server("rest-exec-var-scope-delete").await;
    deploy_and_start(
        &client,
        &base_url,
        &engine,
        SUBPROCESS_FORK_XML,
        "exec_scope_delete",
    )
    .await;
    let child_id = child_execution_id(&engine, "taskA");
    let scope_id = scope_execution_id(&engine);

    for (value, scope) in [("global-value", "global"), ("local-value", "local")] {
        let create = client
            .post(format!(
                "{base_url}/runtime/executions/{child_id}/variables"
            ))
            .basic_auth("admin", Some("test"))
            .json(&json!([{
                "name": "removeMe", "type": "string", "value": value, "scope": scope
            }]))
            .send()
            .await
            .unwrap();
        assert_eq!(create.status(), reqwest::StatusCode::CREATED);
    }

    let delete = client
        .delete(format!(
            "{base_url}/runtime/executions/{child_id}/variables/removeMe"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(delete.status(), reqwest::StatusCode::NO_CONTENT);

    assert_eq!(
        engine
            .get_runtime_service()
            .has_variable_local(child_id.clone(), "removeMe".to_string())
            .unwrap(),
        false,
        "the local copy must be gone"
    );
    assert_eq!(
        engine
            .get_runtime_service()
            .get_variable(scope_id, "removeMe".to_string())
            .unwrap(),
        Some(json!("global-value")),
        "the ancestor copy must survive a local delete"
    );

    // Second delete: nothing left on the local scope.
    let missing = client
        .delete(format!(
            "{base_url}/runtime/executions/{child_id}/variables/removeMe"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(missing.status(), reqwest::StatusCode::NOT_FOUND);
    let body: Value = missing.json().await.unwrap();
    assert_eq!(
        body["details"],
        format!("Execution '{child_id}' does not have a variable 'removeMe' in scope local")
    );
}

/// Java `RestVariable.getScopeFromString`: an unknown scope is a 400.
#[tokio::test]
async fn invalid_scope_is_a_bad_request() {
    let (client, base_url, engine) = start_test_server("rest-exec-var-scope-invalid").await;
    deploy_and_start(
        &client,
        &base_url,
        &engine,
        SIMPLE_TASK_XML,
        "exec_scope_invalid",
    )
    .await;
    let root_id = scope_execution_id(&engine);

    let body_scope = client
        .post(format!("{base_url}/runtime/executions/{root_id}/variables"))
        .basic_auth("admin", Some("test"))
        .json(&json!([{
            "name": "v", "type": "string", "value": "x", "scope": "sideways"
        }]))
        .send()
        .await
        .unwrap();
    assert_eq!(body_scope.status(), reqwest::StatusCode::BAD_REQUEST);
    let body: Value = body_scope.json().await.unwrap();
    assert_eq!(body["details"], "Invalid variable scope: 'sideways'");

    let query_scope = client
        .get(format!(
            "{base_url}/runtime/executions/{root_id}/variables?scope=sideways"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(query_scope.status(), reqwest::StatusCode::BAD_REQUEST);
}

/// Regression guard for the row-level meaning of the LOCAL scope. In Java an
/// `ExecutionEntityImpl` *is* a `VariableScope`, so a process variable held by
/// the root execution is readable from that execution's own LOCAL scope. Rust
/// splits one row into `variables` + `local_variables`, so the LOCAL scope has
/// to be their union or a plain process variable becomes a 404.
#[tokio::test]
async fn process_variables_are_part_of_the_owning_executions_local_scope() {
    let (client, base_url, engine) = start_test_server("rest-exec-var-scope-row-union").await;
    let process_instance_id = deploy_and_start(
        &client,
        &base_url,
        &engine,
        SIMPLE_TASK_XML,
        "exec_scope_row_union",
    )
    .await;
    let root_id = scope_execution_id(&engine);

    // Written through the process-instance endpoint, i.e. into the root row's
    // process-variable map rather than its local map.
    let create = client
        .post(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}/variables"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!([{ "name": "processScoped", "type": "string", "value": "pv" }]))
        .send()
        .await
        .unwrap();
    assert_eq!(create.status(), reqwest::StatusCode::CREATED);

    for url in [
        format!("{base_url}/runtime/executions/{root_id}/variables/processScoped"),
        format!("{base_url}/runtime/executions/{root_id}/variables/processScoped?scope=local"),
    ] {
        let read = client
            .get(&url)
            .basic_auth("admin", Some("test"))
            .send()
            .await
            .unwrap();
        assert_eq!(read.status(), reqwest::StatusCode::OK, "reading {url}");
        let body: Value = read.json().await.unwrap();
        assert_eq!(body["value"], "pv");
        assert_eq!(body["scope"], "local");
    }

    // A LOCAL update must rewrite that same copy instead of shadowing it with a
    // second one, so the process-instance endpoint sees the new value.
    let update = client
        .put(format!(
            "{base_url}/runtime/executions/{root_id}/variables/processScoped"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "name": "processScoped", "type": "string", "value": "pv2" }))
        .send()
        .await
        .unwrap();
    assert_eq!(update.status(), reqwest::StatusCode::OK);
    let read_back = client
        .get(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}/variables/processScoped"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(read_back.status(), reqwest::StatusCode::OK);
    let body: Value = read_back.json().await.unwrap();
    assert_eq!(body["value"], "pv2");
}

/// Java `createOrUpdateExecutionVariable` (collection PUT) is the override
/// variant of POST, and the collection DELETE clears only the local scope.
#[tokio::test]
async fn collection_put_upserts_and_collection_delete_clears_only_locals() {
    let (client, base_url, engine) = start_test_server("rest-exec-var-scope-collection-rw").await;
    deploy_and_start(
        &client,
        &base_url,
        &engine,
        SUBPROCESS_FORK_XML,
        "exec_scope_collection_rw",
    )
    .await;
    let child_id = child_execution_id(&engine, "taskA");
    let scope_id = scope_execution_id(&engine);

    let create = client
        .post(format!(
            "{base_url}/runtime/executions/{child_id}/variables"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!([{ "name": "kept", "type": "string", "value": "v1" }]))
        .send()
        .await
        .unwrap();
    assert_eq!(create.status(), reqwest::StatusCode::CREATED);

    // PUT overrides the existing name instead of conflicting, and creates the
    // new one in the same batch.
    let upsert = client
        .put(format!(
            "{base_url}/runtime/executions/{child_id}/variables"
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
    assert_eq!(
        engine
            .get_runtime_service()
            .get_variable_local(child_id.clone(), "kept".to_string())
            .unwrap(),
        Some(json!("v2"))
    );

    let parent_scoped = client
        .post(format!(
            "{base_url}/runtime/executions/{child_id}/variables"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!([{
            "name": "survives", "type": "string", "value": "p", "scope": "global"
        }]))
        .send()
        .await
        .unwrap();
    assert_eq!(parent_scoped.status(), reqwest::StatusCode::CREATED);

    let delete_all = client
        .delete(format!(
            "{base_url}/runtime/executions/{child_id}/variables"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(delete_all.status(), reqwest::StatusCode::NO_CONTENT);

    let locals = client
        .get(format!(
            "{base_url}/runtime/executions/{child_id}/variables?scope=local"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    let body: Value = locals.json().await.unwrap();
    assert!(
        body.as_array().unwrap().is_empty(),
        "every local variable must be gone, got {body}"
    );
    assert_eq!(
        engine
            .get_runtime_service()
            .get_variable(scope_id, "survives".to_string())
            .unwrap(),
        Some(json!("p")),
        "the parent scope must survive the local clear"
    );
}

/// Java: all variables in one POST must resolve to the same scope.
#[tokio::test]
async fn mixed_scopes_in_one_request_are_rejected() {
    let (client, base_url, engine) = start_test_server("rest-exec-var-scope-mixed").await;
    deploy_and_start(
        &client,
        &base_url,
        &engine,
        SUBPROCESS_FORK_XML,
        "exec_scope_mixed",
    )
    .await;
    let child_id = child_execution_id(&engine, "taskA");

    let mixed = client
        .post(format!(
            "{base_url}/runtime/executions/{child_id}/variables"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!([
            { "name": "a", "type": "string", "value": "1", "scope": "local" },
            { "name": "b", "type": "string", "value": "2", "scope": "global" }
        ]))
        .send()
        .await
        .unwrap();
    assert_eq!(mixed.status(), reqwest::StatusCode::BAD_REQUEST);
    let body: Value = mixed.json().await.unwrap();
    assert_eq!(
        body["details"],
        "Only allowed to update multiple variables in the same scope."
    );

    // Nothing was written: the batch is validated before any write.
    assert_eq!(
        engine
            .get_runtime_service()
            .has_variable_local(child_id, "a".to_string())
            .unwrap(),
        false
    );
}

/// Java `BaseExecutionVariableResource.setVariable:217-224` runs the
/// update-only `hasVariableOnScope` check before the engine cmd, so a missing
/// name on a suspended execution is 404 — the suspended guard
/// (`SetExecutionVariablesCmd` / `NeedsActiveExecutionCmd`, prefix
/// "Cannot set variables to") is never reached. A create-only POST that
/// passes the mode check still hits the guard and is 500 with that message.
///
/// Regression: create-only on a suspended execution remains 500 (green before
/// the order fix). The 404 assertions for update-only / delete are the red
/// cases this work item closes.
#[tokio::test]
async fn suspended_execution_mode_checks_precede_the_suspend_guard() {
    let (client, base_url, engine) = start_test_server("rest-exec-var-scope-suspend-order").await;
    let process_instance_id = deploy_and_start(
        &client,
        &base_url,
        &engine,
        SIMPLE_TASK_XML,
        "exec_scope_suspend_order",
    )
    .await;
    let root_id = scope_execution_id(&engine);

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

    // Create-only: mode check passes → suspended guard → 500.
    let post = client
        .post(format!("{base_url}/runtime/executions/{root_id}/variables"))
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

    // Update-only, missing name: Java setVariable:222-224 → 404.
    let put = client
        .put(format!(
            "{base_url}/runtime/executions/{root_id}/variables/missingOnSuspended"
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
        format!("Execution '{root_id}' does not have a variable with name: 'missingOnSuspended'.")
    );

    // Delete missing: Java ExecutionVariableResource.deleteVariable:197-199 → 404.
    let delete = client
        .delete(format!(
            "{base_url}/runtime/executions/{root_id}/variables/missingOnSuspended"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(delete.status(), reqwest::StatusCode::NOT_FOUND);
}
