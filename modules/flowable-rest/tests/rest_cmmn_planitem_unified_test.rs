// P116: unified plan-item-instance REST surface — the plan-item-instances endpoint
// returns stage / milestone / event listener / human task instances for a case.
//
// Java references:
// - PlanItemInstanceCollectionResource.java:70-158 (GET param parsing)
// - PlanItemInstanceBaseResource.java:59-139 (query builders)
// - PlanItemInstanceResponse.java:33-82 (response fields)
//
// Human-task rows come from ACT_CMMN_HUMAN_TASK; stage/milestone/event-listener rows from
// the ACT_CMMN_RU_PLAN_ITEM_INST mirror (P116). timerEventListener is not modeled by the
// Rust converter, so its type filter matches nothing.

use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_rest::run_server;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::net::TcpListener;

const UNIFIED_CMMN: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/CMMN/20151109/MODEL"
             xmlns:flowable="http://flowable.org/cmmn"
             targetNamespace="http://flowable.org/cmmn">
  <case id="unifiedPlanItemCase" name="Unified Plan Item Case">
    <casePlanModel id="unifiedPlan" name="Unified Plan" autoComplete="false">
      <planItem id="planItemStage" name="Stage A" definitionRef="stageA" />
      <stage id="stageA" name="Stage A">
        <planItem id="planItemInner" name="Inner Task" definitionRef="innerTask" />
        <humanTask id="innerTask" name="Inner Task" isBlocking="true" />
      </stage>
      <planItem id="planItemTrigger" name="Trigger" definitionRef="triggerTask" />
      <humanTask id="triggerTask" name="Trigger" isBlocking="true" />
      <planItem id="planItemKeepalive" name="Keep Alive" definitionRef="keepaliveTask" />
      <humanTask id="keepaliveTask" name="Keep Alive" isBlocking="true" />
      <planItem id="planItemMilestone" name="Shipped" definitionRef="milestoneShipped">
        <entryCriterion id="entryMilestone">
          <sentryRef>sentryAfterTrigger</sentryRef>
        </entryCriterion>
      </planItem>
      <milestone id="milestoneShipped" name="Shipped" />
      <planItem id="planItemListener" name="Watched" definitionRef="listenerWatched" />
      <eventListener id="listenerWatched" name="Watched variable" eventType="variable" eventName="watchedVar" />
      <sentry id="sentryAfterTrigger">
        <planItemOnPart id="onTriggerComplete" sourceRef="planItemTrigger">
          <standardEvent>complete</standardEvent>
        </planItemOnPart>
      </sentry>
    </casePlanModel>
  </case>
</definitions>"#;

const TERMINATED_STAGE_CMMN: &str = r#"
<definitions xmlns="http://www.omg.org/spec/CMMN/20151109/MODEL"
             targetNamespace="http://flowable.org/cmmn">
  <case id="terminatedStageCase" name="Terminated Stage Case">
    <casePlanModel id="planModel" autoComplete="false">
      <planItem id="planItemStage" definitionRef="stageA">
        <exitCriterion id="exitStage" sentryRef="sentryExitStage" />
      </planItem>
      <stage id="stageA" name="Stage A">
        <planItem id="planItemInner" definitionRef="innerTask" />
        <humanTask id="innerTask" name="Inner Task" />
      </stage>
      <planItem id="planItemExit" definitionRef="exitTask" />
      <humanTask id="exitTask" name="Exit Stage" />
      <planItem id="planItemKeepalive" definitionRef="keepaliveTask" />
      <humanTask id="keepaliveTask" name="Keep Alive" />
      <sentry id="sentryExitStage">
        <planItemOnPart id="onExitComplete" sourceRef="planItemExit">
          <standardEvent>complete</standardEvent>
        </planItemOnPart>
      </sentry>
    </casePlanModel>
  </case>
</definitions>
"#;

async fn spawn_server() -> (String, reqwest::Client) {
    let engine = Arc::new(ProcessEngine::new("rest-cmmn-planitem-unified".to_string()));
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

async fn deploy(base_url: &str, client: &reqwest::Client) {
    let response = client
        .post(format!("{base_url}/cmmn-repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "P116 unified deployment",
            "resourceName": "unified.cmmn",
            "resource": UNIFIED_CMMN
        }))
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success());
}

async fn start_case(base_url: &str, client: &reqwest::Client) -> String {
    let response = client
        .post(format!("{base_url}/cmmn-runtime/case-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "caseDefinitionKey": "unifiedPlanItemCase" }))
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success());
    response.json::<Value>().await.unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string()
}

async fn plan_items(
    base_url: &str,
    client: &reqwest::Client,
    query: &str,
) -> Vec<Value> {
    let response = client
        .get(format!("{base_url}/cmmn-runtime/plan-item-instances{query}"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success(), "GET {query} failed");
    response.json::<Value>().await.unwrap()["data"]
        .as_array()
        .unwrap()
        .clone()
}

#[tokio::test]
async fn unified_endpoint_returns_stage_milestone_eventlistener_and_humantask() {
    let (base_url, client) = spawn_server().await;
    deploy(&base_url, &client).await;
    let case_id = start_case(&base_url, &client).await;

    // `CmmnOperation.java:117-210`: the AVAILABLE milestone is materialized too.
    let items = plan_items(&base_url, &client, &format!("?caseInstanceId={case_id}")).await;
    let mut by_type: Vec<&str> = items
        .iter()
        .map(|item| item["planItemDefinitionType"].as_str().unwrap())
        .collect();
    by_type.sort_unstable();
    assert_eq!(
        by_type,
        vec![
            "eventlistener",
            "humantask",
            "humantask",
            "humantask",
            "milestone",
            "stage",
        ],
        "stage + 3 tasks + available listener + available milestone"
    );

    // Type filter for the event listener.
    let listeners = plan_items(
        &base_url,
        &client,
        "?planItemDefinitionType=eventlistener",
    )
    .await;
    assert_eq!(listeners.len(), 1);
    assert_eq!(listeners[0]["state"], "AVAILABLE");
    assert_eq!(listeners[0]["elementId"], "planItemListener");
    assert_eq!(listeners[0]["planItemDefinitionId"], "listenerWatched");
    assert_eq!(listeners[0]["stage"], false);

    // Type filter for the stage.
    let stages = plan_items(&base_url, &client, "?planItemDefinitionType=stage").await;
    assert_eq!(stages.len(), 1);
    assert_eq!(stages[0]["state"], "ACTIVE");
    assert_eq!(stages[0]["elementId"], "planItemStage");
    assert_eq!(stages[0]["planItemDefinitionId"], "stageA");
    assert_eq!(stages[0]["stage"], true);
    assert_eq!(stages[0]["name"], "Stage A");

    // Type filter for human tasks.
    let tasks = plan_items(&base_url, &client, "?planItemDefinitionType=humantask").await;
    assert_eq!(tasks.len(), 3);

    // timerEventListener is not modeled by the Rust converter → nothing matches.
    let timers = plan_items(
        &base_url,
        &client,
        "?planItemDefinitionType=timerEventListener",
    )
    .await;
    assert_eq!(timers.len(), 0);

    // Occur the milestone (complete the trigger task) and the event listener (write the
    // watched variable).
    let trigger = plan_items(
        &base_url,
        &client,
        &format!("?caseInstanceId={case_id}&elementId=planItemTrigger"),
    )
    .await;
    let trigger_id = trigger[0]["id"].as_str().unwrap();
    let complete = client
        .post(format!(
            "{base_url}/cmmn-runtime/plan-item-instances/{trigger_id}/complete"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({}))
        .send()
        .await
        .unwrap();
    assert!(complete.status().is_success());

    let variable_write = client
        .put(format!(
            "{base_url}/cmmn-runtime/case-instances/{case_id}/variables"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!([{ "name": "watchedVar", "value": "go" }]))
        .send()
        .await
        .unwrap();
    assert!(variable_write.status().is_success());

    // Java runtime queries hide terminal plan-item rows; Rust keeps them only for
    // includeEnded/historic access after OccurPlanItemInstanceOperation.java:34-63.
    let milestones = plan_items(
        &base_url,
        &client,
        &format!("?caseInstanceId={case_id}&planItemDefinitionType=milestone"),
    )
    .await;
    assert!(milestones.is_empty());

    let listeners_after = plan_items(
        &base_url,
        &client,
        &format!("?caseInstanceId={case_id}&planItemDefinitionType=eventlistener"),
    )
    .await;
    assert!(listeners_after.is_empty());

    let ended_milestones = plan_items(
        &base_url,
        &client,
        &format!(
            "?caseInstanceId={case_id}&planItemDefinitionType=milestone&includeEnded=true"
        ),
    )
    .await;
    assert_eq!(ended_milestones.len(), 1);
    assert_eq!(ended_milestones[0]["state"], "COMPLETED");
    assert!(ended_milestones[0]["occurredTime"].is_string());

    // State filter on the runtime view matches only the completed human task.
    let completed = plan_items(
        &base_url,
        &client,
        &format!("?caseInstanceId={case_id}&state=COMPLETED"),
    )
    .await;
    assert_eq!(completed.len(), 1);

    // elementId filter matches any type.
    let by_element = plan_items(
        &base_url,
        &client,
        &format!("?caseInstanceId={case_id}&elementId=planItemMilestone"),
    )
    .await;
    assert!(by_element.is_empty());
}

#[tokio::test]
async fn terminated_stage_is_runtime_hidden_and_historic_visible() {
    let (base_url, client) = spawn_server().await;
    let deploy_response = client
        .post(format!("{base_url}/cmmn-repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "P132 terminated stage deployment",
            "resourceName": "terminated-stage.cmmn",
            "resource": TERMINATED_STAGE_CMMN
        }))
        .send()
        .await
        .unwrap();
    assert!(deploy_response.status().is_success());
    let start_response = client
        .post(format!("{base_url}/cmmn-runtime/case-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "caseDefinitionKey": "terminatedStageCase" }))
        .send()
        .await
        .unwrap();
    assert!(start_response.status().is_success());
    let case_id = start_response.json::<Value>().await.unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();

    let exit_task = plan_items(
        &base_url,
        &client,
        &format!("?caseInstanceId={case_id}&elementId=planItemExit"),
    )
    .await;
    let exit_task_id = exit_task[0]["id"].as_str().unwrap();
    let response = client
        .post(format!(
            "{base_url}/cmmn-runtime/plan-item-instances/{exit_task_id}/complete"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({}))
        .send()
        .await
        .unwrap();
    let status = response.status();
    let body = response.text().await.unwrap();
    assert!(status.is_success(), "exit trigger failed: {status} {body}");

    let runtime = plan_items(
        &base_url,
        &client,
        &format!("?caseInstanceId={case_id}&planItemDefinitionType=stage"),
    )
    .await;
    assert!(runtime.is_empty(), "terminal rows are absent from runtime queries");

    let history_response = client
        .get(format!(
            "{base_url}/cmmn-history/historic-plan-item-instances?caseInstanceId={case_id}&planItemDefinitionId=stageA&state=TERMINATED"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(history_response.status().is_success());
    let history: Value = history_response.json().await.unwrap();
    assert_eq!(history["total"], 1);
    assert_eq!(history["data"][0]["state"], "TERMINATED");
    assert_eq!(history["data"][0]["planItemDefinitionId"], "stageA");
    assert!(history["data"][0]["endedAt"].is_string());

    let case_response = client
        .get(format!("{base_url}/cmmn-runtime/case-instances/{case_id}"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(case_response.status().is_success());
    assert_eq!(case_response.json::<Value>().await.unwrap()["state"], "ACTIVE");
}

#[tokio::test]
async fn unified_endpoint_post_body_supports_type_and_state_filters() {
    let (base_url, client) = spawn_server().await;
    deploy(&base_url, &client).await;
    let case_id = start_case(&base_url, &client).await;

    let response = client
        .post(format!("{base_url}/cmmn-query/plan-item-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "caseInstanceId": case_id,
            "planItemDefinitionTypes": ["stage", "humantask"]
        }))
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success());
    let body: Value = response.json().await.unwrap();
    let types = body["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["planItemDefinitionType"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(types.len(), 4, "1 stage + 3 human tasks");
    assert!(types.contains(&"stage"));
    assert!(types.contains(&"humantask"));
}
