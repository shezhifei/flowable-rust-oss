use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::engine::query::Query;
use flowable_engine::identity::entities::{IdentityLink, User};
use flowable_engine::runtime::process_instance::ProcessInstance;
use flowable_engine::task::Task;
use flowable_rest::run_server;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::net::TcpListener;

const PROCESS_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
    <process id="definitionIdentityLinkProcess" name="Definition Identity Link Process" isExecutable="true">
        <startEvent id="start" />
        <sequenceFlow id="flow1" sourceRef="start" targetRef="end" />
        <endEvent id="end" />
    </process>
</definitions>"#;

async fn spawn_server() -> (Arc<ProcessEngine>, String, reqwest::Client) {
    let engine = Arc::new(ProcessEngine::new(
        "rest-bpmn-process-definition-identity-links".to_string(),
    ));
    engine.get_identity_service().save_user(User {
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
async fn process_definition_identity_links_are_listed_and_deleted_without_touching_runtime_links() {
    let (engine, base_url, client) = spawn_server().await;

    let deploy_response = client
        .post(format!("{base_url}/repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "Definition identity links",
            "resourceName": "definition-identity-links.bpmn20.xml",
            "resource": PROCESS_XML
        }))
        .send()
        .await
        .unwrap();
    assert!(deploy_response.status().is_success());

    let process_definition_id = engine
        .get_repository_service()
        .latest_process_definition_by_key("definitionIdentityLinkProcess", None)
        .unwrap()
        .unwrap()
        .id;

    let identity_link_service = engine.get_identity_link_service();
    identity_link_service.add_identity_link(IdentityLink {
        id: format!("process-definition:{process_definition_id}:users:kermit"),
        link_type: "candidate".to_string(),
        user_id: Some("kermit".to_string()),
        group_id: None,
        task_id: None,
        process_instance_id: None,
        process_definition_id: Some(process_definition_id.clone()),
    });
    identity_link_service.add_identity_link(IdentityLink {
        id: format!("process-definition:{process_definition_id}:groups:management"),
        link_type: "candidate".to_string(),
        user_id: None,
        group_id: Some("management".to_string()),
        task_id: None,
        process_instance_id: None,
        process_definition_id: Some(process_definition_id.clone()),
    });
    identity_link_service.add_identity_link(IdentityLink {
        id: "runtime-task:groups:management".to_string(),
        link_type: "candidate".to_string(),
        user_id: None,
        group_id: Some("management".to_string()),
        task_id: Some("task-1".to_string()),
        process_instance_id: None,
        process_definition_id: None,
    });

    let list_response = client
        .get(format!(
            "{base_url}/repository/process-definitions/{process_definition_id}/identitylinks"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(list_response.status(), reqwest::StatusCode::OK);
    let list_body: Value = list_response.json().await.unwrap();
    assert_eq!(list_body.as_array().unwrap().len(), 2);
    assert_eq!(list_body[0]["type"], "candidate");
    assert!(list_body.as_array().unwrap().iter().any(|link| {
        link["user"] == "kermit" && link["group"].is_null() && link["type"] == "candidate"
    }));
    assert!(list_body.as_array().unwrap().iter().any(|link| {
        link["group"] == "management" && link["user"].is_null() && link["type"] == "candidate"
    }));

    let user_link_response = client
        .get(format!(
            "{base_url}/repository/process-definitions/{process_definition_id}/identitylinks/USERS/kermit"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(user_link_response.status(), reqwest::StatusCode::OK);
    let user_link_body: Value = user_link_response.json().await.unwrap();
    assert_eq!(user_link_body["user"], "kermit");

    let delete_response = client
        .delete(format!(
            "{base_url}/repository/process-definitions/{process_definition_id}/identitylinks/GROUPS/management"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(delete_response.status(), reqwest::StatusCode::NO_CONTENT);

    let after_delete_response = client
        .get(format!(
            "{base_url}/repository/process-definitions/{process_definition_id}/identitylinks"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(after_delete_response.status(), reqwest::StatusCode::OK);
    let after_delete_body: Value = after_delete_response.json().await.unwrap();
    assert_eq!(after_delete_body.as_array().unwrap().len(), 1);
    assert_eq!(after_delete_body[0]["user"], "kermit");

    let runtime_group_link = identity_link_service
        .create_identity_link_query()
        .group_id("management".to_string())
        .list()
        .unwrap();
    assert_eq!(runtime_group_link.len(), 1);
    assert_eq!(runtime_group_link[0].task_id.as_deref(), Some("task-1"));
}

#[tokio::test]
async fn runtime_identity_links_use_camel_case_and_keep_snake_case_query_aliases() {
    let (_engine, base_url, client) = spawn_server().await;

    let create_response = client
        .post(format!("{base_url}/runtime/identity-links"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "id": "identity-link-contract-1",
            "linkType": "candidate",
            "userId": "kermit",
            "groupId": null,
            "taskId": "task-1",
            "processInstanceId": "process-instance-1",
            "processDefinitionId": "process-definition-1"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(create_response.status(), reqwest::StatusCode::OK);
    let created: Value = create_response.json().await.unwrap();
    assert_eq!(created["id"], "identity-link-contract-1");
    assert_eq!(created["linkType"], "candidate");
    assert_eq!(created["userId"], "kermit");
    assert_eq!(created["taskId"], "task-1");
    assert_eq!(created["processInstanceId"], "process-instance-1");
    assert_eq!(created["processDefinitionId"], "process-definition-1");
    assert!(created.get("link_type").is_none());
    assert!(created.get("user_id").is_none());
    assert!(created.get("process_definition_id").is_none());

    let snake_create_response = client
        .post(format!("{base_url}/runtime/identity-links"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "id": "identity-link-contract-2",
            "link_type": "participant",
            "group_id": "management",
            "process_definition_id": "process-definition-2"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(snake_create_response.status(), reqwest::StatusCode::OK);
    let snake_created: Value = snake_create_response.json().await.unwrap();
    assert_eq!(snake_created["linkType"], "participant");
    assert_eq!(snake_created["groupId"], "management");
    assert_eq!(snake_created["processDefinitionId"], "process-definition-2");
    assert!(snake_created.get("group_id").is_none());

    let camel_query_response = client
        .get(format!(
            "{base_url}/runtime/identity-links?userId=kermit&linkType=candidate"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(camel_query_response.status(), reqwest::StatusCode::OK);
    let camel_query_body: Value = camel_query_response.json().await.unwrap();
    assert_eq!(camel_query_body.as_array().unwrap().len(), 1);
    assert_eq!(camel_query_body[0]["linkType"], "candidate");
    assert!(camel_query_body[0].get("link_type").is_none());

    let snake_alias_response = client
        .get(format!(
            "{base_url}/runtime/identity-links?process_definition_id=process-definition-1"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(snake_alias_response.status(), reqwest::StatusCode::OK);
    let snake_alias_body: Value = snake_alias_response.json().await.unwrap();
    assert_eq!(snake_alias_body.as_array().unwrap().len(), 1);
    assert_eq!(
        snake_alias_body[0]["processDefinitionId"],
        "process-definition-1"
    );

    let service_alias_response = client
        .get(format!("{base_url}/runtime/identity-links?taskId=task-1"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(service_alias_response.status(), reqwest::StatusCode::OK);
    let service_alias_body: Value = service_alias_response.json().await.unwrap();
    assert_eq!(service_alias_body.as_array().unwrap().len(), 1);
    assert_eq!(service_alias_body[0]["taskId"], "task-1");
    assert!(service_alias_body[0].get("task_id").is_none());

    let other_type_response = client
        .post(format!("{base_url}/runtime/identity-links"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "id": "identity-link-contract-3",
            "linkType": "owner",
            "userId": "kermit",
            "taskId": "task-1"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(other_type_response.status(), reqwest::StatusCode::OK);

    let combined_alias_response = client
        .get(format!(
            "{base_url}/runtime/identity-links?user_id=kermit&linkType=candidate&task_id=task-1"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(combined_alias_response.status(), reqwest::StatusCode::OK);
    let combined_alias_body: Value = combined_alias_response.json().await.unwrap();
    assert_eq!(combined_alias_body.as_array().unwrap().len(), 1);
    assert_eq!(combined_alias_body[0]["id"], "identity-link-contract-1");

    let missing_delete_response = client
        .delete(format!("{base_url}/runtime/identity-links/missing-link"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(
        missing_delete_response.status(),
        reqwest::StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn task_identity_links_hyphen_paths_use_camel_case_and_service_aliases() {
    let (engine, base_url, client) = spawn_server().await;
    let task_id = "task-identity-link-contract";
    let mut session = engine.get_runtime_store().create_session().unwrap();
    engine.get_runtime_store().insert_task(
        &Task::new(
            task_id.to_string(),
            "process-instance-task-links".to_string(),
            "execution-task-links".to_string(),
            "reviewTask".to_string(),
            "Review task".to_string(),
        ),
        &mut session,
    );
    engine.get_runtime_store().insert_task(
        &Task::new(
            "unrelated-task".to_string(),
            "process-instance-task-links".to_string(),
            "execution-task-links".to_string(),
            "otherTask".to_string(),
            "Other task".to_string(),
        ),
        &mut session,
    );
    session.flush_and_commit().unwrap();
    engine
        .get_identity_link_service()
        .add_identity_link(IdentityLink {
            id: "task:unrelated-task:users:fozzie:type:candidate".to_string(),
            link_type: "candidate".to_string(),
            user_id: Some("fozzie".to_string()),
            group_id: None,
            task_id: Some("unrelated-task".to_string()),
            process_instance_id: None,
            process_definition_id: None,
        });

    let create_user_response = client
        .post(format!("{base_url}/runtime/tasks/{task_id}/identity-links"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "userId": "kermit",
            "type": "candidate"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(create_user_response.status(), reqwest::StatusCode::CREATED);
    let create_user_body: Value = create_user_response.json().await.unwrap();
    assert_eq!(create_user_body["user"], "kermit");
    assert!(create_user_body["group"].is_null());
    assert_eq!(create_user_body["type"], "candidate");
    assert_eq!(
        create_user_body["url"],
        format!("/runtime/tasks/{task_id}/identitylinks/users/kermit/candidate")
    );
    assert!(create_user_body.get("taskId").is_none());
    assert!(create_user_body.get("userId").is_none());
    assert!(create_user_body.get("groupId").is_none());

    let create_group_response = client
        .post(format!("{base_url}/runtime/tasks/{task_id}/identity-links"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "group_id": "management",
            "link_type": "candidate"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(create_group_response.status(), reqwest::StatusCode::CREATED);
    let create_group_body: Value = create_group_response.json().await.unwrap();
    assert_eq!(create_group_body["group"], "management");
    assert!(create_group_body["user"].is_null());
    assert_eq!(create_group_body["type"], "candidate");
    assert!(create_group_body.get("taskId").is_none());
    assert!(create_group_body.get("groupId").is_none());

    let list_response = client
        .get(format!("{base_url}/runtime/tasks/{task_id}/identity-links"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(list_response.status(), reqwest::StatusCode::OK);
    let list_body: Value = list_response.json().await.unwrap();
    assert_eq!(list_body.as_array().unwrap().len(), 2);
    assert!(list_body.as_array().unwrap().iter().all(|link| {
        link.as_object()
            .unwrap()
            .keys()
            .all(|key| matches!(key.as_str(), "url" | "user" | "group" | "type"))
    }));
    assert!(list_body.as_array().unwrap().iter().any(|link| {
        link["user"] == "kermit"
            && link["group"].is_null()
            && link["type"] == "candidate"
            && link["url"]
                == format!("/runtime/tasks/{task_id}/identitylinks/users/kermit/candidate")
    }));
    assert!(list_body.as_array().unwrap().iter().any(|link| {
        link["group"] == "management"
            && link["user"].is_null()
            && link["type"] == "candidate"
            && link["url"]
                == format!("/runtime/tasks/{task_id}/identitylinks/groups/management/candidate")
    }));

    let users_response = client
        .get(format!(
            "{base_url}/runtime/tasks/{task_id}/identity-links/USERS"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(users_response.status(), reqwest::StatusCode::OK);
    let users_body: Value = users_response.json().await.unwrap();
    assert_eq!(users_body.as_array().unwrap().len(), 1);
    assert_eq!(users_body[0]["user"], "kermit");
    assert!(users_body[0].get("taskId").is_none());

    let user_link_response = client
        .get(format!(
            "{base_url}/runtime/tasks/{task_id}/identity-links/Users/kermit/candidate"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(user_link_response.status(), reqwest::StatusCode::OK);
    let user_link_body: Value = user_link_response.json().await.unwrap();
    assert_eq!(user_link_body["user"], "kermit");
    assert!(user_link_body.get("taskId").is_none());
    assert_eq!(user_link_body["type"], "candidate");

    let delete_group_response = client
        .delete(format!(
            "{base_url}/runtime/tasks/{task_id}/identity-links/GROUPS/management/candidate"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(
        delete_group_response.status(),
        reqwest::StatusCode::NO_CONTENT
    );

    let after_delete_response = client
        .get(format!("{base_url}/runtime/tasks/{task_id}/identity-links"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(after_delete_response.status(), reqwest::StatusCode::OK);
    let after_delete_body: Value = after_delete_response.json().await.unwrap();
    assert_eq!(after_delete_body.as_array().unwrap().len(), 1);
    assert_eq!(after_delete_body[0]["user"], "kermit");
}

#[tokio::test]
async fn process_instance_identity_links_match_user_only_participant_contract() {
    let (engine, base_url, client) = spawn_server().await;
    let process_instance_id = "process-instance-identity-link-contract";
    let mut session = engine.get_runtime_store().create_session().unwrap();
    engine.get_runtime_store().insert_process_instance(
        &ProcessInstance {
            id: process_instance_id.to_string(),
            name: None,
            process_definition_id: "definition-identity-link-contract:1".to_string(),
            process_definition_key: "definitionIdentityLinkContract".to_string(),
            process_definition_name: None,
            process_definition_version: 1,
            business_key: None,
            business_status: None,
            is_suspended: false,
            tenant_id: None,
            start_time: None,
            start_user_id: None,
            callback_id: None,
            callback_type: None,
            reference_id: None,
            reference_type: None,
            is_ended: false,
            super_execution_id: None,
            root_process_instance_id: None,
        },
        &mut session,
    );
    session.flush_and_commit().unwrap();
    engine
        .get_identity_link_service()
        .add_identity_link(IdentityLink {
            id: "process-instance:unrelated:user:fozzie:type:participant".to_string(),
            link_type: "participant".to_string(),
            user_id: Some("fozzie".to_string()),
            group_id: None,
            task_id: None,
            process_instance_id: Some("unrelated-process-instance".to_string()),
            process_definition_id: None,
        });

    let create_response = client
        .post(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}/identity-links"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "user": "gonzo",
            "type": "participant"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(create_response.status(), reqwest::StatusCode::CREATED);
    let created: Value = create_response.json().await.unwrap();
    assert_eq!(
        created["url"],
        format!(
            "/runtime/process-instances/{process_instance_id}/identitylinks/users/gonzo/participant"
        )
    );
    assert_eq!(created["user"], "gonzo");
    assert!(created["group"].is_null());
    assert_eq!(created["type"], "participant");

    let create_group_response = client
        .post(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}/identitylinks"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "group": "management",
            "type": "participant"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        create_group_response.status(),
        reqwest::StatusCode::BAD_REQUEST
    );

    let list_response = client
        .get(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}/identity-links"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(list_response.status(), reqwest::StatusCode::OK);
    let listed: Value = list_response.json().await.unwrap();
    assert_eq!(listed.as_array().unwrap().len(), 1);
    assert_eq!(listed[0]["user"], "gonzo");
    assert_eq!(listed[0]["type"], "participant");

    let wrong_type_case_response = client
        .get(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}/identitylinks/users/gonzo/Participant"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(
        wrong_type_case_response.status(),
        reqwest::StatusCode::NOT_FOUND
    );

    let get_response = client
        .get(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}/identity-links/users/gonzo/participant"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(get_response.status(), reqwest::StatusCode::OK);
    let link: Value = get_response.json().await.unwrap();
    assert_eq!(link["user"], "gonzo");
    assert_eq!(link["type"], "participant");

    let delete_response = client
        .delete(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}/identitylinks/users/gonzo/participant"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(delete_response.status(), reqwest::StatusCode::NO_CONTENT);

    let after_delete_response = client
        .get(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}/identitylinks"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(after_delete_response.status(), reqwest::StatusCode::OK);
    let after_delete: Value = after_delete_response.json().await.unwrap();
    assert_eq!(after_delete.as_array().unwrap().len(), 0);
}
