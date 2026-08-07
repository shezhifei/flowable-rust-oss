use chrono::{TimeZone, Utc};
use flowable_cmmn_engine::{
    CmmnCase, CmmnCaseInstanceStartRequest, CmmnCasePlanModel, CmmnDeploymentRequest, CmmnEngine,
    CmmnHumanTask, CmmnHumanTaskState, CmmnJob, CmmnJobFamily, CmmnModel, CmmnPlanItem,
};
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::engine::repository_service::PROCESS_DEFINITION_SUSPEND_TIMER_ACTIVITY_ID;
use flowable_engine::engine::time_source::TestTimeSource;
use flowable_engine::persistence::runtime_store::{RuntimeJobType, RuntimeTimerJobState};
use flowable_engine::runtime::process_instance::ProcessInstanceUpdate;
use flowable_rest::run_server;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::net::TcpListener;

const TIMER_WAIT_PROCESS_BPMN: &str = r#"
<bpmn2:definitions xmlns:bpmn2="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="http://flowable.org/bpmn">
  <bpmn2:process id="rest_external_worker_process" isExecutable="true">
    <bpmn2:startEvent id="start" />
    <bpmn2:sequenceFlow id="flow1" sourceRef="start" targetRef="timer" />
    <bpmn2:intermediateCatchEvent id="timer">
      <bpmn2:timerEventDefinition>
        <bpmn2:timeDuration>PT5M</bpmn2:timeDuration>
      </bpmn2:timerEventDefinition>
    </bpmn2:intermediateCatchEvent>
    <bpmn2:sequenceFlow id="flow2" sourceRef="timer" targetRef="end" />
    <bpmn2:endEvent id="end" />
  </bpmn2:process>
</bpmn2:definitions>
"#;

const BPMN_ERROR_BOUNDARY_PROCESS_BPMN: &str = r#"
<bpmn2:definitions xmlns:bpmn2="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="http://flowable.org/bpmn">
  <bpmn2:process id="rest_external_worker_bpmn_error_boundary_process" isExecutable="true">
    <bpmn2:startEvent id="start" />
    <bpmn2:sequenceFlow id="flow1" sourceRef="start" targetRef="workerScope" />
    <bpmn2:subProcess id="workerScope">
      <bpmn2:startEvent id="scopeStart" />
      <bpmn2:sequenceFlow id="scopeFlow1" sourceRef="scopeStart" targetRef="timer" />
      <bpmn2:intermediateCatchEvent id="timer">
        <bpmn2:timerEventDefinition>
          <bpmn2:timeDuration>PT5M</bpmn2:timeDuration>
        </bpmn2:timerEventDefinition>
      </bpmn2:intermediateCatchEvent>
      <bpmn2:sequenceFlow id="scopeFlow2" sourceRef="timer" targetRef="scopeAfterErrorShouldNotRun" />
      <bpmn2:userTask id="scopeAfterErrorShouldNotRun" name="Should Not Run" />
      <bpmn2:sequenceFlow id="scopeFlow3" sourceRef="scopeAfterErrorShouldNotRun" targetRef="scopeEnd" />
      <bpmn2:endEvent id="scopeEnd" />
    </bpmn2:subProcess>
    <bpmn2:boundaryEvent id="catchBusinessError" attachedToRef="workerScope">
      <bpmn2:errorEventDefinition errorCode="BUSINESS_ERROR" />
    </bpmn2:boundaryEvent>
    <bpmn2:sequenceFlow id="errorFlow" sourceRef="catchBusinessError" targetRef="errorTask" />
    <bpmn2:userTask id="errorTask" name="Error Task" />
    <bpmn2:sequenceFlow id="errorEndFlow" sourceRef="errorTask" targetRef="end" />
    <bpmn2:sequenceFlow id="normalFlow" sourceRef="workerScope" targetRef="end" />
    <bpmn2:endEvent id="end" />
  </bpmn2:process>
</bpmn2:definitions>
"#;

fn build_engine(test_name: &str) -> (Arc<ProcessEngine>, Arc<TestTimeSource>) {
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

    (engine, time_source)
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

fn deploy_process(engine: &ProcessEngine, resource_name: &str, bpmn: &str) -> String {
    let repository_service = engine.get_repository_service();
    repository_service
        .deploy(
            repository_service
                .create_deployment()
                .name(resource_name.to_string())
                .add_string(resource_name.to_string(), bpmn.to_string()),
        )
        .unwrap();

    repository_service
        .get_process_definition_ids()
        .unwrap()
        .into_iter()
        .last()
        .unwrap()
}

fn deploy_timer_wait_process(engine: &ProcessEngine) -> String {
    deploy_process(
        engine,
        "rest-external-worker.bpmn20.xml",
        TIMER_WAIT_PROCESS_BPMN,
    )
}

fn start_timer_wait_process(engine: &ProcessEngine) -> String {
    let process_definition_id = deploy_timer_wait_process(engine);
    engine
        .get_runtime_service()
        .start_process_instance(
            engine
                .get_runtime_service()
                .create_process_instance_builder()
                .process_definition_id(process_definition_id),
        )
        .unwrap()
        .id
}

fn start_process(engine: &ProcessEngine, resource_name: &str, bpmn: &str) -> String {
    let process_definition_id = deploy_process(engine, resource_name, bpmn);
    engine
        .get_runtime_service()
        .start_process_instance(
            engine
                .get_runtime_service()
                .create_process_instance_builder()
                .process_definition_id(process_definition_id),
        )
        .unwrap()
        .id
}

fn shared_cmmn_engine(engine: &ProcessEngine) -> Arc<CmmnEngine> {
    engine
        .get_config()
        .cmmn_engine
        .as_ref()
        .expect("test process engine should have a CMMN engine")
        .clone()
}

fn cmmn_external_worker_terminate_model() -> CmmnModel {
    let plan_model = CmmnCasePlanModel::new("casePlanModel", "Case Plan Model")
        .with_human_task(CmmnHumanTask::new("reviewTask", "Review"))
        .with_plan_item(CmmnPlanItem::new("planItemReview", "reviewTask"));

    CmmnModel::new(vec![CmmnCase::new(
        "externalWorkerTerminateCaseDefinition",
        "externalWorkerTerminateCase",
        "External Worker Terminate Case",
        plan_model,
    )])
}

fn start_cmmn_external_worker_case(cmmn_engine: &CmmnEngine) -> (String, String) {
    cmmn_engine
        .deploy(
            CmmnDeploymentRequest::new("external-worker-cmmn-terminate").with_resource(
                "external-worker-terminate.cmmn",
                cmmn_external_worker_terminate_model(),
            ),
        )
        .expect("cmmn deployment");
    let case_definition_id = cmmn_engine
        .repository_service()
        .create_case_definition_query()
        .key("externalWorkerTerminateCase")
        .single_result()
        .expect("case definition query")
        .expect("case definition")
        .id;
    let case_instance = cmmn_engine
        .start_case_instance_by_key(
            "externalWorkerTerminateCase",
            CmmnCaseInstanceStartRequest::new(),
        )
        .expect("cmmn case instance");

    (case_instance.id, case_definition_id)
}

fn insert_locked_cmmn_external_worker_job(
    cmmn_engine: &CmmnEngine,
    job_id: &str,
    case_instance_id: &str,
    case_definition_id: &str,
    worker_id: Option<&str>,
) {
    let mut job = CmmnJob::new(job_id, CmmnJobFamily::Executable);
    job.scope_id = Some(case_instance_id.to_string());
    job.scope_definition_id = Some(case_definition_id.to_string());
    job.element_id = Some("reviewTask".to_string());
    job.lock_owner = worker_id.map(str::to_string);
    cmmn_engine
        .management_service()
        .insert_job(job)
        .expect("cmmn external worker job");
}

#[tokio::test]
async fn external_worker_endpoints_cover_fetch_query_failure_unlock_and_complete() {
    let (engine, time_source) = build_engine("rest-external-worker-happy");
    let (base_url, client) = spawn_server(Arc::clone(&engine)).await;
    let process_instance_id = start_timer_wait_process(&engine);
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let process_definition_id = store
        .find_process_instance(&process_instance_id, &mut session)
        .unwrap()
        .process_definition_id;
    let _ = session.rollback();
    time_source.advance_time(300_001);

    let fetch = client
        .post(format!("{}/external-worker/jobs/fetch-and-lock", base_url))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "workerId": "worker-a",
            "maxJobs": 1,
            "lockDurationMs": 60_000
        }))
        .send()
        .await
        .unwrap();

    assert!(fetch.status().is_success());
    let fetch_body: Value = fetch.json().await.unwrap();
    let jobs = fetch_body.as_array().unwrap();
    assert_eq!(jobs.len(), 1);
    let job = &jobs[0];
    assert_eq!(job["processInstanceId"], process_instance_id);
    assert_eq!(job["processDefinitionId"], process_definition_id);
    assert!(!job["executionId"].as_str().unwrap().is_empty());
    assert_eq!(job["elementId"], "timer");
    assert_eq!(job["jobKind"], "runtimeTimer");
    assert_eq!(job["lockOwner"], "worker-a");
    assert!(job["dueDate"].is_string());
    assert!(job["lockExpirationTime"].is_string());
    let job_id = job["id"].as_str().unwrap().to_string();

    let list_locked = client
        .get(format!(
            "{}/external-worker/jobs?locked=true&start=0&size=10",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert!(list_locked.status().is_success());
    let list_locked_body: Value = list_locked.json().await.unwrap();
    assert_eq!(list_locked_body["start"], 0);
    assert_eq!(list_locked_body["size"], 1);
    assert_eq!(list_locked_body["total"], 1);
    assert_eq!(list_locked_body["data"][0]["id"], job_id);
    assert_eq!(list_locked_body["data"][0]["lockOwner"], "worker-a");

    let failure = client
        .post(format!(
            "{}/external-worker/jobs/{}/failure",
            base_url, job_id
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "workerId": "worker-a",
            "errorMessage": "worker failed",
            "errorDetails": "stacktrace",
            "retries": 3,
            "retryDurationMs": 45_000
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(failure.status(), reqwest::StatusCode::NO_CONTENT);

    let list_failed = client
        .get(format!(
            "{}/external-worker/jobs?id={}&start=0&size=10",
            base_url, job_id
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert!(list_failed.status().is_success());
    let list_failed_body: Value = list_failed.json().await.unwrap();
    assert_eq!(list_failed_body["total"], 1);
    assert_eq!(
        list_failed_body["data"][0]["exceptionMessage"],
        "worker failed"
    );
    assert_eq!(list_failed_body["data"][0]["errorDetails"], "stacktrace");
    assert!(list_failed_body["data"][0]["lockOwner"].is_null());

    time_source.advance_time(45_001);

    let refetch = client
        .post(format!("{}/external-worker/jobs/fetch-and-lock", base_url))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "workerId": "worker-b",
            "maxJobs": 1,
            "lockDurationMs": 60_000
        }))
        .send()
        .await
        .unwrap();

    assert!(refetch.status().is_success());
    let refetch_body: Value = refetch.json().await.unwrap();
    assert_eq!(refetch_body.as_array().unwrap()[0]["id"], job_id);
    assert_eq!(refetch_body.as_array().unwrap()[0]["lockOwner"], "worker-b");

    let unlock = client
        .post(format!(
            "{}/external-worker/jobs/{}/unlock",
            base_url, job_id
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "workerId": "worker-b"
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(unlock.status(), reqwest::StatusCode::NO_CONTENT);

    let reacquire = client
        .post(format!("{}/external-worker/jobs/fetch-and-lock", base_url))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "workerId": "worker-c",
            "maxJobs": 1,
            "lockDurationMs": 60_000
        }))
        .send()
        .await
        .unwrap();

    assert!(reacquire.status().is_success());
    let reacquire_body: Value = reacquire.json().await.unwrap();
    assert_eq!(reacquire_body.as_array().unwrap()[0]["id"], job_id);
    assert_eq!(
        reacquire_body.as_array().unwrap()[0]["lockOwner"],
        "worker-c"
    );

    let complete = client
        .post(format!(
            "{}/external-worker/jobs/{}/complete",
            base_url, job_id
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "workerId": "worker-c"
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(complete.status(), reqwest::StatusCode::NO_CONTENT);

    let list_after_complete = client
        .get(format!(
            "{}/external-worker/jobs?id={}&start=0&size=10",
            base_url, job_id
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert!(list_after_complete.status().is_success());
    let list_after_complete_body: Value = list_after_complete.json().await.unwrap();
    assert_eq!(list_after_complete_body["total"], 0);
}

#[tokio::test]
async fn external_worker_errors_are_structured_and_query_rejects_unknown_fields() {
    let (engine, time_source) = build_engine("rest-external-worker-errors");
    let (base_url, client) = spawn_server(Arc::clone(&engine)).await;
    let _process_instance_id = start_timer_wait_process(&engine);
    time_source.advance_time(300_001);

    let fetch = client
        .post(format!("{}/external-worker/jobs/fetch-and-lock", base_url))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "workerId": "worker-a",
            "maxJobs": 1,
            "lockDurationMs": 1_000
        }))
        .send()
        .await
        .unwrap();

    assert!(fetch.status().is_success());
    let fetch_body: Value = fetch.json().await.unwrap();
    let job_id = fetch_body.as_array().unwrap()[0]["id"]
        .as_str()
        .unwrap()
        .to_string();

    let wrong_worker = client
        .post(format!(
            "{}/external-worker/jobs/{}/complete",
            base_url, job_id
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "workerId": "worker-b"
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(wrong_worker.status(), reqwest::StatusCode::FORBIDDEN);
    let wrong_worker_body: Value = wrong_worker.json().await.unwrap();
    assert_eq!(wrong_worker_body["code"], "FORBIDDEN");
    assert_eq!(wrong_worker_body["message"], "Forbidden");
    assert!(
        wrong_worker_body["details"]
            .as_str()
            .unwrap()
            .contains("different worker")
    );

    time_source.advance_time(1_001);

    let expired_owner_lock = client
        .post(format!(
            "{}/external-worker/jobs/{}/complete",
            base_url, job_id
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "workerId": "worker-a"
        }))
        .send()
        .await
        .unwrap();

    // Java AbstractExternalWorkerJobCmd resolves ownership only. Expiration is
    // cleared asynchronously by the reset worker and does not make an owning
    // worker's command invalid while lockOwner is still present.
    assert_eq!(expired_owner_lock.status(), reqwest::StatusCode::NO_CONTENT);

    let unknown_job = client
        .post(format!(
            "{}/external-worker/jobs/{}/complete",
            base_url, "missing-job"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "workerId": "worker-a"
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(unknown_job.status(), reqwest::StatusCode::NOT_FOUND);
    let unknown_job_body: Value = unknown_job.json().await.unwrap();
    assert_eq!(unknown_job_body["code"], "NOT_FOUND");
    assert_eq!(unknown_job_body["message"], "Not Found");
    assert!(
        unknown_job_body["details"]
            .as_str()
            .unwrap()
            .contains("missing-job")
    );

    let bad_query = client
        .get(format!(
            "{}/external-worker/jobs?unexpectedField=value",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(bad_query.status(), reqwest::StatusCode::BAD_REQUEST);
    let bad_query_body: Value = bad_query.json().await.unwrap();
    assert_eq!(bad_query_body["code"], "BAD_REQUEST");
    assert_eq!(bad_query_body["message"], "Bad Request");
    assert!(
        bad_query_body["details"]
            .as_str()
            .unwrap()
            .contains("unexpectedField")
    );
}

#[tokio::test]
async fn external_worker_canonical_paths_cover_fetch_query_failure_unlock_and_complete() {
    let (engine, time_source) = build_engine("rest-external-worker-paths");
    let (base_url, client) = spawn_server(Arc::clone(&engine)).await;
    let process_instance_id = start_timer_wait_process(&engine);
    time_source.advance_time(300_001);

    let acquire = client
        .post(format!("{}/external-worker/jobs/fetch-and-lock", base_url))
        .basic_auth("admin", Some("test"))
        // P68: topic filters job_handler_configuration. Legacy timer-backed
        // external-worker candidates have no topic — omit so they remain acquirable.
        .json(&json!({
            "workerId": "worker-a",
            "numberOfTasks": 1,
            "lockDuration": "PT1M"
        }))
        .send()
        .await
        .unwrap();

    assert!(acquire.status().is_success());
    let acquire_body: Value = acquire.json().await.unwrap();
    let job = &acquire_body.as_array().unwrap()[0];
    assert_eq!(job["processInstanceId"], process_instance_id);
    assert_eq!(job["lockOwner"], "worker-a");
    let job_id = job["id"].as_str().unwrap().to_string();

    let list = client
        .get(format!(
            "{}/external-worker/jobs?locked=true&start=0&size=10",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert!(list.status().is_success());
    let list_body: Value = list.json().await.unwrap();
    assert_eq!(list_body["total"], 1);
    assert_eq!(list_body["data"][0]["id"], job_id);

    let get = client
        .get(format!("{}/external-worker/jobs/{}", base_url, job_id))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert!(get.status().is_success());
    let get_body: Value = get.json().await.unwrap();
    assert_eq!(get_body["id"], job_id);

    let fail = client
        .post(format!(
            "{}/external-worker/jobs/{}/failure",
            base_url, job_id
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "workerId": "worker-a",
            "errorMessage": "worker failed",
            "errorDetails": "stacktrace",
            "retries": 3,
            "retryTimeout": "PT45S"
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(fail.status(), reqwest::StatusCode::NO_CONTENT);

    time_source.advance_time(45_001);

    let reacquire = client
        .post(format!("{}/external-worker/jobs/fetch-and-lock", base_url))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "workerId": "worker-b",
            "numberOfTasks": 1,
            "lockDuration": "PT1M"
        }))
        .send()
        .await
        .unwrap();

    assert!(reacquire.status().is_success());
    assert_eq!(
        reacquire.json::<Value>().await.unwrap()[0]["lockOwner"],
        "worker-b"
    );

    let unacquire = client
        .post(format!(
            "{}/external-worker/jobs/{}/unlock",
            base_url, job_id
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "workerId": "worker-b"
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(unacquire.status(), reqwest::StatusCode::NO_CONTENT);

    let final_acquire = client
        .post(format!("{}/external-worker/jobs/fetch-and-lock", base_url))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "workerId": "worker-c",
            "numberOfTasks": 1,
            "lockDuration": "PT1M"
        }))
        .send()
        .await
        .unwrap();

    assert!(final_acquire.status().is_success());

    let complete = client
        .post(format!(
            "{}/external-worker/jobs/{}/complete",
            base_url, job_id
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "workerId": "worker-c"
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(complete.status(), reqwest::StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn external_worker_bpmn_error_triggers_matching_boundary_error_path() {
    let (engine, time_source) = build_engine("rest-external-worker-bpmn-error-boundary");
    let (base_url, client) = spawn_server(Arc::clone(&engine)).await;
    let process_instance_id = start_process(
        &engine,
        "rest-external-worker-bpmn-error-boundary.bpmn20.xml",
        BPMN_ERROR_BOUNDARY_PROCESS_BPMN,
    );
    time_source.advance_time(300_001);

    let acquire = client
        .post(format!("{}/external-worker/jobs/fetch-and-lock", base_url))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "workerId": "worker-a",
            "numberOfTasks": 1,
            "lockDuration": "PT1M"
        }))
        .send()
        .await
        .unwrap();

    assert!(acquire.status().is_success());
    let acquire_body: Value = acquire.json().await.unwrap();
    let job_id = acquire_body.as_array().unwrap()[0]["id"]
        .as_str()
        .unwrap()
        .to_string();

    let bpmn_error = client
        .post(format!(
            "{}/external-worker/jobs/{}/bpmnError",
            base_url, job_id
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "workerId": "worker-a",
            "errorCode": "BUSINESS_ERROR"
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(bpmn_error.status(), reqwest::StatusCode::NO_CONTENT);

    let tasks = engine
        .get_task_service()
        .get_tasks_by_process_instance_id(process_instance_id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].task_definition_key, "errorTask");
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    assert!(
        store.find_timer_job_state(&job_id, &mut session).is_none(),
        "handled BPMN error should consume the locked external worker job"
    );
    let _ = session.rollback();
}

#[tokio::test]
async fn external_worker_bpmn_error_without_handler_returns_structured_error() {
    let (engine, time_source) = build_engine("rest-external-worker-bpmn-error-no-handler");
    let (base_url, client) = spawn_server(Arc::clone(&engine)).await;
    let _process_instance_id = start_timer_wait_process(&engine);
    time_source.advance_time(300_001);

    let acquire = client
        .post(format!("{}/external-worker/jobs/fetch-and-lock", base_url))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "workerId": "worker-a",
            "numberOfTasks": 1,
            "lockDuration": "PT1M"
        }))
        .send()
        .await
        .unwrap();

    assert!(acquire.status().is_success());
    let acquire_body: Value = acquire.json().await.unwrap();
    let job_id = acquire_body.as_array().unwrap()[0]["id"]
        .as_str()
        .unwrap()
        .to_string();

    let bpmn_error = client
        .post(format!(
            "{}/external-worker/jobs/{}/bpmnError",
            base_url, job_id
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "workerId": "worker-a",
            "errorCode": "BUSINESS_ERROR"
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(bpmn_error.status(), reqwest::StatusCode::BAD_REQUEST);
    let bpmn_error_body: Value = bpmn_error.json().await.unwrap();
    assert_eq!(bpmn_error_body["code"], "BAD_REQUEST");
    assert_eq!(bpmn_error_body["message"], "Bad Request");
    assert!(
        bpmn_error_body["details"]
            .as_str()
            .unwrap()
            .contains("No matching BPMN error handler")
    );
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    assert!(
        store.find_timer_job_state(&job_id, &mut session).is_some(),
        "unhandled BPMN error must not consume the locked external worker job"
    );
    let _ = session.rollback();
}

#[tokio::test]
async fn external_worker_cmmn_terminate_terminates_locked_cmmn_plan_item() {
    let (engine, _time_source) = build_engine("rest-external-worker-cmmn-terminate");
    let cmmn_engine = shared_cmmn_engine(&engine);
    let (case_instance_id, case_definition_id) =
        start_cmmn_external_worker_case(cmmn_engine.as_ref());
    let (base_url, client) = spawn_server(Arc::clone(&engine)).await;
    let job_id = "cmmn-external-worker-job-terminate";
    insert_locked_cmmn_external_worker_job(
        cmmn_engine.as_ref(),
        job_id,
        &case_instance_id,
        &case_definition_id,
        Some("worker-a"),
    );

    let cmmn_terminate = client
        .post(format!(
            "{}/external-worker/jobs/{}/cmmnTerminate",
            base_url, job_id
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "workerId": "worker-a"
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(cmmn_terminate.status(), reqwest::StatusCode::NO_CONTENT);
    let active_tasks = cmmn_engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(case_instance_id.clone())
        .list()
        .unwrap();
    assert!(active_tasks.is_empty());
    let historic_tasks = cmmn_engine
        .history_service()
        .create_historic_human_task_query()
        .case_instance_id(case_instance_id)
        .state(CmmnHumanTaskState::Terminated)
        .list()
        .unwrap();
    assert_eq!(historic_tasks.len(), 1);
    assert!(
        cmmn_engine.management_service().get_job(job_id).is_err(),
        "successful CMMN terminate should consume the external worker job"
    );
}

#[tokio::test]
async fn external_worker_cmmn_terminate_rejects_wrong_worker_and_keeps_job() {
    let (engine, _time_source) = build_engine("rest-external-worker-cmmn-terminate-wrong-worker");
    let cmmn_engine = shared_cmmn_engine(&engine);
    let (case_instance_id, case_definition_id) =
        start_cmmn_external_worker_case(cmmn_engine.as_ref());
    let (base_url, client) = spawn_server(Arc::clone(&engine)).await;
    let job_id = "cmmn-external-worker-job-wrong-worker";
    insert_locked_cmmn_external_worker_job(
        cmmn_engine.as_ref(),
        job_id,
        &case_instance_id,
        &case_definition_id,
        Some("worker-a"),
    );

    let cmmn_terminate = client
        .post(format!(
            "{}/external-worker/jobs/{}/cmmnTerminate",
            base_url, job_id
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "workerId": "worker-b"
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(cmmn_terminate.status(), reqwest::StatusCode::FORBIDDEN);
    let cmmn_terminate_body: Value = cmmn_terminate.json().await.unwrap();
    assert_eq!(cmmn_terminate_body["code"], "FORBIDDEN");
    assert!(
        cmmn_terminate_body["details"]
            .as_str()
            .unwrap()
            .contains("different worker")
    );
    assert!(
        cmmn_engine.management_service().get_job(job_id).is_ok(),
        "failed CMMN terminate must keep the external worker job"
    );
}

#[tokio::test]
async fn external_worker_remaining_paths_cover_bulk_unlock_and_structured_unsupported_transitions()
{
    let (engine, time_source) = build_engine("rest-external-worker-remaining-paths");
    let (base_url, client) = spawn_server(Arc::clone(&engine)).await;
    let _process_instance_id = start_timer_wait_process(&engine);
    time_source.advance_time(300_001);

    let acquire = client
        .post(format!("{}/external-worker/jobs/fetch-and-lock", base_url))
        .basic_auth("admin", Some("test"))
        // P68: topic filters job_handler_configuration. Legacy timer-backed
        // external-worker candidates have no topic — omit so they remain acquirable.
        .json(&json!({
            "workerId": "worker-a",
            "numberOfTasks": 1,
            "lockDuration": "PT1M"
        }))
        .send()
        .await
        .unwrap();

    assert!(acquire.status().is_success());
    let acquire_body: Value = acquire.json().await.unwrap();
    let job_id = acquire_body.as_array().unwrap()[0]["id"]
        .as_str()
        .unwrap()
        .to_string();

    let bpmn_error = client
        .post(format!(
            "{}/external-worker/jobs/{}/bpmnError",
            base_url, job_id
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "workerId": "worker-a",
            "errorCode": "BUSINESS_ERROR"
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(bpmn_error.status(), reqwest::StatusCode::BAD_REQUEST);
    let bpmn_error_body: Value = bpmn_error.json().await.unwrap();
    assert_eq!(bpmn_error_body["code"], "BAD_REQUEST");
    assert!(
        bpmn_error_body["details"]
            .as_str()
            .unwrap()
            .contains("BPMN error")
    );

    let cmmn_terminate = client
        .post(format!(
            "{}/external-worker/jobs/{}/cmmnTerminate",
            base_url, job_id
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "workerId": "worker-a"
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(cmmn_terminate.status(), reqwest::StatusCode::BAD_REQUEST);
    let cmmn_terminate_body: Value = cmmn_terminate.json().await.unwrap();
    assert_eq!(cmmn_terminate_body["code"], "BAD_REQUEST");
    assert!(
        cmmn_terminate_body["details"]
            .as_str()
            .unwrap()
            .contains("CMMN terminate")
    );

    let bulk_unacquire = client
        .post(format!("{}/external-worker/jobs/bulk-unlock", base_url))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "workerId": "worker-a"
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(bulk_unacquire.status(), reqwest::StatusCode::NO_CONTENT);

    let list_unlocked = client
        .get(format!(
            "{}/external-worker/jobs?id={}&unlocked=true&start=0&size=10",
            base_url, job_id
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert!(list_unlocked.status().is_success());
    let list_unlocked_body: Value = list_unlocked.json().await.unwrap();
    assert_eq!(list_unlocked_body["total"], 1);
    assert!(list_unlocked_body["data"][0]["lockOwner"].is_null());
}

#[tokio::test]
async fn external_worker_bpmn_error_with_variables_propagates_to_error_handler() {
    let (engine, time_source) = build_engine("rest-external-worker-bpmn-error-variables");
    let (base_url, client) = spawn_server(Arc::clone(&engine)).await;
    let process_instance_id = start_process(
        &engine,
        "rest-external-worker-bpmn-error-variables.bpmn20.xml",
        BPMN_ERROR_BOUNDARY_PROCESS_BPMN,
    );
    time_source.advance_time(300_001);

    let acquire = client
        .post(format!("{}/external-worker/jobs/fetch-and-lock", base_url))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "workerId": "worker-a",
            "numberOfTasks": 1,
            "lockDuration": "PT1M"
        }))
        .send()
        .await
        .unwrap();

    assert!(acquire.status().is_success());
    let acquire_body: Value = acquire.json().await.unwrap();
    let job_id = acquire_body.as_array().unwrap()[0]["id"]
        .as_str()
        .unwrap()
        .to_string();

    let bpmn_error = client
        .post(format!(
            "{}/external-worker/jobs/{}/bpmnError",
            base_url, job_id
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "workerId": "worker-a",
            "errorCode": "BUSINESS_ERROR",
            "errorMessage": "something went wrong",
            "variables": [
                {"name": "errorSource", "value": "external-worker"},
                {"name": "errorCode", "value": 42}
            ]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(bpmn_error.status(), reqwest::StatusCode::NO_CONTENT);

    let tasks = engine
        .get_task_service()
        .get_tasks_by_process_instance_id(process_instance_id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].task_definition_key, "errorTask");

    // Single storage: the error variables are written to the process-instance
    // scope execution row and read back through the variable service.
    let variables = engine
        .get_variable_service()
        .get_variables(process_instance_id.clone())
        .unwrap();
    assert_eq!(
        variables.get("errorSource"),
        Some(&serde_json::json!("external-worker"))
    );
    assert_eq!(variables.get("errorCode"), Some(&serde_json::json!(42)));
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    assert!(
        store.find_timer_job_state(&job_id, &mut session).is_none(),
        "handled BPMN error with variables should consume the locked external worker job"
    );
    let _ = session.rollback();
}

#[tokio::test]
async fn external_worker_cmmn_terminate_false_unlocks_without_terminating() {
    let (engine, _time_source) = build_engine("rest-external-worker-cmmn-terminate-false");
    let cmmn_engine = shared_cmmn_engine(&engine);
    let (case_instance_id, case_definition_id) =
        start_cmmn_external_worker_case(cmmn_engine.as_ref());
    let (base_url, client) = spawn_server(Arc::clone(&engine)).await;
    let job_id = "cmmn-external-worker-job-terminate-false";
    insert_locked_cmmn_external_worker_job(
        cmmn_engine.as_ref(),
        job_id,
        &case_instance_id,
        &case_definition_id,
        Some("worker-a"),
    );

    let cmmn_terminate = client
        .post(format!(
            "{}/external-worker/jobs/{}/cmmnTerminate",
            base_url, job_id
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "workerId": "worker-a",
            "terminate": false
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(cmmn_terminate.status(), reqwest::StatusCode::NO_CONTENT);

    let active_tasks = cmmn_engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(case_instance_id.clone())
        .list()
        .unwrap();
    assert_eq!(
        active_tasks.len(),
        1,
        "terminate=false should keep the plan item alive"
    );
    assert!(
        cmmn_engine.management_service().get_job(job_id).is_err(),
        "terminate=false should still consume the external worker job"
    );
}

fn timer_job(
    id: &str,
    process_instance_id: &str,
    job_state: Option<&str>,
    activity_id: &str,
) -> RuntimeTimerJobState {
    RuntimeTimerJobState {
        timer_job_id: id.to_string(),
        process_instance_id: process_instance_id.to_string(),
        execution_id: format!("exec-{id}"),
        activity_id: activity_id.to_string(),
        job_state: job_state.map(str::to_string),
        is_boundary: false,
        attached_activity_id: None,
        cancel_activity: false,
        time_duration: None,
        time_date: None,
        time_cycle: None,
        due_time: Some(1),
        lock_owner: None,
        lock_time: None,
        lock_expiration_time: None,
        retries: Some(3),
        error_message: None,
        error_details: None,
        category: None,
        ..Default::default()
    }
}

#[tokio::test]
async fn external_worker_list_get_only_expose_active_external_worker_family() {
    let (engine, _time_source) = build_engine("rest-external-worker-family-isolation");
    let process_instance_id = start_timer_wait_process(&engine);
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();

    // Active external-worker family (the only visible one).
    store.insert_timer_job_state_with_type(
        &timer_job(
            "ew-active",
            &process_instance_id,
            Some("timer"),
            "externalTask",
        ),
        Some(&RuntimeJobType::ExternalWorker),
        &mut session,
    );
    // Locked external-worker (still active family).
    let mut locked = timer_job(
        "ew-locked",
        &process_instance_id,
        Some("timer"),
        "externalTask",
    );
    locked.lock_owner = Some("worker-lock".to_string());
    locked.lock_expiration_time = Some(store.time_source().now().timestamp_millis() + 60_000);
    store.insert_timer_job_state_with_type(
        &locked,
        Some(&RuntimeJobType::ExternalWorker),
        &mut session,
    );

    // Non-visible families.
    store.insert_timer_job_state_with_type(
        &timer_job("plain-timer", &process_instance_id, Some("timer"), "timer"),
        Some(&RuntimeJobType::Timer),
        &mut session,
    );
    store.insert_timer_job_state_with_type(
        &timer_job(
            "executable-job",
            &process_instance_id,
            Some("executable"),
            "serviceTask",
        ),
        Some(&RuntimeJobType::Other("message".to_string())),
        &mut session,
    );
    store.insert_timer_job_state_with_type(
        &timer_job(
            "async-job",
            &process_instance_id,
            Some("async"),
            "asyncTask",
        ),
        Some(&RuntimeJobType::Other("message".to_string())),
        &mut session,
    );
    store.insert_timer_job_state_with_type(
        &timer_job(
            "suspended-ew",
            &process_instance_id,
            Some("suspended"),
            "externalTask",
        ),
        Some(&RuntimeJobType::ExternalWorker),
        &mut session,
    );
    store.insert_timer_job_state_with_type(
        &timer_job(
            "deadletter-ew",
            &process_instance_id,
            Some("deadletter"),
            "externalTask",
        ),
        Some(&RuntimeJobType::ExternalWorker),
        &mut session,
    );
    store.insert_timer_job_state_with_type(
        &timer_job(
            "history-job",
            &process_instance_id,
            Some("history"),
            "async-history",
        ),
        Some(&RuntimeJobType::History),
        &mut session,
    );
    store.insert_timer_job_state_with_type(
        &timer_job(
            "definition-suspend-timer",
            "",
            Some("timer"),
            PROCESS_DEFINITION_SUSPEND_TIMER_ACTIVITY_ID,
        ),
        Some(&RuntimeJobType::Timer),
        &mut session,
    );
    // Untyped intermediate timer (event-wait legacy path) must NOT be listed as EW.
    store.insert_timer_job_state(
        &timer_job(
            "untyped-timer",
            &process_instance_id,
            Some("timer"),
            "timer",
        ),
        &mut session,
    );
    session.flush_and_commit().unwrap();

    let (base_url, client) = spawn_server(Arc::clone(&engine)).await;

    let list = client
        .get(format!("{}/external-worker/jobs?start=0&size=50", base_url))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(list.status().is_success());
    let body: Value = list.json().await.unwrap();
    assert_eq!(body["total"], 2);
    let ids: Vec<&str> = body["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|j| j["id"].as_str().unwrap())
        .collect();
    assert!(ids.contains(&"ew-active"));
    assert!(ids.contains(&"ew-locked"));

    let get_ok = client
        .get(format!("{}/external-worker/jobs/ew-active", base_url))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(get_ok.status().is_success());

    for foreign_id in [
        "plain-timer",
        "executable-job",
        "async-job",
        "suspended-ew",
        "deadletter-ew",
        "history-job",
        "definition-suspend-timer",
        "untyped-timer",
        "missing-id",
    ] {
        let response = client
            .get(format!("{}/external-worker/jobs/{}", base_url, foreign_id))
            .basic_auth("admin", Some("test"))
            .send()
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            reqwest::StatusCode::NOT_FOUND,
            "{foreign_id}"
        );
    }

    let locked_only = client
        .get(format!(
            "{}/external-worker/jobs?locked=true&start=0&size=10",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    let locked_body: Value = locked_only.json().await.unwrap();
    assert_eq!(locked_body["total"], 1);
    assert_eq!(locked_body["data"][0]["id"], "ew-locked");

    let unlocked_only = client
        .get(format!(
            "{}/external-worker/jobs?unlocked=true&start=0&size=10",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    let unlocked_body: Value = unlocked_only.json().await.unwrap();
    assert_eq!(unlocked_body["total"], 1);
    assert_eq!(unlocked_body["data"][0]["id"], "ew-active");

    let both = client
        .get(format!(
            "{}/external-worker/jobs?locked=true&unlocked=true",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(both.status(), reqwest::StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn external_worker_list_get_hide_jobs_while_process_suspended_and_restore_on_activate() {
    let (engine, time_source) = build_engine("rest-external-worker-suspend-visibility");
    let process_instance_id = start_timer_wait_process(&engine);
    time_source.advance_time(300_001);

    let (base_url, client) = spawn_server(Arc::clone(&engine)).await;

    let fetch = client
        .post(format!("{}/external-worker/jobs/fetch-and-lock", base_url))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "workerId": "worker-a",
            "maxJobs": 1,
            "lockDurationMs": 60_000
        }))
        .send()
        .await
        .unwrap();
    assert!(fetch.status().is_success());
    let job_id = fetch.json::<Value>().await.unwrap()[0]["id"]
        .as_str()
        .unwrap()
        .to_string();

    // Visible while active.
    let before = client
        .get(format!("{}/external-worker/jobs/{}", base_url, job_id))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(before.status().is_success());

    engine
        .get_runtime_service()
        .suspend_process_instance(
            process_instance_id.clone(),
            ProcessInstanceUpdate::default(),
        )
        .expect("suspend");

    let list_suspended = client
        .get(format!("{}/external-worker/jobs?start=0&size=10", base_url))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    let list_body: Value = list_suspended.json().await.unwrap();
    assert_eq!(list_body["total"], 0);

    let get_suspended = client
        .get(format!("{}/external-worker/jobs/{}", base_url, job_id))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(get_suspended.status(), reqwest::StatusCode::NOT_FOUND);

    let suspended_row = engine
        .get_management_service()
        .find_suspended_job_by_id(&job_id)
        .expect("moved to suspended family");
    assert!(suspended_row.lock_owner.is_none());

    engine
        .get_runtime_service()
        .activate_process_instance(process_instance_id, ProcessInstanceUpdate::default())
        .expect("activate");

    let get_restored = client
        .get(format!("{}/external-worker/jobs/{}", base_url, job_id))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(get_restored.status().is_success());
    let list_restored = client
        .get(format!("{}/external-worker/jobs?start=0&size=10", base_url))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(list_restored.json::<Value>().await.unwrap()["total"], 1);
}
