use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_rest::run_server;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::net::TcpListener;

const SIMPLE_USER_TASK_PROCESS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
  <process id="migrationProcess" name="Migration Process" isExecutable="true">
    <startEvent id="start" />
    <sequenceFlow id="flow1" sourceRef="start" targetRef="review" />
    <userTask id="review" name="Review" />
    <sequenceFlow id="flow2" sourceRef="review" targetRef="end" />
    <endEvent id="end" />
  </process>
</definitions>"#;

const SAME_ACTIVITY_TARGET_PROCESS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
  <process id="migrationProcess" name="Migration Process" isExecutable="true">
    <startEvent id="start" />
    <sequenceFlow id="flow1" sourceRef="start" targetRef="review" />
    <userTask id="review" name="Review v2" />
    <sequenceFlow id="flow2" sourceRef="review" targetRef="approve" />
    <userTask id="approve" name="Approve" />
    <sequenceFlow id="flow3" sourceRef="approve" targetRef="end" />
    <endEvent id="end" />
  </process>
</definitions>"#;

const MAPPED_ACTIVITY_TARGET_PROCESS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
  <process id="migrationProcess" name="Migration Process" isExecutable="true">
    <startEvent id="start" />
    <sequenceFlow id="flow1" sourceRef="start" targetRef="approve" />
    <userTask id="approve" name="Approve" />
    <sequenceFlow id="flow2" sourceRef="approve" targetRef="end" />
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

async fn deploy_process(
    engine: &ProcessEngine,
    client: &reqwest::Client,
    base_url: &str,
) -> String {
    deploy_process_resource(engine, client, base_url, SIMPLE_USER_TASK_PROCESS).await
}

async fn deploy_process_resource(
    engine: &ProcessEngine,
    client: &reqwest::Client,
    base_url: &str,
    resource: &str,
) -> String {
    let response = client
        .post(format!("{base_url}/repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "Migration deployment",
            "resourceName": "migration-process.bpmn20.xml",
            "resource": resource
        }))
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success());

    engine
        .get_repository_service()
        .latest_process_definition_by_key("migrationProcess", None)
        .unwrap()
        .unwrap()
        .id
}

async fn start_process_instance(
    client: &reqwest::Client,
    base_url: &str,
    process_definition_id: &str,
) -> String {
    let response = client
        .post(format!("{base_url}/runtime/process-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "processDefinitionId": process_definition_id,
            "businessKey": "migration-safe-path"
        }))
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success());
    let body: Value = response.json().await.unwrap();
    body["id"].as_str().unwrap().to_string()
}

#[tokio::test]
async fn process_instance_validate_migration_accepts_wait_state_mappings() {
    let (engine, base_url, client) = spawn_server("rest-bpmn-migration-validation").await;
    let process_definition_id = deploy_process(&engine, &client, &base_url).await;
    let process_instance_id =
        start_process_instance(&client, &base_url, &process_definition_id).await;

    let missing_instance = client
        .post(format!(
            "{base_url}/runtime/process-instances/missing/validate-migration"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "migrationDocument": {
                "migrateToProcessDefinitionId": process_definition_id
            }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(missing_instance.status(), reqwest::StatusCode::NOT_FOUND);

    let missing_target = client
        .post(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}/validate-migration"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "migrationDocument": {
                "migrateToProcessDefinitionId": "missing-definition"
            }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(missing_target.status(), reqwest::StatusCode::NOT_FOUND);

    let validation = client
        .post(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}/validate-migration"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "migrationDocument": {
                "migrateToProcessDefinitionId": process_definition_id,
                "activityMigrationMappings": [
                    {
                        "fromActivityId": "review",
                        "toActivityId": "review"
                    }
                ]
            }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(validation.status(), reqwest::StatusCode::OK);
    let body: Value = validation.json().await.unwrap();
    assert_eq!(body["valid"], true);
    assert_eq!(body["sourceProcessDefinitionId"], process_definition_id);
    assert_eq!(body["targetProcessDefinitionId"], process_definition_id);
    assert_eq!(body["validationMessages"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn process_instance_migrate_allows_same_definition_safe_noop() {
    let (engine, base_url, client) = spawn_server("rest-bpmn-migration-safe-noop").await;
    let process_definition_id = deploy_process(&engine, &client, &base_url).await;
    let process_instance_id =
        start_process_instance(&client, &base_url, &process_definition_id).await;

    let response = client
        .post(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}/migrate"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "migrationDocument": {
                "migrateToProcessDefinitionId": process_definition_id
            }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::NO_CONTENT);

    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let instance = store
        .find_process_instance(&process_instance_id, &mut session)
        .unwrap();
    assert_eq!(instance.process_definition_id, process_definition_id);
    let _ = session.rollback();

    let mapped = client
        .post(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}/migrate"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "migrationDocument": {
                "migrateToProcessDefinitionId": process_definition_id,
                "activityMigrationMappings": [
                    {
                        "fromActivityId": "review",
                        "toActivityId": "review"
                    }
                ]
            }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(mapped.status(), reqwest::StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn process_instance_migrate_waiting_user_task_to_v2_same_activity_and_continue_on_target_definition()
 {
    let (engine, base_url, client) = spawn_server("rest-bpmn-migration-same-activity").await;
    let v1_definition_id = deploy_process(&engine, &client, &base_url).await;
    let process_instance_id = start_process_instance(&client, &base_url, &v1_definition_id).await;
    let v2_definition_id =
        deploy_process_resource(&engine, &client, &base_url, SAME_ACTIVITY_TARGET_PROCESS).await;

    let response = client
        .post(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}/migrate"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "migrationDocument": {
                "migrateToProcessDefinitionId": v2_definition_id
            }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::NO_CONTENT);

    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let instance = store
        .find_process_instance(&process_instance_id, &mut session)
        .unwrap();
    assert_eq!(instance.process_definition_id, v2_definition_id);
    assert_eq!(instance.process_definition_version, 2);
    let historic_instance = store
        .get_historic_process_instance(&process_instance_id, &mut session)
        .unwrap();
    assert_eq!(historic_instance.process_definition_id, v2_definition_id);
    let _ = session.rollback();

    let review_task = engine
        .get_task_service()
        .get_tasks_by_process_instance_id(process_instance_id.clone())
        .unwrap()
        .pop()
        .unwrap();
    assert_eq!(review_task.task_definition_key, "review");
    engine
        .get_task_service()
        .complete_task_by_id(review_task.id)
        .unwrap();

    let tasks = engine
        .get_task_service()
        .get_tasks_by_process_instance_id(process_instance_id)
        .unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].task_definition_key, "approve");
}

#[tokio::test]
async fn process_instance_migrate_waiting_user_task_with_activity_mapping() {
    let (engine, base_url, client) = spawn_server("rest-bpmn-migration-mapped-activity").await;
    let v1_definition_id = deploy_process(&engine, &client, &base_url).await;
    let process_instance_id = start_process_instance(&client, &base_url, &v1_definition_id).await;
    let v2_definition_id =
        deploy_process_resource(&engine, &client, &base_url, MAPPED_ACTIVITY_TARGET_PROCESS).await;

    let response = client
        .post(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}/migrate"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "migrationDocument": {
                "migrateToProcessDefinitionId": v2_definition_id,
                "activityMigrationMappings": [
                    {
                        "fromActivityId": "review",
                        "toActivityId": "approve"
                    }
                ]
            }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::NO_CONTENT);

    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let execution = store
        .snapshot_executions(&mut session)
        .into_values()
        .find(|execution| {
            execution.process_instance_id.as_deref() == Some(&process_instance_id)
                && execution.activity_id.as_deref() == Some("approve")
                && !execution.is_ended
        })
        .unwrap();
    assert_eq!(
        execution.process_definition_id.as_deref(),
        Some(v2_definition_id.as_str())
    );

    let task = engine
        .get_task_service()
        .get_tasks_by_process_instance_id(process_instance_id.clone())
        .unwrap()
        .pop()
        .unwrap();
    assert_eq!(task.task_definition_key, "approve");
    assert_eq!(task.name, "Approve");

    let historic_activity = store
        .find_historic_activity_instances_by_process_instance_id(&process_instance_id, &mut session)
        .into_iter()
        .find(|activity| activity.end_time.is_none())
        .unwrap();
    assert_eq!(historic_activity.activity_id, "approve");

    engine
        .get_task_service()
        .complete_task_by_id(task.id)
        .unwrap();
    let instance = store
        .find_process_instance(&process_instance_id, &mut session)
        .unwrap();
    assert!(instance.is_ended);
    let _ = session.rollback();
}

#[tokio::test]
async fn process_instance_migrate_accepts_activity_mappings_document_shape() {
    let (engine, base_url, client) = spawn_server("rest-bpmn-migration-activity-mappings").await;
    let v1_definition_id = deploy_process(&engine, &client, &base_url).await;
    let process_instance_id = start_process_instance(&client, &base_url, &v1_definition_id).await;
    let v2_definition_id =
        deploy_process_resource(&engine, &client, &base_url, MAPPED_ACTIVITY_TARGET_PROCESS).await;

    let request_body = json!({
        "toProcessDefinitionId": v2_definition_id,
        "activityMappings": [
            {
                "fromActivityId": "review",
                "toActivityId": "approve"
            }
        ]
    });

    let validation = client
        .post(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}/validate-migration"
        ))
        .basic_auth("admin", Some("test"))
        .json(&request_body)
        .send()
        .await
        .unwrap();
    assert_eq!(validation.status(), reqwest::StatusCode::OK);
    let validation_body: Value = validation.json().await.unwrap();
    assert_eq!(validation_body["valid"], true);
    assert_eq!(
        validation_body["validationMessages"]
            .as_array()
            .unwrap()
            .len(),
        0
    );

    let response = client
        .post(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}/migrate"
        ))
        .basic_auth("admin", Some("test"))
        .json(&request_body)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::NO_CONTENT);

    let task = engine
        .get_task_service()
        .get_tasks_by_process_instance_id(process_instance_id)
        .unwrap()
        .pop()
        .unwrap();
    assert_eq!(task.task_definition_key, "approve");
    assert_eq!(task.name, "Approve");
}

#[tokio::test]
async fn process_instance_migration_expands_multiple_from_activity_ids_to_single_target() {
    let (engine, base_url, client) = spawn_server("rest-bpmn-migration-multi-from").await;
    let v1_definition_id = deploy_process(&engine, &client, &base_url).await;
    let process_instance_id = start_process_instance(&client, &base_url, &v1_definition_id).await;
    let v2_definition_id =
        deploy_process_resource(&engine, &client, &base_url, MAPPED_ACTIVITY_TARGET_PROCESS).await;

    let request_body = json!({
        "migrationDocument": {
            "migrateToProcessDefinitionId": v2_definition_id,
            "activityMigrationMappings": [
                {
                    "fromActivityIds": ["review", "previousReview"],
                    "toActivityId": "approve"
                }
            ]
        }
    });

    let validation = client
        .post(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}/validate-migration"
        ))
        .basic_auth("admin", Some("test"))
        .json(&request_body)
        .send()
        .await
        .unwrap();
    assert_eq!(validation.status(), reqwest::StatusCode::OK);
    let validation_body: Value = validation.json().await.unwrap();
    assert_eq!(validation_body["valid"], true);
    assert_eq!(
        validation_body["validationMessages"]
            .as_array()
            .unwrap()
            .len(),
        0
    );

    let response = client
        .post(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}/migrate"
        ))
        .basic_auth("admin", Some("test"))
        .json(&request_body)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::NO_CONTENT);

    let task = engine
        .get_task_service()
        .get_tasks_by_process_instance_id(process_instance_id)
        .unwrap()
        .pop()
        .unwrap();
    assert_eq!(task.task_definition_key, "approve");
    assert_eq!(task.name, "Approve");
}

#[tokio::test]
async fn process_instance_migration_splits_single_from_activity_id_to_multiple_targets() {
    let (engine, base_url, client) = spawn_server("rest-bpmn-migration-multi-to").await;
    let v1_definition_id = deploy_process(&engine, &client, &base_url).await;
    let process_instance_id = start_process_instance(&client, &base_url, &v1_definition_id).await;
    let v2_definition_id =
        deploy_process_resource(&engine, &client, &base_url, SAME_ACTIVITY_TARGET_PROCESS).await;
    let request_body = json!({
        "migrationDocument": {
            "migrateToProcessDefinitionId": v2_definition_id,
            "activityMigrationMappings": [
                {
                    "fromActivityId": "review",
                    "toActivityIds": ["review", "approve"]
                }
            ]
        }
    });

    let validation = client
        .post(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}/validate-migration"
        ))
        .basic_auth("admin", Some("test"))
        .json(&request_body)
        .send()
        .await
        .unwrap();
    assert_eq!(validation.status(), reqwest::StatusCode::OK);
    let validation_body: Value = validation.json().await.unwrap();
    assert_eq!(validation_body["valid"], true);
    assert_eq!(
        validation_body["validationMessages"]
            .as_array()
            .unwrap()
            .len(),
        0
    );

    let response = client
        .post(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}/migrate"
        ))
        .basic_auth("admin", Some("test"))
        .json(&request_body)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::NO_CONTENT);

    let mut task_keys = engine
        .get_task_service()
        .get_tasks_by_process_instance_id(process_instance_id)
        .unwrap()
        .into_iter()
        .map(|task| task.task_definition_key)
        .collect::<Vec<_>>();
    task_keys.sort();
    assert_eq!(task_keys, vec!["approve".to_string(), "review".to_string()]);
}

#[tokio::test]
async fn process_instance_migration_rejects_many_from_activity_ids_to_many_targets() {
    let (engine, base_url, client) = spawn_server("rest-bpmn-migration-many-to-many").await;
    let v1_definition_id = deploy_process(&engine, &client, &base_url).await;
    let process_instance_id = start_process_instance(&client, &base_url, &v1_definition_id).await;
    let v2_definition_id =
        deploy_process_resource(&engine, &client, &base_url, SAME_ACTIVITY_TARGET_PROCESS).await;
    let request_body = json!({
        "migrationDocument": {
            "migrateToProcessDefinitionId": v2_definition_id,
            "activityMigrationMappings": [
                {
                    "fromActivityIds": ["review", "previousReview"],
                    "toActivityIds": ["review", "approve"]
                }
            ]
        }
    });
    let expected_message =
        "Only single-to-many or many-to-single activity migration mappings are supported";

    let validation = client
        .post(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}/validate-migration"
        ))
        .basic_auth("admin", Some("test"))
        .json(&request_body)
        .send()
        .await
        .unwrap();
    assert_eq!(validation.status(), reqwest::StatusCode::OK);
    let validation_body: Value = validation.json().await.unwrap();
    assert_eq!(validation_body["valid"], false);
    assert!(
        validation_body["validationMessages"]
            .as_array()
            .unwrap()
            .iter()
            .any(|message| message.as_str() == Some(expected_message))
    );

    let response = client
        .post(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}/migrate"
        ))
        .basic_auth("admin", Some("test"))
        .json(&request_body)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
    assert!(response.text().await.unwrap().contains(expected_message));
}

#[tokio::test]
async fn process_definition_migrate_and_batch_migrate_validate_ids_and_safe_noop() {
    let (engine, base_url, client) = spawn_server("rest-bpmn-definition-migration").await;
    let process_definition_id = deploy_process(&engine, &client, &base_url).await;
    let process_instance_id =
        start_process_instance(&client, &base_url, &process_definition_id).await;

    let missing_source = client
        .post(format!(
            "{base_url}/repository/process-definitions/missing/migrate"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "migrationDocument": {
                "migrateToProcessDefinitionId": process_definition_id
            }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(missing_source.status(), reqwest::StatusCode::NOT_FOUND);

    let missing_target = client
        .post(format!(
            "{base_url}/repository/process-definitions/{process_definition_id}/migrate"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "migrationDocument": {
                "migrateToProcessDefinitionId": "missing-definition"
            }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(missing_target.status(), reqwest::StatusCode::NOT_FOUND);

    let migrate = client
        .post(format!(
            "{base_url}/repository/process-definitions/{process_definition_id}/migrate"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "migrationDocument": {
                "migrateToProcessDefinitionId": process_definition_id
            }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(migrate.status(), reqwest::StatusCode::NO_CONTENT);

    let batch_migrate = client
        .post(format!(
            "{base_url}/repository/process-definitions/{process_definition_id}/batch-migrate"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "migrationDocument": {
                "migrateToProcessDefinitionId": process_definition_id
            }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(batch_migrate.status(), reqwest::StatusCode::OK);
    let batch_body: Value = batch_migrate.json().await.unwrap();
    assert_eq!(batch_body["batchType"], "process-instance-migration");
    assert_eq!(batch_body["status"], "completed");
    assert_eq!(batch_body["itemsProcessed"], 1);

    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let instance = store
        .find_process_instance(&process_instance_id, &mut session)
        .unwrap();
    assert_eq!(instance.process_definition_id, process_definition_id);
    let _ = session.rollback();
}
