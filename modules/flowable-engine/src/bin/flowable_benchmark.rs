// Benchmark harness. A benchmark that cannot set up, drive or read its fixture
// must not report a number, so every engine call propagates its typed error out
// of `main` (startup-fatal, like Java `buildProcessEngine()` exceptions escaping
// `main`) instead of panicking with an opaque `unwrap` message. The first failing
// iteration aborts the whole run.

use flowable_engine::el::expression::{Expression, SimpleExpression};
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::runtime::execution::Execution;
use flowable_engine::service::config::{HistoryLevel, ProcessEngineConfiguration};
use std::time::{Duration, Instant};

type BenchError = Box<dyn std::error::Error>;
type BenchOutcome<T> = Result<T, BenchError>;

/// Java `List.get(0)` on an empty deployment throws; report it as an error
/// with context rather than an index-out-of-bounds panic.
fn first_definition_id(engine: &ProcessEngine) -> BenchOutcome<String> {
    engine
        .get_repository_service()
        .get_process_definition_ids()?
        .into_iter()
        .next()
        .ok_or_else(|| "deployment produced no process definition".into())
}

fn deploy(engine: &ProcessEngine, name: &str, xml: &str) -> BenchOutcome<()> {
    let repository = engine.get_repository_service();
    let builder = repository
        .create_deployment()
        .add_string(name.to_string(), xml.to_string());
    repository.deploy(builder)?;
    Ok(())
}

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

fn bench<F>(name: &'static str, iterations: usize, mut f: F) -> BenchOutcome<BenchResult>
where
    F: FnMut() -> BenchOutcome<()>,
{
    let start = Instant::now();
    let mut min = Duration::MAX;
    let mut max = Duration::ZERO;

    for iteration in 0..iterations {
        let iter_start = Instant::now();
        f().map_err(|error| format!("benchmark '{name}' iteration {iteration} failed: {error}"))?;
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

    Ok(BenchResult {
        name,
        iterations,
        total,
        avg,
        min,
        max,
        throughput_per_sec: throughput,
    })
}

fn bench_engine_new() -> BenchOutcome<BenchResult> {
    bench("engine new (in-memory)", 100, || {
        let _engine = ProcessEngine::new_with_memory_backend("bench".to_string())?;
        Ok(())
    })
}

fn bench_deploy_bpmn() -> BenchOutcome<BenchResult> {
    bench("deploy BPMN linear", 200, || {
        let engine = ProcessEngine::new_with_memory_backend("bench".to_string())?;
        deploy(&engine, "linear.bpmn", BPMN_LINEAR)
    })
}

fn bench_start_process() -> BenchOutcome<BenchResult> {
    let engine = ProcessEngine::new_with_memory_backend("bench".to_string())?;
    deploy(&engine, "linear.bpmn", BPMN_LINEAR)?;
    let def_id = first_definition_id(&engine)?;

    bench("start process instance", 500, || {
        let builder = engine
            .get_runtime_service()
            .create_process_instance_builder()
            .process_definition_id(def_id.clone())
            .variable(
                "myVar".to_string(),
                serde_json::Value::String("bench".to_string()),
            );
        engine
            .get_runtime_service()
            .start_process_instance(builder)?;
        Ok(())
    })
}

/// Starts the linear process and completes its single user task.
fn run_linear_lifecycle(engine: &ProcessEngine) -> BenchOutcome<()> {
    deploy(engine, "linear.bpmn", BPMN_LINEAR)?;
    let def_id = first_definition_id(engine)?;
    let builder = engine
        .get_runtime_service()
        .create_process_instance_builder()
        .process_definition_id(def_id);
    let pi = engine
        .get_runtime_service()
        .start_process_instance(builder)?;
    let task = engine
        .get_task_service()
        .get_tasks_by_process_instance_id(pi.id)?
        .into_iter()
        .next()
        .ok_or("linear process did not create its user task")?;
    engine.get_task_service().complete_task_by_id(task.id)?;
    Ok(())
}

fn bench_full_process_lifecycle() -> BenchOutcome<BenchResult> {
    bench("full process lifecycle (start+complete)", 200, || {
        let engine = ProcessEngine::new_with_memory_backend("bench".to_string())?;
        run_linear_lifecycle(&engine)
    })
}

fn bench_complex_process_lifecycle() -> BenchOutcome<BenchResult> {
    bench("complex process lifecycle (4 tasks)", 100, || {
        let engine = ProcessEngine::new_with_memory_backend("bench".to_string())?;
        deploy(&engine, "complex.bpmn", BPMN_COMPLEX)?;
        let def_id = first_definition_id(&engine)?;
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
            .start_process_instance(builder)?;
        let pi_id = pi.id;
        let tasks = engine
            .get_task_service()
            .get_tasks_by_process_instance_id(pi_id.clone())?;
        // Complete all parallel tasks
        for t in tasks {
            engine.get_task_service().complete_task_by_id(t.id)?;
        }
        // Get and complete final task
        let tasks2 = engine
            .get_task_service()
            .get_tasks_by_process_instance_id(pi_id)?;
        for t in tasks2 {
            engine.get_task_service().complete_task_by_id(t.id)?;
        }
        Ok(())
    })
}

fn bench_expression_eval() -> BenchOutcome<BenchResult> {
    let engine = ProcessEngine::new_with_memory_backend("bench".to_string())?;
    deploy(&engine, "complex.bpmn", BPMN_COMPLEX)?;
    let def_id = first_definition_id(&engine)?;

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
        engine
            .get_runtime_service()
            .start_process_instance(builder)?;
        Ok(())
    })
}

fn bench_history_recording() -> BenchOutcome<BenchResult> {
    bench("history recording (task created)", 200, || {
        let engine = ProcessEngine::new_with_memory_backend("bench".to_string())?;
        deploy(&engine, "linear.bpmn", BPMN_LINEAR)?;
        let def_id = first_definition_id(&engine)?;
        let builder = engine
            .get_runtime_service()
            .create_process_instance_builder()
            .process_definition_id(def_id);
        let pi = engine
            .get_runtime_service()
            .start_process_instance(builder)?;
        // history is recorded during start and task creation
        engine
            .get_task_service()
            .get_tasks_by_process_instance_id(pi.id)?;
        Ok(())
    })
}

fn bench_deploy_complex() -> BenchOutcome<BenchResult> {
    bench("deploy BPMN complex", 100, || {
        let engine = ProcessEngine::new_with_memory_backend("bench".to_string())?;
        deploy(&engine, "complex.bpmn", BPMN_COMPLEX)
    })
}

fn bench_timer_job_acquisition() -> BenchOutcome<BenchResult> {
    bench("timer job acquisition (100 candidates)", 100, || {
        let engine = ProcessEngine::new("bench".to_string())?;
        let store = engine.get_runtime_store();
        let mut session = store.create_session()?;
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
            store.insert_timer_job_state(&job, &mut session)?;
        }

        // Flush pending writes so raw pool queries see the data
        session.flush()?;

        let (acquired, _, _) =
            store.acquire_due_timer_jobs("bench-worker", now, 30000, &mut session)?;
        if acquired.len() != 100 {
            return Err(format!("expected 100 acquired timer jobs, got {}", acquired.len()).into());
        }
        Ok(())
    })
}

fn bench_pure_expression_eval() -> BenchOutcome<BenchResult> {
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

    // Warmup: trigger OnceLock compilation and verify every expression really
    // evaluates. The lenient `get_value` folds failures into `None`, so an
    // unchecked warmup would happily time a broken expression.
    for (expr, (text, label)) in compiled.iter().zip(expressions.iter()) {
        if expr.get_value(&execution).is_none() {
            return Err(format!("{label} expression '{text}' did not evaluate").into());
        }
    }

    bench("pure expression eval (5 types x 10000)", 1, || {
        for _ in 0..10000 {
            for expr in &compiled {
                // Throughput loop: results were validated during warmup.
                std::hint::black_box(expr.get_value(&execution));
            }
        }
        Ok(())
    })
}

/// Decomposition experiment: measure complex process start with different history levels.
/// This isolates the history recording cost from the core runtime cost.
fn bench_complex_start_history_decomp() -> BenchOutcome<Vec<BenchResult>> {
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
        let engine = ProcessEngine::new_with_config("bench".to_string(), config)?;
        deploy(&engine, "complex.bpmn", BPMN_COMPLEX)?;
        let def_id = first_definition_id(&engine)?;

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
            engine
                .get_runtime_service()
                .start_process_instance(builder)?;
            Ok(())
        })?;
        results.push(result);
    }

    Ok(results)
}

fn main() -> BenchOutcome<()> {
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
        let engine = ProcessEngine::new_with_memory_backend("warmup".to_string())?;
        run_linear_lifecycle(&engine)?;
    }
    println!("[Warmup] Done.\n");

    println!(
        "{:<40} | {:>6} | {:>10} | {:>10} | {:>10} | {:>10} | {:>12}",
        "Benchmark", "Iters", "Total", "Avg", "Min", "Max", "Throughput/s"
    );
    println!("{}", "-".repeat(120));

    let results = vec![
        bench_engine_new()?,
        bench_deploy_bpmn()?,
        bench_deploy_complex()?,
        bench_start_process()?,
        bench_expression_eval()?,
        bench_pure_expression_eval()?,
        bench_full_process_lifecycle()?,
        bench_complex_process_lifecycle()?,
        bench_history_recording()?,
        bench_timer_job_acquisition()?,
    ];

    for r in &results {
        r.print();
    }

    println!();
    println!("=== Decomposition: History Level Impact ===");
    let decomp_results = bench_complex_start_history_decomp()?;
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
    Ok(())
}
