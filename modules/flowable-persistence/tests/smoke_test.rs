use flowable_persistence::{
    DatabaseConfig, DatabaseKind, DbParams, DbSession, DbSessionFactory, PersistenceError,
    PropertyStatementCatalog, RenderedStatement, SchemaMode, SqlExecutor, SqliteDialect,
    SqlxExecutorFactory, StatementId,
};
use std::sync::Arc;

fn sqlite_config() -> DatabaseConfig {
    DatabaseConfig {
        kind: DatabaseKind::Sqlite,
        url: "sqlite::memory:".to_string(),
        // pool_size 1 ensures all sessions in a test reuse the same in-memory database.
        pool_size: 1,
        schema_mode: SchemaMode::Create,
        table_prefix: None,
        schema: None,
        catalog: None,
    }
}

fn create_sqlite_factory() -> SqlxExecutorFactory {
    let runtime = Arc::new(tokio::runtime::Runtime::new().expect("Failed to create tokio runtime"));
    SqlxExecutorFactory::new(&sqlite_config(), runtime).expect("Failed to create sqlite factory")
}

fn create_session(factory: &SqlxExecutorFactory) -> DbSession {
    let catalog = Arc::new(PropertyStatementCatalog::new(Box::new(SqliteDialect)));
    let pool_factory = factory.clone_for_session();
    let f = DbSessionFactory::new(
        sqlite_config(),
        catalog,
        move || -> Result<Box<dyn SqlExecutor>, PersistenceError> {
            pool_factory.create_executor()
        },
    );
    let mut session = f.create_session().expect("Failed to create session");
    ensure_property_schema(&mut session);
    session
}

fn ensure_property_schema(session: &mut DbSession) {
    let sql = "CREATE TABLE IF NOT EXISTS ACT_GE_PROPERTY (NAME_ TEXT PRIMARY KEY, VALUE_ TEXT, REV_ INTEGER)";
    session
        .execute_raw(RenderedStatement::new(sql.to_string(), DbParams::new()))
        .expect("Failed to ensure schema");
}

#[test]
fn sqlite_insert_and_select_property() -> Result<(), PersistenceError> {
    let factory = create_sqlite_factory();
    let mut session = create_session(&factory);

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
fn sqlite_update_with_affected_row_check() -> Result<(), PersistenceError> {
    let factory = create_sqlite_factory();
    let mut session = create_session(&factory);

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
fn sqlite_optimistic_lock_returns_zero_affected() -> Result<(), PersistenceError> {
    let factory = create_sqlite_factory();
    let mut session = create_session(&factory);

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
fn sqlite_delete_property() -> Result<(), PersistenceError> {
    let factory = create_sqlite_factory();
    let mut session = create_session(&factory);

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
fn sqlite_rollback_discards_inserts() -> Result<(), PersistenceError> {
    let factory = create_sqlite_factory();

    {
        let mut session = create_session(&factory);
        let mut params = DbParams::new();
        params.push("test.property");
        params.push("7.0.0");
        params.push(1i64);
        session.execute(StatementId::InsertProperty, params)?;
        session.rollback()?;
    } // session dropped, connection returned to pool

    let mut session2 = create_session(&factory);
    let mut params = DbParams::new();
    params.push("test.property");
    let row = session2.select_one(StatementId::SelectPropertyByName, params)?;
    assert!(row.is_none(), "Rollback should have discarded the insert");

    session2.commit()?;
    Ok(())
}

#[test]
fn sqlite_commit_persists_across_sessions() -> Result<(), PersistenceError> {
    let factory = create_sqlite_factory();

    {
        let mut session = create_session(&factory);
        let mut params = DbParams::new();
        params.push("test.property");
        params.push("7.0.0");
        params.push(1i64);
        session.execute(StatementId::InsertProperty, params)?;
        session.commit()?;
    } // session dropped, connection returned to pool

    let mut session2 = create_session(&factory);
    let mut params = DbParams::new();
    params.push("test.property");
    let row = session2
        .select_one(StatementId::SelectPropertyByName, params)?
        .unwrap();
    assert_eq!(row.get_text("VALUE_"), Some("7.0.0".to_string()));

    session2.commit()?;
    Ok(())
}

#[test]
fn sqlite_binary_value_round_trip() -> Result<(), PersistenceError> {
    let factory = create_sqlite_factory();
    let mut session = create_session(&factory);

    session.execute_raw(RenderedStatement::new(
        "CREATE TABLE IF NOT EXISTS BLOB_TEST (ID_ TEXT PRIMARY KEY, PAYLOAD_ BLOB)".to_string(),
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

    session.commit()?;
    Ok(())
}
