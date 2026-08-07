use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_rest::run_server;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::net::TcpListener;

async fn spawn_server(test_name: &str) -> (Arc<ProcessEngine>, String, reqwest::Client) {
    let engine = Arc::new(ProcessEngine::new(test_name.to_string()));

    let user = flowable_engine::identity::entities::User {
        id: "admin".to_string(),
        first_name: None,
        last_name: None,
        email: None,
        password: Some("test".to_string()),
        tenant_id: None,
    };
    engine.get_identity_service().save_user(user);

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let base_url = format!("http://{}", addr);

    let engine_clone = Arc::clone(&engine);
    tokio::spawn(async move {
        run_server(engine_clone, listener).await.unwrap();
    });

    (engine, base_url, reqwest::Client::new())
}

async fn deploy_user_task_process(client: &reqwest::Client, base_url: &str) {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="commonContractProcess" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="f1" sourceRef="start" targetRef="task1" />
            <userTask id="task1" name="Contract Task" />
            <sequenceFlow id="f2" sourceRef="task1" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#;

    let response = client
        .post(format!("{}/repository/deployments", base_url))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "Common Contract Deployment",
            "resourceName": "common_contract_process.bpmn20.xml",
            "resource": xml
        }))
        .send()
        .await
        .unwrap();

    assert!(response.status().is_success());

    let _: Value = response.json().await.unwrap();
}

fn user_task_process_xml(process_id: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="{process_id}" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="f1" sourceRef="start" targetRef="task1" />
            <userTask id="task1" name="Contract Task" />
            <sequenceFlow id="f2" sourceRef="task1" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#
    )
}

async fn start_process_instance(
    client: &reqwest::Client,
    base_url: &str,
    process_definition_id: &str,
    business_key: &str,
) -> String {
    let response = client
        .post(format!("{}/runtime/process-instances", base_url))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "processDefinitionId": process_definition_id,
            "businessKey": business_key
        }))
        .send()
        .await
        .unwrap();

    assert!(response.status().is_success());

    let process_instance: Value = response.json().await.unwrap();
    process_instance["id"].as_str().unwrap().to_string()
}

#[tokio::test]
async fn process_definition_without_tenant_query_matches_rest_contract() {
    let (engine, base_url, client) = spawn_server("rest-common-process-definition-tenant").await;
    deploy_user_task_process(&client, &base_url).await;

    engine
        .get_repository_service()
        .deploy(
            engine
                .get_repository_service()
                .create_deployment()
                .name("Tenant Contract Deployment".to_string())
                .tenant_id("tenant-a".to_string())
                .add_string(
                    "tenant_contract_process.bpmn20.xml".to_string(),
                    user_task_process_xml("tenantContractProcess"),
                ),
        )
        .unwrap();

    let without_tenant_response = client
        .get(format!(
            "{}/repository/process-definitions?withoutTenantId=true&sort=key&order=asc",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert!(without_tenant_response.status().is_success());
    let without_tenant_body: Value = without_tenant_response.json().await.unwrap();
    assert_eq!(without_tenant_body["total"], 1);
    assert_eq!(
        without_tenant_body["data"][0]["key"],
        "commonContractProcess"
    );
    assert!(without_tenant_body["data"][0]["tenantId"].is_null());

    let conflicting_tenant_response = client
        .get(format!(
            "{}/repository/process-definitions?tenantId=tenant-a&withoutTenantId=true",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert_eq!(
        conflicting_tenant_response.status(),
        reqwest::StatusCode::BAD_REQUEST
    );
    let conflicting_tenant_body: Value = conflicting_tenant_response.json().await.unwrap();
    assert_eq!(conflicting_tenant_body["code"], "BAD_REQUEST");
    assert_eq!(conflicting_tenant_body["message"], "Bad Request");
    assert!(
        conflicting_tenant_body["details"]
            .as_str()
            .unwrap()
            .contains("withoutTenantId"),
        "details were: {}",
        conflicting_tenant_body["details"]
    );
}

#[tokio::test]
async fn list_resources_return_common_rest_paging_envelopes() {
    let (engine, base_url, client) = spawn_server("rest-common-contract").await;
    deploy_user_task_process(&client, &base_url).await;
    let process_definition_id = engine
        .get_repository_service()
        .get_process_definition_ids()
        .unwrap()[0]
        .clone();

    let process_instance_id =
        start_process_instance(&client, &base_url, &process_definition_id, "common-rest-1").await;

    let process_instances = client
        .get(format!(
            "{}/runtime/process-instances?start=0&size=10",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert!(process_instances.status().is_success());
    let process_instances_body: Value = process_instances.json().await.unwrap();
    assert_eq!(process_instances_body["start"], 0);
    assert_eq!(process_instances_body["size"], 1);
    assert_eq!(process_instances_body["total"], 1);
    assert_eq!(process_instances_body["data"].as_array().unwrap().len(), 1);

    let tasks = client
        .get(format!(
            "{}/runtime/tasks?processInstanceId={}&start=0&size=10",
            base_url, process_instance_id
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert!(tasks.status().is_success());
    let tasks_body: Value = tasks.json().await.unwrap();
    assert_eq!(tasks_body["start"], 0);
    assert_eq!(tasks_body["size"], 1);
    assert_eq!(tasks_body["total"], 1);
    assert_eq!(tasks_body["data"].as_array().unwrap().len(), 1);
    let task_id = tasks_body["data"][0]["id"].as_str().unwrap().to_string();

    let complete = client
        .post(format!("{}/runtime/tasks/{}/complete", base_url, task_id))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "action": "complete" }))
        .send()
        .await
        .unwrap();
    assert!(complete.status().is_success());

    let historic_process_instances = client
        .get(format!(
            "{}/history/historic-process-instances?start=0&size=10",
            base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();

    assert!(historic_process_instances.status().is_success());
    let history_body: Value = historic_process_instances.json().await.unwrap();
    assert_eq!(history_body["start"], 0);
    assert_eq!(history_body["size"], 1);
    assert_eq!(history_body["total"], 1);
    assert_eq!(history_body["data"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn unsupported_query_parameters_fail_with_structured_bad_request() {
    let (engine, base_url, client) = spawn_server("rest-common-bad-request").await;
    deploy_user_task_process(&client, &base_url).await;
    let process_definition_id = engine
        .get_repository_service()
        .get_process_definition_ids()
        .unwrap()[0]
        .clone();

    let process_instance_id =
        start_process_instance(&client, &base_url, &process_definition_id, "common-rest-2").await;

    let cases = [
        format!(
            "{}/runtime/process-instances?unexpectedField=value",
            base_url
        ),
        format!(
            "{}/runtime/tasks?processInstanceId={}&unexpectedField=value",
            base_url, process_instance_id
        ),
        format!(
            "{}/history/historic-process-instances?unexpectedField=value",
            base_url
        ),
    ];

    for url in cases {
        let response = client
            .get(&url)
            .basic_auth("admin", Some("test"))
            .send()
            .await
            .unwrap();

        assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);

        let body: Value = response.json().await.unwrap();
        assert_eq!(body["code"], "BAD_REQUEST");
        assert_eq!(body["message"], "Bad Request");
        assert!(
            body["details"]
                .as_str()
                .unwrap()
                .contains("unexpectedField"),
            "details were: {}",
            body["details"]
        );
    }
}
