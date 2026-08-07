use flowable_persistence::executor::ExecuteResult;
use flowable_persistence::{
    DatabaseConfig, DatabaseKind, DbParams, DbRow, DbSession, DbSessionFactory, Entity, EntityType,
    PersistenceError, PropertyStatementCatalog, RenderedStatement, RevisionedEntity, SqlDialect,
    SqlExecutor, SqliteDialect, SqlxExecutorFactory, StatementCatalog, StatementId,
};
use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::{Arc as SyncArc, Mutex};

#[derive(Debug, Clone)]
struct TestProperty {
    name: String,
    value: String,
    revision: i32,
}

impl Entity for TestProperty {
    fn id(&self) -> &str {
        &self.name
    }

    fn set_id(&mut self, id: String) {
        self.name = id;
    }

    fn entity_type(&self) -> EntityType {
        EntityType::Property
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn clone_box(&self) -> Box<dyn Entity> {
        Box::new(self.clone())
    }
}

impl RevisionedEntity for TestProperty {
    fn revision(&self) -> i32 {
        self.revision
    }

    fn set_revision(&mut self, revision: i32) {
        self.revision = revision;
    }
}

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

struct RecordingCatalog {
    dialect: SqliteDialect,
}

impl RecordingCatalog {
    fn new() -> Self {
        Self {
            dialect: SqliteDialect,
        }
    }
}

impl StatementCatalog for RecordingCatalog {
    fn render(
        &self,
        id: StatementId,
        dialect: &dyn SqlDialect,
        params: &DbParams,
    ) -> Result<RenderedStatement, PersistenceError> {
        PropertyStatementCatalog::new(Box::new(SqliteDialect)).render(id, dialect, params)
    }

    fn dialect(&self) -> &dyn SqlDialect {
        &self.dialect
    }
}

struct RecordingExecutor {
    executed: SyncArc<Mutex<Vec<String>>>,
    rows_affected: VecDeque<u64>,
}

impl RecordingExecutor {
    fn new(executed: SyncArc<Mutex<Vec<String>>>) -> Self {
        Self {
            executed,
            rows_affected: VecDeque::new(),
        }
    }

    fn with_rows(executed: SyncArc<Mutex<Vec<String>>>, rows_affected: Vec<u64>) -> Self {
        Self {
            executed,
            rows_affected: rows_affected.into(),
        }
    }
}

impl SqlExecutor for RecordingExecutor {
    fn execute(&mut self, statement: RenderedStatement) -> Result<ExecuteResult, PersistenceError> {
        self.executed.lock().unwrap().push(statement.sql);
        Ok(ExecuteResult {
            rows_affected: self.rows_affected.pop_front().unwrap_or(1),
        })
    }

    fn fetch_optional(
        &mut self,
        _statement: RenderedStatement,
    ) -> Result<Option<DbRow>, PersistenceError> {
        Ok(None)
    }

    fn fetch_all(&mut self, _statement: RenderedStatement) -> Result<Vec<DbRow>, PersistenceError> {
        Ok(Vec::new())
    }

    fn commit(&mut self) -> Result<(), PersistenceError> {
        Ok(())
    }

    fn rollback(&mut self) -> Result<(), PersistenceError> {
        Ok(())
    }

    fn dialect(&self) -> &dyn SqlDialect {
        &SqliteDialect
    }
}

fn recording_session(executor: RecordingExecutor) -> DbSession {
    DbSession::new(Box::new(executor), Arc::new(RecordingCatalog::new()))
}

#[test]
fn test_entity_cache_insert() -> Result<(), PersistenceError> {
    let mut session = create_test_session()?;

    let property = TestProperty {
        name: "test.prop".to_string(),
        value: "test_value".to_string(),
        revision: 1,
    };

    let mut params = flowable_persistence::DbParams::new();
    params.push(property.name.clone());
    params.push(property.value.clone());
    params.push(property.revision as i64);

    session.insert(property.clone(), StatementId::InsertProperty, params)?;

    // Verify entity is in cache
    let cached: Option<TestProperty> = session.cache().get(EntityType::Property, "test.prop");
    assert!(cached.is_some());
    assert_eq!(cached.unwrap().value, "test_value");

    // Flush and commit
    session.commit()?;

    Ok(())
}

#[test]
fn test_entity_cache_update_with_revision() -> Result<(), PersistenceError> {
    let mut session = create_test_session()?;

    // Insert initial property
    let mut params = flowable_persistence::DbParams::new();
    params.push("test.prop");
    params.push("initial_value");
    params.push(1i64);
    session.execute(StatementId::InsertProperty, params)?;

    // Update with correct revision
    let property = TestProperty {
        name: "test.prop".to_string(),
        value: "updated_value".to_string(),
        revision: 1,
    };

    let mut params = flowable_persistence::DbParams::new();
    params.push(property.value.clone());
    params.push(property.name.clone());
    params.push(property.revision as i64);

    session.update(property, StatementId::UpdateProperty, params)?;

    // Verify entity is in cache with updated value
    let cached: Option<TestProperty> = session.cache().get(EntityType::Property, "test.prop");
    assert!(cached.is_some());
    assert_eq!(cached.unwrap().value, "updated_value");

    session.commit()?;

    Ok(())
}

#[test]
fn test_optimistic_lock_failure() -> Result<(), PersistenceError> {
    let mut session = create_test_session()?;

    // Insert initial property
    let mut params = flowable_persistence::DbParams::new();
    params.push("test.prop");
    params.push("initial_value");
    params.push(1i64);
    session.execute(StatementId::InsertProperty, params)?;

    // Try to update with wrong revision (expecting 999, but actual is 1)
    let property = TestProperty {
        name: "test.prop".to_string(),
        value: "updated_value".to_string(),
        revision: 999, // Wrong revision
    };

    let mut params = flowable_persistence::DbParams::new();
    params.push(property.value.clone());
    params.push(property.name.clone());
    params.push(property.revision as i64);

    session.update(property, StatementId::UpdateProperty, params)?;

    // Flush should detect optimistic lock failure
    let result = session.commit();
    assert!(result.is_err());

    if let Err(e) = result {
        match e {
            PersistenceError::OptimisticLock {
                entity_type,
                id,
                expected,
            } => {
                assert_eq!(entity_type, "Property");
                assert_eq!(id, "test.prop");
                assert_eq!(expected, 999);
            }
            _ => panic!("Expected OptimisticLock error, got {:?}", e),
        }
    }

    Ok(())
}

#[test]
fn test_flush_ordering() -> Result<(), PersistenceError> {
    let mut session = create_test_session()?;

    // Insert a property directly
    let mut params = flowable_persistence::DbParams::new();
    params.push("test.prop");
    params.push("initial_value");
    params.push(1i64);
    session.execute(StatementId::InsertProperty, params)?;

    // Use entity-based insert (goes to pending)
    let property = TestProperty {
        name: "test.prop2".to_string(),
        value: "value2".to_string(),
        revision: 1,
    };

    let mut params = flowable_persistence::DbParams::new();
    params.push(property.name.clone());
    params.push(property.value.clone());
    params.push(property.revision as i64);

    session.insert(property, StatementId::InsertProperty, params)?;

    // Commit flushes pending inserts
    session.commit()?;

    // Verify both properties exist in same session
    let mut params = flowable_persistence::DbParams::new();
    params.push("test.prop");
    let row = session.select_one(StatementId::SelectPropertyByName, params)?;
    assert!(row.is_some());

    let mut params = flowable_persistence::DbParams::new();
    params.push("test.prop2");
    let row = session.select_one(StatementId::SelectPropertyByName, params)?;
    assert!(row.is_some());

    Ok(())
}

#[test]
fn test_flush_ordering_is_inserts_updates_deletes() -> Result<(), PersistenceError> {
    let executed = SyncArc::new(Mutex::new(Vec::new()));
    let mut session = recording_session(RecordingExecutor::new(SyncArc::clone(&executed)));

    let inserted = TestProperty {
        name: "inserted".to_string(),
        value: "value".to_string(),
        revision: 1,
    };
    let updated = TestProperty {
        name: "updated".to_string(),
        value: "value".to_string(),
        revision: 1,
    };
    let deleted = TestProperty {
        name: "deleted".to_string(),
        value: "value".to_string(),
        revision: 1,
    };

    let mut insert_params = DbParams::new();
    insert_params.push(inserted.name.clone());
    insert_params.push(inserted.value.clone());
    insert_params.push(inserted.revision as i64);
    session.insert(inserted, StatementId::InsertProperty, insert_params)?;

    let mut update_params = DbParams::new();
    update_params.push(updated.value.clone());
    update_params.push(updated.name.clone());
    update_params.push(updated.revision as i64);
    session.update(updated, StatementId::UpdateProperty, update_params)?;

    let mut delete_params = DbParams::new();
    delete_params.push(deleted.name.clone());
    session.delete(&deleted, StatementId::DeleteProperty, delete_params)?;

    session.flush()?;

    let executed = executed.lock().unwrap();
    assert!(executed[0].starts_with("INSERT INTO ACT_GE_PROPERTY"));
    assert!(executed[1].starts_with("UPDATE ACT_GE_PROPERTY"));
    assert!(executed[2].starts_with("DELETE FROM ACT_GE_PROPERTY"));

    Ok(())
}

#[test]
fn test_bulk_operations_are_flushed() -> Result<(), PersistenceError> {
    let executed = SyncArc::new(Mutex::new(Vec::new()));
    let mut session = recording_session(RecordingExecutor::new(SyncArc::clone(&executed)));

    let mut first_insert = DbParams::new();
    first_insert.push("bulk-1");
    first_insert.push("value-1");
    first_insert.push(1i64);

    let mut second_insert = DbParams::new();
    second_insert.push("bulk-2");
    second_insert.push("value-2");
    second_insert.push(1i64);

    let mut delete = DbParams::new();
    delete.push("bulk-1");

    session.bulk_insert(
        EntityType::Property,
        vec![
            (StatementId::InsertProperty, first_insert),
            (StatementId::InsertProperty, second_insert),
        ],
    )?;
    session.bulk_delete(
        EntityType::Property,
        vec![(StatementId::DeleteProperty, delete)],
    )?;

    session.flush()?;

    let executed = executed.lock().unwrap();
    assert_eq!(executed.len(), 3);
    assert_eq!(
        executed
            .iter()
            .filter(|sql| sql.starts_with("INSERT INTO ACT_GE_PROPERTY"))
            .count(),
        2
    );
    assert!(executed[2].starts_with("DELETE FROM ACT_GE_PROPERTY"));

    Ok(())
}

#[test]
fn test_delete_optimistic_lock_failure() -> Result<(), PersistenceError> {
    let executed = SyncArc::new(Mutex::new(Vec::new()));
    let mut session = recording_session(RecordingExecutor::with_rows(
        SyncArc::clone(&executed),
        vec![0],
    ));

    let property = TestProperty {
        name: "missing".to_string(),
        value: "value".to_string(),
        revision: 7,
    };

    let mut params = DbParams::new();
    params.push(property.name.clone());
    session.delete_revisioned(&property, StatementId::DeleteProperty, params)?;

    match session.flush() {
        Err(PersistenceError::OptimisticLock {
            entity_type,
            id,
            expected,
        }) => {
            assert_eq!(entity_type, "Property");
            assert_eq!(id, "missing");
            assert_eq!(expected, 7);
        }
        result => panic!("Expected delete optimistic lock failure, got {:?}", result),
    }

    Ok(())
}

#[test]
fn test_entity_cache_clear_on_rollback() -> Result<(), PersistenceError> {
    let mut session = create_test_session()?;

    // Insert a property
    let property = TestProperty {
        name: "test.prop".to_string(),
        value: "test_value".to_string(),
        revision: 1,
    };

    let mut params = flowable_persistence::DbParams::new();
    params.push(property.name.clone());
    params.push(property.value.clone());
    params.push(property.revision as i64);

    session.insert(property, StatementId::InsertProperty, params)?;

    // Verify entity is in cache
    let cached: Option<TestProperty> = session.cache().get(EntityType::Property, "test.prop");
    assert!(cached.is_some());

    // Rollback should clear cache
    session.rollback()?;

    // Cache should be empty
    let cached: Option<TestProperty> = session.cache().get(EntityType::Property, "test.prop");
    assert!(cached.is_none());

    Ok(())
}
