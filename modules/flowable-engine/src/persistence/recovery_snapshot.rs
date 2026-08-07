use crate::persistence::runtime_store::{
    EventSubprocessEventSubscription, EventSubprocessTimerSubscription,
    ProcessEventStartSubscription, ProcessTimerStartSubscription, RuntimeBoundaryEventState,
    RuntimeEventWaitState, RuntimeTimerJobState,
};
use crate::repository::deployment::Deployment;
use crate::repository::process_definition::ProcessDefinition;
use crate::runtime::execution::Execution;
use crate::runtime::process_instance::ProcessInstance;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SnapshotDeployment {
    pub deployment: Deployment,
    pub resources: HashMap<String, Vec<u8>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RecoverySnapshot {
    pub deployments: Vec<SnapshotDeployment>,
    pub process_definitions: Vec<ProcessDefinition>,

    // Deployed process subscriptions
    pub process_timer_start_subscriptions: Vec<ProcessTimerStartSubscription>,
    pub process_event_start_subscriptions: Vec<ProcessEventStartSubscription>,

    // Runtime entity state
    pub process_instances: Vec<ProcessInstance>,
    pub executions: Vec<Execution>,

    // Wait states
    pub event_wait_states: Vec<RuntimeEventWaitState>,
    pub boundary_event_states: Vec<RuntimeBoundaryEventState>,

    // Job states
    pub timer_job_states: Vec<RuntimeTimerJobState>,

    // Subprocess subscriptions
    pub event_subprocess_timer_subscriptions: Vec<EventSubprocessTimerSubscription>,
    pub event_subprocess_event_subscriptions: Vec<EventSubprocessEventSubscription>,

    #[serde(default)]
    pub tasks: Vec<crate::task::Task>,
}
