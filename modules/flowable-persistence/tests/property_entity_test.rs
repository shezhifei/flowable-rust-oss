use flowable_persistence::{
    DatabaseConfig, DatabaseKind, DbSession, DbSessionFactory, PersistenceError,
    PropertyDataManager, PropertyEntity, PropertyStatementCatalog, SqlExecutor, SqliteDialect,
    SqlxExecutorFactory, create_sqlite_session_factory,
};
use std::sync::Arc;

fn create_test_session() -> Result<DbSession, PersistenceError> {
    let config = DatabaseConfig {
        kind: DatabaseKind::Sqlite,
        url: "sqlite::memory:".to_string(),
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
    let create_sql = "CREATE TABLE IF NOT EXISTS ACT_GE_PROPERTY (NAME_ TEXT PRIMARY KEY, VALUE_ TEXT, REV_ INTEGER)";
    let rendered = flowable_persistence::RenderedStatement::new(
        create_sql.to_string(),
        flowable_persistence::DbParams::new(),
    );
    session.execute_raw(rendered)?;

    Ok(session)
}

#[test]
fn test_property_entity_insert_and_find() -> Result<(), PersistenceError> {
    let mut session = create_test_session()?;
    let dm = PropertyDataManager::new();

    let property = PropertyEntity::new("schema.version".to_string(), "7.0.0".to_string());
    dm.insert(&mut session, property)?;

    session.commit()?;

    // Find it back
    let found = dm.find_by_id(&mut session, "schema.version")?;
    assert!(found.is_some());
    let found = found.unwrap();
    assert_eq!(found.name, "schema.version");
    assert_eq!(found.value, "7.0.0");
    assert_eq!(found.revision, 1);

    Ok(())
}

#[test]
fn test_property_entity_update() -> Result<(), PersistenceError> {
    let mut session = create_test_session()?;
    let dm = PropertyDataManager::new();

    let mut property = PropertyEntity::new("schema.version".to_string(), "7.0.0".to_string());
    dm.insert(&mut session, property.clone())?;
    session.commit()?;

    // Update it
    property.value = "7.1.0".to_string();
    dm.update(&mut session, property)?;
    session.commit()?;

    // Find it back
    let found = dm.find_by_id(&mut session, "schema.version")?;
    assert!(found.is_some());
    let found = found.unwrap();
    assert_eq!(found.value, "7.1.0");
    assert_eq!(found.revision, 2); // Revision incremented

    Ok(())
}

#[test]
fn test_property_entity_delete() -> Result<(), PersistenceError> {
    let mut session = create_test_session()?;
    let dm = PropertyDataManager::new();

    let property = PropertyEntity::new("schema.version".to_string(), "7.0.0".to_string());
    dm.insert(&mut session, property.clone())?;
    session.commit()?;

    // Delete it
    dm.delete(&mut session, &property)?;
    session.commit()?;

    // Verify deletion
    let found = dm.find_by_id(&mut session, "schema.version")?;
    assert!(found.is_none());

    Ok(())
}

#[test]
fn test_property_entity_find_all() -> Result<(), PersistenceError> {
    let mut session = create_test_session()?;
    let dm = PropertyDataManager::new();

    let prop1 = PropertyEntity::new("prop1".to_string(), "value1".to_string());
    let prop2 = PropertyEntity::new("prop2".to_string(), "value2".to_string());

    dm.insert(&mut session, prop1)?;
    dm.insert(&mut session, prop2)?;
    session.commit()?;

    let all = dm.find_all(&mut session)?;
    assert_eq!(all.len(), 2);

    Ok(())
}

#[test]
fn sqlx_sqlite_duplicate_property_preserves_typed_error() -> Result<(), PersistenceError> {
    let mut session = create_test_session()?;
    session.property_insert("duplicate.property", "first")?;

    let error = session
        .property_insert("duplicate.property", "second")
        .expect_err("duplicate property should fail");
    assert!(matches!(error, PersistenceError::DuplicateEntity { .. }));
    Ok(())
}

#[test]
fn rusqlite_duplicate_property_preserves_typed_error() -> Result<(), PersistenceError> {
    let config = DatabaseConfig {
        kind: DatabaseKind::Memory,
        url: "memory".to_string(),
        pool_size: 1,
        schema_mode: flowable_persistence::SchemaMode::Create,
        table_prefix: None,
        schema: None,
        catalog: None,
    };
    let catalog = Arc::new(PropertyStatementCatalog::new(Box::new(SqliteDialect)));
    let factory = create_sqlite_session_factory(&config, catalog)?;
    let mut session = factory.create_session()?;

    session.property_insert("duplicate.property", "first")?;
    let error = session
        .property_insert("duplicate.property", "second")
        .expect_err("duplicate property should fail");
    assert!(matches!(error, PersistenceError::DuplicateEntity { .. }));
    Ok(())
}
