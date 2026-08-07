//! Bounded async future delegate + WaitForFuture agenda path.

use flowable_engine::bpmn::behavior::async_delegate_activity_behavior::{
    AsyncLocalServiceTaskDelegate, AsyncLocalServiceTaskDelegateContext,
    AsyncLocalServiceTaskDelegateRegistry,
};
use flowable_engine::engine::async_task_executor::{AsyncTaskExecutor, AsyncTaskExecutorConfig};
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::error::FlowableError;
use flowable_engine::service::config::ProcessEngineConfiguration;
use serde_json::{Value, json};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

const ASYNC_DELEGATE_NAME: &str = "asyncGreetingDelegate";

const ASYNC_DELEGATE_PROCESS_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             xmlns:flowable="http://flowable.org/bpmn"
             targetNamespace="Examples">
    <process id="asyncDelegateProcess" name="Async Delegate Process" isExecutable="true">
        <startEvent id="startEvent1" />
        <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="asyncDelegateTask" />
        <serviceTask id="asyncDelegateTask"
                     name="Async Delegate Task"
                     flowable:class="asyncGreetingDelegate"
                     flowable:resultVariableName="asyncResult">
            <extensionElements>
                <flowable:field name="greeting" stringValue="hello-async" />
            </extensionElements>
        </serviceTask>
        <sequenceFlow id="flow2" sourceRef="asyncDelegateTask" targetRef="endEvent1" />
        <endEvent id="endEvent1" />
    </process>
</definitions>"#;

const SYNC_USER_TASK_PROCESS_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
             targetNamespace="Examples">
    <process id="waitUserProcess" name="Wait User Process" isExecutable="true">
        <startEvent id="startEvent1" />
        <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="userTask1" />
        <userTask id="userTask1" name="Hold" />
        <sequenceFlow id="flow2" sourceRef="userTask1" targetRef="endEvent1" />
        <endEvent id="endEvent1" />
    </process>
</definitions>"#;

struct SlowAsyncDelegate {
    started: Arc<AtomicBool>,
    completed: Arc<AtomicBool>,
    on_background: Arc<AtomicBool>,
    delay_ms: u64,
    main_thread_id: thread::ThreadId,
}

impl AsyncLocalServiceTaskDelegate for SlowAsyncDelegate {
    fn run(&self, context: &AsyncLocalServiceTaskDelegateContext) -> Result<Value, FlowableError> {
        self.started.store(true, Ordering::SeqCst);
        if thread::current().id() != self.main_thread_id {
            self.on_background.store(true, Ordering::SeqCst);
        }
        thread::sleep(Duration::from_millis(self.delay_ms));
        let greeting = context
            .fields
            .get("greeting")
            .and_then(|v| v.as_str())
            .unwrap_or("default");
        self.completed.store(true, Ordering::SeqCst);
        Ok(json!({
            "greeting": greeting,
            "serviceTaskId": context.service_task_id,
            "executionId": context.execution_id,
        }))
    }
}

struct CountingAsyncDelegate {
    invocations: Arc<AtomicUsize>,
}

impl AsyncLocalServiceTaskDelegate for CountingAsyncDelegate {
    fn run(&self, context: &AsyncLocalServiceTaskDelegateContext) -> Result<Value, FlowableError> {
        let n = self.invocations.fetch_add(1, Ordering::SeqCst) + 1;
        Ok(json!({
            "count": n,
            "executionId": context.execution_id,
        }))
    }
}

fn engine_with_async_delegate(
    name: &str,
    delegate: Arc<dyn AsyncLocalServiceTaskDelegate>,
    with_executor: bool,
) -> ProcessEngine {
    let mut async_registry = AsyncLocalServiceTaskDelegateRegistry::new();
    async_registry.register(ASYNC_DELEGATE_NAME, delegate);

    let mut config = ProcessEngineConfiguration::default();
    config.async_service_task_delegate_registry = Some(async_registry);
    if with_executor {
        let pool = AsyncTaskExecutor::new(AsyncTaskExecutorConfig {
            pool_size: 2,
            queue_size: 64,
            keep_alive_ms: 5_000,
            thread_name_prefix: "test-async-delegate".to_string(),
            ..AsyncTaskExecutorConfig::default()
        });
        config.future_task_executor = Some(Arc::new(Mutex::new(Some(pool))));
    }
    ProcessEngine::new_with_config(name.to_string(), config)
}

#[test]
fn execute_async_delegate_sets_variable_on_process_instance() {
    let invocations = Arc::new(AtomicUsize::new(0));
    let engine = engine_with_async_delegate(
        "async-delegate-api",
        Arc::new(CountingAsyncDelegate {
            invocations: Arc::clone(&invocations),
        }),
        false, // sync fallback — still completes the future path
    );

    let repository_service = engine.get_repository_service();
    let runtime_service = engine.get_runtime_service();

    repository_service
        .deploy(
            repository_service
                .create_deployment()
                .name("wait-user".to_string())
                .add_string(
                    "waitUserProcess.bpmn20.xml".to_string(),
                    SYNC_USER_TASK_PROCESS_XML.to_string(),
                ),
        )
        .expect("deploy");

    let process_definition_id = repository_service.get_process_definition_ids().unwrap()[0].clone();
    let process_instance = runtime_service
        .start_process_instance(
            runtime_service
                .create_process_instance_builder()
                .process_definition_id(process_definition_id),
        )
        .expect("start");

    let result = runtime_service
        .execute_async_delegate(
            process_instance.id.clone(),
            ASYNC_DELEGATE_NAME.to_string(),
            "asyncDelegateResult".to_string(),
        )
        .expect("execute_async_delegate");

    assert_eq!(result.get("count").and_then(|v| v.as_u64()), Some(1));
    assert_eq!(invocations.load(Ordering::SeqCst), 1);

    let vars = runtime_service
        .get_variables(process_instance.id.clone())
        .unwrap();
    assert_eq!(
        vars.get("asyncDelegateResult")
            .and_then(|v| v.get("count"))
            .and_then(|v| v.as_u64()),
        Some(1),
        "result variable must be written on the process instance"
    );
}

#[test]
fn execute_async_delegate_runs_on_thread_pool_when_executor_configured() {
    let started = Arc::new(AtomicBool::new(false));
    let completed = Arc::new(AtomicBool::new(false));
    let on_background = Arc::new(AtomicBool::new(false));

    let engine = engine_with_async_delegate(
        "async-delegate-pool",
        Arc::new(SlowAsyncDelegate {
            started: Arc::clone(&started),
            completed: Arc::clone(&completed),
            on_background: Arc::clone(&on_background),
            delay_ms: 50,
            main_thread_id: thread::current().id(),
        }),
        true,
    );

    let repository_service = engine.get_repository_service();
    let runtime_service = engine.get_runtime_service();

    repository_service
        .deploy(
            repository_service
                .create_deployment()
                .name("wait-user".to_string())
                .add_string(
                    "waitUserProcess.bpmn20.xml".to_string(),
                    SYNC_USER_TASK_PROCESS_XML.to_string(),
                ),
        )
        .expect("deploy");

    let process_definition_id = repository_service.get_process_definition_ids().unwrap()[0].clone();
    let process_instance = runtime_service
        .start_process_instance(
            runtime_service
                .create_process_instance_builder()
                .process_definition_id(process_definition_id),
        )
        .expect("start");

    let result = runtime_service
        .execute_async_delegate_with_fields(
            process_instance.id.clone(),
            ASYNC_DELEGATE_NAME.to_string(),
            "poolResult".to_string(),
            {
                let mut fields = serde_json::Map::new();
                fields.insert("greeting".to_string(), json!("from-pool"));
                fields
            },
        )
        .expect("execute_async_delegate with pool");

    assert!(started.load(Ordering::SeqCst));
    assert!(completed.load(Ordering::SeqCst));
    assert!(
        on_background.load(Ordering::SeqCst),
        "delegate must run on the AsyncTaskExecutor worker thread"
    );
    assert_eq!(
        result.get("greeting").and_then(|v| v.as_str()),
        Some("from-pool")
    );

    let vars = runtime_service
        .get_variables(process_instance.id.clone())
        .unwrap();
    assert_eq!(
        vars.get("poolResult")
            .and_then(|v| v.get("greeting"))
            .and_then(|v| v.as_str()),
        Some("from-pool")
    );
}

#[test]
fn async_service_task_waits_for_future_then_process_completes() {
    let started = Arc::new(AtomicBool::new(false));
    let completed = Arc::new(AtomicBool::new(false));
    let on_background = Arc::new(AtomicBool::new(false));

    let engine = engine_with_async_delegate(
        "async-delegate-process",
        Arc::new(SlowAsyncDelegate {
            started: Arc::clone(&started),
            completed: Arc::clone(&completed),
            on_background: Arc::clone(&on_background),
            delay_ms: 40,
            main_thread_id: thread::current().id(),
        }),
        true,
    );

    let repository_service = engine.get_repository_service();
    let runtime_service = engine.get_runtime_service();

    repository_service
        .deploy(
            repository_service
                .create_deployment()
                .name("async-delegate-process".to_string())
                .add_string(
                    "asyncDelegateProcess.bpmn20.xml".to_string(),
                    ASYNC_DELEGATE_PROCESS_XML.to_string(),
                ),
        )
        .expect("deploy");

    let process_definition_id = repository_service.get_process_definition_ids().unwrap()[0].clone();
    let process_instance = runtime_service
        .start_process_instance(
            runtime_service
                .create_process_instance_builder()
                .process_definition_id(process_definition_id),
        )
        .expect("async service task should wait for future and complete process");

    assert!(
        started.load(Ordering::SeqCst) && completed.load(Ordering::SeqCst),
        "async delegate work must finish before start_process_instance returns"
    );
    assert!(
        on_background.load(Ordering::SeqCst),
        "service-task async path should use the background executor"
    );

    let vars = runtime_service
        .get_variables(process_instance.id.clone())
        .unwrap();
    assert_eq!(
        vars.get("asyncResult")
            .and_then(|v| v.get("greeting"))
            .and_then(|v| v.as_str()),
        Some("hello-async"),
        "WaitForFutureOperation should apply resultVariableName before taking outgoing flows"
    );

    let runtime_store = engine.get_runtime_store();
    let mut session = runtime_store.create_session().unwrap();
    let stored = runtime_store
        .find_process_instance(&process_instance.id, &mut session)
        .expect("process instance should remain queryable");
    assert!(
        stored.is_ended,
        "process must end after the async future completes"
    );
}

#[test]
fn unregistered_async_delegate_fails_clearly_via_runtime_api() {
    let mut config = ProcessEngineConfiguration::default();
    config.async_service_task_delegate_registry =
        Some(AsyncLocalServiceTaskDelegateRegistry::new());
    let engine = ProcessEngine::new_with_config("async-delegate-missing".to_string(), config);

    let repository_service = engine.get_repository_service();
    let runtime_service = engine.get_runtime_service();

    repository_service
        .deploy(
            repository_service
                .create_deployment()
                .name("wait-user".to_string())
                .add_string(
                    "waitUserProcess.bpmn20.xml".to_string(),
                    SYNC_USER_TASK_PROCESS_XML.to_string(),
                ),
        )
        .expect("deploy");

    let process_definition_id = repository_service.get_process_definition_ids().unwrap()[0].clone();
    let process_instance = runtime_service
        .start_process_instance(
            runtime_service
                .create_process_instance_builder()
                .process_definition_id(process_definition_id),
        )
        .expect("start");

    let err = runtime_service
        .execute_async_delegate(
            process_instance.id.clone(),
            "doesNotExist".to_string(),
            "x".to_string(),
        )
        .expect_err("missing async delegate should fail");

    let message = err.to_string();
    assert!(
        message.contains("doesNotExist"),
        "error should name the missing delegate: {message}"
    );
}
