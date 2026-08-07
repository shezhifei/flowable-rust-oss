use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_rest::run_server;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::net::TcpListener;

const DECISION_DMN: &str = r#"
<definitions xmlns="https://www.omg.org/spec/DMN/20191111/MODEL/"
             id="loan-defs"
             name="Loan Decisions"
             namespace="http://flowable.org/dmn">
  <decision id="loanEligibility" name="Loan Eligibility">
    <decisionTable id="loanDecisionTable" hitPolicy="FIRST">
      <input id="input1" label="Credit score">
        <inputExpression id="inputExpression1" typeRef="number">
          <text>creditScore</text>
        </inputExpression>
      </input>
      <output id="output1" label="Approved" name="approved" typeRef="boolean" />
      <rule id="rule1">
        <inputEntry id="inputEntry1"><text>730</text></inputEntry>
        <outputEntry id="outputEntry1"><text>true</text></outputEntry>
      </rule>
    </decisionTable>
  </decision>
</definitions>
"#;

const PROCESS_WITH_DECISION: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
    <process id="loanDecisionProcess" name="Loan Decision Process" isExecutable="true">
        <startEvent id="start" />
        <sequenceFlow id="flow1" sourceRef="start" targetRef="eligibility" />
        <businessRuleTask id="eligibility" name="Eligibility" flowable:decisionRef="loanEligibility" />
        <sequenceFlow id="flow2" sourceRef="eligibility" targetRef="end" />
        <endEvent id="end" />
    </process>
</definitions>"#;

const SIMPLE_PROCESS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             targetNamespace="Examples">
    <process id="deleteDeploymentProcess" name="Delete Deployment Process" isExecutable="true">
        <startEvent id="start" />
        <sequenceFlow id="flow1" sourceRef="start" targetRef="end" />
        <endEvent id="end" />
    </process>
</definitions>"#;

const RUNTIME_CASCADE_PROCESS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             targetNamespace="Examples">
    <process id="cascadeUserTaskProcess" name="Cascade User Task Process" isExecutable="true">
        <startEvent id="start" />
        <sequenceFlow id="flow1" sourceRef="start" targetRef="review" />
        <userTask id="review" name="Review" />
        <sequenceFlow id="flow2" sourceRef="review" targetRef="end" />
        <endEvent id="end" />
    </process>
    <process id="cascadeMessageWaitProcess" name="Cascade Message Wait Process" isExecutable="true">
        <startEvent id="start" />
        <sequenceFlow id="flow1" sourceRef="start" targetRef="waitForMessage" />
        <intermediateCatchEvent id="waitForMessage" name="Wait For Message">
            <messageEventDefinition messageRef="cascadeApprovalMessage" />
        </intermediateCatchEvent>
        <sequenceFlow id="flow2" sourceRef="waitForMessage" targetRef="end" />
        <endEvent id="end" />
    </process>
    <process id="cascadeTimerWaitProcess" name="Cascade Timer Wait Process" isExecutable="true">
        <startEvent id="start" />
        <sequenceFlow id="flow1" sourceRef="start" targetRef="waitForTimer" />
        <intermediateCatchEvent id="waitForTimer" name="Wait For Timer">
            <timerEventDefinition>
                <timeDuration>PT5M</timeDuration>
            </timerEventDefinition>
        </intermediateCatchEvent>
        <sequenceFlow id="flow2" sourceRef="waitForTimer" targetRef="end" />
        <endEvent id="end" />
    </process>
</definitions>"#;

async fn spawn_server(test_name: &str) -> (Arc<ProcessEngine>, String, reqwest::Client) {
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

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let engine_for_server = Arc::clone(&engine);
    tokio::spawn(async move {
        run_server(engine_for_server, listener).await.unwrap();
    });

    (engine, base_url, reqwest::Client::new())
}

#[tokio::test]
async fn repository_deployment_delete_cascade_removes_reachable_runtime_state() {
    let (engine, base_url, client) = spawn_server("rest-bpmn-repository-deployment-cascade").await;

    let deploy_response = client
        .post(format!("{base_url}/repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "Cascade runtime deployment",
            "resourceName": "cascade-runtime.bpmn20.xml",
            "resource": RUNTIME_CASCADE_PROCESS
        }))
        .send()
        .await
        .unwrap();
    // P109: deploy (both JSON superset and multipart paths) returns 201.
    assert_eq!(deploy_response.status(), reqwest::StatusCode::CREATED);
    let deployment: Value = deploy_response.json().await.unwrap();
    let deployment_id = deployment["id"].as_str().unwrap();

    let user_task_definition_id = latest_definition_id(&engine, "cascadeUserTaskProcess");
    let message_wait_definition_id = latest_definition_id(&engine, "cascadeMessageWaitProcess");
    let timer_wait_definition_id = latest_definition_id(&engine, "cascadeTimerWaitProcess");

    let user_task_instance_id =
        start_instance(&client, &base_url, &user_task_definition_id, "user-task").await;
    let message_wait_instance_id = start_instance(
        &client,
        &base_url,
        &message_wait_definition_id,
        "message-wait",
    )
    .await;
    let timer_wait_instance_id =
        start_instance(&client, &base_url, &timer_wait_definition_id, "timer-wait").await;

    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    assert_eq!(
        store
            .find_tasks_by_process_instance_id(&user_task_instance_id, &mut session)
            .len(),
        1
    );
    assert!(
        store
            .snapshot_event_wait_states(&mut session)
            .into_values()
            .any(|wait| wait.process_instance_id == message_wait_instance_id)
    );
    assert!(
        store
            .snapshot_timer_job_states(&mut session)
            .into_values()
            .any(|timer| timer.process_instance_id == timer_wait_instance_id)
    );
    let _ = session.rollback();

    let delete_response = client
        .delete(format!(
            "{base_url}/repository/deployments/{deployment_id}?cascade=true"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(delete_response.status(), reqwest::StatusCode::NO_CONTENT);

    for process_instance_id in [
        user_task_instance_id.as_str(),
        message_wait_instance_id.as_str(),
        timer_wait_instance_id.as_str(),
    ] {
        let get_runtime_response = client
            .get(format!(
                "{base_url}/runtime/process-instances/{process_instance_id}"
            ))
            .basic_auth("admin", Some("test"))
            .send()
            .await
            .unwrap();
        assert_eq!(
            get_runtime_response.status(),
            reqwest::StatusCode::NOT_FOUND
        );

        let tasks_response = client
            .post(format!("{base_url}/query/tasks"))
            .basic_auth("admin", Some("test"))
            .json(&json!({
                "processInstanceId": process_instance_id
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(tasks_response.status(), reqwest::StatusCode::OK);
        let tasks_body: Value = tasks_response.json().await.unwrap();
        assert_eq!(tasks_body["total"], 0);
    }

    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    assert!(store.snapshot_event_wait_states(&mut session).is_empty());
    assert!(store.snapshot_timer_job_states(&mut session).is_empty());
    let _ = session.rollback();

    let definitions_after_delete = client
        .get(format!(
            "{base_url}/repository/process-definitions?deploymentId={deployment_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(definitions_after_delete.status(), reqwest::StatusCode::OK);
    let definitions_body: Value = definitions_after_delete.json().await.unwrap();
    assert_eq!(definitions_body["total"], 0);
}

/// Mirrors Flowable Java's `DeploymentEntityManagerImpl.deleteDeployment` cascade
/// path: when `cascade=true`, the engine must delete the runtime process
/// instances AND the historic process instance / historic task rows entirely
/// (via `recordProcessInstanceDeleted` + `deleteHistoricTask`), not just mark
/// them as ended.
#[tokio::test]
async fn repository_deployment_delete_cascade_purges_historic_state() {
    let (engine, base_url, client) =
        spawn_server("rest-bpmn-repository-deployment-cascade-history").await;

    let deploy_response = client
        .post(format!("{base_url}/repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "Cascade history deployment",
            "resourceName": "cascade-history.bpmn20.xml",
            "resource": RUNTIME_CASCADE_PROCESS
        }))
        .send()
        .await
        .unwrap();
    // P109: deploy (both JSON superset and multipart paths) returns 201.
    assert_eq!(deploy_response.status(), reqwest::StatusCode::CREATED);
    let deployment: Value = deploy_response.json().await.unwrap();
    let deployment_id = deployment["id"].as_str().unwrap();

    let user_task_definition_id = latest_definition_id(&engine, "cascadeUserTaskProcess");
    let process_instance_id = start_instance(
        &client,
        &base_url,
        &user_task_definition_id,
        "historic-cascade",
    )
    .await;

    let task_id = {
        let tasks_response = client
            .get(format!(
                "{base_url}/runtime/tasks?processInstanceId={process_instance_id}"
            ))
            .basic_auth("admin", Some("test"))
            .send()
            .await
            .unwrap();
        assert!(tasks_response.status().is_success());
        let body: Value = tasks_response.json().await.unwrap();
        body["data"][0]["id"].as_str().unwrap().to_string()
    };

    // Sanity: historic PI exists and is active (no end time) before cascade delete.
    let historic_before = client
        .get(format!(
            "{base_url}/history/historic-process-instances/{process_instance_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(historic_before.status(), reqwest::StatusCode::OK);
    let historic_before_body: Value = historic_before.json().await.unwrap();
    assert_eq!(historic_before_body["id"], process_instance_id);
    assert!(historic_before_body["endTime"].is_null());

    // Sanity: historic task exists for the running user task.
    let historic_task_before = client
        .get(format!(
            "{base_url}/history/historic-task-instances?processInstanceId={process_instance_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(historic_task_before.status(), reqwest::StatusCode::OK);
    let historic_task_before_body: Value = historic_task_before.json().await.unwrap();
    assert_eq!(historic_task_before_body["total"], 1);
    assert_eq!(historic_task_before_body["data"][0]["id"], task_id.as_str());

    let delete_response = client
        .delete(format!(
            "{base_url}/repository/deployments/{deployment_id}?cascade=true"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(delete_response.status(), reqwest::StatusCode::NO_CONTENT);

    // Historic process instance must be GONE (404), not just marked ended.
    // Java's `recordProcessInstanceDeleted` deletes the row entirely when cascade=true.
    let historic_after = client
        .get(format!(
            "{base_url}/history/historic-process-instances/{process_instance_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(
        historic_after.status(),
        reqwest::StatusCode::NOT_FOUND,
        "cascade=true must delete the historic process instance row"
    );

    // Historic task instance must be GONE (0 results), matching Java's
    // `TaskHelper.deleteHistoricTask` invoked with cascade=true.
    let historic_task_after = client
        .get(format!(
            "{base_url}/history/historic-task-instances?processInstanceId={process_instance_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(historic_task_after.status(), reqwest::StatusCode::OK);
    let historic_task_after_body: Value = historic_task_after.json().await.unwrap();
    assert_eq!(
        historic_task_after_body["total"], 0,
        "cascade=true must delete historic task instances for the purged PI"
    );

    // Historic activity instances for the PI must also be gone (Java:
    // `deleteHistoricActivityInstancesByProcessInstanceId`).
    let historic_activity_after = client
        .get(format!(
            "{base_url}/history/historic-activity-instances?processInstanceId={process_instance_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(historic_activity_after.status(), reqwest::StatusCode::OK);
    let historic_activity_after_body: Value = historic_activity_after.json().await.unwrap();
    assert_eq!(historic_activity_after_body["total"], 0);
}

#[tokio::test]
async fn repository_deployment_delete_removes_repository_artifacts_and_supports_service_alias() {
    let (engine, base_url, client) = spawn_server("rest-bpmn-repository-deployment-delete").await;

    let deploy_response = client
        .post(format!("{base_url}/repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "Delete deployment",
            "resourceName": "delete-deployment.bpmn20.xml",
            "resource": SIMPLE_PROCESS
        }))
        .send()
        .await
        .unwrap();
    // P109: deploy (both JSON superset and multipart paths) returns 201.
    assert_eq!(deploy_response.status(), reqwest::StatusCode::CREATED);
    let deployment: Value = deploy_response.json().await.unwrap();
    let deployment_id = deployment["id"].as_str().unwrap();
    let process_definition_id = engine
        .get_repository_service()
        .latest_process_definition_by_key("deleteDeploymentProcess", None)
        .unwrap()
        .unwrap()
        .id;

    let service_delete = client
        .delete(format!(
            "{base_url}/repository/deployments/{deployment_id}?cascade=true"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(service_delete.status(), reqwest::StatusCode::NO_CONTENT);

    let deleted_deployment = client
        .get(format!("{base_url}/repository/deployments/{deployment_id}"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(deleted_deployment.status(), reqwest::StatusCode::NOT_FOUND);
    let deleted_deployment_body: Value = deleted_deployment.json().await.unwrap();
    assert_eq!(deleted_deployment_body["code"], "NOT_FOUND");
    assert!(
        deleted_deployment_body["details"]
            .as_str()
            .unwrap()
            .contains(deployment_id)
    );

    let deleted_resources = client
        .get(format!(
            "{base_url}/repository/deployments/{deployment_id}/resources"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(deleted_resources.status(), reqwest::StatusCode::NOT_FOUND);

    let definitions_after_delete = client
        .get(format!(
            "{base_url}/repository/process-definitions?deploymentId={deployment_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(definitions_after_delete.status(), reqwest::StatusCode::OK);
    let definitions_body: Value = definitions_after_delete.json().await.unwrap();
    assert_eq!(definitions_body["total"], 0);
    assert!(definitions_body["data"].as_array().unwrap().is_empty());

    for suffix in ["resourcedata", "model"] {
        let process_artifact_response = client
            .get(format!(
                "{base_url}/repository/process-definitions/{process_definition_id}/{suffix}"
            ))
            .basic_auth("admin", Some("test"))
            .send()
            .await
            .unwrap();
        assert_eq!(
            process_artifact_response.status(),
            reqwest::StatusCode::NOT_FOUND
        );
    }

    let missing_delete = client
        .delete(format!("{base_url}/repository/deployments/{deployment_id}"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(missing_delete.status(), reqwest::StatusCode::NOT_FOUND);
    let missing_delete_body: Value = missing_delete.json().await.unwrap();
    assert_eq!(missing_delete_body["code"], "NOT_FOUND");
    assert!(
        missing_delete_body["details"]
            .as_str()
            .unwrap()
            .contains(deployment_id)
    );
}

fn latest_definition_id(engine: &ProcessEngine, key: &str) -> String {
    engine
        .get_repository_service()
        .latest_process_definition_by_key(key, None)
        .unwrap()
        .unwrap()
        .id
}

async fn start_instance(
    client: &reqwest::Client,
    base_url: &str,
    process_definition_id: &str,
    business_key: &str,
) -> String {
    let response = client
        .post(format!("{base_url}/runtime/process-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "processDefinitionId": process_definition_id,
            "businessKey": business_key
        }))
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success());
    let body: Value = response.json().await.unwrap();
    body["id"].as_str().unwrap().to_string()
}

#[tokio::test]
async fn process_definition_decision_endpoints_follow_business_rule_task_refs() {
    let (engine, base_url, client) = spawn_server("rest-bpmn-repository-lifecycle").await;

    let dmn_deploy = client
        .post(format!("{base_url}/dmn-repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "Loan decisions",
            "resourceName": "loan-eligibility.dmn",
            "resource": DECISION_DMN
        }))
        .send()
        .await
        .unwrap();
    assert!(dmn_deploy.status().is_success());

    let bpmn_deploy = client
        .post(format!("{base_url}/repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "Loan decision process",
            "resourceName": "loan-decision-process.bpmn20.xml",
            "resource": PROCESS_WITH_DECISION
        }))
        .send()
        .await
        .unwrap();
    assert!(bpmn_deploy.status().is_success());

    let process_definition_id = engine
        .get_repository_service()
        .latest_process_definition_by_key("loanDecisionProcess", None)
        .unwrap()
        .unwrap()
        .id;

    for suffix in ["decision-tables", "decisions"] {
        let response = client
            .get(format!(
                "{base_url}/repository/process-definitions/{process_definition_id}/{suffix}"
            ))
            .basic_auth("admin", Some("test"))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::OK);
        let body: Value = response.json().await.unwrap();
        assert_eq!(body["total"], 1);
        assert_eq!(body["data"][0]["key"], "loanEligibility");
        assert_eq!(body["data"][0]["name"], "Loan Eligibility");
    }
}
