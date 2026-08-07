// Performance baseline stress tests for flowable-engine.
// Seeds hot tables with 1K/10K/50K rows and measures representative queries.
// Run with: cargo test -p flowable-engine --test performance_baseline_stress -- --nocapture

use chrono::{TimeZone, Utc};
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::engine::time_source::TestTimeSource;
use flowable_engine::persistence::db_session::FilterOp;
use flowable_engine::task::Task;
use std::sync::Arc;
use std::time::Instant;

fn create_engine() -> (ProcessEngine, Arc<TestTimeSource>) {
    let now = Utc.with_ymd_and_hms(2026, 4, 18, 12, 0, 0).unwrap();
    let time_source = Arc::new(TestTimeSource::new(now));
    let engine =
        ProcessEngine::with_time_source("perf_test_engine".to_string(), time_source.clone());
    (engine, time_source)
}

fn timer_bpmn() -> &'static str {
    r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="timerProcess" name="Timer Process">
            <startEvent id="startEvent1" />
            <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="catch1" />
            <intermediateCatchEvent id="catch1">
                <timerEventDefinition>
                    <timeDuration>PT10M</timeDuration>
                </timerEventDefinition>
            </intermediateCatchEvent>
            <sequenceFlow id="flow2" sourceRef="catch1" targetRef="endEvent1" />
            <endEvent id="endEvent1" />
        </process>
    </definitions>"#
}

fn user_task_bpmn() -> &'static str {
    r#"<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
        <process id="userTaskProcess" name="User Task Process">
            <startEvent id="startEvent1" />
            <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="task1" />
            <userTask id="task1" />
            <sequenceFlow id="flow2" sourceRef="task1" targetRef="endEvent1" />
            <endEvent id="endEvent1" />
        </process>
    </definitions>"#
}

fn seed_timer_jobs(engine: &ProcessEngine, count: usize) {
    let repository_service = engine.get_repository_service();
    let runtime_service = engine.get_runtime_service();

    let builder = repository_service
        .create_deployment()
        .name("timer_deployment".to_string())
        .add_string("process.bpmn20.xml".to_string(), timer_bpmn().to_string());
    repository_service.deploy(builder).unwrap();

    let pd_id = repository_service.get_process_definition_ids().unwrap()[0].clone();
    for _ in 0..count {
        let pi_builder = runtime_service
            .create_process_instance_builder()
            .process_definition_id(pd_id.clone());
        runtime_service.start_process_instance(pi_builder).unwrap();
    }
}

fn seed_user_tasks(engine: &ProcessEngine, count: usize) {
    let repository_service = engine.get_repository_service();
    let runtime_service = engine.get_runtime_service();

    let builder = repository_service
        .create_deployment()
        .name("user_task_deployment".to_string())
        .add_string(
            "process.bpmn20.xml".to_string(),
            user_task_bpmn().to_string(),
        );
    repository_service.deploy(builder).unwrap();

    let pd_id = repository_service.get_process_definition_ids().unwrap()[0].clone();
    for _ in 0..count {
        let pi_builder = runtime_service
            .create_process_instance_builder()
            .process_definition_id(pd_id.clone());
        runtime_service.start_process_instance(pi_builder).unwrap();
    }
}

#[test]
fn stress_timer_acquisition() {
    println!("\n=== Timer acquisition stress ===");
    for count in [1_000, 10_000, 50_000] {
        let (engine, _time_source) = create_engine();
        seed_timer_jobs(&engine, count);

        let runtime_store = engine.get_runtime_store();
        let mut session = runtime_store.create_session().unwrap();

        let now = Utc.with_ymd_and_hms(2026, 4, 18, 13, 0, 0).unwrap();
        let now_ms = now.timestamp_millis();
        let lock_timeout_ms = 300_000;

        // Warm-up
        let _ =
            runtime_store.acquire_due_timer_jobs("owner1", now_ms, lock_timeout_ms, &mut session);
        session.rollback().unwrap();
        let mut session = runtime_store.create_session().unwrap();

        let start = Instant::now();
        let (acquired, _recovered, _conflicts) =
            runtime_store.acquire_due_timer_jobs("owner2", now_ms, lock_timeout_ms, &mut session);
        let elapsed = start.elapsed();
        session.rollback().unwrap();

        println!(
            "timer_jobs={:>6} acquired={:>4} elapsed={:>8.2}ms",
            count,
            acquired.len(),
            elapsed.as_secs_f64() * 1000.0
        );
    }
}

#[test]
fn stress_task_query_by_assignee() {
    println!("\n=== Task query by assignee stress ===");
    for count in [1_000, 10_000, 50_000] {
        let (engine, _time_source) = create_engine();
        seed_user_tasks(&engine, count);

        let runtime_store = engine.get_runtime_store();
        let mut session = runtime_store.create_session().unwrap();

        // Load all tasks and assign half of them to a specific user
        let filters = vec![("assignee".to_string(), FilterOp::IsNull)];
        let tasks: Vec<Task> = session
            .find_with_filters("tasks", &filters, Some(("id", true)), None)
            .unwrap();
        let mut updated = 0;
        for (idx, task) in tasks.iter().enumerate() {
            if idx % 2 == 0 {
                let mut t = task.clone();
                t.assignee = Some("kermit".to_string());
                runtime_store.update_task(&t, &mut session);
                updated += 1;
                if updated >= 1000 {
                    break;
                }
            }
        }
        session.flush_and_commit().unwrap();

        let mut session = runtime_store.create_session().unwrap();
        let filters = vec![("assignee".to_string(), FilterOp::Eq("kermit".into()))];

        // Warm-up
        let _ =
            session.find_with_filters::<Task>("tasks", &filters, Some(("due_date", true)), None);
        session.rollback().unwrap();
        let mut session = runtime_store.create_session().unwrap();

        let start = Instant::now();
        let tasks: Vec<Task> = session
            .find_with_filters("tasks", &filters, Some(("due_date", true)), None)
            .unwrap();
        let elapsed = start.elapsed();
        session.rollback().unwrap();

        println!(
            "tasks={:>6} matched={:>4} elapsed={:>8.2}ms",
            count,
            tasks.len(),
            elapsed.as_secs_f64() * 1000.0
        );
    }
}

#[test]
fn stress_task_query_by_process_instance() {
    println!("\n=== Task query by process instance stress ===");
    for count in [1_000, 10_000, 50_000] {
        let (engine, _time_source) = create_engine();
        seed_user_tasks(&engine, count);

        let runtime_store = engine.get_runtime_store();
        let mut session = runtime_store.create_session().unwrap();

        // Find one process instance id to query by
        let filters = vec![("assignee".to_string(), FilterOp::IsNull)];
        let tasks: Vec<Task> = session
            .find_with_filters("tasks", &filters, Some(("id", true)), Some(1))
            .unwrap();
        let pi_id = tasks
            .first()
            .map(|t| t.process_instance_id.clone())
            .unwrap_or_default();
        session.rollback().unwrap();

        let mut session = runtime_store.create_session().unwrap();
        let filters = vec![(
            "process_instance_id".to_string(),
            FilterOp::Eq(pi_id.as_str().into()),
        )];

        // Warm-up
        let _ = session.find_with_filters::<Task>("tasks", &filters, None, None);
        session.rollback().unwrap();
        let mut session = runtime_store.create_session().unwrap();

        let start = Instant::now();
        let tasks: Vec<Task> = session
            .find_with_filters("tasks", &filters, None, None)
            .unwrap();
        let elapsed = start.elapsed();
        session.rollback().unwrap();

        println!(
            "tasks={:>6} matched={:>4} elapsed={:>8.2}ms",
            count,
            tasks.len(),
            elapsed.as_secs_f64() * 1000.0
        );
    }
}
