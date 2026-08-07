use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::service::config::{HistoryLevel, ProcessEngineConfiguration};
use flowable_rest::run_server;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::net::TcpListener;

const RUNTIME_PROCESS_BPMN: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
    <process id="travelRequestProcess" name="Travel Request Process" isExecutable="true">
        <startEvent id="startEvent" flowable:formKey="travelRequest" />
        <sequenceFlow id="flow1" sourceRef="startEvent" targetRef="approveTask" />
        <userTask id="approveTask" name="Approve request" flowable:formKey="expenseApproval" />
        <sequenceFlow id="flow2" sourceRef="approveTask" targetRef="endEvent" />
        <endEvent id="endEvent" />
    </process>
</definitions>"#;

const UNSUPPORTED_PROCESS_BPMN: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
    <process id="unsupportedRuntimeProcess" name="Unsupported Runtime Process" isExecutable="true">
        <startEvent id="startEvent" flowable:formKey="unsupportedRuntime" />
        <sequenceFlow id="flow1" sourceRef="startEvent" targetRef="endEvent" />
        <endEvent id="endEvent" />
    </process>
</definitions>"#;

async fn spawn_server() -> (Arc<ProcessEngine>, String, reqwest::Client) {
    // P112: Java 默认 history=audit(ProcessEngineConfiguration.java:88),而
    // historic-detail 的 variableUpdate/formProperty 明细是 FULL-only
    // (DefaultHistoryManager.java:347-348)。本文件的契约断言覆盖这些明细,
    // 故显式以 FULL 级别起引擎——与 P112 前(默认 Full)的可观测行为一致。
    let mut config = ProcessEngineConfiguration::default();
    config.history_level = HistoryLevel::Full;
    let engine = Arc::new(ProcessEngine::new_with_config(
        "rest-form-runtime".to_string(),
        config,
    ));
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

async fn deploy_runtime_forms(client: &reqwest::Client, base_url: &str) {
    let response = client
        .post(format!("{base_url}/form-repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": "Runtime forms",
            "resources": [
                {
                    "resourceName": "travel-request.form",
                    "resource": json!({
                        "key": "travelRequest",
                        "name": "Travel request",
                        "resourceName": "travel-request.form",
                        "outcomeVariableName": "startDecision",
                        "outcomes": [
                            { "id": "submit", "name": "Submit" },
                            { "id": "save", "name": "Save Draft" }
                        ],
                        "fields": [
                            { "id": "requester", "name": "Requester", "type": "string", "required": true },
                            { "id": "amount", "name": "Amount", "type": "number", "required": true }
                        ]
                    }).to_string()
                },
                {
                    "resourceName": "expense-approval.form",
                    "resource": json!({
                        "key": "expenseApproval",
                        "name": "Expense approval",
                        "resourceName": "expense-approval.form",
                        "outcomes": [
                            { "id": "approve", "name": "Approve" },
                            { "id": "reject", "name": "Reject" }
                        ],
                        "fields": [
                            { "id": "approved", "name": "Approved", "type": "boolean", "required": true },
                            { "id": "comment", "name": "Comment", "type": "string" }
                        ]
                    }).to_string()
                },
                {
                    "resourceName": "unsupported-runtime.form",
                    "resource": json!({
                        "key": "unsupportedRuntime",
                        "name": "Unsupported runtime",
                        "resourceName": "unsupported-runtime.form",
                        "fields": [
                            { "id": "attachment", "name": "Attachment", "type": "custom_widget", "required": true }
                        ]
                    }).to_string()
                }
            ]
        }))
        .send()
        .await
        .unwrap();

    assert!(response.status().is_success());
}

async fn deploy_process(
    client: &reqwest::Client,
    base_url: &str,
    deployment_name: &str,
    resource_name: &str,
    resource: &str,
) {
    let response = client
        .post(format!("{base_url}/repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": deployment_name,
            "resourceName": resource_name,
            "resource": resource
        }))
        .send()
        .await
        .unwrap();

    assert!(response.status().is_success());
}

#[tokio::test]
async fn runtime_form_routes_support_start_and_task_form_flow() {
    let (engine, base_url, client) = spawn_server().await;
    deploy_runtime_forms(&client, &base_url).await;
    deploy_process(
        &client,
        &base_url,
        "Travel Request Deployment",
        "travel-request-process.bpmn20.xml",
        RUNTIME_PROCESS_BPMN,
    )
    .await;

    let process_definition_id = engine
        .get_repository_service()
        .latest_process_definition_by_key("travelRequestProcess", None)
        .unwrap()
        .unwrap()
        .id;

    let start_form_response = client
        .get(format!(
            "{base_url}/form/form-data?processDefinitionId={process_definition_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(start_form_response.status(), reqwest::StatusCode::OK);
    let start_form_body: Value = start_form_response.json().await.unwrap();
    assert_eq!(start_form_body["formKey"], "travelRequest");
    assert_eq!(
        start_form_body["processDefinitionId"],
        process_definition_id
    );
    assert_eq!(start_form_body["formProperties"][0]["id"], "requester");

    let start_form_definition = client
        .get(format!(
            "{base_url}/repository/process-definitions/{process_definition_id}/start-form"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(start_form_definition.status(), reqwest::StatusCode::OK);
    let start_form_definition_body: Value = start_form_definition.json().await.unwrap();
    assert_eq!(start_form_definition_body["key"], "travelRequest");
    assert_eq!(start_form_definition_body["fields"][1]["type"], "number");

    let process_form_definitions = client
        .get(format!(
            "{base_url}/repository/process-definitions/{process_definition_id}/form-definitions"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(process_form_definitions.status(), reqwest::StatusCode::OK);
    let process_form_definitions_body: Value = process_form_definitions.json().await.unwrap();
    assert_eq!(process_form_definitions_body["total"], 2);
    let form_keys = process_form_definitions_body["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["key"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert!(form_keys.contains(&"travelRequest"));
    assert!(form_keys.contains(&"expenseApproval"));

    let start_submission = client
        .post(format!("{base_url}/form/form-data"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "processDefinitionId": process_definition_id,
            "businessKey": "travel-100",
            "outcome": "submit",
            "properties": [
                { "id": "requester", "value": "alice" },
                { "id": "amount", "value": "42.5" }
            ]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(start_submission.status(), reqwest::StatusCode::OK);
    let start_submission_body: Value = start_submission.json().await.unwrap();
    let process_instance_id = start_submission_body["id"].as_str().unwrap().to_string();
    assert_eq!(
        start_submission_body["processDefinitionId"],
        process_definition_id
    );
    assert_eq!(start_submission_body["businessKey"], "travel-100");

    let start_variables = client
        .get(format!(
            "{base_url}/runtime/process-instances/{process_instance_id}/variables"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(start_variables.status(), reqwest::StatusCode::OK);
    let start_variables_body: Value = start_variables.json().await.unwrap();
    assert!(
        start_variables_body
            .as_array()
            .unwrap()
            .iter()
            .any(|variable| {
                variable["name"] == "startDecision" && variable["value"] == "submit"
            })
    );

    let task_list = client
        .get(format!(
            "{base_url}/runtime/tasks?processInstanceId={process_instance_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(task_list.status(), reqwest::StatusCode::OK);
    let task_list_body: Value = task_list.json().await.unwrap();
    assert_eq!(task_list_body["total"], 1);
    let task_id = task_list_body["data"][0]["id"]
        .as_str()
        .unwrap()
        .to_string();

    let task_form_response = client
        .get(format!("{base_url}/form/form-data?taskId={task_id}"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(task_form_response.status(), reqwest::StatusCode::OK);
    let task_form_body: Value = task_form_response.json().await.unwrap();
    assert_eq!(task_form_body["formKey"], "expenseApproval");
    assert_eq!(task_form_body["taskId"], task_id);
    assert_eq!(task_form_body["formProperties"][0]["id"], "approved");

    let task_form_definition = client
        .get(format!("{base_url}/runtime/tasks/{task_id}/form"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(task_form_definition.status(), reqwest::StatusCode::OK);
    let task_form_definition_body: Value = task_form_definition.json().await.unwrap();
    assert_eq!(task_form_definition_body["key"], "expenseApproval");
    assert_eq!(task_form_definition_body["fields"][0]["type"], "boolean");

    let task_submission = client
        .post(format!("{base_url}/form/form-data"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "taskId": task_id,
            "outcome": "approve",
            "properties": [
                { "id": "approved", "value": "true" },
                { "id": "comment", "value": "looks good" }
            ]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(task_submission.status(), reqwest::StatusCode::NO_CONTENT);

    let task_variables = client
        .get(format!(
            "{base_url}/history/historic-variable-instances?processInstanceId={process_instance_id}&variableName=form_expenseApproval_outcome"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(task_variables.status(), reqwest::StatusCode::OK);
    let task_variables_body: Value = task_variables.json().await.unwrap();
    assert!(
        task_variables_body["data"]
            .as_array()
            .unwrap()
            .iter()
            .any(|variable| {
                variable["name"] == "form_expenseApproval_outcome" && variable["value"] == "approve"
            })
    );

    let remaining_tasks = client
        .get(format!(
            "{base_url}/runtime/tasks?processInstanceId={process_instance_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(remaining_tasks.status(), reqwest::StatusCode::OK);
    let remaining_tasks_body: Value = remaining_tasks.json().await.unwrap();
    assert_eq!(remaining_tasks_body["total"], 0);

    let task_log_entries = client
        .get(format!(
            "{base_url}/history/historic-task-log-entries?taskId={task_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(task_log_entries.status(), reqwest::StatusCode::OK);
    let task_log_entries_body: Value = task_log_entries.json().await.unwrap();
    assert_eq!(task_log_entries_body["total"], 2);
    let task_log_types = task_log_entries_body["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["type"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert!(task_log_types.contains(&"USER_TASK_CREATED"));
    assert!(task_log_types.contains(&"USER_TASK_COMPLETED"));
    assert!(
        task_log_entries_body["data"]
            .as_array()
            .unwrap()
            .iter()
            .all(|entry| entry["taskId"] == task_id
                && entry["processInstanceId"] == process_instance_id)
    );

    let historic_task_form = client
        .get(format!(
            "{base_url}/history/historic-task-instances/{task_id}/form"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(historic_task_form.status(), reqwest::StatusCode::OK);
    let historic_task_form_body: Value = historic_task_form.json().await.unwrap();
    assert_eq!(historic_task_form_body["taskId"], task_id);
    assert_eq!(
        historic_task_form_body["processInstanceId"],
        process_instance_id
    );
    assert_eq!(
        historic_task_form_body["formDefinitionKey"],
        "expenseApproval"
    );
    assert_eq!(historic_task_form_body["values"]["approved"], true);
    assert_eq!(historic_task_form_body["values"]["comment"], "looks good");

    let historic_details = client
        .get(format!(
            "{base_url}/history/historic-detail?processInstanceId={process_instance_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(historic_details.status(), reqwest::StatusCode::OK);
    let historic_details_body: Value = historic_details.json().await.unwrap();
    let details = historic_details_body["data"].as_array().unwrap();
    let approved_form_detail = details
        .iter()
        .find(|detail| detail["detailType"] == "formProperty" && detail["propertyId"] == "approved")
        .expect("approved form property detail should be present");
    assert_eq!(approved_form_detail["propertyValue"], true);
    assert_eq!(approved_form_detail["taskId"], task_id);
    let approved_variable_detail = details
        .iter()
        .find(|detail| {
            detail["detailType"] == "variableUpdate" && detail["variable"]["name"] == "approved"
        })
        .expect("approved variable update detail should be present");
    assert_eq!(approved_variable_detail["variable"]["value"], true);
    let approved_variable_detail_id = approved_variable_detail["id"].as_str().unwrap();

    let historic_detail_data = client
        .get(format!(
            "{base_url}/history/historic-detail/{approved_variable_detail_id}/data"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(
        historic_detail_data.status(),
        reqwest::StatusCode::NOT_FOUND
    );

    let queried_historic_details = client
        .post(format!("{base_url}/query/historic-detail"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "processInstanceId": process_instance_id,
            "taskId": task_id,
            "selectOnlyFormProperties": true
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(queried_historic_details.status(), reqwest::StatusCode::OK);
    let queried_historic_details_body: Value = queried_historic_details.json().await.unwrap();
    assert_eq!(queried_historic_details_body["total"], 3);
    assert!(
        queried_historic_details_body["data"]
            .as_array()
            .unwrap()
            .iter()
            .all(|detail| detail["detailType"] == "formProperty" && detail["taskId"] == task_id)
    );
    assert!(
        queried_historic_details_body["data"]
            .as_array()
            .unwrap()
            .iter()
            .any(|detail| {
                detail["propertyId"] == "form_expenseApproval_outcome"
                    && detail["propertyValue"] == "approve"
            })
    );

    let form_instances = client
        .get(format!(
            "{base_url}/form/form-instances?processInstanceId={process_instance_id}&sort=submittedDate&order=desc"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(form_instances.status(), reqwest::StatusCode::OK);
    let form_instances_body: Value = form_instances.json().await.unwrap();
    assert_eq!(form_instances_body["total"], 2);
    assert_eq!(form_instances_body["data"][0]["taskId"], task_id);
    assert_eq!(
        form_instances_body["data"][0]["formDefinitionKey"],
        "expenseApproval"
    );
    assert_eq!(
        form_instances_body["data"][0]["processInstanceId"],
        process_instance_id
    );
    assert!(
        form_instances_body["data"][0]["url"]
            .as_str()
            .unwrap()
            .contains("/form/form-instances/")
    );
    assert!(
        form_instances_body["data"][0]["submittedDate"]
            .as_str()
            .unwrap()
            .contains('T')
    );
    assert!(form_instances_body["data"][0]["tenantId"].is_null());
    assert_eq!(form_instances_body["data"][0]["submittedBy"], "admin");
    assert_eq!(
        form_instances_body["data"][1]["formDefinitionKey"],
        "travelRequest"
    );
    assert_eq!(form_instances_body["data"][1]["submittedBy"], "admin");

    let submitted_date = form_instances_body["data"][0]["submittedDate"]
        .as_str()
        .unwrap()
        .to_string();
    let date_filtered_instances = client
        .get(format!("{base_url}/form/form-instances"))
        .query(&[
            ("submittedDate", submitted_date.as_str()),
            ("submittedDateBefore", "2999-01-01T00:00:00.000Z"),
            ("submittedDateAfter", "1970-01-01T00:00:00.000Z"),
        ])
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(date_filtered_instances.status(), reqwest::StatusCode::OK);
    let date_filtered_body: Value = date_filtered_instances.json().await.unwrap();
    assert_eq!(date_filtered_body["total"], 1);
    assert_eq!(date_filtered_body["data"][0]["taskId"], task_id);

    let submitted_by_like_instances = client
        .get(format!("{base_url}/form/form-instances"))
        .query(&[
            ("submittedByLike", "adm%"),
            ("sort", "submittedBy"),
            ("order", "desc"),
        ])
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(
        submitted_by_like_instances.status(),
        reqwest::StatusCode::OK
    );
    let submitted_by_like_body: Value = submitted_by_like_instances.json().await.unwrap();
    assert_eq!(submitted_by_like_body["total"], 2);
    assert!(
        submitted_by_like_body["data"]
            .as_array()
            .unwrap()
            .iter()
            .all(|instance| instance["submittedBy"] == "admin")
    );

    let task_form_instance_id = form_instances_body["data"][0]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let form_instance = client
        .get(format!(
            "{base_url}/form/form-instances/{task_form_instance_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(form_instance.status(), reqwest::StatusCode::OK);
    let form_instance_body: Value = form_instance.json().await.unwrap();
    assert_eq!(form_instance_body["id"], task_form_instance_id);
    assert_eq!(form_instance_body["taskId"], task_id);
    assert_eq!(form_instance_body["formDefinitionKey"], "expenseApproval");
    assert_eq!(form_instance_body["submittedBy"], "admin");

    let filtered_form_instances = client
        .get(format!(
            "{base_url}/form/form-instances?formDefinitionKey=travelRequest"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(filtered_form_instances.status(), reqwest::StatusCode::OK);
    let filtered_form_instances_body: Value = filtered_form_instances.json().await.unwrap();
    assert_eq!(filtered_form_instances_body["total"], 1);
    assert_eq!(
        filtered_form_instances_body["data"][0]["formDefinitionKey"],
        "travelRequest"
    );

    let submitted_by_instances = client
        .get(format!(
            "{base_url}/form/form-instances?submittedBy=admin&processInstanceId={process_instance_id}"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(submitted_by_instances.status(), reqwest::StatusCode::OK);
    let submitted_by_instances_body: Value = submitted_by_instances.json().await.unwrap();
    assert_eq!(submitted_by_instances_body["total"], 2);
    assert!(
        submitted_by_instances_body["data"]
            .as_array()
            .unwrap()
            .iter()
            .all(|instance| instance["submittedBy"] == "admin")
    );
}

#[tokio::test]
async fn runtime_form_instance_queries_support_tenant_filters_and_sort() {
    let (engine, base_url, client) = spawn_server().await;
    deploy_runtime_forms(&client, &base_url).await;
    engine
        .get_repository_service()
        .deploy(
            engine
                .get_repository_service()
                .create_deployment()
                .name("Tenant Travel Request Deployment".to_string())
                .tenant_id("tenant-a".to_string())
                .add_string(
                    "tenant-travel-request-process.bpmn20.xml".to_string(),
                    RUNTIME_PROCESS_BPMN.to_string(),
                ),
        )
        .unwrap();

    let tenant_process_definition_id = engine
        .get_repository_service()
        .latest_process_definition_by_key("travelRequestProcess", Some("tenant-a"))
        .unwrap()
        .unwrap()
        .id;

    let start_submission = client
        .post(format!("{base_url}/form/form-data"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "processDefinitionId": tenant_process_definition_id,
            "businessKey": "tenant-travel-100",
            "properties": [
                { "id": "requester", "value": "tenant-user" },
                { "id": "amount", "value": 125 }
            ]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(start_submission.status(), reqwest::StatusCode::OK);

    let tenant_filtered = client
        .get(format!("{base_url}/form/form-instances?tenantId=tenant-a"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(tenant_filtered.status(), reqwest::StatusCode::OK);
    let tenant_filtered_body: Value = tenant_filtered.json().await.unwrap();
    assert_eq!(tenant_filtered_body["total"], 1);
    assert_eq!(tenant_filtered_body["data"][0]["tenantId"], "tenant-a");
    assert_eq!(
        tenant_filtered_body["data"][0]["formDefinitionKey"],
        "travelRequest"
    );

    let tenant_like = client
        .get(format!(
            "{base_url}/form/form-instances?tenantIdLike=tenant-%25&sort=tenantId&order=desc"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(tenant_like.status(), reqwest::StatusCode::OK);
    let tenant_like_body: Value = tenant_like.json().await.unwrap();
    assert_eq!(tenant_like_body["total"], 1);
    assert_eq!(tenant_like_body["data"][0]["tenantId"], "tenant-a");

    let without_tenant = client
        .get(format!(
            "{base_url}/form/form-instances?withoutTenantId=true"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(without_tenant.status(), reqwest::StatusCode::OK);
    let without_tenant_body: Value = without_tenant.json().await.unwrap();
    assert_eq!(without_tenant_body["total"], 0);

    let conflicting_tenant_filters = client
        .get(format!(
            "{base_url}/form/form-instances?tenantIdLike=tenant-%25&withoutTenantId=true"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(
        conflicting_tenant_filters.status(),
        reqwest::StatusCode::BAD_REQUEST
    );
    let conflicting_tenant_filters_body: Value = conflicting_tenant_filters.json().await.unwrap();
    assert_eq!(conflicting_tenant_filters_body["code"], "BAD_REQUEST");
    assert!(
        conflicting_tenant_filters_body["details"]
            .as_str()
            .unwrap()
            .contains("withoutTenantId")
    );
}

#[tokio::test]
async fn runtime_form_routes_return_structured_errors() {
    let (_engine, base_url, client) = spawn_server().await;
    deploy_runtime_forms(&client, &base_url).await;
    deploy_process(
        &client,
        &base_url,
        "Unsupported Runtime Deployment",
        "unsupported-runtime-process.bpmn20.xml",
        UNSUPPORTED_PROCESS_BPMN,
    )
    .await;

    let unauthorized = client
        .get(format!("{base_url}/form/form-data"))
        .send()
        .await
        .unwrap();
    assert_eq!(unauthorized.status(), reqwest::StatusCode::UNAUTHORIZED);
    let unauthorized_body: Value = unauthorized.json().await.unwrap();
    assert_eq!(unauthorized_body["code"], "UNAUTHORIZED");

    let bad_request = client
        .get(format!("{base_url}/form/form-data"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert_eq!(bad_request.status(), reqwest::StatusCode::BAD_REQUEST);
    let bad_request_body: Value = bad_request.json().await.unwrap();
    assert_eq!(bad_request_body["code"], "BAD_REQUEST");
    assert!(
        bad_request_body["details"]
            .as_str()
            .unwrap()
            .contains("processDefinitionId")
    );

    let unsupported_process_definition_id = client
        .get(format!(
            "{base_url}/repository/process-definitions?key=unsupportedRuntimeProcess"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    let unsupported_process_definition_body: Value =
        unsupported_process_definition_id.json().await.unwrap();
    let process_definition_id = unsupported_process_definition_body["data"][0]["id"]
        .as_str()
        .unwrap();

    let unsupported = client
        .post(format!("{base_url}/form/form-data"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "processDefinitionId": process_definition_id,
            "properties": [
                { "id": "attachment", "value": "payload" }
            ]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(unsupported.status(), reqwest::StatusCode::BAD_REQUEST);
    let unsupported_body: Value = unsupported.json().await.unwrap();
    assert_eq!(unsupported_body["code"], "BAD_REQUEST");
    assert!(
        unsupported_body["details"]
            .as_str()
            .unwrap()
            .contains("Unsupported")
    );
}
