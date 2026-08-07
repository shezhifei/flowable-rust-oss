//! P112 — History Level full semantics (6 levels + gating + per-definition override).
//!
//! Java sources (verified):
//! - `HistoryLevel.java:26` enum order NONE < INSTANCE < TASK < ACTIVITY < AUDIT < FULL
//! - `HistoryLevel.isAtLeast:60-63` / `getHistoryLevelForKey:41-48`
//! - `DefaultHistoryConfigurationSettings` per-record gates
//! - `ProcessEngineConfiguration.java:88` default history = "audit"
//! - `enableProcessDefinitionHistoryLevel` default false; when true, definition
//!   level **replaces** engine level (`DefaultHistoryConfigurationSettings:118-141`)

use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::engine::query::Query;
use flowable_engine::service::config::{HistoryLevel, ProcessEngineConfiguration};
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;

fn simple_process_xml(process_id: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
        <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
                     xmlns:flowable="http://flowable.org/bpmn"
                     targetNamespace="Examples">
            <process id="{process_id}" isExecutable="true">
                <startEvent id="start" />
                <sequenceFlow id="flow1" sourceRef="start" targetRef="userTask1" />
                <userTask id="userTask1" name="Review" />
                <sequenceFlow id="flow2" sourceRef="userTask1" targetRef="end" />
                <endEvent id="end" />
            </process>
        </definitions>"#
    )
}

fn process_xml_with_history_level(process_id: &str, history_level: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
        <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
                     xmlns:flowable="http://flowable.org/bpmn"
                     targetNamespace="Examples">
            <process id="{process_id}" isExecutable="true">
                <extensionElements>
                    <flowable:historyLevel>{history_level}</flowable:historyLevel>
                </extensionElements>
                <startEvent id="start" />
                <sequenceFlow id="flow1" sourceRef="start" targetRef="userTask1" />
                <userTask id="userTask1" name="Review" />
                <sequenceFlow id="flow2" sourceRef="userTask1" targetRef="end" />
                <endEvent id="end" />
            </process>
        </definitions>"#
    )
}

fn engine_with_level(name: &str, level: HistoryLevel) -> Arc<ProcessEngine> {
    let mut config = ProcessEngineConfiguration::default();
    config.history_level = level;
    config.enable_process_definition_history_level = false;
    Arc::new(ProcessEngine::new_with_config(name.to_string(), config))
}

fn deploy_and_start(
    engine: &ProcessEngine,
    process_id: &str,
    xml: String,
    vars: HashMap<String, serde_json::Value>,
) -> String {
    let repository = engine.get_repository_service();
    repository
        .deploy(
            repository
                .create_deployment()
                .name(format!("{process_id}-dep"))
                .add_string(format!("{process_id}.bpmn20.xml"), xml),
        )
        .expect("deploy");
    let pd_id = repository.get_process_definition_ids().unwrap()[0].clone();
    let runtime = engine.get_runtime_service();
    let mut builder = runtime
        .create_process_instance_builder()
        .process_definition_id(pd_id);
    for (k, v) in vars {
        builder = builder.variable(k, v);
    }
    runtime
        .start_process_instance(builder)
        .expect("start")
        .id
}

fn complete_first_task(engine: &ProcessEngine, pi_id: &str) {
    let task = engine
        .get_task_service()
        .get_tasks_by_process_instance_id(pi_id.to_string())
        .unwrap()
        .into_iter()
        .next()
        .expect("one task");
    engine
        .get_task_service()
        .complete_task_by_id(task.id)
        .expect("complete");
}

fn count_historic_pi(engine: &ProcessEngine, pi_id: &str) -> usize {
    engine
        .get_history_service()
        .create_historic_process_instance_query()
        .process_instance_id(pi_id.to_string())
        .list()
        .unwrap()
        .len()
}

fn count_historic_activities(engine: &ProcessEngine, pi_id: &str) -> usize {
    engine
        .get_history_service()
        .create_historic_activity_instance_query()
        .process_instance_id(pi_id.to_string())
        .list()
        .unwrap()
        .len()
}

fn count_historic_tasks(engine: &ProcessEngine, pi_id: &str) -> usize {
    engine
        .get_history_service()
        .create_historic_task_instance_query()
        .process_instance_id(pi_id.to_string())
        .list()
        .unwrap()
        .len()
}

fn count_historic_variables(engine: &ProcessEngine, pi_id: &str) -> usize {
    engine
        .get_history_service()
        .create_historic_variable_instance_query()
        .process_instance_id(pi_id.to_string())
        .list()
        .unwrap()
        .len()
}

fn count_historic_details(engine: &ProcessEngine, pi_id: &str) -> usize {
    let store = engine.get_runtime_store();
    let mut session = store.create_session().unwrap();
    let count = store
        .list_historic_details(&mut session)
        .into_iter()
        .filter(|d| d.process_instance_id == pi_id)
        .count();
    let _ = session.rollback();
    count
}

// ─── is_at_least / parse ────────────────────────────────────────────────────

#[test]
fn is_at_least_follows_java_declaration_order() {
    // HistoryLevel.java:26 order + isAtLeast:60-63
    assert!(HistoryLevel::Full.is_at_least(HistoryLevel::None));
    assert!(HistoryLevel::Full.is_at_least(HistoryLevel::Audit));
    assert!(HistoryLevel::Audit.is_at_least(HistoryLevel::Activity));
    assert!(HistoryLevel::Activity.is_at_least(HistoryLevel::Task));
    assert!(HistoryLevel::Task.is_at_least(HistoryLevel::Instance));
    assert!(HistoryLevel::Instance.is_at_least(HistoryLevel::None));

    assert!(!HistoryLevel::None.is_at_least(HistoryLevel::Instance));
    assert!(!HistoryLevel::Task.is_at_least(HistoryLevel::Activity));
    assert!(!HistoryLevel::Activity.is_at_least(HistoryLevel::Audit));
    assert!(!HistoryLevel::Audit.is_at_least(HistoryLevel::Full));

    // equal is at least
    assert!(HistoryLevel::Activity.is_at_least(HistoryLevel::Activity));
}

#[test]
fn parse_accepts_case_insensitive_keys_and_rejects_illegal() {
    // HistoryLevel.getHistoryLevelForKey:41-48
    assert_eq!(HistoryLevel::parse("none").unwrap(), HistoryLevel::None);
    assert_eq!(HistoryLevel::parse("INSTANCE").unwrap(), HistoryLevel::Instance);
    assert_eq!(HistoryLevel::parse("Task").unwrap(), HistoryLevel::Task);
    assert_eq!(HistoryLevel::parse("activity").unwrap(), HistoryLevel::Activity);
    assert_eq!(HistoryLevel::parse("audit").unwrap(), HistoryLevel::Audit);
    assert_eq!(HistoryLevel::parse("FULL").unwrap(), HistoryLevel::Full);

    let err = HistoryLevel::parse("variable").unwrap_err();
    assert!(err.contains("Illegal value for history-level"), "{err}");
    let err = HistoryLevel::parse("auto").unwrap_err();
    assert!(err.contains("Illegal value for history-level"), "{err}");
    let err = HistoryLevel::parse("default").unwrap_err();
    assert!(err.contains("Illegal value for history-level"), "{err}");
}

#[test]
fn default_history_level_is_audit() {
    // ProcessEngineConfiguration.java:88
    assert_eq!(HistoryLevel::default(), HistoryLevel::Audit);
    assert_eq!(
        ProcessEngineConfiguration::default().history_level,
        HistoryLevel::Audit
    );
    assert!(!ProcessEngineConfiguration::default().enable_process_definition_history_level);
}

// ─── Per-level write gates (engine-level) ───────────────────────────────────

#[test]
fn none_writes_nothing() {
    let engine = engine_with_level("p112-none", HistoryLevel::None);
    let mut vars = HashMap::new();
    vars.insert("v1".into(), json!("x"));
    let pi = deploy_and_start(
        &engine,
        "p112None",
        simple_process_xml("p112None"),
        vars,
    );
    complete_first_task(&engine, &pi);

    assert_eq!(count_historic_pi(&engine, &pi), 0);
    assert_eq!(count_historic_activities(&engine, &pi), 0);
    assert_eq!(count_historic_tasks(&engine, &pi), 0);
    assert_eq!(count_historic_variables(&engine, &pi), 0);
    assert_eq!(count_historic_details(&engine, &pi), 0);
}

#[test]
fn instance_writes_only_process_instance() {
    // INSTANCE+: PI yes; activity/task/variable no
    // (DefaultHistoryConfigurationSettings:145-147, 155-196, 258-268, 281-283)
    let engine = engine_with_level("p112-instance", HistoryLevel::Instance);
    let mut vars = HashMap::new();
    vars.insert("v1".into(), json!("x"));
    let pi = deploy_and_start(
        &engine,
        "p112Instance",
        simple_process_xml("p112Instance"),
        vars,
    );
    complete_first_task(&engine, &pi);

    assert_eq!(count_historic_pi(&engine, &pi), 1);
    assert_eq!(count_historic_activities(&engine, &pi), 0);
    assert_eq!(count_historic_tasks(&engine, &pi), 0);
    assert_eq!(count_historic_variables(&engine, &pi), 0);
    assert_eq!(count_historic_details(&engine, &pi), 0);
}

#[test]
fn task_level_writes_pi_and_tasks_not_activity_or_variable() {
    // TASK: hasTaskHistoryLevel true; ACTIVITY not reached; variables need ACTIVITY+
    let engine = engine_with_level("p112-task", HistoryLevel::Task);
    let mut vars = HashMap::new();
    vars.insert("v1".into(), json!("x"));
    let pi = deploy_and_start(
        &engine,
        "p112Task",
        simple_process_xml("p112Task"),
        vars,
    );
    complete_first_task(&engine, &pi);

    assert_eq!(count_historic_pi(&engine, &pi), 1);
    assert_eq!(count_historic_tasks(&engine, &pi), 1);
    assert_eq!(count_historic_activities(&engine, &pi), 0);
    assert_eq!(count_historic_variables(&engine, &pi), 0);
    assert_eq!(count_historic_details(&engine, &pi), 0);
}

#[test]
fn activity_level_writes_pi_activity_variable_not_task() {
    // ACTIVITY: PI + activity + variable; task requires TASK or AUDIT+
    // (hasTaskHistoryLevel: ACTIVITY is neither equal TASK nor AUDIT+)
    let engine = engine_with_level("p112-activity", HistoryLevel::Activity);
    let mut vars = HashMap::new();
    vars.insert("v1".into(), json!("x"));
    let pi = deploy_and_start(
        &engine,
        "p112Activity",
        simple_process_xml("p112Activity"),
        vars,
    );
    complete_first_task(&engine, &pi);

    assert_eq!(count_historic_pi(&engine, &pi), 1);
    assert!(
        count_historic_activities(&engine, &pi) > 0,
        "activity history expected at ACTIVITY"
    );
    assert_eq!(
        count_historic_tasks(&engine, &pi),
        0,
        "ACTIVITY alone must not record tasks (hasTaskHistoryLevel)"
    );
    assert_eq!(count_historic_variables(&engine, &pi), 1);
    // Variable detail requires FULL
    assert_eq!(count_historic_details(&engine, &pi), 0);
}

#[test]
fn audit_level_writes_pi_activity_task_variable_not_detail() {
    let engine = engine_with_level("p112-audit", HistoryLevel::Audit);
    let mut vars = HashMap::new();
    vars.insert("v1".into(), json!("x"));
    let pi = deploy_and_start(
        &engine,
        "p112Audit",
        simple_process_xml("p112Audit"),
        vars,
    );
    complete_first_task(&engine, &pi);

    assert_eq!(count_historic_pi(&engine, &pi), 1);
    assert!(count_historic_activities(&engine, &pi) > 0);
    assert_eq!(count_historic_tasks(&engine, &pi), 1);
    assert_eq!(count_historic_variables(&engine, &pi), 1);
    assert_eq!(
        count_historic_details(&engine, &pi),
        0,
        "variable detail is FULL-only"
    );
}

#[test]
fn full_level_writes_everything_including_variable_detail() {
    let engine = engine_with_level("p112-full", HistoryLevel::Full);
    let mut vars = HashMap::new();
    vars.insert("v1".into(), json!("x"));
    let pi = deploy_and_start(
        &engine,
        "p112Full",
        simple_process_xml("p112Full"),
        vars,
    );
    complete_first_task(&engine, &pi);

    assert_eq!(count_historic_pi(&engine, &pi), 1);
    assert!(count_historic_activities(&engine, &pi) > 0);
    assert_eq!(count_historic_tasks(&engine, &pi), 1);
    assert_eq!(count_historic_variables(&engine, &pi), 1);
    assert!(
        count_historic_details(&engine, &pi) > 0,
        "FULL must write historic variable detail"
    );
}

// ─── Per-definition override ────────────────────────────────────────────────

#[test]
fn per_definition_override_replaces_engine_level_when_enabled() {
    // Engine FULL, definition NONE → no history when flag on
    let mut config = ProcessEngineConfiguration::default();
    config.history_level = HistoryLevel::Full;
    config.enable_process_definition_history_level = true;
    let engine = ProcessEngine::new_with_config("p112-pd-none".into(), config);

    let pi = deploy_and_start(
        &engine,
        "p112PdNone",
        process_xml_with_history_level("p112PdNone", "none"),
        HashMap::new(),
    );
    complete_first_task(&engine, &pi);

    assert_eq!(count_historic_pi(&engine, &pi), 0);
    assert_eq!(count_historic_tasks(&engine, &pi), 0);
    assert_eq!(count_historic_activities(&engine, &pi), 0);
}

#[test]
fn per_definition_override_ignored_when_flag_disabled() {
    // Engine FULL, definition NONE, flag off → still FULL writes
    let mut config = ProcessEngineConfiguration::default();
    config.history_level = HistoryLevel::Full;
    config.enable_process_definition_history_level = false;
    let engine = ProcessEngine::new_with_config("p112-pd-flag-off".into(), config);

    let pi = deploy_and_start(
        &engine,
        "p112PdFlagOff",
        process_xml_with_history_level("p112PdFlagOff", "none"),
        HashMap::new(),
    );
    complete_first_task(&engine, &pi);

    assert_eq!(count_historic_pi(&engine, &pi), 1);
    assert_eq!(count_historic_tasks(&engine, &pi), 1);
}

#[test]
fn per_definition_override_can_raise_above_engine() {
    // Engine NONE, definition AUDIT → history when flag on
    let mut config = ProcessEngineConfiguration::default();
    config.history_level = HistoryLevel::None;
    config.enable_process_definition_history_level = true;
    let engine = ProcessEngine::new_with_config("p112-pd-raise".into(), config);

    let mut vars = HashMap::new();
    vars.insert("v1".into(), json!(1));
    let pi = deploy_and_start(
        &engine,
        "p112PdRaise",
        process_xml_with_history_level("p112PdRaise", "audit"),
        vars,
    );
    complete_first_task(&engine, &pi);

    assert_eq!(count_historic_pi(&engine, &pi), 1);
    assert_eq!(count_historic_tasks(&engine, &pi), 1);
    assert!(count_historic_activities(&engine, &pi) > 0);
}

#[test]
fn per_definition_illegal_level_falls_back_to_engine() {
    // Illegal key ignored (DefaultHistoryConfigurationSettings:75-77) → engine AUDIT
    let mut config = ProcessEngineConfiguration::default();
    config.history_level = HistoryLevel::Audit;
    config.enable_process_definition_history_level = true;
    let engine = ProcessEngine::new_with_config("p112-pd-illegal".into(), config);

    let pi = deploy_and_start(
        &engine,
        "p112PdIllegal",
        process_xml_with_history_level("p112PdIllegal", "not-a-level"),
        HashMap::new(),
    );
    complete_first_task(&engine, &pi);

    assert_eq!(count_historic_pi(&engine, &pi), 1);
    assert_eq!(count_historic_tasks(&engine, &pi), 1);
}

#[test]
fn deploy_materializes_history_level_on_process_definition() {
    let engine = engine_with_level("p112-pd-field", HistoryLevel::Audit);
    let repository = engine.get_repository_service();
    repository
        .deploy(
            repository
                .create_deployment()
                .name("p112-pd-field".into())
                .add_string(
                    "p112.bpmn20.xml".into(),
                    process_xml_with_history_level("p112PdField", "activity"),
                ),
        )
        .unwrap();
    let defs = repository.get_process_definitions().unwrap();
    let def = defs
        .into_iter()
        .find(|d| d.key == "p112PdField")
        .expect("definition");
    assert_eq!(def.history_level.as_deref(), Some("activity"));
}
