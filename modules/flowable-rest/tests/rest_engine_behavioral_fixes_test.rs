use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::service::config::ProcessEngineConfiguration;
use flowable_rest::run_server;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::net::TcpListener;

/// Helper: boot a REST server, return (base_url, engine, client).
async fn setup() -> (String, Arc<ProcessEngine>, reqwest::Client) {
    // Shell timeout test needs shell_tasks_enabled (disabled by default).
    let engine = Arc::new(ProcessEngine::new_with_config(
        "rest-engine-behavioral-fixes".to_string(),
        ProcessEngineConfiguration {
            shell_tasks_enabled: true,
            ..Default::default()
        },
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

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let base_url = format!("http://{}", addr);

    let engine_clone = Arc::clone(&engine);
    tokio::spawn(async move {
        run_server(engine_clone, listener).await.unwrap();
    });

    let client = reqwest::Client::new();
    (base_url, engine, client)
}

/// Deploy a BPMN XML and return the process definition ID.
async fn deploy(
    client: &reqwest::Client,
    base_url: &str,
    name: &str,
    xml: &str,
) -> Result<String, String> {
    let response = client
        .post(format!("{base_url}/repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": name,
            "resourceName": format!("{name}.bpmn20.xml"),
            "resource": xml
        }))
        .send()
        .await
        .unwrap();

    if !response.status().is_success() {
        return Err(response.text().await.unwrap());
    }

    // Fetch process definition ID from the deployment
    let definitions_response = client
        .get(format!("{base_url}/repository/process-definitions"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(definitions_response.status().is_success());
    let body: Value = definitions_response.json().await.unwrap();
    let definitions = body["data"].as_array().unwrap();

    // Find the definition matching the deployed name
    let definition_id = definitions
        .iter()
        .find(|d| d["key"].as_str().unwrap() == name)
        .or_else(|| definitions.last())
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();

    Ok(definition_id)
}

/// Start a process instance and return the process instance ID.
async fn start_process(
    client: &reqwest::Client,
    base_url: &str,
    process_definition_id: &str,
    variables: Value,
) -> Result<String, String> {
    let response = client
        .post(format!("{base_url}/runtime/process-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "processDefinitionId": process_definition_id,
            "variables": variables
        }))
        .send()
        .await
        .unwrap();

    if !response.status().is_success() {
        return Err(response.text().await.unwrap());
    }
    let body: Value = response.json().await.unwrap();
    Ok(body["id"].as_str().unwrap().to_string())
}

#[tokio::test]
async fn test_dmn_service_task_deployment() {
    let (base_url, _engine, client) = setup().await;

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
                 xmlns:flowable="http://flowable.org/bpmn"
                 targetNamespace="Examples">
        <process id="dmnServiceTaskProcess" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="flow1" sourceRef="start" targetRef="dmnTask" />
            <serviceTask id="dmnTask" flowable:type="dmn">
                <extensionElements>
                    <flowable:field name="decisionTableReferenceKey" stringValue="someDecision" />
                </extensionElements>
            </serviceTask>
            <sequenceFlow id="flow2" sourceRef="dmnTask" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#;

    // type="dmn" must deploy when a decision table reference key is present.
    // (Without the key, Java ExternalInvocationTaskValidator.java:88-108 rejects at
    // deployment — the old bare model encoded pre-parity lenient behavior.)
    let deploy_res = deploy(&client, &base_url, "dmnServiceTaskProcess", xml).await;
    assert!(
        deploy_res.is_ok(),
        "Failed to deploy dmn service task: {:?}",
        deploy_res
    );
}

#[tokio::test]
async fn test_shell_task_timeout() {
    let (base_url, _engine, client) = setup().await;

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
                 xmlns:flowable="http://flowable.org/bpmn"
                 targetNamespace="Examples">
        <process id="shellTimeoutProcess" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="flow1" sourceRef="start" targetRef="shellTask" />
            <serviceTask id="shellTask" flowable:type="shell">
                <extensionElements>
                    <flowable:command>ping</flowable:command>
                    <flowable:arg>-n</flowable:arg>
                    <flowable:arg>5</flowable:arg>
                    <flowable:arg>127.0.0.1</flowable:arg>
                    <flowable:timeout>50</flowable:timeout>
                </extensionElements>
            </serviceTask>
            <sequenceFlow id="flow2" sourceRef="shellTask" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#;

    let pd_id = deploy(&client, &base_url, "shellTimeoutProcess", xml)
        .await
        .unwrap();

    // Starting the process will execute the shell task. Since timeout is 50ms and ping runs for ~4s,
    // it should fail as a 500. Public details are generic (timeout text is not echoed).
    let start_res = start_process(&client, &base_url, &pd_id, json!([])).await;
    assert!(
        start_res.is_err(),
        "Expected timeout error but started successfully"
    );
    let err_msg = start_res.unwrap_err();
    assert!(
        err_msg.contains("INTERNAL_SERVER_ERROR") || err_msg.contains("Internal server error"),
        "Expected generic 500 error body, got: {}",
        err_msg
    );
    assert!(
        !err_msg.contains("timed out") && !err_msg.to_lowercase().contains("timeout"),
        "5xx must not echo timeout internals, got: {}",
        err_msg
    );
}

#[tokio::test]
async fn test_data_association_assignments() {
    let (base_url, _engine, client) = setup().await;

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
                 xmlns:flowable="http://flowable.org/bpmn"
                 targetNamespace="Examples">
        <process id="assignmentProcess" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="flow1" sourceRef="start" targetRef="userTask" />
            <userTask id="userTask">
                <dataInputAssociation>
                    <assignment>
                        <from>${inputVar}</from>
                        <to>${outputVar}</to>
                    </assignment>
                </dataInputAssociation>
            </userTask>
            <sequenceFlow id="flow2" sourceRef="userTask" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#;

    let pd_id = deploy(&client, &base_url, "assignmentProcess", xml)
        .await
        .unwrap();

    // Start with variable inputVar set to "hello"
    let pi_id = start_process(
        &client,
        &base_url,
        &pd_id,
        json!([
            {"name": "inputVar", "value": "hello"}
        ]),
    )
    .await
    .unwrap();

    // Query variables for the process instance
    let response = client
        .get(format!(
            "{base_url}/runtime/process-instances/{pi_id}/variables"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success());
    let body: Value = response.json().await.unwrap();
    let variables = body.as_array().unwrap();

    let output_var = variables
        .iter()
        .find(|v| v["name"].as_str().unwrap() == "outputVar")
        .expect("Expected outputVar variable to be mapped by assignments");
    assert_eq!(output_var["value"].as_str().unwrap(), "hello");
}

#[tokio::test]
async fn test_adhoc_subprocess_activation() {
    let (base_url, _engine, client) = setup().await;

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
                 xmlns:flowable="http://flowable.org/bpmn"
                 targetNamespace="Examples">
        <process id="adhocProcess" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="flow1" sourceRef="start" targetRef="adhocSubProcess" />
            <adHocSubProcess id="adhocSubProcess">
                <userTask id="innerUserTask" />
            </adHocSubProcess>
            <sequenceFlow id="flow2" sourceRef="adhocSubProcess" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#;

    let pd_id = deploy(&client, &base_url, "adhocProcess", xml)
        .await
        .unwrap();
    let pi_id = start_process(&client, &base_url, &pd_id, json!([]))
        .await
        .unwrap();

    // Fetch executions to find the adhocSubProcess execution ID
    let response = client
        .get(format!(
            "{base_url}/runtime/executions?processInstanceId={pi_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success());
    let body: Value = response.json().await.unwrap();
    let executions = body["data"].as_array().unwrap();

    let adhoc_execution = executions
        .iter()
        .find(|e| e["activityId"].as_str() == Some("adhocSubProcess"))
        .expect("Expected execution with activityId 'adhocSubProcess'");
    let execution_id = adhoc_execution["id"].as_str().unwrap();

    // Manually activate "innerUserTask" inside the adhocSubProcess
    let activate_response = client
        .post(format!(
            "{base_url}/runtime/executions/{execution_id}/activate-activity"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "activityId": "innerUserTask"
        }))
        .send()
        .await
        .unwrap();
    assert!(
        activate_response.status().is_success(),
        "activate-activity failed: {}",
        activate_response.text().await.unwrap()
    );

    // Verify that the task "innerUserTask" has been created!
    let tasks_response = client
        .get(format!(
            "{base_url}/runtime/tasks?processInstanceId={pi_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(tasks_response.status().is_success());
    let tasks_body: Value = tasks_response.json().await.unwrap();
    let tasks = tasks_body["data"].as_array().unwrap();

    assert!(
        tasks
            .iter()
            .any(|t| t["name"].as_str() == Some("innerUserTask")),
        "Expected manually activated 'innerUserTask' task to be active"
    );
}
