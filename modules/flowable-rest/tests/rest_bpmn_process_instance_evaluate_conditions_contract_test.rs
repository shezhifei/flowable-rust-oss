use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_rest::run_server;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::net::TcpListener;

#[tokio::test]
async fn runtime_process_instance_evaluate_conditions_triggers_conditional_waits() {
    let engine = Arc::new(ProcessEngine::new(
        "rest-bpmn-evaluate-conditions".to_string(),
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
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="evaluateConditionsProcess" name="Evaluate Conditions Process" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="f1" sourceRef="start" targetRef="catchApproved" />
            <intermediateCatchEvent id="catchApproved" name="Catch Approved">
                <conditionalEventDefinition>
                    <condition>${approved == true}</condition>
                </conditionalEventDefinition>
            </intermediateCatchEvent>
            <sequenceFlow id="f2" sourceRef="catchApproved" targetRef="afterConditionTask" />
            <userTask id="afterConditionTask" name="Task After Condition" />
            <sequenceFlow id="f3" sourceRef="afterConditionTask" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#;

    let deploy_response = client
        .post(format!("{base_url}/repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "Evaluate Conditions Deployment",
            "resourceName": "evaluate_conditions_process.bpmn20.xml",
            "resource": xml
        }))
        .send()
        .await
        .unwrap();
    assert!(deploy_response.status().is_success());

    let process_definition_id = engine
        .get_repository_service()
        .get_process_definition_ids()
        .unwrap()[0]
        .clone();

    let start_response = client
        .post(format!("{base_url}/runtime/process-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "processDefinitionId": process_definition_id,
            "businessKey": "Evaluate Conditions Instance"
        }))
        .send()
        .await
        .unwrap();
    assert!(start_response.status().is_success());
    let start_body: Value = start_response.json().await.unwrap();
    let process_instance_id = start_body["id"].as_str().unwrap().to_string();

    let evaluate_without_variables_response = client
        .post(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}/evaluate-conditions"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(evaluate_without_variables_response.status(), 500);
    let evaluate_without_variables_body: Value =
        evaluate_without_variables_response.json().await.unwrap();
    // 5xx details are generic (no internal exception text echo).
    assert_eq!(
        evaluate_without_variables_body["details"],
        "Internal server error",
        "unexpected null-condition error body: {evaluate_without_variables_body}"
    );

    let waiting_tasks_response = client
        .post(format!("{base_url}/query/tasks"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "processInstanceId": process_instance_id
        }))
        .send()
        .await
        .unwrap();
    assert!(waiting_tasks_response.status().is_success());
    let waiting_tasks: Value = waiting_tasks_response.json().await.unwrap();
    assert_eq!(waiting_tasks["total"], 0);

    let evaluate_response = client
        .post(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}/evaluate-conditions"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!([{ "name": "approved", "type": "boolean", "value": true }]))
        .send()
        .await
        .unwrap();
    assert_eq!(evaluate_response.status(), 200);

    let tasks_after_evaluation_response = client
        .post(format!("{base_url}/query/tasks"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "processInstanceId": process_instance_id
        }))
        .send()
        .await
        .unwrap();
    assert!(tasks_after_evaluation_response.status().is_success());
    let tasks_after_evaluation: Value = tasks_after_evaluation_response.json().await.unwrap();
    assert_eq!(tasks_after_evaluation["total"], 1);
    assert_eq!(
        tasks_after_evaluation["data"][0]["name"],
        "Task After Condition"
    );

    let missing_response = client
        .post(format!(
            "{base_url}/runtime/process-instances/missing-process/evaluate-conditions"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(missing_response.status(), 404);

    let non_boolean_xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="nonBooleanConditionProcess" name="Non Boolean Condition Process" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="f1" sourceRef="start" targetRef="catchApproved" />
            <intermediateCatchEvent id="catchApproved" name="Catch Approved">
                <conditionalEventDefinition>
                    <condition>${approved}</condition>
                </conditionalEventDefinition>
            </intermediateCatchEvent>
            <sequenceFlow id="f2" sourceRef="catchApproved" targetRef="afterConditionTask" />
            <userTask id="afterConditionTask" name="Task After Condition" />
            <sequenceFlow id="f3" sourceRef="afterConditionTask" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#;

    let non_boolean_deploy_response = client
        .post(format!("{base_url}/repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "Non Boolean Condition Deployment",
            "resourceName": "non_boolean_condition_process.bpmn20.xml",
            "resource": non_boolean_xml
        }))
        .send()
        .await
        .unwrap();
    assert!(non_boolean_deploy_response.status().is_success());

    let non_boolean_start_response = client
        .post(format!("{base_url}/runtime/process-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "processDefinitionKey": "nonBooleanConditionProcess"
        }))
        .send()
        .await
        .unwrap();
    assert!(non_boolean_start_response.status().is_success());
    let non_boolean_start_body: Value = non_boolean_start_response.json().await.unwrap();
    let non_boolean_process_instance_id = non_boolean_start_body["id"].as_str().unwrap();

    let non_boolean_response = client
        .post(format!(
            "{base_url}/runtime/process-instances/{non_boolean_process_instance_id}/evaluate-conditions"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!([{ "name": "approved", "type": "string", "value": "yes" }]))
        .send()
        .await
        .unwrap();
    assert_eq!(non_boolean_response.status(), 500);
    let non_boolean_body: Value = non_boolean_response.json().await.unwrap();
    // 5xx details are generic (no non-Boolean condition text echo).
    assert_eq!(
        non_boolean_body["details"],
        "Internal server error",
        "unexpected non-Boolean error body: {non_boolean_body}"
    );
    assert_eq!(
        engine
            .get_runtime_service()
            .get_variable(
                non_boolean_process_instance_id.to_string(),
                "approved".to_string()
            )
            .unwrap(),
        None,
        "variables submitted with a failing condition command must roll back"
    );

    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let mut suspended_instance = store
        .find_process_instance(&process_instance_id, &mut session)
        .unwrap();
    suspended_instance.is_suspended = true;
    store.update_process_instance(&suspended_instance, &mut session);
    session.flush_and_commit().unwrap();

    // Java parity: EvaluateConditionalEventsCmd extends NeedsActiveExecutionCmd,
    // which raises FlowableException (HTTP 500) for a suspended execution.
    let suspended_response = client
        .post(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}/evaluate-conditions"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(suspended_response.status(), 500);
    let suspended_body: Value = suspended_response.json().await.unwrap();
    // 5xx details are generic (no suspended-execution message echo).
    assert_eq!(
        suspended_body["details"],
        "Internal server error",
        "unexpected suspended error body: {suspended_body}"
    );

    // Java parity: an ended instance no longer has a runtime execution, so
    // NeedsActiveExecutionCmd raises FlowableObjectNotFoundException (404).
    let mut session = store.create_session().unwrap();
    let mut ended_instance = store
        .find_process_instance(&process_instance_id, &mut session)
        .unwrap();
    ended_instance.is_suspended = false;
    ended_instance.is_ended = true;
    store.update_process_instance(&ended_instance, &mut session);
    session.flush_and_commit().unwrap();

    let ended_response = client
        .post(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}/evaluate-conditions"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(ended_response.status(), 404);
}
