use chrono::{DateTime, Utc};
use flowable_cmmn_model::SentryIfPartExpression as SharedSentryIfPartExpression;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum CmmnCaseInstanceState {
    Active,
    Completed,
    Terminated,
    /// Java `CaseInstanceState.SUSPENDED` — required by CMMN job parent resolver.
    /// Serde variant name is preserved as `Suspended` (do not rename existing variants).
    Suspended,
}

impl CmmnCaseInstanceState {
    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            Self::Active => "ACTIVE",
            Self::Completed => "COMPLETED",
            Self::Terminated => "TERMINATED",
            Self::Suspended => "SUSPENDED",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum CmmnStageInstanceState {
    Available,
    /// Java `PlanItemInstanceState.ENABLED` (`PlanItemInstanceState.java:26`).
    Enabled,
    Active,
    Completed,
    Terminated,
}

impl CmmnStageInstanceState {
    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            Self::Available => "AVAILABLE",
            Self::Enabled => "ENABLED",
            Self::Active => "ACTIVE",
            Self::Completed => "COMPLETED",
            Self::Terminated => "TERMINATED",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum CmmnHumanTaskState {
    Available,
    /// Java `PlanItemInstanceState.ENABLED` (`PlanItemInstanceState.java:26`).
    Enabled,
    Active,
    Disabled,
    Completed,
    Terminated,
}

impl CmmnHumanTaskState {
    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            Self::Available => "AVAILABLE",
            Self::Enabled => "ENABLED",
            Self::Active => "ACTIVE",
            Self::Disabled => "DISABLED",
            Self::Completed => "COMPLETED",
            Self::Terminated => "TERMINATED",
        }
    }
}

/// Java `org.flowable.task.api.DelegationState` (TaskEntity.delegationState):
/// set by the delegate/resolve task actions. Serialized as the Java `name()`
/// (`PENDING`/`RESOLVED`) for storage parity; deserialization accepts
/// Java's case-insensitive REST strings (TaskBaseResource.java:70-86).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CmmnDelegationState {
    /// Java `DelegationState.PENDING` — set by `DelegateTaskCmd.java:38`.
    Pending,
    /// Java `DelegationState.RESOLVED` — set by `ResolveTaskCmd.java:55`.
    Resolved,
}

impl CmmnDelegationState {
    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "PENDING",
            Self::Resolved => "RESOLVED",
        }
    }
}

impl Serialize for CmmnDelegationState {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for CmmnDelegationState {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        match value.to_ascii_uppercase().as_str() {
            "PENDING" => Ok(Self::Pending),
            "RESOLVED" => Ok(Self::Resolved),
            other => Err(serde::de::Error::custom(format!(
                "Illegal value for delegationState: {other}"
            ))),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CmmnPlanItem {
    pub id: String,
    pub definition_ref: String,
    pub name: Option<String>,
    pub entry_criterion_ids: Vec<String>,
    pub exit_criterion_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manual_activation_rule: Option<CmmnSentryIfPartExpression>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repetition_rule: Option<CmmnSentryIfPartExpression>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required_rule: Option<CmmnSentryIfPartExpression>,
    // Java parity: ParentCompletionRule.getType() - controls whether this plan item blocks
    // parent (stage/case) completion (PlanItemInstanceContainerUtil.java:86-146). None/"default"
    // means the standard evaluation applies.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_completion_rule: Option<String>,
    // Java parity: completionNeutralRule condition - when satisfied the plan item does not
    // prevent parent completion while AVAILABLE (ExpressionUtil.isCompletionNeutralPlanItemInstance).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completion_neutral_rule: Option<CmmnSentryIfPartExpression>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CmmnPlanItemOnPart {
    pub id: String,
    pub source_ref: String,
    pub standard_event: String,
}

impl CmmnPlanItemOnPart {
    pub const STANDARD_EVENT_COMPLETE: &'static str = "complete";
    pub const STANDARD_EVENT_OCCUR: &'static str = "occur";
    pub const STANDARD_EVENT_TERMINATE: &'static str = "terminate";
    pub const STANDARD_EVENT_ENABLE: &'static str = "enable";
    pub const STANDARD_EVENT_DISABLE: &'static str = "disable";
    pub const STANDARD_EVENT_START: &'static str = "start";
    /// Derived lifecycle event: fires once when a human task or stage leaves the
    /// active lifecycle via `complete` or `terminate` (not a primitive transition).
    pub const STANDARD_EVENT_EXIT: &'static str = "exit";

    pub fn new(
        id: impl Into<String>,
        source_ref: impl Into<String>,
        standard_event: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            source_ref: source_ref.into(),
            standard_event: standard_event.into(),
        }
    }

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

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CmmnCaseFileItemOnPart {
    pub id: String,
    pub case_file_item_ref: String,
    pub standard_event: String,
}

impl CmmnCaseFileItemOnPart {
    pub const STANDARD_EVENT_CREATE: &'static str = "create";
    pub const STANDARD_EVENT_UPDATE: &'static str = "update";
    pub const STANDARD_EVENT_DELETE: &'static str = "delete";
    pub const STANDARD_EVENT_COMPLETE: &'static str = "complete";

    pub fn new(
        id: impl Into<String>,
        case_file_item_ref: impl Into<String>,
        standard_event: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            case_file_item_ref: case_file_item_ref.into(),
            standard_event: standard_event.into(),
        }
    }

    pub fn is_supported_standard_event(value: &str) -> bool {
        matches!(value, "create" | "update" | "delete" | "complete")
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CmmnCaseFileItem {
    pub id: String,
    #[serde(default)]
    pub definition_ref: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    #[serde(default)]
    pub path: String,
    #[serde(default = "default_case_file_item_version")]
    pub version: u64,
    pub name: String,
    pub value: Option<serde_json::Value>,
    pub state: CmmnCaseFileItemState,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum CmmnCaseFileItemState {
    Available,
    Removed,
}

impl CmmnCaseFileItemState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Available => "AVAILABLE",
            Self::Removed => "REMOVED",
        }
    }
}

impl CmmnCaseFileItem {
    pub fn new(id: impl Into<String>, name: impl Into<String>) -> Self {
        let id = id.into();
        Self {
            definition_ref: id.clone(),
            path: format!("/{id}"),
            id,
            parent_id: None,
            version: 1,
            name: name.into(),
            value: None,
            state: CmmnCaseFileItemState::Available,
        }
    }

    pub fn with_value(mut self, value: serde_json::Value) -> Self {
        self.value = Some(value);
        self
    }

    pub fn with_definition_ref(mut self, definition_ref: impl Into<String>) -> Self {
        self.definition_ref = definition_ref.into();
        self
    }
    pub fn with_parent(mut self, parent_id: impl Into<String>) -> Self {
        self.parent_id = Some(parent_id.into());
        self
    }
}

fn default_case_file_item_version() -> u64 {
    1
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CmmnSentry {
    pub id: String,
    pub plan_item_on_parts: Vec<CmmnPlanItemOnPart>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub case_file_item_on_parts: Vec<CmmnCaseFileItemOnPart>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub if_part: Option<CmmnSentryIfPartExpression>,
    /// Sentry trigger mode. `None` means the Java default trigger mode
    /// (`Sentry.java:30-32`: `triggerMode == null || "default"`), where
    /// satisfied parts of a multi-part sentry are persisted across
    /// commands. `Some("onEvent")` mirrors `Sentry.TRIGGER_MODE_ON_EVENT`
    /// (`Sentry.java:24, :34-36`): nothing is persisted and the ifPart
    /// must hold at the moment all onParts are satisfied.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trigger_mode: Option<String>,
}

impl CmmnSentry {
    /// Java `Sentry.TRIGGER_MODE_DEFAULT` (`Sentry.java:23`).
    pub const TRIGGER_MODE_DEFAULT: &'static str = "default";
    /// Java `Sentry.TRIGGER_MODE_ON_EVENT` (`Sentry.java:24`).
    pub const TRIGGER_MODE_ON_EVENT: &'static str = "onEvent";

    pub fn new(id: impl Into<String>, plan_item_on_part: CmmnPlanItemOnPart) -> Self {
        Self {
            id: id.into(),
            plan_item_on_parts: vec![plan_item_on_part],
            case_file_item_on_parts: Vec::new(),
            if_part: None,
            trigger_mode: None,
        }
    }

    pub fn with_plan_item_on_part(mut self, plan_item_on_part: CmmnPlanItemOnPart) -> Self {
        self.plan_item_on_parts.push(plan_item_on_part);
        self
    }

    /// Java `Sentry.setTriggerMode` (`Sentry.java:41-43`).
    pub fn with_trigger_mode(mut self, trigger_mode: impl Into<String>) -> Self {
        self.trigger_mode = Some(trigger_mode.into());
        self
    }

    /// Java `Sentry.isDefaultTriggerMode` (`Sentry.java:30-32`):
    /// `triggerMode == null || TRIGGER_MODE_DEFAULT.equals(triggerMode)`.
    pub fn is_default_trigger_mode(&self) -> bool {
        self.trigger_mode
            .as_deref()
            .is_none_or(|mode| mode == Self::TRIGGER_MODE_DEFAULT)
    }

    /// Java `Sentry.isOnEventTriggerMode` (`Sentry.java:34-36`).
    pub fn is_on_event_trigger_mode(&self) -> bool {
        self.trigger_mode.as_deref() == Some(Self::TRIGGER_MODE_ON_EVENT)
    }

    /// True when this sentry takes the cumulative multi-part evaluation
    /// branch of `AbstractEvaluationCriteriaOperation.evaluateCriteria`
    /// (Java L506-577), i.e. it is neither the single-onPart fast path
    /// (L475-490) nor the ifPart-only path (L492-504).
    pub fn is_multi_part(&self) -> bool {
        let on_part_count = self.plan_item_on_parts.len() + self.case_file_item_on_parts.len();
        on_part_count > 1 || (on_part_count == 1 && self.if_part.is_some())
    }

    pub fn with_case_file_item_on_part(
        mut self,
        case_file_item_on_part: CmmnCaseFileItemOnPart,
    ) -> Self {
        self.case_file_item_on_parts.push(case_file_item_on_part);
        self
    }

    pub fn with_if_part(mut self, expression: impl Into<String>) -> Self {
        let expression = expression.into();
        self.if_part = CmmnSentryIfPartExpression::parse(&expression)
            .map(Some)
            .unwrap_or_else(|e| {
                tracing::warn!(error = %e, expression = %expression, "CMMN ifPart expression parse failed, ignoring");
                None
            });
        self
    }

    /// Evaluate this sentry against a single lifecycle event.
    ///
    /// Mirrors the single-event branch of
    /// `AbstractEvaluationCriteriaOperation.evaluateCriteria`
    /// (`modules/flowable-cmmn-engine/src/main/java/org/flowable/cmmn/engine/impl/agenda/operation/AbstractEvaluationCriteriaOperation.java`,
    /// L466-582): all `plan_item_on_parts` must match the event
    /// (`source_ref` + `standard_event`), and if an `ifPart` is present
    /// its expression must evaluate to true against the supplied
    /// variable context.
    ///
    /// The `case_file_item_on_parts` branch participates in the same
    /// AND-grouping: any case-file onPart whose `case_file_item_ref` or
    /// `standard_event` differs from the event is treated as
    /// unsatisfied and the sentry fires false.
    ///
    /// Cumulative / multi-event sentry evaluation (Java L506-577) is
    /// out of scope for C1: that path requires `SentryPartInstance`
    /// persistence, which lives in the C2 plan-item state machine.
    pub fn evaluate_for_event(
        &self,
        event: &SentryLifecycleEvent,
        ctx: &dyn SentryVariableContext,
    ) -> bool {
        for on_part in &self.plan_item_on_parts {
            if on_part.source_ref != event.source_id
                || on_part.standard_event != event.standard_event
            {
                return false;
            }
        }
        for on_part in &self.case_file_item_on_parts {
            if on_part.case_file_item_ref != event.source_id
                || on_part.standard_event != event.standard_event
            {
                return false;
            }
        }
        match &self.if_part {
            Some(if_part) => if_part.evaluate(ctx).unwrap_or(false),
            None => true,
        }
    }

    /// True when this sentry has at least one onPart or a non-empty
    /// ifPart — i.e. there is a condition that can fire. Mirrors the
    /// Java check used by
    /// `AbstractEvaluationCriteriaOperation.evaluateCriteria` (the
    /// `criteria` list is iterated only when the sentry has any
    /// defined parts).
    pub fn has_parts(&self) -> bool {
        !self.plan_item_on_parts.is_empty()
            || !self.case_file_item_on_parts.is_empty()
            || self.if_part.is_some()
    }
}

/// Lifecycle event consumed by [`CmmnSentry::evaluate_for_event`].
///
/// Java reference: `PlanItemLifeCycleEvent` and the matching
/// `SentryOnPart.standardEvent` strings. `source_id` is the id of the
/// plan item or case file item that produced the event;
/// `standard_event` is the CMMN standard event name
/// (`complete` / `occur` / `terminate` / `enable` / `disable` /
/// `start` / `exit` for plan items; `create` / `update` / `delete` /
/// `complete` for case file items).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SentryLifecycleEvent {
    pub source_id: String,
    pub standard_event: String,
}

impl SentryLifecycleEvent {
    pub fn new(source_id: impl Into<String>, standard_event: impl Into<String>) -> Self {
        Self {
            source_id: source_id.into(),
            standard_event: standard_event.into(),
        }
    }
}

/// Variable context supplied to sentry `ifPart` evaluation.
///
/// Mirrors the read-only `VariableContainer` view Java passes to
/// `Expression.getValue` from
/// `AbstractEvaluationCriteriaOperation.evaluateSentryIfPart`
/// (L717-739). The trait is intentionally minimal — only the
/// operations the CMMN ifPart grammar needs (variable lookup by
/// name) are exposed.
pub trait SentryVariableContext {
    fn get(&self, name: &str) -> Option<&Value>;
}

/// Ergonomic in-memory variable context backed by a
/// `serde_json::Map<String, Value>`. CMMN ifParts see these values
/// under their variable names.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SentryVariableMap {
    variables: Map<String, Value>,
}

impl SentryVariableMap {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, name: impl Into<String>, value: Value) {
        self.variables.insert(name.into(), value);
    }

    pub fn from_pairs<I, K, V>(pairs: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<Value>,
    {
        let mut map = Self::new();
        for (k, v) in pairs {
            map.insert(k, v.into());
        }
        map
    }
}

impl SentryVariableContext for SentryVariableMap {
    fn get(&self, name: &str) -> Option<&Value> {
        self.variables.get(name)
    }
}

impl SentryVariableContext for Map<String, Value> {
    fn get(&self, name: &str) -> Option<&Value> {
        Map::get(self, name)
    }
}

impl<T: SentryVariableContext + ?Sized> SentryVariableContext for &T {
    fn get(&self, name: &str) -> Option<&Value> {
        (*self).get(name)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum CmmnSentryIfPartExpression {
    Comparison(CmmnSentryIfPartCondition),
    Logical {
        operator: CmmnSentryIfPartLogicalOperator,
        operands: Vec<CmmnSentryIfPartExpression>,
    },
    Not {
        operand: Box<CmmnSentryIfPartExpression>,
    },
    Empty {
        variable_name: String,
    },
    Contains {
        collection_variable_name: String,
        value: CmmnSentryIfPartLiteral,
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
    Size {
        collection_variable_name: String,
        operator: CmmnSentryIfPartOperator,
        literal: CmmnSentryIfPartLiteral,
    },
    Length {
        variable_name: String,
        operator: CmmnSentryIfPartOperator,
        literal: CmmnSentryIfPartLiteral,
    },
    MethodCall {
        object: Option<String>,
        method: String,
        args: Vec<CmmnSentryIfPartExpression>,
    },
    Arithmetic {
        left: Box<CmmnSentryIfPartExpression>,
        operator: String,
        right: Box<CmmnSentryIfPartExpression>,
    },
    Ternary {
        condition: Box<CmmnSentryIfPartExpression>,
        true_expr: Box<CmmnSentryIfPartExpression>,
        false_expr: Box<CmmnSentryIfPartExpression>,
    },
    PropertyAccess {
        object: Box<CmmnSentryIfPartExpression>,
        property: String,
    },
    IndexAccess {
        object: Box<CmmnSentryIfPartExpression>,
        index: Box<CmmnSentryIfPartExpression>,
    },
    Literal(CmmnSentryIfPartLiteral),
}

impl CmmnSentryIfPartExpression {
    pub fn parse(expression: &str) -> Result<Self, String> {
        flowable_cmmn_model::parse_sentry_if_part_expression(expression).map(Self::from)
    }

    /// Evaluate this expression against a sentry variable context.
    ///
    /// Mirrors the read-only evaluation Java performs inside
    /// `AbstractEvaluationCriteriaOperation.evaluateSentryIfPart`
    /// (`modules/flowable-cmmn-engine/src/main/java/org/flowable/cmmn/engine/impl/agenda/operation/AbstractEvaluationCriteriaOperation.java`,
    /// L717-739) where the ifPart is passed to
    /// `Expression.getValue(variableContainer)` and the resulting
    /// `Boolean` decides whether the sentry fires.
    ///
    /// Returns `Err` when a structural element references an
    /// evaluation mode not yet supported in C1 (e.g. arithmetic over
    /// non-literal operands, method calls). Callers that need the
    /// strict "sentry fires" answer should treat the error as
    /// "sentry not satisfied" — matching Java's "if condition throws,
    /// log + propagate" path which a future C2/C3 will surface
    /// through the agenda.
    pub fn evaluate(&self, ctx: &dyn SentryVariableContext) -> Result<bool, String> {
        match self {
            Self::Literal(literal) => Ok(literal.truthy(ctx)),
            Self::Comparison(condition) => condition.evaluate(ctx),
            Self::Logical { operator, operands } => {
                let mut iter = operands.iter();
                let Some(first) = iter.next() else {
                    return Ok(false);
                };
                let mut acc = first.evaluate(ctx)?;
                for operand in iter {
                    let value = operand.evaluate(ctx)?;
                    acc = match operator {
                        CmmnSentryIfPartLogicalOperator::And => acc && value,
                        CmmnSentryIfPartLogicalOperator::Or => acc || value,
                    };
                }
                Ok(acc)
            }
            Self::Not { operand } => Ok(!operand.evaluate(ctx)?),
            Self::Empty { variable_name } => Ok(match ctx.get(variable_name) {
                None | Some(Value::Null) => true,
                Some(Value::String(s)) => s.is_empty(),
                Some(Value::Array(arr)) => arr.is_empty(),
                Some(Value::Object(obj)) => obj.is_empty(),
                Some(_) => false,
            }),
            Self::Contains {
                collection_variable_name,
                value,
                expected,
            } => {
                let collection = ctx.get(collection_variable_name);
                let matched = match collection {
                    Some(Value::Array(items)) => {
                        items.iter().any(|item| value.equals_value(item, ctx))
                    }
                    Some(Value::String(s)) => {
                        if let Some(needle) = value.as_string(ctx) {
                            s.contains(&needle)
                        } else {
                            false
                        }
                    }
                    _ => false,
                };
                Ok(matched == *expected)
            }
            Self::StartsWith {
                variable_name,
                prefix,
            } => {
                let s = ctx.get(variable_name).and_then(Value::as_str);
                Ok(s.map(|s| s.starts_with(prefix.as_str())).unwrap_or(false))
            }
            Self::EndsWith {
                variable_name,
                suffix,
            } => {
                let s = ctx.get(variable_name).and_then(Value::as_str);
                Ok(s.map(|s| s.ends_with(suffix.as_str())).unwrap_or(false))
            }
            Self::Matches {
                variable_name,
                regex,
            } => {
                let s = ctx.get(variable_name).and_then(Value::as_str);
                let re = match regex::Regex::new(regex) {
                    Ok(re) => re,
                    Err(error) => return Err(format!("invalid regex in sentry ifPart: {}", error)),
                };
                Ok(s.map(|s| re.is_match(s)).unwrap_or(false))
            }
            Self::Size {
                collection_variable_name,
                operator,
                literal,
            } => {
                let size: Option<i64> = match ctx.get(collection_variable_name) {
                    Some(Value::Array(arr)) => Some(arr.len() as i64),
                    Some(Value::String(s)) => Some(s.chars().count() as i64),
                    Some(Value::Object(obj)) => Some(obj.len() as i64),
                    _ => None,
                };
                match size {
                    Some(value) => literal.apply_numeric(*operator, value),
                    None => Ok(false),
                }
            }
            Self::Length {
                variable_name,
                operator,
                literal,
            } => {
                let size: Option<i64> = match ctx.get(variable_name) {
                    Some(Value::String(s)) => Some(s.chars().count() as i64),
                    Some(Value::Array(arr)) => Some(arr.len() as i64),
                    _ => None,
                };
                match size {
                    Some(value) => literal.apply_numeric(*operator, value),
                    None => Ok(false),
                }
            }
            Self::MethodCall { .. } => {
                Err("C1 sentry ifPart does not yet support method calls".to_string())
            }
            Self::Arithmetic { .. } => {
                Err("C1 sentry ifPart does not yet support arithmetic expressions".to_string())
            }
            Self::Ternary { .. } => {
                Err("C1 sentry ifPart does not yet support ternary expressions".to_string())
            }
            Self::PropertyAccess { .. } | Self::IndexAccess { .. } => {
                Err("C1 sentry ifPart does not yet support property/index access".to_string())
            }
        }
    }
}

impl From<SharedSentryIfPartExpression> for CmmnSentryIfPartExpression {
    fn from(expression: SharedSentryIfPartExpression) -> Self {
        match expression {
            SharedSentryIfPartExpression::Comparison(condition) => {
                let operator = match condition.operator {
                    flowable_cmmn_model::SentryIfPartOperator::Equal => {
                        CmmnSentryIfPartOperator::Equal
                    }
                    flowable_cmmn_model::SentryIfPartOperator::NotEqual => {
                        CmmnSentryIfPartOperator::NotEqual
                    }
                    flowable_cmmn_model::SentryIfPartOperator::GreaterThan => {
                        CmmnSentryIfPartOperator::GreaterThan
                    }
                    flowable_cmmn_model::SentryIfPartOperator::GreaterThanOrEqual => {
                        CmmnSentryIfPartOperator::GreaterThanOrEqual
                    }
                    flowable_cmmn_model::SentryIfPartOperator::LessThan => {
                        CmmnSentryIfPartOperator::LessThan
                    }
                    flowable_cmmn_model::SentryIfPartOperator::LessThanOrEqual => {
                        CmmnSentryIfPartOperator::LessThanOrEqual
                    }
                };
                let literal = match condition.literal {
                    flowable_cmmn_model::SentryIfPartLiteral::Boolean(value) => {
                        CmmnSentryIfPartLiteral::Boolean(value)
                    }
                    flowable_cmmn_model::SentryIfPartLiteral::String(value) => {
                        CmmnSentryIfPartLiteral::String(value)
                    }
                    flowable_cmmn_model::SentryIfPartLiteral::Number(value) => {
                        CmmnSentryIfPartLiteral::Number(value)
                    }
                    flowable_cmmn_model::SentryIfPartLiteral::Null => CmmnSentryIfPartLiteral::Null,
                    flowable_cmmn_model::SentryIfPartLiteral::Variable(value) => {
                        CmmnSentryIfPartLiteral::Variable(value)
                    }
                };
                // The parser represents `size(x)` / `length(x)` as a
                // `Comparison` whose `variable_name` is the
                // string `"size(x)"` / `"length(x)"` (see
                // `flowable_cmmn_model::expression_to_string` for
                // `MethodCall`). Lift that back into the dedicated
                // `Size` / `Length` variants so the evaluator can
                // compute the collection/character length before
                // applying the operator. C1 only supports simple
                // identifier arguments to these helpers.
                if let Some(stripped) = condition
                    .variable_name
                    .strip_prefix("size(")
                    .and_then(|s| s.strip_suffix(')'))
                {
                    return Self::Size {
                        collection_variable_name: stripped.to_string(),
                        operator,
                        literal,
                    };
                }
                if let Some(stripped) = condition
                    .variable_name
                    .strip_prefix("length(")
                    .and_then(|s| s.strip_suffix(')'))
                {
                    return Self::Length {
                        variable_name: stripped.to_string(),
                        operator,
                        literal,
                    };
                }
                Self::Comparison(CmmnSentryIfPartCondition {
                    variable_name: condition.variable_name,
                    operator,
                    literal,
                })
            }
            SharedSentryIfPartExpression::Logical { operator, operands } => Self::Logical {
                operator: match operator {
                    flowable_cmmn_model::SentryIfPartLogicalOperator::And => {
                        CmmnSentryIfPartLogicalOperator::And
                    }
                    flowable_cmmn_model::SentryIfPartLogicalOperator::Or => {
                        CmmnSentryIfPartLogicalOperator::Or
                    }
                },
                operands: operands.into_iter().map(Self::from).collect(),
            },
            SharedSentryIfPartExpression::Not { operand } => Self::Not {
                operand: Box::new(Self::from(*operand)),
            },
            SharedSentryIfPartExpression::Empty { variable_name } => Self::Empty { variable_name },
            SharedSentryIfPartExpression::Contains {
                collection_variable_name,
                value,
                expected,
            } => Self::Contains {
                collection_variable_name,
                value: match value {
                    flowable_cmmn_model::SentryIfPartLiteral::Boolean(value) => {
                        CmmnSentryIfPartLiteral::Boolean(value)
                    }
                    flowable_cmmn_model::SentryIfPartLiteral::String(value) => {
                        CmmnSentryIfPartLiteral::String(value)
                    }
                    flowable_cmmn_model::SentryIfPartLiteral::Number(value) => {
                        CmmnSentryIfPartLiteral::Number(value)
                    }
                    flowable_cmmn_model::SentryIfPartLiteral::Null => CmmnSentryIfPartLiteral::Null,
                    flowable_cmmn_model::SentryIfPartLiteral::Variable(value) => {
                        CmmnSentryIfPartLiteral::Variable(value)
                    }
                },
                expected,
            },
            SharedSentryIfPartExpression::StartsWith {
                variable_name,
                prefix,
            } => Self::StartsWith {
                variable_name,
                prefix,
            },
            SharedSentryIfPartExpression::EndsWith {
                variable_name,
                suffix,
            } => Self::EndsWith {
                variable_name,
                suffix,
            },
            SharedSentryIfPartExpression::Matches {
                variable_name,
                regex,
            } => Self::Matches {
                variable_name,
                regex,
            },
            SharedSentryIfPartExpression::MethodCall {
                object,
                method,
                args,
            } => Self::MethodCall {
                object,
                method,
                args: args.into_iter().map(Self::from).collect(),
            },
            SharedSentryIfPartExpression::Arithmetic {
                left,
                operator,
                right,
            } => Self::Arithmetic {
                left: Box::new(Self::from(*left)),
                operator,
                right: Box::new(Self::from(*right)),
            },
            SharedSentryIfPartExpression::Ternary {
                condition,
                true_expr,
                false_expr,
            } => Self::Ternary {
                condition: Box::new(Self::from(*condition)),
                true_expr: Box::new(Self::from(*true_expr)),
                false_expr: Box::new(Self::from(*false_expr)),
            },
            SharedSentryIfPartExpression::PropertyAccess { object, property } => {
                Self::PropertyAccess {
                    object: Box::new(Self::from(*object)),
                    property,
                }
            }
            SharedSentryIfPartExpression::IndexAccess { object, index } => Self::IndexAccess {
                object: Box::new(Self::from(*object)),
                index: Box::new(Self::from(*index)),
            },
            SharedSentryIfPartExpression::Literal(lit) => Self::Literal(match lit {
                flowable_cmmn_model::SentryIfPartLiteral::Boolean(b) => {
                    CmmnSentryIfPartLiteral::Boolean(b)
                }
                flowable_cmmn_model::SentryIfPartLiteral::String(s) => {
                    CmmnSentryIfPartLiteral::String(s)
                }
                flowable_cmmn_model::SentryIfPartLiteral::Number(n) => {
                    CmmnSentryIfPartLiteral::Number(n)
                }
                flowable_cmmn_model::SentryIfPartLiteral::Null => CmmnSentryIfPartLiteral::Null,
                flowable_cmmn_model::SentryIfPartLiteral::Variable(v) => {
                    CmmnSentryIfPartLiteral::Variable(v)
                }
            }),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CmmnSentryIfPartCondition {
    pub variable_name: String,
    pub operator: CmmnSentryIfPartOperator,
    pub literal: CmmnSentryIfPartLiteral,
}

impl CmmnSentryIfPartCondition {
    fn evaluate(&self, ctx: &dyn SentryVariableContext) -> Result<bool, String> {
        let actual = ctx.get(&self.variable_name);
        self.literal.apply(actual, self.operator, ctx)
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum CmmnSentryIfPartOperator {
    Equal,
    NotEqual,
    GreaterThan,
    GreaterThanOrEqual,
    LessThan,
    LessThanOrEqual,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum CmmnSentryIfPartLogicalOperator {
    And,
    Or,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum CmmnSentryIfPartLiteral {
    Boolean(bool),
    String(String),
    Number(String),
    Null,
    Variable(String),
}

impl CmmnSentryIfPartLiteral {
    /// Resolve the literal's value. `Variable(name)` looks up the
    /// variable context; the other variants are self-contained.
    fn resolve<'a>(
        &'a self,
        ctx: &'a dyn SentryVariableContext,
    ) -> Option<CmmnResolvedLiteral<'a>> {
        match self {
            Self::Boolean(value) => Some(CmmnResolvedLiteral::Owned(Value::Bool(*value))),
            Self::String(value) => Some(CmmnResolvedLiteral::Owned(Value::String(value.clone()))),
            Self::Number(value) => parse_number_literal(value)
                .map(CmmnResolvedLiteral::Owned)
                .or(Some(CmmnResolvedLiteral::Owned(Value::Null))),
            Self::Null => Some(CmmnResolvedLiteral::Owned(Value::Null)),
            Self::Variable(name) => ctx.get(name).map(CmmnResolvedLiteral::Borrowed),
        }
    }

    /// Java-style truthiness used for bare-literal ifPart
    /// expressions (e.g. `<ifPart>` resolving to a Boolean
    /// variable).
    pub(crate) fn truthy(&self, ctx: &dyn SentryVariableContext) -> bool {
        let Some(resolved) = self.resolve(ctx) else {
            return false;
        };
        resolved.truthy()
    }

    /// Equality comparison against a JSON value.
    fn equals_value(&self, candidate: &Value, ctx: &dyn SentryVariableContext) -> bool {
        let Some(resolved) = self.resolve(ctx) else {
            return false;
        };
        resolved.equals(candidate)
    }

    /// String view used by `Contains` on `String` collections.
    fn as_string(&self, ctx: &dyn SentryVariableContext) -> Option<String> {
        let resolved = self.resolve(ctx)?;
        match resolved.into_owned() {
            Value::String(s) => Some(s),
            other => serde_json::to_string(&other).ok(),
        }
    }

    /// Numeric comparison: literal must resolve to a number; the
    /// supplied `actual` is the LHS.
    fn apply_numeric(
        &self,
        operator: CmmnSentryIfPartOperator,
        actual: i64,
    ) -> Result<bool, String> {
        let lhs = match self {
            Self::Number(value) => match value.parse::<i64>() {
                Ok(parsed) => parsed,
                Err(_) => {
                    return Err(format!(
                        "Size/Length literal must be a number, got '{}'",
                        value
                    ));
                }
            },
            _ => return Err("Size/Length literal must be a numeric literal".to_string()),
        };
        Ok(match operator {
            CmmnSentryIfPartOperator::Equal => actual == lhs,
            CmmnSentryIfPartOperator::NotEqual => actual != lhs,
            CmmnSentryIfPartOperator::GreaterThan => actual > lhs,
            CmmnSentryIfPartOperator::GreaterThanOrEqual => actual >= lhs,
            CmmnSentryIfPartOperator::LessThan => actual < lhs,
            CmmnSentryIfPartOperator::LessThanOrEqual => actual <= lhs,
        })
    }

    /// Generic comparison: the literal's resolved value is compared
    /// against `actual` using the CMMN operator. The comparison
    /// tolerates cross-type equality (string vs number) and JSON null
    /// matching Java `Expression.isEqual`.
    fn apply(
        &self,
        actual: Option<&Value>,
        operator: CmmnSentryIfPartOperator,
        ctx: &dyn SentryVariableContext,
    ) -> Result<bool, String> {
        let resolved = self.resolve(ctx);
        match operator {
            CmmnSentryIfPartOperator::Equal | CmmnSentryIfPartOperator::NotEqual => {
                let cmp = match (resolved, actual) {
                    (Some(lhs), Some(rhs)) => lhs.equals(rhs),
                    (Some(lhs), None) => lhs.equals(&Value::Null),
                    (None, Some(_)) => false,
                    (None, None) => true,
                };
                Ok(if matches!(operator, CmmnSentryIfPartOperator::NotEqual) {
                    !cmp
                } else {
                    cmp
                })
            }
            CmmnSentryIfPartOperator::GreaterThan
            | CmmnSentryIfPartOperator::GreaterThanOrEqual
            | CmmnSentryIfPartOperator::LessThan
            | CmmnSentryIfPartOperator::LessThanOrEqual => {
                // Mirrors Java's `Expression.compareTo` on the resolved
                // values. The condition is `actual <op> literal`, so the
                // variable's runtime value is the LHS and the parsed
                // literal is the RHS. We coerce both sides to f64 and let
                // NaN short-circuit to false.
                let lhs = actual.and_then(value_as_f64).ok_or_else(|| {
                    "ordering comparisons require comparable numeric values".to_string()
                })?;
                let rhs = resolved
                    .as_ref()
                    .and_then(CmmnResolvedLiteral::as_f64)
                    .ok_or_else(|| {
                        "ordering comparisons require comparable numeric values".to_string()
                    })?;
                Ok(match operator {
                    CmmnSentryIfPartOperator::GreaterThan => lhs > rhs,
                    CmmnSentryIfPartOperator::GreaterThanOrEqual => lhs >= rhs,
                    CmmnSentryIfPartOperator::LessThan => lhs < rhs,
                    CmmnSentryIfPartOperator::LessThanOrEqual => lhs <= rhs,
                    _ => false,
                })
            }
        }
    }
}

/// Borrowed-or-owned resolved value, used to avoid cloning during
/// sentry evaluation.
enum CmmnResolvedLiteral<'a> {
    Borrowed(&'a Value),
    Owned(Value),
}

impl<'a> CmmnResolvedLiteral<'a> {
    fn into_owned(self) -> Value {
        match self {
            Self::Borrowed(value) => value.clone(),
            Self::Owned(value) => value,
        }
    }

    fn truthy(&self) -> bool {
        match self.as_ref() {
            Value::Null => false,
            Value::Bool(b) => *b,
            Value::String(s) => !s.is_empty(),
            Value::Array(arr) => !arr.is_empty(),
            Value::Object(obj) => !obj.is_empty(),
            Value::Number(n) => n.as_f64().map(|f| f != 0.0).unwrap_or(false),
        }
    }

    fn equals(&self, other: &Value) -> bool {
        let lhs = self.as_ref();
        if lhs == other {
            return true;
        }
        // Numeric coercion: compare number-to-number even when the
        // JSON representation differs (integer vs float).
        match (lhs, other) {
            (Value::Number(a), Value::Number(b)) => a
                .as_f64()
                .zip(b.as_f64())
                .map(|(x, y)| x == y)
                .unwrap_or(false),
            // Stringification tolerance: a numeric literal on one side
            // and a JSON string on the other compare equal if the
            // string parses to the same number. Mirrors Java
            // `Expression.isEqual` behaviour.
            (Value::String(s), Value::Number(n)) => s
                .parse::<f64>()
                .ok()
                .zip(n.as_f64())
                .map(|(a, b)| a == b)
                .unwrap_or(false),
            (Value::Number(n), Value::String(s)) => n
                .as_f64()
                .zip(s.parse::<f64>().ok())
                .map(|(a, b)| a == b)
                .unwrap_or(false),
            _ => false,
        }
    }

    fn as_f64(&self) -> Option<f64> {
        value_as_f64(self.as_ref())
    }
}

fn value_as_f64(value: &Value) -> Option<f64> {
    match value {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.parse::<f64>().ok(),
        Value::Bool(true) => Some(1.0),
        Value::Bool(false) => Some(0.0),
        _ => None,
    }
}

impl<'a> AsRef<Value> for CmmnResolvedLiteral<'a> {
    fn as_ref(&self) -> &Value {
        match self {
            Self::Borrowed(value) => value,
            Self::Owned(value) => value,
        }
    }
}

fn parse_number_literal(value: &str) -> Option<Value> {
    if let Ok(int_value) = value.parse::<i64>() {
        return Some(Value::Number(int_value.into()));
    }
    if let Ok(float_value) = value.parse::<f64>() {
        if let Some(number) = serde_json::Number::from_f64(float_value) {
            return Some(Value::Number(number));
        }
    }
    None
}

pub(crate) fn is_supported_number_literal(value: &str) -> bool {
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

impl CmmnPlanItem {
    pub fn new(id: impl Into<String>, definition_ref: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            definition_ref: definition_ref.into(),
            name: None,
            entry_criterion_ids: Vec::new(),
            exit_criterion_ids: Vec::new(),
            manual_activation_rule: None,
            repetition_rule: None,
            required_rule: None,
            parent_completion_rule: None,
            completion_neutral_rule: None,
        }
    }

    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    pub fn with_entry_criterion(mut self, criterion_id: impl Into<String>) -> Self {
        self.entry_criterion_ids.push(criterion_id.into());
        self
    }

    pub fn with_exit_criterion(mut self, criterion_id: impl Into<String>) -> Self {
        self.exit_criterion_ids.push(criterion_id.into());
        self
    }

    pub fn with_manual_activation_rule(mut self, expression: impl Into<String>) -> Self {
        let expression = expression.into();
        self.manual_activation_rule = CmmnSentryIfPartExpression::parse(&expression)
            .map(Some)
            .unwrap_or_else(|e| {
                tracing::warn!(error = %e, expression = %expression, "CMMN manualActivationRule parse failed, ignoring");
                None
            });
        self
    }

    pub fn with_repetition_rule(mut self, expression: impl Into<String>) -> Self {
        let expression = expression.into();
        self.repetition_rule = CmmnSentryIfPartExpression::parse(&expression)
            .map(Some)
            .unwrap_or_else(|e| {
                tracing::warn!(error = %e, expression = %expression, "CMMN repetitionRule parse failed, ignoring");
                None
            });
        self
    }

    pub fn with_required_rule(mut self, expression: impl Into<String>) -> Self {
        let expression = expression.into();
        self.required_rule = CmmnSentryIfPartExpression::parse(&expression)
            .map(Some)
            .unwrap_or_else(|e| {
                tracing::warn!(error = %e, expression = %expression, "CMMN requiredRule parse failed, ignoring");
                None
            });
        self
    }

    // Java parity: ParentCompletionRule.getType() constant (default|ignore|ignoreIfAvailable|
    // ignoreIfAvailableOrEnabled|ignoreAfterFirstCompletion|
    // ignoreAfterFirstCompletionIfAvailableOrEnabled).
    pub fn with_parent_completion_rule(mut self, rule_type: impl Into<String>) -> Self {
        self.parent_completion_rule = Some(rule_type.into());
        self
    }

    // Java parity: completionNeutralRule condition.
    pub fn with_completion_neutral_rule(mut self, expression: impl Into<String>) -> Self {
        let expression = expression.into();
        self.completion_neutral_rule = CmmnSentryIfPartExpression::parse(&expression)
            .map(Some)
            .unwrap_or_else(|e| {
                tracing::warn!(error = %e, expression = %expression, "CMMN completionNeutralRule parse failed, ignoring");
                None
            });
        self
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CmmnHumanTask {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub form_key: Option<String>,
    // Java parity: Task.java:20 — blocking defaults to true; a non-blocking human
    // task never creates a task entry and completes its plan item immediately
    // (HumanTaskActivityBehavior.java:173-177).
    #[serde(default = "default_human_task_blocking")]
    pub blocking: bool,
    // Java parity: HumanTask.java:31 — variable receiving the created task id
    // (HumanTaskActivityBehavior.java:456-464).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id_variable_name: Option<String>,
    // Java parity: HumanTask.java:32 — variable receiving the completing user on
    // the complete transition (HumanTaskActivityBehavior.java:498-507).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_completer_variable_name: Option<String>,
    // Java parity: HumanTask.java:23-34 — flowable extension attributes applied
    // to the created task entity (HumanTaskActivityBehavior.java:107-147).
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
}

fn default_human_task_blocking() -> bool {
    true
}

impl CmmnHumanTask {
    pub fn new(id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            description: None,
            form_key: None,
            blocking: default_human_task_blocking(),
            task_id_variable_name: None,
            task_completer_variable_name: None,
            assignee: None,
            owner: None,
            priority: None,
            due_date: None,
            category: None,
            candidate_users: Vec::new(),
            candidate_groups: Vec::new(),
        }
    }

    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    pub fn with_form_key(mut self, form_key: impl Into<String>) -> Self {
        self.form_key = Some(form_key.into());
        self
    }

    pub fn with_blocking(mut self, blocking: bool) -> Self {
        self.blocking = blocking;
        self
    }

    pub fn with_task_id_variable_name(mut self, variable_name: impl Into<String>) -> Self {
        self.task_id_variable_name = Some(variable_name.into());
        self
    }

    pub fn with_task_completer_variable_name(mut self, variable_name: impl Into<String>) -> Self {
        self.task_completer_variable_name = Some(variable_name.into());
        self
    }

    pub fn with_assignee(mut self, assignee: impl Into<String>) -> Self {
        self.assignee = Some(assignee.into());
        self
    }

    pub fn with_owner(mut self, owner: impl Into<String>) -> Self {
        self.owner = Some(owner.into());
        self
    }

    pub fn with_priority(mut self, priority: impl Into<String>) -> Self {
        self.priority = Some(priority.into());
        self
    }

    pub fn with_due_date(mut self, due_date: impl Into<String>) -> Self {
        self.due_date = Some(due_date.into());
        self
    }

    pub fn with_category(mut self, category: impl Into<String>) -> Self {
        self.category = Some(category.into());
        self
    }

    pub fn with_candidate_users(mut self, candidate_users: Vec<String>) -> Self {
        self.candidate_users = candidate_users;
        self
    }

    pub fn with_candidate_groups(mut self, candidate_groups: Vec<String>) -> Self {
        self.candidate_groups = candidate_groups;
        self
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CmmnDecisionTask {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decision_ref: Option<String>,
}

impl CmmnDecisionTask {
    pub fn new(id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            decision_ref: None,
        }
    }

    pub fn with_decision_ref(mut self, decision_ref: impl Into<String>) -> Self {
        self.decision_ref = Some(decision_ref.into());
        self
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CmmnMilestone {
    pub id: String,
    pub name: String,
    // Java parity: Milestone.java:22-23 (milestoneVariable + businessStatus). Both are literal
    // values here: Java evaluates them as expressions on reach
    // (MilestoneActivityBehavior.java:47-61), but the Rust engine has no expression engine.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub milestone_variable: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub business_status: Option<String>,
}

impl CmmnMilestone {
    pub fn new(id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            milestone_variable: None,
            business_status: None,
        }
    }

    pub fn with_milestone_variable(mut self, milestone_variable: impl Into<String>) -> Self {
        self.milestone_variable = Some(milestone_variable.into());
        self
    }

    pub fn with_business_status(mut self, business_status: impl Into<String>) -> Self {
        self.business_status = Some(business_status.into());
        self
    }
}

/// Java `flowable:eventOutParameter` on an event-registry event listener
/// (`EventInstanceCmmnUtil.handleEventInstanceOutParameters`, EventInstanceCmmnUtil.java:46-68).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CmmnEventOutParameter {
    /// Payload field name (`source` attribute).
    pub source: String,
    /// Case variable name (`target` attribute).
    pub target: String,
    /// When true, Java sets a transient variable (not persisted). Rust currently
    /// maps non-transient only; transient out-params are skipped at apply time.
    #[serde(default)]
    pub is_transient: bool,
}

impl CmmnEventOutParameter {
    pub fn new(source: impl Into<String>, target: impl Into<String>) -> Self {
        Self {
            source: source.into(),
            target: target.into(),
            is_transient: false,
        }
    }

    pub fn with_transient(mut self, is_transient: bool) -> Self {
        self.is_transient = is_transient;
        self
    }
}

/// Java `flowable:eventCorrelationParameter` on an event-registry event listener
/// (`EventRegistryEventListenerActivityBehaviour.getCorrelationKey`, :156-188).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CmmnEventCorrelationParameter {
    pub name: String,
    /// Expression text (`value` attribute), e.g. `${customerIdVar}` or a literal.
    pub value: String,
}

impl CmmnEventCorrelationParameter {
    pub fn new(name: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            value: value.into(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CmmnEventListener {
    pub id: String,
    pub name: Option<String>,
    /// Subscription event type. For event-registry listeners this is the event
    /// definition key (`EventRegistryEventListenerActivityBehaviour.java:146`);
    /// for variable listeners the literal `"variable"`; for timer event listeners
    /// the internal marker `"timer"`.
    pub event_type: String,
    pub event_name: Option<String>,
    // Java parity: EventListener.java:20 availableConditionExpression - evaluated when the
    // listener would become available; only a Boolean true result makes it available
    // (AbstractEvaluationCriteriaOperation.java:584-604).
    // Stored as the raw expression text so `${…}` UEL and the CMMN if-part dialect can
    // both be evaluated at runtime (P69).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub available_condition: Option<String>,
    // Java parity: VariableEventListener.java:23-24 - variable listeners carry the watched
    // variable name in event_name (subscription eventName) and the change type here
    // (subscription configuration JSON, EvaluateVariableEventListenersOperation.java:80-95).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variable_change_type: Option<String>,
    // Java parity: TimerEventListener.java:20-30 timerExpression - the ISO-8601
    // duration / date / repetition expression (TimerExpressionXmlConverter.java:39-49).
    // `Some` marks this listener as a timerEventListener and switches the activation
    // path from event subscription to timer job scheduling
    // (TimerEventListenerActivityBehaviour.java:66-78).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timer_expression: Option<String>,
    /// Event-registry out-parameter mappings (`eventOutParameter` extensions).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub event_out_parameters: Vec<CmmnEventOutParameter>,
    /// Event-registry correlation parameters (`eventCorrelationParameter` extensions).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub event_correlation_parameters: Vec<CmmnEventCorrelationParameter>,
}

impl CmmnEventListener {
    // Java parity: CmmnXmlConstants / VariableEventListenerActivityBehaviour - subscription
    // eventType used for variable event listeners is the literal "variable"
    // (EvaluateVariableEventListenersOperation.java:59).
    pub const EVENT_TYPE_VARIABLE: &'static str = "variable";
    /// Internal marker event_type for timer event listeners (Java `TimerEventListener`
    /// has no eventType attribute; TimerEventListenerXmlConverter.java:36-44).
    pub const EVENT_TYPE_TIMER: &'static str = "timer";
    // Java parity: VariableListenerEventDefinition change type constants
    // (EvaluateVariableEventListenersOperation.java:81,93-95).
    pub const CHANGE_TYPE_ALL: &'static str = "all";
    pub const CHANGE_TYPE_CREATE: &'static str = "create";
    pub const CHANGE_TYPE_UPDATE: &'static str = "update";
    pub const CHANGE_TYPE_UPDATE_CREATE: &'static str = "update-create";

    pub fn new(id: impl Into<String>, event_type: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: None,
            event_type: event_type.into(),
            event_name: None,
            available_condition: None,
            variable_change_type: None,
            timer_expression: None,
            event_out_parameters: Vec::new(),
            event_correlation_parameters: Vec::new(),
        }
    }

    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    pub fn with_event_name(mut self, event_name: impl Into<String>) -> Self {
        self.event_name = Some(event_name.into());
        self
    }

    pub fn with_available_condition(mut self, condition: impl Into<String>) -> Self {
        self.available_condition = Some(condition.into());
        self
    }

    /// Marks this listener as a timerEventListener with the given ISO-8601 timer
    /// expression (TimerEventListenerActivityBehaviour.java:96-152).
    pub fn with_timer_expression(mut self, expression: impl Into<String>) -> Self {
        self.timer_expression = Some(expression.into());
        self
    }

    /// Java parity: `TimerEventListener extends EventListener` — a listener is a timer
    /// event listener when it carries a timerExpression (TimerEventListener.java:20).
    pub fn is_timer(&self) -> bool {
        self.timer_expression.is_some()
    }

    pub fn with_variable_change_type(mut self, change_type: impl Into<String>) -> Self {
        self.variable_change_type = Some(change_type.into());
        self
    }

    pub fn with_event_out_parameter(mut self, parameter: CmmnEventOutParameter) -> Self {
        self.event_out_parameters.push(parameter);
        self
    }

    pub fn with_event_correlation_parameter(
        mut self,
        parameter: CmmnEventCorrelationParameter,
    ) -> Self {
        self.event_correlation_parameters.push(parameter);
        self
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CmmnStage {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub plan_items: Vec<CmmnPlanItem>,
    pub stages: Vec<CmmnStage>,
    pub human_tasks: Vec<CmmnHumanTask>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub decision_tasks: Vec<CmmnDecisionTask>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub process_tasks: Vec<CmmnProcessTask>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub case_tasks: Vec<CmmnCaseTask>,
    pub milestones: Vec<CmmnMilestone>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub event_listeners: Vec<CmmnEventListener>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sentries: Vec<CmmnSentry>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub planning_tables: Vec<CmmnPlanningTable>,
    // Java parity: Stage.java:29-30 (autoComplete flag + autoCompleteCondition expression)
    #[serde(default)]
    pub auto_complete: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_complete_condition: Option<CmmnSentryIfPartExpression>,
}

impl CmmnStage {
    pub fn new(id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            description: None,
            plan_items: Vec::new(),
            stages: Vec::new(),
            human_tasks: Vec::new(),
            decision_tasks: Vec::new(),
            process_tasks: Vec::new(),
            case_tasks: Vec::new(),
            milestones: Vec::new(),
            event_listeners: Vec::new(),
            sentries: Vec::new(),
            planning_tables: Vec::new(),
            auto_complete: false,
            auto_complete_condition: None,
        }
    }

    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    pub fn with_plan_item(mut self, plan_item: CmmnPlanItem) -> Self {
        self.plan_items.push(plan_item);
        self
    }

    pub fn with_stage(mut self, stage: CmmnStage) -> Self {
        self.stages.push(stage);
        self
    }

    pub fn with_human_task(mut self, human_task: CmmnHumanTask) -> Self {
        self.human_tasks.push(human_task);
        self
    }

    pub fn with_decision_task(mut self, decision_task: CmmnDecisionTask) -> Self {
        self.decision_tasks.push(decision_task);
        self
    }

    pub fn with_process_task(mut self, process_task: CmmnProcessTask) -> Self {
        self.process_tasks.push(process_task);
        self
    }

    pub fn with_case_task(mut self, case_task: CmmnCaseTask) -> Self {
        self.case_tasks.push(case_task);
        self
    }

    pub fn with_milestone(mut self, milestone: CmmnMilestone) -> Self {
        self.milestones.push(milestone);
        self
    }

    pub fn with_event_listener(mut self, event_listener: CmmnEventListener) -> Self {
        self.event_listeners.push(event_listener);
        self
    }

    pub fn with_sentry(mut self, sentry: CmmnSentry) -> Self {
        self.sentries.push(sentry);
        self
    }

    pub fn with_planning_table(mut self, mut planning_table: CmmnPlanningTable) -> Self {
        for discretionary_item in &mut planning_table.discretionary_items {
            if discretionary_item.planning_table.is_none() {
                discretionary_item.planning_table = Some(planning_table.id.clone());
            }
            if discretionary_item.parent_stage_id.is_none() {
                discretionary_item.parent_stage_id = Some(self.id.clone());
            }
        }
        self.planning_tables.push(planning_table);
        self
    }

    pub fn with_auto_complete(mut self, auto_complete: bool) -> Self {
        self.auto_complete = auto_complete;
        self
    }

    pub fn with_auto_complete_condition(mut self, expression: impl Into<String>) -> Self {
        let expression = expression.into();
        self.auto_complete_condition = CmmnSentryIfPartExpression::parse(&expression)
            .map(Some)
            .unwrap_or_else(|e| {
                tracing::warn!(error = %e, expression = %expression, "CMMN autoComplete condition parse failed, ignoring");
                None
            });
        self
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CmmnCasePlanModel {
    pub id: String,
    pub name: String,
    pub plan_items: Vec<CmmnPlanItem>,
    pub stages: Vec<CmmnStage>,
    pub human_tasks: Vec<CmmnHumanTask>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub decision_tasks: Vec<CmmnDecisionTask>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub process_tasks: Vec<CmmnProcessTask>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub case_tasks: Vec<CmmnCaseTask>,
    pub milestones: Vec<CmmnMilestone>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub event_listeners: Vec<CmmnEventListener>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sentries: Vec<CmmnSentry>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub planning_tables: Vec<CmmnPlanningTable>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_form_key: Option<String>,
    // Java parity: Stage.java:29-30 - case plan model is a Stage in Java, shares autoComplete
    #[serde(default)]
    pub auto_complete: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_complete_condition: Option<CmmnSentryIfPartExpression>,
}

impl CmmnCasePlanModel {
    pub fn new(id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            plan_items: Vec::new(),
            stages: Vec::new(),
            human_tasks: Vec::new(),
            decision_tasks: Vec::new(),
            process_tasks: Vec::new(),
            case_tasks: Vec::new(),
            milestones: Vec::new(),
            event_listeners: Vec::new(),
            sentries: Vec::new(),
            planning_tables: Vec::new(),
            start_form_key: None,
            auto_complete: false,
            auto_complete_condition: None,
        }
    }

    pub fn with_plan_item(mut self, plan_item: CmmnPlanItem) -> Self {
        self.plan_items.push(plan_item);
        self
    }

    pub fn with_stage(mut self, stage: CmmnStage) -> Self {
        self.stages.push(stage);
        self
    }

    pub fn with_human_task(mut self, human_task: CmmnHumanTask) -> Self {
        self.human_tasks.push(human_task);
        self
    }

    pub fn with_decision_task(mut self, decision_task: CmmnDecisionTask) -> Self {
        self.decision_tasks.push(decision_task);
        self
    }

    pub fn with_process_task(mut self, process_task: CmmnProcessTask) -> Self {
        self.process_tasks.push(process_task);
        self
    }

    pub fn with_case_task(mut self, case_task: CmmnCaseTask) -> Self {
        self.case_tasks.push(case_task);
        self
    }

    pub fn with_milestone(mut self, milestone: CmmnMilestone) -> Self {
        self.milestones.push(milestone);
        self
    }

    pub fn with_event_listener(mut self, event_listener: CmmnEventListener) -> Self {
        self.event_listeners.push(event_listener);
        self
    }

    pub fn with_sentry(mut self, sentry: CmmnSentry) -> Self {
        self.sentries.push(sentry);
        self
    }

    pub fn with_planning_table(mut self, mut planning_table: CmmnPlanningTable) -> Self {
        for discretionary_item in &mut planning_table.discretionary_items {
            if discretionary_item.planning_table.is_none() {
                discretionary_item.planning_table = Some(planning_table.id.clone());
            }
        }
        self.planning_tables.push(planning_table);
        self
    }

    pub fn with_start_form_key(mut self, form_key: impl Into<String>) -> Self {
        self.start_form_key = Some(form_key.into());
        self
    }

    pub fn with_auto_complete(mut self, auto_complete: bool) -> Self {
        self.auto_complete = auto_complete;
        self
    }

    pub fn with_auto_complete_condition(mut self, expression: impl Into<String>) -> Self {
        let expression = expression.into();
        self.auto_complete_condition = CmmnSentryIfPartExpression::parse(&expression)
            .map(Some)
            .unwrap_or_else(|e| {
                tracing::warn!(error = %e, expression = %expression, "CMMN autoComplete condition parse failed, ignoring");
                None
            });
        self
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CmmnCase {
    pub id: String,
    pub key: String,
    pub name: String,
    pub description: Option<String>,
    pub case_plan_model: CmmnCasePlanModel,
    /// Java `Case.getLifecycleListeners()` (Case.java:20 `implements HasLifecycleListeners`) —
    /// `flowable:caseLifecycleListener` entries, fired on case instance state transitions
    /// (CaseInstanceLifeCycleListenerUtil.java:41-48).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lifecycle_listeners: Vec<CmmnLifecycleListener>,
    /// `flowable:planItemLifecycleListener` entries, flattened by owning plan item definition
    /// id. Java stores them on each `PlanItemDefinition`
    /// (PlanItemDefinition.java:21 `implements HasLifecycleListeners`) and looks them up through
    /// `planItemInstance.getPlanItemDefinition().getLifecycleListeners()`
    /// (CmmnListenerNotificationHelper.java:111). A flat map keyed by definition id is
    /// equivalent here because the converter already rejects duplicate definition ids within a
    /// case (`validate_nested_uniqueness`), and `CmmnPlanItemInstance` carries
    /// `plan_item_definition_id`.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub plan_item_lifecycle_listeners: BTreeMap<String, Vec<CmmnLifecycleListener>>,
    /// Java `Case.startEventType` (ExtensionElementsXMLConverter.java:410-411).
    /// Non-empty → definition-level event-registry start subscription candidate
    /// (CmmnDeployer.java:212-222).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_event_type: Option<String>,
    /// Java `startEventCorrelationConfiguration` text: `storeAsUniqueReferenceId` /
    /// `manualSubscription` (CmmnXmlConstants.java:228-230).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_correlation_configuration: Option<String>,
    /// Case-level static `eventCorrelationParameter` (name, value) pairs
    /// (CmmnCorrelationUtil.java:29-46). Used to build subscription `configuration`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub start_correlation_parameters: Vec<CmmnEventCorrelationParameter>,
}

/// Java `CmmnXmlConstants.START_EVENT_CORRELATION_STORE_AS_UNIQUE_REFERENCE_ID`.
pub const START_EVENT_CORRELATION_STORE_AS_UNIQUE_REFERENCE_ID: &str = "storeAsUniqueReferenceId";
/// Java `CmmnXmlConstants.START_EVENT_CORRELATION_MANUAL`.
pub const START_EVENT_CORRELATION_MANUAL: &str = "manualSubscription";
/// Java `ReferenceTypes.EVENT_CASE` (ReferenceTypes.java:30).
pub const REFERENCE_TYPE_EVENT_CASE: &str = "event-to-cmmn-1.1-case";

/// Java `FlowableListener` as used for CMMN lifecycle listeners
/// (FlowableListener.java:20-93, built by ListenerXmlConverterUtil.java:28-53).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CmmnLifecycleListener {
    pub implementation_type: CmmnListenerImplementationType,
    pub implementation: String,
    /// Java `sourceState` / `targetState`, holding the **lowercase** CMMN spec state values
    /// (`CaseInstanceState.java:28-33`, `PlanItemInstanceState.java`). Absent means "match any
    /// state" (CaseInstanceLifeCycleListenerUtil.java:76-78).
    pub source_state: Option<String>,
    pub target_state: Option<String>,
    /// Java `event` attribute (ListenerXmlConverterUtil.java:44). Carried for fidelity; the
    /// lifecycle listener path filters on source/target state only.
    pub event: Option<String>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum CmmnListenerImplementationType {
    Class,
    Expression,
    DelegateExpression,
}

impl From<flowable_cmmn_model::ListenerImplementationType> for CmmnListenerImplementationType {
    fn from(value: flowable_cmmn_model::ListenerImplementationType) -> Self {
        match value {
            flowable_cmmn_model::ListenerImplementationType::Class => Self::Class,
            flowable_cmmn_model::ListenerImplementationType::Expression => Self::Expression,
            flowable_cmmn_model::ListenerImplementationType::DelegateExpression => {
                Self::DelegateExpression
            }
        }
    }
}

impl From<flowable_cmmn_model::FlowableListener> for CmmnLifecycleListener {
    fn from(listener: flowable_cmmn_model::FlowableListener) -> Self {
        Self {
            implementation_type: listener.implementation_type.into(),
            implementation: listener.implementation,
            source_state: listener.source_state,
            target_state: listener.target_state,
            event: listener.event,
        }
    }
}

impl CmmnLifecycleListener {
    /// Java `CaseInstanceLifeCycleListenerUtil.stateMatches`
    /// (CaseInstanceLifeCycleListenerUtil.java:76-78) and its plan-item twin
    /// (CmmnListenerNotificationHelper.java:158-160): an empty expected state matches any
    /// actual state.
    ///
    /// `actual` is a Rust-convention UPPERCASE state while the XML holds the lowercase CMMN
    /// spec value, so the comparison is case-insensitive.
    fn state_matches(expected: Option<&str>, actual: &str) -> bool {
        match expected {
            None => true,
            Some(expected) => expected.eq_ignore_ascii_case(actual),
        }
    }

    /// Java's combined filter: `stateMatches(sourceState, oldState) && stateMatches(targetState,
    /// newState)` (CaseInstanceLifeCycleListenerUtil.java:48,
    /// CmmnListenerNotificationHelper.java:115).
    pub(crate) fn matches(&self, old_state: &str, new_state: &str) -> bool {
        Self::state_matches(self.source_state.as_deref(), old_state)
            && Self::state_matches(self.target_state.as_deref(), new_state)
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct CmmnCaseFileModel {
    pub item_definitions: Vec<CmmnCaseFileItemDefinition>,
    pub items: Vec<CmmnCaseFileItemDefinitionNode>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CmmnCaseFileItemDefinition {
    pub id: String,
    pub name: Option<String>,
    pub definition_type: Option<String>,
    pub structure_ref: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CmmnCaseFileItemDefinitionNode {
    pub id: String,
    pub name: Option<String>,
    pub definition_ref: String,
    pub children: Vec<CmmnCaseFileItemDefinitionNode>,
}

impl CmmnCase {
    pub fn new(
        id: impl Into<String>,
        key: impl Into<String>,
        name: impl Into<String>,
        case_plan_model: CmmnCasePlanModel,
    ) -> Self {
        Self {
            id: id.into(),
            key: key.into(),
            name: name.into(),
            description: None,
            case_plan_model,
            lifecycle_listeners: Vec::new(),
            plan_item_lifecycle_listeners: BTreeMap::new(),
            start_event_type: None,
            start_correlation_configuration: None,
            start_correlation_parameters: Vec::new(),
        }
    }

    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Registers a `flowable:caseLifecycleListener` (Case.java:20).
    pub fn with_lifecycle_listener(mut self, listener: CmmnLifecycleListener) -> Self {
        self.lifecycle_listeners.push(listener);
        self
    }

    /// Registers a `flowable:planItemLifecycleListener` against a plan item definition id
    /// (PlanItemDefinition.java:21).
    pub fn with_plan_item_lifecycle_listener(
        mut self,
        plan_item_definition_id: impl Into<String>,
        listener: CmmnLifecycleListener,
    ) -> Self {
        self.plan_item_lifecycle_listeners
            .entry(plan_item_definition_id.into())
            .or_default()
            .push(listener);
        self
    }

    /// The plan item listeners registered for one definition id, in declaration order.
    pub(crate) fn plan_item_listeners(&self, plan_item_definition_id: &str) -> &[CmmnLifecycleListener] {
        self.plan_item_lifecycle_listeners
            .get(plan_item_definition_id)
            .map(Vec::as_slice)
            .unwrap_or_default()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CmmnModel {
    pub cases: Vec<CmmnCase>,
}

impl CmmnModel {
    pub fn new(cases: Vec<CmmnCase>) -> Self {
        Self { cases }
    }
}

impl From<flowable_cmmn_model::CmmnDefinitions> for CmmnModel {
    fn from(definitions: flowable_cmmn_model::CmmnDefinitions) -> Self {
        Self::new(definitions.cases.into_iter().map(CmmnCase::from).collect())
    }
}

impl From<flowable_cmmn_model::Case> for CmmnCase {
    fn from(case: flowable_cmmn_model::Case) -> Self {
        // Java keeps the plan item listeners on each PlanItemDefinition; flatten them by
        // definition id here (see the field doc on `plan_item_lifecycle_listeners`).
        let mut plan_item_lifecycle_listeners = BTreeMap::new();
        collect_plan_model_lifecycle_listeners(
            &case.case_plan_model,
            &mut plan_item_lifecycle_listeners,
        );

        let mut converted = Self::new(
            case.id.clone(),
            case.id,
            case.name.clone().unwrap_or_else(|| "Case".to_string()),
            CmmnCasePlanModel::from(case.case_plan_model),
        );
        converted.lifecycle_listeners = case
            .lifecycle_listeners
            .into_iter()
            .map(CmmnLifecycleListener::from)
            .collect();
        converted.plan_item_lifecycle_listeners = plan_item_lifecycle_listeners;
        // P136: case-level event-registry start subscription fields
        // (ExtensionElementsXMLConverter.java:396-411; CmmnXmlConstants.java:224-230).
        converted.start_event_type = case.start_event_type;
        converted.start_correlation_configuration = case.start_correlation_configuration;
        converted.start_correlation_parameters = case
            .start_correlation_parameters
            .into_iter()
            .map(|p| CmmnEventCorrelationParameter::new(p.name, p.value))
            .collect();
        converted
    }
}

fn insert_lifecycle_listeners(
    definition_id: &str,
    listeners: &[flowable_cmmn_model::FlowableListener],
    out: &mut BTreeMap<String, Vec<CmmnLifecycleListener>>,
) {
    if listeners.is_empty() {
        return;
    }
    out.entry(definition_id.to_string())
        .or_default()
        .extend(listeners.iter().cloned().map(CmmnLifecycleListener::from));
}

/// The case plan model is itself a `Stage` in Java, so it can own
/// `planItemLifecycleListener` entries just like a nested stage
/// (Case.getPlanModel() returns a Stage extends PlanItemDefinition).
fn collect_plan_model_lifecycle_listeners(
    plan_model: &flowable_cmmn_model::CasePlanModel,
    out: &mut BTreeMap<String, Vec<CmmnLifecycleListener>>,
) {
    insert_lifecycle_listeners(&plan_model.id, &plan_model.lifecycle_listeners, out);
    for human_task in &plan_model.human_tasks {
        insert_lifecycle_listeners(&human_task.id, &human_task.lifecycle_listeners, out);
    }
    for decision_task in &plan_model.decision_tasks {
        insert_lifecycle_listeners(&decision_task.id, &decision_task.lifecycle_listeners, out);
    }
    for process_task in &plan_model.process_tasks {
        insert_lifecycle_listeners(&process_task.id, &process_task.lifecycle_listeners, out);
    }
    for case_task in &plan_model.case_tasks {
        insert_lifecycle_listeners(&case_task.id, &case_task.lifecycle_listeners, out);
    }
    for milestone in &plan_model.milestones {
        insert_lifecycle_listeners(&milestone.id, &milestone.lifecycle_listeners, out);
    }
    for event_listener in &plan_model.event_listeners {
        insert_lifecycle_listeners(&event_listener.id, &event_listener.lifecycle_listeners, out);
    }
    for stage in &plan_model.stages {
        collect_stage_lifecycle_listeners(stage, out);
    }
}

fn collect_stage_lifecycle_listeners(
    stage: &flowable_cmmn_model::Stage,
    out: &mut BTreeMap<String, Vec<CmmnLifecycleListener>>,
) {
    insert_lifecycle_listeners(&stage.id, &stage.lifecycle_listeners, out);
    for human_task in &stage.human_tasks {
        insert_lifecycle_listeners(&human_task.id, &human_task.lifecycle_listeners, out);
    }
    for decision_task in &stage.decision_tasks {
        insert_lifecycle_listeners(&decision_task.id, &decision_task.lifecycle_listeners, out);
    }
    for process_task in &stage.process_tasks {
        insert_lifecycle_listeners(&process_task.id, &process_task.lifecycle_listeners, out);
    }
    for case_task in &stage.case_tasks {
        insert_lifecycle_listeners(&case_task.id, &case_task.lifecycle_listeners, out);
    }
    for milestone in &stage.milestones {
        insert_lifecycle_listeners(&milestone.id, &milestone.lifecycle_listeners, out);
    }
    for event_listener in &stage.event_listeners {
        insert_lifecycle_listeners(&event_listener.id, &event_listener.lifecycle_listeners, out);
    }
    for child_stage in &stage.stages {
        collect_stage_lifecycle_listeners(child_stage, out);
    }
}

impl From<flowable_cmmn_model::CaseFileModel> for CmmnCaseFileModel {
    fn from(model: flowable_cmmn_model::CaseFileModel) -> Self {
        Self {
            item_definitions: model
                .item_definitions
                .into_iter()
                .map(|definition| CmmnCaseFileItemDefinition {
                    id: definition.id,
                    name: definition.name,
                    definition_type: definition.definition_type,
                    structure_ref: definition.structure_ref,
                })
                .collect(),
            items: model
                .items
                .into_iter()
                .map(CmmnCaseFileItemDefinitionNode::from)
                .collect(),
        }
    }
}

impl From<flowable_cmmn_model::CaseFileItem> for CmmnCaseFileItemDefinitionNode {
    fn from(item: flowable_cmmn_model::CaseFileItem) -> Self {
        Self {
            id: item.id,
            name: item.name,
            definition_ref: item.definition_ref,
            children: item.children.into_iter().map(Self::from).collect(),
        }
    }
}

impl From<flowable_cmmn_model::CasePlanModel> for CmmnCasePlanModel {
    fn from(model: flowable_cmmn_model::CasePlanModel) -> Self {
        let mut converted = Self::new(
            model.id,
            model.name.unwrap_or_else(|| "Case Plan Model".to_string()),
        );
        if let Some(form_key) = model.form_key {
            converted = converted.with_start_form_key(form_key);
        }
        for plan_item in model.plan_items {
            converted = converted.with_plan_item(CmmnPlanItem::from(plan_item));
        }
        for stage in model.stages {
            converted = converted.with_stage(CmmnStage::from(stage));
        }
        for human_task in model.human_tasks {
            converted = converted.with_human_task(CmmnHumanTask::from(human_task));
        }
        for decision_task in model.decision_tasks {
            converted = converted.with_decision_task(CmmnDecisionTask::from(decision_task));
        }
        for process_task in model.process_tasks {
            converted = converted.with_process_task(CmmnProcessTask::from(process_task));
        }
        for case_task in model.case_tasks {
            converted = converted.with_case_task(CmmnCaseTask::from(case_task));
        }
        for milestone in model.milestones {
            converted = converted.with_milestone(CmmnMilestone::from(milestone));
        }
        for event_listener in model.event_listeners {
            converted = converted.with_event_listener(CmmnEventListener::from(event_listener));
        }
        for sentry in model.sentries {
            converted = converted.with_sentry(CmmnSentry::from(sentry));
        }
        for planning_table in model.planning_tables {
            converted = converted.with_planning_table(CmmnPlanningTable::from(planning_table));
        }
        converted.auto_complete = model.auto_complete;
        converted
    }
}

impl From<flowable_cmmn_model::Stage> for CmmnStage {
    fn from(stage: flowable_cmmn_model::Stage) -> Self {
        let mut converted = Self::new(stage.id, stage.name.unwrap_or_else(|| "Stage".to_string()));
        for plan_item in stage.plan_items {
            converted = converted.with_plan_item(CmmnPlanItem::from(plan_item));
        }
        for child_stage in stage.stages {
            converted = converted.with_stage(CmmnStage::from(child_stage));
        }
        for human_task in stage.human_tasks {
            converted = converted.with_human_task(CmmnHumanTask::from(human_task));
        }
        for decision_task in stage.decision_tasks {
            converted = converted.with_decision_task(CmmnDecisionTask::from(decision_task));
        }
        for process_task in stage.process_tasks {
            converted = converted.with_process_task(CmmnProcessTask::from(process_task));
        }
        for case_task in stage.case_tasks {
            converted = converted.with_case_task(CmmnCaseTask::from(case_task));
        }
        for milestone in stage.milestones {
            converted = converted.with_milestone(CmmnMilestone::from(milestone));
        }
        for event_listener in stage.event_listeners {
            converted = converted.with_event_listener(CmmnEventListener::from(event_listener));
        }
        for sentry in stage.sentries {
            converted = converted.with_sentry(CmmnSentry::from(sentry));
        }
        for planning_table in stage.planning_tables {
            converted = converted.with_planning_table(CmmnPlanningTable::from(planning_table));
        }
        converted.auto_complete = stage.auto_complete;
        converted
    }
}

impl From<flowable_cmmn_model::PlanItem> for CmmnPlanItem {
    fn from(plan_item: flowable_cmmn_model::PlanItem) -> Self {
        let mut converted = Self::new(plan_item.id, plan_item.definition_ref);
        if let Some(name) = plan_item.name {
            converted = converted.with_name(name);
        }
        for entry_criterion in plan_item.entry_criteria {
            converted = converted.with_entry_criterion(entry_criterion.sentry_ref);
        }
        for exit_criterion in plan_item.exit_criteria {
            converted = converted.with_exit_criterion(exit_criterion.sentry_ref);
        }
        converted.manual_activation_rule = plan_item
            .manual_activation_rule
            .map(CmmnSentryIfPartExpression::from);
        converted.repetition_rule = plan_item
            .repetition_rule
            .map(CmmnSentryIfPartExpression::from);
        converted.required_rule = plan_item
            .required_rule
            .map(CmmnSentryIfPartExpression::from);
        converted.parent_completion_rule = plan_item.parent_completion_rule;
        converted.completion_neutral_rule = plan_item
            .completion_neutral_rule
            .map(CmmnSentryIfPartExpression::from);
        converted
    }
}

impl From<flowable_cmmn_model::PlanningTable> for CmmnPlanningTable {
    fn from(planning_table: flowable_cmmn_model::PlanningTable) -> Self {
        let mut converted = Self::new(
            planning_table.id,
            planning_table
                .name
                .unwrap_or_else(|| "Planning Table".to_string()),
        );
        for discretionary_item in planning_table.discretionary_items {
            converted =
                converted.with_discretionary_item(CmmnDiscretionaryItem::from(discretionary_item));
        }
        converted
    }
}

impl From<flowable_cmmn_model::DiscretionaryItem> for CmmnDiscretionaryItem {
    fn from(discretionary_item: flowable_cmmn_model::DiscretionaryItem) -> Self {
        Self::new(
            discretionary_item.id,
            discretionary_item
                .name
                .unwrap_or_else(|| "Discretionary Item".to_string()),
            discretionary_item.definition_ref,
        )
    }
}

impl From<flowable_cmmn_model::HumanTask> for CmmnHumanTask {
    fn from(task: flowable_cmmn_model::HumanTask) -> Self {
        let mut converted = Self::new(
            task.id,
            task.name.unwrap_or_else(|| "Human Task".to_string()),
        )
        .with_blocking(task.is_blocking);
        if let Some(form_key) = task.form_key {
            converted = converted.with_form_key(form_key);
        }
        if let Some(assignee) = task.assignee {
            converted = converted.with_assignee(assignee);
        }
        if let Some(owner) = task.owner {
            converted = converted.with_owner(owner);
        }
        if let Some(priority) = task.priority {
            converted = converted.with_priority(priority);
        }
        if let Some(due_date) = task.due_date {
            converted = converted.with_due_date(due_date);
        }
        if let Some(category) = task.category {
            converted = converted.with_category(category);
        }
        if !task.candidate_users.is_empty() {
            converted = converted.with_candidate_users(task.candidate_users);
        }
        if !task.candidate_groups.is_empty() {
            converted = converted.with_candidate_groups(task.candidate_groups);
        }
        if let Some(variable_name) = task.task_id_variable_name {
            converted = converted.with_task_id_variable_name(variable_name);
        }
        if let Some(variable_name) = task.task_completer_variable_name {
            converted = converted.with_task_completer_variable_name(variable_name);
        }
        converted
    }
}

impl From<flowable_cmmn_model::DecisionTask> for CmmnDecisionTask {
    fn from(task: flowable_cmmn_model::DecisionTask) -> Self {
        let mut converted = Self::new(
            task.id,
            task.name.unwrap_or_else(|| "Decision Task".to_string()),
        );
        if let Some(decision_ref) = task.decision_ref {
            converted = converted.with_decision_ref(decision_ref);
        }
        converted
    }
}

impl From<flowable_cmmn_model::ProcessTask> for CmmnProcessTask {
    fn from(task: flowable_cmmn_model::ProcessTask) -> Self {
        let mut converted = Self::new(
            task.id,
            task.name.unwrap_or_else(|| "Process Task".to_string()),
        )
        .with_blocking(task.is_blocking);
        if let Some(process_ref) = task.process_ref {
            converted = converted.with_process_ref(process_ref);
        }
        converted
    }
}

impl From<flowable_cmmn_model::CaseTask> for CmmnCaseTask {
    fn from(task: flowable_cmmn_model::CaseTask) -> Self {
        let mut converted = Self::new(
            task.id,
            task.name.unwrap_or_else(|| "Case Task".to_string()),
        )
        .with_blocking(task.is_blocking);
        if let Some(case_ref) = task.case_ref {
            converted = converted.with_case_ref(case_ref);
        }
        converted
    }
}

impl From<flowable_cmmn_model::Milestone> for CmmnMilestone {
    fn from(milestone: flowable_cmmn_model::Milestone) -> Self {
        Self::new(
            milestone.id,
            milestone.name.unwrap_or_else(|| "Milestone".to_string()),
        )
    }
}

impl From<flowable_cmmn_model::EventListener> for CmmnEventListener {
    fn from(listener: flowable_cmmn_model::EventListener) -> Self {
        let mut converted = Self::new(listener.id, listener.event_type);
        if let Some(name) = listener.name {
            converted = converted.with_name(name);
        }
        if let Some(event_name) = listener.event_name {
            converted = converted.with_event_name(event_name);
        }
        if let Some(condition) = listener.available_condition {
            converted = converted.with_available_condition(condition);
        }
        if let Some(expression) = listener.timer_expression {
            converted = converted.with_timer_expression(expression);
        }
        converted
    }
}

impl From<flowable_cmmn_model::Sentry> for CmmnSentry {
    fn from(sentry: flowable_cmmn_model::Sentry) -> Self {
        Self {
            id: sentry.id,
            plan_item_on_parts: sentry
                .plan_item_on_parts
                .into_iter()
                .map(CmmnPlanItemOnPart::from)
                .collect(),
            // P118: converter now parses caseFileItemOnPart (CMMN11CaseModel.xsd:1027-1042);
            // previously this field was always dropped here, so runtime never saw XML-sourced
            // case-file onParts even when the model carried them.
            case_file_item_on_parts: sentry
                .case_file_item_on_parts
                .into_iter()
                .map(CmmnCaseFileItemOnPart::from)
                .collect(),
            if_part: sentry.if_part.map(CmmnSentryIfPartExpression::from),
            // The converter model carries no triggerMode; `None` keeps the
            // Java default trigger mode (Sentry.java:30-32).
            trigger_mode: None,
        }
    }
}

impl From<flowable_cmmn_model::PlanItemOnPart> for CmmnPlanItemOnPart {
    fn from(on_part: flowable_cmmn_model::PlanItemOnPart) -> Self {
        Self::new(on_part.id, on_part.source_ref, on_part.standard_event)
    }
}

impl From<flowable_cmmn_model::CaseFileItemOnPart> for CmmnCaseFileItemOnPart {
    fn from(on_part: flowable_cmmn_model::CaseFileItemOnPart) -> Self {
        // Converter maps XML sourceRef → case_file_item_ref
        // (CMMN11CaseModel.xsd:1034-1039).
        Self::new(on_part.id, on_part.case_file_item_ref, on_part.standard_event)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CmmnDeploymentResource {
    pub resource_name: String,
    pub model: CmmnModel,
    #[serde(default)]
    pub resource_bytes: Vec<u8>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CmmnDeploymentRequest {
    pub name: Option<String>,
    pub category: Option<String>,
    pub key: Option<String>,
    pub tenant_id: Option<String>,
    pub parent_deployment_id: Option<String>,
    pub enable_duplicate_filtering: bool,
    pub validate_schema: bool,
    pub resources: Vec<CmmnDeploymentResource>,
}

impl CmmnDeploymentRequest {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: Some(name.into()),
            category: None,
            key: None,
            tenant_id: None,
            parent_deployment_id: None,
            enable_duplicate_filtering: false,
            validate_schema: true,
            resources: Vec::new(),
        }
    }

    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    pub fn with_category(mut self, category: impl Into<String>) -> Self {
        self.category = Some(category.into());
        self
    }

    pub fn with_key(mut self, key: impl Into<String>) -> Self {
        self.key = Some(key.into());
        self
    }

    pub fn with_tenant_id(mut self, tenant_id: impl Into<String>) -> Self {
        self.tenant_id = Some(tenant_id.into());
        self
    }

    pub fn with_parent_deployment_id(mut self, parent_deployment_id: impl Into<String>) -> Self {
        self.parent_deployment_id = Some(parent_deployment_id.into());
        self
    }

    pub fn with_resource(mut self, resource_name: impl Into<String>, model: CmmnModel) -> Self {
        self.resources.push(CmmnDeploymentResource {
            resource_name: resource_name.into(),
            model,
            resource_bytes: Vec::new(),
        });
        self
    }

    pub fn with_resource_bytes(
        mut self,
        resource_name: impl Into<String>,
        model: CmmnModel,
        resource_bytes: impl Into<Vec<u8>>,
    ) -> Self {
        self.resources.push(CmmnDeploymentResource {
            resource_name: resource_name.into(),
            model,
            resource_bytes: resource_bytes.into(),
        });
        self
    }

    pub fn enable_duplicate_filtering(mut self) -> Self {
        self.enable_duplicate_filtering = true;
        self
    }

    pub fn disable_schema_validation(mut self) -> Self {
        self.validate_schema = false;
        self
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CmmnDeployment {
    pub id: String,
    pub name: Option<String>,
    pub category: Option<String>,
    pub key: Option<String>,
    pub tenant_id: Option<String>,
    pub parent_deployment_id: Option<String>,
    pub resource_names: Vec<String>,
    pub deployed_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CmmnCaseDefinition {
    pub id: String,
    pub case_id: String,
    pub deployment_id: String,
    pub key: String,
    pub name: String,
    pub version: i32,
    pub category: Option<String>,
    pub tenant_id: Option<String>,
    pub resource_name: String,
    pub diagram_resource_name: Option<String>,
    pub model: CmmnCase,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CmmnCaseInstanceStartRequest {
    pub business_key: Option<String>,
    pub name: Option<String>,
    pub tenant_id: Option<String>,
    pub started_by: Option<String>,
    /// Java `CaseInstanceBuilderImpl.referenceId/referenceType`
    /// (`CaseInstanceBuilderImpl.java:45-46,191-198`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reference_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reference_type: Option<String>,
    pub variables: Value,
    /// Java CaseInstanceBuilder.transientVariables — visible to expression
    /// evaluation during start but never persisted (CaseInstanceHelperImpl.java:275).
    /// The Rust engine merges these into the variable scope for the initial
    /// activation and keeps them off the stored case instance.
    #[serde(default, skip_serializing_if = "is_empty_value")]
    pub transient_variables: Value,
    /// Java CaseInstanceBuilder.outcome — only used when completing through a
    /// start form (CaseInstanceHelperImpl.java:410); without a form engine it is
    /// accepted and dropped (same precedent as the P99 task outcome).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<String>,
    /// Java CaseInstanceBuilder.overrideCaseDefinitionTenantId — overrides the
    /// tenant of the created case instance (CaseInstanceHelperImpl.java:325-326);
    /// the definition lookup still uses `tenant_id`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub override_definition_tenant_id: Option<String>,
    /// Java CaseInstanceBuilder.callbackId — parent BPMN execution id for
    /// `EXECUTION_CHILD_CASE` (DefaultCaseInstanceService.java:74-75).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub callback_id: Option<String>,
    /// Java CaseInstanceBuilder.callbackType — `bpmn-2.0-to-cmmn-1.1-child-case`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub callback_type: Option<String>,
    /// Optional predefined case instance id (CaseInstanceService.generateNewCaseInstanceId).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub predefined_case_instance_id: Option<String>,
}

fn is_empty_value(value: &Value) -> bool {
    value.is_null() || value.as_object().is_some_and(Map::is_empty)
}

impl CmmnCaseInstanceStartRequest {
    pub fn new() -> Self {
        Self {
            business_key: None,
            name: None,
            tenant_id: None,
            started_by: None,
            reference_id: None,
            reference_type: None,
            variables: Value::Object(Map::new()),
            transient_variables: Value::Object(Map::new()),
            outcome: None,
            override_definition_tenant_id: None,
            callback_id: None,
            callback_type: None,
            predefined_case_instance_id: None,
        }
    }

    pub fn with_business_key(mut self, business_key: impl Into<String>) -> Self {
        self.business_key = Some(business_key.into());
        self
    }

    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    pub fn with_tenant_id(mut self, tenant_id: impl Into<String>) -> Self {
        self.tenant_id = Some(tenant_id.into());
        self
    }

    pub fn with_started_by(mut self, started_by: impl Into<String>) -> Self {
        self.started_by = Some(started_by.into());
        self
    }

    /// Mirrors Java `CaseInstanceBuilderImpl.referenceId`
    /// (`CaseInstanceBuilderImpl.java:191-194`).
    pub fn with_reference_id(mut self, reference_id: impl Into<String>) -> Self {
        self.reference_id = Some(reference_id.into());
        self
    }

    /// Mirrors Java `CaseInstanceBuilderImpl.referenceType`
    /// (`CaseInstanceBuilderImpl.java:196-200`).
    pub fn with_reference_type(mut self, reference_type: impl Into<String>) -> Self {
        self.reference_type = Some(reference_type.into());
        self
    }

    pub fn with_variables(mut self, variables: Value) -> Self {
        self.variables = variables;
        self
    }

    pub fn with_transient_variables(mut self, transient_variables: Value) -> Self {
        self.transient_variables = transient_variables;
        self
    }

    pub fn with_outcome(mut self, outcome: impl Into<String>) -> Self {
        self.outcome = Some(outcome.into());
        self
    }

    pub fn with_override_definition_tenant_id(mut self, tenant_id: impl Into<String>) -> Self {
        self.override_definition_tenant_id = Some(tenant_id.into());
        self
    }

    pub fn with_callback(mut self, callback_id: impl Into<String>, callback_type: impl Into<String>) -> Self {
        self.callback_id = Some(callback_id.into());
        self.callback_type = Some(callback_type.into());
        self
    }

    pub fn with_predefined_case_instance_id(mut self, id: impl Into<String>) -> Self {
        self.predefined_case_instance_id = Some(id.into());
        self
    }
}

impl Default for CmmnCaseInstanceStartRequest {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CmmnCaseInstance {
    pub id: String,
    pub case_definition_id: String,
    pub deployment_id: String,
    pub case_definition_key: String,
    pub case_definition_name: String,
    pub case_definition_version: i32,
    pub business_key: Option<String>,
    pub name: String,
    pub tenant_id: Option<String>,
    pub started_by: Option<String>,
    /// Java case reference metadata supplied by `CaseInstanceBuilderImpl`
    /// (`CaseInstanceBuilderImpl.java:45-46,191-198`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reference_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reference_type: Option<String>,
    pub started_at: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
    pub state: CmmnCaseInstanceState,
    // Java parity: CaseInstanceEntity businessStatus, updated when a milestone with a declared
    // businessStatus is reached (MilestoneActivityBehavior.java:55-61) or via
    // CmmnRuntimeService#updateBusinessStatus.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub business_status: Option<String>,
    pub variables: Map<String, Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub case_file_items: Vec<CmmnCaseFileItem>,
    /// Java CaseInstanceEntity.callbackId (parent BPMN execution id for child case).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub callback_id: Option<String>,
    /// Java CaseInstanceEntity.callbackType (`bpmn-2.0-to-cmmn-1.1-child-case`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub callback_type: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CmmnStageInstance {
    pub id: String,
    pub case_instance_id: String,
    pub case_definition_id: String,
    pub parent_stage_instance_id: Option<String>,
    pub plan_item_id: String,
    pub stage_definition_id: String,
    pub name: String,
    pub activated_at: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
    pub state: CmmnStageInstanceState,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CmmnStageOverview {
    pub id: String,
    pub name: String,
    pub current: bool,
    pub ended: bool,
    pub end_time: Option<DateTime<Utc>>,
}

/// P116: unified runtime plan item instance (Java `PlanItemInstanceEntity`,
/// ACT_CMMN_RU_PLAN_ITEM_INST). One row per stage / milestone / event listener
/// plan item instance, mirrored from the type-specific runtime tables so the
/// unified plan-item-instance query surface reads one table. Human-task plan
/// items stay backed by ACT_CMMN_HUMAN_TASK (`CmmnHumanTaskQuery`).
///
/// `plan_item_definition_type` matches Java's
/// `planItemDefinition.getClass().getSimpleName().toLowerCase()`
/// (`PlanItemInstanceEntityManagerImpl.java:94-99`): `stage`, `milestone`,
/// `eventlistener`, `humantask`, `timereventlistener`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CmmnPlanItemInstance {
    pub id: String,
    pub case_instance_id: String,
    pub case_definition_id: String,
    /// Parent stage plan item instance id (Java `stageInstanceId`). Null for the
    /// stage itself and for top-level plan items.
    pub stage_instance_id: Option<String>,
    /// Java `elementId` — the plan item id (`PlanItemInstanceEntityManagerImpl.java:92`).
    pub plan_item_id: String,
    pub plan_item_definition_id: String,
    /// Java `planItemDefinitionType` (`stage` / `milestone` / `eventlistener`).
    pub plan_item_definition_type: String,
    pub name: String,
    /// Rust-convention UPPERCASE state
    /// (`AVAILABLE`/`ENABLED`/`ACTIVE`/`COMPLETED`/`TERMINATED`/`DISABLED`).
    pub state: String,
    pub created_at: DateTime<Utc>,
    /// Java `PlanItemInstanceEntity.lastEnabledTime`, recorded by
    /// `EnablePlanItemInstanceOperation.java:44-51`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_enabled_at: Option<DateTime<Utc>>,
    pub ended_at: Option<DateTime<Utc>>,
    /// Java `occurredTime` — set for milestones (on occur) and event listeners
    /// (on trigger).
    pub occurred_at: Option<DateTime<Utc>>,
    pub assignee: Option<String>,
    pub tenant_id: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CmmnHumanTaskInstance {
    pub id: String,
    pub case_instance_id: String,
    pub case_definition_id: String,
    pub case_definition_key: String,
    pub stage_instance_id: Option<String>,
    pub plan_item_id: String,
    pub task_definition_id: String,
    pub name: String,
    pub activated_at: DateTime<Utc>,
    /// Java `PlanItemInstanceEntity.lastEnabledTime`, recorded by
    /// `EnablePlanItemInstanceOperation.java:44-51`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_enabled_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub completed_by: Option<String>,
    pub state: CmmnHumanTaskState,
    // Java parity: attributes copied from the HumanTask definition onto the
    // created TaskEntity (HumanTaskActivityBehavior.java:107-147).
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
    /// Java `TaskEntity.delegationState` (DelegateTaskCmd.java:38 / ResolveTaskCmd.java:55);
    /// serde default keeps rows written before the field existed loadable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delegation_state: Option<CmmnDelegationState>,
    /// Task-local variables: Java stores them on the TaskEntity's own scope
    /// (ACT_RU_VARIABLE with TASK_ID_ set, EXECUTION_ID_ null), separate from the
    /// case-scoped variables the task reads via its parent scope
    /// (DefaultCmmnTaskVariableScopeResolver.java:34-43). They shadow case
    /// variables on non-local reads (VariableScopeImpl.java:203-225 collectVariables
    /// merges parent first, then local) and are deleted with the task entity on
    /// completion/termination (CMMN TaskHelper.java:109-128).
    /// serde default keeps rows written before the field existed loadable.
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub task_local_variables: Map<String, Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CmmnHumanTaskCompletionRequest {
    pub completed_by: Option<String>,
    /// Java `TaskActionRequest.outcome` → `CompleteTaskWithFormCmd`; only used
    /// through the form engine (CompleteTaskWithFormCmd.java:131-132), so without
    /// a form it is accepted and dropped — faithful to Java's formInfo == null path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<String>,
    /// Completion variables with GLOBAL scope — Java writes them on the task
    /// entity, which for CMMN is case-scoped (CompleteTaskCmd.java:100-101), so
    /// they land on the case instance.
    #[serde(default)]
    pub variables: Vec<(String, Value)>,
}

impl CmmnHumanTaskCompletionRequest {
    pub fn new() -> Self {
        Self {
            completed_by: None,
            outcome: None,
            variables: Vec::new(),
        }
    }

    pub fn with_completed_by(mut self, completed_by: impl Into<String>) -> Self {
        self.completed_by = Some(completed_by.into());
        self
    }

    pub fn with_outcome(mut self, outcome: impl Into<String>) -> Self {
        self.outcome = Some(outcome.into());
        self
    }

    pub fn with_variables(mut self, variables: Vec<(String, Value)>) -> Self {
        self.variables = variables;
        self
    }
}

/// Field updates for a human task (Java `TaskBaseResource.populateTaskFromRequest`,
/// TaskBaseResource.java:91-127). Outer `Option` = field present in the request;
/// inner value applies — `None` clears the field ("explicit null clears").
///
/// `Option<Option<T>>` alone cannot distinguish a present `null` (a clear) from a
/// missing field (untouched) — serde maps JSON null to the outer `None` — so each
/// field uses a `deserialize_with` helper that always returns `Some(inner)` when
/// the field is present.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CmmnHumanTaskUpdate {
    #[serde(deserialize_with = "deserialize_clearable")]
    pub name: Option<Option<String>>,
    #[serde(deserialize_with = "deserialize_clearable")]
    pub assignee: Option<Option<String>>,
    #[serde(deserialize_with = "deserialize_clearable")]
    pub owner: Option<Option<String>>,
    /// Java TaskRequest.priority is an Integer (TaskBaseResource.java:110-112);
    /// Rust stores it as a String, so a JSON number or string is accepted.
    #[serde(deserialize_with = "deserialize_priority")]
    pub priority: Option<Option<String>>,
    #[serde(deserialize_with = "deserialize_clearable")]
    pub due_date: Option<Option<String>>,
    #[serde(deserialize_with = "deserialize_clearable")]
    pub category: Option<Option<String>>,
    /// Java TaskRequest.delegationState (TaskBaseResource.java:123-126).
    #[serde(
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_clearable_delegation"
    )]
    pub delegation_state: Option<Option<CmmnDelegationState>>,
}

/// `present` → `Some(value)` where JSON `null` becomes `Some(None)` (a clear).
fn deserialize_clearable<'de, D>(deserializer: D) -> Result<Option<Option<String>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Option::<String>::deserialize(deserializer)?;
    Ok(Some(value))
}

/// Accept a JSON number or string for `priority`, mapping `null` to a clear and
/// any other scalar to its string form.
fn deserialize_priority<'de, D>(deserializer: D) -> Result<Option<Option<String>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    // Option<Value>::deserialize: JSON null → None, present value → Some(v).
    let value = Option::<Value>::deserialize(deserializer)?;
    Ok(Some(match value {
        None => None,
        Some(Value::String(string)) => Some(string),
        Some(Value::Null) => None,
        Some(other) => Some(other.to_string()),
    }))
}

/// `present` → `Some(value)` for the delegation-state field (null → clear).
fn deserialize_clearable_delegation<'de, D>(
    deserializer: D,
) -> Result<Option<Option<CmmnDelegationState>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Option::<CmmnDelegationState>::deserialize(deserializer)?;
    Ok(Some(value))
}

impl Default for CmmnHumanTaskCompletionRequest {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CmmnHumanTaskCompletionResult {
    pub task: CmmnHumanTaskInstance,
    pub case_instance: CmmnCaseInstance,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CmmnHistoricCaseInstance {
    pub case_instance_id: String,
    pub case_definition_id: String,
    pub deployment_id: String,
    pub case_definition_key: String,
    pub case_definition_name: String,
    pub case_definition_version: i32,
    pub business_key: Option<String>,
    pub name: String,
    pub tenant_id: Option<String>,
    pub started_by: Option<String>,
    /// Java historic reference predicates map to `REFERENCE_ID_` / `REFERENCE_TYPE_`
    /// (`HistoricCaseInstance.xml:789-793`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reference_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reference_type: Option<String>,
    /// Java stores the authenticated finishing user in `END_USER_ID_`
    /// (`DefaultCmmnHistoryManager.java:89-90`). Rust has no thread-local
    /// authentication context, so only explicit engine callers populate it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_by: Option<String>,
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub state: CmmnCaseInstanceState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub business_status: Option<String>,
    /// Java `HistoricCaseInstanceEntity.callbackId` / `CALLBACK_ID_`.
    ///
    /// Kept optional and serde-defaulted so historic JSON written before P128
    /// remains readable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub callback_id: Option<String>,
    /// Java `HistoricCaseInstanceEntity.callbackType` / `CALLBACK_TYPE_`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub callback_type: Option<String>,
    pub variables: Map<String, Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub case_file_items: Vec<CmmnCaseFileItem>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CmmnHistoricHumanTaskInstance {
    pub task_id: String,
    pub case_instance_id: String,
    pub case_definition_id: String,
    pub case_definition_key: String,
    pub stage_instance_id: Option<String>,
    pub plan_item_id: String,
    pub task_definition_id: String,
    pub name: String,
    pub activated_at: DateTime<Utc>,
    /// Historic copy of Java `PlanItemInstanceEntity.lastEnabledTime`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_enabled_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub completed_by: Option<String>,
    pub state: CmmnHumanTaskState,
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
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CmmnHistoricMilestoneInstance {
    pub id: String,
    pub case_instance_id: String,
    pub case_definition_id: String,
    pub case_definition_key: String,
    pub milestone_id: String,
    pub name: String,
    pub tenant_id: Option<String>,
    pub time: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CmmnIdentityLink {
    pub id: String,
    pub scope_type: String,
    pub scope_id: String,
    pub link_type: String,
    pub user_id: Option<String>,
    pub group_id: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum CmmnJobFamily {
    Executable,
    Timer,
    Deadletter,
    History,
    Suspended,
}

impl CmmnJobFamily {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Executable => "executable",
            Self::Timer => "timer",
            Self::Deadletter => "deadletter",
            Self::History => "history",
            Self::Suspended => "suspended",
        }
    }
}

/// Scope type constant matching Java `ScopeTypes.CMMN`.
pub const CMMN_SCOPE_TYPE: &str = "cmmn";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CmmnJob {
    pub id: String,
    pub family: CmmnJobFamily,
    /// Java `AbstractJobEntity.jobType`; unlike Rust's table-family discriminator this
    /// survives a move to deadletter and identifies history-origin jobs there.
    #[serde(default)]
    pub job_type: Option<String>,
    pub state: String,
    pub scope_id: Option<String>,
    pub sub_scope_id: Option<String>,
    pub scope_definition_id: Option<String>,
    pub element_id: Option<String>,
    pub tenant_id: Option<String>,
    pub due_date: Option<DateTime<Utc>>,
    pub lock_owner: Option<String>,
    pub retries: i32,
    pub exception_message: Option<String>,
    pub exception_stacktrace: Option<String>,
    pub created_at: DateTime<Utc>,
    /// Job handler TYPE string (Java `JobEntity.jobHandlerType`).
    #[serde(default)]
    pub handler_type: Option<String>,
    /// Handler configuration payload (JSON string or free-form).
    #[serde(default)]
    pub configuration: Option<String>,
    /// Java `Job.scopeType`. Defaults to CMMN for jobs created in this engine.
    /// Missing in older DATA_ blobs is treated as CMMN by the parent resolver.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope_type: Option<String>,
}

impl CmmnJob {
    pub fn new(id: impl Into<String>, family: CmmnJobFamily) -> Self {
        Self {
            id: id.into(),
            // Java DefaultJobManager.copyJobInfo preserves jobType across family moves
            // (DefaultJobManager.java:769-792). Callers can refine message/external-worker
            // types, while history receives the required literal automatically.
            job_type: Some(family.as_str().to_string()),
            state: family.as_str().to_string(),
            family,
            scope_id: None,
            sub_scope_id: None,
            scope_definition_id: None,
            element_id: None,
            tenant_id: None,
            due_date: None,
            lock_owner: None,
            retries: 1,
            exception_message: None,
            exception_stacktrace: None,
            created_at: Utc::now(),
            handler_type: None,
            configuration: None,
            scope_type: Some(CMMN_SCOPE_TYPE.to_string()),
        }
    }

    pub fn with_handler(
        mut self,
        handler_type: impl Into<String>,
        configuration: Option<String>,
    ) -> Self {
        self.handler_type = Some(handler_type.into());
        self.configuration = configuration;
        self
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CmmnMigrationDocument {
    pub target_case_definition_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CmmnMigrationValidationResult {
    pub valid: bool,
    pub validation_messages: Vec<String>,
}

impl CmmnMigrationValidationResult {
    pub fn valid() -> Self {
        Self {
            valid: true,
            validation_messages: Vec::new(),
        }
    }

    pub fn invalid(message: impl Into<String>) -> Self {
        Self {
            valid: false,
            validation_messages: vec![message.into()],
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct CmmnChangePlanItemStateRequest {
    pub activate_plan_item_definition_ids: Vec<String>,
    pub move_to_available_plan_item_definition_ids: Vec<String>,
    pub terminate_plan_item_definition_ids: Vec<String>,
    pub add_waiting_for_repetition_plan_item_definition_ids: Vec<String>,
    pub remove_waiting_for_repetition_plan_item_definition_ids: Vec<String>,
    pub change_plan_item_ids: Vec<(String, String)>,
    pub change_plan_item_ids_with_definition_id: Vec<(String, String)>,
    pub change_plan_item_definitions_with_new_target_ids: Vec<CmmnPlanItemDefinitionWithTargetIds>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CmmnPlanItemDefinitionWithTargetIds {
    pub existing_plan_item_definition_id: String,
    pub new_plan_item_id: String,
    pub new_plan_item_definition_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CmmnEventSubscription {
    pub id: String,
    pub event_type: String,
    pub event_name: Option<String>,
    pub activity_id: Option<String>,
    pub case_instance_id: Option<String>,
    pub case_definition_id: Option<String>,
    pub plan_item_instance_id: Option<String>,
    pub tenant_id: Option<String>,
    pub configuration: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum CmmnTaskAssociationKind {
    ProcessTask,
    CaseTask,
}

impl CmmnTaskAssociationKind {
    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            Self::ProcessTask => "PROCESS_TASK",
            Self::CaseTask => "CASE_TASK",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum CmmnTaskAssociationState {
    Active,
    Completed,
    Failed,
}

impl CmmnTaskAssociationState {
    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            Self::Active => "ACTIVE",
            Self::Completed => "COMPLETED",
            Self::Failed => "FAILED",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CmmnTaskInstanceAssociation {
    pub id: String,
    pub kind: CmmnTaskAssociationKind,
    pub state: CmmnTaskAssociationState,
    pub case_instance_id: String,
    pub case_definition_id: String,
    pub case_definition_key: String,
    pub stage_instance_id: Option<String>,
    pub plan_item_id: String,
    pub task_definition_id: String,
    pub child_definition_key: String,
    pub child_instance_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub failure_message: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CmmnProcessTaskStartRequest {
    pub process_definition_key: String,
    pub parent_case_instance_id: String,
    pub parent_case_definition_id: String,
    pub parent_case_definition_key: String,
    pub parent_plan_item_id: String,
    pub parent_task_definition_id: String,
    pub business_key: Option<String>,
    pub tenant_id: Option<String>,
    pub variables: Map<String, Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CmmnProcessTaskStartResult {
    pub process_instance_id: String,
    pub completed: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct PagedResult<T> {
    pub start: usize,
    pub size: usize,
    pub total: usize,
    pub data: Vec<T>,
}

impl From<&CmmnCaseInstance> for CmmnHistoricCaseInstance {
    fn from(value: &CmmnCaseInstance) -> Self {
        Self {
            case_instance_id: value.id.clone(),
            case_definition_id: value.case_definition_id.clone(),
            deployment_id: value.deployment_id.clone(),
            case_definition_key: value.case_definition_key.clone(),
            case_definition_name: value.case_definition_name.clone(),
            case_definition_version: value.case_definition_version,
            business_key: value.business_key.clone(),
            name: value.name.clone(),
            tenant_id: value.tenant_id.clone(),
            started_by: value.started_by.clone(),
            reference_id: value.reference_id.clone(),
            reference_type: value.reference_type.clone(),
            // Java obtains this from Authentication at the terminal transition
            // (DefaultCmmnHistoryManager.java:89-90); generic conversion has no actor.
            finished_by: None,
            started_at: value.started_at,
            completed_at: value.ended_at,
            state: value.state.clone(),
            business_status: value.business_status.clone(),
            callback_id: value.callback_id.clone(),
            callback_type: value.callback_type.clone(),
            variables: value.variables.clone(),
            case_file_items: value.case_file_items.clone(),
        }
    }
}

impl From<&CmmnHumanTaskInstance> for CmmnHistoricHumanTaskInstance {
    fn from(value: &CmmnHumanTaskInstance) -> Self {
        Self {
            task_id: value.id.clone(),
            case_instance_id: value.case_instance_id.clone(),
            case_definition_id: value.case_definition_id.clone(),
            case_definition_key: value.case_definition_key.clone(),
            stage_instance_id: value.stage_instance_id.clone(),
            plan_item_id: value.plan_item_id.clone(),
            task_definition_id: value.task_definition_id.clone(),
            name: value.name.clone(),
            activated_at: value.activated_at,
            last_enabled_at: value.last_enabled_at,
            completed_at: value.completed_at,
            completed_by: value.completed_by.clone(),
            state: value.state.clone(),
            assignee: value.assignee.clone(),
            owner: value.owner.clone(),
            priority: value.priority.clone(),
            due_date: value.due_date.clone(),
            category: value.category.clone(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CmmnPlanningTable {
    pub id: String,
    pub name: String,
    pub discretionary_items: Vec<CmmnDiscretionaryItem>,
}

impl CmmnPlanningTable {
    pub fn new(id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            discretionary_items: Vec::new(),
        }
    }

    pub fn with_discretionary_item(mut self, discretionary_item: CmmnDiscretionaryItem) -> Self {
        self.discretionary_items.push(discretionary_item);
        self
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CmmnDiscretionaryItem {
    pub id: String,
    pub name: String,
    pub definition_ref: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub planning_table: Option<String>,
    pub required: bool,
    pub manual_activation: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_stage_id: Option<String>,
}

impl CmmnDiscretionaryItem {
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        definition_ref: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            definition_ref: definition_ref.into(),
            planning_table: None,
            required: false,
            manual_activation: true,
            parent_stage_id: None,
        }
    }

    pub fn with_planning_table(mut self, planning_table: impl Into<String>) -> Self {
        self.planning_table = Some(planning_table.into());
        self
    }

    pub fn with_required(mut self, required: bool) -> Self {
        self.required = required;
        self
    }

    pub fn with_manual_activation(mut self, manual_activation: bool) -> Self {
        self.manual_activation = manual_activation;
        self
    }

    pub fn with_parent_stage_id(mut self, parent_stage_id: impl Into<String>) -> Self {
        self.parent_stage_id = Some(parent_stage_id.into());
        self
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CmmnPlanFragment {
    pub id: String,
    pub name: String,
    pub plan_items: Vec<CmmnPlanItem>,
    pub sentries: Vec<CmmnSentry>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub human_tasks: Vec<CmmnHumanTask>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub decision_tasks: Vec<CmmnDecisionTask>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub process_tasks: Vec<CmmnProcessTask>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub case_tasks: Vec<CmmnCaseTask>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub milestones: Vec<CmmnMilestone>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub event_listeners: Vec<CmmnEventListener>,
}

impl CmmnPlanFragment {
    pub fn new(id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            plan_items: Vec::new(),
            sentries: Vec::new(),
            human_tasks: Vec::new(),
            decision_tasks: Vec::new(),
            process_tasks: Vec::new(),
            case_tasks: Vec::new(),
            milestones: Vec::new(),
            event_listeners: Vec::new(),
        }
    }

    pub fn with_plan_item(mut self, plan_item: CmmnPlanItem) -> Self {
        self.plan_items.push(plan_item);
        self
    }

    pub fn with_sentry(mut self, sentry: CmmnSentry) -> Self {
        self.sentries.push(sentry);
        self
    }

    pub fn with_human_task(mut self, human_task: CmmnHumanTask) -> Self {
        self.human_tasks.push(human_task);
        self
    }

    pub fn with_decision_task(mut self, decision_task: CmmnDecisionTask) -> Self {
        self.decision_tasks.push(decision_task);
        self
    }

    pub fn with_process_task(mut self, process_task: CmmnProcessTask) -> Self {
        self.process_tasks.push(process_task);
        self
    }

    pub fn with_case_task(mut self, case_task: CmmnCaseTask) -> Self {
        self.case_tasks.push(case_task);
        self
    }

    pub fn with_milestone(mut self, milestone: CmmnMilestone) -> Self {
        self.milestones.push(milestone);
        self
    }

    pub fn with_event_listener(mut self, event_listener: CmmnEventListener) -> Self {
        self.event_listeners.push(event_listener);
        self
    }
}

/// Java parity: IOParameter.java (flowable-cmmn-model) — a declared variable copy between a
/// parent case and its child case/process instance, applied by IOParameterUtil.java:56-92.
/// `sourceExpression`/`targetExpression` are out of scope: the Rust engine has no `${}`
/// expression engine, so only plain `source`/`target` variable names are supported.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CmmnIOParameter {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
}

impl CmmnIOParameter {
    pub fn new(source: impl Into<String>, target: impl Into<String>) -> Self {
        Self {
            source: Some(source.into()),
            target: Some(target.into()),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CmmnProcessTask {
    pub id: String,
    pub name: String,
    pub is_blocking: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub process_ref: Option<String>,
    // Java parity: ChildTask.java:21-26 (businessKey + in/out parameters). The business key is a
    // literal override here because the Rust engine has no expression engine.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub business_key: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub in_parameters: Vec<CmmnIOParameter>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub out_parameters: Vec<CmmnIOParameter>,
}

impl CmmnProcessTask {
    pub fn new(id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            is_blocking: true,
            process_ref: None,
            business_key: None,
            in_parameters: Vec::new(),
            out_parameters: Vec::new(),
        }
    }

    pub fn with_blocking(mut self, is_blocking: bool) -> Self {
        self.is_blocking = is_blocking;
        self
    }

    pub fn with_process_ref(mut self, process_ref: impl Into<String>) -> Self {
        self.process_ref = Some(process_ref.into());
        self
    }

    pub fn with_business_key(mut self, business_key: impl Into<String>) -> Self {
        self.business_key = Some(business_key.into());
        self
    }

    pub fn with_in_parameter(mut self, parameter: CmmnIOParameter) -> Self {
        self.in_parameters.push(parameter);
        self
    }

    pub fn with_out_parameter(mut self, parameter: CmmnIOParameter) -> Self {
        self.out_parameters.push(parameter);
        self
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CmmnCaseTask {
    pub id: String,
    pub name: String,
    pub is_blocking: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub case_ref: Option<String>,
    // Java parity: ChildTask.java:21-26 (businessKey + in/out parameters). The business key is a
    // literal override here because the Rust engine has no expression engine.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub business_key: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub in_parameters: Vec<CmmnIOParameter>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub out_parameters: Vec<CmmnIOParameter>,
}

impl CmmnCaseTask {
    pub fn new(id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            is_blocking: true,
            case_ref: None,
            business_key: None,
            in_parameters: Vec::new(),
            out_parameters: Vec::new(),
        }
    }

    pub fn with_blocking(mut self, is_blocking: bool) -> Self {
        self.is_blocking = is_blocking;
        self
    }

    pub fn with_case_ref(mut self, case_ref: impl Into<String>) -> Self {
        self.case_ref = Some(case_ref.into());
        self
    }

    pub fn with_business_key(mut self, business_key: impl Into<String>) -> Self {
        self.business_key = Some(business_key.into());
        self
    }

    pub fn with_in_parameter(mut self, parameter: CmmnIOParameter) -> Self {
        self.in_parameters.push(parameter);
        self
    }

    pub fn with_out_parameter(mut self, parameter: CmmnIOParameter) -> Self {
        self.out_parameters.push(parameter);
        self
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CmmnGenericPlanItem {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub definition_type: Option<String>,
}

impl CmmnGenericPlanItem {
    pub fn new(id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            definition_type: None,
        }
    }

    pub fn with_definition_type(mut self, definition_type: impl Into<String>) -> Self {
        self.definition_type = Some(definition_type.into());
        self
    }
}
