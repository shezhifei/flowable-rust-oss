use schemars::{JsonSchema, Schema, SchemaGenerator};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FormDeploymentRequest {
    pub name: String,
    pub resources: Vec<FormDeploymentResource>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FormDeploymentResource {
    pub resource_name: String,
    pub resource: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FormDeployment {
    pub id: String,
    pub name: String,
    pub deployed_at: i64,
    pub resource_names: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FormDefinition {
    pub id: String,
    pub deployment_id: String,
    pub key: String,
    pub name: String,
    pub description: Option<String>,
    pub version: i32,
    pub resource_name: String,
    pub form_payload: Value,
    pub outcomes: Option<Vec<FormOutcome>>,
    pub outcome_variable_name: Option<String>,
    pub layout: Option<serde_json::Value>,
    #[serde(default = "default_active")]
    pub active: Option<bool>,
}

fn default_active() -> Option<bool> {
    Some(true)
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct FormEnumValue {
    pub id: String,
    pub name: String,
}

/// A single option item (e.g., for dropdown, radio, checkbox).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct FormOption {
    pub id: String,
    pub name: String,
}

/// Layout definition for a form field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct LayoutDefinition {
    pub row: Option<i32>,
    pub col: Option<i32>,
    pub col_span: Option<i32>,
}

/// Base form field containing all common properties.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct BaseFormField {
    pub id: String,
    pub name: Option<String>,
    #[serde(rename = "type")]
    pub field_type: Option<String>,
    pub value: Option<serde_json::Value>,
    pub readable: Option<bool>,
    pub writable: Option<bool>,
    pub required: Option<bool>,
    pub read_only: Option<bool>,
    pub placeholder: Option<String>,
    pub params: Option<HashMap<String, String>>,
    pub layout: Option<LayoutDefinition>,
    pub date_pattern: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub enum_values: Vec<FormEnumValue>,
}

/// Option-based form field (dropdown, radio, checkbox).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct OptionFormField {
    #[serde(flatten)]
    pub base: BaseFormField,
    pub option_type: Option<String>,
    #[serde(default)]
    pub has_empty_value: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<FormOption>,
    pub options_expression: Option<String>,
}

/// Expression-driven form field.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ExpressionFormField {
    #[serde(flatten)]
    pub base: BaseFormField,
    pub expression: String,
}

/// Container form field that can hold nested fields in a 2D row structure.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct FormContainer {
    #[serde(flatten)]
    pub base: BaseFormField,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<Vec<FormFieldModel>>,
}

/// A form outcome (submit button / result option).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct FormOutcome {
    pub id: Option<String>,
    pub name: Option<String>,
}

/// Polymorphic form field model.
#[derive(Debug, Clone, PartialEq)]
pub enum FormFieldModel {
    Container(FormContainer),
    OptionField(OptionFormField),
    ExpressionField(ExpressionFormField),
    BaseField(BaseFormField),
}

#[allow(dead_code)]
#[derive(JsonSchema)]
#[schemars(tag = "fieldType")]
enum FormFieldModelSchema {
    Container {
        #[schemars(flatten)]
        field: FormContainer,
    },
    #[schemars(rename = "OptionFormField")]
    OptionField {
        #[schemars(flatten)]
        field: OptionFormField,
    },
    #[schemars(rename = "ExpressionFormField")]
    ExpressionField {
        #[schemars(flatten)]
        field: ExpressionFormField,
    },
    BaseField {
        #[schemars(flatten)]
        field: BaseFormField,
    },
}

impl JsonSchema for FormFieldModel {
    fn schema_name() -> Cow<'static, str> {
        "FormFieldModel".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        FormFieldModelSchema::json_schema(generator)
    }
}

/// Editable Flowable form document persisted by the modeler repository.
///
/// Runtime/deployment metadata belongs to [`FormDefinition`]; the modeler
/// projection intentionally contains only author-owned form semantics.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FormModel {
    pub key: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<FormFieldModel>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub outcomes: Vec<FormOutcome>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outcome_variable_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub layout: Option<Value>,
}

impl serde::Serialize for FormFieldModel {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut value = match self {
            FormFieldModel::Container(c) => serde_json::to_value(c),
            FormFieldModel::OptionField(o) => serde_json::to_value(o),
            FormFieldModel::ExpressionField(e) => serde_json::to_value(e),
            FormFieldModel::BaseField(b) => serde_json::to_value(b),
        }
        .map_err(serde::ser::Error::custom)?;

        if let Some(obj) = value.as_object_mut() {
            let field_type = match self {
                FormFieldModel::Container(_) => "Container",
                FormFieldModel::OptionField(_) => "OptionFormField",
                FormFieldModel::ExpressionField(_) => "ExpressionFormField",
                FormFieldModel::BaseField(_) => "BaseField",
            };
            obj.insert(
                "fieldType".to_string(),
                serde_json::Value::String(field_type.to_string()),
            );
        }

        value.serialize(serializer)
    }
}

impl<'de> serde::Deserialize<'de> for FormFieldModel {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value: serde_json::Value = serde_json::Value::deserialize(deserializer)?;
        let field_type = value
            .get("fieldType")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        match field_type {
            "Container" => {
                let container: FormContainer =
                    serde_json::from_value(value).map_err(serde::de::Error::custom)?;
                Ok(FormFieldModel::Container(container))
            }
            "OptionFormField" => {
                let field: OptionFormField =
                    serde_json::from_value(value).map_err(serde::de::Error::custom)?;
                Ok(FormFieldModel::OptionField(field))
            }
            "ExpressionFormField" => {
                let field: ExpressionFormField =
                    serde_json::from_value(value).map_err(serde::de::Error::custom)?;
                Ok(FormFieldModel::ExpressionField(field))
            }
            "BaseField" => {
                let field: BaseFormField =
                    serde_json::from_value(value).map_err(serde::de::Error::custom)?;
                Ok(FormFieldModel::BaseField(field))
            }
            "" => Err(serde::de::Error::custom(
                "Form field is missing required 'fieldType' property",
            )),
            other => Err(serde::de::Error::custom(format!(
                "Unknown form field fieldType '{}'",
                other
            ))),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FormProperty {
    pub id: String,
    pub name: Option<String>,
    #[serde(rename = "type")]
    pub field_type: String,
    pub value: Option<Value>,
    pub readable: bool,
    pub writable: bool,
    pub required: bool,
    pub date_pattern: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub enum_values: Vec<FormEnumValue>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FormData {
    pub form_definition_id: String,
    pub form_key: Option<String>,
    pub deployment_id: String,
    pub process_definition_id: Option<String>,
    pub task_id: Option<String>,
    pub form_properties: Vec<FormProperty>,
    pub outcomes: Option<Vec<FormOutcome>>,
    pub layout: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub form_fields: Option<Vec<FormFieldModel>>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FormSubmissionProperty {
    pub id: String,
    pub value: Value,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FormSubmissionRequest {
    pub process_definition_id: Option<String>,
    pub task_id: Option<String>,
    pub business_key: Option<String>,
    pub outcome: Option<String>,
    #[serde(default)]
    pub properties: Vec<FormSubmissionProperty>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum FormSubmissionResult {
    ProcessInstance(flowable_engine::runtime::process_instance::ProcessInstance),
    TaskCompleted(FormInstance),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FormInstance {
    pub id: String,
    pub form_definition_id: String,
    pub form_definition_key: String,
    pub form_definition_name: String,
    pub deployment_id: String,
    pub process_definition_id: Option<String>,
    pub process_instance_id: Option<String>,
    pub task_id: Option<String>,
    pub scope_type: String,
    pub scope_id: String,
    /// Scope definition id (process definition id for BPMN start/task forms,
    /// or explicit CMMN/case definition id for scope-based forms).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope_definition_id: Option<String>,
    pub submitted_at: i64,
    #[serde(default)]
    pub submitted_by: Option<String>,
    /// Tenant owning this form instance (stored; not derived at query time).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tenant_id: Option<String>,
    /// Id of the persisted form-values document (Java `formValuesId`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub form_values_id: Option<String>,
    /// Canonical JSON bytes of `values` generated at write time
    /// (Java `formValueBytes`). Legacy rows may omit this field; readers
    /// derive bytes from `values` without mutating storage during queries.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub form_value_bytes: Option<Vec<u8>>,
    /// Selected form outcome (Java `saveFormInstance(..., outcome)`).
    /// Stored on the instance JSON document; not a separate query column.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<String>,
    /// Typed API view of submitted field values.
    pub values: BTreeMap<String, Value>,
}

/// Canonical JSON bytes for form instance values (write path).
pub fn build_form_value_bytes(values: &BTreeMap<String, Value>) -> Vec<u8> {
    serde_json::to_vec(values).unwrap_or_else(|_| b"{}".to_vec())
}

/// Resolve form value bytes for an instance (read path).
///
/// Prefers stored `form_value_bytes`; for legacy rows without bytes, derives
/// from the typed `values` map without writing back to storage.
pub fn form_instance_values_bytes(instance: &FormInstance) -> Vec<u8> {
    if let Some(ref bytes) = instance.form_value_bytes {
        return bytes.clone();
    }
    build_form_value_bytes(&instance.values)
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PagedResult<T> {
    pub start: usize,
    pub size: usize,
    pub total: usize,
    pub data: Vec<T>,
}
