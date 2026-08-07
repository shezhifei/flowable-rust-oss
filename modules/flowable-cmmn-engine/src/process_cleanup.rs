use crate::error::CmmnError;

/// Cross-engine hook used by CMMN cascade delete to remove BPMN child process
/// instances started by process tasks.
///
/// When no cleanup service is injected, cascade delete refuses to proceed with a
/// deterministic conflict so BPMN children are never left as silent orphans.
pub trait ProcessInstanceCleanup: Send + Sync {
    /// Delete a BPMN process instance with cascade semantics (runtime + owned children).
    fn delete_process_instance_cascade(&self, process_instance_id: &str) -> Result<(), CmmnError>;
}
