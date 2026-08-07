use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::error::FlowableError;
use flowable_engine::service::config::ProcessEngineConfiguration;

fn secure_script_engine() -> ProcessEngine {
    let config = ProcessEngineConfiguration {
        enable_secure_scripting: true,
        supported_script_languages: vec!["javascript".to_string()],
        ..Default::default()
    };
    ProcessEngine::new_with_config("default".to_string(), config)
}

fn deploy_skip_expression_process(process_engine: &ProcessEngine, process_id: &str) -> String {
    let repository_service = process_engine.get_repository_service();
    let xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
                 xmlns:flowable="http://flowable.org/bpmn"
                 targetNamespace="Examples">
        <process id="{process_id}" name="Script Task Skip Expression Process">
            <startEvent id="startEvent1" />
            <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="scriptTask1" />
            <scriptTask id="scriptTask1" name="Maybe Execute Script" scriptFormat="javascript" flowable:skipExpression="${{skipScript}}">
                <script>var ran = 1;</script>
            </scriptTask>
            <sequenceFlow id="flow2" sourceRef="scriptTask1" targetRef="waitTask" />
            <userTask id="waitTask" name="Wait" />
            <sequenceFlow id="flow3" sourceRef="waitTask" targetRef="endEvent1" />
            <endEvent id="endEvent1" />
        </process>
    </definitions>"#
    );

    let builder = repository_service
        .create_deployment()
        .name(format!("{process_id} Deployment"))
        .add_string(format!("{process_id}.bpmn20.xml"), xml);

    repository_service.deploy(builder).unwrap();
    repository_service.get_process_definition_ids().unwrap()[0].clone()
}

/// Verifies that a script task with secure scripting enabled executes through
/// the secure runtime (not a pass-through) and the process completes.
#[test]
fn script_task_executes_through_secure_runtime_to_end() {
    let config = ProcessEngineConfiguration {
        enable_secure_scripting: true,
        supported_script_languages: vec!["javascript".to_string()],
        ..Default::default()
    };
    let process_engine = ProcessEngine::new_with_config("default".to_string(), config);

    let repository_service = process_engine.get_repository_service();
    let runtime_service = process_engine.get_runtime_service();
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="scriptTaskProcess" name="Script Task Process">
            <startEvent id="startEvent1" />
            <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="scriptTask1" />
            <scriptTask id="scriptTask1" name="Execute Script" scriptFormat="javascript">
                <script>var result = 42;</script>
            </scriptTask>
            <sequenceFlow id="flow2" sourceRef="scriptTask1" targetRef="endEvent1" />
            <endEvent id="endEvent1" />
        </process>
    </definitions>"#;

    let builder = repository_service
        .create_deployment()
        .name("Script Task Deployment".to_string())
        .add_string("scriptTaskProcess.bpmn20.xml".to_string(), xml.to_string());

    repository_service.deploy(builder).unwrap();
    let process_definition_id = repository_service.get_process_definition_ids().unwrap()[0].clone();

    let builder = runtime_service
        .create_process_instance_builder()
        .process_definition_id(process_definition_id)
        .name("Script Task Instance".to_string());

    let process_instance = runtime_service.start_process_instance(builder).unwrap();

    let runtime_store = process_engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let stored_pi = runtime_store
        .find_process_instance(&process_instance.id, &mut session)
        .expect("Process instance should be in runtime store");
    session.rollback().unwrap();
    assert!(
        stored_pi.is_ended,
        "Process instance should be ended after script task execution through secure runtime"
    );
}

#[test]
fn script_task_skip_expression_true_skips_script_execution_and_takes_outgoing_flow() {
    let process_engine = secure_script_engine();
    let process_definition_id =
        deploy_skip_expression_process(&process_engine, "scriptSkipTrueProcess");
    let runtime_service = process_engine.get_runtime_service();

    let process_instance = runtime_service
        .start_process_instance(
            runtime_service
                .create_process_instance_builder()
                .process_definition_id(process_definition_id)
                .variable(
                    "_FLOWABLE_SKIP_EXPRESSION_ENABLED".to_string(),
                    serde_json::Value::Bool(true),
                )
                .variable("skipScript".to_string(), serde_json::Value::Bool(true)),
        )
        .unwrap();

    let runtime_store = process_engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let executions = runtime_store.snapshot_executions(&mut session);
    session.rollback().unwrap();
    let execution = executions
        .get(&process_instance.id)
        .expect("root execution should wait after the skipped script task");

    assert_eq!(execution.activity_id.as_deref(), Some("waitTask"));
    assert!(
        !execution.variables.contains_key("ran"),
        "skipExpression=true should skip script execution and not auto-store script variables"
    );
}

#[test]
fn script_task_skip_expression_is_ignored_until_enabled() {
    let process_engine = secure_script_engine();
    let process_definition_id =
        deploy_skip_expression_process(&process_engine, "scriptSkipDisabledProcess");
    let runtime_service = process_engine.get_runtime_service();

    let process_instance = runtime_service
        .start_process_instance(
            runtime_service
                .create_process_instance_builder()
                .process_definition_id(process_definition_id)
                .variable("skipScript".to_string(), serde_json::Value::Bool(true)),
        )
        .unwrap();

    let runtime_store = process_engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let executions = runtime_store.snapshot_executions(&mut session);
    session.rollback().unwrap();
    let execution = executions
        .get(&process_instance.id)
        .expect("root execution should wait after script execution");

    assert_eq!(execution.activity_id.as_deref(), Some("waitTask"));
    assert_eq!(
        execution.variables.get("ran"),
        Some(&serde_json::Value::Number(1.into())),
        "skipExpression semantics require the enable variable before skipping"
    );
}

#[test]
fn script_task_skip_expression_false_executes_script_normally() {
    let process_engine = secure_script_engine();
    let process_definition_id =
        deploy_skip_expression_process(&process_engine, "scriptSkipFalseProcess");
    let runtime_service = process_engine.get_runtime_service();

    let process_instance = runtime_service
        .start_process_instance(
            runtime_service
                .create_process_instance_builder()
                .process_definition_id(process_definition_id)
                .variable(
                    "_FLOWABLE_SKIP_EXPRESSION_ENABLED".to_string(),
                    serde_json::Value::Bool(true),
                )
                .variable("skipScript".to_string(), serde_json::Value::Bool(false)),
        )
        .unwrap();

    let runtime_store = process_engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let executions = runtime_store.snapshot_executions(&mut session);
    session.rollback().unwrap();
    let execution = executions
        .get(&process_instance.id)
        .expect("root execution should wait after the executed script task");

    assert_eq!(execution.activity_id.as_deref(), Some("waitTask"));
    assert_eq!(
        execution.variables.get("ran"),
        Some(&serde_json::Value::Number(1.into()))
    );
}

#[test]
fn script_task_skip_expression_requires_boolean_result() {
    let process_engine = secure_script_engine();
    let process_definition_id =
        deploy_skip_expression_process(&process_engine, "scriptSkipInvalidProcess");
    let runtime_service = process_engine.get_runtime_service();

    let result = runtime_service.start_process_instance(
        runtime_service
            .create_process_instance_builder()
            .process_definition_id(process_definition_id)
            .variable(
                "_FLOWABLE_SKIP_EXPRESSION_ENABLED".to_string(),
                serde_json::Value::Bool(true),
            )
            .variable(
                "skipScript".to_string(),
                serde_json::Value::String("yes".to_string()),
            ),
    );

    match result {
        Err(FlowableError::ExecutionError(message)) => {
            assert!(
                message.contains("skipExpression")
                    && message.contains("scriptTask1")
                    && message.contains("boolean"),
                "expected structured skipExpression boolean error, got: {message}"
            );
        }
        other => panic!("expected skipExpression execution error, got {other:?}"),
    }
}

/// Verifies that a script task fails with structured error when secure scripting is disabled.
#[test]
fn script_task_rejects_when_secure_scripting_disabled() {
    let config = flowable_engine::service::config::ProcessEngineConfiguration::default();
    let process_engine = ProcessEngine::new_with_config("default".to_string(), config);

    let repository_service = process_engine.get_repository_service();
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="scriptRejectProcess" name="Script Reject Process">
            <startEvent id="startEvent1" />
            <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="scriptTask1" />
            <scriptTask id="scriptTask1" name="Execute Script" scriptFormat="javascript">
                <script>var x = 1;</script>
            </scriptTask>
            <sequenceFlow id="flow2" sourceRef="scriptTask1" targetRef="endEvent1" />
            <endEvent id="endEvent1" />
        </process>
    </definitions>"#;

    let builder = repository_service
        .create_deployment()
        .name("Script Reject Deployment".to_string())
        .add_string(
            "scriptRejectProcess.bpmn20.xml".to_string(),
            xml.to_string(),
        );

    // Deploy-time validation rejects script tasks when secure scripting is disabled
    let result = repository_service.deploy(builder);
    assert!(
        result.is_err(),
        "Script task should fail at deploy time when secure scripting is disabled"
    );
}
