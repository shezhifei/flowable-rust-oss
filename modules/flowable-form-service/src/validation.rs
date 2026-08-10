use std::collections::HashSet;

use crate::field_types::{self, FormFieldVariant};
use crate::models::{BaseFormField, FormFieldModel, FormModel, LayoutDefinition, OptionFormField};

pub const MISSING_FIELD_TYPE: &str = "flowable-form-field-type-required";
pub const UNSUPPORTED_FIELD_TYPE: &str = "flowable-form-field-type-unsupported";
pub const INCOMPATIBLE_FIELD_VARIANT: &str = "flowable-form-field-variant-incompatible";
pub const INCOMPATIBLE_WRITEABILITY: &str = "flowable-form-field-writeability-incompatible";
pub const INVALID_OPTIONS: &str = "flowable-form-field-options-invalid";
pub const DYNAMIC_OPTIONS_UNSUPPORTED: &str = "flowable-form-dynamic-options-unsupported";
pub const INVALID_EXPRESSION: &str = "flowable-form-field-expression-invalid";
pub const INVALID_CONTAINER: &str = "flowable-form-field-container-invalid";
pub const INVALID_LAYOUT: &str = "flowable-form-field-layout-invalid";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FormModelValidationIssue {
    pub element_id: Option<String>,
    pub code: &'static str,
    pub message: String,
}

impl FormModelValidationIssue {
    pub fn stable_message(&self) -> String {
        format!("[{}] {}", self.code, self.message)
    }
}

/// Validate field semantics shared by modeler save and form deployment.
///
/// The validator never rewrites `BaseFormField.field_type`; callers may decode
/// and re-encode unknown imported values losslessly, while save/deploy paths
/// can reject them before creating an unusable definition.
pub fn validate_form_model(model: &FormModel) -> Vec<FormModelValidationIssue> {
    validate_form_model_with_supported_type(model, |_| false)
}

/// Variant of [`validate_form_model`] used by runtimes with registered custom
/// value handlers. Custom types remain BaseField-compatible and writable;
/// exact Flowable types always use the stricter built-in capability contract.
pub fn validate_form_model_with_supported_type(
    model: &FormModel,
    is_additionally_supported: impl Fn(&str) -> bool,
) -> Vec<FormModelValidationIssue> {
    let mut issues = Vec::new();
    let mut ids = HashSet::new();
    validate_fields(
        &model.fields,
        &mut ids,
        &mut issues,
        &is_additionally_supported,
    );
    issues
}

fn validate_fields(
    fields: &[FormFieldModel],
    ids: &mut HashSet<String>,
    issues: &mut Vec<FormModelValidationIssue>,
    is_additionally_supported: &dyn Fn(&str) -> bool,
) {
    for field in fields {
        let (base, actual_variant) = match field {
            FormFieldModel::Container(field) => (&field.base, FormFieldVariant::Container),
            FormFieldModel::OptionField(field) => (&field.base, FormFieldVariant::OptionFormField),
            FormFieldModel::ExpressionField(field) => {
                (&field.base, FormFieldVariant::ExpressionFormField)
            }
            FormFieldModel::BaseField(field) => (field, FormFieldVariant::BaseField),
        };

        validate_id(base, ids, issues);
        validate_layout(base, issues);
        let Some(field_type) = base
            .field_type
            .as_deref()
            .filter(|field_type| !field_type.trim().is_empty())
        else {
            push(
                issues,
                base,
                MISSING_FIELD_TYPE,
                "form field type is required".to_string(),
            );
            continue;
        };
        let Some(capability) = field_types::form_field_capability(field_type) else {
            if is_additionally_supported(field_type) {
                if actual_variant != FormFieldVariant::BaseField {
                    push(
                        issues,
                        base,
                        INCOMPATIBLE_FIELD_VARIANT,
                        format!(
                            "custom form field type `{field_type}` requires BaseField, not {:?}",
                            actual_variant
                        ),
                    );
                }
                continue;
            }
            push(
                issues,
                base,
                UNSUPPORTED_FIELD_TYPE,
                format!("form field type `{field_type}` is not supported"),
            );
            continue;
        };

        if capability.required_variant != actual_variant {
            push(
                issues,
                base,
                INCOMPATIBLE_FIELD_VARIANT,
                format!(
                    "form field type `{field_type}` requires {:?}, not {:?}",
                    capability.required_variant, actual_variant
                ),
            );
        }
        validate_writeability(
            base,
            capability.writable,
            capability.supports_required,
            issues,
        );

        match field {
            FormFieldModel::Container(container) => {
                if container.fields.iter().any(Vec::is_empty) {
                    push(
                        issues,
                        base,
                        INVALID_CONTAINER,
                        "form container rows must not be empty".to_string(),
                    );
                }
                for row in &container.fields {
                    validate_fields(row, ids, issues, is_additionally_supported);
                }
            }
            FormFieldModel::OptionField(option) => validate_options(option, issues),
            FormFieldModel::ExpressionField(expression) => {
                if expression.expression.trim().is_empty() {
                    push(
                        issues,
                        base,
                        INVALID_EXPRESSION,
                        "expression form field requires a non-empty expression".to_string(),
                    );
                } else if !has_balanced_uel_segments(&expression.expression) {
                    push(
                        issues,
                        base,
                        INVALID_EXPRESSION,
                        "expression form field contains an unclosed `${...}` segment".to_string(),
                    );
                }
            }
            FormFieldModel::BaseField(_) => {}
        }
    }
}

fn has_balanced_uel_segments(expression: &str) -> bool {
    let bytes = expression.as_bytes();
    let mut index = 0usize;
    while index + 1 < bytes.len() {
        if bytes[index] != b'$' || bytes[index + 1] != b'{' {
            index += 1;
            continue;
        }
        index += 2;
        let mut depth = 1usize;
        while index < bytes.len() && depth > 0 {
            match bytes[index] {
                b'{' => depth += 1,
                b'}' => depth -= 1,
                _ => {}
            }
            index += 1;
        }
        if depth != 0 {
            return false;
        }
    }
    true
}

fn validate_id(
    field: &BaseFormField,
    ids: &mut HashSet<String>,
    issues: &mut Vec<FormModelValidationIssue>,
) {
    let id = field.id.trim();
    if id.is_empty() {
        push(
            issues,
            field,
            "flowable-form-field-id-required",
            "form field id is required".to_string(),
        );
    } else if !ids.insert(id.to_string()) {
        push(
            issues,
            field,
            "flowable-form-field-id-duplicate",
            format!("duplicate form field id `{id}`"),
        );
    }
}

fn validate_writeability(
    field: &BaseFormField,
    writable_type: bool,
    supports_required: bool,
    issues: &mut Vec<FormModelValidationIssue>,
) {
    if field.read_only == Some(true) && field.writable == Some(true) {
        push(
            issues,
            field,
            INCOMPATIBLE_WRITEABILITY,
            "read-only form field cannot also be writable".to_string(),
        );
    } else if !writable_type && field.writable == Some(true) {
        push(
            issues,
            field,
            INCOMPATIBLE_WRITEABILITY,
            "structural, expression, and display fields cannot be writable".to_string(),
        );
    }
    if !supports_required && field.required == Some(true) {
        push(
            issues,
            field,
            INCOMPATIBLE_WRITEABILITY,
            "structural, expression, and display fields cannot be required".to_string(),
        );
    }
}

fn validate_options(field: &OptionFormField, issues: &mut Vec<FormModelValidationIssue>) {
    let base_type = field.base.field_type.as_deref().unwrap_or_default();
    if field.option_type.as_deref().is_some_and(|option_type| {
        let option_type = option_type.trim();
        !option_type.is_empty() && option_type != "static" && option_type != base_type
    }) {
        push(
            issues,
            &field.base,
            INVALID_OPTIONS,
            "optionType must be `static` or match the option field type".to_string(),
        );
    }
    let mut option_ids = HashSet::new();
    for option in &field.options {
        if option.id.trim().is_empty() || option.name.trim().is_empty() {
            push(
                issues,
                &field.base,
                INVALID_OPTIONS,
                "option id and name must both be non-empty".to_string(),
            );
        } else if !option_ids.insert(option.id.trim()) {
            push(
                issues,
                &field.base,
                INVALID_OPTIONS,
                format!("duplicate option id `{}`", option.id.trim()),
            );
        }
    }
    let has_dynamic_options = field
        .options_expression
        .as_deref()
        .is_some_and(|expression| !expression.trim().is_empty());
    if has_dynamic_options {
        push(
            issues,
            &field.base,
            DYNAMIC_OPTIONS_UNSUPPORTED,
            "dynamic optionsExpression is not supported by this runtime; use static options"
                .to_string(),
        );
    } else if field.options.is_empty() {
        push(
            issues,
            &field.base,
            INVALID_OPTIONS,
            "option form field requires at least one static option".to_string(),
        );
    }
}

fn validate_layout(field: &BaseFormField, issues: &mut Vec<FormModelValidationIssue>) {
    let Some(LayoutDefinition { row, col, col_span }) = field.layout.as_ref() else {
        return;
    };
    if row.is_some_and(|value| value < 0)
        || col.is_some_and(|value| value < 0)
        || col_span.is_some_and(|value| value <= 0)
    {
        push(
            issues,
            field,
            INVALID_LAYOUT,
            "layout row/col must be non-negative and colSpan must be positive".to_string(),
        );
    }
}

fn push(
    issues: &mut Vec<FormModelValidationIssue>,
    field: &BaseFormField,
    code: &'static str,
    message: String,
) {
    issues.push(FormModelValidationIssue {
        element_id: (!field.id.trim().is_empty()).then(|| field.id.clone()),
        code,
        message,
    });
}
