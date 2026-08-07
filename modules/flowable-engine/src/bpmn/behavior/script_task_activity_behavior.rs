use crate::agenda::FlowableEngineAgenda;
use crate::delegate::activity_behavior::ActivityBehavior;
use crate::error::FlowableError;
use crate::interceptor::command_context::CommandContext;
use crate::runtime::execution::Execution;
use crate::scripting::secure_context::SecureScriptContext;
use crate::scripting::secure_engine::{SecureScriptEngine, validate_script_task};

pub struct ScriptTaskActivityBehavior;

impl Default for ScriptTaskActivityBehavior {
    fn default() -> Self {
        Self::new()
    }
}

impl ScriptTaskActivityBehavior {
    pub fn new() -> Self {
        Self
    }
}

impl ActivityBehavior for ScriptTaskActivityBehavior {
    fn execute(
        &self,
        execution: &mut Execution,
        command_context: &mut CommandContext,
    ) -> Result<(), FlowableError> {
        let config = command_context.config.clone();

        // Resolve script metadata from the BPMN model
        let (script_format, script_body, result_variable, auto_store, skip_expression) = {
            let activity_id = execution.activity_id.as_ref().ok_or_else(|| {
                FlowableError::ExecutionError("Script task execution has no activity_id".into())
            })?;
            let process_def_id = execution.process_definition_id.as_ref().ok_or_else(|| {
                FlowableError::ExecutionError(
                    "Script task execution has no process_definition_id".into(),
                )
            })?;

            let model = command_context
                .deployment_manager
                .get_bpmn_model(process_def_id)
                .ok_or_else(|| {
                    FlowableError::ExecutionError(format!(
                        "No BPMN model found for process definition: {}",
                        process_def_id
                    ))
                })?;
            let process = model.main_process.as_ref().ok_or_else(|| {
                FlowableError::ExecutionError("No main process in BPMN model".into())
            })?;

            let flow_element =
                crate::agenda::continue_process_operation::find_flow_element(process, activity_id)
                    .ok_or_else(|| {
                        FlowableError::ExecutionError(format!(
                            "ScriptTask element '{}' not found in process model",
                            activity_id
                        ))
                    })?;

            match flow_element {
                flowable_bpmn_model::model::FlowElementEnum::ScriptTask(st) => (
                    st.script_format.clone(),
                    st.script.clone(),
                    st.result_variable.clone(),
                    st.auto_store_variables,
                    st.skip_expression.clone(),
                ),
                _ => {
                    return Err(FlowableError::ExecutionError(
                        "Activity is not a ScriptTask element".into(),
                    ));
                }
            }
        };

        let evaluation_execution =
            crate::engine::variable_service::evaluation_execution(command_context, execution);
        if crate::bpmn::skip_expression::should_skip_flow_element(
            skip_expression.as_deref(),
            "ScriptTask",
            evaluation_execution.activity_id.as_deref(),
            &evaluation_execution,
        )? {
            command_context
                .agenda
                .plan_take_outgoing_sequence_flows_operation(execution.clone());
            return Ok(());
        }

        let language = script_format.as_deref().unwrap_or("javascript");

        // Validate that the script task is acceptable for the current config
        validate_script_task(
            Some(language),
            config.enable_secure_scripting,
            &config.supported_script_languages,
        )?;

        // Build the secure context from the parent-chain-merged execution so
        // the script can resolve process-level variables on forked children.
        let proc_vars = evaluation_execution.process_variables();
        let mut context = SecureScriptContext::from_variables(proc_vars);

        // Execute through the secure engine
        let engine = SecureScriptEngine::new(config.supported_script_languages.clone());

        let script_text = script_body.as_deref().unwrap_or("");

        // P54b S1 / Java `ScriptTaskActivityBehavior#safelyExecuteScript` (lines 99–120):
        // after a script exception Java walks `ExceptionUtils.getRootCause` and, when the
        // root cause is `BpmnError`, routes it through `ErrorPropagation` so an error
        // boundary / event-subprocess can catch it.
        //
        // Prerequisite check (SecureScriptEngine / evaluator / parser): script failures
        // are only `FlowableError::ExecutionError(String)` with free-form messages. The
        // M9 sandbox has no `throw` / BpmnError host API and no structured error type
        // that carries an `errorCode`. Without that channel we cannot implement
        // rootCause-instanceof-BpmnError propagation without inventing a non-Java
        // script surface — so this path intentionally maps script errors straight to
        // `FlowableError` (no ErrorPropagation). Revisit when the script engine gains
        // a typed BpmnError (or equivalent) with errorCode.
        let result = engine.execute(language, script_text, &mut context)?;

        // Write result variable if configured
        if let Some(ref var_name) = result_variable
            && let Some(ref val) = result
        {
            execution.set_process_variable(var_name.clone(), val.clone());
        }

        // Auto-store all variables produced by the script
        if auto_store || result_variable.is_none() {
            let result_vars = context.into_result_variables();
            for (name, value) in result_vars {
                execution.set_process_variable(name, value);
            }
        }

        let activity_id = execution.activity_id.as_deref().unwrap_or("<unknown>");
        let details = format!(
            "Script task '{}' executed with language '{}'",
            activity_id, language
        );
        command_context.history_manager.record_audit_event(
            "script-task-executed",
            execution.process_instance_id.as_deref(),
            execution.process_definition_id.as_deref(),
            Some(&details),
            &mut command_context.session,
        );

        // Continue to outgoing sequence flows
        command_context
            .agenda
            .plan_take_outgoing_sequence_flows_operation(execution.clone());
        Ok(())
    }
}
