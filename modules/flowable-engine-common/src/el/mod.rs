//! Shared SimpleExpression (UEL-subset) evaluator and method registry.
//!
//! Extracted from `flowable-engine` so BPMN and CMMN can evaluate expressions
//! without creating a dependency cycle (`engine` already depends on `cmmn-engine`).

pub mod expression;
pub mod method_registry;
pub mod variable_container;

pub use expression::{evaluate_composite_expression, Expression, SimpleExpression};
pub use method_registry::{
    with_expression_method_registry, ExpressionMethodRegistry,
};
pub use variable_container::{MapVariableContainer, VariableContainer};
