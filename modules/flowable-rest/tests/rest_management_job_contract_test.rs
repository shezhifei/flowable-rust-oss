//! Java contract parity tests for the management job family REST API
//! (batch P2-JOB-A): bulk deadletter moves, single deadletter move routing,
//! execute error classification, query contract (fields, paging, sort
//! whitelists, sort/order echo), suspended move semantics and the
//! HistoryJobResponse shape.

use chrono::{TimeZone, Utc};
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::engine::time_source::TestTimeSource;
use flowable_engine::persistence::runtime_store::{RuntimeJobType, RuntimeTimerJobState};
use flowable_engine::runtime::execution::Execution;
use flowable_engine::runtime::process_instance::ProcessInstance;
use flowable_rest::run_server;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::net::TcpListener;

const TIMER_TO_FAILING_SERVICE_BPMN: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
    <process id="timerToFailingService" isExecutable="true">
        <startEvent id="startEvent" />
        <sequenceFlow id="flow1" sourceRef="startEvent" targetRef="timerCatch" />
        <intermediateCatchEvent id="timerCatch">
            <timerEventDefinition>
                <timeDuration>PT5M</timeDuration>
            </timerEventDefinition>
        </intermediateCatchEvent>
        <sequenceFlow id="flow2" sourceRef="timerCatch" targetRef="failingTask" />
        <serviceTask id="failingTask" flowable:class="com.example.UnregisteredDelegate" />
        <sequenceFlow id="flow3" sourceRef="failingTask" targetRef="endEvent" />
        <endEvent id="endEvent" />
    </process>
</definitions>"#;

fn build_engine(test_name: &str) -> Arc<ProcessEngine> {
    let now = Utc.with_ymd_and_hms(2026, 4, 21, 12, 0, 0).unwrap();
    let time_source = Arc::new(TestTimeSource::new(now));
    let db_store =
        Arc::new(flowable_engine::persistence::db_store::DbStore::new_in_memory().unwrap());
    let engine = Arc::new(ProcessEngine::build(
        test_name.to_string(),
        Arc::clone(&time_source) as Arc<_>,
        db_store,
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

    engine
}

async fn spawn_server(engine: Arc<ProcessEngine>) -> (String, reqwest::Client) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let base_url = format!("http://{}", addr);

    let engine_clone = Arc::clone(&engine);
    tokio::spawn(async move {
        run_server(engine_clone, listener).await.unwrap();
    });

    (base_url, reqwest::Client::new())
}

fn job(id: &str, state: &str) -> RuntimeTimerJobState {
    RuntimeTimerJobState {
        timer_job_id: id.to_string(),
        process_instance_id: String::new(),
        execution_id: String::new(),
        activity_id: "activity".to_string(),
        job_state: Some(state.to_string()),
        is_boundary: false,
        attached_activity_id: None,
        cancel_activity: false,
        time_duration: None,
        time_date: None,
        time_cycle: None,
        due_time: Some(1_775_000_000_000),
        lock_owner: None,
        lock_time: None,
        lock_expiration_time: None,
        retries: Some(1),
        error_message: None,
        error_details: None,
        category: None,
        ..Default::default()
    }
}

fn insert_job(engine: &ProcessEngine, job: &RuntimeTimerJobState) {
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    store.insert_timer_job_state(job, &mut session);
    session.flush_and_commit().unwrap();
}

fn insert_typed_job(engine: &ProcessEngine, job: &RuntimeTimerJobState, job_type: RuntimeJobType) {
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    store.insert_timer_job_state_with_type(job, Some(&job_type), &mut session);
    session.flush_and_commit().unwrap();
}

fn insert_process_instance(
    engine: &ProcessEngine,
    id: &str,
    suspended: bool,
    tenant: Option<&str>,
) {
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    store.insert_process_instance(
        &ProcessInstance {
            id: id.to_string(),
            name: None,
            process_definition_id: format!("definition-{id}"),
            process_definition_key: "contractKey".to_string(),
            process_definition_name: None,
            process_definition_version: 1,
            business_key: None,
            business_status: None,
            is_suspended: suspended,
            tenant_id: tenant.map(str::to_string),
            start_time: None,
            start_user_id: None,
            callback_id: None,
            callback_type: None,
            reference_id: None,
            reference_type: None,
            is_ended: false,
            super_execution_id: None,
            root_process_instance_id: None,
        },
        &mut session,
    );
    session.flush_and_commit().unwrap();
}

async fn post_action(
    client: &reqwest::Client,
    base_url: &str,
    path: &str,
    body: &Value,
) -> reqwest::Response {
    client
        .post(format!("{base_url}{path}"))
        .basic_auth("admin", Some("test"))
        .json(body)
        .send()
        .await
        .unwrap()
}

async fn get(client: &reqwest::Client, base_url: &str, path: &str) -> reqwest::Response {
    client
        .get(format!("{base_url}{path}"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap()
}

// ---------------------------------------------------------------------------
// Task 1: POST /management/deadletter-jobs bulk endpoint
// ---------------------------------------------------------------------------

#[tokio::test]
async fn bulk_deadletter_move_moves_all_existing_jobs_atomically() {
    let engine = build_engine("rest-management-bulk-deadletter-move");
    let mut first = job("bulk-dead-1", "deadletter");
    first.retries = Some(0);
    let mut second = job("bulk-dead-2", "deadletter");
    second.retries = Some(0);
    insert_job(&engine, &first);
    insert_job(&engine, &second);

    let (base_url, client) = spawn_server(Arc::clone(&engine)).await;

    let moved = post_action(
        &client,
        &base_url,
        "/management/deadletter-jobs",
        &json!({ "action": "move", "jobIds": ["bulk-dead-1", "bulk-dead-2"] }),
    )
    .await;
    assert_eq!(moved.status(), reqwest::StatusCode::NO_CONTENT);

    let management_service = engine.get_management_service();
    for id in ["bulk-dead-1", "bulk-dead-2"] {
        let revived = management_service
            .find_executable_job_by_id(id)
            .unwrap_or_else(|| panic!("{id} must be executable after the bulk move"));
        // Default retries come from the engine async executor configuration.
        assert_eq!(revived.retries, Some(3));
        assert!(management_service.find_deadletter_job_by_id(id).is_none());
    }
}

#[tokio::test]
async fn bulk_deadletter_move_with_a_missing_id_is_404_and_writes_nothing() {
    let engine = build_engine("rest-management-bulk-deadletter-missing");
    let mut existing = job("bulk-dead-existing", "deadletter");
    existing.retries = Some(0);
    insert_job(&engine, &existing);

    let (base_url, client) = spawn_server(Arc::clone(&engine)).await;

    let missing = post_action(
        &client,
        &base_url,
        "/management/deadletter-jobs",
        &json!({ "action": "move", "jobIds": ["bulk-dead-existing", "bulk-dead-missing"] }),
    )
    .await;
    assert_eq!(missing.status(), reqwest::StatusCode::NOT_FOUND);
    let body: Value = missing.json().await.unwrap();
    assert!(
        body["details"]
            .as_str()
            .unwrap()
            .contains("bulk-dead-missing"),
        "404 response must list the missing ids, got {:?}",
        body["details"]
    );

    // Zero writes: the existing job must still be a deadletter job.
    let management_service = engine.get_management_service();
    assert!(
        management_service
            .find_deadletter_job_by_id("bulk-dead-existing")
            .is_some()
    );
    assert!(
        management_service
            .find_executable_job_by_id("bulk-dead-existing")
            .is_none()
    );
}

#[tokio::test]
async fn bulk_deadletter_move_rejects_unsupported_actions() {
    let engine = build_engine("rest-management-bulk-deadletter-bad-action");
    insert_job(&engine, &job("bulk-dead-action", "deadletter"));
    insert_job(&engine, &job("timer-family-id", "timer"));

    let (base_url, client) = spawn_server(Arc::clone(&engine)).await;

    for body in [
        json!({ "action": "reschedule", "jobIds": ["bulk-dead-action"] }),
        json!({ "jobIds": ["bulk-dead-action"] }),
    ] {
        let rejected = post_action(&client, &base_url, "/management/deadletter-jobs", &body).await;
        assert_eq!(rejected.status(), reqwest::StatusCode::BAD_REQUEST);
        let rejected_body: Value = rejected.json().await.unwrap();
        assert!(
            rejected_body["details"]
                .as_str()
                .unwrap()
                .contains("only 'move' or 'moveToHistoryJob' is supported")
        );
    }

    // A job id from another family counts as missing for the deadletter bulk move.
    let wrong_family = post_action(
        &client,
        &base_url,
        "/management/deadletter-jobs",
        &json!({ "action": "moveToHistoryJob", "jobIds": ["bulk-dead-action", "timer-family-id"] }),
    )
    .await;
    assert_eq!(wrong_family.status(), reqwest::StatusCode::NOT_FOUND);
    let wrong_family_body: Value = wrong_family.json().await.unwrap();
    assert!(
        wrong_family_body["details"]
            .as_str()
            .unwrap()
            .contains("timer-family-id")
    );
    // And nothing was written.
    assert!(
        engine
            .get_management_service()
            .find_deadletter_job_by_id("bulk-dead-action")
            .is_some()
    );
}

#[tokio::test]
async fn bulk_deadletter_move_to_history_jobs_routes_history_origin_jobs() {
    let engine = build_engine("rest-management-bulk-deadletter-to-history");
    let mut history_origin = job("bulk-dead-history-origin", "deadletter");
    history_origin.retries = Some(0);
    insert_typed_job(&engine, &history_origin, RuntimeJobType::History);

    let (base_url, client) = spawn_server(Arc::clone(&engine)).await;

    let moved = post_action(
        &client,
        &base_url,
        "/management/deadletter-jobs",
        &json!({ "action": "moveToHistoryJob", "jobIds": ["bulk-dead-history-origin"] }),
    )
    .await;
    assert_eq!(moved.status(), reqwest::StatusCode::NO_CONTENT);
    let management_service = engine.get_management_service();
    let history_job = management_service
        .find_history_job_by_id("bulk-dead-history-origin")
        .expect("history-origin deadletter must land in the history family");
    // Java moveToHistoryJob uses asyncHistoryExecutorNumberOfRetries (default 10).
    assert_eq!(history_job.retries, Some(10));
}

#[tokio::test]
async fn bulk_deadletter_move_routes_mixed_origins_by_persisted_type() {
    let engine = build_engine("rest-management-bulk-deadletter-mixed-origin");
    let mut runtime_origin = job("bulk-dead-runtime", "deadletter");
    runtime_origin.retries = Some(0);
    insert_job(&engine, &runtime_origin);
    let mut history_origin = job("bulk-dead-history", "deadletter");
    history_origin.retries = Some(0);
    insert_typed_job(&engine, &history_origin, RuntimeJobType::History);

    let (base_url, client) = spawn_server(Arc::clone(&engine)).await;

    let moved = post_action(
        &client,
        &base_url,
        "/management/deadletter-jobs",
        &json!({ "action": "move", "jobIds": ["bulk-dead-runtime", "bulk-dead-history"] }),
    )
    .await;
    assert_eq!(moved.status(), reqwest::StatusCode::NO_CONTENT);
    let management_service = engine.get_management_service();
    assert!(
        management_service
            .find_executable_job_by_id("bulk-dead-runtime")
            .is_some()
    );
    assert!(
        management_service
            .find_history_job_by_id("bulk-dead-history")
            .is_some()
    );
}

// ---------------------------------------------------------------------------
// Task 2: single deadletter move parity
// ---------------------------------------------------------------------------

#[tokio::test]
async fn single_deadletter_move_auto_routes_history_origin_and_defaults_retries() {
    let engine = build_engine("rest-management-single-deadletter-origin");
    let mut history_origin = job("single-dead-history-origin", "deadletter");
    history_origin.retries = Some(0);
    insert_typed_job(&engine, &history_origin, RuntimeJobType::History);
    let mut runtime_origin = job("single-dead-runtime-origin", "deadletter");
    runtime_origin.retries = Some(0);
    insert_job(&engine, &runtime_origin);

    let (base_url, client) = spawn_server(Arc::clone(&engine)).await;

    // Java JobResource: a history-origin deadletter job is moved back to the
    // history family instead of failing with a 400.
    let moved_history = post_action(
        &client,
        &base_url,
        "/management/deadletter-jobs/single-dead-history-origin",
        &json!({ "action": "move" }),
    )
    .await;
    assert_eq!(moved_history.status(), reqwest::StatusCode::NO_CONTENT);
    let management_service = engine.get_management_service();
    let history_job = management_service
        .find_history_job_by_id("single-dead-history-origin")
        .expect("history-origin deadletter must auto-route to the history family");
    // Default retries = engine `number_of_retries` configuration value (3).
    assert_eq!(history_job.retries, Some(3));

    let moved_runtime = post_action(
        &client,
        &base_url,
        "/management/deadletter-jobs/single-dead-runtime-origin",
        &json!({ "action": "move" }),
    )
    .await;
    assert_eq!(moved_runtime.status(), reqwest::StatusCode::NO_CONTENT);
    let executable_job = management_service
        .find_executable_job_by_id("single-dead-runtime-origin")
        .expect("runtime-origin deadletter must become executable");
    assert_eq!(executable_job.retries, Some(3));
}

// ---------------------------------------------------------------------------
// Task 3: execute error classification, timersOnly/messagesOnly, stacktrace
// ---------------------------------------------------------------------------

#[tokio::test]
async fn job_execute_failure_is_500_and_invalid_action_is_400() {
    let engine = build_engine("rest-management-execute-error-contract");
    let repository_service = engine.get_repository_service();
    repository_service
        .deploy(
            repository_service
                .create_deployment()
                .name("timer-to-failing-service.bpmn20.xml".to_string())
                .add_string(
                    "timer-to-failing-service.bpmn20.xml".to_string(),
                    TIMER_TO_FAILING_SERVICE_BPMN.to_string(),
                ),
        )
        .unwrap();
    let process_definition_id = repository_service
        .get_process_definition_ids()
        .unwrap()
        .into_iter()
        .last()
        .unwrap();
    engine
        .get_runtime_service()
        .start_process_instance(
            engine
                .get_runtime_service()
                .create_process_instance_builder()
                .process_definition_id(process_definition_id),
        )
        .unwrap();
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let timer_job_id = store
        .snapshot_timer_job_states(&mut session)
        .into_values()
        .next()
        .expect("timer catch must create a timer job")
        .timer_job_id;
    let _ = session.rollback();

    let (base_url, client) = spawn_server(Arc::clone(&engine)).await;

    // The timer trigger continues into a service task with an unregistered
    // delegate: the execution failure propagates as a 500 (Java JobResource).
    let executed = post_action(
        &client,
        &base_url,
        &format!("/management/timer-jobs/{timer_job_id}"),
        &json!({ "action": "execute" }),
    )
    .await;
    assert_eq!(
        executed.status(),
        reqwest::StatusCode::INTERNAL_SERVER_ERROR
    );

    let invalid_action = post_action(
        &client,
        &base_url,
        &format!("/management/jobs/{timer_job_id}"),
        &json!({ "action": "not-a-real-action" }),
    )
    .await;
    assert_eq!(invalid_action.status(), reqwest::StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn timers_only_and_messages_only_copresence_is_400_regardless_of_values() {
    let engine = build_engine("rest-management-timers-messages-copresence");
    let (base_url, client) = spawn_server(Arc::clone(&engine)).await;

    for query in [
        "timersOnly=true&messagesOnly=true",
        "timersOnly=true&messagesOnly=false",
        "timersOnly=false&messagesOnly=false",
        "timersOnly=false&messagesOnly=true",
    ] {
        for family in ["jobs", "timer-jobs", "suspended-jobs", "deadletter-jobs"] {
            let response = get(&client, &base_url, &format!("/management/{family}?{query}")).await;
            assert_eq!(
                response.status(),
                reqwest::StatusCode::BAD_REQUEST,
                "{family}?{query} must be rejected"
            );
        }
    }

    // Each parameter on its own stays legal.
    let timers_only = get(&client, &base_url, "/management/jobs?timersOnly=false").await;
    assert!(timers_only.status().is_success());
    let messages_only = get(&client, &base_url, "/management/jobs?messagesOnly=false").await;
    assert!(messages_only.status().is_success());
}

#[tokio::test]
async fn jobs_stacktrace_does_not_leak_other_job_families() {
    let engine = build_engine("rest-management-jobs-stacktrace-family-filter");
    let mut timer = job("stacktrace-timer", "timer");
    timer.error_message = Some("timer failure".to_string());
    timer.error_details = Some("timer stacktrace".to_string());
    insert_job(&engine, &timer);
    let mut deadletter = job("stacktrace-deadletter", "deadletter");
    deadletter.error_message = Some("deadletter failure".to_string());
    deadletter.error_details = Some("deadletter stacktrace".to_string());
    insert_job(&engine, &deadletter);
    let mut executable = job("stacktrace-executable", "executable");
    executable.error_message = Some("executable failure".to_string());
    executable.error_details = Some("executable stacktrace".to_string());
    insert_job(&engine, &executable);

    let (base_url, client) = spawn_server(Arc::clone(&engine)).await;

    // Java only queries the executable table for /management/jobs.
    for id in ["stacktrace-timer", "stacktrace-deadletter"] {
        let response = get(
            &client,
            &base_url,
            &format!("/management/jobs/{id}/exception-stacktrace"),
        )
        .await;
        assert_eq!(
            response.status(),
            reqwest::StatusCode::NOT_FOUND,
            "{id} belongs to another family and must be 404 on the jobs stacktrace"
        );
    }

    let executable_stacktrace = get(
        &client,
        &base_url,
        "/management/jobs/stacktrace-executable/exception-stacktrace",
    )
    .await;
    assert!(executable_stacktrace.status().is_success());
    assert_eq!(
        executable_stacktrace.text().await.unwrap(),
        "executable stacktrace"
    );
}

// ---------------------------------------------------------------------------
// Task 4: query contract
// ---------------------------------------------------------------------------

#[tokio::test]
async fn management_job_query_supports_the_java_field_set() {
    let engine = build_engine("rest-management-query-field-contract");
    insert_process_instance(&engine, "process-q", false, Some("tenant-a"));
    {
        let store = engine.get_runtime_store();
        let mut session = store.create_session().unwrap();
        store.insert_execution(
            &Execution {
                id: "execution-q".to_string(),
                process_instance_id: Some("process-q".to_string()),
                process_definition_id: Some("definition-process-q".to_string()),
                tenant_id: Some("tenant-a".to_string()),
                activity_id: Some("q-activity".to_string()),
                activity_name: Some("Q Activity".to_string()),
                ..Default::default()
            },
            &mut session,
        );
        session.flush_and_commit().unwrap();
    }

    let mut due_timer = job("q-due-timer", "timer");
    due_timer.process_instance_id = "process-q".to_string();
    due_timer.execution_id = "execution-q".to_string();
    due_timer.activity_id = "q-activity".to_string();
    due_timer.retries = Some(2);
    due_timer.lock_owner = Some("worker-1".to_string());
    due_timer.due_time = Some(1_775_000_000_000); // 2026-03-31: before engine now
    insert_job(&engine, &due_timer);

    let mut future_async = job("q-future-async", "async");
    future_async.process_instance_id = "process-q".to_string();
    future_async.execution_id = "execution-q".to_string();
    future_async.activity_id = "q-activity".to_string();
    future_async.retries = Some(0);
    future_async.due_time = Some(1_777_593_600_000); // 2026-05-01: after engine now
    insert_job(&engine, &future_async);

    // Zero retries but due: proves `executable` only looks at the duedate.
    let mut due_zero_retries = job("q-due-zero-retries", "executable");
    due_zero_retries.retries = Some(0);
    due_zero_retries.due_time = Some(1_775_000_000_000);
    insert_job(&engine, &due_zero_retries);

    let standalone = job("q-standalone", "timer");
    insert_job(&engine, &standalone);

    let (base_url, client) = spawn_server(Arc::clone(&engine)).await;

    async fn ids(client: &reqwest::Client, base_url: &str, path: &str) -> (Value, Vec<String>) {
        let response = get(client, base_url, path).await;
        assert!(
            response.status().is_success(),
            "{path} must succeed, got {}",
            response.status()
        );
        let body: Value = response.json().await.unwrap();
        let ids = body["data"]
            .as_array()
            .unwrap()
            .iter()
            .map(|job| job["id"].as_str().unwrap().to_string())
            .collect::<Vec<_>>();
        (body, ids)
    }

    // elementId / elementName
    let (_, filtered) = ids(&client, &base_url, "/management/jobs?elementId=q-activity").await;
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0], "q-future-async");
    let (_, filtered) = ids(
        &client,
        &base_url,
        "/management/timer-jobs?elementName=Q%20Activity",
    )
    .await;
    assert_eq!(filtered, vec!["q-due-timer".to_string()]);

    // handlerType / handlerTypes
    let (_, filtered) = ids(&client, &base_url, "/management/jobs?handlerType=async").await;
    assert_eq!(filtered, vec!["q-future-async".to_string()]);
    let (_, filtered) = ids(
        &client,
        &base_url,
        "/management/timer-jobs?handlerTypes=timer,async",
    )
    .await;
    assert!(filtered.contains(&"q-due-timer".to_string()));
    assert!(filtered.contains(&"q-standalone".to_string()));

    // withoutProcessInstanceId
    let (_, filtered) = ids(
        &client,
        &base_url,
        "/management/timer-jobs?withoutProcessInstanceId=true",
    )
    .await;
    assert_eq!(filtered, vec!["q-standalone".to_string()]);

    // locked / unlocked
    let (_, filtered) = ids(&client, &base_url, "/management/timer-jobs?locked=true").await;
    assert_eq!(filtered, vec!["q-due-timer".to_string()]);
    let (_, filtered) = ids(&client, &base_url, "/management/timer-jobs?unlocked=true").await;
    assert_eq!(filtered, vec!["q-standalone".to_string()]);

    // withoutScopeId / withoutScopeType: only rows with null/empty scope fields.
    let (_, filtered) = ids(
        &client,
        &base_url,
        "/management/timer-jobs?withoutScopeId=true&withoutScopeType=true",
    )
    .await;
    assert_eq!(filtered.len(), 2);

    // Seed a scoped deadletter job and prove withoutScope* excludes it.
    let mut scoped = job("q-scoped-dl", "deadletter");
    scoped.scope_id = Some("case-9".to_string());
    scoped.scope_type = Some("cmmn".to_string());
    scoped.category = Some("orders".to_string());
    scoped.correlation_id = Some("corr-scoped".to_string());
    insert_job(&engine, &scoped);
    let plain_dl = job("q-plain-dl", "deadletter");
    insert_job(&engine, &plain_dl);

    let (_, filtered) = ids(
        &client,
        &base_url,
        "/management/deadletter-jobs?withoutScopeId=true",
    )
    .await;
    assert_eq!(filtered, vec!["q-plain-dl".to_string()]);
    let (_, filtered) = ids(
        &client,
        &base_url,
        "/management/deadletter-jobs?withoutScopeType=true",
    )
    .await;
    assert_eq!(filtered, vec!["q-plain-dl".to_string()]);
    let (_, filtered) = ids(
        &client,
        &base_url,
        "/management/deadletter-jobs?category=orders",
    )
    .await;
    assert_eq!(filtered, vec!["q-scoped-dl".to_string()]);
    let (_, filtered) = ids(
        &client,
        &base_url,
        "/management/deadletter-jobs?correlationId=corr-scoped",
    )
    .await;
    assert_eq!(filtered, vec!["q-scoped-dl".to_string()]);
    let (_, filtered) = ids(
        &client,
        &base_url,
        "/management/deadletter-jobs?scopeId=case-9&scopeType=cmmn",
    )
    .await;
    assert_eq!(filtered, vec!["q-scoped-dl".to_string()]);

    // executable: duedate <= now only; zero-retries jobs are not excluded.
    let (_, filtered) = ids(&client, &base_url, "/management/jobs?executable=true").await;
    assert_eq!(filtered, vec!["q-due-zero-retries".to_string()]);

    // tenantIdLike uses SQL LIKE semantics: '%' is required as a wildcard.
    let (_, filtered) = ids(
        &client,
        &base_url,
        "/management/timer-jobs?tenantIdLike=tenant-%25",
    )
    .await;
    assert_eq!(filtered, vec!["q-due-timer".to_string()]);
    let (_, filtered) = ids(
        &client,
        &base_url,
        "/management/timer-jobs?tenantIdLike=tenant",
    )
    .await;
    assert!(filtered.is_empty());

    // noRetriesLeft only applies to the suspended family.
    let mut suspended_zero = job("q-suspended-zero", "suspended");
    suspended_zero.retries = Some(0);
    insert_job(&engine, &suspended_zero);
    let mut suspended_left = job("q-suspended-left", "suspended");
    suspended_left.retries = Some(4);
    insert_job(&engine, &suspended_left);
    let (_, filtered) = ids(
        &client,
        &base_url,
        "/management/suspended-jobs?noRetriesLeft=true",
    )
    .await;
    assert_eq!(filtered, vec!["q-suspended-zero".to_string()]);
}

#[tokio::test]
async fn management_job_lists_default_to_page_size_10_and_echo_sort_order() {
    let engine = build_engine("rest-management-paging-sort-contract");
    for index in 0..12 {
        insert_job(
            &engine,
            &job(&format!("paged-job-{index:02}"), "executable"),
        );
    }

    let (base_url, client) = spawn_server(Arc::clone(&engine)).await;

    let default_page = get(&client, &base_url, "/management/jobs").await;
    assert!(default_page.status().is_success());
    let default_body: Value = default_page.json().await.unwrap();
    assert_eq!(default_body["total"], 12);
    assert_eq!(default_body["size"], 10);
    assert_eq!(default_body["data"].as_array().unwrap().len(), 10);
    // Java DataResponse echoes the effective sort/order (defaults id/asc).
    assert_eq!(default_body["sort"], "id");
    assert_eq!(default_body["order"], "asc");

    let second_page = get(&client, &base_url, "/management/jobs?start=10").await;
    let second_body: Value = second_page.json().await.unwrap();
    assert_eq!(second_body["data"].as_array().unwrap().len(), 2);

    let sorted = get(
        &client,
        &base_url,
        "/management/jobs?sort=retries&order=desc&size=2",
    )
    .await;
    let sorted_body: Value = sorted.json().await.unwrap();
    assert_eq!(sorted_body["sort"], "retries");
    assert_eq!(sorted_body["order"], "desc");

    // Invalid sort properties are rejected; createTime is a valid batch B sort.
    let rejected = get(&client, &base_url, "/management/jobs?sort=bogus").await;
    assert_eq!(rejected.status(), reqwest::StatusCode::BAD_REQUEST);
    let by_create = get(
        &client,
        &base_url,
        "/management/jobs?sort=createTime&order=asc&size=2",
    )
    .await;
    assert!(
        by_create.status().is_success(),
        "sort=createTime must be accepted after batch B columns"
    );
    let by_create_body: Value = by_create.json().await.unwrap();
    assert_eq!(by_create_body["sort"], "createTime");
    let bad_order = get(&client, &base_url, "/management/jobs?order=sideways").await;
    assert_eq!(bad_order.status(), reqwest::StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn history_job_query_uses_the_history_sort_whitelist() {
    let engine = build_engine("rest-management-history-sort-contract");
    let mut first = job("history-sort-a", "history");
    first.retries = Some(2);
    insert_job(&engine, &first);
    let mut second = job("history-sort-b", "history");
    second.retries = Some(5);
    insert_job(&engine, &second);

    let (base_url, client) = spawn_server(Arc::clone(&engine)).await;

    // HistoryJobQueryProperties only allows id/retries/tenantId.
    for sort in ["dueDate", "executionId", "processInstanceId", "createTime"] {
        let rejected = get(
            &client,
            &base_url,
            &format!("/management/history-jobs?sort={sort}"),
        )
        .await;
        assert_eq!(
            rejected.status(),
            reqwest::StatusCode::BAD_REQUEST,
            "history sort={sort} must be rejected"
        );
    }

    let sorted = get(
        &client,
        &base_url,
        "/management/history-jobs?sort=retries&order=desc",
    )
    .await;
    assert!(sorted.status().is_success());
    let sorted_body: Value = sorted.json().await.unwrap();
    assert_eq!(sorted_body["sort"], "retries");
    assert_eq!(sorted_body["order"], "desc");
    assert_eq!(sorted_body["data"][0]["id"], "history-sort-b");
}

// ---------------------------------------------------------------------------
// Task 5: suspended move semantics
// ---------------------------------------------------------------------------

#[tokio::test]
async fn suspended_move_preserves_retries_and_rejects_suspended_parents() {
    let engine = build_engine("rest-management-suspended-move-contract");
    insert_process_instance(&engine, "process-active", false, None);
    insert_process_instance(&engine, "process-suspended", true, None);

    let mut preserved = job("suspended-preserved", "suspended");
    preserved.process_instance_id = "process-active".to_string();
    preserved.retries = Some(0);
    preserved.error_message = Some("kept failure".to_string());
    insert_job(&engine, &preserved);

    let mut blocked = job("suspended-blocked", "suspended");
    blocked.process_instance_id = "process-suspended".to_string();
    blocked.retries = Some(2);
    insert_job(&engine, &blocked);

    let (base_url, client) = spawn_server(Arc::clone(&engine)).await;

    // Java moveSuspendedJobToExecutableJob copies the retries unchanged.
    let moved = post_action(
        &client,
        &base_url,
        "/management/suspended-jobs/suspended-preserved",
        &json!({ "action": "move" }),
    )
    .await;
    assert_eq!(moved.status(), reqwest::StatusCode::NO_CONTENT);
    let management_service = engine.get_management_service();
    let activated = management_service
        .find_executable_job_by_id("suspended-preserved")
        .expect("suspended job must be activated");
    assert_eq!(activated.retries, Some(0));
    assert_eq!(activated.error_message.as_deref(), Some("kept failure"));

    // Parent process instance is suspended: activation is refused.
    let refused = post_action(
        &client,
        &base_url,
        "/management/suspended-jobs/suspended-blocked",
        &json!({ "action": "move" }),
    )
    .await;
    assert_eq!(refused.status(), reqwest::StatusCode::BAD_REQUEST);
    assert!(
        management_service
            .find_suspended_job_by_id("suspended-blocked")
            .is_some(),
        "rejected activation must leave the job suspended"
    );
}

// ---------------------------------------------------------------------------
// Task 6: history job response shape
// ---------------------------------------------------------------------------

#[tokio::test]
async fn history_job_response_matches_the_java_history_job_response_shape() {
    let engine = build_engine("rest-management-history-response-shape");
    insert_process_instance(&engine, "process-history", false, Some("tenant-h"));
    {
        let store = engine.get_runtime_store();
        let mut session = store.create_session().unwrap();
        store.insert_execution(
            &Execution {
                id: "execution-history".to_string(),
                process_instance_id: Some("process-history".to_string()),
                tenant_id: Some("tenant-h".to_string()),
                ..Default::default()
            },
            &mut session,
        );
        session.flush_and_commit().unwrap();
    }
    let mut history = job("history-shape", "history");
    history.process_instance_id = "process-history".to_string();
    history.execution_id = "execution-history".to_string();
    history.retries = Some(7);
    history.error_message = Some("history failure".to_string());
    history.lock_owner = Some("history-worker".to_string());
    history.lock_expiration_time = Some(1_775_000_060_000);
    insert_job(&engine, &history);

    let (base_url, client) = spawn_server(Arc::clone(&engine)).await;

    let single = get(&client, &base_url, "/management/history-jobs/history-shape").await;
    assert!(single.status().is_success());
    let body: Value = single.json().await.unwrap();
    assert_eq!(body["id"], "history-shape");
    assert_eq!(body["url"], "/management/history-jobs/history-shape");
    assert_eq!(body["jobHandlerType"], "history");
    assert_eq!(body["retries"], 7);
    assert_eq!(body["exceptionMessage"], "history failure");
    assert_eq!(body["tenantId"], "tenant-h");
    assert_eq!(body["lockOwner"], "history-worker");
    assert_eq!(body["lockExpirationTime"], "2026-03-31T23:34:20+00:00");
    // Seeded history row without config payload: jobHandlerConfiguration /
    // customValues remain null (documented gap when no source on the create path).
    // advancedJobHandlerConfiguration falls back to time_duration when set.
    assert!(body["jobHandlerConfiguration"].is_null());
    assert!(body["customValues"].is_null());
    assert!(body["scopeType"].is_null());
    // createTime is stamped on insert (batch B).
    assert!(
        !body["createTime"].is_null(),
        "createTime must be persisted"
    );
    // The shared ManagementJobResponse fields are gone from the history shape.
    assert!(body.get("jobType").is_none());
    assert!(body.get("processInstanceId").is_none());

    let list = get(&client, &base_url, "/management/history-jobs").await;
    let list_body: Value = list.json().await.unwrap();
    assert_eq!(list_body["data"][0]["jobHandlerType"], "history");
    assert_eq!(list_body["sort"], "id");
    assert_eq!(list_body["order"], "asc");
}

// ---------------------------------------------------------------------------
// Batch B: createTime / correlationId / handlerType / filters / execute
// ---------------------------------------------------------------------------

#[tokio::test]
async fn job_create_time_and_sort_are_persisted() {
    let engine = build_engine("rest-management-create-time-sort");
    let mut earlier = job("create-time-early", "timer");
    earlier.create_time = Some(1_700_000_000_000);
    earlier.correlation_id = Some("corr-early".to_string());
    earlier.handler_type = Some("trigger-timer".to_string());
    insert_job(&engine, &earlier);
    let mut later = job("create-time-late", "timer");
    later.create_time = Some(1_800_000_000_000);
    later.correlation_id = Some("corr-late".to_string());
    later.handler_type = Some("trigger-timer".to_string());
    insert_job(&engine, &later);

    let (base_url, client) = spawn_server(Arc::clone(&engine)).await;

    let single = get(
        &client,
        &base_url,
        "/management/timer-jobs/create-time-early",
    )
    .await;
    assert!(single.status().is_success());
    let body: Value = single.json().await.unwrap();
    assert!(!body["createTime"].is_null());
    assert_eq!(body["correlationId"], "corr-early");
    assert_eq!(body["handlerType"], "trigger-timer");

    let asc = get(
        &client,
        &base_url,
        "/management/timer-jobs?sort=createTime&order=asc",
    )
    .await;
    let asc_body: Value = asc.json().await.unwrap();
    assert_eq!(asc_body["data"][0]["id"], "create-time-early");
    assert_eq!(asc_body["data"][1]["id"], "create-time-late");

    let desc = get(
        &client,
        &base_url,
        "/management/timer-jobs?sort=createTime&order=desc",
    )
    .await;
    let desc_body: Value = desc.json().await.unwrap();
    assert_eq!(desc_body["data"][0]["id"], "create-time-late");
    assert_eq!(desc_body["data"][1]["id"], "create-time-early");
}

#[tokio::test]
async fn job_direct_column_filters_for_tenant_process_def_and_element() {
    let engine = build_engine("rest-management-direct-column-filters");
    let mut matching = job("direct-match", "executable");
    matching.tenant_id = Some("tenant-direct".to_string());
    matching.process_definition_id = Some("pd-direct".to_string());
    matching.activity_id = "el-direct".to_string();
    matching.element_name = Some("Element Direct".to_string());
    insert_job(&engine, &matching);
    let mut other = job("direct-other", "executable");
    other.tenant_id = Some("tenant-other".to_string());
    other.process_definition_id = Some("pd-other".to_string());
    other.activity_id = "el-other".to_string();
    other.element_name = Some("Element Other".to_string());
    insert_job(&engine, &other);

    let (base_url, client) = spawn_server(Arc::clone(&engine)).await;

    let by_tenant = get(
        &client,
        &base_url,
        "/management/jobs?tenantId=tenant-direct",
    )
    .await;
    let tenant_body: Value = by_tenant.json().await.unwrap();
    assert_eq!(tenant_body["data"].as_array().unwrap().len(), 1);
    assert_eq!(tenant_body["data"][0]["id"], "direct-match");

    let by_pd = get(
        &client,
        &base_url,
        "/management/jobs?processDefinitionId=pd-direct",
    )
    .await;
    let pd_body: Value = by_pd.json().await.unwrap();
    assert_eq!(pd_body["data"][0]["id"], "direct-match");

    let by_element = get(
        &client,
        &base_url,
        "/management/jobs?elementId=el-direct&elementName=Element%20Direct",
    )
    .await;
    let el_body: Value = by_element.json().await.unwrap();
    assert_eq!(el_body["data"][0]["id"], "direct-match");

    let miss = get(
        &client,
        &base_url,
        "/management/jobs?tenantId=tenant-missing",
    )
    .await;
    let miss_body: Value = miss.json().await.unwrap();
    assert_eq!(miss_body["data"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn history_job_exposes_advanced_handler_configuration() {
    let engine = build_engine("rest-management-history-handler-cfg");
    let mut history = job("history-cfg", "history");
    history.handler_type = Some("async-history".to_string());
    history.job_handler_configuration = Some(r#"{"source":"cfg"}"#.to_string());
    history.advanced_job_handler_configuration =
        Some(r#"{"operations":[{"type":"start"}]}"#.to_string());
    history.custom_values = Some(r#"{"k":"v"}"#.to_string());
    history.time_duration = Some(r#"{"operations":[]}"#.to_string());
    insert_job(&engine, &history);

    let (base_url, client) = spawn_server(Arc::clone(&engine)).await;
    let response = get(&client, &base_url, "/management/history-jobs/history-cfg").await;
    assert!(response.status().is_success());
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["jobHandlerType"], "async-history");
    assert_eq!(body["jobHandlerConfiguration"], r#"{"source":"cfg"}"#);
    assert_eq!(
        body["advancedJobHandlerConfiguration"],
        r#"{"operations":[{"type":"start"}]}"#
    );
    assert_eq!(body["customValues"], r#"{"k":"v"}"#);
    assert!(!body["createTime"].is_null());
}

#[tokio::test]
async fn execute_async_continuation_job_via_management_jobs() {
    let engine = build_engine("rest-management-execute-async");
    // Executable-family async continuation with missing execution fails at
    // handler time → 500 + retries decrement (FailedJobListener parity).
    let mut failing = job("async-fail-exec", "executable");
    failing.time_duration = Some("__flowable_async_continuation".to_string());
    failing.handler_type = Some("async-continuation".to_string());
    failing.retries = Some(2);
    failing.process_instance_id = "missing-pi".to_string();
    failing.execution_id = "missing-ex".to_string();
    insert_job(&engine, &failing);

    // Sanity: job is visible on the executable family before execute.
    assert!(
        engine
            .get_management_service()
            .find_executable_job_by_id("async-fail-exec")
            .is_some()
    );

    let (base_url, client) = spawn_server(Arc::clone(&engine)).await;
    let response = post_action(
        &client,
        &base_url,
        "/management/jobs/async-fail-exec",
        &json!({ "action": "execute" }),
    )
    .await;
    assert_eq!(
        response.status(),
        reqwest::StatusCode::INTERNAL_SERVER_ERROR,
        "failed execute must map to 500, got {}",
        response.status()
    );

    let after = engine
        .get_management_service()
        .find_job_by_id("async-fail-exec")
        .expect("job should still exist after failed execute");
    assert_eq!(after.retries, Some(1), "retries must decrement on failure");
}

#[tokio::test]
async fn move_to_history_job_uses_async_history_retries_default() {
    let engine = build_engine("rest-management-history-retries-default");
    // Confirm engine default is Java's 10.
    assert_eq!(engine.get_config().async_history.number_of_retries, 10);

    let mut dead = job("hist-dl-retries", "deadletter");
    dead.retries = Some(0);
    insert_typed_job(&engine, &dead, RuntimeJobType::History);

    let (base_url, client) = spawn_server(Arc::clone(&engine)).await;
    let response = post_action(
        &client,
        &base_url,
        "/management/deadletter-jobs/hist-dl-retries",
        &json!({ "action": "moveToHistoryJob" }),
    )
    .await;
    assert_eq!(
        response.status(),
        reqwest::StatusCode::NO_CONTENT,
        "moveToHistoryJob must succeed for history-origin deadletter"
    );
    let moved = engine
        .get_management_service()
        .find_history_job_by_id("hist-dl-retries")
        .expect("moved history job");
    assert_eq!(
        moved.retries,
        Some(10),
        "moveToHistoryJob must use async_history.number_of_retries"
    );
}
