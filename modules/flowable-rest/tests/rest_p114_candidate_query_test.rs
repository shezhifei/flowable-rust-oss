//! P114: CMMN human task candidate query via REST — candidateUser /
//! candidateGroup / candidateGroups(candidateGroupIn) / candidateOrAssigned /
//! ignoreAssignee, with identity group expansion.
//!
//! Java parity: TaskCollectionResource.getTasks param parsing
//! (TaskCollectionResource.java:185-203, 321-323) and TaskQueryRequest JSON keys
//! (TaskQueryRequest.java:47-50, 83). The CMMN REST group expansion is backed by
//! the ProcessEngine identity service (Java TaskQueryImpl.getGroupsForCandidateUser,
//! TaskQueryImpl.java:2021-2032).

use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::identity::entities::{Group, User};
use flowable_rest::run_server;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::net::TcpListener;

const CANDIDATE_CMMN: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/CMMN/20151109/MODEL"
             xmlns:flowable="http://flowable.org/cmmn"
             targetNamespace="http://flowable.org/cmmn">
  <case id="p114CandidateCase" name="P114 candidate case">
    <casePlanModel id="planModel" name="Plan Model" autoComplete="false">
      <planItem id="planItemReview" name="Review" definitionRef="reviewTask" />
      <planItem id="planItemApprove" name="Approve" definitionRef="approveTask" />
      <planItem id="planItemAudit" name="Audit" definitionRef="auditTask" />
      <humanTask id="reviewTask" name="Review"
                 flowable:candidateUsers="alice, bob"
                 flowable:candidateGroups="managers,auditors" />
      <humanTask id="approveTask" name="Approve"
                 flowable:candidateGroups="sales" />
      <humanTask id="auditTask" name="Audit" />
    </casePlanModel>
  </case>
</definitions>"#;

async fn spawn_server(test_name: &str) -> (Arc<ProcessEngine>, String, reqwest::Client) {
    let engine = Arc::new(ProcessEngine::new(test_name.to_string()));
    engine.get_identity_service().save_user(User {
        id: "admin".to_string(),
        first_name: None,
        last_name: None,
        email: None,
        password: Some("test".to_string()),
        tenant_id: None,
    });
    // Identity fixture: charlie/carol belong to `managers`.
    engine.get_identity_service().save_group(Group {
        id: "managers".to_string(),
        name: "Managers".to_string(),
        group_type: None,
    });
    engine
        .get_identity_service()
        .create_membership("charlie".to_string(), "managers".to_string());
    engine
        .get_identity_service()
        .create_membership("carol".to_string(), "managers".to_string());

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let engine_for_server = Arc::clone(&engine);
    tokio::spawn(async move {
        run_server(engine_for_server, listener).await.unwrap();
    });

    (engine, base_url, reqwest::Client::new())
}

async fn deploy_and_start_case(
    base_url: &str,
    client: &reqwest::Client,
) -> String {
    let deploy_response = client
        .post(format!("{base_url}/cmmn-repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "P114 candidate query deployment",
            "resourceName": "p114-candidate.cmmn",
            "resource": CANDIDATE_CMMN
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        deploy_response.status(),
        reqwest::StatusCode::CREATED,
        "deploy must return 201"
    );

    let start_response = client
        .post(format!("{base_url}/cmmn-runtime/case-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "caseDefinitionKey": "p114CandidateCase" }))
        .send()
        .await
        .unwrap();
    assert!(start_response.status().is_success());
    start_response.json::<Value>().await.unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string()
}

async fn get_task_names(base_url: &str, client: &reqwest::Client, query: &str) -> Vec<String> {
    let response = client
        .get(format!("{base_url}/cmmn-runtime/tasks{query}"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        reqwest::StatusCode::OK,
        "GET /cmmn-runtime/tasks?{query}"
    );
    let body: Value = response.json().await.unwrap();
    let mut names = body["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|task| task["name"].as_str().unwrap().to_string())
        .collect::<Vec<_>>();
    names.sort();
    names
}

async fn post_query_names(
    base_url: &str,
    client: &reqwest::Client,
    body: Value,
) -> Vec<String> {
    let response = client
        .post(format!("{base_url}/cmmn-query/tasks"))
        .basic_auth("admin", Some("test"))
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        reqwest::StatusCode::OK,
        "POST /cmmn-query/tasks {body}"
    );
    let body: Value = response.json().await.unwrap();
    let mut names = body["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|task| task["name"].as_str().unwrap().to_string())
        .collect::<Vec<_>>();
    names.sort();
    names
}

/// T1 GET candidateUser: direct link hit and identity group expansion hit.
#[tokio::test]
async fn t1_candidate_user_direct_and_group_expansion() {
    let (_engine, base_url, client) = spawn_server("rest-p114-t1").await;
    deploy_and_start_case(&base_url, &client).await;

    // Direct candidate link.
    assert_eq!(
        get_task_names(&base_url, &client, "?candidateUser=alice").await,
        vec!["Review"]
    );
    // Group expansion via the ProcessEngine identity membership.
    assert_eq!(
        get_task_names(&base_url, &client, "?candidateUser=charlie").await,
        vec!["Review"]
    );
    // Unknown user → empty.
    assert_eq!(
        get_task_names(&base_url, &client, "?candidateUser=nobody").await.len(),
        0
    );
}

/// T2 GET candidateGroup and candidateGroups (CSV → candidateGroupIn), plus the
/// POST candidateGroupIn array body.
#[tokio::test]
async fn t2_candidate_group_and_group_in() {
    let (_engine, base_url, client) = spawn_server("rest-p114-t2").await;
    deploy_and_start_case(&base_url, &client).await;

    assert_eq!(
        get_task_names(&base_url, &client, "?candidateGroup=sales").await,
        vec!["Approve"]
    );
    // GET `candidateGroups` (plural CSV) → candidateGroupIn.
    assert_eq!(
        get_task_names(&base_url, &client, "?candidateGroups=sales,auditors").await,
        vec!["Approve", "Review"]
    );
    // POST `candidateGroupIn` array.
    assert_eq!(
        post_query_names(
            &base_url,
            &client,
            json!({ "candidateGroupIn": ["sales", "auditors"] })
        )
        .await,
        vec!["Approve", "Review"]
    );
}

/// T3 ignoreAssignee: claimed candidate tasks are excluded by default and kept
/// with ignoreAssignee=true.
#[tokio::test]
async fn t3_ignore_assignee_via_rest() {
    let (_engine, base_url, client) = spawn_server("rest-p114-t3").await;
    deploy_and_start_case(&base_url, &client).await;

    // Claim the Review task (keeps its candidate links).
    let review_id = client
        .get(format!("{base_url}/cmmn-runtime/tasks?name=Review"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap()["data"][0]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let claim = client
        .post(format!("{base_url}/cmmn-runtime/tasks/{review_id}"))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "action": "claim", "assignee": "zed" }))
        .send()
        .await
        .unwrap();
    assert_eq!(claim.status(), reqwest::StatusCode::OK);

    // Default: assigned candidate task is excluded.
    assert_eq!(
        get_task_names(&base_url, &client, "?candidateUser=alice").await.len(),
        0
    );
    // ignoreAssignee=true keeps it.
    assert_eq!(
        get_task_names(&base_url, &client, "?candidateUser=alice&ignoreAssignee=true").await,
        vec!["Review"]
    );
}

/// T4 candidateOrAssigned: assigned tasks and candidate (group-expanded) tasks.
#[tokio::test]
async fn t4_candidate_or_assigned_via_rest() {
    let (_engine, base_url, client) = spawn_server("rest-p114-t4").await;
    deploy_and_start_case(&base_url, &client).await;

    // Claim the bare Audit task for carol.
    let audit_id = client
        .get(format!("{base_url}/cmmn-runtime/tasks?name=Audit"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap()["data"][0]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let claim = client
        .post(format!("{base_url}/cmmn-runtime/tasks/{audit_id}"))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "action": "claim", "assignee": "carol" }))
        .send()
        .await
        .unwrap();
    assert_eq!(claim.status(), reqwest::StatusCode::OK);

    // carol: assigned Audit + candidate on Review via the managers group.
    assert_eq!(
        get_task_names(&base_url, &client, "?candidateOrAssigned=carol").await,
        vec!["Audit", "Review"]
    );
    // POST candidateOrAssigned.
    assert_eq!(
        post_query_names(
            &base_url,
            &client,
            json!({ "candidateOrAssigned": "carol" })
        )
        .await,
        vec!["Audit", "Review"]
    );
}
