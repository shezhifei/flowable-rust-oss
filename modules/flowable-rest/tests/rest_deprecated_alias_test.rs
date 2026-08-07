use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_rest::run_server;
use reqwest::StatusCode;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::net::TcpListener;

const USER_TASK_PROCESS_BPMN: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
    <process id="deprecatedAliasSurfaceCutoverProcess" isExecutable="true">
        <startEvent id="start" />
        <sequenceFlow id="flow1" sourceRef="start" targetRef="task1" />
        <userTask id="task1" name="Rust Native Task" />
        <sequenceFlow id="flow2" sourceRef="task1" targetRef="end" />
        <endEvent id="end" />
    </process>
</definitions>"#;

async fn spawn_server(engine: Arc<ProcessEngine>) -> (String, reqwest::Client) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());

    let engine_clone = Arc::clone(&engine);
    tokio::spawn(async move {
        run_server(engine_clone, listener).await.unwrap();
    });

    (base_url, reqwest::Client::new())
}

fn build_engine(test_name: &str) -> Arc<ProcessEngine> {
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
    engine
}

#[tokio::test]
async fn native_business_rest_contract_remains_default_after_deprecated_alias_cutover() {
    let engine = build_engine("rest-deprecated-business-surface-cutover");
    let (base_url, client) = spawn_server(Arc::clone(&engine)).await;

    let deployment = client
        .post(format!("{base_url}/repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "Rust Native Deployment",
            "resourceName": "deprecated_alias_surface_cutover.bpmn20.xml",
            "resource": USER_TASK_PROCESS_BPMN
        }))
        .send()
        .await
        .unwrap();
    // P109: deploy (both JSON superset and multipart paths) returns 201.
    assert_eq!(deployment.status(), StatusCode::CREATED);

    let process_definition_id = engine
        .get_repository_service()
        .get_process_definition_ids()
        .unwrap()
        .into_iter()
        .next()
        .expect("deployment should register a process definition");

    let started = client
        .post(format!("{base_url}/runtime/process-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "processDefinitionId": process_definition_id,
            "businessKey": "rust-native-default"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(started.status(), StatusCode::OK);
    let started_body: Value = started.json().await.unwrap();
    let process_instance_id = started_body["id"].as_str().unwrap();

    let tasks = client
        .get(format!(
            "{base_url}/runtime/tasks?processInstanceId={process_instance_id}&start=0&size=10"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(tasks.status(), StatusCode::OK);
    let tasks_body: Value = tasks.json().await.unwrap();
    assert_eq!(tasks_body["total"], 1);
    assert_eq!(tasks_body["data"][0]["name"], "Rust Native Task");
}

#[tokio::test]
async fn deprecated_service_business_aliases_are_not_registered_by_default() {
    let engine = build_engine("rest-deprecated-business-aliases-removed");
    let (base_url, client) = spawn_server(engine).await;

    // Remaining deprecated alias surface:
    // - All `/service/...` business and management aliases are intentionally
    //   not registered by the default Rust REST server. Only the native
    //   `/management/...` and bare REST contract paths are exposed.
    let removed_aliases = [
        "/service/management/directory/support",
        "/service/management/operations/support",
        "/service/management/jmx/runtime",
        "/service/management/jmx/connector-descriptor",
        "/service/management/jmx/mbean-registry",
        "/service/management/jmx/operations-bus",
        "/service/management/jmx/runtime-ledger",
        "/service/management/jmx/timer-ledger",
        "/service/management/operations/topology",
        "/service/management/platform/support",
        "/service/management/platform/topology-certification",
        "/service/repository/deployments",
        "/service/repository/process-definitions",
        "/service/runtime/process-instances",
        "/service/runtime/tasks",
        "/service/history/historic-process-instances",
        "/service/external-worker/jobs",
        "/service/identity/tokens",
        "/service/form-repository/form-definitions",
        "/service/content-service/content-items",
    ];

    for path in removed_aliases {
        let response = client
            .get(format!("{base_url}{path}"))
            .basic_auth("admin", Some("test"))
            .send()
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::NOT_FOUND,
            "{path} should not be a default deprecated alias"
        );
    }
}

#[tokio::test]
async fn java_style_external_worker_paths_are_not_registered() {
    let engine = build_engine("rest-deprecated-external-worker-paths-removed");
    let (base_url, client) = spawn_server(engine).await;

    // Java-style external-worker paths that previously shadowed the native
    // `/external-worker/jobs/...` surface are intentionally not registered
    // by the default Rust REST server. Only the canonical paths are exposed.
    let removed_paths = [
        ("GET", "/jobs"),
        ("POST", "/acquire/jobs"),
        ("POST", "/unacquire/jobs"),
        ("GET", "/jobs/non-existent"),
        ("POST", "/acquire/jobs/non-existent/complete"),
        ("POST", "/acquire/jobs/non-existent/fail"),
        ("POST", "/acquire/jobs/non-existent/bpmnError"),
        ("POST", "/acquire/jobs/non-existent/cmmnTerminate"),
        ("POST", "/unacquire/jobs/non-existent"),
    ];

    for (method, path) in removed_paths {
        let request = match method {
            "GET" => client.get(format!("{base_url}{path}")),
            "POST" => client.post(format!("{base_url}{path}")),
            _ => unreachable!("unsupported method in this test"),
        };
        let response = request
            .basic_auth("admin", Some("test"))
            .json(&json!({}))
            .send()
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::NOT_FOUND,
            "{method} {path} should not be a default Java-style alias"
        );
    }
}
