use flowable_cmmn_model::{
    Case, CaseFileItem, CaseFileItemDefinition, CaseFileItemOnPart, CaseFileModel, CasePlanModel, CaseTask,
    CmmnDefinitions, DecisionTask, DiscretionaryItem, EntryCriterion, EventCorrelationParameter,
    EventListener, FlowableListener, HumanTask, ListenerImplementationType, Milestone, PlanItem,
    PlanItemOnPart, PlanningTable, ProcessTask, Sentry, SentryIfPartExpression, Stage,
    parse_sentry_if_part_expression,
};
use quick_xml::events::Event as XmlEvent;
use quick_xml::reader::Reader;
use roxmltree::{Document, Node, ParsingOptions};
use std::collections::{BTreeMap, HashSet};
use std::error::Error;
use std::fmt::{Display, Formatter};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CmmnConverterError {
    InvalidXml(String),
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
    Structural(String),
}

impl Display for CmmnConverterError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidXml(message) => write!(f, "invalid CMMN XML: {message}"),
            Self::MissingAttribute { element, attribute } => {
                write!(f, "missing required attribute `{attribute}` on `{element}`")
            }
            Self::UnsupportedAttribute { element, attribute } => {
                write!(f, "unsupported attribute `{attribute}` on `{element}`")
            }
            Self::UnsupportedElement { parent, element } => {
                write!(f, "unsupported `{element}` element inside `{parent}`")
            }
            Self::Structural(message) => f.write_str(message),
        }
    }
}

impl Error for CmmnConverterError {}

pub struct CmmnXmlConverter;

/// Maximum XML element nesting depth accepted (M3): bounds converter
/// recursion over plan items / case file items / sentries.
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
/// Total XML node budget; rejects quadratic / pathological documents before
/// conversion work begins.
const XML_NODES_LIMIT: u32 = 1_000_000;

/// Parse with a bounded node budget and reject overly-deep nesting so hostile
/// documents cannot drive converter recursion into stack overflow.
fn parse_document<'a>(xml: &'a str) -> Result<Document<'a>, CmmnConverterError> {
    reject_deep_nesting(xml)?;
    let document = Document::parse_with_options(
        xml,
        ParsingOptions {
            nodes_limit: XML_NODES_LIMIT,
            ..Default::default()
        },
    )
    .map_err(|error| CmmnConverterError::InvalidXml(error.to_string()))?;
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
fn reject_deep_nesting(xml: &str) -> Result<(), CmmnConverterError> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    let mut depth: usize = 0;
    loop {
        match reader.read_event() {
            Ok(XmlEvent::Start(_)) => {
                depth += 1;
                if depth > MAX_XML_NESTING_DEPTH {
                    return Err(CmmnConverterError::InvalidXml(format!(
                        "XML element nesting exceeds the limit of {MAX_XML_NESTING_DEPTH} levels"
                    )));
                }
            }
            // Self-closing elements occupy a level without opening one.
            Ok(XmlEvent::Empty(_)) => {
                if depth + 1 > MAX_XML_NESTING_DEPTH {
                    return Err(CmmnConverterError::InvalidXml(format!(
                        "XML element nesting exceeds the limit of {MAX_XML_NESTING_DEPTH} levels"
                    )));
                }
            }
            Ok(XmlEvent::End(_)) => depth = depth.saturating_sub(1),
            Ok(XmlEvent::Eof) => return Ok(()),
            Ok(_) => {}
            Err(error) => {
                return Err(CmmnConverterError::InvalidXml(format!("malformed XML: {error}")));
            }
        }
    }
}


impl CmmnXmlConverter {
    pub fn new() -> Self {
        Self
    }

    pub fn parse_definitions(&self, xml: &str) -> Result<CmmnDefinitions, CmmnConverterError> {
        let document = parse_document(xml)?;
        let root = document.root_element();
        expect_element_name(root, "definitions")?;
        warn_unknown_attributes(
            root,
            &[
                "id",
                "name",
                "targetNamespace",
                "expressionLanguage",
                "typeLanguage",
                "exporter",
                "exporterVersion",
            ],
        );

        let mut definitions = CmmnDefinitions {
            id: root.attribute("id").map(ToOwned::to_owned),
            name: root.attribute("name").map(ToOwned::to_owned),
            target_namespace: root.attribute("targetNamespace").map(ToOwned::to_owned),
            expression_language: root.attribute("expressionLanguage").map(ToOwned::to_owned),
            type_language: root.attribute("typeLanguage").map(ToOwned::to_owned),
            exporter: root.attribute("exporter").map(ToOwned::to_owned),
            exporter_version: root.attribute("exporterVersion").map(ToOwned::to_owned),
            namespaces: collect_namespaces(root),
            cases: Vec::new(),
        };

        let mut case_ids = HashSet::new();
        for child in element_children(root) {
            match child.tag_name().name() {
                "case" => {
                    let case_definition = parse_case(child)?;
                    if !case_ids.insert(case_definition.id.clone()) {
                        return Err(CmmnConverterError::Structural(format!(
                            "duplicate case id `{}` is not allowed in the owned M16 subset",
                            case_definition.id
                        )));
                    }
                    definitions.cases.push(case_definition);
                }
                other => skip_unknown_child("definitions", other),
            }
        }

        Ok(definitions)
    }
}

impl Default for CmmnXmlConverter {
    fn default() -> Self {
        Self::new()
    }
}

pub fn parse_cmmn_definitions(xml: &str) -> Result<CmmnDefinitions, CmmnConverterError> {
    CmmnXmlConverter::new().parse_definitions(xml)
}

/// Parse case-file definition networks without changing the stable `Case`
/// struct-literal API. The returned key is the owning case id.
pub fn parse_cmmn_case_file_models(
    xml: &str,
) -> Result<Vec<(String, CaseFileModel)>, CmmnConverterError> {
    let document = parse_document(xml)?;
    let mut result = Vec::new();
    for case in document
        .descendants()
        .filter(|node| node.is_element() && node.tag_name().name() == "case")
    {
        let case_id = required_attribute(case, "case", "id")?.to_string();
        for child in
            element_children(case).filter(|child| child.tag_name().name() == "caseFileModel")
        {
            result.push((case_id.clone(), parse_case_file_model(child)?));
        }
    }
    Ok(result)
}

fn parse_case(node: Node<'_, '_>) -> Result<Case, CmmnConverterError> {
    warn_unknown_attributes(node, &["id", "name"]);
    let id = required_attribute(node, "case", "id")?.to_string();
    let name = node.attribute("name").map(ToOwned::to_owned);

    let mut case_plan_model = None;
    let mut case_file_model = None;
    let mut lifecycle_listeners = Vec::new();
    let mut start_event_type = None;
    let mut start_correlation_configuration = None;
    let mut start_correlation_parameters = Vec::new();
    for child in element_children(node) {
        match child.tag_name().name() {
            "casePlanModel" => {
                if case_plan_model.is_some() {
                    return Err(CmmnConverterError::Structural(format!(
                        "case `{id}` must contain exactly one `casePlanModel`"
                    )));
                }
                case_plan_model = Some(parse_case_plan_model(child)?);
            }
            "caseFileModel" => {
                if case_file_model.is_some() {
                    return Err(CmmnConverterError::Structural(format!(
                        "case `{id}` must contain at most one `caseFileModel`"
                    )));
                }
                case_file_model = Some(parse_case_file_model(child)?);
            }
            // Java: ExtensionElementsXMLConverter.java:121-127 harvests caseLifecycleListener
            // and eventType onto the current case element (`Case implements
            // HasLifecycleListeners`, Case.java:20). P136 also reads
            // startEventCorrelationConfiguration / eventCorrelationParameter
            // (CmmnXmlConstants.java:224-230).
            "extensionElements" => {
                let parsed = parse_case_extension_elements(child);
                if parsed.start_event_type.is_some() {
                    start_event_type = parsed.start_event_type;
                }
                if parsed.start_correlation_configuration.is_some() {
                    start_correlation_configuration = parsed.start_correlation_configuration;
                }
                start_correlation_parameters.extend(parsed.start_correlation_parameters);
                lifecycle_listeners.extend(parsed.lifecycle_listeners);
            }
            other => skip_unknown_child("case", other),
        }
    }

    Ok(Case {
        id: id.clone(),
        name,
        case_plan_model: case_plan_model.ok_or_else(|| {
            CmmnConverterError::Structural(format!(
                "case `{id}` must contain exactly one `casePlanModel`"
            ))
        })?,
        lifecycle_listeners,
        start_event_type,
        start_correlation_configuration,
        start_correlation_parameters,
    })
}

fn parse_case_file_model(node: Node<'_, '_>) -> Result<CaseFileModel, CmmnConverterError> {
    warn_unknown_attributes(node, &["id", "name"]);
    let mut model = CaseFileModel::default();
    for child in element_children(node) {
        match child.tag_name().name() {
            "caseFileItemDefinition" => model
                .item_definitions
                .push(parse_case_file_item_definition(child)?),
            "caseFileItem" => model.items.push(parse_case_file_item(child)?),
            other => skip_unknown_child("caseFileModel", other),
        }
    }
    let definition_ids = model
        .item_definitions
        .iter()
        .map(|definition| definition.id.as_str())
        .collect::<HashSet<_>>();
    validate_case_file_item_refs(&model.items, &definition_ids)?;
    Ok(model)
}

fn parse_case_file_item_definition(
    node: Node<'_, '_>,
) -> Result<CaseFileItemDefinition, CmmnConverterError> {
    warn_unknown_attributes(node, &["id", "name", "definitionType", "structureRef"]);
    Ok(CaseFileItemDefinition {
        id: required_attribute(node, "caseFileItemDefinition", "id")?.to_string(),
        name: normalized_attribute(node, "name"),
        definition_type: normalized_attribute(node, "definitionType"),
        structure_ref: normalized_attribute(node, "structureRef"),
    })
}

fn parse_case_file_item(node: Node<'_, '_>) -> Result<CaseFileItem, CmmnConverterError> {
    warn_unknown_attributes(node, &["id", "name", "definitionRef"]);
    let mut children = Vec::new();
    for child in element_children(node) {
        match child.tag_name().name() {
            "caseFileItem" => children.push(parse_case_file_item(child)?),
            other => skip_unknown_child("caseFileItem", other),
        }
    }
    Ok(CaseFileItem {
        id: required_attribute(node, "caseFileItem", "id")?.to_string(),
        name: normalized_attribute(node, "name"),
        definition_ref: required_attribute(node, "caseFileItem", "definitionRef")?.to_string(),
        children,
    })
}

fn validate_case_file_item_refs(
    items: &[CaseFileItem],
    definitions: &HashSet<&str>,
) -> Result<(), CmmnConverterError> {
    for item in items {
        if !definitions.contains(item.definition_ref.as_str()) {
            return Err(CmmnConverterError::Structural(format!(
                "caseFileItem `{}` references unknown definition `{}`",
                item.id, item.definition_ref
            )));
        }
        validate_case_file_item_refs(&item.children, definitions)?;
    }
    Ok(())
}

fn parse_case_plan_model(node: Node<'_, '_>) -> Result<CasePlanModel, CmmnConverterError> {
    let parsed = parse_plan_item_container(node, "casePlanModel")?;

    Ok(CasePlanModel {
        id: parsed.id,
        name: parsed.name,
        auto_complete: parsed.auto_complete,
        form_key: parsed.form_key,
        plan_items: parsed.plan_items,
        human_tasks: parsed.human_tasks,
        decision_tasks: parsed.decision_tasks,
        process_tasks: parsed.process_tasks,
        case_tasks: parsed.case_tasks,
        milestones: parsed.milestones,
        event_listeners: parsed.event_listeners,
        sentries: parsed.sentries,
        planning_tables: parsed.planning_tables,
        stages: parsed.stages,
        lifecycle_listeners: parsed.lifecycle_listeners,
    })
}

fn parse_stage(node: Node<'_, '_>) -> Result<Stage, CmmnConverterError> {
    let parsed = parse_plan_item_container(node, "stage")?;

    Ok(Stage {
        id: parsed.id,
        name: parsed.name,
        auto_complete: parsed.auto_complete,
        plan_items: parsed.plan_items,
        human_tasks: parsed.human_tasks,
        decision_tasks: parsed.decision_tasks,
        process_tasks: parsed.process_tasks,
        case_tasks: parsed.case_tasks,
        milestones: parsed.milestones,
        event_listeners: parsed.event_listeners,
        sentries: parsed.sentries,
        planning_tables: parsed.planning_tables,
        stages: parsed.stages,
        lifecycle_listeners: parsed.lifecycle_listeners,
    })
}

fn parse_plan_item_container(
    node: Node<'_, '_>,
    element: &'static str,
) -> Result<ParsedContainer, CmmnConverterError> {
    warn_unknown_attributes(node, &["id", "name", "autoComplete", "formKey"]);
    let id = required_attribute(node, element, "id")?.to_string();
    let name = node.attribute("name").map(ToOwned::to_owned);
    let auto_complete = parse_boolean_attribute(node, element, "autoComplete", false)?;
    let form_key = normalized_attribute(node, "formKey");

    let mut plan_items = Vec::new();
    let mut human_tasks = Vec::new();
    let mut decision_tasks = Vec::new();
    let mut process_tasks = Vec::new();
    let mut case_tasks = Vec::new();
    let mut milestones = Vec::new();
    let mut event_listeners = Vec::new();
    let mut sentries = Vec::new();
    let mut planning_tables = Vec::new();
    let mut stages = Vec::new();
    let mut lifecycle_listeners = Vec::new();

    for child in element_children(node) {
        match child.tag_name().name() {
            "planItem" => plan_items.push(parse_plan_item(child)?),
            "humanTask" => human_tasks.push(parse_human_task(child)?),
            "decisionTask" => decision_tasks.push(parse_decision_task(child)?),
            "processTask" => process_tasks.push(parse_process_task(child)?),
            "caseTask" => case_tasks.push(parse_case_task(child)?),
            "milestone" => milestones.push(parse_milestone(child)?),
            "eventListener" => event_listeners.push(parse_event_listener(child)?),
            "timerEventListener" => event_listeners.push(parse_timer_event_listener(child)?),
            "sentry" => sentries.push(parse_sentry(child)?),
            "planningTable" => planning_tables.push(parse_planning_table(child)?),
            "stage" => stages.push(parse_stage(child)?),
            // Java: ExtensionElementsXMLConverter.java:121-124 harvests
            // planItemLifecycleListener onto the current element; a casePlanModel/stage is a
            // Stage extends PlanItemDefinition implements HasLifecycleListeners
            // (PlanItemDefinition.java:21).
            "extensionElements" => {
                lifecycle_listeners.extend(parse_lifecycle_listeners_in_extension_elements(
                    element,
                    child,
                    "planItemLifecycleListener",
                ));
            }
            other => skip_unknown_child(element, other),
        }
    }

    validate_local_definition_refs(
        element,
        &id,
        &plan_items,
        &human_tasks,
        &decision_tasks,
        &process_tasks,
        &case_tasks,
        &milestones,
        &event_listeners,
        &stages,
        &planning_tables,
    )?;
    validate_sentries(
        element,
        &id,
        &plan_items,
        &human_tasks,
        &milestones,
        &event_listeners,
        &stages,
        &sentries,
    )?;
    validate_nested_uniqueness(
        element,
        &id,
        &plan_items,
        &human_tasks,
        &decision_tasks,
        &process_tasks,
        &case_tasks,
        &milestones,
        &event_listeners,
        &stages,
        &planning_tables,
    )?;

    Ok(ParsedContainer {
        id,
        name,
        auto_complete,
        form_key,
        plan_items,
        human_tasks,
        decision_tasks,
        process_tasks,
        case_tasks,
        milestones,
        event_listeners,
        sentries,
        planning_tables,
        stages,
        lifecycle_listeners,
    })
}

fn parse_plan_item(node: Node<'_, '_>) -> Result<PlanItem, CmmnConverterError> {
    warn_unknown_attributes(node, &["id", "name", "definitionRef"]);
    let id = required_attribute(node, "planItem", "id")?.to_string();
    let name = node.attribute("name").map(ToOwned::to_owned);
    let definition_ref = required_attribute(node, "planItem", "definitionRef")?.to_string();

    let mut entry_criteria = Vec::new();
    let mut exit_criteria = Vec::new();
    let mut manual_activation_rule = None;
    let mut repetition_rule = None;
    let mut required_rule = None;
    let mut parent_completion_rule = None;
    let mut completion_neutral_rule = None;
    for child in element_children(node) {
        match child.tag_name().name() {
            "entryCriterion" => entry_criteria.push(parse_entry_criterion(child)?),
            "exitCriterion" => exit_criteria.push(parse_exit_criterion(child)?),
            "itemControl" => {
                if manual_activation_rule.is_some() || repetition_rule.is_some() {
                    return Err(CmmnConverterError::Structural(format!(
                        "planItem `{id}` must contain at most one `itemControl` in the owned M16 subset"
                    )));
                }
                let parsed = parse_plan_item_control(child, &id)?;
                manual_activation_rule = parsed.manual_activation_rule;
                repetition_rule = parsed.repetition_rule;
                required_rule = parsed.required_rule;
                parent_completion_rule = parsed.parent_completion_rule;
                completion_neutral_rule = parsed.completion_neutral_rule;
            }
            other => skip_unknown_child("planItem", other),
        }
    }

    Ok(PlanItem {
        id,
        name,
        definition_ref,
        entry_criteria,
        exit_criteria,
        manual_activation_rule,
        repetition_rule,
        required_rule,
        parent_completion_rule,
        completion_neutral_rule,
    })
}

fn parse_planning_table(node: Node<'_, '_>) -> Result<PlanningTable, CmmnConverterError> {
    warn_unknown_attributes(node, &["id", "name"]);
    let id = required_attribute(node, "planningTable", "id")?.to_string();
    let name = node.attribute("name").map(ToOwned::to_owned);
    let mut discretionary_items = Vec::new();

    for child in element_children(node) {
        match child.tag_name().name() {
            "discretionaryItem" => discretionary_items.push(parse_discretionary_item(child)?),
            other => skip_unknown_child("planningTable", other),
        }
    }

    Ok(PlanningTable {
        id,
        name,
        discretionary_items,
    })
}

fn parse_discretionary_item(node: Node<'_, '_>) -> Result<DiscretionaryItem, CmmnConverterError> {
    warn_unknown_attributes(node, &["id", "name", "definitionRef"]);
    let id = required_attribute(node, "discretionaryItem", "id")?.to_string();
    let name = node.attribute("name").map(ToOwned::to_owned);
    let definition_ref =
        required_attribute(node, "discretionaryItem", "definitionRef")?.to_string();

    // Java: CmmnXmlConverter.java:222-226 skips unregistered children; discretionaryItem
    // itself has no dedicated child converters beyond attributes.
    skip_all_unknown_children("discretionaryItem", node);

    Ok(DiscretionaryItem {
        id,
        name,
        definition_ref,
    })
}

fn parse_plan_item_control(
    node: Node<'_, '_>,
    plan_item_id: &str,
) -> Result<ParsedPlanItemControl, CmmnConverterError> {
    warn_unknown_attributes(node, &[]);
    let mut manual_activation_rule = None;
    let mut repetition_rule = None;
    let mut required_rule = None;
    let mut parent_completion_rule = None;
    let mut completion_neutral_rule = None;

    for child in element_children(node) {
        match child.tag_name().name() {
            "manualActivationRule" => {
                if manual_activation_rule.is_some() {
                    return Err(CmmnConverterError::Structural(format!(
                        "planItem `{plan_item_id}` itemControl must contain at most one `manualActivationRule` in the owned M16 subset"
                    )));
                }
                manual_activation_rule = Some(parse_plan_item_rule_condition(
                    child,
                    plan_item_id,
                    "manualActivationRule",
                )?);
            }
            "repetitionRule" => {
                if repetition_rule.is_some() {
                    return Err(CmmnConverterError::Structural(format!(
                        "planItem `{plan_item_id}` itemControl must contain at most one `repetitionRule` in the owned M16 subset"
                    )));
                }
                repetition_rule = Some(parse_plan_item_rule_condition(
                    child,
                    plan_item_id,
                    "repetitionRule",
                )?);
            }
            "requiredRule" => {
                if required_rule.is_some() {
                    return Err(CmmnConverterError::Structural(format!(
                        "planItem `{plan_item_id}` itemControl must contain at most one `requiredRule` in the owned M16 subset"
                    )));
                }
                required_rule = Some(parse_plan_item_rule_condition(
                    child,
                    plan_item_id,
                    "requiredRule",
                )?);
            }
            // Java: ParentCompletionRule (parentCompletionRule.getType()) controls whether a
            // child plan item blocks parent completion; see PlanItemInstanceContainerUtil.java.
            "parentCompletionRule" => {
                if parent_completion_rule.is_some() {
                    return Err(CmmnConverterError::Structural(format!(
                        "planItem `{plan_item_id}` itemControl must contain at most one `parentCompletionRule` in the owned M16 subset"
                    )));
                }
                parent_completion_rule = Some(parse_parent_completion_rule(child, plan_item_id)?);
            }
            // Java: completionNeutralRule marks an optional plan item as not preventing parent
            // completion while AVAILABLE (ExpressionUtil.isCompletionNeutralPlanItemInstance).
            "completionNeutralRule" => {
                if completion_neutral_rule.is_some() {
                    return Err(CmmnConverterError::Structural(format!(
                        "planItem `{plan_item_id}` itemControl must contain at most one `completionNeutralRule` in the owned M16 subset"
                    )));
                }
                completion_neutral_rule = Some(parse_plan_item_rule_condition(
                    child,
                    plan_item_id,
                    "completionNeutralRule",
                )?);
            }
            other => skip_unknown_child("itemControl", other),
        }
    }

    Ok(ParsedPlanItemControl {
        manual_activation_rule,
        repetition_rule,
        required_rule,
        parent_completion_rule,
        completion_neutral_rule,
    })
}

// Java: ParentCompletionRule.java defines the supported type constants. A missing/`default`
// type behaves like the standard (non-ignoring) completion evaluation.
fn parse_parent_completion_rule(
    node: Node<'_, '_>,
    plan_item_id: &str,
) -> Result<String, CmmnConverterError> {
    warn_unknown_attributes(node, &["type"]);
    // Java ignores unexpected children on parentCompletionRule (attribute-driven type only).
    skip_all_unknown_children("parentCompletionRule", node);
    let rule_type = required_attribute(node, "parentCompletionRule", "type")?.to_string();
    match rule_type.as_str() {
        "default"
        | "ignore"
        | "ignoreIfAvailable"
        | "ignoreIfAvailableOrEnabled"
        | "ignoreAfterFirstCompletion"
        | "ignoreAfterFirstCompletionIfAvailableOrEnabled" => Ok(rule_type),
        other => Err(CmmnConverterError::Structural(format!(
            "planItem `{plan_item_id}` parentCompletionRule type `{other}` is unsupported in the owned M16 subset"
        ))),
    }
}

fn parse_plan_item_rule_condition(
    node: Node<'_, '_>,
    plan_item_id: &str,
    rule_name: &'static str,
) -> Result<SentryIfPartExpression, CmmnConverterError> {
    warn_unknown_attributes(node, &[]);
    let mut condition = None;

    for child in element_children(node) {
        match child.tag_name().name() {
            "condition" => {
                if condition.is_some() {
                    return Err(CmmnConverterError::Structural(format!(
                        "planItem `{plan_item_id}` {rule_name} must contain exactly one `condition` in the owned M16 subset"
                    )));
                }
                warn_unknown_attributes(child, &[]);
                // Java ConditionXmlConverter only reads text content; nested elements are ignored
                // by the unregistered-element path (CmmnXmlConverter.java:222-226).
                skip_all_unknown_children("condition", child);
                condition = Some(normalize_plan_item_rule_condition(
                    child.text().unwrap_or_default(),
                    plan_item_id,
                    rule_name,
                )?);
            }
            other => skip_unknown_child(rule_name, other),
        }
    }

    condition.ok_or_else(|| {
        CmmnConverterError::Structural(format!(
            "planItem `{plan_item_id}` {rule_name} must contain exactly one `condition` in the owned M16 subset"
        ))
    })
}

fn normalize_plan_item_rule_condition(
    expression: &str,
    plan_item_id: &str,
    rule_name: &'static str,
) -> Result<SentryIfPartExpression, CmmnConverterError> {
    let expression = expression.trim();
    let expression = expression
        .strip_prefix("${")
        .and_then(|value| value.strip_suffix('}'))
        .unwrap_or(expression)
        .trim();
    parse_sentry_if_part_expression(expression).map_err(|message| {
        CmmnConverterError::Structural(format!(
            "planItem `{plan_item_id}` {rule_name} condition `{expression}` is unsupported in the owned M16 subset; {message}"
        ))
    })
}

fn parse_entry_criterion(node: Node<'_, '_>) -> Result<EntryCriterion, CmmnConverterError> {
    parse_criterion(node, "entryCriterion")
}

fn parse_exit_criterion(node: Node<'_, '_>) -> Result<EntryCriterion, CmmnConverterError> {
    parse_criterion(node, "exitCriterion")
}

fn parse_criterion(
    node: Node<'_, '_>,
    element_name: &'static str,
) -> Result<EntryCriterion, CmmnConverterError> {
    warn_unknown_attributes(node, &["id", "sentryRef"]);

    let mut sentry_ref = node.attribute("sentryRef").map(ToOwned::to_owned);

    for child in element_children(node) {
        if child.tag_name().name() == "sentryRef" {
            sentry_ref = Some(child.text().unwrap_or_default().trim().to_string());
        } else {
            // Java: CmmnXmlConverter.java:222-226 skips unregistered criterion children.
            skip_unknown_child(element_name, child.tag_name().name());
        }
    }

    let sentry_ref = sentry_ref.ok_or_else(|| {
        CmmnConverterError::Structural(format!("{element_name} has no sentryRef"))
    })?;

    Ok(EntryCriterion {
        id: required_attribute(node, element_name, "id")?.to_string(),
        sentry_ref,
    })
}

fn parse_human_task(node: Node<'_, '_>) -> Result<HumanTask, CmmnConverterError> {
    warn_unknown_attributes(
        node,
        &[
            "id",
            "name",
            "isBlocking",
            "formKey",
            "assignee",
            "owner",
            "priority",
            "dueDate",
            "category",
            "candidateUsers",
            "candidateGroups",
            "taskIdVariableName",
            "taskCompleterVariableName",
        ],
    );

    // Java HumanTaskXmlConverter has no required children; extensionElements is handled by
    // ExtensionElementsXMLConverter, which harvests planItemLifecycleListener entries
    // (ExtensionElementsXMLConverter.java:121-124). Other children are skipped
    // (CmmnXmlConverter.java:222-226).
    let lifecycle_listeners =
        parse_lifecycle_listeners("humanTask", node, "planItemLifecycleListener");

    // Java parity: HumanTaskXmlConverter.java:37-61 reads the flowable extension
    // attributes; candidateUsers/candidateGroups are comma-delimited lists
    // (CmmnXmlUtil.parseDelimitedList).
    Ok(HumanTask {
        id: required_attribute(node, "humanTask", "id")?.to_string(),
        name: node.attribute("name").map(ToOwned::to_owned),
        is_blocking: parse_boolean_attribute(node, "humanTask", "isBlocking", true)?,
        form_key: normalized_attribute(node, "formKey"),
        assignee: normalized_attribute(node, "assignee"),
        owner: normalized_attribute(node, "owner"),
        priority: normalized_attribute(node, "priority"),
        due_date: normalized_attribute(node, "dueDate"),
        category: normalized_attribute(node, "category"),
        candidate_users: parse_delimited_list(normalized_attribute(node, "candidateUsers")),
        candidate_groups: parse_delimited_list(normalized_attribute(node, "candidateGroups")),
        task_id_variable_name: normalized_attribute(node, "taskIdVariableName"),
        task_completer_variable_name: normalized_attribute(node, "taskCompleterVariableName"),
        lifecycle_listeners,
    })
}

fn parse_decision_task(node: Node<'_, '_>) -> Result<DecisionTask, CmmnConverterError> {
    warn_unknown_attributes(node, &["id", "name", "decisionRef"]);
    // Java: DecisionTaskXmlConverter + DecisionRefExpressionXmlConverter; extensionElements is
    // handled by ExtensionElementsXMLConverter (planItemLifecycleListener,
    // ExtensionElementsXMLConverter.java:121-124), other children are skipped
    // (CmmnXmlConverter.java:222-226).
    let lifecycle_listeners =
        parse_lifecycle_listeners("decisionTask", node, "planItemLifecycleListener");

    Ok(DecisionTask {
        id: required_attribute(node, "decisionTask", "id")?.to_string(),
        name: node.attribute("name").map(ToOwned::to_owned),
        decision_ref: normalized_attribute(node, "decisionRef"),
        lifecycle_listeners,
    })
}

fn parse_process_task(node: Node<'_, '_>) -> Result<ProcessTask, CmmnConverterError> {
    warn_unknown_attributes(node, &["id", "name", "isBlocking", "processRef"]);
    // Java: ProcessTaskXmlConverter; extensionElements is handled by
    // ExtensionElementsXMLConverter (planItemLifecycleListener,
    // ExtensionElementsXMLConverter.java:121-124), other children skipped
    // (CmmnXmlConverter.java:222-226).
    let lifecycle_listeners =
        parse_lifecycle_listeners("processTask", node, "planItemLifecycleListener");

    Ok(ProcessTask {
        id: required_attribute(node, "processTask", "id")?.to_string(),
        name: node.attribute("name").map(ToOwned::to_owned),
        is_blocking: parse_boolean_attribute(node, "processTask", "isBlocking", true)?,
        process_ref: normalized_attribute(node, "processRef"),
        lifecycle_listeners,
    })
}

fn parse_case_task(node: Node<'_, '_>) -> Result<CaseTask, CmmnConverterError> {
    warn_unknown_attributes(node, &["id", "name", "isBlocking", "caseRef"]);
    // Java: CaseTaskXmlConverter; extensionElements is handled by ExtensionElementsXMLConverter
    // (planItemLifecycleListener, ExtensionElementsXMLConverter.java:121-124), other children
    // skipped (CmmnXmlConverter.java:222-226).
    let lifecycle_listeners =
        parse_lifecycle_listeners("caseTask", node, "planItemLifecycleListener");

    Ok(CaseTask {
        id: required_attribute(node, "caseTask", "id")?.to_string(),
        name: node.attribute("name").map(ToOwned::to_owned),
        is_blocking: parse_boolean_attribute(node, "caseTask", "isBlocking", true)?,
        case_ref: normalized_attribute(node, "caseRef"),
        lifecycle_listeners,
    })
}

fn parse_milestone(node: Node<'_, '_>) -> Result<Milestone, CmmnConverterError> {
    warn_unknown_attributes(node, &["id", "name"]);
    // Java: MilestoneXmlConverter; extensionElements is handled by ExtensionElementsXMLConverter
    // (planItemLifecycleListener, ExtensionElementsXMLConverter.java:121-124), other children
    // skipped (CmmnXmlConverter.java:222-226).
    let lifecycle_listeners =
        parse_lifecycle_listeners("milestone", node, "planItemLifecycleListener");

    Ok(Milestone {
        id: required_attribute(node, "milestone", "id")?.to_string(),
        name: node.attribute("name").map(ToOwned::to_owned),
        lifecycle_listeners,
    })
}

fn parse_event_listener(node: Node<'_, '_>) -> Result<EventListener, CmmnConverterError> {
    // Java GenericEventListenerXmlConverter.java:68-73 also reads flowable:availableCondition.
    warn_unknown_attributes(node, &["id", "name", "eventType", "eventName", "availableCondition"]);
    // Java: extensionElements is handled by ExtensionElementsXMLConverter
    // (planItemLifecycleListener, ExtensionElementsXMLConverter.java:121-124) — an event listener
    // is a PlanItemDefinition and therefore HasLifecycleListeners (PlanItemDefinition.java:21).
    // Other children are skipped (CmmnXmlConverter.java:222-226).
    let lifecycle_listeners =
        parse_lifecycle_listeners("eventListener", node, "planItemLifecycleListener");

    let event_type = normalized_attribute(node, "eventType").ok_or_else(|| {
        CmmnConverterError::MissingAttribute {
            element: "eventListener".to_string(),
            attribute: "eventType",
        }
    })?;

    Ok(EventListener {
        id: required_attribute(node, "eventListener", "id")?.to_string(),
        name: node.attribute("name").map(ToOwned::to_owned),
        event_type,
        event_name: normalized_attribute(node, "eventName"),
        available_condition: normalized_attribute(node, "availableCondition"),
        timer_expression: None,
        lifecycle_listeners,
    })
}

/// Java parity: `TimerEventListenerXmlConverter.java:36-44` (name + availableCondition
/// attributes) + `TimerExpressionXmlConverter.java:39-49` (timerExpression child text).
/// A timerEventListener has no eventType attribute; it is marked internally with
/// `EventListener::EVENT_TYPE_TIMER` and carries the parsed timerExpression.
fn parse_timer_event_listener(node: Node<'_, '_>) -> Result<EventListener, CmmnConverterError> {
    warn_unknown_attributes(node, &["id", "name", "availableCondition"]);

    let mut timer_expression = None;
    let mut lifecycle_listeners = Vec::new();
    for child in element_children(node) {
        match child.tag_name().name() {
            "timerExpression" => {
                if timer_expression.is_some() {
                    return Err(CmmnConverterError::Structural(format!(
                        "timerEventListener `{}` must contain at most one `timerExpression`",
                        node.attribute("id").unwrap_or("")
                    )));
                }
                let text = child.text().unwrap_or("").trim();
                // Java TimerExpressionXmlConverter.java:42-44 only sets the expression when
                // non-empty; an empty <timerExpression/> leaves it unset.
                if !text.is_empty() {
                    timer_expression = Some(text.to_string());
                }
            }
            // Java: ExtensionElementsXMLConverter.java:121-124 harvests
            // planItemLifecycleListener onto the current PlanItemDefinition
            // (PlanItemDefinition.java:21 implements HasLifecycleListeners).
            "extensionElements" => {
                lifecycle_listeners.extend(parse_lifecycle_listeners_in_extension_elements(
                    "timerEventListener",
                    child,
                    "planItemLifecycleListener",
                ));
            }
            other => skip_unknown_child("timerEventListener", other),
        }
    }

    Ok(EventListener {
        id: required_attribute(node, "timerEventListener", "id")?.to_string(),
        name: node.attribute("name").map(ToOwned::to_owned),
        event_type: EventListener::EVENT_TYPE_TIMER.to_string(),
        event_name: None,
        available_condition: normalized_attribute(node, "availableCondition"),
        timer_expression,
        lifecycle_listeners,
    })
}

fn parse_sentry(node: Node<'_, '_>) -> Result<Sentry, CmmnConverterError> {
    // Java SentryXmlConverter.java:38-39 also reads name + flowable:triggerMode; we allow
    // them (and any other unknown attrs) via warn_unknown_attributes rather than reject.
    warn_unknown_attributes(node, &["id", "name", "triggerMode"]);
    let id = required_attribute(node, "sentry", "id")?.to_string();
    let mut plan_item_on_parts = Vec::new();
    let mut case_file_item_on_parts = Vec::new();
    let mut if_part = None;

    for child in element_children(node) {
        match child.tag_name().name() {
            "planItemOnPart" => {
                plan_item_on_parts.push(parse_plan_item_on_part(child)?);
            }
            // CMMN 1.1 XSD tCaseFileItemOnPart (CMMN11CaseModel.xsd:1027-1042): sourceRef +
            // standardEvent. Java open-source CmmnXmlConverter has no CaseFileItemOnPart
            // converter registered (CmmnXmlConverter.java:96-141) so it silently skips;
            // Rust engine already evaluates case_file_item_on_parts, so we parse here.
            "caseFileItemOnPart" => {
                case_file_item_on_parts.push(parse_case_file_item_on_part(child)?);
            }
            "ifPart" => {
                if if_part.is_some() {
                    return Err(CmmnConverterError::Structural(format!(
                        "sentry `{id}` must contain at most one `ifPart` in the owned M16 subset"
                    )));
                }
                if_part = Some(parse_if_part(child, &id)?);
            }
            other => skip_unknown_child("sentry", other),
        }
    }

    if plan_item_on_parts.is_empty() && case_file_item_on_parts.is_empty() && if_part.is_none() {
        return Err(CmmnConverterError::Structural(format!(
            "sentry `{id}` must contain at least one `planItemOnPart`, `caseFileItemOnPart`, or `ifPart`"
        )));
    }

    Ok(Sentry {
        id: id.clone(),
        plan_item_on_parts,
        case_file_item_on_parts,
        if_part,
    })
}

fn parse_if_part(
    node: Node<'_, '_>,
    sentry_id: &str,
) -> Result<SentryIfPartExpression, CmmnConverterError> {
    warn_unknown_attributes(node, &[]);
    let mut condition = None;

    for child in element_children(node) {
        match child.tag_name().name() {
            "condition" => {
                if condition.is_some() {
                    return Err(CmmnConverterError::Structural(format!(
                        "sentry `{sentry_id}` ifPart must contain exactly one `condition` in the owned M16 subset"
                    )));
                }
                warn_unknown_attributes(child, &[]);
                // Java ConditionXmlConverter only reads text; nested elements are skipped
                // (CmmnXmlConverter.java:222-226).
                skip_all_unknown_children("condition", child);
                condition = Some(normalize_if_part_condition(
                    child.text().unwrap_or_default(),
                    sentry_id,
                )?);
            }
            other => skip_unknown_child("ifPart", other),
        }
    }

    condition.ok_or_else(|| {
        CmmnConverterError::Structural(format!(
            "sentry `{sentry_id}` ifPart must contain exactly one `condition` in the owned M16 subset"
        ))
    })
}

fn normalize_if_part_condition(
    expression: &str,
    sentry_id: &str,
) -> Result<SentryIfPartExpression, CmmnConverterError> {
    let expression = expression.trim();
    let expression = expression
        .strip_prefix("${")
        .and_then(|value| value.strip_suffix('}'))
        .unwrap_or(expression)
        .trim();
    parse_sentry_if_part_expression(expression).map_err(|message| {
        CmmnConverterError::Structural(format!(
            "sentry `{sentry_id}` ifPart condition `{expression}` is unsupported in the owned M16 subset; {message}"
        ))
    })
}

fn parse_plan_item_on_part(node: Node<'_, '_>) -> Result<PlanItemOnPart, CmmnConverterError> {
    // Java PlanItemOnPartXmlConverter.java:38-39 also reads name; exitCriterionRef is in XSD.
    warn_unknown_attributes(node, &["id", "name", "sourceRef", "exitCriterionRef"]);
    let id = required_attribute(node, "planItemOnPart", "id")?.to_string();
    let source_ref = required_attribute(node, "planItemOnPart", "sourceRef")?.to_string();
    let mut standard_event = None;

    for child in element_children(node) {
        match child.tag_name().name() {
            "standardEvent" => {
                if standard_event.is_some() {
                    return Err(CmmnConverterError::Structural(format!(
                        "planItemOnPart `{id}` must contain exactly one `standardEvent` in the owned M16 subset"
                    )));
                }
                warn_unknown_attributes(child, &[]);
                // Java StandardEventXmlConverter only reads text content.
                skip_all_unknown_children("standardEvent", child);
                let event = child.text().unwrap_or_default().trim().to_string();
                if !is_bounded_plan_item_on_part_standard_event(&event) {
                    return Err(CmmnConverterError::Structural(format!(
                        "planItemOnPart `{id}` standardEvent `{event}` is unsupported; owned M16 subset supports only `complete`, `occur`, `terminate`, `start`, `enable`, `disable`, and `exit`"
                    )));
                }
                standard_event = Some(event);
            }
            other => skip_unknown_child("planItemOnPart", other),
        }
    }

    Ok(PlanItemOnPart {
        id: id.clone(),
        source_ref,
        standard_event: standard_event.ok_or_else(|| {
            CmmnConverterError::Structural(format!(
                "planItemOnPart `{id}` must contain exactly one `standardEvent` in the owned M16 subset"
            ))
        })?,
    })
}

/// Parse `<caseFileItemOnPart sourceRef="..." > <standardEvent>...</standardEvent>`.
///
/// XSD: `tCaseFileItemOnPart` in CMMN11CaseModel.xsd:1027-1042 (`sourceRef` → caseFileItem,
/// `standardEvent` of type `CaseFileItemTransition`).
/// Model fields: `case_file_item_ref` ← `sourceRef`, `standard_event` ← child text.
fn parse_case_file_item_on_part(
    node: Node<'_, '_>,
) -> Result<CaseFileItemOnPart, CmmnConverterError> {
    warn_unknown_attributes(node, &["id", "name", "sourceRef"]);
    let id = required_attribute(node, "caseFileItemOnPart", "id")?.to_string();
    let case_file_item_ref =
        required_attribute(node, "caseFileItemOnPart", "sourceRef")?.to_string();
    let mut standard_event = None;

    for child in element_children(node) {
        match child.tag_name().name() {
            "standardEvent" => {
                if standard_event.is_some() {
                    return Err(CmmnConverterError::Structural(format!(
                        "caseFileItemOnPart `{id}` must contain exactly one `standardEvent`"
                    )));
                }
                warn_unknown_attributes(child, &[]);
                skip_all_unknown_children("standardEvent", child);
                let event = child.text().unwrap_or_default().trim().to_string();
                if !CaseFileItemOnPart::is_supported_standard_event(&event) {
                    return Err(CmmnConverterError::Structural(format!(
                        "caseFileItemOnPart `{id}` standardEvent `{event}` is unsupported; supported events are `create`, `update`, `delete`, and `complete`"
                    )));
                }
                standard_event = Some(event);
            }
            other => skip_unknown_child("caseFileItemOnPart", other),
        }
    }

    Ok(CaseFileItemOnPart {
        id: id.clone(),
        case_file_item_ref,
        standard_event: standard_event.ok_or_else(|| {
            CmmnConverterError::Structural(format!(
                "caseFileItemOnPart `{id}` must contain exactly one `standardEvent`"
            ))
        })?,
    })
}

fn validate_sentries(
    element: &'static str,
    id: &str,
    plan_items: &[PlanItem],
    human_tasks: &[HumanTask],
    milestones: &[Milestone],
    event_listeners: &[EventListener],
    stages: &[Stage],
    sentries: &[Sentry],
) -> Result<(), CmmnConverterError> {
    let plan_item_ids = plan_items
        .iter()
        .map(|plan_item| plan_item.id.as_str())
        .collect::<HashSet<_>>();
    let event_listener_ids = event_listeners
        .iter()
        .map(|event_listener| event_listener.id.as_str())
        .collect::<HashSet<_>>();
    let human_task_ids = human_tasks
        .iter()
        .map(|human_task| human_task.id.as_str())
        .collect::<HashSet<_>>();
    let milestone_ids = milestones
        .iter()
        .map(|milestone| milestone.id.as_str())
        .collect::<HashSet<_>>();
    let stage_ids = stages
        .iter()
        .map(|stage| stage.id.as_str())
        .collect::<HashSet<_>>();
    let mut sentry_ids = HashSet::new();
    for sentry in sentries {
        if !sentry_ids.insert(sentry.id.as_str()) {
            return Err(CmmnConverterError::Structural(format!(
                "duplicate sentry id `{}` inside `{element}` `{id}` is not allowed in the owned M16 subset",
                sentry.id
            )));
        }
        let mut on_part_ids = HashSet::new();
        for on_part in &sentry.plan_item_on_parts {
            if !on_part_ids.insert(on_part.id.as_str()) {
                return Err(CmmnConverterError::Structural(format!(
                    "duplicate planItemOnPart id `{}` inside sentry `{}` is not allowed in the owned M16 subset",
                    on_part.id, sentry.id
                )));
            }
            if !plan_item_ids.contains(on_part.source_ref.as_str()) {
                return Err(CmmnConverterError::Structural(format!(
                    "sentry `{}` sourceRef `{}` inside `{element}` `{id}` must reference a direct planItem in the owned M16 subset",
                    sentry.id, on_part.source_ref
                )));
            }
            let source_definition_ref = plan_items
                .iter()
                .find(|plan_item| plan_item.id == on_part.source_ref)
                .map(|plan_item| plan_item.definition_ref.as_str());
            if on_part.standard_event == PlanItemOnPart::STANDARD_EVENT_OCCUR
                && !source_definition_ref.is_some_and(|definition_ref| {
                    event_listener_ids.contains(definition_ref)
                        || milestone_ids.contains(definition_ref)
                })
            {
                return Err(CmmnConverterError::Structural(format!(
                    "sentry `{}` sourceRef `{}` inside `{element}` `{id}` uses standardEvent `occur`, which requires a direct eventListener or milestone planItem source in the owned M16 subset",
                    sentry.id, on_part.source_ref
                )));
            }
            if matches!(on_part.standard_event.as_str(), "enable" | "disable")
                && !source_definition_ref
                    .is_some_and(|definition_ref| human_task_ids.contains(definition_ref))
            {
                return Err(CmmnConverterError::Structural(format!(
                    "sentry `{}` sourceRef `{}` inside `{element}` `{id}` uses standardEvent `{}`, which requires a direct humanTask planItem source in the owned M16 subset",
                    sentry.id, on_part.source_ref, on_part.standard_event
                )));
            }
            if on_part.standard_event == PlanItemOnPart::STANDARD_EVENT_START
                && !source_definition_ref.is_some_and(|definition_ref| {
                    human_task_ids.contains(definition_ref) || stage_ids.contains(definition_ref)
                })
            {
                return Err(CmmnConverterError::Structural(format!(
                    "sentry `{}` sourceRef `{}` inside `{element}` `{id}` uses standardEvent `start`, which requires a direct humanTask or stage planItem source in the owned M16 subset",
                    sentry.id, on_part.source_ref
                )));
            }
            if on_part.standard_event == PlanItemOnPart::STANDARD_EVENT_EXIT
                && !source_definition_ref.is_some_and(|definition_ref| {
                    human_task_ids.contains(definition_ref) || stage_ids.contains(definition_ref)
                })
            {
                return Err(CmmnConverterError::Structural(format!(
                    "sentry `{}` sourceRef `{}` inside `{element}` `{id}` uses standardEvent `exit`, which requires a direct humanTask or stage planItem source in the owned M16 subset",
                    sentry.id, on_part.source_ref
                )));
            }
        }
    }

    let mut criterion_ids = HashSet::new();
    for plan_item in plan_items {
        for (criterion_type, criteria) in [
            ("entryCriterion", &plan_item.entry_criteria),
            ("exitCriterion", &plan_item.exit_criteria),
        ] {
            for criterion in criteria {
                if !criterion_ids.insert(criterion.id.as_str()) {
                    return Err(CmmnConverterError::Structural(format!(
                        "duplicate {criterion_type} id `{}` inside `{element}` `{id}` is not allowed in the owned M16 subset",
                        criterion.id
                    )));
                }
                if !sentry_ids.contains(criterion.sentry_ref.as_str()) {
                    return Err(CmmnConverterError::Structural(format!(
                        "{criterion_type} `{}` sentryRef `{}` inside `{element}` `{id}` must reference a direct sentry in the owned M16 subset",
                        criterion.id, criterion.sentry_ref
                    )));
                }
            }
        }
    }

    Ok(())
}

fn is_bounded_plan_item_on_part_standard_event(value: &str) -> bool {
    matches!(
        value,
        PlanItemOnPart::STANDARD_EVENT_COMPLETE
            | PlanItemOnPart::STANDARD_EVENT_OCCUR
            | PlanItemOnPart::STANDARD_EVENT_TERMINATE
            | PlanItemOnPart::STANDARD_EVENT_START
            | PlanItemOnPart::STANDARD_EVENT_ENABLE
            | PlanItemOnPart::STANDARD_EVENT_DISABLE
            | PlanItemOnPart::STANDARD_EVENT_EXIT
    )
}

#[allow(clippy::too_many_arguments)]
fn validate_local_definition_refs(
    element: &'static str,
    id: &str,
    plan_items: &[PlanItem],
    human_tasks: &[HumanTask],
    decision_tasks: &[DecisionTask],
    process_tasks: &[ProcessTask],
    case_tasks: &[CaseTask],
    milestones: &[Milestone],
    event_listeners: &[EventListener],
    stages: &[Stage],
    planning_tables: &[PlanningTable],
) -> Result<(), CmmnConverterError> {
    let mut local_definition_ids = HashSet::new();
    for human_task in human_tasks {
        if !local_definition_ids.insert(human_task.id.as_str()) {
            return Err(CmmnConverterError::Structural(format!(
                "duplicate direct definition id `{}` inside `{element}` `{id}` is not allowed in the owned M16 subset",
                human_task.id
            )));
        }
    }

    for decision_task in decision_tasks {
        if !local_definition_ids.insert(decision_task.id.as_str()) {
            return Err(CmmnConverterError::Structural(format!(
                "duplicate direct definition id `{}` inside `{element}` `{id}` is not allowed in the owned M16 subset",
                decision_task.id
            )));
        }
    }

    for process_task in process_tasks {
        if !local_definition_ids.insert(process_task.id.as_str()) {
            return Err(CmmnConverterError::Structural(format!(
                "duplicate direct definition id `{}` inside `{element}` `{id}` is not allowed in the owned M16 subset",
                process_task.id
            )));
        }
    }

    for case_task in case_tasks {
        if !local_definition_ids.insert(case_task.id.as_str()) {
            return Err(CmmnConverterError::Structural(format!(
                "duplicate direct definition id `{}` inside `{element}` `{id}` is not allowed in the owned M16 subset",
                case_task.id
            )));
        }
    }

    for milestone in milestones {
        if !local_definition_ids.insert(milestone.id.as_str()) {
            return Err(CmmnConverterError::Structural(format!(
                "duplicate direct definition id `{}` inside `{element}` `{id}` is not allowed in the owned M16 subset",
                milestone.id
            )));
        }
    }

    for event_listener in event_listeners {
        if !local_definition_ids.insert(event_listener.id.as_str()) {
            return Err(CmmnConverterError::Structural(format!(
                "duplicate direct definition id `{}` inside `{element}` `{id}` is not allowed in the owned M16 subset",
                event_listener.id
            )));
        }
    }

    for stage in stages {
        if !local_definition_ids.insert(stage.id.as_str()) {
            return Err(CmmnConverterError::Structural(format!(
                "duplicate direct definition id `{}` inside `{element}` `{id}` is not allowed in the owned M16 subset",
                stage.id
            )));
        }
    }

    let mut local_plan_item_ids = HashSet::new();
    for plan_item in plan_items {
        if !local_plan_item_ids.insert(plan_item.id.as_str()) {
            return Err(CmmnConverterError::Structural(format!(
                "duplicate planItem id `{}` inside `{element}` `{id}` is not allowed in the owned M16 subset",
                plan_item.id
            )));
        }
        if !local_definition_ids.contains(plan_item.definition_ref.as_str()) {
            return Err(CmmnConverterError::Structural(format!(
                "planItem `{}` definitionRef `{}` inside `{element}` `{id}` must reference a direct child definition in the owned M16 subset",
                plan_item.id, plan_item.definition_ref
            )));
        }
    }

    for planning_table in planning_tables {
        let mut discretionary_item_ids = HashSet::new();
        for discretionary_item in &planning_table.discretionary_items {
            if !discretionary_item_ids.insert(discretionary_item.id.as_str()) {
                return Err(CmmnConverterError::Structural(format!(
                    "duplicate discretionaryItem id `{}` inside planningTable `{}` is not allowed in the owned M16 subset",
                    discretionary_item.id, planning_table.id
                )));
            }
            if !local_definition_ids.contains(discretionary_item.definition_ref.as_str()) {
                return Err(CmmnConverterError::Structural(format!(
                    "discretionaryItem `{}` definitionRef `{}` inside planningTable `{}` must reference a direct child definition in the owned M16 subset",
                    discretionary_item.id, discretionary_item.definition_ref, planning_table.id
                )));
            }
        }
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn validate_nested_uniqueness(
    element: &'static str,
    id: &str,
    plan_items: &[PlanItem],
    human_tasks: &[HumanTask],
    decision_tasks: &[DecisionTask],
    process_tasks: &[ProcessTask],
    case_tasks: &[CaseTask],
    milestones: &[Milestone],
    event_listeners: &[EventListener],
    stages: &[Stage],
    planning_tables: &[PlanningTable],
) -> Result<(), CmmnConverterError> {
    let mut plan_item_ids = HashSet::new();
    let mut definition_ids = HashSet::new();
    collect_nested_ids(
        element,
        id,
        plan_items,
        human_tasks,
        decision_tasks,
        process_tasks,
        case_tasks,
        milestones,
        event_listeners,
        stages,
        planning_tables,
        &mut plan_item_ids,
        &mut definition_ids,
    )
}

#[allow(clippy::too_many_arguments)]
fn collect_nested_ids(
    element: &'static str,
    id: &str,
    plan_items: &[PlanItem],
    human_tasks: &[HumanTask],
    decision_tasks: &[DecisionTask],
    process_tasks: &[ProcessTask],
    case_tasks: &[CaseTask],
    milestones: &[Milestone],
    event_listeners: &[EventListener],
    stages: &[Stage],
    planning_tables: &[PlanningTable],
    plan_item_ids: &mut HashSet<String>,
    definition_ids: &mut HashSet<String>,
) -> Result<(), CmmnConverterError> {
    for plan_item in plan_items {
        if !plan_item_ids.insert(plan_item.id.clone()) {
            return Err(CmmnConverterError::Structural(format!(
                "duplicate planItem id `{}` found under `{element}` `{id}`; nested ids must stay unique in the owned M16 subset",
                plan_item.id
            )));
        }
    }

    for human_task in human_tasks {
        if !definition_ids.insert(human_task.id.clone()) {
            return Err(CmmnConverterError::Structural(format!(
                "duplicate plan item definition id `{}` found under `{element}` `{id}`; nested definition ids must stay unique in the owned M16 subset",
                human_task.id
            )));
        }
    }

    for decision_task in decision_tasks {
        if !definition_ids.insert(decision_task.id.clone()) {
            return Err(CmmnConverterError::Structural(format!(
                "duplicate plan item definition id `{}` found under `{element}` `{id}`; nested definition ids must stay unique in the owned M16 subset",
                decision_task.id
            )));
        }
    }

    for process_task in process_tasks {
        if !definition_ids.insert(process_task.id.clone()) {
            return Err(CmmnConverterError::Structural(format!(
                "duplicate plan item definition id `{}` found under `{element}` `{id}`; nested definition ids must stay unique in the owned M16 subset",
                process_task.id
            )));
        }
    }

    for case_task in case_tasks {
        if !definition_ids.insert(case_task.id.clone()) {
            return Err(CmmnConverterError::Structural(format!(
                "duplicate plan item definition id `{}` found under `{element}` `{id}`; nested definition ids must stay unique in the owned M16 subset",
                case_task.id
            )));
        }
    }

    for milestone in milestones {
        if !definition_ids.insert(milestone.id.clone()) {
            return Err(CmmnConverterError::Structural(format!(
                "duplicate plan item definition id `{}` found under `{element}` `{id}`; nested definition ids must stay unique in the owned M16 subset",
                milestone.id
            )));
        }
    }

    for event_listener in event_listeners {
        if !definition_ids.insert(event_listener.id.clone()) {
            return Err(CmmnConverterError::Structural(format!(
                "duplicate plan item definition id `{}` found under `{element}` `{id}`; nested definition ids must stay unique in the owned M16 subset",
                event_listener.id
            )));
        }
    }

    for planning_table in planning_tables {
        if !definition_ids.insert(planning_table.id.clone()) {
            return Err(CmmnConverterError::Structural(format!(
                "duplicate planningTable id `{}` found under `{element}` `{id}`; nested ids must stay unique in the owned M16 subset",
                planning_table.id
            )));
        }
        for discretionary_item in &planning_table.discretionary_items {
            if !plan_item_ids.insert(discretionary_item.id.clone()) {
                return Err(CmmnConverterError::Structural(format!(
                    "duplicate discretionaryItem id `{}` found under `{element}` `{id}`; nested ids must stay unique in the owned M16 subset",
                    discretionary_item.id
                )));
            }
        }
    }

    for stage in stages {
        if !definition_ids.insert(stage.id.clone()) {
            return Err(CmmnConverterError::Structural(format!(
                "duplicate plan item definition id `{}` found under `{element}` `{id}`; nested definition ids must stay unique in the owned M16 subset",
                stage.id
            )));
        }

        collect_nested_ids(
            "stage",
            &stage.id,
            &stage.plan_items,
            &stage.human_tasks,
            &stage.decision_tasks,
            &stage.process_tasks,
            &stage.case_tasks,
            &stage.milestones,
            &stage.event_listeners,
            &stage.stages,
            &stage.planning_tables,
            plan_item_ids,
            definition_ids,
        )?;
    }

    Ok(())
}

fn parse_boolean_attribute(
    node: Node<'_, '_>,
    element: &'static str,
    attribute: &'static str,
    default: bool,
) -> Result<bool, CmmnConverterError> {
    match node.attribute(attribute) {
        Some("true") => Ok(true),
        Some("false") => Ok(false),
        Some(other) => Err(CmmnConverterError::Structural(format!(
            "attribute `{attribute}` on `{element}` must be `true` or `false`, found `{other}`"
        ))),
        None => Ok(default),
    }
}

fn required_attribute<'a>(
    node: Node<'a, 'a>,
    element: &'static str,
    attribute: &'static str,
) -> Result<&'a str, CmmnConverterError> {
    node.attribute(attribute)
        .ok_or(CmmnConverterError::MissingAttribute {
            element: element.to_string(),
            attribute,
        })
}

fn normalized_attribute(node: Node<'_, '_>, attribute: &str) -> Option<String> {
    node.attributes()
        .find(|candidate| candidate.name() == attribute)
        .map(|candidate| candidate.value())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

/// Java attribute policy: converters only read attributes they know
/// (`BaseCmmnXmlConverter` / individual `*XmlConverter` classes); unknown attributes are
/// never validated or rejected. Rust previously hard-failed — align to skip + warn.
fn warn_unknown_attributes(node: Node<'_, '_>, allowed: &[&str]) {
    for attribute in node.attributes() {
        let name = attribute.name();
        // Namespace declarations are not CMMN model attributes.
        if name == "xmlns" || name.starts_with("xmlns:") {
            continue;
        }
        if !allowed.iter().any(|allowed_name| name == *allowed_name) {
            eprintln!(
                "[cmmn-converter] warning: ignoring unsupported attribute `{name}` on `{}` (Java converters only read known attributes; see BaseCmmnXmlConverter.java:39-65)",
                node.tag_name().name()
            );
        }
    }
}

/// Java element policy: `CmmnXmlConverter.convertToCmmnModel` (CmmnXmlConverter.java:222-226)
/// only dispatches elements present in `elementConverters`; unregistered local names are
/// silently skipped. Rust previously rejected — align to skip + warn.
fn skip_unknown_child(parent: &str, element: &str) {
    eprintln!(
        "[cmmn-converter] warning: skipping unsupported element `{element}` inside `{parent}` (Java CmmnXmlConverter.java:222-226 skips unregistered elements)"
    );
}

fn skip_all_unknown_children(parent: &str, node: Node<'_, '_>) {
    for child in element_children(node) {
        skip_unknown_child(parent, child.tag_name().name());
    }
}

/// Like [`skip_all_unknown_children`], but first harvests the lifecycle listeners out of an
/// `extensionElements` child. Java reaches these through
/// `ExtensionElementsXMLConverter.readLifecycleListener` (ExtensionElementsXMLConverter.java:
/// 121-124, :369-383), which appends to the current element when it implements
/// `HasLifecycleListeners` (HasLifecycleListeners.java:21-25).
///
/// `element_name` is the listener element Java accepts for this owner:
/// `caseLifecycleListener` on a `case` (CmmnXmlConstants.java:65) and
/// `planItemLifecycleListener` on any plan item definition (CmmnXmlConstants.java:64).
fn parse_lifecycle_listeners(
    parent: &str,
    node: Node<'_, '_>,
    element_name: &str,
) -> Vec<FlowableListener> {
    let mut listeners = Vec::new();
    for child in element_children(node) {
        if child.tag_name().name() == "extensionElements" {
            listeners.extend(parse_lifecycle_listeners_in_extension_elements(
                parent,
                child,
                element_name,
            ));
        } else {
            skip_unknown_child(parent, child.tag_name().name());
        }
    }
    listeners
}

/// The `extensionElements`-scoped half of [`parse_lifecycle_listeners`], for parse functions that
/// already own their child dispatch loop.
fn parse_lifecycle_listeners_in_extension_elements(
    parent: &str,
    extension_elements: Node<'_, '_>,
    element_name: &str,
) -> Vec<FlowableListener> {
    let mut listeners = Vec::new();
    for extension in element_children(extension_elements) {
        let tag = extension.tag_name().name();
        if tag == element_name {
            match parse_flowable_listener(extension, element_name) {
                Some(listener) => listeners.push(listener),
                // Java's ListenerXmlConverterUtil.convertToListener returns a listener with a
                // null implementationType when none of class / expression / delegateExpression /
                // type is present; the notification helper then creates no listener at all
                // (CmmnListenerNotificationHelper.java:88-100 leaves it null). Skipping with a
                // warning is the observable equivalent.
                None => eprintln!(
                    "[cmmn-converter] warning: skipping `{element_name}` inside `{parent}` with no `class`, `expression` or `delegateExpression` attribute (Java ListenerXmlConverterUtil.java:31-42 leaves the implementation type null and CmmnListenerNotificationHelper.java:88-100 then creates no listener)"
                ),
            }
        } else {
            skip_unknown_child("extensionElements", tag);
        }
    }
    listeners
}

/// Case-level `extensionElements` harvest: lifecycle listeners + event-registry start
/// subscription triple (eventType / startEventCorrelationConfiguration /
/// eventCorrelationParameter).
///
/// Java: ExtensionElementsXMLConverter.java:121-127, :396-411;
/// CmmnXmlConstants.java:224-230; CmmnCorrelationUtil.java:29-46.
struct ParsedCaseExtensions {
    lifecycle_listeners: Vec<FlowableListener>,
    start_event_type: Option<String>,
    start_correlation_configuration: Option<String>,
    start_correlation_parameters: Vec<EventCorrelationParameter>,
}

fn parse_case_extension_elements(extension_elements: Node<'_, '_>) -> ParsedCaseExtensions {
    let mut lifecycle_listeners = Vec::new();
    let mut start_event_type = None;
    let mut start_correlation_configuration = None;
    let mut start_correlation_parameters = Vec::new();

    for extension in element_children(extension_elements) {
        let tag = extension.tag_name().name();
        match tag {
            // CmmnXmlConstants.java:65 — caseLifecycleListener on Case.
            "caseLifecycleListener" => match parse_flowable_listener(extension, tag) {
                Some(listener) => lifecycle_listeners.push(listener),
                None => eprintln!(
                    "[cmmn-converter] warning: skipping `caseLifecycleListener` inside `case` with no `class`, `expression` or `delegateExpression` attribute (Java ListenerXmlConverterUtil.java:31-42)"
                ),
            },
            // CmmnXmlConstants.java:224 ELEMENT_EVENT_TYPE; when current element is Case,
            // Case.setStartEventType (ExtensionElementsXMLConverter.java:410-411).
            "eventType" => {
                let text = extension.text().unwrap_or("").trim();
                if !text.is_empty() {
                    start_event_type = Some(text.to_string());
                }
            }
            // CmmnXmlConstants.java:228 START_EVENT_CORRELATION_CONFIGURATION.
            "startEventCorrelationConfiguration" => {
                let text = extension.text().unwrap_or("").trim();
                if !text.is_empty() {
                    start_correlation_configuration = Some(text.to_string());
                }
            }
            // CmmnXmlConstants.java:225 ELEMENT_EVENT_CORRELATION_PARAMETER —
            // name/value attributes, static (CmmnCorrelationUtil.java:37-40).
            "eventCorrelationParameter" => {
                warn_unknown_attributes(extension, &["name", "value"]);
                if let Some(name) = normalized_attribute(extension, "name") {
                    let value = extension
                        .attribute("value")
                        .map(str::trim)
                        .unwrap_or("")
                        .to_string();
                    start_correlation_parameters.push(EventCorrelationParameter::new(name, value));
                } else {
                    eprintln!(
                        "[cmmn-converter] warning: skipping `eventCorrelationParameter` without `name` attribute"
                    );
                }
            }
            other => skip_unknown_child("extensionElements", other),
        }
    }

    ParsedCaseExtensions {
        lifecycle_listeners,
        start_event_type,
        start_correlation_configuration,
        start_correlation_parameters,
    }
}

/// Java `ListenerXmlConverterUtil.convertToListener` (ListenerXmlConverterUtil.java:28-53).
/// The implementation attributes are mutually exclusive and resolved in the order
/// class → expression → delegateExpression (:31-42).
fn parse_flowable_listener(node: Node<'_, '_>, element_name: &str) -> Option<FlowableListener> {
    warn_unknown_attributes(
        node,
        &[
            "class",
            "expression",
            "delegateExpression",
            "event",
            "sourceState",
            "targetState",
            "onTransaction",
        ],
    );
    // Java parses child `field` elements only for script listeners
    // (ListenerXmlConverterUtil.java:49-51); anything else here is unregistered.
    skip_all_unknown_children(element_name, node);

    let (implementation_type, implementation) =
        if let Some(class) = normalized_attribute(node, "class") {
            (ListenerImplementationType::Class, class)
        } else if let Some(expression) = normalized_attribute(node, "expression") {
            (ListenerImplementationType::Expression, expression)
        } else if let Some(delegate) = normalized_attribute(node, "delegateExpression") {
            (ListenerImplementationType::DelegateExpression, delegate)
        } else {
            return None;
        };

    Some(FlowableListener {
        implementation_type,
        implementation,
        source_state: normalized_attribute(node, "sourceState"),
        target_state: normalized_attribute(node, "targetState"),
        event: normalized_attribute(node, "event"),
    })
}

/// Splits a comma-delimited attribute value into trimmed, non-empty entries.
/// Java parity: `CmmnXmlUtil.parseDelimitedList` used for candidateUsers and
/// candidateGroups (HumanTaskXmlConverter.java:53-61).
fn parse_delimited_list(value: Option<String>) -> Vec<String> {
    value
        .into_iter()
        .flat_map(|raw| {
            raw.split(',')
                .map(str::trim)
                .filter(|entry| !entry.is_empty())
                .map(ToOwned::to_owned)
                .collect::<Vec<_>>()
        })
        .collect()
}

fn expect_element_name(
    node: Node<'_, '_>,
    expected: &'static str,
) -> Result<(), CmmnConverterError> {
    if node.tag_name().name() == expected {
        Ok(())
    } else {
        Err(CmmnConverterError::Structural(format!(
            "expected root element `{expected}`, found `{}`",
            node.tag_name().name()
        )))
    }
}

fn element_children<'a>(node: Node<'a, 'a>) -> impl Iterator<Item = Node<'a, 'a>> {
    node.children().filter(|child| child.is_element())
}

fn collect_namespaces(node: Node<'_, '_>) -> BTreeMap<String, String> {
    let mut namespaces = BTreeMap::new();
    for namespace in node.namespaces() {
        namespaces.insert(
            namespace.name().unwrap_or_default().to_string(),
            namespace.uri().to_string(),
        );
    }
    namespaces
}

struct ParsedContainer {
    id: String,
    name: Option<String>,
    auto_complete: bool,
    form_key: Option<String>,
    plan_items: Vec<PlanItem>,
    human_tasks: Vec<HumanTask>,
    decision_tasks: Vec<DecisionTask>,
    process_tasks: Vec<ProcessTask>,
    case_tasks: Vec<CaseTask>,
    milestones: Vec<Milestone>,
    event_listeners: Vec<EventListener>,
    sentries: Vec<Sentry>,
    planning_tables: Vec<PlanningTable>,
    stages: Vec<Stage>,
    /// `flowable:planItemLifecycleListener` entries: a `casePlanModel` / `stage` is a `Stage`,
    /// which extends `PlanItemDefinition implements HasLifecycleListeners`
    /// (PlanItemDefinition.java:21).
    lifecycle_listeners: Vec<FlowableListener>,
}

struct ParsedPlanItemControl {
    manual_activation_rule: Option<SentryIfPartExpression>,
    repetition_rule: Option<SentryIfPartExpression>,
    required_rule: Option<SentryIfPartExpression>,
    parent_completion_rule: Option<String>,
    completion_neutral_rule: Option<SentryIfPartExpression>,
}
