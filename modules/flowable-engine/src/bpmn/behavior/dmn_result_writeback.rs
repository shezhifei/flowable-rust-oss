//! Shared DMN result writeback onto process executions.
//!
//! Java reference: `DmnActivityBehavior.java:197-267`
//! - Decision service multi/single → ObjectNode under decisionServiceKey / flat outputs
//! - Decision multi-hit → JSON array under decisionKey
//! - Decision single-hit → each output as its own process variable
//! - Rust extension (businessRuleTask only): `resultVariableName` overrides the write
//!   target name when set. serviceTask type=dmn always passes `None`.

use crate::runtime::execution::Execution;
use flowable_dmn_engine::DmnExecutionResult;
use serde_json::{Map, Value};

/// Write DMN results onto the process execution.
///
/// Java reference: `DmnActivityBehavior.java:197-267`
pub(crate) fn write_dmn_result_to_execution(
    execution: &mut Execution,
    decision_key: &str,
    result: &DmnExecutionResult,
    result_variable_name: Option<&str>,
    multiple_results: bool,
) {
    // Decision service path (Java setDecisionServiceVariablesOnExecution :197-234)
    if let Some(service_result) = result.decision_service_result.as_ref() {
        write_decision_service_variables(
            execution,
            decision_key,
            service_result,
            result_variable_name,
            multiple_results,
        );
        return;
    }

    // Single decision path (Java setVariablesOnExecution :236-267)
    write_decision_variables(
        execution,
        decision_key,
        &result.decision_result,
        result_variable_name,
        multiple_results,
    );
}

fn write_decision_variables(
    execution: &mut Execution,
    decision_key: &str,
    decision_result: &[Map<String, Value>],
    result_variable_name: Option<&str>,
    multiple_results: bool,
) {
    // Java: if (executionResult == null || (executionResult.isEmpty() && !multipleResults)) return;
    if decision_result.is_empty() && !multiple_results {
        return;
    }

    // Multi-hit: size > 1 OR multipleResults flag → JSON array under decisionKey
    // (Java DmnActivityBehavior.java:244-257)
    if decision_result.len() > 1 || multiple_results {
        let array = Value::Array(
            decision_result
                .iter()
                .map(|row| Value::Object(row.clone()))
                .collect(),
        );
        let name = result_variable_name.unwrap_or(decision_key);
        execution.set_process_variable(name.to_string(), array);
        return;
    }

    // Single rule result (Java :258-266)
    let Some(row) = decision_result.first() else {
        return;
    };

    if let Some(variable_name) = result_variable_name {
        // Rust extension (businessRuleTask): prefer resultVariableName — write the
        // whole row (or single scalar if only one output) under that name.
        if row.len() == 1 {
            if let Some((_, value)) = row.iter().next() {
                execution.set_process_variable(variable_name.to_string(), value.clone());
            }
        } else {
            execution.set_process_variable(variable_name.to_string(), Value::Object(row.clone()));
        }
    } else {
        for (name, value) in row {
            execution.set_process_variable(name.clone(), value.clone());
        }
    }
}

fn write_decision_service_variables(
    execution: &mut Execution,
    decision_service_key: &str,
    service_result: &std::collections::BTreeMap<String, Vec<Map<String, Value>>>,
    result_variable_name: Option<&str>,
    multiple_results: bool,
) {
    // Java: if (executionResult == null || (executionResult.isEmpty() && !multipleResults)) return;
    if service_result.is_empty() && !multiple_results {
        return;
    }

    let has_multiple = service_result.values().any(|rows| rows.len() > 1) || multiple_results;

    if has_multiple {
        // Java setDecisionServiceVariablesOnExecution :205-221 —
        // ObjectNode: decisionKey → ArrayNode of row ObjectNodes
        let mut decision_result_node = Map::new();
        for (decision_name, rows) in service_result {
            let array = Value::Array(
                rows.iter()
                    .map(|row| Value::Object(row.clone()))
                    .collect(),
            );
            decision_result_node.insert(decision_name.clone(), array);
        }
        let name = result_variable_name.unwrap_or(decision_service_key);
        execution.set_process_variable(name.to_string(), Value::Object(decision_result_node));
    } else {
        // Single rule per decision — flat write each output
        // (Java :222-231); resultVariableName takes the whole merged map.
        if let Some(variable_name) = result_variable_name {
            let mut merged = Map::new();
            for rows in service_result.values() {
                if let Some(row) = rows.first() {
                    for (k, v) in row {
                        merged.insert(k.clone(), v.clone());
                    }
                }
            }
            execution.set_process_variable(variable_name.to_string(), Value::Object(merged));
        } else {
            for rows in service_result.values() {
                if let Some(row) = rows.first() {
                    for (name, value) in row {
                        execution.set_process_variable(name.clone(), value.clone());
                    }
                }
            }
        }
    }
}
