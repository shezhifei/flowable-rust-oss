//! P2 parity (Java Flowable 8): a deployment resource row whose bytes are no longer
//! valid UTF-8 BPMN XML can only mean stored-data corruption. Java re-parsing such a
//! resource throws `FlowableException` (HTTP 500); the engine must not silently fall
//! back to default/absent user-task properties.
//!
//! Covered here:
//!  * non-UTF-8 bytes and malformed XML both surface as `StorageError::Deserialization`
//!    from both `resolve_user_task_properties` (batch path, with and without cache) and
//!    `resolve_user_task_assignee` (single-property path);
//!  * a genuinely absent resource row keeps the lenient `None`/default semantics;
//!  * a valid resource still resolves the declared `assignee` (non-regression).

// Tests opt out of the workspace `clippy::unwrap_used` ratchet on purpose: a failing
// fixture setup or assertion should abort loudly. Production code under `src/` is held
// to the lint; see the root Cargo.toml `[workspace.lints]` table.
#![allow(clippy::unwrap_used)]

use std::sync::Arc;

use flowable_engine::engine::bpmn_model_cache::BpmnModelCache;
use flowable_engine::engine::deployment_manager::DeploymentManager;
use flowable_engine::persistence::StorageError;
use flowable_engine::persistence::db_store::DbStore;
use flowable_engine::persistence::runtime_store::RuntimeStore;
use flowable_engine::repository::process_definition::ProcessDefinition;
use flowable_engine::runtime::execution::Execution;

const DEPLOYMENT_ID: &str = "dep-corrupt";
const RESOURCE_NAME: &str = "diagram.bpmn20.xml";
const PROCESS_DEFINITION_ID: &str = "corruptProcess";
const EXECUTION_ID: &str = "exec-1";
const PROCESS_INSTANCE_ID: &str = "pi-1";
const TASK_KEY: &str = "userTask1";

fn process_definition() -> ProcessDefinition {
    ProcessDefinition {
        id: PROCESS_DEFINITION_ID.to_string(),
        category: None,
        name: Some("Corrupt Resource Process".to_string()),
        key: PROCESS_DEFINITION_ID.to_string(),
        description: None,
        version: 1,
        resource_name: Some(RESOURCE_NAME.to_string()),
        deployment_id: Some(DEPLOYMENT_ID.to_string()),
        diagram_resource_name: None,
        has_start_form_key: false,
        has_graphical_notation: false,
        is_suspended: false,
        tenant_id: None,
        engine_version: None,
        app_version: None,
        history_level: None,
    }
}

/// Raw INSERT carrying the resource blob. `bytes_literal` is a SQL literal
/// (`X'..'` blob literal or a text literal).
fn insert_resource_sql(bytes_literal: &str) -> String {
    format!(
        "INSERT INTO deployment_resources \
         (deployment_id, name, resource_type, content_type, bytes, created_at) \
         VALUES ('{DEPLOYMENT_ID}', '{RESOURCE_NAME}', NULL, NULL, {bytes_literal}, NULL)"
    )
}

fn valid_bpmn_xml_literal() -> String {
    // CAST makes the literal a BLOB (the column affinity keeps plain strings as TEXT,
    // which the blob reader would skip). The XML itself only uses double quotes.
    r#"CAST('<?xml version="1.0" encoding="UTF-8"?>
    <definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL"
                 xmlns:flowable="http://flowable.org/bpmn"
                 targetNamespace="Examples">
        <process id="corruptProcess" name="Corrupt Resource Process" isExecutable="true">
            <startEvent id="startEvent1" />
            <sequenceFlow id="flow1" sourceRef="startEvent1" targetRef="userTask1" />
            <userTask id="userTask1" name="Review" flowable:assignee="kermit" />
            <sequenceFlow id="flow2" sourceRef="userTask1" targetRef="endEvent1" />
            <endEvent id="endEvent1" />
        </process>
    </definitions>' AS BLOB)"#
        .to_string()
}

struct Harness {
    deployment_manager: DeploymentManager,
    runtime_store: RuntimeStore,
}

fn harness(with_cache: bool) -> Harness {
    let db_store = Arc::new(DbStore::new_in_memory().unwrap());
    let deployment_manager = DeploymentManager::new_with_memory_backend_for_test(db_store.clone());
    let mut runtime_store = RuntimeStore::new_with_memory_backend_for_test(db_store);
    if with_cache {
        runtime_store = runtime_store.with_bpmn_model_cache(Arc::new(BpmnModelCache::new()));
    }
    Harness {
        deployment_manager,
        runtime_store,
    }
}

impl Harness {
    /// Seed the process-definition row, an execution referencing it, and optionally
    /// the deployment-resource blob expressed as raw SQL.
    fn seed(&self, resource_sql: Option<&str>) {
        let mut session = self.deployment_manager.create_session().unwrap();
        self.deployment_manager
            .insert_process_definition(process_definition(), &mut session);
        session.flush_and_commit().unwrap();

        let mut session = self.runtime_store.create_session().unwrap();
        let execution = Execution {
            id: EXECUTION_ID.to_string(),
            process_instance_id: Some(PROCESS_INSTANCE_ID.to_string()),
            process_definition_id: Some(PROCESS_DEFINITION_ID.to_string()),
            ..Default::default()
        };
        self.runtime_store
            .insert_execution(&execution, &mut session)
            .unwrap();
        if let Some(sql) = resource_sql {
            session.execute_raw_sql(sql).unwrap();
        }
        session.flush_and_commit().unwrap();
    }
}

fn assert_corrupt_resource_error(error: StorageError) {
    match error {
        StorageError::Deserialization(message) => {
            assert!(
                message.contains("corrupt deployment resource"),
                "unexpected message: {message}"
            );
            assert!(message.contains(DEPLOYMENT_ID), "message: {message}");
            assert!(message.contains(RESOURCE_NAME), "message: {message}");
        }
        other => panic!("expected StorageError::Deserialization, got {other:?}"),
    }
}

#[test]
fn non_utf8_resource_bytes_surface_error_from_batch_resolver() {
    let harness = harness(false);
    // 0xFF/0xFE can never start a valid UTF-8 sequence.
    harness.seed(Some(&insert_resource_sql("X'FFFEFDFC'")));

    let mut session = harness.runtime_store.create_session().unwrap();
    let result = harness.runtime_store.resolve_user_task_properties(
        PROCESS_INSTANCE_ID,
        EXECUTION_ID,
        TASK_KEY,
        &mut session,
    );
    assert_corrupt_resource_error(result.unwrap_err());
}

#[test]
fn non_utf8_resource_bytes_surface_error_from_single_property_resolver() {
    let harness = harness(false);
    harness.seed(Some(&insert_resource_sql("X'FFFEFDFC'")));

    let mut session = harness.runtime_store.create_session().unwrap();
    let result = harness.runtime_store.resolve_user_task_assignee(
        PROCESS_INSTANCE_ID,
        EXECUTION_ID,
        TASK_KEY,
        &mut session,
    );
    assert_corrupt_resource_error(result.unwrap_err());
}

#[test]
fn malformed_xml_resource_surfaces_error_from_batch_resolver() {
    let harness = harness(false);
    harness.seed(Some(&insert_resource_sql(
        "CAST('<definitions></process>' AS BLOB)",
    )));

    let mut session = harness.runtime_store.create_session().unwrap();
    let result = harness.runtime_store.resolve_user_task_properties(
        PROCESS_INSTANCE_ID,
        EXECUTION_ID,
        TASK_KEY,
        &mut session,
    );
    assert_corrupt_resource_error(result.unwrap_err());
}

#[test]
fn malformed_xml_resource_surfaces_error_from_single_property_resolver() {
    let harness = harness(false);
    harness.seed(Some(&insert_resource_sql(
        "CAST('<definitions></process>' AS BLOB)",
    )));

    let mut session = harness.runtime_store.create_session().unwrap();
    let result = harness.runtime_store.resolve_user_task_assignee(
        PROCESS_INSTANCE_ID,
        EXECUTION_ID,
        TASK_KEY,
        &mut session,
    );
    assert_corrupt_resource_error(result.unwrap_err());
}

#[test]
fn corrupt_resource_surfaces_error_through_model_cache_path() {
    // With a cache attached, a cache miss still parses the stored bytes; None from the
    // cache can only mean the stored bytes failed UTF-8/BPMN decoding.
    let harness = harness(true);
    harness.seed(Some(&insert_resource_sql("X'FFFEFDFC'")));

    let mut session = harness.runtime_store.create_session().unwrap();
    let result = harness.runtime_store.resolve_user_task_properties(
        PROCESS_INSTANCE_ID,
        EXECUTION_ID,
        TASK_KEY,
        &mut session,
    );
    assert_corrupt_resource_error(result.unwrap_err());
}

#[test]
fn absent_resource_keeps_lenient_none_semantics() {
    // No deployment_resources row at all: this is the legitimate Java no-row SELECT
    // result, so resolvers keep returning None / default properties.
    let harness = harness(false);
    harness.seed(None);

    let mut session = harness.runtime_store.create_session().unwrap();
    let single = harness.runtime_store.resolve_user_task_assignee(
        PROCESS_INSTANCE_ID,
        EXECUTION_ID,
        TASK_KEY,
        &mut session,
    );
    assert_eq!(single.unwrap(), None);

    let batch = harness.runtime_store.resolve_user_task_properties(
        PROCESS_INSTANCE_ID,
        EXECUTION_ID,
        TASK_KEY,
        &mut session,
    );
    assert_eq!(batch.unwrap(), Default::default());
}

#[test]
fn valid_resource_still_resolves_user_task_properties() {
    // Non-regression: an intact resource parses and yields the declared assignee from
    // both resolver paths.
    let harness = harness(false);
    harness.seed(Some(&insert_resource_sql(&valid_bpmn_xml_literal())));

    let mut session = harness.runtime_store.create_session().unwrap();
    let single = harness.runtime_store.resolve_user_task_assignee(
        PROCESS_INSTANCE_ID,
        EXECUTION_ID,
        TASK_KEY,
        &mut session,
    );
    assert_eq!(single.unwrap(), Some("kermit".to_string()));

    let batch = harness.runtime_store.resolve_user_task_properties(
        PROCESS_INSTANCE_ID,
        EXECUTION_ID,
        TASK_KEY,
        &mut session,
    );
    assert_eq!(batch.unwrap().assignee, Some("kermit".to_string()));
}
