use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::task::Task;
use flowable_rest::run_server;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::net::TcpListener;

#[tokio::test]
async fn bpmn_task_query_identitylink_runtime_and_history_paths_are_available() {
    let engine = Arc::new(ProcessEngine::new("rest-bpmn-second-wave".to_string()));

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
    let addr = listener.local_addr().unwrap();
    let base_url = format!("http://{}", addr);

    let engine_clone = Arc::clone(&engine);
    tokio::spawn(async move {
        run_server(engine_clone, listener).await.unwrap();
    });

    let client = reqwest::Client::new();
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="secondWaveProcess" name="Second Wave Process" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="f1" sourceRef="start" targetRef="task1" />
            <userTask id="task1" name="Review Request" />
            <sequenceFlow id="f2" sourceRef="task1" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#;

    let deploy_response = client
        .post(format!("{base_url}/repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "Second Wave Deployment",
            "resourceName": "second_wave_process.bpmn20.xml",
            "resource": xml
        }))
        .send()
        .await
        .unwrap();
    assert!(deploy_response.status().is_success());

    let process_definition_id = engine
        .get_repository_service()
        .get_process_definition_ids()
        .unwrap()[0]
        .clone();

    let start_response = client
        .post(format!("{base_url}/runtime/process-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "processDefinitionId": process_definition_id,
            "businessKey": "Second Wave Instance"
        }))
        .send()
        .await
        .unwrap();
    assert!(start_response.status().is_success());
    let start_body: Value = start_response.json().await.unwrap();
    let process_instance_id = start_body["id"].as_str().unwrap().to_string();

    let task_query_response = client
        .post(format!("{base_url}/query/tasks"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "processInstanceId": process_instance_id
        }))
        .send()
        .await
        .unwrap();
    assert!(task_query_response.status().is_success());
    let task_query_body: Value = task_query_response.json().await.unwrap();
    assert_eq!(task_query_body["total"], 1);
    let task_id = task_query_body["data"][0]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let execution_id = task_query_body["data"][0]["executionId"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(task_query_body["data"][0]["name"], "Review Request");

    let task_response = client
        .get(format!("{base_url}/runtime/tasks/{task_id}"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(task_response.status().is_success());
    let task_body: Value = task_response.json().await.unwrap();
    assert_eq!(task_body["id"], task_id);
    assert_eq!(task_body["processInstanceId"], process_instance_id);

    let mut child_task = Task::new(
        "manual-subtask-1".to_string(),
        process_instance_id.clone(),
        execution_id.clone(),
        "manualSubTask".to_string(),
        "Manual follow-up".to_string(),
    );
    child_task.parent_task_id = Some(task_id.clone());
    let mut session = engine.get_runtime_store().create_session().unwrap();
    engine
        .get_runtime_store()
        .insert_task(&child_task, &mut session);
    session.flush_and_commit().unwrap();

    let subtasks_response = client
        .get(format!("{base_url}/runtime/tasks/{task_id}/subtasks"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(subtasks_response.status().is_success());
    let subtasks_body: Value = subtasks_response.json().await.unwrap();
    assert_eq!(subtasks_body.as_array().unwrap().len(), 1);
    assert_eq!(subtasks_body[0]["id"], "manual-subtask-1");
    assert_eq!(subtasks_body[0]["name"], "Manual follow-up");
    assert_eq!(subtasks_body[0]["parentTaskId"], task_id);

    let missing_subtasks_response = client
        .get(format!("{base_url}/runtime/tasks/missing-task/subtasks"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(missing_subtasks_response.status(), 404);

    let create_task_link_response = client
        .post(format!("{base_url}/runtime/tasks/{task_id}/identitylinks"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "user": "kermit",
            "type": "candidate"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(create_task_link_response.status(), 201);

    let task_links_response = client
        .get(format!("{base_url}/runtime/tasks/{task_id}/identitylinks"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(task_links_response.status().is_success());
    let task_links: Value = task_links_response.json().await.unwrap();
    assert_eq!(task_links.as_array().unwrap().len(), 1);
    assert_eq!(task_links[0]["user"], "kermit");
    assert_eq!(task_links[0]["type"], "candidate");

    let historic_task_links_response = client
        .get(format!(
            "{base_url}/history/historic-task-instances/{task_id}/identitylinks"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(historic_task_links_response.status().is_success());
    let historic_task_links: Value = historic_task_links_response.json().await.unwrap();
    assert_eq!(historic_task_links.as_array().unwrap().len(), 1);
    assert_eq!(historic_task_links[0]["userId"], "kermit");
    assert_eq!(historic_task_links[0]["type"], "candidate");

    let task_user_links_response = client
        .get(format!(
            "{base_url}/runtime/tasks/{task_id}/identitylinks/users"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(task_user_links_response.status().is_success());
    let task_user_links: Value = task_user_links_response.json().await.unwrap();
    assert_eq!(task_user_links.as_array().unwrap().len(), 1);
    assert_eq!(task_user_links[0]["user"], "kermit");

    let task_link_response = client
        .get(format!(
            "{base_url}/runtime/tasks/{task_id}/identitylinks/users/kermit/candidate"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(task_link_response.status().is_success());
    let task_link: Value = task_link_response.json().await.unwrap();
    assert_eq!(task_link["user"], "kermit");
    assert_eq!(task_link["type"], "candidate");

    let delete_task_link_response = client
        .delete(format!(
            "{base_url}/runtime/tasks/{task_id}/identitylinks/users/kermit/candidate"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(delete_task_link_response.status(), 204);

    let task_links_after_delete_response = client
        .get(format!("{base_url}/runtime/tasks/{task_id}/identitylinks"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(task_links_after_delete_response.status().is_success());
    let task_links_after_delete: Value = task_links_after_delete_response.json().await.unwrap();
    assert_eq!(task_links_after_delete.as_array().unwrap().len(), 0);

    let create_process_link_response = client
        .post(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}/identitylinks"
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "user": "gonzo",
            "type": "participant"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(create_process_link_response.status(), 201);

    let process_link_response = client
        .get(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}/identitylinks/users/gonzo/participant"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(process_link_response.status().is_success());
    let process_link: Value = process_link_response.json().await.unwrap();
    assert_eq!(process_link["user"], "gonzo");
    assert_eq!(process_link["type"], "participant");

    let historic_process_links_response = client
        .get(format!(
            "{base_url}/history/historic-process-instances/{process_instance_id}/identitylinks"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(historic_process_links_response.status().is_success());
    let historic_process_links: Value = historic_process_links_response.json().await.unwrap();
    let historic_process_links = historic_process_links.as_array().unwrap();
    assert_eq!(historic_process_links.len(), 2);
    assert!(
        historic_process_links
            .iter()
            .any(|link| link["userId"] == "gonzo" && link["type"] == "participant")
    );
    assert!(
        historic_process_links
            .iter()
            .any(|link| link["userId"] == "admin" && link["type"] == "starter")
    );

    let delete_process_link_response = client
        .delete(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}/identitylinks/users/gonzo/participant"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(delete_process_link_response.status(), 204);

    let process_links_after_delete_response = client
        .get(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}/identitylinks"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(process_links_after_delete_response.status().is_success());
    let process_links_after_delete: Value =
        process_links_after_delete_response.json().await.unwrap();
    let process_links_after_delete = process_links_after_delete.as_array().unwrap();
    assert_eq!(process_links_after_delete.len(), 1);
    assert_eq!(process_links_after_delete[0]["user"], "admin");
    assert_eq!(process_links_after_delete[0]["type"], "starter");

    engine
        .get_variable_service()
        .set_variable(
            execution_id.clone(),
            "approval".to_string(),
            json!("accepted"),
        )
        .unwrap();

    let executions_response = client
        .post(format!("{base_url}/query/executions"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "processInstanceId": process_instance_id
        }))
        .send()
        .await
        .unwrap();
    assert!(executions_response.status().is_success());
    let executions_body: Value = executions_response.json().await.unwrap();
    assert!(executions_body["total"].as_u64().unwrap() >= 1);
    assert!(
        executions_body["data"]
            .as_array()
            .unwrap()
            .iter()
            .any(|execution| execution["id"] == execution_id)
    );

    let executions_list_response = client
        .get(format!(
            "{base_url}/runtime/executions?processInstanceId={process_instance_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(executions_list_response.status().is_success());
    let executions_list_body: Value = executions_list_response.json().await.unwrap();
    assert!(
        executions_list_body["data"]
            .as_array()
            .unwrap()
            .iter()
            .any(|execution| execution["id"] == execution_id)
    );

    let execution_get_response = client
        .get(format!("{base_url}/runtime/executions/{execution_id}"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(execution_get_response.status().is_success());
    let execution_get_body: Value = execution_get_response.json().await.unwrap();
    assert_eq!(execution_get_body["id"], execution_id);
    assert_eq!(execution_get_body["processInstanceId"], process_instance_id);

    let execution_activities_response = client
        .get(format!(
            "{base_url}/runtime/executions/{execution_id}/activities"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(execution_activities_response.status().is_success());
    let execution_activities_body: Value = execution_activities_response.json().await.unwrap();
    assert!(
        execution_activities_body
            .as_array()
            .unwrap()
            .iter()
            .any(|activity| activity == "task1")
    );

    let activity_instances_response = client
        .post(format!("{base_url}/query/activity-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "processInstanceId": process_instance_id
        }))
        .send()
        .await
        .unwrap();
    assert!(activity_instances_response.status().is_success());
    let activity_instances_body: Value = activity_instances_response.json().await.unwrap();
    assert!(
        activity_instances_body["data"]
            .as_array()
            .unwrap()
            .iter()
            .any(|activity| activity["activityId"] == "task1")
    );

    let runtime_activity_instances_response = client
        .get(format!(
            "{base_url}/runtime/activity-instances?processInstanceId={process_instance_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(runtime_activity_instances_response.status().is_success());
    let runtime_activity_instances_body: Value =
        runtime_activity_instances_response.json().await.unwrap();
    assert!(
        runtime_activity_instances_body["data"]
            .as_array()
            .unwrap()
            .iter()
            .any(|activity| activity["activityId"] == "task1")
    );

    let variable_instances_response = client
        .post(format!("{base_url}/query/variable-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "processInstanceId": process_instance_id
        }))
        .send()
        .await
        .unwrap();
    assert!(variable_instances_response.status().is_success());
    let variable_instances_body: Value = variable_instances_response.json().await.unwrap();
    assert_eq!(variable_instances_body["total"], 1);
    assert_eq!(variable_instances_body["data"][0]["name"], "approval");
    assert_eq!(variable_instances_body["data"][0]["value"], "accepted");

    let historic_task_response = client
        .post(format!("{base_url}/query/historic-task-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "processInstanceId": process_instance_id
        }))
        .send()
        .await
        .unwrap();
    assert!(historic_task_response.status().is_success());
    let historic_task_body: Value = historic_task_response.json().await.unwrap();
    assert_eq!(historic_task_body["total"], 1);
    assert_eq!(historic_task_body["data"][0]["id"], task_id);

    let historic_task_list_response = client
        .get(format!(
            "{base_url}/history/historic-task-instances?processInstanceId={process_instance_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(historic_task_list_response.status().is_success());
    let historic_task_list_body: Value = historic_task_list_response.json().await.unwrap();
    assert_eq!(historic_task_list_body["total"], 1);
    assert_eq!(historic_task_list_body["data"][0]["id"], task_id);

    let historic_task_get_response = client
        .get(format!(
            "{base_url}/history/historic-task-instances/{task_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(historic_task_get_response.status().is_success());
    let historic_task_get_body: Value = historic_task_get_response.json().await.unwrap();
    assert_eq!(historic_task_get_body["id"], task_id);
    assert_eq!(
        historic_task_get_body["processInstanceId"],
        process_instance_id
    );

    let historic_activity_response = client
        .post(format!("{base_url}/query/historic-activity-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "processInstanceId": process_instance_id
        }))
        .send()
        .await
        .unwrap();
    assert!(historic_activity_response.status().is_success());
    let historic_activity_body: Value = historic_activity_response.json().await.unwrap();
    assert!(
        historic_activity_body["data"]
            .as_array()
            .unwrap()
            .iter()
            .any(|activity| activity["activityId"] == "task1")
    );

    let historic_activity_list_response = client
        .get(format!(
            "{base_url}/history/historic-activity-instances?processInstanceId={process_instance_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(historic_activity_list_response.status().is_success());
    let historic_activity_list_body: Value = historic_activity_list_response.json().await.unwrap();
    assert!(
        historic_activity_list_body["data"]
            .as_array()
            .unwrap()
            .iter()
            .any(|activity| activity["activityId"] == "task1")
    );

    let historic_variable_response = client
        .post(format!("{base_url}/query/historic-variable-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "processInstanceId": process_instance_id
        }))
        .send()
        .await
        .unwrap();
    assert!(historic_variable_response.status().is_success());
    let historic_variable_body: Value = historic_variable_response.json().await.unwrap();
    assert_eq!(historic_variable_body["total"], 1);
    assert_eq!(historic_variable_body["data"][0]["name"], "approval");
    assert_eq!(historic_variable_body["data"][0]["value"], "accepted");
}
