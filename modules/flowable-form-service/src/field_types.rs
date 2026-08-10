//! Flowable 6.8 form-field wire contract.
//!
//! The persisted `type` property deliberately remains a free string in the
//! model.  These constants and capabilities describe known values without
//! rewriting imported documents.

/// Single-line text input.
pub const SINGLE_LINE_TEXT: &str = "text";
/// Multi-line text input.
pub const MULTI_LINE_TEXT: &str = "multi-line-text";
pub const INTEGER: &str = "integer";
pub const DECIMAL: &str = "decimal";
pub const AMOUNT: &str = "amount";
pub const DATE: &str = "date";
pub const BOOLEAN: &str = "boolean";
pub const RADIO_BUTTONS: &str = "radio-buttons";
pub const DROPDOWN: &str = "dropdown";
pub const UPLOAD: &str = "upload";
pub const EXPRESSION: &str = "expression";
pub const PEOPLE: &str = "people";
pub const FUNCTIONAL_GROUP: &str = "functional-group";
pub const CONTAINER: &str = "container";
pub const HYPERLINK: &str = "hyperlink";
pub const SPACER: &str = "spacer";
pub const HORIZONTAL_LINE: &str = "horizontal-line";
pub const HEADLINE: &str = "headline";
pub const HEADLINE_WITH_LINE: &str = "headline-with-line";

/// The exact public constants declared by Flowable 6.8 `FormFieldTypes`.
pub const FLOWABLE_6_8_FIELD_TYPES: &[&str] = &[
    SINGLE_LINE_TEXT,
    MULTI_LINE_TEXT,
    INTEGER,
    DECIMAL,
    AMOUNT,
    DATE,
    BOOLEAN,
    RADIO_BUTTONS,
    DROPDOWN,
    UPLOAD,
    EXPRESSION,
    PEOPLE,
    FUNCTIONAL_GROUP,
    CONTAINER,
    HYPERLINK,
    SPACER,
    HORIZONTAL_LINE,
    HEADLINE,
    HEADLINE_WITH_LINE,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FormFieldCategory {
    Value,
    Option,
    Identity,
    Expression,
    Container,
    Display,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FormFieldVariant {
    BaseField,
    OptionFormField,
    ExpressionFormField,
    Container,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FormFieldCapability {
    pub wire_type: &'static str,
    pub category: FormFieldCategory,
    pub required_variant: FormFieldVariant,
    /// Key used by the runtime handler registry. `None` means the field never
    /// accepts submitted values.
    pub runtime_handler_type: Option<&'static str>,
    pub writable: bool,
    pub supports_required: bool,
}

const fn capability(
    wire_type: &'static str,
    category: FormFieldCategory,
    required_variant: FormFieldVariant,
    runtime_handler_type: Option<&'static str>,
    writable: bool,
    supports_required: bool,
) -> FormFieldCapability {
    FormFieldCapability {
        wire_type,
        category,
        required_variant,
        runtime_handler_type,
        writable,
        supports_required,
    }
}

/// Resolve an exact Flowable 6.8 wire value.
pub fn flowable_6_8_field_capability(field_type: &str) -> Option<FormFieldCapability> {
    match field_type {
        SINGLE_LINE_TEXT => Some(capability(
            SINGLE_LINE_TEXT,
            FormFieldCategory::Value,
            FormFieldVariant::BaseField,
            Some(SINGLE_LINE_TEXT),
            true,
            true,
        )),
        MULTI_LINE_TEXT => Some(capability(
            MULTI_LINE_TEXT,
            FormFieldCategory::Value,
            FormFieldVariant::BaseField,
            Some(SINGLE_LINE_TEXT),
            true,
            true,
        )),
        INTEGER => Some(capability(
            INTEGER,
            FormFieldCategory::Value,
            FormFieldVariant::BaseField,
            Some(INTEGER),
            true,
            true,
        )),
        DECIMAL => Some(capability(
            DECIMAL,
            FormFieldCategory::Value,
            FormFieldVariant::BaseField,
            Some(DECIMAL),
            true,
            true,
        )),
        AMOUNT => Some(capability(
            AMOUNT,
            FormFieldCategory::Value,
            FormFieldVariant::BaseField,
            Some(DECIMAL),
            true,
            true,
        )),
        DATE => Some(capability(
            DATE,
            FormFieldCategory::Value,
            FormFieldVariant::BaseField,
            Some(DATE),
            true,
            true,
        )),
        BOOLEAN => Some(capability(
            BOOLEAN,
            FormFieldCategory::Value,
            FormFieldVariant::BaseField,
            Some(BOOLEAN),
            true,
            true,
        )),
        RADIO_BUTTONS => Some(capability(
            RADIO_BUTTONS,
            FormFieldCategory::Option,
            FormFieldVariant::OptionFormField,
            Some("radio"),
            true,
            true,
        )),
        DROPDOWN => Some(capability(
            DROPDOWN,
            FormFieldCategory::Option,
            FormFieldVariant::OptionFormField,
            Some(DROPDOWN),
            true,
            true,
        )),
        UPLOAD => Some(capability(
            UPLOAD,
            FormFieldCategory::Value,
            FormFieldVariant::BaseField,
            Some(UPLOAD),
            true,
            true,
        )),
        EXPRESSION => Some(capability(
            EXPRESSION,
            FormFieldCategory::Expression,
            FormFieldVariant::ExpressionFormField,
            None,
            false,
            false,
        )),
        PEOPLE => Some(capability(
            PEOPLE,
            FormFieldCategory::Identity,
            FormFieldVariant::BaseField,
            Some(PEOPLE),
            true,
            true,
        )),
        FUNCTIONAL_GROUP => Some(capability(
            FUNCTIONAL_GROUP,
            FormFieldCategory::Identity,
            FormFieldVariant::BaseField,
            Some(FUNCTIONAL_GROUP),
            true,
            true,
        )),
        CONTAINER => Some(capability(
            CONTAINER,
            FormFieldCategory::Container,
            FormFieldVariant::Container,
            None,
            false,
            false,
        )),
        HYPERLINK | SPACER | HORIZONTAL_LINE | HEADLINE | HEADLINE_WITH_LINE => {
            let wire_type = match field_type {
                HYPERLINK => HYPERLINK,
                SPACER => SPACER,
                HORIZONTAL_LINE => HORIZONTAL_LINE,
                HEADLINE => HEADLINE,
                _ => HEADLINE_WITH_LINE,
            };
            Some(capability(
                wire_type,
                FormFieldCategory::Display,
                FormFieldVariant::BaseField,
                None,
                false,
                false,
            ))
        }
        _ => None,
    }
}

/// Resolve old Rust/legacy aliases while preserving their original wire text.
///
/// Aliases are accepted for existing deployments and custom API clients, but
/// they are not members of [`FLOWABLE_6_8_FIELD_TYPES`].
pub fn legacy_field_capability(field_type: &str) -> Option<FormFieldCapability> {
    let (wire_type, runtime_handler_type, variant) =
        match field_type.trim().to_ascii_lowercase().as_str() {
            "string" => (
                SINGLE_LINE_TEXT,
                SINGLE_LINE_TEXT,
                FormFieldVariant::BaseField,
            ),
            "long" => (INTEGER, INTEGER, FormFieldVariant::BaseField),
            "double" | "float" | "number" => (DECIMAL, DECIMAL, FormFieldVariant::BaseField),
            "enum" => (DROPDOWN, DROPDOWN, FormFieldVariant::OptionFormField),
            "radio" => (RADIO_BUTTONS, "radio", FormFieldVariant::OptionFormField),
            _ => return None,
        };
    Some(capability(
        wire_type,
        if matches!(variant, FormFieldVariant::OptionFormField) {
            FormFieldCategory::Option
        } else {
            FormFieldCategory::Value
        },
        variant,
        Some(runtime_handler_type),
        true,
        true,
    ))
}

/// Resolve an exact 6.8 type or a documented compatibility alias.
pub fn form_field_capability(field_type: &str) -> Option<FormFieldCapability> {
    flowable_6_8_field_capability(field_type).or_else(|| legacy_field_capability(field_type))
}

/// Resolve a persisted type to the registered runtime handler key.
pub fn runtime_handler_type(field_type: &str) -> Option<&'static str> {
    form_field_capability(field_type).and_then(|capability| capability.runtime_handler_type)
}
