//! P123 — CMMN repository/management residuals: PUT case-definition category, management
//! job filters/actions, subtasks.
//!
//! Java references:
//! - `CaseDefinitionResource.java:87-109` — PUT `/cmmn-repository/case-definitions/{id}`
//!   is an action endpoint; null body → 400 "No action found"; `category` → set category
//!   and patch the response without re-fetching; other actions → 400 "Invalid action".
//! - `JobCollectionResource.java:108-189` — GET `/cmmn-management/jobs` accepts
//!   caseInstanceId/planItemInstanceId/scopeDefinitionId/scopeType/elementId/
//!   withoutScopeId/timersOnly/messagesOnly/dueBefore/dueAfter/withException/
//!   exceptionMessage/tenantId/tenantIdLike/withoutTenantId; timersOnly+messagesOnly → 400.
//! - `JobResource.java:216-231` — POST `/cmmn-management/jobs/{jobId}` only accepts
//!   `{"action":"execute"}`, 204 on success, 404 when the job is absent.
//! - `JobResource.java:239-266` — POST `/cmmn-management/timer-jobs/{jobId}` accepts `move`
//!   and `reschedule`; Rust implements only `move` (recorded as a P123 deviation).
//! - `JobResource.java:126-136, 143-153, 198-208` — DELETE on the executable/timer/
//!   deadletter job paths, 204 / 404.
//! - `TaskSubTaskCollectionResource.java:42-46` — GET
//!   `/cmmn-runtime/tasks/{taskId}/subtasks` resolves the task (404 when missing) and
//!   returns `taskService.getSubTasks(taskId)`. P123 verified this list is necessarily
//!   empty for case-produced tasks: `flowable-cmmn-engine` never calls `setParentTaskId`,
//!   and the relation is only creatable through `POST /cmmn-runtime/tasks`
//!   (`TaskCollectionResource.java:357`, `TaskRequest.java:113-118`), which Rust does not
//!   expose. The empty body is therefore the correct contract, not a stub.
//!
//! Fixture note: the Rust CMMN XML converter does not model `timerEventListener` (see
//! `rest_cmmn_planitem_unified_test.rs:10-11`), so the timer-job fixture is built through
//! the programmatic model API exactly as `p117_timer_event_listener_test.rs` does, then
//! exercised over REST.

use flowable_cmmn_engine::{
    CmmnCase, CmmnCaseInstanceStartRequest, CmmnCasePlanModel, CmmnDeploymentRequest, CmmnEngine,
    CmmnEventListener, CmmnHumanTask, CmmnModel, CmmnPlanItem,
};
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_rest::run_server;
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::net::TcpListener;

const CASE_KEY: &str = "p123Case";

/// A case plan with a timer event listener (the job fixture) plus a human task that keeps
/// the case open so the plan does not auto-complete out from under the assertions.
fn p123_model() -> CmmnModel {
    let plan_model = CmmnCasePlanModel::new("case-plan-model", "Case plan model")
        .with_event_listener(
            CmmnEventListener::new("timer-listener", CmmnEventListener::EVENT_TYPE_TIMER)
                .with_name("Timer listener")
                .with_timer_expression("PT2H"),
        )
        .with_plan_item(CmmnPlanItem::new("plan-item-timer", "timer-listener"))
        .with_human_task(CmmnHumanTask::new("task-review", "Review"))
        .with_plan_item(CmmnPlanItem::new("plan-item-review", "task-review"));
    CmmnModel::new(vec![CmmnCase::new(
        "case-p123",
        CASE_KEY,
        "P123 repo mgmt case",
        plan_model,
    )])
}

async fn spawn_server(test_name: &str) -> (Arc<CmmnEngine>, String, reqwest::Client) {
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

    let cmmn_engine = engine
        .get_config()
        .cmmn_engine
        .as_ref()
        .expect("test process engine should have a CMMN engine")
        .clone();

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        run_server(engine, listener).await.unwrap();
    });

    (cmmn_engine, base_url, reqwest::Client::new())
}

/// Deploy the programmatic model and return the resolved case-definition id.
fn deploy(cmmn_engine: &CmmnEngine, deployment_key: &str) -> String {
    cmmn_engine
        .deploy(
            CmmnDeploymentRequest::new(deployment_key)
                .with_resource(format!("{deployment_key}.cmmn"), p123_model()),
        )
        .expect("deployment");
    cmmn_engine
        .repository_service()
        .create_case_definition_query()
        .key(CASE_KEY)
        .single_result()
        .expect("case definition query")
        .expect("case definition")
        .id
}

fn start_case(cmmn_engine: &CmmnEngine) -> String {
    cmmn_engine
        .start_case_instance_by_key(CASE_KEY, CmmnCaseInstanceStartRequest::new())
        .expect("case instance")
        .id
}

async fn get_json(client: &reqwest::Client, url: String) -> Value {
    let response = client
        .get(url)
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    response.json().await.unwrap()
}

// ---------------------------------------------------------------------------
// 1. PUT /cmmn-repository/case-definitions/{id}
// ---------------------------------------------------------------------------

#[tokio::test]
async fn put_case_definition_category_success() {
    // Java CaseDefinitionResource.java:98-105: the `category` action sets the category and
    // patches the response DTO without re-fetching the definition.
    let (cmmn_engine, base_url, client) = spawn_server("p123_put_category").await;
    let definition_id = deploy(&cmmn_engine, "p123-put-category");

    let response = client
        .put(format!(
            "{base_url}/cmmn-repository/case-definitions/{definition_id}"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "category": "finance" }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["category"].as_str(), Some("finance"));
    assert_eq!(body["id"].as_str(), Some(definition_id.as_str()));

    // The category is persisted, not just echoed.
    let stored = cmmn_engine
        .repository_service()
        .get_case_definition(&definition_id)
        .expect("case definition");
    assert_eq!(stored.category.as_deref(), Some("finance"));
}

#[tokio::test]
async fn put_case_definition_unknown_id_404() {
    // Java BaseCaseDefinitionResource: a missing definition raises 404 before the action
    // is interpreted.
    let (_cmmn_engine, base_url, client) = spawn_server("p123_put_category_404").await;

    let response = client
        .put(format!(
            "{base_url}/cmmn-repository/case-definitions/does-not-exist"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "category": "finance" }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn put_case_definition_empty_body_400() {
    // Java CaseDefinitionResource.java:92-94: only a *null* body yields "No action found in
    // request body." An absent body deserializes to null in Spring, so this is the
    // empty-payload case.
    let (cmmn_engine, base_url, client) = spawn_server("p123_put_empty_body").await;
    let definition_id = deploy(&cmmn_engine, "p123-put-empty");

    let response = client
        .put(format!(
            "{base_url}/cmmn-repository/case-definitions/{definition_id}"
        ))
        .basic_auth("admin", Some("test"))
        .header("content-type", "application/json")
        .body("")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
    assert!(response.text().await.unwrap().contains("No action found"));
}

#[tokio::test]
async fn put_case_definition_no_category_400() {
    // Java CaseDefinitionResource.java:98-108: `{}` deserializes to a non-null request with
    // a null category, so it falls through the category branch to
    // "Invalid action: 'null'." — it does *not* hit the null-body check on line 92.
    let (cmmn_engine, base_url, client) = spawn_server("p123_put_no_category").await;
    let definition_id = deploy(&cmmn_engine, "p123-put-no-category");

    let response = client
        .put(format!(
            "{base_url}/cmmn-repository/case-definitions/{definition_id}"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
    let body = response.text().await.unwrap();
    assert!(body.contains("Invalid action"), "unexpected body: {body}");
    assert!(body.contains("null"), "unexpected body: {body}");
}

#[tokio::test]
async fn put_case_definition_invalid_action_400() {
    // Java CaseDefinitionResource.java:106-108: an unrecognised action → 400
    // "Invalid action: '...'.".
    let (cmmn_engine, base_url, client) = spawn_server("p123_put_invalid_action").await;
    let definition_id = deploy(&cmmn_engine, "p123-put-invalid");

    let response = client
        .put(format!(
            "{base_url}/cmmn-repository/case-definitions/{definition_id}"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "action": "bogus" }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
    assert!(response.text().await.unwrap().contains("Invalid action"));
}

// ---------------------------------------------------------------------------
// 2. CMMN management job query + actions
// ---------------------------------------------------------------------------

#[tokio::test]
async fn management_timer_job_query_filters() {
    // Java JobCollectionResource.java:112-182: the collection filters reach the engine
    // query instead of being parsed and dropped.
    let (cmmn_engine, base_url, client) = spawn_server("p123_job_query").await;
    let definition_id = deploy(&cmmn_engine, "p123-job-query");
    let case_id = start_case(&cmmn_engine);

    // Baseline: the timer event listener scheduled exactly one timer job.
    let all = get_json(&client, format!("{base_url}/cmmn-management/timer-jobs")).await;
    assert_eq!(all["total"].as_u64(), Some(1));
    let job = &all["data"][0];
    let job_id = job["id"].as_str().unwrap().to_string();
    let element_id = job["elementId"].as_str().unwrap().to_string();
    assert_eq!(job["scopeType"].as_str(), Some("cmmn"));

    // Matching filters return the job.
    for query in [
        format!("caseInstanceId={case_id}"),
        format!("scopeDefinitionId={definition_id}"),
        format!("caseDefinitionId={definition_id}"),
        format!("elementId={element_id}"),
        format!("id={job_id}"),
        "scopeType=cmmn".to_string(),
        "timersOnly=true".to_string(),
        "withoutTenantId=true".to_string(),
    ] {
        let body = get_json(
            &client,
            format!("{base_url}/cmmn-management/timer-jobs?{query}"),
        )
        .await;
        assert_eq!(body["total"].as_u64(), Some(1), "filter `{query}` should match");
    }

    // Non-matching filters exclude it.
    for query in [
        "caseInstanceId=other-case".to_string(),
        "elementId=other-element".to_string(),
        "scopeType=bpmn".to_string(),
        "messagesOnly=true".to_string(),
        "withException=true".to_string(),
        "tenantId=acme".to_string(),
        "withoutScopeId=true".to_string(),
    ] {
        let body = get_json(
            &client,
            format!("{base_url}/cmmn-management/timer-jobs?{query}"),
        )
        .await;
        assert_eq!(body["total"].as_u64(), Some(0), "filter `{query}` should not match");
    }

    // dueBefore/dueAfter bracket the due date (the timer is PT2H out).
    let far_future = "2999-01-01T00:00:00.000Z";
    let long_past = "2000-01-01T00:00:00.000Z";
    let body = get_json(
        &client,
        format!("{base_url}/cmmn-management/timer-jobs?dueBefore={far_future}"),
    )
    .await;
    assert_eq!(body["total"].as_u64(), Some(1));
    let body = get_json(
        &client,
        format!("{base_url}/cmmn-management/timer-jobs?dueBefore={long_past}"),
    )
    .await;
    assert_eq!(body["total"].as_u64(), Some(0));
    let body = get_json(
        &client,
        format!("{base_url}/cmmn-management/timer-jobs?dueAfter={long_past}"),
    )
    .await;
    assert_eq!(body["total"].as_u64(), Some(1));
}

#[tokio::test]
async fn management_job_query_timers_and_messages_400() {
    // Java JobCollectionResource.java:139-146: timersOnly + messagesOnly is rejected.
    let (_cmmn_engine, base_url, client) = spawn_server("p123_job_query_conflict").await;

    let response = client
        .get(format!(
            "{base_url}/cmmn-management/jobs?timersOnly=true&messagesOnly=true"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
    assert!(response.text().await.unwrap().contains("Only one of"));
}

#[tokio::test]
async fn post_timer_job_move_then_execute_job() {
    // Java JobResource.java:248-254 (`move` → moveTimerToExecutableJob) and
    // JobResource.java:216-231 (`execute` → executeJob, 204).
    let (cmmn_engine, base_url, client) = spawn_server("p123_move_execute").await;
    deploy(&cmmn_engine, "p123-move-execute");
    start_case(&cmmn_engine);

    let timer_jobs = get_json(&client, format!("{base_url}/cmmn-management/timer-jobs")).await;
    let job_id = timer_jobs["data"][0]["id"].as_str().unwrap().to_string();

    // No executable job yet — the timer is still pending.
    let executable = get_json(&client, format!("{base_url}/cmmn-management/jobs")).await;
    assert_eq!(executable["total"].as_u64(), Some(0));

    // `move` transfers the timer row into the executable family.
    let response = client
        .post(format!("{base_url}/cmmn-management/timer-jobs/{job_id}"))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "action": "move" }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::NO_CONTENT);

    let executable = get_json(&client, format!("{base_url}/cmmn-management/jobs")).await;
    assert_eq!(executable["total"].as_u64(), Some(1));
    assert_eq!(executable["data"][0]["id"].as_str(), Some(job_id.as_str()));
    let timer_jobs = get_json(&client, format!("{base_url}/cmmn-management/timer-jobs")).await;
    assert_eq!(timer_jobs["total"].as_u64(), Some(0));

    // `execute` runs the handler and removes the job.
    let response = client
        .post(format!("{base_url}/cmmn-management/jobs/{job_id}"))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "action": "execute" }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::NO_CONTENT);

    let executable = get_json(&client, format!("{base_url}/cmmn-management/jobs")).await;
    assert_eq!(executable["total"].as_u64(), Some(0));
}

#[tokio::test]
async fn post_job_invalid_action_and_unknown_id() {
    // Java JobResource.java:225-231: a non-`execute` action → 400; an unknown job → 404.
    let (cmmn_engine, base_url, client) = spawn_server("p123_job_action_errors").await;
    deploy(&cmmn_engine, "p123-job-action-errors");
    start_case(&cmmn_engine);

    let timer_jobs = get_json(&client, format!("{base_url}/cmmn-management/timer-jobs")).await;
    let timer_job_id = timer_jobs["data"][0]["id"].as_str().unwrap().to_string();

    // Unknown executable job → 404.
    let response = client
        .post(format!("{base_url}/cmmn-management/jobs/does-not-exist"))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "action": "execute" }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::NOT_FOUND);

    // Wrong action on a timer job → 400.
    let response = client
        .post(format!(
            "{base_url}/cmmn-management/timer-jobs/{timer_job_id}"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "action": "bogus" }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
    assert!(response.text().await.unwrap().contains("Invalid action"));

    // `reschedule` without a due date → 400 (JobResource.java:256-262).
    let response = client
        .post(format!(
            "{base_url}/cmmn-management/timer-jobs/{timer_job_id}"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "action": "reschedule" }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn delete_timer_job() {
    // Java JobResource.java:143-153: DELETE on the timer-job path → 204, then 404.
    let (cmmn_engine, base_url, client) = spawn_server("p123_delete_timer_job").await;
    deploy(&cmmn_engine, "p123-delete-timer-job");
    start_case(&cmmn_engine);

    let timer_jobs = get_json(&client, format!("{base_url}/cmmn-management/timer-jobs")).await;
    let job_id = timer_jobs["data"][0]["id"].as_str().unwrap().to_string();

    let response = client
        .delete(format!("{base_url}/cmmn-management/timer-jobs/{job_id}"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::NO_CONTENT);

    let timer_jobs = get_json(&client, format!("{base_url}/cmmn-management/timer-jobs")).await;
    assert_eq!(timer_jobs["total"].as_u64(), Some(0));

    // Deleting again → 404.
    let response = client
        .delete(format!("{base_url}/cmmn-management/timer-jobs/{job_id}"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::NOT_FOUND);
}

// ---------------------------------------------------------------------------
// 3. Subtasks
// ---------------------------------------------------------------------------

#[tokio::test]
async fn get_task_subtasks_is_empty_for_case_tasks() {
    // Java TaskSubTaskCollectionResource.java:42-46 returns `getSubTasks(taskId)`. P123
    // verified `flowable-cmmn-engine` never sets a parent task id, so a case-produced task
    // can have no subtasks — the empty list is the aligned contract, not a stub.
    let (cmmn_engine, base_url, client) = spawn_server("p123_subtasks").await;
    deploy(&cmmn_engine, "p123-subtasks");
    let case_id = start_case(&cmmn_engine);

    let tasks = get_json(
        &client,
        format!("{base_url}/cmmn-runtime/tasks?caseInstanceId={case_id}"),
    )
    .await;
    assert_eq!(tasks["total"].as_u64(), Some(1));
    let task_id = tasks["data"][0]["id"].as_str().unwrap().to_string();

    let subtasks = get_json(
        &client,
        format!("{base_url}/cmmn-runtime/tasks/{task_id}/subtasks"),
    )
    .await;
    assert_eq!(subtasks.as_array().expect("array").len(), 0);
}

#[tokio::test]
async fn get_task_subtasks_unknown_task_404() {
    // Java TaskSubTaskCollectionResource.java:44 resolves the task first: a missing task
    // id raises 404 rather than returning an empty list.
    let (_cmmn_engine, base_url, client) = spawn_server("p123_subtasks_404").await;

    let response = client
        .get(format!(
            "{base_url}/cmmn-runtime/tasks/does-not-exist/subtasks"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::NOT_FOUND);
}
