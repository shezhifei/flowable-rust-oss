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

async fn deploy_and_start_user_task_process(
    client: &reqwest::Client,
    base_url: &str,
    engine: &ProcessEngine,
) -> (String, String) {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="asyncVariablesProcess" name="Async Variables Process" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="flow1" sourceRef="start" targetRef="reviewTask" />
            <userTask id="reviewTask" name="Review Async Variables" />
            <sequenceFlow id="flow2" sourceRef="reviewTask" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#;

    let deploy_response = client
        .post(format!("{base_url}/repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "Async Variables Deployment",
            "resourceName": "async_variables.bpmn20.xml",
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
            "businessKey": "async-variables"
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

async fn assert_variable_value(
    client: &reqwest::Client,
    base_url: &str,
    path: &str,
    expected: Value,
) {
    let response = client
        .get(format!("{base_url}{path}"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success());
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["value"], expected);
}

/// A pending async write is not visible on any read endpoint until the
/// `set-async-variables` job has run (Java: the value sits in a
/// `bpmn-async-variables` entry, not on the execution).
async fn assert_variable_absent(client: &reqwest::Client, base_url: &str, path: &str) {
    let response = client
        .get(format!("{base_url}{path}"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        reqwest::StatusCode::NOT_FOUND,
        "the variable must not be visible before the async job runs"
    );
}

fn pending_async_variable_job_ids(engine: &ProcessEngine) -> Vec<String> {
    engine
        .get_management_service()
        .list_executable_jobs()
        .into_iter()
        .filter(|job| job.handler_type.as_deref() == Some(job_handler_types::SET_ASYNC_VARIABLES))
        .map(|job| job.timer_job_id)
        .collect()
}

/// Java parity: a `variables-async` write schedules exactly one
/// `set-async-variables` job; the variable becomes visible only once the async
/// executor has run it (`SetAsyncVariablesJobHandler`).
fn drive_scheduled_async_variables_job(engine: &ProcessEngine) {
    let jobs = pending_async_variable_job_ids(engine);
    assert_eq!(
        jobs.len(),
        1,
        "the async write must schedule exactly one set-async-variables job"
    );
    engine
        .get_management_service()
        .execute_job(&jobs[0])
        .expect("the set-async-variables job should execute successfully");
    assert!(
        pending_async_variable_job_ids(engine).is_empty(),
        "the job must be consumed once it has run"
    );
}

#[tokio::test]
async fn bpmn_runtime_variable_async_endpoints_persist_variables_and_return_no_content() {
    let (client, base_url, engine) = start_test_server("rest-bpmn-variables-async").await;
    let (process_instance_id, execution_id) =
        deploy_and_start_user_task_process(&client, &base_url, &engine).await;

    let process_create_response = client
        .post(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}/variables-async"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!([{
            "name": "processAsyncCreated",
            "type": "json",
            "value": {
                "source": "process-post",
                "version": 1
            }
        }]))
        .send()
        .await
        .unwrap();
    // Java parity: `ProcessInstanceVariableCollectionResource.
    // createExecutionVariableAsync` carries no `@ResponseStatus`, so the
    // unconditional `response.setStatus(201)` in
    // `BaseVariableCollectionResource.createExecutionVariable` stands.
    assert_eq!(
        process_create_response.status(),
        reqwest::StatusCode::CREATED
    );

    // Java: the 201 only means the set-async-variables job was scheduled.
    assert_variable_absent(
        &client,
        &base_url,
        &format!("/runtime/process-instances/{process_instance_id}/variables/processAsyncCreated"),
    )
    .await;
    drive_scheduled_async_variables_job(&engine);
    assert_variable_value(
        &client,
        &base_url,
        &format!("/runtime/process-instances/{process_instance_id}/variables/processAsyncCreated"),
        json!({
            "source": "process-post",
            "version": 1
        }),
    )
    .await;

    let process_update_response = client
        .put(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}/variables-async/processAsyncCreated"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "processAsyncCreated",
            "type": "json",
            "value": {
                "source": "process-put",
                "version": 2
            }
        }))
        .send()
        .await
        .unwrap();
    // Java parity: `ProcessInstanceVariableResource.updateVariableAsync`
    // carries `@ResponseStatus(NO_CONTENT)`, and `setSimpleVariable` never
    // touches the status — a single-variable async PUT stays 204.
    assert_eq!(
        process_update_response.status(),
        reqwest::StatusCode::NO_CONTENT
    );

    // The pending update does not change the visible value until the job runs.
    assert_variable_value(
        &client,
        &base_url,
        &format!("/runtime/process-instances/{process_instance_id}/variables/processAsyncCreated"),
        json!({
            "source": "process-post",
            "version": 1
        }),
    )
    .await;
    drive_scheduled_async_variables_job(&engine);
    assert_variable_value(
        &client,
        &base_url,
        &format!("/runtime/process-instances/{process_instance_id}/variables/processAsyncCreated"),
        json!({
            "source": "process-put",
            "version": 2
        }),
    )
    .await;

    let process_upsert_response = client
        .put(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}/variables-async"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!([{
            "name": "processAsyncCreated",
            "type": "json",
            "value": {
                "source": "process-collection-put",
                "version": 3
            }
        }]))
        .send()
        .await
        .unwrap();
    // Java parity: `ProcessInstanceVariableCollectionResource.
    // createOrUpdateExecutionVariableAsync` also has no `@ResponseStatus`, so
    // the base class `setStatus(201)` applies to the upsert variant too.
    assert_eq!(
        process_upsert_response.status(),
        reqwest::StatusCode::CREATED
    );
    drive_scheduled_async_variables_job(&engine);
    assert_variable_value(
        &client,
        &base_url,
        &format!("/runtime/process-instances/{process_instance_id}/variables/processAsyncCreated"),
        json!({
            "source": "process-collection-put",
            "version": 3
        }),
    )
    .await;

    let execution_create_or_update_response = client
        .put(format!(
            "{base_url}/runtime/executions/{execution_id}/variables-async"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!([{
            "name": "executionAsyncCreated",
            "type": "integer",
            "value": 10
        }]))
        .send()
        .await
        .unwrap();
    // Java parity: `ExecutionVariableCollectionResource.
    // createOrUpdateExecutionVariableAsync` carries
    // `@ResponseStatus(NO_CONTENT)`, which Spring applies after the handler
    // ran, overriding the base class `setStatus(201)` — 204.
    assert_eq!(
        execution_create_or_update_response.status(),
        reqwest::StatusCode::NO_CONTENT
    );

    assert_variable_absent(
        &client,
        &base_url,
        &format!("/runtime/executions/{execution_id}/variables/executionAsyncCreated"),
    )
    .await;
    drive_scheduled_async_variables_job(&engine);
    assert_variable_value(
        &client,
        &base_url,
        &format!("/runtime/executions/{execution_id}/variables/executionAsyncCreated"),
        json!(10),
    )
    .await;

    let execution_update_response = client
        .put(format!(
            "{base_url}/runtime/executions/{execution_id}/variables-async/executionAsyncCreated"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "executionAsyncCreated",
            "type": "integer",
            "value": 11
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        execution_update_response.status(),
        reqwest::StatusCode::NO_CONTENT
    );

    // The pending update does not change the visible value until the job runs.
    assert_variable_value(
        &client,
        &base_url,
        &format!("/runtime/executions/{execution_id}/variables/executionAsyncCreated"),
        json!(10),
    )
    .await;
    drive_scheduled_async_variables_job(&engine);
    assert_variable_value(
        &client,
        &base_url,
        &format!("/runtime/executions/{execution_id}/variables/executionAsyncCreated"),
        json!(11),
    )
    .await;
}

#[tokio::test]
async fn bpmn_runtime_variable_async_endpoints_return_404_for_unknown_targets() {
    let (client, base_url, _engine) = start_test_server("rest-bpmn-variables-async-404").await;

    // Java parity: SetAsyncExecutionVariablesCmd extends NeedsActiveExecutionCmd,
    // which raises FlowableObjectNotFoundException (404) for unknown executions.
    let missing_process_response = client
        .post(format!(
            "{base_url}/runtime/process-instances/missing-instance/variables-async"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!([{ "name": "v", "type": "integer", "value": 1 }]))
        .send()
        .await
        .unwrap();
    assert_eq!(missing_process_response.status(), 404);

    let missing_execution_response = client
        .put(format!(
            "{base_url}/runtime/executions/missing-execution/variables-async"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!([{ "name": "v", "type": "integer", "value": 1 }]))
        .send()
        .await
        .unwrap();
    assert_eq!(missing_execution_response.status(), 404);

    let missing_single_variable_response = client
        .put(format!(
            "{base_url}/runtime/executions/missing-execution/variables-async/v"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "name": "v", "type": "integer", "value": 1 }))
        .send()
        .await
        .unwrap();
    assert_eq!(missing_single_variable_response.status(), 404);
}

#[tokio::test]
async fn bpmn_runtime_variable_async_endpoints_reject_suspended_execution_with_500() {
    let (client, base_url, engine) = start_test_server("rest-bpmn-variables-async-suspended").await;
    let (process_instance_id, execution_id) =
        deploy_and_start_user_task_process(&client, &base_url, &engine).await;

    // Suspend via the public REST action so the cascading to executions is
    // exercised the same way Java does it.
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

    // Java parity: NeedsActiveExecutionCmd raises FlowableException (500) for
    // suspended executions on the async variable write endpoints.
    let process_async_response = client
        .post(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}/variables-async"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!([{ "name": "suspendedVar", "type": "integer", "value": 1 }]))
        .send()
        .await
        .unwrap();
    assert_eq!(process_async_response.status(), 500);

    let execution_async_response = client
        .put(format!(
            "{base_url}/runtime/executions/{execution_id}/variables-async"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!([{ "name": "suspendedVar", "type": "integer", "value": 1 }]))
        .send()
        .await
        .unwrap();
    assert_eq!(execution_async_response.status(), 500);

    let execution_single_async_response = client
        .put(format!(
            "{base_url}/runtime/executions/{execution_id}/variables-async/suspendedVar"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "name": "suspendedVar", "type": "integer", "value": 1 }))
        .send()
        .await
        .unwrap();
    // Java parity: `BaseExecutionVariableResource.setVariable` runs the
    // update-only `hasVariableOnScope` check in the request thread, BEFORE the
    // async command's suspended-execution guard is reached — an absent variable
    // is a 404 here even on a suspended execution.
    assert_eq!(execution_single_async_response.status(), 404);

    // No variable may have been written by the rejected requests.
    let variable_response = client
        .get(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}/variables/suspendedVar"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(variable_response.status(), 404);
    assert!(
        pending_async_variable_job_ids(&engine).is_empty(),
        "a rejected async write must not schedule a job"
    );
}

/// Regression guard (green before the async-job switch): the create-only
/// duplicate check of Java `BaseVariableCollectionResource.createExecutionVariable`
/// runs in the request thread, so a conflicting POST `variables-async` is a
/// synchronous 409 and schedules no job.
#[tokio::test]
async fn create_async_conflict_is_rejected_synchronously_without_a_job() {
    let (client, base_url, engine) = start_test_server("rest-bpmn-variables-async-409").await;
    let (process_instance_id, _execution_id) =
        deploy_and_start_user_task_process(&client, &base_url, &engine).await;

    // Seed the variable through the synchronous collection endpoint.
    let seed_response = client
        .post(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}/variables"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!([{ "name": "dupAsync", "type": "integer", "value": 1 }]))
        .send()
        .await
        .unwrap();
    assert_eq!(seed_response.status(), reqwest::StatusCode::CREATED);

    let conflict_response = client
        .post(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}/variables-async"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!([{ "name": "dupAsync", "type": "integer", "value": 2 }]))
        .send()
        .await
        .unwrap();
    assert_eq!(conflict_response.status(), reqwest::StatusCode::CONFLICT);
    assert!(
        pending_async_variable_job_ids(&engine).is_empty(),
        "a create rejected with 409 must not schedule a set-async-variables job"
    );

    // The stored value is untouched.
    assert_variable_value(
        &client,
        &base_url,
        &format!("/runtime/process-instances/{process_instance_id}/variables/dupAsync"),
        json!(1),
    )
    .await;
}
