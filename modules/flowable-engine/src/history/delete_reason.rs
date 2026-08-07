//! Java-parity delete reason constants
//! (`org.flowable.engine.history.DeleteReason`).
//!
//! Used when ending historic process/task/activity instances that did not
//! complete normally. Strings must match Java exactly so history queries and
//! REST responses stay interoperable.
//!
//! Some runtime paths append an activity id to the bare constant (see
//! `boundary_event_interrupting` / `event_subprocess_interrupting` /
//! `terminate_end_event`). Call sites should use those helpers when Java
//! does; bare constants remain for equality checks against the interface
//! literals and for reasons that never get a suffix.

/// Java `DeleteReason.PROCESS_INSTANCE_DELETED`.
pub const PROCESS_INSTANCE_DELETED: &str = "process instance deleted";

/// Java `DeleteReason.TERMINATE_END_EVENT` bare constant (without activity id).
/// Runtime terminate paths often append ` ({activityId})`.
pub const TERMINATE_END_EVENT: &str = "terminate end event";

/// Java `DeleteReason.BOUNDARY_EVENT_INTERRUPTING`.
pub const BOUNDARY_EVENT_INTERRUPTING: &str = "boundary event";

/// Java `DeleteReason.EVENT_SUBPROCESS_INTERRUPTING`.
pub const EVENT_SUBPROCESS_INTERRUPTING: &str = "event subprocess";

/// Java `DeleteReason.EVENT_BASED_GATEWAY_CANCEL`.
pub const EVENT_BASED_GATEWAY_CANCEL: &str = "event based gateway cancel";

/// Java `DeleteReason.TRANSACTION_CANCELED`.
pub const TRANSACTION_CANCELED: &str = "transaction canceled";

/// Java `BoundaryEventActivityBehavior#deleteChildExecutions`:
/// `BOUNDARY_EVENT_INTERRUPTING + " (" + outgoingExecutionEntity.getCurrentActivityId() + ")"`.
pub fn boundary_event_interrupting(boundary_activity_id: &str) -> String {
    format!("{BOUNDARY_EVENT_INTERRUPTING} ({boundary_activity_id})")
}

/// Java `EventSubProcess*StartEventActivityBehavior#trigger`:
/// `EVENT_SUBPROCESS_INTERRUPTING + "(" + startEvent.getId() + ")"`
/// (no space before the parenthesis — matches Java string concat).
pub fn event_subprocess_interrupting(start_event_id: &str) -> String {
    format!("{EVENT_SUBPROCESS_INTERRUPTING}({start_event_id})")
}

/// Java `TerminateEndEventActivityBehavior#createDeleteReason`:
/// `TERMINATE_END_EVENT + " (" + activityId + ")"`.
pub fn terminate_end_event(activity_id: &str) -> String {
    format!("{TERMINATE_END_EVENT} ({activity_id})")
}
