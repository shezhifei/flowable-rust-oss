use crate::DmnConverterError;
use flowable_dmn_model::{
    AuthorityRequirement, CollectOperator, Decision, DecisionRule, DecisionService, DecisionTable,
    DmnDefinition, HitPolicy, InputClause, KnowledgeSource, LiteralExpression, OutputClause,
    UnaryTests,
};
use quick_xml::{
    Writer,
    events::{BytesDecl, BytesEnd, BytesStart, BytesText, Event},
};

const DMN_13_NAMESPACE: &str = "https://www.omg.org/spec/DMN/20191111/MODEL/";

pub struct DmnXmlWriter;

impl DmnXmlWriter {
    pub fn new() -> Self {
        Self
    }

    pub fn write_definition(
        &self,
        definition: &DmnDefinition,
    ) -> Result<String, DmnConverterError> {
        let mut writer = Writer::new_with_indent(Vec::new(), b' ', 2);
        event(
            &mut writer,
            Event::Decl(BytesDecl::new("1.0", Some("UTF-8"), None)),
        )?;

        let mut root = BytesStart::new("definitions");
        let default_namespace = definition
            .namespaces
            .get("")
            .map(String::as_str)
            .unwrap_or(DMN_13_NAMESPACE);
        root.push_attribute(("xmlns", default_namespace));
        for (prefix, namespace) in &definition.namespaces {
            if prefix.is_empty() || prefix == "xmlns" {
                continue;
            }
            let name = format!("xmlns:{prefix}");
            root.push_attribute((name.as_str(), namespace.as_str()));
        }
        push_optional(&mut root, "id", definition.id.as_deref());
        push_optional(&mut root, "name", definition.name.as_deref());
        push_optional(&mut root, "namespace", definition.namespace.as_deref());
        push_optional(
            &mut root,
            "expressionLanguage",
            definition.expression_language.as_deref(),
        );
        push_optional(
            &mut root,
            "typeLanguage",
            definition.type_language.as_deref(),
        );
        push_optional(&mut root, "exporter", definition.exporter.as_deref());
        push_optional(
            &mut root,
            "exporterVersion",
            definition.exporter_version.as_deref(),
        );
        event(&mut writer, Event::Start(root))?;

        for decision in &definition.decisions {
            write_decision(&mut writer, decision)?;
        }
        for service in &definition.decision_services {
            write_decision_service(&mut writer, service)?;
        }
        for source in &definition.knowledge_sources {
            write_knowledge_source(&mut writer, source)?;
        }
        for requirement in &definition.authority_requirements {
            write_authority_requirement(&mut writer, requirement)?;
        }

        event(&mut writer, Event::End(BytesEnd::new("definitions")))?;
        String::from_utf8(writer.into_inner())
            .map_err(|error| DmnConverterError::Serialization(error.to_string()))
    }
}

impl Default for DmnXmlWriter {
    fn default() -> Self {
        Self::new()
    }
}

pub fn write_dmn_definition(definition: &DmnDefinition) -> Result<String, DmnConverterError> {
    DmnXmlWriter::new().write_definition(definition)
}

fn write_decision(
    writer: &mut Writer<Vec<u8>>,
    decision: &Decision,
) -> Result<(), DmnConverterError> {
    let mut node = BytesStart::new("decision");
    node.push_attribute(("id", decision.id.as_str()));
    push_optional(&mut node, "name", decision.name.as_deref());
    event(writer, Event::Start(node))?;

    for required in &decision.required_decisions {
        event(
            writer,
            Event::Start(BytesStart::new("informationRequirement")),
        )?;
        let mut reference = BytesStart::new("requiredDecision");
        let href = as_href(required);
        reference.push_attribute(("href", href.as_str()));
        event(writer, Event::Empty(reference))?;
        event(writer, Event::End(BytesEnd::new("informationRequirement")))?;
    }
    write_decision_table(writer, &decision.decision_table)?;
    event(writer, Event::End(BytesEnd::new("decision")))
}

fn write_decision_table(
    writer: &mut Writer<Vec<u8>>,
    table: &DecisionTable,
) -> Result<(), DmnConverterError> {
    let mut node = BytesStart::new("decisionTable");
    node.push_attribute(("id", table.id.as_str()));
    node.push_attribute(("hitPolicy", hit_policy(table.hit_policy.clone())));
    if let Some(operator) = &table.collect_operator {
        node.push_attribute(("aggregation", collect_operator(operator.clone())));
    }
    event(writer, Event::Start(node))?;

    for input in &table.inputs {
        write_input(writer, input)?;
    }
    for output in &table.outputs {
        write_output(writer, output)?;
    }
    for rule in &table.rules {
        write_rule(writer, rule)?;
    }
    event(writer, Event::End(BytesEnd::new("decisionTable")))
}

fn write_input(writer: &mut Writer<Vec<u8>>, input: &InputClause) -> Result<(), DmnConverterError> {
    let mut node = BytesStart::new("input");
    push_optional(&mut node, "id", input.id.as_deref());
    push_optional(&mut node, "label", input.label.as_deref());
    event(writer, Event::Start(node))?;
    write_literal_expression(writer, "inputExpression", &input.input_expression, true)?;
    event(writer, Event::End(BytesEnd::new("input")))
}

fn write_output(
    writer: &mut Writer<Vec<u8>>,
    output: &OutputClause,
) -> Result<(), DmnConverterError> {
    let mut node = BytesStart::new("output");
    push_optional(&mut node, "id", output.id.as_deref());
    push_optional(&mut node, "label", output.label.as_deref());
    push_optional(&mut node, "name", output.name.as_deref());
    push_optional(&mut node, "typeRef", output.type_ref.as_deref());
    if output.output_values.is_none() {
        return event(writer, Event::Empty(node));
    }
    event(writer, Event::Start(node))?;
    write_unary_tests(
        writer,
        "outputValues",
        output.output_values.as_ref().unwrap(),
    )?;
    event(writer, Event::End(BytesEnd::new("output")))
}

fn write_rule(writer: &mut Writer<Vec<u8>>, rule: &DecisionRule) -> Result<(), DmnConverterError> {
    let mut node = BytesStart::new("rule");
    push_optional(&mut node, "id", rule.id.as_deref());
    event(writer, Event::Start(node))?;
    for input in &rule.input_entries {
        write_unary_tests(writer, "inputEntry", input)?;
    }
    for output in &rule.output_entries {
        write_literal_expression(writer, "outputEntry", output, false)?;
    }
    event(writer, Event::End(BytesEnd::new("rule")))
}

fn write_unary_tests(
    writer: &mut Writer<Vec<u8>>,
    name: &'static str,
    tests: &UnaryTests,
) -> Result<(), DmnConverterError> {
    let mut node = BytesStart::new(name);
    push_optional(&mut node, "id", tests.id.as_deref());
    event(writer, Event::Start(node))?;
    write_text(writer, tests.text.as_deref().unwrap_or_default())?;
    event(writer, Event::End(BytesEnd::new(name)))
}

fn write_literal_expression(
    writer: &mut Writer<Vec<u8>>,
    name: &'static str,
    expression: &LiteralExpression,
    include_type: bool,
) -> Result<(), DmnConverterError> {
    let mut node = BytesStart::new(name);
    push_optional(&mut node, "id", expression.id.as_deref());
    if include_type {
        push_optional(&mut node, "typeRef", expression.type_ref.as_deref());
    }
    event(writer, Event::Start(node))?;
    write_text(writer, expression.text.as_deref().unwrap_or_default())?;
    event(writer, Event::End(BytesEnd::new(name)))
}

fn write_text(writer: &mut Writer<Vec<u8>>, value: &str) -> Result<(), DmnConverterError> {
    event(writer, Event::Start(BytesStart::new("text")))?;
    event(writer, Event::Text(BytesText::new(value)))?;
    event(writer, Event::End(BytesEnd::new("text")))
}

fn write_decision_service(
    writer: &mut Writer<Vec<u8>>,
    service: &DecisionService,
) -> Result<(), DmnConverterError> {
    let mut node = BytesStart::new("decisionService");
    node.push_attribute(("id", service.id.as_str()));
    node.push_attribute(("name", service.name.as_str()));
    event(writer, Event::Start(node))?;
    for required in &service.required_decisions {
        write_href(writer, "requiredDecision", required)?;
    }
    for output in &service.output_decisions {
        write_href(writer, "outputDecision", output)?;
    }
    event(writer, Event::End(BytesEnd::new("decisionService")))
}

fn write_knowledge_source(
    writer: &mut Writer<Vec<u8>>,
    source: &KnowledgeSource,
) -> Result<(), DmnConverterError> {
    let mut node = BytesStart::new("knowledgeSource");
    node.push_attribute(("id", source.id.as_str()));
    node.push_attribute(("name", source.name.as_str()));
    if let Some(description) = &source.description {
        event(writer, Event::Start(node))?;
        event(writer, Event::Start(BytesStart::new("description")))?;
        event(writer, Event::Text(BytesText::new(description)))?;
        event(writer, Event::End(BytesEnd::new("description")))?;
        event(writer, Event::End(BytesEnd::new("knowledgeSource")))
    } else {
        event(writer, Event::Empty(node))
    }
}

fn write_authority_requirement(
    writer: &mut Writer<Vec<u8>>,
    requirement: &AuthorityRequirement,
) -> Result<(), DmnConverterError> {
    let mut node = BytesStart::new("authorityRequirement");
    node.push_attribute(("id", requirement.id.as_str()));
    event(writer, Event::Start(node))?;
    if let Some(value) = &requirement.required_authority {
        write_href(writer, "requiredAuthority", value)?;
    }
    if let Some(value) = &requirement.required_decision {
        write_href(writer, "requiredDecision", value)?;
    }
    if let Some(value) = &requirement.decision {
        write_href(writer, "decision", value)?;
    }
    event(writer, Event::End(BytesEnd::new("authorityRequirement")))
}

fn write_href(
    writer: &mut Writer<Vec<u8>>,
    name: &'static str,
    value: &str,
) -> Result<(), DmnConverterError> {
    let mut node = BytesStart::new(name);
    let href = as_href(value);
    node.push_attribute(("href", href.as_str()));
    event(writer, Event::Empty(node))
}

fn as_href(value: &str) -> String {
    if value.starts_with('#') {
        value.to_string()
    } else {
        format!("#{value}")
    }
}

fn hit_policy(policy: HitPolicy) -> &'static str {
    match policy {
        HitPolicy::First => "FIRST",
        HitPolicy::Unique => "UNIQUE",
        HitPolicy::Any => "ANY",
        HitPolicy::RuleOrder => "RULE ORDER",
        HitPolicy::OutputOrder => "OUTPUT ORDER",
        HitPolicy::Priority => "PRIORITY",
        HitPolicy::Collect => "COLLECT",
        HitPolicy::Complete => "COMPLETE",
    }
}

fn collect_operator(operator: CollectOperator) -> &'static str {
    match operator {
        CollectOperator::Count => "COUNT",
        CollectOperator::Sum => "SUM",
        CollectOperator::Min => "MIN",
        CollectOperator::Max => "MAX",
    }
}

fn push_optional(node: &mut BytesStart<'_>, name: &'static str, value: Option<&str>) {
    if let Some(value) = value {
        node.push_attribute((name, value));
    }
}

fn event(writer: &mut Writer<Vec<u8>>, event: Event<'_>) -> Result<(), DmnConverterError> {
    writer
        .write_event(event)
        .map_err(|error| DmnConverterError::Serialization(error.to_string()))
}
