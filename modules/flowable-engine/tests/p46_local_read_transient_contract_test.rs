//! P46: execution-local read APIs must include transient variables, and
//! `removeVariable` must never delete a transient — Java VariableScopeImpl parity.
//!
//! Java: `getVariableInstanceLocal` returns the transient first
//! (VariableScopeImpl.java:348-352), `getVariablesLocal` merges transient over
//! persistent locals (VariableScopeImpl.java:455-469), `hasVariableLocal`
//! answers true for a transient alone (VariableScopeImpl.java:425-427), and
//! `removeVariable` only consults/removes persistent instances
//! (VariableScopeImpl.java:801-811; transient removal is the separate
//! `removeTransientVariable` family, :1027-1036).
//!
//! Since P45 strips transient variables on commit, the observable surface is
//! command-internal only: these tests run a probe `Command` through the engine's
//! command executor so the transient write and the local reads share one
//! command context, exactly like a delegate would experience mid-command.

use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::engine::variable_service::{
    DeleteVariableCmd, GetVariableLocalCmd, GetVariablesLocalCmd, HasVariableLocalCmd,
};
use flowable_engine::interceptor::command::Command;
use flowable_engine::interceptor::command_context::CommandContext;
use flowable_engine::interceptor::command_executor::CommandExecutor;
use serde_json::json;
use std::collections::HashMap;

const ONE_TASK_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL" targetNamespace="Examples">
  <process id="p46OneTask" isExecutable="true">
    <startEvent id="start" />
    <sequenceFlow id="f1" sourceRef="start" targetRef="task1" />
    <userTask id="task1" name="Task 1" />
    <sequenceFlow id="f2" sourceRef="task1" targetRef="end" />
    <endEvent id="end" />
  </process>
</definitions>"#;

fn engine_with_started_instance(name: &str) -> (ProcessEngine, String) {
    let engine = ProcessEngine::new(name.into());
    let repo = engine.get_repository_service();
    repo.deploy(
        repo.create_deployment()
            .name("p46-one-task".into())
            .add_string("p46.bpmn20.xml".into(), ONE_TASK_XML.to_string()),
    )
    .unwrap();
    let runtime = engine.get_runtime_service();
    let pi = runtime
        .start_process_instance(
            runtime
                .create_process_instance_builder()
                .process_definition_key("p46OneTask".into()),
        )
        .unwrap();
    (engine, pi.id)
}

/// Writes transient variables onto an execution mid-command, then replays the
/// local read commands inside the same command context (Java: a delegate calling
/// `execution.getVariableLocal(...)` after `setTransientVariable(...)`).
struct LocalReadProbeCmd {
    execution_id: String,
}

struct LocalReadProbeResult {
    get_transient_only: Option<serde_json::Value>,
    get_shadowed: Option<serde_json::Value>,
    has_transient_only: bool,
    has_absent: bool,
    all_locals: HashMap<String, serde_json::Value>,
}

impl Command<LocalReadProbeResult> for LocalReadProbeCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<LocalReadProbeResult, flowable_engine::error::FlowableError> {
        {
            let (store, session) = command_context.store_and_session();
            let mut execution = store.find_execution(&self.execution_id, session).unwrap();
            execution.set_transient_variable("tOnly".into(), json!("tv"));
            execution.set_transient_variable("shared".into(), json!("transient-val"));
            store.update_execution(&execution, session);
        }

        Ok(LocalReadProbeResult {
            get_transient_only: GetVariableLocalCmd::new(self.execution_id.clone(), "tOnly".into())
                .execute(command_context)?,
            get_shadowed: GetVariableLocalCmd::new(self.execution_id.clone(), "shared".into())
                .execute(command_context)?,
            has_transient_only: HasVariableLocalCmd::new(
                self.execution_id.clone(),
                "tOnly".into(),
            )
            .execute(command_context)?,
            has_absent: HasVariableLocalCmd::new(self.execution_id.clone(), "absent".into())
                .execute(command_context)?,
            all_locals: GetVariablesLocalCmd::new(self.execution_id.clone())
                .execute(command_context)?,
        })
    }
}

/// Java VariableScopeImpl.java:348-352 / :425-427 / :455-469 — local reads see
/// transient variables, and a transient shadows a same-named persistent local.
#[test]
fn local_read_commands_include_transient_within_command() {
    let (engine, pi_id) = engine_with_started_instance("p46-local-reads");
    let variables = engine.get_variable_service();

    // Persistent execution-local values written in earlier commands.
    variables
        .set_variable_local(pi_id.clone(), "shared".into(), json!("local-val"))
        .unwrap();
    variables
        .set_variable_local(pi_id.clone(), "localOnly".into(), json!("lv"))
        .unwrap();

    let result = engine
        .get_command_executor()
        .execute(&LocalReadProbeCmd {
            execution_id: pi_id.clone(),
        })
        .unwrap();

    assert_eq!(
        result.get_transient_only,
        Some(json!("tv")),
        "getVariableLocal must return a transient-only variable (VariableScopeImpl.java:348-352)"
    );
    assert_eq!(
        result.get_shadowed,
        Some(json!("transient-val")),
        "transient must shadow the same-named persistent local (VariableScopeImpl.java:350-352)"
    );
    assert!(
        result.has_transient_only,
        "hasVariableLocal must answer true for a transient alone (VariableScopeImpl.java:425-427)"
    );
    assert!(!result.has_absent, "unknown names still answer false");
    assert_eq!(
        result.all_locals.get("tOnly"),
        Some(&json!("tv")),
        "getVariablesLocal must include transient entries (VariableScopeImpl.java:464-468)"
    );
    assert_eq!(
        result.all_locals.get("shared"),
        Some(&json!("transient-val")),
        "getVariablesLocal: transient entry overwrites the persistent local of the same name"
    );
    assert_eq!(
        result.all_locals.get("localOnly"),
        Some(&json!("lv")),
        "persistent locals stay present alongside transients"
    );

    // Post-command: P45 strip keeps the cross-command surface transient-free.
    assert_eq!(
        variables
            .get_variable_local(pi_id.clone(), "tOnly".into())
            .unwrap(),
        None,
        "transient must not survive the probe command (P45 strip)"
    );
    assert_eq!(
        variables
            .get_variable_local(pi_id, "shared".into())
            .unwrap(),
        Some(json!("local-val")),
        "after commit the persistent local is visible again, un-shadowed"
    );
}

/// Writes a transient shadowing a durable variable, deletes the name, and reads
/// back within the same command (Java: removeVariable leaves transient intact).
struct DeleteKeepsTransientProbeCmd {
    execution_id: String,
}

struct DeleteKeepsTransientResult {
    transient_after_delete: Option<serde_json::Value>,
    durable_map_has_name: bool,
}

impl Command<DeleteKeepsTransientResult> for DeleteKeepsTransientProbeCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<DeleteKeepsTransientResult, flowable_engine::error::FlowableError> {
        {
            let (store, session) = command_context.store_and_session();
            let mut execution = store.find_execution(&self.execution_id, session).unwrap();
            execution.set_transient_variable("both".into(), json!("transient-v"));
            store.update_execution(&execution, session);
        }

        DeleteVariableCmd::new(self.execution_id.clone(), "both".into())
            .execute(command_context)?;

        let transient_after_delete =
            GetVariableLocalCmd::new(self.execution_id.clone(), "both".into())
                .execute(command_context)?;
        let durable_map_has_name = {
            let (store, session) = command_context.store_and_session();
            let execution = store.find_execution(&self.execution_id, session).unwrap();
            execution.variables.contains_key("both")
        };
        Ok(DeleteKeepsTransientResult {
            transient_after_delete,
            durable_map_has_name,
        })
    }
}

/// Java VariableScopeImpl.java:801-811 — removeVariable deletes only the
/// persistent instance; the transient of the same name is untouched.
#[test]
fn remove_variable_deletes_durable_but_keeps_transient() {
    let (engine, pi_id) = engine_with_started_instance("p46-delete-keeps-transient");
    let variables = engine.get_variable_service();

    variables
        .set_variable(pi_id.clone(), "both".into(), json!("durable-v"))
        .unwrap();

    let result = engine
        .get_command_executor()
        .execute(&DeleteKeepsTransientProbeCmd {
            execution_id: pi_id.clone(),
        })
        .unwrap();

    assert!(
        !result.durable_map_has_name,
        "removeVariable must delete the persistent instance (VariableScopeImpl.java:801-811)"
    );
    assert_eq!(
        result.transient_after_delete,
        Some(json!("transient-v")),
        "removeVariable must not touch the transient of the same name (VariableScopeImpl.java:801-811 vs :1027-1036)"
    );

    // Cross-command: the durable is gone and the transient was stripped on commit.
    assert_eq!(
        variables.get_variable(pi_id, "both".into()).unwrap(),
        None,
        "after the command neither durable nor transient remains"
    );
}
