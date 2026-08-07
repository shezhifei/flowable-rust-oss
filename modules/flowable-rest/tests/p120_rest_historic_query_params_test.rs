//! P120 — CMMN historic query parameter surface over REST.
//!
//! Java parity: `HistoricCaseInstanceCollectionResource.getHistoricCasenstances`
//! (HistoricCaseInstanceCollectionResource.java:104-301) and
//! `HistoricTaskInstanceCollectionResource.getHistoricTaskInstances`
//! (HistoricTaskInstanceCollectionResource.java:97-306).
//!
//! Java parses these through `@RequestParam Map<String, String> allRequestParams`,
//! so an unknown param is silently ignored and a legal one is never a 400. The
//! Rust surface uses `deny_unknown_fields`, so before P120 most of the Java-legal
//! params below answered 400. Each case here pins the new behaviour: the param is
//! accepted *and* filters. Params the Rust historic model cannot express stay
//! rejected, and the last test pins that shape too.

use flowable_cmmn_engine::{CmmnCaseInstanceStartRequest, CmmnEngine};
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_rest::run_server;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::net::TcpListener;

const P120_REST_CMMN: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/CMMN/20151109/MODEL"
             xmlns:flowable="http://flowable.org/cmmn"
             targetNamespace="http://flowable.org/cmmn">
  <case id="p120RestCase" name="P120 rest case">
    <casePlanModel id="p120RestPlan" name="P120 rest plan" autoComplete="true">
      <planItem id="planItemReview" name="Review application" definitionRef="reviewTask" />
      <humanTask id="reviewTask" name="Review application" />
    </casePlanModel>
  </case>
  <case id="p120RestAssignedCase" name="P120 rest assigned case">
    <casePlanModel id="p120RestAssignedPlan" name="P120 rest assigned plan" autoComplete="true">
      <planItem id="planItemAssigned" name="Assigned review" definitionRef="assignedTask" />
      <humanTask id="assignedTask" name="Assigned review"
                 flowable:assignee="fozzie"
                 flowable:owner="gonzo"
                 flowable:category="finance"
                 flowable:candidateGroups="sales" />
    </casePlanModel>
  </case>
  <case id="p120RestCandidateCase" name="P120 rest candidate case">
    <casePlanModel id="p120RestCandidatePlan" name="P120 rest candidate plan" autoComplete="true">
      <planItem id="planItemCandidate" name="Candidate review" definitionRef="candidateTask" />
      <humanTask id="candidateTask" name="Candidate review"
                 flowable:candidateUsers="kermit"
                 flowable:candidateGroups="sales,support" />
    </casePlanModel>
  </case>
</definitions>"#;

struct Fixture {
    base_url: String,
    client: reqwest::Client,
    /// p120RestCase, completed, businessKey BK-100, name "Alpha review".
    alpha: String,
    /// p120RestCase, running, businessKey BK-200, name "Beta review".
    beta: String,
    /// p120RestCase, running, started by `kermit`, businessKey BK-300.
    gamma: String,
    /// p120RestAssignedCase, running; its task carries assignee/owner/category.
    assigned: String,
    /// p120RestCandidateCase, completed; its task carries the candidate links.
    candidate: String,
    plain_definition_id: String,
    assigned_definition_id: String,
    assigned_task: String,
    candidate_task: String,
}

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

fn shared_cmmn_engine(engine: &ProcessEngine) -> Arc<CmmnEngine> {
    engine
        .get_config()
        .cmmn_engine
        .as_ref()
        .expect("test process engine should have a CMMN engine")
        .clone()
}

async fn get_ok(client: &reqwest::Client, base_url: &str, path_and_query: &str) -> Value {
    let response = client
        .get(format!("{base_url}{path_and_query}"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    let status = response.status();
    let body = response.text().await.unwrap();
    assert_eq!(
        status,
        reqwest::StatusCode::OK,
        "GET {path_and_query} must be accepted, got {status}: {body}"
    );
    serde_json::from_str(&body).unwrap()
}

fn total(body: &Value) -> u64 {
    body["total"].as_u64().unwrap()
}

fn ids(body: &Value) -> Vec<String> {
    body["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["id"].as_str().unwrap().to_string())
        .collect()
}

/// `total` for a historic-case query built from `query`.
async fn case_total(fixture: &Fixture, query: &str) -> u64 {
    total(
        &get_ok(
            &fixture.client,
            &fixture.base_url,
            &format!("/cmmn-history/historic-case-instances?{query}"),
        )
        .await,
    )
}

/// `total` for a historic-task query built from `query`.
async fn task_total(fixture: &Fixture, query: &str) -> u64 {
    total(
        &get_ok(
            &fixture.client,
            &fixture.base_url,
            &format!("/cmmn-history/historic-task-instances?{query}"),
        )
        .await,
    )
}

async fn start_case(fixture_client: &reqwest::Client, base_url: &str, body: Value) -> String {
    let response = fixture_client
        .post(format!("{base_url}/cmmn-runtime/case-instances"))
        .basic_auth("admin", Some("test"))
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::CREATED);
    response.json::<Value>().await.unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string()
}

async fn active_task_id(client: &reqwest::Client, base_url: &str, case_instance_id: &str) -> String {
    let body = get_ok(
        client,
        base_url,
        &format!("/cmmn-runtime/tasks?caseInstanceId={case_instance_id}"),
    )
    .await;
    body["data"][0]["id"].as_str().unwrap().to_string()
}

async fn complete_task(client: &reqwest::Client, base_url: &str, task_id: &str) {
    let response = client
        .post(format!("{base_url}/cmmn-runtime/tasks/{task_id}"))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "action": "complete" }))
        .send()
        .await
        .unwrap();
    assert!(
        response.status().is_success(),
        "task completion failed: {}",
        response.text().await.unwrap()
    );
}

async fn setup(test_name: &str) -> Fixture {
    let (process_engine, base_url, client) = spawn_server(test_name).await;
    let cmmn_engine = shared_cmmn_engine(&process_engine);

    let deploy_response = client
        .post(format!("{base_url}/cmmn-repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "P120 historic query deployment",
            "resourceName": "p120-rest.cmmn",
            "resource": P120_REST_CMMN
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        deploy_response.status(),
        reqwest::StatusCode::CREATED,
        "deployment failed: {}",
        deploy_response.text().await.unwrap()
    );

    let definition_id = |key: &str| {
        cmmn_engine
            .repository_service()
            .create_case_definition_query()
            .key(key)
            .single_result()
            .expect("case definition query")
            .expect("case definition")
            .id
    };
    let plain_definition_id = definition_id("p120RestCase");
    let assigned_definition_id = definition_id("p120RestAssignedCase");

    // The CMMN deployment REST resource takes no category, and the Rust case
    // definition inherits its category from the deployment, so the category the
    // `caseDefinitionCategory` family filters on is set through the engine.
    cmmn_engine
        .repository_service()
        .set_case_definition_category(&plain_definition_id, Some("finance-cases"))
        .expect("set case definition category");

    let alpha = start_case(
        &client,
        &base_url,
        json!({
            "caseDefinitionKey": "p120RestCase",
            "businessKey": "BK-100",
            "name": "Alpha review"
        }),
    )
    .await;
    let alpha_task = active_task_id(&client, &base_url, &alpha).await;
    complete_task(&client, &base_url, &alpha_task).await;

    let beta = start_case(
        &client,
        &base_url,
        json!({
            "caseDefinitionKey": "p120RestCase",
            "businessKey": "BK-200",
            "name": "Beta review"
        }),
    )
    .await;

    // `startedBy` reads START_USER_ID_, which the Rust start-case REST resource
    // never populates (no authenticated-user propagation into the CMMN start
    // request), so this instance is started through the engine to give the filter
    // something to match.
    let gamma = cmmn_engine
        .start_case_instance_by_key(
            "p120RestCase",
            CmmnCaseInstanceStartRequest::new()
                .with_business_key("BK-300")
                .with_name("Gamma review")
                .with_started_by("kermit"),
        )
        .expect("engine-started case")
        .id;

    let assigned = start_case(
        &client,
        &base_url,
        json!({ "caseDefinitionKey": "p120RestAssignedCase" }),
    )
    .await;
    let assigned_task = active_task_id(&client, &base_url, &assigned).await;

    let candidate = start_case(
        &client,
        &base_url,
        json!({ "caseDefinitionKey": "p120RestCandidateCase" }),
    )
    .await;
    let candidate_task = active_task_id(&client, &base_url, &candidate).await;
    // Completing the candidate task proves the candidate identity links survive
    // into history — Rust keeps one shared identity-link table and only drops
    // rows when the case history itself is deleted.
    complete_task(&client, &base_url, &candidate_task).await;

    // `businessStatus` is only written by a milestone's businessStatus attribute
    // (MilestoneActivityBehavior.java:59), so it is set directly here.
    cmmn_engine
        .runtime_service()
        .update_business_status(&alpha, "approved")
        .expect("business status");

    Fixture {
        base_url,
        client,
        alpha,
        beta,
        gamma,
        assigned,
        candidate,
        plain_definition_id,
        assigned_definition_id,
        assigned_task,
        candidate_task,
    }
}

#[tokio::test]
async fn historic_case_instance_query_params_are_accepted_and_filter() {
    let fixture = setup("rest-p120-historic-case-params").await;

    // Baseline: five historic cases, three of them on p120RestCase.
    assert_eq!(case_total(&fixture, "start=0&size=50").await, 5);

    // caseInstanceId / caseInstanceIds (Java parses the latter as a CSV set,
    // HistoricCaseInstanceCollectionResource.java:112-114).
    assert_eq!(
        case_total(&fixture, &format!("caseInstanceId={}", fixture.alpha)).await,
        1
    );
    assert_eq!(
        case_total(
            &fixture,
            &format!("caseInstanceIds={},{}", fixture.alpha, fixture.beta)
        )
        .await,
        2
    );

    // caseDefinitionId / Key / KeyLike / KeyLikeIgnoreCase.
    assert_eq!(
        case_total(
            &fixture,
            &format!("caseDefinitionId={}", fixture.plain_definition_id)
        )
        .await,
        3
    );
    assert_eq!(
        case_total(&fixture, "caseDefinitionKey=p120RestCase").await,
        3
    );
    assert_eq!(
        case_total(&fixture, "caseDefinitionKeyLike=p120Rest%25").await,
        5
    );
    assert_eq!(
        case_total(&fixture, "caseDefinitionKeyLikeIgnoreCase=P120RESTCASE").await,
        3
    );

    // caseDefinitionCategory family — only p120RestCase carries a category.
    assert_eq!(
        case_total(&fixture, "caseDefinitionCategory=finance-cases").await,
        3
    );
    assert_eq!(
        case_total(&fixture, "caseDefinitionCategoryLike=finance%25").await,
        3
    );
    assert_eq!(
        case_total(&fixture, "caseDefinitionCategoryLikeIgnoreCase=FINANCE%25").await,
        3
    );
    assert_eq!(case_total(&fixture, "caseDefinitionCategory=other").await, 0);

    // caseDefinitionName family.
    assert_eq!(
        case_total(&fixture, "caseDefinitionName=P120%20rest%20case").await,
        3
    );
    assert_eq!(
        case_total(&fixture, "caseDefinitionNameLike=P120%20rest%25").await,
        5
    );
    assert_eq!(
        case_total(
            &fixture,
            "caseDefinitionNameLikeIgnoreCase=p120%20REST%20CASE"
        )
        .await,
        3
    );

    // name / nameLike / nameLikeIgnoreCase.
    assert_eq!(case_total(&fixture, "name=Alpha%20review").await, 1);
    assert_eq!(case_total(&fixture, "nameLike=%25review").await, 3);
    assert_eq!(case_total(&fixture, "nameLikeIgnoreCase=beta%25").await, 1);

    // businessKey family.
    assert_eq!(case_total(&fixture, "businessKey=BK-100").await, 1);
    assert_eq!(case_total(&fixture, "businessKeyLike=BK-%25").await, 3);
    assert_eq!(
        case_total(&fixture, "businessKeyLikeIgnoreCase=bk-1%25").await,
        1
    );

    // businessStatus family.
    assert_eq!(case_total(&fixture, "businessStatus=approved").await, 1);
    assert_eq!(case_total(&fixture, "businessStatusLike=appr%25").await, 1);
    assert_eq!(
        case_total(&fixture, "businessStatusLikeIgnoreCase=APPR%25").await,
        1
    );

    // startedBy → START_USER_ID_.
    let started_by = get_ok(
        &fixture.client,
        &fixture.base_url,
        "/cmmn-history/historic-case-instances?startedBy=kermit",
    )
    .await;
    assert_eq!(total(&started_by), 1);
    assert_eq!(ids(&started_by), vec![fixture.gamma.clone()]);

    // finished / finishedBefore / finishedAfter — the two completed cases.
    assert_eq!(case_total(&fixture, "finished=true").await, 2);
    assert_eq!(case_total(&fixture, "finished=false").await, 3);
    assert_eq!(
        case_total(&fixture, "finishedAfter=2000-01-01T00:00:00Z").await,
        2
    );
    assert_eq!(
        case_total(&fixture, "finishedBefore=2000-01-01T00:00:00Z").await,
        0
    );

    // startedBefore / startedAfter bracket START_TIME_ inclusively.
    assert_eq!(
        case_total(
            &fixture,
            "startedAfter=2000-01-01T00:00:00Z&startedBefore=2999-01-01T00:00:00Z"
        )
        .await,
        5
    );
    assert_eq!(
        case_total(&fixture, "startedAfter=2999-01-01T00:00:00Z").await,
        0
    );

    // Java's CaseInstanceState literals are lower case (CaseInstanceState.java:28-33),
    // so `state=completed` is a legal Java request; it used to 400 here.
    assert_eq!(case_total(&fixture, "state=completed").await, 2);
    assert_eq!(case_total(&fixture, "state=COMPLETED").await, 2);
    assert_eq!(case_total(&fixture, "state=active").await, 3);

    // tenant family — nothing in this fixture is tenant-scoped.
    assert_eq!(case_total(&fixture, "withoutTenantId=true").await, 5);
    assert_eq!(case_total(&fixture, "tenantId=tenant-a").await, 0);
    assert_eq!(case_total(&fixture, "tenantIdLike=tenant%25").await, 0);
    assert_eq!(
        case_total(&fixture, "tenantIdLikeIgnoreCase=TENANT%25").await,
        0
    );

    // sort/order run before the page window, so a size-1 page returns the global
    // first row rather than the first row of an unsorted page.
    let sorted = get_ok(
        &fixture.client,
        &fixture.base_url,
        "/cmmn-history/historic-case-instances?sort=startTime&order=asc&start=0&size=1",
    )
    .await;
    assert_eq!(total(&sorted), 5);
    assert_eq!(ids(&sorted), vec![fixture.alpha.clone()]);
    assert_eq!(sorted["sort"], "startTime");
    assert_eq!(sorted["order"], "asc");

    let descending = get_ok(
        &fixture.client,
        &fixture.base_url,
        "/cmmn-history/historic-case-instances?sort=caseInstanceId&order=desc",
    )
    .await;
    let mut expected = ids(&descending);
    expected.sort();
    expected.reverse();
    assert_eq!(ids(&descending), expected);

    // The same param surface backs the POST query resource.
    let posted = fixture
        .client
        .post(format!(
            "{}/cmmn-query/historic-case-instances",
            fixture.base_url
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "caseDefinitionKey": "p120RestCase", "finished": false }))
        .send()
        .await
        .unwrap();
    assert_eq!(posted.status(), reqwest::StatusCode::OK);
    assert_eq!(total(&posted.json::<Value>().await.unwrap()), 2);
}

#[tokio::test]
async fn historic_task_instance_query_params_are_accepted_and_filter() {
    let fixture = setup("rest-p120-historic-task-params").await;

    // Baseline: one human task per case instance.
    assert_eq!(task_total(&fixture, "start=0&size=50").await, 5);

    // taskId is an alias of the Rust `id` on this handler
    // (HistoricTaskInstanceCollectionResource.java:101-103).
    let by_task_id = get_ok(
        &fixture.client,
        &fixture.base_url,
        &format!(
            "/cmmn-history/historic-task-instances?taskId={}",
            fixture.assigned_task
        ),
    )
    .await;
    assert_eq!(total(&by_task_id), 1);
    // P120 surfaces the historic assignee, which used to serialise as null.
    assert_eq!(by_task_id["data"][0]["assignee"], "fozzie");
    assert_eq!(by_task_id["data"][0]["caseInstanceId"], fixture.assigned);
    // Java CmmnRestResponseFactory.java:901-902 serialises the definitionRef
    // target, matching PlanItemInstanceEntityManagerImpl.java:92-95.
    assert_eq!(by_task_id["data"][0]["planItemDefinitionId"], "assignedTask");

    // caseInstanceId / caseDefinitionId.
    assert_eq!(
        task_total(&fixture, &format!("caseInstanceId={}", fixture.assigned)).await,
        1
    );
    assert_eq!(
        task_total(
            &fixture,
            &format!("caseDefinitionId={}", fixture.assigned_definition_id)
        )
        .await,
        1
    );

    // taskName / taskNameLike / taskNameLikeIgnoreCase.
    assert_eq!(task_total(&fixture, "taskName=Assigned%20review").await, 1);
    assert_eq!(task_total(&fixture, "taskNameLike=%25review").await, 2);
    assert_eq!(
        task_total(&fixture, "taskNameLikeIgnoreCase=REVIEW%25").await,
        3
    );

    // taskDefinitionKey / taskDefinitionKeyLike.
    assert_eq!(
        task_total(&fixture, "taskDefinitionKey=assignedTask").await,
        1
    );
    assert_eq!(
        task_total(&fixture, "taskDefinitionKeyLike=%25Task").await,
        5
    );

    // taskAssignee(Like) / taskOwner(Like) / taskCategory.
    assert_eq!(task_total(&fixture, "taskAssignee=fozzie").await, 1);
    assert_eq!(task_total(&fixture, "taskAssigneeLike=foz%25").await, 1);
    assert_eq!(task_total(&fixture, "taskOwner=gonzo").await, 1);
    assert_eq!(task_total(&fixture, "taskOwnerLike=gon%25").await, 1);
    assert_eq!(task_total(&fixture, "taskCategory=finance").await, 1);

    // taskDeleteReason is accepted, but the Rust CMMN engine never records a
    // delete reason, so an equality filter selects nothing.
    assert_eq!(task_total(&fixture, "taskDeleteReason=completed").await, 0);

    // finished / unfinished — the alpha and candidate tasks are completed.
    assert_eq!(task_total(&fixture, "finished=true").await, 2);
    assert_eq!(task_total(&fixture, "finished=false").await, 3);

    // taskCreatedBefore/After and taskCompletedBefore/After.
    assert_eq!(
        task_total(
            &fixture,
            "taskCreatedAfter=2000-01-01T00:00:00Z&taskCreatedBefore=2999-01-01T00:00:00Z"
        )
        .await,
        5
    );
    assert_eq!(
        task_total(&fixture, "taskCompletedAfter=2000-01-01T00:00:00Z").await,
        2
    );
    assert_eq!(
        task_total(&fixture, "taskCompletedBefore=2000-01-01T00:00:00Z").await,
        0
    );

    // `state` is a Rust-only extension on this route (Java's CMMN historic task
    // resource has no state param) and keeps its pre-P120 upper-case spelling.
    assert_eq!(task_total(&fixture, "state=COMPLETED").await, 2);

    // taskCandidateGroup carries the implicit `ASSIGNEE_ is null` gate
    // (HistoricTaskInstance.xml:1485-1487), so the assigned task is excluded even
    // though it also declares the `sales` candidate group.
    let by_candidate_group = get_ok(
        &fixture.client,
        &fixture.base_url,
        "/cmmn-history/historic-task-instances?taskCandidateGroup=sales",
    )
    .await;
    assert_eq!(total(&by_candidate_group), 1);
    assert_eq!(ids(&by_candidate_group), vec![fixture.candidate_task.clone()]);

    // ignoreTaskAssignee drops that gate
    // (HistoricTaskInstanceCollectionResource.java:289-291).
    let ignoring_assignee = get_ok(
        &fixture.client,
        &fixture.base_url,
        "/cmmn-history/historic-task-instances?taskCandidateGroup=sales&ignoreTaskAssignee=true",
    )
    .await;
    assert_eq!(total(&ignoring_assignee), 2);
    assert!(ids(&ignoring_assignee).contains(&fixture.assigned_task));

    // taskInvolvedUser matches the assignee, the owner, or any identity link, and
    // carries no assignee gate.
    assert_eq!(task_total(&fixture, "taskInvolvedUser=fozzie").await, 1);
    assert_eq!(task_total(&fixture, "taskInvolvedUser=gonzo").await, 1);
    let involved_candidate = get_ok(
        &fixture.client,
        &fixture.base_url,
        "/cmmn-history/historic-task-instances?taskInvolvedUser=kermit",
    )
    .await;
    assert_eq!(total(&involved_candidate), 1);
    assert_eq!(ids(&involved_candidate), vec![fixture.candidate_task.clone()]);
    assert_eq!(
        involved_candidate["data"][0]["caseInstanceId"],
        fixture.candidate
    );

    // Sorting runs before the page window here too.
    let sorted = get_ok(
        &fixture.client,
        &fixture.base_url,
        "/cmmn-history/historic-task-instances?sort=name&order=asc&start=0&size=1",
    )
    .await;
    assert_eq!(total(&sorted), 5);
    assert_eq!(sorted["data"][0]["name"], "Assigned review");

    // The historic plan-item aliases share this handler, so they take the same
    // params.
    for alias in [
        "/cmmn-history/historic-plan-item-instances",
        "/cmmn-history/historic-planitem-instances",
    ] {
        let body = get_ok(
            &fixture.client,
            &fixture.base_url,
            &format!("{alias}?taskAssignee=fozzie"),
        )
        .await;
        assert_eq!(total(&body), 1, "alias {alias}");
    }
}

/// The intentional cuts stay rejected, with the structured `deny_unknown_fields`
/// error the pre-P120 surface produced.
#[tokio::test]
async fn historic_query_params_outside_the_rust_model_stay_rejected() {
    let fixture = setup("rest-p120-historic-param-cuts").await;

    let rejected = [
        // No priority / due date / description on the Rust historic human task.
        ("/cmmn-history/historic-task-instances", "taskPriority=50"),
        (
            "/cmmn-history/historic-task-instances",
            "taskDescription=whatever",
        ),
        ("/cmmn-history/historic-task-instances", "dueDateAfter=2000-01-01T00:00:00Z"),
        // Java's CMMN historic task resource never parses these; only the shared
        // engine interface has them.
        (
            "/cmmn-history/historic-task-instances",
            "taskCandidateUser=kermit",
        ),
        (
            "/cmmn-history/historic-task-instances",
            "taskCandidateOrAssigned=fozzie",
        ),
    ];

    for (path, query) in rejected {
        let response = fixture
            .client
            .get(format!("{}{path}?{query}", fixture.base_url))
            .basic_auth("admin", Some("test"))
            .send()
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            reqwest::StatusCode::BAD_REQUEST,
            "{path}?{query} must stay rejected"
        );
        let body: Value = response.json().await.unwrap();
        let param = query.split('=').next().unwrap();
        assert!(
            body["details"].as_str().unwrap().contains(param),
            "{path}?{query} error must name the param: {body}"
        );
    }

    // An unsupported sort property is a 400 rather than a silent no-op.
    let bad_sort = fixture
        .client
        .get(format!(
            "{}/cmmn-history/historic-case-instances?sort=bogusProperty",
            fixture.base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(bad_sort.status(), reqwest::StatusCode::BAD_REQUEST);
}
