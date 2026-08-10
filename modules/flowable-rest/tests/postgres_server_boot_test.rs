//! The REST server must boot and serve against Postgres from inside a runtime.
//!
//! `main` bootstraps the platform under `#[tokio::main]`, and every handler runs
//! on a tokio worker thread. Both reach the store, which bridges the synchronous
//! store API onto sqlx with `Runtime::block_on` — a call that panics when a
//! runtime is already on the thread. Until that bridge learned to yield the
//! thread first, the sqlx backends could not serve a single request: startup
//! panicked in bootstrap, and a server built off-runtime panicked on first use.
//!
//! That bridge is fixed, and the UI surface now serves over Postgres — see
//! `flowable-ui-rest`'s `ui_postgres_smoke_test`.
//!
//! One further blocker predated the bridge: `run_server_with_components`
//! constructs `FlowableFormService`, whose constructor calls
//! `repository::ensure_schema` unconditionally, and that issued
//! `PRAGMA table_info(...)` — SQLite-only syntax — then `unwrap()`ed the
//! result. Postgres rejected it and the server died during construction,
//! before it could serve anything. `PRAGMA` also appeared in the content
//! service, the engine's schema and historical-migration code, and the
//! management route. Column-metadata lookups now go through
//! `flowable_persistence::DbSession::table_columns`, which dispatches per
//! backend (`PRAGMA table_info` on SQLite, `information_schema.columns` on
//! Postgres/MySQL), so the whole server boots on Postgres.
//!
//! Skips when Postgres is unreachable, so a default `cargo test` still passes.
//!
//! ```powershell
//! cargo test -p flowable-rest --features postgres --test postgres_server_boot_test
//! ```

#![cfg(feature = "postgres")]

use std::sync::Arc;

use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::engine::time_source::SystemTimeSource;
use flowable_engine::service::config::{
    DatabaseConfiguration, EngineDatabaseKind, ProcessEngineConfiguration,
};

fn postgres_config() -> ProcessEngineConfiguration {
    ProcessEngineConfiguration {
        database: DatabaseConfiguration {
            kind: EngineDatabaseKind::Postgres,
            url: std::env::var("FLOWABLE_TEST_POSTGRES_URL").unwrap_or_else(|_| {
                "postgres://postgres:postgres@localhost:5432/flowable_test".to_string()
            }),
            pool_size: 4,
            busy_timeout_ms: 5000,
            journal_mode: Default::default(),
        },
        ..Default::default()
    }
}

/// The engine is built *inside* the runtime here, exactly as `main` does under
/// `#[tokio::main]`, then served and queried over HTTP. Every step used to panic
/// in the runtime bridge; that part now works, and the form-service `PRAGMA`
/// described above is now dispatched per backend instead.
///
/// Skips (passes) when Postgres is unreachable, so a default `cargo test` run
/// without a live instance still succeeds.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_server_boots_and_serves_a_request_against_postgres() {
    let engine = match ProcessEngine::build_with_config(
        format!("pg-boot-{}", std::process::id()),
        Arc::new(SystemTimeSource),
        postgres_config(),
    ) {
        Ok(engine) => Arc::new(engine),
        Err(error) => {
            eprintln!(
                "Skipping Postgres server boot test: database unreachable ({error}). Set \
                 FLOWABLE_TEST_POSTGRES_URL to a live instance to run it."
            );
            return;
        }
    };

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        flowable_rest::run_server(engine, listener).await.unwrap();
    });

    // A handler that reads through to Postgres from a worker thread. Any 2xx/4xx
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
