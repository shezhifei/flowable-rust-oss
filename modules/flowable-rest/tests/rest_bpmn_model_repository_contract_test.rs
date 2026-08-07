use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_rest::run_server;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::net::TcpListener;

const MODEL_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             targetNamespace="Examples">
    <process id="modelRepositoryProcess" name="Model Repository Process" isExecutable="true">
        <startEvent id="start" />
        <sequenceFlow id="flow1" sourceRef="start" targetRef="task1" />
        <userTask id="task1" name="Review model" />
        <sequenceFlow id="flow2" sourceRef="task1" targetRef="end" />
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
async fn bpmn_repository_model_endpoints_return_deployed_model_source_and_extra() {
    let (_engine, base_url, client) = spawn_server("rest-bpmn-model-repository").await;

    let deploy_response = client
        .post(format!("{base_url}/repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "Model repository deployment",
            "resourceName": "model-repository-process.bpmn20.xml",
            "resource": MODEL_XML
        }))
        .send()
        .await
        .unwrap();
    assert!(deploy_response.status().is_success());
    let deployment: Value = deploy_response.json().await.unwrap();
    let deployment_id = deployment["id"].as_str().unwrap();

    let definitions_response = client
        .get(format!(
            "{base_url}/repository/process-definitions?keyLike=modelRepository%&nameLike=%Repository%&deploymentId={deployment_id}&latest=true&suspended=false&sort=version&order=desc"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(definitions_response.status(), reqwest::StatusCode::OK);
    let definitions_body: Value = definitions_response.json().await.unwrap();
    assert_eq!(definitions_body["total"], 1);
    let definition = &definitions_body["data"][0];
    assert_eq!(definition["key"], "modelRepositoryProcess");
    assert_eq!(
        definition["url"],
        format!(
            "/repository/process-definitions/{}",
            definition["id"].as_str().unwrap()
        )
    );
    assert_eq!(definition["deploymentId"], deployment_id);
    assert_eq!(definition["suspended"], false);
    assert_eq!(definition["graphicalNotationDefined"], false);
    assert_eq!(definition["startFormDefined"], false);

    let list_response = client
        .get(format!(
            "{base_url}/repository/models?key=modelRepositoryProcess"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(list_response.status(), reqwest::StatusCode::OK);
    let list_body: Value = list_response.json().await.unwrap();
    assert_eq!(list_body["total"], 1);
    let listed = &list_body["data"][0];
    assert_eq!(listed["key"], "modelRepositoryProcess");
    assert_eq!(listed["name"], "Model Repository Process");
    assert_eq!(listed["deploymentId"], deployment_id);

    let model_id = listed["id"].as_str().unwrap();
    assert_eq!(
        listed["sourceUrl"],
        format!("/repository/models/{model_id}/source")
    );
    assert_eq!(
        listed["sourceExtraUrl"],
        format!("/repository/models/{model_id}/source-extra")
    );

    let detail_response = client
        .get(format!("{base_url}/repository/models/{model_id}"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(detail_response.status(), reqwest::StatusCode::OK);
    let detail_body: Value = detail_response.json().await.unwrap();
    assert_eq!(detail_body["id"], model_id);
    assert_eq!(detail_body["key"], "modelRepositoryProcess");
    assert_eq!(detail_body["deploymentId"], deployment_id);
    assert!(detail_body["version"].as_i64().unwrap() >= 1);

    let source_response = client
        .get(format!("{base_url}/repository/models/{model_id}/source"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(source_response.status(), reqwest::StatusCode::OK);
    assert_eq!(
        source_response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .unwrap(),
        "application/xml"
    );
    let source_body = source_response.text().await.unwrap();
    assert_eq!(source_body, MODEL_XML);

    let extra_response = client
        .get(format!(
            "{base_url}/repository/models/{model_id}/source-extra"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(extra_response.status(), reqwest::StatusCode::OK);
    assert_eq!(
        extra_response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .unwrap(),
        "application/json"
    );
    let extra_body: Value = extra_response.json().await.unwrap();
    assert_eq!(
        extra_body["processes"][0]["name"],
        "Model Repository Process"
    );
    assert!(
        !extra_body["processes"][0]["flowElements"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn bpmn_repository_model_crud_matches_rest_contract() {
    let (_engine, base_url, client) = spawn_server("rest-bpmn-model-repository-crud").await;

    let create_response = client
        .post(format!("{base_url}/repository/models"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "Draft approval model",
            "key": "draftApproval",
            "category": "http://example.com/category",
            "metaInfo": "{\"description\":\"initial\"}",
            "tenantId": "tenant-a"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(create_response.status(), reqwest::StatusCode::CREATED);
    let created: Value = create_response.json().await.unwrap();
    let model_id = created["id"].as_str().unwrap();
    assert!(!model_id.is_empty());
    assert_eq!(created["name"], "Draft approval model");
    assert_eq!(created["key"], "draftApproval");
    assert_eq!(created["category"], "http://example.com/category");
    assert_eq!(created["version"], 1);
    assert_eq!(created["metaInfo"], "{\"description\":\"initial\"}");
    assert_eq!(created["tenantId"], "tenant-a");
    assert!(created["deploymentId"].is_null());
    assert_eq!(created["url"], format!("/repository/models/{model_id}"));
    assert_eq!(
        created["sourceUrl"],
        format!("/repository/models/{model_id}/source")
    );
    assert_eq!(
        created["sourceExtraUrl"],
        format!("/repository/models/{model_id}/source-extra")
    );

    let list_response = client
        .get(format!(
            "{base_url}/repository/models?key=draftApproval&tenantId=tenant-a"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(list_response.status(), reqwest::StatusCode::OK);
    let list_body: Value = list_response.json().await.unwrap();
    assert_eq!(list_body["total"], 1);
    assert_eq!(list_body["data"][0]["id"], model_id);

    let update_response = client
        .put(format!("{base_url}/repository/models/{model_id}"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "Draft approval model v2",
            "key": "draftApprovalV2",
            "category": "http://example.com/category/updated",
            "version": 7,
            "meta_info": "{\"description\":\"updated\"}",
            "tenant_id": "tenant-b"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(update_response.status(), reqwest::StatusCode::OK);
    let updated: Value = update_response.json().await.unwrap();
    assert_eq!(updated["id"], model_id);
    assert_eq!(updated["name"], "Draft approval model v2");
    assert_eq!(updated["key"], "draftApprovalV2");
    assert_eq!(updated["category"], "http://example.com/category/updated");
    assert_eq!(updated["version"], 7);
    assert_eq!(updated["metaInfo"], "{\"description\":\"updated\"}");
    assert_eq!(updated["tenantId"], "tenant-b");
    assert_ne!(updated["createTime"], updated["lastUpdateTime"]);

    let get_response = client
        .get(format!("{base_url}/repository/models/{model_id}"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(get_response.status(), reqwest::StatusCode::OK);
    let detail: Value = get_response.json().await.unwrap();
    assert_eq!(detail["key"], "draftApprovalV2");

    let delete_response = client
        .delete(format!("{base_url}/repository/models/{model_id}"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(delete_response.status(), reqwest::StatusCode::NO_CONTENT);

    let missing_after_delete = client
        .get(format!("{base_url}/repository/models/{model_id}"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(
        missing_after_delete.status(),
        reqwest::StatusCode::NOT_FOUND
    );
    let missing_body: Value = missing_after_delete.json().await.unwrap();
    assert_eq!(missing_body["code"], "NOT_FOUND");
    assert!(missing_body["details"].as_str().unwrap().contains(model_id));
}

#[tokio::test]
async fn bpmn_repository_model_source_put_persists_bytes_and_content_type() {
    let (_engine, base_url, client) = spawn_server("rest-bpmn-model-repository-source-put").await;

    let create_response = client
        .post(format!("{base_url}/repository/models"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "Writable source model",
            "key": "writableSource"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(create_response.status(), reqwest::StatusCode::CREATED);
    let created: Value = create_response.json().await.unwrap();
    let model_id = created["id"].as_str().unwrap();

    let source_bytes = br#"{"stencilset":{"namespace":"http://b3mn.org/stencilset/bpmn2.0#"},"properties":{"process_id":"writableSource"}}"#;
    let put_source = client
        .put(format!("{base_url}/repository/models/{model_id}/source"))
        .basic_auth("admin", Some("test"))
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(source_bytes.as_slice().to_vec())
        .send()
        .await
        .unwrap();
    assert_eq!(put_source.status(), reqwest::StatusCode::NO_CONTENT);

    let get_source = client
        .get(format!("{base_url}/repository/models/{model_id}/source"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(get_source.status(), reqwest::StatusCode::OK);
    assert_eq!(
        get_source
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .unwrap(),
        "application/json"
    );
    assert_eq!(get_source.bytes().await.unwrap().as_ref(), source_bytes);

    let extra_bytes = br#"{"editor":"flowable-rust","notes":["source-extra"]}"#;
    let put_extra = client
        .put(format!(
            "{base_url}/repository/models/{model_id}/source-extra"
        ))
        .basic_auth("admin", Some("test"))
        .header(
            reqwest::header::CONTENT_TYPE,
            "application/vnd.flowable.extra+json",
        )
        .body(extra_bytes.as_slice().to_vec())
        .send()
        .await
        .unwrap();
    assert_eq!(put_extra.status(), reqwest::StatusCode::NO_CONTENT);

    let get_extra = client
        .get(format!(
            "{base_url}/repository/models/{model_id}/source-extra"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(get_extra.status(), reqwest::StatusCode::OK);
    assert_eq!(
        get_extra
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .unwrap(),
        "application/vnd.flowable.extra+json"
    );
    assert_eq!(get_extra.bytes().await.unwrap().as_ref(), extra_bytes);

    let empty_extra = client
        .put(format!(
            "{base_url}/repository/models/{model_id}/source-extra"
        ))
        .basic_auth("admin", Some("test"))
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(Vec::new())
        .send()
        .await
        .unwrap();
    assert_eq!(empty_extra.status(), reqwest::StatusCode::NO_CONTENT);

    let get_empty_extra = client
        .get(format!(
            "{base_url}/repository/models/{model_id}/source-extra"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(get_empty_extra.status(), reqwest::StatusCode::OK);
    assert_eq!(
        get_empty_extra
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .unwrap(),
        "application/json"
    );
    assert!(get_empty_extra.bytes().await.unwrap().is_empty());
}

#[tokio::test]
async fn bpmn_repository_model_crud_returns_structured_bad_request_and_not_found() {
    let (_engine, base_url, client) = spawn_server("rest-bpmn-model-repository-crud-errors").await;

    let missing_key = client
        .post(format!("{base_url}/repository/models"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "Missing key"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(missing_key.status(), reqwest::StatusCode::BAD_REQUEST);
    let missing_key_body: Value = missing_key.json().await.unwrap();
    assert_eq!(missing_key_body["code"], "BAD_REQUEST");
    assert!(
        missing_key_body["details"]
            .as_str()
            .unwrap()
            .contains("key")
    );

    let blank_key = client
        .post(format!("{base_url}/repository/models"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "key": "   "
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(blank_key.status(), reqwest::StatusCode::BAD_REQUEST);
    let blank_key_body: Value = blank_key.json().await.unwrap();
    assert_eq!(blank_key_body["code"], "BAD_REQUEST");
    assert!(blank_key_body["details"].as_str().unwrap().contains("key"));

    let missing_update = client
        .put(format!("{base_url}/repository/models/missing-model"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "Missing"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(missing_update.status(), reqwest::StatusCode::NOT_FOUND);
    let missing_update_body: Value = missing_update.json().await.unwrap();
    assert_eq!(missing_update_body["code"], "NOT_FOUND");
    assert!(
        missing_update_body["details"]
            .as_str()
            .unwrap()
            .contains("missing-model")
    );

    let missing_delete = client
        .delete(format!("{base_url}/repository/models/missing-model"))
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
            .contains("missing-model")
    );

    let missing_source_update = client
        .put(format!("{base_url}/repository/models/missing-model/source"))
        .basic_auth("admin", Some("test"))
        .header(reqwest::header::CONTENT_TYPE, "application/xml")
        .body("<definitions />")
        .send()
        .await
        .unwrap();
    assert_eq!(
        missing_source_update.status(),
        reqwest::StatusCode::NOT_FOUND
    );
    let missing_source_update_body: Value = missing_source_update.json().await.unwrap();
    assert_eq!(missing_source_update_body["code"], "NOT_FOUND");
    assert!(
        missing_source_update_body["details"]
            .as_str()
            .unwrap()
            .contains("missing-model")
    );

    let missing_extra_update = client
        .put(format!(
            "{base_url}/repository/models/missing-model/source-extra"
        ))
        .basic_auth("admin", Some("test"))
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body("{}")
        .send()
        .await
        .unwrap();
    assert_eq!(
        missing_extra_update.status(),
        reqwest::StatusCode::NOT_FOUND
    );
    let missing_extra_update_body: Value = missing_extra_update.json().await.unwrap();
    assert_eq!(missing_extra_update_body["code"], "NOT_FOUND");
    assert!(
        missing_extra_update_body["details"]
            .as_str()
            .unwrap()
            .contains("missing-model")
    );
}
