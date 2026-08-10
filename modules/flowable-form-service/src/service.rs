use crate::field_types::{self, FormFieldCategory};
use crate::handler::{FormFieldEnrichContext, FormFieldHandler, default_handlers};
use crate::models::{
    BaseFormField, ExpressionFormField, FormContainer, FormData, FormDefinition, FormDeployment,
    FormDeploymentRequest, FormEnumValue, FormFieldModel, FormInstance, FormOption,
    FormOutcome, FormProperty, FormSubmissionProperty, FormSubmissionRequest, FormSubmissionResult,
    LayoutDefinition, OptionFormField, form_instance_values_bytes,
};
use crate::query::{FormDefinitionQuery, FormInstanceQuery};
use crate::repository;
use crate::start_form::{StartProcessInstanceWithFormCmd, StartProcessInstanceWithFormInput};
use crate::task_form::{CompleteTaskWithFormCmd, CompleteTaskWithFormInput};
use flowable_bpmn_model::model::{BpmnModel, FlowElementEnum};
use flowable_content_service::repository as content_repository;
use flowable_engine::engine::process_engine::ProcessEngine;
use flowable_engine::error::FlowableError;
use flowable_engine::interceptor::command_executor::CommandExecutor;
use flowable_engine::task::Task;
use flowable_engine_common::el::{
    Expression, MapVariableContainer, SimpleExpression, evaluate_composite_expression,
};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;
use uuid::Uuid;

#[derive(Clone)]
pub struct FlowableFormService {
    engine: Arc<ProcessEngine>,
    handlers: BTreeMap<String, Arc<dyn FormFieldHandler>>,
}

impl FlowableFormService {
    pub fn new(engine: Arc<ProcessEngine>) -> Self {
        repository::ensure_schema(&engine.get_runtime_store());
        Self {
            engine,
            handlers: default_handlers(),
        }
    }

    /// 使用自定义 handler 集合构造服务。
    /// 传入的 handlers 会与默认 handler 合并（自定义 handler 优先）。
    pub fn with_handlers(
        engine: Arc<ProcessEngine>,
        custom_handlers: BTreeMap<String, Arc<dyn FormFieldHandler>>,
    ) -> Self {
        repository::ensure_schema(&engine.get_runtime_store());
        let mut handlers = default_handlers();
        // A custom handler registered under a compatibility alias also owns
        // that alias's canonical runtime route (for example `amount` routes
        // through `decimal`, and `multi-line-text` through `text`).
        for (field_type, handler) in custom_handlers {
            if let Some(runtime_type) = field_types::runtime_handler_type(&field_type) {
                handlers.insert(runtime_type.to_string(), Arc::clone(&handler));
            }
            handlers.insert(field_type, handler);
        }
        Self { engine, handlers }
    }

    /// 返回当前注册的所有 handler 的只读引用。
    pub fn get_handlers(&self) -> &BTreeMap<String, Arc<dyn FormFieldHandler>> {
        &self.handlers
    }

    pub fn deploy(&self, request: FormDeploymentRequest) -> Result<FormDeployment, FlowableError> {
        if request.name.trim().is_empty() {
            return Err(FlowableError::DeploymentValidationError(
                "Form deployment name is required".to_string(),
            ));
        }
        if request.resources.is_empty() {
            return Err(FlowableError::DeploymentValidationError(
                "Form deployment resources are required".to_string(),
            ));
        }

        let mut parsed_resources = request
            .resources
            .into_iter()
            .map(parse_form_resource)
            .collect::<Result<Vec<_>, _>>()?;
        parsed_resources.sort_by(|left, right| left.resource_name.cmp(&right.resource_name));

        let store = self.engine.get_runtime_store();
        let deployment_id = format!("form-deployment:{}", Uuid::new_v4());
        let deployed_at = store.time_source().now().timestamp_millis();
        let deployment = FormDeployment {
            id: deployment_id.clone(),
            name: request.name,
            deployed_at,
            resource_names: parsed_resources
                .iter()
                .map(|resource| resource.resource_name.clone())
                .collect(),
        };

        repository::insert_form_deployment(&store, deployment.clone());
        for resource in parsed_resources {
            let current_version = repository::list_form_definitions_by_key(&store, &resource.key)
                .into_iter()
                .map(|item| item.version)
                .max()
                .unwrap_or(0);
            repository::insert_form_definition(
                &store,
                FormDefinition {
                    id: format!("{}:{}", deployment_id, resource.resource_name),
                    deployment_id: deployment_id.clone(),
                    key: resource.key,
                    name: resource.name,
                    description: resource.description,
                    version: current_version + 1,
                    resource_name: resource.resource_name,
                    form_payload: resource.form_payload,
                    outcomes: resource.outcomes,
                    outcome_variable_name: resource.outcome_variable_name,
                    layout: resource.layout,
                    active: Some(true),
                },
            );
        }

        Ok(deployment)
    }

    pub fn create_form_definition_query(&self) -> FormDefinitionQuery {
        FormDefinitionQuery::new(Arc::clone(&self.engine))
    }

    pub fn create_form_instance_query(&self) -> FormInstanceQuery {
        FormInstanceQuery::new(Arc::clone(&self.engine))
    }

    pub fn get_form_definition(
        &self,
        form_definition_id: &str,
    ) -> Result<FormDefinition, FlowableError> {
        let store = self.engine.get_runtime_store();
        repository::find_form_definition(&store, form_definition_id).ok_or_else(|| {
            FlowableError::NotFound(format!(
                "Form definition '{}' was not found",
                form_definition_id
            ))
        })
    }

    pub fn get_form_instance(&self, form_instance_id: &str) -> Result<FormInstance, FlowableError> {
        let store = self.engine.get_runtime_store();
        repository::find_form_instance(&store, form_instance_id).ok_or_else(|| {
            FlowableError::NotFound(format!(
                "Form instance '{}' was not found",
                form_instance_id
            ))
        })
    }

    /// Java `FormService.getFormInstanceValues` — returns canonical value bytes.
    ///
    /// For legacy rows without stored `form_value_bytes`, derives bytes from the
    /// typed `values` map without mutating storage.
    pub fn get_form_instance_values(
        &self,
        form_instance_id: &str,
    ) -> Result<Vec<u8>, FlowableError> {
        let instance = self.get_form_instance(form_instance_id)?;
        Ok(form_instance_values_bytes(&instance))
    }

    /// Java `FormService.deleteFormInstance`.
    pub fn delete_form_instance(&self, form_instance_id: &str) -> Result<(), FlowableError> {
        let store = self.engine.get_runtime_store();
        let deleted = repository::delete_form_instance(&store, form_instance_id)?;
        if !deleted {
            return Err(FlowableError::NotFound(format!(
                "Form instance '{}' was not found",
                form_instance_id
            )));
        }
        Ok(())
    }

    /// Java `FormService.deleteFormInstancesByFormDefinition`.
    pub fn delete_form_instances_by_form_definition(
        &self,
        form_definition_id: &str,
    ) -> Result<usize, FlowableError> {
        let store = self.engine.get_runtime_store();
        repository::delete_form_instances_by_form_definition(&store, form_definition_id)
    }

    /// Java `FormService.deleteFormInstancesByProcessDefinition`.
    pub fn delete_form_instances_by_process_definition(
        &self,
        process_definition_id: &str,
    ) -> Result<usize, FlowableError> {
        let store = self.engine.get_runtime_store();
        repository::delete_form_instances_by_process_definition(&store, process_definition_id)
    }

    /// Java `FormService.deleteFormInstancesByScopeDefinition`.
    pub fn delete_form_instances_by_scope_definition(
        &self,
        scope_definition_id: &str,
    ) -> Result<usize, FlowableError> {
        let store = self.engine.get_runtime_store();
        repository::delete_form_instances_by_scope_definition(&store, scope_definition_id)
    }

    pub fn get_start_form_data(
        &self,
        process_definition_id: &str,
    ) -> Result<FormData, FlowableError> {
        let process_definition = self
            .engine
            .get_repository_service()
            .get_process_definition(process_definition_id)?;
        let model = self
            .engine
            .get_repository_service()
            .get_bpmn_model(process_definition_id)?;
        let form_key = find_start_form_key(&model).ok_or_else(|| {
            FlowableError::NotFound(format!(
                "Process definition '{}' does not have a start form",
                process_definition_id
            ))
        })?;
        let definition = latest_form_definition_by_key(&self.engine, &form_key)?;

        let mut form_data = build_form_data(
            definition,
            Some(process_definition.id),
            None,
            BTreeMap::new(),
        )?;
        self.enrich_form_data(&mut form_data)?;
        Ok(form_data)
    }

    pub fn get_task_form_data(&self, task_id: &str) -> Result<FormData, FlowableError> {
        let store = self.engine.get_runtime_store();
        // Scope the read session so its BEGIN IMMEDIATE lock is released (via
        // rollback + drop) before calling methods that create their own sessions
        // (get_bpmn_model, latest_form_definition_by_key, load_task_values).
        // SQLite shared-cache mode holds a table-level write lock from BEGIN
        // IMMEDIATE even for reads, which would block those nested sessions.
        let (task, process_instance) = {
            let mut session = store.create_session().unwrap();
            let task = store.find_task(task_id, &mut session).ok_or_else(|| {
                FlowableError::NotFound(format!("Task '{}' was not found", task_id))
            })?;
            let process_instance = store
                .find_process_instance(&task.process_instance_id, &mut session)
                .ok_or_else(|| {
                    FlowableError::NotFound(format!(
                        "Process instance '{}' was not found for task '{}'",
                        task.process_instance_id, task.id
                    ))
                })?;
            session.rollback().ok();
            (task, process_instance)
        };
        let model = self
            .engine
            .get_repository_service()
            .get_bpmn_model(&process_instance.process_definition_id)?;
        let form_key = find_task_form_key(&model, &task.task_definition_key).ok_or_else(|| {
            FlowableError::NotFound(format!("Task '{}' does not have a form", task_id))
        })?;
        let definition = latest_form_definition_by_key(&self.engine, &form_key)?;
        let values = load_task_values(&self.engine, &task)?;

        let mut form_data = build_form_data(
            definition,
            Some(process_instance.process_definition_id),
            Some(task.id),
            values,
        )?;
        self.enrich_form_data(&mut form_data)?;
        Ok(form_data)
    }

    pub fn submit_form(
        &self,
        request: FormSubmissionRequest,
    ) -> Result<FormSubmissionResult, FlowableError> {
        self.submit_form_internal(request, None)
    }

    pub fn submit_form_as(
        &self,
        request: FormSubmissionRequest,
        submitted_by: impl Into<String>,
    ) -> Result<FormSubmissionResult, FlowableError> {
        self.submit_form_internal(request, Some(submitted_by.into()))
    }

    fn submit_form_internal(
        &self,
        request: FormSubmissionRequest,
        submitted_by: Option<String>,
    ) -> Result<FormSubmissionResult, FlowableError> {
        let submitted_by = normalize_submitted_by(submitted_by);
        match (
            request.process_definition_id.as_deref(),
            request.task_id.as_deref(),
        ) {
            (Some(process_definition_id), None) => {
                let form_data = self.get_start_form_data(process_definition_id)?;
                let mut values = self.validate_and_coerce_properties(
                    &form_data.form_properties,
                    request.properties,
                )?;
                self.apply_outcome_variable(&form_data, request.outcome, &mut values)?;
                // Java StartProcessInstanceWithFormCmd — process start, form
                // instance, historic details and submit hooks in one command
                // so any failure rolls back the whole submission atomically.
                let definition = self.get_form_definition(&form_data.form_definition_id)?;
                let cmd = StartProcessInstanceWithFormCmd::new(StartProcessInstanceWithFormInput {
                    process_definition_id: process_definition_id.to_string(),
                    business_key: request.business_key,
                    form_definition: definition,
                    form_properties: form_data.form_properties.clone(),
                    values,
                    submitted_by,
                    handlers: self.handlers.clone(),
                });
                let (process_instance, _form_instance) =
                    self.engine.get_command_executor().execute(&cmd)?;
                Ok(FormSubmissionResult::ProcessInstance(process_instance))
            }
            (None, Some(task_id)) => {
                let form_data = self.get_task_form_data(task_id)?;
                let mut values = self.validate_and_coerce_properties(
                    &form_data.form_properties,
                    request.properties,
                )?;
                let outcome = request.outcome.clone();
                self.apply_outcome_variable(&form_data, request.outcome, &mut values)?;
                let form_instance = self.complete_task_with_form(
                    task_id,
                    &form_data,
                    submitted_by,
                    values,
                    outcome,
                )?;
                Ok(FormSubmissionResult::TaskCompleted(form_instance))
            }
            (Some(_), Some(_)) => Err(FlowableError::DeploymentValidationError(
                "Submit form request must target either processDefinitionId or taskId".to_string(),
            )),
            (None, None) => Err(FlowableError::DeploymentValidationError(
                "Submit form request requires processDefinitionId or taskId".to_string(),
            )),
        }
    }

    fn complete_task_with_form(
        &self,
        task_id: &str,
        form_data: &FormData,
        submitted_by: Option<String>,
        values: BTreeMap<String, Value>,
        outcome: Option<String>,
    ) -> Result<FormInstance, FlowableError> {
        // Java CompleteTaskWithFormCmd — single command for variables + form
        // instance + task complete + field submit hooks (see task_form.rs).
        let definition = self.get_form_definition(&form_data.form_definition_id)?;
        let task_variables: HashMap<String, Value> =
            values.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        let cmd = CompleteTaskWithFormCmd::new(CompleteTaskWithFormInput {
            task_id: task_id.to_string(),
            form_definition_id: form_data.form_definition_id.clone(),
            outcome,
            task_variables,
            form_instance_values: values,
            local_scope: false,
            transient_variables: HashMap::new(),
            submitted_by,
            form_definition: Some(definition),
            process_definition_id: form_data.process_definition_id.clone(),
            form_properties: form_data.form_properties.clone(),
            handlers: self.handlers.clone(),
        });
        self.engine.get_command_executor().execute(&cmd)
    }

    /// Complete a runtime task with an explicit form definition id (REST
    /// `action: complete` + `formDefinitionId`).
    ///
    /// Java: `TaskResource.completeTask` → `TaskCompletionBuilder.formDefinitionId`
    /// → `CompleteTaskWithFormCmd`.
    pub fn complete_task_with_form_definition(
        &self,
        task_id: impl Into<String>,
        form_definition_id: impl Into<String>,
        outcome: Option<String>,
        variables: HashMap<String, Value>,
        local_scope: bool,
        transient_variables: HashMap<String, Value>,
        submitted_by: Option<String>,
    ) -> Result<FormInstance, FlowableError> {
        let task_id = task_id.into();
        let form_definition_id = form_definition_id.into();
        let definition = self.get_form_definition(&form_definition_id)?;

        // Build form properties from definition for type coercion when fields
        // are declared; undeclared variables pass through (Java form engine
        // returns processed maps from getVariablesFromFormSubmission).
        let (form_properties, _, _, _) =
            parse_form_payload(&definition.form_payload, &BTreeMap::new())?;

        let properties: Vec<FormSubmissionProperty> = if form_properties.is_empty() {
            variables
                .into_iter()
                .map(|(id, value)| FormSubmissionProperty { id, value })
                .collect()
        } else {
            // Only known writable fields go through handlers; extra variables
            // are kept as-is (REST may send mixed process variables).
            let known: BTreeSet<_> = form_properties.iter().map(|p| p.id.as_str()).collect();
            let mut known_props = Vec::new();
            let mut passthrough = BTreeMap::new();
            for (id, value) in variables {
                if known.contains(id.as_str()) {
                    known_props.push(FormSubmissionProperty { id, value });
                } else {
                    passthrough.insert(id, value);
                }
            }
            let mut coerced = self.validate_and_coerce_properties(&form_properties, known_props)?;
            coerced.extend(passthrough);
            // Re-materialize as properties for outcome application path below.
            coerced
                .into_iter()
                .map(|(id, value)| FormSubmissionProperty { id, value })
                .collect()
        };

        let mut values: BTreeMap<String, Value> =
            properties.into_iter().map(|p| (p.id, p.value)).collect();

        // Form instance stores submitted values before outcome variable is merged
        // (Java: saveFormInstance(formVariables) vs complete with taskVariables).
        let form_instance_values = values.clone();

        let form_data = FormData {
            form_definition_id: definition.id.clone(),
            form_key: Some(definition.key.clone()),
            deployment_id: definition.deployment_id.clone(),
            process_definition_id: None,
            task_id: Some(task_id.clone()),
            form_properties,
            outcomes: definition.outcomes.clone(),
            layout: definition.layout.clone(),
            form_fields: None,
        };
        self.apply_outcome_variable(&form_data, outcome.clone(), &mut values)?;

        let task_variables: HashMap<String, Value> =
            values.iter().map(|(k, v)| (k.clone(), v.clone())).collect();

        let form_properties = form_data.form_properties.clone();
        let cmd = CompleteTaskWithFormCmd::new(CompleteTaskWithFormInput {
            task_id,
            form_definition_id,
            outcome,
            task_variables,
            form_instance_values,
            local_scope,
            transient_variables,
            submitted_by,
            form_definition: Some(definition),
            process_definition_id: None,
            form_properties,
            handlers: self.handlers.clone(),
        });
        self.engine.get_command_executor().execute(&cmd)
    }

    /// Enrich form field values for read paths (Java `enrichFormFields`).
    ///
    /// Replaces stored upload ids with content metadata in the returned
    /// `FormData` without changing persisted form values.
    fn enrich_form_data(&self, form_data: &mut FormData) -> Result<(), FlowableError> {
        // Collect content ids referenced by upload fields.
        let mut content_ids = BTreeSet::new();
        for property in &form_data.form_properties {
            let normalized = normalize_field_type(&property.field_type);
            if normalized != "upload" {
                continue;
            }
            if let Some(value) = property.value.as_ref() {
                if let Ok(ids) =
                    crate::handler::UploadFieldHandler::parse_content_item_ids(value)
                {
                    content_ids.extend(ids);
                }
            }
        }
        if content_ids.is_empty() {
            return Ok(());
        }

        let store = self.engine.get_runtime_store();
        let mut session = store.db_store().create_session().map_err(|e| {
            FlowableError::Internal(format!("Database error: {e}"))
        })?;
        let id_list: Vec<String> = content_ids.into_iter().collect();
        let items = content_repository::find_content_items_by_ids_in_session(&mut session, &id_list)
            .map_err(|e| FlowableError::Internal(format!("Database error: {e}")))?;
        session.rollback().ok();

        let content_by_id: BTreeMap<String, _> =
            items.into_iter().map(|item| (item.id.clone(), item)).collect();
        let ctx = FormFieldEnrichContext {
            content_by_id: &content_by_id,
        };

        for property in &mut form_data.form_properties {
            let normalized = normalize_field_type(&property.field_type);
            let Some(handler) = self.handlers.get(&normalized) else {
                continue;
            };
            if let Some(value) = property.value.as_ref() {
                property.value = Some(handler.enrich_on_read(property, value, &ctx)?);
            }
        }
        Ok(())
    }



    fn apply_outcome_variable(
        &self,
        form_data: &FormData,
        outcome: Option<String>,
        values: &mut BTreeMap<String, Value>,
    ) -> Result<(), FlowableError> {
        let Some(outcome) = normalize_outcome(outcome) else {
            return Ok(());
        };
        let definition = self.get_form_definition(&form_data.form_definition_id)?;
        let variable_name = definition
            .outcome_variable_name
            .as_deref()
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| format!("form_{}_outcome", definition.key));
        values.insert(variable_name, Value::String(outcome));
        Ok(())
    }
}

struct ParsedFormDefinition {
    key: String,
    name: String,
    description: Option<String>,
    resource_name: String,
    form_payload: Value,
    outcomes: Option<Vec<FormOutcome>>,
    outcome_variable_name: Option<String>,
    layout: Option<Value>,
}

fn parse_form_resource(
    resource: crate::models::FormDeploymentResource,
) -> Result<ParsedFormDefinition, FlowableError> {
    if !resource.resource_name.ends_with(".form") {
        return Err(FlowableError::DeploymentValidationError(format!(
            "Unsupported form resource '{}'",
            resource.resource_name
        )));
    }

    let value: Value = serde_json::from_str(&resource.resource).map_err(|error| {
        FlowableError::DeploymentValidationError(format!(
            "Form resource '{}' is not valid JSON: {}",
            resource.resource_name, error
        ))
    })?;
    let object = value.as_object().ok_or_else(|| {
        FlowableError::DeploymentValidationError(format!(
            "Form resource '{}' must be a JSON object",
            resource.resource_name
        ))
    })?;

    validate_resource_name_field(object, &resource.resource_name)?;

    let key = required_string(object, "key", &resource.resource_name)?;
    let name = required_string(object, "name", &resource.resource_name)?;
    let description = optional_string(object, "description");
    let outcome_variable_name = optional_string(object, "outcomeVariableName");
    let outcomes = parse_form_outcomes(&value)?;
    let layout = value.get("layout").cloned();

    // Deployment is deliberately lenient about field types: Java 6.8 parses
    // them generically and defers rejection to runtime, so vendor types must
    // deploy cleanly here. Strict contract validation lives at the modeler
    // boundary (`validate_form_model`).

    Ok(ParsedFormDefinition {
        key,
        name,
        description,
        resource_name: resource.resource_name,
        form_payload: value,
        outcomes,
        outcome_variable_name,
        layout,
    })
}

fn latest_form_definition_by_key(
    engine: &Arc<ProcessEngine>,
    form_key: &str,
) -> Result<FormDefinition, FlowableError> {
    let store = engine.get_runtime_store();
    repository::list_form_definitions_by_key(&store, form_key)
        .into_iter()
        .filter(|d| d.active.unwrap_or(true))
        .max_by(|left, right| {
            left.version
                .cmp(&right.version)
                .then(left.id.cmp(&right.id))
        })
        .ok_or_else(|| {
            FlowableError::NotFound(format!(
                "No active form definition was found for key '{}'",
                form_key
            ))
        })
}

fn build_form_data(
    definition: FormDefinition,
    process_definition_id: Option<String>,
    task_id: Option<String>,
    values: BTreeMap<String, Value>,
) -> Result<FormData, FlowableError> {
    let (form_properties, form_fields, outcomes, layout) =
        parse_form_payload(&definition.form_payload, &values)?;

    Ok(FormData {
        form_definition_id: definition.id.clone(),
        form_key: Some(definition.key.clone()),
        deployment_id: definition.deployment_id,
        process_definition_id,
        task_id,
        form_properties,
        outcomes,
        layout,
        form_fields,
    })
}

/// Parse the full form payload, returning (form_properties, form_fields, outcomes, layout).
#[allow(clippy::type_complexity)]
fn parse_form_payload(
    payload: &Value,
    values: &BTreeMap<String, Value>,
) -> Result<
    (
        Vec<FormProperty>,
        Option<Vec<FormFieldModel>>,
        Option<Vec<FormOutcome>>,
        Option<serde_json::Value>,
    ),
    FlowableError,
> {
    let fields = payload
        .get("fields")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    let mut form_properties = Vec::new();
    collect_flat_form_properties(&fields, values, &mut form_properties)?;

    let form_fields = if fields.iter().any(|f| f.get("fieldType").is_some()) {
        let mut parsed: Vec<FormFieldModel> = fields
            .iter()
            .map(parse_form_field_model)
            .collect::<Result<Vec<_>, _>>()?;
        let scope = MapVariableContainer::from_map(
            values
                .iter()
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect(),
        );
        enrich_form_field_values(&mut parsed, values, &scope);
        Some(parsed)
    } else {
        None
    };

    let outcomes = parse_form_outcomes(payload)?;

    let layout = payload.get("layout").cloned();

    Ok((form_properties, form_fields, outcomes, layout))
}

fn collect_flat_form_properties(
    fields: &[Value],
    values: &BTreeMap<String, Value>,
    properties: &mut Vec<FormProperty>,
) -> Result<(), FlowableError> {
    for field in fields {
        let object = field.as_object().ok_or_else(|| {
            FlowableError::DeploymentValidationError(
                "Form field definitions must be JSON objects".to_string(),
            )
        })?;
        let field_type = required_string(object, "type", "fields[]")?;
        let capability = field_types::form_field_capability(&field_type);
        let category = capability.map(|value| value.category);

        if !matches!(
            category,
            Some(
                FormFieldCategory::Expression
                    | FormFieldCategory::Container
                    | FormFieldCategory::Display
            )
        ) {
            properties.push(parse_flat_form_property(field, values)?);
        }

        if matches!(category, Some(FormFieldCategory::Container)) {
            if let Some(rows) = object.get("fields").and_then(Value::as_array) {
                for row in rows {
                    let nested = row.as_array().ok_or_else(|| {
                        FlowableError::DeploymentValidationError(
                            "Form container rows must be JSON arrays".to_string(),
                        )
                    })?;
                    collect_flat_form_properties(nested, values, properties)?;
                }
            }
        }
    }
    Ok(())
}

fn enrich_form_field_values(
    fields: &mut [FormFieldModel],
    values: &BTreeMap<String, Value>,
    scope: &MapVariableContainer,
) {
    for field in fields {
        match field {
            FormFieldModel::Container(container) => {
                for row in &mut container.fields {
                    enrich_form_field_values(row, values, scope);
                }
            }
            FormFieldModel::OptionField(field) => {
                field.base.value = values.get(&field.base.id).cloned();
            }
            FormFieldModel::ExpressionField(field) => {
                field.base.value = evaluate_form_expression(&field.expression, scope);
            }
            FormFieldModel::BaseField(field) => {
                if field.field_type.as_deref() == Some(field_types::HYPERLINK)
                    && let Some(url) = field
                        .params
                        .as_ref()
                        .and_then(|params| params.get("hyperlinkUrl"))
                {
                    field.value = evaluate_form_expression(url, scope);
                } else {
                    field.value = values.get(&field.id).cloned();
                }
            }
        }
    }
}

fn evaluate_form_expression(expression: &str, scope: &MapVariableContainer) -> Option<Value> {
    let expression = expression.trim();
    if is_single_uel_expression(expression) {
        SimpleExpression::new(expression.to_string()).get_value(scope)
    } else if expression.contains("${") {
        Some(Value::String(evaluate_composite_expression(
            expression, scope,
        )))
    } else if expression.is_empty() {
        None
    } else {
        Some(Value::String(expression.to_string()))
    }
}

fn is_single_uel_expression(expression: &str) -> bool {
    let bytes = expression.as_bytes();
    if bytes.len() < 3 || !expression.starts_with("${") {
        return false;
    }
    let mut depth = 1usize;
    for (index, byte) in bytes.iter().enumerate().skip(2) {
        match byte {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return index == bytes.len() - 1;
                }
            }
            _ => {}
        }
    }
    false
}

fn parse_form_outcomes(payload: &Value) -> Result<Option<Vec<FormOutcome>>, FlowableError> {
    payload
        .get("outcomes")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .map(|item| {
                    let obj = item.as_object();
                    Ok(FormOutcome {
                        id: obj
                            .and_then(|o| o.get("id"))
                            .and_then(|v| v.as_str())
                            .map(str::to_string),
                        name: obj
                            .and_then(|o| o.get("name"))
                            .and_then(|v| v.as_str())
                            .map(str::to_string),
                    })
                })
                .collect::<Result<Vec<_>, FlowableError>>()
        })
        .transpose()
}

/// Parse a single field into a flat FormProperty.
fn parse_flat_form_property(
    field: &Value,
    values: &BTreeMap<String, Value>,
) -> Result<FormProperty, FlowableError> {
    let object = field.as_object().ok_or_else(|| {
        FlowableError::DeploymentValidationError(
            "Form field definitions must be JSON objects".to_string(),
        )
    })?;
    let id = required_string(object, "id", "fields[]")?;
    let field_type = required_string(object, "type", "fields[]")?;

    Ok(FormProperty {
        id: id.clone(),
        name: optional_string(object, "name"),
        field_type,
        value: values.get(&id).cloned(),
        readable: object
            .get("readable")
            .and_then(Value::as_bool)
            .unwrap_or(true),
        writable: !object
            .get("readOnly")
            .and_then(Value::as_bool)
            .unwrap_or(false)
            && object
                .get("writable")
                .and_then(Value::as_bool)
                .unwrap_or(true),
        required: object
            .get("required")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        date_pattern: optional_string(object, "datePattern"),
        enum_values: parse_enum_values(object),
    })
}

/// Parse a single field into a FormFieldModel, handling nested containers recursively.
fn parse_form_field_model(field: &Value) -> Result<FormFieldModel, FlowableError> {
    let object = field.as_object().ok_or_else(|| {
        FlowableError::DeploymentValidationError(
            "Form field definitions must be JSON objects".to_string(),
        )
    })?;

    let field_type = object
        .get("fieldType")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    match field_type {
        "Container" => {
            let base = parse_base_form_field(object)?;
            let nested_fields = object
                .get("fields")
                .and_then(|v| v.as_array())
                .map(|rows| {
                    rows.iter()
                        .map(|row| {
                            row.as_array()
                                .map(|cols| {
                                    cols.iter()
                                        .map(parse_form_field_model)
                                        .collect::<Result<Vec<_>, _>>()
                                })
                                .unwrap_or_else(|| Ok(Vec::new()))
                        })
                        .collect::<Result<Vec<_>, _>>()
                })
                .transpose()?
                .unwrap_or_default();

            Ok(FormFieldModel::Container(FormContainer {
                base,
                fields: nested_fields,
            }))
        }
        "OptionFormField" => {
            let base = parse_base_form_field(object)?;
            let options = object
                .get("options")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .map(|item| {
                            let obj = item.as_object();
                            Ok(FormOption {
                                id: obj
                                    .and_then(|o| o.get("id"))
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                                    .to_string(),
                                name: obj
                                    .and_then(|o| o.get("name"))
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                                    .to_string(),
                            })
                        })
                        .collect::<Result<Vec<_>, FlowableError>>()
                })
                .transpose()?
                .unwrap_or_default();

            Ok(FormFieldModel::OptionField(OptionFormField {
                base,
                option_type: optional_string(object, "optionType"),
                has_empty_value: object
                    .get("hasEmptyValue")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false),
                options,
                options_expression: optional_string(object, "optionsExpression"),
            }))
        }
        "ExpressionFormField" => {
            let base = parse_base_form_field(object)?;
            let expression = object
                .get("expression")
                .and_then(|v| v.as_str())
                .map(str::to_string)
                .unwrap_or_default();

            Ok(FormFieldModel::ExpressionField(ExpressionFormField {
                base,
                expression,
            }))
        }
        "BaseField" => {
            let base = parse_base_form_field(object)?;
            Ok(FormFieldModel::BaseField(base))
        }
        "" => Err(FlowableError::DeploymentValidationError(
            "Form field is missing required 'fieldType' property".to_string(),
        )),
        other => Err(FlowableError::DeploymentValidationError(format!(
            "Unknown form field fieldType '{}'",
            other
        ))),
    }
}

/// Parse common base field properties from a JSON object.
fn parse_base_form_field(object: &Map<String, Value>) -> Result<BaseFormField, FlowableError> {
    let id = required_string(object, "id", "fields[]")?;
    let layout = object
        .get("layout")
        .and_then(Value::as_object)
        .map(|layout| LayoutDefinition {
            row: layout.get("row").and_then(Value::as_i64).map(|n| n as i32),
            col: layout.get("col").and_then(Value::as_i64).map(|n| n as i32),
            col_span: layout
                .get("colSpan")
                .and_then(Value::as_i64)
                .map(|n| n as i32),
        });

    let params = object.get("params").and_then(|v| v.as_object()).map(|obj| {
        obj.iter()
            .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
            .collect::<HashMap<String, String>>()
    });

    Ok(BaseFormField {
        id,
        name: optional_string(object, "name"),
        field_type: optional_string(object, "type"),
        value: None,
        readable: object.get("readable").and_then(|v| v.as_bool()),
        writable: object.get("writable").and_then(|v| v.as_bool()),
        required: object.get("required").and_then(|v| v.as_bool()),
        read_only: object.get("readOnly").and_then(|v| v.as_bool()),
        placeholder: optional_string(object, "placeholder"),
        params,
        layout,
        date_pattern: optional_string(object, "datePattern"),
        enum_values: parse_enum_values(object),
    })
}

fn parse_enum_values(object: &Map<String, Value>) -> Vec<FormEnumValue> {
    object
        .get("enumValues")
        .or_else(|| object.get("options"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_object)
        .filter_map(|item| {
            Some(FormEnumValue {
                id: item.get("id")?.as_str()?.to_string(),
                name: item
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
            })
        })
        .collect()
}

impl FlowableFormService {
    fn validate_and_coerce_properties(
        &self,
        form_properties: &[FormProperty],
        properties: Vec<FormSubmissionProperty>,
    ) -> Result<BTreeMap<String, Value>, FlowableError> {
        let field_map = form_properties
            .iter()
            .map(|property| (property.id.as_str(), property))
            .collect::<BTreeMap<_, _>>();
        let mut values = BTreeMap::new();
        let mut seen = BTreeSet::new();

        for property in properties {
            if !seen.insert(property.id.clone()) {
                return Err(FlowableError::DeploymentValidationError(format!(
                    "Form field '{}' was submitted multiple times",
                    property.id
                )));
            }

            let field = field_map
                .get(property.id.as_str())
                .copied()
                .ok_or_else(|| {
                    FlowableError::DeploymentValidationError(format!(
                        "Form field '{}' is not defined for this form",
                        property.id
                    ))
                })?;
            if !field.writable {
                return Err(FlowableError::DeploymentValidationError(format!(
                    "Form field '{}' is not writable",
                    property.id
                )));
            }

            // 通过 handler dispatch 进行校验和类型转换
            let normalized_type = normalize_field_type(&field.field_type);
            let coerced = match self.handlers.get(&normalized_type) {
                Some(handler) => {
                    handler.validate(field, &property.value)?;
                    handler.coerce(field, property.value)?
                }
                None => {
                    // Java form field type errors surface as illegal argument → 400.
                    // Use BadRequest so REST maps to 400 (not ExecutionError → 500).
                    return Err(FlowableError::BadRequest(format!(
                        "Unsupported field type: {}",
                        normalized_type
                    )));
                }
            };
            values.insert(property.id, coerced);
        }

        // 检查 required 字段
        for field in form_properties {
            if field.required && !values.contains_key(&field.id) {
                return Err(FlowableError::DeploymentValidationError(format!(
                    "Required form field '{}' is missing",
                    field.id
                )));
            }
        }

        Ok(values)
    }
}

fn load_task_values(
    engine: &Arc<ProcessEngine>,
    task: &Task,
) -> Result<BTreeMap<String, Value>, FlowableError> {
    // The task execution's parent chain reaches the process-instance scope
    // execution row, which is the single process-level variable store.
    let task_values = engine
        .get_variable_service()
        .get_variables(task.execution_id.clone())?;
    Ok(task_values.into_iter().collect())
}

fn find_start_form_key(model: &BpmnModel) -> Option<String> {
    model
        .main_process
        .as_ref()?
        .flow_elements
        .iter()
        .find_map(|element| match element {
            FlowElementEnum::StartEvent(start_event) => start_event.form_key.clone(),
            _ => None,
        })
}

fn find_task_form_key(model: &BpmnModel, task_definition_key: &str) -> Option<String> {
    let process = model.main_process.as_ref()?;
    match process.flow_element_map.get(task_definition_key) {
        Some(FlowElementEnum::UserTask(user_task)) => user_task.form_key.clone(),
        _ => None,
    }
}

fn normalize_field_type(field_type: &str) -> String {
    field_types::runtime_handler_type(field_type)
        .map(str::to_string)
        .unwrap_or_else(|| field_type.trim().to_ascii_lowercase())
}

fn normalize_submitted_by(submitted_by: Option<String>) -> Option<String> {
    submitted_by.and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

fn normalize_outcome(outcome: Option<String>) -> Option<String> {
    outcome.and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

fn validate_resource_name_field(
    object: &Map<String, Value>,
    resource_name: &str,
) -> Result<(), FlowableError> {
    if let Some(value) = object.get("resourceName").and_then(Value::as_str)
        && value != resource_name
    {
        return Err(FlowableError::DeploymentValidationError(format!(
            "Form resource '{}' declared mismatched resourceName '{}'",
            resource_name, value
        )));
    }

    Ok(())
}

fn required_string(
    object: &Map<String, Value>,
    field: &str,
    resource_name: &str,
) -> Result<String, FlowableError> {
    object
        .get(field)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| {
            FlowableError::DeploymentValidationError(format!(
                "Form resource '{}' is missing string field '{}'",
                resource_name, field
            ))
        })
}

fn optional_string(object: &Map<String, Value>, field: &str) -> Option<String> {
    object
        .get(field)
        .and_then(Value::as_str)
        .map(str::to_string)
}
