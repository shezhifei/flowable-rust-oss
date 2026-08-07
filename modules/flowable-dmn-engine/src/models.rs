use chrono::{
    DateTime, Datelike, NaiveDate, NaiveDateTime, NaiveTime, SecondsFormat, Timelike, Utc,
};
use flowable_dmn_model as dmn_model;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, HashMap};
use std::convert::TryFrom;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum CollectOperator {
    Count,
    Sum,
    Min,
    Max,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum DmnHitPolicy {
    First,
    Unique,
    Any,
    RuleOrder,
    OutputOrder,
    Priority,
    Collect,
    Complete,
    Batch,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum DmnUnaryTest {
    Any,
    Equals(Value),
    NotEquals(Value),
    StringFunction {
        function: DmnStringFunction,
        needle: String,
    },
    StringTransform {
        transform: DmnStringTransform,
        expected: String,
    },
    StringTransformComparison {
        transform: DmnStringTransform,
        operator: DmnComparisonOperator,
        expected: Value,
    },
    GreaterThan(Value),
    GreaterThanOrEqual(Value),
    LessThan(Value),
    LessThanOrEqual(Value),
    Range {
        start: Value,
        end: Value,
        start_inclusive: bool,
        end_inclusive: bool,
    },
    AnyOf(Vec<DmnUnaryTest>),
    Not(Box<DmnUnaryTest>),
    And(Vec<DmnUnaryTest>),
    Or(Vec<DmnUnaryTest>),
    InstanceOf {
        type_name: String,
    },
    Substring {
        start: i32,
        length: Option<i32>,
        expected: String,
    },
    Replace {
        pattern: String,
        replacement: String,
        flags: Option<String>,
        expected: String,
    },
    ListContains {
        needle: DmnListContainsNeedle,
    },
    /// FEEL membership unary test: `? in ("a", "b")` / `in (1, 2, 3)`.
    InList {
        values: Vec<Value>,
    },
    /// Comparison whose right-hand side must be evaluated per row, because it is
    /// not a deploy-time constant (date aliases `fn_now()` / `fn_date(...)`).
    /// Java rewrites `fn_*` to the `date:*` EL functions and lets JUEL evaluate
    /// them at execution time (`ELInputEntryExpressionPreParser.java:26-29`).
    DeferredComparison {
        operator: DmnDeferredOperator,
        source: String,
    },
    /// EL pass-through: the whole entry is `#{...}` / `${...}` and evaluates to a
    /// boolean. Java returns the text unchanged from the pre-parser
    /// (`ELInputEntryExpressionPreParser.java:31-33`) and evaluates it through
    /// `RuleExpressionCondition`, which rejects null / non-Boolean results
    /// (`RuleExpressionCondition.java:36-50`). The input value is *not* compared
    /// against the result — the expression carries its own comparison.
    ElCondition {
        source: String,
    },
    /// Java `.property` shorthand: the entry is appended verbatim to the input
    /// variable (`ELInputEntryExpressionPreParser.java:42-46`), so `.name == "x"`
    /// becomes `#{input.name == "x"}`. Rust applies the path to the already
    /// resolved input value instead, so the input variable name is not needed.
    PropertyPath {
        path: Vec<String>,
        test: Box<DmnUnaryTest>,
    },
}

/// Operators usable with a per-row evaluated right-hand side.
/// Mirrors the operator set the Java pre-parser preserves verbatim
/// (`ELInputEntryExpressionPreParser.java:22`) plus the implicit `==`
/// it inserts for bare operands (`:53-62`).
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum DmnDeferredOperator {
    Equals,
    NotEquals,
    GreaterThan,
    GreaterThanOrEqual,
    LessThan,
    LessThanOrEqual,
}

/// Right-hand side of `list contains(?, ...)`.
///
/// Literals keep the existing M44 behaviour. Simple unquoted identifiers are
/// treated as variable references resolved from decision execution inputs.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum DmnListContainsNeedle {
    Literal(Value),
    Variable(String),
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum DmnStringFunction {
    Contains,
    StartsWith,
    EndsWith,
    Matches,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum DmnStringTransform {
    LowerCase,
    UpperCase,
    StringLength,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum DmnComparisonOperator {
    GreaterThan,
    GreaterThanOrEqual,
    LessThan,
    LessThanOrEqual,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct DmnInputClause {
    pub id: String,
    pub input_variable: String,
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub type_ref: Option<String>,
}

impl DmnInputClause {
    pub fn new(id: impl Into<String>, input_variable: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            input_variable: input_variable.into(),
            label: None,
            type_ref: None,
        }
    }

    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    pub fn with_type_ref(mut self, type_ref: impl Into<String>) -> Self {
        self.type_ref = Some(type_ref.into());
        self
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct DmnOutputClause {
    pub id: String,
    pub name: String,
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub type_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub output_values: Vec<Value>,
}

impl DmnOutputClause {
    pub fn new(id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            label: None,
            type_ref: None,
            output_values: Vec::new(),
        }
    }

    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    pub fn with_type_ref(mut self, type_ref: impl Into<String>) -> Self {
        self.type_ref = Some(type_ref.into());
        self
    }

    pub fn with_output_values(mut self, output_values: Vec<Value>) -> Self {
        self.output_values = output_values;
        self
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct DmnRuleInputEntry {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub expression: DmnUnaryTest,
}

impl DmnRuleInputEntry {
    pub fn new(expression: DmnUnaryTest) -> Self {
        Self {
            id: None,
            expression,
        }
    }

    pub fn with_id(mut self, id: impl Into<String>) -> Self {
        self.id = Some(id.into());
        self
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct DmnRuleOutputEntry {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// Pre-parsed static snapshot retained for backward compatibility with
    /// already-deployed definition JSON (`repository.rs` serializes the whole
    /// definition). Runtime evaluation prefers `expression` when non-empty;
    /// when `expression` is absent/empty (legacy rows / `DmnRuleOutputEntry::new`)
    /// the engine falls back to this field without re-evaluating.
    pub value: Value,
    /// Original output-entry text evaluated at runtime via [`FeelExpressionEngine`].
    /// Java wraps bare text with `#{...}` (`ELOutputEntryExpressionPreParser.java:23-33`)
    /// and executes via JUEL (`ELExpressionExecutor.java:76`); Rust peels shells then
    /// evaluates a FEEL subset. Empty / `-` means skip (Java `RuleEngineExecutorImpl.java:291-296`).
    /// `#[serde(default)]` keeps old deployment JSON without this field deserializable.
    #[serde(default)]
    pub expression: String,
}

impl DmnRuleOutputEntry {
    /// Build a static output entry from a pre-materialized value.
    /// Leaves `expression` empty so runtime uses the legacy `value` path
    /// (compatible with existing programmatic tests and pre-P81 deployments).
    pub fn new(value: Value) -> Self {
        Self {
            id: None,
            value,
            expression: String::new(),
        }
    }

    /// Build an output entry whose text is evaluated at runtime (FEEL subset).
    pub fn from_expression(expression: impl Into<String>) -> Self {
        let expression = expression.into();
        // Empty / dash → Null snapshot (runtime skips; Java
        // RuleEngineExecutorImpl.java:291-296). Non-empty: best-effort static
        // snapshot for deploy-time typeRef checks of pure literals.
        let value = if expression.trim().is_empty() || expression.trim() == "-" {
            Value::Null
        } else {
            parse_literal_expression(&expression).unwrap_or(Value::Null)
        };
        Self {
            id: None,
            value,
            expression,
        }
    }

    pub fn with_id(mut self, id: impl Into<String>) -> Self {
        self.id = Some(id.into());
        self
    }

    pub fn with_expression(mut self, expression: impl Into<String>) -> Self {
        self.expression = expression.into();
        self
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct DmnRule {
    pub id: String,
    pub input_entries: Vec<DmnRuleInputEntry>,
    pub output_entries: Vec<DmnRuleOutputEntry>,
    pub description: Option<String>,
}

impl DmnRule {
    pub fn new(
        id: impl Into<String>,
        input_entries: Vec<DmnRuleInputEntry>,
        output_entries: Vec<DmnRuleOutputEntry>,
    ) -> Self {
        Self {
            id: id.into(),
            input_entries,
            output_entries,
            description: None,
        }
    }

    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Annotation {
    pub id: String,
    pub name: String,
    pub expression: String,
}

impl Annotation {
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        expression: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            expression: expression.into(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct DmnDecision {
    pub id: String,
    pub key: String,
    pub name: String,
    pub hit_policy: DmnHitPolicy,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub collect_operator: Option<CollectOperator>,
    pub inputs: Vec<DmnInputClause>,
    pub outputs: Vec<DmnOutputClause>,
    pub rules: Vec<DmnRule>,
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub required_decisions: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub annotations: Vec<Annotation>,
}

impl DmnDecision {
    pub fn new(
        id: impl Into<String>,
        key: impl Into<String>,
        name: impl Into<String>,
        hit_policy: DmnHitPolicy,
        inputs: Vec<DmnInputClause>,
        outputs: Vec<DmnOutputClause>,
        rules: Vec<DmnRule>,
    ) -> Self {
        Self {
            id: id.into(),
            key: key.into(),
            name: name.into(),
            hit_policy,
            collect_operator: None,
            inputs,
            outputs,
            rules,
            description: None,
            required_decisions: Vec::new(),
            annotations: Vec::new(),
        }
    }

    pub fn with_collect_operator(mut self, collect_operator: CollectOperator) -> Self {
        self.collect_operator = Some(collect_operator);
        self
    }

    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    pub fn with_annotations(mut self, annotations: Vec<Annotation>) -> Self {
        self.annotations = annotations;
        self
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DmnModel {
    pub id: String,
    pub name: String,
    pub decisions: Vec<DmnDecision>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub decision_services: Vec<DecisionService>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub knowledge_sources: Vec<KnowledgeSource>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub authority_requirements: Vec<AuthorityRequirement>,
}

impl DmnModel {
    pub fn new(decisions: Vec<DmnDecision>) -> Self {
        Self {
            id: format!("drd-{}", uuid::Uuid::new_v4()),
            name: "Unnamed DRD".to_string(),
            decisions,
            decision_services: Vec::new(),
            knowledge_sources: Vec::new(),
            authority_requirements: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DecisionService {
    pub id: String,
    pub name: String,
    pub required_decisions: Vec<String>,
    pub output_decisions: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeSource {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    #[serde(rename = "type")]
    pub type_: Option<String>,
    pub owner: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AuthorityRequirement {
    pub id: String,
    pub required_authority: Option<String>,
    pub required_decision: Option<String>,
    pub decision: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct DmnDeploymentResource {
    pub resource_name: String,
    pub model: DmnModel,
    #[serde(default)]
    pub resource_bytes: Vec<u8>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct DmnDeploymentRequest {
    pub name: String,
    pub category: Option<String>,
    pub parent_deployment_id: Option<String>,
    pub tenant_id: Option<String>,
    pub resources: Vec<DmnDeploymentResource>,
}

impl DmnDeploymentRequest {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            category: None,
            parent_deployment_id: None,
            tenant_id: None,
            resources: Vec::new(),
        }
    }

    pub fn with_category(mut self, category: impl Into<String>) -> Self {
        self.category = Some(category.into());
        self
    }

    pub fn with_parent_deployment_id(mut self, parent_deployment_id: impl Into<String>) -> Self {
        self.parent_deployment_id = Some(parent_deployment_id.into());
        self
    }

    pub fn with_tenant_id(mut self, tenant_id: impl Into<String>) -> Self {
        self.tenant_id = Some(tenant_id.into());
        self
    }

    pub fn with_resource(mut self, resource_name: impl Into<String>, model: DmnModel) -> Self {
        self.resources.push(DmnDeploymentResource {
            resource_name: resource_name.into(),
            model,
            resource_bytes: Vec::new(),
        });
        self
    }

    pub fn with_resource_bytes(
        mut self,
        resource_name: impl Into<String>,
        model: DmnModel,
        resource_bytes: impl Into<Vec<u8>>,
    ) -> Self {
        self.resources.push(DmnDeploymentResource {
            resource_name: resource_name.into(),
            model,
            resource_bytes: resource_bytes.into(),
        });
        self
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct DmnDeployment {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_deployment_id: Option<String>,
    pub tenant_id: Option<String>,
    pub resource_names: Vec<String>,
    pub deployed_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct DmnDecisionDefinition {
    pub id: String,
    pub decision_id: String,
    pub deployment_id: String,
    pub key: String,
    pub name: String,
    pub version: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_deployment_id: Option<String>,
    pub tenant_id: Option<String>,
    pub resource_name: String,
    pub hit_policy: DmnHitPolicy,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub collect_operator: Option<CollectOperator>,
    pub inputs: Vec<DmnInputClause>,
    pub outputs: Vec<DmnOutputClause>,
    pub rules: Vec<DmnRule>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub required_decisions: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct DmnExecutionRequest {
    pub variables: Value,
    pub business_key: Option<String>,
    pub tenant_id: Option<String>,
    pub parent_deployment_id: Option<String>,
    pub disable_history: bool,
    /// Java `ExecuteDecisionContext.fallbackToDefaultTenant` (:32), set by
    /// `ExecuteDecisionBuilder.fallbackToDefaultTenant()`. When the key+tenant
    /// lookup misses, `AbstractExecuteDecisionCmd.resolveDefinition`
    /// (:90-103, :141-160) retries against the default tenant.
    #[serde(default)]
    pub fallback_to_default_tenant: bool,
    /// Java `ExecuteDecisionContext.instanceId` (:26) — the process instance id
    /// (`DmnActivityBehavior.java:100`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instance_id: Option<String>,
    /// Java `ExecuteDecisionContext.executionId` (:27) — the BPMN execution id
    /// (`DmnActivityBehavior.java:101`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_id: Option<String>,
    /// Java `ExecuteDecisionContext.activityId` (:28) — the calling task id
    /// (`DmnActivityBehavior.java:102`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub activity_id: Option<String>,
    /// Java `ExecuteDecisionContext.scopeType` (:29), persisted to `SCOPE_TYPE_`
    /// (`PersistHistoricDecisionExecutionCmd.java:59`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope_type: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DmnExpressionExecution {
    pub id: String,
    pub result: Value,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DmnRuleExecutionAudit {
    pub rule_number: usize,
    pub rule_id: String,
    pub valid: bool,
    #[serde(default)]
    pub condition_results: Vec<DmnExpressionExecution>,
    #[serde(default)]
    pub conclusion_results: Vec<DmnExpressionExecution>,
    /// Java `RuleExecutionAuditContainer.validationMessage` (:38, :96-102) —
    /// set on hit-policy soft violations when `strictMode=false`
    /// (e.g. UNIQUE/ANY; `HitPolicyUnique.java:49-50`, `HitPolicyAny.java:60-61`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validation_message: Option<String>,
    /// Java `RuleExecutionAuditContainer.exceptionMessage` (:35-36, :88-94) —
    /// set on hit-policy hard violations in strict mode before throw
    /// (`HitPolicyUnique.java:45-46`, `HitPolicyAny.java:53-54`). Not always
    /// surfaced on the thrown path in Rust (failed history clears audits).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exception_message: Option<String>,
}

impl DmnExecutionRequest {
    pub fn new(variables: Value) -> Self {
        Self {
            variables,
            business_key: None,
            tenant_id: None,
            parent_deployment_id: None,
            disable_history: false,
            fallback_to_default_tenant: false,
            instance_id: None,
            execution_id: None,
            activity_id: None,
            scope_type: None,
        }
    }

    pub fn with_business_key(mut self, business_key: impl Into<String>) -> Self {
        self.business_key = Some(business_key.into());
        self
    }

    pub fn with_tenant_id(mut self, tenant_id: impl Into<String>) -> Self {
        self.tenant_id = Some(tenant_id.into());
        self
    }

    pub fn disable_history(mut self) -> Self {
        self.disable_history = true;
        self
    }

    /// Java `ExecuteDecisionBuilder.fallbackToDefaultTenant()`.
    pub fn fallback_to_default_tenant(mut self) -> Self {
        self.fallback_to_default_tenant = true;
        self
    }

    /// Java `ExecuteDecisionBuilder.instanceId/executionId/activityId`
    /// (`DmnActivityBehavior.java:100-102`).
    pub fn with_audit_correlation(
        mut self,
        instance_id: Option<String>,
        execution_id: Option<String>,
        activity_id: Option<String>,
    ) -> Self {
        self.instance_id = instance_id;
        self.execution_id = execution_id;
        self.activity_id = activity_id;
        self
    }

    /// Java `ExecuteDecisionBuilder.scopeType()`.
    pub fn with_scope_type(mut self, scope_type: impl Into<String>) -> Self {
        self.scope_type = Some(scope_type.into());
        self
    }
}

/// Row-based DMN execution result (Java `DecisionExecutionAuditContainer`).
///
/// Java stores `List<Map<String,Object>> decisionResult` — one map per matched
/// rule — plus `multipleResults` (see `DecisionExecutionAuditContainer.java:48-49`
/// and `AbstractHitPolicy.java:64-71`). Pre-P79 Rust used columnar
/// `Map<outputName, Value|Array>`; that shape is no longer produced.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct DmnExecutionResult {
    pub execution_id: String,
    pub decision_definition_id: String,
    pub deployment_id: String,
    pub decision_key: String,
    pub decision_name: String,
    pub decision_version: i32,
    pub hit_policy: DmnHitPolicy,
    pub matched_rule_id: Option<String>,
    #[serde(default)]
    pub matched_rule_count: usize,
    #[serde(default)]
    pub rule_executions: Vec<DmnRuleExecutionAudit>,
    pub business_key: Option<String>,
    pub executed_at: DateTime<Utc>,
    pub inputs: Map<String, Value>,
    /// Row-shaped decision result: one map per matched rule
    /// (Java `decisionResult: List<Map<String,Object>>`).
    #[serde(default)]
    pub decision_result: Vec<Map<String, Value>>,
    /// Java `multipleResults` — true for RULE_ORDER / OUTPUT_ORDER /
    /// COLLECT(without aggregator) and Rust extensions Complete / Batch.
    #[serde(default)]
    pub multiple_results: bool,
    /// Populated when a decision *service* was executed
    /// (Java `DecisionServiceExecutionAuditContainer.decisionServiceResult`).
    /// Uses `BTreeMap` because `serde_json::Map` only serializes `Map<String, Value>`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decision_service_result: Option<BTreeMap<String, Vec<Map<String, Value>>>>,
    /// Java `DecisionExecutionAuditContainer.validationMessage` (:56, :241-247) —
    /// decision-level soft hit-policy violation when `strictMode=false`
    /// (`HitPolicyUnique.java:73`, `HitPolicyAny.java:74`,
    /// `HitPolicyPriority.java:66-69`, `HitPolicyOutputOrder.java:58`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validation_message: Option<String>,
}

impl DmnExecutionResult {
    /// First matched rule's outputs, if any.
    pub fn first_result(&self) -> Option<&Map<String, Value>> {
        self.decision_result.first()
    }

    /// Output value from the first matched row (single-result convenience).
    pub fn get_output(&self, name: &str) -> Option<&Value> {
        self.first_result().and_then(|row| row.get(name))
    }

    /// Stack variables for required-decision chaining.
    /// Java `AbstractHitPolicy.updateStackWithDecisionResults` — each row is
    /// applied in order so later rows overwrite earlier keys.
    /// (`ComposeDecisionResultBehavior.java` / `AbstractHitPolicy.java:75-77`)
    pub fn stack_variables(&self) -> Map<String, Value> {
        stack_variables_from_rows(&self.decision_result)
    }
}

/// Historic audit payload. Persist shape follows `DmnExecutionResult`
/// (row-based). Deserialize accepts legacy columnar `outputs` so pre-P79
/// history rows remain readable.
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct HistoricDecisionExecution {
    pub execution_id: String,
    pub decision_definition_id: String,
    pub deployment_id: String,
    pub decision_key: String,
    pub decision_name: String,
    pub decision_version: i32,
    pub hit_policy: DmnHitPolicy,
    pub matched_rule_id: Option<String>,
    #[serde(default)]
    pub matched_rule_count: usize,
    #[serde(default)]
    pub rule_executions: Vec<DmnRuleExecutionAudit>,
    pub business_key: Option<String>,
    #[serde(default)]
    pub tenant_id: Option<String>,
    /// Java `HistoricDecisionExecution.instanceId` → `INSTANCE_ID_`
    /// (`PersistHistoricDecisionExecutionCmd.java:56`). Process instance id for
    /// BPMN-driven executions; `None` for direct API executions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instance_id: Option<String>,
    /// Java `HistoricDecisionExecution.executionId` → `EXECUTION_ID_`
    /// (`PersistHistoricDecisionExecutionCmd.java:57`).
    ///
    /// Named `scope_execution_id` because this struct's `execution_id` already
    /// holds the DMN execution's own id (Java's `ID_`/primary key), so the
    /// Java column name is not available here.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope_execution_id: Option<String>,
    /// Java `HistoricDecisionExecution.activityId` → `ACTIVITY_ID_`
    /// (`PersistHistoricDecisionExecutionCmd.java:58`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub activity_id: Option<String>,
    /// Java `HistoricDecisionExecution.scopeType` → `SCOPE_TYPE_`
    /// (`PersistHistoricDecisionExecutionCmd.java:59`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope_type: Option<String>,
    /// Java `HistoricDecisionExecution.failed` → `FAILED_`
    /// (`PersistHistoricDecisionExecutionCmd.java:60`), fed by
    /// `DecisionExecutionAuditContainer.setFailedWithException`
    /// (`RuleEngineExecutorImpl.java:94-97,154-158`): an evaluation error is
    /// captured on the audit container rather than aborting the audit, so the
    /// row is still written — with `failed = true` and no results.
    #[serde(default)]
    pub failed: bool,
    pub executed_at: DateTime<Utc>,
    pub inputs: Map<String, Value>,
    /// Row-shaped result (Java `decisionResult`).
    #[serde(default)]
    pub decision_result: Vec<Map<String, Value>>,
    #[serde(default)]
    pub multiple_results: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decision_service_result: Option<BTreeMap<String, Vec<Map<String, Value>>>>,
    /// Java persists the whole `DecisionExecutionAuditContainer` as
    /// `EXECUTION_JSON_` (`PersistHistoricDecisionExecutionCmd.java:73`), so a
    /// decision-level soft hit-policy violation lands in history too
    /// (`DecisionExecutionAuditContainer.java:56,241-247`). Absent in pre-P91
    /// history JSON.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validation_message: Option<String>,
}

impl HistoricDecisionExecution {
    pub fn first_result(&self) -> Option<&Map<String, Value>> {
        self.decision_result.first()
    }

    pub fn get_output(&self, name: &str) -> Option<&Value> {
        self.first_result().and_then(|row| row.get(name))
    }
}

impl<'de> Deserialize<'de> for HistoricDecisionExecution {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Raw {
            execution_id: String,
            decision_definition_id: String,
            deployment_id: String,
            decision_key: String,
            decision_name: String,
            decision_version: i32,
            hit_policy: DmnHitPolicy,
            matched_rule_id: Option<String>,
            #[serde(default)]
            matched_rule_count: usize,
            #[serde(default)]
            rule_executions: Vec<DmnRuleExecutionAudit>,
            business_key: Option<String>,
            #[serde(default)]
            tenant_id: Option<String>,
            /// P83 correlation columns; absent in pre-P83 history JSON.
            #[serde(default)]
            instance_id: Option<String>,
            #[serde(default)]
            scope_execution_id: Option<String>,
            #[serde(default)]
            activity_id: Option<String>,
            #[serde(default)]
            scope_type: Option<String>,
            /// P83 `FAILED_`; absent in pre-P83 history JSON.
            #[serde(default)]
            failed: bool,
            executed_at: DateTime<Utc>,
            inputs: Map<String, Value>,
            #[serde(default)]
            decision_result: Vec<Map<String, Value>>,
            /// Pre-P79 columnar shape: `Map<outputName, Value|Array>`.
            #[serde(default)]
            outputs: Option<Map<String, Value>>,
            #[serde(default)]
            multiple_results: bool,
            #[serde(default)]
            decision_service_result: Option<BTreeMap<String, Vec<Map<String, Value>>>>,
            /// P91 decision-level soft violation; absent in pre-P91 history JSON.
            #[serde(default)]
            validation_message: Option<String>,
        }

        let raw = Raw::deserialize(deserializer)?;
        let decision_result = if !raw.decision_result.is_empty() {
            raw.decision_result
        } else if let Some(outputs) = raw.outputs {
            columnar_outputs_to_rows(outputs)
        } else {
            Vec::new()
        };

        Ok(Self {
            execution_id: raw.execution_id,
            decision_definition_id: raw.decision_definition_id,
            deployment_id: raw.deployment_id,
            decision_key: raw.decision_key,
            decision_name: raw.decision_name,
            decision_version: raw.decision_version,
            hit_policy: raw.hit_policy,
            matched_rule_id: raw.matched_rule_id,
            matched_rule_count: raw.matched_rule_count,
            rule_executions: raw.rule_executions,
            business_key: raw.business_key,
            tenant_id: raw.tenant_id,
            instance_id: raw.instance_id,
            scope_execution_id: raw.scope_execution_id,
            activity_id: raw.activity_id,
            scope_type: raw.scope_type,
            failed: raw.failed,
            executed_at: raw.executed_at,
            inputs: raw.inputs,
            decision_result,
            multiple_results: raw.multiple_results,
            decision_service_result: raw.decision_service_result,
            validation_message: raw.validation_message,
        })
    }
}

/// Convert pre-P79 columnar `Map<name, scalar|array>` into row maps.
pub fn columnar_outputs_to_rows(outputs: Map<String, Value>) -> Vec<Map<String, Value>> {
    if outputs.is_empty() {
        return Vec::new();
    }

    let max_len = outputs
        .values()
        .map(|value| match value {
            Value::Array(items) => items.len(),
            _ => 1,
        })
        .max()
        .unwrap_or(0);
    if max_len == 0 {
        return Vec::new();
    }

    let mut rows = vec![Map::new(); max_len];
    for (key, value) in outputs {
        match value {
            Value::Array(items) => {
                for (index, item) in items.into_iter().enumerate() {
                    if let Some(row) = rows.get_mut(index) {
                        row.insert(key.clone(), item);
                    }
                }
            }
            other => {
                if let Some(row) = rows.first_mut() {
                    row.insert(key, other);
                }
            }
        }
    }
    rows
}

/// Java `updateStackWithDecisionResults` — last row wins per key.
pub fn stack_variables_from_rows(rows: &[Map<String, Value>]) -> Map<String, Value> {
    let mut stack = Map::new();
    for row in rows {
        for (key, value) in row {
            stack.insert(key.clone(), value.clone());
        }
    }
    stack
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct PagedResult<T> {
    pub start: usize,
    pub size: usize,
    pub total: usize,
    pub data: Vec<T>,
}

impl TryFrom<dmn_model::DmnDefinition> for DmnModel {
    type Error = crate::error::DmnError;

    fn try_from(value: dmn_model::DmnDefinition) -> Result<Self, Self::Error> {
        let id = value
            .id
            .clone()
            .unwrap_or_else(|| format!("drd-{}", uuid::Uuid::new_v4()));
        let name = value
            .name
            .clone()
            .unwrap_or_else(|| "Unnamed DRD".to_string());

        let decisions = value
            .decisions
            .into_iter()
            .map(DmnDecision::try_from)
            .collect::<Result<Vec<_>, _>>()?;

        let decision_services = value
            .decision_services
            .into_iter()
            .map(|ds| DecisionService {
                id: ds.id,
                name: ds.name,
                required_decisions: ds.required_decisions,
                output_decisions: ds.output_decisions,
            })
            .collect();

        let knowledge_sources = value
            .knowledge_sources
            .into_iter()
            .map(|ks| KnowledgeSource {
                id: ks.id,
                name: ks.name,
                description: ks.description,
                type_: ks.type_,
                owner: ks.owner,
            })
            .collect();

        let authority_requirements = value
            .authority_requirements
            .into_iter()
            .map(|ar| AuthorityRequirement {
                id: ar.id,
                required_authority: ar.required_authority,
                required_decision: ar.required_decision,
                decision: ar.decision,
            })
            .collect();

        Ok(Self {
            id,
            name,
            decisions,
            decision_services,
            knowledge_sources,
            authority_requirements,
        })
    }
}

impl TryFrom<dmn_model::Decision> for DmnDecision {
    type Error = crate::error::DmnError;

    fn try_from(value: dmn_model::Decision) -> Result<Self, Self::Error> {
        Ok(Self {
            id: value.id.clone(),
            key: value.id,
            name: value.name.unwrap_or_else(|| "Unnamed decision".to_string()),
            hit_policy: DmnHitPolicy::try_from(value.decision_table.hit_policy)?,
            collect_operator: value
                .decision_table
                .collect_operator
                .map(CollectOperator::try_from)
                .transpose()?,
            inputs: value
                .decision_table
                .inputs
                .into_iter()
                .map(DmnInputClause::try_from)
                .collect::<Result<Vec<_>, _>>()?,
            outputs: value
                .decision_table
                .outputs
                .into_iter()
                .map(DmnOutputClause::try_from)
                .collect::<Result<Vec<_>, _>>()?,
            rules: value
                .decision_table
                .rules
                .into_iter()
                .map(DmnRule::try_from)
                .collect::<Result<Vec<_>, _>>()?,
            description: None,
            required_decisions: value.required_decisions,
            annotations: Vec::new(),
        })
    }
}

impl TryFrom<dmn_model::HitPolicy> for DmnHitPolicy {
    type Error = crate::error::DmnError;

    fn try_from(value: dmn_model::HitPolicy) -> Result<Self, Self::Error> {
        match value {
            dmn_model::HitPolicy::First => Ok(Self::First),
            dmn_model::HitPolicy::Unique => Ok(Self::Unique),
            dmn_model::HitPolicy::Any => Ok(Self::Any),
            dmn_model::HitPolicy::RuleOrder => Ok(Self::RuleOrder),
            dmn_model::HitPolicy::OutputOrder => Ok(Self::OutputOrder),
            dmn_model::HitPolicy::Priority => Ok(Self::Priority),
            dmn_model::HitPolicy::Collect => Ok(Self::Collect),
            dmn_model::HitPolicy::Complete => Ok(Self::Complete),
        }
    }
}

impl TryFrom<dmn_model::CollectOperator> for CollectOperator {
    type Error = crate::error::DmnError;

    fn try_from(value: dmn_model::CollectOperator) -> Result<Self, Self::Error> {
        match value {
            dmn_model::CollectOperator::Count => Ok(Self::Count),
            dmn_model::CollectOperator::Sum => Ok(Self::Sum),
            dmn_model::CollectOperator::Min => Ok(Self::Min),
            dmn_model::CollectOperator::Max => Ok(Self::Max),
        }
    }
}

impl TryFrom<dmn_model::InputClause> for DmnInputClause {
    type Error = crate::error::DmnError;

    fn try_from(value: dmn_model::InputClause) -> Result<Self, Self::Error> {
        let input_expression = value.input_expression;
        let input_variable = input_expression
            .text
            .filter(|text| !text.trim().is_empty())
            .ok_or_else(|| {
                crate::error::DmnError::validation(
                    "DMN inputExpression text is required in the owned M15 subset",
                )
            })?;
        Ok(Self {
            id: value
                .id
                .unwrap_or_else(|| format!("input-{}", value.input_number)),
            input_variable,
            label: value.label,
            type_ref: input_expression.type_ref,
        })
    }
}

impl TryFrom<dmn_model::OutputClause> for DmnOutputClause {
    type Error = crate::error::DmnError;

    fn try_from(value: dmn_model::OutputClause) -> Result<Self, Self::Error> {
        let output_values = value
            .output_values
            .map(|values| parse_output_values(values.text.as_deref().unwrap_or_default()))
            .transpose()?
            .unwrap_or_default();
        Ok(Self {
            id: value
                .id
                .unwrap_or_else(|| format!("output-{}", value.output_number)),
            name: value
                .name
                .unwrap_or_else(|| format!("output{}", value.output_number)),
            label: value.label,
            type_ref: value.type_ref,
            output_values,
        })
    }
}

impl TryFrom<dmn_model::DecisionRule> for DmnRule {
    type Error = crate::error::DmnError;

    fn try_from(value: dmn_model::DecisionRule) -> Result<Self, Self::Error> {
        Ok(Self {
            id: value
                .id
                .unwrap_or_else(|| format!("rule-{}", value.rule_number)),
            input_entries: value
                .input_entries
                .into_iter()
                .map(DmnRuleInputEntry::try_from)
                .collect::<Result<Vec<_>, _>>()?,
            output_entries: value
                .output_entries
                .into_iter()
                .map(DmnRuleOutputEntry::try_from)
                .collect::<Result<Vec<_>, _>>()?,
            description: None,
        })
    }
}

impl TryFrom<dmn_model::UnaryTests> for DmnRuleInputEntry {
    type Error = crate::error::DmnError;

    fn try_from(value: dmn_model::UnaryTests) -> Result<Self, Self::Error> {
        Ok(Self {
            id: value.id,
            expression: parse_unary_test(value.text.as_deref().unwrap_or("-"))?,
        })
    }
}

impl TryFrom<dmn_model::LiteralExpression> for DmnRuleOutputEntry {
    type Error = crate::error::DmnError;

    fn try_from(value: dmn_model::LiteralExpression) -> Result<Self, Self::Error> {
        // Preserve original text for runtime FEEL evaluation
        // (Java keeps LiteralExpression text and evaluates at execution:
        // RuleEngineExecutorImpl.java:248-254).
        let expression = value.text.unwrap_or_default();
        let parsed = if expression.trim().is_empty() || expression.trim() == "-" {
            Value::Null
        } else {
            // Static snapshot for deploy-time typeRef normalization of literals
            // and for legacy consumers of `value`. Non-literal expressions fall
            // through parse_literal_expression as a string; deploy skips coerce.
            parse_literal_expression(&expression)?
        };
        Ok(Self {
            id: value.id,
            value: parsed,
            expression,
        })
    }
}

/// Peel a single outer `#{...}` / `${...}` shell so FEEL evaluation sees the
/// inner expression. Java wraps bare output text with `#{...}`
/// (`ELOutputEntryExpressionPreParser.java:23-33`); Rust evaluates FEEL, not JUEL.
pub(crate) fn strip_expression_shells(expression: &str) -> &str {
    let trimmed = expression.trim();
    for (prefix, suffix) in [("#{", "}"), ("${", "}")] {
        if let Some(inner) = trimmed
            .strip_prefix(prefix)
            .and_then(|s| s.strip_suffix(suffix))
        {
            return inner.trim();
        }
    }
    trimmed
}

/// Whether output-entry text is a pure static literal (not a runtime expression).
/// Deploy-time typeRef coerce keeps validating static literals; non-literals are
/// deferred to execution (`RuleEngineExecutorImpl.java:253-254`).
pub(crate) fn is_static_output_literal(text: &str) -> bool {
    let trimmed = strip_expression_shells(text.trim());
    if trimmed.is_empty() || trimmed == "-" {
        return true;
    }
    if trimmed.eq_ignore_ascii_case("null")
        || trimmed.eq_ignore_ascii_case("true")
        || trimmed.eq_ignore_ascii_case("false")
    {
        return true;
    }
    if parse_feel_temporal_constructor(trimmed).is_some() {
        return true;
    }
    if parse_feel_duration_constructor(trimmed).is_some() {
        return true;
    }
    if parse_quoted_string_literal(trimmed).is_some() {
        return true;
    }
    if let Ok(value) = serde_json::from_str::<Value>(trimmed)
        && (value.is_number() || value.is_array() || value.is_object())
    {
        return true;
    }
    false
}

fn parse_unary_test(text: &str) -> Result<DmnUnaryTest, crate::error::DmnError> {
    let trimmed = text.trim();
    if trimmed.is_empty() || trimmed == "-" {
        return Ok(DmnUnaryTest::Any);
    }

    // EL pass-through wins over the unary-test grammar: Java short-circuits the
    // pre-parser and hands the text straight to the expression manager
    // (`ELInputEntryExpressionPreParser.java:31-33`).
    if let Some(expression) = parse_el_condition_unary_test(trimmed)? {
        return Ok(expression);
    }

    // `.property` shorthand (`ELInputEntryExpressionPreParser.java:42-46`).
    if let Some(expression) = parse_property_path_unary_test(trimmed)? {
        return Ok(expression);
    }

    if is_not_unary_test_candidate(trimmed) {
        return parse_not_unary_test(trimmed);
    }

    let alternatives = split_comma_separated_literals(trimmed);
    if alternatives.len() > 1 {
        return Ok(DmnUnaryTest::AnyOf(
            alternatives
                .into_iter()
                .map(parse_unary_test)
                .collect::<Result<Vec<_>, _>>()?,
        ));
    }

    // Date aliases must be evaluated per row, so they are checked before the
    // literal/comparison paths would freeze them into a deploy-time constant.
    if let Some(expression) = parse_deferred_date_unary_test(trimmed)? {
        return Ok(expression);
    }

    if let Some(expression) = parse_string_function_unary_test(trimmed)? {
        return Ok(expression);
    }

    if let Some(expression) = parse_list_contains_unary_test(trimmed)? {
        return Ok(expression);
    }

    if let Some(expression) = parse_in_list_unary_test(trimmed)? {
        return Ok(expression);
    }

    if let Some(expression) = parse_string_transform_unary_test(trimmed)? {
        return Ok(expression);
    }

    if is_range_like(trimmed) {
        return parse_feel_range(trimmed);
    }

    if let Some((operator, operand)) = comparison_parts(trimmed) {
        let expected = parse_literal_expression(operand)?;
        return match operator {
            ">" => Ok(DmnUnaryTest::GreaterThan(expected)),
            ">=" => Ok(DmnUnaryTest::GreaterThanOrEqual(expected)),
            "<" => Ok(DmnUnaryTest::LessThan(expected)),
            "<=" => Ok(DmnUnaryTest::LessThanOrEqual(expected)),
            other => {
                return Err(crate::error::DmnError::unsupported(
                    "unary test comparison operator",
                    format!(
                        "comparison_parts returned unsupported operator '{other}'; expected one of '>', '>=', '<', '<='"
                    ),
                ));
            }
        };
    }

    // JUEL spells equality `==`. Java's pre-parser leaves an entry that already
    // starts with an operator untouched and hands it to the expression manager
    // (ELInputEntryExpressionPreParser.java:53-62), so `== x` is as valid as the
    // FEEL `= x`. Checked before the single `=` so the second character is not
    // swallowed into the operand.
    if let Some(operand) = trimmed.strip_prefix("==") {
        return Ok(DmnUnaryTest::Equals(parse_literal_expression(
            operand.trim(),
        )?));
    }

    if let Some(operand) = trimmed.strip_prefix('=') {
        return Ok(DmnUnaryTest::Equals(parse_literal_expression(
            operand.trim(),
        )?));
    }

    if let Some(operand) = trimmed.strip_prefix("!=") {
        return Ok(DmnUnaryTest::NotEquals(parse_literal_expression(
            operand.trim(),
        )?));
    }

    if trimmed.contains("..") || trimmed.contains(',') || trimmed.starts_with('[') {
        return Err(crate::error::DmnError::unsupported(
            "unary test",
            format!("unsupported unary test '{trimmed}' in owned M15 subset"),
        ));
    }

    Ok(DmnUnaryTest::Equals(parse_literal_expression(trimmed)?))
}

fn parse_not_unary_test(text: &str) -> Result<DmnUnaryTest, crate::error::DmnError> {
    let unsupported = || {
        crate::error::DmnError::unsupported(
            "unary test",
            format!(
                "unsupported unary test '{text}' in owned M15 subset; only not(<single supported unary test>) is supported"
            ),
        )
    };

    let argument = strip_function_argument(text, "not").ok_or_else(unsupported)?;
    let argument = argument.trim();
    if argument.is_empty() {
        return Err(unsupported());
    }

    // Support not("blocked", "closed") -> And(Not(Equals("blocked")), Not(Equals("closed")))
    let alternatives = split_comma_separated_literals(argument);
    if alternatives.len() > 1 {
        let not_tests = alternatives
            .into_iter()
            .map(|alt| {
                let expr = parse_unary_test(alt)?;
                match expr {
                    DmnUnaryTest::Any | DmnUnaryTest::AnyOf(_) => Err(unsupported()),
                    expr => Ok(DmnUnaryTest::Not(Box::new(expr))),
                }
            })
            .collect::<Result<Vec<_>, _>>()?;
        return Ok(DmnUnaryTest::And(not_tests));
    }

    if is_unsupported_not_argument(argument) {
        return Err(unsupported());
    }

    let expression = parse_unary_test(argument)?;
    match expression {
        DmnUnaryTest::Any | DmnUnaryTest::AnyOf(_) => Err(unsupported()),
        // Allow nested not: not(not("vip")) -> the inner not is already parsed
        expression => Ok(DmnUnaryTest::Not(Box::new(expression))),
    }
}

fn is_not_unary_test_candidate(text: &str) -> bool {
    text.strip_prefix("not")
        .is_some_and(|remaining| remaining.trim_start().starts_with('('))
}

fn strip_function_argument<'a>(text: &'a str, function_name: &str) -> Option<&'a str> {
    let remaining = text.strip_prefix(function_name)?.trim_start();
    let remaining = remaining.strip_prefix('(')?;
    let mut quote = None;
    let mut depth = 1usize;

    for (index, character) in remaining.char_indices() {
        match (quote, character) {
            (Some(active), current) if current == active => quote = None,
            (None, '\'' | '"') => quote = Some(character),
            (None, '(') => depth += 1,
            (None, ')') => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return (index + character.len_utf8() == remaining.len())
                        .then_some(&remaining[..index]);
                }
            }
            _ => {}
        }
    }

    None
}

fn is_unsupported_not_argument(argument: &str) -> bool {
    let trimmed = argument.trim();
    if trimmed == "-" {
        return true;
    }
    // Allow nested not: not(not("vip")) should be supported
    if trimmed.starts_with("not") {
        // Check if it's a valid nested not expression
        if is_not_unary_test_candidate(trimmed) {
            return false; // Allow nested not
        }
        return true;
    }
    if comparison_parts(trimmed).is_some()
        || is_range_like(trimmed)
        || parse_string_function_unary_test(trimmed)
            .ok()
            .flatten()
            .is_some()
        || parse_string_transform_unary_test(trimmed)
            .ok()
            .flatten()
            .is_some()
        || parse_list_contains_unary_test(trimmed)
            .ok()
            .flatten()
            .is_some()
        || parse_in_list_unary_test(trimmed).ok().flatten().is_some()
        || trimmed.starts_with('=')
        || trimmed.starts_with("!=")
        || parse_quoted_string_literal(trimmed).is_some()
        || trimmed.eq_ignore_ascii_case("null")
        || trimmed.eq_ignore_ascii_case("true")
        || trimmed.eq_ignore_ascii_case("false")
        || parse_feel_temporal_constructor(trimmed).is_some()
        || parse_feel_duration_constructor(trimmed).is_some()
        || serde_json::from_str::<Value>(trimmed).is_ok_and(|value| value.is_number())
    {
        return false;
    }

    true
}

fn parse_string_function_unary_test(
    text: &str,
) -> Result<Option<DmnUnaryTest>, crate::error::DmnError> {
    for (function_name, function) in [
        ("ends with", DmnStringFunction::EndsWith),
        ("starts with", DmnStringFunction::StartsWith),
        ("contains", DmnStringFunction::Contains),
        ("matches", DmnStringFunction::Matches),
    ] {
        let Some(arguments) = text
            .strip_prefix(function_name)
            .and_then(|remaining| remaining.strip_prefix('('))
            .and_then(|remaining| remaining.strip_suffix(')'))
        else {
            continue;
        };

        let arguments = split_comma_separated_literals(arguments);
        if arguments.len() != 2 || arguments[0] != "?" {
            return Err(unsupported_string_function_unary_test(text));
        }

        let needle = parse_quoted_string_literal(arguments[1])
            .ok_or_else(|| unsupported_string_function_unary_test(text))?;
        if function == DmnStringFunction::Matches {
            regex::Regex::new(&needle).map_err(|error| {
                crate::error::DmnError::validation(format!(
                    "invalid matches regex '{needle}' in unary test '{text}': {error}"
                ))
            })?;
        }
        return Ok(Some(DmnUnaryTest::StringFunction { function, needle }));
    }

    if text.starts_with("contains(")
        || text.starts_with("starts with(")
        || text.starts_with("ends with(")
        || text.starts_with("matches(")
    {
        return Err(unsupported_string_function_unary_test(text));
    }

    Ok(None)
}

fn parse_list_contains_unary_test(
    text: &str,
) -> Result<Option<DmnUnaryTest>, crate::error::DmnError> {
    let Some(arguments) = text
        .strip_prefix("list contains")
        .and_then(|remaining| remaining.trim_start().strip_prefix('('))
        .and_then(|remaining| remaining.strip_suffix(')'))
    else {
        if text.starts_with("list contains(") || text.starts_with("list contains ") {
            return Err(unsupported_list_contains_unary_test(text));
        }
        return Ok(None);
    };

    let arguments = split_comma_separated_literals(arguments);
    if arguments.len() != 2 || arguments[0].trim() != "?" {
        return Err(unsupported_list_contains_unary_test(text));
    }

    let needle = parse_list_contains_needle(arguments[1].trim(), text)?;
    Ok(Some(DmnUnaryTest::ListContains { needle }))
}

fn parse_list_contains_needle(
    text: &str,
    full_expression: &str,
) -> Result<DmnListContainsNeedle, crate::error::DmnError> {
    let trimmed = text.trim();
    if trimmed.is_empty() || trimmed == "?" {
        return Err(unsupported_list_contains_unary_test(full_expression));
    }

    // Explicit literals (quoted strings, booleans, null, numbers, collections,
    // FEEL temporal/duration constructors) stay literal values.
    if is_list_contains_literal_token(trimmed) {
        return Ok(DmnListContainsNeedle::Literal(parse_literal_expression(
            trimmed,
        )?));
    }

    // Simple unquoted identifier → variable reference from execution inputs.
    if is_simple_feel_name(trimmed) {
        return Ok(DmnListContainsNeedle::Variable(trimmed.to_string()));
    }

    Err(unsupported_list_contains_unary_test(full_expression))
}

fn is_list_contains_literal_token(text: &str) -> bool {
    if parse_quoted_string_literal(text).is_some()
        || text.eq_ignore_ascii_case("null")
        || text.eq_ignore_ascii_case("true")
        || text.eq_ignore_ascii_case("false")
        || parse_feel_temporal_constructor(text).is_some()
        || parse_feel_duration_constructor(text).is_some()
    {
        return true;
    }
    serde_json::from_str::<Value>(text)
        .is_ok_and(|value| value.is_number() || value.is_array() || value.is_object())
}

/// Simple FEEL name token: `[A-Za-z_][A-Za-z0-9_]*`.
fn is_simple_feel_name(text: &str) -> bool {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) if first.is_ascii_alphabetic() || first == '_' => {
            chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
        }
        _ => false,
    }
}

fn unsupported_list_contains_unary_test(text: &str) -> crate::error::DmnError {
    crate::error::DmnError::unsupported(
        "unary test",
        format!(
            "unsupported list contains unary test '{text}'; only list contains(?, <literal or simple variable>) is supported"
        ),
    )
}

/// Parse FEEL membership unary tests:
/// - `? in ("a", "b")`
/// - `in (1, 2, 3)`
fn parse_in_list_unary_test(text: &str) -> Result<Option<DmnUnaryTest>, crate::error::DmnError> {
    let trimmed = text.trim();
    let list_body = if let Some(remaining) = trimmed.strip_prefix('?') {
        let remaining = remaining.trim_start();
        let Some(after_in) = remaining.strip_prefix("in") else {
            return Ok(None);
        };
        let after_in = after_in.trim_start();
        let Some(body) = after_in.strip_prefix('(').and_then(|s| s.strip_suffix(')')) else {
            return Err(unsupported_in_list_unary_test(text));
        };
        body
    } else if let Some(remaining) = trimmed.strip_prefix("in") {
        // Reject `instance of` and similar identifiers.
        if remaining.starts_with(|c: char| c.is_ascii_alphanumeric() || c == '_') {
            return Ok(None);
        }
        let remaining = remaining.trim_start();
        let Some(body) = remaining
            .strip_prefix('(')
            .and_then(|s| s.strip_suffix(')'))
        else {
            if remaining.starts_with('(') {
                return Err(unsupported_in_list_unary_test(text));
            }
            return Ok(None);
        };
        body
    } else {
        return Ok(None);
    };

    let members = split_comma_separated_literals(list_body);
    if members.is_empty() {
        return Err(unsupported_in_list_unary_test(text));
    }

    let mut values = Vec::with_capacity(members.len());
    for member in members {
        values.push(parse_literal_expression(member.trim())?);
    }

    Ok(Some(DmnUnaryTest::InList { values }))
}

fn unsupported_in_list_unary_test(text: &str) -> crate::error::DmnError {
    crate::error::DmnError::unsupported(
        "unary test",
        format!(
            "unsupported in-list unary test '{text}'; only ? in (<literal>, ...) / in (<literal>, ...) are supported"
        ),
    )
}

fn parse_string_transform_unary_test(
    text: &str,
) -> Result<Option<DmnUnaryTest>, crate::error::DmnError> {
    if let Some(expression) = parse_string_length_transform_unary_test(text)? {
        return Ok(Some(expression));
    }
    if let Some(expression) = parse_substring_transform_unary_test(text)? {
        return Ok(Some(expression));
    }
    if let Some(expression) = parse_replace_transform_unary_test(text)? {
        return Ok(Some(expression));
    }

    for (function_name, transform) in [
        ("lower case", DmnStringTransform::LowerCase),
        ("upper case", DmnStringTransform::UpperCase),
    ] {
        let Some(remaining) = text.strip_prefix(function_name) else {
            continue;
        };
        let Some(arguments) = remaining.strip_prefix('(') else {
            return Err(unsupported_string_transform_unary_test(text));
        };
        let Some((argument, comparison)) = arguments.split_once(')') else {
            return Err(unsupported_string_transform_unary_test(text));
        };
        if argument.trim() != "?" {
            return Err(unsupported_string_transform_unary_test(text));
        }
        let Some(expected) = comparison.trim_start().strip_prefix('=') else {
            return Err(unsupported_string_transform_unary_test(text));
        };
        let expected = parse_quoted_string_literal(expected)
            .ok_or_else(|| unsupported_string_transform_unary_test(text))?;
        return Ok(Some(DmnUnaryTest::StringTransform {
            transform,
            expected,
        }));
    }

    if text.starts_with("lower case")
        || text.starts_with("upper case")
        || text.starts_with("string length")
        || text.starts_with("substring")
        || text.starts_with("replace")
    {
        return Err(unsupported_string_transform_unary_test(text));
    }

    Ok(None)
}

fn parse_string_length_transform_unary_test(
    text: &str,
) -> Result<Option<DmnUnaryTest>, crate::error::DmnError> {
    let Some(remaining) = text.strip_prefix("string length") else {
        return Ok(None);
    };
    let Some(arguments) = remaining.strip_prefix('(') else {
        return Err(unsupported_string_transform_unary_test(text));
    };
    let Some((argument, comparison)) = arguments.split_once(')') else {
        return Err(unsupported_string_transform_unary_test(text));
    };
    if argument.trim() != "?" {
        return Err(unsupported_string_transform_unary_test(text));
    }

    let Some((operator, expected)) = comparison_parts(comparison.trim()) else {
        return Err(unsupported_string_transform_unary_test(text));
    };
    let expected = parse_literal_expression(expected)?;
    if !expected.is_number() {
        return Err(unsupported_string_transform_unary_test(text));
    }

    Ok(Some(DmnUnaryTest::StringTransformComparison {
        transform: DmnStringTransform::StringLength,
        operator: parse_comparison_operator(operator)?,
        expected,
    }))
}

fn parse_substring_transform_unary_test(
    text: &str,
) -> Result<Option<DmnUnaryTest>, crate::error::DmnError> {
    let Some((arguments, comparison)) = split_function_call_comparison(text, "substring") else {
        return Ok(None);
    };
    let arguments = split_comma_separated_literals(arguments);
    if !(arguments.len() == 2 || arguments.len() == 3) || arguments[0] != "?" {
        return Err(unsupported_string_transform_unary_test(text));
    }

    let start = arguments[1]
        .parse::<i32>()
        .map_err(|_| unsupported_string_transform_unary_test(text))?;
    let length = arguments
        .get(2)
        .map(|value| {
            value
                .parse::<i32>()
                .map_err(|_| unsupported_string_transform_unary_test(text))
        })
        .transpose()?;
    let expected = parse_transform_equals_literal(text, comparison)?;

    Ok(Some(DmnUnaryTest::Substring {
        start,
        length,
        expected,
    }))
}

fn parse_replace_transform_unary_test(
    text: &str,
) -> Result<Option<DmnUnaryTest>, crate::error::DmnError> {
    let Some((arguments, comparison)) = split_function_call_comparison(text, "replace") else {
        return Ok(None);
    };
    let arguments = split_comma_separated_literals(arguments);
    if !(arguments.len() == 3 || arguments.len() == 4) || arguments[0] != "?" {
        return Err(unsupported_string_transform_unary_test(text));
    }

    let pattern = parse_quoted_string_literal(arguments[1])
        .ok_or_else(|| unsupported_string_transform_unary_test(text))?;
    let replacement = parse_quoted_string_literal(arguments[2])
        .ok_or_else(|| unsupported_string_transform_unary_test(text))?;
    let flags = arguments
        .get(3)
        .map(|value| {
            parse_quoted_string_literal(value)
                .ok_or_else(|| unsupported_string_transform_unary_test(text))
        })
        .transpose()?;
    let expected = parse_transform_equals_literal(text, comparison)?;

    Ok(Some(DmnUnaryTest::Replace {
        pattern,
        replacement,
        flags,
        expected,
    }))
}

fn split_function_call_comparison<'a>(
    text: &'a str,
    function_name: &str,
) -> Option<(&'a str, &'a str)> {
    let remaining = text.strip_prefix(function_name)?.trim_start();
    let remaining = remaining.strip_prefix('(')?;
    let mut quote = None;
    let mut depth = 1usize;

    for (index, character) in remaining.char_indices() {
        match (quote, character) {
            (Some(active), current) if current == active => quote = None,
            (None, '\'' | '"') => quote = Some(character),
            (None, '(') => depth += 1,
            (None, ')') => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    let arguments = &remaining[..index];
                    let comparison = &remaining[index + character.len_utf8()..];
                    return Some((arguments, comparison));
                }
            }
            _ => {}
        }
    }

    None
}

fn parse_transform_equals_literal(
    text: &str,
    comparison: &str,
) -> Result<String, crate::error::DmnError> {
    let expected = comparison
        .trim_start()
        .strip_prefix('=')
        .ok_or_else(|| unsupported_string_transform_unary_test(text))?;
    parse_quoted_string_literal(expected)
        .ok_or_else(|| unsupported_string_transform_unary_test(text))
}

fn parse_comparison_operator(
    operator: &str,
) -> Result<DmnComparisonOperator, crate::error::DmnError> {
    match operator {
        ">" => Ok(DmnComparisonOperator::GreaterThan),
        ">=" => Ok(DmnComparisonOperator::GreaterThanOrEqual),
        "<" => Ok(DmnComparisonOperator::LessThan),
        "<=" => Ok(DmnComparisonOperator::LessThanOrEqual),
        other => Err(crate::error::DmnError::unsupported(
            "comparison operator",
            format!(
                "comparison_parts returned unsupported operator '{other}'; expected one of '>', '>=', '<', '<='"
            ),
        )),
    }
}

fn unsupported_string_function_unary_test(text: &str) -> crate::error::DmnError {
    crate::error::DmnError::unsupported(
        "unary test",
        format!(
            "unsupported string function unary test '{text}'; only contains(?, \"literal\"), starts with(?, \"literal\"), ends with(?, \"literal\"), and matches(?, \"regex\") are supported"
        ),
    )
}

fn unsupported_string_transform_unary_test(text: &str) -> crate::error::DmnError {
    crate::error::DmnError::unsupported(
        "unary test",
        format!(
            "unsupported string transform unary test '{text}'; only lower case(?) = \"literal\", upper case(?) = \"literal\", and string length(?) <number comparison> are supported"
        ),
    )
}

fn is_range_like(text: &str) -> bool {
    text.contains("..") || text.starts_with('[') || text.starts_with('(')
}

fn parse_feel_range(text: &str) -> Result<DmnUnaryTest, crate::error::DmnError> {
    let mut chars = text.chars();
    let start_delimiter = chars
        .next()
        .ok_or_else(|| malformed_range_error(text, "missing start delimiter"))?;
    let end_delimiter = text
        .chars()
        .next_back()
        .ok_or_else(|| malformed_range_error(text, "missing end delimiter"))?;

    let start_inclusive = match start_delimiter {
        '[' => true,
        '(' => false,
        _ => return Err(malformed_range_error(text, "expected '[' or '('")),
    };
    let end_inclusive = match end_delimiter {
        ']' => true,
        ')' => false,
        _ => return Err(malformed_range_error(text, "expected ']' or ')'")),
    };

    let inner = &text[start_delimiter.len_utf8()..text.len() - end_delimiter.len_utf8()];
    let Some((start_text, end_text)) = inner.split_once("..") else {
        return Err(malformed_range_error(text, "missing '..' delimiter"));
    };
    if end_text.contains("..") {
        return Err(malformed_range_error(text, "multiple '..' delimiters"));
    }

    let start_text = start_text.trim();
    let end_text = end_text.trim();
    if start_text.is_empty() || end_text.is_empty() {
        return Err(malformed_range_error(text, "range endpoints are required"));
    }

    let start = parse_literal_expression(start_text)?;
    let end = parse_literal_expression(end_text)?;

    Ok(DmnUnaryTest::Range {
        start,
        end,
        start_inclusive,
        end_inclusive,
    })
}

fn malformed_range_error(text: &str, reason: &str) -> crate::error::DmnError {
    crate::error::DmnError::validation(format!("malformed unary range '{text}': {reason}"))
}

fn comparison_parts(text: &str) -> Option<(&'static str, &str)> {
    for operator in [">=", "<=", ">", "<"] {
        if let Some(operand) = text.strip_prefix(operator) {
            return Some((operator, operand.trim()));
        }
    }
    None
}

/// Coerce a value to a date for the `fn_*` aliases. Java accepts `Date`,
/// `LocalDate` and `yyyy-MM-dd` strings (`DateUtil.java:33-46`); Rust's temporal
/// values are strings, so date, datetime and RFC3339 spellings are all narrowed
/// to a calendar date.
fn parse_alias_date(value: &Value) -> Result<NaiveDate, crate::error::DmnError> {
    let text = value.as_str().ok_or_else(|| {
        crate::error::DmnError::execution(format!(
            "date function expects a date string, got {value}"
        ))
    })?;
    let text = text.trim();

    if let Ok(date) = NaiveDate::parse_from_str(text, "%Y-%m-%d") {
        return Ok(date);
    }
    if let Ok(datetime) = DateTime::parse_from_rfc3339(text) {
        return Ok(datetime.date_naive());
    }
    if let Ok(datetime) = NaiveDateTime::parse_from_str(text, "%Y-%m-%dT%H:%M:%S") {
        return Ok(datetime.date());
    }

    Err(crate::error::DmnError::execution(format!(
        "cannot parse '{text}' as a date (expected yyyy-MM-dd)"
    )))
}

/// Java `DateUtil.intValue` accepts an Integer or anything whose `toString`
/// parses as one (`DateUtil.java:73-82`).
fn alias_int_value(value: &Value) -> Result<i32, crate::error::DmnError> {
    if let Some(number) = value.as_i64() {
        return i32::try_from(number).map_err(|_| {
            crate::error::DmnError::execution(format!("date shift amount {number} out of range"))
        });
    }
    if let Some(text) = value.as_str()
        && let Ok(number) = text.trim().parse::<i32>()
    {
        return Ok(number);
    }

    Err(crate::error::DmnError::execution(format!(
        "date shift amount must be an integer, got {value}"
    )))
}

/// Add a (possibly negative) number of months, clamping the day to the target
/// month's length — Joda `plusMonths`/`minusMonths` semantics
/// (`DateUtil.java:52-54`, `:62-64`), e.g. Jan 31 + 1 month = Feb 28.
fn shift_months(date: NaiveDate, months: i64) -> Result<NaiveDate, crate::error::DmnError> {
    let overflow =
        || crate::error::DmnError::execution("date shift overflowed the supported range");

    let total = (date.year() as i64) * 12 + (date.month0() as i64) + months;
    let year = i32::try_from(total.div_euclid(12)).map_err(|_| overflow())?;
    let month = total.rem_euclid(12) as u32 + 1;

    let last_day = NaiveDate::from_ymd_opt(year, month, 1)
        .ok_or_else(overflow)?
        .checked_add_months(chrono::Months::new(1))
        .ok_or_else(overflow)?
        .pred_opt()
        .ok_or_else(overflow)?
        .day();

    NaiveDate::from_ymd_opt(year, month, date.day().min(last_day)).ok_or_else(overflow)
}

/// Date-function aliases the Java pre-parser rewrites to `date:*` EL functions
/// (`ELInputEntryExpressionPreParser.java:26-29`). Rust registers them natively
/// on the FEEL engine instead of doing a textual rewrite, because `date:toDate`
/// is JUEL prefix syntax that the FEEL grammar cannot lex.
pub(crate) const DATE_ALIAS_FUNCTIONS: [&str; 4] =
    ["fn_date", "fn_now", "fn_addDate", "fn_subtractDate"];

/// Whether the text calls one of the date aliases, so the operand has to be
/// evaluated per row rather than frozen at deploy time (`fn_now()` in particular).
fn contains_date_alias_call(text: &str) -> bool {
    DATE_ALIAS_FUNCTIONS.iter().any(|alias| {
        text.match_indices(alias).any(|(index, _)| {
            // Reject identifier suffixes (`fn_dateish`) but allow a call:
            // the next non-space character must open the argument list.
            let before_ok = index == 0
                || !matches!(text.as_bytes()[index - 1], b'_' | b'-')
                    && !text[..index].ends_with(|c: char| c.is_alphanumeric());
            let after = text[index + alias.len()..].trim_start();
            before_ok && after.starts_with('(')
        })
    })
}

/// Operand contains a date alias → defer the comparison to execution time.
/// Java reaches the same place by rewriting to `date:*` and letting JUEL
/// evaluate the whole `#{input <op> date:now()}` expression per row.
fn parse_deferred_date_unary_test(
    text: &str,
) -> Result<Option<DmnUnaryTest>, crate::error::DmnError> {
    if !contains_date_alias_call(text) {
        return Ok(None);
    }

    // Operator handling mirrors the Java pre-parser: a leading comparison
    // operator is preserved, anything else gets an implicit `==`
    // (`ELInputEntryExpressionPreParser.java:53-62`).
    let (operator, operand) = if let Some(rest) = text.strip_prefix(">=") {
        (DmnDeferredOperator::GreaterThanOrEqual, rest)
    } else if let Some(rest) = text.strip_prefix("<=") {
        (DmnDeferredOperator::LessThanOrEqual, rest)
    } else if let Some(rest) = text.strip_prefix("!=") {
        (DmnDeferredOperator::NotEquals, rest)
    } else if let Some(rest) = text.strip_prefix("==") {
        (DmnDeferredOperator::Equals, rest)
    } else if let Some(rest) = text.strip_prefix('>') {
        (DmnDeferredOperator::GreaterThan, rest)
    } else if let Some(rest) = text.strip_prefix('<') {
        (DmnDeferredOperator::LessThan, rest)
    } else if let Some(rest) = text.strip_prefix('=') {
        (DmnDeferredOperator::Equals, rest)
    } else {
        (DmnDeferredOperator::Equals, text)
    };

    let operand = operand.trim();
    if operand.is_empty() {
        return Err(crate::error::DmnError::unsupported(
            "unary test",
            format!("date alias unary test '{text}' has no operand"),
        ));
    }

    Ok(Some(DmnUnaryTest::DeferredComparison {
        operator,
        source: operand.to_string(),
    }))
}

/// Java treats an entry containing `#{` or `${` *and* `}` as a complete EL
/// expression and returns it untouched (`ELInputEntryExpressionPreParser.java:31-33`).
fn parse_el_condition_unary_test(
    text: &str,
) -> Result<Option<DmnUnaryTest>, crate::error::DmnError> {
    if !(text.contains("#{") || text.contains("${")) || !text.contains('}') {
        return Ok(None);
    }

    // Rust evaluates FEEL, not JUEL: only a single expression filling the whole
    // entry can be peeled. Embedded/templated EL (`a ${x} b`) has no FEEL
    // equivalent, so it is a hard deploy-time error rather than a silent match.
    let inner = text
        .strip_prefix("#{")
        .or_else(|| text.strip_prefix("${"))
        .and_then(|rest| rest.strip_suffix('}'))
        .map(str::trim)
        .filter(|inner| !inner.is_empty() && !inner.contains("#{") && !inner.contains("${"))
        .ok_or_else(|| {
            crate::error::DmnError::unsupported(
                "unary test",
                format!(
                    "input entry '{text}' is not a single `#{{...}}` / `${{...}}` expression; \
                     embedded or templated EL is not supported by the FEEL subset"
                ),
            )
        })?;

    Ok(Some(DmnUnaryTest::ElCondition {
        source: inner.to_string(),
    }))
}

/// Java `.property` shorthand (`ELInputEntryExpressionPreParser.java:42-46`).
/// Only taken when the text starts with `.` followed by an identifier — a
/// leading `.` before a digit is a bare decimal literal (`.5`), which Java
/// routes through the operator branch for date/number typeRefs.
fn parse_property_path_unary_test(
    text: &str,
) -> Result<Option<DmnUnaryTest>, crate::error::DmnError> {
    let Some(rest) = text.strip_prefix('.') else {
        return Ok(None);
    };
    if !rest.starts_with(|c: char| c.is_alphabetic() || c == '_') {
        return Ok(None);
    }

    // Split the dotted path from the remaining unary test: `.a.b >= 3`.
    let split = rest
        .find(|c: char| !(c.is_alphanumeric() || c == '_' || c == '.'))
        .unwrap_or(rest.len());
    let (path_text, remainder) = rest.split_at(split);

    let path: Vec<String> = path_text.split('.').map(str::to_string).collect();
    if path.iter().any(String::is_empty) {
        return Err(crate::error::DmnError::unsupported(
            "unary test",
            format!("malformed property path in unary test '{text}'"),
        ));
    }

    // A bare `.flag` is `#{input.flag}` in Java — a boolean property, not a
    // comparison. Anything else is the usual unary test against the property.
    let remainder = remainder.trim();
    let test = if remainder.is_empty() {
        DmnUnaryTest::Equals(Value::Bool(true))
    } else {
        parse_unary_test(remainder)?
    };

    Ok(Some(DmnUnaryTest::PropertyPath {
        path,
        test: Box::new(test),
    }))
}

fn parse_literal_expression(text: &str) -> Result<Value, crate::error::DmnError> {
    let trimmed = text.trim();
    if trimmed.eq_ignore_ascii_case("null") {
        return Ok(Value::Null);
    }
    if trimmed.eq_ignore_ascii_case("true") {
        return Ok(Value::Bool(true));
    }
    if trimmed.eq_ignore_ascii_case("false") {
        return Ok(Value::Bool(false));
    }
    if let Some(temporal) = parse_feel_temporal_constructor(trimmed) {
        return Ok(Value::String(temporal.to_string()));
    }
    if let Some(duration) = parse_feel_duration_constructor(trimmed) {
        return Ok(Value::String(duration.to_string()));
    }
    if let Some(value) = parse_quoted_string_literal(trimmed) {
        return Ok(Value::String(value));
    }
    let for_json = normalize_bare_decimal_point(trimmed);
    if let Ok(value) = serde_json::from_str::<Value>(for_json.as_ref())
        && (value.is_number() || value.is_array() || value.is_object())
    {
        return Ok(value);
    }
    Ok(Value::String(trimmed.to_string()))
}

/// JSON number syntax requires a digit on both sides of the decimal point, but
/// the JUEL scanner Java runs input entries through accepts a bare leading or
/// trailing point: `.` followed by a digit is not a DOT token, it falls into the
/// number scan and yields a FLOAT (`Scanner.java:390-394,429-430`), and a
/// trailing point with zero fraction digits is likewise a FLOAT (`:332-345`).
/// So `#{input == .5}` and `#{input == 5.}` are both numeric in Java
/// (`ELInputEntryExpressionPreParser.java:39-40,53-59` insert the implicit `==`).
///
/// Pad the missing digit so `serde_json` sees a well-formed number. Only these
/// two shapes are rewritten — a `parse::<f64>()` fallback is deliberately not
/// used, because it also accepts `nan` / `inf` / `infinity` and would silently
/// turn genuine string operands into numbers.
///
/// A leading `+` is not handled: JSON rejects `+0.5` just as it rejects the
/// plain `+5` this parser already declines, so padding it would be dead code.
fn normalize_bare_decimal_point(trimmed: &str) -> std::borrow::Cow<'_, str> {
    let (sign, digits) = match trimmed.strip_prefix('-') {
        Some(digits) => ("-", digits),
        None => ("", trimmed),
    };

    // Leading point: `.5` -> `0.5` (a digit must follow, else it is a path).
    if let Some(fraction) = digits.strip_prefix('.')
        && fraction.starts_with(|c: char| c.is_ascii_digit())
    {
        return std::borrow::Cow::Owned(format!("{sign}0.{fraction}"));
    }

    // Trailing point: `5.` -> `5.0` (integer digits only, nothing after).
    if let Some(integer) = digits.strip_suffix('.')
        && !integer.is_empty()
        && integer.bytes().all(|b| b.is_ascii_digit())
    {
        return std::borrow::Cow::Owned(format!("{sign}{integer}.0"));
    }

    std::borrow::Cow::Borrowed(trimmed)
}

fn parse_quoted_string_literal(text: &str) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.len() < 2 {
        return None;
    }

    let quote = trimmed.chars().next()?;
    if quote != '"' && quote != '\'' {
        return None;
    }
    if !trimmed.ends_with(quote) {
        return None;
    }

    let value = &trimmed[quote.len_utf8()..trimmed.len() - quote.len_utf8()];
    if value.contains(quote) {
        return None;
    }

    Some(value.to_string())
}

fn parse_feel_temporal_constructor(text: &str) -> Option<&str> {
    for function_name in ["date and time", "dateTime", "date", "time"] {
        if let Some(argument) = parse_feel_string_constructor_argument(text, function_name) {
            return Some(argument);
        }
    }
    None
}

fn parse_feel_duration_constructor(text: &str) -> Option<&str> {
    parse_feel_string_constructor_argument(text, "duration")
}

fn parse_feel_string_constructor_argument<'a>(
    text: &'a str,
    function_name: &str,
) -> Option<&'a str> {
    let argument = text
        .strip_prefix(function_name)?
        .strip_prefix('(')?
        .strip_suffix(')')?
        .trim();
    let quote = argument.chars().next()?;
    if quote != '"' && quote != '\'' {
        return None;
    }
    let duration = argument.strip_prefix(quote)?.strip_suffix(quote)?;
    if duration.contains(quote) {
        return None;
    }

    Some(duration)
}

fn parse_output_values(text: &str) -> Result<Vec<Value>, crate::error::DmnError> {
    split_comma_separated_literals(text)
        .into_iter()
        .map(parse_literal_expression)
        .collect()
}

fn split_comma_separated_literals(text: &str) -> Vec<&str> {
    let mut values = Vec::new();
    let mut start = 0;
    let mut quote = None;
    let mut depth = 0usize;

    for (index, character) in text.char_indices() {
        match (quote, character) {
            (Some(active), current) if current == active => quote = None,
            (None, '\'' | '"') => quote = Some(character),
            (None, '(') => depth += 1,
            (None, ')') => depth = depth.saturating_sub(1),
            (None, ',') if depth == 0 => {
                let value = text[start..index].trim();
                if !value.is_empty() {
                    values.push(value);
                }
                start = index + character.len_utf8();
            }
            _ => {}
        }
    }

    let value = text[start..].trim();
    if !value.is_empty() {
        values.push(value);
    }
    values
}

pub(crate) fn normalized_type_ref(type_ref: &str) -> String {
    let normalized = type_ref
        .trim()
        .rsplit(':')
        .next()
        .unwrap_or(type_ref)
        .to_ascii_lowercase();

    match normalized.as_str() {
        "date and time" => "datetime".to_string(),
        "day time duration" => "daytimeduration".to_string(),
        "year month duration" => "yearmonthduration".to_string(),
        _ => normalized,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TemporalTypeRef {
    Date,
    Time,
    DateTime,
    Duration,
    YearMonthDuration,
}

pub(crate) fn temporal_type_ref(type_ref: &str) -> Option<TemporalTypeRef> {
    match normalized_type_ref(type_ref).as_str() {
        "date" => Some(TemporalTypeRef::Date),
        "time" => Some(TemporalTypeRef::Time),
        "datetime" => Some(TemporalTypeRef::DateTime),
        "duration" | "daytimeduration" => Some(TemporalTypeRef::Duration),
        "yearmonthduration" => Some(TemporalTypeRef::YearMonthDuration),
        _ => None,
    }
}

pub(crate) fn normalize_temporal_value(type_ref: &str, value: &Value) -> Option<Value> {
    if value.is_null() {
        return Some(Value::Null);
    }

    let temporal_type = temporal_type_ref(type_ref)?;
    let text = value.as_str()?.trim();
    if text.is_empty() {
        return None;
    }

    let normalized = match temporal_type {
        TemporalTypeRef::Date => NaiveDate::parse_from_str(text, "%Y-%m-%d")
            .ok()?
            .format("%Y-%m-%d")
            .to_string(),
        TemporalTypeRef::Time => NaiveTime::parse_from_str(text, "%H:%M:%S")
            .ok()?
            .format("%H:%M:%S")
            .to_string(),
        TemporalTypeRef::DateTime => normalize_datetime_text(text)?,
        TemporalTypeRef::Duration => normalize_day_time_duration_text(text)?,
        TemporalTypeRef::YearMonthDuration => normalize_year_month_duration_text(text)?,
    };

    Some(Value::String(normalized))
}

pub(crate) fn compare_temporal_values(
    type_ref: &str,
    actual: &Value,
    expected: &Value,
) -> Option<std::cmp::Ordering> {
    let temporal_type = temporal_type_ref(type_ref)?;
    let actual = normalize_temporal_value(type_ref, actual)?;
    let expected = normalize_temporal_value(type_ref, expected)?;
    let actual = actual.as_str()?;
    let expected = expected.as_str()?;

    match temporal_type {
        TemporalTypeRef::Date => Some(
            NaiveDate::parse_from_str(actual, "%Y-%m-%d")
                .ok()?
                .cmp(&NaiveDate::parse_from_str(expected, "%Y-%m-%d").ok()?),
        ),
        TemporalTypeRef::Time => Some(
            NaiveTime::parse_from_str(actual, "%H:%M:%S")
                .ok()?
                .cmp(&NaiveTime::parse_from_str(expected, "%H:%M:%S").ok()?),
        ),
        TemporalTypeRef::DateTime => compare_datetime_text(actual, expected),
        TemporalTypeRef::Duration => Some(
            parse_day_time_duration_nanos(actual)?.cmp(&parse_day_time_duration_nanos(expected)?),
        ),
        TemporalTypeRef::YearMonthDuration => Some(
            parse_year_month_duration_months(actual)?
                .cmp(&parse_year_month_duration_months(expected)?),
        ),
    }
}

#[derive(Debug)]
enum ComparableDateTime {
    Local(NaiveDateTime),
    Instant(DateTime<Utc>),
}

fn normalize_datetime_text(text: &str) -> Option<String> {
    if has_datetime_offset(text) {
        return DateTime::parse_from_rfc3339(text)
            .ok()
            .map(|date_time| date_time.with_timezone(&Utc))
            .map(|date_time| date_time.to_rfc3339_opts(SecondsFormat::AutoSi, true));
    }

    parse_local_datetime(text).map(format_local_datetime)
}

fn compare_datetime_text(actual: &str, expected: &str) -> Option<std::cmp::Ordering> {
    match (
        parse_comparable_datetime(actual)?,
        parse_comparable_datetime(expected)?,
    ) {
        (ComparableDateTime::Instant(actual), ComparableDateTime::Instant(expected)) => {
            Some(actual.cmp(&expected))
        }
        (ComparableDateTime::Local(actual), ComparableDateTime::Local(expected)) => {
            Some(actual.cmp(&expected))
        }
        (ComparableDateTime::Instant(actual), ComparableDateTime::Local(expected)) => {
            Some(actual.naive_utc().cmp(&expected))
        }
        (ComparableDateTime::Local(actual), ComparableDateTime::Instant(expected)) => {
            Some(actual.cmp(&expected.naive_utc()))
        }
    }
}

fn parse_comparable_datetime(text: &str) -> Option<ComparableDateTime> {
    if has_datetime_offset(text) {
        return DateTime::parse_from_rfc3339(text)
            .ok()
            .map(|date_time| ComparableDateTime::Instant(date_time.with_timezone(&Utc)));
    }

    parse_local_datetime(text).map(ComparableDateTime::Local)
}

fn parse_local_datetime(text: &str) -> Option<NaiveDateTime> {
    NaiveDateTime::parse_from_str(text, "%Y-%m-%dT%H:%M:%S%.f").ok()
}

fn format_local_datetime(date_time: NaiveDateTime) -> String {
    if date_time.nanosecond() == 0 {
        date_time.format("%Y-%m-%dT%H:%M:%S").to_string()
    } else {
        date_time.format("%Y-%m-%dT%H:%M:%S%.f").to_string()
    }
}

fn has_datetime_offset(text: &str) -> bool {
    let Some(time_start) = text.find('T') else {
        return false;
    };
    let time = &text[time_start + 1..];
    time.ends_with('Z') || time.contains('+') || time.contains('-')
}

fn normalize_day_time_duration_text(text: &str) -> Option<String> {
    format_day_time_duration(parse_day_time_duration_nanos(text)?)
}

fn normalize_year_month_duration_text(text: &str) -> Option<String> {
    format_year_month_duration(parse_year_month_duration_months(text)?)
}

fn parse_year_month_duration_months(text: &str) -> Option<i128> {
    let mut text = text.trim();
    if text.is_empty() {
        return None;
    }

    let negative = text.starts_with('-');
    if negative {
        text = &text[1..];
    }

    let body = text.strip_prefix('P')?;
    let mut cursor = DayTimeDurationCursor::new(body);
    let mut saw_component = false;
    let mut total_months = 0_i128;

    if let Some(years) = cursor.take_number_before('Y')? {
        saw_component = true;
        total_months = total_months.checked_add(years.checked_mul(12)?)?;
    }
    if let Some(months) = cursor.take_number_before('M')? {
        saw_component = true;
        total_months = total_months.checked_add(months)?;
    }

    if !cursor.is_empty() || !saw_component {
        return None;
    }
    if negative {
        total_months.checked_neg()
    } else {
        Some(total_months)
    }
}

fn parse_day_time_duration_nanos(text: &str) -> Option<i128> {
    let mut text = text.trim();
    if text.is_empty() {
        return None;
    }

    let negative = text.starts_with('-');
    if negative {
        text = &text[1..];
    }

    let body = text.strip_prefix('P')?;
    if body.contains('W') {
        return signed_duration_nanos(parse_week_duration_nanos(body)?, negative, true);
    }

    let mut cursor = DayTimeDurationCursor::new(body);
    let mut saw_component = false;
    let mut total_nanos = 0_i128;

    if let Some(days) = cursor.take_number_before('D')? {
        saw_component = true;
        total_nanos = total_nanos.checked_add(days.checked_mul(NANOS_PER_DAY)?)?;
    }

    if cursor.is_empty() {
        return signed_duration_nanos(total_nanos, negative, saw_component);
    }

    cursor.strip_time_marker()?;
    let mut saw_time_component = false;

    if let Some(hours) = cursor.take_number_before('H')? {
        saw_component = true;
        saw_time_component = true;
        total_nanos = total_nanos.checked_add(hours.checked_mul(NANOS_PER_HOUR)?)?;
    }
    if let Some(minutes) = cursor.take_number_before('M')? {
        saw_component = true;
        saw_time_component = true;
        total_nanos = total_nanos.checked_add(minutes.checked_mul(NANOS_PER_MINUTE)?)?;
    }
    if let Some(seconds) = cursor.take_seconds_before('S')? {
        saw_component = true;
        saw_time_component = true;
        total_nanos = total_nanos.checked_add(seconds)?;
    }

    if !cursor.is_empty() {
        return None;
    }
    if !saw_time_component {
        return None;
    }

    signed_duration_nanos(total_nanos, negative, saw_component)
}

const NANOS_PER_SECOND: i128 = 1_000_000_000;
const NANOS_PER_MINUTE: i128 = 60 * NANOS_PER_SECOND;
const NANOS_PER_HOUR: i128 = 60 * NANOS_PER_MINUTE;
const NANOS_PER_DAY: i128 = 24 * NANOS_PER_HOUR;

fn parse_week_duration_nanos(text: &str) -> Option<i128> {
    let mut cursor = DayTimeDurationCursor::new(text);
    let weeks = cursor.take_number_before('W')??;
    if !cursor.is_empty() {
        return None;
    }

    weeks.checked_mul(7)?.checked_mul(NANOS_PER_DAY)
}

fn signed_duration_nanos(nanos: i128, negative: bool, saw_component: bool) -> Option<i128> {
    if !saw_component {
        return None;
    }
    if negative {
        nanos.checked_neg()
    } else {
        Some(nanos)
    }
}

struct DayTimeDurationCursor<'a> {
    text: &'a str,
}

impl<'a> DayTimeDurationCursor<'a> {
    fn new(text: &'a str) -> Self {
        Self { text }
    }

    fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    fn strip_time_marker(&mut self) -> Option<()> {
        self.text = self.text.strip_prefix('T')?;
        Some(())
    }

    fn take_number_before(&mut self, marker: char) -> Option<Option<i128>> {
        let Some(marker_index) = self.text.find(marker) else {
            return Some(None);
        };

        let number_text = &self.text[..marker_index];
        if number_text.is_empty()
            || !number_text
                .chars()
                .all(|character| character.is_ascii_digit())
        {
            return None;
        }
        let number = number_text.parse::<i128>().ok()?;
        self.text = &self.text[marker_index + marker.len_utf8()..];
        Some(Some(number))
    }

    fn take_seconds_before(&mut self, marker: char) -> Option<Option<i128>> {
        let Some(marker_index) = self.text.find(marker) else {
            return Some(None);
        };

        let seconds = parse_seconds_nanos(&self.text[..marker_index])?;
        self.text = &self.text[marker_index + marker.len_utf8()..];
        Some(Some(seconds))
    }
}

fn parse_seconds_nanos(text: &str) -> Option<i128> {
    let (whole_text, fraction_text) = match text.split_once('.') {
        Some((_, "")) => return None,
        Some((whole_text, fraction_text)) => (whole_text, fraction_text),
        None => (text, ""),
    };
    if whole_text.is_empty()
        || !whole_text
            .chars()
            .all(|character| character.is_ascii_digit())
    {
        return None;
    }
    if !fraction_text.is_empty()
        && (fraction_text.len() > 9
            || !fraction_text
                .chars()
                .all(|character| character.is_ascii_digit()))
    {
        return None;
    }

    let whole = whole_text.parse::<i128>().ok()?;
    let mut fraction = 0_i128;
    if !fraction_text.is_empty() {
        let mut padded = fraction_text.to_string();
        while padded.len() < 9 {
            padded.push('0');
        }
        fraction = padded.parse::<i128>().ok()?;
    }

    whole.checked_mul(NANOS_PER_SECOND)?.checked_add(fraction)
}

fn format_day_time_duration(nanos: i128) -> Option<String> {
    let negative = nanos < 0;
    let mut remaining = nanos.checked_abs()?;

    if remaining == 0 {
        return Some("PT0S".to_string());
    }

    let days = remaining / NANOS_PER_DAY;
    remaining %= NANOS_PER_DAY;
    let hours = remaining / NANOS_PER_HOUR;
    remaining %= NANOS_PER_HOUR;
    let minutes = remaining / NANOS_PER_MINUTE;
    remaining %= NANOS_PER_MINUTE;
    let seconds = remaining / NANOS_PER_SECOND;
    let fractional_nanos = remaining % NANOS_PER_SECOND;

    let mut normalized = String::new();
    if negative {
        normalized.push('-');
    }
    normalized.push('P');
    if days > 0 {
        normalized.push_str(&format!("{days}D"));
    }
    if hours > 0 || minutes > 0 || seconds > 0 || days == 0 {
        normalized.push('T');
        if hours > 0 {
            normalized.push_str(&format!("{hours}H"));
        }
        if minutes > 0 {
            normalized.push_str(&format!("{minutes}M"));
        }
        if seconds > 0 || fractional_nanos > 0 {
            normalized.push_str(&format_duration_seconds(seconds, fractional_nanos));
        }
    }

    Some(normalized)
}

fn format_year_month_duration(months: i128) -> Option<String> {
    let negative = months < 0;
    let months = months.checked_abs()?;

    let mut normalized = String::new();
    if negative {
        normalized.push('-');
    }
    normalized.push('P');
    normalized.push_str(&format!("{months}M"));

    Some(normalized)
}

fn format_duration_seconds(seconds: i128, fractional_nanos: i128) -> String {
    if fractional_nanos == 0 {
        return format!("{seconds}S");
    }

    let mut fraction = format!("{fractional_nanos:09}");
    while fraction.ends_with('0') {
        fraction.pop();
    }
    format!("{seconds}.{fraction}S")
}

pub(crate) fn numeric_value(value: &Value) -> Option<Value> {
    if value.is_number() {
        return Some(value.clone());
    }

    let text = value.as_str()?.trim();
    if text.is_empty() {
        return None;
    }

    serde_json::from_str::<Value>(text)
        .ok()
        .filter(Value::is_number)
}

pub(crate) fn number_to_i64(value: &Value) -> Option<i64> {
    let number = value.as_number()?;
    if let Some(integer) = number.as_i64() {
        return Some(integer);
    }

    let unsigned = number.as_u64()?;
    i64::try_from(unsigned).ok()
}

#[derive(Clone, Debug, Default)]
pub struct FeelExpressionEngine;

impl FeelExpressionEngine {
    pub fn new() -> Self {
        Self
    }

    pub fn evaluate(
        &self,
        expression: &str,
        context: &HashMap<String, Value>,
    ) -> Result<Value, crate::error::DmnError> {
        let trimmed = expression.trim();
        if trimmed.is_empty() {
            return Ok(Value::Null);
        }
        match crate::feel::evaluate(trimmed, context) {
            Ok(value) => Ok(value),
            // The compatibility evaluator still owns the long-tail function
            // catalogue while functions are migrated into the typed FEEL
            // registry. Syntax with contexts, comprehensions, quantified
            // expressions, paths, filters and ranges never reaches this path.
            Err(_) => self.parse_or_expression(trimmed, context),
        }
    }

    fn parse_or_expression(
        &self,
        expression: &str,
        context: &HashMap<String, Value>,
    ) -> Result<Value, crate::error::DmnError> {
        let parts = self.split_logical_operators(expression, "or");
        if parts.len() > 1 {
            for part in &parts {
                let value = self.parse_and_expression(part.trim(), context)?;
                if value.as_bool().unwrap_or(false) {
                    return Ok(Value::Bool(true));
                }
            }
            return Ok(Value::Bool(false));
        }
        self.parse_and_expression(expression, context)
    }

    fn parse_and_expression(
        &self,
        expression: &str,
        context: &HashMap<String, Value>,
    ) -> Result<Value, crate::error::DmnError> {
        let parts = self.split_logical_operators(expression, "and");
        if parts.len() > 1 {
            for part in &parts {
                let value = self.parse_not_expression(part.trim(), context)?;
                if !value.as_bool().unwrap_or(false) {
                    return Ok(Value::Bool(false));
                }
            }
            return Ok(Value::Bool(true));
        }
        self.parse_not_expression(expression, context)
    }

    fn parse_not_expression(
        &self,
        expression: &str,
        context: &HashMap<String, Value>,
    ) -> Result<Value, crate::error::DmnError> {
        if let Some(inner) = expression
            .strip_prefix("not(")
            .and_then(|s| s.strip_suffix(')'))
        {
            let value = self.parse_comparison(inner.trim(), context)?;
            return Ok(Value::Bool(!value.as_bool().unwrap_or(false)));
        }
        self.parse_comparison(expression, context)
    }

    fn parse_comparison(
        &self,
        expression: &str,
        context: &HashMap<String, Value>,
    ) -> Result<Value, crate::error::DmnError> {
        for op in ["!=", ">=", "<=", "=", ">", "<"] {
            if let Some((left, right)) = self.split_binary_operator(expression, op) {
                let left_val = self.parse_additive(left.trim(), context)?;
                let right_val = self.parse_additive(right.trim(), context)?;
                return self.compare_values(&left_val, &right_val, op);
            }
        }
        self.parse_additive(expression, context)
    }

    fn parse_additive(
        &self,
        expression: &str,
        context: &HashMap<String, Value>,
    ) -> Result<Value, crate::error::DmnError> {
        let mut parts: Vec<(char, &str)> = Vec::new();
        let mut depth = 0;
        let mut last = 0;
        let chars: Vec<char> = expression.chars().collect();
        let mut next_op = '+';
        let mut i = 0;
        while i < chars.len() {
            match chars[i] {
                '(' => depth += 1,
                ')' => depth -= 1,
                '+' | '-' if depth == 0 && i > 0 => {
                    parts.push((next_op, &expression[last..i]));
                    next_op = chars[i];
                    last = i + 1;
                }
                _ => {}
            }
            i += 1;
        }
        parts.push((next_op, &expression[last..]));

        if parts.len() > 1 {
            let mut result = self.parse_multiplicative(parts[0].1.trim(), context)?;
            for (op, part) in &parts[1..] {
                let right = self.parse_multiplicative(part.trim(), context)?;
                result = self.apply_arithmetic(&result, &right, *op)?;
            }
            return Ok(result);
        }
        self.parse_multiplicative(expression, context)
    }

    fn parse_multiplicative(
        &self,
        expression: &str,
        context: &HashMap<String, Value>,
    ) -> Result<Value, crate::error::DmnError> {
        let mut parts: Vec<(char, &str)> = Vec::new();
        let mut depth = 0;
        let mut last = 0;
        let chars: Vec<char> = expression.chars().collect();
        let mut next_op = '*';
        let mut i = 0;
        while i < chars.len() {
            match chars[i] {
                '(' => depth += 1,
                ')' => depth -= 1,
                '*' | '/' | '%' if depth == 0 && i > 0 => {
                    parts.push((next_op, &expression[last..i]));
                    next_op = chars[i];
                    last = i + 1;
                }
                _ => {}
            }
            i += 1;
        }
        parts.push((next_op, &expression[last..]));

        if parts.len() > 1 {
            let mut result = self.parse_unary(parts[0].1.trim(), context)?;
            for (op, part) in &parts[1..] {
                let right = self.parse_unary(part.trim(), context)?;
                result = self.apply_arithmetic(&result, &right, *op)?;
            }
            return Ok(result);
        }
        self.parse_unary(expression, context)
    }

    fn parse_unary(
        &self,
        expression: &str,
        context: &HashMap<String, Value>,
    ) -> Result<Value, crate::error::DmnError> {
        if let Some(inner) = expression.strip_prefix('-') {
            let value = self.parse_primary(inner.trim(), context)?;
            if let Some(n) = value.as_f64() {
                return Ok(Value::from(-n));
            }
            if let Some(n) = value.as_i64() {
                return Ok(Value::from(-n));
            }
            return Err(crate::error::DmnError::execution(
                "cannot negate non-numeric value",
            ));
        }
        self.parse_primary(expression, context)
    }

    fn parse_primary(
        &self,
        expression: &str,
        context: &HashMap<String, Value>,
    ) -> Result<Value, crate::error::DmnError> {
        let trimmed = expression.trim();
        if trimmed.is_empty() {
            return Ok(Value::Null);
        }

        if trimmed == "null" || trimmed == "Null" || trimmed == "NULL" {
            return Ok(Value::Null);
        }
        if trimmed == "true" || trimmed == "True" {
            return Ok(Value::Bool(true));
        }
        if trimmed == "false" || trimmed == "False" {
            return Ok(Value::Bool(false));
        }

        if let Some(inner) = trimmed.strip_prefix('(').and_then(|s| s.strip_suffix(')')) {
            return self.evaluate(inner, context);
        }

        if trimmed.starts_with('"') || trimmed.starts_with('\'') {
            return self.parse_string_literal(trimmed);
        }

        if let Ok(number) = trimmed.parse::<i64>() {
            return Ok(Value::from(number));
        }
        if let Ok(number) = trimmed.parse::<f64>() {
            return Ok(Value::from(number));
        }

        if let Some(paren_pos) = trimmed.find('(') {
            let func_name = trimmed[..paren_pos].trim();
            let args_str = trimmed[paren_pos + 1..].trim_end_matches(')');
            let args = self.split_function_args(args_str);
            return self.evaluate_function(func_name, &args, context);
        }

        if let Some(value) = context.get(trimmed) {
            return Ok(value.clone());
        }

        Ok(Value::String(trimmed.to_string()))
    }

    fn parse_string_literal(&self, expression: &str) -> Result<Value, crate::error::DmnError> {
        let quote = expression.chars().next().unwrap();
        let inner = &expression[quote.len_utf8()..expression.len() - quote.len_utf8()];
        Ok(Value::String(inner.to_string()))
    }

    fn split_logical_operators<'a>(&self, expression: &'a str, op: &str) -> Vec<&'a str> {
        let mut parts = Vec::new();
        let mut depth = 0;
        let mut last = 0;
        let op_len = op.len();
        let chars: Vec<char> = expression.chars().collect();
        let mut i = 0;

        while i < chars.len() {
            match chars[i] {
                '(' => depth += 1,
                ')' => depth -= 1,
                _ if depth == 0 && i + op_len <= chars.len() => {
                    let slice: String = chars[i..i + op_len].iter().collect();
                    if slice.eq_ignore_ascii_case(op) {
                        let before_ok = i == 0 || chars[i - 1] == ' ' || chars[i - 1] == ')';
                        let after_ok = i + op_len >= chars.len()
                            || chars[i + op_len] == ' '
                            || chars[i + op_len] == '(';
                        if before_ok && after_ok {
                            parts.push(&expression[last..i]);
                            last = i + op_len;
                            i += op_len;
                            continue;
                        }
                    }
                }
                _ => {}
            }
            i += 1;
        }
        parts.push(&expression[last..]);
        parts
    }

    fn split_binary_operator<'a>(
        &self,
        expression: &'a str,
        op: &str,
    ) -> Option<(&'a str, &'a str)> {
        let mut depth = 0;
        let chars: Vec<char> = expression.chars().collect();
        let op_len = op.len();
        let mut i = 0;

        while i < chars.len() {
            match chars[i] {
                '(' => depth += 1,
                ')' => depth -= 1,
                _ if depth == 0 && i + op_len <= chars.len() => {
                    let slice: String = chars[i..i + op_len].iter().collect();
                    if slice == op {
                        let left = &expression[..i];
                        let right = &expression[i + op_len..];
                        if !left.trim().is_empty() && !right.trim().is_empty() {
                            return Some((left, right));
                        }
                    }
                }
                _ => {}
            }
            i += 1;
        }
        None
    }

    fn apply_arithmetic(
        &self,
        left: &Value,
        right: &Value,
        op: char,
    ) -> Result<Value, crate::error::DmnError> {
        if let (Some(l), Some(r)) = (left.as_i64(), right.as_i64()) {
            return match op {
                '+' => Ok(Value::from(l + r)),
                '-' => Ok(Value::from(l - r)),
                '*' => Ok(Value::from(l * r)),
                '/' => {
                    if r == 0 {
                        Err(crate::error::DmnError::execution("division by zero"))
                    } else {
                        Ok(Value::from(l as f64 / r as f64))
                    }
                }
                '%' => {
                    if r == 0 {
                        Err(crate::error::DmnError::execution("modulo by zero"))
                    } else {
                        Ok(Value::from(l % r))
                    }
                }
                _ => Err(crate::error::DmnError::execution(format!(
                    "unknown operator '{op}'"
                ))),
            };
        }
        let l = left.as_f64().unwrap_or(0.0);
        let r = right.as_f64().unwrap_or(0.0);
        match op {
            '+' => Ok(Value::from(l + r)),
            '-' => Ok(Value::from(l - r)),
            '*' => Ok(Value::from(l * r)),
            '/' => {
                if r == 0.0 {
                    Err(crate::error::DmnError::execution("division by zero"))
                } else {
                    Ok(Value::from(l / r))
                }
            }
            '%' => {
                if r == 0.0 {
                    Err(crate::error::DmnError::execution("modulo by zero"))
                } else {
                    Ok(Value::from(l % r))
                }
            }
            _ => Err(crate::error::DmnError::execution(format!(
                "unknown operator '{op}'"
            ))),
        }
    }

    fn compare_values(
        &self,
        left: &Value,
        right: &Value,
        op: &str,
    ) -> Result<Value, crate::error::DmnError> {
        let result = match op {
            "=" => self.values_equal(left, right),
            "!=" => !self.values_equal(left, right),
            ">" => self.values_compare(left, right) == Some(std::cmp::Ordering::Greater),
            ">=" => matches!(
                self.values_compare(left, right),
                Some(std::cmp::Ordering::Greater | std::cmp::Ordering::Equal)
            ),
            "<" => self.values_compare(left, right) == Some(std::cmp::Ordering::Less),
            "<=" => matches!(
                self.values_compare(left, right),
                Some(std::cmp::Ordering::Less | std::cmp::Ordering::Equal)
            ),
            _ => false,
        };
        Ok(Value::Bool(result))
    }

    fn values_equal(&self, left: &Value, right: &Value) -> bool {
        if left.is_number()
            && right.is_number()
            && let (Some(l), Some(r)) = (left.as_f64(), right.as_f64())
        {
            return (l - r).abs() < f64::EPSILON;
        }
        left == right
    }

    fn values_compare(&self, left: &Value, right: &Value) -> Option<std::cmp::Ordering> {
        if let (Some(l), Some(r)) = (left.as_f64(), right.as_f64()) {
            return l.partial_cmp(&r);
        }
        if let (Some(l), Some(r)) = (left.as_str(), right.as_str()) {
            return Some(l.cmp(r));
        }
        None
    }

    fn split_function_args(&self, args_str: &str) -> Vec<String> {
        let mut args = Vec::new();
        let mut current = String::new();
        let mut depth = 0;
        let mut in_string = false;
        let mut string_char = ' ';

        for ch in args_str.chars() {
            if in_string {
                current.push(ch);
                if ch == string_char {
                    in_string = false;
                }
                continue;
            }
            match ch {
                '"' | '\'' => {
                    in_string = true;
                    string_char = ch;
                    current.push(ch);
                }
                '(' => {
                    depth += 1;
                    current.push(ch);
                }
                ')' => {
                    depth -= 1;
                    current.push(ch);
                }
                ',' if depth == 0 => {
                    args.push(current.trim().to_string());
                    current.clear();
                }
                _ => current.push(ch),
            }
        }
        if !current.trim().is_empty() {
            args.push(current.trim().to_string());
        }
        args
    }

    fn evaluate_function(
        &self,
        name: &str,
        args: &[String],
        context: &HashMap<String, Value>,
    ) -> Result<Value, crate::error::DmnError> {
        let evaluated_args: Vec<Value> = args
            .iter()
            .map(|a| self.evaluate(a, context))
            .collect::<Result<Vec<_>, _>>()?;

        match name {
            "abs" => self.fn_abs(&evaluated_args),
            "ceiling" | "ceil" => self.fn_ceiling(&evaluated_args),
            "floor" => self.fn_floor(&evaluated_args),
            "round" => self.fn_round(&evaluated_args),
            "sqrt" => self.fn_sqrt(&evaluated_args),
            "modulo" => self.fn_modulo(&evaluated_args),
            "decimal" => self.fn_decimal(&evaluated_args),
            "even" => self.fn_even(&evaluated_args),
            "odd" => self.fn_odd(&evaluated_args),
            "contains" => self.fn_contains(&evaluated_args),
            "starts with" => self.fn_starts_with(&evaluated_args),
            "ends with" => self.fn_ends_with(&evaluated_args),
            "matches" => self.fn_matches(&evaluated_args),
            "string length" => self.fn_string_length(&evaluated_args),
            "upper case" => self.fn_upper_case(&evaluated_args),
            "lower case" => self.fn_lower_case(&evaluated_args),
            "substring" => self.fn_substring(&evaluated_args),
            "replace" => self.fn_replace(&evaluated_args),
            "trim" => self.fn_trim(&evaluated_args),
            "append" => self.fn_append(&evaluated_args),
            "concatenate" => self.fn_concatenate(&evaluated_args),
            "count" => self.fn_count(&evaluated_args),
            "distinct values" => self.fn_distinct_values(&evaluated_args),
            "flatten" => self.fn_flatten(&evaluated_args),
            "reverse" => self.fn_reverse(&evaluated_args),
            "list contains" => self.fn_list_contains(&evaluated_args),
            "index of" => self.fn_index_of(&evaluated_args),
            "sublist" => self.fn_sublist(&evaluated_args),
            "union" => self.fn_union(&evaluated_args),
            "intersect" => self.fn_intersect(&evaluated_args),
            "except" => self.fn_except(&evaluated_args),
            "sum" => self.fn_sum(&evaluated_args),
            "mean" => self.fn_mean(&evaluated_args),
            "min" => self.fn_min(&evaluated_args),
            "max" => self.fn_max(&evaluated_args),
            "now" => Ok(Value::String(Utc::now().to_rfc3339())),
            "today" => Ok(Value::String(Utc::now().format("%Y-%m-%d").to_string())),
            // DMN date aliases. Java rewrites `fn_*` to these `date:*` EL
            // functions in the input-entry pre-parser
            // (`ELInputEntryExpressionPreParser.java:26-29`); both spellings are
            // accepted here so pre-rewritten definitions keep working.
            // Semantics follow `el/util/DateUtil.java:28-71` — all four are
            // date-only (Joda `LocalDate`), *not* datetimes.
            "fn_date" | "date:toDate" => self.fn_to_date(&evaluated_args),
            "fn_now" | "date:now" => {
                // DateUtil.now() is `new LocalDate().toDate()` — midnight today,
                // so it maps to FEEL `today`, not FEEL `now` (DateUtil.java:69-71).
                Ok(Value::String(Utc::now().format("%Y-%m-%d").to_string()))
            }
            "fn_addDate" | "date:addDate" => self.fn_shift_date(&evaluated_args, 1),
            "fn_subtractDate" | "date:subtractDate" => self.fn_shift_date(&evaluated_args, -1),
            "year" => self.fn_year(&evaluated_args),
            "month" => self.fn_month(&evaluated_args),
            "day" => self.fn_day(&evaluated_args),
            "hour" => self.fn_hour(&evaluated_args),
            "minute" => self.fn_minute(&evaluated_args),
            "second" => self.fn_second(&evaluated_args),
            _ => Err(crate::error::DmnError::unsupported(
                "FEEL function",
                format!("unsupported FEEL function '{name}'"),
            )),
        }
    }

    fn fn_abs(&self, args: &[Value]) -> Result<Value, crate::error::DmnError> {
        let n = args
            .first()
            .and_then(|v| v.as_f64())
            .ok_or_else(|| crate::error::DmnError::execution("abs requires a number"))?;
        Ok(Value::from(n.abs()))
    }

    fn fn_ceiling(&self, args: &[Value]) -> Result<Value, crate::error::DmnError> {
        let n = args
            .first()
            .and_then(|v| v.as_f64())
            .ok_or_else(|| crate::error::DmnError::execution("ceiling requires a number"))?;
        Ok(Value::from(n.ceil()))
    }

    fn fn_floor(&self, args: &[Value]) -> Result<Value, crate::error::DmnError> {
        let n = args
            .first()
            .and_then(|v| v.as_f64())
            .ok_or_else(|| crate::error::DmnError::execution("floor requires a number"))?;
        Ok(Value::from(n.floor()))
    }

    fn fn_round(&self, args: &[Value]) -> Result<Value, crate::error::DmnError> {
        let n = args
            .first()
            .and_then(|v| v.as_f64())
            .ok_or_else(|| crate::error::DmnError::execution("round requires a number"))?;
        let precision = args.get(1).and_then(|v| v.as_i64()).unwrap_or(0);
        let factor = 10f64.powi(precision as i32);
        Ok(Value::from((n * factor).round() / factor))
    }

    fn fn_sqrt(&self, args: &[Value]) -> Result<Value, crate::error::DmnError> {
        let n = args
            .first()
            .and_then(|v| v.as_f64())
            .ok_or_else(|| crate::error::DmnError::execution("sqrt requires a number"))?;
        Ok(Value::from(n.sqrt()))
    }

    fn fn_modulo(&self, args: &[Value]) -> Result<Value, crate::error::DmnError> {
        let a = args
            .first()
            .and_then(|v| v.as_f64())
            .ok_or_else(|| crate::error::DmnError::execution("modulo requires two numbers"))?;
        let b = args
            .get(1)
            .and_then(|v| v.as_f64())
            .ok_or_else(|| crate::error::DmnError::execution("modulo requires two numbers"))?;
        Ok(Value::from(a % b))
    }

    fn fn_decimal(&self, args: &[Value]) -> Result<Value, crate::error::DmnError> {
        let n = args
            .first()
            .and_then(|v| v.as_f64())
            .ok_or_else(|| crate::error::DmnError::execution("decimal requires a number"))?;
        let scale = args.get(1).and_then(|v| v.as_i64()).unwrap_or(0);
        let factor = 10f64.powi(scale as i32);
        Ok(Value::from((n * factor).round() / factor))
    }

    fn fn_even(&self, args: &[Value]) -> Result<Value, crate::error::DmnError> {
        let n = args
            .first()
            .and_then(|v| v.as_i64())
            .ok_or_else(|| crate::error::DmnError::execution("even requires an integer"))?;
        Ok(Value::Bool(n % 2 == 0))
    }

    fn fn_odd(&self, args: &[Value]) -> Result<Value, crate::error::DmnError> {
        let n = args
            .first()
            .and_then(|v| v.as_i64())
            .ok_or_else(|| crate::error::DmnError::execution("odd requires an integer"))?;
        Ok(Value::Bool(n % 2 != 0))
    }

    fn fn_contains(&self, args: &[Value]) -> Result<Value, crate::error::DmnError> {
        let haystack = args.first().and_then(|v| v.as_str()).unwrap_or("");
        let needle = args.get(1).and_then(|v| v.as_str()).unwrap_or("");
        Ok(Value::Bool(haystack.contains(needle)))
    }

    fn fn_starts_with(&self, args: &[Value]) -> Result<Value, crate::error::DmnError> {
        let haystack = args.first().and_then(|v| v.as_str()).unwrap_or("");
        let needle = args.get(1).and_then(|v| v.as_str()).unwrap_or("");
        Ok(Value::Bool(haystack.starts_with(needle)))
    }

    fn fn_ends_with(&self, args: &[Value]) -> Result<Value, crate::error::DmnError> {
        let haystack = args.first().and_then(|v| v.as_str()).unwrap_or("");
        let needle = args.get(1).and_then(|v| v.as_str()).unwrap_or("");
        Ok(Value::Bool(haystack.ends_with(needle)))
    }

    fn fn_matches(&self, args: &[Value]) -> Result<Value, crate::error::DmnError> {
        let text = args.first().and_then(|v| v.as_str()).unwrap_or("");
        let pattern = args.get(1).and_then(|v| v.as_str()).unwrap_or("");
        match regex::Regex::new(pattern) {
            Ok(re) => Ok(Value::Bool(re.is_match(text))),
            Err(e) => Err(crate::error::DmnError::execution(format!(
                "invalid regex: {e}"
            ))),
        }
    }

    fn fn_string_length(&self, args: &[Value]) -> Result<Value, crate::error::DmnError> {
        let s = args.first().and_then(|v| v.as_str()).unwrap_or("");
        Ok(Value::from(s.chars().count() as u64))
    }

    fn fn_upper_case(&self, args: &[Value]) -> Result<Value, crate::error::DmnError> {
        let s = args.first().and_then(|v| v.as_str()).unwrap_or("");
        Ok(Value::String(s.to_uppercase()))
    }

    fn fn_lower_case(&self, args: &[Value]) -> Result<Value, crate::error::DmnError> {
        let s = args.first().and_then(|v| v.as_str()).unwrap_or("");
        Ok(Value::String(s.to_lowercase()))
    }

    fn fn_substring(&self, args: &[Value]) -> Result<Value, crate::error::DmnError> {
        let s = args.first().and_then(|v| v.as_str()).unwrap_or("");
        let start = args.get(1).and_then(|v| v.as_i64()).unwrap_or(1);
        let length = args.get(2).and_then(|v| v.as_i64());
        let chars: Vec<char> = s.chars().collect();
        let char_len = chars.len() as i64;
        let start_idx = if start > 0 {
            (start - 1).min(char_len)
        } else if start < 0 {
            (char_len + start).max(0)
        } else {
            0
        } as usize;
        let end_idx = if let Some(len) = length {
            if len <= 0 {
                start_idx
            } else {
                (start_idx + len as usize).min(chars.len())
            }
        } else {
            chars.len()
        };
        let result: String = chars[start_idx..end_idx].iter().collect();
        Ok(Value::String(result))
    }

    fn fn_replace(&self, args: &[Value]) -> Result<Value, crate::error::DmnError> {
        let input = args.first().and_then(|v| v.as_str()).unwrap_or("");
        let pattern = args.get(1).and_then(|v| v.as_str()).unwrap_or("");
        let replacement = args.get(2).and_then(|v| v.as_str()).unwrap_or("");
        let flags = args.get(3).and_then(|v| v.as_str());
        let mut builder = regex::RegexBuilder::new(pattern);
        if let Some(f) = flags {
            if f.contains('i') {
                builder.case_insensitive(true);
            }
            if f.contains('s') {
                builder.dot_matches_new_line(true);
            }
            if f.contains('m') {
                builder.multi_line(true);
            }
        }
        match builder.build() {
            Ok(re) => Ok(Value::String(
                re.replace_all(input, replacement).to_string(),
            )),
            Err(e) => Err(crate::error::DmnError::execution(format!(
                "invalid regex: {e}"
            ))),
        }
    }

    fn fn_trim(&self, args: &[Value]) -> Result<Value, crate::error::DmnError> {
        let s = args.first().and_then(|v| v.as_str()).unwrap_or("");
        Ok(Value::String(s.trim().to_string()))
    }

    fn fn_append(&self, args: &[Value]) -> Result<Value, crate::error::DmnError> {
        let mut list = args
            .first()
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        if let Some(item) = args.get(1) {
            list.push(item.clone());
        }
        Ok(Value::Array(list))
    }

    fn fn_concatenate(&self, args: &[Value]) -> Result<Value, crate::error::DmnError> {
        let mut result = Vec::new();
        for arg in args {
            if let Some(arr) = arg.as_array() {
                result.extend(arr.iter().cloned());
            } else {
                result.push(arg.clone());
            }
        }
        Ok(Value::Array(result))
    }

    fn fn_count(&self, args: &[Value]) -> Result<Value, crate::error::DmnError> {
        if args.len() == 1
            && let Some(arr) = args.first().and_then(|v| v.as_array())
        {
            return Ok(Value::from(arr.len() as u64));
        }
        Ok(Value::from(args.len() as u64))
    }

    fn fn_distinct_values(&self, args: &[Value]) -> Result<Value, crate::error::DmnError> {
        let list = args
            .first()
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let mut result = Vec::new();
        for item in list {
            if !result.contains(&item) {
                result.push(item);
            }
        }
        Ok(Value::Array(result))
    }

    fn fn_flatten(&self, args: &[Value]) -> Result<Value, crate::error::DmnError> {
        let list = args
            .first()
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let mut result = Vec::new();
        for item in list {
            if let Some(arr) = item.as_array() {
                result.extend(arr.iter().cloned());
            } else {
                result.push(item);
            }
        }
        Ok(Value::Array(result))
    }

    fn fn_reverse(&self, args: &[Value]) -> Result<Value, crate::error::DmnError> {
        let mut list = args
            .first()
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        list.reverse();
        Ok(Value::Array(list))
    }

    fn fn_list_contains(&self, args: &[Value]) -> Result<Value, crate::error::DmnError> {
        let list = args
            .first()
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let item = args.get(1).cloned().unwrap_or(Value::Null);
        Ok(Value::Bool(list.contains(&item)))
    }

    fn fn_index_of(&self, args: &[Value]) -> Result<Value, crate::error::DmnError> {
        let list = args
            .first()
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let item = args.get(1).cloned().unwrap_or(Value::Null);
        let indices: Vec<Value> = list
            .iter()
            .enumerate()
            .filter(|(_, v)| **v == item)
            .map(|(i, _)| Value::from((i + 1) as u64))
            .collect();
        Ok(Value::Array(indices))
    }

    fn fn_sublist(&self, args: &[Value]) -> Result<Value, crate::error::DmnError> {
        let list = args
            .first()
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let start = args.get(1).and_then(|v| v.as_i64()).unwrap_or(1);
        let length = args.get(2).and_then(|v| v.as_i64());
        let start_idx = if start > 0 {
            (start - 1) as usize
        } else {
            (list.len() as i64 + start).max(0) as usize
        };
        let end_idx = if let Some(len) = length {
            (start_idx + len as usize).min(list.len())
        } else {
            list.len()
        };
        Ok(Value::Array(list[start_idx..end_idx].to_vec()))
    }

    fn fn_union(&self, args: &[Value]) -> Result<Value, crate::error::DmnError> {
        let mut result = Vec::new();
        for arg in args {
            if let Some(arr) = arg.as_array() {
                for item in arr {
                    if !result.contains(item) {
                        result.push(item.clone());
                    }
                }
            }
        }
        Ok(Value::Array(result))
    }

    fn fn_intersect(&self, args: &[Value]) -> Result<Value, crate::error::DmnError> {
        let first = args
            .first()
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let mut result = first;
        for arg in &args[1..] {
            if let Some(arr) = arg.as_array() {
                result.retain(|item| arr.contains(item));
            }
        }
        Ok(Value::Array(result))
    }

    fn fn_except(&self, args: &[Value]) -> Result<Value, crate::error::DmnError> {
        let first = args
            .first()
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let mut result = first;
        for arg in &args[1..] {
            if let Some(arr) = arg.as_array() {
                result.retain(|item| !arr.contains(item));
            }
        }
        Ok(Value::Array(result))
    }

    fn fn_sum(&self, args: &[Value]) -> Result<Value, crate::error::DmnError> {
        let list = args
            .first()
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let total: f64 = list.iter().filter_map(|v| v.as_f64()).sum();
        Ok(Value::from(total))
    }

    fn fn_mean(&self, args: &[Value]) -> Result<Value, crate::error::DmnError> {
        let list = args
            .first()
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let count = list.len();
        if count == 0 {
            return Err(crate::error::DmnError::execution(
                "mean requires at least one number",
            ));
        }
        let total: f64 = list.iter().filter_map(|v| v.as_f64()).sum();
        Ok(Value::from(total / count as f64))
    }

    fn fn_min(&self, args: &[Value]) -> Result<Value, crate::error::DmnError> {
        let list = args
            .first()
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        match list
            .iter()
            .filter_map(|v| v.as_f64())
            .reduce(|a, b| a.min(b))
        {
            Some(v) => Ok(Value::from(v)),
            None => Err(crate::error::DmnError::execution(
                "min requires at least one number",
            )),
        }
    }

    fn fn_max(&self, args: &[Value]) -> Result<Value, crate::error::DmnError> {
        let list = args
            .first()
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        match list
            .iter()
            .filter_map(|v| v.as_f64())
            .reduce(|a, b| a.max(b))
        {
            Some(v) => Ok(Value::from(v)),
            None => Err(crate::error::DmnError::execution(
                "max requires at least one number",
            )),
        }
    }

    /// `date:toDate` — Java parses with the fixed `yyyy-MM-dd` Joda pattern and
    /// passes `Date`/`LocalDate` inputs straight through (`DateUtil.java:28-47`).
    /// Rust keeps dates as `%Y-%m-%d` strings, so this normalises to that shape.
    fn fn_to_date(&self, args: &[Value]) -> Result<Value, crate::error::DmnError> {
        let value = args.first().ok_or_else(|| {
            // Java: "date object cannot be empty" (DateUtil.java:29-31).
            crate::error::DmnError::execution("fn_date requires a date argument")
        })?;

        Ok(Value::String(
            parse_alias_date(value)?.format("%Y-%m-%d").to_string(),
        ))
    }

    /// `date:addDate` / `date:subtractDate` — `(startDate, years, months, days)`
    /// applied in that order on a Joda `LocalDate` (`DateUtil.java:49-67`).
    /// `sign` is +1 for add and -1 for subtract.
    fn fn_shift_date(&self, args: &[Value], sign: i32) -> Result<Value, crate::error::DmnError> {
        let start = args
            .first()
            .ok_or_else(|| crate::error::DmnError::execution("date shift requires a start date"))?;
        let mut date = parse_alias_date(start)?;

        // Java's intValue() throws NumberFormatException on non-numeric input;
        // a missing argument would NPE. Both surface as execution errors here.
        let component = |index: usize| -> Result<i32, crate::error::DmnError> {
            let value = args.get(index).ok_or_else(|| {
                crate::error::DmnError::execution(
                    "date shift requires (startDate, years, months, days)",
                )
            })?;
            alias_int_value(value)
        };

        let years = component(1)? * sign;
        let months = component(2)? * sign;
        let days = component(3)? * sign;

        // Years and months are applied as two separate clamping steps, not as a
        // single (years * 12 + months) shift: Joda clamps after *each* of
        // plusYears / plusMonths (DateUtil.java:52-54), and the two differ when
        // the year shift itself clamps. 2024-02-29 +1y +1m is 2025-03-28 via
        // Joda (Feb 29 -> Feb 28 -> Mar 28) but 2025-03-29 if collapsed to +13
        // months.
        date = shift_months(date, years as i64 * 12)?;
        date = shift_months(date, months as i64)?;
        date = date
            .checked_add_signed(chrono::Duration::days(days as i64))
            .ok_or_else(|| crate::error::DmnError::execution("date shift overflowed"))?;

        Ok(Value::String(date.format("%Y-%m-%d").to_string()))
    }

    fn fn_year(&self, args: &[Value]) -> Result<Value, crate::error::DmnError> {
        let s = args.first().and_then(|v| v.as_str()).unwrap_or("");
        if let Ok(dt) = NaiveDate::parse_from_str(s, "%Y-%m-%d") {
            return Ok(Value::from(dt.year() as i64));
        }
        if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
            return Ok(Value::from(dt.year() as i64));
        }
        Err(crate::error::DmnError::execution(
            "cannot extract year from value",
        ))
    }

    fn fn_month(&self, args: &[Value]) -> Result<Value, crate::error::DmnError> {
        let s = args.first().and_then(|v| v.as_str()).unwrap_or("");
        if let Ok(dt) = NaiveDate::parse_from_str(s, "%Y-%m-%d") {
            return Ok(Value::from(dt.month() as i64));
        }
        if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
            return Ok(Value::from(dt.month() as i64));
        }
        Err(crate::error::DmnError::execution(
            "cannot extract month from value",
        ))
    }

    fn fn_day(&self, args: &[Value]) -> Result<Value, crate::error::DmnError> {
        let s = args.first().and_then(|v| v.as_str()).unwrap_or("");
        if let Ok(dt) = NaiveDate::parse_from_str(s, "%Y-%m-%d") {
            return Ok(Value::from(dt.day() as i64));
        }
        if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
            return Ok(Value::from(dt.day() as i64));
        }
        Err(crate::error::DmnError::execution(
            "cannot extract day from value",
        ))
    }

    fn fn_hour(&self, args: &[Value]) -> Result<Value, crate::error::DmnError> {
        let s = args.first().and_then(|v| v.as_str()).unwrap_or("");
        if let Ok(t) = NaiveTime::parse_from_str(s, "%H:%M:%S") {
            return Ok(Value::from(t.hour() as i64));
        }
        if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
            return Ok(Value::from(dt.hour() as i64));
        }
        Err(crate::error::DmnError::execution(
            "cannot extract hour from value",
        ))
    }

    fn fn_minute(&self, args: &[Value]) -> Result<Value, crate::error::DmnError> {
        let s = args.first().and_then(|v| v.as_str()).unwrap_or("");
        if let Ok(t) = NaiveTime::parse_from_str(s, "%H:%M:%S") {
            return Ok(Value::from(t.minute() as i64));
        }
        if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
            return Ok(Value::from(dt.minute() as i64));
        }
        Err(crate::error::DmnError::execution(
            "cannot extract minute from value",
        ))
    }

    fn fn_second(&self, args: &[Value]) -> Result<Value, crate::error::DmnError> {
        let s = args.first().and_then(|v| v.as_str()).unwrap_or("");
        if let Ok(t) = NaiveTime::parse_from_str(s, "%H:%M:%S") {
            return Ok(Value::from(t.second() as i64));
        }
        if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
            return Ok(Value::from(dt.second() as i64));
        }
        Err(crate::error::DmnError::execution(
            "cannot extract second from value",
        ))
    }
}

#[cfg(test)]
mod feel_tests {
    use super::*;
    use serde_json::json;

    fn ctx(vars: &[(&str, Value)]) -> HashMap<String, Value> {
        vars.iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect()
    }

    #[test]
    fn evaluates_arithmetic_expressions() {
        let engine = FeelExpressionEngine::new();
        let context = HashMap::new();
        assert_eq!(engine.evaluate("2 + 3", &context).unwrap(), Value::from(5));
        assert_eq!(engine.evaluate("10 - 4", &context).unwrap(), Value::from(6));
        assert_eq!(engine.evaluate("3 * 4", &context).unwrap(), Value::from(12));
        assert_eq!(
            engine.evaluate("10 / 2", &context).unwrap(),
            Value::from(5.0)
        );
        assert_eq!(engine.evaluate("10 % 3", &context).unwrap(), Value::from(1));
    }

    #[test]
    fn evaluates_comparison_expressions() {
        let engine = FeelExpressionEngine::new();
        let context = HashMap::new();
        assert_eq!(
            engine.evaluate("5 > 3", &context).unwrap(),
            Value::Bool(true)
        );
        assert_eq!(
            engine.evaluate("5 < 3", &context).unwrap(),
            Value::Bool(false)
        );
        assert_eq!(
            engine.evaluate("5 = 5", &context).unwrap(),
            Value::Bool(true)
        );
        assert_eq!(
            engine.evaluate("5 != 3", &context).unwrap(),
            Value::Bool(true)
        );
        assert_eq!(
            engine.evaluate("5 >= 5", &context).unwrap(),
            Value::Bool(true)
        );
        assert_eq!(
            engine.evaluate("5 <= 4", &context).unwrap(),
            Value::Bool(false)
        );
    }

    #[test]
    fn evaluates_logical_expressions() {
        let engine = FeelExpressionEngine::new();
        let context = HashMap::new();
        assert_eq!(
            engine.evaluate("true and true", &context).unwrap(),
            Value::Bool(true)
        );
        assert_eq!(
            engine.evaluate("true and false", &context).unwrap(),
            Value::Bool(false)
        );
        assert_eq!(
            engine.evaluate("false or true", &context).unwrap(),
            Value::Bool(true)
        );
        assert_eq!(
            engine.evaluate("false or false", &context).unwrap(),
            Value::Bool(false)
        );
        assert_eq!(
            engine.evaluate("not(false)", &context).unwrap(),
            Value::Bool(true)
        );
    }

    #[test]
    fn evaluates_variable_expressions() {
        let engine = FeelExpressionEngine::new();
        let context = ctx(&[("x", Value::from(10)), ("y", Value::from(20))]);
        assert_eq!(engine.evaluate("x + y", &context).unwrap(), Value::from(30));
        assert_eq!(
            engine.evaluate("x > 5", &context).unwrap(),
            Value::Bool(true)
        );
    }

    #[test]
    fn evaluates_string_functions() {
        let engine = FeelExpressionEngine::new();
        let context = HashMap::new();
        assert_eq!(
            engine
                .evaluate("contains(\"hello world\", \"world\")", &context)
                .unwrap(),
            Value::Bool(true)
        );
        assert_eq!(
            engine
                .evaluate("starts with(\"hello\", \"hel\")", &context)
                .unwrap(),
            Value::Bool(true)
        );
        assert_eq!(
            engine
                .evaluate("ends with(\"hello\", \"llo\")", &context)
                .unwrap(),
            Value::Bool(true)
        );
        assert_eq!(
            engine.evaluate("upper case(\"hello\")", &context).unwrap(),
            Value::String("HELLO".to_string())
        );
        assert_eq!(
            engine.evaluate("lower case(\"HELLO\")", &context).unwrap(),
            Value::String("hello".to_string())
        );
    }

    #[test]
    fn evaluates_number_functions() {
        let engine = FeelExpressionEngine::new();
        let context = HashMap::new();
        assert_eq!(
            engine.evaluate("abs(-5)", &context).unwrap(),
            Value::from(5.0)
        );
        assert_eq!(
            engine.evaluate("floor(3.7)", &context).unwrap(),
            Value::from(3.0)
        );
        assert_eq!(
            engine.evaluate("ceiling(3.2)", &context).unwrap(),
            Value::from(4.0)
        );
        assert_eq!(
            engine.evaluate("sqrt(9)", &context).unwrap(),
            Value::from(3.0)
        );
    }

    #[test]
    fn evaluates_list_functions() {
        let engine = FeelExpressionEngine::new();
        let context = HashMap::new();
        assert_eq!(
            engine.evaluate("count(1, 2, 3)", &context).unwrap(),
            Value::from(3u64)
        );
    }

    #[test]
    fn evaluates_sum_function() {
        let engine = FeelExpressionEngine::new();
        let mut context = HashMap::new();
        context.insert("numbers".to_string(), json!([1, 2, 3, 4, 5]));
        assert_eq!(
            engine.evaluate("sum(numbers)", &context).unwrap(),
            Value::from(15.0)
        );
    }

    #[test]
    fn evaluates_mean_function() {
        let engine = FeelExpressionEngine::new();
        let mut context = HashMap::new();
        context.insert("numbers".to_string(), json!([1.0, 2.0, 3.0, 4.0, 5.0]));
        assert_eq!(
            engine.evaluate("mean(numbers)", &context).unwrap(),
            Value::from(3.0)
        );
    }

    #[test]
    fn evaluates_min_function() {
        let engine = FeelExpressionEngine::new();
        let mut context = HashMap::new();
        context.insert("numbers".to_string(), json!([3, 1, 4, 1, 5]));
        assert_eq!(
            engine.evaluate("min(numbers)", &context).unwrap(),
            Value::from(1.0)
        );
    }

    #[test]
    fn evaluates_max_function() {
        let engine = FeelExpressionEngine::new();
        let mut context = HashMap::new();
        context.insert("numbers".to_string(), json!([3, 1, 4, 1, 5]));
        assert_eq!(
            engine.evaluate("max(numbers)", &context).unwrap(),
            Value::from(5.0)
        );
    }

    #[test]
    fn sum_empty_list_returns_zero() {
        let engine = FeelExpressionEngine::new();
        let mut context = HashMap::new();
        context.insert("empty".to_string(), json!([]));
        assert_eq!(
            engine.evaluate("sum(empty)", &context).unwrap(),
            Value::from(0.0)
        );
    }

    #[test]
    fn mean_empty_list_returns_error() {
        let engine = FeelExpressionEngine::new();
        let mut context = HashMap::new();
        context.insert("empty".to_string(), json!([]));
        let result = engine.evaluate("mean(empty)", &context);
        assert!(result.is_err());
    }

    #[test]
    fn evaluates_parenthesized_expressions() {
        let engine = FeelExpressionEngine::new();
        let context = HashMap::new();
        assert_eq!(
            engine.evaluate("(2 + 3) * 4", &context).unwrap(),
            Value::from(20)
        );
    }
}
