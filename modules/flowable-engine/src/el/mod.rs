//! Expression Language surface for the process engine.
//!
//! SimpleExpression and the method registry live in `flowable-engine-common`
//! so CMMN (and other engines) can evaluate EL without depending on
//! `flowable-engine`. This module re-exports them at the historical
//! `flowable_engine::el::{expression, method_registry}` paths and keeps
//! BPMN-only condition wrappers here.

pub mod condition;
pub mod uel_expression_condition;

// Re-export moved modules so existing `crate::el::expression::…` /
// `flowable_engine::el::method_registry::…` import paths stay stable.
pub use flowable_engine_common::el::{expression, method_registry, variable_container};
