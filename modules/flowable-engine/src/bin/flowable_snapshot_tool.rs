use flowable_engine::engine::historical_migration::HistoricalMigrationRawDialect;
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::error::FlowableError;
use serde::Serialize;
use std::fmt;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;

#[derive(Debug)]
enum SnapshotError {
    Flowable(FlowableError),
    Serde(serde_json::Error),
    Io(std::io::Error),
    Msg(String),
}

impl fmt::Display for SnapshotError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SnapshotError::Flowable(e) => write!(f, "{e}"),
            SnapshotError::Serde(e) => write!(f, "{e}"),
            SnapshotError::Io(e) => write!(f, "{e}"),
            SnapshotError::Msg(msg) => f.write_str(msg),
        }
    }
}

impl From<FlowableError> for SnapshotError {
    fn from(e: FlowableError) -> Self {
        Self::Flowable(e)
    }
}

impl From<serde_json::Error> for SnapshotError {
    fn from(e: serde_json::Error) -> Self {
        Self::Serde(e)
    }
}

impl From<std::io::Error> for SnapshotError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();

    if args.len() < 2 || matches!(args[1].as_str(), "--help" | "-h") {
        print_help();
        std::process::exit(if args.len() < 2 { 1 } else { 0 });
    }

    let result = dispatch(&args[1], &args[2..]);

    if let Err(error) = result {
        eprintln!("Error: {error}");
        print_help();
        std::process::exit(1);
    }
}

fn dispatch(subcommand: &str, args: &[String]) -> Result<(), SnapshotError> {
    match subcommand {
        "export" => run_export(args),
        "import" => run_import(args),
        "export-historical-bundle" => run_export_historical_bundle(args),
        "inspect-historical-source" => run_inspect_historical_source(args),
        "import-historical-source" => run_import_historical_source(args),
        "inspect-historical-bundle" => run_inspect_historical_bundle(args),
        "import-historical-bundle" => run_import_historical_bundle(args),
        "inspect-historical-raw" => run_inspect_historical_raw(args),
        "import-historical-raw" => run_import_historical_raw(args),
        "inspect-historical" | "inspect-historical-sqlite" => run_inspect_historical_sqlite(args),
        "import-historical" | "import-historical-sqlite" => run_import_historical_sqlite(args),
        other => Err(SnapshotError::Msg(format!("unknown subcommand: {other}"))),
    }
}

fn run_export(args: &[String]) -> Result<(), SnapshotError> {
    let db_path = required_arg(args, "--db")?;
    let snapshot_path = required_arg(args, "--snapshot")?;
    let engine_name = optional_arg(args, "--engine-name")
        .unwrap_or_else(|| "flowable_snapshot_export".to_string());

    let engine = ProcessEngine::new_with_db_path(engine_name, &db_path);
    engine.export_recovery_snapshot_to_file(&snapshot_path)?;

    println!("Exported recovery snapshot to {}", snapshot_path);
    Ok(())
}

fn run_import(args: &[String]) -> Result<(), SnapshotError> {
    let db_path = required_arg(args, "--db")?;
    let snapshot_path = required_arg(args, "--snapshot")?;
    let engine_name = optional_arg(args, "--engine-name")
        .unwrap_or_else(|| "flowable_snapshot_import".to_string());

    let engine = ProcessEngine::new_with_db_path(engine_name, &db_path);
    engine.import_recovery_snapshot_from_file(&snapshot_path)?;

    println!("Imported recovery snapshot from {}", snapshot_path);
    Ok(())
}

fn run_inspect_historical_sqlite(args: &[String]) -> Result<(), SnapshotError> {
    let source_db = required_arg(args, "--source-db")?;
    let report_path = optional_arg(args, "--report");
    let report = ProcessEngine::inspect_historical_migration_sqlite(&source_db)?;

    if let Some(path) = report_path {
        write_json_file(&path, &report)?;
        println!("Wrote historical migration report to {}", path);
    } else {
        println!("{}", serde_json::to_string_pretty(&report)?);
    }

    Ok(())
}

fn run_export_historical_bundle(args: &[String]) -> Result<(), SnapshotError> {
    let source_db = required_arg(args, "--source-db")?;
    let output_bundle = required_arg(args, "--output-bundle")?;
    let result = ProcessEngine::export_historical_migration_bundle(&source_db, &output_bundle)?;

    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(())
}

fn run_inspect_historical_bundle(args: &[String]) -> Result<(), SnapshotError> {
    let source_bundle = required_arg(args, "--source-bundle")?;
    let report_path = optional_arg(args, "--report");
    let report = ProcessEngine::inspect_historical_migration_bundle(&source_bundle)?;

    if let Some(path) = report_path {
        write_json_file(&path, &report)?;
        println!("Wrote historical migration report to {}", path);
    } else {
        println!("{}", serde_json::to_string_pretty(&report)?);
    }

    Ok(())
}

fn run_inspect_historical_source(args: &[String]) -> Result<(), SnapshotError> {
    let source_manifest = required_arg(args, "--source-manifest")?;
    let report_path = optional_arg(args, "--report");
    let report = ProcessEngine::inspect_historical_migration_source_manifest(&source_manifest)?;

    if let Some(path) = report_path {
        write_json_file(&path, &report)?;
        println!("Wrote historical migration report to {}", path);
    } else {
        println!("{}", serde_json::to_string_pretty(&report)?);
    }

    Ok(())
}

fn run_import_historical_sqlite(args: &[String]) -> Result<(), SnapshotError> {
    let source_db = required_arg(args, "--source-db")?;
    let target_db = required_arg(args, "--target-db")?;
    let report_path = optional_arg(args, "--report");
    let engine_name = optional_arg(args, "--engine-name")
        .unwrap_or_else(|| "flowable_historical_import".to_string());

    let engine = ProcessEngine::new_with_db_path(engine_name, &target_db);
    let result = engine.import_historical_migration_from_sqlite(&source_db)?;

    if let Some(path) = report_path {
        write_json_file(&path, &result)?;
        println!("Wrote historical migration import result to {}", path);
    } else {
        println!("{}", serde_json::to_string_pretty(&result)?);
    }

    Ok(())
}

fn run_import_historical_source(args: &[String]) -> Result<(), SnapshotError> {
    let source_manifest = required_arg(args, "--source-manifest")?;
    let target_db = required_arg(args, "--target-db")?;
    let report_path = optional_arg(args, "--report");
    let engine_name = optional_arg(args, "--engine-name")
        .unwrap_or_else(|| "flowable_historical_source_import".to_string());

    let engine = ProcessEngine::new_with_db_path(engine_name, &target_db);
    let result = engine.import_historical_migration_from_source_manifest(&source_manifest)?;

    if let Some(path) = report_path {
        write_json_file(&path, &result)?;
        println!("Wrote historical migration import result to {}", path);
    } else {
        println!("{}", serde_json::to_string_pretty(&result)?);
    }

    Ok(())
}

fn run_import_historical_bundle(args: &[String]) -> Result<(), SnapshotError> {
    let source_bundle = required_arg(args, "--source-bundle")?;
    let target_db = required_arg(args, "--target-db")?;
    let report_path = optional_arg(args, "--report");
    let engine_name = optional_arg(args, "--engine-name")
        .unwrap_or_else(|| "flowable_historical_bundle_import".to_string());

    let engine = ProcessEngine::new_with_db_path(engine_name, &target_db);
    let result = engine.import_historical_migration_from_bundle(&source_bundle)?;

    if let Some(path) = report_path {
        write_json_file(&path, &result)?;
        println!("Wrote historical migration import result to {}", path);
    } else {
        println!("{}", serde_json::to_string_pretty(&result)?);
    }

    Ok(())
}

fn run_inspect_historical_raw(args: &[String]) -> Result<(), SnapshotError> {
    let source_dump = required_arg(args, "--source-dump")?;
    let dialect = parse_raw_dialect(&required_arg(args, "--dialect")?)?;
    let report_path = optional_arg(args, "--report");
    let report = ProcessEngine::inspect_historical_migration_sql_dump(&source_dump, dialect)?;

    if let Some(path) = report_path {
        write_json_file(&path, &report)?;
        println!("Wrote historical migration report to {}", path);
    } else {
        println!("{}", serde_json::to_string_pretty(&report)?);
    }

    Ok(())
}

fn run_import_historical_raw(args: &[String]) -> Result<(), SnapshotError> {
    let source_dump = required_arg(args, "--source-dump")?;
    let dialect = parse_raw_dialect(&required_arg(args, "--dialect")?)?;
    let target_db = required_arg(args, "--target-db")?;
    let report_path = optional_arg(args, "--report");
    let engine_name = optional_arg(args, "--engine-name")
        .unwrap_or_else(|| "flowable_historical_raw_import".to_string());

    let engine = ProcessEngine::new_with_db_path(engine_name, &target_db);
    let result = engine.import_historical_migration_from_sql_dump(&source_dump, dialect)?;

    if let Some(path) = report_path {
        write_json_file(&path, &result)?;
        println!("Wrote historical migration import result to {}", path);
    } else {
        println!("{}", serde_json::to_string_pretty(&result)?);
    }

    Ok(())
}

fn parse_raw_dialect(value: &str) -> Result<HistoricalMigrationRawDialect, SnapshotError> {
    match value {
        "mysql" => Ok(HistoricalMigrationRawDialect::Mysql),
        "postgres" => Ok(HistoricalMigrationRawDialect::Postgres),
        "h2" => Ok(HistoricalMigrationRawDialect::H2),
        other => Err(SnapshotError::Msg(format!(
            "unsupported --dialect value: {other} (expected mysql, postgres or h2)"
        ))),
    }
}

fn optional_arg(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|arg| arg == flag)
        .and_then(|index| args.get(index + 1).cloned())
}

fn required_arg(args: &[String], flag: &str) -> Result<String, SnapshotError> {
    optional_arg(args, flag).ok_or_else(|| SnapshotError::Msg(format!("{flag} is required")))
}

fn print_help() {
    eprintln!("Usage:");
    eprintln!(
        "  flowable_snapshot_tool export --db <path> --snapshot <path> [--engine-name <name>]"
    );
    eprintln!(
        "  flowable_snapshot_tool import --db <path> --snapshot <path> [--engine-name <name>]"
    );
    eprintln!(
        "  flowable_snapshot_tool export-historical-bundle --source-db <path> --output-bundle <path>"
    );
    eprintln!(
        "  flowable_snapshot_tool inspect-historical-source --source-manifest <path> [--report <path>]"
    );
    eprintln!(
        "  flowable_snapshot_tool import-historical-source --source-manifest <path> --target-db <path> [--engine-name <name>] [--report <path>]"
    );
    eprintln!(
        "  flowable_snapshot_tool inspect-historical-bundle --source-bundle <path> [--report <path>]"
    );
    eprintln!(
        "  flowable_snapshot_tool import-historical-bundle --source-bundle <path> --target-db <path> [--engine-name <name>] [--report <path>]"
    );
    eprintln!(
        "  flowable_snapshot_tool inspect-historical-raw --source-dump <path> --dialect <mysql|postgres|h2> [--report <path>]"
    );
    eprintln!(
        "  flowable_snapshot_tool import-historical-raw --source-dump <path> --dialect <mysql|postgres|h2> --target-db <path> [--engine-name <name>] [--report <path>]"
    );
    eprintln!("  flowable_snapshot_tool inspect-historical --source-db <path> [--report <path>]");
    eprintln!(
        "  flowable_snapshot_tool import-historical --source-db <path> --target-db <path> [--engine-name <name>] [--report <path>]"
    );
    eprintln!(
        "  flowable_snapshot_tool inspect-historical-sqlite --source-db <path> [--report <path>]"
    );
    eprintln!(
        "  flowable_snapshot_tool import-historical-sqlite --source-db <path> --target-db <path> [--engine-name <name>] [--report <path>]"
    );
}

fn write_json_file<T: Serialize>(path: &str, value: &T) -> Result<(), SnapshotError> {
    let file = File::create(Path::new(path))?;
    let mut writer = BufWriter::new(file);
    serde_json::to_writer_pretty(&mut writer, value)?;
    writer.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::dispatch;

    #[test]
    fn export_historical_bundle_dispatches_to_bundle_handler() {
        let args: Vec<String> = Vec::new();
        let error = dispatch("export-historical-bundle", &args).unwrap_err();
        assert_eq!(error.to_string(), "--source-db is required");
    }

    #[test]
    fn inspect_historical_bundle_dispatches_to_bundle_handler() {
        let args: Vec<String> = Vec::new();
        let error = dispatch("inspect-historical-bundle", &args).unwrap_err();
        assert_eq!(error.to_string(), "--source-bundle is required");
    }

    #[test]
    fn import_historical_bundle_dispatches_to_bundle_handler() {
        let args: Vec<String> = Vec::new();
        let error = dispatch("import-historical-bundle", &args).unwrap_err();
        assert_eq!(error.to_string(), "--source-bundle is required");
    }

    #[test]
    fn inspect_historical_raw_dispatches_to_raw_handler() {
        let args: Vec<String> = Vec::new();
        let error = dispatch("inspect-historical-raw", &args).unwrap_err();
        assert_eq!(error.to_string(), "--source-dump is required");
    }

    #[test]
    fn import_historical_raw_dispatches_to_raw_handler() {
        let args: Vec<String> = Vec::new();
        let error = dispatch("import-historical-raw", &args).unwrap_err();
        assert_eq!(error.to_string(), "--source-dump is required");
    }

    #[test]
    fn inspect_historical_alias_dispatches_to_handler() {
        let args: Vec<String> = Vec::new();
        let error = dispatch("inspect-historical", &args).unwrap_err();
        assert_eq!(error.to_string(), "--source-db is required");
    }

    #[test]
    fn import_historical_alias_dispatches_to_handler() {
        let args: Vec<String> = Vec::new();
        let error = dispatch("import-historical", &args).unwrap_err();
        assert_eq!(error.to_string(), "--source-db is required");
    }

    #[test]
    fn inspect_historical_source_dispatches_to_source_handler() {
        let args: Vec<String> = Vec::new();
        let error = dispatch("inspect-historical-source", &args).unwrap_err();
        assert_eq!(error.to_string(), "--source-manifest is required");
    }

    #[test]
    fn import_historical_source_dispatches_to_source_handler() {
        let args: Vec<String> = Vec::new();
        let error = dispatch("import-historical-source", &args).unwrap_err();
        assert_eq!(error.to_string(), "--source-manifest is required");
    }
}
