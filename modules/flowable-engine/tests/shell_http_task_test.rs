use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::service::config::ProcessEngineConfiguration;
use serde_json::json;

fn shell_enabled_engine(name: &str) -> ProcessEngine {
    // Explicit opt-in: shell tasks are disabled by default (security deviation from Java).
    ProcessEngine::new_with_config(
        name.to_string(),
        ProcessEngineConfiguration {
            shell_tasks_enabled: true,
            ..Default::default()
        },
    )
}

fn deploy_and_start(xml: &str) -> (ProcessEngine, String) {
    let process_engine = shell_enabled_engine("default");
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();

    let builder = repository_service
        .create_deployment()
        .name("Shell/HTTP Task Test Deployment".to_string())
        .add_string("test_process.bpmn20.xml".to_string(), xml.to_string());
    repository_service.deploy(builder).unwrap();

    let process_definition_id = repository_service.get_process_definition_ids().unwrap()[0].clone();
    let process_instance = runtime_service
        .start_process_instance_by_id(process_definition_id, None)
        .unwrap();

    (process_engine, process_instance.id)
}

#[test]
fn test_shell_task_disabled_by_default() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
                 xmlns:flowable="http://flowable.org/bpmn"
                 targetNamespace="Examples">
        <process id="shellDisabledProcess" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="flow1" sourceRef="start" targetRef="shellTask" />
            <serviceTask id="shellTask" flowable:type="shell" flowable:resultVariableName="shellResult">
                <extensionElements>
                    <flowable:command>cmd</flowable:command>
                    <flowable:arg>/c</flowable:arg>
                    <flowable:arg>echo should-not-run</flowable:arg>
                </extensionElements>
            </serviceTask>
            <sequenceFlow id="flow2" sourceRef="shellTask" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#;

    let process_engine = ProcessEngine::new("shell-disabled-default".to_string());
    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();
    repository_service
        .deploy(
            repository_service
                .create_deployment()
                .name("shell disabled".to_string())
                .add_string("shell.bpmn20.xml".to_string(), xml.to_string()),
        )
        .unwrap();
    let process_definition_id = repository_service.get_process_definition_ids().unwrap()[0].clone();
    let err = runtime_service
        .start_process_instance_by_id(process_definition_id, None)
        .expect_err("shell tasks must be disabled by default");
    let msg = err.to_string();
    assert!(
        msg.contains("disabled") && msg.contains("shell_tasks_enabled"),
        "expected disable message with enable hint, got: {msg}"
    );
}

#[test]
fn test_shell_task_executes_command_and_captures_output() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
                 xmlns:flowable="http://flowable.org/bpmn"
                 targetNamespace="Examples">
        <process id="shellProcess" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="flow1" sourceRef="start" targetRef="shellTask" />
            <serviceTask id="shellTask" flowable:type="shell" flowable:resultVariableName="shellResult">
                <extensionElements>
                    <flowable:command>cmd</flowable:command>
                    <flowable:arg>/c</flowable:arg>
                    <flowable:arg>echo Hello World</flowable:arg>
                </extensionElements>
            </serviceTask>
            <sequenceFlow id="flow2" sourceRef="shellTask" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#;

    let (process_engine, process_instance_id) = deploy_and_start(xml);

    let runtime_store = process_engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let execution = runtime_store
        .find_execution(&process_instance_id, &mut session)
        .expect("execution should exist");

    let shell_result = execution
        .persistent_process_variable("shellResult")
        .expect("shellResult variable should be set");

    let result_obj = shell_result
        .as_object()
        .expect("shellResult should be an object");
    assert_eq!(
        result_obj["service"],
        json!("shell"),
        "Expected service to be 'shell'"
    );
    assert_eq!(
        result_obj["command"],
        json!("cmd"),
        "Expected command to be 'cmd'"
    );

    let stdout = result_obj["stdout"]
        .as_str()
        .expect("stdout should be a string");
    assert!(
        stdout.contains("Hello World"),
        "Expected shell output to contain 'Hello World', got: {}",
        stdout
    );
}

#[test]
fn test_shell_task_captures_exit_code() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
                 xmlns:flowable="http://flowable.org/bpmn"
                 targetNamespace="Examples">
        <process id="shellExitCodeProcess" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="flow1" sourceRef="start" targetRef="shellTask" />
            <serviceTask id="shellTask" flowable:type="shell" flowable:resultVariableName="shellResult">
                <extensionElements>
                    <flowable:command>cmd</flowable:command>
                    <flowable:arg>/c</flowable:arg>
                    <flowable:arg>exit</flowable:arg>
                    <flowable:arg>0</flowable:arg>
                </extensionElements>
            </serviceTask>
            <sequenceFlow id="flow2" sourceRef="shellTask" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#;

    let (process_engine, process_instance_id) = deploy_and_start(xml);

    let runtime_store = process_engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let execution = runtime_store
        .find_execution(&process_instance_id, &mut session)
        .expect("execution should exist");

    let shell_result = execution
        .persistent_process_variable("shellResult")
        .expect("shellResult variable should be set");

    let result_obj = shell_result
        .as_object()
        .expect("shellResult should be an object");
    assert_eq!(result_obj["exitCode"], json!(0), "Expected exit code 0");
}

/// P51 S3 — Java ShellActivityBehavior: outputVariable / errorCodeVariable naming.
#[test]
fn test_shell_task_java_variable_names_output_and_error_code() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
                 xmlns:flowable="http://flowable.org/bpmn"
                 targetNamespace="Examples">
        <process id="shellJavaVarsProcess" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="flow1" sourceRef="start" targetRef="shellTask" />
            <serviceTask id="shellTask" flowable:type="shell" flowable:resultVariableName="shellResult">
                <extensionElements>
                    <flowable:command>cmd</flowable:command>
                    <flowable:arg>/c</flowable:arg>
                    <flowable:arg>echo Portable</flowable:arg>
                    <flowable:outputVariable>shellOut</flowable:outputVariable>
                    <flowable:errorCodeVariable>shellCode</flowable:errorCodeVariable>
                </extensionElements>
            </serviceTask>
            <sequenceFlow id="flow2" sourceRef="shellTask" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#;

    let (process_engine, process_instance_id) = deploy_and_start(xml);

    let runtime_store = process_engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let execution = runtime_store
        .find_execution(&process_instance_id, &mut session)
        .expect("execution should exist");

    let shell_out = execution
        .persistent_process_variable("shellOut")
        .expect("Java outputVariable name must be populated");
    assert!(
        shell_out
            .as_str()
            .is_some_and(|s| s.contains("Portable")),
        "outputVariable should capture stdout, got {shell_out}"
    );

    let shell_code = execution
        .persistent_process_variable("shellCode")
        .expect("Java errorCodeVariable name must be populated");
    assert_eq!(shell_code, json!(0), "errorCodeVariable should be exit code 0");
}

/// P51 S3 — Java ShellActivityBehavior.java:108-118 redirectErrorStream + env.clear.
#[test]
fn test_shell_task_redirect_error_merges_stderr_into_output_variable() {
    // Write pure stderr via PowerShell (cmd `echo x 1>&2` is unreliable under Command::output).
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
                 xmlns:flowable="http://flowable.org/bpmn"
                 targetNamespace="Examples">
        <process id="shellRedirectProcess" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="flow1" sourceRef="start" targetRef="shellTask" />
            <serviceTask id="shellTask" flowable:type="shell" flowable:resultVariableName="shellResult">
                <extensionElements>
                    <flowable:command>powershell</flowable:command>
                    <flowable:arg>-NoProfile</flowable:arg>
                    <flowable:arg>-Command</flowable:arg>
                    <flowable:arg>[Console]::Error.WriteLine('ERRMSG')</flowable:arg>
                    <flowable:redirectError>true</flowable:redirectError>
                    <flowable:outputVariable>mergedOut</flowable:outputVariable>
                </extensionElements>
            </serviceTask>
            <sequenceFlow id="flow2" sourceRef="shellTask" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#;

    let (process_engine, process_instance_id) = deploy_and_start(xml);

    let runtime_store = process_engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let execution = runtime_store
        .find_execution(&process_instance_id, &mut session)
        .expect("execution should exist");

    let shell_result = execution
        .persistent_process_variable("shellResult")
        .expect("shellResult should be set");
    assert_eq!(shell_result["redirectError"], json!(true));

    let merged = execution
        .persistent_process_variable("mergedOut")
        .expect("outputVariable should receive merged stream");
    assert!(
        merged.as_str().is_some_and(|s| s.contains("ERRMSG")),
        "redirectError must merge stderr into outputVariable, got {merged}"
    );
}

#[test]
fn test_shell_task_clean_env_flag_is_recorded_and_executes() {
    // Absolute path required: cleanEnv clears PATH, so bare `cmd` cannot be resolved.
    let cmd_path = std::env::var("COMSPEC").unwrap_or_else(|_| r"C:\Windows\System32\cmd.exe".into());
    let xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
                 xmlns:flowable="http://flowable.org/bpmn"
                 targetNamespace="Examples">
        <process id="shellCleanEnvProcess" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="flow1" sourceRef="start" targetRef="shellTask" />
            <serviceTask id="shellTask" flowable:type="shell" flowable:resultVariableName="shellResult">
                <extensionElements>
                    <flowable:command>{}</flowable:command>
                    <flowable:arg>/c</flowable:arg>
                    <flowable:arg>echo cleaned</flowable:arg>
                    <flowable:cleanEnv>true</flowable:cleanEnv>
                </extensionElements>
            </serviceTask>
            <sequenceFlow id="flow2" sourceRef="shellTask" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#,
        cmd_path.replace('\\', "\\\\")
    );

    let (process_engine, process_instance_id) = deploy_and_start(&xml);

    let runtime_store = process_engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let execution = runtime_store
        .find_execution(&process_instance_id, &mut session)
        .expect("execution should exist");

    let shell_result = execution
        .persistent_process_variable("shellResult")
        .expect("shellResult should be set");
    assert_eq!(shell_result["cleanEnv"], json!(true));
    assert_eq!(shell_result["exitCode"], json!(0));
    let stdout = shell_result["stdout"].as_str().unwrap_or("");
    assert!(
        stdout.contains("cleaned"),
        "cleanEnv must still allow the shell command to run, got {stdout}"
    );
}

#[test]
fn test_http_task_makes_request_and_captures_response() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
                 xmlns:flowable="http://flowable.org/bpmn"
                 targetNamespace="Examples">
        <process id="httpProcess" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="flow1" sourceRef="start" targetRef="httpTask" />
            <serviceTask id="httpTask" flowable:type="http" flowable:resultVariableName="httpResult">
                <extensionElements>
                    <flowable:requestMethod>GET</flowable:requestMethod>
                    <flowable:requestUrl>https://api.example.com/data</flowable:requestUrl>
                </extensionElements>
            </serviceTask>
            <sequenceFlow id="flow2" sourceRef="httpTask" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#;

    let (process_engine, process_instance_id) = deploy_and_start(xml);

    let runtime_store = process_engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let execution = runtime_store
        .find_execution(&process_instance_id, &mut session)
        .expect("execution should exist");

    let http_result = execution
        .persistent_process_variable("httpResult")
        .expect("httpResult variable should be set");

    let result_obj = http_result
        .as_object()
        .expect("httpResult should be an object");
    assert!(
        result_obj.contains_key("service"),
        "Expected service field in response"
    );
    assert_eq!(
        result_obj["service"],
        json!("http"),
        "Expected service to be 'http'"
    );
    assert!(
        result_obj.contains_key("response"),
        "Expected response field"
    );
    assert!(
        result_obj["response"].get("statusCode").is_some(),
        "Expected statusCode in response"
    );
}

#[test]
fn test_http_task_post_with_body() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
                 xmlns:flowable="http://flowable.org/bpmn"
                 targetNamespace="Examples">
        <process id="httpPostProcess" isExecutable="true">
            <startEvent id="start" />
            <sequenceFlow id="flow1" sourceRef="start" targetRef="httpTask" />
            <serviceTask id="httpTask" flowable:type="http" flowable:resultVariableName="httpResult">
                <extensionElements>
                    <flowable:requestMethod>POST</flowable:requestMethod>
                    <flowable:requestUrl>https://api.example.com/submit</flowable:requestUrl>
                    <flowable:requestBody>{"key":"value"}</flowable:requestBody>
                </extensionElements>
            </serviceTask>
            <sequenceFlow id="flow2" sourceRef="httpTask" targetRef="end" />
            <endEvent id="end" />
        </process>
    </definitions>"#;

    let (process_engine, process_instance_id) = deploy_and_start(xml);

    let runtime_store = process_engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let execution = runtime_store
        .find_execution(&process_instance_id, &mut session)
        .expect("execution should exist");

    let http_result = execution
        .persistent_process_variable("httpResult")
        .expect("httpResult variable should be set");

    let result_obj = http_result
        .as_object()
        .expect("httpResult should be an object");
    assert!(
        result_obj.contains_key("response"),
        "Expected response field"
    );
    assert!(
        result_obj["response"].get("statusCode").is_some(),
        "Expected statusCode in response"
    );

    let request = result_obj["request"]
        .as_object()
        .expect("Expected request field");
    assert_eq!(request["method"], json!("POST"), "Expected POST method");
}
