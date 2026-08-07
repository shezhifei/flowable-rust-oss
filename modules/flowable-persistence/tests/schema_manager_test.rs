//! Schema management tests for Phase 3.
//!
//! Covers create, drop, update, validate, idempotency, out-of-order detection,
//! and `DbSessionFactory::ensure_schema` integration.

use flowable_persistence::{
    DatabaseConfig, DatabaseKind, DbParams, DbSessionFactory, FlowableSchemaManager,
    PersistenceError, PropertyStatementCatalog, RenderedStatement, SchemaManager, SchemaMode,
    SqlExecutor, SqliteDialect, SqlxExecutorFactory, get_all_scripts,
};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Barrier, Condvar, Mutex};
use std::thread;
use std::time::Duration;

fn sqlite_config() -> DatabaseConfig {
    DatabaseConfig {
        kind: DatabaseKind::Sqlite,
        url: "sqlite::memory:".to_string(),
        pool_size: 1,
        schema_mode: SchemaMode::Create,
        table_prefix: None,
        schema: None,
        catalog: None,
    }
}

fn create_factory() -> SqlxExecutorFactory {
    let runtime = Arc::new(tokio::runtime::Runtime::new().unwrap());
    SqlxExecutorFactory::new(&sqlite_config(), runtime).unwrap()
}

fn create_executor(factory: &SqlxExecutorFactory) -> Box<dyn SqlExecutor> {
    factory.create_executor().unwrap()
}

fn create_manager() -> FlowableSchemaManager {
    let mut manager = FlowableSchemaManager::new();
    for script in get_all_scripts() {
        manager.add_script(script);
    }
    manager
}

fn table_exists(executor: &mut dyn SqlExecutor, table: &str) -> bool {
    let sql = format!("SELECT 1 FROM {} LIMIT 1", table);
    let rendered = RenderedStatement::new(sql, DbParams::new());
    executor.fetch_optional(rendered).is_ok()
}

fn count_tables(executor: &mut dyn SqlExecutor) -> usize {
    let sql = "SELECT name FROM sqlite_master WHERE type='table' AND name LIKE 'ACT_%'";
    let rendered = RenderedStatement::new(sql.to_string(), DbParams::new());
    executor.fetch_all(rendered).map(|r| r.len()).unwrap_or(0)
}

struct CoordinatedSchemaManager {
    active: Arc<AtomicUsize>,
    max_active: Arc<AtomicUsize>,
    attempts: Arc<AtomicUsize>,
    entered: Arc<(Mutex<bool>, Condvar)>,
    release: Arc<(Mutex<bool>, Condvar)>,
    fail_first: Arc<AtomicBool>,
}

impl SchemaManager for CoordinatedSchemaManager {
    fn create_schema(&mut self, _executor: &mut dyn SqlExecutor) -> Result<(), PersistenceError> {
        self.attempts.fetch_add(1, Ordering::SeqCst);
        let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
        self.max_active.fetch_max(active, Ordering::SeqCst);

        let (entered_lock, entered_condvar) = &*self.entered;
        *entered_lock.lock().unwrap() = true;
        entered_condvar.notify_all();

        let (release_lock, release_condvar) = &*self.release;
        let mut released = release_lock.lock().unwrap();
        while !*released {
            released = release_condvar.wait(released).unwrap();
        }

        self.active.fetch_sub(1, Ordering::SeqCst);
        if self.fail_first.swap(false, Ordering::SeqCst) {
            Err(PersistenceError::Schema("planned failure".to_string()))
        } else {
            Ok(())
        }
    }

    fn update_schema(&mut self, _executor: &mut dyn SqlExecutor) -> Result<(), PersistenceError> {
        Ok(())
    }

    fn drop_schema(&mut self, _executor: &mut dyn SqlExecutor) -> Result<(), PersistenceError> {
        Ok(())
    }

    fn validate_schema(&mut self, _executor: &mut dyn SqlExecutor) -> Result<(), PersistenceError> {
        Ok(())
    }

    fn get_schema_version(
        &mut self,
        _executor: &mut dyn SqlExecutor,
    ) -> Result<Option<String>, PersistenceError> {
        Ok(None)
    }
}

fn coordinated_factory(
    config: DatabaseConfig,
    manager: CoordinatedSchemaManager,
) -> DbSessionFactory {
    let runtime = Arc::new(tokio::runtime::Runtime::new().unwrap());
    let pool_factory = SqlxExecutorFactory::new(&config, runtime).unwrap();
    let catalog = Arc::new(PropertyStatementCatalog::new(Box::new(SqliteDialect)));

    DbSessionFactory::new(config, catalog, move || pool_factory.create_executor())
        .with_schema_manager(manager)
}

#[test]
fn create_schema_creates_all_tables() {
    let factory = create_factory();
    let mut executor = create_executor(&factory);
    let mut manager = create_manager();

    manager.create_schema(executor.as_mut()).unwrap();

    let scripts = manager.get_scripts_for_database("sqlite");
    assert!(!scripts.is_empty());
    for script in scripts {
        assert!(
            table_exists(executor.as_mut(), &script.component),
            "Table {} should exist after create_schema",
            script.component
        );
    }
}

#[test]
fn create_schema_is_idempotent() {
    let factory = create_factory();
    let mut executor = create_executor(&factory);
    let mut manager = create_manager();

    manager.create_schema(executor.as_mut()).unwrap();
    let after_first = count_tables(executor.as_mut());

    // Second call should be a no-op (no error, no duplicate tables).
    manager.create_schema(executor.as_mut()).unwrap();
    let after_second = count_tables(executor.as_mut());

    assert_eq!(after_first, after_second);
}

#[test]
fn validate_schema_passes_when_all_tables_exist() {
    let factory = create_factory();
    let mut executor = create_executor(&factory);
    let mut manager = create_manager();

    manager.create_schema(executor.as_mut()).unwrap();
    manager.validate_schema(executor.as_mut()).unwrap();
}

#[test]
fn validate_schema_fails_when_tables_missing() {
    let factory = create_factory();
    let mut executor = create_executor(&factory);
    let mut manager = create_manager();

    // Do NOT create schema — validate should fail.
    let result = manager.validate_schema(executor.as_mut());
    assert!(
        result.is_err(),
        "validate_schema should fail when tables are missing"
    );
}

#[test]
fn drop_schema_removes_all_tables() {
    let factory = create_factory();
    let mut executor = create_executor(&factory);
    let mut manager = create_manager();

    manager.create_schema(executor.as_mut()).unwrap();
    assert!(count_tables(executor.as_mut()) > 0);

    manager.drop_schema(executor.as_mut()).unwrap();

    let scripts = manager.get_scripts_for_database("sqlite");
    for script in scripts {
        assert!(
            !table_exists(executor.as_mut(), &script.component),
            "Table {} should be dropped",
            script.component
        );
    }
}

#[test]
fn drop_create_rebuilds_schema() {
    let factory = create_factory();
    let mut executor = create_executor(&factory);
    let mut manager = create_manager();

    manager.create_schema(executor.as_mut()).unwrap();

    // Drop then recreate.
    manager.drop_schema(executor.as_mut()).unwrap();
    manager.create_schema(executor.as_mut()).unwrap();

    let scripts = manager.get_scripts_for_database("sqlite");
    for script in scripts {
        assert!(table_exists(executor.as_mut(), &script.component));
    }
}

#[test]
fn update_schema_from_fresh_database() {
    let factory = create_factory();
    let mut executor = create_executor(&factory);
    let mut manager = create_manager();

    // Fresh database — no version record.
    manager.update_schema(executor.as_mut()).unwrap();

    // Version should now be recorded.
    let version = manager.get_schema_version(executor.as_mut()).unwrap();
    assert_eq!(version, Some("7.1.2".to_string()));
}

#[test]
fn update_schema_no_op_when_at_latest() {
    let factory = create_factory();
    let mut executor = create_executor(&factory);
    let mut manager = create_manager();

    manager.create_schema(executor.as_mut()).unwrap();

    // Already at latest version — should be a no-op.
    manager.update_schema(executor.as_mut()).unwrap();
}

#[test]
fn update_schema_fails_on_unknown_version() {
    let factory = create_factory();
    let mut executor = create_executor(&factory);
    let mut manager = create_manager();

    // Create schema first so ACT_GE_PROPERTY exists.
    manager.create_schema(executor.as_mut()).unwrap();

    // Overwrite version with an unrecognized value older than latest.
    let sql = "UPDATE ACT_GE_PROPERTY SET VALUE_ = '6.0.0', REV_ = REV_ + 1 WHERE NAME_ = 'schema.version'";
    let rendered = RenderedStatement::new(sql.to_string(), DbParams::new());
    executor.execute(rendered).unwrap();

    let result = manager.update_schema(executor.as_mut());
    assert!(
        result.is_err(),
        "update_schema should fail when database version is unknown"
    );
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("not recognized"),
        "Error should mention unrecognized version: {}",
        err
    );
}

#[test]
fn update_schema_fails_on_out_of_order() {
    let factory = create_factory();
    let mut executor = create_executor(&factory);
    let mut manager = create_manager();

    // Create schema first so ACT_GE_PROPERTY exists.
    manager.create_schema(executor.as_mut()).unwrap();

    // Overwrite version with a future value (newer than scripts).
    let sql = "UPDATE ACT_GE_PROPERTY SET VALUE_ = '99.99.99', REV_ = REV_ + 1 WHERE NAME_ = 'schema.version'";
    let rendered = RenderedStatement::new(sql.to_string(), DbParams::new());
    executor.execute(rendered).unwrap();

    let result = manager.update_schema(executor.as_mut());
    assert!(
        result.is_err(),
        "update_schema should fail when database version is newer than scripts"
    );
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("newer than"),
        "Error should mention out-of-order: {}",
        err
    );
}

#[test]
fn schema_version_is_recorded_after_create() {
    let factory = create_factory();
    let mut executor = create_executor(&factory);
    let mut manager = create_manager();

    manager.create_schema(executor.as_mut()).unwrap();

    let version = manager.get_schema_version(executor.as_mut()).unwrap();
    assert_eq!(version, Some("7.1.2".to_string()));
}

#[test]
fn db_session_factory_ensure_schema_create_mode() {
    let factory = create_factory();
    let pool_factory = factory.clone_for_session();

    let catalog = Arc::new(PropertyStatementCatalog::new(Box::new(SqliteDialect)));
    let config = DatabaseConfig {
        schema_mode: SchemaMode::Create,
        ..sqlite_config()
    };

    let f = DbSessionFactory::new(
        config,
        catalog,
        move || -> Result<Box<dyn SqlExecutor>, PersistenceError> {
            pool_factory.create_executor()
        },
    )
    .with_schema_manager(create_manager());

    f.ensure_schema().unwrap();

    // Verify tables exist by querying through a normal session.
    let mut session = f.create_session().unwrap();
    let sql = "SELECT name FROM sqlite_master WHERE type='table' AND name LIKE 'ACT_%'";
    let rows = session
        .select_raw(RenderedStatement::new(sql.to_string(), DbParams::new()))
        .unwrap();
    assert!(
        rows.len() >= 6,
        "Expected at least 6 ACT_ tables, found {}",
        rows.len()
    );
}

#[test]
fn db_session_factory_ensure_schema_drop_create_mode() {
    let factory = create_factory();
    let pool_factory = factory.clone_for_session();

    let catalog = Arc::new(PropertyStatementCatalog::new(Box::new(SqliteDialect)));
    let config = DatabaseConfig {
        schema_mode: SchemaMode::DropCreate,
        ..sqlite_config()
    };

    let f = DbSessionFactory::new(
        config,
        catalog,
        move || -> Result<Box<dyn SqlExecutor>, PersistenceError> {
            pool_factory.create_executor()
        },
    )
    .with_schema_manager(create_manager());

    // First ensure schema exists.
    f.ensure_schema().unwrap();

    // Verify standard tables exist.
    let mut session = f.create_session().unwrap();
    let sql = "SELECT name FROM sqlite_master WHERE type='table' AND name = 'ACT_GE_PROPERTY'";
    let rows = session
        .select_raw(RenderedStatement::new(sql.to_string(), DbParams::new()))
        .unwrap();
    assert!(
        !rows.is_empty(),
        "ACT_GE_PROPERTY should exist before DropCreate"
    );
    drop(session);

    // DropCreate should rebuild the schema.
    f.ensure_schema().unwrap();

    // Verify standard tables still exist after rebuild.
    let mut session = f.create_session().unwrap();
    let sql = "SELECT name FROM sqlite_master WHERE type='table' AND name = 'ACT_GE_PROPERTY'";
    let rows = session
        .select_raw(RenderedStatement::new(sql.to_string(), DbParams::new()))
        .unwrap();
    assert!(
        !rows.is_empty(),
        "ACT_GE_PROPERTY should exist after DropCreate"
    );
}

#[test]
fn db_session_factory_serializes_schema_initialization_per_physical_database() {
    let database = tempfile::NamedTempFile::new().unwrap();
    let config = DatabaseConfig {
        url: format!("sqlite:{}", database.path().display()),
        pool_size: 2,
        schema_mode: SchemaMode::Create,
        ..sqlite_config()
    };
    let active = Arc::new(AtomicUsize::new(0));
    let max_active = Arc::new(AtomicUsize::new(0));
    let attempts = Arc::new(AtomicUsize::new(0));
    let entered = Arc::new((Mutex::new(false), Condvar::new()));
    let release = Arc::new((Mutex::new(false), Condvar::new()));
    let fail_first = Arc::new(AtomicBool::new(false));
    let barrier = Arc::new(Barrier::new(3));

    let make_factory = || {
        coordinated_factory(
            config.clone(),
            CoordinatedSchemaManager {
                active: Arc::clone(&active),
                max_active: Arc::clone(&max_active),
                attempts: Arc::clone(&attempts),
                entered: Arc::clone(&entered),
                release: Arc::clone(&release),
                fail_first: Arc::clone(&fail_first),
            },
        )
    };

    let first_factory = make_factory();
    let second_factory = make_factory();
    let first_barrier = Arc::clone(&barrier);
    let second_barrier = Arc::clone(&barrier);
    let first = thread::spawn(move || {
        first_barrier.wait();
        first_factory.ensure_schema()
    });
    let second = thread::spawn(move || {
        second_barrier.wait();
        second_factory.ensure_schema()
    });

    barrier.wait();
    let (entered_lock, entered_condvar) = &*entered;
    let entered_guard = entered_lock.lock().unwrap();
    let (entered_guard, wait_result) = entered_condvar
        .wait_timeout_while(entered_guard, Duration::from_secs(2), |entered| !*entered)
        .unwrap();
    assert!(!wait_result.timed_out());
    drop(entered_guard);
    thread::sleep(Duration::from_millis(100));
    assert_eq!(attempts.load(Ordering::SeqCst), 1);

    let (release_lock, release_condvar) = &*release;
    *release_lock.lock().unwrap() = true;
    release_condvar.notify_all();

    first.join().unwrap().unwrap();
    second.join().unwrap().unwrap();
    assert_eq!(max_active.load(Ordering::SeqCst), 1);
    assert_eq!(attempts.load(Ordering::SeqCst), 2);
}

#[test]
fn db_session_factory_retries_schema_initialization_after_failure() {
    let database = tempfile::NamedTempFile::new().unwrap();
    let config = DatabaseConfig {
        url: format!("sqlite:{}", database.path().display()),
        schema_mode: SchemaMode::Create,
        ..sqlite_config()
    };
    let attempts = Arc::new(AtomicUsize::new(0));
    let release = Arc::new((Mutex::new(true), Condvar::new()));
    let factory = coordinated_factory(
        config,
        CoordinatedSchemaManager {
            active: Arc::new(AtomicUsize::new(0)),
            max_active: Arc::new(AtomicUsize::new(0)),
            attempts: Arc::clone(&attempts),
            entered: Arc::new((Mutex::new(false), Condvar::new())),
            release,
            fail_first: Arc::new(AtomicBool::new(true)),
        },
    );

    assert!(factory.ensure_schema().is_err());
    factory.ensure_schema().unwrap();
    assert_eq!(attempts.load(Ordering::SeqCst), 2);
}

#[test]
fn db_session_factory_ensure_schema_false_mode_is_no_op() {
    let factory = create_factory();
    let pool_factory = factory.clone_for_session();

    let catalog = Arc::new(PropertyStatementCatalog::new(Box::new(SqliteDialect)));
    let config = DatabaseConfig {
        schema_mode: SchemaMode::False,
        ..sqlite_config()
    };

    let f = DbSessionFactory::new(
        config,
        catalog,
        move || -> Result<Box<dyn SqlExecutor>, PersistenceError> {
            pool_factory.create_executor()
        },
    )
    .with_schema_manager(create_manager());

    f.ensure_schema().unwrap();

    // No tables should have been created.
    let mut session = f.create_session().unwrap();
    let sql = "SELECT name FROM sqlite_master WHERE type='table' AND name LIKE 'ACT_%'";
    let rows = session
        .select_raw(RenderedStatement::new(sql.to_string(), DbParams::new()))
        .unwrap();
    assert!(rows.is_empty(), "False mode should not create any tables");
}

fn manager_through(version: &str) -> FlowableSchemaManager {
    let mut manager = FlowableSchemaManager::new();
    for script in get_all_scripts()
        .into_iter()
        .filter(|script| script.version.as_str() <= version)
    {
        manager.add_script(script);
    }
    manager
}

/// Production path: an existing 7.0.0 database opened with SchemaMode::True must run
/// `update_schema` so later migrations (incl. 7.1.0 CMMN ALTER columns) become available
/// (Java `databaseSchemaUpdate=true` parity). Advances to latest (7.1.2).
#[test]
fn ensure_schema_true_mode_upgrades_7_0_0_to_latest_and_exposes_cmmn_columns() {
    let database = tempfile::NamedTempFile::new().unwrap();
    let url = format!("sqlite:{}", database.path().display());

    // --- Phase 1: bootstrap a "7.0.0-only" database (old binary / old scripts) ---
    {
        let config = DatabaseConfig {
            url: url.clone(),
            pool_size: 2,
            schema_mode: SchemaMode::Create,
            ..sqlite_config()
        };
        let runtime = Arc::new(tokio::runtime::Runtime::new().unwrap());
        let pool_factory = SqlxExecutorFactory::new(&config, runtime).unwrap();
        let catalog = Arc::new(PropertyStatementCatalog::new(Box::new(SqliteDialect)));
        let pool_for_session = pool_factory.clone_for_session();
        let factory = DbSessionFactory::new(config, catalog, move || pool_for_session.create_executor())
            .with_schema_manager(manager_through("7.0.0"));
        factory.ensure_schema().unwrap();

        let mut session = factory.create_session().unwrap();
        session
            .execute_raw(RenderedStatement::new(
                "INSERT INTO ACT_CMMN_DEPLOYMENT (ID_, NAME_, TENANT_ID_, DEPLOYED_AT_, DATA_) \
                 VALUES ('dep-old', 'legacy', NULL, '2026-07-12T00:00:00Z', '{}')"
                    .to_string(),
                DbParams::new(),
            ))
            .unwrap();
        session.commit().unwrap();

        let version = session
            .select_raw(RenderedStatement::new(
                "SELECT VALUE_ FROM ACT_GE_PROPERTY WHERE NAME_ = 'schema.version'".to_string(),
                DbParams::new(),
            ))
            .unwrap();
        assert_eq!(
            version[0].get_text("VALUE_").as_deref(),
            Some("7.0.0"),
            "seed database must record schema version 7.0.0"
        );
    }

    // --- Phase 2: "new binary" starts with SchemaMode::True (default production) ---
    {
        let config = DatabaseConfig {
            url: url.clone(),
            pool_size: 2,
            schema_mode: SchemaMode::True,
            ..sqlite_config()
        };
        let runtime = Arc::new(tokio::runtime::Runtime::new().unwrap());
        let pool_factory = SqlxExecutorFactory::new(&config, runtime).unwrap();
        let catalog = Arc::new(PropertyStatementCatalog::new(Box::new(SqliteDialect)));
        let pool_for_session = pool_factory.clone_for_session();
        let factory = DbSessionFactory::new(config, catalog, move || pool_for_session.create_executor())
            .with_schema_manager(create_manager()); // full scripts including 7.1.0

        factory
            .ensure_schema()
            .expect("SchemaMode::True must upgrade an older schema on startup");

        let mut session = factory.create_session().unwrap();

        // Version advanced to latest.
        let version = session
            .select_raw(RenderedStatement::new(
                "SELECT VALUE_ FROM ACT_GE_PROPERTY WHERE NAME_ = 'schema.version'".to_string(),
                DbParams::new(),
            ))
            .unwrap();
        assert_eq!(
            version[0].get_text("VALUE_").as_deref(),
            Some("7.1.2"),
            "ensure_schema(True) must advance schema.version to 7.1.2"
        );

        // 7.1.0 CMMN columns must be queryable; existing row preserved with null new cols.
        let deployment = session
            .select_raw(RenderedStatement::new(
                "SELECT ID_, CATEGORY_, KEY_, PARENT_DEPLOYMENT_ID_ FROM ACT_CMMN_DEPLOYMENT \
                 WHERE ID_ = 'dep-old'"
                    .to_string(),
                DbParams::new(),
            ))
            .expect("7.1.0 columns must be selectable after upgrade");
        assert_eq!(deployment.len(), 1);
        assert_eq!(deployment[0].get_text("ID_").as_deref(), Some("dep-old"));
        assert_eq!(deployment[0].get_text("CATEGORY_"), None);
        assert_eq!(deployment[0].get_text("KEY_"), None);
        assert_eq!(deployment[0].get_text("PARENT_DEPLOYMENT_ID_"), None);
    }
}

/// SchemaMode::Create must remain create-only (no versioned upgrade) so operators can
/// opt out of automatic migrations.
#[test]
fn ensure_schema_create_mode_does_not_upgrade_existing_7_0_0_schema() {
    let database = tempfile::NamedTempFile::new().unwrap();
    let url = format!("sqlite:{}", database.path().display());

    {
        let config = DatabaseConfig {
            url: url.clone(),
            pool_size: 2,
            schema_mode: SchemaMode::Create,
            ..sqlite_config()
        };
        let runtime = Arc::new(tokio::runtime::Runtime::new().unwrap());
        let pool_factory = SqlxExecutorFactory::new(&config, runtime).unwrap();
        let catalog = Arc::new(PropertyStatementCatalog::new(Box::new(SqliteDialect)));
        let pool_for_session = pool_factory.clone_for_session();
        let factory = DbSessionFactory::new(config, catalog, move || pool_for_session.create_executor())
            .with_schema_manager(manager_through("7.0.0"));
        factory.ensure_schema().unwrap();
    }

    {
        let config = DatabaseConfig {
            url,
            pool_size: 2,
            schema_mode: SchemaMode::Create,
            ..sqlite_config()
        };
        let runtime = Arc::new(tokio::runtime::Runtime::new().unwrap());
        let pool_factory = SqlxExecutorFactory::new(&config, runtime).unwrap();
        let catalog = Arc::new(PropertyStatementCatalog::new(Box::new(SqliteDialect)));
        let pool_for_session = pool_factory.clone_for_session();
        let factory = DbSessionFactory::new(config, catalog, move || pool_for_session.create_executor())
            .with_schema_manager(create_manager());
        factory.ensure_schema().unwrap();

        let mut session = factory.create_session().unwrap();
        let version = session
            .select_raw(RenderedStatement::new(
                "SELECT VALUE_ FROM ACT_GE_PROPERTY WHERE NAME_ = 'schema.version'".to_string(),
                DbParams::new(),
            ))
            .unwrap();
        assert_eq!(
            version[0].get_text("VALUE_").as_deref(),
            Some("7.0.0"),
            "SchemaMode::Create must not run update_schema"
        );
    }
}

/// Version newer than the code must refuse to start under SchemaMode::True.
#[test]
fn ensure_schema_true_mode_rejects_schema_newer_than_code() {
    let database = tempfile::NamedTempFile::new().unwrap();
    let url = format!("sqlite:{}", database.path().display());

    {
        let config = DatabaseConfig {
            url: url.clone(),
            pool_size: 2,
            schema_mode: SchemaMode::Create,
            ..sqlite_config()
        };
        let runtime = Arc::new(tokio::runtime::Runtime::new().unwrap());
        let pool_factory = SqlxExecutorFactory::new(&config, runtime).unwrap();
        let catalog = Arc::new(PropertyStatementCatalog::new(Box::new(SqliteDialect)));
        let pool_for_session = pool_factory.clone_for_session();
        let factory = DbSessionFactory::new(config, catalog, move || pool_for_session.create_executor())
            .with_schema_manager(create_manager());
        factory.ensure_schema().unwrap();

        let mut session = factory.create_session().unwrap();
        session
            .execute_raw(RenderedStatement::new(
                "UPDATE ACT_GE_PROPERTY SET VALUE_ = '99.99.99', REV_ = REV_ + 1 \
                 WHERE NAME_ = 'schema.version'"
                    .to_string(),
                DbParams::new(),
            ))
            .unwrap();
        session.commit().unwrap();
    }

    let config = DatabaseConfig {
        url,
        pool_size: 2,
        schema_mode: SchemaMode::True,
        ..sqlite_config()
    };
    let runtime = Arc::new(tokio::runtime::Runtime::new().unwrap());
    let pool_factory = SqlxExecutorFactory::new(&config, runtime).unwrap();
    let catalog = Arc::new(PropertyStatementCatalog::new(Box::new(SqliteDialect)));
    let pool_for_session = pool_factory.clone_for_session();
    let factory = DbSessionFactory::new(config, catalog, move || pool_for_session.create_executor())
        .with_schema_manager(create_manager());

    let err = factory
        .ensure_schema()
        .expect_err("DB newer than code must refuse to start");
    let msg = err.to_string();
    assert!(
        msg.contains("newer than"),
        "error should mention out-of-order version: {msg}"
    );
}
