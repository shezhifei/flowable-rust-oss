use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::service::config::{
    EngineDatabaseKind, HttpServiceRuntimeMode, ProcessEngineConfiguration,
    RealHttpClientConfiguration,
};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::Arc;
use std::thread;

#[test]
fn http_service_task_executes_owned_runtime_and_stores_result_variable() {
    let process_engine = ProcessEngine::new("default".to_string());
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();
    let task_service = process_engine.get_task_service();

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
                 xmlns:flowable="http://flowable.org/bpmn"
                 targetNamespace="Examples">
        <process id="httpRuntimeProcess" name="HTTP Runtime Process">
            <startEvent id="startEvent1" />
            <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="httpTask1" />
            <serviceTask id="httpTask1"
                         name="Invoke HTTP"
                         flowable:type="http"
                         flowable:resultVariableName="httpResult">
                <extensionElements>
                    <flowable:requestMethod>POST</flowable:requestMethod>
                    <flowable:requestUrl>https://example.flowable.local/orders</flowable:requestUrl>
                    <flowable:requestHeaders>{"Accept":"application/json"}</flowable:requestHeaders>
                    <flowable:requestBody>{"orderId":42,"approved":true}</flowable:requestBody>
                    <flowable:basicAuthenticationUsername>api-user</flowable:basicAuthenticationUsername>
                    <flowable:basicAuthenticationPassword>api-password</flowable:basicAuthenticationPassword>
                    <flowable:bodyEncoding>form</flowable:bodyEncoding>
                    <flowable:requestTimeout>5000</flowable:requestTimeout>
                    <flowable:connectTimeout>1000</flowable:connectTimeout>
                    <flowable:followRedirects>false</flowable:followRedirects>
                </extensionElements>
            </serviceTask>
            <sequenceFlow id="flow2" sourceRef="httpTask1" targetRef="userTask1" />
            <userTask id="userTask1" name="Review HTTP Result" />
        </process>
    </definitions>"#;

    let builder = repository_service
        .create_deployment()
        .name("HTTP Runtime Deployment".to_string())
        .add_string("httpRuntimeProcess.bpmn20.xml".to_string(), xml.to_string());

    repository_service.deploy(builder).unwrap();
    let process_definition_id = repository_service.get_process_definition_ids().unwrap()[0].clone();

    let process_instance = runtime_service
        .start_process_instance(
            runtime_service
                .create_process_instance_builder()
                .process_definition_id(process_definition_id)
                .name("HTTP Runtime Instance".to_string()),
        )
        .unwrap();

    let tasks = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(
        tasks.len(),
        1,
        "HTTP task should continue into the user task"
    );
    assert_eq!(tasks[0].name, "Review HTTP Result");

    let http_result = runtime_service
        .get_variable(process_instance.id.clone(), "httpResult".to_string())
        .unwrap()
        .expect("httpResult should be stored on the execution");

    assert_eq!(http_result["service"], "http");
    assert_eq!(http_result["request"]["method"], "POST");
    assert_eq!(http_result["request"]["timeoutMs"], 5000);
    assert_eq!(http_result["request"]["connectTimeoutMs"], 1000);
    assert_eq!(http_result["request"]["followRedirects"], false);
    assert_eq!(http_result["request"]["bodyEncoding"], "form");
    assert_eq!(http_result["request"]["hasBasicAuth"], true);
    assert_eq!(http_result["request"]["basicAuth"]["hasBasicAuth"], true);
    assert_eq!(http_result["request"]["basicAuth"]["username"], "api-user");
    assert!(
        !http_result.to_string().contains("api-password"),
        "result variable must not leak the basic auth password"
    );
    assert_eq!(
        http_result["request"]["url"],
        "https://example.flowable.local/orders"
    );
    assert_eq!(http_result["response"]["statusCode"], 200);
    assert_eq!(http_result["response"]["body"]["echo"]["orderId"], 42);
    assert_eq!(http_result["response"]["body"]["accepted"], true);
}

#[test]
fn http_service_task_can_use_real_http_runtime_against_local_echo_server() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("echo server should bind");
    let address = listener
        .local_addr()
        .expect("echo server should expose local address");

    let server = thread::spawn(move || {
        let (mut stream, _) = listener
            .accept()
            .expect("echo server should accept connection");
        let mut buffer = [0_u8; 8192];
        let bytes_read = stream
            .read(&mut buffer)
            .expect("echo server should read request");
        let request = String::from_utf8_lossy(&buffer[..bytes_read]).to_string();
        let parts: Vec<&str> = request.split("\r\n\r\n").collect();
        let request_body = parts.get(1).copied().unwrap_or("").trim().to_string();

        let response_body = format!(
            "{{\"transport\":\"real\",\"requestBody\":{}}}",
            request_body
        );
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nX-Flowable-Transport: real-http-client\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            response_body.len(),
            response_body
        );

        stream
            .write_all(response.as_bytes())
            .expect("echo server should write response");
        request
    });

    let config = ProcessEngineConfiguration {
        http_service: flowable_engine::service::config::HttpServiceTaskConfiguration {
            runtime_mode: HttpServiceRuntimeMode::Real,
            real_client: RealHttpClientConfiguration {
                allow_private_networks: true,
                ..Default::default()
            },
            ..Default::default()
        },
        ..Default::default()
    };
    let process_engine = ProcessEngine::new_with_config("real-http".to_string(), config);
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();
    let task_service = process_engine.get_task_service();

    let xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
                 xmlns:flowable="http://flowable.org/bpmn"
                 targetNamespace="Examples">
        <process id="httpRealRuntimeProcess" name="HTTP Real Runtime Process">
            <startEvent id="startEvent1" />
            <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="httpTask1" />
            <serviceTask id="httpTask1"
                         name="Invoke HTTP"
                         flowable:type="http"
                         flowable:resultVariableName="httpResult">
                <extensionElements>
                    <flowable:requestMethod>POST</flowable:requestMethod>
                    <flowable:requestUrl>http://{address}/echo</flowable:requestUrl>
                    <flowable:requestHeaders>{{"Accept":"application/json","X-Test":"m41"}}</flowable:requestHeaders>
                    <flowable:requestBody>{{"orderId":42,"approved":true}}</flowable:requestBody>
                    <flowable:basicAuthenticationUsername>real-user</flowable:basicAuthenticationUsername>
                    <flowable:basicAuthenticationPassword>real-password</flowable:basicAuthenticationPassword>
                    <flowable:bodyEncoding>json</flowable:bodyEncoding>
                </extensionElements>
            </serviceTask>
            <sequenceFlow id="flow2" sourceRef="httpTask1" targetRef="userTask1" />
            <userTask id="userTask1" name="Review HTTP Result" />
        </process>
    </definitions>"#
    );

    let builder = repository_service
        .create_deployment()
        .name("HTTP Real Runtime Deployment".to_string())
        .add_string("httpRealRuntimeProcess.bpmn20.xml".to_string(), xml);

    repository_service.deploy(builder).unwrap();
    let process_definition_id = repository_service.get_process_definition_ids().unwrap()[0].clone();

    let process_instance = runtime_service
        .start_process_instance(
            runtime_service
                .create_process_instance_builder()
                .process_definition_id(process_definition_id)
                .name("HTTP Real Runtime Instance".to_string()),
        )
        .unwrap();

    let tasks = task_service
        .get_tasks_by_process_instance_id(process_instance.id.clone())
        .unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].name, "Review HTTP Result");

    let http_result = runtime_service
        .get_variable(process_instance.id.clone(), "httpResult".to_string())
        .unwrap()
        .expect("httpResult should be stored on the execution");

    assert_eq!(http_result["request"]["method"], "POST");
    assert_eq!(http_result["request"]["headers"]["X-Test"], "m41");
    assert_eq!(http_result["request"]["bodyEncoding"], "json");
    assert_eq!(http_result["request"]["basicAuth"]["username"], "real-user");
    assert!(
        !http_result.to_string().contains("real-password"),
        "result variable must not leak the basic auth password"
    );
    assert_eq!(http_result["response"]["statusCode"], 200);
    assert_eq!(
        http_result["response"]["headers"]["x-flowable-transport"],
        "real-http-client"
    );
    assert_eq!(http_result["response"]["body"]["transport"], "real");
    assert_eq!(
        http_result["response"]["body"]["requestBody"]["orderId"],
        42
    );

    let captured_request = server.join().expect("echo server should finish");
    assert!(captured_request.starts_with("POST /echo HTTP/1.1"));
    let captured_request_lower = captured_request.to_ascii_lowercase();
    assert!(captured_request_lower.contains("x-test: m41"));
    assert!(
        captured_request_lower.contains("authorization: basic cmvhbc11c2vyonjlywwtcgfzc3dvcmq=")
    );
    assert!(captured_request.contains("\r\n\r\n{"));
    assert!(captured_request.contains("\"orderId\":42"));
    assert!(captured_request.contains("\"approved\":true"));
}

#[test]
fn build_with_config_returns_error_when_http_runtime_initialization_fails() {
    let mut config = ProcessEngineConfiguration::default();
    config.http_service.runtime_mode = HttpServiceRuntimeMode::Real;
    config.http_service.real_client.user_agent = Some("invalid\nuser-agent".to_string());

    let result = ProcessEngine::build_with_config(
        "bad-http-runtime".to_string(),
        Arc::new(flowable_engine::engine::time_source::SystemTimeSource),
        config,
    );

    assert!(
        result.is_err(),
        "invalid HTTP runtime configuration must be returned as an engine build error"
    );
}

#[test]
fn try_new_with_config_returns_database_initialization_error() {
    let mut config = ProcessEngineConfiguration::default();
    config.database.kind = EngineDatabaseKind::Sqlite;
    config.database.url = "invalid\0database".to_string();

    let result = ProcessEngine::try_new_with_config("bad-database".to_string(), config);

    assert!(result.is_err());
}
