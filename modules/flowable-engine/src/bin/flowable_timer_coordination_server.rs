use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::service::config::{AuthPolicy, ServicePolicyConfig};
use flowable_engine::service::timer_coordination_service::TimerCoordinationService;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

fn main() {
    let args: Vec<String> = std::env::args().collect();

    let db_path = parse_arg(&args, "--db").unwrap_or_else(|| {
        eprintln!("Usage: flowable_timer_coordination_server --db <path> [--bind <addr>] [--owner-id <id>] [--admin-token <token>] [--read-token <token>]");
        std::process::exit(1);
    });

    let config_path = parse_arg(&args, "--config");
    let bind_addr_arg = parse_arg(&args, "--bind");

    let owner_id = parse_arg(&args, "--owner-id")
        .unwrap_or_else(|| format!("admin-server:{}", uuid::Uuid::new_v4()));

    let admin_token = parse_arg(&args, "--admin-token");
    let read_token = parse_arg(&args, "--read-token");

    let mut config = if let Some(path) = config_path {
        ServicePolicyConfig::load_from_file(&path).unwrap_or_else(|e| {
            eprintln!("Failed to load config from {}: {}", path, e);
            std::process::exit(1);
        })
    } else {
        ServicePolicyConfig::default()
    };

    if let Some(addr) = bind_addr_arg {
        config.bind_addr = addr;
    }

    if let Some(t) = admin_token {
        config.auth_keys.insert(
            t,
            AuthPolicy {
                actor_id: "admin-cli".to_string(),
                subject: None,
                issuer: None,
                role: "admin".to_string(),
                tenant_id: None,
            },
        );
    }
    if let Some(t) = read_token {
        config.auth_keys.insert(
            t,
            AuthPolicy {
                actor_id: "read-cli".to_string(),
                subject: None,
                issuer: None,
                role: "read".to_string(),
                tenant_id: None,
            },
        );
    }

    println!(
        "[flowable_timer_coordination_server] starting: db={}, bind={}, owner={}",
        db_path, config.bind_addr, owner_id
    );

    let mut engine_config = flowable_engine::service::config::ProcessEngineConfiguration::default();
    engine_config.database.kind = flowable_engine::service::config::EngineDatabaseKind::Sqlite;
    engine_config.database.url = db_path.clone();

    let engine = ProcessEngine::build_with_config(
        owner_id,
        Arc::new(flowable_engine::engine::time_source::SystemTimeSource),
        engine_config,
    )
    .unwrap_or_else(|error| {
        eprintln!("Failed to build process engine: {error}");
        std::process::exit(1);
    });

    let runtime_service = engine.get_runtime_service();

    let shutdown_requested = Arc::new(AtomicBool::new(false));
    let s_req = Arc::clone(&shutdown_requested);
    ctrlc::set_handler(move || {
        s_req.store(true, Ordering::SeqCst);
    })
    .expect("Failed to install Ctrl-C handler");

    let bind_addr = config.bind_addr.clone();
    let service = TimerCoordinationService::new(Arc::clone(&runtime_service), config);

    let handle = service.start(Arc::clone(&shutdown_requested));
    println!(
        "[flowable_timer_coordination_server] listening on http://{} (Ctrl-C to stop)",
        bind_addr
    );

    // Wait for shutdown
    while !shutdown_requested.load(Ordering::SeqCst) {
        std::thread::sleep(std::time::Duration::from_millis(100));
    }

    let _ = handle.join();
    println!("[flowable_timer_coordination_server] stopped");
}

fn parse_arg(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1).cloned())
}
