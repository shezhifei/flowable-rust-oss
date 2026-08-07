//! Java parity for REST variable type conversion and response inference.
//!
//! References:
//! - `LongRestVariableConverter.java:24-47` distinguishes Java `Long` from
//!   `Integer` and converts REST numbers through `Number.longValue()`.
//! - `IntegerRestVariableConverter.java:24-47` converts REST numbers through
//!   `Number.intValue()`.
//! - `RestResponseFactory.java:381-399` rejects an explicitly named variable
//!   type when no converter is registered.

use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_rest::run_server;
use reqwest::StatusCode;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::net::TcpListener;

const PROCESS_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
  <process id="p36VariableTypes" isExecutable="true">
    <startEvent id="start" />
    <sequenceFlow id="toTask" sourceRef="start" targetRef="review" />
    <userTask id="review" name="Review" />
    <sequenceFlow id="toEnd" sourceRef="review" targetRef="end" />
    <endEvent id="end" />
  </process>
</definitions>"#;

struct Fixture {
    client: reqwest::Client,
    base_url: String,
    process_definition_id: String,
}

impl Fixture {
    async fn new(name: &str) -> Self {
        let engine = Arc::new(ProcessEngine::new(name.to_string()));
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
        engine
            .get_repository_service()
            .deploy(
                engine
                    .get_repository_service()
                    .create_deployment()
                    .name(name.to_string())
                    .add_string("p36.bpmn20.xml".to_string(), PROCESS_XML.to_string()),
            )
            .unwrap();
        let process_definition_id = engine
            .get_repository_service()
            .get_process_definition_ids()
            .unwrap()[0]
            .clone();

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let server_engine = Arc::clone(&engine);
        tokio::spawn(async move {
            run_server(server_engine, listener).await.unwrap();
        });

        Self {
            client: reqwest::Client::new(),
            base_url,
            process_definition_id,
        }
    }

    async fn start(&self, variables: Value) -> (String, String, String) {
        let response = self
            .client
            .post(format!("{}/runtime/process-instances", self.base_url))
            .basic_auth("admin", Some("test"))
            .json(&json!({
                "processDefinitionId": self.process_definition_id,
                "variables": variables
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let process: Value = response.json().await.unwrap();
        let process_instance_id = process["id"].as_str().unwrap().to_string();

        let tasks = self
            .client
            .get(format!(
                "{}/runtime/tasks?processInstanceId={process_instance_id}",
                self.base_url
            ))
            .basic_auth("admin", Some("test"))
            .send()
            .await
            .unwrap();
        assert_eq!(tasks.status(), StatusCode::OK);
        let tasks: Value = tasks.json().await.unwrap();
        (
            process_instance_id,
            tasks["data"][0]["executionId"]
                .as_str()
                .unwrap()
                .to_string(),
            tasks["data"][0]["id"].as_str().unwrap().to_string(),
        )
    }

    async fn get_json(&self, path: &str) -> Value {
        let response = self
            .client
            .get(format!("{}{}", self.base_url, path))
            .basic_auth("admin", Some("test"))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK, "GET {path} failed");
        response.json().await.unwrap()
    }
}

fn variable_type<'a>(variables: &'a Value, name: &str, type_field: &str) -> &'a str {
    variables
        .as_array()
        .unwrap()
        .iter()
        .find(|variable| variable["name"] == name)
        .unwrap_or_else(|| panic!("variable {name} missing from {variables}"))[type_field]
        .as_str()
        .unwrap()
}

#[tokio::test]
async fn get_infers_integer_or_long_across_runtime_task_query_and_history() {
    let fixture = Fixture::new("p36-get-integer-long").await;
    let (process_instance_id, execution_id, task_id) = fixture
        .start(json!([
            { "name": "minInteger", "value": -2147483648_i64 },
            { "name": "maxInteger", "value": 2147483647_i64 },
            { "name": "negativeLong", "value": -2147483649_i64 },
            { "name": "positiveLong", "value": 2147483648_i64 }
        ]))
        .await;

    let task_write = fixture
        .client
        .post(format!(
            "{}/runtime/tasks/{task_id}/variables",
            fixture.base_url
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!([
            { "name": "taskInteger", "value": 7 },
            { "name": "taskLong", "value": 2147483648_i64 }
        ]))
        .send()
        .await
        .unwrap();
    assert_eq!(task_write.status(), StatusCode::CREATED);

    let process_variables = fixture
        .get_json(&format!(
            "/runtime/process-instances/{process_instance_id}/variables"
        ))
        .await;
    assert_eq!(
        variable_type(&process_variables, "minInteger", "type"),
        "integer"
    );
    assert_eq!(
        variable_type(&process_variables, "maxInteger", "type"),
        "integer"
    );
    assert_eq!(
        variable_type(&process_variables, "negativeLong", "type"),
        "long"
    );
    assert_eq!(
        variable_type(&process_variables, "positiveLong", "type"),
        "long"
    );

    let execution_variables = fixture
        .get_json(&format!("/runtime/executions/{execution_id}/variables"))
        .await;
    assert_eq!(
        variable_type(&execution_variables, "positiveLong", "type"),
        "long"
    );

    let task_variables = fixture
        .get_json(&format!("/runtime/tasks/{task_id}/variables"))
        .await;
    assert_eq!(
        variable_type(&task_variables, "taskInteger", "type"),
        "integer"
    );
    assert_eq!(variable_type(&task_variables, "taskLong", "type"), "long");

    let runtime_instances = fixture
        .get_json(&format!(
            "/runtime/variable-instances?processInstanceId={process_instance_id}&variableType=long"
        ))
        .await;
    assert_eq!(
        runtime_instances["total"], 3,
        "body was {runtime_instances}"
    );
    assert!(
        runtime_instances["data"]
            .as_array()
            .unwrap()
            .iter()
            .all(|variable| variable["type"] == "long")
    );

    let process_query = fixture
        .client
        .post(format!(
            "{}/query/process-instances?includeProcessVariables=true",
            fixture.base_url
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "processInstanceId": process_instance_id }))
        .send()
        .await
        .unwrap();
    assert_eq!(process_query.status(), StatusCode::OK);
    let process_query: Value = process_query.json().await.unwrap();
    assert_eq!(
        variable_type(
            &process_query["data"][0]["variables"],
            "positiveLong",
            "type"
        ),
        "long"
    );

    let historic_instances = fixture
        .get_json(&format!(
            "/history/historic-variable-instances?processInstanceId={process_instance_id}&variableType=long"
        ))
        .await;
    assert_eq!(
        historic_instances["total"], 3,
        "body was {historic_instances}"
    );
    assert!(
        historic_instances["data"]
            .as_array()
            .unwrap()
            .iter()
            .all(|variable| variable["variableType"] == "long")
    );
}

#[tokio::test]
async fn explicit_types_convert_values_and_validate_converter_input() {
    let fixture = Fixture::new("p36-explicit-conversion").await;
    let (process_instance_id, _, task_id) = fixture
        .start(json!([
            { "name": "integerValue", "type": "integer", "value": 42.9 },
            { "name": "longValue", "type": "long", "value": 2147483648.9_f64 },
            { "name": "doubleValue", "type": "double", "value": 7 },
            { "name": "booleanValue", "type": "boolean", "value": true },
            { "name": "stringValue", "type": "string", "value": "value" },
            { "name": "jsonValue", "type": "json", "value": { "nested": true } },
            { "name": "nullValue", "type": "integer", "value": null }
        ]))
        .await;
    let variables = fixture
        .get_json(&format!(
            "/runtime/process-instances/{process_instance_id}/variables"
        ))
        .await;
    let value = |name: &str| {
        variables
            .as_array()
            .unwrap()
            .iter()
            .find(|variable| variable["name"] == name)
            .unwrap()["value"]
            .clone()
    };
    assert_eq!(value("integerValue"), json!(42));
    assert_eq!(value("longValue"), json!(2147483648_i64));
    assert_eq!(value("doubleValue"), json!(7.0));
    assert_eq!(value("booleanValue"), json!(true));
    assert_eq!(value("stringValue"), json!("value"));
    assert_eq!(value("jsonValue"), json!({ "nested": true }));
    assert_eq!(value("nullValue"), Value::Null);

    let invalid_integer = fixture
        .client
        .put(format!(
            "{}/runtime/tasks/{task_id}/variables/missing",
            fixture.base_url
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!({ "name": "missing", "type": "integer", "value": "42" }))
        .send()
        .await
        .unwrap();
    assert_eq!(invalid_integer.status(), StatusCode::BAD_REQUEST);
    assert!(
        invalid_integer
            .text()
            .await
            .unwrap()
            .contains("Converter can only convert integers")
    );
}

#[tokio::test]
async fn unknown_explicit_type_is_rejected_on_process_execution_and_task_writes() {
    let fixture = Fixture::new("p36-unknown-type").await;

    let invalid_start = fixture
        .client
        .post(format!("{}/runtime/process-instances", fixture.base_url))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "processDefinitionId": fixture.process_definition_id,
            "variables": [{ "name": "bad", "type": "mystery", "value": 1 }]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(invalid_start.status(), StatusCode::BAD_REQUEST);
    assert!(
        invalid_start
            .text()
            .await
            .unwrap()
            .contains("Variable 'bad' has unsupported type: 'mystery'.")
    );

    let (process_instance_id, execution_id, task_id) = fixture.start(json!([])).await;
    let invalid_execution = fixture
        .client
        .post(format!(
            "{}/runtime/executions/{execution_id}/variables",
            fixture.base_url
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!([{ "name": "bad", "type": "mystery", "value": 1 }]))
        .send()
        .await
        .unwrap();
    assert_eq!(invalid_execution.status(), StatusCode::BAD_REQUEST);

    let invalid_task_variable = fixture
        .client
        .post(format!(
            "{}/runtime/tasks/{task_id}/variables",
            fixture.base_url
        ))
        .basic_auth("admin", Some("test"))
        .json(&json!([{ "name": "bad", "type": "mystery", "value": 1 }]))
        .send()
        .await
        .unwrap();
    assert_eq!(invalid_task_variable.status(), StatusCode::BAD_REQUEST);

    let invalid_complete = fixture
        .client
        .post(format!("{}/runtime/tasks/{task_id}", fixture.base_url))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "action": "complete",
            "variables": [{ "name": "bad", "type": "mystery", "value": 1 }]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(invalid_complete.status(), StatusCode::BAD_REQUEST);

    let task_still_exists = fixture
        .client
        .get(format!(
            "{}/runtime/tasks?processInstanceId={process_instance_id}",
            fixture.base_url
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    let task_still_exists: Value = task_still_exists.json().await.unwrap();
    assert_eq!(task_still_exists["total"], 1);
}
