use flowable_engine::el::expression::{Expression, SimpleExpression};
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::runtime::execution::Execution;
use flowable_engine::service::config::{HistoryLevel, ProcessEngineConfiguration};
use std::time::{Duration, Instant};

const BPMN_LINEAR: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="bench">
    <process id="linearProcess" isExecutable="true">
        <startEvent id="start" />
        <sequenceFlow id="f1" sourceRef="start" targetRef="task1" />
        <userTask id="task1" name="Task 1" assignee="user1" />
        <sequenceFlow id="f2" sourceRef="task1" targetRef="end" />
        <endEvent id="end" />
    </process>
</definitions>"#;

const BPMN_COMPLEX: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="bench">
    <process id="complexProcess" isExecutable="true">
        <startEvent id="start" />
        <sequenceFlow id="f1" sourceRef="start" targetRef="gw1" />
        <parallelGateway id="gw1" />
        <sequenceFlow id="f2" sourceRef="gw1" targetRef="task1" />
        <sequenceFlow id="f3" sourceRef="gw1" targetRef="task2" />
        <sequenceFlow id="f4" sourceRef="gw1" targetRef="task3" />
        <userTask id="task1" name="Task 1" assignee="${assignee1}" />
        <userTask id="task2" name="Task 2" assignee="${assignee2}" />
        <userTask id="task3" name="Task 3" assignee="${assignee3}" />
        <sequenceFlow id="f5" sourceRef="task1" targetRef="gw2" />
        <sequenceFlow id="f6" sourceRef="task2" targetRef="gw2" />
        <sequenceFlow id="f7" sourceRef="task3" targetRef="gw2" />
        <parallelGateway id="gw2" />
        <sequenceFlow id="f8" sourceRef="gw2" targetRef="task4" />
        <userTask id="task4" name="Final Task" assignee="user4" />
        <sequenceFlow id="f9" sourceRef="task4" targetRef="end" />
        <endEvent id="end" />
    </process>
</definitions>"#;

struct BenchResult {
    name: &'static str,
    iterations: usize,
    total: Duration,
    avg: Duration,
    min: Duration,
    max: Duration,
    throughput_per_sec: f64,
}

impl BenchResult {
    fn print(&self) {
        println!(
            "{:<40} | {:>6} | {:>10.2?} | {:>10.2?} | {:>10.2?} | {:>10.2?} | {:>12.1}",
            self.name,
            self.iterations,
            self.total,
            self.avg,
            self.min,
            self.max,
            self.throughput_per_sec,
        );
    }
}

fn bench<F>(name: &'static str, iterations: usize, mut f: F) -> BenchResult
where
    F: FnMut(),
{
    let start = Instant::now();
    let mut min = Duration::MAX;
    let mut max = Duration::ZERO;

    for _ in 0..iterations {
        let iter_start = Instant::now();
        f();
        let elapsed = iter_start.elapsed();
        if elapsed < min {
            min = elapsed;
        }
        if elapsed > max {
            max = elapsed;
        }
    }

    let total = start.elapsed();
    let avg = total / iterations as u32;
    let throughput = iterations as f64 / total.as_secs_f64();

    BenchResult {
        name,
        iterations,
        total,
        avg,
        min,
        max,
        throughput_per_sec: throughput,
    }
}

fn bench_engine_new() -> BenchResult {
    bench("engine new (in-memory)", 100, || {
        let _engine = ProcessEngine::new_with_memory_backend("bench".to_string());
    })
}

fn bench_deploy_bpmn() -> BenchResult {
    bench("deploy BPMN linear", 200, || {
        let engine = ProcessEngine::new_with_memory_backend("bench".to_string());
        let deploy_builder = engine
            .get_repository_service()
            .create_deployment()
            .add_string("linear.bpmn".to_string(), BPMN_LINEAR.to_string());
        engine
            .get_repository_service()
            .deploy(deploy_builder)
            .unwrap();
    })
}

fn bench_start_process() -> BenchResult {
    let engine = ProcessEngine::new_with_memory_backend("bench".to_string());
    let deploy_builder = engine
        .get_repository_service()
        .create_deployment()
        .add_string("linear.bpmn".to_string(), BPMN_LINEAR.to_string());
    engine
        .get_repository_service()
        .deploy(deploy_builder)
        .unwrap();
    let def_id = engine
        .get_repository_service()
        .get_process_definition_ids()
        .unwrap()[0]
        .clone();

    bench("start process instance", 500, || {
        let builder = engine
            .get_runtime_service()
            .create_process_instance_builder()
            .process_definition_id(def_id.clone())
            .variable(
                "myVar".to_string(),
                serde_json::Value::String("bench".to_string()),
            );
        let _pi = engine
            .get_runtime_service()
            .start_process_instance(builder)
            .unwrap();
    })
}

fn bench_full_process_lifecycle() -> BenchResult {
    bench("full process lifecycle (start+complete)", 200, || {
        let engine = ProcessEngine::new_with_memory_backend("bench".to_string());
        let deploy_builder = engine
            .get_repository_service()
            .create_deployment()
            .add_string("linear.bpmn".to_string(), BPMN_LINEAR.to_string());
        engine
            .get_repository_service()
            .deploy(deploy_builder)
            .unwrap();
        let def_id = engine
            .get_repository_service()
            .get_process_definition_ids()
            .unwrap()[0]
            .clone();
        let builder = engine
            .get_runtime_service()
            .create_process_instance_builder()
            .process_definition_id(def_id);
        let pi = engine
            .get_runtime_service()
            .start_process_instance(builder)
            .unwrap();
        let tasks = engine
            .get_task_service()
            .get_tasks_by_process_instance_id(pi.id)
            .unwrap();
        engine
            .get_task_service()
            .complete_task_by_id(tasks[0].id.clone())
            .unwrap();
    })
}

fn bench_complex_process_lifecycle() -> BenchResult {
    bench("complex process lifecycle (4 tasks)", 100, || {
        let engine = ProcessEngine::new_with_memory_backend("bench".to_string());
        let deploy_builder = engine
            .get_repository_service()
            .create_deployment()
            .add_string("complex.bpmn".to_string(), BPMN_COMPLEX.to_string());
        engine
            .get_repository_service()
            .deploy(deploy_builder)
            .unwrap();
        let def_id = engine
            .get_repository_service()
            .get_process_definition_ids()
            .unwrap()[0]
            .clone();
        let builder = engine
            .get_runtime_service()
            .create_process_instance_builder()
            .process_definition_id(def_id)
            .variable(
                "assignee1".to_string(),
                serde_json::Value::String("u1".to_string()),
            )
            .variable(
                "assignee2".to_string(),
                serde_json::Value::String("u2".to_string()),
            )
            .variable(
                "assignee3".to_string(),
                serde_json::Value::String("u3".to_string()),
            );
        let pi = engine
            .get_runtime_service()
            .start_process_instance(builder)
            .unwrap();
        let pi_id = pi.id;
        let tasks = engine
            .get_task_service()
            .get_tasks_by_process_instance_id(pi_id.clone())
            .unwrap();
        // Complete all parallel tasks
        for t in &tasks {
            engine
                .get_task_service()
                .complete_task_by_id(t.id.clone())
                .unwrap();
        }
        // Get and complete final task
        let tasks2 = engine
            .get_task_service()
            .get_tasks_by_process_instance_id(pi_id)
            .unwrap();
        for t in &tasks2 {
            engine
                .get_task_service()
                .complete_task_by_id(t.id.clone())
                .unwrap();
        }
    })
}

fn bench_expression_eval() -> BenchResult {
    let engine = ProcessEngine::new_with_memory_backend("bench".to_string());
    let deploy_builder = engine
        .get_repository_service()
        .create_deployment()
        .add_string("complex.bpmn".to_string(), BPMN_COMPLEX.to_string());
    engine
        .get_repository_service()
        .deploy(deploy_builder)
        .unwrap();
    let def_id = engine
        .get_repository_service()
        .get_process_definition_ids()
        .unwrap()[0]
        .clone();

    bench("expression evaluation (10 vars)", 500, || {
        let mut builder = engine
            .get_runtime_service()
            .create_process_instance_builder()
            .process_definition_id(def_id.clone());
        for i in 0..10 {
            builder = builder.variable(
                format!("assignee{}", i + 1),
                serde_json::Value::String(format!("user{}", i + 1)),
            );
        }
        let _pi = engine
            .get_runtime_service()
            .start_process_instance(builder)
            .unwrap();
    })
}

fn bench_history_recording() -> BenchResult {
    bench("history recording (task created)", 200, || {
        let engine = ProcessEngine::new_with_memory_backend("bench".to_string());
        let deploy_builder = engine
            .get_repository_service()
            .create_deployment()
            .add_string("linear.bpmn".to_string(), BPMN_LINEAR.to_string());
        engine
            .get_repository_service()
            .deploy(deploy_builder)
            .unwrap();
        let def_id = engine
            .get_repository_service()
            .get_process_definition_ids()
            .unwrap()[0]
            .clone();
        let builder = engine
            .get_runtime_service()
            .create_process_instance_builder()
            .process_definition_id(def_id);
        let pi = engine
            .get_runtime_service()
            .start_process_instance(builder)
            .unwrap();
        // history is recorded during start and task creation
        let _tasks = engine
            .get_task_service()
            .get_tasks_by_process_instance_id(pi.id)
            .unwrap();
    })
}

fn bench_deploy_complex() -> BenchResult {
    bench("deploy BPMN complex", 100, || {
        let engine = ProcessEngine::new_with_memory_backend("bench".to_string());
        let deploy_builder = engine
            .get_repository_service()
            .create_deployment()
            .add_string("complex.bpmn".to_string(), BPMN_COMPLEX.to_string());
        engine
            .get_repository_service()
            .deploy(deploy_builder)
            .unwrap();
    })
}

fn bench_timer_job_acquisition() -> BenchResult {
    bench("timer job acquisition (100 candidates)", 100, || {
        let engine = ProcessEngine::new("bench".to_string());
        let store = engine.get_runtime_store();
        let mut session = store.create_session().unwrap();
        let now = chrono::Utc::now().timestamp_millis();

        // Insert 100 timer jobs
        for i in 0..100 {
            let job = flowable_engine::persistence::runtime_store::RuntimeTimerJobState {
                timer_job_id: format!("timer_{}", i),
                job_state: Some("timer".to_string()),
                due_time: Some(now - 1000), // all due
                retries: Some(3),
                lock_owner: None,
                lock_time: None,
                process_instance_id: "pi1".to_string(),
                execution_id: "e1".to_string(),
                activity_id: "act1".to_string(),
                is_boundary: false,
                attached_activity_id: None,
                cancel_activity: false,
                time_duration: None,
                time_date: None,
                time_cycle: None,
                lock_expiration_time: None,
                error_message: None,
                error_details: None,
                category: None,
                ..Default::default()
            };
            store.insert_timer_job_state(&job, &mut session);
        }

        // Flush pending writes so raw pool queries see the data
        session.flush().unwrap();

        let (acquired, _, _) =
            store.acquire_due_timer_jobs("bench-worker", now, 30000, &mut session);
        assert!(acquired.len() == 100);
    })
}

fn bench_pure_expression_eval() -> BenchResult {
    // Isolated expression engine benchmark — no DB, no engine, no BPMN parsing.
    // Measures raw get_value() throughput for 5 expression types, 10000 evals each.
    let expressions = [
        ("${assignee1}", "variable lookup"),
        ("${approved == true}", "bool comparison"),
        ("${count > 5}", "numeric comparison"),
        ("${a && b || c}", "logical combo"),
        ("${name + '_' + id}", "string concat"),
    ];

    let mut execution = Execution {
        id: "exec1".to_string(),
        ..Default::default()
    };
    execution.variables.insert(
        "assignee1".to_string(),
        serde_json::Value::String("user1".to_string()),
    );
    execution
        .variables
        .insert("approved".to_string(), serde_json::Value::Bool(true));
    execution.variables.insert(
        "count".to_string(),
        serde_json::Value::Number(serde_json::Number::from(42)),
    );
    execution
        .variables
        .insert("a".to_string(), serde_json::Value::Bool(true));
    execution
        .variables
        .insert("b".to_string(), serde_json::Value::Bool(false));
    execution
        .variables
        .insert("c".to_string(), serde_json::Value::Bool(true));
    execution.variables.insert(
        "name".to_string(),
        serde_json::Value::String("test".to_string()),
    );
    execution.variables.insert(
        "id".to_string(),
        serde_json::Value::String("123".to_string()),
    );

    // Pre-create expressions (compilation happens once, not measured)
    let compiled: Vec<SimpleExpression> = expressions
        .iter()
        .map(|(text, _)| SimpleExpression::new(text.to_string()))
        .collect();

    // Warmup: trigger OnceLock compilation
    for expr in &compiled {
        let _ = expr.get_value(&execution);
    }

    bench("pure expression eval (5 types x 10000)", 1, || {
        for _ in 0..10000 {
            for expr in &compiled {
                let _ = expr.get_value(&execution);
            }
        }
    })
}

/// Decomposition experiment: measure complex process start with different history levels.
/// This isolates the history recording cost from the core runtime cost.
fn bench_complex_start_history_decomp() -> Vec<BenchResult> {
    let mut results = Vec::new();

    for (level, label) in [
        (HistoryLevel::None, "complex start (history=None)"),
        (HistoryLevel::Audit, "complex start (history=Audit)"),
        (HistoryLevel::Full, "complex start (history=Full)"),
    ] {
        let config = ProcessEngineConfiguration {
            history_level: level,
            ..Default::default()
        };
        let engine = ProcessEngine::new_with_config("bench".to_string(), config);
        let deploy_builder = engine
            .get_repository_service()
            .create_deployment()
            .add_string("complex.bpmn".to_string(), BPMN_COMPLEX.to_string());
        engine
            .get_repository_service()
            .deploy(deploy_builder)
            .unwrap();
        let def_id = engine
            .get_repository_service()
            .get_process_definition_ids()
            .unwrap()[0]
            .clone();

        let result = bench(label, 500, || {
            let mut builder = engine
                .get_runtime_service()
                .create_process_instance_builder()
                .process_definition_id(def_id.clone());
            for i in 0..10 {
                builder = builder.variable(
                    format!("assignee{}", i + 1),
                    serde_json::Value::String(format!("user{}", i + 1)),
                );
            }
            let _pi = engine
                .get_runtime_service()
                .start_process_instance(builder)
                .unwrap();
        });
        results.push(result);
    }

    results
}

fn main() {
    println!();
    println!("=== Flowable Rust Engine Benchmark ===");
    println!(
        "Platform: {} {}",
        std::env::consts::OS,
        std::env::consts::ARCH
    );
    println!();

    // Warmup
    println!("[Warmup] Running 3 warmup iterations...");
    for _ in 0..3 {
        let engine = ProcessEngine::new_with_memory_backend("warmup".to_string());
        let deploy_builder = engine
            .get_repository_service()
            .create_deployment()
            .add_string("linear.bpmn".to_string(), BPMN_LINEAR.to_string());
        engine
            .get_repository_service()
            .deploy(deploy_builder)
            .unwrap();
        let def_id = engine
            .get_repository_service()
            .get_process_definition_ids()
            .unwrap()[0]
            .clone();
        let builder = engine
            .get_runtime_service()
            .create_process_instance_builder()
            .process_definition_id(def_id);
        let pi = engine
            .get_runtime_service()
            .start_process_instance(builder)
            .unwrap();
        let tasks = engine
            .get_task_service()
            .get_tasks_by_process_instance_id(pi.id)
            .unwrap();
        engine
            .get_task_service()
            .complete_task_by_id(tasks[0].id.clone())
            .unwrap();
    }
    println!("[Warmup] Done.\n");

    println!(
        "{:<40} | {:>6} | {:>10} | {:>10} | {:>10} | {:>10} | {:>12}",
        "Benchmark", "Iters", "Total", "Avg", "Min", "Max", "Throughput/s"
    );
    println!("{}", "-".repeat(120));

    let results = vec![
        bench_engine_new(),
        bench_deploy_bpmn(),
        bench_deploy_complex(),
        bench_start_process(),
        bench_expression_eval(),
        bench_pure_expression_eval(),
        bench_full_process_lifecycle(),
        bench_complex_process_lifecycle(),
        bench_history_recording(),
        bench_timer_job_acquisition(),
    ];

    for r in &results {
        r.print();
    }

    println!();
    println!("=== Decomposition: History Level Impact ===");
    let decomp_results = bench_complex_start_history_decomp();
    for r in &decomp_results {
        r.print();
    }

    println!();
    println!("=== Summary ===");
    println!(
        "Total time: {:.2?}",
        results.iter().map(|r| r.total).sum::<Duration>()
    );
    println!(
        "Total iterations: {}",
        results.iter().map(|r| r.iterations).sum::<usize>()
    );
}
