//! Start process instance with a start form as a single engine command.
//!
//! Java parity:
//! - `ProcessInstanceBuilderImpl.startWithForm()` →
//!   `StartProcessInstanceWithFormCmd` (flowable-engine/.../cmd/...)
//! - Form instance via `FormService.createFormInstanceWithScopeId(...)`
//!
//! Transaction design: process instance start, form instance insert, historic
//! form-property details and field submit hooks (content association) share
//! one command session and commit/roll back together. Any handler or storage
//! error rolls back the freshly started process instance too — no
//! service-layer compensation ("start then delete") is allowed.

use crate::handler::{FormFieldHandler, FormFieldSubmitContext};
use crate::models::{FormDefinition, FormInstance, FormProperty, build_form_value_bytes};
use crate::repository;
use flowable_engine::cmd::start_process_instance_cmd::StartProcessInstanceCmd;
use flowable_engine::error::FlowableError;
use flowable_engine::history::historic_entities::HistoricDetail;
use flowable_engine::interceptor::command::Command;
use flowable_engine::interceptor::command_context::CommandContext;
use flowable_engine::persistence::StorageError;
use flowable_engine::runtime::process_instance::ProcessInstance;
use flowable_engine::runtime::process_instance_builder::ProcessInstanceBuilder;
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::Arc;
use uuid::Uuid;

/// Input for starting a process instance from a submitted start form.
#[derive(Clone)]
pub struct StartProcessInstanceWithFormInput {
    pub process_definition_id: String,
    pub business_key: Option<String>,
    /// Pre-loaded start form definition metadata.
    pub form_definition: FormDefinition,
    /// Form field properties for handler dispatch (type + required metadata).
    pub form_properties: Vec<FormProperty>,
    /// Coerced submitted values (outcome variable already merged). Used as
    /// process start variables and stored on the form instance.
    pub values: BTreeMap<String, Value>,
    pub submitted_by: Option<String>,
    /// Registered field handlers (submit hooks). Empty skips lifecycle hooks.
    pub handlers: BTreeMap<String, Arc<dyn FormFieldHandler>>,
}

impl std::fmt::Debug for StartProcessInstanceWithFormInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StartProcessInstanceWithFormInput")
            .field("process_definition_id", &self.process_definition_id)
            .field("business_key", &self.business_key)
            .field("form_definition", &self.form_definition)
            .field("form_properties", &self.form_properties)
            .field("values", &self.values)
            .field("submitted_by", &self.submitted_by)
            .field("handlers_count", &self.handlers.len())
            .finish()
    }
}

/// Start process instance + persist start form instance in one session.
///
/// Java: `StartProcessInstanceWithFormCmd.execute`:
///   form model by id → getVariablesFromFormSubmission → start process
///   instance → createFormInstanceWithScopeId — all in one command.
pub struct StartProcessInstanceWithFormCmd {
    input: StartProcessInstanceWithFormInput,
}

impl StartProcessInstanceWithFormCmd {
    pub fn new(input: StartProcessInstanceWithFormInput) -> Self {
        Self { input }
    }
}

impl Command<(ProcessInstance, FormInstance)> for StartProcessInstanceWithFormCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<(ProcessInstance, FormInstance), FlowableError> {
        let input = &self.input;

        // Start the process instance inside this command (same session), the
        // same nested-command pattern CallActivityBehavior uses for child
        // process starts. A later failure rolls the whole start back.
        let mut builder = ProcessInstanceBuilder::new()
            .process_definition_id(input.process_definition_id.clone());
        if let Some(business_key) = input.business_key.clone() {
            builder = builder.business_key(business_key);
        }
        for (name, value) in &input.values {
            builder = builder.variable(name.clone(), value.clone());
        }
        let start_cmd = StartProcessInstanceCmd::new(builder);
        let process_instance = start_cmd.execute(command_context)?;

        // Form instance shares the session with the process start above.
        let (store, session) = command_context.store_and_session();
        let submitted_at = store.time_source().now().timestamp_millis();
        let form_values_id = format!("form-values:{}", Uuid::new_v4());
        let form_value_bytes = build_form_value_bytes(&input.values);
        let definition = &input.form_definition;
        let form_instance = FormInstance {
            id: format!("form-instance:{}", Uuid::new_v4()),
            form_definition_id: definition.id.clone(),
            form_definition_key: definition.key.clone(),
            form_definition_name: definition.name.clone(),
            deployment_id: definition.deployment_id.clone(),
            process_definition_id: Some(process_instance.process_definition_id.clone()),
            process_instance_id: Some(process_instance.id.clone()),
            task_id: None,
            scope_type: "start".to_string(),
            scope_id: process_instance.id.clone(),
            // For BPMN start forms, scope definition is the process definition.
            scope_definition_id: Some(process_instance.process_definition_id.clone()),
            submitted_at,
            submitted_by: input.submitted_by.clone(),
            // Tenant comes from the process instance started in this command
            // (inherited from the process definition).
            tenant_id: process_instance.tenant_id.clone(),
            form_values_id: Some(form_values_id),
            form_value_bytes: Some(form_value_bytes),
            outcome: None,
            values: input.values.clone(),
        };
        repository::insert_form_instance_in_session(session, &form_instance)
            .map_err(map_storage_error)?;

        // Historic form property details (Java history level AUDIT+), same
        // session as the process start and form instance.
        for (property_id, property_value) in &form_instance.values {
            let detail = HistoricDetail {
                id: Uuid::new_v4().to_string(),
                process_instance_id: process_instance.id.clone(),
                execution_id: None,
                activity_instance_id: None,
                task_id: None,
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

        // Java FormFieldHandler.handleFormFieldsOnSubmit — same session as
        // process start + form instance (ADR-4: transactional upload
        // association). A handler error rolls everything back.
        if !input.handlers.is_empty() && !input.values.is_empty() {
            let field_by_id: BTreeMap<&str, &FormProperty> = input
                .form_properties
                .iter()
                .map(|p| (p.id.as_str(), p))
                .collect();
            let process_instance_id = process_instance.id.clone();
            let process_definition_id = process_instance.process_definition_id.clone();
            let tenant_id = form_instance.tenant_id.clone();
            let submitted_by = form_instance.submitted_by.clone();

            // Re-borrow session for handler submit hooks.
            let (_store, session) = command_context.store_and_session();
            for (field_id, value) in &input.values {
                let Some(property) = field_by_id.get(field_id.as_str()).copied() else {
                    continue;
                };
                let normalized = property.field_type.trim().to_ascii_lowercase();
                let Some(handler) = input.handlers.get(&normalized) else {
                    continue;
                };
                let mut ctx = FormFieldSubmitContext {
                    task_id: None,
                    process_instance_id: Some(process_instance_id.as_str()),
                    scope_id: Some(process_instance_id.as_str()),
                    scope_type: Some("start"),
                    scope_definition_id: Some(process_definition_id.as_str()),
                    tenant_id: tenant_id.as_deref(),
                    user_id: submitted_by.as_deref(),
                    session,
                };
                handler.handle_submit(property, value, &mut ctx)?;
            }
        }

        Ok((process_instance, form_instance))
    }
}

fn map_storage_error(error: StorageError) -> FlowableError {
    FlowableError::Internal(format!("Database error: {}", error))
}
