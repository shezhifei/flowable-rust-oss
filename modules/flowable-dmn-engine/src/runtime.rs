use crate::error::DmnError;
use crate::models::{
    CollectOperator, DmnComparisonOperator, DmnDecisionDefinition, DmnDeferredOperator,
    DmnExecutionRequest, DmnExecutionResult, DmnExpressionExecution, DmnHitPolicy, DmnOutputClause,
    DmnRule, DmnRuleExecutionAudit, DmnRuleOutputEntry, DmnStringFunction, DmnUnaryTest,
    FeelExpressionEngine, HistoricDecisionExecution, compare_temporal_values,
    normalize_temporal_value, normalized_type_ref, number_to_i64, numeric_value,
    strip_expression_shells,
};
use crate::repository::DmnRepositoryService;
use crate::store::DmnStore;
use chrono::Utc;
use flowable_persistence::entity::dmn_execution_history::{
    DmnExecutionHistoryDataManager, DmnExecutionHistoryEntity,
};
use serde_json::{Map, Value};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

/// Per-rule output evaluation shared by decision results and audit conclusions
/// so expressions with side effects (e.g. date defaults) stay consistent.
struct EvaluatedRuleOutputs {
    /// Coerced values keyed by output clause name (empty entries omitted).
    values: Map<String, Value>,
    /// Conclusion audit entries in clause order (empty entries → Null).
    conclusions: Vec<DmnExpressionExecution>,
}

type RuleOutputCache = HashMap<String, EvaluatedRuleOutputs>;

#[derive(Clone)]
pub struct DmnDecisionService {
    store: DmnStore,
    repository_service: DmnRepositoryService,
    /// Java `DmnEngineConfiguration.strictMode` (`DmnEngineConfiguration.java:202`).
    /// Hit policies read this at evaluation time (e.g. `HitPolicyUnique.java:44`).
    strict_mode: bool,
}

impl DmnDecisionService {
    pub(crate) fn new(
        store: DmnStore,
        repository_service: DmnRepositoryService,
        strict_mode: bool,
    ) -> Self {
        Self {
            store,
            repository_service,
            strict_mode,
        }
    }

    /// Java `DmnEngineConfiguration.isStrictMode()` (:1109-1111).
    pub fn strict_mode(&self) -> bool {
        self.strict_mode
    }

    pub fn execute_by_key(
        &self,
        decision_key: &str,
        request: DmnExecutionRequest,
    ) -> Result<DmnExecutionResult, DmnError> {
        match self.repository_service.latest_decision_by_key_with_fallback(
            decision_key,
            request.tenant_id.as_deref(),
            request.parent_deployment_id.as_deref(),
            request.fallback_to_default_tenant,
        ) {
            Ok(definition) => {
                let mut context = request.variables.as_object().cloned().ok_or_else(|| {
                    DmnError::execution("DMN execution variables must be a JSON object")
                })?;
                let mut resolved = HashSet::new();
                let mut visiting = HashSet::new();
                self.execute_definition_with_dependencies(
                    definition,
                    &request,
                    &mut context,
                    &mut resolved,
                    &mut visiting,
                )
            }
            Err(DmnError::NotFound { .. }) => {
                self.execute_decision_service_by_key(decision_key, request)
            }
            Err(error) => Err(error),
        }
    }

    fn execute_decision_service_by_key(
        &self,
        decision_service_key: &str,
        request: DmnExecutionRequest,
    ) -> Result<DmnExecutionResult, DmnError> {
        let Some((service, deployment)) = self.repository_service.latest_decision_service_by_key(
            decision_service_key,
            request.tenant_id.as_deref(),
            request.parent_deployment_id.as_deref(),
            request.fallback_to_default_tenant,
        )?
        else {
            return Err(DmnError::not_found(format!(
                "DMN decision or decision service '{}' was not found",
                decision_service_key
            )));
        };

        let mut context =
            request.variables.as_object().cloned().ok_or_else(|| {
                DmnError::execution("DMN execution variables must be a JSON object")
            })?;
        let mut resolved = HashSet::new();
        let mut visiting = HashSet::new();
        let mut output_result = None;
        let mut matched_rule_count = 0;
        let mut rule_executions = Vec::new();

        for decision_key in &service.required_decisions {
            if resolved.contains(decision_key) {
                continue;
            }

            let definition = self.repository_service.latest_decision_by_key_with_fallback(
                decision_key,
                request.tenant_id.as_deref(),
                request.parent_deployment_id.as_deref(),
                request.fallback_to_default_tenant,
            )?;
            self.execute_definition_with_dependencies(
                definition,
                &request,
                &mut context,
                &mut resolved,
                &mut visiting,
            )?;
        }

        // Java `DmnDecisionServiceImpl.composeDecisionServiceResult` (:214-232):
        // per-output-decision rows + multipleResults if >1 output decision or
        // any child reports multipleResults.
        let mut decision_service_result: std::collections::BTreeMap<
            String,
            Vec<Map<String, Value>>,
        > = std::collections::BTreeMap::new();
        let mut multiple_results = service.output_decisions.len() > 1;
        let mut flattened_decision_result = Vec::new();

        for decision_key in &service.output_decisions {
            if resolved.contains(decision_key) {
                continue;
            }

            let definition = self.repository_service.latest_decision_by_key_with_fallback(
                decision_key,
                request.tenant_id.as_deref(),
                request.parent_deployment_id.as_deref(),
                request.fallback_to_default_tenant,
            )?;
            let result = self.execute_definition_with_dependencies(
                definition,
                &request,
                &mut context,
                &mut resolved,
                &mut visiting,
            )?;
            matched_rule_count += result.matched_rule_count;
            rule_executions.extend(result.rule_executions.clone());
            multiple_results = multiple_results || result.multiple_results;
            if !result.decision_result.is_empty() {
                decision_service_result
                    .insert(decision_key.clone(), result.decision_result.clone());
                flattened_decision_result.extend(result.decision_result.clone());
            }
            output_result = Some(result);
        }

        let Some(last_output_result) = output_result else {
            return Err(DmnError::execution(format!(
                "DMN decision service '{}' did not declare executable decisions",
                decision_service_key
            )));
        };

        let inputs = request.variables.as_object().cloned().ok_or_else(|| {
            DmnError::execution(format!(
                "DMN decision service '{}' received non-object variables",
                decision_service_key
            ))
        })?;

        Ok(DmnExecutionResult {
            execution_id: last_output_result.execution_id,
            decision_definition_id: format!(
                "dmn-decision-service:{}:{}",
                deployment.id, service.id
            ),
            deployment_id: deployment.id,
            decision_key: service.id,
            decision_name: service.name,
            decision_version: 1,
            hit_policy: last_output_result.hit_policy,
            matched_rule_id: if service.output_decisions.len() == 1 {
                last_output_result.matched_rule_id
            } else {
                None
            },
            matched_rule_count,
            rule_executions,
            business_key: request.business_key,
            executed_at: last_output_result.executed_at,
            inputs,
            // For single-output-decision services, expose that decision's rows
            // as decision_result; multi-output flattens (REST Java does the same).
            decision_result: if service.output_decisions.len() == 1 {
                decision_service_result
                    .values()
                    .next()
                    .cloned()
                    .unwrap_or_default()
            } else {
                flattened_decision_result
            },
            multiple_results,
            decision_service_result: Some(decision_service_result),
            // Decision-service composition does not re-run hit policies; surface
            // the last output decision's soft-violation message if any.
            validation_message: last_output_result.validation_message,
        })
    }

    fn execute_definition_with_dependencies(
        &self,
        definition: DmnDecisionDefinition,
        request: &DmnExecutionRequest,
        context: &mut Map<String, Value>,
        resolved: &mut HashSet<String>,
        visiting: &mut HashSet<String>,
    ) -> Result<DmnExecutionResult, DmnError> {
        if !visiting.insert(definition.key.clone()) {
            return Err(DmnError::execution(format!(
                "Cyclic DMN decision requirement detected at '{}'",
                definition.key
            )));
        }

        for required_decision_key in &definition.required_decisions {
            if resolved.contains(required_decision_key) {
                continue;
            }

            let required_definition = self.repository_service.latest_decision_by_key_with_fallback(
                required_decision_key,
                request.tenant_id.as_deref(),
                request.parent_deployment_id.as_deref(),
                request.fallback_to_default_tenant,
            )?;
            self.execute_definition_with_dependencies(
                required_definition,
                request,
                context,
                resolved,
                visiting,
            )?;
        }

        let result = self.execute_definition(
            definition.clone(),
            // Correlation/tenant/history settings ride along to child decisions:
            // Java keeps one `ExecuteDecisionContext` for the whole execution
            // (`AbstractExecuteDecisionCmd.java:60-66`).
            DmnExecutionRequest {
                variables: Value::Object(context.clone()),
                ..request.clone()
            },
        )?;
        // Java `AbstractHitPolicy.updateStackWithDecisionResults` (:75-77)
        context.extend(result.stack_variables());
        resolved.insert(definition.key.clone());
        visiting.remove(&definition.key);
        Ok(result)
    }

    fn execute_definition(
        &self,
        definition: DmnDecisionDefinition,
        request: DmnExecutionRequest,
    ) -> Result<DmnExecutionResult, DmnError> {
        let raw_inputs =
            request.variables.as_object().cloned().ok_or_else(|| {
                DmnError::execution("DMN execution variables must be a JSON object")
            })?;
        let inputs = coerce_inputs_for_type_refs(&definition, raw_inputs)?;

        // Evaluate matching-rule outputs once; hit policy and audit share results
        // (avoids double-eval of non-deterministic expressions).
        //
        // Java `RuleEngineExecutorImpl.evaluateDecisionTable` (:145-159) wraps
        // exactly this region in a try/catch: on failure it clears the rule
        // results and records the error on the audit container instead of
        // aborting, so `finalizeDecisionExecutionAudit` still writes a history
        // row with `FAILED_ = true`. The caller then throws
        // (`DmnActivityBehavior.java:112-114`) — hence we persist and re-raise.
        let evaluated = (|| {
            let evaluated_outputs = precompute_matching_rule_outputs(&definition, &inputs)?;
            let evaluation =
                evaluate_hit_policy(&definition, &inputs, &evaluated_outputs, self.strict_mode)?;
            let mut rule_executions =
                build_rule_execution_audit(&definition, &inputs, &evaluated_outputs)?;
            // Java hit policies write rule-level validation/exception messages
            // onto RuleExecutionAuditContainer (HitPolicyUnique.java:45-50,
            // HitPolicyAny.java:53-61). Merge after building the base audit.
            apply_hit_policy_rule_messages(&mut rule_executions, &evaluation);
            Ok::<_, DmnError>((evaluation, rule_executions))
        })();
        let (evaluation, rule_executions) = match evaluated {
            Ok(pair) => pair,
            Err(error) => {
                if !request.disable_history {
                    persist_history(
                        &self.store,
                        &failed_history_record(&definition, &request, inputs),
                    )?;
                }
                return Err(error);
            }
        };
        let executed_at = Utc::now();
        let result = DmnExecutionResult {
            execution_id: format!("dmn-execution:{}", Uuid::new_v4()),
            decision_definition_id: definition.id.clone(),
            deployment_id: definition.deployment_id.clone(),
            decision_key: definition.key.clone(),
            decision_name: definition.name.clone(),
            decision_version: definition.version,
            hit_policy: definition.hit_policy.clone(),
            matched_rule_id: evaluation.matched_rule_id.clone(),
            matched_rule_count: evaluation.matched_rule_count,
            rule_executions: rule_executions.clone(),
            business_key: request.business_key.clone(),
            executed_at,
            inputs: inputs.clone(),
            decision_result: evaluation.decision_result.clone(),
            multiple_results: evaluation.multiple_results,
            decision_service_result: None,
            // Java DecisionExecutionAuditContainer.validationMessage (:56, :241-247)
            validation_message: evaluation.validation_message.clone(),
        };

        let historic = HistoricDecisionExecution {
            execution_id: result.execution_id.clone(),
            decision_definition_id: result.decision_definition_id.clone(),
            deployment_id: result.deployment_id.clone(),
            decision_key: result.decision_key.clone(),
            decision_name: result.decision_name.clone(),
            decision_version: result.decision_version,
            hit_policy: result.hit_policy.clone(),
            matched_rule_id: evaluation.matched_rule_id,
            matched_rule_count: evaluation.matched_rule_count,
            rule_executions,
            business_key: request.business_key,
            tenant_id: request.tenant_id,
            // Java `PersistHistoricDecisionExecutionCmd.java:56-59` — process
            // correlation carried on the ExecuteDecisionContext.
            instance_id: request.instance_id,
            scope_execution_id: request.execution_id,
            activity_id: request.activity_id,
            scope_type: request.scope_type,
            failed: false,
            executed_at,
            inputs,
            decision_result: evaluation.decision_result,
            multiple_results: evaluation.multiple_results,
            decision_service_result: None,
            // Java persists the audit container's validationMessage into
            // EXECUTION_JSON_ (`PersistHistoricDecisionExecutionCmd.java:73`).
            validation_message: evaluation.validation_message.clone(),
        };

        if !request.disable_history {
            persist_history(&self.store, &historic)?;
        }
        Ok(result)
    }
}

fn coerce_inputs_for_type_refs(
    definition: &DmnDecisionDefinition,
    mut inputs: Map<String, Value>,
) -> Result<Map<String, Value>, DmnError> {
    for input_clause in &definition.inputs {
        let Some(type_ref) = input_clause.type_ref.as_deref() else {
            continue;
        };
        let Some(value) = inputs.get(&input_clause.input_variable) else {
            continue;
        };

        let coerced = coerce_input_value(&input_clause.input_variable, type_ref, value)?;
        inputs.insert(input_clause.input_variable.clone(), coerced);
    }
    Ok(inputs)
}

fn coerce_input_value(
    input_variable: &str,
    type_ref: &str,
    value: &Value,
) -> Result<Value, DmnError> {
    if value.is_null() {
        return Ok(Value::Null);
    }

    match normalized_type_ref(type_ref).as_str() {
        "string" => value
            .as_str()
            .map(|value| Value::String(value.to_string()))
            .ok_or_else(|| incompatible_type_ref_error(input_variable, type_ref, value)),
        "boolean" => value
            .as_bool()
            .map(Value::Bool)
            .ok_or_else(|| incompatible_type_ref_error(input_variable, type_ref, value)),
        "integer" => coerce_integer_value(
            input_variable,
            type_ref,
            value,
            i32::MIN as i64,
            i32::MAX as i64,
        ),
        "long" => coerce_integer_value(input_variable, type_ref, value, i64::MIN, i64::MAX),
        "double" | "number" => coerce_number_value(input_variable, type_ref, value),
        "date" | "time" | "datetime" | "duration" | "daytimeduration" | "yearmonthduration" => {
            normalize_temporal_value(type_ref, value)
                .ok_or_else(|| incompatible_type_ref_error(input_variable, type_ref, value))
        }
        "context" => value
            .as_object()
            .map(|value| Value::Object(value.clone()))
            .ok_or_else(|| incompatible_type_ref_error(input_variable, type_ref, value)),
        "list" => value
            .as_array()
            .map(|value| Value::Array(value.clone()))
            .ok_or_else(|| incompatible_type_ref_error(input_variable, type_ref, value)),
        _ => Err(DmnError::unsupported(
            "typeRef",
            format!(
                "unsupported input typeRef '{}' for input '{}'; supported input typeRefs are string, boolean, integer, long, double, number, date, time, dateTime, date and time, duration, dayTimeDuration, yearMonthDuration, context, and list",
                type_ref, input_variable
            ),
        )),
    }
}

fn coerce_integer_value(
    input_variable: &str,
    type_ref: &str,
    value: &Value,
    min: i64,
    max: i64,
) -> Result<Value, DmnError> {
    let Some(number) = numeric_value(value) else {
        return Err(incompatible_type_ref_error(input_variable, type_ref, value));
    };
    let Some(integer) = number_to_i64(&number) else {
        return Err(incompatible_type_ref_error(input_variable, type_ref, value));
    };
    if integer < min || integer > max {
        return Err(incompatible_type_ref_error(input_variable, type_ref, value));
    }

    Ok(Value::from(integer))
}

fn coerce_number_value(
    input_variable: &str,
    type_ref: &str,
    value: &Value,
) -> Result<Value, DmnError> {
    numeric_value(value).ok_or_else(|| incompatible_type_ref_error(input_variable, type_ref, value))
}

fn incompatible_type_ref_error(input_variable: &str, type_ref: &str, value: &Value) -> DmnError {
    DmnError::execution(format!(
        "DMN input '{}' with typeRef '{}' received incompatible value {}",
        input_variable, type_ref, value
    ))
}

/// Per-rule hit-policy messages written onto audit containers
/// (Java `RuleExecutionAuditContainer` exception/validation fields).
struct RuleHitPolicyMessage {
    validation_message: Option<String>,
    exception_message: Option<String>,
}

/// Hit-policy evaluation produces row-shaped results
/// (Java `ComposeDecisionResultBehavior` / `AbstractHitPolicy.composeDecisionResults`).
struct HitPolicyEvaluation {
    matched_rule_id: Option<String>,
    matched_rule_count: usize,
    decision_result: Vec<Map<String, Value>>,
    multiple_results: bool,
    /// Decision-level soft violation (`DecisionExecutionAuditContainer.validationMessage`).
    validation_message: Option<String>,
    /// Rule-level messages keyed by rule id.
    rule_messages: HashMap<String, RuleHitPolicyMessage>,
}

fn single_row_result(
    matched_rule_id: Option<String>,
    matched_rule_count: usize,
    outputs: Map<String, Value>,
) -> HitPolicyEvaluation {
    let decision_result = if outputs.is_empty() {
        Vec::new()
    } else {
        vec![outputs]
    };
    HitPolicyEvaluation {
        matched_rule_id,
        matched_rule_count,
        decision_result,
        multiple_results: false,
        validation_message: None,
        rule_messages: HashMap::new(),
    }
}

fn empty_hit_result() -> HitPolicyEvaluation {
    HitPolicyEvaluation {
        matched_rule_id: None,
        matched_rule_count: 0,
        decision_result: Vec::new(),
        multiple_results: false,
        validation_message: None,
        rule_messages: HashMap::new(),
    }
}

fn evaluate_hit_policy(
    definition: &DmnDecisionDefinition,
    inputs: &Map<String, Value>,
    evaluated_outputs: &RuleOutputCache,
    strict_mode: bool,
) -> Result<HitPolicyEvaluation, DmnError> {
    match definition.hit_policy {
        DmnHitPolicy::First => evaluate_first_hit(definition, inputs, evaluated_outputs),
        DmnHitPolicy::Unique => {
            evaluate_unique_hit(definition, inputs, evaluated_outputs, strict_mode)
        }
        DmnHitPolicy::Any => evaluate_any_hit(definition, inputs, evaluated_outputs, strict_mode),
        DmnHitPolicy::RuleOrder => evaluate_rule_order_hit(definition, inputs, evaluated_outputs),
        DmnHitPolicy::OutputOrder => {
            evaluate_output_order_hit(definition, inputs, evaluated_outputs, strict_mode)
        }
        DmnHitPolicy::Priority => {
            evaluate_priority_hit(definition, inputs, evaluated_outputs, strict_mode)
        }
        DmnHitPolicy::Collect => evaluate_collect_hit(definition, inputs, evaluated_outputs),
        // Rust extension (no Java counterpart): multi-row like RULE_ORDER.
        // Java RULE_ORDER has no hit-policy violation path — no strictMode
        // branching needed (`HitPolicyRuleOrder.java`).
        DmnHitPolicy::Complete => {
            let matched_rules = matching_rules(definition, inputs)?;
            evaluate_rule_order_matches(definition, &matched_rules, evaluated_outputs)
        }
        // Rust extension (no Java counterpart): multi-row like RULE_ORDER.
        // Same: no violation path / no strictMode handling.
        DmnHitPolicy::Batch => evaluate_batch_hit(definition, inputs, evaluated_outputs),
    }
}

fn evaluate_first_hit(
    definition: &DmnDecisionDefinition,
    inputs: &Map<String, Value>,
    evaluated_outputs: &RuleOutputCache,
) -> Result<HitPolicyEvaluation, DmnError> {
    for rule in &definition.rules {
        if rule_matches(definition, inputs, rule)? {
            return Ok(single_row_result(
                Some(rule.id.clone()),
                1,
                cached_outputs_for_rule(evaluated_outputs, rule)?,
            ));
        }
    }

    Ok(empty_hit_result())
}

fn evaluate_unique_hit(
    definition: &DmnDecisionDefinition,
    inputs: &Map<String, Value>,
    evaluated_outputs: &RuleOutputCache,
    strict_mode: bool,
) -> Result<HitPolicyEvaluation, DmnError> {
    let matched_rules = matching_rules(definition, inputs)?;
    match matched_rules.as_slice() {
        [] => Ok(empty_hit_result()),
        [rule] => Ok(single_row_result(
            Some(rule.id.clone()),
            1,
            cached_outputs_for_rule(evaluated_outputs, rule)?,
        )),
        rules => {
            // Java HitPolicyUnique.evaluateRuleValidity (:38-55): ≥2 valid rules.
            let message = format!(
                "HitPolicy UNIQUE violated; multiple valid rules: {}.",
                joined_rule_ids(rules)
            );
            if strict_mode {
                // strict: exception on rules then throw (HitPolicyUnique.java:44-47)
                return Err(DmnError::execution(format!(
                    "UNIQUE hit policy violation for decision '{}': matched rules {}",
                    definition.key,
                    joined_rule_ids(rules)
                )));
            }
            // non-strict: rule-level validationMessage + break, then compose by
            // key-merge (later non-null overwrites earlier)
            // (HitPolicyUnique.java:48-51, :62-74).
            let mut rule_messages = HashMap::new();
            for rule in rules {
                rule_messages.insert(
                    rule.id.clone(),
                    RuleHitPolicyMessage {
                        validation_message: Some(message.clone()),
                        exception_message: None,
                    },
                );
            }
            let merged = merge_unique_outputs_by_key(rules, evaluated_outputs)?;
            Ok(HitPolicyEvaluation {
                matched_rule_id: None,
                matched_rule_count: rules.len(),
                decision_result: if merged.is_empty() {
                    Vec::new()
                } else {
                    vec![merged]
                },
                multiple_results: false,
                // HitPolicyUnique.java:73
                validation_message: Some(
                    "HitPolicy UNIQUE violated; multiple valid rules. Setting last valid rule result as final result."
                        .to_string(),
                ),
                rule_messages,
            })
        }
    }
}

/// UNIQUE non-strict compose: later non-null values overwrite earlier keys
/// (`HitPolicyUnique.java:62-71`).
fn merge_unique_outputs_by_key(
    rules: &[&DmnRule],
    evaluated_outputs: &RuleOutputCache,
) -> Result<Map<String, Value>, DmnError> {
    let mut merged = Map::new();
    for rule in rules {
        let row = cached_outputs_for_rule(evaluated_outputs, rule)?;
        for (key, value) in row {
            if !value.is_null() {
                merged.insert(key, value);
            }
        }
    }
    Ok(merged)
}

fn evaluate_any_hit(
    definition: &DmnDecisionDefinition,
    inputs: &Map<String, Value>,
    evaluated_outputs: &RuleOutputCache,
    strict_mode: bool,
) -> Result<HitPolicyEvaluation, DmnError> {
    let matched_rules = matching_rules(definition, inputs)?;
    let Some(first_rule) = matched_rules.first() else {
        return Ok(empty_hit_result());
    };

    let first_outputs = cached_outputs_for_rule(evaluated_outputs, first_rule)?;
    if let Some(conflicting_rule) = matched_rules
        .iter()
        .skip(1)
        .find_map(
            |rule| match cached_outputs_for_rule(evaluated_outputs, rule) {
                Ok(outputs) if outputs != first_outputs => Some(Ok(*rule)),
                Ok(_) => None,
                Err(error) => Some(Err(error)),
            },
        )
        .transpose()?
    {
        // Java HitPolicyAny.composeDecisionResults (:45-64): multi-row outputs differ.
        let rule_message = format!(
            "HitPolicy ANY violated; both rule '{}' and '{}' are valid but outputs differ.",
            first_rule.id, conflicting_rule.id
        );
        if strict_mode {
            return Err(DmnError::execution(format!(
                "ANY hit policy violation for decision '{}': matched rules {} produce different outputs; first conflicting rule '{}'",
                definition.key,
                joined_rule_ids(&matched_rules),
                conflicting_rule.id
            )));
        }
        // non-strict: two-level validationMessage; result is LAST matched row
        // (HitPolicyAny.java:57-64, :73-78 get(size-1)). Distinct from the
        // success path which keeps the first row for content-equal matches.
        let last_rule = matched_rules
            .last()
            .expect("matched_rules non-empty after conflict");
        let last_outputs = cached_outputs_for_rule(evaluated_outputs, last_rule)?;
        let mut rule_messages = HashMap::new();
        // Java sets messages on the compared pair; also mark all matched rules
        // that participated so audit consumers see the violation.
        for rule in &matched_rules {
            rule_messages.insert(
                rule.id.clone(),
                RuleHitPolicyMessage {
                    validation_message: Some(rule_message.clone()),
                    exception_message: None,
                },
            );
        }
        return Ok(HitPolicyEvaluation {
            matched_rule_id: Some(last_rule.id.clone()),
            matched_rule_count: matched_rules.len(),
            decision_result: if last_outputs.is_empty() {
                Vec::new()
            } else {
                vec![last_outputs]
            },
            multiple_results: false,
            // HitPolicyAny.java:74
            validation_message: Some(
                "HitPolicy ANY violated; multiple valid rules with different outcomes. Setting last valid rule result as final result."
                    .to_string(),
            ),
            rule_messages,
        });
    }

    // Success path: identical outputs — keep first row (existing Rust contract;
    // Java always takes last but values are equal so consumers see the same map).
    Ok(single_row_result(
        Some(first_rule.id.clone()),
        matched_rules.len(),
        first_outputs,
    ))
}

fn evaluate_collect_hit(
    definition: &DmnDecisionDefinition,
    inputs: &Map<String, Value>,
    evaluated_outputs: &RuleOutputCache,
) -> Result<HitPolicyEvaluation, DmnError> {
    let matched_rules = matching_rules(definition, inputs)?;
    if let Some(operator) = &definition.collect_operator {
        return evaluate_collect_aggregation(
            definition,
            &matched_rules,
            operator,
            evaluated_outputs,
        );
    }
    evaluate_rule_order_matches(definition, &matched_rules, evaluated_outputs)
}

fn evaluate_rule_order_hit(
    definition: &DmnDecisionDefinition,
    inputs: &Map<String, Value>,
    evaluated_outputs: &RuleOutputCache,
) -> Result<HitPolicyEvaluation, DmnError> {
    let matched_rules = matching_rules(definition, inputs)?;
    evaluate_rule_order_matches(definition, &matched_rules, evaluated_outputs)
}

fn evaluate_output_order_hit(
    definition: &DmnDecisionDefinition,
    inputs: &Map<String, Value>,
    evaluated_outputs: &RuleOutputCache,
    strict_mode: bool,
) -> Result<HitPolicyEvaluation, DmnError> {
    let mut matched_rules = matching_rules(definition, inputs)?;
    // HitPolicyOutputOrder.java:44-60 — check outputValues presence before sort;
    // missing → strict throw / non-strict decision-level validationMessage.
    let validation_message =
        ensure_output_values_for_order_policy(definition, &matched_rules, strict_mode, false)?;
    if has_any_output_values(definition) {
        sort_rules_by_output_priority(definition, &mut matched_rules, evaluated_outputs)?;
    }
    // else: sort no-op, keep rule order (HitPolicyOutputOrder.java:63-74)
    let mut evaluation =
        evaluate_rule_order_matches(definition, &matched_rules, evaluated_outputs)?;
    evaluation.validation_message = validation_message;
    Ok(evaluation)
}

fn evaluate_priority_hit(
    definition: &DmnDecisionDefinition,
    inputs: &Map<String, Value>,
    evaluated_outputs: &RuleOutputCache,
    strict_mode: bool,
) -> Result<HitPolicyEvaluation, DmnError> {
    let mut matched_rules = matching_rules(definition, inputs)?;
    // HitPolicyPriority.java:60-72 — violation only fires inside the comparator,
    // i.e. only when ≥2 rows are actually compared. Single-match never triggers.
    let validation_message =
        ensure_output_values_for_order_policy(definition, &matched_rules, strict_mode, true)?;
    if has_any_output_values(definition) {
        sort_rules_by_output_priority(definition, &mut matched_rules, evaluated_outputs)?;
    }
    let matched_rule_count = matched_rules.len();
    let Some(rule) = matched_rules.first() else {
        return Ok(empty_hit_result());
    };

    let mut evaluation = single_row_result(
        Some(rule.id.clone()),
        matched_rule_count,
        cached_outputs_for_rule(evaluated_outputs, rule)?,
    );
    evaluation.validation_message = validation_message;
    Ok(evaluation)
}

/// True when any output clause declares a non-empty `outputValues` list.
fn has_any_output_values(definition: &DmnDecisionDefinition) -> bool {
    definition
        .outputs
        .iter()
        .any(|output| !output.output_values.is_empty())
}

/// PRIORITY / OUTPUT_ORDER missing-outputValues handling.
///
/// - `priority_style=true`: only when `matched_rules.len() >= 2` (comparator path,
///   `HitPolicyPriority.java:60-72`).
/// - `priority_style=false`: always when no outputValues present
///   (`HitPolicyOutputOrder.java:53-60`), even with 0/1 matches.
fn ensure_output_values_for_order_policy(
    definition: &DmnDecisionDefinition,
    matched_rules: &[&DmnRule],
    strict_mode: bool,
    priority_style: bool,
) -> Result<Option<String>, DmnError> {
    if has_any_output_values(definition) {
        return Ok(None);
    }
    if priority_style && matched_rules.len() < 2 {
        return Ok(None);
    }

    if strict_mode {
        return Err(DmnError::execution(format!(
            "HitPolicy {:?} violated; no output values present.",
            definition.hit_policy
        )));
    }

    // Decision-level only (PRIORITY/OUTPUT_ORDER do not write rule-level messages).
    let message = if priority_style {
        // HitPolicyPriority.java:66-69
        format!(
            "HitPolicy {:?} violated; no output values present. Setting first valid result as final result.",
            definition.hit_policy
        )
    } else {
        // HitPolicyOutputOrder.java:54
        format!(
            "HitPolicy: {:?} violated; no output values present",
            definition.hit_policy
        )
    };
    Ok(Some(message))
}

/// RULE_ORDER / OUTPUT_ORDER / COLLECT(no agg) / Complete — multi-row results.
/// Java `HitPolicyRuleOrder` / `HitPolicyOutputOrder` set `multipleResults=true`
/// (`HitPolicyRuleOrder.java:23`, `HitPolicyOutputOrder.java:32`).
fn evaluate_rule_order_matches(
    _definition: &DmnDecisionDefinition,
    matched_rules: &[&DmnRule],
    evaluated_outputs: &RuleOutputCache,
) -> Result<HitPolicyEvaluation, DmnError> {
    if matched_rules.is_empty() {
        return Ok(HitPolicyEvaluation {
            matched_rule_id: None,
            matched_rule_count: 0,
            decision_result: Vec::new(),
            // RULE_ORDER family always flags multipleResults even when empty
            // matches — Java AbstractHitPolicy(true) sets the field regardless.
            multiple_results: true,
            validation_message: None,
            rule_messages: HashMap::new(),
        });
    }

    let mut decision_result = Vec::with_capacity(matched_rules.len());
    for rule in matched_rules {
        decision_result.push(cached_outputs_for_rule(evaluated_outputs, rule)?);
    }

    let matched_rule_id = match matched_rules {
        [rule] => Some(rule.id.clone()),
        _ => None,
    };
    Ok(HitPolicyEvaluation {
        matched_rule_id,
        matched_rule_count: matched_rules.len(),
        decision_result,
        multiple_results: true,
        validation_message: None,
        rule_messages: HashMap::new(),
    })
}

fn sort_rules_by_output_priority(
    definition: &DmnDecisionDefinition,
    rules: &mut Vec<&DmnRule>,
    evaluated_outputs: &RuleOutputCache,
) -> Result<(), DmnError> {
    // Eagerly surface structural errors (missing output column / entry).
    for rule in rules.iter() {
        let _ = output_priority_rank(definition, rule, evaluated_outputs)?;
    }
    // Java OutputOrderComparator.java:31-33 — indexOf returns -1 for values not
    // in the list, so unknowns sort *before* declared values. Rank = position+1,
    // unknown → 0 (never error; prior Rust used usize::MAX and hard-failed).
    rules.sort_by_key(|rule| {
        output_priority_rank(definition, rule, evaluated_outputs).unwrap_or(0)
    });
    Ok(())
}

/// Rank for OUTPUT_ORDER / PRIORITY sort.
///
/// Lower ranks win. Values absent from `outputValues` rank first
/// (`OutputOrderComparator.java:31-33` indexOf = -1). Values present at index
/// `i` rank as `i + 1`. This is never a hit-policy violation — only "no
/// outputValues declared on any column" is (handled by
/// [`ensure_output_values_for_order_policy`]).
fn output_priority_rank(
    definition: &DmnDecisionDefinition,
    rule: &DmnRule,
    evaluated_outputs: &RuleOutputCache,
) -> Result<usize, DmnError> {
    let output = definition.outputs.first().ok_or_else(|| {
        DmnError::execution(format!(
            "{:?} hit policy for decision '{}' requires one output",
            definition.hit_policy, definition.key
        ))
    })?;
    // Prefer runtime-evaluated value so expression outputs rank correctly.
    let value = evaluated_outputs
        .get(&rule.id)
        .and_then(|evaluated| evaluated.values.get(&output.name).cloned())
        .or_else(|| rule.output_entries.first().map(|entry| entry.value.clone()))
        .ok_or_else(|| {
            DmnError::execution(format!(
                "{:?} hit policy for decision '{}' requires one output entry per rule",
                definition.hit_policy, definition.key
            ))
        })?;
    let value = coerce_runtime_output_value(definition, output, &value)?;

    // Not in list → 0 (ranks first). In list at i → i+1.
    // OutputOrderComparator.java:31-33
    Ok(output
        .output_values
        .iter()
        .position(|candidate| candidate == &value)
        .map(|index| index + 1)
        .unwrap_or(0))
}

fn apply_hit_policy_rule_messages(
    rule_executions: &mut [DmnRuleExecutionAudit],
    evaluation: &HitPolicyEvaluation,
) {
    for audit in rule_executions.iter_mut() {
        if let Some(message) = evaluation.rule_messages.get(&audit.rule_id) {
            if message.validation_message.is_some() {
                audit.validation_message = message.validation_message.clone();
            }
            if message.exception_message.is_some() {
                audit.exception_message = message.exception_message.clone();
            }
        }
    }
}

/// COLLECT with aggregator → single aggregated row; `multipleResults=false`
/// (Java `HitPolicyCollect.java:75-80`).
fn evaluate_collect_aggregation(
    definition: &DmnDecisionDefinition,
    matched_rules: &[&DmnRule],
    operator: &CollectOperator,
    evaluated_outputs: &RuleOutputCache,
) -> Result<HitPolicyEvaluation, DmnError> {
    if definition.outputs.is_empty() {
        return Err(DmnError::execution(
            "COLLECT aggregation requires at least one output",
        ));
    }

    let mut outputs = Map::new();

    // For Count operator, return the count for each output
    if *operator == CollectOperator::Count {
        let count_value = Value::from(matched_rules.len() as u64);
        for output_clause in &definition.outputs {
            let coerced = coerce_runtime_output_value(definition, output_clause, &count_value)?;
            outputs.insert(output_clause.name.clone(), coerced);
        }
    } else {
        // For Sum/Min/Max, aggregate each output independently
        for (output_index, output_clause) in definition.outputs.iter().enumerate() {
            let aggregate = match operator {
                CollectOperator::Sum => Value::from(sum_collect_values_at_index(
                    definition,
                    matched_rules,
                    output_index,
                    evaluated_outputs,
                )?),
                CollectOperator::Min => {
                    match numeric_collect_values_at_index(
                        definition,
                        matched_rules,
                        output_index,
                        evaluated_outputs,
                    )?
                    .into_iter()
                    .reduce(f64::min)
                    {
                        Some(value) => Value::from(value),
                        None => Value::Null,
                    }
                }
                CollectOperator::Max => {
                    match numeric_collect_values_at_index(
                        definition,
                        matched_rules,
                        output_index,
                        evaluated_outputs,
                    )?
                    .into_iter()
                    .reduce(f64::max)
                    {
                        Some(value) => Value::from(value),
                        None => Value::Null,
                    }
                }
                CollectOperator::Count => {
                    return Err(DmnError::execution(
                        "CollectOperator::Count should be handled before this match",
                    ));
                }
            };
            let coerced = coerce_runtime_output_value(definition, output_clause, &aggregate)?;
            outputs.insert(output_clause.name.clone(), coerced);
        }
    }

    let matched_rule_id = match matched_rules {
        [rule] => Some(rule.id.clone()),
        _ => None,
    };
    Ok(single_row_result(
        matched_rule_id,
        matched_rules.len(),
        outputs,
    ))
}

fn sum_collect_values_at_index(
    definition: &DmnDecisionDefinition,
    matched_rules: &[&DmnRule],
    output_index: usize,
    evaluated_outputs: &RuleOutputCache,
) -> Result<f64, DmnError> {
    Ok(
        numeric_collect_values_at_index(
            definition,
            matched_rules,
            output_index,
            evaluated_outputs,
        )?
        .into_iter()
        .sum(),
    )
}

fn numeric_collect_values_at_index(
    definition: &DmnDecisionDefinition,
    matched_rules: &[&DmnRule],
    output_index: usize,
    evaluated_outputs: &RuleOutputCache,
) -> Result<Vec<f64>, DmnError> {
    matched_rules
        .iter()
        .map(|rule| {
            let output = definition.outputs.get(output_index).ok_or_else(|| {
                DmnError::execution("COLLECT aggregation requires output at index")
            })?;
            let value = evaluated_outputs
                .get(&rule.id)
                .and_then(|evaluated| evaluated.values.get(&output.name).cloned())
                .ok_or_else(|| {
                    DmnError::execution(format!(
                        "COLLECT aggregation for decision '{}' requires numeric output values; rule '{}' produced non-numeric output at index {}",
                        definition.key, rule.id, output_index
                    ))
                })?;
            value.as_f64().ok_or_else(|| {
                DmnError::execution(format!(
                    "COLLECT aggregation for decision '{}' requires numeric output values; rule '{}' produced non-numeric output at index {}",
                    definition.key, rule.id, output_index
                ))
            })
        })
        .collect()
}

/// Rust Batch extension: one row per matched rule (same shape as RULE_ORDER).
fn evaluate_batch_hit(
    definition: &DmnDecisionDefinition,
    inputs: &Map<String, Value>,
    evaluated_outputs: &RuleOutputCache,
) -> Result<HitPolicyEvaluation, DmnError> {
    let matched_rules = matching_rules(definition, inputs)?;
    evaluate_rule_order_matches(definition, &matched_rules, evaluated_outputs)
}

fn matching_rules<'a>(
    definition: &'a DmnDecisionDefinition,
    inputs: &Map<String, Value>,
) -> Result<Vec<&'a DmnRule>, DmnError> {
    let mut matched = Vec::new();
    for rule in &definition.rules {
        if rule_matches(definition, inputs, rule)? {
            matched.push(rule);
        }
    }
    Ok(matched)
}

fn build_rule_execution_audit(
    definition: &DmnDecisionDefinition,
    inputs: &Map<String, Value>,
    evaluated_outputs: &RuleOutputCache,
) -> Result<Vec<DmnRuleExecutionAudit>, DmnError> {
    definition
        .rules
        .iter()
        .enumerate()
        .map(|(rule_index, rule)| {
            let condition_results = definition
                .inputs
                .iter()
                .zip(&rule.input_entries)
                .enumerate()
                .map(|(input_index, (input_clause, input_entry))| {
                    let actual = inputs
                        .get(&input_clause.input_variable)
                        .cloned()
                        .unwrap_or(Value::Null);
                    // Pre-existing Rust behaviour: every input entry is audited,
                    // whereas Java stops at the first false
                    // (RuleEngineExecutorImpl.java:210-213). Kept as-is so the
                    // audit contract is unchanged; only expression errors, which
                    // Java also propagates (:199-208), now surface here.
                    let result = unary_test_matches(
                        &input_entry.expression,
                        &actual,
                        input_clause.type_ref.as_deref(),
                        inputs,
                    )?;
                    Ok(DmnExpressionExecution {
                        id: input_entry
                            .id
                            .clone()
                            .unwrap_or_else(|| format!("{}:input:{}", rule.id, input_index + 1)),
                        result: Value::Bool(result),
                    })
                })
                .collect::<Result<Vec<_>, DmnError>>()?;
            let valid = condition_results
                .iter()
                .all(|condition| condition.result == Value::Bool(true));
            // Share precomputed evaluation with decision_result
            // (Java RuleEngineExecutorImpl.java:253-265 evaluates once then audits).
            let conclusion_results = if valid {
                evaluated_outputs
                    .get(&rule.id)
                    .map(|evaluated| evaluated.conclusions.clone())
                    .unwrap_or_default()
            } else {
                Vec::new()
            };

            Ok(DmnRuleExecutionAudit {
                rule_number: rule_index + 1,
                rule_id: rule.id.clone(),
                valid,
                condition_results,
                conclusion_results,
                validation_message: None,
                exception_message: None,
            })
        })
        .collect()
}

fn rule_matches(
    definition: &DmnDecisionDefinition,
    inputs: &Map<String, Value>,
    rule: &DmnRule,
) -> Result<bool, DmnError> {
    for (input_clause, input_entry) in definition.inputs.iter().zip(&rule.input_entries) {
        let actual = inputs
            .get(&input_clause.input_variable)
            .cloned()
            .unwrap_or(Value::Null);
        // Java breaks out of the condition loop on the first false
        // (RuleEngineExecutorImpl.java:210-213), so later entries are not
        // evaluated — which also means their expression errors do not surface.
        if !unary_test_matches(
            &input_entry.expression,
            &actual,
            input_clause.type_ref.as_deref(),
            inputs,
        )? {
            return Ok(false);
        }
    }
    Ok(true)
}

fn unary_test_matches(
    expression: &DmnUnaryTest,
    actual: &Value,
    type_ref: Option<&str>,
    inputs: &Map<String, Value>,
) -> Result<bool, DmnError> {
    Ok(match expression {
        DmnUnaryTest::Any => true,
        DmnUnaryTest::Equals(expected) => actual == expected,
        DmnUnaryTest::NotEquals(expected) => actual != expected,
        DmnUnaryTest::StringFunction { function, needle } => {
            string_function_matches(*function, actual, needle)
        }
        DmnUnaryTest::StringTransform {
            transform,
            expected,
        } => string_transform_matches(*transform, actual, expected),
        DmnUnaryTest::StringTransformComparison {
            transform,
            operator,
            expected,
        } => string_transform_comparison_matches(*transform, *operator, actual, expected),
        DmnUnaryTest::GreaterThan(expected) => compare_values(actual, expected, type_ref)
            .is_some_and(|ordering| ordering == std::cmp::Ordering::Greater),
        DmnUnaryTest::GreaterThanOrEqual(expected) => compare_values(actual, expected, type_ref)
            .is_some_and(|ordering| {
                matches!(
                    ordering,
                    std::cmp::Ordering::Greater | std::cmp::Ordering::Equal
                )
            }),
        DmnUnaryTest::LessThan(expected) => compare_values(actual, expected, type_ref)
            .is_some_and(|ordering| ordering == std::cmp::Ordering::Less),
        DmnUnaryTest::LessThanOrEqual(expected) => compare_values(actual, expected, type_ref)
            .is_some_and(|ordering| {
                matches!(
                    ordering,
                    std::cmp::Ordering::Less | std::cmp::Ordering::Equal
                )
            }),
        DmnUnaryTest::Range {
            start,
            end,
            start_inclusive,
            end_inclusive,
        } => range_matches(
            actual,
            start,
            end,
            *start_inclusive,
            *end_inclusive,
            type_ref,
        ),
        DmnUnaryTest::AnyOf(expressions) => {
            let mut matched = false;
            for expression in expressions {
                if unary_test_matches(expression, actual, type_ref, inputs)? {
                    matched = true;
                    break;
                }
            }
            matched
        }
        DmnUnaryTest::Not(expression) => !unary_test_matches(expression, actual, type_ref, inputs)?,
        DmnUnaryTest::And(expressions) => {
            let mut matched = true;
            for expression in expressions {
                if !unary_test_matches(expression, actual, type_ref, inputs)? {
                    matched = false;
                    break;
                }
            }
            matched
        }
        DmnUnaryTest::Or(expressions) => {
            let mut matched = false;
            for expression in expressions {
                if unary_test_matches(expression, actual, type_ref, inputs)? {
                    matched = true;
                    break;
                }
            }
            matched
        }
        DmnUnaryTest::InstanceOf { type_name } => instance_of_matches(type_name, actual),
        DmnUnaryTest::Substring {
            start,
            length,
            expected,
        } => {
            let Some(actual_str) = actual.as_str() else {
                return Ok(false);
            };
            let chars: Vec<char> = actual_str.chars().collect();
            let char_len = chars.len() as i32;

            let start_idx = if *start > 0 {
                (*start - 1).min(char_len)
            } else if *start < 0 {
                (char_len + *start).max(0)
            } else {
                0
            } as usize;

            let sub_chars = if let Some(len) = length {
                if *len <= 0 {
                    &chars[start_idx..start_idx]
                } else {
                    let end_idx = (start_idx + *len as usize).min(chars.len());
                    &chars[start_idx..end_idx]
                }
            } else {
                &chars[start_idx..]
            };

            let substring: String = sub_chars.iter().collect();
            &substring == expected
        }
        DmnUnaryTest::Replace {
            pattern,
            replacement,
            flags,
            expected,
        } => {
            let Some(actual_str) = actual.as_str() else {
                return Ok(false);
            };
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
                if f.contains('x') {
                    builder.ignore_whitespace(true);
                }
            }
            match builder.build() {
                Ok(re) => {
                    let result = re.replace_all(actual_str, replacement.as_str());
                    result == *expected
                }
                Err(_) => false,
            }
        }
        DmnUnaryTest::ListContains { needle } => {
            let Some(actual_list) = actual.as_array() else {
                return Ok(false);
            };
            let resolved = match needle {
                crate::models::DmnListContainsNeedle::Literal(value) => value,
                crate::models::DmnListContainsNeedle::Variable(name) => {
                    inputs.get(name).unwrap_or(&Value::Null)
                }
            };
            actual_list.contains(resolved)
        }
        DmnUnaryTest::InList { values } => values.iter().any(|expected| actual == expected),

        // Per-row evaluated right-hand side (date aliases). Java builds
        // `#{input <op> date:now()}` and lets JUEL evaluate it each row
        // (ELInputEntryExpressionPreParser.java:26-29 + :53-62).
        DmnUnaryTest::DeferredComparison { operator, source } => {
            let scope: HashMap<String, Value> = inputs
                .iter()
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect();
            let expected = FeelExpressionEngine::new()
                .evaluate(source, &scope)
                .map_err(|error| {
                    // Java wraps input-entry failures in
                    // FlowableDmnExpressionException and rethrows, failing the
                    // decision (ELExpressionExecutor.java:57-60,
                    // RuleEngineExecutorImpl.java:199-208).
                    DmnError::execution(format!(
                        "failed to evaluate input entry expression '{source}': {error}"
                    ))
                })?;

            match operator {
                DmnDeferredOperator::Equals => *actual == expected,
                DmnDeferredOperator::NotEquals => *actual != expected,
                DmnDeferredOperator::GreaterThan => compare_values(actual, &expected, type_ref)
                    .is_some_and(|ordering| ordering == std::cmp::Ordering::Greater),
                DmnDeferredOperator::GreaterThanOrEqual => {
                    compare_values(actual, &expected, type_ref).is_some_and(|ordering| {
                        matches!(
                            ordering,
                            std::cmp::Ordering::Greater | std::cmp::Ordering::Equal
                        )
                    })
                }
                DmnDeferredOperator::LessThan => compare_values(actual, &expected, type_ref)
                    .is_some_and(|ordering| ordering == std::cmp::Ordering::Less),
                DmnDeferredOperator::LessThanOrEqual => compare_values(actual, &expected, type_ref)
                    .is_some_and(|ordering| {
                        matches!(
                            ordering,
                            std::cmp::Ordering::Less | std::cmp::Ordering::Equal
                        )
                    }),
            }
        }

        // EL pass-through: the entry is the whole condition and must yield a
        // boolean. Java's RuleExpressionCondition rejects null and non-Boolean
        // results (RuleExpressionCondition.java:36-50).
        DmnUnaryTest::ElCondition { source } => {
            let scope: HashMap<String, Value> = inputs
                .iter()
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect();
            let evaluated = FeelExpressionEngine::new()
                .evaluate(source, &scope)
                .map_err(|error| {
                    DmnError::execution(format!(
                        "failed to evaluate input entry expression '{source}': {error}"
                    ))
                })?;

            evaluated.as_bool().ok_or_else(|| {
                DmnError::execution(format!(
                    "input entry expression '{source}' returned non-Boolean: {evaluated}"
                ))
            })?
        }

        // `.property` shorthand. Java only takes this branch for typeRefs other
        // than date/number (ELInputEntryExpressionPreParser.java:39-47); Rust
        // applies the path to the resolved input value instead of splicing the
        // input variable name into an EL string.
        DmnUnaryTest::PropertyPath { path, test } => {
            let is_date_or_number = type_ref
                .map(crate::models::normalized_type_ref)
                .is_some_and(|type_ref| {
                    matches!(
                        type_ref.as_str(),
                        "date" | "number" | "double" | "integer" | "long"
                    )
                });
            if is_date_or_number {
                false
            } else {
                let mut current = actual;
                for key in path {
                    match current.get(key) {
                        Some(next) => current = next,
                        // Java would raise a PropertyNotFoundException here; the
                        // rule simply does not match rather than failing the
                        // decision, because absent optional properties are common.
                        None => return Ok(false),
                    }
                }
                unary_test_matches(test, current, None, inputs)?
            }
        }
    })
}

fn instance_of_matches(type_name: &str, actual: &Value) -> bool {
    match type_name.to_lowercase().as_str() {
        "string" => actual.is_string(),
        "number" | "double" | "long" | "integer" => actual.is_number(),
        "boolean" => actual.is_boolean(),
        "null" => actual.is_null(),
        "date" => {
            if let Some(s) = actual.as_str() {
                chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").is_ok()
            } else {
                false
            }
        }
        "time" => {
            if let Some(s) = actual.as_str() {
                chrono::NaiveTime::parse_from_str(s, "%H:%M:%S").is_ok()
            } else {
                false
            }
        }
        "datetime" => {
            if let Some(s) = actual.as_str() {
                chrono::DateTime::parse_from_rfc3339(s).is_ok()
                    || chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S").is_ok()
            } else {
                false
            }
        }
        "duration" => {
            if let Some(s) = actual.as_str() {
                s.starts_with('P')
            } else {
                false
            }
        }
        "context" => actual.is_object(),
        "list" => actual.is_array(),
        _ => false,
    }
}

fn string_function_matches(function: DmnStringFunction, actual: &Value, needle: &str) -> bool {
    let Some(actual) = actual.as_str() else {
        return false;
    };

    match function {
        DmnStringFunction::Contains => actual.contains(needle),
        DmnStringFunction::StartsWith => actual.starts_with(needle),
        DmnStringFunction::EndsWith => actual.ends_with(needle),
        DmnStringFunction::Matches => {
            regex::Regex::new(needle).is_ok_and(|regex| regex.is_match(actual))
        }
    }
}

fn string_transform_matches(
    transform: crate::models::DmnStringTransform,
    actual: &Value,
    expected: &str,
) -> bool {
    let Some(actual) = actual.as_str() else {
        return false;
    };

    match transform {
        crate::models::DmnStringTransform::LowerCase => actual.to_lowercase() == expected,
        crate::models::DmnStringTransform::UpperCase => actual.to_uppercase() == expected,
        crate::models::DmnStringTransform::StringLength => false,
    }
}

fn string_transform_comparison_matches(
    transform: crate::models::DmnStringTransform,
    operator: DmnComparisonOperator,
    actual: &Value,
    expected: &Value,
) -> bool {
    let Some(actual) = string_transform_comparison_value(transform, actual) else {
        return false;
    };
    compare_numbers(&actual, expected)
        .is_some_and(|ordering| comparison_matches(operator, ordering))
}

fn string_transform_comparison_value(
    transform: crate::models::DmnStringTransform,
    actual: &Value,
) -> Option<Value> {
    let actual = actual.as_str()?;

    match transform {
        crate::models::DmnStringTransform::StringLength => {
            Some(Value::from(actual.chars().count() as u64))
        }
        crate::models::DmnStringTransform::LowerCase
        | crate::models::DmnStringTransform::UpperCase => None,
    }
}

fn comparison_matches(operator: DmnComparisonOperator, ordering: std::cmp::Ordering) -> bool {
    match operator {
        DmnComparisonOperator::GreaterThan => ordering == std::cmp::Ordering::Greater,
        DmnComparisonOperator::GreaterThanOrEqual => {
            matches!(
                ordering,
                std::cmp::Ordering::Greater | std::cmp::Ordering::Equal
            )
        }
        DmnComparisonOperator::LessThan => ordering == std::cmp::Ordering::Less,
        DmnComparisonOperator::LessThanOrEqual => {
            matches!(
                ordering,
                std::cmp::Ordering::Less | std::cmp::Ordering::Equal
            )
        }
    }
}

fn range_matches(
    actual: &Value,
    start: &Value,
    end: &Value,
    start_inclusive: bool,
    end_inclusive: bool,
    type_ref: Option<&str>,
) -> bool {
    let Some(start_ordering) = compare_values(actual, start, type_ref) else {
        return false;
    };
    let Some(end_ordering) = compare_values(actual, end, type_ref) else {
        return false;
    };

    let starts_after = match start_ordering {
        std::cmp::Ordering::Greater => true,
        std::cmp::Ordering::Equal => start_inclusive,
        std::cmp::Ordering::Less => false,
    };
    let ends_before = match end_ordering {
        std::cmp::Ordering::Less => true,
        std::cmp::Ordering::Equal => end_inclusive,
        std::cmp::Ordering::Greater => false,
    };

    starts_after && ends_before
}

fn compare_values(
    actual: &Value,
    expected: &Value,
    type_ref: Option<&str>,
) -> Option<std::cmp::Ordering> {
    if let Some(type_ref) = type_ref
        && let Some(ordering) = compare_temporal_values(type_ref, actual, expected)
    {
        return Some(ordering);
    }

    compare_numbers(actual, expected)
}

fn compare_numbers(actual: &Value, expected: &Value) -> Option<std::cmp::Ordering> {
    let actual = actual.as_number()?;
    let expected = expected.as_number()?;

    match (
        actual.as_i64(),
        actual.as_u64(),
        expected.as_i64(),
        expected.as_u64(),
    ) {
        (Some(actual), _, Some(expected), _) => Some(actual.cmp(&expected)),
        (Some(actual), _, _, Some(expected)) => {
            if actual < 0 {
                Some(std::cmp::Ordering::Less)
            } else {
                Some((actual as u64).cmp(&expected))
            }
        }
        (_, Some(actual), Some(expected), _) => {
            if expected < 0 {
                Some(std::cmp::Ordering::Greater)
            } else {
                Some(actual.cmp(&(expected as u64)))
            }
        }
        (_, Some(actual), _, Some(expected)) => Some(actual.cmp(&expected)),
        _ => actual.as_f64()?.partial_cmp(&expected.as_f64()?),
    }
}

fn precompute_matching_rule_outputs(
    definition: &DmnDecisionDefinition,
    inputs: &Map<String, Value>,
) -> Result<RuleOutputCache, DmnError> {
    let mut cache = RuleOutputCache::new();
    for rule in &definition.rules {
        if rule_matches(definition, inputs, rule)? {
            cache.insert(
                rule.id.clone(),
                evaluate_rule_outputs(definition, rule, inputs)?,
            );
        }
    }
    Ok(cache)
}

fn cached_outputs_for_rule(
    evaluated_outputs: &RuleOutputCache,
    rule: &DmnRule,
) -> Result<Map<String, Value>, DmnError> {
    evaluated_outputs
        .get(&rule.id)
        .map(|evaluated| evaluated.values.clone())
        .ok_or_else(|| {
            DmnError::execution(format!(
                "internal error: missing precomputed outputs for rule '{}'",
                rule.id
            ))
        })
}

/// Evaluate all output entries for a matched rule in clause order.
/// Scope: coerced inputs + output defaults + prior outputs in this rule
/// (Java `ELExecutionContextBuilder.java:97-115`, `RuleEngineExecutorImpl.java:235-257`).
fn evaluate_rule_outputs(
    definition: &DmnDecisionDefinition,
    rule: &DmnRule,
    inputs: &Map<String, Value>,
) -> Result<EvaluatedRuleOutputs, DmnError> {
    let mut scope = build_output_evaluation_scope(definition, inputs);
    let mut values = Map::new();
    let mut conclusions = Vec::new();

    for (output_index, (output_clause, output_entry)) in definition
        .outputs
        .iter()
        .zip(&rule.output_entries)
        .enumerate()
    {
        let conclusion_id = output_entry
            .id
            .clone()
            .unwrap_or_else(|| format!("{}:output:{}", rule.id, output_index + 1));

        match evaluate_single_output_entry(definition, output_clause, output_entry, &scope)? {
            Some(coerced) => {
                // Put coerced result into scope for subsequent entries
                // (Java RuleEngineExecutorImpl.java:256-257).
                scope.insert(output_clause.name.clone(), coerced.clone());
                values.insert(output_clause.name.clone(), coerced.clone());
                conclusions.push(DmnExpressionExecution {
                    id: conclusion_id,
                    result: coerced,
                });
            }
            None => {
                // Empty entry: skip evaluation, audit records null
                // (Java RuleEngineExecutorImpl.java:291-296).
                conclusions.push(DmnExpressionExecution {
                    id: conclusion_id,
                    result: Value::Null,
                });
            }
        }
    }

    Ok(EvaluatedRuleOutputs {
        values,
        conclusions,
    })
}

/// Build the JUEL/FEEL variable stack for output evaluation.
/// Java `ELExecutionContextBuilder.java:97-115`: boolean input defaults FALSE;
/// missing output vars get number→0, date→now, else "".
fn build_output_evaluation_scope(
    definition: &DmnDecisionDefinition,
    inputs: &Map<String, Value>,
) -> HashMap<String, Value> {
    let mut scope: HashMap<String, Value> = inputs
        .iter()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();

    // Boolean inputs default to FALSE when absent
    // (Java ELExecutionContextBuilder.java:97-100).
    for input_clause in &definition.inputs {
        if let Some(type_ref) = input_clause.type_ref.as_deref()
            && normalized_type_ref(type_ref) == "boolean"
            && !scope.contains_key(&input_clause.input_variable)
        {
            scope.insert(input_clause.input_variable.clone(), Value::Bool(false));
        }
    }

    // Output variable defaults when missing or null
    // (Java ELExecutionContextBuilder.java:105-114).
    for output_clause in &definition.outputs {
        let missing_or_null = match scope.get(&output_clause.name) {
            None => true,
            Some(Value::Null) => true,
            Some(_) => false,
        };
        if missing_or_null {
            let default_value = match output_clause
                .type_ref
                .as_deref()
                .map(normalized_type_ref)
                .as_deref()
            {
                // Java only special-cases typeRef "number" → 0D and "date" → new Date().
                Some("number") => Value::from(0.0),
                Some("date") => Value::String(Utc::now().to_rfc3339()),
                _ => Value::String(String::new()),
            };
            scope.insert(output_clause.name.clone(), default_value);
        }
    }

    scope
}

/// Evaluate one output entry: empty/`-` skip; peel shells; FEEL eval; typeRef coerce.
/// Returns `None` when the entry is empty and should be omitted from the result map.
fn evaluate_single_output_entry(
    definition: &DmnDecisionDefinition,
    output_clause: &DmnOutputClause,
    output_entry: &DmnRuleOutputEntry,
    scope: &HashMap<String, Value>,
) -> Result<Option<Value>, DmnError> {
    let raw = output_entry.expression.trim();

    // Empty / dash: Java skips evaluation (RuleEngineExecutorImpl.java:291-296).
    // Legacy path: pre-P81 deployments and `DmnRuleOutputEntry::new` only set
    // `value` with empty `expression` — use the static snapshot instead of skipping.
    if raw.is_empty() || raw == "-" {
        if !output_entry.value.is_null() {
            return Ok(Some(coerce_runtime_output_value(
                definition,
                output_clause,
                &output_entry.value,
            )?));
        }
        return Ok(None);
    }

    // Peel #{...} / ${...} then evaluate FEEL (not engine-common SimpleExpression).
    let expr = strip_expression_shells(raw);
    // Explicit empty handling — do not rely on FeelExpressionEngine blank→Null
    // (Java skips; FEEL engine Null would incorrectly populate the result).
    if expr.is_empty() {
        return Ok(None);
    }

    let engine = FeelExpressionEngine::new();
    let evaluated = engine.evaluate(expr, scope).map_err(|error| {
        // Expression failure fails the whole decision execution
        // (Java RuleEngineExecutorImpl.java:274-289) — no literal fallback.
        DmnError::execution(format!(
            "failed to evaluate output expression for decision '{}' output '{}': {}",
            definition.key, output_clause.name, error
        ))
    })?;

    // Evaluate first, then typeRef coerce (Java RuleEngineExecutorImpl.java:253-254).
    let coerced = coerce_runtime_output_value(definition, output_clause, &evaluated)?;
    Ok(Some(coerced))
}

fn coerce_runtime_output_value(
    definition: &DmnDecisionDefinition,
    output_clause: &DmnOutputClause,
    value: &Value,
) -> Result<Value, DmnError> {
    let Some(type_ref) = output_clause.type_ref.as_deref() else {
        return Ok(value.clone());
    };
    if value.is_null() {
        return Ok(Value::Null);
    }

    match normalized_type_ref(type_ref).as_str() {
        "string" => value
            .as_str()
            .map(|value| Value::String(value.to_string()))
            .ok_or_else(|| incompatible_output_type_ref_error(definition, output_clause, value)),
        "boolean" => value
            .as_bool()
            .map(Value::Bool)
            .ok_or_else(|| incompatible_output_type_ref_error(definition, output_clause, value)),
        "integer" => coerce_runtime_integer_output(
            definition,
            output_clause,
            value,
            i32::MIN as i64,
            i32::MAX as i64,
        ),
        "long" => {
            coerce_runtime_integer_output(definition, output_clause, value, i64::MIN, i64::MAX)
        }
        // Java ExecutionVariableFactory.java:60-69 — typeRef "number" always
        // yields Double (integer 7 → 7.0). Sole engine exit is
        // RuleEngineExecutorImpl.java:246,253-254 → getExecutionVariable.
        // Large integers > 2^53 lose precision, matching Double.valueOf.
        "double" | "number" => {
            let number = numeric_value(value).ok_or_else(|| {
                incompatible_output_type_ref_error(definition, output_clause, value)
            })?;
            number.as_f64().map(Value::from).ok_or_else(|| {
                incompatible_output_type_ref_error(definition, output_clause, value)
            })
        }
        "date" | "time" | "datetime" | "duration" | "daytimeduration" | "yearmonthduration" => {
            normalize_temporal_value(type_ref, value)
                .ok_or_else(|| incompatible_output_type_ref_error(definition, output_clause, value))
        }
        "context" => value
            .as_object()
            .map(|value| Value::Object(value.clone()))
            .ok_or_else(|| incompatible_output_type_ref_error(definition, output_clause, value)),
        "list" => value
            .as_array()
            .map(|value| Value::Array(value.clone()))
            .ok_or_else(|| incompatible_output_type_ref_error(definition, output_clause, value)),
        _ => Err(DmnError::unsupported(
            "typeRef",
            format!(
                "unsupported output typeRef '{}' for decision '{}' output '{}'; supported output typeRefs are string, boolean, integer, long, double, number, date, time, dateTime, date and time, duration, dayTimeDuration, yearMonthDuration, context, and list",
                type_ref, definition.key, output_clause.name
            ),
        )),
    }
}

fn coerce_runtime_integer_output(
    definition: &DmnDecisionDefinition,
    output_clause: &DmnOutputClause,
    value: &Value,
    min: i64,
    max: i64,
) -> Result<Value, DmnError> {
    let Some(number) = numeric_value(value) else {
        return Err(incompatible_output_type_ref_error(
            definition,
            output_clause,
            value,
        ));
    };
    let Some(integer) = number_to_i64(&number) else {
        return Err(incompatible_output_type_ref_error(
            definition,
            output_clause,
            value,
        ));
    };
    if integer < min || integer > max {
        return Err(incompatible_output_type_ref_error(
            definition,
            output_clause,
            value,
        ));
    }

    Ok(Value::from(integer))
}

fn incompatible_output_type_ref_error(
    definition: &DmnDecisionDefinition,
    output_clause: &DmnOutputClause,
    value: &Value,
) -> DmnError {
    DmnError::execution(format!(
        "DMN decision '{}' output '{}' with typeRef '{}' produced incompatible value {}",
        definition.key,
        output_clause.name,
        output_clause.type_ref.as_deref().unwrap_or("<none>"),
        value
    ))
}

fn joined_rule_ids(rules: &[&DmnRule]) -> String {
    rules
        .iter()
        .map(|rule| rule.id.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

/// Audit row for a decision whose evaluation raised.
///
/// Java records the failure on the audit container rather than aborting
/// (`RuleEngineExecutorImpl.java:94-97,154-158`), so `finalizeDecisionExecutionAudit`
/// (`DmnDecisionServiceImpl.java:238-248`) still persists a history row via
/// `PersistHistoricDecisionExecutionCmd` — with `FAILED_ = true` (:62-65) and,
/// because `executionContext.getRuleResults().clear()` ran (:156), no results.
/// Correlation/tenant/business-key columns (:56-60) are written as on the happy
/// path; the caller then throws (`DmnActivityBehavior.java:112-115`).
fn failed_history_record(
    definition: &DmnDecisionDefinition,
    request: &DmnExecutionRequest,
    inputs: Map<String, Value>,
) -> HistoricDecisionExecution {
    HistoricDecisionExecution {
        execution_id: format!("dmn-execution:{}", Uuid::new_v4()),
        decision_definition_id: definition.id.clone(),
        deployment_id: definition.deployment_id.clone(),
        decision_key: definition.key.clone(),
        decision_name: definition.name.clone(),
        decision_version: definition.version,
        hit_policy: definition.hit_policy.clone(),
        // Java clears rule results on failure (`RuleEngineExecutorImpl.java:156`),
        // so no rule matched/was audited from this engine's point of view.
        matched_rule_id: None,
        matched_rule_count: 0,
        rule_executions: Vec::new(),
        business_key: request.business_key.clone(),
        tenant_id: request.tenant_id.clone(),
        instance_id: request.instance_id.clone(),
        scope_execution_id: request.execution_id.clone(),
        activity_id: request.activity_id.clone(),
        scope_type: request.scope_type.clone(),
        failed: true,
        executed_at: Utc::now(),
        inputs,
        decision_result: Vec::new(),
        multiple_results: false,
        decision_service_result: None,
        // Hard failures carry no validationMessage in Java — the audit
        // container's failed/exception path is separate
        // (`DecisionExecutionAuditContainer.setFailedWithException`).
        validation_message: None,
    }
}

fn persist_history(store: &DmnStore, historic: &HistoricDecisionExecution) -> Result<(), DmnError> {
    let mut session = store.create_session()?;
    let mut entity = DmnExecutionHistoryEntity::new(
        historic.execution_id.clone(),
        historic.decision_key.clone(),
        historic.decision_definition_id.clone(),
        historic.deployment_id.clone(),
        historic.executed_at.to_rfc3339(),
        serde_json::to_string(historic)?,
    );
    entity.set_business_key(historic.business_key.clone());
    entity.set_tenant_id(historic.tenant_id.clone());
    // Java `PersistHistoricDecisionExecutionCmd.java:56-59`.
    entity.set_scope_correlation(
        historic.instance_id.clone(),
        historic.scope_execution_id.clone(),
        historic.activity_id.clone(),
        historic.scope_type.clone(),
    );
    let manager = DmnExecutionHistoryDataManager::new();
    manager.insert(&mut session, entity)?;
    session.commit()?;
    Ok(())
}
