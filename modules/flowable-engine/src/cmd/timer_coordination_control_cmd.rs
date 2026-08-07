use std::sync::Arc;

use crate::interceptor::command::Command;
use crate::interceptor::command_context::CommandContext;
use crate::persistence::runtime_store::{TimerCoordinatorStatus, TimerNodeStatus};

fn wrong_command_variant(result_type: &str) -> crate::error::FlowableError {
    crate::error::FlowableError::Internal(format!(
        "Wrong timer coordination command variant for {result_type}"
    ))
}

pub enum TimerCoordinationControlCmd {
    Status,
    Nodes,
    Release {
        node_id: Arc<str>,
        fencing_token: i64,
    },
    StepDown,
    Deregister {
        node_id: Arc<str>,
    },
    Cleanup,
    Audit(crate::service::audit::TimerAdminAuditInput),
}

impl TimerCoordinationControlCmd {
    pub fn status() -> Self {
        Self::Status
    }

    pub fn nodes() -> Self {
        Self::Nodes
    }

    pub fn release(node_id: Arc<str>, fencing_token: i64) -> Self {
        Self::Release {
            node_id,
            fencing_token,
        }
    }

    pub fn step_down() -> Self {
        Self::StepDown
    }

    pub fn deregister(node_id: Arc<str>) -> Self {
        Self::Deregister { node_id }
    }

    pub fn cleanup() -> Self {
        Self::Cleanup
    }

    pub fn audit(input: crate::service::audit::TimerAdminAuditInput) -> Self {
        Self::Audit(input)
    }
}

#[derive(Debug, Clone)]
pub struct CoordinatorStatusResult {
    pub status: TimerCoordinatorStatus,
}

#[derive(Debug, Clone)]
pub struct NodesListResult {
    pub nodes: Vec<TimerNodeStatus>,
}

#[derive(Debug, Clone)]
pub struct ReleaseResult {
    pub success: bool,
}

#[derive(Debug, Clone)]
pub struct StepDownResult {
    pub success: bool,
    pub new_fencing_token: i64,
}

#[derive(Debug, Clone)]
pub struct DeregisterResult {
    pub success: bool,
}

#[derive(Debug, Clone)]
pub struct CleanupResult {
    pub cleaned_count: usize,
}

#[derive(Debug, Clone)]
pub struct AuditAdminActionResult {}

impl Command<CoordinatorStatusResult> for TimerCoordinationControlCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<CoordinatorStatusResult, crate::error::FlowableError> {
        match self {
            TimerCoordinationControlCmd::Status => {
                let (store, session) = command_context.store_and_session();
                let status = store.get_timer_coordinator_status(session);
                Ok(CoordinatorStatusResult { status })
            }
            _ => Err(wrong_command_variant("CoordinatorStatusResult")),
        }
    }
}

impl Command<NodesListResult> for TimerCoordinationControlCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<NodesListResult, crate::error::FlowableError> {
        match self {
            TimerCoordinationControlCmd::Nodes => {
                let (store, session) = command_context.store_and_session();
                let nodes = store.list_timer_nodes(session);
                Ok(NodesListResult { nodes })
            }
            _ => Err(wrong_command_variant("NodesListResult")),
        }
    }
}

impl Command<ReleaseResult> for TimerCoordinationControlCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<ReleaseResult, crate::error::FlowableError> {
        match self {
            TimerCoordinationControlCmd::Release {
                node_id,
                fencing_token,
            } => {
                let (store, session) = command_context.store_and_session();
                let success = store.release_coordinator_lease(
                    "timer-coordinator",
                    node_id.as_ref(),
                    *fencing_token,
                    session,
                );
                Ok(ReleaseResult { success })
            }
            _ => Err(wrong_command_variant("ReleaseResult")),
        }
    }
}

impl Command<StepDownResult> for TimerCoordinationControlCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<StepDownResult, crate::error::FlowableError> {
        match self {
            TimerCoordinationControlCmd::StepDown => {
                let (store, session) = command_context.store_and_session();
                let success = store.force_step_down(session);
                let new_fencing_token = if success {
                    let status = store.get_timer_coordinator_status(session);
                    status.fencing_token
                } else {
                    0
                };
                Ok(StepDownResult {
                    success,
                    new_fencing_token,
                })
            }
            _ => Err(wrong_command_variant("StepDownResult")),
        }
    }
}

impl Command<DeregisterResult> for TimerCoordinationControlCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<DeregisterResult, crate::error::FlowableError> {
        match self {
            TimerCoordinationControlCmd::Deregister { node_id } => {
                let (store, session) = command_context.store_and_session();
                let success = store.deregister_timer_node(node_id.as_ref(), session);
                Ok(DeregisterResult { success })
            }
            _ => Err(wrong_command_variant("DeregisterResult")),
        }
    }
}

impl Command<CleanupResult> for TimerCoordinationControlCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<CleanupResult, crate::error::FlowableError> {
        match self {
            TimerCoordinationControlCmd::Cleanup => {
                let (store, session) = command_context.store_and_session();
                let cleaned_count = store.cleanup_expired_timer_nodes(session);
                Ok(CleanupResult { cleaned_count })
            }
            _ => Err(wrong_command_variant("CleanupResult")),
        }
    }
}

impl Command<AuditAdminActionResult> for TimerCoordinationControlCmd {
    fn execute(
        &self,
        command_context: &mut CommandContext,
    ) -> Result<AuditAdminActionResult, crate::error::FlowableError> {
        match self {
            TimerCoordinationControlCmd::Audit(input) => {
                let now = command_context
                    .runtime_store
                    .time_source()
                    .now()
                    .timestamp_millis();
                let record = crate::service::audit::TimerAdminAuditRecord {
                    id: uuid::Uuid::new_v4().to_string(),
                    request_id: input.request_id.clone(),
                    timestamp: now,
                    tenant_id: input.tenant_id.clone(),
                    issuer: input.issuer.clone(),
                    subject: input.subject.clone(),
                    actor: input.actor.clone(),
                    action: input.action.clone(),
                    target: input.target.clone(),
                    outcome: input.outcome.clone(),
                    profile_id: input.profile_id.clone(),
                };
                let (store, session) = command_context.store_and_session();
                store.insert_timer_admin_audit_record(record, session);
                Ok(AuditAdminActionResult {})
            }
            _ => Err(wrong_command_variant("AuditAdminActionResult")),
        }
    }
}
