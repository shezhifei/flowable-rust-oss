use std::{fs, sync::Arc};

use axum::{
    body::{Body, to_bytes},
    extract::Extension,
    http::{Request, StatusCode},
};
use flowable_engine::{
    engine::process_engine::ProcessEngine,
    identity::entities::{Group, User},
    repository::model::RepositoryModel,
};
use flowable_ui_rest::{
    auth::{AuthMode, UiAuthConfig},
    modeler,
};
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tower::ServiceExt;

const BPMN: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="https://flowable.org/modeler-test">
  <process id="leave" name="Leave approval" isExecutable="true">
    <startEvent id="start"/>
    <userTask id="review" name="Review request"/>
    <endEvent id="end"/>
    <sequenceFlow id="f1" sourceRef="start" targetRef="review"/>
    <sequenceFlow id="f2" sourceRef="review" targetRef="end"/>
  </process>
</definitions>"#;

const DMN: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="https://www.omg.org/spec/DMN/20191111/MODEL/" id="definitions" name="Eligibility" namespace="https://flowable.org/modeler-test">
  <decision id="eligibility" name="Eligibility">
    <decisionTable id="eligibilityTable" hitPolicy="FIRST">
      <input id="ageInput"><inputExpression id="ageExpression" typeRef="integer"><text>age</text></inputExpression></input>
      <output id="resultOutput" name="result" typeRef="string"/>
      <rule id="adultRule"><inputEntry id="adultInput"><text>&gt;= 18</text></inputEntry><outputEntry id="adultOutput"><text>"adult"</text></outputEntry></rule>
    </decisionTable>
  </decision>
</definitions>"#;

async fn spawn(test_name: &str) -> (Arc<ProcessEngine>, String, reqwest::Client) {
    let config = UiAuthConfig {
        mode: AuthMode::Disabled,
        ..UiAuthConfig::default()
    };
    spawn_with_config(test_name, config).await
}

async fn spawn_with_config(
    test_name: &str,
    config: UiAuthConfig,
) -> (Arc<ProcessEngine>, String, reqwest::Client) {
    let engine = Arc::new(ProcessEngine::new(test_name.to_string()));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let app = flowable_ui_rest::ui_router_with_config(Arc::new(config))
        .layer(Extension(Arc::clone(&engine)));
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (engine, base_url, reqwest::Client::new())
}

#[tokio::test]
async fn modeler_endpoints_require_a_ui_session_when_auth_is_enforced() {
    let (engine, base_url, client) =
        spawn_with_config("ui-modeler-auth-enforced", UiAuthConfig::default()).await;
    seed_model(
        &engine,
        "protected-model",
        "protected.bpmn20.xml",
        "application/xml",
        BPMN.as_bytes().to_vec(),
    );

    let response = client
        .get(format!(
            "{base_url}/modeler-app/rest/models/protected-model/editor/bpmn-json"
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(response.text().await.unwrap(), "");
}

fn seed_model(
    engine: &Arc<ProcessEngine>,
    id: &str,
    resource_name: &str,
    content_type: &str,
    source: Vec<u8>,
) {
    let repository = engine.get_repository_service();
    repository
        .create_repository_model(RepositoryModel {
            id: id.to_string(),
            name: Some(id.to_string()),
            key: id.to_string(),
            category: None,
            version: 1,
            meta_info: None,
            deployment_id: None,
            resource_name: Some(resource_name.to_string()),
            process_definition_id: None,
            tenant_id: None,
            create_time: 0,
            last_update_time: 0,
            source_content_type: content_type.to_string(),
            source_extra_content_type: "application/json".to_string(),
        })
        .unwrap();
    repository
        .update_repository_model_source(id, content_type.to_string(), source)
        .unwrap();
}

#[tokio::test]
async fn bpmn_editor_endpoints_convert_layout_validate_persist_and_render_png() {
    let (engine, base_url, client) = spawn("ui-modeler-bpmn").await;
    seed_model(
        &engine,
        "bpmn-model",
        "leave.bpmn20.xml",
        "application/xml",
        BPMN.as_bytes().to_vec(),
    );

    let editor_response = client
        .get(format!(
            "{base_url}/modeler-app/rest/models/bpmn-model/editor/bpmn-json"
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(editor_response.status(), StatusCode::OK);
    let mut document: Value = editor_response.json().await.unwrap();
    assert_eq!(document["schemaVersion"], "1.0");
    assert_eq!(document["model"]["processes"][0]["id"], "leave");
    assert_eq!(
        document["model"]["processes"][0]["flowElements"][1]["elementType"],
        "userTask"
    );

    let layout_response = client
        .post(format!("{base_url}/modeler-app/rest/editor/layout"))
        .json(&document)
        .send()
        .await
        .unwrap();
    assert_eq!(layout_response.status(), StatusCode::OK);
    let laid_out: Value = layout_response.json().await.unwrap();
    assert!(laid_out["model"]["locationMap"]["review"].is_object());

    let validation: Value = client
        .post(format!(
            "{base_url}/modeler-app/rest/models/bpmn-model/validate"
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(validation, json!({ "valid": true, "errors": [] }));

    let thumbnail = client
        .get(format!(
            "{base_url}/modeler-app/rest/models/bpmn-model/thumbnail"
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(thumbnail.status(), StatusCode::OK);
    assert_eq!(thumbnail.headers()["content-type"], "image/png");
    assert_eq!(&thumbnail.bytes().await.unwrap()[..8], b"\x89PNG\r\n\x1a\n");

    document["model"]["processes"][0]["name"] = json!("Updated leave approval");
    let update = client
        .put(format!(
            "{base_url}/modeler-app/rest/models/bpmn-model/editor/bpmn-json"
        ))
        .json(&document)
        .send()
        .await
        .unwrap();
    assert_eq!(update.status(), StatusCode::NO_CONTENT);
    let stored = engine
        .get_repository_service()
        .get_repository_model_source("bpmn-model")
        .unwrap();
    let stored = String::from_utf8(stored.bytes).unwrap();
    assert!(stored.contains("name=\"Updated leave approval\""));

    let missing = client
        .get(format!(
            "{base_url}/modeler-app/rest/models/missing/editor/bpmn-json"
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
    let missing: Value = missing.json().await.unwrap();
    assert_eq!(missing["messageKey"], "GENERAL.ERROR.NOT-FOUND");
}

#[tokio::test]
async fn dmn_and_form_editor_endpoints_roundtrip_and_reject_invalid_forms_without_writing() {
    let (engine, base_url, client) = spawn("ui-modeler-dmn-form").await;
    seed_model(
        &engine,
        "dmn-model",
        "eligibility.dmn.xml",
        "application/xml",
        DMN.as_bytes().to_vec(),
    );
    let form = json!({
        "schemaVersion": "1.0",
        "model": {
            "key": "leaveForm",
            "name": "Leave form",
            "fields": [],
            "outcomes": []
        }
    });
    seed_model(
        &engine,
        "form-model",
        "leave.form.json",
        "application/json",
        serde_json::to_vec(&form).unwrap(),
    );

    let mut dmn: Value = client
        .get(format!(
            "{base_url}/modeler-app/rest/models/dmn-model/editor/dmn-json"
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(dmn["model"]["decisions"][0]["id"], "eligibility");
    dmn["model"]["name"] = json!("Updated eligibility");
    let dmn_update = client
        .put(format!(
            "{base_url}/modeler-app/rest/models/dmn-model/editor/dmn-json"
        ))
        .json(&dmn)
        .send()
        .await
        .unwrap();
    assert_eq!(dmn_update.status(), StatusCode::NO_CONTENT);

    let mut form_document: Value = client
        .get(format!(
            "{base_url}/modeler-app/rest/form-models/form-model/editor/form-json"
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(form_document["model"]["key"], "leaveForm");
    form_document["model"]["name"] = json!("Updated leave form");
    let form_update = client
        .put(format!(
            "{base_url}/modeler-app/rest/form-models/form-model/editor/form-json"
        ))
        .json(&form_document)
        .send()
        .await
        .unwrap();
    assert_eq!(form_update.status(), StatusCode::NO_CONTENT);
    let persisted_valid = engine
        .get_repository_service()
        .get_repository_model_source("form-model")
        .unwrap()
        .bytes;

    form_document["model"]["key"] = json!("");
    let invalid_update = client
        .put(format!(
            "{base_url}/modeler-app/rest/form-models/form-model/editor/form-json"
        ))
        .json(&form_document)
        .send()
        .await
        .unwrap();
    assert_eq!(invalid_update.status(), StatusCode::BAD_REQUEST);
    let after_invalid = engine
        .get_repository_service()
        .get_repository_model_source("form-model")
        .unwrap()
        .bytes;
    assert_eq!(after_invalid, persisted_valid);
}

#[tokio::test]
async fn modeler_spa_serves_deep_links_without_shadowing_rest_routes() {
    let dist = tempfile::tempdir().unwrap();
    fs::write(
        dist.path().join("index.html"),
        "<!doctype html><title>modeler-test-shell</title>",
    )
    .unwrap();

    let app = modeler::router_with_static_dir(Some(dist.path()));
    let deep_link = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/modeler-app/models/bpmn-model")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(deep_link.status(), StatusCode::OK);
    let body = to_bytes(deep_link.into_body(), usize::MAX).await.unwrap();
    assert!(String::from_utf8_lossy(&body).contains("modeler-test-shell"));

    let rest = app
        .oneshot(
            Request::builder()
                .uri("/modeler-app/rest/models/missing/editor/bpmn-json")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(rest.status(), StatusCode::UNAUTHORIZED);
    let body = to_bytes(rest.into_body(), usize::MAX).await.unwrap();
    assert!(!String::from_utf8_lossy(&body).contains("modeler-test-shell"));
}

// ── Gap endpoints: import / editor users & groups / clone / parent-relations ──

fn bpmn_upload(file_name: &str, content: &str) -> reqwest::multipart::Form {
    reqwest::multipart::Form::new().part(
        "file",
        reqwest::multipart::Part::text(content.to_string()).file_name(file_name.to_string()),
    )
}

fn save_user(engine: &Arc<ProcessEngine>, id: &str, first: Option<&str>, last: Option<&str>) {
    engine.get_identity_service().save_user(User {
        id: id.to_string(),
        first_name: first.map(str::to_string),
        last_name: last.map(str::to_string),
        email: None,
        password: Some("test".to_string()),
        tenant_id: None,
    });
}

#[tokio::test]
async fn bpmn_import_endpoints_create_models_from_uploaded_xml() {
    let (engine, base_url, client) = spawn("ui-modeler-import-bpmn").await;

    // Java ModelsResource.importProcessModel (multipart file upload).
    let response = client
        .post(format!("{base_url}/modeler-app/rest/import-process-model"))
        .multipart(bpmn_upload("leave.bpmn20.xml", BPMN))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let model: Value = response.json().await.unwrap();
    assert_eq!(model["key"], "leave");
    assert_eq!(model["name"], "Leave approval");
    assert_eq!(model["modelType"], 0);
    assert_eq!(model["version"], 1);
    assert_eq!(model["latestVersion"], true);
    let imported_id = model["id"].as_str().unwrap().to_string();

    // The stored source is the XML itself, readable through the editor protocol.
    let stored = engine
        .get_repository_service()
        .get_repository_model_source(&imported_id)
        .unwrap();
    assert_eq!(String::from_utf8(stored.bytes).unwrap(), BPMN);
    let editor = client
        .get(format!(
            "{base_url}/modeler-app/rest/models/{imported_id}/editor/bpmn-json"
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(editor.status(), StatusCode::OK);

    // Java rejects unsupported file names and unparseable XML with 400.
    let response = client
        .post(format!("{base_url}/modeler-app/rest/import-process-model"))
        .multipart(bpmn_upload("leave.txt", BPMN))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let response = client
        .post(format!("{base_url}/modeler-app/rest/import-process-model"))
        .multipart(bpmn_upload("broken.bpmn", "not xml at all"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    // Java ModelsResource.importProcessModelText (first-party variant: JSON body).
    let response = client
        .post(format!(
            "{base_url}/modeler-app/rest/import-process-model/text"
        ))
        .json(&json!({ "xml": BPMN, "name": "Imported leave" }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let model: Value = response.json().await.unwrap();
    assert_eq!(model["key"], "leave");
    assert_eq!(model["name"], "Imported leave");

    // Java ApiModelsResource.importProcessModel (`/api/editor` servlet).
    let response = client
        .post(format!("{base_url}/api/editor/import-process-model"))
        .multipart(bpmn_upload("leave.bpmn", BPMN))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let model: Value = response.json().await.unwrap();
    assert_eq!(model["key"], "leave");
}

#[tokio::test]
async fn decision_table_import_endpoints_create_dmn_models() {
    let (engine, base_url, client) = spawn("ui-modeler-import-dmn").await;

    // Java DecisionTableResource.importDecisionTable.
    let response = client
        .post(format!(
            "{base_url}/modeler-app/rest/decision-table-models/import-decision-table"
        ))
        .multipart(bpmn_upload("eligibility.dmn", DMN))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let model: Value = response.json().await.unwrap();
    // Java: key is the first decision's id, name the DMN definition name.
    assert_eq!(model["key"], "eligibility");
    assert_eq!(model["name"], "Eligibility");
    assert_eq!(model["modelType"], 4);
    let imported_id = model["id"].as_str().unwrap().to_string();

    let editor = client
        .get(format!(
            "{base_url}/modeler-app/rest/models/{imported_id}/editor/dmn-json"
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(editor.status(), StatusCode::OK);
    let stored = engine
        .get_repository_service()
        .get_repository_model_source(&imported_id)
        .unwrap();
    assert_eq!(String::from_utf8(stored.bytes).unwrap(), DMN);

    // Java DecisionTableResource.importDecisionTableText.
    let response = client
        .post(format!(
            "{base_url}/modeler-app/rest/decision-table-models/import-decision-table-text"
        ))
        .json(&json!({ "xml": DMN }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let model: Value = response.json().await.unwrap();
    assert_eq!(model["key"], "eligibility");

    // Java rejects unsupported file names with 400.
    let response = client
        .post(format!(
            "{base_url}/modeler-app/rest/decision-table-models/import-decision-table"
        ))
        .multipart(bpmn_upload("eligibility.txt", DMN))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn editor_users_and_groups_list_idm_entries_with_optional_filter() {
    let (engine, base_url, client) = spawn("ui-modeler-editor-users").await;
    save_user(&engine, "bob", Some("Bob"), Some("Baker"));
    save_user(&engine, "carol", Some("Carol"), Some("Smith"));
    engine.get_identity_service().save_group(Group {
        id: "sales".to_string(),
        name: "Sales".to_string(),
        group_type: Some("assignment".to_string()),
    });
    engine.get_identity_service().save_group(Group {
        id: "engineering".to_string(),
        name: "Engineering".to_string(),
        group_type: Some("assignment".to_string()),
    });

    let users: Value = client
        .get(format!("{base_url}/modeler-app/rest/editor-users"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(users["size"], 2);
    assert_eq!(users["total"], 2);
    assert_eq!(users["start"], 0);
    let bob = &users["data"][0];
    assert_eq!(bob["id"], "bob");
    assert_eq!(bob["fullName"], "Bob Baker");

    let filtered: Value = client
        .get(format!("{base_url}/modeler-app/rest/editor-users?filter=carol"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(filtered["size"], 1);
    assert_eq!(filtered["data"][0]["id"], "carol");

    // Java orders groups by name ascending and filters on the name.
    let groups: Value = client
        .get(format!("{base_url}/modeler-app/rest/editor-groups"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(groups["size"], 2);
    assert_eq!(groups["data"][0]["id"], "engineering");
    assert_eq!(groups["data"][1]["id"], "sales");
    assert_eq!(groups["data"][1]["type"], "assignment");

    let filtered: Value = client
        .get(format!(
            "{base_url}/modeler-app/rest/editor-groups?filter=sal"
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(filtered["size"], 1);
    assert_eq!(filtered["data"][0]["id"], "sales");
}

#[tokio::test]
async fn clone_copies_model_and_source_and_parent_relations_follows_java_404() {
    let (engine, base_url, client) = spawn("ui-modeler-clone").await;
    seed_model(
        &engine,
        "bpmn-model",
        "leave.bpmn20.xml",
        "application/xml",
        BPMN.as_bytes().to_vec(),
    );

    // Java ModelsResource.duplicateModel with the default -copy key/name.
    let response = client
        .post(format!(
            "{base_url}/modeler-app/rest/models/bpmn-model/clone"
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let model: Value = response.json().await.unwrap();
    assert_eq!(model["key"], "bpmn-model-copy");
    assert_eq!(model["name"], "bpmn-model (copy)");
    assert_eq!(model["modelType"], 0);
    let clone_id = model["id"].as_str().unwrap().to_string();
    let clone_source = engine
        .get_repository_service()
        .get_repository_model_source(&clone_id)
        .unwrap();
    assert_eq!(String::from_utf8(clone_source.bytes).unwrap(), BPMN);

    // Java rejects a duplicate key with 409.
    let response = client
        .post(format!(
            "{base_url}/modeler-app/rest/models/bpmn-model/clone"
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);

    // Explicit name/key in the body win over the defaults.
    let response = client
        .post(format!(
            "{base_url}/modeler-app/rest/models/bpmn-model/clone"
        ))
        .json(&json!({ "name": "Leave copy", "key": "leaveCopy" }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let model: Value = response.json().await.unwrap();
    assert_eq!(model["key"], "leaveCopy");
    assert_eq!(model["name"], "Leave copy");

    // Cloning an unknown model is a 404 (Java: unknown original model).
    let response = client
        .post(format!("{base_url}/modeler-app/rest/models/missing/clone"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    // Java ModelRelationResource.getModelRelations: 404 for unknown models,
    // otherwise a (here always empty) ModelInformation list.
    let response = client
        .get(format!(
            "{base_url}/modeler-app/rest/models/bpmn-model/parent-relations"
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.json::<Value>().await.unwrap(), json!([]));
    let response = client
        .get(format!(
            "{base_url}/modeler-app/rest/models/missing/parent-relations"
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}
