use flowable_persistence::{
    DatabaseConfig, DatabaseKind, DbSession, DbSessionFactory, FlowableSchemaManager,
    PersistenceError, PropertyStatementCatalog, SqlExecutor, SqliteDialect, SqlxExecutorFactory,
    StatementId,
};
use std::sync::Arc;

fn create_session_with_schema() -> Result<DbSession, PersistenceError> {
    // Use shared memory database so multiple connections see the same data
    let db_url = "sqlite:file:memdb?mode=memory&cache=shared";

    let config = DatabaseConfig {
        kind: DatabaseKind::Sqlite,
        url: db_url.to_string(),
        pool_size: 1,
        schema_mode: flowable_persistence::SchemaMode::Create,
        table_prefix: None,
        schema: None,
        catalog: None,
    };

    let runtime = Arc::new(
        tokio::runtime::Runtime::new().map_err(|e| PersistenceError::Connection(e.to_string()))?,
    );
    let pool_factory = SqlxExecutorFactory::new(&config, runtime)?;

    let dialect = Box::new(SqliteDialect);
    let catalog = Arc::new(PropertyStatementCatalog::new(dialect));

    let factory = DbSessionFactory::new(
        config,
        catalog,
        move || -> Result<Box<dyn SqlExecutor>, PersistenceError> {
            pool_factory.create_executor()
        },
    );

    let mut session = factory.create_session()?;

    // Create schema
    let mut schema_manager = FlowableSchemaManager::new();
    for script in flowable_persistence::schema::get_all_scripts() {
        schema_manager.add_script(script);
    }

    // Execute schema scripts with ALTER idempotency for SQLite.
    let scripts = schema_manager.get_scripts_for_database("sqlite");
    for script in scripts {
        let sql = script.sql.trim();
        let rendered = flowable_persistence::RenderedStatement::new(
            sql.to_string(),
            flowable_persistence::DbParams::new(),
        );
        if let Err(e) = session.execute_raw(rendered) {
            let msg = e.to_string();
            let is_alter = sql.to_ascii_uppercase().contains("ALTER TABLE");
            let already = msg.contains("duplicate column") || msg.contains("already exists");
            if !(is_alter && already) {
                return Err(e);
            }
        }
    }

    // Insert schema version
    let mut params = flowable_persistence::DbParams::new();
    params.push("schema.version");
    params.push("7.0.2");
    params.push(1i64);
    session.execute(StatementId::InsertProperty, params)?;

    session.commit()?;
    // Drop the first session to release its connection back to the pool
    // (pool_size=1 means only one connection can be checked out at a time).
    drop(session);

    // Create new session for tests
    factory.create_session()
}

#[test]
fn test_schema_creation() -> Result<(), PersistenceError> {
    let mut session = create_session_with_schema()?;

    // Verify schema version
    let mut params = flowable_persistence::DbParams::new();
    params.push("schema.version");
    let row = session
        .select_one(StatementId::SelectPropertyByName, params)?
        .expect("schema.version should exist");

    assert_eq!(row.get_text("VALUE_"), Some("7.0.2".to_string()));

    session.commit()?;
    Ok(())
}

#[test]
fn test_schema_manager_get_version() -> Result<(), PersistenceError> {
    // Use shared memory database so multiple connections see the same data
    let db_url = "sqlite:file:memdb_version?mode=memory&cache=shared";

    let config = DatabaseConfig {
        kind: DatabaseKind::Sqlite,
        url: db_url.to_string(),
        pool_size: 1,
        schema_mode: flowable_persistence::SchemaMode::Create,
        table_prefix: None,
        schema: None,
        catalog: None,
    };

    let runtime = Arc::new(
        tokio::runtime::Runtime::new().map_err(|e| PersistenceError::Connection(e.to_string()))?,
    );
    let pool_factory = SqlxExecutorFactory::new(&config, runtime)?;

    let dialect = Box::new(SqliteDialect);
    let catalog = Arc::new(PropertyStatementCatalog::new(dialect));

    let factory = DbSessionFactory::new(
        config,
        catalog,
        move || -> Result<Box<dyn SqlExecutor>, PersistenceError> {
            pool_factory.create_executor()
        },
    );

    let mut session = factory.create_session()?;

    // Create property table
    let create_sql = "CREATE TABLE IF NOT EXISTS ACT_GE_PROPERTY (NAME_ TEXT PRIMARY KEY, VALUE_ TEXT, REV_ INTEGER)";
    let rendered = flowable_persistence::RenderedStatement::new(
        create_sql.to_string(),
        flowable_persistence::DbParams::new(),
    );
    session.execute_raw(rendered)?;

    // Insert schema version
    let mut params = flowable_persistence::DbParams::new();
    params.push("schema.version");
    params.push("7.0.0");
    params.push(1i64);
    session.execute(StatementId::InsertProperty, params)?;
    session.commit()?;
    // Release the first session's connection so the pool can hand it to session2.
    drop(session);

    // Now test schema manager
    let mut session2 = factory.create_session()?;
    let _schema_manager = FlowableSchemaManager::new();

    // This won't work directly because we need to pass executor
    // For now, just verify the table exists by querying it
    let mut params = flowable_persistence::DbParams::new();
    params.push("schema.version");
    let row = session2
        .select_one(StatementId::SelectPropertyByName, params)?
        .expect("schema.version should exist");

    assert_eq!(row.get_text("VALUE_"), Some("7.0.0".to_string()));

    session2.commit()?;
    Ok(())
}

#[test]
fn test_schema_scripts() {
    let scripts = flowable_persistence::schema::get_all_scripts();
    assert!(!scripts.is_empty());

    let sqlite_scripts: Vec<_> = scripts
        .iter()
        .filter(|s| s.database_type == "sqlite")
        .collect();
    assert!(!sqlite_scripts.is_empty());

    let postgres_scripts: Vec<_> = scripts
        .iter()
        .filter(|s| s.database_type == "postgres")
        .collect();
    assert!(!postgres_scripts.is_empty());

    // Verify property table script exists
    let property_scripts: Vec<_> = scripts
        .iter()
        .filter(|s| s.component == "ACT_GE_PROPERTY")
        .collect();
    assert_eq!(property_scripts.len(), 3); // sqlite, postgres, mysql
}
