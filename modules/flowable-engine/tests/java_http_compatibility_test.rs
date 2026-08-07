use flowable_engine::agenda::future_operations::PendingFutureRegistry;
use flowable_engine::bpmn::http_handler::{
    HttpHandlerRegistry, HttpRequestHandler, HttpRequestHandlerContext, HttpResponseHandler,
    HttpResponseHandlerContext,
};
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::error::FlowableError;
use flowable_engine::service::config::{
    HttpServiceRuntimeMode, HttpServiceTaskConfiguration, ProcessEngineConfiguration,
    RealHttpClientConfiguration,
};
use serde_json::json;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

struct JavaStyleRequestHandler;

impl HttpRequestHandler for JavaStyleRequestHandler {
    fn handle_request(
        &self,
        context: &mut HttpRequestHandlerContext<'_>,
    ) -> Result<(), FlowableError> {
        context
            .request
            .headers
            .insert("X-Java-Handler".to_string(), "request".to_string());
        context.execution.set_process_variable(
            "requestHandlerCalled".to_string(),
            json!(context.fields.get("marker")),
        );
        Ok(())
    }
}

struct JavaStyleResponseHandler;

impl HttpResponseHandler for JavaStyleResponseHandler {
    fn handle_response(
        &self,
        context: &mut HttpResponseHandlerContext<'_>,
    ) -> Result<(), FlowableError> {
        context.execution.set_process_variable(
            "responseHandlerStatus".to_string(),
            json!(context.exchange.response.status_code),
        );
        if let Some(request_mutation) = context.execution.process_variable("requestHandlerCalled") {
            context.execution.set_process_variable(
                "responseHandlerSawRequestMutation".to_string(),
                request_mutation,
            );
        }
        Ok(())
    }
}

struct ThreadRecordingResponseHandler {
    observed_thread: Arc<Mutex<Option<thread::ThreadId>>>,
}

impl HttpResponseHandler for ThreadRecordingResponseHandler {
    fn handle_response(
        &self,
        _context: &mut HttpResponseHandlerContext<'_>,
    ) -> Result<(), FlowableError> {
        *self.observed_thread.lock().unwrap() = Some(thread::current().id());
        Ok(())
    }
}

struct FailingResponseHandler;

impl HttpResponseHandler for FailingResponseHandler {
    fn handle_response(
        &self,
        _context: &mut HttpResponseHandlerContext<'_>,
    ) -> Result<(), FlowableError> {
        Err(FlowableError::ExecutionError(
            "response handler failed inside command transaction".to_string(),
        ))
    }
}

fn deploy_process(engine: &ProcessEngine, process_id: &str, extensions: &str) -> String {
    let xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" xmlns:flowable="http://flowable.org/bpmn" targetNamespace="Examples">
  <process id="{process_id}" isExecutable="true">
    <startEvent id="start" />
    <sequenceFlow id="f1" sourceRef="start" targetRef="httpTask" />
    <serviceTask id="httpTask" flowable:type="http" flowable:resultVariableName="httpResult">
      <extensionElements>{extensions}</extensionElements>
    </serviceTask>
    <sequenceFlow id="f2" sourceRef="httpTask" targetRef="review" />
    <userTask id="review" name="Review" />
  </process>
</definitions>"#
    );
    engine
        .get_repository_service()
        .deploy(
            engine
                .get_repository_service()
                .create_deployment()
                .name(format!("{process_id}-deployment"))
                .add_string(format!("{process_id}.bpmn20.xml"), xml),
        )
        .unwrap();
    engine
        .get_repository_service()
        .get_process_definition_ids()
        .unwrap()[0]
        .clone()
}

fn deploy_and_start(engine: &ProcessEngine, process_id: &str, extensions: &str) -> String {
    let definition_id = deploy_process(engine, process_id, extensions);
    engine
        .get_runtime_service()
        .start_process_instance_by_id(definition_id, None)
        .unwrap()
        .id
}

#[test]
fn java_request_and_response_variables_coexist_with_rust_structured_result() {
    let engine = ProcessEngine::new("java-http-compat".to_string());
    let process_instance_id = deploy_and_start(
        &engine,
        "javaHttpCompatibility",
        r#"
          <flowable:requestMethod>POST</flowable:requestMethod>
          <flowable:requestUrl>https://example.flowable.local/orders</flowable:requestUrl>
          <flowable:requestHeaders>{"Accept":"application/json"}</flowable:requestHeaders>
          <flowable:requestBody>{"orderId":42}</flowable:requestBody>
          <flowable:disallowRedirects>true</flowable:disallowRedirects>
          <flowable:saveRequestVariables>true</flowable:saveRequestVariables>
          <flowable:saveResponseParameters>true</flowable:saveResponseParameters>
          <flowable:saveResponseVariableAsJson>true</flowable:saveResponseVariableAsJson>
          <flowable:responseVariableName>javaBody</flowable:responseVariableName>
          <flowable:resultVariablePrefix>javaHttp</flowable:resultVariablePrefix>
        "#,
    );
    let runtime = engine.get_runtime_service();
    assert_eq!(
        runtime
            .get_variable(
                process_instance_id.clone(),
                "javaHttpRequestMethod".to_string()
            )
            .unwrap(),
        Some(json!("POST"))
    );
    assert_eq!(
        runtime
            .get_variable(
                process_instance_id.clone(),
                "javaHttpDisallowRedirects".to_string()
            )
            .unwrap(),
        Some(json!(true))
    );
    assert_eq!(
        runtime
            .get_variable(
                process_instance_id.clone(),
                "javaHttpResponseStatusCode".to_string()
            )
            .unwrap(),
        Some(json!(200))
    );
    assert_eq!(
        runtime
            .get_variable(process_instance_id.clone(), "javaBody".to_string())
            .unwrap()
            .unwrap()["accepted"],
        json!(true)
    );
    assert_eq!(
        runtime
            .get_variable(process_instance_id.clone(), "httpResult".to_string())
            .unwrap()
            .unwrap()["service"],
        json!("http")
    );
    assert_eq!(
        engine
            .get_task_service()
            .get_tasks_by_process_instance_id(process_instance_id)
            .unwrap()[0]
            .name,
        "Review"
    );
}

#[test]
fn java_ignore_exception_continues_and_preserves_rust_error_result() {
    let config = ProcessEngineConfiguration {
        http_service: HttpServiceTaskConfiguration {
            runtime_mode: HttpServiceRuntimeMode::Real,
            real_client: RealHttpClientConfiguration {
                allow_private_networks: true,
                ..Default::default()
            },
            ..Default::default()
        },
        ..Default::default()
    };
    let engine = ProcessEngine::new_with_config("java-http-ignore".to_string(), config);
    let process_instance_id = deploy_and_start(
        &engine,
        "javaHttpIgnoreException",
        r#"
          <flowable:field name="requestMethod"><flowable:string>GET</flowable:string></flowable:field>
          <flowable:field name="requestUrl"><flowable:string>http://127.0.0.1:1/unreachable</flowable:string></flowable:field>
          <flowable:field name="requestTimeout"><flowable:string>100</flowable:string></flowable:field>
          <flowable:field name="ignoreException"><flowable:string>true</flowable:string></flowable:field>
          <flowable:field name="resultVariablePrefix"><flowable:string>javaHttp</flowable:string></flowable:field>
        "#,
    );
    let runtime = engine.get_runtime_service();
    let error_message = runtime
        .get_variable(
            process_instance_id.clone(),
            "javaHttpErrorMessage".to_string(),
        )
        .unwrap()
        .unwrap();
    assert!(error_message.as_str().unwrap().contains("HTTP"));
    let rust_result = runtime
        .get_variable(process_instance_id.clone(), "httpResult".to_string())
        .unwrap()
        .unwrap();
    assert_eq!(rust_result["error"]["ignored"], json!(true));
    assert_eq!(
        engine
            .get_task_service()
            .get_tasks_by_process_instance_id(process_instance_id)
            .unwrap()[0]
            .name,
        "Review"
    );
}

#[test]
fn java_response_variables_are_applied_before_async_continuation() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut buffer = [0_u8; 2048];
        let _ = stream.read(&mut buffer).unwrap();
        let body = r#"{"source":"async-java-compat"}"#;
        write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).unwrap();
    });
    let config = ProcessEngineConfiguration {
        http_service: HttpServiceTaskConfiguration {
            runtime_mode: HttpServiceRuntimeMode::Async,
            real_client: RealHttpClientConfiguration {
                allow_private_networks: true,
                ..Default::default()
            },
            ..Default::default()
        },
        ..Default::default()
    };
    let engine = ProcessEngine::new_with_config("java-http-async".to_string(), config);
    let process_instance_id = deploy_and_start(
        &engine,
        "javaHttpAsyncCompatibility",
        &format!(
            r#"
          <flowable:field name="requestMethod"><flowable:string>GET</flowable:string></flowable:field>
          <flowable:field name="requestUrl"><flowable:string>http://{address}/async</flowable:string></flowable:field>
          <flowable:field name="saveResponseParameters"><flowable:string>true</flowable:string></flowable:field>
          <flowable:field name="saveResponseVariableAsJson"><flowable:string>true</flowable:string></flowable:field>
          <flowable:field name="responseVariableName"><flowable:string>javaAsyncBody</flowable:string></flowable:field>
          <flowable:field name="resultVariablePrefix"><flowable:string>javaAsync</flowable:string></flowable:field>
        "#
        ),
    );
    server.join().unwrap();
    let runtime = engine.get_runtime_service();
    assert_eq!(
        runtime
            .get_variable(
                process_instance_id.clone(),
                "javaAsyncResponseStatusCode".to_string()
            )
            .unwrap(),
        Some(json!(200))
    );
    assert_eq!(
        runtime
            .get_variable(process_instance_id.clone(), "javaAsyncBody".to_string())
            .unwrap()
            .unwrap()["source"],
        json!("async-java-compat")
    );
    let rust_result = runtime
        .get_variable(process_instance_id.clone(), "httpResult".to_string())
        .unwrap()
        .unwrap();
    assert_eq!(rust_result["response"]["statusCode"], json!(200));
    assert!(rust_result.get("__flowableHttpCompatibility").is_none());
    assert_eq!(
        engine
            .get_task_service()
            .get_tasks_by_process_instance_id(process_instance_id)
            .unwrap()[0]
            .name,
        "Review"
    );
}

#[test]
fn java_fail_status_codes_raise_stable_http_execution_error() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut buffer = [0_u8; 1024];
        let _ = stream.read(&mut buffer).unwrap();
        let body = r#"{"failure":"compat"}"#;
        write!(stream, "HTTP/1.1 404 Not Found\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).unwrap();
    });
    let config = ProcessEngineConfiguration {
        http_service: HttpServiceTaskConfiguration {
            runtime_mode: HttpServiceRuntimeMode::Real,
            real_client: RealHttpClientConfiguration {
                allow_private_networks: true,
                ..Default::default()
            },
            ..Default::default()
        },
        ..Default::default()
    };
    let engine = ProcessEngine::new_with_config("java-http-status".to_string(), config);
    let definition_id = deploy_process(
        &engine,
        "javaHttpFailStatus",
        &format!(
            r#"
          <flowable:field name="requestMethod"><flowable:string>GET</flowable:string></flowable:field>
          <flowable:field name="requestUrl"><flowable:string>http://{address}/missing</flowable:string></flowable:field>
          <flowable:field name="failStatusCodes"><flowable:string>404</flowable:string></flowable:field>
        "#
        ),
    );
    let error = engine
        .get_runtime_service()
        .start_process_instance_by_id(definition_id, None)
        .unwrap_err();
    server.join().unwrap();
    assert!(
        error.to_string().contains("HTTP404"),
        "unexpected status error: {error}"
    );
}

#[test]
fn java_http_field_expressions_resolve_against_process_variables() {
    let engine = ProcessEngine::new("java-http-expressions".to_string());
    let definition_id = deploy_process(
        &engine,
        "javaHttpExpressions",
        r#"
          <flowable:field name="requestMethod"><flowable:expression>${httpMethod}</flowable:expression></flowable:field>
          <flowable:field name="requestUrl"><flowable:expression>${targetUrl}</flowable:expression></flowable:field>
          <flowable:field name="requestHeaders"><flowable:expression>${httpHeaders}</flowable:expression></flowable:field>
          <flowable:field name="requestBody"><flowable:expression>${httpPayload}</flowable:expression></flowable:field>
        "#,
    );
    let builder = engine
        .get_runtime_service()
        .create_process_instance_builder()
        .process_definition_id(definition_id)
        .variable("httpMethod".to_string(), json!("POST"))
        .variable(
            "targetUrl".to_string(),
            json!("https://example.flowable.local/expression"),
        )
        .variable(
            "httpHeaders".to_string(),
            json!({"X-Flowable-Test": "expression"}),
        )
        .variable(
            "httpPayload".to_string(),
            json!({"source": "process-variable", "count": 2}),
        );
    let process_instance = engine
        .get_runtime_service()
        .start_process_instance(builder)
        .unwrap();
    let result = engine
        .get_runtime_service()
        .get_variable(process_instance.id, "httpResult".to_string())
        .unwrap()
        .unwrap();
    assert_eq!(result["request"]["method"], json!("POST"));
    assert_eq!(
        result["request"]["url"],
        json!("https://example.flowable.local/expression")
    );
    assert_eq!(
        result["request"]["headers"]["X-Flowable-Test"],
        json!("expression")
    );
    assert_eq!(
        result["request"]["body"],
        json!({"source": "process-variable", "count": 2})
    );
}

fn assert_java_handle_status_codes_triggers_error_boundary_event(
    runtime_mode: HttpServiceRuntimeMode,
    engine_name: &str,
) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut buffer = [0_u8; 1024];
        let _ = stream.read(&mut buffer).unwrap();
        let body = r#"{"failure":"handled"}"#;
        write!(stream, "HTTP/1.1 404 Not Found\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).unwrap();
    });
    let xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" xmlns:flowable="http://flowable.org/bpmn" targetNamespace="Examples">
  <error id="http404Error" errorCode="HTTP404" />
  <process id="javaHandledHttpStatus" isExecutable="true">
    <startEvent id="start" />
    <sequenceFlow id="f1" sourceRef="start" targetRef="httpTask" />
    <serviceTask id="httpTask" flowable:type="http" flowable:resultVariableName="httpResult">
      <extensionElements>
        <flowable:field name="requestMethod"><flowable:string>GET</flowable:string></flowable:field>
        <flowable:field name="requestUrl"><flowable:string>http://{address}/missing</flowable:string></flowable:field>
        <flowable:field name="handleStatusCodes"><flowable:string>404</flowable:string></flowable:field>
      </extensionElements>
    </serviceTask>
    <boundaryEvent id="httpErrorBoundary" attachedToRef="httpTask">
      <errorEventDefinition errorRef="http404Error" />
    </boundaryEvent>
    <sequenceFlow id="normal" sourceRef="httpTask" targetRef="unexpected" />
    <sequenceFlow id="handled" sourceRef="httpErrorBoundary" targetRef="recovery" />
    <userTask id="unexpected" name="Unexpected normal path" />
    <userTask id="recovery" name="HTTP recovery" />
  </process>
</definitions>"#
    );
    let config = ProcessEngineConfiguration {
        http_service: HttpServiceTaskConfiguration {
            runtime_mode,
            real_client: RealHttpClientConfiguration {
                allow_private_networks: true,
                ..Default::default()
            },
            ..Default::default()
        },
        ..Default::default()
    };
    let engine = ProcessEngine::new_with_config(engine_name.to_string(), config);
    engine
        .get_repository_service()
        .deploy(
            engine
                .get_repository_service()
                .create_deployment()
                .name("java-http-boundary-deployment".to_string())
                .add_string("java-http-boundary.bpmn20.xml".to_string(), xml),
        )
        .unwrap();
    let process_definition_id = engine
        .get_repository_service()
        .get_process_definition_ids()
        .unwrap()
        .into_iter()
        .find(|id| id.contains("javaHandledHttpStatus"))
        .expect("HTTP boundary process definition should be deployed");
    let process_instance = engine
        .get_runtime_service()
        .start_process_instance_by_id(process_definition_id, None)
        .unwrap();
    server.join().unwrap();
    let tasks = engine
        .get_task_service()
        .get_tasks_by_process_instance_id(process_instance.id)
        .unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].name, "HTTP recovery");
}

#[test]
fn java_handle_status_codes_triggers_error_boundary_event() {
    assert_java_handle_status_codes_triggers_error_boundary_event(
        HttpServiceRuntimeMode::Real,
        "java-http-boundary-sync",
    );
}

#[test]
fn java_handle_status_codes_triggers_error_boundary_event_after_async_completion() {
    assert_java_handle_status_codes_triggers_error_boundary_event(
        HttpServiceRuntimeMode::Async,
        "java-http-boundary-async",
    );
}

#[test]
fn java_handle_status_codes_triggers_error_event_subprocess() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut buffer = [0_u8; 1024];
        let _ = stream.read(&mut buffer).unwrap();
        write!(
            stream,
            "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        )
        .unwrap();
    });
    let xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" xmlns:flowable="http://flowable.org/bpmn" targetNamespace="Examples">
  <error id="http404Error" errorCode="HTTP404" />
  <process id="javaHandledHttpEventSubprocess" isExecutable="true">
    <startEvent id="start" />
    <sequenceFlow id="f1" sourceRef="start" targetRef="httpTask" />
    <serviceTask id="httpTask" flowable:type="http">
      <extensionElements>
        <flowable:field name="requestMethod"><flowable:string>GET</flowable:string></flowable:field>
        <flowable:field name="requestUrl"><flowable:string>http://{address}/missing</flowable:string></flowable:field>
        <flowable:field name="handleStatusCodes"><flowable:string>404</flowable:string></flowable:field>
      </extensionElements>
    </serviceTask>
    <sequenceFlow id="normal" sourceRef="httpTask" targetRef="unexpected" />
    <userTask id="unexpected" name="Unexpected normal path" />
    <subProcess id="httpErrorEventSubprocess" triggeredByEvent="true">
      <startEvent id="httpErrorStart" isInterrupting="true">
        <errorEventDefinition errorRef="http404Error" />
      </startEvent>
      <sequenceFlow id="errorFlow" sourceRef="httpErrorStart" targetRef="eventRecovery" />
      <userTask id="eventRecovery" name="HTTP event subprocess recovery" />
    </subProcess>
  </process>
</definitions>"#
    );
    let config = ProcessEngineConfiguration {
        http_service: HttpServiceTaskConfiguration {
            runtime_mode: HttpServiceRuntimeMode::Real,
            real_client: RealHttpClientConfiguration {
                allow_private_networks: true,
                ..Default::default()
            },
            ..Default::default()
        },
        ..Default::default()
    };
    let engine = ProcessEngine::new_with_config("java-http-event-subprocess".to_string(), config);
    engine
        .get_repository_service()
        .deploy(
            engine
                .get_repository_service()
                .create_deployment()
                .name("java-http-event-subprocess-deployment".to_string())
                .add_string("java-http-event-subprocess.bpmn20.xml".to_string(), xml),
        )
        .unwrap();
    let definition_id = engine
        .get_repository_service()
        .get_process_definition_ids()
        .unwrap()
        .into_iter()
        .find(|id| id.contains("javaHandledHttpEventSubprocess"))
        .unwrap();
    let process_instance = engine
        .get_runtime_service()
        .start_process_instance_by_id(definition_id, None)
        .unwrap();
    server.join().unwrap();
    let tasks = engine
        .get_task_service()
        .get_tasks_by_process_instance_id(process_instance.id)
        .unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].name, "HTTP event subprocess recovery");
}

#[test]
fn java_uncaught_handled_status_is_reported_as_bpmn_error_code() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut buffer = [0_u8; 1024];
        let _ = stream.read(&mut buffer).unwrap();
        write!(
            stream,
            "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        )
        .unwrap();
    });
    let config = ProcessEngineConfiguration {
        http_service: HttpServiceTaskConfiguration {
            runtime_mode: HttpServiceRuntimeMode::Real,
            real_client: RealHttpClientConfiguration {
                allow_private_networks: true,
                ..Default::default()
            },
            ..Default::default()
        },
        ..Default::default()
    };
    let engine =
        ProcessEngine::new_with_config("java-http-uncaught-bpmn-error".to_string(), config);
    let definition_id = deploy_process(
        &engine,
        "javaHttpUncaughtHandledStatus",
        &format!(
            r#"
          <flowable:field name="requestMethod"><flowable:string>GET</flowable:string></flowable:field>
          <flowable:field name="requestUrl"><flowable:string>http://{address}/missing</flowable:string></flowable:field>
          <flowable:field name="handleStatusCodes"><flowable:string>404</flowable:string></flowable:field>
        "#
        ),
    );
    let error = engine
        .get_runtime_service()
        .start_process_instance_by_id(definition_id, None)
        .unwrap_err();
    server.join().unwrap();
    assert!(error.to_string().contains("HTTP404"));
}

#[test]
fn java_http_request_and_response_handlers_use_independent_rust_registry() {
    let mut handlers = HttpHandlerRegistry::new();
    handlers.register_request_handler(
        "com.example.RequestHandler",
        Arc::new(JavaStyleRequestHandler),
    );
    handlers.register_response_handler(
        "com.example.ResponseHandler",
        Arc::new(JavaStyleResponseHandler),
    );
    let config = ProcessEngineConfiguration {
        http_handler_registry: Some(handlers),
        ..Default::default()
    };
    let engine = ProcessEngine::new_with_config("java-http-handlers".to_string(), config);
    let definition_id = deploy_process(
        &engine,
        "javaHttpHandlers",
        r#"
          <flowable:field name="requestMethod"><flowable:string>GET</flowable:string></flowable:field>
          <flowable:field name="requestUrl"><flowable:string>https://example.flowable.local/handler</flowable:string></flowable:field>
          <flowable:httpRequestHandler class="com.example.RequestHandler">
            <flowable:field name="marker"><flowable:string>request-field</flowable:string></flowable:field>
          </flowable:httpRequestHandler>
          <flowable:httpResponseHandler delegateExpression="${responseHandlerName}" />
        "#,
    );
    let builder = engine
        .get_runtime_service()
        .create_process_instance_builder()
        .process_definition_id(definition_id)
        .variable(
            "responseHandlerName".to_string(),
            json!("com.example.ResponseHandler"),
        );
    let process_instance = engine
        .get_runtime_service()
        .start_process_instance(builder)
        .unwrap();
    let runtime = engine.get_runtime_service();
    assert_eq!(
        runtime
            .get_variable(
                process_instance.id.clone(),
                "requestHandlerCalled".to_string()
            )
            .unwrap(),
        Some(json!("request-field"))
    );
    assert_eq!(
        runtime
            .get_variable(
                process_instance.id.clone(),
                "responseHandlerStatus".to_string()
            )
            .unwrap(),
        Some(json!(200))
    );
    assert_eq!(
        runtime
            .get_variable(process_instance.id, "httpResult".to_string())
            .unwrap()
            .unwrap()["request"]["headers"]["X-Java-Handler"],
        json!("request")
    );
}

#[test]
fn java_http_handlers_preserve_mutations_after_async_completion() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut buffer = [0_u8; 2048];
        let _ = stream.read(&mut buffer).unwrap();
        let body = r#"{"handler":"async"}"#;
        write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).unwrap();
    });
    let mut handlers = HttpHandlerRegistry::new();
    handlers.register_request_handler(
        "com.example.RequestHandler",
        Arc::new(JavaStyleRequestHandler),
    );
    handlers.register_response_handler(
        "com.example.ResponseHandler",
        Arc::new(JavaStyleResponseHandler),
    );
    let config = ProcessEngineConfiguration {
        http_service: HttpServiceTaskConfiguration {
            runtime_mode: HttpServiceRuntimeMode::Async,
            real_client: RealHttpClientConfiguration {
                allow_private_networks: true,
                ..Default::default()
            },
            ..Default::default()
        },
        http_handler_registry: Some(handlers),
        ..Default::default()
    };
    let engine = ProcessEngine::new_with_config("java-http-handlers-async".to_string(), config);
    let process_instance_id = deploy_and_start(
        &engine,
        "javaHttpHandlersAsync",
        &format!(
            r#"
          <flowable:field name="requestMethod"><flowable:string>GET</flowable:string></flowable:field>
          <flowable:field name="requestUrl"><flowable:string>http://{address}/handler</flowable:string></flowable:field>
          <flowable:httpRequestHandler class="com.example.RequestHandler">
            <flowable:field name="marker"><flowable:string>async-request-field</flowable:string></flowable:field>
          </flowable:httpRequestHandler>
          <flowable:httpResponseHandler class="com.example.ResponseHandler" />
        "#
        ),
    );
    server.join().unwrap();
    let runtime = engine.get_runtime_service();
    assert_eq!(
        runtime
            .get_variable(
                process_instance_id.clone(),
                "responseHandlerStatus".to_string()
            )
            .unwrap(),
        Some(json!(200))
    );
    assert_eq!(
        runtime
            .get_variable(
                process_instance_id.clone(),
                "responseHandlerSawRequestMutation".to_string()
            )
            .unwrap(),
        Some(json!("async-request-field")),
        "request-handler mutations must be visible to the response handler in the same command"
    );
    assert_eq!(
        runtime
            .get_variable(process_instance_id, "httpResult".to_string())
            .unwrap()
            .unwrap()["request"]["headers"]["X-Java-Handler"],
        json!("request")
    );
}

fn assert_response_handler_runs_on_command_thread(
    runtime_mode: HttpServiceRuntimeMode,
    engine_name: &str,
    parallel_field: Option<bool>,
) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut buffer = [0_u8; 2048];
        let _ = stream.read(&mut buffer).unwrap();
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        )
        .unwrap();
    });
    let observed_thread = Arc::new(Mutex::new(None));
    let mut handlers = HttpHandlerRegistry::new();
    handlers.register_response_handler(
        "com.example.ThreadRecordingResponseHandler",
        Arc::new(ThreadRecordingResponseHandler {
            observed_thread: Arc::clone(&observed_thread),
        }),
    );
    let config = ProcessEngineConfiguration {
        http_service: HttpServiceTaskConfiguration {
            runtime_mode,
            real_client: RealHttpClientConfiguration {
                allow_private_networks: true,
                ..Default::default()
            },
            ..Default::default()
        },
        http_handler_registry: Some(handlers),
        ..Default::default()
    };
    let engine = ProcessEngine::new_with_config(engine_name.to_string(), config);
    let parallel_extension = parallel_field
        .map(|value| {
            format!(
                r#"<flowable:field name="parallelInSameTransaction"><flowable:string>{value}</flowable:string></flowable:field>"#
            )
        })
        .unwrap_or_default();
    let definition_id = deploy_process(
        &engine,
        engine_name,
        &format!(
            r#"
          <flowable:field name="requestMethod"><flowable:string>GET</flowable:string></flowable:field>
          <flowable:field name="requestUrl"><flowable:string>http://{address}/thread</flowable:string></flowable:field>
          {parallel_extension}
          <flowable:httpResponseHandler class="com.example.ThreadRecordingResponseHandler" />
        "#
        ),
    );

    let command_thread = thread::current().id();
    engine
        .get_runtime_service()
        .start_process_instance_by_id(definition_id, None)
        .unwrap();
    server.join().unwrap();

    assert_eq!(
        *observed_thread.lock().unwrap(),
        Some(command_thread),
        "response handlers must execute on the engine command/transaction thread"
    );
}

#[test]
fn java_response_handler_runs_on_command_thread_for_inline_transport() {
    assert_response_handler_runs_on_command_thread(
        HttpServiceRuntimeMode::Real,
        "javaHttpInlineHandlerThread",
        Some(false),
    );
}

#[test]
fn java_response_handler_returns_to_command_thread_for_parallel_transport() {
    assert_response_handler_runs_on_command_thread(
        HttpServiceRuntimeMode::Real,
        "javaHttpParallelHandlerThread",
        Some(true),
    );
}

#[test]
fn rust_async_extension_keeps_response_handler_on_command_thread_when_java_field_is_absent() {
    assert_response_handler_runs_on_command_thread(
        HttpServiceRuntimeMode::Async,
        "rustAsyncHttpHandlerThread",
        None,
    );
}

#[test]
fn java_http_handler_failure_rolls_back_request_mutations_and_runtime_state() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let request_count = Arc::new(AtomicUsize::new(0));
    let server_request_count = Arc::clone(&request_count);
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        server_request_count.fetch_add(1, Ordering::SeqCst);
        let mut buffer = [0_u8; 2048];
        let _ = stream.read(&mut buffer).unwrap();
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        )
        .unwrap();
    });
    let mut handlers = HttpHandlerRegistry::new();
    handlers.register_request_handler(
        "com.example.RequestHandler",
        Arc::new(JavaStyleRequestHandler),
    );
    handlers.register_response_handler(
        "com.example.FailingResponseHandler",
        Arc::new(FailingResponseHandler),
    );
    let pending_futures = Arc::new(PendingFutureRegistry::new());
    let config = ProcessEngineConfiguration {
        http_service: HttpServiceTaskConfiguration {
            runtime_mode: HttpServiceRuntimeMode::Async,
            real_client: RealHttpClientConfiguration {
                allow_private_networks: true,
                ..Default::default()
            },
            ..Default::default()
        },
        http_handler_registry: Some(handlers),
        pending_future_registry: Arc::clone(&pending_futures),
        ..Default::default()
    };
    let engine = ProcessEngine::new_with_config("java-http-handler-rollback".to_string(), config);
    let definition_id = deploy_process(
        &engine,
        "javaHttpHandlerRollback",
        &format!(
            r#"
          <flowable:field name="requestMethod"><flowable:string>GET</flowable:string></flowable:field>
          <flowable:field name="requestUrl"><flowable:string>http://{address}/rollback</flowable:string></flowable:field>
          <flowable:field name="parallelInSameTransaction"><flowable:string>true</flowable:string></flowable:field>
          <flowable:httpRequestHandler class="com.example.RequestHandler">
            <flowable:field name="marker"><flowable:string>must-roll-back</flowable:string></flowable:field>
          </flowable:httpRequestHandler>
          <flowable:httpResponseHandler class="com.example.FailingResponseHandler" />
        "#
        ),
    );

    let error = engine
        .get_runtime_service()
        .start_process_instance_by_id(definition_id, None)
        .unwrap_err();
    server.join().unwrap();
    assert_eq!(
        request_count.load(Ordering::SeqCst),
        1,
        "a failed command must not silently repeat the external HTTP side effect"
    );
    assert!(
        pending_futures.is_empty(),
        "terminal async HTTP failures must release their pending future"
    );
    assert!(
        error
            .to_string()
            .contains("response handler failed inside command transaction"),
        "unexpected response handler error: {error}"
    );

    let snapshot = engine.export_recovery_snapshot();
    assert!(
        snapshot.process_instances.is_empty(),
        "failed HTTP command must not commit a process instance"
    );
    assert!(
        snapshot.executions.is_empty(),
        "request-handler variable mutations must roll back with the execution"
    );
    assert!(
        snapshot.tasks.is_empty(),
        "outgoing work must not be created after a failed response handler"
    );
}

#[test]
fn java_http_script_handlers_use_secure_script_engine() {
    let config = ProcessEngineConfiguration {
        enable_secure_scripting: true,
        supported_script_languages: vec!["javascript".to_string()],
        ..Default::default()
    };
    let engine = ProcessEngine::new_with_config("java-http-script-handlers".to_string(), config);
    let definition_id = deploy_process(
        &engine,
        "javaHttpScriptHandlers",
        r#"
          <flowable:field name="requestMethod"><flowable:string>GET</flowable:string></flowable:field>
          <flowable:field name="requestUrl"><flowable:string>https://example.flowable.local/script</flowable:string></flowable:field>
          <flowable:httpRequestHandler type="script">
            <flowable:script language="javascript" resultVariable="requestScriptResult">
              var marker = "request-script";
              execution.setVariable("requestScriptCalled", marker);
              return marker;
            </flowable:script>
          </flowable:httpRequestHandler>
          <flowable:httpResponseHandler type="script">
            <flowable:script language="javascript">
              execution.setVariable("responseScriptCalled", "response-script");
            </flowable:script>
          </flowable:httpResponseHandler>
        "#,
    );
    let process_instance = engine
        .get_runtime_service()
        .start_process_instance_by_id(definition_id, None)
        .unwrap();
    let runtime = engine.get_runtime_service();
    assert_eq!(
        runtime
            .get_variable(
                process_instance.id.clone(),
                "requestScriptCalled".to_string()
            )
            .unwrap(),
        Some(json!("request-script"))
    );
    assert_eq!(
        runtime
            .get_variable(
                process_instance.id.clone(),
                "requestScriptResult".to_string()
            )
            .unwrap(),
        Some(json!("request-script"))
    );
    assert_eq!(
        runtime
            .get_variable(process_instance.id, "responseScriptCalled".to_string())
            .unwrap(),
        Some(json!("response-script"))
    );
}
