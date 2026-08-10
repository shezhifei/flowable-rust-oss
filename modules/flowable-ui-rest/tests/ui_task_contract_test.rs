//! Task UI contract tests (stream B).

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::identity::entities::User;
use flowable_ui_rest::task::{
    create_rest_variable, rest_variable_value, router_with_engine, RestVariable, RestVariableScope,
};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use std::sync::Arc;
use tower::ServiceExt;

async fn body_json(res: axum::response::Response) -> Value {
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap_or(Value::Null)
}

fn test_engine() -> Arc<ProcessEngine> {
    let engine = Arc::new(ProcessEngine::new("ui-task-test".into()));
    engine.get_identity_service().save_user(User {
        id: "admin".into(),
        first_name: Some("Test".into()),
        last_name: Some("Admin".into()),
        email: Some("admin@example.com".into()),
        password: Some("test".into()),
        tenant_id: None,
    });
    engine
}

#[tokio::test]
async fn task_health_probe() {
    let app = router_with_engine(test_engine());
    let res = app
        .oneshot(
            Request::builder()
                .uri("/app/rest/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let v = body_json(res).await;
    assert_eq!(v["app"], "task");
    assert_eq!(v["engine"], true);
}

#[test]
fn rest_variable_types_cover_converters() {
    let cases = vec![
        ("s", json!("hi"), "string"),
        ("i", json!(7), "integer"),
        ("b", json!(false), "boolean"),
        ("d", json!(1.5), "double"),
    ];
    for (name, val, ty) in cases {
        let rv = create_rest_variable(name, Some(val.clone()), Some(RestVariableScope::Global), true);
        assert_eq!(rv.r#type.as_deref(), Some(ty), "name={name}");
        assert_eq!(rv.scope.as_deref(), Some("global"));
        let back = rest_variable_value(&rv).unwrap().unwrap();
        match ty {
            "double" => assert!((back.as_f64().unwrap() - 1.5).abs() < f64::EPSILON),
            _ => assert_eq!(back, val),
        }
    }
}

#[test]
fn rest_variable_serde_matches_java_field_names() {
    let rv = RestVariable {
        name: "amount".into(),
        r#type: Some("integer".into()),
        value: Some(json!(10)),
        scope: Some("local".into()),
        value_url: None,
    };
    let s = serde_json::to_value(&rv).unwrap();
    assert_eq!(s["name"], "amount");
    assert_eq!(s["type"], "integer");
    assert_eq!(s["value"], 10);
    assert_eq!(s["scope"], "local");
    assert!(s.get("valueUrl").is_none());
}

#[tokio::test]
async fn create_list_claim_complete_task_flow() {
    let engine = test_engine();
    let app = router_with_engine(Arc::clone(&engine));

    // Create standalone task
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/app/rest/tasks")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&json!({
                        "name": "Review invoice",
                        "description": "check totals"
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let created = body_json(res).await;
    let task_id = created["id"].as_str().unwrap().to_string();
    assert_eq!(created["name"], "Review invoice");
    assert_eq!(created["assignee"]["id"], "admin");

    // Query open tasks
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/app/rest/query/tasks")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&json!({ "assignment": "assignee", "size": 25 })).unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let list = body_json(res).await;
    assert!(list["total"].as_i64().unwrap() >= 1);
    assert!(list["data"].as_array().unwrap().iter().any(|t| t["id"] == task_id));

    // Comment before complete
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/app/rest/tasks/{task_id}/comments"))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&json!({ "message": "looks good" })).unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    // Claim (already assignee, should still succeed)
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(format!("/app/rest/tasks/{task_id}/action/claim"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    // Complete
    let res = app
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(format!("/app/rest/tasks/{task_id}/action/complete"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
}

#[tokio::test]
async fn workflow_users_lists_identity() {
    let app = router_with_engine(test_engine());
    let res = app
        .oneshot(
            Request::builder()
                .uri("/app/rest/workflow-users")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let v = body_json(res).await;
    assert!(v["data"].as_array().unwrap().iter().any(|u| u["id"] == "admin"));
}

#[tokio::test]
async fn content_create_and_list_for_task() {
    let app = router_with_engine(test_engine());
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/app/rest/tasks")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&json!({ "name": "with content" })).unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let task_id = body_json(res).await["id"].as_str().unwrap().to_string();

    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/app/rest/tasks/{task_id}/content"))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&json!({
                        "name": "note.txt",
                        "content": "hello",
                        "mimeType": "text/plain"
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let created = body_json(res).await;
    assert_eq!(created["name"], "note.txt");

    let res = app
        .oneshot(
            Request::builder()
                .uri(format!("/app/rest/tasks/{task_id}/content"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let list = body_json(res).await;
    assert!(list["data"].as_array().unwrap().iter().any(|c| c["name"] == "note.txt"));
}

#[tokio::test]
async fn debugger_gate_and_allowed_flag() {
    let app = router_with_engine(test_engine());
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/app/rest/debugger")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    // default false
    let v = body_json(res).await;
    assert_eq!(v, json!(false));

    let res = app
        .oneshot(
            Request::builder()
                .uri("/app/rest/debugger/breakpoints")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn assign_task_returns_representation() {
    let app = router_with_engine(test_engine());
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/app/rest/tasks")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&json!({ "name": "Assign me" })).unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let task_id = body_json(res).await["id"].as_str().unwrap().to_string();

    let res = app
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(format!("/app/rest/tasks/{task_id}/action/assign"))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&json!({ "assignee": "admin" })).unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let v = body_json(res).await;
    assert_eq!(v["assignee"]["id"], "admin");
}

/// Enforced deployments must act as the session user, not the
/// `FLOWABLE_UI_DEFAULT_USER` fallback: the disabled-mode dev identity stands
/// in for a real session here, and the queries/claims must follow it.
#[tokio::test]
async fn session_user_drives_task_queries_and_claims() {
    use flowable_engine::engine::query::Query as _;
    use flowable_ui_rest::auth::{AuthMode, UiAuthConfig};
    use flowable_ui_rest::ui_router_with_config;

    let engine = test_engine();
    engine.get_identity_service().save_user(User {
        id: "worker".into(),
        first_name: Some("Case".into()),
        last_name: Some("Worker".into()),
        email: None,
        password: Some("test".into()),
        tenant_id: None,
    });

    let make_task = |name: &str, assignee: &str| {
        let mut task = flowable_engine::task::Task::new(
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            name.to_string(),
        );
        task.assignee = Some(assignee.to_string());
        engine.get_task_service().create_task(task).unwrap().id
    };
    let admin_task = make_task("Admin paperwork", "admin");
    let worker_task = make_task("Worker paperwork", "worker");
    let mut unassigned = flowable_engine::task::Task::new(
        String::new(),
        String::new(),
        String::new(),
        String::new(),
        "Unclaimed paperwork".to_string(),
    );
    unassigned.assignee = None;
    let unassigned_task = engine.get_task_service().create_task(unassigned).unwrap().id;

    let config = Arc::new(UiAuthConfig {
        mode: AuthMode::Disabled,
        dev_user_id: "worker".to_string(),
        ..UiAuthConfig::default()
    });
    let app = ui_router_with_config(config).layer(axum::Extension(Arc::clone(&engine)));

    // "Assigned to me" follows the session user, not the fallback admin.
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/app/rest/query/tasks")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&json!({ "assignment": "assignee", "size": 25 })).unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = res.status();
    let body = body_json(res).await;
    assert!(status == StatusCode::OK, "query failed: {body}");
    let ids: Vec<&str> = body["data"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|t| t["id"].as_str())
        .collect();
    assert!(ids.contains(&worker_task.as_str()), "worker task listed: {ids:?}");
    assert!(!ids.contains(&admin_task.as_str()), "admin task hidden: {ids:?}");

    // Claiming takes the task for the session user.
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(format!("/app/rest/tasks/{unassigned_task}/action/claim"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let claim_status = res.status();
    let claim_body = body_json(res).await;
    assert!(
        claim_status == StatusCode::OK,
        "claim failed: {claim_body}"
    );
    let claimed = engine
        .get_task_service()
        .create_task_query()
        .list()
        .unwrap()
        .into_iter()
        .find(|t| t.id == unassigned_task)
        .unwrap();
    assert_eq!(claimed.assignee.as_deref(), Some("worker"));
}

/// Java flowable-ui-task `AccountResource.getAccount`: the workflow app reads
/// the current user before issuing any task query, so this endpoint must exist
/// and describe the session user.
#[tokio::test]
async fn account_returns_the_session_user() {
    use flowable_ui_rest::auth::{AuthMode, UiAuthConfig};
    use flowable_ui_rest::ui_router_with_config;

    let engine = test_engine();
    let config = Arc::new(UiAuthConfig {
        mode: AuthMode::Disabled,
        dev_user_id: "admin".to_string(),
        ..UiAuthConfig::default()
    });
    let app = ui_router_with_config(config).layer(axum::Extension(Arc::clone(&engine)));

    let res = app
        .oneshot(
            Request::builder()
                .uri("/app/rest/account")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = body_json(res).await;
    assert_eq!(body["id"], "admin");
    assert_eq!(body["fullName"], "Test Admin");
    assert!(body["groups"].is_array());
    assert!(body["privileges"].is_array());
}

/// Java `RuntimeDisplayJsonClientResource`: the workflow app's diagram view
/// (`display/displaymodel.js`) fetches these four paths directly, so the
/// display JSON must carry elements/flows with graphic info and highlighting.
#[tokio::test]
async fn display_json_endpoints_render_deployed_diagram() {
    const BPMN: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:bpmndi="http://www.omg.org/spec/BPMN/20100524/DI"
             xmlns:omgdc="http://www.omg.org/spec/DD/20100524/DC"
             xmlns:omgdi="http://www.omg.org/spec/DD/20100524/DI"
             targetNamespace="Examples">
    <process id="displayProcess" isExecutable="true">
        <startEvent id="start" />
        <sequenceFlow id="f1" sourceRef="start" targetRef="task1" />
        <userTask id="task1" name="User Task" />
        <sequenceFlow id="f2" sourceRef="task1" targetRef="end" />
        <endEvent id="end" />
    </process>
    <bpmndi:BPMNDiagram id="diagram">
        <bpmndi:BPMNPlane id="plane" bpmnElement="displayProcess">
            <bpmndi:BPMNShape id="start_di" bpmnElement="start">
                <omgdc:Bounds x="100" y="100" width="36" height="36" />
            </bpmndi:BPMNShape>
            <bpmndi:BPMNShape id="task1_di" bpmnElement="task1">
                <omgdc:Bounds x="200" y="90" width="100" height="80" />
            </bpmndi:BPMNShape>
            <bpmndi:BPMNShape id="end_di" bpmnElement="end">
                <omgdc:Bounds x="360" y="100" width="36" height="36" />
            </bpmndi:BPMNShape>
            <bpmndi:BPMNEdge id="f1_di" bpmnElement="f1">
                <omgdi:waypoint x="136" y="118" />
                <omgdi:waypoint x="200" y="118" />
            </bpmndi:BPMNEdge>
            <bpmndi:BPMNEdge id="f2_di" bpmnElement="f2">
                <omgdi:waypoint x="300" y="118" />
                <omgdi:waypoint x="360" y="118" />
            </bpmndi:BPMNEdge>
        </bpmndi:BPMNPlane>
    </bpmndi:BPMNDiagram>
</definitions>"#;

    let engine = test_engine();
    let app = router_with_engine(Arc::clone(&engine));

    engine
        .get_repository_service()
        .deploy(
            engine
                .get_repository_service()
                .create_deployment()
                .add_string("display.bpmn20.xml".to_string(), BPMN.to_string()),
        )
        .unwrap();
    let pd_id = engine
        .get_repository_service()
        .get_process_definition_ids()
        .unwrap()[0]
        .clone();

    // 1) Process-definition diagram carries elements/flows with graphic info.
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/app/rest/process-definitions/{pd_id}/model-json"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let v = body_json(res).await;
    let elements = v["elements"].as_array().unwrap();
    let task1 = elements.iter().find(|e| e["id"] == "task1").unwrap();
    assert_eq!(task1["type"], "UserTask");
    assert_eq!(task1["x"], 200.0);
    assert_eq!(task1["width"], 100.0);
    let flows = v["flows"].as_array().unwrap();
    assert!(flows.iter().any(|f| {
        f["id"] == "f1"
            && f["waypoints"]
                .as_array()
                .map(|w| !w.is_empty())
                .unwrap_or(false)
    }));

    // Start an instance so highlighting has data to work with.
    let pi = engine
        .get_runtime_service()
        .start_process_instance(
            engine
                .get_runtime_service()
                .create_process_instance_builder()
                .process_definition_id(pd_id),
        )
        .unwrap();

    // 2) Runtime instance diagram highlights completed/current activities.
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/app/rest/process-instances/{}/model-json", pi.id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let v = body_json(res).await;
    let elements = v["elements"].as_array().unwrap();
    let task1 = elements.iter().find(|e| e["id"] == "task1").unwrap();
    assert_eq!(task1["current"], true);
    assert_eq!(task1["completed"], false);
    let start = elements.iter().find(|e| e["id"] == "start").unwrap();
    assert_eq!(start["completed"], true);
    assert!(
        v["currentActivities"]
            .as_array()
            .unwrap()
            .iter()
            .any(|a| a == "task1")
    );
    assert!(
        v["completedActivities"]
            .as_array()
            .unwrap()
            .iter()
            .any(|a| a == "start")
    );

    // 3) Historic view highlights completed activities only.
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/app/rest/process-instances/history/{}/model-json",
                    pi.id
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let v = body_json(res).await;
    let elements = v["elements"].as_array().unwrap();
    let start = elements.iter().find(|e| e["id"] == "start").unwrap();
    assert_eq!(start["completed"], true);
    assert!(
        v["completedActivities"]
            .as_array()
            .unwrap()
            .iter()
            .any(|a| a == "start")
    );

    // 4) Debugger view: active instance falls through to the runtime display.
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/app/rest/process-instances/debugger/{}/model-json",
                    pi.id
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let v = body_json(res).await;
    let elements = v["elements"].as_array().unwrap();
    let task1 = elements.iter().find(|e| e["id"] == "task1").unwrap();
    assert_eq!(task1["current"], true);

    // 5) Unknown ids resolve to 404, like the other task-module lookups.
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/app/rest/process-instances/does-not-exist/model-json")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
    let res = app
        .oneshot(
            Request::builder()
                .uri("/app/rest/process-instances/history/does-not-exist/model-json")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
}

/// Java RelatedContentResource raw-content family: multipart upload + raw download.
#[tokio::test]
async fn raw_content_upload_download_and_temporary() {
    let app = router_with_engine(test_engine());

    // Create a task to attach content to.
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/app/rest/tasks")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&json!({ "name": "raw-content-task" })).unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let task_id = body_json(res).await["id"].as_str().unwrap().to_string();

    let boundary = "----flowableBoundary7MA4YWxkTrZu0gW";
    let body = format!(
        "--{boundary}\r\n\
         Content-Disposition: form-data; name=\"file\"; filename=\"note.txt\"\r\n\
         Content-Type: text/plain\r\n\
         \r\n\
         hello raw\r\n\
         --{boundary}--\r\n"
    );

    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/app/rest/tasks/{task_id}/raw-content"))
                .header(
                    "content-type",
                    format!("multipart/form-data; boundary={boundary}"),
                )
                .body(Body::from(body.clone()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK, "raw-content upload");
    let created = body_json(res).await;
    assert_eq!(created["name"], "note.txt");
    assert_eq!(created["contentAvailable"], true);
    let content_id = created["id"].as_str().unwrap().to_string();

    // /raw-content/text returns the JSON as a bare string body.
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/app/rest/tasks/{task_id}/raw-content/text"))
                .header(
                    "content-type",
                    format!("multipart/form-data; boundary={boundary}"),
                )
                .body(Body::from(body.replace("note.txt", "note2.txt").replace("hello raw", "text body")))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let text = String::from_utf8(
        res.into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .to_vec(),
    )
    .unwrap();
    let parsed: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(parsed["name"], "note2.txt");

    // Temporary content (no task/process) + raw download.
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/app/rest/content")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&json!({
                        "name": "temp.txt",
                        "content": "temp body",
                        "mimeType": "text/plain"
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let temp_id = body_json(res).await["id"].as_str().unwrap().to_string();

    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/app/rest/content/{content_id}/raw"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let raw = res.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(&raw[..], b"hello raw");

    let res = app
        .oneshot(
            Request::builder()
                .uri(format!("/app/rest/content/{temp_id}/raw"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let raw = res.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(&raw[..], b"temp body");
}

/// CMMN case-instance introspection endpoints (Java CaseInstanceResource).
#[tokio::test]
async fn cmmn_case_instance_introspection_endpoints() {
    use flowable_cmmn_engine::{
        CmmnCase, CmmnCaseInstanceStartRequest, CmmnCasePlanModel, CmmnDeploymentRequest,
        CmmnEventListener, CmmnHumanTask, CmmnModel, CmmnPlanItem,
    };

    let engine = test_engine();
    let cmmn = engine
        .get_config()
        .cmmn_engine
        .as_ref()
        .expect("ProcessEngine default config attaches an in-memory CMMN engine")
        .clone();

    let plan_model = CmmnCasePlanModel::new("case-plan-model", "Case plan model")
        .with_human_task(CmmnHumanTask::new("task-manual", "Manual task"))
        .with_plan_item(
            CmmnPlanItem::new("plan-item-manual", "task-manual")
                .with_manual_activation_rule("true"),
        )
        .with_event_listener(
            CmmnEventListener::new("approval-event-listener", "message")
                .with_name("Wait for approval")
                .with_event_name("approvalReceived"),
        )
        .with_plan_item(CmmnPlanItem::new(
            "plan-item-approval-event",
            "approval-event-listener",
        ));
    let model = CmmnModel::new(vec![CmmnCase::new(
        "case-ui-task-cmmn",
        "uiTaskCmmnCase",
        "UI task CMMN case",
        plan_model,
    )]);
    cmmn.deploy(
        CmmnDeploymentRequest::new("ui-task-cmmn-deployment")
            .with_resource("ui-task-cmmn.cmmn", model),
    )
    .expect("deploy");
    let case_instance = cmmn
        .runtime_service()
        .start_case_instance_by_key("uiTaskCmmnCase", CmmnCaseInstanceStartRequest::new())
        .expect("start");
    let case_id = case_instance.id.clone();
    let case_def_id = case_instance.case_definition_id.clone();

    let app = router_with_engine(Arc::clone(&engine));

    // start-form shells (no form key configured → id null)
    for uri in [
        format!("/app/rest/case-definitions/{case_def_id}/start-form"),
        format!("/app/rest/case-instances/{case_id}/start-form"),
    ] {
        let res = app
            .clone()
            .oneshot(Request::builder().uri(&uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK, "uri={uri}");
        let v = body_json(res).await;
        assert!(v["fields"].is_array());
    }

    // stages / milestones return ResultListDataRepresentation
    for uri in [
        format!("/app/rest/case-instances/{case_id}/active-stages"),
        format!("/app/rest/case-instances/{case_id}/ended-stages"),
        format!("/app/rest/case-instances/{case_id}/available-milestones"),
        format!("/app/rest/case-instances/{case_id}/ended-milestones"),
    ] {
        let res = app
            .clone()
            .oneshot(Request::builder().uri(&uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK, "uri={uri}");
        let v = body_json(res).await;
        assert!(v["data"].is_array(), "uri={uri} body={v}");
    }

    // available user-event-listeners include the message listener
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/app/rest/case-instances/{case_id}/available-user-event-listeners"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let v = body_json(res).await;
    let listeners = v["data"].as_array().unwrap();
    assert!(
        !listeners.is_empty(),
        "expected at least one available user-event-listener, got {v}"
    );
    let uel_id = listeners[0]["id"].as_str().unwrap().to_string();

    // enabled plan-item-instances (manual activation parks ENABLED)
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/app/rest/case-instances/{case_id}/enabled-planitem-instances"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let v = body_json(res).await;
    let enabled = v["data"].as_array().unwrap();
    assert!(
        !enabled.is_empty(),
        "expected enabled plan items for manual activation, got {v}"
    );
    let plan_item_id = enabled[0]["id"].as_str().unwrap().to_string();

    // start enabled plan item
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/app/rest/case-instances/{case_id}/enabled-planitem-instances/{plan_item_id}"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    // trigger user event listener
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/app/rest/case-instances/{case_id}/trigger-user-event-listener/{uel_id}"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK, "trigger uel");

    // CMMN display-json routes resolve (empty graphics ok)
    for uri in [
        format!("/app/rest/case-definitions/{case_def_id}/model-json"),
        format!("/app/rest/case-instances/{case_id}/model-json"),
        format!("/app/rest/case-instances/history/{case_id}/model-json"),
    ] {
        let res = app
            .clone()
            .oneshot(Request::builder().uri(&uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK, "uri={uri}");
    }
}
