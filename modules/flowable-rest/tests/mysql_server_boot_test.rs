//! The REST server must boot and serve against MySQL from inside a runtime.
//!
//! The MySQL twin of `postgres_server_boot_test`: `main` bootstraps the platform
//! under `#[tokio::main]`, and every handler runs on a tokio worker thread. Both
//! reach the store, which bridges the synchronous store API onto sqlx with
//! `Runtime::block_on`. That bridge yields the thread first, and the column
//! metadata lookups that used to issue SQLite-only `PRAGMA table_info(...)` now
//! go through `flowable_persistence::DbSession::table_columns`, which dispatches
//! to `information_schema.columns` on MySQL. This test is what proves the whole
//! server — not just the engine — comes up on that path.
//!
//! Whole-server MySQL is selected in production with
//! `FLOWABLE_DATABASE_URL=mysql://...` plus `--features mysql`; this test builds
//! the same engine configuration directly.
//!
//! Skips when MySQL is unreachable, so a default `cargo test` still passes. As
//! of this commit no local instance exists, so the live path is **unrun**.
//!
//! ```powershell
//! $env:FLOWABLE_TEST_MYSQL_URL = "mysql://user:pass@localhost:3306/flowable_test"
//! cargo test -p flowable-rest --features mysql --test mysql_server_boot_test
//! ```

#![cfg(feature = "mysql")]

use std::sync::Arc;

use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::engine::time_source::SystemTimeSource;
use flowable_engine::service::config::{
    DatabaseConfiguration, EngineDatabaseKind, ProcessEngineConfiguration,
};

fn mysql_config() -> ProcessEngineConfiguration {
    ProcessEngineConfiguration {
        database: DatabaseConfiguration {
            kind: EngineDatabaseKind::Mysql,
            url: std::env::var("FLOWABLE_TEST_MYSQL_URL").unwrap_or_else(|_| {
                "mysql://flowable:flowable@localhost:3306/flowable_test".to_string()
            }),
            pool_size: 4,
            // Matches `mysql_engine_integration_test`, but note it does not speed
            // up the skip path: this knob only reaches the rusqlite PRAGMA, and
            // the MySQL pool's acquire timeout is a fixed 60s
            // (`sqlx_executor.rs`), so an absent instance costs ~60s once.
            busy_timeout_ms: 2000,
            journal_mode: Default::default(),
        },
        ..Default::default()
    }
}

/// The engine is built *inside* the runtime here, exactly as `main` does under
/// `#[tokio::main]`, then served and queried over HTTP.
///
/// Skips (passes) when MySQL is unreachable, so a default `cargo test` run
/// without a live instance still succeeds.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_server_boots_and_serves_a_request_against_mysql() {
    let engine = match ProcessEngine::build_with_config(
        format!("mysql-boot-{}", std::process::id()),
        Arc::new(SystemTimeSource),
        mysql_config(),
    ) {
        Ok(engine) => Arc::new(engine),
        Err(error) => {
            eprintln!(
                "Skipping MySQL server boot test: database unreachable ({error}). Set \
                 FLOWABLE_TEST_MYSQL_URL to a live instance to run it."
            );
            return;
        }
    };

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        flowable_rest::run_server(engine, listener).await.unwrap();
    });

    // A handler that reads through to MySQL from a worker thread. Any 2xx/4xx
    // proves the bridge held; a panic would drop the connection instead.
    let response = reqwest::Client::new()
        .get(format!("{base_url}/repository/deployments"))
        .basic_auth("kermit", Some("kermit"))
        .send()
        .await
        .expect("the handler must not panic the connection");
    assert!(
        response.status().is_success() || response.status().is_client_error(),
        "unexpected status {}",
        response.status()
    );
}
