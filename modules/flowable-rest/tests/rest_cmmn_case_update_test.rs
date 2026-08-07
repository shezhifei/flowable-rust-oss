// P95: CMMN REST write surface — PUT case-instance, PUT plan-item actions,
// case variable sync write (POST/PUT/DELETE collection + PUT/DELETE single).
//
// Java references:
// - CaseInstanceResource.java:88
// - PlanItemInstanceResource.java:59
// - CaseInstanceVariableCollectionResource.java:83/141/180
// - CaseInstanceVariableResource.java:88/176

use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_rest::run_server;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::net::TcpListener;

async fn spawn_server() -> (String, reqwest::Client) {
    let engine = Arc::new(ProcessEngine::new("cmmn-case-update-test".to_string()));
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

    (base_url, reqwest::Client::new())
}

const BASIC_CMMN: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/CMMN/20151109/MODEL"
             xmlns:flowable="http://flowable.org/cmmn"
             targetNamespace="Examples">
    <case id="updateCase" name="Update Case">
        <casePlanModel id="casePlanModel">
            <planItem id="planItem1" name="Task 1" definitionRef="humanTask1" />
            <planItem id="planItem2" name="Task 2" definitionRef="humanTask2" />
            <humanTask id="humanTask1" name="Task 1" />
            <humanTask id="humanTask2" name="Task 2" />
        </casePlanModel>
    </case>
</definitions>"#;

const MANUAL_ACTIVATION_CMMN: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/CMMN/20151109/MODEL"
             xmlns:flowable="http://flowable.org/cmmn"
             targetNamespace="Examples">
    <case id="manualCase" name="Manual Activation Case">
        <casePlanModel id="casePlanModel">
            <planItem id="planItemManual" name="Manual task" definitionRef="humanTaskManual">
                <itemControl>
                    <manualActivationRule>
                        <condition><![CDATA[${true}]]></condition>
                    </manualActivationRule>
                </itemControl>
            </planItem>
            <humanTask id="humanTaskManual" name="Manual task" />
        </casePlanModel>
    </case>
</definitions>"#;

async fn deploy_cmmn(client: &reqwest::Client, base_url: &str, resource: &str, name: &str) {
    let response = client
        .post(format!("{base_url}/cmmn-repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": name,
            "resourceName": "test-case.cmmn",
            "resource": resource
        }))
        .send()
        .await
        .unwrap();
    assert!(
        response.status().is_success(),
        "deploy failed: {}",
        response.text().await.unwrap_or_default()
    );
}

async fn start_case(client: &reqwest::Client, base_url: &str, key: &str) -> String {
    let response = client
        .post(format!("{base_url}/cmmn-runtime/case-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "caseDefinitionKey": key }))
        .send()
        .await
        .unwrap();
    assert!(
        response.status().is_success(),
        "start case failed: {}",
        response.text().await.unwrap_or_default()
    );
    let body: Value = response.json().await.unwrap();
    body["id"].as_str().unwrap().to_string()
}

async fn first_plan_item(
    client: &reqwest::Client,
    base_url: &str,
    case_id: &str,
) -> (String, String) {
    let response = client
        .get(format!(
            "{base_url}/cmmn-runtime/plan-item-instances?caseInstanceId={case_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success());
    let body: Value = response.json().await.unwrap();
    let item = &body["data"].as_array().unwrap()[0];
    (
        item["id"].as_str().unwrap().to_string(),
        item["state"].as_str().unwrap().to_string(),
    )
}

#[tokio::test]
async fn put_case_instance_updates_name_and_business_key() {
    // Java: CaseInstanceResource.java:114-117
    let (base_url, client) = spawn_server().await;
    deploy_cmmn(&client, &base_url, BASIC_CMMN, "Update Name Test").await;
    let case_id = start_case(&client, &base_url, "updateCase").await;

    let response = client
        .put(format!("{base_url}/cmmn-runtime/case-instances/{case_id}"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "Renamed via REST",
            "businessKey": "BK-REST-1"
        }))
        .send()
        .await
        .unwrap();
    assert!(
        response.status().is_success(),
        "put case failed: {}",
        response.text().await.unwrap_or_default()
    );
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["name"].as_str().unwrap(), "Renamed via REST");
    assert_eq!(body["businessKey"].as_str().unwrap(), "BK-REST-1");
}

#[tokio::test]
async fn put_case_instance_evaluate_criteria() {
    // Java: CaseInstanceResource.java:101
    let (base_url, client) = spawn_server().await;
    deploy_cmmn(&client, &base_url, BASIC_CMMN, "Evaluate Criteria Test").await;
    let case_id = start_case(&client, &base_url, "updateCase").await;

    let response = client
        .put(format!("{base_url}/cmmn-runtime/case-instances/{case_id}"))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "action": "evaluateCriteria" }))
        .send()
        .await
        .unwrap();
    assert!(
        response.status().is_success(),
        "evaluateCriteria failed: {}",
        response.text().await.unwrap_or_default()
    );
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["id"].as_str().unwrap(), case_id);
}

#[tokio::test]
async fn put_plan_item_disable_and_enable() {
    // Java DisablePlanItemInstanceCmd.java:44-45 requires ENABLED before the
    // REST disable/enable action pair.
    let (base_url, client) = spawn_server().await;
    deploy_cmmn(
        &client,
        &base_url,
        MANUAL_ACTIVATION_CMMN,
        "Plan Item Action Test",
    )
    .await;
    let case_id = start_case(&client, &base_url, "manualCase").await;
    let (task_id, _) = first_plan_item(&client, &base_url, &case_id).await;

    let disable = client
        .put(format!(
            "{base_url}/cmmn-runtime/plan-item-instances/{task_id}"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "action": "disable" }))
        .send()
        .await
        .unwrap();
    assert!(
        disable.status().is_success(),
        "disable failed: {}",
        disable.text().await.unwrap_or_default()
    );
    let disabled: Value = disable.json().await.unwrap();
    assert_eq!(disabled["state"].as_str().unwrap(), "DISABLED");

    let enable = client
        .put(format!(
            "{base_url}/cmmn-runtime/plan-item-instances/{task_id}"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "action": "enable" }))
        .send()
        .await
        .unwrap();
    assert!(
        enable.status().is_success(),
        "enable failed: {}",
        enable.text().await.unwrap_or_default()
    );
    let enabled: Value = enable.json().await.unwrap();
    // EnablePlanItemInstanceOperation.java:39-51 stores ENABLED.
    assert_eq!(enabled["state"].as_str().unwrap(), "ENABLED");
}

#[tokio::test]
async fn put_plan_item_trigger_and_start_manual_task() {
    // Java StartPlanItemInstanceCmd.java:54-58: ENABLED -> ACTIVE.
    let (base_url, client) = spawn_server().await;
    deploy_cmmn(
        &client,
        &base_url,
        MANUAL_ACTIVATION_CMMN,
        "Manual Activation Test",
    )
    .await;
    let case_id = start_case(&client, &base_url, "manualCase").await;
    let (task_id, state) = first_plan_item(&client, &base_url, &case_id).await;
    assert_eq!(state, "ENABLED", "manual activation parks as ENABLED");

    // trigger
    let trigger = client
        .put(format!(
            "{base_url}/cmmn-runtime/plan-item-instances/{task_id}"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "action": "trigger" }))
        .send()
        .await
        .unwrap();
    // First case already triggered — start a second case for start action
    if trigger.status().is_success() {
        let body: Value = trigger.json().await.unwrap();
        assert_eq!(body["state"].as_str().unwrap(), "ACTIVE");
    } else {
        panic!(
            "trigger failed: {}",
            trigger.text().await.unwrap_or_default()
        );
    }

    // Second case for start action
    let case_id2 = start_case(&client, &base_url, "manualCase").await;
    let (task_id2, _) = first_plan_item(&client, &base_url, &case_id2).await;
    let start = client
        .put(format!(
            "{base_url}/cmmn-runtime/plan-item-instances/{task_id2}"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "action": "start" }))
        .send()
        .await
        .unwrap();
    assert!(
        start.status().is_success(),
        "start failed: {}",
        start.text().await.unwrap_or_default()
    );
    let started: Value = start.json().await.unwrap();
    assert_eq!(started["state"].as_str().unwrap(), "ACTIVE");
}

#[tokio::test]
async fn post_case_variables_creates() {
    // Java: CaseInstanceVariableCollectionResource.java:141
    let (base_url, client) = spawn_server().await;
    deploy_cmmn(&client, &base_url, BASIC_CMMN, "Var Post Test").await;
    let case_id = start_case(&client, &base_url, "updateCase").await;

    let response = client
        .post(format!(
            "{base_url}/cmmn-runtime/case-instances/{case_id}/variables"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!([
            { "name": "alpha", "type": "string", "value": "one" },
            { "name": "beta", "type": "integer", "value": 2 }
        ]))
        .send()
        .await
        .unwrap();
    assert_eq!(
        response.status().as_u16(),
        201,
        "post variables failed: {}",
        response.text().await.unwrap_or_default()
    );

    let list = client
        .get(format!(
            "{base_url}/cmmn-runtime/case-instances/{case_id}/variables"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    let vars: Value = list.json().await.unwrap();
    let arr = vars.as_array().unwrap();
    assert!(arr.iter().any(|v| v["name"] == "alpha"));
    assert!(arr.iter().any(|v| v["name"] == "beta"));
}

#[tokio::test]
async fn put_case_variables_bulk_updates() {
    // Java: CaseInstanceVariableCollectionResource.java:83
    let (base_url, client) = spawn_server().await;
    deploy_cmmn(&client, &base_url, BASIC_CMMN, "Var Put Bulk Test").await;
    let case_id = start_case(&client, &base_url, "updateCase").await;

    let response = client
        .put(format!(
            "{base_url}/cmmn-runtime/case-instances/{case_id}/variables"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "name": "bulk", "type": "string", "value": "v1" }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        response.status().as_u16(),
        201,
        "put variables failed: {}",
        response.text().await.unwrap_or_default()
    );

    let single = client
        .get(format!(
            "{base_url}/cmmn-runtime/case-instances/{case_id}/variables/bulk"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(single.status().is_success());
    let var: Value = single.json().await.unwrap();
    assert_eq!(var["value"].as_str().unwrap(), "v1");
}

#[tokio::test]
async fn put_single_case_variable_updates() {
    // Java: CaseInstanceVariableResource.java:88
    let (base_url, client) = spawn_server().await;
    deploy_cmmn(&client, &base_url, BASIC_CMMN, "Var Put Single Test").await;
    let case_id = start_case(&client, &base_url, "updateCase").await;

    let create = client
        .post(format!(
            "{base_url}/cmmn-runtime/case-instances/{case_id}/variables"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "name": "score", "type": "integer", "value": 1 }))
        .send()
        .await
        .unwrap();
    assert_eq!(create.status().as_u16(), 201);

    let update = client
        .put(format!(
            "{base_url}/cmmn-runtime/case-instances/{case_id}/variables/score"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "name": "score", "type": "integer", "value": 99 }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        update.status().as_u16(),
        201,
        "put single variable failed: {}",
        update.text().await.unwrap_or_default()
    );
    let body: Value = update.json().await.unwrap();
    assert_eq!(body["value"].as_i64().unwrap(), 99);
}

#[tokio::test]
async fn delete_single_case_variable() {
    // Java: CaseInstanceVariableResource.java:176
    let (base_url, client) = spawn_server().await;
    deploy_cmmn(&client, &base_url, BASIC_CMMN, "Var Delete Single Test").await;
    let case_id = start_case(&client, &base_url, "updateCase").await;

    client
        .post(format!(
            "{base_url}/cmmn-runtime/case-instances/{case_id}/variables"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "name": "temp", "type": "string", "value": "x" }))
        .send()
        .await
        .unwrap();

    let delete = client
        .delete(format!(
            "{base_url}/cmmn-runtime/case-instances/{case_id}/variables/temp"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(
        delete.status().as_u16(),
        204,
        "delete single failed: {}",
        delete.text().await.unwrap_or_default()
    );

    let get = client
        .get(format!(
            "{base_url}/cmmn-runtime/case-instances/{case_id}/variables/temp"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(get.status().as_u16(), 404);
}

#[tokio::test]
async fn delete_all_case_variables() {
    // Java: CaseInstanceVariableCollectionResource.java:180
    let (base_url, client) = spawn_server().await;
    deploy_cmmn(&client, &base_url, BASIC_CMMN, "Var Delete All Test").await;
    let case_id = start_case(&client, &base_url, "updateCase").await;

    client
        .post(format!(
            "{base_url}/cmmn-runtime/case-instances/{case_id}/variables"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!([
            { "name": "a", "type": "string", "value": "1" },
            { "name": "b", "type": "string", "value": "2" }
        ]))
        .send()
        .await
        .unwrap();

    let delete = client
        .delete(format!(
            "{base_url}/cmmn-runtime/case-instances/{case_id}/variables"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(
        delete.status().as_u16(),
        204,
        "delete all failed: {}",
        delete.text().await.unwrap_or_default()
    );

    let list = client
        .get(format!(
            "{base_url}/cmmn-runtime/case-instances/{case_id}/variables"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    let vars: Value = list.json().await.unwrap();
    assert!(vars.as_array().unwrap().is_empty());
}
