//! Complete task with form as a single engine command.
//!
//! Java parity:
//! - `CompleteTaskWithFormCmd` (flowable-engine/.../cmd/CompleteTaskWithFormCmd.java)
//! - `TaskCompletionBuilderImpl.complete()` when `formDefinitionId != null`
//! - Form instance via `FormService.saveFormInstance(..., outcome)`
//!
//! Transaction design: form definition lookup, form instance insert, process
//! variables, historic form-property details, and task complete share one
//! command session and commit/roll back together (mirrors P2-ATTACHMENT
//! content-service session-backed pattern).

use crate::handler::{FormFieldHandler, FormFieldSubmitContext};
use crate::models::{FormDefinition, FormInstance, FormProperty, build_form_value_bytes};
use crate::repository;
use flowable_engine::engine::task_service::complete_task_by_id_in_context;
use flowable_engine::error::FlowableError;
use flowable_engine::history::historic_entities::HistoricDetail;
use flowable_engine::interceptor::command::Command;
use flowable_engine::interceptor::command_context::CommandContext;
use flowable_engine::persistence::StorageError;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use uuid::Uuid;

/// Injectable mid-command failure for rollback contract tests.
/// Java has no equivalent; forces an error after form instance is staged but
/// before task complete — both roll back with the session.
pub const FORCE_FAIL_FORM_OUTCOME: &str = "__flowable_force_fail_form__";

/// Input for completing a task with a form definition (REST complete + form submit).
#[derive(Clone)]
pub struct CompleteTaskWithFormInput {
    pub task_id: String,
    pub form_definition_id: String,
    pub outcome: Option<String>,
    /// Variables applied on task complete (Java `taskVariables` from
    /// `getVariablesFromFormSubmission`, including outcome variable).
    pub task_variables: HashMap<String, Value>,
    /// Raw submitted form values stored on the form instance
    /// (Java `saveFormInstance(formVariables, ...)`).
    pub form_instance_values: BTreeMap<String, Value>,
    pub local_scope: bool,
    pub transient_variables: HashMap<String, Value>,
    pub submitted_by: Option<String>,
    /// Pre-loaded definition metadata (id must match `form_definition_id`).
    /// When None, the command loads the definition from the session.
    pub form_definition: Option<FormDefinition>,
    pub process_definition_id: Option<String>,
    /// Form field properties for handler dispatch (type + required metadata).
    pub form_properties: Vec<FormProperty>,
    /// Registered field handlers (submit hooks). Empty skips lifecycle hooks.
    pub handlers: BTreeMap<String, Arc<dyn FormFieldHandler>>,
}

impl std::fmt::Debug for CompleteTaskWithFormInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CompleteTaskWithFormInput")
            .field("task_id", &self.task_id)
            .field("form_definition_id", &self.form_definition_id)
            .field("outcome", &self.outcome)
            .field("task_variables", &self.task_variables)
            .field("form_instance_values", &self.form_instance_values)
            .field("local_scope", &self.local_scope)
            .field("transient_variables", &self.transient_variables)
            .field("submitted_by", &self.submitted_by)
            .field("form_definition", &self.form_definition)
            .field("process_definition_id", &self.process_definition_id)
            .field("form_properties", &self.form_properties)
            .field("handlers_count", &self.handlers.len())
            .finish()
    }
}

/// Complete task + persist form instance (+ outcome) in one session.
///
/// Java: `CompleteTaskWithFormCmd.execute`:
///   form model by id → optional field validation → getVariablesFromFormSubmission
///   → saveFormInstance(..., outcome) → TaskHelper.completeTask(...)
pub struct CompleteTaskWithFormCmd {
    input: CompleteTaskWithFormInput,
}

impl CompleteTaskWithFormCmd {
    pub fn new(input: CompleteTaskWithFormInput) -> Self {
        Self { input }
    }
}

impl Command<FormInstance> for CompleteTaskWithFormCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<FormInstance, FlowableError> {
        let input = &self.input;

        // Java NeedsActiveTaskCmd: runtime task + not suspended.
        let (store, session) = command_context.store_and_session();
        let task = store.find_task(&input.task_id, session).ok_or_else(|| {
            FlowableError::NotFound(format!("No task found for task id {}", input.task_id))
        })?;
        if task.is_suspended() {
            // Java CompleteTaskWithFormCmd.getSuspendedTaskExceptionPrefix = "Cannot complete"
            return Err(FlowableError::ExecutionError(format!(
                "Cannot complete a suspended task '{}'",
                input.task_id
            )));
        }

        let process_instance_id = if task.process_instance_id.is_empty() {
            None
        } else {
            Some(task.process_instance_id.clone())
        };
        let process_definition_id = input
            .process_definition_id
            .clone()
            .or_else(|| {
                process_instance_id.as_ref().and_then(|pi_id| {
                    store
                        .find_process_instance(pi_id, session)
                        .map(|pi| pi.process_definition_id)
                })
            });

        // Java FormRepositoryService.getFormModelById — missing form model is
        // treated as null and form processing is skipped. Rust form-service
        // stores real definitions: explicit formDefinitionId that does not
        // exist is a client error (NotFound → REST 404), no partial side effects.
        let definition = if let Some(def) = input.form_definition.clone() {
            def
        } else {
            repository::find_form_definition_in_session(session, &input.form_definition_id)
                .map_err(map_storage_error)?
                .ok_or_else(|| {
                    FlowableError::NotFound(format!(
                        "Form definition '{}' was not found",
                        input.form_definition_id
                    ))
                })?
        };
        if definition.id != input.form_definition_id {
            return Err(FlowableError::BadRequest(format!(
                "Form definition id mismatch: expected '{}', got '{}'",
                input.form_definition_id, definition.id
            )));
        }

        let submitted_at = store.time_source().now().timestamp_millis();
        let tenant_id = task.tenant_id.clone().or_else(|| {
            process_instance_id.as_ref().and_then(|pi_id| {
                store
                    .find_process_instance(pi_id, session)
                    .and_then(|pi| pi.tenant_id)
            })
        });
        let form_values_id = format!("form-values:{}", Uuid::new_v4());
        let form_value_bytes = build_form_value_bytes(&input.form_instance_values);
        let form_instance = FormInstance {
            id: format!("form-instance:{}", Uuid::new_v4()),
            form_definition_id: definition.id.clone(),
            form_definition_key: definition.key.clone(),
            form_definition_name: definition.name.clone(),
            deployment_id: definition.deployment_id.clone(),
            process_definition_id: process_definition_id.clone(),
            process_instance_id: process_instance_id.clone(),
            task_id: Some(input.task_id.clone()),
            scope_type: "task".to_string(),
            scope_id: input.task_id.clone(),
            // BPMN task forms: scope definition is the process definition.
            scope_definition_id: process_definition_id.clone(),
            submitted_at,
            submitted_by: input.submitted_by.clone(),
            tenant_id,
            form_values_id: Some(form_values_id),
            form_value_bytes: Some(form_value_bytes),
            // Java saveFormInstance(..., outcome) — persist selected outcome.
            outcome: normalize_outcome_owned(input.outcome.clone()),
            values: input.form_instance_values.clone(),
        };

        // Re-borrow after building instance (same pattern as CreateTaskAttachmentCmd).
        let (store, session) = command_context.store_and_session();
        repository::insert_form_instance_in_session(session, &form_instance)
            .map_err(map_storage_error)?;

        // Historic form property details (Java history level AUDIT+).
        if let Some(pi_id) = process_instance_id.as_deref() {
            for (property_id, property_value) in &form_instance.values {
                let detail = HistoricDetail {
                    id: Uuid::new_v4().to_string(),
                    process_instance_id: pi_id.to_string(),
                    execution_id: None,
                    activity_instance_id: None,
                    task_id: Some(input.task_id.clone()),
                    time: store.time_source().now(),
                    detail_type: "formProperty".to_string(),
                    revision: None,
                    variable_name: None,
                    variable_type: None,
                    value: None,
                    property_id: Some(property_id.clone()),
                    property_value: Some(property_value.clone()),
                };
                store.insert_historic_detail(detail, session);
            }
        }

        // Java FormFieldHandler.handleFormFieldsOnSubmit — same session as
        // form instance + task complete (ADR-4: transactional upload association).
        if !input.handlers.is_empty() && !input.form_instance_values.is_empty() {
            let field_by_id: BTreeMap<&str, &FormProperty> = input
                .form_properties
                .iter()
                .map(|p| (p.id.as_str(), p))
                .collect();
            let scope_type = "task";
            let scope_id = input.task_id.as_str();
            let tenant_id = form_instance.tenant_id.clone();
            let submitted_by = form_instance.submitted_by.clone();
            let process_instance_id_owned = process_instance_id.clone();
            let process_definition_id_owned = process_definition_id.clone();

            // Re-borrow session for handler submit hooks.
            let (_store, session) = command_context.store_and_session();
            for (field_id, value) in &input.form_instance_values {
                let Some(property) = field_by_id.get(field_id.as_str()).copied() else {
                    continue;
                };
                let normalized = property.field_type.trim().to_ascii_lowercase();
                let Some(handler) = input.handlers.get(&normalized) else {
                    continue;
                };
                let mut ctx = FormFieldSubmitContext {
                    task_id: Some(input.task_id.as_str()),
                    process_instance_id: process_instance_id_owned.as_deref(),
                    scope_id: Some(scope_id),
                    scope_type: Some(scope_type),
                    scope_definition_id: process_definition_id_owned.as_deref(),
                    tenant_id: tenant_id.as_deref(),
                    user_id: submitted_by.as_deref(),
                    session,
                };
                handler.handle_submit(property, value, &mut ctx)?;
            }
        }

        // Injectable mid-command failure: form instance staged but not committed.
        if input.outcome.as_deref() == Some(FORCE_FAIL_FORM_OUTCOME) {
            return Err(FlowableError::BadRequest(
                "Forced form complete failure".to_string(),
            ));
        }

        // Java TaskHelper.completeTask after saveFormInstance (same command).
        complete_task_by_id_in_context(
            command_context,
            &input.task_id,
            &input.task_variables,
            &input.transient_variables,
            input.local_scope,
        )?;

        Ok(form_instance)
    }
}

fn normalize_outcome_owned(outcome: Option<String>) -> Option<String> {
    outcome.and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

fn map_storage_error(error: StorageError) -> FlowableError {
    FlowableError::Internal(format!("Database error: {}", error))
}
