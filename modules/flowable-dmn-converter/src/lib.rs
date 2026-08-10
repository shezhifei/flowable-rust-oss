use flowable_dmn_model::{
    CollectOperator, Decision, DecisionRule, DecisionTable, DmnDefinition, HitPolicy, InputClause,
    LiteralExpression, OutputClause, UnaryTests,
};
use quick_xml::events::Event as XmlEvent;
use quick_xml::reader::Reader;
use roxmltree::{Document, Node, ParsingOptions};
use std::error::Error;
use std::fmt::{Display, Formatter};

mod writer;

pub use writer::{DmnXmlWriter, write_dmn_definition};

/// Maximum XML element nesting depth accepted (M3): bounds converter
/// recursion over decisions / decision tables / expressions.
///
/// Deliberately far lower than the BPMN converter's 512. BPMN is parsed by
/// `quick_xml`, an iterative pull parser with no depth-proportional stack use,
/// so a high cap costs it nothing. CMMN and DMN are parsed by `roxmltree`,
/// whose parser recurses per element and overflows the thread stack well below
/// 512. Measured on this workspace (nested-element chain, roxmltree 0.20):
///
/// | build   | thread stack        | deepest OK | overflows |
/// |---------|---------------------|-----------:|----------:|
/// | debug   | ~1 MiB (main)       |        150 |       200 |
/// | debug   | 2 MiB (test/spawn)  |        300 |       400 |
/// | release | 2 MiB (tokio worker)|       2000 |      3000 |
///
/// A 512 cap therefore *admitted* documents that abort the process in any debug
/// build. 64 sits under the tightest of those ceilings while leaving ~9x room
/// over the deepest XML in this repository (depth 7 across every fixture), and
/// allows far more nested stages / decision structure than a real model uses.
const MAX_XML_NESTING_DEPTH: usize = 64;
/// Total XML node budget; rejects pathological documents before conversion
/// work begins.
const XML_NODES_LIMIT: u32 = 1_000_000;

/// Parse with a bounded node budget and reject overly-deep nesting so hostile
/// documents cannot drive converter recursion into stack overflow.
fn parse_document<'a>(xml: &'a str) -> Result<Document<'a>, DmnConverterError> {
    reject_deep_nesting(xml)?;
    let document = Document::parse_with_options(
        xml,
        ParsingOptions {
            nodes_limit: XML_NODES_LIMIT,
            ..Default::default()
        },
    )
    .map_err(|error| DmnConverterError::InvalidXml(error.to_string()))?;
    Ok(document)
}

/// Reject over-deep nesting **before** roxmltree sees the document.
///
/// This must run pre-parse, not post-parse: `roxmltree`'s parser recurses per
/// element, so a deeply-nested document overflows the thread stack *inside*
/// `Document::parse_with_options` and aborts the process. A guard that walks
/// the parsed tree can therefore never fire for the documents it exists to
/// reject. Measured on this workspace: a debug build dies below 200 levels, a
/// release build on a 2 MiB stack around 2000 -- both reachable by an attacker,
/// and neither bounded by `nodes_limit` (a 3000-deep chain is only 3000 nodes).
///
/// `quick_xml::Reader` is a pull parser with an explicit stack, so counting
/// depth with it cannot itself overflow. Same boundary convention as the BPMN
/// converter's `validate_well_formed_xml` (root element = depth 1); the cap
/// itself is lower here -- see `MAX_XML_NESTING_DEPTH`.
///
/// Lexer errors are rejected rather than passed through: a document quick-xml
/// cannot tokenize is not valid CMMN/DMN, and failing closed keeps a document
/// that stalls the scan early from reaching the recursive parser unchecked.
fn reject_deep_nesting(xml: &str) -> Result<(), DmnConverterError> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    let mut depth: usize = 0;
    loop {
        match reader.read_event() {
            Ok(XmlEvent::Start(_)) => {
                depth += 1;
                if depth > MAX_XML_NESTING_DEPTH {
                    return Err(DmnConverterError::InvalidXml(format!(
                        "XML element nesting exceeds the limit of {MAX_XML_NESTING_DEPTH} levels"
                    )));
                }
            }
            // Self-closing elements occupy a level without opening one.
            Ok(XmlEvent::Empty(_)) => {
                if depth + 1 > MAX_XML_NESTING_DEPTH {
                    return Err(DmnConverterError::InvalidXml(format!(
                        "XML element nesting exceeds the limit of {MAX_XML_NESTING_DEPTH} levels"
                    )));
                }
            }
            Ok(XmlEvent::End(_)) => depth = depth.saturating_sub(1),
            Ok(XmlEvent::Eof) => return Ok(()),
            Ok(_) => {}
            Err(error) => {
                return Err(DmnConverterError::InvalidXml(format!(
                    "malformed XML: {error}"
                )));
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DmnConverterError {
    InvalidXml(String),
    Serialization(String),
    MissingAttribute {
        element: String,
        attribute: &'static str,
    },
    UnsupportedAttribute {
        element: String,
        attribute: String,
    },
    UnsupportedElement {
        parent: String,
        element: String,
    },
    UnsupportedHitPolicy(String),
    UnsupportedAggregation(String),
    Structural(String),
}

impl Display for DmnConverterError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidXml(message) => write!(f, "invalid DMN XML: {message}"),
            Self::Serialization(message) => write!(f, "failed to serialize DMN XML: {message}"),
            Self::MissingAttribute { element, attribute } => {
                write!(f, "missing required attribute `{attribute}` on `{element}`")
            }
            Self::UnsupportedAttribute { element, attribute } => {
                write!(f, "unsupported attribute `{attribute}` on `{element}`")
            }
            Self::UnsupportedElement { parent, element } => {
                write!(f, "unsupported `{element}` element inside `{parent}`")
            }
            Self::UnsupportedHitPolicy(value) => {
                write!(
                    f,
                    "unsupported decisionTable hitPolicy `{value}`; supported hit policies are `FIRST`, `UNIQUE`, `ANY`, `RULE ORDER`, `OUTPUT ORDER`, `PRIORITY`, `COLLECT`, and `COMPLETE`"
                )
            }
            Self::UnsupportedAggregation(value) => {
                write!(
                    f,
                    "unsupported decisionTable aggregation `{value}`; supported COLLECT aggregations are `COUNT`, `SUM`, `MIN`, and `MAX`"
                )
            }
            Self::Structural(message) => f.write_str(message),
        }
    }
}

impl Error for DmnConverterError {}

pub struct DmnXmlConverter;

impl DmnXmlConverter {
    pub fn new() -> Self {
        Self
    }

    /// DMN 1.1 / 1.2 / 1.3 target namespaces. Java picks the XSD by namespace
    /// (`DmnXMLConverter.java:83-87,135-162`) and validates by default
    /// (`DmnParse.java:54` `validateSchema = true`). Rust does no XSD
    /// validation — its structural checks + `reject_unknown_attributes` are
    /// stricter in practice — but the namespace gate rejects wrong-spec
    /// documents (e.g. a BPMN `<definitions>` deployed to the DMN endpoint
    /// previously parsed into an empty model).
    const DMN_11_NAMESPACE: &str = "http://www.omg.org/spec/DMN/20151101/dmn.xsd";
    const DMN_12_NAMESPACE: &str = "http://www.omg.org/spec/DMN/20180521/MODEL/";
    const DMN_13_NAMESPACE: &str = "https://www.omg.org/spec/DMN/20191111/MODEL/";

    pub fn parse_definition(&self, xml: &str) -> Result<DmnDefinition, DmnConverterError> {
        self.parse_definition_with_validation(xml, true)
    }

    /// `validate_namespace=false` mirrors Java's per-deployment opt-out
    /// (`DeploymentSettings.IS_DMN_XSD_VALIDATION_ENABLED`,
    /// `ParsedDeploymentBuilder.java:81-82`). Not exposed over REST.
    pub fn parse_definition_with_validation(
        &self,
        xml: &str,
        validate_namespace: bool,
    ) -> Result<DmnDefinition, DmnConverterError> {
        let document = parse_document(xml)?;
        let root = document.root_element();
        expect_element_name(root, "definitions")?;
        if validate_namespace {
            // Absent xmlns stays accepted (legacy Rust fixtures); a present
            // but non-DMN namespace is rejected, matching Java's namespace→XSD
            // dispatch failing on unknown namespaces.
            if let Some(namespace) = root.tag_name().namespace() {
                if ![
                    Self::DMN_11_NAMESPACE,
                    Self::DMN_12_NAMESPACE,
                    Self::DMN_13_NAMESPACE,
                ]
                .contains(&namespace)
                {
                    return Err(DmnConverterError::Structural(format!(
                        "unsupported DMN namespace `{namespace}`; expected one of \
                         `{}` (DMN 1.1), `{}` (DMN 1.2), `{}` (DMN 1.3)",
                        Self::DMN_11_NAMESPACE,
                        Self::DMN_12_NAMESPACE,
                        Self::DMN_13_NAMESPACE
                    )));
                }
            }
        }
        reject_unknown_attributes(
            root,
            &[
                "id",
                "name",
                "namespace",
                "expressionLanguage",
                "typeLanguage",
                "exporter",
                "exporterVersion",
            ],
        )?;

        let mut definition = DmnDefinition {
            id: root.attribute("id").map(ToOwned::to_owned),
            name: root.attribute("name").map(ToOwned::to_owned),
            namespace: root.attribute("namespace").map(ToOwned::to_owned),
            expression_language: root.attribute("expressionLanguage").map(ToOwned::to_owned),
            type_language: root.attribute("typeLanguage").map(ToOwned::to_owned),
            exporter: root.attribute("exporter").map(ToOwned::to_owned),
            exporter_version: root.attribute("exporterVersion").map(ToOwned::to_owned),
            namespaces: collect_namespaces(root),
            decisions: Vec::new(),
            decision_services: Vec::new(),
            knowledge_sources: Vec::new(),
            authority_requirements: Vec::new(),
        };

        for child in element_children(root) {
            match child.tag_name().name() {
                "decision" => definition.decisions.push(parse_decision(child)?),
                "decisionService" => definition
                    .decision_services
                    .push(parse_decision_service(child)?),
                "knowledgeSource" => definition
                    .knowledge_sources
                    .push(parse_knowledge_source(child)?),
                "authorityRequirement" => definition
                    .authority_requirements
                    .push(parse_authority_requirement(child)?),
                _ => {} // Ignore association, inputData, dmndi etc.
            }
        }

        Ok(definition)
    }
}

impl Default for DmnXmlConverter {
    fn default() -> Self {
        Self::new()
    }
}

pub fn parse_dmn_definition(xml: &str) -> Result<DmnDefinition, DmnConverterError> {
    DmnXmlConverter::new().parse_definition(xml)
}

fn parse_decision(node: Node<'_, '_>) -> Result<Decision, DmnConverterError> {
    reject_unknown_attributes(node, &["id", "name"])?;
    let id = required_attribute(node, "decision", "id")?.to_string();
    let name = node.attribute("name").map(ToOwned::to_owned);

    let mut decision_table = None;
    let mut required_decisions = Vec::new();

    for child in element_children(node) {
        match child.tag_name().name() {
            "decisionTable" => {
                if decision_table.is_some() {
                    return Err(DmnConverterError::Structural(format!(
                        "decision `{id}` must contain exactly one `decisionTable`"
                    )));
                }
                decision_table = Some(parse_decision_table(child)?);
            }
            "informationRequirement" => {
                if let Some(href) = child.attribute("href") {
                    let req_id = href.strip_prefix('#').unwrap_or(href).to_string();
                    required_decisions.push(req_id);
                } else {
                    for sub_child in element_children(child) {
                        if sub_child.tag_name().name() == "requiredDecision"
                            && let Some(href) = sub_child.attribute("href")
                        {
                            let req_id = href.strip_prefix('#').unwrap_or(href).to_string();
                            required_decisions.push(req_id);
                        }
                    }
                }
            }
            _ => {} // Ignore variable, authorityRequirement inside decision
        }
    }

    Ok(Decision {
        id,
        name,
        decision_table: decision_table.ok_or_else(|| {
            DmnConverterError::Structural(
                "decision must contain exactly one `decisionTable`".to_string(),
            )
        })?,
        required_decisions,
    })
}

fn parse_decision_service(
    node: Node<'_, '_>,
) -> Result<flowable_dmn_model::DecisionService, DmnConverterError> {
    reject_unknown_attributes(node, &["id", "name"])?;
    let id = required_attribute(node, "decisionService", "id")?.to_string();
    let name = required_attribute(node, "decisionService", "name")?.to_string();

    let mut required_decisions = Vec::new();
    let mut output_decisions = Vec::new();

    for child in element_children(node) {
        match child.tag_name().name() {
            "requiredDecision" | "encapsulatedDecision" => {
                if let Some(href) = child.attribute("href") {
                    let req_id = href.strip_prefix('#').unwrap_or(href).to_string();
                    required_decisions.push(req_id);
                }
            }
            "outputDecision" => {
                if let Some(href) = child.attribute("href") {
                    let out_id = href.strip_prefix('#').unwrap_or(href).to_string();
                    output_decisions.push(out_id);
                }
            }
            _ => {}
        }
    }

    Ok(flowable_dmn_model::DecisionService {
        id,
        name,
        required_decisions,
        output_decisions,
    })
}

fn parse_knowledge_source(
    node: Node<'_, '_>,
) -> Result<flowable_dmn_model::KnowledgeSource, DmnConverterError> {
    reject_unknown_attributes(node, &["id", "name"])?;
    let id = required_attribute(node, "knowledgeSource", "id")?.to_string();
    let name = required_attribute(node, "knowledgeSource", "name")?.to_string();

    let mut description = None;
    for child in element_children(node) {
        if child.tag_name().name() == "description" {
            description = child.text().map(|t| t.trim().to_string());
        }
    }

    Ok(flowable_dmn_model::KnowledgeSource {
        id,
        name,
        description,
        type_: None,
        owner: None,
    })
}

fn parse_authority_requirement(
    node: Node<'_, '_>,
) -> Result<flowable_dmn_model::AuthorityRequirement, DmnConverterError> {
    reject_unknown_attributes(node, &["id"])?;
    let id = required_attribute(node, "authorityRequirement", "id")?.to_string();

    let mut required_authority = None;
    let mut required_decision = None;
    let mut decision = None;

    for child in element_children(node) {
        match child.tag_name().name() {
            "requiredDecision" => {
                if let Some(href) = child.attribute("href") {
                    required_decision = Some(href.strip_prefix('#').unwrap_or(href).to_string());
                }
            }
            "requiredAuthority" => {
                if let Some(href) = child.attribute("href") {
                    required_authority = Some(href.strip_prefix('#').unwrap_or(href).to_string());
                }
            }
            "decision" => {
                if let Some(href) = child.attribute("href") {
                    decision = Some(href.strip_prefix('#').unwrap_or(href).to_string());
                }
            }
            _ => {}
        }
    }

    Ok(flowable_dmn_model::AuthorityRequirement {
        id,
        required_authority,
        required_decision,
        decision,
    })
}

fn parse_decision_table(node: Node<'_, '_>) -> Result<DecisionTable, DmnConverterError> {
    reject_unknown_attributes(node, &["id", "hitPolicy", "aggregation", "collectOperator"])?;
    let id = required_attribute(node, "decisionTable", "id")?.to_string();
    let hit_policy = match node.attribute("hitPolicy") {
        Some("FIRST") => HitPolicy::First,
        Some("UNIQUE") => HitPolicy::Unique,
        Some("ANY") => HitPolicy::Any,
        Some("RULE ORDER") => HitPolicy::RuleOrder,
        Some("OUTPUT ORDER") => HitPolicy::OutputOrder,
        Some("PRIORITY") => HitPolicy::Priority,
        Some("COLLECT") => HitPolicy::Collect,
        Some("COMPLETE") => HitPolicy::Complete,
        Some(other) => return Err(DmnConverterError::UnsupportedHitPolicy(other.to_string())),
        None => {
            return Err(DmnConverterError::MissingAttribute {
                element: "decisionTable".to_string(),
                attribute: "hitPolicy",
            });
        }
    };
    let collect_operator = parse_collect_operator(node)?;
    if collect_operator.is_some() && hit_policy != HitPolicy::Collect {
        return Err(DmnConverterError::Structural(format!(
            "decisionTable `{id}` uses COLLECT aggregation but hitPolicy is not COLLECT"
        )));
    }

    let mut inputs = Vec::new();
    let mut outputs = Vec::new();
    let mut rules = Vec::new();

    for child in element_children(node) {
        match child.tag_name().name() {
            "input" => inputs.push(parse_input_clause(child, inputs.len() + 1)?),
            "output" => outputs.push(parse_output_clause(child, outputs.len() + 1)?),
            "rule" => rules.push(parse_decision_rule(child, rules.len() + 1)?),
            other => {
                return Err(DmnConverterError::UnsupportedElement {
                    parent: "decisionTable".to_string(),
                    element: other.to_string(),
                });
            }
        }
    }

    for rule in &rules {
        if rule.input_entries.len() != inputs.len() {
            return Err(DmnConverterError::Structural(format!(
                "rule {} has {} inputEntry element(s), but decisionTable `{id}` defines {} input clause(s)",
                rule.rule_number,
                rule.input_entries.len(),
                inputs.len()
            )));
        }
        if rule.output_entries.len() != outputs.len() {
            return Err(DmnConverterError::Structural(format!(
                "rule {} has {} outputEntry element(s), but decisionTable `{id}` defines {} output clause(s)",
                rule.rule_number,
                rule.output_entries.len(),
                outputs.len()
            )));
        }
    }

    Ok(DecisionTable {
        id,
        hit_policy,
        collect_operator,
        inputs,
        outputs,
        rules,
    })
}

fn parse_collect_operator(
    node: Node<'_, '_>,
) -> Result<Option<CollectOperator>, DmnConverterError> {
    let aggregation = node.attribute("aggregation");
    let collect_operator = node.attribute("collectOperator");

    match (aggregation, collect_operator) {
        (Some(left), Some(right)) if left != right => Err(DmnConverterError::Structural(format!(
            "decisionTable `{}` declares conflicting aggregation `{left}` and collectOperator `{right}`",
            node.attribute("id").unwrap_or("<unknown>")
        ))),
        (Some(value), _) | (_, Some(value)) => parse_collect_operator_value(value).map(Some),
        (None, None) => Ok(None),
    }
}

fn parse_collect_operator_value(value: &str) -> Result<CollectOperator, DmnConverterError> {
    match value {
        "COUNT" => Ok(CollectOperator::Count),
        "SUM" => Ok(CollectOperator::Sum),
        "MIN" => Ok(CollectOperator::Min),
        "MAX" => Ok(CollectOperator::Max),
        other => Err(DmnConverterError::UnsupportedAggregation(other.to_string())),
    }
}

fn parse_input_clause(
    node: Node<'_, '_>,
    input_number: usize,
) -> Result<InputClause, DmnConverterError> {
    reject_unknown_attributes(node, &["id", "label"])?;
    let id = node.attribute("id").map(ToOwned::to_owned);
    let label = node.attribute("label").map(ToOwned::to_owned);

    let mut input_expression = None;
    for child in element_children(node) {
        match child.tag_name().name() {
            "inputExpression" => {
                if input_expression.is_some() {
                    return Err(DmnConverterError::Structural(format!(
                        "input clause {} must contain exactly one `inputExpression`",
                        input_number
                    )));
                }
                input_expression = Some(parse_input_expression(child)?);
            }
            other => {
                return Err(DmnConverterError::UnsupportedElement {
                    parent: "input".to_string(),
                    element: other.to_string(),
                });
            }
        }
    }

    Ok(InputClause {
        id,
        label,
        input_number,
        input_expression: input_expression.ok_or_else(|| {
            DmnConverterError::Structural(format!(
                "input clause {} must contain exactly one `inputExpression`",
                input_number
            ))
        })?,
    })
}

fn parse_input_expression(node: Node<'_, '_>) -> Result<LiteralExpression, DmnConverterError> {
    reject_unknown_attributes(node, &["id", "typeRef"])?;
    Ok(LiteralExpression {
        id: node.attribute("id").map(ToOwned::to_owned),
        type_ref: node.attribute("typeRef").map(ToOwned::to_owned),
        text: Some(parse_text_child(node, "inputExpression")?),
    })
}

fn parse_output_clause(
    node: Node<'_, '_>,
    output_number: usize,
) -> Result<OutputClause, DmnConverterError> {
    reject_unknown_attributes(node, &["id", "label", "name", "typeRef"])?;
    let mut output_values = None;
    for child in element_children(node) {
        match child.tag_name().name() {
            "outputValues" => {
                if output_values.is_some() {
                    return Err(DmnConverterError::Structural(format!(
                        "output clause {} must contain at most one `outputValues`",
                        output_number
                    )));
                }
                output_values = Some(parse_output_values(child)?);
            }
            other => {
                return Err(DmnConverterError::UnsupportedElement {
                    parent: "output".to_string(),
                    element: other.to_string(),
                });
            }
        }
    }
    Ok(OutputClause {
        id: node.attribute("id").map(ToOwned::to_owned),
        label: node.attribute("label").map(ToOwned::to_owned),
        name: node.attribute("name").map(ToOwned::to_owned),
        type_ref: node.attribute("typeRef").map(ToOwned::to_owned),
        output_values,
        output_number,
    })
}

fn parse_output_values(node: Node<'_, '_>) -> Result<UnaryTests, DmnConverterError> {
    reject_unknown_attributes(node, &["id"])?;
    Ok(UnaryTests {
        id: node.attribute("id").map(ToOwned::to_owned),
        text: Some(parse_text_child(node, "outputValues")?),
    })
}

fn parse_decision_rule(
    node: Node<'_, '_>,
    rule_number: usize,
) -> Result<DecisionRule, DmnConverterError> {
    reject_unknown_attributes(node, &["id"])?;
    let id = node.attribute("id").map(ToOwned::to_owned);
    let mut input_entries = Vec::new();
    let mut output_entries = Vec::new();

    for child in element_children(node) {
        match child.tag_name().name() {
            "inputEntry" => input_entries.push(parse_input_entry(child)?),
            "outputEntry" => output_entries.push(parse_output_entry(child)?),
            other => {
                return Err(DmnConverterError::UnsupportedElement {
                    parent: "rule".to_string(),
                    element: other.to_string(),
                });
            }
        }
    }

    Ok(DecisionRule {
        id,
        rule_number,
        input_entries,
        output_entries,
    })
}

fn parse_input_entry(node: Node<'_, '_>) -> Result<UnaryTests, DmnConverterError> {
    reject_unknown_attributes(node, &["id"])?;
    Ok(UnaryTests {
        id: node.attribute("id").map(ToOwned::to_owned),
        text: Some(parse_text_child(node, "inputEntry")?),
    })
}

fn parse_output_entry(node: Node<'_, '_>) -> Result<LiteralExpression, DmnConverterError> {
    reject_unknown_attributes(node, &["id"])?;
    Ok(LiteralExpression {
        id: node.attribute("id").map(ToOwned::to_owned),
        type_ref: None,
        text: Some(parse_text_child(node, "outputEntry")?),
    })
}

fn parse_text_child(node: Node<'_, '_>, parent: &'static str) -> Result<String, DmnConverterError> {
    let mut text_node = None;
    for child in element_children(node) {
        match child.tag_name().name() {
            "text" => {
                if text_node.is_some() {
                    return Err(DmnConverterError::Structural(format!(
                        "`{parent}` must contain exactly one `text` child"
                    )));
                }
                text_node = Some(child);
            }
            other => {
                return Err(DmnConverterError::UnsupportedElement {
                    parent: parent.to_string(),
                    element: other.to_string(),
                });
            }
        }
    }

    let text_node = text_node.ok_or_else(|| {
        DmnConverterError::Structural(format!("`{parent}` must contain exactly one `text` child"))
    })?;
    Ok(text_node.text().unwrap_or_default().to_string())
}

fn required_attribute<'a>(
    node: Node<'a, 'a>,
    element: &'static str,
    attribute: &'static str,
) -> Result<&'a str, DmnConverterError> {
    node.attribute(attribute)
        .ok_or(DmnConverterError::MissingAttribute {
            element: element.to_string(),
            attribute,
        })
}

fn reject_unknown_attributes(
    node: Node<'_, '_>,
    allowed: &[&str],
) -> Result<(), DmnConverterError> {
    for attribute in node.attributes() {
        if !allowed
            .iter()
            .any(|allowed_name| attribute.name() == *allowed_name)
        {
            return Err(DmnConverterError::UnsupportedAttribute {
                element: node.tag_name().name().to_string(),
                attribute: attribute.name().to_string(),
            });
        }
    }
    Ok(())
}

fn expect_element_name(
    node: Node<'_, '_>,
    expected: &'static str,
) -> Result<(), DmnConverterError> {
    if node.tag_name().name() == expected {
        Ok(())
    } else {
        Err(DmnConverterError::Structural(format!(
            "expected root element `{expected}`, found `{}`",
            node.tag_name().name()
        )))
    }
}

fn element_children<'a>(node: Node<'a, 'a>) -> impl Iterator<Item = Node<'a, 'a>> {
    node.children().filter(|child| child.is_element())
}

fn collect_namespaces(node: Node<'_, '_>) -> std::collections::BTreeMap<String, String> {
    let mut namespaces = std::collections::BTreeMap::new();
    for namespace in node.namespaces() {
        namespaces.insert(
            namespace.name().unwrap_or_default().to_string(),
            namespace.uri().to_string(),
        );
    }
    namespaces
}
