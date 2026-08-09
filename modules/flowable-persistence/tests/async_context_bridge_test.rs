//! The sqlx backends must be usable from inside a tokio runtime.
//!
//! The store API is synchronous and sqlx is not, so every statement bridges the
//! two with `Runtime::block_on`. Called bare, that panics with "Cannot start a
//! runtime from within a runtime" whenever a runtime is already on the thread —
//! which is every axum handler, and anything under `#[tokio::main]` or
//! `#[tokio::test]`. That made the Postgres and MySQL backends unusable from a
//! server: `flowable-rest` panicked on its first request, and its bootstrap
//! panicked before reaching one.
//!
//! These cases cover both runtime flavours plus teardown, over SQLite so they run
//! everywhere without a server. The path under test is shared by all three
//! backends — `SqlxExecutorFactory` and the executors are backend-generic about
//! how they reach the runtime — so SQLite is a faithful stand-in, and
//! `ui_postgres_smoke_test` exercises the same bridge against a real Postgres.

use flowable_persistence::{
    DatabaseConfig, DatabaseKind, DbParams, RenderedStatement, SchemaMode, SqlxExecutorFactory,
    shared_runtime,
};

fn sqlite_config() -> DatabaseConfig {
    DatabaseConfig {
        kind: DatabaseKind::Sqlite,
        // Shared-cache in-memory, so the pool's connections see one database.
        url: "sqlite::memory:".to_string(),
        pool_size: 2,
        schema_mode: SchemaMode::False,
        table_prefix: None,
        schema: None,
        catalog: None,
    }
}

/// Builds a factory and runs one statement through it, which is the whole bridge:
/// pool creation, `BEGIN`, execute, `COMMIT`.
fn round_trip() {
    let factory = SqlxExecutorFactory::new(&sqlite_config(), shared_runtime().expect("runtime"))
        .expect("factory");
    let mut executor = factory.create_executor().expect("executor");

    executor
        .execute(RenderedStatement {
            sql: "CREATE TABLE bridge_probe (ID_ TEXT)".to_string(),
            params: DbParams::new(),
        })
        .expect("DDL through the bridge");
    executor
        .execute(RenderedStatement {
            sql: "INSERT INTO bridge_probe (ID_) VALUES (?)".to_string(),
            params: {
                let mut params = DbParams::new();
                params.push("probe".to_string());
                params
            },
        })
        .expect("insert through the bridge");
    executor.commit().expect("commit through the bridge");
}

/// axum serves on a multi-thread runtime, so this is the production shape.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn statements_run_from_a_multi_thread_runtime() {
    round_trip();
}

/// `#[tokio::test]`'s default flavour, where `block_in_place` is not allowed and
/// the bridge has to hop threads instead.
#[tokio::test(flavor = "current_thread")]
async fn statements_run_from_a_current_thread_runtime() {
    round_trip();
}

/// A spawned task is a step removed from the test's own thread; the bridge has to
/// hold there too, since that is where handler code actually runs.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn statements_run_from_a_spawned_task() {
    tokio::spawn(async { round_trip() }).await.expect("task");
}

/// `spawn_blocking` is the idiomatic way to call blocking code from async, so a
/// caller doing the textbook thing must not be the one who gets a panic. Its
/// threads still carry a runtime handle, so the bridge has to notice.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn statements_run_from_spawn_blocking() {
    tokio::task::spawn_blocking(round_trip)
        .await
        .expect("blocking task");
}

/// Same, under the current-thread flavour, where the multi-thread escape is not
/// available.
#[tokio::test(flavor = "current_thread")]
async fn statements_run_from_spawn_blocking_on_a_current_thread_runtime() {
    tokio::task::spawn_blocking(round_trip)
        .await
        .expect("blocking task");
}

/// Teardown is the other end of the bridge: dropping a `Runtime` blocks, which is
/// illegal in an async context. Everything built here goes out of scope inside
/// the runtime, which is what a request-scoped session does — and what a
/// per-factory runtime would panic on. Callers that hand in a runtime of their
/// own still own its teardown; [`shared_runtime`] is what the engine uses and is
/// never dropped.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn factories_and_executors_can_be_dropped_inside_a_runtime() {
    let factory = SqlxExecutorFactory::new(&sqlite_config(), shared_runtime().expect("runtime"))
        .expect("factory");
    let executor = factory.create_executor().expect("executor");
    drop(executor);
    drop(factory);
}

/// Nothing about the fix should change the plain synchronous path, which is how
/// every existing test and the engine's own SQLite backend reach the store.
#[test]
fn statements_still_run_with_no_runtime_in_scope() {
    round_trip();
}
