//! Optional outbound Event Registry dispatch hook for BPMN `send-event` tasks.
//!
//! Java: `SendEventTaskActivityBehavior` → `EventRegistry.sendEventOutbound` →
//! `DefaultOutboundEventProcessor` (transform + channel adapter). The Rust
//! event-registry-service crate depends on this engine crate, so the BPMN path
//! cannot call the service pipeline directly (cycle). Hosts (and
//! `FlowableEventRegistryService`) install a hook on
//! [`crate::service::config::ProcessEngineConfiguration::outbound_event_dispatch`].
//!
//! When no hook is installed the registry no-ops successfully — parity with an
//! in-memory adapter for engine-only unit tests.

use crate::error::FlowableError;
use serde_json::Value;
use std::fmt;
use std::sync::{Arc, Mutex};

/// Payload handed from the BPMN send-event activity to the outbound pipeline.
#[derive(Debug, Clone)]
pub struct OutboundEventDispatchRequest {
    pub channel_key: String,
    pub channel_configuration: Value,
    pub event_type: String,
    pub payload: Value,
    pub dispatch_token: Option<String>,
}

/// Host-provided transform + channel-adapter dispatch (service crate implements).
pub trait OutboundEventDispatchHook: Send + Sync {
    fn dispatch_outbound(
        &self,
        request: &OutboundEventDispatchRequest,
    ) -> Result<(), FlowableError>;
}

/// Object-safe handle stored on the shared dispatch registry.
pub type OutboundEventDispatchHandle = Arc<dyn OutboundEventDispatchHook>;

impl fmt::Debug for dyn OutboundEventDispatchHook {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("OutboundEventDispatchHook")
    }
}

/// Clone-shared registry so service can install after [`ProcessEngine`](crate::engine::process_engine::ProcessEngine)
/// construction without mutating a frozen `Arc` of the whole configuration.
#[derive(Clone, Default)]
pub struct OutboundEventDispatchRegistry {
    inner: Arc<Mutex<Option<OutboundEventDispatchHandle>>>,
}

impl fmt::Debug for OutboundEventDispatchRegistry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OutboundEventDispatchRegistry")
            .field("installed", &self.inner.lock().unwrap().is_some())
            .finish()
    }
}

impl OutboundEventDispatchRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Install (or replace) the host outbound pipeline. Last writer wins.
    pub fn install(&self, hook: OutboundEventDispatchHandle) {
        *self.inner.lock().unwrap() = Some(hook);
    }

    /// Remove any installed hook (engine-only no-op path).
    pub fn clear(&self) {
        *self.inner.lock().unwrap() = None;
    }

    pub fn is_installed(&self) -> bool {
        self.inner.lock().unwrap().is_some()
    }

    /// Run transform + adapter when a hook is installed; otherwise succeed as
    /// in-memory no-op (engine unit tests without event-registry-service).
    pub fn dispatch(&self, request: &OutboundEventDispatchRequest) -> Result<(), FlowableError> {
        let guard = self.inner.lock().unwrap();
        match guard.as_ref() {
            Some(hook) => hook.dispatch_outbound(request),
            None => Ok(()),
        }
    }
}
