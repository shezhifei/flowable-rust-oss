use flowable_dmn_engine::{
    DmnComparisonOperator, DmnDecisionDefinition, DmnDeferredOperator, DmnHitPolicy,
    DmnStringFunction, DmnStringTransform, DmnUnaryTest,
};
use flowable_dmn_model::{Decision, DmnDefinition, HitPolicy};
use std::error::Error;
use std::fmt::{Display, Formatter};

#[derive(Debug, Clone, Default, PartialEq)]
pub struct DmnSvgGeneratorOptions {
    pub advanced: DmnAdvancedSvgGeneratorOptions,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct DmnAdvancedSvgGeneratorOptions {
    pub font_family: Option<String>,
    pub color_scheme: Option<String>,
    pub scale: Option<f32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DmnSvgGeneratorError {
    UnsupportedOptions { options: Vec<&'static str> },
    NotFound { id: String },
    Structural(String),
}

impl Display for DmnSvgGeneratorError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedOptions { options } => {
                write!(f, "unsupported DMN SVG options: {}", options.join(", "))
            }
            Self::NotFound { id } => write!(f, "DMN decision '{id}' was not found"),
            Self::Structural(message) => f.write_str(message),
        }
    }
}

impl Error for DmnSvgGeneratorError {}

pub struct DmnSvgGenerator;

impl DmnSvgGenerator {
    pub fn new() -> Self {
        Self
    }

    pub fn generate_definition_svg(
        &self,
        definition: &DmnDefinition,
    ) -> Result<String, DmnSvgGeneratorError> {
        self.generate_definition_svg_with_options(definition, &DmnSvgGeneratorOptions::default())
    }

    pub fn generate_definition_svg_with_options(
        &self,
        definition: &DmnDefinition,
        options: &DmnSvgGeneratorOptions,
    ) -> Result<String, DmnSvgGeneratorError> {
        reject_advanced_options(options)?;
        let decision = definition.decisions.first().ok_or_else(|| {
            DmnSvgGeneratorError::Structural(
                "DMN definition must contain at least one decision".to_string(),
            )
        })?;
        self.generate_decision_svg(decision)
    }

    pub fn generate_decision_svg(
        &self,
        decision: &Decision,
    ) -> Result<String, DmnSvgGeneratorError> {
        self.generate_decision_svg_with_options(decision, &DmnSvgGeneratorOptions::default())
    }

    pub fn generate_decision_svg_with_options(
        &self,
        decision: &Decision,
        options: &DmnSvgGeneratorOptions,
    ) -> Result<String, DmnSvgGeneratorError> {
        reject_advanced_options(options)?;
        if !matches!(decision.decision_table.hit_policy, HitPolicy::First) {
            return Err(DmnSvgGeneratorError::Structural(
                "Owned M18 subset only supports FIRST hit policy".to_string(),
            ));
        }
        if decision.decision_table.inputs.is_empty() || decision.decision_table.outputs.is_empty() {
            return Err(DmnSvgGeneratorError::Structural(
                "Owned M18 DMN subset requires at least one input and one output".to_string(),
            ));
        }

        let columns = decision.decision_table.inputs.len() + decision.decision_table.outputs.len();
        let cell_width = 120usize;
        let width = 48 + (columns * cell_width);
        let height = 114 + (decision.decision_table.rules.len() * 24) + 24;
        let title = decision.name.as_deref().unwrap_or(&decision.id);

        let mut body = String::new();
        body.push_str(&format!(
            "<title id=\"dmn-title-{id}\">{title}</title>\n",
            id = escape_xml(&decision.id),
            title = escape_xml(title)
        ));
        body.push_str(&format!(
            "<rect x=\"1\" y=\"1\" width=\"{}\" height=\"{}\" rx=\"10\" fill=\"#fcfcfd\" stroke=\"#1f2937\" stroke-width=\"2\"/>\n",
            width - 2,
            height - 2
        ));
        body.push_str(&format!(
            "<rect x=\"24\" y=\"24\" width=\"{}\" height=\"44\" rx=\"8\" fill=\"#1f2937\"/>\n",
            width - 48
        ));
        body.push_str(&format!(
            "<text x=\"40\" y=\"51\" font-family=\"monospace\" font-size=\"18\" font-weight=\"700\" fill=\"#f9fafb\">{}</text>\n",
            escape_xml(title)
        ));
        body.push_str(&format!(
            "<rect x=\"{}\" y=\"34\" width=\"61\" height=\"24\" rx=\"12\" fill=\"#f59e0b\"/>\n",
            width - 101
        ));
        body.push_str(&format!(
            "<text x=\"{}\" y=\"50\" text-anchor=\"middle\" font-family=\"monospace\" font-size=\"12\" font-weight=\"700\" fill=\"#111827\">FIRST</text>\n",
            width - 71
        ));

        let mut x = 24usize;
        for input in &decision.decision_table.inputs {
            body.push_str(&header_cell(
                x,
                80,
                cell_width,
                input
                    .label
                    .as_deref()
                    .unwrap_or(input.input_expression.text.as_deref().unwrap_or("input")),
            ));
            x += cell_width;
        }
        for output in &decision.decision_table.outputs {
            body.push_str(&header_cell(
                x,
                80,
                cell_width,
                output
                    .label
                    .as_deref()
                    .or(output.name.as_deref())
                    .unwrap_or("output"),
            ));
            x += cell_width;
        }

        for (row_index, rule) in decision.decision_table.rules.iter().enumerate() {
            let row_y = 114 + (row_index * 24);
            let mut cell_x = 24usize;
            for input_entry in &rule.input_entries {
                body.push_str(&value_cell(
                    cell_x,
                    row_y,
                    cell_width,
                    input_entry.text.as_deref().unwrap_or("-"),
                ));
                cell_x += cell_width;
            }
            for output_entry in &rule.output_entries {
                body.push_str(&value_cell(
                    cell_x,
                    row_y,
                    cell_width,
                    output_entry.text.as_deref().unwrap_or("null"),
                ));
                cell_x += cell_width;
            }
        }

        Ok(format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{width}\" height=\"{height}\" viewBox=\"0 0 {width} {height}\" role=\"img\" aria-labelledby=\"dmn-title-{id}\">\n{body}</svg>\n",
            width = width,
            height = height,
            id = escape_xml(&decision.id),
            body = body
        ))
    }

    pub fn generate_decision_svg_by_id(
        &self,
        definition: &DmnDefinition,
        decision_id: &str,
    ) -> Result<String, DmnSvgGeneratorError> {
        let decision = definition
            .decisions
            .iter()
            .find(|decision| decision.id == decision_id)
            .ok_or_else(|| DmnSvgGeneratorError::NotFound {
                id: decision_id.to_string(),
            })?;
        self.generate_decision_svg(decision)
    }

    pub fn generate_engine_definition_svg(
        &self,
        definition: &DmnDecisionDefinition,
    ) -> Result<String, DmnSvgGeneratorError> {
        if !matches!(definition.hit_policy, DmnHitPolicy::First) {
            return Err(DmnSvgGeneratorError::Structural(
                "Owned M18 subset only supports FIRST hit policy".to_string(),
            ));
        }
        if definition.inputs.is_empty() || definition.outputs.is_empty() {
            return Err(DmnSvgGeneratorError::Structural(
                "Owned M18 DMN subset requires at least one input and one output".to_string(),
            ));
        }

        let columns = definition.inputs.len() + definition.outputs.len();
        let cell_width = 120usize;
        let width = 48 + (columns * cell_width);
        let height = 114 + (definition.rules.len() * 24) + 24;
        let title = &definition.name;

        let mut body = String::new();
        body.push_str(&format!(
            "<title id=\"dmn-title-{id}\">{title}</title>\n",
            id = escape_xml(&definition.id),
            title = escape_xml(title)
        ));
        body.push_str(&format!(
            "<rect x=\"1\" y=\"1\" width=\"{}\" height=\"{}\" rx=\"10\" fill=\"#fcfcfd\" stroke=\"#1f2937\" stroke-width=\"2\"/>\n",
            width - 2,
            height - 2
        ));
        body.push_str(&format!(
            "<rect x=\"24\" y=\"24\" width=\"{}\" height=\"44\" rx=\"8\" fill=\"#1f2937\"/>\n",
            width - 48
        ));
        body.push_str(&format!(
            "<text x=\"40\" y=\"51\" font-family=\"monospace\" font-size=\"18\" font-weight=\"700\" fill=\"#f9fafb\">{}</text>\n",
            escape_xml(title)
        ));
        body.push_str(&format!(
            "<rect x=\"{}\" y=\"34\" width=\"61\" height=\"24\" rx=\"12\" fill=\"#f59e0b\"/>\n",
            width - 101
        ));
        body.push_str(&format!(
            "<text x=\"{}\" y=\"50\" text-anchor=\"middle\" font-family=\"monospace\" font-size=\"12\" font-weight=\"700\" fill=\"#111827\">{}</text>\n",
            width - 71,
            hit_policy_label(definition.hit_policy.clone())
        ));

        let mut x = 24usize;
        for input in &definition.inputs {
            body.push_str(&header_cell(
                x,
                80,
                cell_width,
                input.label.as_deref().unwrap_or(&input.input_variable),
            ));
            x += cell_width;
        }
        for output in &definition.outputs {
            body.push_str(&header_cell(
                x,
                80,
                cell_width,
                output.label.as_deref().unwrap_or(&output.name),
            ));
            x += cell_width;
        }

        for (row_index, rule) in definition.rules.iter().enumerate() {
            let row_y = 114 + (row_index * 24);
            let mut cell_x = 24usize;
            for input_entry in &rule.input_entries {
                body.push_str(&value_cell(
                    cell_x,
                    row_y,
                    cell_width,
                    &unary_test_label(&input_entry.expression),
                ));
                cell_x += cell_width;
            }
            for output_entry in &rule.output_entries {
                body.push_str(&value_cell(
                    cell_x,
                    row_y,
                    cell_width,
                    &output_entry.value.to_string(),
                ));
                cell_x += cell_width;
            }
        }

        Ok(format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{width}\" height=\"{height}\" viewBox=\"0 0 {width} {height}\" role=\"img\" aria-labelledby=\"dmn-title-{id}\">\n{body}</svg>\n",
            width = width,
            height = height,
            id = escape_xml(&definition.id),
            body = body
        ))
    }
}

impl Default for DmnSvgGenerator {
    fn default() -> Self {
        Self::new()
    }
}

fn reject_advanced_options(options: &DmnSvgGeneratorOptions) -> Result<(), DmnSvgGeneratorError> {
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
        Err(DmnSvgGeneratorError::UnsupportedOptions {
            options: unsupported,
        })
    }
}

fn header_cell(x: usize, y: usize, width: usize, text: &str) -> String {
    format!(
        "<rect x=\"{x}\" y=\"{y}\" width=\"{width}\" height=\"34\" fill=\"#e2e8f0\" stroke=\"#475569\" stroke-width=\"1.5\"/>\n<text x=\"{tx}\" y=\"{ty}\" font-family=\"monospace\" font-size=\"13\" font-weight=\"700\" fill=\"#0f172a\">{text}</text>\n",
        x = x,
        y = y,
        width = width,
        tx = x + 12,
        ty = y + 21,
        text = escape_xml(text)
    )
}

fn value_cell(x: usize, y: usize, width: usize, text: &str) -> String {
    format!(
        "<rect x=\"{x}\" y=\"{y}\" width=\"{width}\" height=\"24\" fill=\"#ffffff\" stroke=\"#94a3b8\" stroke-width=\"1\"/>\n<text x=\"{tx}\" y=\"{ty}\" font-family=\"monospace\" font-size=\"12\" fill=\"#334155\">{text}</text>\n",
        x = x,
        y = y,
        width = width,
        tx = x + 12,
        ty = y + 16,
        text = escape_xml(text)
    )
}

fn hit_policy_label(hit_policy: DmnHitPolicy) -> &'static str {
    match hit_policy {
        DmnHitPolicy::First => "FIRST",
        DmnHitPolicy::Unique => "UNIQUE",
        DmnHitPolicy::Any => "ANY",
        DmnHitPolicy::RuleOrder => "RULE ORDER",
        DmnHitPolicy::OutputOrder => "OUTPUT ORDER",
        DmnHitPolicy::Priority => "PRIORITY",
        DmnHitPolicy::Collect => "COLLECT",
        DmnHitPolicy::Complete => "COMPLETE",
        DmnHitPolicy::Batch => "BATCH",
    }
}

fn unary_test_label(test: &DmnUnaryTest) -> String {
    match test {
        DmnUnaryTest::Any => "-".to_string(),
        // Runtime-evaluated entries render as their source text (P84).
        DmnUnaryTest::DeferredComparison { operator, source } => {
            let operator = match operator {
                DmnDeferredOperator::Equals => "==",
                DmnDeferredOperator::NotEquals => "!=",
                DmnDeferredOperator::GreaterThan => ">",
                DmnDeferredOperator::GreaterThanOrEqual => ">=",
                DmnDeferredOperator::LessThan => "<",
                DmnDeferredOperator::LessThanOrEqual => "<=",
            };
            format!("{operator} {source}")
        }
        DmnUnaryTest::ElCondition { source } => format!("#{{{source}}}"),
        DmnUnaryTest::PropertyPath { path, test } => {
            format!(".{} {}", path.join("."), unary_test_label(test))
        }
        DmnUnaryTest::Equals(value) => value.to_string(),
        DmnUnaryTest::NotEquals(value) => format!("!= {value}"),
        DmnUnaryTest::StringFunction { function, needle } => {
            format!("{}(?, \"{}\")", string_function_label(*function), needle)
        }
        DmnUnaryTest::StringTransform {
            transform,
            expected,
        } => format!(
            "{}(?) = \"{}\"",
            string_transform_label(*transform),
            expected
        ),
        DmnUnaryTest::StringTransformComparison {
            transform,
            operator,
            expected,
        } => format!(
            "{}(?) {} {}",
            string_transform_label(*transform),
            comparison_operator_label(*operator),
            expected
        ),
        DmnUnaryTest::GreaterThan(value) => format!("> {value}"),
        DmnUnaryTest::GreaterThanOrEqual(value) => format!(">= {value}"),
        DmnUnaryTest::LessThan(value) => format!("< {value}"),
        DmnUnaryTest::LessThanOrEqual(value) => format!("<= {value}"),
        DmnUnaryTest::Range {
            start,
            end,
            start_inclusive,
            end_inclusive,
        } => format!(
            "{}{}..{}{}",
            if *start_inclusive { "[" } else { "(" },
            start,
            end,
            if *end_inclusive { "]" } else { ")" }
        ),
        DmnUnaryTest::AnyOf(tests) => tests
            .iter()
            .map(unary_test_label)
            .collect::<Vec<_>>()
            .join(", "),
        DmnUnaryTest::Not(test) => format!("not({})", unary_test_label(test)),
        DmnUnaryTest::Substring {
            start,
            length,
            expected,
        } => {
            if let Some(len) = length {
                format!("substring(?, {start}, {len}) = \"{expected}\"")
            } else {
                format!("substring(?, {start}) = \"{expected}\"")
            }
        }
        DmnUnaryTest::Replace {
            pattern,
            replacement,
            flags,
            expected,
        } => {
            if let Some(f) = flags {
                format!("replace(?, \"{pattern}\", \"{replacement}\", \"{f}\") = \"{expected}\"")
            } else {
                format!("replace(?, \"{pattern}\", \"{replacement}\") = \"{expected}\"")
            }
        }
        DmnUnaryTest::And(tests) => tests
            .iter()
            .map(unary_test_label)
            .collect::<Vec<_>>()
            .join(" and "),
        DmnUnaryTest::Or(tests) => tests
            .iter()
            .map(unary_test_label)
            .collect::<Vec<_>>()
            .join(" or "),
        DmnUnaryTest::InstanceOf { type_name } => format!("instance of({type_name})"),
        DmnUnaryTest::ListContains { needle } => match needle {
            flowable_dmn_engine::DmnListContainsNeedle::Literal(value) => {
                format!("list contains(?, {value})")
            }
            flowable_dmn_engine::DmnListContainsNeedle::Variable(name) => {
                format!("list contains(?, {name})")
            }
        },
        DmnUnaryTest::InList { values } => {
            let rendered = values
                .iter()
                .map(|value| value.to_string())
                .collect::<Vec<_>>()
                .join(", ");
            format!("? in ({rendered})")
        }
    }
}

fn string_function_label(function: DmnStringFunction) -> &'static str {
    match function {
        DmnStringFunction::Contains => "contains",
        DmnStringFunction::StartsWith => "starts with",
        DmnStringFunction::EndsWith => "ends with",
        DmnStringFunction::Matches => "matches",
    }
}

fn string_transform_label(transform: DmnStringTransform) -> &'static str {
    match transform {
        DmnStringTransform::LowerCase => "lower case",
        DmnStringTransform::UpperCase => "upper case",
        DmnStringTransform::StringLength => "string length",
    }
}

fn comparison_operator_label(operator: DmnComparisonOperator) -> &'static str {
    match operator {
        DmnComparisonOperator::GreaterThan => ">",
        DmnComparisonOperator::GreaterThanOrEqual => ">=",
        DmnComparisonOperator::LessThan => "<",
        DmnComparisonOperator::LessThanOrEqual => "<=",
    }
}

fn escape_xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
