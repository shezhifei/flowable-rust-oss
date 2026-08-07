use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::interceptor::command_context::CommandContext;
use crate::runtime::execution::Execution;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompensationSubscription {
    pub id: String,
    pub process_instance_id: String,
    pub execution_id: String,
    pub activity_id: String,
    pub compensation_activity_id: String,
    #[serde(default)]
    pub subscription_order: i64,
    /// Snapshot of the scope variables visible when the compensated activity
    /// completed. Java parity: `ScopeUtil.createCopyOfSubProcessExecutionForCompensation`
    /// copies the scope's (non-transient) variables onto the compensation
    /// event-scope execution at subscription-creation time, so later variable
    /// writes are invisible to the compensation handler.
    #[serde(default)]
    pub variables_snapshot: HashMap<String, serde_json::Value>,
}

/// Captures the variables the completed activity's execution could resolve at
/// completion time (own maps plus the parent chain / process-instance scope),
/// excluding the execution's own transient variables (Java copies only
/// non-transient variables).
///
/// P44 取证: Java `ScopeUtil.createCopyOfSubProcessExecutionForCompensation`
/// calls `getVariables()` which walks the scope chain collecting non-transient
/// variables only. The Rust `evaluation_execution` helper merges the root
/// execution's `transient_variables` into the regular variable map before this
/// function runs, so root-scope transients currently leak into the snapshot.
/// The execution's OWN transient variables are correctly excluded (only
/// `.variables` and `.local_variables` are read below). Closing the root
/// transient leak is deferred to P45 (transient variable lifecycle) because the
/// transient/non-transient distinction is already lost inside the merge.
pub(crate) fn snapshot_scope_variables(
    command_context: &mut CommandContext,
    execution: &Execution,
) -> HashMap<String, serde_json::Value> {
    let evaluation_execution =
        crate::engine::variable_service::evaluation_execution(command_context, execution);
    let mut snapshot = evaluation_execution.variables;
    snapshot.extend(evaluation_execution.local_variables);
    snapshot
}
