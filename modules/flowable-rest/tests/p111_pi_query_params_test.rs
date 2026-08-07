// P111 — BPMN REST process-instance query parameter surface.
// GET 8-family additions (activeActivityId, super/subProcessInstanceId,
// excludeSubprocesses, includeProcessVariablesNames, callbackIds,
// processDefinitionCategory*/EngineVersion) + POST field-name alignment
// (processInstanceName*/processBusinessKey*/processBusinessStatus* dual-name
// via serde alias) and POST-only params (processInstanceIds,
// processDefinitionIds/Keys/excludeProcessDefinitionKeys, deploymentId( In),
// activeActivityIds, callbackIds).
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_rest::run_server;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::net::TcpListener;

const QUERY_PROCESS_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
    <process id="queryProcess" isExecutable="true">
        <startEvent id="start" />
        <sequenceFlow id="f1" sourceRef="start" targetRef="task1" />
        <userTask id="task1" name="Task 1" />
        <sequenceFlow id="f2" sourceRef="task1" targetRef="end" />
        <endEvent id="end" />
    </process>
</definitions>"#;

async fn setup() -> (String, Arc<ProcessEngine>) {
    let engine = Arc::new(ProcessEngine::new("rest-p111-query".to_string()));
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
    (base_url, engine)
}

struct TestContext {
    base_url: String,
    process_instance_id: String,
    execution_id: String,
    deployment_id: String,
}

async fn deploy_and_start(
    client: &reqwest::Client,
    base_url: &str,
    engine: &Arc<ProcessEngine>,
) -> TestContext {
    let res = client
        .post(format!("{base_url}/repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "P111 Query Deployment",
            "resourceName": "query_process.bpmn20.xml",
            "resource": QUERY_PROCESS_XML
        }))
        .send()
        .await
        .unwrap();
    assert!(res.status().is_success(), "deploy failed: {}", res.status());
    let deploy_body: Value = res.json().await.unwrap();
    let deployment_id = deploy_body["id"].as_str().unwrap().to_string();

    let process_definition_id = engine
        .get_repository_service()
        .get_process_definition_ids()
        .unwrap()[0]
        .clone();

    let res = client
        .post(format!("{base_url}/runtime/process-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "processDefinitionId": process_definition_id,
            "businessKey": "My Key"
        }))
        .send()
        .await
        .unwrap();
    assert!(res.status().is_success(), "start failed: {}", res.status());
    let started: Value = res.json().await.unwrap();
    let process_instance_id = started["id"].as_str().unwrap().to_string();

    let tasks = client
        .get(format!(
            "{base_url}/runtime/tasks?processInstanceId={process_instance_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(tasks.status().is_success());
    let tasks_body: Value = tasks.json().await.unwrap();
    let execution_id = tasks_body["data"][0]["executionId"].as_str().unwrap().to_string();

    TestContext {
        base_url: base_url.to_string(),
        process_instance_id,
        execution_id,
        deployment_id,
    }
}

async fn get_pi(client: &reqwest::Client, base_url: &str, query: &str) -> Value {
    let res = client
        .get(format!("{base_url}/runtime/process-instances?{query}"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(
        res.status().is_success(),
        "GET ?{query} failed: {}",
        res.status()
    );
    res.json().await.unwrap()
}

async fn post_pi(client: &reqwest::Client, base_url: &str, body: Value) -> Value {
    let res = client
        .post(format!("{base_url}/query/process-instances"))
        .basic_auth("admin", Some("test"))
        .json(&body)
        .send()
        .await
        .unwrap();
    assert!(
        res.status().is_success(),
        "POST failed with {}: {body}",
        res.status()
    );
    res.json().await.unwrap()
}

#[tokio::test]
async fn p111_get_process_instance_query_params() {
    let client = reqwest::Client::new();
    let (base_url, engine) = setup().await;
    let ctx = deploy_and_start(&client, &base_url, &engine).await;

    // Give the instance a name, businessStatus, callbackId; and a variable.
    engine
        .get_variable_service()
        .set_variable(
            ctx.execution_id.to_string(),
            "route".to_string(),
            json!("Accepted"),
        )
        .unwrap();
    let update = client
        .put(format!(
            "{}/runtime/process-instances/{}",
            ctx.base_url, ctx.process_instance_id
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "My Instance",
            "businessStatus": "Ready",
            "callbackId": "callback-1"
        }))
        .send()
        .await
        .unwrap();
    assert!(update.status().is_success(), "update failed: {}", update.status());

    // activeActivityId — instance is waiting at userTask task1.
    let body = get_pi(&client, &ctx.base_url, "activeActivityId=task1").await;
    assert_eq!(body["total"], 1, "activeActivityId body: {body}");
    assert_eq!(body["data"][0]["id"], ctx.process_instance_id);

    // excludeSubprocesses — top-level instance has no super execution.
    let body = get_pi(&client, &ctx.base_url, "excludeSubprocesses=true").await;
    assert_eq!(body["total"], 1, "excludeSubprocesses body: {body}");

    // includeProcessVariablesNames — only the named variable is returned.
    let body = get_pi(&client, &ctx.base_url, "includeProcessVariablesNames=route").await;
    assert_eq!(body["total"], 1, "includeProcessVariablesNames body: {body}");
    assert_eq!(
        body["data"][0]["variables"],
        json!([{
            "name": "route",
            "type": "string",
            "value": "Accepted",
            "scope": "global"
        }])
    );

    // callbackIds — set on the instance via update.
    let body = get_pi(&client, &ctx.base_url, "callbackIds=callback-1").await;
    assert_eq!(body["total"], 1, "callbackIds body: {body}");

    // processDefinitionCategory — populated from the BPMN targetNamespace.
    let body = get_pi(&client, &ctx.base_url, "processDefinitionCategory=Examples").await;
    assert_eq!(body["total"], 1, "processDefinitionCategory body: {body}");

    // Remaining GET families must parse without a 400 (Java legal query params).
    for query in [
        "processDefinitionCategoryLike=Exam%",
        "processDefinitionCategoryLikeIgnoreCase=examples",
        "processDefinitionEngineVersion=v1",
        "superProcessInstanceId=does-not-exist",
        "subProcessInstanceId=does-not-exist",
    ] {
        let res = client
            .get(format!("{}/runtime/process-instances?{query}", ctx.base_url))
            .basic_auth("admin", Some("test"))
            .send()
            .await
            .unwrap();
        assert!(res.status().is_success(), "GET ?{query} should not 400");
    }
}

#[tokio::test]
async fn p111_post_process_instance_query_params() {
    let client = reqwest::Client::new();
    let (base_url, engine) = setup().await;
    let ctx = deploy_and_start(&client, &base_url, &engine).await;

    engine
        .get_variable_service()
        .set_variable(
            ctx.execution_id.to_string(),
            "route".to_string(),
            json!("Accepted"),
        )
        .unwrap();
    let update = client
        .put(format!(
            "{}/runtime/process-instances/{}",
            ctx.base_url, ctx.process_instance_id
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "My Instance",
            "businessStatus": "Ready",
            "callbackId": "callback-1"
        }))
        .send()
        .await
        .unwrap();
    assert!(update.status().is_success());

    // POST field-name alignment: Java names and legacy aliases both work.
    let body = post_pi(&client, &ctx.base_url, json!({"processInstanceName": "My Instance"})).await;
    assert_eq!(body["total"], 1, "processInstanceName body: {body}");
    let body = post_pi(&client, &ctx.base_url, json!({"name": "My Instance"})).await;
    assert_eq!(body["total"], 1, "name alias body: {body}");

    let body = post_pi(&client, &ctx.base_url, json!({"processBusinessKey": "My Key"})).await;
    assert_eq!(body["total"], 1, "processBusinessKey body: {body}");
    let body = post_pi(&client, &ctx.base_url, json!({"businessKey": "My Key"})).await;
    assert_eq!(body["total"], 1, "businessKey alias body: {body}");

    let body = post_pi(&client, &ctx.base_url, json!({"processBusinessStatus": "Ready"})).await;
    assert_eq!(body["total"], 1, "processBusinessStatus body: {body}");
    let body = post_pi(&client, &ctx.base_url, json!({"businessStatus": "Ready"})).await;
    assert_eq!(body["total"], 1, "businessStatus alias body: {body}");

    // POST-only params.
    let body = post_pi(
        &client,
        &ctx.base_url,
        json!({"processInstanceIds": [ctx.process_instance_id]}),
    )
    .await;
    assert_eq!(body["total"], 1, "processInstanceIds body: {body}");

    let body = post_pi(
        &client,
        &ctx.base_url,
        json!({"deploymentIdIn": [ctx.deployment_id]}),
    )
    .await;
    assert_eq!(body["total"], 1, "deploymentIdIn body: {body}");

    let body = post_pi(
        &client,
        &ctx.base_url,
        json!({"deploymentId": ctx.deployment_id}),
    )
    .await;
    assert_eq!(body["total"], 1, "deploymentId body: {body}");

    let body = post_pi(&client, &ctx.base_url, json!({"activeActivityIds": ["task1"]})).await;
    assert_eq!(body["total"], 1, "activeActivityIds body: {body}");

    let body = post_pi(
        &client,
        &ctx.base_url,
        json!({"processDefinitionKeys": ["queryProcess"]}),
    )
    .await;
    assert_eq!(body["total"], 1, "processDefinitionKeys body: {body}");

    let body = post_pi(
        &client,
        &ctx.base_url,
        json!({"excludeProcessDefinitionKeys": ["other"]}),
    )
    .await;
    assert_eq!(body["total"], 1, "excludeProcessDefinitionKeys body: {body}");

    let body = post_pi(
        &client,
        &ctx.base_url,
        json!({"callbackIds": ["callback-1"]}),
    )
    .await;
    assert_eq!(body["total"], 1, "callbackIds POST body: {body}");

    // CMMN scope params are accepted without effect (tasks.rs precedent).
    let body = post_pi(
        &client,
        &ctx.base_url,
        json!({
            "rootScopeId": "root-1",
            "parentScopeId": "parent-1",
            "parentCaseInstanceId": "case-1"
        }),
    )
    .await;
    assert_eq!(body["total"], 1, "CMMN accept body: {body}");
}
