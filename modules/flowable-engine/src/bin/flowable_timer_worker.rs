#![cfg_attr(test, allow(dead_code))]

//! Standalone timer-worker entrypoint.
//!
//! Connects to an existing Flowable database and runs the timer-worker
//! poll loop, consuming the same acquire / execute / renew contract as
//! the embedded `TimerExecutor`.
//!
//! Usage:
//!   flowable_timer_worker --db <path> [--mode async-executor] [--poll-interval-ms <ms>] [--owner-id <id>]
//!   flowable_timer_worker --db <path> --mode async-executor [--pool-size <n>] [--queue-size <n>] [--owner-id <id>]
//!
//! Control subcommand:
//!   flowable_timer_worker control --db <path> <command> [args]
//!   Commands:
//!     status          - Get coordinator status
//!     nodes           - List all timer nodes
//!     release         - Safely release leadership (requires --owner-id and --fencing-token)
//!     step-down       - Admin step-down (force release)
//!     deregister <id> - Deregister a timer node
//!     cleanup         - Clean up expired timer nodes

use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::engine::timer_worker::{TimerWorker, TimerWorkerConfig};
use flowable_engine::service::config::ProcessEngineConfiguration;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

fn main() {
    let args: Vec<String> = std::env::args().collect();

    // Check if first argument is "control"
    if args.len() > 1 && args[1] == "control" {
        control_subcommand(&args[2..]);
    } else {
        worker_main(&args);
    }
}

fn control_subcommand(args: &[String]) {
    if args.is_empty() || args[0] == "--help" {
        print_control_help();
        std::process::exit(0);
    }

    let db_path = parse_arg(args, "--db");
    let server_url = parse_arg(args, "--server-url");
    let auth_token = parse_arg(args, "--auth-token");

    if db_path.is_none() && server_url.is_none() {
        eprintln!(
            "Error: Either --db <path> or --server-url <url> is required for control commands"
        );
        std::process::exit(1);
    }

    let command = &args[0];

    if let Some(url) = server_url {
        let mut client =
            flowable_engine::service::timer_coordination_client::TimerCoordinationClient::new(url);
        if let Some(token) = auth_token {
            client = client.with_auth(token);
        }
        match command.as_str() {
            "status" => match client.get_status() {
                Ok(status) => println!("{}", serde_json::to_string_pretty(&status).unwrap()),
                Err(e) => eprintln!("Error: {}", e),
            },
            "nodes" => match client.get_nodes() {
                Ok(nodes) => println!("{}", serde_json::to_string_pretty(&nodes).unwrap()),
                Err(e) => eprintln!("Error: {}", e),
            },
            "release" => {
                let fencing_token: i64 = parse_arg(args, "--fencing-token")
                    .and_then(|s| s.parse().ok())
                    .unwrap_or_else(|| {
                        eprintln!("Error: --fencing-token <token> is required for release command");
                        std::process::exit(1);
                    });
                match client.release_leadership(fencing_token) {
                    Ok(success) => println!("{}", serde_json::to_string_pretty(&success).unwrap()),
                    Err(e) => eprintln!("Error: {}", e),
                }
            }
            "step-down" => match client.admin_step_down() {
                Ok((success, new_token)) => {
                    let result = serde_json::json!({
                        "success": success,
                        "new_fencing_token": new_token
                    });
                    println!("{}", serde_json::to_string_pretty(&result).unwrap());
                }
                Err(e) => eprintln!("Error: {}", e),
            },
            "deregister" => {
                let node_id = parse_arg(args, "--node-id").unwrap_or_else(|| {
                    if args.len() > 1 && !args[1].starts_with("--") {
                        args[1].clone()
                    } else {
                        eprintln!("Error: node ID is required for deregister command");
                        std::process::exit(1);
                    }
                });
                match client.deregister_node(&node_id) {
                    Ok(success) => println!("{}", serde_json::to_string_pretty(&success).unwrap()),
                    Err(e) => eprintln!("Error: {}", e),
                }
            }
            "cleanup" => match client.cleanup_expired_nodes() {
                Ok(cleaned) => {
                    let result = serde_json::json!({
                        "cleaned_count": cleaned
                    });
                    println!("{}", serde_json::to_string_pretty(&result).unwrap());
                }
                Err(e) => eprintln!("Error: {}", e),
            },
            _ => {
                eprintln!("Unknown command: {}", command);
                print_control_help();
                std::process::exit(1);
            }
        }
        return;
    }

    // Direct DB mode
    let db_path = db_path.unwrap();
    let owner_id = parse_arg(args, "--owner-id").unwrap_or_else(|| "admin".to_string());

    let mut config = flowable_engine::service::config::ProcessEngineConfiguration::default();
    config.database.kind = flowable_engine::service::config::EngineDatabaseKind::Sqlite;
    config.database.url = db_path.clone();

    let engine = ProcessEngine::build_with_config(
        owner_id,
        Arc::new(flowable_engine::engine::time_source::SystemTimeSource),
        config,
    )
    .unwrap_or_else(|error| {
        eprintln!("Failed to build process engine: {error}");
        std::process::exit(1);
    });

    let runtime_service = engine.get_runtime_service();

    match command.as_str() {
        "status" => {
            let status = runtime_service.get_timer_coordinator_status();
            println!("{}", serde_json::to_string_pretty(&status).unwrap());
        }
        "nodes" => {
            let nodes = runtime_service.list_timer_nodes().unwrap();
            println!("{}", serde_json::to_string_pretty(&nodes).unwrap());
        }
        "release" => {
            let fencing_token: i64 = parse_arg(args, "--fencing-token")
                .and_then(|s| s.parse().ok())
                .unwrap_or_else(|| {
                    eprintln!("Error: --fencing-token <token> is required for release command");
                    std::process::exit(1);
                });
            let success = runtime_service.release_leadership(fencing_token).unwrap();
            println!("{}", serde_json::to_string_pretty(&success).unwrap());
        }
        "step-down" => {
            let (success, new_token) = runtime_service.admin_step_down().unwrap();
            let result = serde_json::json!({
                "success": success,
                "new_fencing_token": new_token
            });
            println!("{}", serde_json::to_string_pretty(&result).unwrap());
        }
        "deregister" => {
            let node_id = parse_arg(args, "--node-id").unwrap_or_else(|| {
                if args.len() > 1 && !args[1].starts_with("--") {
                    args[1].clone()
                } else {
                    eprintln!("Error: node ID is required for deregister command");
                    eprintln!(
                        "Usage: flowable_timer_worker control --db <path> deregister --node-id <id>"
                    );
                    std::process::exit(1);
                }
            });
            let success = runtime_service
                .deregister_timer_node(&node_id)
                .unwrap_or(false);
            println!("{}", serde_json::to_string_pretty(&success).unwrap());
        }
        "cleanup" => {
            let cleaned = runtime_service.cleanup_expired_timer_nodes().unwrap();
            let result = serde_json::json!({
                "cleaned_count": cleaned
            });
            println!("{}", serde_json::to_string_pretty(&result).unwrap());
        }
        _ => {
            eprintln!("Unknown command: {}", command);
            print_control_help();
            std::process::exit(1);
        }
    }
}

fn print_control_help() {
    println!("Timer Coordination Control Commands");
    println!(
        "Usage: flowable_timer_worker control [--db <path> | --server-url <url>] <command> [options]"
    );
    println!();
    println!("Commands:");
    println!("  status                    - Get coordinator status");
    println!("  nodes                     - List all timer nodes");
    println!("  release --fencing-token <token>");
    println!("                            - Safely release leadership (requires --owner-id)");
    println!("  step-down                 - Admin step-down (force release)");
    println!("  deregister --node-id <id> - Deregister a timer node");
    println!("  cleanup                   - Clean up expired timer nodes");
    println!();
    println!("Global options:");
    println!("  --db <path>               - Database file path (for local execution)");
    println!(
        "  --server-url <url>        - URL of the timer coordination server (e.g. 127.0.0.1:8080)"
    );
    println!("  --auth-token <token>      - Bearer token if using --server-url");
    println!("  --owner-id <id>           - Owner ID (default: admin, only used with --db)");
}

fn worker_main(args: &[String]) {
    let db_path = parse_arg(args, "--db").unwrap_or_else(|| {
        eprintln!(
            "Usage: flowable_timer_worker --db <path> [--mode async-executor] [--poll-interval-ms <ms>] [--owner-id <id>]"
        );
        eprintln!("       flowable_timer_worker control --db <path> <command>");
        std::process::exit(1);
    });

    let mode = parse_arg(args, "--mode").unwrap_or_default();

    if mode == "async-executor" {
        async_executor_main(args, &db_path);
        return;
    }

    let poll_interval_ms: u64 = parse_arg(args, "--poll-interval-ms")
        .and_then(|s| s.parse().ok())
        .unwrap_or(100);

    let max_jitter_ms: u64 = parse_arg(args, "--max-jitter-ms")
        .and_then(|s| s.parse().ok())
        .unwrap_or(50);

    let owner_id = parse_arg(args, "--owner-id")
        .unwrap_or_else(|| format!("standalone-worker:{}", uuid::Uuid::new_v4()));

    println!(
        "[flowable_timer_worker] starting: db={}, poll_interval={}ms, jitter={}ms, owner={}",
        db_path, poll_interval_ms, max_jitter_ms, owner_id
    );

    let mut config = ProcessEngineConfiguration::default();
    config.database.kind = flowable_engine::service::config::EngineDatabaseKind::Sqlite;
    config.database.url = db_path.clone();

    let engine = ProcessEngine::build_with_config(
        owner_id,
        Arc::new(flowable_engine::engine::time_source::SystemTimeSource),
        config,
    )
    .unwrap_or_else(|error| {
        eprintln!("Failed to build process engine: {error}");
        std::process::exit(1);
    });

    let runtime_service = engine.get_runtime_service();
    let worker = TimerWorker::new(Arc::clone(&runtime_service), "standalone");
    let config = TimerWorkerConfig {
        poll_interval_ms,
        heartbeat_interval_ms: 60_000,
        max_jitter_ms,
        coordinator_lease_timeout_ms: 300_000,
    };
    let shutdown_requested = Arc::new(AtomicBool::new(false));
    install_shutdown_handler(Arc::clone(&shutdown_requested));

    println!("[flowable_timer_worker] running (Ctrl-C to stop)");
    run_worker_loop(worker, config, shutdown_requested);
}

/// Starts the engine with the dual async executor (thread pool + acquisition threads)
/// instead of the single-threaded TimerWorker poll loop.
fn async_executor_main(args: &[String], db_path: &str) {
    let owner_id = parse_arg(args, "--owner-id")
        .unwrap_or_else(|| format!("async-executor:{}", uuid::Uuid::new_v4()));

    let pool_size: usize = parse_arg(args, "--pool-size")
        .and_then(|s| s.parse().ok())
        .unwrap_or(8);

    let queue_size: usize = parse_arg(args, "--queue-size")
        .and_then(|s| s.parse().ok())
        .unwrap_or(2048);

    println!(
        "[flowable_timer_worker] starting async-executor mode: db={}, pool_size={}, queue_size={}, owner={}",
        db_path, pool_size, queue_size, owner_id
    );

    let mut config = ProcessEngineConfiguration::default();
    config.async_executor.enabled = true;
    config.async_executor.pool_size = pool_size;
    config.async_executor.queue_size = queue_size;
    config.database.kind = flowable_engine::service::config::EngineDatabaseKind::Sqlite;
    config.database.url = db_path.to_string();

    let engine = ProcessEngine::build_with_config(
        owner_id,
        Arc::new(flowable_engine::engine::time_source::SystemTimeSource),
        config,
    )
    .unwrap_or_else(|error| {
        eprintln!("Failed to build process engine: {error}");
        std::process::exit(1);
    });

    let shutdown_requested = Arc::new(AtomicBool::new(false));
    install_shutdown_handler(Arc::clone(&shutdown_requested));

    engine.start_timer_executor();
    println!("[flowable_timer_worker] async executor running (Ctrl-C to stop)");

    while !shutdown_requested.load(Ordering::SeqCst) {
        thread::sleep(Duration::from_millis(200));
    }

    println!("[flowable_timer_worker] shutting down async executor...");
    engine.stop_timer_executor();
    println!("[flowable_timer_worker] stopped");
}

fn parse_arg(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1).cloned())
}

fn install_shutdown_handler(shutdown_requested: Arc<AtomicBool>) {
    ctrlc::set_handler(move || {
        shutdown_requested.store(true, Ordering::SeqCst);
    })
    .expect("Failed to install Ctrl-C handler");
}

pub(crate) fn run_worker_loop(
    worker: TimerWorker,
    config: TimerWorkerConfig,
    shutdown_requested: Arc<AtomicBool>,
) {
    while !shutdown_requested.load(Ordering::SeqCst) {
        let works = worker.acquire_due_timers(config.coordinator_lease_timeout_ms);

        for work in works {
            if shutdown_requested.load(Ordering::SeqCst) {
                break;
            }
            execute_with_heartbeat(
                &worker,
                &work,
                Duration::from_millis(config.heartbeat_interval_ms.max(1)),
            );
        }

        if shutdown_requested.load(Ordering::SeqCst) {
            break;
        }

        let current_poll = config.poll_interval_ms + config.get_jitter_ms();
        sleep_with_shutdown(&shutdown_requested, Duration::from_millis(current_poll));
    }

    worker.graceful_shutdown();
}

fn sleep_with_shutdown(shutdown_requested: &AtomicBool, duration: Duration) {
    let mut remaining_ms = duration.as_millis() as u64;
    while remaining_ms > 0 && !shutdown_requested.load(Ordering::SeqCst) {
        let step_ms = remaining_ms.min(50);
        thread::sleep(Duration::from_millis(step_ms));
        remaining_ms -= step_ms;
    }
}

fn execute_with_heartbeat(
    worker: &TimerWorker,
    work: &flowable_engine::engine::timer_worker::TimerWork,
    heartbeat_interval: Duration,
) {
    let heartbeat_stop = Arc::new(AtomicBool::new(false));
    let heartbeat_runtime_service = worker.runtime_service.clone();
    let heartbeat_work = work.clone();
    let heartbeat_stop_thread = Arc::clone(&heartbeat_stop);
    let hb_token = worker.get_fencing_token();

    let heartbeat_handle = thread::spawn(move || {
        let heartbeat_worker = TimerWorker::new(heartbeat_runtime_service, "standalone_hb");
        heartbeat_worker.set_fencing_token(hb_token);
        while !heartbeat_stop_thread.load(Ordering::SeqCst) {
            sleep_with_shutdown(&heartbeat_stop_thread, heartbeat_interval);
            if heartbeat_stop_thread.load(Ordering::SeqCst) {
                break;
            }
            heartbeat_worker.renew_timer_lease(&heartbeat_work);
        }
    });

    worker.execute_timer(work);

    heartbeat_stop.store(true, Ordering::SeqCst);
    let _ = heartbeat_handle.join();
}
