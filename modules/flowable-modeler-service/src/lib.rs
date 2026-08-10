//! Testable application boundary for editor conversion and derived artifacts.
//! HTTP authentication, routing, and repository persistence stay in `flowable-ui-rest`.

use flowable_bpmn_converter::{BpmnXMLConverter, write_bpmn_model};
use flowable_bpmn_layout::ensure_layout;
use flowable_dmn_converter::{parse_dmn_definition, write_dmn_definition};
use flowable_dmn_engine::validate_editor_definition;
use flowable_form_service::validate_form_model;
use flowable_image_generator::{generate_process_diagram_svg, svg_to_png_bytes};
use flowable_modeler_protocol::{
    BpmnEditorDocument, DmnEditorDocument, FormEditorDocument, ProtocolVersion,
};
use serde::Serialize;
use std::{
    error::Error,
    fmt::{Display, Formatter},
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ValidationIssue {
    pub element_id: Option<String>,
    pub line: Option<usize>,
    pub message: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ValidationResult {
    pub valid: bool,
    pub errors: Vec<ValidationIssue>,
}

impl ValidationResult {
    pub fn valid() -> Self {
        Self {
            valid: true,
            errors: Vec::new(),
        }
    }
    pub fn invalid(message: impl Into<String>) -> Self {
        Self {
            valid: false,
            errors: vec![ValidationIssue {
                element_id: None,
                line: None,
                message: message.into(),
            }],
        }
    }

    fn from_errors(errors: Vec<ValidationIssue>) -> Self {
        Self {
            valid: errors.is_empty(),
            errors,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelerServiceError(String);
impl Display for ModelerServiceError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl Error for ModelerServiceError {}
fn error(value: impl Display) -> ModelerServiceError {
    ModelerServiceError(value.to_string())
}

pub fn decode_bpmn_xml(xml: &str) -> Result<BpmnEditorDocument, ModelerServiceError> {
    Ok(BpmnEditorDocument::new(
        BpmnXMLConverter::new()
            .try_convert_to_bpmn_model(xml)
            .map_err(error)?,
    ))
}

pub fn encode_bpmn_xml(document: &BpmnEditorDocument) -> Result<String, ModelerServiceError> {
    require_v1(document.schema_version)?;
    let xml = write_bpmn_model(&document.model).map_err(error)?;
    BpmnXMLConverter::new()
        .try_convert_to_bpmn_model(&xml)
        .map_err(error)?;
    Ok(xml)
}

pub fn validate_bpmn(document: &BpmnEditorDocument) -> ValidationResult {
    encode_bpmn_xml(document)
        .map(|_| ValidationResult::valid())
        .unwrap_or_else(|e| ValidationResult::invalid(e.to_string()))
}

pub fn layout_bpmn(
    document: &BpmnEditorDocument,
) -> Result<BpmnEditorDocument, ModelerServiceError> {
    require_v1(document.schema_version)?;
    let mut model = document.model.clone();
    ensure_layout(&mut model).map_err(error)?;
    Ok(BpmnEditorDocument::new(model))
}

pub fn bpmn_thumbnail_png(document: &BpmnEditorDocument) -> Result<Vec<u8>, ModelerServiceError> {
    require_v1(document.schema_version)?;
    let mut model = document.model.clone();
    if model.location_map.is_empty() {
        ensure_layout(&mut model).map_err(error)?;
    }
    let svg = generate_process_diagram_svg(&model).map_err(error)?;
    svg_to_png_bytes(&svg).map_err(error)
}

pub fn decode_dmn_xml(xml: &str) -> Result<DmnEditorDocument, ModelerServiceError> {
    Ok(DmnEditorDocument::new(
        parse_dmn_definition(xml).map_err(error)?,
    ))
}

pub fn encode_dmn_xml(document: &DmnEditorDocument) -> Result<String, ModelerServiceError> {
    require_v1(document.schema_version)?;
    let xml = write_dmn_definition(&document.model).map_err(error)?;
    parse_dmn_definition(&xml).map_err(error)?;
    Ok(xml)
}

pub fn validate_dmn(document: &DmnEditorDocument) -> ValidationResult {
    require_v1(document.schema_version)
        .and_then(|_| validate_editor_definition(&document.model).map_err(error))
        .and_then(|_| encode_dmn_xml(document).map(|_| ()))
        .map(|_| ValidationResult::valid())
        .unwrap_or_else(|e| ValidationResult::invalid(e.to_string()))
}

pub fn decode_form_json(json: &[u8]) -> Result<FormEditorDocument, ModelerServiceError> {
    let document: FormEditorDocument = serde_json::from_slice(json).map_err(error)?;
    require_v1(document.schema_version)?;
    Ok(document)
}

pub fn encode_form_json(document: &FormEditorDocument) -> Result<Vec<u8>, ModelerServiceError> {
    require_v1(document.schema_version)?;
    serde_json::to_vec_pretty(document).map_err(error)
}

pub fn validate_form(document: &FormEditorDocument) -> ValidationResult {
    let mut errors = Vec::new();
    if document.model.key.trim().is_empty() {
        errors.push(ValidationIssue {
            element_id: None,
            line: None,
            message: "form key is required".to_string(),
        });
    }
    if document.model.name.trim().is_empty() {
        errors.push(ValidationIssue {
            element_id: None,
            line: None,
            message: "form name is required".to_string(),
        });
    }
    errors.extend(
        validate_form_model(&document.model)
            .into_iter()
            .map(|issue| {
                let message = issue.stable_message();
                ValidationIssue {
                    element_id: issue.element_id,
                    line: None,
                    message,
                }
            }),
    );
    ValidationResult::from_errors(errors)
}

fn require_v1(version: ProtocolVersion) -> Result<(), ModelerServiceError> {
    match version {
        ProtocolVersion::V1 => Ok(()),
    }
}
