use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::service::config::ProcessEngineConfiguration;
use flowable_rest::run_server;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::net::TcpListener;

async fn setup(name: &str) -> (String, Arc<ProcessEngine>, reqwest::Client) {
    let config = ProcessEngineConfiguration {
        enable_secure_scripting: true,
        supported_script_languages: vec!["javascript".to_string(), "groovy".to_string()],
        ..Default::default()
    };
    let engine = Arc::new(ProcessEngine::new_with_config(name.to_string(), config));

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
    let engine_clone = Arc::clone(&engine);
    tokio::spawn(async move {
        run_server(engine_clone, listener).await.unwrap();
    });

    (base_url, engine, reqwest::Client::new())
}

async fn deploy(client: &reqwest::Client, base_url: &str, name: &str, xml: &str) -> String {
    let response = client
        .post(format!("{base_url}/repository/deployments"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "name": name,
            "resourceName": format!("{name}.bpmn20.xml"),
            "resource": xml
        }))
        .send()
        .await
        .unwrap();

    assert!(
        response.status().is_success(),
        "deployment failed: {}",
        response.text().await.unwrap()
    );

    let definitions_response = client
        .get(format!("{base_url}/repository/process-definitions"))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(definitions_response.status().is_success());
    let body: Value = definitions_response.json().await.unwrap();
    let definitions = body["data"].as_array().unwrap();
    definitions
        .iter()
        .find(|d| d["key"].as_str().unwrap() == name)
        .or_else(|| definitions.last())
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string()
}

async fn start_and_get_variables(
    client: &reqwest::Client,
    base_url: &str,
    pd_id: &str,
    input_vars: Value,
) -> Vec<Value> {
    let response = client
        .post(format!("{base_url}/runtime/process-instances"))
        .basic_auth("admin", Some("test"))
        .json(&json!({
            "processDefinitionId": pd_id,
            "variables": input_vars
        }))
        .send()
        .await
        .unwrap();

    assert!(
        response.status().is_success(),
        "start failed: {}",
        response.text().await.unwrap()
    );
    let body: Value = response.json().await.unwrap();
    let pi_id = body["id"].as_str().unwrap();

    let vars_response = client
        .get(format!(
            "{base_url}/runtime/process-instances/{pi_id}/variables"
        ))
        .basic_auth("admin", Some("test"))
        .send()
        .await
        .unwrap();
    assert!(vars_response.status().is_success());
    let vars: Value = vars_response.json().await.unwrap();
    vars.as_array().unwrap().clone()
}

fn find_var<'a>(vars: &'a [Value], name: &str) -> &'a Value {
    vars.iter()
        .find(|v| v["name"].as_str().unwrap() == name)
        .unwrap_or_else(|| panic!("Variable '{}' not found in {:?}", name, vars))
}

fn script_task_bpmn(process_id: &str, script: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
    <process id="{process_id}" isExecutable="true">
        <startEvent id="start" />
        <sequenceFlow id="flow1" sourceRef="start" targetRef="scriptTask" />
        <scriptTask id="scriptTask" scriptFormat="javascript">
            <script><![CDATA[{script}]]></script>
        </scriptTask>
        <sequenceFlow id="flow2" sourceRef="scriptTask" targetRef="waitTask" />
        <userTask id="waitTask" />
        <sequenceFlow id="flow3" sourceRef="waitTask" targetRef="end" />
        <endEvent id="end" />
    </process>
</definitions>"#
    )
}

#[tokio::test]
async fn script_division_and_modulo_operators() {
    let (base_url, _engine, client) = setup("script-div-mod").await;
    let xml = script_task_bpmn(
        "divModProcess",
        r#"
        var total = 100;
        var count = 3;
        var average = total / count;
        var remainder = total % count;
        "#,
    );

    let pd_id = deploy(&client, &base_url, "divModProcess", &xml).await;
    let vars = start_and_get_variables(&client, &base_url, &pd_id, json!([])).await;

    let average = find_var(&vars, "average");
    let avg_val = average["value"].as_f64().unwrap();
    assert!((avg_val - 33.333333).abs() < 0.01, "average = {}", avg_val);

    let remainder = find_var(&vars, "remainder");
    assert_eq!(remainder["value"], json!(1));
}

#[tokio::test]
async fn script_comparison_and_logical_operators() {
    let (base_url, _engine, client) = setup("script-cmp-logic").await;
    let xml = script_task_bpmn(
        "cmpLogicProcess",
        r#"
        var a = 10;
        var b = 20;
        var isGreater = a > b;
        var isEqual = a == a;
        var combined = a < b && b > 15;
        var negated = !(a > b);
        "#,
    );

    let pd_id = deploy(&client, &base_url, "cmpLogicProcess", &xml).await;
    let vars = start_and_get_variables(&client, &base_url, &pd_id, json!([])).await;

    assert_eq!(find_var(&vars, "isGreater")["value"], json!(false));
    assert_eq!(find_var(&vars, "isEqual")["value"], json!(true));
    assert_eq!(find_var(&vars, "combined")["value"], json!(true));
    assert_eq!(find_var(&vars, "negated")["value"], json!(true));
}

#[tokio::test]
async fn script_if_else_control_flow() {
    let (base_url, _engine, client) = setup("script-if-else").await;
    let xml = script_task_bpmn(
        "ifElseProcess",
        r#"
        var age = 25;
        var status = "unknown";
        if (age >= 18) {
            status = "adult";
        } else {
            status = "minor";
        }
        "#,
    );

    let pd_id = deploy(&client, &base_url, "ifElseProcess", &xml).await;
    let vars = start_and_get_variables(&client, &base_url, &pd_id, json!([])).await;

    assert_eq!(find_var(&vars, "status")["value"], json!("adult"));
}

#[tokio::test]
async fn script_for_loop() {
    let (base_url, _engine, client) = setup("script-for-loop").await;
    let xml = script_task_bpmn(
        "forLoopProcess",
        r#"
        var sum = 0;
        for (var i = 1; i <= 10; i += 1) {
            sum += i;
        }
        "#,
    );

    let pd_id = deploy(&client, &base_url, "forLoopProcess", &xml).await;
    let vars = start_and_get_variables(&client, &base_url, &pd_id, json!([])).await;

    assert_eq!(find_var(&vars, "sum")["value"], json!(55));
}

#[tokio::test]
async fn script_while_loop() {
    let (base_url, _engine, client) = setup("script-while-loop").await;
    let xml = script_task_bpmn(
        "whileLoopProcess",
        r#"
        var n = 1;
        var count = 0;
        while (n < 100) {
            n = n * 2;
            count += 1;
        }
        "#,
    );

    let pd_id = deploy(&client, &base_url, "whileLoopProcess", &xml).await;
    let vars = start_and_get_variables(&client, &base_url, &pd_id, json!([])).await;

    // 1->2->4->8->16->32->64->128: 7 iterations, n=128
    assert_eq!(find_var(&vars, "n")["value"], json!(128));
    assert_eq!(find_var(&vars, "count")["value"], json!(7));
}

#[tokio::test]
async fn script_function_definition_and_call() {
    let (base_url, _engine, client) = setup("script-function").await;
    let xml = script_task_bpmn(
        "functionProcess",
        r#"
        function calculateTax(amount, rate) {
            return amount * rate;
        }
        var total = 1000;
        var tax = calculateTax(total, 0.1);
        var net = total + tax;
        "#,
    );

    let pd_id = deploy(&client, &base_url, "functionProcess", &xml).await;
    let vars = start_and_get_variables(&client, &base_url, &pd_id, json!([])).await;

    assert_eq!(find_var(&vars, "tax")["value"], json!(100));
    assert_eq!(find_var(&vars, "net")["value"], json!(1100));
}

#[tokio::test]
async fn script_object_and_array_access() {
    let (base_url, _engine, client) = setup("script-obj-arr").await;
    let xml = script_task_bpmn(
        "objArrProcess",
        r#"
        var arr = [10, 20, 30];
        var first = arr[0];
        var arrLen = arr.length;
        var obj = {"name": "flowable", "version": 7};
        var appName = obj.name;
        var ver = obj["version"];
        "#,
    );

    let pd_id = deploy(&client, &base_url, "objArrProcess", &xml).await;
    let vars = start_and_get_variables(&client, &base_url, &pd_id, json!([])).await;

    assert_eq!(find_var(&vars, "first")["value"], json!(10));
    assert_eq!(find_var(&vars, "arrLen")["value"], json!(3));
    assert_eq!(find_var(&vars, "appName")["value"], json!("flowable"));
    assert_eq!(find_var(&vars, "ver")["value"], json!(7));
}

#[tokio::test]
async fn script_math_stdlib_functions() {
    let (base_url, _engine, client) = setup("script-math").await;
    let xml = script_task_bpmn(
        "mathProcess",
        r#"
        var floored = Math.floor(3.7);
        var ceiled = Math.ceil(3.2);
        var rounded = Math.round(3.5);
        var absolute = Math.abs(-42);
        var powered = Math.pow(2, 10);
        "#,
    );

    let pd_id = deploy(&client, &base_url, "mathProcess", &xml).await;
    let vars = start_and_get_variables(&client, &base_url, &pd_id, json!([])).await;

    assert_eq!(find_var(&vars, "floored")["value"], json!(3));
    assert_eq!(find_var(&vars, "ceiled")["value"], json!(4));
    assert_eq!(find_var(&vars, "rounded")["value"], json!(4));
    assert_eq!(find_var(&vars, "absolute")["value"], json!(42));
    assert_eq!(find_var(&vars, "powered")["value"], json!(1024));
}

#[tokio::test]
async fn script_string_methods() {
    let (base_url, _engine, client) = setup("script-string").await;
    let xml = script_task_bpmn(
        "stringProcess",
        r#"
        var greeting = "Hello, World!";
        var upper = greeting.toUpperCase();
        var lower = greeting.toLowerCase();
        var trimmed = "  spaces  ".trim();
        var idx = greeting.indexOf("World");
        var len = greeting.length;
        "#,
    );

    let pd_id = deploy(&client, &base_url, "stringProcess", &xml).await;
    let vars = start_and_get_variables(&client, &base_url, &pd_id, json!([])).await;

    assert_eq!(find_var(&vars, "upper")["value"], json!("HELLO, WORLD!"));
    assert_eq!(find_var(&vars, "lower")["value"], json!("hello, world!"));
    assert_eq!(find_var(&vars, "trimmed")["value"], json!("spaces"));
    assert_eq!(find_var(&vars, "idx")["value"], json!(7));
    assert_eq!(find_var(&vars, "len")["value"], json!(13));
}

#[tokio::test]
async fn script_reads_process_variables() {
    let (base_url, _engine, client) = setup("script-read-vars").await;
    let xml = script_task_bpmn(
        "readVarsProcess",
        r#"
        var result = price * quantity;
        var discount = result * 0.1;
        var total = result - discount;
        "#,
    );

    let pd_id = deploy(&client, &base_url, "readVarsProcess", &xml).await;
    let vars = start_and_get_variables(
        &client,
        &base_url,
        &pd_id,
        json!([
            {"name": "price", "value": 50},
            {"name": "quantity", "value": 3}
        ]),
    )
    .await;

    assert_eq!(find_var(&vars, "result")["value"], json!(150));
    assert_eq!(find_var(&vars, "total")["value"], json!(135));
}
