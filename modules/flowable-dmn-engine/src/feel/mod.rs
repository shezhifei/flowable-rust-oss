pub mod ast;
pub mod evaluator;
pub mod lexer;
pub mod parser;
pub mod token;

use crate::error::DmnError;
use serde_json::Value;
use std::collections::HashMap;

pub fn evaluate(source: &str, context: &HashMap<String, Value>) -> Result<Value, DmnError> {
    evaluator::evaluate(&parser::parse(lexer::lex(source)?)?, context)
}
