use chrono::{TimeZone, Utc};
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::engine::time_source::TestTimeSource;
use flowable_engine::identity::entities::{BatchEntity, BatchPartEntity};
use flowable_engine::persistence::runtime_store::RuntimeTimerJobState;
use flowable_engine::runtime::execution::Execution;
use flowable_engine::runtime::process_instance::ProcessInstance;
use flowable_rest::run_server;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::net::TcpListener;

const TIMER_WAIT_PROCESS_BPMN: &str = r#"
<bpmn2:definitions xmlns:bpmn2="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="http://flowable.org/bpmn">
  <bpmn2:process id="rest_management_timer_process" isExecutable="true">
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

fn deploy_timer_wait_process(engine: &ProcessEngine) -> String {
    let repository_service = engine.get_repository_service();
    repository_service
        .deploy(
            repository_service
                .create_deployment()
                .name("rest-management-timer.bpmn20.xml".to_string())
                .add_string(
                    "rest-management-timer.bpmn20.xml".to_string(),
                    TIMER_WAIT_PROCESS_BPMN.to_string(),
                ),
        )
        .unwrap();

    repository_service
        .get_process_definition_ids()
        .unwrap()
        .into_iter()
        .last()
        .unwrap()
}

fn start_timer_wait_process(engine: &ProcessEngine) {
    let process_definition_id = deploy_timer_wait_process(engine);
    engine
        .get_runtime_service()
        .start_process_instance(
            engine
                .get_runtime_service()
                .create_process_instance_builder()
                .process_definition_id(process_definition_id),
        )
        .unwrap();
}

fn timer_job_state_data(engine: &ProcessEngine, job_id: &str) -> Value {
    let runtime_store = engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let mut params = flowable_engine::persistence::DbParams::new();
    params.push(job_id);
    let data: String = session
        .raw_query_one("SELECT data FROM timer_job_states WHERE id = ?1", params)
        .unwrap()
        .and_then(|r| r.get_text("data"))
        .unwrap();
    let _ = session.rollback();
    serde_json::from_str(&data).unwrap()
}

#[tokio::test]
async fn management_paths_expose_engine_properties_tables_and_timer_jobs() {
    let (engine, _time_source) = build_engine("rest-management-native-test");
    let (base_url, client) = spawn_server(Arc::clone(&engine)).await;

    let engine_info = client
        .get(format!("{}/management/engine", base_url))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(engine_info.status().is_success());
    let engine_info_body: Value = engine_info.json().await.unwrap();
    assert_eq!(engine_info_body["name"], "rest-management-native-test");
    assert!(engine_info_body["version"].is_string());

    let engine_name_property = client
        .get(format!(
            "{}/management/engine-properties/engineName",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(engine_name_property.status().is_success());
    let engine_name_property_body: Value = engine_name_property.json().await.unwrap();
    assert_eq!(engine_name_property_body["name"], "engineName");
    assert_eq!(
        engine_name_property_body["value"],
        "rest-management-native-test"
    );

    let properties = client
        .get(format!("{}/management/properties", base_url))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(properties.status().is_success());
    let properties_body: Value = properties.json().await.unwrap();
    assert!(properties_body["engineName"].is_string());
    assert!(properties_body["schemaTableCount"].as_u64().unwrap() > 0);

    let tables = client
        .get(format!("{}/management/tables", base_url))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(tables.status().is_success());
    let tables_body: Value = tables.json().await.unwrap();
    assert!(
        tables_body
            .as_array()
            .unwrap()
            .iter()
            .any(|table| table["name"] == "users")
    );
    let users_table = tables_body
        .as_array()
        .unwrap()
        .iter()
        .find(|table| table["name"] == "users")
        .unwrap();
    assert!(users_table["count"].as_u64().unwrap() > 0);
    assert_eq!(users_table["url"], "/management/tables/users");
    assert!(users_table.get("rowCount").is_none());

    let table = client
        .get(format!("{}/management/tables/users", base_url))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(table.status().is_success());
    let table_body = table.json::<Value>().await.unwrap();
    assert_eq!(table_body["name"], "users");
    assert!(table_body["count"].as_u64().unwrap() > 0);
    assert_eq!(table_body["url"], "/management/tables/users");
    assert!(table_body.get("rowCount").is_none());

    let columns = client
        .get(format!("{}/management/tables/users/columns", base_url))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(columns.status().is_success());
    let columns_body: Value = columns.json().await.unwrap();
    assert_eq!(columns_body["tableName"], "users");
    assert!(
        columns_body["columnNames"]
            .as_array()
            .unwrap()
            .iter()
            .any(|column| column == "id")
    );
    assert_eq!(
        columns_body["columnNames"].as_array().unwrap().len(),
        columns_body["columnTypes"].as_array().unwrap().len()
    );

    let created_property = client
        .post(format!("{}/management/engine-properties", base_url))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "native.custom.property",
            "value": "created"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(created_property.status(), reqwest::StatusCode::CREATED);

    let duplicate_property = client
        .post(format!("{}/management/engine-properties", base_url))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "native.custom.property",
            "value": "duplicate"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(duplicate_property.status(), reqwest::StatusCode::CONFLICT);

    let updated_property = client
        .put(format!(
            "{}/management/engine-properties/native.custom.property",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "value": "updated" }))
        .send()
        .await
        .unwrap();
    assert_eq!(updated_property.status(), reqwest::StatusCode::OK);

    let engine_properties = client
        .get(format!("{}/management/engine-properties", base_url))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(engine_properties.status().is_success());
    let engine_properties_body: Value = engine_properties.json().await.unwrap();
    let custom_property = engine_properties_body
        .as_array()
        .unwrap()
        .iter()
        .find(|property| property["name"] == "native.custom.property")
        .unwrap();
    assert_eq!(custom_property["value"], "updated");
    assert_eq!(custom_property["revision"], 2);
    let engine_name_property = engine_properties_body
        .as_array()
        .unwrap()
        .iter()
        .find(|property| property["name"] == "engineName")
        .unwrap();
    assert_eq!(engine_name_property["value"], "rest-management-native-test");

    let deleted_property = client
        .delete(format!(
            "{}/management/engine-properties/native.custom.property",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(deleted_property.status(), reqwest::StatusCode::NO_CONTENT);

    let missing_property = client
        .get(format!(
            "{}/management/engine-properties/native.custom.property",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(missing_property.status(), reqwest::StatusCode::NOT_FOUND);

    let deleted_again = client
        .delete(format!(
            "{}/management/engine-properties/native.custom.property",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(deleted_again.status(), reqwest::StatusCode::NOT_FOUND);

    let jobs = client
        .get(format!(
            "{}/management/timer-jobs?start=0&size=10",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(jobs.status().is_success());
    let jobs_body: Value = jobs.json().await.unwrap();
    assert_eq!(jobs_body["start"], 0);
    assert!(jobs_body["data"].is_array());
}

#[tokio::test]
async fn management_table_data_matches_sort_and_default_paging_contract() {
    let (engine, _time_source) = build_engine("rest-management-table-data-contract");
    let identity_service = engine.get_identity_service();
    for id in [
        "user-09", "user-08", "user-07", "user-06", "user-05", "user-04", "user-03", "user-02",
        "user-01", "user-00", "user-10",
    ] {
        identity_service.save_user(flowable_engine::identity::entities::User {
            id: id.to_string(),
            first_name: None,
            last_name: None,
            email: None,
            password: None,
            tenant_id: None,
        });
    }

    let (base_url, client) = spawn_server(Arc::clone(&engine)).await;

    let ascending = client
        .get(format!(
            "{}/management/tables/users/data?orderAscendingColumn=id",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(ascending.status().is_success());
    let ascending_body: Value = ascending.json().await.unwrap();
    assert_eq!(ascending_body["start"], 0);
    assert_eq!(ascending_body["size"], 10);
    assert_eq!(ascending_body["total"], 12);
    assert_eq!(ascending_body["data"][0]["id"], "admin");
    assert_eq!(ascending_body["data"][1]["id"], "user-00");
    assert_eq!(ascending_body["data"][9]["id"], "user-08");

    let descending = client
        .get(format!(
            "{}/management/tables/users/data?start=1&size=3&orderDescendingColumn=id",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(descending.status().is_success());
    let descending_body: Value = descending.json().await.unwrap();
    assert_eq!(descending_body["start"], 1);
    assert_eq!(descending_body["size"], 3);
    assert_eq!(descending_body["sort"], "id");
    assert_eq!(descending_body["order"], "desc");
    assert_eq!(descending_body["data"][0]["id"], "user-09");
    assert_eq!(descending_body["data"][2]["id"], "user-07");

    let conflicting_sort = client
        .get(format!(
            "{}{}",
            base_url,
            "/management/tables/users/data?orderAscendingColumn=id&orderDescendingColumn=id"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(conflicting_sort.status(), reqwest::StatusCode::BAD_REQUEST);
    let conflicting_sort_body: Value = conflicting_sort.json().await.unwrap();
    assert_eq!(conflicting_sort_body["code"], "BAD_REQUEST");
    assert!(
        conflicting_sort_body["details"].as_str().unwrap().contains(
            "Only one of 'orderAscendingColumn' or 'orderDescendingColumn' can be supplied"
        )
    );
}

#[tokio::test]
async fn management_executable_jobs_use_a_distinct_job_family() {
    let (engine, _time_source) = build_engine("rest-management-executable-jobs");
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    store.insert_timer_job_state(
        &RuntimeTimerJobState {
            timer_job_id: "exec-job-1".to_string(),
            process_instance_id: "process-1".to_string(),
            execution_id: "execution-1".to_string(),
            activity_id: "timer-activity".to_string(),
            job_state: Some("executable".to_string()),
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
        },
        &mut session,
    );
    store.insert_timer_job_state(
        &RuntimeTimerJobState {
            timer_job_id: "deadletter-job-1".to_string(),
            process_instance_id: "process-1".to_string(),
            execution_id: "execution-dead".to_string(),
            activity_id: "dead-activity".to_string(),
            job_state: Some("deadletter".to_string()),
            is_boundary: false,
            attached_activity_id: None,
            cancel_activity: false,
            time_duration: None,
            time_date: None,
            time_cycle: None,
            due_time: Some(1_775_000_001_000),
            lock_owner: None,
            lock_time: None,
            lock_expiration_time: None,
            retries: Some(0),
            error_message: Some("failed".to_string()),
            error_details: None,
            category: None,
            ..Default::default()
        },
        &mut session,
    );
    session.flush_and_commit().unwrap();
    let (base_url, client) = spawn_server(Arc::clone(&engine)).await;

    let jobs = client
        .get(format!(
            "{}/management/jobs?processInstanceId=process-1",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(jobs.status().is_success());
    let jobs_body: Value = jobs.json().await.unwrap();
    assert_eq!(jobs_body["total"], 1);
    assert_eq!(jobs_body["data"][0]["id"], "exec-job-1");
    assert_eq!(jobs_body["data"][0]["jobType"], "executable");

    let filtered_jobs = client
        .get(format!(
            "{}/management/jobs?executionId=execution-1",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(filtered_jobs.status().is_success());
    assert_eq!(filtered_jobs.json::<Value>().await.unwrap()["total"], 1);

    let job = client
        .get(format!("{}/management/jobs/exec-job-1", base_url))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(job.status().is_success());
    assert_eq!(job.json::<Value>().await.unwrap()["id"], "exec-job-1");

    let deadletter_via_executable = client
        .get(format!("{}/management/jobs/deadletter-job-1", base_url))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(deadletter_via_executable.status(), 404);
}

#[tokio::test]
async fn management_jobs_expose_common_fields_and_query_flags() {
    let (engine, _time_source) = build_engine("rest-management-job-fields");
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    store.insert_process_instance(
        &ProcessInstance {
            id: "process-fields".to_string(),
            name: None,
            process_definition_id: "process-definition-fields".to_string(),
            process_definition_key: "processFields".to_string(),
            process_definition_name: None,
            process_definition_version: 1,
            business_key: None,
            business_status: None,
            is_suspended: false,
            tenant_id: Some("tenant-a".to_string()),
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
    store.insert_execution(
        &Execution {
            id: "execution-fields".to_string(),
            process_instance_id: Some("process-fields".to_string()),
            process_definition_id: Some("process-definition-fields".to_string()),
            tenant_id: Some("tenant-a".to_string()),
            activity_id: Some("timer-activity".to_string()),
            activity_name: Some("Timer activity".to_string()),
            ..Default::default()
        },
        &mut session,
    );

    for (id, state, retries) in [
        ("field-job", Some("executable"), Some(2)),
        ("message-job", Some("async"), Some(1)),
        ("no-retries-job", Some("executable"), Some(0)),
    ] {
        store.insert_timer_job_state(
            &RuntimeTimerJobState {
                timer_job_id: id.to_string(),
                process_instance_id: "process-fields".to_string(),
                execution_id: "execution-fields".to_string(),
                activity_id: "timer-activity".to_string(),
                job_state: state.map(str::to_string),
                is_boundary: false,
                attached_activity_id: None,
                cancel_activity: false,
                time_duration: None,
                time_date: None,
                time_cycle: None,
                due_time: Some(1_775_000_000_000),
                lock_owner: Some("worker-a".to_string()),
                lock_time: Some(1_775_000_001_000),
                lock_expiration_time: Some(1_775_000_060_000),
                retries,
                error_message: None,
                error_details: None,
                category: None,
                ..Default::default()
            },
            &mut session,
        );
    }
    session.flush_and_commit().unwrap();

    let (base_url, client) = spawn_server(Arc::clone(&engine)).await;

    let job = client
        .get(format!("{}/management/jobs/field-job", base_url))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(job.status().is_success());
    let job_body: Value = job.json().await.unwrap();
    assert_eq!(job_body["id"], "field-job");
    assert_eq!(job_body["url"], "/management/jobs/field-job");
    assert_eq!(job_body["processInstanceId"], "process-fields");
    assert_eq!(
        job_body["processInstanceUrl"],
        "/runtime/process-instances/process-fields"
    );
    assert_eq!(job_body["processDefinitionId"], "process-definition-fields");
    assert_eq!(
        job_body["processDefinitionUrl"],
        "/repository/process-definitions/process-definition-fields"
    );
    assert_eq!(job_body["executionId"], "execution-fields");
    assert_eq!(
        job_body["executionUrl"],
        "/runtime/executions/execution-fields"
    );
    assert_eq!(job_body["elementId"], "timer-activity");
    assert_eq!(job_body["elementName"], "Timer activity");
    assert_eq!(job_body["handlerType"], "timer");
    assert_eq!(job_body["dueDate"], "2026-03-31T23:33:20+00:00");
    assert_eq!(job_body["lockOwner"], "worker-a");
    assert_eq!(job_body["lockExpirationTime"], "2026-03-31T23:34:20+00:00");
    assert_eq!(job_body["tenantId"], "tenant-a");

    let filtered = client
        .get(format!(
            "{}{}",
            base_url,
            "/management/jobs?processDefinitionId=process-definition-fields&withRetriesLeft=true&executable=true&sort=id&order=asc"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(filtered.status().is_success());
    let filtered_body: Value = filtered.json().await.unwrap();
    assert_eq!(filtered_body["total"], 2);
    assert_eq!(filtered_body["data"][0]["id"], "field-job");
    assert_eq!(filtered_body["data"][1]["id"], "message-job");

    let timers_only = client
        .get(format!("{}/management/jobs?timersOnly=true", base_url))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(timers_only.status().is_success());
    let timers_only_body: Value = timers_only.json().await.unwrap();
    assert_eq!(timers_only_body["total"], 2);
    let timer_ids = timers_only_body["data"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|job| job["id"].as_str())
        .collect::<Vec<_>>();
    assert!(timer_ids.contains(&"field-job"));
    assert!(timer_ids.contains(&"no-retries-job"));

    let messages_only = client
        .get(format!("{}/management/jobs?messagesOnly=true", base_url))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(messages_only.status().is_success());
    let messages_only_body: Value = messages_only.json().await.unwrap();
    assert_eq!(messages_only_body["total"], 1);
    assert_eq!(messages_only_body["data"][0]["id"], "message-job");

    let conflicting_type_filter = client
        .get(format!(
            "{}/management/jobs?timersOnly=true&messagesOnly=true",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(
        conflicting_type_filter.status(),
        reqwest::StatusCode::BAD_REQUEST
    );
}

#[tokio::test]
async fn management_deadletter_timer_jobs_and_exception_stacktrace_are_backed_by_failed_timer_state()
 {
    let (engine, time_source) = build_engine("rest-management-deadletter");
    let (base_url, client) = spawn_server(Arc::clone(&engine)).await;
    start_timer_wait_process(&engine);
    time_source.advance_time(300_001);

    let acquire = client
        .post(format!("{}/external-worker/jobs/fetch-and-lock", base_url))
        .basic_auth("admin", Some("test"))
        // P68: omit topic for legacy timer-backed EW candidates (no job_handler_configuration).
        .json(&serde_json::json!({
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

    let fail = client
        .post(format!(
            "{}/external-worker/jobs/{}/failure",
            base_url, job_id
        ))
        .basic_auth("admin", Some("test"))
        .json(&serde_json::json!({
            "workerId": "worker-a",
            "errorMessage": "worker failed permanently",
            "errorDetails": "stacktrace line 1\nstacktrace line 2",
            "retries": 0,
            "retryTimeout": "PT45S"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(fail.status(), reqwest::StatusCode::NO_CONTENT);

    let deadletters = client
        .get(format!(
            "{}/management/deadletter-jobs?start=0&size=10",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(deadletters.status().is_success());
    let deadletters_body: Value = deadletters.json().await.unwrap();
    assert_eq!(deadletters_body["total"], 1);
    assert_eq!(deadletters_body["data"][0]["id"], job_id);
    assert_eq!(
        deadletters_body["data"][0]["exceptionMessage"],
        "worker failed permanently"
    );
    assert_eq!(deadletters_body["data"][0]["retries"], 0);

    let deadletter = client
        .get(format!(
            "{}/management/deadletter-jobs/{}",
            base_url, job_id
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(deadletter.status().is_success());
    assert_eq!(deadletter.json::<Value>().await.unwrap()["id"], job_id);

    let stacktrace = client
        .get(format!(
            "{}/management/deadletter-jobs/{}/exception-stacktrace",
            base_url, job_id
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(stacktrace.status().is_success());
    assert_eq!(
        stacktrace
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .unwrap()
            .to_str()
            .unwrap(),
        "text/plain"
    );
    assert_eq!(
        stacktrace.text().await.unwrap(),
        "stacktrace line 1\nstacktrace line 2"
    );

    let job_stacktrace = client
        .get(format!(
            "{}/management/jobs/{}/exception-stacktrace",
            base_url, job_id
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    // Java JobBaseResource.getJobById only queries the executable job table:
    // a deadletter job id must not leak a stacktrace through the jobs family.
    assert_eq!(job_stacktrace.status(), reqwest::StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn management_history_and_suspended_job_paths_filter_real_persisted_job_states() {
    let (engine, _time_source) = build_engine("rest-management-job-state-contract");
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    store.insert_timer_job_state(
        &RuntimeTimerJobState {
            timer_job_id: "history-job-1".to_string(),
            process_instance_id: "process-1".to_string(),
            execution_id: "execution-1".to_string(),
            activity_id: "history-activity".to_string(),
            job_state: Some("history".to_string()),
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
        },
        &mut session,
    );
    store.insert_timer_job_state(
        &RuntimeTimerJobState {
            timer_job_id: "suspended-job-1".to_string(),
            process_instance_id: "process-2".to_string(),
            execution_id: "execution-2".to_string(),
            activity_id: "suspended-activity".to_string(),
            job_state: Some("suspended".to_string()),
            is_boundary: false,
            attached_activity_id: None,
            cancel_activity: false,
            time_duration: None,
            time_date: None,
            time_cycle: None,
            due_time: Some(1_775_000_001_000),
            lock_owner: None,
            lock_time: None,
            lock_expiration_time: None,
            retries: Some(1),
            error_message: Some("suspended failure".to_string()),
            error_details: Some(
                "suspended stacktrace line 1\nsuspended stacktrace line 2".to_string(),
            ),
            category: None,
            ..Default::default()
        },
        &mut session,
    );
    session.flush_and_commit().unwrap();

    let (base_url, client) = spawn_server(Arc::clone(&engine)).await;

    let history_jobs = client
        .get(format!(
            "{}/management/history-jobs?start=0&size=10",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(history_jobs.status().is_success());
    let history_jobs_body: Value = history_jobs.json().await.unwrap();
    assert_eq!(history_jobs_body["total"], 1);
    assert_eq!(history_jobs_body["data"][0]["id"], "history-job-1");
    // Java HistoryJobResponse shape (RestResponseFactory.createHistoryJobResponse).
    assert_eq!(history_jobs_body["data"][0]["jobHandlerType"], "history");
    assert_eq!(history_jobs_body["data"][0]["retries"], 1);
    assert_eq!(
        history_jobs_body["data"][0]["url"],
        "/management/history-jobs/history-job-1"
    );
    assert!(history_jobs_body["data"][0].get("jobType").is_none());

    let history_job = client
        .get(format!(
            "{}/management/history-jobs/history-job-1",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(history_job.status().is_success());
    assert_eq!(
        history_job.json::<Value>().await.unwrap()["id"],
        "history-job-1"
    );

    let missing_history_job = client
        .get(format!(
            "{}/management/history-jobs/missing-history-job",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(missing_history_job.status(), reqwest::StatusCode::NOT_FOUND);
    let missing_history_body: Value = missing_history_job.json().await.unwrap();
    assert_eq!(missing_history_body["code"], "NOT_FOUND");

    let suspended_jobs = client
        .get(format!(
            "{}/management/suspended-jobs?start=0&size=10",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(suspended_jobs.status().is_success());
    let suspended_jobs_body: Value = suspended_jobs.json().await.unwrap();
    assert_eq!(suspended_jobs_body["total"], 1);
    assert_eq!(suspended_jobs_body["data"][0]["id"], "suspended-job-1");
    assert_eq!(suspended_jobs_body["data"][0]["jobType"], "suspended");
    assert_eq!(
        suspended_jobs_body["data"][0]["exceptionMessage"],
        "suspended failure"
    );

    let suspended_job = client
        .get(format!(
            "{}/management/suspended-jobs/suspended-job-1",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(suspended_job.status().is_success());
    assert_eq!(
        suspended_job.json::<Value>().await.unwrap()["id"],
        "suspended-job-1"
    );

    let stacktrace = client
        .get(format!(
            "{}/management/suspended-jobs/suspended-job-1/exception-stacktrace",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(stacktrace.status().is_success());
    assert_eq!(
        stacktrace.text().await.unwrap(),
        "suspended stacktrace line 1\nsuspended stacktrace line 2"
    );

    let missing_stacktrace = client
        .get(format!(
            "{}/management/suspended-jobs/missing-suspended-job/exception-stacktrace",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(missing_stacktrace.status(), reqwest::StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn management_history_job_post_matches_execute_only_contract() {
    let (engine, _time_source) = build_engine("rest-management-history-post-contract");
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    store.insert_timer_job_state(
        &RuntimeTimerJobState {
            timer_job_id: "history-post-contract".to_string(),
            process_instance_id: "process-history-post".to_string(),
            execution_id: "execution-history-post".to_string(),
            activity_id: "history-activity".to_string(),
            job_state: Some("history".to_string()),
            is_boundary: false,
            attached_activity_id: None,
            cancel_activity: false,
            time_duration: None,
            time_date: None,
            time_cycle: None,
            due_time: Some(1_775_000_000_000),
            lock_owner: Some("history-worker".to_string()),
            lock_time: Some(10),
            lock_expiration_time: Some(20),
            retries: Some(7),
            error_message: Some("history failure".to_string()),
            error_details: Some("history stacktrace".to_string()),
            category: None,
            ..Default::default()
        },
        &mut session,
    );
    session.flush_and_commit().unwrap();

    let (base_url, client) = spawn_server(Arc::clone(&engine)).await;

    let missing_action = client
        .post(format!(
            "{}/management/history-jobs/history-post-contract",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "retries": 3 }))
        .send()
        .await
        .unwrap();
    assert_eq!(missing_action.status(), reqwest::StatusCode::BAD_REQUEST);
    let missing_action_body: Value = missing_action.json().await.unwrap();
    assert_eq!(missing_action_body["code"], "BAD_REQUEST");
    assert!(
        missing_action_body["details"]
            .as_str()
            .unwrap()
            .contains("Field 'action' is required")
    );

    for action in [
        "move",
        "moveToExecutableJob",
        "moveToHistoryJob",
        "setRetries",
    ] {
        let unsupported = client
            .post(format!(
                "{}/management/history-jobs/history-post-contract",
                base_url
            ))
            .basic_auth("admin", Some("test"))
            .json(&json!({ "action": action, "retries": 3 }))
            .send()
            .await
            .unwrap();
        assert_eq!(unsupported.status(), reqwest::StatusCode::BAD_REQUEST);
        let unsupported_body: Value = unsupported.json().await.unwrap();
        assert_eq!(unsupported_body["code"], "BAD_REQUEST");
        assert!(
            unsupported_body["details"]
                .as_str()
                .unwrap()
                .contains("Supported actions: execute")
        );
    }

    let unchanged_history = engine
        .get_management_service()
        .find_history_job_by_id("history-post-contract")
        .unwrap();
    assert_eq!(unchanged_history.retries, Some(7));
    assert_eq!(
        unchanged_history.lock_owner.as_deref(),
        Some("history-worker")
    );
    assert!(
        engine
            .get_management_service()
            .find_executable_job_by_id("history-post-contract")
            .is_none()
    );
    assert!(
        engine
            .get_management_service()
            .find_deadletter_job_by_id("history-post-contract")
            .is_none()
    );
}

#[tokio::test]
async fn management_job_mutations_delete_and_reject_unsupported_actions() {
    let (engine, _time_source) = build_engine("rest-management-job-delete-mutations");
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    for (id, state, retries) in [
        ("exec-delete-job", Some("executable"), Some(1)),
        ("timer-delete-job", Some("timer"), Some(1)),
        ("deadletter-delete-job", Some("deadletter"), Some(0)),
        ("history-delete-job", Some("history"), Some(1)),
        ("suspended-delete-job", Some("suspended"), Some(1)),
    ] {
        store.insert_timer_job_state(
            &RuntimeTimerJobState {
                timer_job_id: id.to_string(),
                process_instance_id: "process-delete".to_string(),
                execution_id: format!("execution-{id}"),
                activity_id: "activity-delete".to_string(),
                job_state: state.map(str::to_string),
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
                retries,
                error_message: None,
                error_details: None,
                category: None,
                ..Default::default()
            },
            &mut session,
        );
    }
    session.flush_and_commit().unwrap();

    let (base_url, client) = spawn_server(Arc::clone(&engine)).await;
    let wrong_family = client
        .delete(format!(
            "{}/management/suspended-jobs/exec-delete-job",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(wrong_family.status(), reqwest::StatusCode::NOT_FOUND);
    assert!(
        engine
            .get_management_service()
            .find_executable_job_by_id("exec-delete-job")
            .is_some()
    );

    for (path, id) in [
        ("jobs", "exec-delete-job"),
        ("timer-jobs", "timer-delete-job"),
        ("deadletter-jobs", "deadletter-delete-job"),
        ("history-jobs", "history-delete-job"),
        ("suspended-jobs", "suspended-delete-job"),
    ] {
        let deleted = client
            .delete(format!("{}/management/{}/{}", base_url, path, id))
            .basic_auth("admin", Some("test"))
            .send()
            .await
            .unwrap();
        assert_eq!(deleted.status(), reqwest::StatusCode::NO_CONTENT);

        let missing = client
            .get(format!("{}/management/{}/{}", base_url, path, id))
            .basic_auth("admin", Some("test"))
            .send()
            .await
            .unwrap();
        assert_eq!(missing.status(), reqwest::StatusCode::NOT_FOUND);
    }

    let unsupported = client
        .post(format!("{}/management/timer-jobs/missing", base_url))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "action": "unknownAction" }))
        .send()
        .await
        .unwrap();
    assert_eq!(unsupported.status(), reqwest::StatusCode::BAD_REQUEST);
    let body: Value = unsupported.json().await.unwrap();
    assert_eq!(body["code"], "BAD_REQUEST");
    assert!(
        body["details"]
            .as_str()
            .unwrap()
            .contains("Supported actions")
    );
}

#[tokio::test]
async fn management_job_post_actions_execute_move_retry_and_reschedule_real_state() {
    let (engine, _time_source) = build_engine("rest-management-job-post-mutations");
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    store.insert_timer_job_state(
        &RuntimeTimerJobState {
            timer_job_id: "exec-move-deadletter".to_string(),
            process_instance_id: "process-1".to_string(),
            execution_id: "execution-exec-move".to_string(),
            activity_id: "activity-exec".to_string(),
            job_state: Some("timer".to_string()),
            is_boundary: false,
            attached_activity_id: None,
            cancel_activity: false,
            time_duration: None,
            time_date: None,
            time_cycle: None,
            due_time: Some(1_775_000_000_000),
            lock_owner: Some("old-worker".to_string()),
            lock_time: Some(1),
            lock_expiration_time: Some(2),
            retries: Some(2),
            error_message: None,
            error_details: None,
            category: None,
            ..Default::default()
        },
        &mut session,
    );
    store.insert_timer_job_state(
        &RuntimeTimerJobState {
            timer_job_id: "timer-reschedule".to_string(),
            process_instance_id: "process-1".to_string(),
            execution_id: "execution-timer-reschedule".to_string(),
            activity_id: "activity-timer".to_string(),
            job_state: Some("timer".to_string()),
            is_boundary: false,
            attached_activity_id: None,
            cancel_activity: false,
            time_duration: Some("PT5M".to_string()),
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
        },
        &mut session,
    );
    store.insert_timer_job_state(
        &RuntimeTimerJobState {
            timer_job_id: "deadletter-move-exec".to_string(),
            process_instance_id: "process-1".to_string(),
            execution_id: "execution-deadletter-move".to_string(),
            activity_id: "activity-deadletter".to_string(),
            job_state: Some("deadletter".to_string()),
            is_boundary: false,
            attached_activity_id: None,
            cancel_activity: false,
            time_duration: None,
            time_date: None,
            time_cycle: None,
            due_time: Some(1_775_000_000_000),
            lock_owner: Some("old-worker".to_string()),
            lock_time: Some(1),
            lock_expiration_time: Some(2),
            retries: Some(0),
            error_message: Some("failed".to_string()),
            error_details: Some("stacktrace".to_string()),
            category: None,
            ..Default::default()
        },
        &mut session,
    );
    store.insert_timer_job_state(
        &RuntimeTimerJobState {
            timer_job_id: "history-execute".to_string(),
            process_instance_id: "process-1".to_string(),
            execution_id: "execution-history-execute".to_string(),
            activity_id: "activity-history".to_string(),
            job_state: Some("history".to_string()),
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
        },
        &mut session,
    );
    session.flush_and_commit().unwrap();

    let (base_url, client) = spawn_server(Arc::clone(&engine)).await;

    let moved_to_deadletter = client
        .post(format!("{}/management/jobs/exec-move-deadletter", base_url))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "action": "move" }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        moved_to_deadletter.status(),
        reqwest::StatusCode::NO_CONTENT
    );
    assert!(
        engine
            .get_management_service()
            .find_deadletter_job_by_id("exec-move-deadletter")
            .is_some()
    );

    let rescheduled = client
        .post(format!(
            "{}/management/timer-jobs/timer-reschedule",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "action": "reschedule",
            "timeDate": "2026-05-01T00:00:00Z"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(rescheduled.status(), reqwest::StatusCode::NO_CONTENT);
    let mut session = store.create_session().unwrap();
    let rescheduled_state = store
        .find_timer_job_state("timer-reschedule", &mut session)
        .unwrap();
    let _ = session.rollback();
    assert_eq!(rescheduled_state.time_date.unwrap(), "2026-05-01T00:00:00Z");
    assert_eq!(rescheduled_state.due_time, Some(1_777_593_600_000));

    let moved_to_exec = client
        .post(format!(
            "{}/management/deadletter-jobs/deadletter-move-exec",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "action": "move", "retries": 4 }))
        .send()
        .await
        .unwrap();
    assert_eq!(moved_to_exec.status(), reqwest::StatusCode::NO_CONTENT);
    let moved_state = engine
        .get_management_service()
        .find_executable_job_by_id("deadletter-move-exec")
        .unwrap();
    assert_eq!(moved_state.retries, Some(4));
    assert!(moved_state.lock_owner.is_none());

    let executed_history = client
        .post(format!(
            "{}/management/history-jobs/history-execute",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "action": "execute" }))
        .send()
        .await
        .unwrap();
    assert_eq!(executed_history.status(), reqwest::StatusCode::NO_CONTENT);
    assert!(
        engine
            .get_management_service()
            .find_history_job_by_id("history-execute")
            .is_none()
    );

    let mut session = store.create_session().unwrap();
    store.insert_timer_job_state(
        &RuntimeTimerJobState {
            timer_job_id: "history-unsupported".to_string(),
            process_instance_id: "process-1".to_string(),
            execution_id: "execution-history-unsupported".to_string(),
            activity_id: "activity-history".to_string(),
            job_state: Some("history".to_string()),
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
        },
        &mut session,
    );
    session.flush_and_commit().unwrap();
    let unsupported_history_action = client
        .post(format!(
            "{}/management/history-jobs/history-unsupported",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "action": "retry" }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        unsupported_history_action.status(),
        reqwest::StatusCode::BAD_REQUEST
    );
    let unsupported_history_body: Value = unsupported_history_action.json().await.unwrap();
    assert_eq!(unsupported_history_body["code"], "BAD_REQUEST");
    assert!(
        unsupported_history_body["details"]
            .as_str()
            .unwrap()
            .contains("Supported actions: execute")
    );
}

#[tokio::test]
async fn management_job_post_actions_accept_aliases_and_persist_fields() {
    let (engine, _time_source) = build_engine("rest-management-job-post-aliases");
    for (id, state, retries, error_message, error_details) in [
        (
            "exec-move-deadletter-fields",
            Some("executable"),
            Some(3),
            None,
            None,
        ),
        ("exec-retry-alias", Some("executable"), Some(1), None, None),
        (
            "deadletter-retry-alias",
            Some("deadletter"),
            Some(0),
            Some("failed before retry"),
            Some("stacktrace before retry"),
        ),
        (
            "suspended-move-exec",
            Some("suspended"),
            Some(0),
            Some("suspended failure"),
            Some("suspended stacktrace"),
        ),
    ] {
        let store = engine.get_runtime_store();
        let mut session = store.create_session().unwrap();
        store.insert_timer_job_state(
            &RuntimeTimerJobState {
                timer_job_id: id.to_string(),
                process_instance_id: "process-aliases".to_string(),
                execution_id: format!("execution-{id}"),
                activity_id: "activity-alias".to_string(),
                job_state: state.map(str::to_string),
                is_boundary: false,
                attached_activity_id: None,
                cancel_activity: false,
                time_duration: None,
                time_date: None,
                time_cycle: None,
                due_time: Some(1_775_000_000_000),
                lock_owner: Some("old-worker".to_string()),
                lock_time: Some(1),
                lock_expiration_time: Some(2),
                retries,
                error_message: error_message.map(str::to_string),
                error_details: error_details.map(str::to_string),
                category: None,
                ..Default::default()
            },
            &mut session,
        );
        session.flush_and_commit().unwrap();
    }

    let (base_url, client) = spawn_server(Arc::clone(&engine)).await;

    let moved_to_deadletter = client
        .post(format!(
            "{}/management/jobs/exec-move-deadletter-fields",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "action": "moveToDeadLetterJob",
            "exceptionMessage": "manual deadletter",
            "deleteReason": "operator requested quarantine"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        moved_to_deadletter.status(),
        reqwest::StatusCode::NO_CONTENT
    );
    let deadletter_state = engine
        .get_management_service()
        .find_deadletter_job_by_id("exec-move-deadletter-fields")
        .unwrap();
    assert_eq!(
        deadletter_state.error_message.as_deref(),
        Some("manual deadletter")
    );
    assert!(deadletter_state.lock_owner.is_none());
    let deadletter_data = timer_job_state_data(&engine, "exec-move-deadletter-fields");
    assert_eq!(
        deadletter_data["deleteReason"],
        "operator requested quarantine"
    );

    let set_retries_alias = client
        .post(format!("{}/management/jobs/exec-retry-alias", base_url))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "action": "retry", "retries": 5 }))
        .send()
        .await
        .unwrap();
    assert_eq!(set_retries_alias.status(), reqwest::StatusCode::NO_CONTENT);
    let retried_exec = engine
        .get_management_service()
        .find_executable_job_by_id("exec-retry-alias")
        .unwrap();
    assert_eq!(retried_exec.retries, Some(5));

    let moved_deadletter_alias = client
        .post(format!(
            "{}/management/deadletter-jobs/deadletter-retry-alias",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "action": "moveToExecutableJob", "retries": 6 }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        moved_deadletter_alias.status(),
        reqwest::StatusCode::NO_CONTENT
    );
    let moved_deadletter_state = engine
        .get_management_service()
        .find_executable_job_by_id("deadletter-retry-alias")
        .unwrap();
    assert_eq!(moved_deadletter_state.retries, Some(6));
    assert!(moved_deadletter_state.lock_owner.is_none());
    assert!(
        engine
            .get_management_service()
            .find_deadletter_job_by_id("deadletter-retry-alias")
            .is_none()
    );

    let moved_suspended = client
        .post(format!(
            "{}/management/suspended-jobs/suspended-move-exec",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "action": "moveToExecutableJob", "retries": 2 }))
        .send()
        .await
        .unwrap();
    assert_eq!(moved_suspended.status(), reqwest::StatusCode::NO_CONTENT);

    let executable_jobs = client
        .get(format!(
            "{}/management/jobs?processInstanceId=process-aliases",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(executable_jobs.status().is_success());
    let executable_jobs_body: Value = executable_jobs.json().await.unwrap();
    let executable_ids = executable_jobs_body["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|job| job["id"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert!(executable_ids.contains(&"suspended-move-exec"));
    let moved_suspended_state = engine
        .get_management_service()
        .find_executable_job_by_id("suspended-move-exec")
        .unwrap();
    // Java moveSuspendedJobToExecutableJob preserves the retry count unchanged.
    assert_eq!(moved_suspended_state.retries, Some(0));
    assert!(moved_suspended_state.lock_owner.is_none());
    assert!(
        engine
            .get_management_service()
            .find_suspended_job_by_id("suspended-move-exec")
            .is_none()
    );

    let unsupported_history_action = client
        .post(format!(
            "{}/management/history-jobs/history-missing",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "action": "moveToExecutableJob", "retries": 1 }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        unsupported_history_action.status(),
        reqwest::StatusCode::BAD_REQUEST
    );
}

#[tokio::test]
async fn management_jobs_accept_exact_history_move_due_date_and_query_sort_filters() {
    let (engine, _time_source) = build_engine("rest-management-exact-actions-and-query-fields");
    let due_early = Utc
        .with_ymd_and_hms(2026, 4, 21, 12, 0, 1)
        .unwrap()
        .timestamp_millis();
    let due_middle = Utc
        .with_ymd_and_hms(2026, 4, 21, 12, 0, 2)
        .unwrap()
        .timestamp_millis();
    let due_late = Utc
        .with_ymd_and_hms(2026, 4, 21, 12, 0, 3)
        .unwrap()
        .timestamp_millis();

    for (id, state, due_time, retries, error_message) in [
        (
            "exec-filter-alpha",
            Some("executable"),
            Some(due_early),
            Some(2),
            Some("alpha failure"),
        ),
        (
            "exec-filter-beta",
            Some("executable"),
            Some(due_late),
            Some(5),
            Some("beta failure"),
        ),
        (
            "exec-filter-clean",
            Some("executable"),
            Some(due_middle),
            Some(1),
            None,
        ),
        (
            "deadletter-move-history-exact",
            Some("deadletter"),
            Some(due_middle),
            Some(0),
            Some("deadletter failure"),
        ),
        (
            "timer-reschedule-due-date-alias",
            Some("timer"),
            Some(due_middle),
            Some(1),
            None,
        ),
    ] {
        let store = engine.get_runtime_store();
        let mut session = store.create_session().unwrap();
        store.insert_timer_job_state(
            &RuntimeTimerJobState {
                timer_job_id: id.to_string(),
                process_instance_id: "process-query".to_string(),
                execution_id: format!("execution-{id}"),
                activity_id: if id == "deadletter-move-history-exact" {
                    "async-history"
                } else {
                    "activity-query"
                }
                .to_string(),
                job_state: state.map(str::to_string),
                is_boundary: false,
                attached_activity_id: None,
                cancel_activity: false,
                time_duration: None,
                time_date: None,
                time_cycle: None,
                due_time,
                lock_owner: Some("worker-before-action".to_string()),
                lock_time: Some(1),
                lock_expiration_time: Some(2),
                retries,
                error_message: error_message.map(str::to_string),
                error_details: None,
                category: None,
                ..Default::default()
            },
            &mut session,
        );
        session.flush_and_commit().unwrap();
    }

    let (base_url, client) = spawn_server(Arc::clone(&engine)).await;

    let moved_to_history = client
        .post(format!(
            "{}/management/deadletter-jobs/deadletter-move-history-exact",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "action": "moveToHistoryJob", "retries": 3 }))
        .send()
        .await
        .unwrap();
    assert_eq!(moved_to_history.status(), reqwest::StatusCode::NO_CONTENT);
    let moved_history_state = engine
        .get_management_service()
        .find_history_job_by_id("deadletter-move-history-exact")
        .unwrap();
    assert_eq!(moved_history_state.retries, Some(3));
    assert!(moved_history_state.lock_owner.is_none());

    let rescheduled_with_due_date = client
        .post(format!(
            "{}/management/timer-jobs/timer-reschedule-due-date-alias",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "action": "reschedule",
            "dueDate": "2026-05-03T00:00:00Z"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        rescheduled_with_due_date.status(),
        reqwest::StatusCode::NO_CONTENT
    );
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let rescheduled_state = store
        .find_timer_job_state("timer-reschedule-due-date-alias", &mut session)
        .unwrap();
    let _ = session.rollback();
    assert_eq!(
        rescheduled_state.time_date.as_deref(),
        Some("2026-05-03T00:00:00Z")
    );
    assert_eq!(rescheduled_state.due_time, Some(1_777_766_400_000));

    let sorted_filtered = client
        .get(format!(
            "{}{}",
            base_url,
            "/management/jobs?processInstanceId=process-query&withException=true&dueBefore=2026-04-21T12%3A00%3A04Z&sort=dueDate&order=desc"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(sorted_filtered.status().is_success());
    let sorted_filtered_body: Value = sorted_filtered.json().await.unwrap();
    assert_eq!(sorted_filtered_body["total"], 2);
    assert_eq!(sorted_filtered_body["data"][0]["id"], "exec-filter-beta");
    assert_eq!(sorted_filtered_body["data"][1]["id"], "exec-filter-alpha");

    let exception_message_filtered = client
        .get(format!(
            "{}/management/jobs?exceptionMessage=alpha%20failure",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(exception_message_filtered.status().is_success());
    let exception_message_body: Value = exception_message_filtered.json().await.unwrap();
    assert_eq!(exception_message_body["total"], 1);
    assert_eq!(exception_message_body["data"][0]["id"], "exec-filter-alpha");

    let tenant_sort = client
        .get(format!("{}/management/jobs?sort=tenantId", base_url))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(tenant_sort.status().is_success());
    assert_eq!(tenant_sort.json::<Value>().await.unwrap()["total"], 3);
}

#[tokio::test]
async fn management_timer_reschedule_accepts_end_date_and_calendar_name() {
    // P64: calendarName is looked up in the engine-local registry and drives the
    // immediate due date. The built-in `cycle` calendar yields now+10m for R3/PT10M
    // (TestTimeSource is fixed at 2026-04-21T12:00:00Z → due 12:10:00Z).
    let (engine, _time_source) = build_engine("rest-management-timer-reschedule-fields");
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    store.insert_timer_job_state(
        &RuntimeTimerJobState {
            timer_job_id: "timer-reschedule-fields".to_string(),
            process_instance_id: "process-1".to_string(),
            execution_id: "execution-timer-reschedule-fields".to_string(),
            activity_id: "activity-timer".to_string(),
            job_state: Some("timer".to_string()),
            is_boundary: false,
            attached_activity_id: None,
            cancel_activity: false,
            time_duration: Some("PT5M".to_string()),
            time_date: None,
            time_cycle: None,
            due_time: Some(1_775_000_000_000),
            lock_owner: Some("old-worker".to_string()),
            lock_time: Some(1),
            lock_expiration_time: Some(2),
            retries: Some(1),
            error_message: None,
            error_details: None,
            category: None,
            ..Default::default()
        },
        &mut session,
    );
    session.flush_and_commit().unwrap();

    let (base_url, client) = spawn_server(Arc::clone(&engine)).await;

    let rescheduled = client
        .post(format!(
            "{}/management/timer-jobs/timer-reschedule-fields",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "action": "reschedule",
            "timeCycle": "R3/PT10M",
            "endDate": "2026-05-02T00:00:00Z",
            "calendarName": "cycle"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(rescheduled.status(), reqwest::StatusCode::NO_CONTENT);
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let rescheduled_state = store
        .find_timer_job_state("timer-reschedule-fields", &mut session)
        .unwrap();
    let _ = session.rollback();
    assert!(
        rescheduled_state
            .time_cycle
            .as_deref()
            .unwrap_or("")
            .starts_with("R3/"),
        "cycle is prepared: {:?}",
        rescheduled_state.time_cycle
    );
    // 2026-04-21T12:10:00Z
    assert_eq!(rescheduled_state.due_time, Some(1_776_773_400_000));
    assert!(rescheduled_state.lock_owner.is_none());
    assert_eq!(
        rescheduled_state.end_date.as_deref(),
        Some("2026-05-02T00:00:00Z")
    );
    assert_eq!(rescheduled_state.calendar_name.as_deref(), Some("cycle"));
    let rescheduled_data = timer_job_state_data(&engine, "timer-reschedule-fields");
    assert_eq!(rescheduled_data["end_date"], "2026-05-02T00:00:00Z");
    assert_eq!(rescheduled_data["calendar_name"], "cycle");

    let missing_timer_value = client
        .post(format!(
            "{}/management/timer-jobs/timer-reschedule-fields",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "action": "reschedule",
            "endDate": "2026-05-02T00:00:00Z",
            "calendarName": "cycle"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        missing_timer_value.status(),
        reqwest::StatusCode::BAD_REQUEST
    );
    let missing_timer_value_body: Value = missing_timer_value.json().await.unwrap();
    assert_eq!(missing_timer_value_body["code"], "BAD_REQUEST");
    // Java RescheduleTimerJobCmd.java:43-45 exact missing-value validation message.
    assert!(
        missing_timer_value_body["details"]
            .as_str()
            .unwrap()
            .contains(
                "A non-null value is required for one of timeDate, timeDuration, or timeCycle"
            )
    );
}

#[tokio::test]
async fn management_executable_job_execute_triggers_timer_wait_job() {
    let (engine, time_source) = build_engine("rest-management-execute-job");
    start_timer_wait_process(&engine);
    time_source.advance_time(300_001);
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let job_id = store
        .snapshot_timer_job_states(&mut session)
        .into_iter()
        .map(|(_, job)| job)
        .next()
        .unwrap()
        .timer_job_id;
    session.rollback().unwrap();
    engine
        .get_management_service()
        .move_timer_to_executable_job(&job_id)
        .expect("timer wait job should move to executable before manual execution");

    let (base_url, client) = spawn_server(Arc::clone(&engine)).await;
    let executed = client
        .post(format!("{}/management/jobs/{}", base_url, job_id))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "action": "execute" }))
        .send()
        .await
        .unwrap();
    assert_eq!(executed.status(), reqwest::StatusCode::NO_CONTENT);
    assert!(
        engine
            .get_management_service()
            .find_job_by_id(&job_id)
            .is_none()
    );
}

#[tokio::test]
async fn management_batches_expose_real_batch_documents_and_parts() {
    let (engine, _time_source) = build_engine("rest-management-batches");
    let batch_service = engine.get_batch_service();
    batch_service.create_batch(BatchEntity {
        id: "batch-1".to_string(),
        batch_type: "processMigration".to_string(),
        search_key: Some("batch-search".to_string()),
        search_key2: Some("batch-search-2".to_string()),
        status: "in-progress".to_string(),
        total_items: 1,
        items_processed: 0,
        create_time: 1_775_000_000_000,
        end_time: None,
        tenant_id: Some("tenant-a".to_string()),
        batch_document_json: Some(r#"{"migration":"planned"}"#.to_string()),
    });
    batch_service.create_batch(BatchEntity {
        id: "batch-without-tenant".to_string(),
        batch_type: "asyncHistory".to_string(),
        search_key: Some("without-tenant-search".to_string()),
        search_key2: None,
        status: "completed".to_string(),
        total_items: 1,
        items_processed: 1,
        create_time: 1_775_000_002_000,
        end_time: Some(1_775_000_003_000),
        tenant_id: None,
        batch_document_json: None,
    });
    batch_service.create_batch_part(BatchPartEntity {
        id: "batch-part-1".to_string(),
        batch_id: "batch-1".to_string(),
        batch_type: "processMigration".to_string(),
        search_key: Some("part-search".to_string()),
        search_key2: Some("part-search-2".to_string()),
        scope_id: Some("scope-1".to_string()),
        sub_scope_id: Some("sub-scope-1".to_string()),
        scope_type: Some("bpmn".to_string()),
        create_time: 1_775_000_001_000,
        complete_time: None,
        status: "waiting".to_string(),
        tenant_id: Some("tenant-a".to_string()),
        batch_part_document_json: Some(r#"{"part":"ready"}"#.to_string()),
    });

    let (base_url, client) = spawn_server(Arc::clone(&engine)).await;

    let batches = client
        .get(format!(
            "{}/management/batches?batchType=processMigration&status=in-progress&start=0&size=10",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(batches.status().is_success());
    let batches_body: Value = batches.json().await.unwrap();
    assert_eq!(batches_body["start"], 0);
    assert_eq!(batches_body["size"], 1);
    assert_eq!(batches_body["total"], 1);
    assert_eq!(batches_body["data"][0]["id"], "batch-1");
    assert_eq!(
        batches_body["data"][0]["url"],
        "/management/batches/batch-1"
    );
    assert_eq!(batches_body["data"][0]["batchType"], "processMigration");
    assert_eq!(batches_body["data"][0]["searchKey"], "batch-search");
    assert_eq!(batches_body["data"][0]["tenantId"], "tenant-a");
    assert_eq!(
        batches_body["data"][0]["createTime"],
        "2026-03-31T23:33:20+00:00"
    );
    assert_eq!(
        batches_body["data"][0]["completeTime"],
        serde_json::Value::Null
    );

    let deprecated_query_alias = client
        .get(format!(
            "{}/management/batches?batch_type=processMigration&status=in-progress",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(
        deprecated_query_alias.status(),
        reqwest::StatusCode::BAD_REQUEST
    );
    let deprecated_query_alias_body: Value = deprecated_query_alias.json().await.unwrap();
    assert_eq!(deprecated_query_alias_body["code"], "BAD_REQUEST");
    assert!(
        deprecated_query_alias_body["details"]
            .as_str()
            .unwrap()
            .contains("batch_type")
            || deprecated_query_alias_body["details"]
                .as_str()
                .unwrap()
                .contains("unknown field")
            || deprecated_query_alias_body["details"]
                .as_str()
                .unwrap()
                .contains("unknown variant")
            || deprecated_query_alias_body["details"]
                .as_str()
                .unwrap()
                .contains("Invalid query parameters"),
        "details: {}",
        deprecated_query_alias_body["details"]
    );

    let search_key_filtered = client
        .get(format!(
            "{}{}",
            base_url,
            "/management/batches?searchKey=batch-search&searchKey2=batch-search-2&sort=createTime&order=desc"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(search_key_filtered.status().is_success());
    let search_key_filtered_body: Value = search_key_filtered.json().await.unwrap();
    assert_eq!(search_key_filtered_body["total"], 1);
    assert_eq!(search_key_filtered_body["data"][0]["id"], "batch-1");

    let tenant_filtered = client
        .get(format!("{}/management/batches?tenantId=tenant-a", base_url))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(tenant_filtered.status().is_success());
    let tenant_filtered_body: Value = tenant_filtered.json().await.unwrap();
    assert_eq!(tenant_filtered_body["total"], 1);
    assert_eq!(tenant_filtered_body["data"][0]["id"], "batch-1");
    assert_eq!(tenant_filtered_body["data"][0]["tenantId"], "tenant-a");

    let tenant_like_filtered = client
        .get(format!(
            "{}/management/batches?tenantIdLike=tenant-&sort=tenantId&order=desc",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(tenant_like_filtered.status().is_success());
    let tenant_like_filtered_body: Value = tenant_like_filtered.json().await.unwrap();
    assert_eq!(tenant_like_filtered_body["total"], 1);
    assert_eq!(tenant_like_filtered_body["data"][0]["id"], "batch-1");

    let tenant_sorted = client
        .get(format!(
            "{}/management/batches?sort=tenantId&order=asc&size=10",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(tenant_sorted.status().is_success());
    let tenant_sorted_body: Value = tenant_sorted.json().await.unwrap();
    assert_eq!(tenant_sorted_body["total"], 2);
    assert_eq!(tenant_sorted_body["data"][0]["id"], "batch-without-tenant");
    assert!(tenant_sorted_body["data"][0]["tenantId"].is_null());
    assert_eq!(tenant_sorted_body["data"][1]["id"], "batch-1");

    let without_tenant_filtered = client
        .get(format!(
            "{}/management/batches?withoutTenantId=true",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(without_tenant_filtered.status().is_success());
    assert_eq!(
        without_tenant_filtered.json::<Value>().await.unwrap()["data"][0]["id"],
        "batch-without-tenant"
    );

    let invalid_batch_sort = client
        .get(format!("{}/management/batches?sort=unknown", base_url))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(
        invalid_batch_sort.status(),
        reqwest::StatusCode::BAD_REQUEST
    );

    let created = client
        .post(format!("{}/management/batches", base_url))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "id": "batch-2",
            "batchType": "asyncHistory",
            "searchKey": "batch-search-created",
            "searchKey2": "batch-search-created-2",
            "status": "completed",
            "totalItems": 2,
            "itemsProcessed": 2,
            "createTime": 1_775_000_010_000u64,
            "endTime": 1_775_000_011_000u64,
            "tenantId": "tenant-b",
            "batchDocumentJson": "{\"created\":true}"
        }))
        .send()
        .await
        .unwrap();
    assert!(created.status().is_success());
    let created_body: Value = created.json().await.unwrap();
    assert_eq!(created_body["id"], "batch-2");
    assert_eq!(created_body["batchType"], "asyncHistory");
    assert_eq!(created_body["itemsProcessed"], 2);
    assert_eq!(created_body["tenantId"], "tenant-b");

    let created_tenant_filtered = client
        .get(format!("{}/management/batches?tenantId=tenant-b", base_url))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(created_tenant_filtered.status().is_success());
    let created_tenant_filtered_body: Value = created_tenant_filtered.json().await.unwrap();
    assert_eq!(created_tenant_filtered_body["total"], 1);
    assert_eq!(created_tenant_filtered_body["data"][0]["id"], "batch-2");
    assert_eq!(
        created_tenant_filtered_body["data"][0]["tenantId"],
        "tenant-b"
    );

    let batch_document = client
        .get(format!(
            "{}/management/batches/batch-1/batch-document",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(batch_document.status().is_success());
    assert_eq!(
        batch_document
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .unwrap()
            .to_str()
            .unwrap(),
        "application/json"
    );
    assert_eq!(
        batch_document.json::<Value>().await.unwrap()["migration"],
        "planned"
    );

    let batch_parts = client
        .get(format!(
            "{}/management/batches/batch-1/batch-parts?status=waiting",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(batch_parts.status().is_success());
    let batch_parts_body: Value = batch_parts.json().await.unwrap();
    assert_eq!(batch_parts_body.as_array().unwrap().len(), 1);
    assert_eq!(batch_parts_body[0]["id"], "batch-part-1");
    assert_eq!(batch_parts_body[0]["batchId"], "batch-1");
    assert_eq!(batch_parts_body[0]["batchType"], "processMigration");
    assert_eq!(batch_parts_body[0]["searchKey"], "part-search");
    assert_eq!(batch_parts_body[0]["searchKey2"], "part-search-2");
    assert_eq!(batch_parts_body[0]["scopeId"], "scope-1");
    assert_eq!(batch_parts_body[0]["subScopeId"], "sub-scope-1");
    assert_eq!(batch_parts_body[0]["scopeType"], "bpmn");
    assert_eq!(batch_parts_body[0]["status"], "waiting");
    assert_eq!(batch_parts_body[0]["tenantId"], "tenant-a");
    assert_eq!(
        batch_parts_body[0]["createTime"],
        "2026-03-31T23:33:21+00:00"
    );
    assert_eq!(batch_parts_body[0]["completeTime"], serde_json::Value::Null);

    let batch_part = client
        .get(format!("{}/management/batch-parts/batch-part-1", base_url))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(batch_part.status().is_success());
    assert_eq!(
        batch_part.json::<Value>().await.unwrap()["id"],
        "batch-part-1"
    );

    let batch_part_document = client
        .get(format!(
            "{}/management/batch-parts/batch-part-1/batch-part-document",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(batch_part_document.status().is_success());
    assert_eq!(
        batch_part_document.json::<Value>().await.unwrap()["part"],
        "ready"
    );
}
