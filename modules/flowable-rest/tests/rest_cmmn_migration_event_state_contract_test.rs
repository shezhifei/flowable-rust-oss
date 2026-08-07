use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_rest::run_server;
use reqwest::StatusCode;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::net::TcpListener;

const MIGRATION_CMMN: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/CMMN/20151109/MODEL"
             targetNamespace="http://flowable.org/cmmn">
  <case id="migrationCase" name="Migration Case">
    <casePlanModel id="migrationPlan" name="Migration Plan" autoComplete="false">
      <planItem id="planItemReview" name="Review" definitionRef="reviewTask" />
      <humanTask id="reviewTask" name="Review" isBlocking="true" />
    </casePlanModel>
  </case>
</definitions>"#;

const OTHER_CMMN: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/CMMN/20151109/MODEL"
             targetNamespace="http://flowable.org/cmmn">
  <case id="otherMigrationCase" name="Other Migration Case">
    <casePlanModel id="otherMigrationPlan" name="Other Migration Plan" autoComplete="false">
      <planItem id="planItemApprove" name="Approve" definitionRef="approveTask" />
      <humanTask id="approveTask" name="Approve" isBlocking="true" />
    </casePlanModel>
  </case>
</definitions>"#;

const TWO_TASK_CMMN: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/CMMN/20151109/MODEL"
             targetNamespace="http://flowable.org/cmmn">
  <case id="twoTaskCase" name="Two Task Case">
    <casePlanModel id="twoTaskPlan" name="Two Task Plan" autoComplete="false">
      <planItem id="planItemReview" name="Review" definitionRef="reviewTask" />
      <planItem id="planItemApprove" name="Approve" definitionRef="approveTask" />
      <humanTask id="reviewTask" name="Review" isBlocking="true" />
      <humanTask id="approveTask" name="Approve" isBlocking="true" />
    </casePlanModel>
  </case>
</definitions>"#;

const EVENT_LISTENER_CMMN: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/CMMN/20151109/MODEL"
             targetNamespace="http://flowable.org/cmmn">
  <case id="eventListenerCase" name="Event Listener Case">
    <casePlanModel id="eventListenerPlan" name="Event Listener Plan" autoComplete="false">
      <planItem id="planItemApprovalEvent" name="Approval Event" definitionRef="approvalEventListener" />
      <eventListener id="approvalEventListener" name="Wait for approval" eventType="message" eventName="approvalReceived" />
    </casePlanModel>
  </case>
</definitions>"#;

async fn spawn_server(test_name: &str) -> (String, reqwest::Client) {
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

    tokio::spawn(async move {
        run_server(engine, listener).await.unwrap();
    });

    (base_url, reqwest::Client::new())
}

async fn deploy_definition(
    base_url: &str,
    client: &reqwest::Client,
    name: &str,
    resource_name: &str,
    resource: &str,
) -> String {
    let response = client
        .post(format!("{base_url}/cmmn-repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": name,
            "resourceName": resource_name,
            "resource": resource
        }))
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success());

    let definitions_response = client
        .get(format!(
            "{base_url}/cmmn-repository/case-definitions?start=0&size=20"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(definitions_response.status(), StatusCode::OK);
    let body: Value = definitions_response.json().await.unwrap();
    body["data"]
        .as_array()
        .unwrap()
        .iter()
        .find(|definition| definition["resourceName"] == resource_name)
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string()
}

async fn start_case(base_url: &str, client: &reqwest::Client, case_definition_id: &str) -> String {
    let response = client
        .post(format!("{base_url}/cmmn-runtime/case-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "caseDefinitionId": case_definition_id,
            "businessKey": "migration-bk"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    response.json::<Value>().await.unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string()
}

#[tokio::test]
async fn cmmn_runtime_migration_validates_and_executes_safe_noop() {
    let (base_url, client) = spawn_server("rest-cmmn-migration-runtime-noop").await;
    let case_definition_id = deploy_definition(
        &base_url,
        &client,
        "Migration Deployment",
        "migration.cmmn",
        MIGRATION_CMMN,
    )
    .await;
    let case_instance_id = start_case(&base_url, &client, &case_definition_id).await;

    let validation_response = client
        .post(format!(
            "{base_url}/cmmn-runtime/case-instances/{case_instance_id}/validate-migration"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "targetCaseDefinitionId": case_definition_id
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(validation_response.status(), StatusCode::OK);
    let validation_body: Value = validation_response.json().await.unwrap();
    assert_eq!(validation_body["valid"], true);
    assert_eq!(validation_body["validationMessages"], json!([]));

    let migrate_response = client
        .post(format!(
            "{base_url}/cmmn-runtime/case-instances/{case_instance_id}/migrate"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "targetCaseDefinitionId": case_definition_id
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(migrate_response.status(), StatusCode::OK);

    let case_response = client
        .get(format!(
            "{base_url}/cmmn-runtime/case-instances/{case_instance_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(case_response.status(), StatusCode::OK);
    let case_body: Value = case_response.json().await.unwrap();
    assert_eq!(case_body["caseDefinitionId"], case_definition_id);
    assert_eq!(case_body["state"], "ACTIVE");
}

#[tokio::test]
async fn cmmn_runtime_migration_rejects_active_cross_definition_instances() {
    let (base_url, client) = spawn_server("rest-cmmn-migration-runtime-cross-def").await;
    let source_definition_id = deploy_definition(
        &base_url,
        &client,
        "Source Migration Deployment",
        "migration-source.cmmn",
        MIGRATION_CMMN,
    )
    .await;
    let target_definition_id = deploy_definition(
        &base_url,
        &client,
        "Target Migration Deployment",
        "migration-target.cmmn",
        OTHER_CMMN,
    )
    .await;
    let case_instance_id = start_case(&base_url, &client, &source_definition_id).await;

    let validation_response = client
        .post(format!(
            "{base_url}/cmmn-runtime/case-instances/{case_instance_id}/validate-migration"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "targetCaseDefinitionId": target_definition_id
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(validation_response.status(), StatusCode::OK);
    let validation_body: Value = validation_response.json().await.unwrap();
    assert_eq!(validation_body["valid"], false);
    assert!(
        validation_body["validationMessages"].as_array().unwrap()[0]
            .as_str()
            .unwrap()
            .contains("active plan item")
    );

    let migrate_response = client
        .post(format!(
            "{base_url}/cmmn-runtime/case-instances/{case_instance_id}/migrate"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "targetCaseDefinitionId": target_definition_id
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(migrate_response.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn cmmn_repository_and_historic_migration_paths_execute_safe_noop() {
    let (base_url, client) = spawn_server("rest-cmmn-migration-repository-history").await;
    let case_definition_id = deploy_definition(
        &base_url,
        &client,
        "Migration Deployment",
        "migration.cmmn",
        MIGRATION_CMMN,
    )
    .await;
    let case_instance_id = start_case(&base_url, &client, &case_definition_id).await;

    let tasks_response = client
        .get(format!(
            "{base_url}/cmmn-runtime/tasks?caseInstanceId={case_instance_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    let tasks_body: Value = tasks_response.json().await.unwrap();
    let task_id = tasks_body["data"][0]["id"].as_str().unwrap();

    let complete_response = client
        .post(format!(
            "{base_url}/cmmn-runtime/plan-item-instances/{task_id}/complete"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(complete_response.status(), StatusCode::OK);

    for path in [
        format!("/cmmn-repository/case-definitions/{case_definition_id}/migrate"),
        format!("/cmmn-repository/case-definitions/{case_definition_id}/batch-migrate"),
        format!(
            "/cmmn-repository/case-definitions/{case_definition_id}/migrate-historic-instances"
        ),
        format!(
            "/cmmn-repository/case-definitions/{case_definition_id}/batch-migrate-historic-instances"
        ),
        format!("/cmmn-history/historic-case-instances/{case_instance_id}/migrate"),
    ] {
        let response = client
            .post(format!("{base_url}{path}"))
            .basic_auth("admin", Some("test"))
            .json(&json!({
                "targetCaseDefinitionId": case_definition_id
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK, "path {path}");
    }
}

#[tokio::test]
async fn cmmn_runtime_event_subscriptions_are_created_listed_and_cleaned() {
    let (base_url, client) = spawn_server("rest-cmmn-event-subscriptions").await;
    let case_definition_id = deploy_definition(
        &base_url,
        &client,
        "Event Listener Deployment",
        "event-listener.cmmn",
        EVENT_LISTENER_CMMN,
    )
    .await;
    let case_instance_id = start_case(&base_url, &client, &case_definition_id).await;

    let list_response = client
        .get(format!(
            "{base_url}/cmmn-runtime/event-subscriptions?eventType=message&start=0&size=10"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(list_response.status(), StatusCode::OK);
    let list_body: Value = list_response.json().await.unwrap();
    assert_eq!(list_body["start"], 0);
    assert_eq!(list_body["size"], 1);
    assert_eq!(list_body["total"], 1);
    let subscription = &list_body["data"][0];
    assert_eq!(subscription["eventType"], "message");
    assert_eq!(subscription["eventName"], "approvalReceived");
    assert_eq!(subscription["activityId"], "approvalEventListener");
    assert_eq!(subscription["caseInstanceId"], case_instance_id);
    assert_eq!(subscription["caseDefinitionId"], case_definition_id);
    assert_eq!(subscription["planItemInstanceId"], "planItemApprovalEvent");
    let event_subscription_id = subscription["id"].as_str().unwrap();

    let get_response = client
        .get(format!(
            "{base_url}/cmmn-runtime/event-subscriptions/{event_subscription_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(get_response.status(), StatusCode::OK);
    let get_body: Value = get_response.json().await.unwrap();
    assert_eq!(get_body["id"], event_subscription_id);

    let delete_response = client
        .delete(format!(
            "{base_url}/cmmn-runtime/case-instances/{case_instance_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(delete_response.status(), StatusCode::NO_CONTENT);

    let cleaned_response = client
        .get(format!(
            "{base_url}/cmmn-runtime/event-subscriptions?caseInstanceId={case_instance_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(cleaned_response.status(), StatusCode::OK);
    let cleaned_body: Value = cleaned_response.json().await.unwrap();
    assert_eq!(cleaned_body["total"], 0);
    assert_eq!(cleaned_body["data"], json!([]));
}

#[tokio::test]
async fn cmmn_change_state_supports_terminating_active_plan_item_subset() {
    let (base_url, client) = spawn_server("rest-cmmn-change-state-terminate").await;
    let case_definition_id = deploy_definition(
        &base_url,
        &client,
        "Migration Deployment",
        "migration.cmmn",
        MIGRATION_CMMN,
    )
    .await;
    let case_instance_id = start_case(&base_url, &client, &case_definition_id).await;

    let response = client
        .post(format!(
            "{base_url}/cmmn-runtime/case-instances/{case_instance_id}/change-state"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "terminatePlanItemDefinitionIds": ["reviewTask"]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let runtime_tasks_response = client
        .get(format!(
            "{base_url}/cmmn-runtime/tasks?caseInstanceId={case_instance_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(runtime_tasks_response.status(), StatusCode::OK);
    let runtime_tasks_body: Value = runtime_tasks_response.json().await.unwrap();
    assert_eq!(runtime_tasks_body["total"], 0);

    let historic_response = client
        .get(format!(
            "{base_url}/cmmn-history/historic-case-instances/{case_instance_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(historic_response.status(), StatusCode::OK);
    let historic_body: Value = historic_response.json().await.unwrap();
    assert_eq!(historic_body["state"], "COMPLETED");

    let historic_plan_items_response = client
        .get(format!(
            "{base_url}/cmmn-history/historic-plan-item-instances?caseInstanceId={case_instance_id}&start=0&size=10"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(historic_plan_items_response.status(), StatusCode::OK);
    let historic_plan_items_body: Value = historic_plan_items_response.json().await.unwrap();
    assert_eq!(historic_plan_items_body["total"], 1);
    assert_eq!(historic_plan_items_body["data"][0]["state"], "TERMINATED");
}

#[tokio::test]
async fn cmmn_change_state_move_to_available_removes_runtime_task_without_completed_history() {
    let (base_url, client) = spawn_server("rest-cmmn-change-state-move-available").await;
    let case_definition_id = deploy_definition(
        &base_url,
        &client,
        "Migration Deployment",
        "migration.cmmn",
        MIGRATION_CMMN,
    )
    .await;
    let case_instance_id = start_case(&base_url, &client, &case_definition_id).await;

    let response = client
        .post(format!(
            "{base_url}/cmmn-runtime/case-instances/{case_instance_id}/change-state"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "moveToAvailablePlanItemDefinitionIds": ["reviewTask"]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let runtime_tasks_response = client
        .get(format!(
            "{base_url}/cmmn-runtime/tasks?caseInstanceId={case_instance_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(runtime_tasks_response.status(), StatusCode::OK);
    let runtime_tasks_body: Value = runtime_tasks_response.json().await.unwrap();
    assert_eq!(runtime_tasks_body["total"], 0);

    let runtime_case_response = client
        .get(format!(
            "{base_url}/cmmn-runtime/case-instances/{case_instance_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(runtime_case_response.status(), StatusCode::OK);
    let runtime_case_body: Value = runtime_case_response.json().await.unwrap();
    assert_eq!(runtime_case_body["state"], "ACTIVE");

    let available_history_response = client
        .get(format!(
            "{base_url}/cmmn-history/historic-plan-item-instances?caseInstanceId={case_instance_id}&state=AVAILABLE&start=0&size=10"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(available_history_response.status(), StatusCode::OK);
    let available_history_body: Value = available_history_response.json().await.unwrap();
    assert_eq!(available_history_body["total"], 1);
    assert_eq!(available_history_body["data"][0]["state"], "AVAILABLE");

    for state in ["COMPLETED", "TERMINATED"] {
        let history_response = client
            .get(format!(
                "{base_url}/cmmn-history/historic-plan-item-instances?caseInstanceId={case_instance_id}&state={state}&start=0&size=10"
            ))
            .basic_auth("admin", Some("test"))
            .send()
            .await
            .unwrap();
        assert_eq!(history_response.status(), StatusCode::OK);
        let history_body: Value = history_response.json().await.unwrap();
        assert_eq!(history_body["total"], 0, "unexpected {state} history");
    }
}

#[tokio::test]
async fn cmmn_change_state_activates_multiple_available_human_tasks_in_one_request() {
    let (base_url, client) = spawn_server("rest-cmmn-change-state-multi-activate").await;
    let case_definition_id = deploy_definition(
        &base_url,
        &client,
        "Two Task Deployment",
        "two-task.cmmn",
        TWO_TASK_CMMN,
    )
    .await;
    let case_instance_id = start_case(&base_url, &client, &case_definition_id).await;

    let move_response = client
        .post(format!(
            "{base_url}/cmmn-runtime/case-instances/{case_instance_id}/change-state"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "moveToAvailablePlanItemDefinitionIds": ["reviewTask", "approveTask"]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(move_response.status(), StatusCode::OK);

    let after_move_response = client
        .get(format!(
            "{base_url}/cmmn-runtime/tasks?caseInstanceId={case_instance_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(after_move_response.status(), StatusCode::OK);
    let after_move_body: Value = after_move_response.json().await.unwrap();
    assert_eq!(after_move_body["total"], 0);

    let activate_response = client
        .post(format!(
            "{base_url}/cmmn-runtime/case-instances/{case_instance_id}/change-state"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "activatePlanItemDefinitionIds": ["reviewTask", "approveTask"]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(activate_response.status(), StatusCode::OK);

    let runtime_tasks_response = client
        .get(format!(
            "{base_url}/cmmn-runtime/tasks?caseInstanceId={case_instance_id}&start=0&size=10"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(runtime_tasks_response.status(), StatusCode::OK);
    let runtime_tasks_body: Value = runtime_tasks_response.json().await.unwrap();
    assert_eq!(runtime_tasks_body["total"], 2);
    let mut plan_item_definition_ids = runtime_tasks_body["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|task| task["planItemDefinitionId"].as_str().unwrap().to_string())
        .collect::<Vec<_>>();
    plan_item_definition_ids.sort();
    assert_eq!(
        plan_item_definition_ids,
        // Java PlanItemInstanceEntityManagerImpl.java:92-95.
        vec!["approveTask".to_string(), "reviewTask".to_string()]
    );
}

#[tokio::test]
async fn cmmn_change_state_rejects_complex_unsupported_requests() {
    let (base_url, client) = spawn_server("rest-cmmn-change-state-unsupported").await;
    let case_definition_id = deploy_definition(
        &base_url,
        &client,
        "Migration Deployment",
        "migration.cmmn",
        MIGRATION_CMMN,
    )
    .await;
    let case_instance_id = start_case(&base_url, &client, &case_definition_id).await;

    let response = client
        .post(format!(
            "{base_url}/cmmn-runtime/case-instances/{case_instance_id}/change-state"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "addWaitingForRepetitionPlanItemDefinitionIds": ["reviewTask"]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}
