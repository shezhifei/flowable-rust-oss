//! No-database guards on the SQL each dialect composes.
//!
//! The MySQL execution path used to inherit SQLite-only syntax in two places:
//! hand-written `INSERT OR REPLACE INTO` (a syntax error on MySQL and Postgres)
//! and `CREATE INDEX IF NOT EXISTS` (unsupported by MySQL 8.0). These tests pin
//! both down without needing a live server, so a regression fails on every
//! developer machine rather than only where MySQL happens to be reachable.

use flowable_persistence::{
    MemoryDialect, MysqlDialect, PostgresDialect, SqlDialect, SqliteDialect, get_all_scripts,
    render_upsert,
};

const COLUMNS: &[&str] = &["ID_", "NAME_", "REV_"];

#[test]
fn mysql_create_index_has_no_if_not_exists() {
    let sql = MysqlDialect.create_index_if_not_exists("idx_x", "t", "c");
    assert!(
        !sql.to_ascii_uppercase().contains("IF NOT EXISTS"),
        "MySQL 8.0 rejects CREATE INDEX IF NOT EXISTS: {sql}"
    );
    assert_eq!(sql, "CREATE INDEX idx_x ON t(c)");
}

#[test]
fn other_dialects_keep_create_index_if_not_exists() {
    for dialect in [
        &SqliteDialect as &dyn SqlDialect,
        &PostgresDialect,
        &MemoryDialect,
    ] {
        let sql = dialect.create_index_if_not_exists("idx_x", "t", "c");
        assert_eq!(
            sql,
            "CREATE INDEX IF NOT EXISTS idx_x ON t(c)",
            "unexpected CREATE INDEX for {:?}",
            dialect.database_kind()
        );
    }
}

#[test]
fn insert_or_replace_into_is_dialect_specific() {
    assert_eq!(
        SqliteDialect.insert_or_replace_into(),
        "INSERT OR REPLACE INTO"
    );
    assert_eq!(
        MemoryDialect.insert_or_replace_into(),
        "INSERT OR REPLACE INTO"
    );
    assert_eq!(MysqlDialect.insert_or_replace_into(), "REPLACE INTO");
    assert_eq!(PostgresDialect.insert_or_replace_into(), "INSERT INTO");
}

#[test]
fn sqlite_upsert_uses_insert_or_replace() {
    assert_eq!(
        render_upsert(&SqliteDialect, "ACT_CO_CONTENT_ITEM", "ID_", COLUMNS),
        "INSERT OR REPLACE INTO ACT_CO_CONTENT_ITEM (ID_, NAME_, REV_) VALUES (?, ?, ?)"
    );
}

#[test]
fn mysql_upsert_uses_replace_into_without_on_conflict() {
    let sql = render_upsert(&MysqlDialect, "ACT_CO_CONTENT_ITEM", "ID_", COLUMNS);
    assert_eq!(
        sql,
        "REPLACE INTO ACT_CO_CONTENT_ITEM (ID_, NAME_, REV_) VALUES (?, ?, ?)"
    );
    let upper = sql.to_ascii_uppercase();
    assert!(
        !upper.contains("ON CONFLICT"),
        "MySQL has no ON CONFLICT clause: {sql}"
    );
    assert!(
        !upper.contains("INSERT OR REPLACE"),
        "SQLite-only syntax leaked onto the MySQL path: {sql}"
    );
}

#[test]
fn postgres_upsert_updates_every_non_pk_column_on_conflict() {
    assert_eq!(
        render_upsert(&PostgresDialect, "ACT_CO_CONTENT_ITEM", "ID_", COLUMNS),
        "INSERT INTO ACT_CO_CONTENT_ITEM (ID_, NAME_, REV_) VALUES ($1, $2, $3) \
         ON CONFLICT (ID_) DO UPDATE SET NAME_ = EXCLUDED.NAME_, REV_ = EXCLUDED.REV_"
    );
}

#[test]
fn postgres_upsert_excludes_pk_case_insensitively() {
    let sql = render_upsert(&PostgresDialect, "t", "id_", &["ID_", "NAME_"]);
    assert_eq!(
        sql,
        "INSERT INTO t (ID_, NAME_) VALUES ($1, $2) \
         ON CONFLICT (id_) DO UPDATE SET NAME_ = EXCLUDED.NAME_"
    );
}

#[test]
fn no_dialect_emits_sqlite_only_syntax_outside_sqlite() {
    for dialect in [&MysqlDialect as &dyn SqlDialect, &PostgresDialect] {
        let sql = render_upsert(dialect, "t", "ID_", COLUMNS).to_ascii_uppercase();
        assert!(
            !sql.contains("INSERT OR REPLACE"),
            "{:?} must not emit INSERT OR REPLACE: {sql}",
            dialect.database_kind()
        );
    }
}

/// Every schema script written for SQLite needs MySQL and Postgres twins,
/// otherwise the table simply never appears on those backends. The reverse is
/// allowed: 7.1.1 widens `VARCHAR` id columns for Postgres and MySQL only,
/// because SQLite neither enforces the length nor supports `ALTER COLUMN TYPE`.
#[test]
fn every_sqlite_schema_script_has_mysql_and_postgres_twins() {
    let scripts = get_all_scripts();
    let has = |version: &str, component: &str, backend: &str| {
        scripts.iter().any(|script| {
            script.version == version
                && script.component == component
                && script.database_type == backend
        })
    };

    let mut missing = Vec::new();
    for script in scripts.iter().filter(|s| s.database_type == "sqlite") {
        for backend in ["mysql", "postgres"] {
            if !has(&script.version, &script.component, backend) {
                missing.push(format!("{} {} {backend}", script.version, script.component));
            }
        }
    }
    assert!(
        missing.is_empty(),
        "schema scripts present for SQLite but missing elsewhere: {missing:?}"
    );
}

/// The MySQL schema scripts must be free of the SQLite-only DDL the SQLite
/// twins use: `CREATE INDEX IF NOT EXISTS` (MySQL 8.0 rejects it) and
/// `AUTOINCREMENT` (MySQL spells it `AUTO_INCREMENT`). `CREATE TABLE IF NOT
/// EXISTS` is fine on MySQL and deliberately not flagged.
#[test]
fn mysql_schema_scripts_avoid_sqlite_only_ddl() {
    let mut offenders = Vec::new();
    for script in get_all_scripts()
        .iter()
        .filter(|s| s.database_type == "mysql")
    {
        for statement in script.sql.split(';') {
            let upper = statement.trim().to_ascii_uppercase();
            let sqlite_only = (upper.starts_with("CREATE INDEX")
                && upper.contains("IF NOT EXISTS"))
                || (upper.starts_with("CREATE UNIQUE INDEX") && upper.contains("IF NOT EXISTS"))
                || upper.contains("AUTOINCREMENT")
                || upper.contains("INSERT OR REPLACE");
            if sqlite_only {
                offenders.push(format!(
                    "{} {}: {}",
                    script.version,
                    script.component,
                    statement.trim()
                ));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "SQLite-only DDL on the MySQL schema path: {offenders:#?}"
    );
}
