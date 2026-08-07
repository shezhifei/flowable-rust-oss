use flowable_cmmn_engine::CmmnCaseDefinition;
use flowable_cmmn_model::{Case, CmmnDefinitions, PlanItemDefinitionRef};
use std::error::Error;
use std::fmt::{Display, Formatter};

#[derive(Debug, Clone, Default, PartialEq)]
pub struct CmmnSvgGeneratorOptions {
    pub advanced: CmmnAdvancedSvgGeneratorOptions,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct CmmnAdvancedSvgGeneratorOptions {
    pub font_family: Option<String>,
    pub color_scheme: Option<String>,
    pub scale: Option<f32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CmmnSvgGeneratorError {
    UnsupportedOptions { options: Vec<&'static str> },
    NotFound { id: String },
    Structural(String),
}

impl Display for CmmnSvgGeneratorError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedOptions { options } => {
                write!(f, "unsupported CMMN SVG options: {}", options.join(", "))
            }
            Self::NotFound { id } => write!(f, "CMMN case '{id}' was not found"),
            Self::Structural(message) => f.write_str(message),
        }
    }
}

impl Error for CmmnSvgGeneratorError {}

pub struct CmmnSvgGenerator;

impl CmmnSvgGenerator {
    pub fn new() -> Self {
        Self
    }

    pub fn generate_definitions_svg(
        &self,
        definitions: &CmmnDefinitions,
    ) -> Result<String, CmmnSvgGeneratorError> {
        self.generate_definitions_svg_with_options(definitions, &CmmnSvgGeneratorOptions::default())
    }

    pub fn generate_definitions_svg_with_options(
        &self,
        definitions: &CmmnDefinitions,
        options: &CmmnSvgGeneratorOptions,
    ) -> Result<String, CmmnSvgGeneratorError> {
        reject_advanced_options(options)?;
        let case = definitions.cases.first().ok_or_else(|| {
            CmmnSvgGeneratorError::Structural(
                "CMMN definitions must contain at least one case".to_string(),
            )
        })?;
        self.generate_case_svg(case)
    }

    pub fn generate_case_svg(&self, case: &Case) -> Result<String, CmmnSvgGeneratorError> {
        self.generate_case_svg_with_options(case, &CmmnSvgGeneratorOptions::default())
    }

    pub fn generate_case_svg_with_options(
        &self,
        case: &Case,
        options: &CmmnSvgGeneratorOptions,
    ) -> Result<String, CmmnSvgGeneratorError> {
        reject_advanced_options(options)?;
        let plan_model_name = case.case_plan_model.name.as_deref().ok_or_else(|| {
            CmmnSvgGeneratorError::Structural(
                "Owned M18 CMMN subset requires casePlanModel name".to_string(),
            )
        })?;
        if case.case_plan_model.stages.is_empty() && case.case_plan_model.human_tasks.is_empty() {
            return Err(CmmnSvgGeneratorError::Structural(
                "Owned M18 CMMN subset requires at least one stage or human task".to_string(),
            ));
        }

        let title = case.name.as_deref().unwrap_or(&case.id);
        let stage = case.case_plan_model.stages.first();
        let root_task = case.case_plan_model.human_tasks.first();

        let stage_name = stage
            .and_then(|stage| stage.name.as_deref())
            .unwrap_or("Stage");
        let nested_task = stage
            .and_then(|stage| stage.plan_items.first())
            .and_then(|plan_item| case.find_plan_item_definition(&plan_item.definition_ref))
            .and_then(|definition| match definition {
                PlanItemDefinitionRef::HumanTask(task) => Some(task),
                PlanItemDefinitionRef::Stage(_)
                | PlanItemDefinitionRef::DecisionTask(_)
                | PlanItemDefinitionRef::ProcessTask(_)
                | PlanItemDefinitionRef::CaseTask(_)
                | PlanItemDefinitionRef::EventListener(_)
                | PlanItemDefinitionRef::Milestone(_) => None,
            })
            .ok_or_else(|| {
                CmmnSvgGeneratorError::Structural(
                    "Owned M18 CMMN subset requires stage plan items to resolve to direct humanTask definitions".to_string(),
                )
            })?;
        let root_task = root_task.ok_or_else(|| {
            CmmnSvgGeneratorError::Structural(
                "Owned M18 CMMN subset requires one root-level human task".to_string(),
            )
        })?;

        Ok(format!(
            concat!(
                "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"384\" height=\"300\" viewBox=\"0 0 384 300\" role=\"img\" aria-labelledby=\"cmmn-title-{id}\">\n",
                "<title id=\"cmmn-title-{id}\">{title}</title>\n",
                "<rect x=\"1\" y=\"1\" width=\"382\" height=\"298\" rx=\"12\" fill=\"#f8fafc\" stroke=\"#0f172a\" stroke-width=\"2\"/>\n",
                "<rect x=\"24\" y=\"24\" width=\"336\" height=\"44\" rx=\"10\" fill=\"#0f172a\"/>\n",
                "<text x=\"40\" y=\"51\" font-family=\"monospace\" font-size=\"18\" font-weight=\"700\" fill=\"#f8fafc\">{title}</text>\n",
                "<text x=\"40\" y=\"88\" font-family=\"monospace\" font-size=\"13\" font-weight=\"700\" fill=\"#334155\">{plan_model_name}</text>\n",
                "<rect x=\"24\" y=\"104\" width=\"336\" height=\"120\" rx=\"10\" fill=\"#dbeafe\" stroke=\"#2563eb\" stroke-width=\"2\"/>\n",
                "<text x=\"40\" y=\"128\" font-family=\"monospace\" font-size=\"14\" font-weight=\"700\" fill=\"#1e3a8a\">Stage: {stage_name}</text>\n",
                "<rect x=\"40\" y=\"144\" width=\"304\" height=\"56\" rx=\"8\" fill=\"#ffffff\" stroke=\"#64748b\" stroke-width=\"1.5\"/>\n",
                "<text x=\"56\" y=\"167\" font-family=\"monospace\" font-size=\"13\" font-weight=\"700\" fill=\"#0f172a\">{nested_task_name}</text>\n",
                "<text x=\"56\" y=\"186\" font-family=\"monospace\" font-size=\"11\" fill=\"#475569\">humanTask | {nested_task_mode}</text>\n",
                "<rect x=\"24\" y=\"240\" width=\"336\" height=\"56\" rx=\"8\" fill=\"#ffffff\" stroke=\"#64748b\" stroke-width=\"1.5\"/>\n",
                "<text x=\"40\" y=\"263\" font-family=\"monospace\" font-size=\"13\" font-weight=\"700\" fill=\"#0f172a\">{root_task_name}</text>\n",
                "<text x=\"40\" y=\"282\" font-family=\"monospace\" font-size=\"11\" fill=\"#475569\">humanTask | {root_task_mode}</text>\n",
                "</svg>\n"
            ),
            id = escape_xml(&case.id),
            title = escape_xml(title),
            plan_model_name = escape_xml(plan_model_name),
            stage_name = escape_xml(stage_name),
            nested_task_name = escape_xml(nested_task.name.as_deref().unwrap_or(&nested_task.id)),
            nested_task_mode = if nested_task.is_blocking {
                "blocking"
            } else {
                "non-blocking"
            },
            root_task_name = escape_xml(root_task.name.as_deref().unwrap_or(&root_task.id)),
            root_task_mode = if root_task.is_blocking {
                "blocking"
            } else {
                "non-blocking"
            }
        ))
    }

    pub fn generate_case_svg_by_id(
        &self,
        definitions: &CmmnDefinitions,
        case_id: &str,
    ) -> Result<String, CmmnSvgGeneratorError> {
        let case =
            definitions
                .find_case(case_id)
                .ok_or_else(|| CmmnSvgGeneratorError::NotFound {
                    id: case_id.to_string(),
                })?;
        self.generate_case_svg(case)
    }

    pub fn generate_engine_case_definition_svg(
        &self,
        definition: &CmmnCaseDefinition,
    ) -> Result<String, CmmnSvgGeneratorError> {
        let stage = definition
            .model
            .case_plan_model
            .stages
            .first()
            .ok_or_else(|| {
                CmmnSvgGeneratorError::Structural(
                    "Owned M18 CMMN subset requires one stage in the case plan model".to_string(),
                )
            })?;
        let root_task = definition
            .model
            .case_plan_model
            .human_tasks
            .first()
            .ok_or_else(|| {
                CmmnSvgGeneratorError::Structural(
                    "Owned M18 CMMN subset requires one root-level human task".to_string(),
                )
            })?;
        let nested_task = stage.human_tasks.first().ok_or_else(|| {
            CmmnSvgGeneratorError::Structural(
                "Owned M18 CMMN subset requires one human task inside the first stage".to_string(),
            )
        })?;
        let title = &definition.name;
        let plan_model_name = &definition.model.case_plan_model.name;

        Ok(format!(
            concat!(
                "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"384\" height=\"300\" viewBox=\"0 0 384 300\" role=\"img\" aria-labelledby=\"cmmn-title-{id}\">\n",
                "<title id=\"cmmn-title-{id}\">{title}</title>\n",
                "<rect x=\"1\" y=\"1\" width=\"382\" height=\"298\" rx=\"12\" fill=\"#f8fafc\" stroke=\"#0f172a\" stroke-width=\"2\"/>\n",
                "<rect x=\"24\" y=\"24\" width=\"336\" height=\"44\" rx=\"10\" fill=\"#0f172a\"/>\n",
                "<text x=\"40\" y=\"51\" font-family=\"monospace\" font-size=\"18\" font-weight=\"700\" fill=\"#f8fafc\">{title}</text>\n",
                "<text x=\"40\" y=\"88\" font-family=\"monospace\" font-size=\"13\" font-weight=\"700\" fill=\"#334155\">{plan_model_name}</text>\n",
                "<rect x=\"24\" y=\"104\" width=\"336\" height=\"120\" rx=\"10\" fill=\"#dbeafe\" stroke=\"#2563eb\" stroke-width=\"2\"/>\n",
                "<text x=\"40\" y=\"128\" font-family=\"monospace\" font-size=\"14\" font-weight=\"700\" fill=\"#1e3a8a\">Stage: {stage_name}</text>\n",
                "<rect x=\"40\" y=\"144\" width=\"304\" height=\"56\" rx=\"8\" fill=\"#ffffff\" stroke=\"#64748b\" stroke-width=\"1.5\"/>\n",
                "<text x=\"56\" y=\"167\" font-family=\"monospace\" font-size=\"13\" font-weight=\"700\" fill=\"#0f172a\">{nested_task_name}</text>\n",
                "<text x=\"56\" y=\"186\" font-family=\"monospace\" font-size=\"11\" fill=\"#475569\">humanTask | {nested_task_mode}</text>\n",
                "<rect x=\"24\" y=\"240\" width=\"336\" height=\"56\" rx=\"8\" fill=\"#ffffff\" stroke=\"#64748b\" stroke-width=\"1.5\"/>\n",
                "<text x=\"40\" y=\"263\" font-family=\"monospace\" font-size=\"13\" font-weight=\"700\" fill=\"#0f172a\">{root_task_name}</text>\n",
                "<text x=\"40\" y=\"282\" font-family=\"monospace\" font-size=\"11\" fill=\"#475569\">humanTask | {root_task_mode}</text>\n",
                "</svg>\n"
            ),
            id = escape_xml(&definition.id),
            title = escape_xml(title),
            plan_model_name = escape_xml(plan_model_name),
            stage_name = escape_xml(&stage.name),
            nested_task_name = escape_xml(&nested_task.name),
            nested_task_mode = "non-blocking",
            root_task_name = escape_xml(&root_task.name),
            root_task_mode = "blocking",
        ))
    }
}

impl Default for CmmnSvgGenerator {
    fn default() -> Self {
        Self::new()
    }
}

fn reject_advanced_options(options: &CmmnSvgGeneratorOptions) -> Result<(), CmmnSvgGeneratorError> {
    let mut unsupported = Vec::new();
    if options.advanced.font_family.is_some() {
        unsupported.push("font_family");
    }
    if options.advanced.color_scheme.is_some() {
        unsupported.push("color_scheme");
    }
    if options.advanced.scale.is_some() {
        unsupported.push("scale");
    }
    if unsupported.is_empty() {
        Ok(())
    } else {
        Err(CmmnSvgGeneratorError::UnsupportedOptions {
            options: unsupported,
        })
    }
}

fn escape_xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}
