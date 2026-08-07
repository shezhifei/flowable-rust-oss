//! CMMN lifecycle listener execution.
//!
//! Java fires these from two helpers:
//! - `CaseInstanceLifeCycleListenerUtil.callLifecycleListeners`
//!   (CaseInstanceLifeCycleListenerUtil.java:35-85) for case instance state transitions
//! - `CmmnListenerNotificationHelper.executeLifecycleListeners`
//!   (CmmnListenerNotificationHelper.java:103-160) for plan item state transitions
//!
//! Both share the same shape: return early when the state did not actually change, take the
//! listeners declared on the model element, keep the ones whose `sourceState`/`targetState`
//! filters match, then append the listeners registered on the engine configuration.
//!
//! Java resolves `class` / `delegateExpression` through Spring / the bean registry. Rust has no
//! bean container, so a `class` or `delegateExpression` listener resolves through a name →
//! handler registry on the engine (`CmmnLifecycleListenerRegistry`), mirroring the BPMN-side
//! `LocalExecutionListenerRegistry` precedent (bpmn/listener/listener_registry.rs). An
//! `expression` listener is evaluated with the shared Rust EL dialect (P104), matching
//! `ExpressionPlanItemLifecycleListener.stateChanged`, which does
//! `expression.getValue(planItemInstance)`.

use crate::error::CmmnError;
use crate::models::{CmmnLifecycleListener, CmmnListenerImplementationType};
use flowable_engine_common::el::{
    Expression, ExpressionMethodRegistry, MapVariableContainer, SimpleExpression,
    with_expression_method_registry,
};
use serde_json::{Map, Value};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

thread_local! {
    /// The registry of the `CmmnRuntimeService` whose call is currently on the stack.
    ///
    /// Most of the CMMN state machine lives in free functions that take a `DbSession` rather
    /// than `&self` (`maybe_complete_case` and friends), so the registry is installed once at
    /// the service entry point and read back where a transition actually fires. This mirrors
    /// `with_expression_method_registry` in flowable-engine-common.
    static CURRENT_REGISTRY: RefCell<Option<Arc<RwLock<CmmnLifecycleListenerRegistry>>>> =
        const { RefCell::new(None) };
}

/// Installs `registry` as the current lifecycle listener registry until dropped, restoring the
/// previous value afterwards so nesting is safe. Service methods that drive state transitions
/// install it for the rest of their body with `let _guard = …;`.
pub(crate) struct LifecycleListenerRegistryGuard {
    previous: Option<Arc<RwLock<CmmnLifecycleListenerRegistry>>>,
}

impl LifecycleListenerRegistryGuard {
    pub(crate) fn install(registry: &Arc<RwLock<CmmnLifecycleListenerRegistry>>) -> Self {
        let previous =
            CURRENT_REGISTRY.with(|current| current.borrow_mut().replace(Arc::clone(registry)));
        Self { previous }
    }
}

impl Drop for LifecycleListenerRegistryGuard {
    fn drop(&mut self) {
        let previous = self.previous.take();
        CURRENT_REGISTRY.with(|current| {
            *current.borrow_mut() = previous;
        });
    }
}

fn current_lifecycle_listener_registry() -> Option<Arc<RwLock<CmmnLifecycleListenerRegistry>>> {
    CURRENT_REGISTRY.with(|current| current.borrow().clone())
}

/// Which kind of element changed state — Java dispatches case instance transitions through
/// `CaseInstanceLifeCycleListenerUtil` and plan item transitions through
/// `CmmnListenerNotificationHelper`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CmmnLifecycleScope {
    /// A case instance state transition (CaseInstanceLifeCycleListenerUtil.java:35-85).
    CaseInstance,
    /// A plan item instance state transition (CmmnListenerNotificationHelper.java:103-160).
    PlanItem,
}

/// What Java passes to the listener: the entity that changed state plus the old and new state.
///
/// Java hands over the whole `CaseInstance` / `DelegatePlanItemInstance`. Rust passes the ids and
/// the states plus a variable snapshot, which is what a listener can act on without a live
/// command context.
#[derive(Clone, Debug)]
pub struct CmmnLifecycleListenerContext {
    pub scope: CmmnLifecycleScope,
    pub case_instance_id: String,
    pub case_definition_id: String,
    /// The plan item instance id, for `CmmnLifecycleScope::PlanItem` only.
    pub plan_item_instance_id: Option<String>,
    /// The plan item definition id that declared the listener, for `PlanItem` only.
    pub plan_item_definition_id: Option<String>,
    /// Java `planItemDefinitionType` (`stage` / `milestone` / `humantask` / …).
    pub plan_item_definition_type: Option<String>,
    /// The lowercase CMMN spec state names Java uses
    /// (CaseInstanceState.java:28-33, PlanItemInstanceState.java).
    pub old_state: String,
    pub new_state: String,
    pub tenant_id: Option<String>,
    /// Case variable snapshot, for expression evaluation.
    pub variables: Map<String, Value>,
}

/// Rust equivalent of Java `CaseInstanceLifecycleListener` / `PlanItemInstanceLifecycleListener`
/// (both single-method `stateChanged` interfaces).
pub trait CmmnLifecycleListenerHandler: Send + Sync {
    fn state_changed(&self, context: &CmmnLifecycleListenerContext) -> Result<(), CmmnError>;
}

/// Name → handler registry, the minimal stand-in for Java's bean container. A `class` listener
/// is looked up by its literal `class` attribute value; a `delegateExpression` listener by the
/// bean name inside `${…}`.
#[derive(Clone, Default)]
pub struct CmmnLifecycleListenerRegistry {
    handlers: BTreeMap<String, Arc<dyn CmmnLifecycleListenerHandler>>,
    /// Bean/static methods usable from `expression` listeners, so an expression listener can
    /// produce a side effect (`${auditBean.record(...)}`) the way Java's UEL can. Rust's
    /// `SimpleExpression` is otherwise read-only.
    expression_methods: ExpressionMethodRegistry,
}

impl std::fmt::Debug for CmmnLifecycleListenerRegistry {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CmmnLifecycleListenerRegistry")
            .field("handlers", &self.handlers.keys().collect::<Vec<_>>())
            .field("expression_methods", &self.expression_methods)
            .finish()
    }
}

impl CmmnLifecycleListenerRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a handler under the name a `class` attribute or a `${bean}`
    /// `delegateExpression` will refer to.
    pub fn register(
        &mut self,
        name: impl Into<String>,
        handler: Arc<dyn CmmnLifecycleListenerHandler>,
    ) {
        self.handlers.insert(name.into(), handler);
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn CmmnLifecycleListenerHandler>> {
        self.handlers.get(name).cloned()
    }

    /// The expression method registry used while evaluating `expression` listeners.
    pub fn expression_methods(&self) -> &ExpressionMethodRegistry {
        &self.expression_methods
    }
}

/// Run every listener in `listeners` whose state filter matches, against the ambient registry.
///
/// Shared by the case-instance path (CaseInstanceLifeCycleListenerUtil.java:41-48) and the
/// plan-item path (CmmnListenerNotificationHelper.java:111-115); both do "same-state early
/// return, filter, execute" in that order.
pub(crate) fn fire_matching_lifecycle_listeners(
    listeners: &[CmmnLifecycleListener],
    context: &CmmnLifecycleListenerContext,
) -> Result<(), CmmnError> {
    // CaseInstanceLifeCycleListenerUtil.java:36-38 / CmmnListenerNotificationHelper.java:104-106
    if context.old_state.eq_ignore_ascii_case(&context.new_state) {
        return Ok(());
    }
    if listeners.is_empty() {
        return Ok(());
    }
    let registry = current_lifecycle_listener_registry();
    let guard = registry.as_ref().and_then(|registry| registry.read().ok());
    for listener in listeners {
        // CaseInstanceLifeCycleListenerUtil.java:48 / CmmnListenerNotificationHelper.java:115
        if !listener.matches(&context.old_state, &context.new_state) {
            continue;
        }
        execute_lifecycle_listener(listener, context, guard.as_deref())?;
    }
    Ok(())
}

/// Java `CaseInstanceLifeCycleListenerUtil.stateMatches` is applied before this point (see
/// `CmmnLifecycleListener::matches`); this only runs the listener body.
///
/// Errors propagate to the caller. Java lets a listener exception escape and roll back the
/// command (`CmmnListenerNotificationHelper.executeLifecycleListener` does not catch —
/// CmmnListenerNotificationHelper.java:145-152), so failing the transition is the aligned
/// behaviour.
pub(crate) fn execute_lifecycle_listener(
    listener: &CmmnLifecycleListener,
    context: &CmmnLifecycleListenerContext,
    registry: Option<&CmmnLifecycleListenerRegistry>,
) -> Result<(), CmmnError> {
    match listener.implementation_type {
        // Java ExpressionPlanItemLifecycleListener.stateChanged: `expression.getValue(instance)`.
        // The value is discarded; only the side effect matters.
        CmmnListenerImplementationType::Expression => {
            evaluate_expression_listener(&listener.implementation, context, registry);
            Ok(())
        }
        // Java resolves `class` by instantiating it and `delegateExpression` by resolving the
        // bean (CmmnListenerNotificationHelper.java:162-169 createCaseLifecycleListener). Rust
        // resolves both through the name → handler registry.
        CmmnListenerImplementationType::Class => {
            invoke_registered_listener(&listener.implementation, context, registry)
        }
        CmmnListenerImplementationType::DelegateExpression => {
            let bean_name = delegate_expression_bean_name(&listener.implementation);
            invoke_registered_listener(bean_name, context, registry)
        }
    }
}

/// Strip the `${…}` wrapper off a `delegateExpression` to get the bean name Java would resolve.
/// A non-`${…}` value is used as-is.
fn delegate_expression_bean_name(implementation: &str) -> &str {
    let trimmed = implementation.trim();
    trimmed
        .strip_prefix("${")
        .or_else(|| trimmed.strip_prefix("#{"))
        .and_then(|rest| rest.strip_suffix('}'))
        .map(str::trim)
        .unwrap_or(trimmed)
}

fn invoke_registered_listener(
    name: &str,
    context: &CmmnLifecycleListenerContext,
    registry: Option<&CmmnLifecycleListenerRegistry>,
) -> Result<(), CmmnError> {
    let handler = registry.and_then(|registry| registry.get(name));
    let handler = handler.ok_or_else(|| {
        CmmnError::execution(format!(
            "no CMMN lifecycle listener `{name}` is registered on the engine (Java would resolve \
             it as a class or Spring bean; register it with \
             `CmmnEngine::register_lifecycle_listener`)"
        ))
    })?;
    handler.state_changed(context)
}

/// Evaluate an `expression` listener body. The result is discarded — Java's
/// `ExpressionPlanItemLifecycleListener` ignores it too.
fn evaluate_expression_listener(
    implementation: &str,
    context: &CmmnLifecycleListenerContext,
    registry: Option<&CmmnLifecycleListenerRegistry>,
) {
    let trimmed = implementation.trim();
    if trimmed.is_empty() {
        return;
    }
    let scope = expression_scope(context);
    match registry {
        // Route through the registry so `${bean.method(...)}` side effects resolve.
        Some(registry) => {
            with_expression_method_registry(registry.expression_methods(), || {
                let _ = SimpleExpression::new(trimmed.to_string()).get_value(&scope);
            });
        }
        None => {
            let _ = SimpleExpression::new(trimmed.to_string()).get_value(&scope);
        }
    }
}

/// Variable scope for an expression listener: the case variables, plus the transition metadata
/// Java exposes through the delegate instance.
fn expression_scope(context: &CmmnLifecycleListenerContext) -> MapVariableContainer {
    let mut variables = context.variables.clone();
    variables.insert(
        "caseInstanceId".to_string(),
        Value::String(context.case_instance_id.clone()),
    );
    variables.insert(
        "oldState".to_string(),
        Value::String(context.old_state.clone()),
    );
    variables.insert(
        "newState".to_string(),
        Value::String(context.new_state.clone()),
    );
    if let Some(plan_item_instance_id) = &context.plan_item_instance_id {
        variables.insert(
            "planItemInstanceId".to_string(),
            Value::String(plan_item_instance_id.clone()),
        );
    }
    if let Some(plan_item_definition_id) = &context.plan_item_definition_id {
        variables.insert(
            "planItemDefinitionId".to_string(),
            Value::String(plan_item_definition_id.clone()),
        );
    }
    MapVariableContainer::from_json_map(&variables).with_tenant_id(context.tenant_id.clone())
}
