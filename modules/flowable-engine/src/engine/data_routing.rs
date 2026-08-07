use crate::error::FlowableError;
use flowable_bpmn_model::model::{BpmnModel, DataAssociation, DataStore};
use std::collections::HashMap;

pub struct DataRoutingService;

impl DataRoutingService {
    pub fn resolve_data_store<'a>(
        model: &'a BpmnModel,
        data_store_ref: &str,
    ) -> Result<&'a DataStore, FlowableError> {
        model.data_stores.get(data_store_ref).ok_or_else(|| {
            FlowableError::DeploymentValidationError(format!(
                "DataStore reference {} not found in model",
                data_store_ref
            ))
        })
    }

    pub fn apply_data_input_associations(
        associations: &[DataAssociation],
        process_variables: &HashMap<String, serde_json::Value>,
    ) -> Result<HashMap<String, serde_json::Value>, FlowableError> {
        let mut task_vars = HashMap::new();
        for assoc in associations {
            for assignment in &assoc.assignments {
                if let Some(from_expr) = &assignment.from
                    && let Some(to_expr) = &assignment.to
                {
                    let val = resolve_from_expr(from_expr, process_variables);
                    let target_name = extract_target_name(to_expr);
                    task_vars.insert(target_name, val);
                }
            }

            if let Some(source) = &assoc.source_ref
                && let Some(target) = &assoc.target_ref
                && let Some(val) = process_variables.get(source)
            {
                task_vars.insert(target.clone(), val.clone());
            }
        }
        Ok(task_vars)
    }

    pub fn apply_data_output_associations(
        associations: &[DataAssociation],
        task_variables: &HashMap<String, serde_json::Value>,
        process_variables: &mut HashMap<String, serde_json::Value>,
    ) -> Result<(), FlowableError> {
        for assoc in associations {
            for assignment in &assoc.assignments {
                if let Some(from_expr) = &assignment.from
                    && let Some(to_expr) = &assignment.to
                {
                    let val = resolve_from_expr(from_expr, task_variables);
                    let target_name = extract_target_name(to_expr);
                    process_variables.insert(target_name, val);
                }
            }

            if let Some(source) = &assoc.source_ref
                && let Some(target) = &assoc.target_ref
                && let Some(val) = task_variables.get(source)
            {
                process_variables.insert(target.clone(), val.clone());
            }
        }
        Ok(())
    }
}

fn resolve_from_expr(
    from_expr: &str,
    variables: &HashMap<String, serde_json::Value>,
) -> serde_json::Value {
    let clean_from = from_expr.trim();
    let var_name = if clean_from.starts_with("${") && clean_from.ends_with('}') {
        clean_from[2..clean_from.len() - 1].trim()
    } else {
        clean_from
    };

    if let Some(v) = variables.get(var_name) {
        v.clone()
    } else {
        if var_name.eq_ignore_ascii_case("true") {
            serde_json::Value::Bool(true)
        } else if var_name.eq_ignore_ascii_case("false") {
            serde_json::Value::Bool(false)
        } else if let Ok(n) = var_name.parse::<i64>() {
            serde_json::Value::Number(n.into())
        } else if let Ok(f) = var_name.parse::<f64>() {
            if let Some(n) = serde_json::Number::from_f64(f) {
                serde_json::Value::Number(n)
            } else {
                serde_json::Value::String(var_name.to_string())
            }
        } else {
            serde_json::Value::String(var_name.to_string())
        }
    }
}

fn extract_target_name(to_expr: &str) -> String {
    let clean_to = to_expr.trim();
    if clean_to.starts_with("${") && clean_to.ends_with('}') {
        clean_to[2..clean_to.len() - 1].trim().to_string()
    } else {
        clean_to.to_string()
    }
}
