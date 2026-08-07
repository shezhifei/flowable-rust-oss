//! Runtime + task `getDataObject(s)` API.
//!
//! Java parity: `GetDataObjectsCmd` / `GetDataObjectCmd` /
//! `GetTaskDataObjectsCmd` / `GetTaskDataObjectCmd` and `DataObjectImpl`.

use crate::engine::variable_service::require_execution;
use crate::error::FlowableError;
use crate::interceptor::command::Command;
use crate::interceptor::command_context::CommandContext;
use crate::runtime::execution::Execution;
use flowable_bpmn_model::model::{FlowElementEnum, ValuedDataObject};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};

/// Runtime DataObject with type/description/localization metadata.
///
/// Java: `org.flowable.engine.runtime.DataObject` / `DataObjectImpl`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DataObject {
    pub id: String,
    pub process_instance_id: String,
    pub execution_id: String,
    pub name: String,
    pub value: Value,
    pub description: Option<String>,
    pub localized_name: Option<String>,
    pub localized_description: Option<String>,
    pub data_type: Option<String>,
    pub data_object_definition_key: Option<String>,
}

fn collect_execution_variables_local(
    execution: &Execution,
) -> HashMap<String, (String, Value)> {
    let mut out = HashMap::new();
    for (name, value) in &execution.local_variables {
        out.insert(name.clone(), (execution.id.clone(), value.clone()));
    }
    for (name, value) in &execution.variables {
        out.entry(name.clone())
            .or_insert_with(|| (execution.id.clone(), value.clone()));
    }
    out
}

/// Collects visible variables with owning execution id (nearest scope wins).
fn collect_variables_with_owners(
    command_context: &mut CommandContext,
    execution_id: &str,
    is_local: bool,
) -> Result<HashMap<String, (String, Value)>, FlowableError> {
    let execution = require_execution(command_context, execution_id)?;
    if is_local {
        return Ok(collect_execution_variables_local(&execution));
    }

    let mut all: HashMap<String, (String, Value)> = HashMap::new();
    let mut current_id = Some(execution_id.to_string());
    while let Some(id) = current_id {
        let execution = {
            let (store, session) = command_context.store_and_session();
            store.find_execution(&id, session)
        };
        let Some(execution) = execution else {
            break;
        };
        for (name, value) in &execution.transient_variables {
            all.entry(name.clone())
                .or_insert_with(|| (execution.id.clone(), value.clone()));
        }
        for (name, value) in &execution.local_variables {
            all.entry(name.clone())
                .or_insert_with(|| (execution.id.clone(), value.clone()));
        }
        for (name, value) in &execution.variables {
            all.entry(name.clone())
                .or_insert_with(|| (execution.id.clone(), value.clone()));
        }
        current_id = execution.parent_id.clone();
    }
    Ok(all)
}

/// Walks up from `owner_execution_id` until an `is_scope` execution is found
/// (Java GetDataObjectsCmd loop).
fn find_scope_execution(
    command_context: &mut CommandContext,
    owner_execution_id: &str,
) -> Option<Execution> {
    let mut current_id = Some(owner_execution_id.to_string());
    while let Some(id) = current_id {
        let execution = {
            let (store, session) = command_context.store_and_session();
            store.find_execution(&id, session)
        }?;
        if execution.is_scope || execution.parent_id.is_none() {
            return Some(execution);
        }
        current_id = execution.parent_id.clone();
    }
    None
}

fn data_objects_for_scope(
    command_context: &mut CommandContext,
    scope_execution: &Execution,
) -> Vec<ValuedDataObject> {
    let process_definition_id = match scope_execution.process_definition_id.as_deref() {
        Some(id) => id,
        None => return Vec::new(),
    };
    let Some(bpmn_model) = command_context
        .deployment_manager
        .get_bpmn_model(process_definition_id)
    else {
        return Vec::new();
    };
    let Some(main_process) = bpmn_model.main_process.as_ref() else {
        return Vec::new();
    };

    // Process-instance scope (no parent): main process data objects.
    if scope_execution.parent_id.is_none() {
        return main_process.data_objects.clone();
    }

    // Subprocess / activity scope: look up activity and its data objects.
    let Some(activity_id) = scope_execution.activity_id.as_deref() else {
        return Vec::new();
    };
    match main_process.flow_element_map.get(activity_id) {
        Some(FlowElementEnum::SubProcess(sub)) => sub.data_objects.clone(),
        Some(FlowElementEnum::Transaction(tx)) => tx.sub_process.data_objects.clone(),
        Some(FlowElementEnum::EventSubProcess(esp)) => esp.sub_process.data_objects.clone(),
        Some(FlowElementEnum::AdhocSubProcess(adhoc)) => adhoc.sub_process.data_objects.clone(),
        _ => {
            // Nested subprocesses may live only under a parent's flow_elements
            // (flow_element_map is process-level; nested maps may also hold them).
            find_data_objects_recursive(&main_process.flow_elements, activity_id)
                .unwrap_or_default()
        }
    }
}

fn find_data_objects_recursive(
    elements: &[FlowElementEnum],
    activity_id: &str,
) -> Option<Vec<ValuedDataObject>> {
    for element in elements {
        match element {
            FlowElementEnum::SubProcess(sub) => {
                if sub.activity.flow_node.flow_element.base_element.id.as_deref()
                    == Some(activity_id)
                {
                    return Some(sub.data_objects.clone());
                }
                if let Some(found) =
                    find_data_objects_recursive(&sub.flow_elements, activity_id)
                {
                    return Some(found);
                }
            }
            FlowElementEnum::Transaction(tx) => {
                if tx
                    .sub_process
                    .activity
                    .flow_node
                    .flow_element
                    .base_element
                    .id
                    .as_deref()
                    == Some(activity_id)
                {
                    return Some(tx.sub_process.data_objects.clone());
                }
                if let Some(found) =
                    find_data_objects_recursive(&tx.sub_process.flow_elements, activity_id)
                {
                    return Some(found);
                }
            }
            FlowElementEnum::EventSubProcess(esp) => {
                if esp
                    .sub_process
                    .activity
                    .flow_node
                    .flow_element
                    .base_element
                    .id
                    .as_deref()
                    == Some(activity_id)
                {
                    return Some(esp.sub_process.data_objects.clone());
                }
                if let Some(found) =
                    find_data_objects_recursive(&esp.sub_process.flow_elements, activity_id)
                {
                    return Some(found);
                }
            }
            FlowElementEnum::AdhocSubProcess(adhoc) => {
                if adhoc
                    .sub_process
                    .activity
                    .flow_node
                    .flow_element
                    .base_element
                    .id
                    .as_deref()
                    == Some(activity_id)
                {
                    return Some(adhoc.sub_process.data_objects.clone());
                }
                if let Some(found) =
                    find_data_objects_recursive(&adhoc.sub_process.flow_elements, activity_id)
                {
                    return Some(found);
                }
            }
            _ => {}
        }
    }
    None
}

fn to_data_object(
    owner_execution_id: &str,
    process_instance_id: &str,
    name: &str,
    value: Value,
    definition: &ValuedDataObject,
) -> DataObject {
    DataObject {
        id: format!("{}:{}", owner_execution_id, name),
        process_instance_id: process_instance_id.to_string(),
        execution_id: owner_execution_id.to_string(),
        name: name.to_string(),
        value,
        description: definition.documentation.clone(),
        localized_name: None,
        localized_description: None,
        data_type: definition.data_type.clone(),
        data_object_definition_key: definition.base_element.id.clone(),
    }
}

fn resolve_data_objects(
    command_context: &mut CommandContext,
    execution_id: &str,
    names: Option<&[String]>,
    is_local: bool,
) -> Result<HashMap<String, DataObject>, FlowableError> {
    if execution_id.is_empty() {
        return Err(FlowableError::BadRequest("executionId is null".to_string()));
    }
    let variables = collect_variables_with_owners(command_context, execution_id, is_local)?;
    let name_filter: Option<HashSet<&str>> =
        names.map(|n| n.iter().map(String::as_str).collect());

    let mut result = HashMap::new();
    for (name, (owner_id, value)) in variables {
        if let Some(filter) = &name_filter
            && !filter.contains(name.as_str())
        {
            continue;
        }
        let Some(scope) = find_scope_execution(command_context, &owner_id) else {
            continue;
        };
        let definitions = data_objects_for_scope(command_context, &scope);
        let Some(definition) = definitions.iter().find(|d| d.name.as_deref() == Some(name.as_str()))
        else {
            // Variable exists but is not a modeled data object — skip (Java).
            continue;
        };
        let process_instance_id = scope
            .process_instance_id
            .clone()
            .unwrap_or_else(|| scope.id.clone());
        result.insert(
            name.clone(),
            to_data_object(&owner_id, &process_instance_id, &name, value, definition),
        );
    }
    Ok(result)
}

// ── Commands ──

pub struct GetDataObjectsCmd {
    execution_id: String,
    names: Option<Vec<String>>,
    is_local: bool,
}

impl GetDataObjectsCmd {
    pub fn new(execution_id: String, is_local: bool) -> Self {
        Self {
            execution_id,
            names: None,
            is_local,
        }
    }

    pub fn with_names(execution_id: String, names: Vec<String>, is_local: bool) -> Self {
        Self {
            execution_id,
            names: Some(names),
            is_local,
        }
    }
}

impl Command<HashMap<String, DataObject>> for GetDataObjectsCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<HashMap<String, DataObject>, FlowableError> {
        resolve_data_objects(
            command_context,
            &self.execution_id,
            self.names.as_deref(),
            self.is_local,
        )
    }
}

pub struct GetDataObjectCmd {
    execution_id: String,
    name: String,
    is_local: bool,
}

impl GetDataObjectCmd {
    pub fn new(execution_id: String, name: String, is_local: bool) -> Self {
        Self {
            execution_id,
            name,
            is_local,
        }
    }
}

impl Command<Option<DataObject>> for GetDataObjectCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<Option<DataObject>, FlowableError> {
        if self.execution_id.is_empty() {
            return Err(FlowableError::BadRequest("executionId is null".to_string()));
        }
        if self.name.is_empty() {
            return Err(FlowableError::BadRequest(
                "dataObjectName is null".to_string(),
            ));
        }
        let mut map = resolve_data_objects(
            command_context,
            &self.execution_id,
            Some(std::slice::from_ref(&self.name)),
            self.is_local,
        )?;
        Ok(map.remove(&self.name))
    }
}

pub struct GetTaskDataObjectsCmd {
    task_id: String,
    names: Option<Vec<String>>,
}

impl GetTaskDataObjectsCmd {
    pub fn new(task_id: String) -> Self {
        Self {
            task_id,
            names: None,
        }
    }

    pub fn with_names(task_id: String, names: Vec<String>) -> Self {
        Self {
            task_id,
            names: Some(names),
        }
    }
}

impl Command<HashMap<String, DataObject>> for GetTaskDataObjectsCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<HashMap<String, DataObject>, FlowableError> {
        if self.task_id.is_empty() {
            return Err(FlowableError::BadRequest("taskId is null".to_string()));
        }
        let task = command_context
            .task_entity_manager
            .find_task_by_id(&self.task_id, &mut command_context.session)
            .ok_or_else(|| {
                FlowableError::NotFound(format!("task {} doesn't exist", self.task_id))
            })?;
        if task.execution_id.is_empty() {
            return Ok(HashMap::new());
        }
        resolve_data_objects(
            command_context,
            &task.execution_id,
            self.names.as_deref(),
            false,
        )
    }
}

pub struct GetTaskDataObjectCmd {
    task_id: String,
    name: String,
}

impl GetTaskDataObjectCmd {
    pub fn new(task_id: String, name: String) -> Self {
        Self { task_id, name }
    }
}

impl Command<Option<DataObject>> for GetTaskDataObjectCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<Option<DataObject>, FlowableError> {
        if self.task_id.is_empty() {
            return Err(FlowableError::BadRequest("taskId is null".to_string()));
        }
        if self.name.is_empty() {
            return Err(FlowableError::BadRequest(
                "variableName is null".to_string(),
            ));
        }
        let mut map = GetTaskDataObjectsCmd::with_names(
            self.task_id.clone(),
            vec![self.name.clone()],
        )
        .execute(command_context)?;
        Ok(map.remove(&self.name))
    }
}

