//! Shared SimpleExpression (UEL-subset) evaluator and method registry.
//!
//! Extracted from `flowable-engine` so BPMN and CMMN can evaluate expressions
//! without creating a dependency cycle (`engine` already depends on `cmmn-engine`).

pub mod expression;
pub mod method_registry;
pub mod variable_container;

pub use expression::{
    Expression, ExpressionEvalError, SimpleExpression, evaluate_composite_expression,
};
pub use method_registry::{ExpressionMethodRegistry, with_expression_method_registry};
pub use variable_container::{MapVariableContainer, VariableContainer};
