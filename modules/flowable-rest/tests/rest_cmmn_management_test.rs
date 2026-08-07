//! CMMN management REST contracts: suspended job delete + family isolation.

use chrono::{TimeZone, Utc};
use flowable_cmmn_engine::{
    CmmnCase, CmmnCaseInstanceStartRequest, CmmnCasePlanModel, CmmnDeploymentRequest, CmmnEngine,
    CmmnHumanTask, CmmnJob, CmmnJobFamily, CmmnModel, CmmnPlanItem,
};
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::engine::time_source::TestTimeSource;
use flowable_rest::run_server;
use serde_json::Value;
use std::sync::Arc;
use tokio::net::TcpListener;

fn build_engine(test_name: &str) -> (Arc<ProcessEngine>, Arc<CmmnEngine>) {
    let now = Utc.with_ymd_and_hms(2026, 4, 21, 12, 0, 0).unwrap();
    let time_source = Arc::new(TestTimeSource::new(now));
    let db_store =
        Arc::new(flowable_engine::persistence::db_store::DbStore::new_in_memory().unwrap());
    let engine = Arc::new(ProcessEngine::build(
        test_name.to_string(),
        Arc::clone(&time_source) as Arc<_>,
        db_store,
    ));

    let user = flowable_engine::identity::entities::User {
        id: "admin".to_string(),
        first_name: None,
        last_name: None,
        email: None,
        password: Some("test".to_string()),
        tenant_id: None,
    };
    engine.get_identity_service().save_user(user);

    let cmmn = engine
        .get_config()
        .cmmn_engine
        .as_ref()
        .expect("process engine should embed a CMMN engine")
        .clone();
    (engine, cmmn)
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

fn simple_case_model() -> CmmnModel {
    let plan_model = CmmnCasePlanModel::new("casePlanModel", "Case Plan Model")
        .with_human_task(CmmnHumanTask::new("reviewTask", "Review"))
        .with_plan_item(CmmnPlanItem::new("planItemReview", "reviewTask"));
    CmmnModel::new(vec![CmmnCase::new(
        "mgmtCaseDefinition",
        "mgmtCase",
        "Management Case",
        plan_model,
    )])
}

fn start_case(cmmn: &CmmnEngine) -> String {
    cmmn.deploy(
        CmmnDeploymentRequest::new("cmmn-mgmt").with_resource("mgmt.cmmn", simple_case_model()),
    )
    .expect("deploy");
    cmmn.start_case_instance_by_key("mgmtCase", CmmnCaseInstanceStartRequest::new())
        .expect("start")
        .id
}

fn insert_family(cmmn: &CmmnEngine, id: &str, family: CmmnJobFamily, case_id: &str) {
    let mut job = CmmnJob::new(id, family);
    job.scope_id = Some(case_id.to_string());
    job.exception_stacktrace = Some(format!("stack for {id}"));
    cmmn.management_service().insert_job(job).expect("insert");
}

#[tokio::test]
async fn cmmn_suspended_delete_returns_204_and_family_mismatch_404() {
    let (engine, cmmn) = build_engine("rest-cmmn-mgmt-delete");
    let case_id = start_case(cmmn.as_ref());
    insert_family(cmmn.as_ref(), "susp-1", CmmnJobFamily::Suspended, &case_id);
    insert_family(cmmn.as_ref(), "exec-1", CmmnJobFamily::Executable, &case_id);
    insert_family(cmmn.as_ref(), "timer-1", CmmnJobFamily::Timer, &case_id);
    insert_family(cmmn.as_ref(), "dl-1", CmmnJobFamily::Deadletter, &case_id);
    insert_family(cmmn.as_ref(), "hist-1", CmmnJobFamily::History, &case_id);

    let (base_url, client) = spawn_server(engine).await;

    let deleted = client
        .delete(format!("{base_url}/cmmn-management/suspended-jobs/susp-1"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(deleted.status(), reqwest::StatusCode::NO_CONTENT);
    assert!(deleted.text().await.unwrap().is_empty());
    assert!(cmmn.management_service().get_job("susp-1").is_err());

    for (path_id, family) in [
        ("exec-1", CmmnJobFamily::Executable),
        ("timer-1", CmmnJobFamily::Timer),
        ("dl-1", CmmnJobFamily::Deadletter),
        ("hist-1", CmmnJobFamily::History),
        ("unknown-id", CmmnJobFamily::Suspended),
    ] {
        let response = client
            .delete(format!(
                "{base_url}/cmmn-management/suspended-jobs/{path_id}"
            ))
            .basic_auth("admin", Some("test"))
            .send()
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            reqwest::StatusCode::NOT_FOUND,
            "{path_id}"
        );
        if path_id != "unknown-id" {
            assert_eq!(
                cmmn.management_service()
                    .get_job(path_id)
                    .expect("unchanged")
                    .family,
                family
            );
        }
    }
}

#[tokio::test]
async fn cmmn_management_list_get_stacktrace_are_family_isolated() {
    let (engine, cmmn) = build_engine("rest-cmmn-mgmt-family-iso");
    let case_id = start_case(cmmn.as_ref());
    let created = Utc.with_ymd_and_hms(2026, 5, 1, 10, 0, 0).unwrap();

    let mut suspended = CmmnJob::new("susp-iso", CmmnJobFamily::Suspended);
    suspended.scope_id = Some(case_id.clone());
    suspended.created_at = created;
    suspended.exception_stacktrace = Some("suspended stack".to_string());
    cmmn.management_service()
        .insert_job(suspended)
        .expect("insert");

    let mut timer = CmmnJob::new("timer-iso", CmmnJobFamily::Timer);
    timer.scope_id = Some(case_id);
    timer.created_at = created;
    timer.exception_stacktrace = Some("timer stack".to_string());
    cmmn.management_service().insert_job(timer).expect("insert");

    let (base_url, client) = spawn_server(engine).await;

    let susp_list = client
        .get(format!(
            "{base_url}/cmmn-management/suspended-jobs?start=0&size=10"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(susp_list.status().is_success());
    let susp_body: Value = susp_list.json().await.unwrap();
    assert_eq!(susp_body["total"], 1);
    assert_eq!(susp_body["data"][0]["id"], "susp-iso");
    assert_eq!(susp_body["data"][0]["jobType"], "suspended");

    let timer_via_suspended = client
        .get(format!(
            "{base_url}/cmmn-management/suspended-jobs/timer-iso"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(timer_via_suspended.status(), reqwest::StatusCode::NOT_FOUND);

    let susp_get = client
        .get(format!(
            "{base_url}/cmmn-management/suspended-jobs/susp-iso"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(susp_get.status().is_success());

    let stack = client
        .get(format!(
            "{base_url}/cmmn-management/suspended-jobs/susp-iso/exception-stacktrace"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(stack.status().is_success());
    assert_eq!(stack.text().await.unwrap(), "suspended stack");

    let timer_stack_via_suspended = client
        .get(format!(
            "{base_url}/cmmn-management/suspended-jobs/timer-iso/exception-stacktrace"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(
        timer_stack_via_suspended.status(),
        reqwest::StatusCode::NOT_FOUND
    );

    let timer_list = client
        .get(format!(
            "{base_url}/cmmn-management/timer-jobs?start=0&size=10"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    let timer_body: Value = timer_list.json().await.unwrap();
    assert_eq!(timer_body["total"], 1);
    assert_eq!(timer_body["data"][0]["id"], "timer-iso");
}
