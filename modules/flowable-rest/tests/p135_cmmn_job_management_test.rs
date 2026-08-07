//! P135 CMMN job-management REST contracts.

use chrono::{TimeZone, Utc};
use flowable_cmmn_engine::{
    CmmnCase, CmmnCaseInstanceStartRequest, CmmnCasePlanModel, CmmnDeploymentRequest, CmmnEngine,
    CmmnEventListener, CmmnHumanTask, CmmnJob, CmmnJobFamily, CmmnModel, CmmnPlanItem,
    TYPE_SET_ASYNC_VARIABLES,
};
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::engine::time_source::TestTimeSource;
use flowable_rest::run_server;
use serde_json::json;
use std::sync::Arc;
use tokio::net::TcpListener;

fn build_engine(test_name: &str) -> (Arc<ProcessEngine>, Arc<CmmnEngine>) {
    let now = Utc.with_ymd_and_hms(2026, 8, 5, 12, 0, 0).unwrap();
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
    let cmmn = engine
        .get_config()
        .cmmn_engine
        .as_ref()
        .expect("embedded CMMN engine")
        .clone();
    (engine, cmmn)
}

async fn spawn_server(engine: Arc<ProcessEngine>) -> (String, reqwest::Client) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let engine_clone = Arc::clone(&engine);
    tokio::spawn(async move {
        run_server(engine_clone, listener).await.unwrap();
    });
    (format!("http://{address}"), reqwest::Client::new())
}

fn scheduled_timer(cmmn: &CmmnEngine) -> String {
    let plan_model = CmmnCasePlanModel::new("case-plan-model", "Case plan model")
        .with_event_listener(
            CmmnEventListener::new("timer-listener", CmmnEventListener::EVENT_TYPE_TIMER)
                .with_timer_expression("PT1H"),
        )
        .with_plan_item(CmmnPlanItem::new("plan-item-timer", "timer-listener"))
        .with_human_task(CmmnHumanTask::new("keepalive", "Keep alive"))
        .with_plan_item(CmmnPlanItem::new("plan-item-keepalive", "keepalive"));
    let model = CmmnModel::new(vec![CmmnCase::new(
        "p135-rest-definition",
        "p135RestTimer",
        "P135 REST timer",
        plan_model,
    )]);
    cmmn.deploy(
        CmmnDeploymentRequest::new("p135-rest-deployment").with_resource("p135-rest.cmmn", model),
    )
    .expect("deployment");
    let case_id = cmmn
        .start_case_instance_by_key("p135RestTimer", CmmnCaseInstanceStartRequest::new())
        .expect("case instance")
        .id;
    cmmn.management_service()
        .create_job_query()
        .family(CmmnJobFamily::Timer)
        .scope_id(case_id)
        .single_result()
        .expect("timer query")
        .expect("timer")
        .id
}

#[tokio::test]
async fn timer_reschedule_rebuilds_job_and_maps_success_not_found_and_bad_actions() {
    let (engine, cmmn) = build_engine("p135-cmmn-rest-reschedule");
    let old_id = scheduled_timer(cmmn.as_ref());
    let (base_url, client) = spawn_server(engine).await;
    let timer_url = |id: &str| format!("{base_url}/cmmn-management/timer-jobs/{id}");

    // Java JobResource.java:239-266 returns 204 for a valid reschedule action.
    let response = client
        .post(timer_url(&old_id))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "action": "reschedule", "dueDate": "PT2H" }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::NO_CONTENT);

    let new_job = cmmn
        .management_service()
        .create_job_query()
        .family(CmmnJobFamily::Timer)
        .single_result()
        .expect("timer query")
        .expect("rebuilt timer");
    assert_ne!(new_job.id, old_id);

    let old_get = client
        .get(timer_url(&old_id))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(old_get.status(), reqwest::StatusCode::NOT_FOUND);
    let new_get = client
        .get(timer_url(&new_job.id))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(new_get.status(), reqwest::StatusCode::OK);

    let unknown = client
        .post(timer_url("missing-timer"))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "action": "reschedule", "dueDate": "PT1H" }))
        .send()
        .await
        .unwrap();
    assert_eq!(unknown.status(), reqwest::StatusCode::NOT_FOUND);

    for body in [
        json!({ "action": "reschedule" }),
        json!({ "action": "unsupported", "dueDate": "PT1H" }),
    ] {
        let invalid = client
            .post(timer_url(&new_job.id))
            .basic_auth("admin", Some("test"))
            .json(&body)
            .send()
            .await
            .unwrap();
        assert_eq!(invalid.status(), reqwest::StatusCode::BAD_REQUEST, "{body}");
    }
}

#[tokio::test]
async fn deadletter_move_routes_by_job_type_and_executes_revived_job() {
    let (engine, cmmn) = build_engine("p135-cmmn-rest-deadletter");
    let timer_id = scheduled_timer(cmmn.as_ref());
    let case_id = cmmn
        .management_service()
        .get_job(&timer_id)
        .expect("timer")
        .scope_id
        .expect("case id");

    let mut runtime = CmmnJob::new("deadletter-runtime-rest", CmmnJobFamily::Deadletter)
        .with_handler(
            TYPE_SET_ASYNC_VARIABLES,
            Some(r#"{"revived":true}"#.to_string()),
        );
    runtime.job_type = Some("message".to_string());
    runtime.scope_id = Some(case_id.clone());
    runtime.retries = 0;
    cmmn.management_service()
        .insert_job(runtime)
        .expect("runtime deadletter");

    let mut history = CmmnJob::new("deadletter-history-rest", CmmnJobFamily::Deadletter);
    history.job_type = Some("history".to_string());
    history.retries = 0;
    cmmn.management_service()
        .insert_job(history)
        .expect("history deadletter");

    let mut wrong_destination =
        CmmnJob::new("deadletter-wrong-rest", CmmnJobFamily::Deadletter);
    wrong_destination.job_type = Some("message".to_string());
    cmmn.management_service()
        .insert_job(wrong_destination)
        .expect("wrong destination fixture");

    let (base_url, client) = spawn_server(engine).await;
    let deadletter_url =
        |id: &str| format!("{base_url}/cmmn-management/deadletter-jobs/{id}");

    // Java JobResource.java:306-323 routes non-history jobType to executable.
    let moved = client
        .post(deadletter_url("deadletter-runtime-rest"))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "action": "move" }))
        .send()
        .await
        .unwrap();
    assert_eq!(moved.status(), reqwest::StatusCode::NO_CONTENT);
    assert_eq!(
        cmmn.management_service()
            .get_job("deadletter-runtime-rest")
            .expect("revived executable")
            .family,
        CmmnJobFamily::Executable
    );

    let executed = client
        .post(format!(
            "{base_url}/cmmn-management/jobs/deadletter-runtime-rest"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "action": "execute" }))
        .send()
        .await
        .unwrap();
    assert_eq!(executed.status(), reqwest::StatusCode::NO_CONTENT);
    assert!(
        cmmn.management_service()
            .get_job("deadletter-runtime-rest")
            .is_err()
    );
    assert_eq!(
        cmmn.runtime_service()
            .get_case_instance(&case_id)
            .expect("case")
            .variables["revived"],
        true
    );

    // Java JobResource.java:316-319 routes history jobType back to history.
    let moved_history = client
        .post(deadletter_url("deadletter-history-rest"))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "action": "move" }))
        .send()
        .await
        .unwrap();
    assert_eq!(moved_history.status(), reqwest::StatusCode::NO_CONTENT);
    assert_eq!(
        cmmn.management_service()
            .get_job("deadletter-history-rest")
            .expect("revived history")
            .family,
        CmmnJobFamily::History
    );

    let wrong = client
        .post(deadletter_url("deadletter-wrong-rest"))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "action": "moveToHistoryJob" }))
        .send()
        .await
        .unwrap();
    assert_eq!(wrong.status(), reqwest::StatusCode::BAD_REQUEST);
    assert_eq!(
        cmmn.management_service()
            .get_job("deadletter-wrong-rest")
            .expect("failed move is transactional")
            .family,
        CmmnJobFamily::Deadletter
    );

    let missing = client
        .post(deadletter_url("missing-deadletter"))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "action": "move" }))
        .send()
        .await
        .unwrap();
    assert_eq!(missing.status(), reqwest::StatusCode::NOT_FOUND);

    let invalid = client
        .post(deadletter_url("deadletter-wrong-rest"))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "action": "unsupported" }))
        .send()
        .await
        .unwrap();
    assert_eq!(invalid.status(), reqwest::StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn history_job_execute_runs_direct_fixture_and_maps_error_paths() {
    let (engine, cmmn) = build_engine("p135-cmmn-rest-history-execute");
    let timer_id = scheduled_timer(cmmn.as_ref());
    let case_id = cmmn
        .management_service()
        .get_job(&timer_id)
        .expect("timer")
        .scope_id
        .expect("case id");

    let mut history = CmmnJob::new("history-rest", CmmnJobFamily::History).with_handler(
        TYPE_SET_ASYNC_VARIABLES,
        Some(r#"{"historyRestExecuted":true}"#.to_string()),
    );
    history.scope_id = Some(case_id.clone());
    cmmn.management_service()
        .insert_job(history)
        .expect("direct history fixture");
    let executable = CmmnJob::new("executable-not-history", CmmnJobFamily::Executable)
        .with_handler(
            TYPE_SET_ASYNC_VARIABLES,
            Some(r#"{"wrongFamilyExecuted":true}"#.to_string()),
        );
    cmmn.management_service()
        .insert_job(executable)
        .expect("executable fixture");

    let (base_url, client) = spawn_server(engine).await;
    let history_url = |id: &str| format!("{base_url}/cmmn-management/history-jobs/{id}");
    let executed = client
        .post(history_url("history-rest"))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "action": "execute" }))
        .send()
        .await
        .unwrap();
    assert_eq!(executed.status(), reqwest::StatusCode::NO_CONTENT);
    assert!(cmmn.management_service().get_job("history-rest").is_err());
    assert_eq!(
        cmmn.runtime_service()
            .get_case_instance(&case_id)
            .expect("case")
            .variables["historyRestExecuted"],
        true
    );

    let missing = client
        .post(history_url("missing-history"))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "action": "execute" }))
        .send()
        .await
        .unwrap();
    assert_eq!(missing.status(), reqwest::StatusCode::NOT_FOUND);

    let mismatch = client
        .post(history_url("executable-not-history"))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "action": "execute" }))
        .send()
        .await
        .unwrap();
    assert_eq!(mismatch.status(), reqwest::StatusCode::NOT_FOUND);
    assert!(
        cmmn.management_service()
            .get_job("executable-not-history")
            .is_ok()
    );

    let invalid = client
        .post(history_url("executable-not-history"))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "action": "move" }))
        .send()
        .await
        .unwrap();
    assert_eq!(invalid.status(), reqwest::StatusCode::BAD_REQUEST);
}
