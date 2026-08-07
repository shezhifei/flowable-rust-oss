//! MySQL smoke test for the sqlx dual-backend slice.
//!
//! Opt-in via the `mysql` cargo feature and `FLOWABLE_TEST_MYSQL_URL`:
//!
//! ```powershell
//! $env:FLOWABLE_TEST_MYSQL_URL = "mysql://user:pass@localhost:3306/flowable_test"
//! cargo test -p flowable-persistence --features mysql --test smoke_test_mysql
//! ```
//!
//! When `FLOWABLE_TEST_MYSQL_URL` is not set, the suite defaults to
//! `mysql://flowable:flowable@localhost:3306/flowable_test` and will fail
//! visibly if no MySQL instance is available.

use flowable_persistence::{
    DatabaseConfig, DatabaseKind, DbParams, DbSession, DbSessionFactory, MysqlDialect,
    PersistenceError, PropertyStatementCatalog, RenderedStatement, SchemaMode, SqlExecutor,
    SqlxExecutorFactory, StatementId,
};
use std::sync::{Arc, Mutex};

/// Serialize MySQL smoke tests because they share the same database and key space.
static MYSQL_TEST_LOCK: Mutex<()> = Mutex::new(());

fn mysql_url() -> String {
    std::env::var("FLOWABLE_TEST_MYSQL_URL")
        .unwrap_or_else(|_| "mysql://flowable:flowable@localhost:3306/flowable_test".to_string())
}

fn locked_mysql_url() -> (String, std::sync::MutexGuard<'static, ()>) {
    (
        mysql_url(),
        MYSQL_TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()),
    )
}

fn mysql_config(url: &str) -> DatabaseConfig {
    DatabaseConfig {
        kind: DatabaseKind::Mysql,
        url: url.to_string(),
        pool_size: 4,
        schema_mode: SchemaMode::Create,
        table_prefix: None,
        schema: None,
        catalog: None,
    }
}

fn create_mysql_factory(url: &str) -> SqlxExecutorFactory {
    let runtime = Arc::new(tokio::runtime::Runtime::new().expect("Failed to create tokio runtime"));
    SqlxExecutorFactory::new(&mysql_config(url), runtime).expect("Failed to create mysql factory")
}

fn create_session(factory: &SqlxExecutorFactory) -> DbSession {
    let catalog = Arc::new(PropertyStatementCatalog::new(Box::new(MysqlDialect)));
    let pool_factory = factory.clone_for_session();
    let url = mysql_url();
    let f = DbSessionFactory::new(
        mysql_config(&url),
        catalog,
        move || -> Result<Box<dyn SqlExecutor>, PersistenceError> {
            pool_factory.create_executor()
        },
    );
    // MySQL DDL (CREATE TABLE) implicitly commits the current transaction.
    // Create schema on a dedicated short-lived session before returning a
    // clean transactional session for test work.
    {
        let mut bootstrap = f
            .create_session()
            .expect("Failed to create bootstrap session");
        ensure_property_schema(&mut bootstrap);
        bootstrap
            .commit()
            .expect("Failed to commit schema bootstrap");
    }
    f.create_session().expect("Failed to create session")
}

fn ensure_property_schema(session: &mut DbSession) {
    let sql = "CREATE TABLE IF NOT EXISTS ACT_GE_PROPERTY (NAME_ VARCHAR(64) PRIMARY KEY, VALUE_ VARCHAR(300), REV_ INTEGER)";
    session
        .execute_raw(RenderedStatement::new(sql.to_string(), DbParams::new()))
        .expect("Failed to ensure property schema");
}

fn cleanup_property(session: &mut DbSession, name: &str) {
    let mut params = DbParams::new();
    params.push(name);
    let _ = session.execute_raw(RenderedStatement::new(
        "DELETE FROM ACT_GE_PROPERTY WHERE NAME_ = ?".to_string(),
        params,
    ));
}

fn cleanup_property_committed(factory: &SqlxExecutorFactory, name: &str) {
    let mut session = create_session(factory);
    cleanup_property(&mut session, name);
    session.commit().expect("Failed to commit cleanup");
}

#[test]
fn mysql_insert_and_select_property() -> Result<(), PersistenceError> {
    let (url, _guard) = locked_mysql_url();
    let factory = create_mysql_factory(&url);
    let mut session = create_session(&factory);
    cleanup_property(&mut session, "test.property");

    let mut params = DbParams::new();
    params.push("test.property");
    params.push("7.0.0");
    params.push(1i64);
    session.execute(StatementId::InsertProperty, params)?;

    let mut params = DbParams::new();
    params.push("test.property");
    let row = session
        .select_one(StatementId::SelectPropertyByName, params)?
        .unwrap();
    assert_eq!(row.get_text("NAME_"), Some("test.property".to_string()));
    assert_eq!(row.get_text("VALUE_"), Some("7.0.0".to_string()));
    assert_eq!(row.get_integer("REV_"), Some(1));

    session.commit()?;
    Ok(())
}

#[test]
fn mysql_update_with_affected_row_check() -> Result<(), PersistenceError> {
    let (url, _guard) = locked_mysql_url();
    let factory = create_mysql_factory(&url);
    let mut session = create_session(&factory);
    cleanup_property(&mut session, "test.property");

    let mut params = DbParams::new();
    params.push("test.property");
    params.push("7.0.0");
    params.push(1i64);
    session.execute(StatementId::InsertProperty, params)?;

    let mut params = DbParams::new();
    params.push("7.1.0");
    params.push("test.property");
    params.push(1i64);
    let result = session.execute(StatementId::UpdateProperty, params)?;
    assert_eq!(result.rows_affected, 1);

    let mut params = DbParams::new();
    params.push("test.property");
    let row = session
        .select_one(StatementId::SelectPropertyByName, params)?
        .unwrap();
    assert_eq!(row.get_text("VALUE_"), Some("7.1.0".to_string()));
    assert_eq!(row.get_integer("REV_"), Some(2));

    session.commit()?;
    Ok(())
}

#[test]
fn mysql_optimistic_lock_returns_zero_affected() -> Result<(), PersistenceError> {
    let (url, _guard) = locked_mysql_url();
    let factory = create_mysql_factory(&url);
    let mut session = create_session(&factory);
    cleanup_property(&mut session, "test.property");

    let mut params = DbParams::new();
    params.push("test.property");
    params.push("7.0.0");
    params.push(1i64);
    session.execute(StatementId::InsertProperty, params)?;

    let mut params = DbParams::new();
    params.push("7.1.0");
    params.push("test.property");
    params.push(999i64);
    let result = session.execute(StatementId::UpdateProperty, params)?;
    assert_eq!(result.rows_affected, 0);

    session.commit()?;
    Ok(())
}

#[test]
fn mysql_delete_property() -> Result<(), PersistenceError> {
    let (url, _guard) = locked_mysql_url();
    let factory = create_mysql_factory(&url);
    let mut session = create_session(&factory);
    cleanup_property(&mut session, "test.property");

    let mut params = DbParams::new();
    params.push("test.property");
    params.push("7.0.0");
    params.push(1i64);
    session.execute(StatementId::InsertProperty, params)?;

    let mut params = DbParams::new();
    params.push("test.property");
    let result = session.execute(StatementId::DeleteProperty, params)?;
    assert_eq!(result.rows_affected, 1);

    let mut params = DbParams::new();
    params.push("test.property");
    let row = session.select_one(StatementId::SelectPropertyByName, params)?;
    assert!(row.is_none());

    session.commit()?;
    Ok(())
}

#[test]
fn mysql_rollback_discards_inserts() -> Result<(), PersistenceError> {
    let (url, _guard) = locked_mysql_url();
    let factory = create_mysql_factory(&url);
    cleanup_property_committed(&factory, "test.property");

    {
        let mut session = create_session(&factory);
        let mut params = DbParams::new();
        params.push("test.property");
        params.push("7.0.0");
        params.push(1i64);
        session.execute(StatementId::InsertProperty, params)?;
        session.rollback()?;
    }

    let mut session2 = create_session(&factory);
    let mut params = DbParams::new();
    params.push("test.property");
    let row = session2.select_one(StatementId::SelectPropertyByName, params)?;
    assert!(row.is_none(), "Rollback should have discarded the insert");

    session2.commit()?;
    Ok(())
}

#[test]
fn mysql_commit_persists_across_sessions() -> Result<(), PersistenceError> {
    let (url, _guard) = locked_mysql_url();
    let factory = create_mysql_factory(&url);

    {
        let mut session = create_session(&factory);
        cleanup_property(&mut session, "test.property");
        let mut params = DbParams::new();
        params.push("test.property");
        params.push("7.0.0");
        params.push(1i64);
        session.execute(StatementId::InsertProperty, params)?;
        session.commit()?;
    }

    let mut session2 = create_session(&factory);
    let mut params = DbParams::new();
    params.push("test.property");
    let row = session2
        .select_one(StatementId::SelectPropertyByName, params)?
        .unwrap();
    assert_eq!(row.get_text("VALUE_"), Some("7.0.0".to_string()));

    cleanup_property(&mut session2, "test.property");
    session2.commit()?;
    Ok(())
}

#[test]
fn mysql_binary_value_round_trip() -> Result<(), PersistenceError> {
    let (url, _guard) = locked_mysql_url();
    let factory = create_mysql_factory(&url);
    let mut session = create_session(&factory);

    session.execute_raw(RenderedStatement::new(
        "DROP TABLE IF EXISTS BLOB_TEST".to_string(),
        DbParams::new(),
    ))?;
    session.execute_raw(RenderedStatement::new(
        "CREATE TABLE BLOB_TEST (ID_ VARCHAR(64) PRIMARY KEY, PAYLOAD_ BLOB)".to_string(),
        DbParams::new(),
    ))?;

    let payload: Vec<u8> = vec![0x00, 0x01, 0xFF, 0xFE, 0xDE, 0xAD, 0xBE, 0xEF];
    let mut params = DbParams::new();
    params.push("blob-1");
    params.push(payload.clone());
    session.execute_raw(RenderedStatement::new(
        "INSERT INTO BLOB_TEST (ID_, PAYLOAD_) VALUES (?, ?)".to_string(),
        params,
    ))?;

    let mut params = DbParams::new();
    params.push("blob-1");
    let row = session
        .select_one_raw(RenderedStatement::new(
            "SELECT PAYLOAD_ FROM BLOB_TEST WHERE ID_ = ?".to_string(),
            params,
        ))?
        .expect("Missing blob row");
    assert_eq!(row.get_blob("PAYLOAD_"), Some(payload));

    session.execute_raw(RenderedStatement::new(
        "DROP TABLE BLOB_TEST".to_string(),
        DbParams::new(),
    ))?;
    session.commit()?;
    Ok(())
}
