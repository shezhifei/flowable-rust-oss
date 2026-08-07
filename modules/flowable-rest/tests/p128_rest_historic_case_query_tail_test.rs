//! P128 — REST plumbing for the historic CMMN case-query tail.

use flowable_cmmn_engine::{
    CmmnCaseInstanceStartRequest, CmmnEngine, CmmnHumanTaskCompletionRequest, CmmnHumanTaskState,
    CmmnIdentityLink,
};
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_rest::run_server;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::net::TcpListener;

const MODEL: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/CMMN/20151109/MODEL"
             targetNamespace="http://flowable.org/cmmn">
  <case id="p128RestCase" name="P128 REST case">
    <casePlanModel id="p128Plan" name="P128 plan" autoComplete="true">
      <planItem id="reviewPlanItem" name="Review" definitionRef="reviewTask" />
      <humanTask id="reviewTask" name="Review" />
    </casePlanModel>
  </case>
</definitions>"#;

struct Fixture {
    base_url: String,
    client: reqwest::Client,
    callback_case: String,
    plain_case: String,
}

async fn setup(test_name: &str) -> Fixture {
    let process_engine = Arc::new(ProcessEngine::new(test_name.to_string()));
    process_engine
        .get_identity_service()
        .save_user(flowable_engine::identity::entities::User {
            id: "admin".to_string(),
            first_name: None,
            last_name: None,
            email: None,
            password: Some("test".to_string()),
            tenant_id: None,
        });
    let cmmn_engine = process_engine
        .get_config()
        .cmmn_engine
        .as_ref()
        .expect("CMMN engine")
        .clone();

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let server_engine = Arc::clone(&process_engine);
    tokio::spawn(async move {
        run_server(server_engine, listener).await.unwrap();
    });
    let client = reqwest::Client::new();

    let deployment = client
        .post(format!("{base_url}/cmmn-repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "P128 historic query tail",
            "resourceName": "p128.cmmn",
            "resource": MODEL
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        deployment.status(),
        reqwest::StatusCode::CREATED,
        "deployment failed: {}",
        deployment.text().await.unwrap()
    );

    let callback_case = cmmn_engine
        .start_case_instance_by_key(
            "p128RestCase",
            CmmnCaseInstanceStartRequest::new()
                .with_name("callback")
                .with_callback("execution-128", "bpmn-2.0-to-cmmn-1.1-child-case")
                .with_variables(json!({ "alpha": 1, "secret": "hidden" })),
        )
        .expect("callback case")
        .id;
    let plain_case = cmmn_engine
        .start_case_instance_by_key(
            "p128RestCase",
            CmmnCaseInstanceStartRequest::new()
                .with_name("plain")
                .with_variables(json!({ "alpha": 2, "secret": "plain" })),
        )
        .expect("plain case")
        .id;

    let plain_task = active_task_id(&cmmn_engine, &plain_case);
    cmmn_engine
        .complete_human_task(&plain_task, CmmnHumanTaskCompletionRequest::new())
        .expect("complete plain case");
    cmmn_engine
        .identity_link_service()
        .add_identity_link(CmmnIdentityLink {
            id: format!("participant-{plain_case}"),
            scope_type: "caseInstance".to_string(),
            scope_id: plain_case.clone(),
            link_type: "participant".to_string(),
            user_id: Some("kermit".to_string()),
            group_id: None,
        })
        .expect("historic participant");

    Fixture {
        base_url,
        client,
        callback_case,
        plain_case,
    }
}

fn active_task_id(engine: &CmmnEngine, case_instance_id: &str) -> String {
    engine
        .runtime_service()
        .create_human_task_query()
        .case_instance_id(case_instance_id)
        .state(CmmnHumanTaskState::Active)
        .single_result()
        .expect("task query")
        .expect("active task")
        .id
}

async fn get(fixture: &Fixture, query: &str) -> (reqwest::StatusCode, Value) {
    let response = fixture
        .client
        .get(format!(
            "{}/cmmn-history/historic-case-instances?{query}",
            fixture.base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    let status = response.status();
    let body = response.json().await.unwrap();
    (status, body)
}

fn only_id(body: &Value) -> &str {
    assert_eq!(body["total"], 1);
    body["data"][0]["id"].as_str().unwrap()
}

#[tokio::test]
async fn get_and_post_apply_the_implementable_tail_filters() {
    let fixture = setup("rest-p128-tail-filters").await;

    for query in [
        "callbackId=execution-128",
        "callbackIds=missing,execution-128",
        "callbackType=bpmn-2.0-to-cmmn-1.1-child-case",
        "activePlanItemDefinitionId=reviewTask",
    ] {
        let (status, body) = get(&fixture, query).await;
        assert_eq!(status, reqwest::StatusCode::OK, "{query}: {body}");
        assert_eq!(only_id(&body), fixture.callback_case, "{query}");
    }

    let (status, body) = get(&fixture, "withoutCaseInstanceCallbackId=true").await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert_eq!(only_id(&body), fixture.plain_case);

    let (status, body) = get(&fixture, "involvedUser=kermit").await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert_eq!(only_id(&body), fixture.plain_case);

    let response = fixture
        .client
        .post(format!(
            "{}/cmmn-query/historic-case-instances",
            fixture.base_url
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "callbackIds": ["execution-128", "missing"],
            "callbackType": "bpmn-2.0-to-cmmn-1.1-child-case"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert_eq!(
        only_id(&response.json::<Value>().await.unwrap()),
        fixture.callback_case
    );
}

#[tokio::test]
async fn historic_variable_inclusion_supports_all_and_selected_names() {
    let fixture = setup("rest-p128-historic-variables").await;

    let (status, all) = get(
        &fixture,
        "callbackId=execution-128&includeCaseVariables=true",
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    let mut names = all["data"][0]["variables"]
        .as_array()
        .unwrap()
        .iter()
        .map(|variable| variable["name"].as_str().unwrap())
        .collect::<Vec<_>>();
    names.sort_unstable();
    assert_eq!(names, vec!["alpha", "secret"]);

    let (status, selected) = get(
        &fixture,
        "callbackId=execution-128&includeCaseVariablesNames=alpha",
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert_eq!(
        selected["data"][0]["variables"].as_array().unwrap().len(),
        1
    );
    assert_eq!(selected["data"][0]["variables"][0]["name"], "alpha");
    assert_eq!(selected["data"][0]["variables"][0]["value"], 1);
}

#[tokio::test]
async fn parameters_without_reliable_rust_history_data_remain_rejected() {
    let fixture = setup("rest-p128-tail-rejections").await;
    let rejected = [
        ("rootScopeId", "root-1"),
        ("parentScopeId", "parent-1"),
        ("parentCaseInstanceId", "case-1"),
        ("withoutCaseInstanceParentId", "true"),
        ("lastReactivatedBefore", "2030-01-01T00:00:00Z"),
        ("lastReactivatedAfter", "2000-01-01T00:00:00Z"),
        ("lastReactivatedBy", "kermit"),
    ];

    for (parameter, value) in rejected {
        let (status, body) = get(&fixture, &format!("{parameter}={value}")).await;
        assert_eq!(
            status,
            reqwest::StatusCode::BAD_REQUEST,
            "{parameter} must remain rejected: {body}"
        );
        assert!(
            body["details"].as_str().unwrap().contains(parameter),
            "error must identify {parameter}: {body}"
        );
    }
}
