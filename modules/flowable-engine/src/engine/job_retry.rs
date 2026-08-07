use crate::el::expression::{Expression, SimpleExpression};
use crate::engine::time_source::parse_iso8601_duration;
use crate::error::FlowableError;
use crate::runtime::execution::Execution;
use serde_json::Value;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FailedJobRetryCycle {
    pub(crate) repetitions: i32,
    pub(crate) delay_ms: i64,
}

pub(crate) fn resolve_failed_job_retry_cycle(
    raw_value: &str,
    execution: &Execution,
) -> Result<FailedJobRetryCycle, FlowableError> {
    let resolved = resolve_cycle_text(raw_value, execution)?;
    parse_failed_job_retry_cycle(&resolved).ok_or_else(|| {
        FlowableError::ExecutionError(format!(
            "failedJobRetryTimeCycle has invalid value '{resolved}' for execution '{}'",
            execution.id
        ))
    })
}

fn resolve_cycle_text(raw_value: &str, execution: &Execution) -> Result<String, FlowableError> {
    let trimmed = raw_value.trim();
    if !(trimmed.starts_with("${") && trimmed.ends_with('}')) {
        return Ok(trimmed.to_string());
    }
    let value = SimpleExpression::new(trimmed.to_string())
        .get_value(execution)
        .ok_or_else(|| {
            FlowableError::ExecutionError(format!(
                "failedJobRetryTimeCycle expression '{trimmed}' resolved to no value for execution '{}'",
                execution.id
            ))
        })?;
    match value {
        Value::String(value) => Ok(value),
        Value::Null => Err(FlowableError::ExecutionError(format!(
            "failedJobRetryTimeCycle expression '{trimmed}' resolved to null for execution '{}'",
            execution.id
        ))),
        value => Ok(value.to_string()),
    }
}

fn parse_failed_job_retry_cycle(value: &str) -> Option<FailedJobRetryCycle> {
    let (repeat, duration) = value.trim().split_once('/')?;
    let repetitions = repeat.strip_prefix('R')?.parse::<i32>().ok()?;
    if repetitions <= 0 {
        return None;
    }
    let delay_ms = parse_iso8601_duration(duration)?;
    Some(FailedJobRetryCycle {
        repetitions,
        delay_ms,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn resolves_literal_and_expression_retry_cycles() {
        let mut execution = Execution {
            id: "execution-1".to_string(),
            ..Default::default()
        };
        execution.set_process_variable("retryCycle".to_string(), json!("R2/PT1M"));

        assert_eq!(
            resolve_failed_job_retry_cycle("R3/PT10S", &execution).unwrap(),
            FailedJobRetryCycle {
                repetitions: 3,
                delay_ms: 10_000,
            }
        );
        assert_eq!(
            resolve_failed_job_retry_cycle("${retryCycle}", &execution).unwrap(),
            FailedJobRetryCycle {
                repetitions: 2,
                delay_ms: 60_000,
            }
        );
    }

    #[test]
    fn rejects_invalid_retry_cycles() {
        let execution = Execution {
            id: "execution-1".to_string(),
            ..Default::default()
        };
        assert!(resolve_failed_job_retry_cycle("PT1M", &execution).is_err());
        assert!(resolve_failed_job_retry_cycle("R0/PT1M", &execution).is_err());
        assert!(resolve_failed_job_retry_cycle("${missing}", &execution).is_err());
    }
}
