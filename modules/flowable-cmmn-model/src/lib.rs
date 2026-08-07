use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CmmnDefinitions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_namespace: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expression_language: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub type_language: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exporter: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exporter_version: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub namespaces: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cases: Vec<Case>,
}

impl CmmnDefinitions {
    pub fn find_case(&self, id: &str) -> Option<&Case> {
        self.cases
            .iter()
            .find(|case_definition| case_definition.id == id)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Case {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub case_plan_model: CasePlanModel,
    /// Java `Case implements HasLifecycleListeners` (HasLifecycleListeners.java:21-25) —
    /// `flowable:caseLifecycleListener` entries parsed out of the case's
    /// `extensionElements` (ExtensionElementsXMLConverter.java:121-124).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lifecycle_listeners: Vec<FlowableListener>,
    /// Java `Case.startEventType` — `flowable:eventType` text when the extension sits on the
    /// case element (ExtensionElementsXMLConverter.java:396-411, CmmnXmlConstants.java:224).
    /// Non-empty means this case definition has an event-registry start subscription candidate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_event_type: Option<String>,
    /// Java `startEventCorrelationConfiguration` extension text
    /// (CmmnXmlConstants.java:228-230): `storeAsUniqueReferenceId` or `manualSubscription`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_correlation_configuration: Option<String>,
    /// Java case-level `eventCorrelationParameter` extensions (name/value attributes).
    /// Static values — no expression evaluation
    /// (CmmnCorrelationUtil.java:29-46, CmmnXmlConstants.java:225).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub start_correlation_parameters: Vec<EventCorrelationParameter>,
}

/// Java `eventCorrelationParameter` extension (CmmnXmlConstants.java:225) —
/// `name` / `value` attributes; values are taken as-is for case start events
/// (CmmnCorrelationUtil.java:35-40).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EventCorrelationParameter {
    pub name: String,
    pub value: String,
}

impl EventCorrelationParameter {
    pub fn new(name: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            value: value.into(),
        }
    }
}

/// Java `FlowableListener` (FlowableListener.java:20-93) as used by the CMMN lifecycle
/// listener elements. `implementation_type` records which of the mutually exclusive
/// `class` / `expression` / `delegateExpression` attributes was present; Java resolves
/// them in exactly that precedence order
/// (ListenerXmlConverterUtil.java:31-42).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FlowableListener {
    pub implementation_type: ListenerImplementationType,
    pub implementation: String,
    /// Java `sourceState` / `targetState`; absent means "match any state"
    /// (CmmnListenerNotificationHelper.java:158-160 `StringUtils.isEmpty` check).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_state: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_state: Option<String>,
    /// Java `event` attribute (ListenerXmlConverterUtil.java:44). Parsed and carried for
    /// fidelity; the CMMN lifecycle listener path filters on source/target state only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ListenerImplementationType {
    /// Java `ImplementationType.IMPLEMENTATION_TYPE_CLASS`.
    Class,
    /// Java `ImplementationType.IMPLEMENTATION_TYPE_EXPRESSION`.
    Expression,
    /// Java `ImplementationType.IMPLEMENTATION_TYPE_DELEGATEEXPRESSION`.
    DelegateExpression,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CaseFileModel {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub item_definitions: Vec<CaseFileItemDefinition>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<CaseFileItem>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CaseFileItemDefinition {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub definition_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub structure_ref: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CaseFileItem {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub definition_ref: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<CaseFileItem>,
}

impl Case {
    pub fn find_plan_item(&self, id: &str) -> Option<&PlanItem> {
        self.case_plan_model.find_plan_item(id)
    }

    pub fn find_plan_item_definition(&self, id: &str) -> Option<PlanItemDefinitionRef<'_>> {
        self.case_plan_model.find_plan_item_definition(id)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CasePlanModel {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub auto_complete: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub form_key: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub plan_items: Vec<PlanItem>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub human_tasks: Vec<HumanTask>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub decision_tasks: Vec<DecisionTask>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub process_tasks: Vec<ProcessTask>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub case_tasks: Vec<CaseTask>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub milestones: Vec<Milestone>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub event_listeners: Vec<EventListener>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sentries: Vec<Sentry>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub planning_tables: Vec<PlanningTable>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stages: Vec<Stage>,
    /// The case plan model is itself a `Stage` in Java (`Case.getPlanModel()` returns a
    /// `Stage`, which extends `PlanItemDefinition implements HasLifecycleListeners`,
    /// PlanItemDefinition.java:21), so `flowable:planItemLifecycleListener` is valid here.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lifecycle_listeners: Vec<FlowableListener>,
}

impl CasePlanModel {
    pub fn find_plan_item(&self, id: &str) -> Option<&PlanItem> {
        find_plan_item_in_container(&self.plan_items, &self.stages, id)
    }

    pub fn find_plan_item_definition(&self, id: &str) -> Option<PlanItemDefinitionRef<'_>> {
        find_definition_in_container(
            &self.human_tasks,
            &self.decision_tasks,
            &self.process_tasks,
            &self.case_tasks,
            &self.milestones,
            &self.event_listeners,
            &self.stages,
            id,
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Stage {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub auto_complete: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub plan_items: Vec<PlanItem>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub human_tasks: Vec<HumanTask>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub decision_tasks: Vec<DecisionTask>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub process_tasks: Vec<ProcessTask>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub case_tasks: Vec<CaseTask>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub milestones: Vec<Milestone>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub event_listeners: Vec<EventListener>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sentries: Vec<Sentry>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub planning_tables: Vec<PlanningTable>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stages: Vec<Stage>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lifecycle_listeners: Vec<FlowableListener>,
}

impl Stage {
    pub fn find_plan_item(&self, id: &str) -> Option<&PlanItem> {
        find_plan_item_in_container(&self.plan_items, &self.stages, id)
    }

    pub fn find_plan_item_definition(&self, id: &str) -> Option<PlanItemDefinitionRef<'_>> {
        find_definition_in_container(
            &self.human_tasks,
            &self.decision_tasks,
            &self.process_tasks,
            &self.case_tasks,
            &self.milestones,
            &self.event_listeners,
            &self.stages,
            id,
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PlanningTable {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub discretionary_items: Vec<DiscretionaryItem>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DiscretionaryItem {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub definition_ref: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HumanTask {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub is_blocking: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub form_key: Option<String>,
    // Java parity: HumanTask.java:23-34 — flowable extension attributes carried
    // on the human task definition and applied to the created task entity
    // (HumanTaskActivityBehavior.java:107-110).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assignee: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub due_date: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub candidate_users: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub candidate_groups: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id_variable_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_completer_variable_name: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lifecycle_listeners: Vec<FlowableListener>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DecisionTask {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decision_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lifecycle_listeners: Vec<FlowableListener>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProcessTask {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub is_blocking: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub process_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lifecycle_listeners: Vec<FlowableListener>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CaseTask {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub is_blocking: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub case_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lifecycle_listeners: Vec<FlowableListener>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Milestone {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lifecycle_listeners: Vec<FlowableListener>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EventListener {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub event_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event_name: Option<String>,
    // Java parity: EventListener.java:20-29 availableConditionExpression - gates whether
    // the listener becomes available (AbstractEvaluationCriteriaOperation.java:584-604).
    // Parsed from the flowable:availableCondition attribute on eventListener /
    // timerEventListener elements (TimerEventListenerXmlConverter.java:36-44).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub available_condition: Option<String>,
    // Java parity: TimerEventListener.java:18-30 timerExpression - the ISO-8601
    // duration / date / repetition expression (TimerExpressionXmlConverter.java:39-49).
    // `Some` marks this listener as a timerEventListener (event_type = "timer").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timer_expression: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lifecycle_listeners: Vec<FlowableListener>,
}

impl EventListener {
    /// Marker event_type for timer event listeners (Java `TimerEventListener` has no
    /// eventType attribute; TimerEventListenerXmlConverter.java:36-44 only reads name).
    pub const EVENT_TYPE_TIMER: &'static str = "timer";

    /// Java parity: `TimerEventListener extends EventListener` — a listener is a timer
    /// event listener when it carries a timerExpression (TimerEventListener.java:20).
    pub fn is_timer(&self) -> bool {
        self.timer_expression.is_some()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PlanItem {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub definition_ref: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub entry_criteria: Vec<EntryCriterion>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exit_criteria: Vec<EntryCriterion>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manual_activation_rule: Option<SentryIfPartExpression>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repetition_rule: Option<SentryIfPartExpression>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required_rule: Option<SentryIfPartExpression>,
    // Java: ParentCompletionRule.getType() (default|ignore|ignoreIfAvailable|
    // ignoreIfAvailableOrEnabled|ignoreAfterFirstCompletion|
    // ignoreAfterFirstCompletionIfAvailableOrEnabled) - controls whether this plan item
    // blocks parent (stage/case) completion; see PlanItemInstanceContainerUtil.java:86-146.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_completion_rule: Option<String>,
    // Java: completionNeutralRule condition - when satisfied the plan item does not prevent
    // parent completion while AVAILABLE (ExpressionUtil.isCompletionNeutralPlanItemInstance).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completion_neutral_rule: Option<SentryIfPartExpression>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EntryCriterion {
    pub id: String,
    pub sentry_ref: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Sentry {
    pub id: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub plan_item_on_parts: Vec<PlanItemOnPart>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub case_file_item_on_parts: Vec<CaseFileItemOnPart>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub if_part: Option<SentryIfPartExpression>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum SentryIfPartExpression {
    Comparison(SentryIfPartCondition),
    Logical {
        operator: SentryIfPartLogicalOperator,
        operands: Vec<SentryIfPartExpression>,
    },
    Not {
        operand: Box<SentryIfPartExpression>,
    },
    Empty {
        variable_name: String,
    },
    Contains {
        collection_variable_name: String,
        value: SentryIfPartLiteral,
        expected: bool,
    },
    StartsWith {
        variable_name: String,
        prefix: String,
    },
    EndsWith {
        variable_name: String,
        suffix: String,
    },
    Matches {
        variable_name: String,
        regex: String,
    },
    MethodCall {
        object: Option<String>,
        method: String,
        args: Vec<SentryIfPartExpression>,
    },
    Arithmetic {
        left: Box<SentryIfPartExpression>,
        operator: String,
        right: Box<SentryIfPartExpression>,
    },
    Ternary {
        condition: Box<SentryIfPartExpression>,
        true_expr: Box<SentryIfPartExpression>,
        false_expr: Box<SentryIfPartExpression>,
    },
    PropertyAccess {
        object: Box<SentryIfPartExpression>,
        property: String,
    },
    IndexAccess {
        object: Box<SentryIfPartExpression>,
        index: Box<SentryIfPartExpression>,
    },
    Literal(SentryIfPartLiteral),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SentryIfPartCondition {
    pub variable_name: String,
    pub operator: SentryIfPartOperator,
    pub literal: SentryIfPartLiteral,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SentryIfPartOperator {
    Equal,
    NotEqual,
    GreaterThan,
    GreaterThanOrEqual,
    LessThan,
    LessThanOrEqual,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SentryIfPartLogicalOperator {
    And,
    Or,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "type", content = "value")]
pub enum SentryIfPartLiteral {
    Boolean(bool),
    String(String),
    Number(String),
    Null,
    Variable(String),
}

pub fn parse_sentry_if_part_expression(expression: &str) -> Result<SentryIfPartExpression, String> {
    let expression = expression.trim();
    let expression = expression
        .strip_prefix("${")
        .and_then(|value| value.strip_suffix('}'))
        .unwrap_or(expression)
        .trim();

    parse_supported_if_part_expression(expression)
        .map_err(|message| format!("Unsupported CMMN ifPart condition '{expression}': {message}"))
}

pub fn parse_sentry_value_expression(expression: &str) -> Result<SentryIfPartExpression, String> {
    let expression = expression.trim();
    let expression = expression
        .strip_prefix("${")
        .and_then(|value| value.strip_suffix('}'))
        .unwrap_or(expression)
        .trim();

    let tokens = tokenize_if_part_expression(expression)?;
    let mut parser = IfPartParser::new(tokens);
    let parsed = parser.parse_expression()?;
    if !parser.is_at_end() {
        return Err("unexpected trailing tokens in expression".to_string());
    }
    Ok(parsed)
}

fn parse_supported_if_part_expression(expression: &str) -> Result<SentryIfPartExpression, String> {
    let tokens = tokenize_if_part_expression(expression)?;
    let mut parser = IfPartParser::new(tokens);
    let parsed = ensure_boolean_context(parser.parse_expression()?);
    if !parser.is_at_end() {
        return Err("unexpected trailing tokens in ifPart expression".to_string());
    }
    Ok(parsed)
}

fn ensure_boolean_context(expr: SentryIfPartExpression) -> SentryIfPartExpression {
    if let SentryIfPartExpression::Literal(SentryIfPartLiteral::Variable(var_name)) = &expr
        && is_supported_if_part_variable_name(var_name)
    {
        return boolean_variable_path_expression(var_name.clone());
    }
    expr
}

fn boolean_variable_path_expression(variable_name: String) -> SentryIfPartExpression {
    SentryIfPartExpression::Comparison(SentryIfPartCondition {
        variable_name,
        operator: SentryIfPartOperator::Equal,
        literal: SentryIfPartLiteral::Boolean(true),
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum IfPartToken {
    Identifier(String),
    Literal(SentryIfPartLiteral),
    Equal,
    NotEqual,
    GreaterThan,
    GreaterThanOrEqual,
    LessThan,
    LessThanOrEqual,
    And,
    Or,
    Not,
    LeftParen,
    RightParen,
    Comma,
    Plus,
    Minus,
    Multiply,
    Divide,
    Modulo,
    Question,
    Colon,
    LeftBracket,
    RightBracket,
    Dot,
}

/// Maximum recursive nesting depth for CMMN ifPart / value expression parsing.
/// P142c: deployer-controlled expressions must not stack-overflow the parser.
/// Each nested `(...)` / ternary re-enters the full precedence chain, so 128
/// overflows Windows debug stacks; 64 is the practical abuse ceiling.
const MAX_IF_PART_NESTING_DEPTH: usize = 64;

struct IfPartParser {
    tokens: Vec<IfPartToken>,
    index: usize,
    /// Current recursive nesting depth of `parse_expression`.
    depth: usize,
}

impl IfPartParser {
    fn new(tokens: Vec<IfPartToken>) -> Self {
        Self {
            tokens,
            index: 0,
            depth: 0,
        }
    }

    fn parse_expression(&mut self) -> Result<SentryIfPartExpression, String> {
        if self.depth >= MAX_IF_PART_NESTING_DEPTH {
            return Err(format!(
                "expression nesting exceeds maximum depth of {MAX_IF_PART_NESTING_DEPTH}"
            ));
        }
        self.depth += 1;
        let result = self.parse_ternary();
        self.depth -= 1;
        result
    }

    fn parse_ternary(&mut self) -> Result<SentryIfPartExpression, String> {
        let condition = self.parse_or()?;
        if self.consume(&IfPartToken::Question) {
            let true_expr = self.parse_expression()?;
            if !self.consume(&IfPartToken::Colon) {
                return Err("ternary expression missing `:`".to_string());
            }
            let false_expr = self.parse_ternary()?;
            Ok(SentryIfPartExpression::Ternary {
                condition: Box::new(ensure_boolean_context(condition)),
                true_expr: Box::new(true_expr),
                false_expr: Box::new(false_expr),
            })
        } else {
            Ok(condition)
        }
    }

    fn parse_or(&mut self) -> Result<SentryIfPartExpression, String> {
        let first = self.parse_and()?;
        if self.peek() == Some(&IfPartToken::Or) {
            let mut operands = vec![ensure_boolean_context(first)];
            while self.consume(&IfPartToken::Or) {
                operands.push(ensure_boolean_context(self.parse_and()?));
            }
            Ok(logical_expression(
                SentryIfPartLogicalOperator::Or,
                operands,
            ))
        } else {
            Ok(first)
        }
    }

    fn parse_and(&mut self) -> Result<SentryIfPartExpression, String> {
        let first = self.parse_comparison()?;
        if self.peek() == Some(&IfPartToken::And) {
            let mut operands = vec![ensure_boolean_context(first)];
            while self.consume(&IfPartToken::And) {
                operands.push(ensure_boolean_context(self.parse_comparison()?));
            }
            Ok(logical_expression(
                SentryIfPartLogicalOperator::And,
                operands,
            ))
        } else {
            Ok(first)
        }
    }

    fn peek_comparison_operator(&self) -> Option<SentryIfPartOperator> {
        match self.peek() {
            Some(IfPartToken::Equal) => Some(SentryIfPartOperator::Equal),
            Some(IfPartToken::NotEqual) => Some(SentryIfPartOperator::NotEqual),
            Some(IfPartToken::GreaterThan) => Some(SentryIfPartOperator::GreaterThan),
            Some(IfPartToken::GreaterThanOrEqual) => Some(SentryIfPartOperator::GreaterThanOrEqual),
            Some(IfPartToken::LessThan) => Some(SentryIfPartOperator::LessThan),
            Some(IfPartToken::LessThanOrEqual) => Some(SentryIfPartOperator::LessThanOrEqual),
            _ => None,
        }
    }

    fn parse_comparison(&mut self) -> Result<SentryIfPartExpression, String> {
        let left = self.parse_additive()?;
        if let Some(op) = self.peek_comparison_operator() {
            self.advance();
            let right = self.parse_additive()?;
            let var_name = expression_to_string(&left)?;
            let lit = expression_to_literal(&right)?;

            if matches!(lit, SentryIfPartLiteral::Null)
                && !matches!(
                    op,
                    SentryIfPartOperator::Equal | SentryIfPartOperator::NotEqual
                )
            {
                return Err("null literal supports only `==` and `!=` comparisons".to_string());
            }

            Ok(SentryIfPartExpression::Comparison(SentryIfPartCondition {
                variable_name: var_name,
                operator: op,
                literal: lit,
            }))
        } else {
            Ok(left)
        }
    }

    fn parse_additive(&mut self) -> Result<SentryIfPartExpression, String> {
        let mut left = self.parse_multiplicative()?;
        while let Some(token) = self.peek() {
            match token {
                IfPartToken::Plus => {
                    self.advance();
                    let right = self.parse_multiplicative()?;
                    left = SentryIfPartExpression::Arithmetic {
                        left: Box::new(left),
                        operator: "+".to_string(),
                        right: Box::new(right),
                    };
                }
                IfPartToken::Minus => {
                    self.advance();
                    let right = self.parse_multiplicative()?;
                    left = SentryIfPartExpression::Arithmetic {
                        left: Box::new(left),
                        operator: "-".to_string(),
                        right: Box::new(right),
                    };
                }
                _ => break,
            }
        }
        Ok(left)
    }

    fn parse_multiplicative(&mut self) -> Result<SentryIfPartExpression, String> {
        let mut left = self.parse_unary()?;
        while let Some(token) = self.peek() {
            match token {
                IfPartToken::Multiply => {
                    self.advance();
                    let right = self.parse_unary()?;
                    left = SentryIfPartExpression::Arithmetic {
                        left: Box::new(left),
                        operator: "*".to_string(),
                        right: Box::new(right),
                    };
                }
                IfPartToken::Divide => {
                    self.advance();
                    let right = self.parse_unary()?;
                    left = SentryIfPartExpression::Arithmetic {
                        left: Box::new(left),
                        operator: "/".to_string(),
                        right: Box::new(right),
                    };
                }
                IfPartToken::Modulo => {
                    self.advance();
                    let right = self.parse_unary()?;
                    left = SentryIfPartExpression::Arithmetic {
                        left: Box::new(left),
                        operator: "%".to_string(),
                        right: Box::new(right),
                    };
                }
                _ => break,
            }
        }
        Ok(left)
    }

    fn parse_unary(&mut self) -> Result<SentryIfPartExpression, String> {
        if self.consume(&IfPartToken::Not) {
            return Ok(SentryIfPartExpression::Not {
                operand: Box::new(self.parse_not_operand()?),
            });
        }
        self.parse_primary()
    }

    fn parse_not_operand(&mut self) -> Result<SentryIfPartExpression, String> {
        if self.next_tokens_are_parenthesized_variable_path() {
            return self.parse_parenthesized_boolean_variable_path();
        }
        let operand = ensure_boolean_context(self.parse_unary()?);
        Ok(operand)
    }

    fn parse_parenthesized_boolean_variable_path(
        &mut self,
    ) -> Result<SentryIfPartExpression, String> {
        self.advance();
        let variable_name = match self.advance() {
            Some(IfPartToken::Identifier(value)) if is_supported_if_part_variable_name(&value) => {
                value
            }
            Some(IfPartToken::Identifier(_)) => {
                return Err("not function argument must be a case variable path".to_string());
            }
            _ => return Err("not function requires a case variable path argument".to_string()),
        };
        if !self.consume(&IfPartToken::RightParen) {
            return Err("not function boolean path wrapper supports one argument".to_string());
        }

        Ok(SentryIfPartExpression::Comparison(SentryIfPartCondition {
            variable_name,
            operator: SentryIfPartOperator::Equal,
            literal: SentryIfPartLiteral::Boolean(true),
        }))
    }

    fn parse_primary(&mut self) -> Result<SentryIfPartExpression, String> {
        let mut expr = self.parse_primary_base()?;
        loop {
            if self.consume(&IfPartToken::LeftBracket) {
                let index = self.parse_expression()?;
                if !self.consume(&IfPartToken::RightBracket) {
                    return Err("expected `]` after index expression".to_string());
                }
                expr = SentryIfPartExpression::IndexAccess {
                    object: Box::new(expr),
                    index: Box::new(index),
                };
            } else if self.consume(&IfPartToken::Dot) {
                let property = match self.advance() {
                    Some(IfPartToken::Identifier(prop)) => prop,
                    _ => return Err("expected identifier after `.`".to_string()),
                };
                if self.peek() == Some(&IfPartToken::LeftParen) {
                    let object_str = expression_to_string(&expr)?;
                    self.advance(); // consume `(`
                    let mut args = Vec::new();
                    if self.peek() != Some(&IfPartToken::RightParen) {
                        args.push(self.parse_expression_argument_until(&[
                            IfPartToken::Comma,
                            IfPartToken::RightParen,
                        ])?);
                        while self.consume(&IfPartToken::Comma) {
                            args.push(self.parse_expression_argument_until(&[
                                IfPartToken::Comma,
                                IfPartToken::RightParen,
                            ])?);
                        }
                    }
                    if !self.consume(&IfPartToken::RightParen) {
                        return Err("expected `)` after method arguments".to_string());
                    }
                    expr = SentryIfPartExpression::MethodCall {
                        object: Some(object_str),
                        method: property,
                        args,
                    };
                } else {
                    expr = SentryIfPartExpression::PropertyAccess {
                        object: Box::new(expr),
                        property,
                    };
                }
            } else {
                break;
            }
        }
        Ok(expr)
    }

    fn parse_primary_base(&mut self) -> Result<SentryIfPartExpression, String> {
        if self.consume(&IfPartToken::LeftParen) {
            let expression = self.parse_expression()?;
            if !self.consume(&IfPartToken::RightParen) {
                return Err(
                    "parentheses must be balanced in supported ifPart expressions".to_string(),
                );
            }
            return Ok(expression);
        }
        if self.next_identifier_is_empty_function() {
            return self.parse_empty_function();
        }
        if self.next_identifier_is_contains_function() {
            return self.parse_contains_function_comparison();
        }
        if self.next_identifier_is_starts_with_function() {
            return self.parse_starts_with_function();
        }
        if self.next_identifier_is_ends_with_function() {
            return self.parse_ends_with_function();
        }
        if self.next_identifier_is_matches_function() {
            return self.parse_matches_function();
        }

        if self.next_identifier_is_sizing_function() {
            let func_name = match self.peek() {
                Some(IfPartToken::Identifier(name)) => name.clone(),
                _ => return Err("expected identifier for sizing function".to_string()),
            };
            let operand = self.parse_sizing_function_operand()?;
            return Ok(SentryIfPartExpression::MethodCall {
                object: Some(operand),
                method: func_name,
                args: vec![],
            });
        }

        if let Some(IfPartToken::Identifier(name)) = self.peek()
            && self.peek_next() == Some(&IfPartToken::LeftParen)
        {
            let name = name.clone();
            self.advance(); // consume identifier
            self.advance(); // consume `(`
            let mut args = Vec::new();
            if self.peek() != Some(&IfPartToken::RightParen) {
                args.push(self.parse_expression_argument_until(&[
                    IfPartToken::Comma,
                    IfPartToken::RightParen,
                ])?);
                while self.consume(&IfPartToken::Comma) {
                    args.push(self.parse_expression_argument_until(&[
                        IfPartToken::Comma,
                        IfPartToken::RightParen,
                    ])?);
                }
            }
            if !self.consume(&IfPartToken::RightParen) {
                return Err("expected `)` after method arguments".to_string());
            }
            let (object, method) = split_method_call_identifier(&name);
            return Ok(SentryIfPartExpression::MethodCall {
                object,
                method,
                args,
            });
        }

        match self.advance() {
            Some(IfPartToken::Literal(lit)) => Ok(SentryIfPartExpression::Literal(lit)),
            Some(IfPartToken::Identifier(name)) => Ok(SentryIfPartExpression::Literal(
                SentryIfPartLiteral::Variable(name),
            )),
            _ => Err("expected literal or identifier".to_string()),
        }
    }

    fn parse_empty_function(&mut self) -> Result<SentryIfPartExpression, String> {
        self.advance();
        if !self.consume(&IfPartToken::LeftParen) {
            return Err("empty function requires `(`".to_string());
        }

        let variable_name = match self.advance() {
            Some(IfPartToken::Identifier(value)) if is_supported_if_part_variable_name(&value) => {
                value
            }
            Some(IfPartToken::Identifier(_)) => {
                return Err("empty function argument must be a case variable path".to_string());
            }
            _ => {
                return Err("empty function requires a case variable path argument".to_string());
            }
        };

        if !self.consume(&IfPartToken::RightParen) {
            return Err("empty function supports exactly one variable argument".to_string());
        }

        Ok(SentryIfPartExpression::Empty { variable_name })
    }

    fn parse_contains_function_comparison(&mut self) -> Result<SentryIfPartExpression, String> {
        self.advance();
        if !self.consume(&IfPartToken::LeftParen) {
            return Err("contains function requires `(`".to_string());
        }

        let collection =
            self.parse_expression_argument_until(&[IfPartToken::Comma, IfPartToken::RightParen])?;
        let collection_variable_name = expression_to_string(&collection)?;

        if !self.consume(&IfPartToken::Comma) {
            return Err("contains function requires exactly two arguments".to_string());
        }

        let value = expression_to_literal(
            &self.parse_expression_argument_until(&[IfPartToken::RightParen])?,
        )?;

        if !self.consume(&IfPartToken::RightParen) {
            return Err("contains function supports exactly two arguments".to_string());
        }

        let expected = if self.consume(&IfPartToken::Equal) {
            match self.advance() {
                Some(IfPartToken::Literal(SentryIfPartLiteral::Boolean(value))) => value,
                _ => {
                    return Err(
                        "contains function comparison right side must be true or false".to_string(),
                    );
                }
            }
        } else if self.consume(&IfPartToken::NotEqual) {
            match self.advance() {
                Some(IfPartToken::Literal(SentryIfPartLiteral::Boolean(value))) => !value,
                _ => {
                    return Err(
                        "contains function comparison right side must be true or false".to_string(),
                    );
                }
            }
        } else {
            true
        };

        Ok(SentryIfPartExpression::Contains {
            collection_variable_name,
            value,
            expected,
        })
    }

    fn next_identifier_is_starts_with_function(&self) -> bool {
        matches!(
            (self.peek(), self.peek_next()),
            (Some(IfPartToken::Identifier(name)), Some(IfPartToken::LeftParen))
                if name == "startsWith"
        )
    }

    fn next_identifier_is_ends_with_function(&self) -> bool {
        matches!(
            (self.peek(), self.peek_next()),
            (Some(IfPartToken::Identifier(name)), Some(IfPartToken::LeftParen))
                if name == "endsWith"
        )
    }

    fn next_identifier_is_matches_function(&self) -> bool {
        matches!(
            (self.peek(), self.peek_next()),
            (Some(IfPartToken::Identifier(name)), Some(IfPartToken::LeftParen))
                if name == "matches"
        )
    }

    fn parse_starts_with_function(&mut self) -> Result<SentryIfPartExpression, String> {
        self.advance();
        if !self.consume(&IfPartToken::LeftParen) {
            return Err("startsWith function requires `(`".to_string());
        }

        let variable_name = match self.advance() {
            Some(IfPartToken::Identifier(value)) if is_supported_if_part_variable_name(&value) => {
                value
            }
            _ => {
                return Err("startsWith first argument must be a case variable path".to_string());
            }
        };

        if !self.consume(&IfPartToken::Comma) {
            return Err("startsWith function requires exactly two arguments".to_string());
        }

        let prefix = match self.advance() {
            Some(IfPartToken::Literal(SentryIfPartLiteral::String(value))) => value,
            _ => {
                return Err("startsWith second argument must be a string literal".to_string());
            }
        };

        if !self.consume(&IfPartToken::RightParen) {
            return Err("startsWith function supports exactly two arguments".to_string());
        }

        Ok(SentryIfPartExpression::StartsWith {
            variable_name,
            prefix,
        })
    }

    fn parse_ends_with_function(&mut self) -> Result<SentryIfPartExpression, String> {
        self.advance();
        if !self.consume(&IfPartToken::LeftParen) {
            return Err("endsWith function requires `(`".to_string());
        }

        let variable_name = match self.advance() {
            Some(IfPartToken::Identifier(value)) if is_supported_if_part_variable_name(&value) => {
                value
            }
            _ => {
                return Err("endsWith first argument must be a case variable path".to_string());
            }
        };

        if !self.consume(&IfPartToken::Comma) {
            return Err("endsWith function requires exactly two arguments".to_string());
        }

        let suffix = match self.advance() {
            Some(IfPartToken::Literal(SentryIfPartLiteral::String(value))) => value,
            _ => {
                return Err("endsWith second argument must be a string literal".to_string());
            }
        };

        if !self.consume(&IfPartToken::RightParen) {
            return Err("endsWith function supports exactly two arguments".to_string());
        }

        Ok(SentryIfPartExpression::EndsWith {
            variable_name,
            suffix,
        })
    }

    fn parse_matches_function(&mut self) -> Result<SentryIfPartExpression, String> {
        self.advance();
        if !self.consume(&IfPartToken::LeftParen) {
            return Err("matches function requires `(`".to_string());
        }

        let variable_name = match self.advance() {
            Some(IfPartToken::Identifier(value)) if is_supported_if_part_variable_name(&value) => {
                value
            }
            _ => {
                return Err("matches first argument must be a case variable path".to_string());
            }
        };

        if !self.consume(&IfPartToken::Comma) {
            return Err("matches function requires exactly two arguments".to_string());
        }

        let regex = match self.advance() {
            Some(IfPartToken::Literal(SentryIfPartLiteral::String(value))) => value,
            _ => {
                return Err("matches second argument must be a string literal".to_string());
            }
        };

        if !self.consume(&IfPartToken::RightParen) {
            return Err("matches function supports exactly two arguments".to_string());
        }

        Ok(SentryIfPartExpression::Matches {
            variable_name,
            regex,
        })
    }

    fn parse_sizing_function_operand(&mut self) -> Result<String, String> {
        let function_name = match self.advance() {
            Some(IfPartToken::Identifier(function_name)) => function_name,
            _ => return Err("supported sizing function requires a function name".to_string()),
        };
        if !self.consume(&IfPartToken::LeftParen) {
            return Err(format!("{function_name} function requires `(`"));
        }

        let operand = self.parse_expression_argument_until(&[IfPartToken::RightParen])?;
        let variable_name = expression_to_string(&operand)?;

        if !self.consume(&IfPartToken::RightParen) {
            return Err(format!(
                "{function_name} function supports exactly one variable argument"
            ));
        }

        Ok(variable_name.to_string())
    }

    fn consume(&mut self, expected: &IfPartToken) -> bool {
        if self.peek() == Some(expected) {
            self.index += 1;
            true
        } else {
            false
        }
    }

    fn advance(&mut self) -> Option<IfPartToken> {
        let token = self.tokens.get(self.index).cloned();
        if token.is_some() {
            self.index += 1;
        }
        token
    }

    fn peek(&self) -> Option<&IfPartToken> {
        self.tokens.get(self.index)
    }

    fn peek_next(&self) -> Option<&IfPartToken> {
        self.tokens.get(self.index + 1)
    }

    fn parse_expression_argument_until(
        &mut self,
        terminators: &[IfPartToken],
    ) -> Result<SentryIfPartExpression, String> {
        let start = self.index;
        let mut index = self.index;
        let mut depth = 0usize;

        while index < self.tokens.len() {
            let token = &self.tokens[index];
            if depth == 0 && terminators.iter().any(|terminator| terminator == token) {
                break;
            }

            match token {
                IfPartToken::LeftParen | IfPartToken::LeftBracket => depth += 1,
                IfPartToken::RightParen | IfPartToken::RightBracket => {
                    if depth == 0 {
                        break;
                    }
                    depth -= 1;
                }
                _ => {}
            }
            index += 1;
        }

        if index == start {
            return Err("function argument must not be empty".to_string());
        }

        // Inherit parent depth so nested function arguments cannot reset the
        // stack-depth budget (P142c resource limit).
        let mut argument_parser = IfPartParser {
            tokens: self.tokens[start..index].to_vec(),
            index: 0,
            depth: self.depth,
        };
        let expression = argument_parser.parse_expression()?;
        if !argument_parser.is_at_end() {
            return Err("unexpected trailing tokens in function argument".to_string());
        }
        self.index = index;
        Ok(expression)
    }

    fn next_tokens_are_parenthesized_variable_path(&self) -> bool {
        matches!(
            (self.peek(), self.peek_next(), self.tokens.get(self.index + 2)),
            (
                Some(IfPartToken::LeftParen),
                Some(IfPartToken::Identifier(variable_name)),
                Some(IfPartToken::RightParen)
            ) if is_supported_if_part_variable_name(variable_name)
        )
    }

    fn next_identifier_is_empty_function(&self) -> bool {
        matches!(
            (self.peek(), self.peek_next()),
            (Some(IfPartToken::Identifier(name)), Some(IfPartToken::LeftParen))
                if name == "empty"
        )
    }

    fn next_identifier_is_sizing_function(&self) -> bool {
        matches!(
            (self.peek(), self.peek_next()),
            (Some(IfPartToken::Identifier(name)), Some(IfPartToken::LeftParen))
                if name == "size" || name == "length"
        )
    }

    fn next_identifier_is_contains_function(&self) -> bool {
        matches!(
            (self.peek(), self.peek_next()),
            (Some(IfPartToken::Identifier(name)), Some(IfPartToken::LeftParen))
                if name == "contains"
        )
    }

    fn is_at_end(&self) -> bool {
        self.index == self.tokens.len()
    }
}

fn logical_expression(
    operator: SentryIfPartLogicalOperator,
    operands: Vec<SentryIfPartExpression>,
) -> SentryIfPartExpression {
    if operands.len() == 1 {
        let mut operands = operands;
        return operands.remove(0);
    }
    SentryIfPartExpression::Logical { operator, operands }
}

fn split_method_call_identifier(identifier: &str) -> (Option<String>, String) {
    if let Some(pos) = identifier.rfind('.') {
        (
            Some(identifier[..pos].to_string()),
            identifier[pos + 1..].to_string(),
        )
    } else {
        (None, identifier.to_string())
    }
}

fn expression_to_string(expr: &SentryIfPartExpression) -> Result<String, String> {
    match expr {
        SentryIfPartExpression::Literal(SentryIfPartLiteral::Variable(name)) => Ok(name.clone()),
        SentryIfPartExpression::PropertyAccess { object, property } => {
            let obj_str = expression_to_string(object)?;
            Ok(format!("{obj_str}.{property}"))
        }
        SentryIfPartExpression::IndexAccess { object, index } => {
            let obj_str = expression_to_string(object)?;
            let idx_str = expression_to_string(index)?;
            Ok(format!("{obj_str}[{idx_str}]"))
        }
        SentryIfPartExpression::MethodCall {
            object,
            method,
            args,
        } => {
            let mut arg_strs = Vec::new();
            for arg in args {
                arg_strs.push(expression_to_string(arg)?);
            }
            let args_joined = arg_strs.join(", ");
            if (method == "size" || method == "length") && args.is_empty() {
                if let Some(obj) = object {
                    Ok(format!("{method}({obj})"))
                } else {
                    Ok(format!("{method}({args_joined})"))
                }
            } else {
                if let Some(obj) = object {
                    Ok(format!("{obj}.{method}({args_joined})"))
                } else {
                    Ok(format!("{method}({args_joined})"))
                }
            }
        }
        SentryIfPartExpression::Literal(SentryIfPartLiteral::Number(n)) => Ok(n.clone()),
        SentryIfPartExpression::Literal(SentryIfPartLiteral::String(s)) => Ok(format!("'{s}'")),
        SentryIfPartExpression::Literal(SentryIfPartLiteral::Boolean(b)) => Ok(b.to_string()),
        SentryIfPartExpression::Literal(SentryIfPartLiteral::Null) => Ok("null".to_string()),
        SentryIfPartExpression::Arithmetic {
            left,
            operator,
            right,
        } => {
            let l = expression_to_string(left)?;
            let r = expression_to_string(right)?;
            Ok(format!("{l} {operator} {r}"))
        }
        SentryIfPartExpression::Comparison(cond) => {
            let lit_str = match &cond.literal {
                SentryIfPartLiteral::Boolean(b) => b.to_string(),
                SentryIfPartLiteral::String(s) => format!("'{s}'"),
                SentryIfPartLiteral::Number(n) => n.clone(),
                SentryIfPartLiteral::Null => "null".to_string(),
                SentryIfPartLiteral::Variable(v) => v.clone(),
            };
            let op_str = match cond.operator {
                SentryIfPartOperator::Equal => "==",
                SentryIfPartOperator::NotEqual => "!=",
                SentryIfPartOperator::GreaterThan => ">",
                SentryIfPartOperator::GreaterThanOrEqual => ">=",
                SentryIfPartOperator::LessThan => "<",
                SentryIfPartOperator::LessThanOrEqual => "<=",
            };
            Ok(format!("{} {op_str} {lit_str}", cond.variable_name))
        }
        SentryIfPartExpression::Logical { operator, operands } => {
            let op_str = match operator {
                SentryIfPartLogicalOperator::And => "&&",
                SentryIfPartLogicalOperator::Or => "||",
            };
            let mut parts = Vec::new();
            for op in operands {
                parts.push(format!("({})", expression_to_string(op)?));
            }
            Ok(parts.join(&format!(" {op_str} ")))
        }
        SentryIfPartExpression::Not { operand } => {
            let op_str = expression_to_string(operand)?;
            Ok(format!("!({op_str})"))
        }
        _ => Err("cannot serialize expression to string representation".to_string()),
    }
}

fn expression_to_literal(expr: &SentryIfPartExpression) -> Result<SentryIfPartLiteral, String> {
    match expr {
        SentryIfPartExpression::Literal(lit) => Ok(lit.clone()),
        _ => {
            let s = expression_to_string(expr)?;
            Ok(SentryIfPartLiteral::Variable(s))
        }
    }
}

fn tokenize_if_part_expression(expression: &str) -> Result<Vec<IfPartToken>, String> {
    let mut tokens = Vec::new();
    let bytes = expression.as_bytes();
    let mut index = 0;

    while index < bytes.len() {
        match bytes[index] {
            byte if byte.is_ascii_whitespace() => index += 1,
            b'(' => {
                tokens.push(IfPartToken::LeftParen);
                index += 1;
            }
            b')' => {
                tokens.push(IfPartToken::RightParen);
                index += 1;
            }
            b',' => {
                tokens.push(IfPartToken::Comma);
                index += 1;
            }
            b'!' if bytes.get(index + 1) == Some(&b'=') => {
                tokens.push(IfPartToken::NotEqual);
                index += 2;
            }
            b'!' => {
                tokens.push(IfPartToken::Not);
                index += 1;
            }
            b'=' if bytes.get(index + 1) == Some(&b'=') => {
                tokens.push(IfPartToken::Equal);
                index += 2;
            }
            b'=' => return Err("supported subset requires `==` for equality".to_string()),
            b'>' if bytes.get(index + 1) == Some(&b'=') => {
                tokens.push(IfPartToken::GreaterThanOrEqual);
                index += 2;
            }
            b'>' => {
                tokens.push(IfPartToken::GreaterThan);
                index += 1;
            }
            b'<' if bytes.get(index + 1) == Some(&b'=') => {
                tokens.push(IfPartToken::LessThanOrEqual);
                index += 2;
            }
            b'<' => {
                tokens.push(IfPartToken::LessThan);
                index += 1;
            }
            b'&' if bytes.get(index + 1) == Some(&b'&') => {
                tokens.push(IfPartToken::And);
                index += 2;
            }
            b'&' => return Err("supported subset requires `&&` for logical and".to_string()),
            b'|' if bytes.get(index + 1) == Some(&b'|') => {
                tokens.push(IfPartToken::Or);
                index += 2;
            }
            b'|' => return Err("supported subset requires `||` for logical or".to_string()),
            b'+' => {
                tokens.push(IfPartToken::Plus);
                index += 1;
            }
            b'-' => {
                let is_negative_number =
                    if index + 1 < bytes.len() && bytes[index + 1].is_ascii_digit() {
                        !matches!(
                            tokens.last(),
                            Some(IfPartToken::Identifier(_))
                                | Some(IfPartToken::Literal(_))
                                | Some(IfPartToken::RightParen)
                        )
                    } else {
                        false
                    };

                if is_negative_number {
                    let (value, next_index) = read_if_part_number(expression, index)?;
                    tokens.push(IfPartToken::Literal(SentryIfPartLiteral::Number(value)));
                    index = next_index;
                } else {
                    tokens.push(IfPartToken::Minus);
                    index += 1;
                }
            }
            b'*' => {
                tokens.push(IfPartToken::Multiply);
                index += 1;
            }
            b'/' => {
                tokens.push(IfPartToken::Divide);
                index += 1;
            }
            b'%' => {
                tokens.push(IfPartToken::Modulo);
                index += 1;
            }
            b'?' => {
                tokens.push(IfPartToken::Question);
                index += 1;
            }
            b':' => {
                tokens.push(IfPartToken::Colon);
                index += 1;
            }
            b'[' => {
                tokens.push(IfPartToken::LeftBracket);
                index += 1;
            }
            b']' => {
                tokens.push(IfPartToken::RightBracket);
                index += 1;
            }
            b'.' => {
                tokens.push(IfPartToken::Dot);
                index += 1;
            }
            b'\'' | b'"' => {
                let (value, next_index) = read_quoted_if_part_string(expression, index)?;
                tokens.push(IfPartToken::Literal(SentryIfPartLiteral::String(value)));
                index = next_index;
            }
            b'0'..=b'9' => {
                let (value, next_index) = read_if_part_number(expression, index)?;
                tokens.push(IfPartToken::Literal(SentryIfPartLiteral::Number(value)));
                index = next_index;
            }
            byte if is_identifier_start_byte(byte) => {
                let (word, next_index) = read_if_part_variable_reference(expression, index);
                index = next_index;
                match word {
                    "and" => tokens.push(IfPartToken::And),
                    "or" => tokens.push(IfPartToken::Or),
                    "not" => tokens.push(IfPartToken::Not),
                    "true" => tokens.push(IfPartToken::Literal(SentryIfPartLiteral::Boolean(true))),
                    "false" => {
                        tokens.push(IfPartToken::Literal(SentryIfPartLiteral::Boolean(false)))
                    }
                    "null" => tokens.push(IfPartToken::Literal(SentryIfPartLiteral::Null)),
                    _ => tokens.push(IfPartToken::Identifier(word.to_string())),
                }
            }
            other => {
                return Err(format!(
                    "unsupported token `{}` in ifPart expression",
                    other as char
                ));
            }
        }
    }

    Ok(tokens)
}

fn read_if_part_variable_reference(expression: &str, start: usize) -> (&str, usize) {
    let bytes = expression.as_bytes();
    let mut index = start + 1;

    while index < bytes.len() && is_identifier_byte(bytes[index]) {
        index += 1;
    }

    loop {
        match bytes.get(index) {
            Some(b'.')
                if bytes
                    .get(index + 1)
                    .is_some_and(|candidate| is_identifier_start_byte(*candidate)) =>
            {
                index += 2;
                while index < bytes.len() && is_identifier_byte(bytes[index]) {
                    index += 1;
                }
            }
            Some(b'[')
                if bytes
                    .get(index + 1)
                    .is_some_and(|candidate| candidate.is_ascii_digit()) =>
            {
                index += 2;
                while index < bytes.len() && bytes[index].is_ascii_digit() {
                    index += 1;
                }
                if bytes.get(index) != Some(&b']') {
                    break;
                }
                index += 1;
            }
            Some(b'[')
                if bytes
                    .get(index + 1)
                    .is_some_and(|candidate| *candidate == b'\'' || *candidate == b'"') =>
            {
                let quote = bytes[index + 1];
                index += 2;
                let key_start = index;
                while index < bytes.len() && bytes[index] != quote {
                    if bytes[index] == b'\\' {
                        break;
                    }
                    index += 1;
                }
                if key_start == index
                    || bytes.get(index) != Some(&quote)
                    || bytes.get(index + 1) != Some(&b']')
                {
                    break;
                }
                index += 2;
            }
            _ => break,
        }
    }

    (&expression[start..index], index)
}

fn read_quoted_if_part_string(expression: &str, start: usize) -> Result<(String, usize), String> {
    let bytes = expression.as_bytes();
    let quote = bytes[start];
    let mut index = start + 1;
    while index < bytes.len() {
        match bytes[index] {
            b'\\' => return Err("quoted string literals must not contain escapes".to_string()),
            byte if byte == quote => {
                return Ok((expression[start + 1..index].to_string(), index + 1));
            }
            _ => index += 1,
        }
    }
    Err("quoted string literal must be terminated".to_string())
}

fn read_if_part_number(expression: &str, start: usize) -> Result<(String, usize), String> {
    let bytes = expression.as_bytes();
    let mut index = start;
    if bytes[index] == b'-' {
        index += 1;
    }

    let integer_start = index;
    while index < bytes.len() && bytes[index].is_ascii_digit() {
        index += 1;
    }
    if integer_start == index {
        return Err("number literal must contain digits".to_string());
    }

    if bytes.get(index) == Some(&b'.') {
        index += 1;
        let fractional_start = index;
        while index < bytes.len() && bytes[index].is_ascii_digit() {
            index += 1;
        }
        if fractional_start == index {
            return Err("number literal must contain fractional digits after `.`".to_string());
        }
    }

    let value = expression[start..index].to_string();
    if is_supported_number_literal(&value) {
        Ok((value, index))
    } else {
        Err("right side must be a boolean, quoted string, or number literal".to_string())
    }
}

fn is_identifier_start_byte(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_'
}

fn is_identifier_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

pub fn is_supported_number_literal(value: &str) -> bool {
    let mut chars = value.chars().peekable();
    if matches!(chars.peek(), Some('-')) {
        chars.next();
    }

    let mut integer_digits = 0;
    while matches!(chars.peek(), Some(candidate) if candidate.is_ascii_digit()) {
        integer_digits += 1;
        chars.next();
    }
    if integer_digits == 0 {
        return false;
    }

    if matches!(chars.peek(), Some('.')) {
        chars.next();
        let mut fractional_digits = 0;
        while matches!(chars.peek(), Some(candidate) if candidate.is_ascii_digit()) {
            fractional_digits += 1;
            chars.next();
        }
        if fractional_digits == 0 {
            return false;
        }
    }

    chars.next().is_none()
}

fn is_supported_if_part_variable_name(value: &str) -> bool {
    let bytes = value.as_bytes();
    let Some(first) = bytes.first() else {
        return false;
    };
    if !is_identifier_start_byte(*first) {
        return false;
    }

    let mut index = 1;
    while index < bytes.len() && is_identifier_byte(bytes[index]) {
        index += 1;
    }

    while index < bytes.len() {
        match bytes[index] {
            b'.' => {
                index += 1;
                if !bytes
                    .get(index)
                    .is_some_and(|candidate| is_identifier_start_byte(*candidate))
                {
                    return false;
                }
                index += 1;
                while index < bytes.len() && is_identifier_byte(bytes[index]) {
                    index += 1;
                }
            }
            b'[' => {
                index += 1;
                match bytes.get(index) {
                    Some(candidate) if candidate.is_ascii_digit() => {
                        while index < bytes.len() && bytes[index].is_ascii_digit() {
                            index += 1;
                        }
                        if bytes.get(index) != Some(&b']') {
                            return false;
                        }
                        index += 1;
                    }
                    Some(quote @ (b'\'' | b'"')) => {
                        let quote = *quote;
                        index += 1;
                        let key_start = index;
                        while index < bytes.len() && bytes[index] != quote {
                            if bytes[index] == b'\\' {
                                return false;
                            }
                            index += 1;
                        }
                        if key_start == index
                            || bytes.get(index) != Some(&quote)
                            || bytes.get(index + 1) != Some(&b']')
                        {
                            return false;
                        }
                        index += 2;
                    }
                    _ => return false,
                }
            }
            _ => return false,
        }
    }

    true
}

#[allow(dead_code)]
fn parse_sizing_property_method_operand(value: &str) -> Option<(&'static str, &str)> {
    if let Some(variable_name) = value.strip_suffix(".size")
        && is_supported_if_part_variable_name(variable_name)
    {
        return Some(("size", variable_name));
    }
    if let Some(variable_name) = value.strip_suffix(".length")
        && is_supported_if_part_variable_name(variable_name)
    {
        return Some(("length", variable_name));
    }
    None
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PlanItemOnPart {
    pub id: String,
    pub source_ref: String,
    pub standard_event: String,
}

impl PlanItemOnPart {
    pub const STANDARD_EVENT_COMPLETE: &'static str = "complete";
    pub const STANDARD_EVENT_OCCUR: &'static str = "occur";
    pub const STANDARD_EVENT_TERMINATE: &'static str = "terminate";
    pub const STANDARD_EVENT_START: &'static str = "start";
    pub const STANDARD_EVENT_ENABLE: &'static str = "enable";
    pub const STANDARD_EVENT_DISABLE: &'static str = "disable";
    /// Derived lifecycle event: fires on human-task `complete` or `terminate`.
    pub const STANDARD_EVENT_EXIT: &'static str = "exit";

    /// Owned runtime-supported planItemOnPart standard events.
    /// Events outside this set (reenable, suspend, resume, parentResume, fault, …)
    /// remain intentionally unsupported and must be rejected structurally.
    pub fn is_supported_standard_event(value: &str) -> bool {
        matches!(
            value,
            Self::STANDARD_EVENT_COMPLETE
                | Self::STANDARD_EVENT_OCCUR
                | Self::STANDARD_EVENT_TERMINATE
                | Self::STANDARD_EVENT_ENABLE
                | Self::STANDARD_EVENT_DISABLE
                | Self::STANDARD_EVENT_START
                | Self::STANDARD_EVENT_EXIT
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaseFileItemOnPart {
    pub id: String,
    pub case_file_item_ref: String,
    pub standard_event: String,
}

impl CaseFileItemOnPart {
    pub const STANDARD_EVENT_CREATE: &'static str = "create";
    pub const STANDARD_EVENT_UPDATE: &'static str = "update";
    pub const STANDARD_EVENT_DELETE: &'static str = "delete";
    pub const STANDARD_EVENT_COMPLETE: &'static str = "complete";

    pub fn is_supported_standard_event(value: &str) -> bool {
        matches!(value, "create" | "update" | "delete" | "complete")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanItemDefinitionRef<'a> {
    Stage(&'a Stage),
    HumanTask(&'a HumanTask),
    DecisionTask(&'a DecisionTask),
    ProcessTask(&'a ProcessTask),
    CaseTask(&'a CaseTask),
    Milestone(&'a Milestone),
    EventListener(&'a EventListener),
}

impl<'a> PlanItemDefinitionRef<'a> {
    pub fn id(self) -> &'a str {
        match self {
            Self::Stage(stage) => &stage.id,
            Self::HumanTask(human_task) => &human_task.id,
            Self::DecisionTask(decision_task) => &decision_task.id,
            Self::ProcessTask(process_task) => &process_task.id,
            Self::CaseTask(case_task) => &case_task.id,
            Self::Milestone(milestone) => &milestone.id,
            Self::EventListener(event_listener) => &event_listener.id,
        }
    }

    pub fn name(self) -> Option<&'a str> {
        match self {
            Self::Stage(stage) => stage.name.as_deref(),
            Self::HumanTask(human_task) => human_task.name.as_deref(),
            Self::DecisionTask(decision_task) => decision_task.name.as_deref(),
            Self::ProcessTask(process_task) => process_task.name.as_deref(),
            Self::CaseTask(case_task) => case_task.name.as_deref(),
            Self::Milestone(milestone) => milestone.name.as_deref(),
            Self::EventListener(event_listener) => event_listener.name.as_deref(),
        }
    }
}

fn find_plan_item_in_container<'a>(
    plan_items: &'a [PlanItem],
    stages: &'a [Stage],
    id: &str,
) -> Option<&'a PlanItem> {
    if let Some(plan_item) = plan_items.iter().find(|plan_item| plan_item.id == id) {
        return Some(plan_item);
    }

    stages
        .iter()
        .find_map(|stage| find_plan_item_in_container(&stage.plan_items, &stage.stages, id))
}

#[allow(clippy::too_many_arguments)]
fn find_definition_in_container<'a>(
    human_tasks: &'a [HumanTask],
    decision_tasks: &'a [DecisionTask],
    process_tasks: &'a [ProcessTask],
    case_tasks: &'a [CaseTask],
    milestones: &'a [Milestone],
    event_listeners: &'a [EventListener],
    stages: &'a [Stage],
    id: &str,
) -> Option<PlanItemDefinitionRef<'a>> {
    if let Some(human_task) = human_tasks.iter().find(|human_task| human_task.id == id) {
        return Some(PlanItemDefinitionRef::HumanTask(human_task));
    }

    if let Some(decision_task) = decision_tasks
        .iter()
        .find(|decision_task| decision_task.id == id)
    {
        return Some(PlanItemDefinitionRef::DecisionTask(decision_task));
    }

    if let Some(process_task) = process_tasks
        .iter()
        .find(|process_task| process_task.id == id)
    {
        return Some(PlanItemDefinitionRef::ProcessTask(process_task));
    }

    if let Some(case_task) = case_tasks.iter().find(|case_task| case_task.id == id) {
        return Some(PlanItemDefinitionRef::CaseTask(case_task));
    }

    if let Some(milestone) = milestones.iter().find(|milestone| milestone.id == id) {
        return Some(PlanItemDefinitionRef::Milestone(milestone));
    }

    if let Some(event_listener) = event_listeners
        .iter()
        .find(|event_listener| event_listener.id == id)
    {
        return Some(PlanItemDefinitionRef::EventListener(event_listener));
    }

    if let Some(stage) = stages.iter().find(|stage| stage.id == id) {
        return Some(PlanItemDefinitionRef::Stage(stage));
    }

    stages.iter().find_map(|stage| {
        find_definition_in_container(
            &stage.human_tasks,
            &stage.decision_tasks,
            &stage.process_tasks,
            &stage.case_tasks,
            &stage.milestones,
            &stage.event_listeners,
            &stage.stages,
            id,
        )
    })
}

#[cfg(test)]
mod tests {
    use super::{
        Case, CasePlanModel, HumanTask, PlanItem, PlanItemDefinitionRef, SentryIfPartCondition,
        SentryIfPartExpression, SentryIfPartLiteral, SentryIfPartLogicalOperator,
        SentryIfPartOperator, Stage, parse_sentry_if_part_expression,
    };

    #[test]
    fn resolves_nested_plan_items_and_definitions() {
        let case_definition = Case {
            id: "case-1".to_string(),
            name: Some("Case".to_string()),
            lifecycle_listeners: Vec::new(),
            start_event_type: None,
            start_correlation_configuration: None,
            start_correlation_parameters: Vec::new(),
            case_plan_model: CasePlanModel {
                id: "cpm".to_string(),
                name: Some("Plan".to_string()),
                auto_complete: false,
                form_key: None,
                plan_items: vec![PlanItem {
                    id: "pi-root".to_string(),
                    name: Some("Root".to_string()),
                    definition_ref: "stage-a".to_string(),
                    entry_criteria: vec![],
                    exit_criteria: vec![],
                    manual_activation_rule: None,
                    repetition_rule: None,
                    required_rule: None,
                    parent_completion_rule: None,
                    completion_neutral_rule: None,
                }],
                human_tasks: vec![],
                decision_tasks: vec![],
                process_tasks: vec![],
                case_tasks: vec![],
                milestones: vec![],
                event_listeners: vec![],
                sentries: vec![],
                planning_tables: vec![],
                lifecycle_listeners: Vec::new(),
                stages: vec![Stage {
                    id: "stage-a".to_string(),
                    name: Some("Stage A".to_string()),
                    auto_complete: true,
                    plan_items: vec![PlanItem {
                        id: "pi-nested".to_string(),
                        name: Some("Nested".to_string()),
                        definition_ref: "task-a".to_string(),
                        entry_criteria: vec![],
                        exit_criteria: vec![],
                        manual_activation_rule: None,
                        repetition_rule: None,
                        required_rule: None,
                        parent_completion_rule: None,
                        completion_neutral_rule: None,
                    }],
                    human_tasks: vec![HumanTask {
                        id: "task-a".to_string(),
                        name: Some("Task A".to_string()),
                        is_blocking: true,
                        form_key: None,
                        ..Default::default()
                    }],
                    decision_tasks: vec![],
                    process_tasks: vec![],
                    case_tasks: vec![],
                    milestones: vec![],
                    event_listeners: vec![],
                    sentries: vec![],
                    planning_tables: vec![],
                    stages: vec![],
                    lifecycle_listeners: Vec::new(),
                }],
            },
        };

        assert_eq!(
            case_definition
                .find_plan_item("pi-nested")
                .map(|plan_item| plan_item.id.as_str()),
            Some("pi-nested")
        );

        let definition = case_definition
            .find_plan_item_definition("task-a")
            .expect("task definition should be found");
        assert_eq!(definition.id(), "task-a");
        assert_eq!(definition.name(), Some("Task A"));

        assert_eq!(
            case_definition.find_plan_item_definition("stage-a"),
            Some(PlanItemDefinitionRef::Stage(
                case_definition
                    .case_plan_model
                    .stages
                    .first()
                    .expect("stage exists")
            ))
        );
    }

    #[test]
    fn parses_parenthesized_if_part_comparison() {
        assert_eq!(
            parse_sentry_if_part_expression("${(approved == true)}").expect("expression"),
            SentryIfPartExpression::Comparison(SentryIfPartCondition {
                variable_name: "approved".to_string(),
                operator: SentryIfPartOperator::Equal,
                literal: SentryIfPartLiteral::Boolean(true),
            })
        );
    }

    #[test]
    fn parses_parenthesized_if_part_logical_groups() {
        assert_eq!(
            parse_sentry_if_part_expression("${(approved == true) && (amount == 42)}")
                .expect("and expression"),
            SentryIfPartExpression::Logical {
                operator: SentryIfPartLogicalOperator::And,
                operands: vec![
                    SentryIfPartExpression::Comparison(SentryIfPartCondition {
                        variable_name: "approved".to_string(),
                        operator: SentryIfPartOperator::Equal,
                        literal: SentryIfPartLiteral::Boolean(true),
                    }),
                    SentryIfPartExpression::Comparison(SentryIfPartCondition {
                        variable_name: "amount".to_string(),
                        operator: SentryIfPartOperator::Equal,
                        literal: SentryIfPartLiteral::Number("42".to_string()),
                    }),
                ],
            }
        );

        assert_eq!(
            parse_sentry_if_part_expression("${(approved == true) || (decision != 'denied')}")
                .expect("or expression"),
            SentryIfPartExpression::Logical {
                operator: SentryIfPartLogicalOperator::Or,
                operands: vec![
                    SentryIfPartExpression::Comparison(SentryIfPartCondition {
                        variable_name: "approved".to_string(),
                        operator: SentryIfPartOperator::Equal,
                        literal: SentryIfPartLiteral::Boolean(true),
                    }),
                    SentryIfPartExpression::Comparison(SentryIfPartCondition {
                        variable_name: "decision".to_string(),
                        operator: SentryIfPartOperator::NotEqual,
                        literal: SentryIfPartLiteral::String("denied".to_string()),
                    }),
                ],
            }
        );
    }

    #[test]
    fn parses_textual_if_part_logical_operators() {
        assert_eq!(
            parse_sentry_if_part_expression("${(approved == true) and (amount == 42)}")
                .expect("textual and expression"),
            SentryIfPartExpression::Logical {
                operator: SentryIfPartLogicalOperator::And,
                operands: vec![
                    SentryIfPartExpression::Comparison(SentryIfPartCondition {
                        variable_name: "approved".to_string(),
                        operator: SentryIfPartOperator::Equal,
                        literal: SentryIfPartLiteral::Boolean(true),
                    }),
                    SentryIfPartExpression::Comparison(SentryIfPartCondition {
                        variable_name: "amount".to_string(),
                        operator: SentryIfPartOperator::Equal,
                        literal: SentryIfPartLiteral::Number("42".to_string()),
                    }),
                ],
            }
        );

        assert_eq!(
            parse_sentry_if_part_expression("${(approved == true) or (decision != 'denied')}")
                .expect("textual or expression"),
            SentryIfPartExpression::Logical {
                operator: SentryIfPartLogicalOperator::Or,
                operands: vec![
                    SentryIfPartExpression::Comparison(SentryIfPartCondition {
                        variable_name: "approved".to_string(),
                        operator: SentryIfPartOperator::Equal,
                        literal: SentryIfPartLiteral::Boolean(true),
                    }),
                    SentryIfPartExpression::Comparison(SentryIfPartCondition {
                        variable_name: "decision".to_string(),
                        operator: SentryIfPartOperator::NotEqual,
                        literal: SentryIfPartLiteral::String("denied".to_string()),
                    }),
                ],
            }
        );
    }

    #[test]
    fn parses_null_and_empty_if_part_expressions() {
        assert_eq!(
            parse_sentry_if_part_expression("${optionalValue == null}")
                .expect("null equality expression"),
            SentryIfPartExpression::Comparison(SentryIfPartCondition {
                variable_name: "optionalValue".to_string(),
                operator: SentryIfPartOperator::Equal,
                literal: SentryIfPartLiteral::Null,
            })
        );

        assert_eq!(
            parse_sentry_if_part_expression("${optionalValue != null}")
                .expect("null inequality expression"),
            SentryIfPartExpression::Comparison(SentryIfPartCondition {
                variable_name: "optionalValue".to_string(),
                operator: SentryIfPartOperator::NotEqual,
                literal: SentryIfPartLiteral::Null,
            })
        );

        assert_eq!(
            parse_sentry_if_part_expression("${empty(comment)}").expect("empty function"),
            SentryIfPartExpression::Empty {
                variable_name: "comment".to_string(),
            }
        );

        assert_eq!(
            parse_sentry_if_part_expression("${not empty(comment)}")
                .expect("textual negated empty function"),
            SentryIfPartExpression::Not {
                operand: Box::new(SentryIfPartExpression::Empty {
                    variable_name: "comment".to_string(),
                }),
            }
        );

        assert_eq!(
            parse_sentry_if_part_expression("${!empty(comment)}")
                .expect("bang negated empty function"),
            SentryIfPartExpression::Not {
                operand: Box::new(SentryIfPartExpression::Empty {
                    variable_name: "comment".to_string(),
                }),
            }
        );
    }

    #[test]
    fn parses_property_path_and_indexed_if_part_expressions() {
        assert_eq!(
            parse_sentry_if_part_expression("${customer.name == 'Alice'}")
                .expect("property path expression"),
            SentryIfPartExpression::Comparison(SentryIfPartCondition {
                variable_name: "customer.name".to_string(),
                operator: SentryIfPartOperator::Equal,
                literal: SentryIfPartLiteral::String("Alice".to_string()),
            })
        );

        assert_eq!(
            parse_sentry_if_part_expression("${customer.age >= 18}")
                .expect("numeric property path expression"),
            SentryIfPartExpression::Comparison(SentryIfPartCondition {
                variable_name: "customer.age".to_string(),
                operator: SentryIfPartOperator::GreaterThanOrEqual,
                literal: SentryIfPartLiteral::Number("18".to_string()),
            })
        );

        assert_eq!(
            parse_sentry_if_part_expression("${items[0].status == 'open'}")
                .expect("indexed property path expression"),
            SentryIfPartExpression::Comparison(SentryIfPartCondition {
                variable_name: "items[0].status".to_string(),
                operator: SentryIfPartOperator::Equal,
                literal: SentryIfPartLiteral::String("open".to_string()),
            })
        );

        assert_eq!(
            parse_sentry_if_part_expression("${empty(customer.email)}")
                .expect("empty property path expression"),
            SentryIfPartExpression::Empty {
                variable_name: "customer.email".to_string(),
            }
        );
    }

    #[test]
    fn parses_variable_rhs_bracket_key_and_size_if_part_expressions() {
        assert_eq!(
            parse_sentry_if_part_expression("${customer.age >= minAge}")
                .expect("variable rhs numeric expression"),
            SentryIfPartExpression::Comparison(SentryIfPartCondition {
                variable_name: "customer.age".to_string(),
                operator: SentryIfPartOperator::GreaterThanOrEqual,
                literal: SentryIfPartLiteral::Variable("minAge".to_string()),
            })
        );

        assert_eq!(
            parse_sentry_if_part_expression("${customer['status'] == expectedStatus}")
                .expect("single quoted bracket key expression"),
            SentryIfPartExpression::Comparison(SentryIfPartCondition {
                variable_name: "customer['status']".to_string(),
                operator: SentryIfPartOperator::Equal,
                literal: SentryIfPartLiteral::Variable("expectedStatus".to_string()),
            })
        );

        assert_eq!(
            parse_sentry_if_part_expression("${customer[\"name\"] == 'Alice'}")
                .expect("double quoted bracket key expression"),
            SentryIfPartExpression::Comparison(SentryIfPartCondition {
                variable_name: "customer[\"name\"]".to_string(),
                operator: SentryIfPartOperator::Equal,
                literal: SentryIfPartLiteral::String("Alice".to_string()),
            })
        );

        assert_eq!(
            parse_sentry_if_part_expression("${approved == expectedApproval}")
                .expect("boolean variable rhs expression"),
            SentryIfPartExpression::Comparison(SentryIfPartCondition {
                variable_name: "approved".to_string(),
                operator: SentryIfPartOperator::Equal,
                literal: SentryIfPartLiteral::Variable("expectedApproval".to_string()),
            })
        );

        assert_eq!(
            parse_sentry_if_part_expression("${optionalValue == expectedNull}")
                .expect("null variable rhs expression"),
            SentryIfPartExpression::Comparison(SentryIfPartCondition {
                variable_name: "optionalValue".to_string(),
                operator: SentryIfPartOperator::Equal,
                literal: SentryIfPartLiteral::Variable("expectedNull".to_string()),
            })
        );

        assert_eq!(
            parse_sentry_if_part_expression("${size(items) >= minimumItemCount}")
                .expect("size function expression"),
            SentryIfPartExpression::Comparison(SentryIfPartCondition {
                variable_name: "size(items)".to_string(),
                operator: SentryIfPartOperator::GreaterThanOrEqual,
                literal: SentryIfPartLiteral::Variable("minimumItemCount".to_string()),
            })
        );

        assert_eq!(
            parse_sentry_if_part_expression("${items.size() >= minimumItemCount}")
                .expect("property size method expression"),
            SentryIfPartExpression::Comparison(SentryIfPartCondition {
                variable_name: "size(items)".to_string(),
                operator: SentryIfPartOperator::GreaterThanOrEqual,
                literal: SentryIfPartLiteral::Variable("minimumItemCount".to_string()),
            })
        );

        assert_eq!(
            parse_sentry_if_part_expression("${length(customer.name) >= minimumNameLength}")
                .expect("length function expression"),
            SentryIfPartExpression::Comparison(SentryIfPartCondition {
                variable_name: "length(customer.name)".to_string(),
                operator: SentryIfPartOperator::GreaterThanOrEqual,
                literal: SentryIfPartLiteral::Variable("minimumNameLength".to_string()),
            })
        );

        assert_eq!(
            parse_sentry_if_part_expression("${customer.name.length() >= minimumNameLength}")
                .expect("property length method expression"),
            SentryIfPartExpression::Comparison(SentryIfPartCondition {
                variable_name: "length(customer.name)".to_string(),
                operator: SentryIfPartOperator::GreaterThanOrEqual,
                literal: SentryIfPartLiteral::Variable("minimumNameLength".to_string()),
            })
        );
    }

    #[test]
    fn parses_method_call_syntax_inside_property_paths() {
        let parsed = parse_sentry_if_part_expression("${customer.name() == 'Alice'}")
            .expect("method calls are now supported");
        assert_eq!(
            parsed,
            SentryIfPartExpression::Comparison(SentryIfPartCondition {
                variable_name: "customer.name()".to_string(),
                operator: SentryIfPartOperator::Equal,
                literal: SentryIfPartLiteral::String("Alice".to_string()),
            })
        );
    }

    #[test]
    fn parses_nested_mixed_priority_numeric_and_negated_if_part_expressions() {
        assert_eq!(
            parse_sentry_if_part_expression(
                "${(approved == true && amount > 100) || reviewer == 'lead'}",
            )
            .expect("nested mixed priority expression"),
            SentryIfPartExpression::Logical {
                operator: SentryIfPartLogicalOperator::Or,
                operands: vec![
                    SentryIfPartExpression::Logical {
                        operator: SentryIfPartLogicalOperator::And,
                        operands: vec![
                            SentryIfPartExpression::Comparison(SentryIfPartCondition {
                                variable_name: "approved".to_string(),
                                operator: SentryIfPartOperator::Equal,
                                literal: SentryIfPartLiteral::Boolean(true),
                            }),
                            SentryIfPartExpression::Comparison(SentryIfPartCondition {
                                variable_name: "amount".to_string(),
                                operator: SentryIfPartOperator::GreaterThan,
                                literal: SentryIfPartLiteral::Number("100".to_string()),
                            }),
                        ],
                    },
                    SentryIfPartExpression::Comparison(SentryIfPartCondition {
                        variable_name: "reviewer".to_string(),
                        operator: SentryIfPartOperator::Equal,
                        literal: SentryIfPartLiteral::String("lead".to_string()),
                    }),
                ],
            }
        );

        assert_eq!(
            parse_sentry_if_part_expression("${!(approved == true)}")
                .expect("bang negated expression"),
            SentryIfPartExpression::Not {
                operand: Box::new(SentryIfPartExpression::Comparison(SentryIfPartCondition {
                    variable_name: "approved".to_string(),
                    operator: SentryIfPartOperator::Equal,
                    literal: SentryIfPartLiteral::Boolean(true),
                })),
            }
        );

        assert_eq!(
            parse_sentry_if_part_expression("${not(rejected == true)}")
                .expect("textual negated expression"),
            SentryIfPartExpression::Not {
                operand: Box::new(SentryIfPartExpression::Comparison(SentryIfPartCondition {
                    variable_name: "rejected".to_string(),
                    operator: SentryIfPartOperator::Equal,
                    literal: SentryIfPartLiteral::Boolean(true),
                })),
            }
        );

        assert_eq!(
            parse_sentry_if_part_expression("${amount >= 10 && amount <= 20}")
                .expect("numeric range expression"),
            SentryIfPartExpression::Logical {
                operator: SentryIfPartLogicalOperator::And,
                operands: vec![
                    SentryIfPartExpression::Comparison(SentryIfPartCondition {
                        variable_name: "amount".to_string(),
                        operator: SentryIfPartOperator::GreaterThanOrEqual,
                        literal: SentryIfPartLiteral::Number("10".to_string()),
                    }),
                    SentryIfPartExpression::Comparison(SentryIfPartCondition {
                        variable_name: "amount".to_string(),
                        operator: SentryIfPartOperator::LessThanOrEqual,
                        literal: SentryIfPartLiteral::Number("20".to_string()),
                    }),
                ],
            }
        );
    }

    #[test]
    fn rejects_function_and_malformed_if_part_expressions_without_panic() {
        let function = parse_sentry_if_part_expression("${isApproved() == true}")
            .expect("function call expression is now supported");
        assert_eq!(
            function,
            SentryIfPartExpression::Comparison(SentryIfPartCondition {
                variable_name: "isApproved()".to_string(),
                operator: SentryIfPartOperator::Equal,
                literal: SentryIfPartLiteral::Boolean(true),
            })
        );

        let malformed =
            parse_sentry_if_part_expression("${approved == true &&}").expect_err("malformed");
        assert!(
            malformed.contains("Unsupported CMMN ifPart condition"),
            "unexpected error: {malformed}"
        );

        parse_sentry_if_part_expression("${contains(customer.name, 'Ann') == true}")
            .expect("contains string literal expression");
        parse_sentry_if_part_expression("${contains(tags, expectedTag) == true}")
            .expect("contains array variable expression");

        assert_eq!(
            parse_sentry_if_part_expression("${contains(tags, expectedTag)}")
                .expect("standalone contains expression"),
            SentryIfPartExpression::Contains {
                collection_variable_name: "tags".to_string(),
                value: SentryIfPartLiteral::Variable("expectedTag".to_string()),
                expected: true,
            }
        );

        assert_eq!(
            parse_sentry_if_part_expression("${contains(customer.name, 'Ann') != false}")
                .expect("contains not-equal false expression"),
            SentryIfPartExpression::Contains {
                collection_variable_name: "customer.name".to_string(),
                value: SentryIfPartLiteral::String("Ann".to_string()),
                expected: true,
            }
        );

        assert_eq!(
            parse_sentry_if_part_expression("${contains(customer.name, 'Bob') != true}")
                .expect("contains not-equal true expression"),
            SentryIfPartExpression::Contains {
                collection_variable_name: "customer.name".to_string(),
                value: SentryIfPartLiteral::String("Bob".to_string()),
                expected: false,
            }
        );

        assert_eq!(
            parse_sentry_if_part_expression("${not(empty(items))}")
                .expect("not empty function expression"),
            SentryIfPartExpression::Not {
                operand: Box::new(SentryIfPartExpression::Empty {
                    variable_name: "items".to_string(),
                }),
            }
        );

        assert_eq!(
            parse_sentry_if_part_expression("${not(contains(tags, expectedTag))}")
                .expect("not contains function expression"),
            SentryIfPartExpression::Not {
                operand: Box::new(SentryIfPartExpression::Contains {
                    collection_variable_name: "tags".to_string(),
                    value: SentryIfPartLiteral::Variable("expectedTag".to_string()),
                    expected: true,
                }),
            }
        );

        assert_eq!(
            parse_sentry_if_part_expression("${not(customer.active)}")
                .expect("not boolean property path expression"),
            SentryIfPartExpression::Not {
                operand: Box::new(SentryIfPartExpression::Comparison(SentryIfPartCondition {
                    variable_name: "customer.active".to_string(),
                    operator: SentryIfPartOperator::Equal,
                    literal: SentryIfPartLiteral::Boolean(true),
                })),
            }
        );

        assert_eq!(
            parse_sentry_if_part_expression("${customer.active}")
                .expect("bare boolean property path expression"),
            SentryIfPartExpression::Comparison(SentryIfPartCondition {
                variable_name: "customer.active".to_string(),
                operator: SentryIfPartOperator::Equal,
                literal: SentryIfPartLiteral::Boolean(true),
            })
        );

        assert_eq!(
            parse_sentry_if_part_expression("${caseFlags.ready}")
                .expect("bare boolean case flag path expression"),
            SentryIfPartExpression::Comparison(SentryIfPartCondition {
                variable_name: "caseFlags.ready".to_string(),
                operator: SentryIfPartOperator::Equal,
                literal: SentryIfPartLiteral::Boolean(true),
            })
        );

        assert_eq!(
            parse_sentry_if_part_expression("${customer.active && caseFlags.ready}")
                .expect("bare boolean property path logical and expression"),
            SentryIfPartExpression::Logical {
                operator: SentryIfPartLogicalOperator::And,
                operands: vec![
                    SentryIfPartExpression::Comparison(SentryIfPartCondition {
                        variable_name: "customer.active".to_string(),
                        operator: SentryIfPartOperator::Equal,
                        literal: SentryIfPartLiteral::Boolean(true),
                    }),
                    SentryIfPartExpression::Comparison(SentryIfPartCondition {
                        variable_name: "caseFlags.ready".to_string(),
                        operator: SentryIfPartOperator::Equal,
                        literal: SentryIfPartLiteral::Boolean(true),
                    }),
                ],
            }
        );

        assert_eq!(
            parse_sentry_if_part_expression("${customer.active || caseFlags.ready}")
                .expect("bare boolean property path logical or expression"),
            SentryIfPartExpression::Logical {
                operator: SentryIfPartLogicalOperator::Or,
                operands: vec![
                    SentryIfPartExpression::Comparison(SentryIfPartCondition {
                        variable_name: "customer.active".to_string(),
                        operator: SentryIfPartOperator::Equal,
                        literal: SentryIfPartLiteral::Boolean(true),
                    }),
                    SentryIfPartExpression::Comparison(SentryIfPartCondition {
                        variable_name: "caseFlags.ready".to_string(),
                        operator: SentryIfPartOperator::Equal,
                        literal: SentryIfPartLiteral::Boolean(true),
                    }),
                ],
            }
        );

        assert_eq!(
            parse_sentry_if_part_expression("${customer.active && not(caseFlags.blocked)}")
                .expect("bare boolean property path logical and with not expression"),
            SentryIfPartExpression::Logical {
                operator: SentryIfPartLogicalOperator::And,
                operands: vec![
                    SentryIfPartExpression::Comparison(SentryIfPartCondition {
                        variable_name: "customer.active".to_string(),
                        operator: SentryIfPartOperator::Equal,
                        literal: SentryIfPartLiteral::Boolean(true),
                    }),
                    SentryIfPartExpression::Not {
                        operand: Box::new(SentryIfPartExpression::Comparison(
                            SentryIfPartCondition {
                                variable_name: "caseFlags.blocked".to_string(),
                                operator: SentryIfPartOperator::Equal,
                                literal: SentryIfPartLiteral::Boolean(true),
                            },
                        )),
                    },
                ],
            }
        );

        let complex_not = parse_sentry_if_part_expression("${not(customer.active && approved)}")
            .expect("complex nested not operands are now supported");
        assert_eq!(
            complex_not,
            SentryIfPartExpression::Not {
                operand: Box::new(SentryIfPartExpression::Logical {
                    operator: SentryIfPartLogicalOperator::And,
                    operands: vec![
                        SentryIfPartExpression::Comparison(SentryIfPartCondition {
                            variable_name: "customer.active".to_string(),
                            operator: SentryIfPartOperator::Equal,
                            literal: SentryIfPartLiteral::Boolean(true),
                        }),
                        SentryIfPartExpression::Comparison(SentryIfPartCondition {
                            variable_name: "approved".to_string(),
                            operator: SentryIfPartOperator::Equal,
                            literal: SentryIfPartLiteral::Boolean(true),
                        }),
                    ],
                }),
            }
        );

        let method_not = parse_sentry_if_part_expression("${not(isApproved())}")
            .expect("method calls inside not are now supported");
        assert_eq!(
            method_not,
            SentryIfPartExpression::Not {
                operand: Box::new(SentryIfPartExpression::MethodCall {
                    object: None,
                    method: "isApproved".to_string(),
                    args: vec![],
                }),
            }
        );

        let complex_contains =
            parse_sentry_if_part_expression("${contains(customer.name + suffix, 'x') == true}")
                .expect("complex contains arguments are now supported");
        assert_eq!(
            complex_contains,
            SentryIfPartExpression::Contains {
                collection_variable_name: "customer.name + suffix".to_string(),
                value: SentryIfPartLiteral::String("x".to_string()),
                expected: true,
            }
        );

        let contains_method = parse_sentry_if_part_expression("${customer.name.contains('x')}")
            .expect("method calls are now supported");
        assert_eq!(
            contains_method,
            SentryIfPartExpression::MethodCall {
                object: Some("customer.name".to_string()),
                method: "contains".to_string(),
                args: vec![SentryIfPartExpression::Literal(
                    SentryIfPartLiteral::String("x".to_string())
                )],
            }
        );

        let size_method_with_argument = parse_sentry_if_part_expression("${items.size(extra) > 0}")
            .expect("property size method arguments are now supported");
        assert_eq!(
            size_method_with_argument,
            SentryIfPartExpression::Comparison(SentryIfPartCondition {
                variable_name: "items.size(extra)".to_string(),
                operator: SentryIfPartOperator::GreaterThan,
                literal: SentryIfPartLiteral::Number("0".to_string()),
            })
        );

        let dynamic_index = parse_sentry_if_part_expression("${items[index].status == 'open'}")
            .expect("dynamic indexes are now supported");
        assert_eq!(
            dynamic_index,
            SentryIfPartExpression::Comparison(SentryIfPartCondition {
                variable_name: "items[index].status".to_string(),
                operator: SentryIfPartOperator::Equal,
                literal: SentryIfPartLiteral::String("open".to_string()),
            })
        );

        let arithmetic = parse_sentry_if_part_expression("${customer.age + 1 >= minAge}")
            .expect("arithmetic is now supported");
        assert_eq!(
            arithmetic,
            SentryIfPartExpression::Comparison(SentryIfPartCondition {
                variable_name: "customer.age + 1".to_string(),
                operator: SentryIfPartOperator::GreaterThanOrEqual,
                literal: SentryIfPartLiteral::Variable("minAge".to_string()),
            })
        );

        let complex_length =
            parse_sentry_if_part_expression("${length(customer.name + suffix) >= 5}")
                .expect("complex length arguments are now supported");
        assert_eq!(
            complex_length,
            SentryIfPartExpression::Comparison(SentryIfPartCondition {
                variable_name: "length(customer.name + suffix)".to_string(),
                operator: SentryIfPartOperator::GreaterThanOrEqual,
                literal: SentryIfPartLiteral::Number("5".to_string()),
            })
        );

        let ternary = parse_sentry_if_part_expression("${approved ? true : false}")
            .expect("ternary is now supported");
        assert_eq!(
            ternary,
            SentryIfPartExpression::Ternary {
                condition: Box::new(SentryIfPartExpression::Comparison(SentryIfPartCondition {
                    variable_name: "approved".to_string(),
                    operator: SentryIfPartOperator::Equal,
                    literal: SentryIfPartLiteral::Boolean(true),
                })),
                true_expr: Box::new(SentryIfPartExpression::Literal(
                    SentryIfPartLiteral::Boolean(true)
                )),
                false_expr: Box::new(SentryIfPartExpression::Literal(
                    SentryIfPartLiteral::Boolean(false)
                )),
            }
        );
    }

    /// P142c: deeply nested parenthesized ifPart expressions must return a
    /// parse error (not panic / stack overflow).
    #[test]
    fn p142c_if_part_nesting_depth_limit() {
        let deep = format!(
            "${{{}}}",
            format!("{}true{}", "(".repeat(200), ")".repeat(200))
        );
        let err = parse_sentry_if_part_expression(&deep).expect_err("200 nested parens");
        assert!(
            err.contains("maximum depth") || err.contains("nesting"),
            "unexpected error: {err}"
        );

        let ok = format!(
            "${{{}}}",
            format!("{}true{}", "(".repeat(10), ")".repeat(10))
        );
        parse_sentry_if_part_expression(&ok).expect("shallow nesting must still parse");
    }
}
